//! Receives tray changes from the upstream hook without polling Explorer.

use crate::protocol::{self, MAX_PACKET, Packet};
use std::{
    ffi::OsStr,
    io,
    os::windows::ffi::OsStrExt,
    path::Path,
    ptr::{null, null_mut},
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};
use windows_sys::Win32::{
    Foundation::{
        CloseHandle, ERROR_ALREADY_EXISTS, ERROR_IO_PENDING, ERROR_MORE_DATA, ERROR_PIPE_CONNECTED,
        FreeLibrary, GetLastError, HANDLE, HWND, INVALID_HANDLE_VALUE, WAIT_OBJECT_0, WAIT_TIMEOUT,
    },
    Storage::FileSystem::{
        FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED, PIPE_ACCESS_INBOUND, ReadFile,
    },
    System::{
        IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED},
        LibraryLoader::{GetProcAddress, LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW},
        Pipes::{
            ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, GetNamedPipeClientProcessId,
            PIPE_READMODE_MESSAGE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_MESSAGE, PIPE_WAIT,
        },
        Threading::{
            CreateEventW, CreateMutexW, ReleaseMutex, WaitForMultipleObjects, WaitForSingleObject,
        },
    },
    UI::WindowsAndMessaging::{
        FindWindowW, GetShellWindow, GetWindowThreadProcessId, HHOOK, HWND_BROADCAST, PostMessageW,
        PostThreadMessageW, RegisterWindowMessageW, SendNotifyMessageW, SetWindowsHookExW,
        UnhookWindowsHookEx, WH_GETMESSAGE, WM_APP, WM_NULL,
    },
};

pub const TRAY_CHANGED: u32 = WM_APP + 10;
pub type Inbox = Arc<Mutex<Vec<Event>>>;

pub enum Event {
    Connected,
    Disconnected(String),
    Changed(protocol::Change),
}

pub fn wide(text: impl AsRef<OsStr>) -> Vec<u16> {
    text.as_ref().encode_wide().chain(Some(0)).collect()
}

struct Handle(HANDLE);

impl Handle {
    fn new(raw: HANDLE) -> io::Result<Self> {
        if raw.is_null() || raw == INVALID_HANDLE_VALUE {
            Err(io::Error::last_os_error())
        } else {
            Ok(Self(raw))
        }
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

// Only thread-agnostic Win32 kernel handles cross threads.
unsafe impl Send for Handle {}
unsafe impl Sync for Handle {}

pub struct Instance {
    handle: Handle,
}

impl Instance {
    pub fn acquire(name: &str) -> io::Result<Self> {
        let name = wide(name);
        let handle = Handle::new(unsafe { CreateMutexW(null(), 1, name.as_ptr()) })?;
        if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "A tray monitor is already running.",
            ));
        }
        Ok(Self { handle })
    }
}

impl Drop for Instance {
    fn drop(&mut self) {
        unsafe { ReleaseMutex(self.handle.0) };
    }
}

pub struct Monitor {
    stop: Arc<Handle>,
    worker: Option<thread::JoinHandle<()>>,
}

impl Monitor {
    pub fn start(window: HWND, inbox: Inbox, library: &Path) -> io::Result<Self> {
        if !library.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "YASBTrayHook.dll is missing.",
            ));
        }
        let stop = Arc::new(event()?);
        let worker_stop = stop.clone();
        let target = window as usize;
        let path = wide(library.as_os_str());
        let worker = thread::Builder::new()
            .name("explorer-tray".into())
            .spawn(move || {
                run_worker(worker_stop.0, target as HWND, &inbox, &path);
            })?;
        Ok(Self {
            stop,
            worker: Some(worker),
        })
    }
}

impl Drop for Monitor {
    fn drop(&mut self) {
        unsafe { windows_sys::Win32::System::Threading::SetEvent(self.stop.0) };
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn event() -> io::Result<Handle> {
    Handle::new(unsafe { CreateEventW(null(), 1, 0, null()) })
}

fn deliver(window: HWND, inbox: &Inbox, event: Event) {
    let mut events = inbox.lock().unwrap_or_else(|err| err.into_inner());
    let notify = events.is_empty();
    events.push(event);
    if notify {
        unsafe { PostMessageW(window, TRAY_CHANGED, 0, 0) };
    }
}

fn stopped(stop: HANDLE, wait: Duration) -> bool {
    unsafe { WaitForSingleObject(stop, wait.as_millis() as u32) != WAIT_TIMEOUT }
}

fn run_worker(stop: HANDLE, window: HWND, inbox: &Inbox, path: &[u16]) {
    // The upstream Explorer DLL expects these names. Hold its mutex on this worker
    // so Explorer can detach its hook if the application exits or crashes.
    while !stopped(stop, Duration::ZERO) {
        let session = (|| {
            let watchdog = Instance::acquire(r"Global\YASBTrayHookAlive")?;
            let result = receive(stop, window, inbox, path);
            drop(watchdog);
            result
        })();
        if stopped(stop, Duration::ZERO) {
            break;
        }
        let status = session.err().map_or_else(
            || "Explorer disconnected".to_owned(),
            |error| error.to_string(),
        );
        deliver(window, inbox, Event::Disconnected(status));
        if stopped(stop, Duration::from_secs(2)) {
            break;
        }
    }
}

fn receive(stop: HANDLE, window: HWND, inbox: &Inbox, path: &[u16]) -> io::Result<()> {
    let shell = unsafe { GetShellWindow() };
    let mut explorer_pid = 0;
    if !shell.is_null() {
        unsafe { GetWindowThreadProcessId(shell, &mut explorer_pid) };
    }
    let tray = unsafe { FindWindowW(wide("Shell_TrayWnd").as_ptr(), null()) };
    let mut tray_pid = 0;
    let tray_thread = if tray.is_null() {
        0
    } else {
        unsafe { GetWindowThreadProcessId(tray, &mut tray_pid) }
    };
    if explorer_pid == 0 || tray_thread == 0 || tray_pid != explorer_pid {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "Explorer's tray window is unavailable.",
        ));
    }

    let name = wide(r"\\.\pipe\yasb_systray_monitor");
    let pipe = Handle::new(unsafe {
        CreateNamedPipeW(
            name.as_ptr(),
            PIPE_ACCESS_INBOUND | FILE_FLAG_OVERLAPPED | FILE_FLAG_FIRST_PIPE_INSTANCE,
            PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            1,
            0,
            16 * 1024,
            0,
            null(),
        )
    })?;

    let io_event = event()?;
    let injection = Injection::start(path, tray_thread)?;
    let mut op = operation(&io_event);
    let connected = unsafe { ConnectNamedPipe(pipe.0, &mut op) };
    if connected == 0 {
        match unsafe { GetLastError() } {
            ERROR_PIPE_CONNECTED => {}
            ERROR_IO_PENDING => {
                wait_io(stop, pipe.0, &mut op, Some(5000))?;
            }
            error => return Err(io::Error::from_raw_os_error(error as i32)),
        }
    }

    let mut client_pid = 0;
    if unsafe { GetNamedPipeClientProcessId(pipe.0, &mut client_pid) } == 0
        || client_pid != explorer_pid
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Unexpected tray pipe client.",
        ));
    }

    let mut message = Vec::new();
    let mut buffer = [0u8; 8192];
    let mut injection = Some(injection);
    while !stopped(stop, Duration::ZERO) {
        let mut op = operation(&io_event);
        let result = unsafe {
            ReadFile(
                pipe.0,
                buffer.as_mut_ptr(),
                buffer.len() as u32,
                null_mut(),
                &mut op,
            )
        };
        let (read, more) = if result != 0 {
            completed_io(pipe.0, &op)?
        } else {
            match unsafe { GetLastError() } {
                ERROR_MORE_DATA => completed_io(pipe.0, &op)?,
                ERROR_IO_PENDING => {
                    wait_io(stop, pipe.0, &mut op, injection.as_ref().map(|_| 5000))?
                }
                error => return Err(io::Error::from_raw_os_error(error as i32)),
            }
        };
        if read as usize > buffer.len() || message.len() + read as usize > MAX_PACKET {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Oversized tray packet.",
            ));
        }
        message.extend_from_slice(&buffer[..read as usize]);
        if !more {
            match protocol::parse(&message) {
                Some(Packet::Ready) if injection.is_some() => {
                    drop(injection.take());
                    deliver(window, inbox, Event::Connected);
                }
                Some(Packet::Change(change)) => deliver(window, inbox, Event::Changed(change)),
                _ => {}
            }
            message.clear();
        }
    }
    unsafe { DisconnectNamedPipe(pipe.0) };
    Ok(())
}

fn operation(event: &Handle) -> OVERLAPPED {
    unsafe { windows_sys::Win32::System::Threading::ResetEvent(event.0) };
    OVERLAPPED {
        hEvent: event.0,
        ..unsafe { std::mem::zeroed() }
    }
}

fn wait_io(
    stop: HANDLE,
    pipe: HANDLE,
    op: &mut OVERLAPPED,
    timeout: Option<u32>,
) -> io::Result<(u32, bool)> {
    let handles = [stop, op.hEvent];
    let status =
        unsafe { WaitForMultipleObjects(2, handles.as_ptr(), 0, timeout.unwrap_or(u32::MAX)) };
    let mut read = 0;
    if status != WAIT_OBJECT_0 + 1 {
        // The stack OVERLAPPED and read buffer must outlive pending kernel I/O.
        unsafe {
            CancelIoEx(pipe, op);
            GetOverlappedResult(pipe, op, &mut read, 1);
        }
        return Err(io::Error::new(
            if status == WAIT_TIMEOUT {
                io::ErrorKind::TimedOut
            } else {
                io::ErrorKind::Interrupted
            },
            "Explorer pipe wait ended.",
        ));
    }
    completed_io(pipe, op)
}

fn completed_io(pipe: HANDLE, op: &OVERLAPPED) -> io::Result<(u32, bool)> {
    let mut read = 0;
    if unsafe { GetOverlappedResult(pipe, op, &mut read, 0) } != 0 {
        return Ok((read, false));
    }
    let error = unsafe { GetLastError() };
    if error == ERROR_MORE_DATA {
        Ok((read, true))
    } else {
        Err(io::Error::from_raw_os_error(error as i32))
    }
}

struct Injection {
    module: windows_sys::Win32::Foundation::HMODULE,
    hook: HHOOK,
}

impl Injection {
    fn start(path: &[u16], explorer_thread: u32) -> io::Result<Self> {
        let module =
            unsafe { LoadLibraryExW(path.as_ptr(), null_mut(), LOAD_LIBRARY_SEARCH_SYSTEM32) };
        if module.is_null() {
            return Err(io::Error::last_os_error());
        }
        let mut injection = Self {
            module,
            hook: null_mut(),
        };
        let Some(procedure) = (unsafe { GetProcAddress(module, c"GetMsgProc".as_ptr().cast()) })
        else {
            return Err(io::Error::last_os_error());
        };
        // GetMsgProc is exported with the Win32 HOOKPROC signature.
        let procedure = unsafe {
            std::mem::transmute::<_, unsafe extern "system" fn(i32, usize, isize) -> isize>(
                procedure,
            )
        };
        injection.hook =
            unsafe { SetWindowsHookExW(WH_GETMESSAGE, Some(procedure), module, explorer_thread) };
        if injection.hook.is_null()
            || unsafe { PostThreadMessageW(explorer_thread, WM_NULL, 0, 0) } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(injection)
    }
}

impl Drop for Injection {
    fn drop(&mut self) {
        if !self.hook.is_null() {
            unsafe { UnhookWindowsHookEx(self.hook) };
        }
        unsafe { FreeLibrary(self.module) };
    }
}

pub fn request_icons() {
    let message = unsafe { RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()) };
    if message != 0 {
        unsafe { SendNotifyMessageW(HWND_BROADCAST, message, 0, 0) };
    }
}
