use crate::{
    composite::{self, Pixel},
    hook::{self, Event, Inbox, Instance, Monitor, wide},
    model::{Edge, Item, Settings, Tray},
    style::{self, Style},
};
use std::{
    cell::RefCell,
    fs, io,
    mem::{size_of, zeroed},
    path::{Path, PathBuf},
    process::Command,
    ptr::{null, null_mut},
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM},
    Graphics::Gdi::*,
    System::{
        LibraryLoader::GetModuleHandleW,
        Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW},
    },
    UI::{
        HiDpi::{
            DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForWindow,
            SetProcessDpiAwarenessContext,
        },
        Input::KeyboardAndMouse::{
            GetKeyState, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent, VK_MENU,
        },
        WindowsAndMessaging::*,
    },
};

const OVERFLOW_COLUMNS: usize = 6;
const WM_MOUSELEAVE: u32 = 0x02A3;
const PAINT_FRAME: u32 = WM_APP + 11;
const NEXT: usize = usize::MAX - 1;
const PREVIOUS: usize = usize::MAX - 2;

thread_local! {
    static UI: RefCell<Option<Ui>> = const { RefCell::new(None) };
}

struct Slot {
    target: usize,
    bounds: RECT,
}

#[derive(Default)]
struct View {
    slots: Vec<Slot>,
    hover: Option<usize>,
    status: String,
    geometry: Option<(i32, i32, i32, i32, i32)>,
    repaint_pending: bool,
}

struct Ui {
    dock: HWND,
    popup: HWND,
    dock_view: View,
    popup_view: View,
    inbox: Inbox,
    tray: Tray,
    settings: Settings,
    settings_path: PathBuf,
    style: Style,
    style_path: PathBuf,
    connected: bool,
    connection_status: String,
    popup_open: bool,
    page: usize,
}

pub fn run() -> io::Result<()> {
    let _instance = Instance::acquire("yasb_reborn").map_err(|error| {
        if error.kind() == io::ErrorKind::AlreadyExists {
            io::Error::new(
                error.kind(),
                "Exit YASB or the other Floating Tray instance before starting this copy.",
            )
        } else {
            error
        }
    })?;
    let exe_dir = std::env::current_exe()?.parent().unwrap().to_owned();
    let dll = exe_dir.join("YASBTrayHook.dll");
    let settings_path =
        PathBuf::from(std::env::var_os("LOCALAPPDATA").ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "LOCALAPPDATA is unavailable.")
        })?)
        .join("FloatingTray")
        .join("settings.txt");
    let settings = Settings::parse(&fs::read_to_string(&settings_path).unwrap_or_default());
    let style_path = settings_path.with_file_name("style.ini");
    let style = match read_style(&style_path) {
        Ok(style) => style,
        Err(error) => {
            show_error(&format!(
                "Could not load style.ini. Using the default style.\n{error}"
            ));
            Style::default()
        }
    };

    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let class = wide("FloatingTray");
        let instance = GetModuleHandleW(null());
        let wnd_class = WNDCLASSW {
            style: CS_DBLCLKS,
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            lpszClassName: class.as_ptr(),
            ..zeroed()
        };
        if RegisterClassW(&wnd_class) == 0 {
            return Err(io::Error::last_os_error());
        }
        let create = |title: &str, owner| {
            CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_LAYERED,
                class.as_ptr(),
                wide(title).as_ptr(),
                WS_POPUP,
                0,
                0,
                1,
                1,
                owner,
                null_mut(),
                instance,
                null(),
            )
        };
        let dock = create("Floating Tray", null_mut());
        if dock.is_null() {
            return Err(io::Error::last_os_error());
        }
        let popup = create("Tray icons", dock);
        if popup.is_null() {
            return Err(io::Error::last_os_error());
        }
        let inbox = Inbox::default();
        let monitor = Monitor::start(dock, inbox.clone(), &dll)?;
        let mut ui = Ui {
            dock,
            popup,
            dock_view: View::default(),
            popup_view: View::default(),
            inbox,
            tray: Tray::default(),
            settings,
            settings_path,
            style,
            style_path,
            connected: false,
            connection_status: "Connecting to Explorer…".to_owned(),
            popup_open: false,
            page: 0,
        };
        ui.layout();
        UI.with(|slot| slot.replace(Some(ui)));
        ShowWindow(dock, SW_SHOWNOACTIVATE);
        let mut message: MSG = zeroed();
        let result = loop {
            match GetMessageW(&mut message, null_mut(), 0, 0) {
                -1 => break Err(io::Error::last_os_error()),
                0 => break Ok(()),
                _ => {
                    TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            }
        };
        drop(monitor);
        UI.with(|slot| slot.replace(None));
        result
    }
}

pub fn show_error(message: &str) {
    unsafe {
        MessageBoxW(
            null_mut(),
            wide(message).as_ptr(),
            wide("Floating Tray").as_ptr(),
            MB_OK | MB_ICONERROR,
        );
    }
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let mut menu = None;
    let handled = UI.with(|slot| {
        let Ok(mut slot) = slot.try_borrow_mut() else {
            return false;
        };
        let Some(ui) = slot.as_mut() else {
            return false;
        };
        let is_popup = hwnd == ui.popup;
        match message {
            hook::TRAY_CHANGED => ui.update_icons(),
            WM_DISPLAYCHANGE | WM_DPICHANGED | WM_SETTINGCHANGE if !is_popup => ui.layout(),
            WM_LBUTTONUP => ui.click(is_popup, lparam, WM_LBUTTONUP),
            WM_LBUTTONDBLCLK => ui.click(is_popup, lparam, WM_LBUTTONDBLCLK),
            WM_MBUTTONUP => ui.click(is_popup, lparam, WM_MBUTTONUP),
            WM_RBUTTONUP => {
                if ui
                    .hit(is_popup, lparam)
                    .and_then(|target| ui.tray.items.get(target))
                    .is_some()
                {
                    ui.click(is_popup, lparam, WM_RBUTTONUP);
                } else {
                    menu = Some((ui.dock, ui.settings.edge));
                }
            }
            WM_MOUSEWHEEL if is_popup => ui.page_by(if (wparam >> 16) as i16 > 0 { -1 } else { 1 }),
            WM_MOUSEMOVE => ui.hover(hwnd, is_popup, lparam),
            WM_MOUSELEAVE => ui.leave(hwnd, is_popup),
            PAINT_FRAME => {
                let view = if is_popup {
                    &mut ui.popup_view
                } else {
                    &mut ui.dock_view
                };
                view.repaint_pending = false;
                if !is_popup || ui.popup_open {
                    paint(hwnd, view, &ui.tray, ui.style);
                }
            }
            WM_ERASEBKGND => return true,
            WM_CLOSE if is_popup => ui.hide_popup(),
            WM_DESTROY if !is_popup => unsafe { PostQuitMessage(0) },
            _ => return false,
        }
        true
    });
    if let Some((dock, edge)) = menu {
        show_menu(dock, edge);
    }
    if handled {
        0
    } else {
        unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
    }
}

impl Ui {
    fn update_icons(&mut self) {
        let updates = {
            let mut inbox = self.inbox.lock().unwrap_or_else(|error| error.into_inner());
            std::mem::take(&mut *inbox)
        };
        let mut refresh = false;
        for update in updates {
            match update {
                Event::Connected => {
                    self.connected = true;
                    refresh = true;
                }
                Event::Disconnected(status) => {
                    self.connected = false;
                    self.connection_status = status;
                    self.tray.items.clear();
                    refresh = false;
                }
                Event::Changed(change) => self.tray.apply(change, exe_for_window),
            }
        }
        self.layout();
        if refresh {
            hook::request_icons();
        }
    }

    fn view_mut(&mut self, popup: bool) -> &mut View {
        if popup {
            &mut self.popup_view
        } else {
            &mut self.dock_view
        }
    }

    fn hit(&self, popup: bool, lparam: LPARAM) -> Option<usize> {
        let (x, y) = (
            (lparam as u16 as i16) as i32,
            ((lparam >> 16) as u16 as i16) as i32,
        );
        let view = if popup {
            &self.popup_view
        } else {
            &self.dock_view
        };
        view.slots
            .iter()
            .find(|slot| {
                let rect = slot.bounds;
                x >= rect.left && x < rect.right && y >= rect.top && y < rect.bottom
            })
            .map(|slot| slot.target)
    }

    fn click(&mut self, popup: bool, lparam: LPARAM, button: u32) {
        let Some(target) = self.hit(popup, lparam) else {
            return;
        };
        match target {
            usize::MAX if button == WM_LBUTTONUP => {
                self.popup_open = !self.popup_open;
                self.page = 0;
                self.layout();
            }
            NEXT if button == WM_LBUTTONUP => self.page_by(1),
            PREVIOUS if button == WM_LBUTTONUP => self.page_by(-1),
            _ => {
                let Some(item) = self.tray.items.get(target) else {
                    return;
                };
                if button == WM_LBUTTONUP && unsafe { GetKeyState(VK_MENU as i32) } < 0 {
                    if let Some(key) = item.key.as_deref() {
                        self.settings.toggle(key);
                        self.save();
                        self.hide_popup();
                        self.layout();
                    }
                } else {
                    send_click(item, button);
                    if popup {
                        self.hide_popup();
                    }
                }
            }
        }
    }

    fn hover(&mut self, hwnd: HWND, popup: bool, lparam: LPARAM) {
        let target = self.hit(popup, lparam);
        let previous = self.view_mut(popup).hover;
        if target != previous {
            self.view_mut(popup).hover = target;
            let text = target
                .and_then(|index| self.tray.items.get(index))
                .map(Item::label);
            if popup {
                self.popup_view.status = text.unwrap_or("Alt+click to pin an icon").to_owned();
            }
            request_repaint(hwnd, self.view_mut(popup));
        }
        if previous.is_none() && target.is_some() {
            let mut tracking = TRACKMOUSEEVENT {
                cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                dwFlags: TME_LEAVE,
                hwndTrack: hwnd,
                dwHoverTime: 0,
            };
            unsafe { TrackMouseEvent(&mut tracking) };
        }
    }

    fn leave(&mut self, hwnd: HWND, popup: bool) {
        self.view_mut(popup).hover = None;
        if popup && self.connected && !self.popup_view.slots.is_empty() {
            self.popup_view.status = "Alt+click to pin an icon".to_owned();
        }
        request_repaint(hwnd, self.view_mut(popup));
    }

    fn page_by(&mut self, amount: isize) {
        self.page = self.page.saturating_add_signed(amount);
        self.layout();
    }

    fn hide_popup(&mut self) {
        self.popup_open = false;
        unsafe { ShowWindow(self.popup, SW_HIDE) };
    }

    fn save(&self) {
        if let Err(error) =
            fs::create_dir_all(self.settings_path.parent().unwrap()).and_then(|_| {
                let temp = self.settings_path.with_extension("tmp");
                fs::write(&temp, self.settings.encode())?;
                fs::rename(temp, &self.settings_path)
            })
        {
            show_error(&format!("Settings could not be saved. {error}"));
        }
    }

    fn layout(&mut self) {
        let dpi = unsafe { GetDpiForWindow(self.dock) }.max(96) as i32;
        let scaled = |value| value * dpi / 96;
        let style = self.style;
        let cell = scaled(style.icon_size + 2 * style.icon_padding);
        let strip = scaled(style.trigger_size);
        let pad = scaled(style.padding);
        let gap = scaled(style.edge_margin);
        let radius = scaled(2 * style.corner_radius);
        let work = work_area();
        let vertical = self.settings.edge.vertical();
        let (mut pinned, mut overflow) = self.tray.visible(&self.settings);
        let length = if vertical {
            work.bottom - work.top
        } else {
            work.right - work.left
        };
        let capacity = ((length - 2 * (strip + pad + gap)) / cell).max(0) as usize;
        if pinned.len() > capacity {
            let rest = pinned.split_off(capacity);
            overflow.splice(..0, rest);
        }

        self.dock_view.slots.clear();
        let cross = cell + 2 * pad;
        let axis = if pinned.is_empty() {
            self.dock_view.slots.push(Slot {
                target: usize::MAX,
                bounds: rect(pad, pad, cell, cell),
            });
            cross
        } else {
            let project = |start, size| {
                if vertical {
                    rect(pad, start, cell, size)
                } else {
                    rect(start, pad, size, cell)
                }
            };
            let mut cursor = pad;
            self.dock_view.slots.push(Slot {
                target: usize::MAX,
                bounds: project(cursor, strip),
            });
            cursor += strip;
            for index in pinned {
                self.dock_view.slots.push(Slot {
                    target: index,
                    bounds: project(cursor, cell),
                });
                cursor += cell;
            }
            self.dock_view.slots.push(Slot {
                target: usize::MAX,
                bounds: project(cursor, strip),
            });
            cursor + strip + pad
        };

        let (width, height) = if vertical {
            (cross, axis)
        } else {
            (axis, cross)
        };
        let origin = self.settings.edge.anchor(
            (work.left, work.top, work.right, work.bottom),
            (width, height),
            gap,
        );
        place(
            self.dock,
            &mut self.dock_view,
            rect(origin.0, origin.1, width, height),
            radius,
        );
        if !self.popup_open {
            unsafe { ShowWindow(self.popup, SW_HIDE) };
            return;
        }

        let columns =
            OVERFLOW_COLUMNS.min(((work.right - work.left - 2 * pad) / cell).max(1) as usize);
        let rows = (((work.bottom - work.top) / 3 / cell).max(1) as usize).min(6);
        let page_size = columns * rows;
        let pages = overflow.len().div_ceil(page_size).max(1);
        self.page = self.page.min(pages - 1);
        self.popup_view.slots.clear();
        let page = &overflow[(self.page * page_size).min(overflow.len())
            ..((self.page + 1) * page_size).min(overflow.len())];
        let footer = scaled((style.font_size + 10).max(28));
        let pop_width = (columns as i32 * cell + 2 * pad)
            .max(scaled(204))
            .min(work.right - work.left);
        let row_count = page.len().div_ceil(columns).max(1) as i32;
        let pop_height = row_count * cell + footer + 2 * pad;
        let grid_left = (pop_width - columns as i32 * cell) / 2;
        for (offset, &index) in page.iter().enumerate() {
            let left = (offset % columns) as i32 * cell + grid_left;
            let top = (offset / columns) as i32 * cell + pad;
            self.popup_view.slots.push(Slot {
                target: index,
                bounds: rect(left, top, cell, cell),
            });
        }
        if self.page > 0 {
            self.popup_view.slots.push(Slot {
                target: PREVIOUS,
                bounds: rect(pad, pop_height - footer - pad, footer, footer),
            });
        }
        if self.page + 1 < pages {
            self.popup_view.slots.push(Slot {
                target: NEXT,
                bounds: rect(
                    pop_width - footer - pad,
                    pop_height - footer - pad,
                    footer,
                    footer,
                ),
            });
        }
        self.popup_view.status = if !page.is_empty() {
            "Alt+click to pin an icon".to_owned()
        } else if self.connected {
            "No unpinned tray icons".to_owned()
        } else {
            self.connection_status.clone()
        };
        let (x, y) = match self.settings.edge {
            Edge::Right => (
                origin.0 - pop_width - gap,
                origin.1 + (height - pop_height) / 2,
            ),
            Edge::Left => (origin.0 + width + gap, origin.1 + (height - pop_height) / 2),
            Edge::Top => (origin.0 + (width - pop_width) / 2, origin.1 + height + gap),
            Edge::Bottom => (
                origin.0 + (width - pop_width) / 2,
                origin.1 - pop_height - gap,
            ),
        };
        let x = x.clamp(work.left, (work.right - pop_width).max(work.left));
        let y = y.clamp(work.top, (work.bottom - pop_height).max(work.top));
        place(
            self.popup,
            &mut self.popup_view,
            rect(x, y, pop_width, pop_height),
            radius,
        );
        unsafe { ShowWindow(self.popup, SW_SHOWNOACTIVATE) };
    }
}

fn show_menu(dock: HWND, edge: Edge) {
    const FIRST_EDGE: usize = 1;
    const REFRESH: usize = 10;
    const EXIT: usize = 11;
    const EDIT_STYLE: usize = 12;
    const RELOAD_STYLE: usize = 13;
    unsafe {
        let menu = CreatePopupMenu();
        if menu.is_null() {
            return;
        }
        for (index, choice) in Edge::ALL.into_iter().enumerate() {
            let flags = MF_STRING
                | if choice == edge {
                    MF_CHECKED
                } else {
                    MF_UNCHECKED
                };
            AppendMenuW(
                menu,
                flags,
                FIRST_EDGE + index,
                wide(choice.name()).as_ptr(),
            );
        }
        AppendMenuW(menu, MF_SEPARATOR, 0, null());
        AppendMenuW(menu, MF_STRING, REFRESH, wide("Refresh icons").as_ptr());
        AppendMenuW(menu, MF_STRING, EDIT_STYLE, wide("Edit style").as_ptr());
        AppendMenuW(menu, MF_STRING, RELOAD_STYLE, wide("Reload style").as_ptr());
        AppendMenuW(menu, MF_STRING, EXIT, wide("Exit").as_ptr());
        let mut position: POINT = zeroed();
        GetCursorPos(&mut position);
        SetForegroundWindow(dock);
        let command = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON,
            position.x,
            position.y,
            0,
            dock,
            null(),
        ) as usize;
        PostMessageW(dock, WM_NULL, 0, 0);
        DestroyMenu(menu);
        match command {
            EXIT => {
                DestroyWindow(dock);
            }
            REFRESH => hook::request_icons(),
            EDIT_STYLE | RELOAD_STYLE => {
                let path = UI.with(|slot| slot.borrow().as_ref().map(|ui| ui.style_path.clone()));
                if let Some(path) = path {
                    if command == EDIT_STYLE {
                        if let Err(error) = open_style(&path) {
                            show_error(&format!("Could not open style.ini.\n{error}"));
                        }
                    } else {
                        match read_style(&path) {
                            Ok(style) => UI.with(|slot| {
                                if let Some(ui) = slot.borrow_mut().as_mut() {
                                    ui.style = style;
                                    ui.layout();
                                }
                            }),
                            Err(error) => {
                                show_error(&format!("Could not reload style.ini.\n{error}"))
                            }
                        }
                    }
                }
            }
            FIRST_EDGE..=4 => UI.with(|slot| {
                if let Some(ui) = slot.borrow_mut().as_mut() {
                    ui.settings.edge = Edge::ALL[command - FIRST_EDGE];
                    ui.save();
                    ui.layout();
                }
            }),
            _ => {}
        }
    }
}

fn open_style(path: &Path) -> io::Result<()> {
    use std::io::Write;
    fs::create_dir_all(path.parent().unwrap())?;
    match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
    {
        Ok(mut file) => file.write_all(style::TEMPLATE.as_bytes())?,
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error),
    }
    let notepad =
        PathBuf::from(std::env::var_os("SystemRoot").unwrap_or_else(|| r"C:\Windows".into()))
            .join("System32")
            .join("notepad.exe");
    Command::new(notepad).arg(path).spawn()?;
    Ok(())
}

fn read_style(path: &Path) -> io::Result<Style> {
    match fs::read_to_string(path) {
        Ok(source) => {
            Style::parse(&source).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Style::default()),
        Err(error) => Err(error),
    }
}

fn work_area() -> RECT {
    unsafe {
        let mut work = zeroed();
        if SystemParametersInfoW(SPI_GETWORKAREA, 0, (&mut work as *mut RECT).cast(), 0) == 0 {
            work = rect(
                0,
                0,
                GetSystemMetrics(SM_CXSCREEN),
                GetSystemMetrics(SM_CYSCREEN),
            );
        }
        work
    }
}

fn place(window: HWND, view: &mut View, bounds: RECT, radius: i32) {
    view.geometry = Some((
        bounds.left,
        bounds.top,
        bounds.right - bounds.left,
        bounds.bottom - bounds.top,
        radius,
    ));
    request_repaint(window, view);
}

fn request_repaint(window: HWND, view: &mut View) {
    if !view.repaint_pending {
        view.repaint_pending = unsafe { PostMessageW(window, PAINT_FRAME, 0, 0) } != 0;
    }
}

fn rect(x: i32, y: i32, width: i32, height: i32) -> RECT {
    RECT {
        left: x,
        top: y,
        right: x + width,
        bottom: y + height,
    }
}

// GDI draws into our DIBs; read their pixels only after flushing pending draws.
struct Surface {
    dc: HDC,
    bitmap: HBITMAP,
    previous: HGDIOBJ,
    bits: *mut Pixel,
    width: usize,
    height: usize,
}

impl Surface {
    fn new(width: i32, height: i32) -> Option<Self> {
        unsafe {
            let mut info: BITMAPINFO = zeroed();
            info.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
            info.bmiHeader.biWidth = width;
            info.bmiHeader.biHeight = -height;
            info.bmiHeader.biPlanes = 1;
            info.bmiHeader.biBitCount = 32;
            info.bmiHeader.biCompression = BI_RGB;
            let mut bits = null_mut();
            let dc = CreateCompatibleDC(null_mut());
            if dc.is_null() {
                return None;
            }
            let bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
            if bitmap.is_null() {
                DeleteDC(dc);
                return None;
            }
            let previous = SelectObject(dc, bitmap);
            if previous.is_null() || bits.is_null() {
                DeleteDC(dc);
                DeleteObject(bitmap);
                return None;
            }
            std::ptr::write_bytes(bits.cast::<Pixel>(), 0, width as usize * height as usize);
            Some(Self {
                dc,
                bitmap,
                previous,
                bits: bits.cast(),
                width: width as usize,
                height: height as usize,
            })
        }
    }

    fn pixels(&mut self) -> &mut [Pixel] {
        unsafe {
            GdiFlush();
            std::slice::from_raw_parts_mut(self.bits, self.width * self.height)
        }
    }
}

impl Drop for Surface {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.previous);
            DeleteObject(self.bitmap);
            DeleteDC(self.dc);
        }
    }
}

fn paint(hwnd: HWND, view: &View, tray: &Tray, style: Style) {
    let Some((left, top, width, height, radius)) = view.geometry else {
        return;
    };
    let Some(mut surface) = Surface::new(width, height) else {
        return;
    };

    let dpi = unsafe { GetDpiForWindow(hwnd) }.max(96) as i32;
    let scaled = |value| value * dpi / 96;
    let area = rect(0, 0, width, height);
    fill(surface.dc, &area, style.background);
    rounded(surface.dc, area, radius, style.background, style.border);

    let mut labels = Vec::new();
    for slot in &view.slots {
        if view.hover == Some(slot.target) {
            let mut bounds = slot.bounds;
            unsafe { InflateRect(&mut bounds, -scaled(1), -scaled(1)) };
            rounded(
                surface.dc,
                bounds,
                scaled(10).min(radius),
                style.hover,
                style.hover,
            );
        }
        let label = match slot.target {
            usize::MAX if view.slots.len() == 1 => "+",
            NEXT => "›",
            PREVIOUS => "‹",
            _ => continue,
        };
        labels.push((label, slot.bounds, style.text));
    }

    if !view.status.is_empty() {
        let footer_height = scaled((style.font_size + 10).max(28));
        let side = footer_height + scaled(style.padding);
        let footer = rect(
            side,
            height - side,
            (width - 2 * side).max(1),
            footer_height,
        );
        labels.push((&view.status, footer, style.muted_text));
    }

    // Apply panel opacity before compositing labels and tray icons above it.
    let alpha = composite::panel_alpha(style.opacity);
    let stride = width as usize;
    let rows = height as usize;
    let pixels = surface.pixels();
    for pixel in pixels.iter_mut() {
        composite::premultiply(pixel, alpha);
    }
    for slot in &view.slots {
        if let Some(item) = tray.items.get(slot.target) {
            draw_icon(
                pixels,
                stride,
                rows,
                item,
                slot.bounds,
                scaled(style.icon_size) as usize,
            );
        }
    }
    if !labels.is_empty() {
        draw_labels(pixels, width, height, scaled(style.font_size), &labels);
    }

    composite::clip_corners(pixels, stride, rows, (radius / 2) as usize);

    let size = SIZE {
        cx: width,
        cy: height,
    };
    let position = POINT { x: left, y: top };
    let origin = POINT { x: 0, y: 0 };
    let blend = BLENDFUNCTION {
        BlendOp: AC_SRC_OVER as u8,
        BlendFlags: 0,
        SourceConstantAlpha: 255,
        AlphaFormat: AC_SRC_ALPHA as u8,
    };
    unsafe {
        UpdateLayeredWindow(
            hwnd,
            null_mut(),
            &position,
            &size,
            surface.dc,
            &origin,
            0,
            &blend,
            ULW_ALPHA,
        );
    }
}

fn draw_labels(
    pixels: &mut [Pixel],
    width: i32,
    height: i32,
    font_size: i32,
    labels: &[(&str, RECT, u32)],
) {
    let Some(mut mask) = Surface::new(width, height) else {
        return;
    };
    fill(mask.dc, &rect(0, 0, width, height), 0);
    unsafe {
        // A grayscale text mask yields clean edges over any desktop color.
        SetBkMode(mask.dc, TRANSPARENT as i32);
        let font = CreateFontW(
            -font_size,
            0,
            0,
            0,
            FW_NORMAL as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET as u32,
            0,
            0,
            ANTIALIASED_QUALITY as u32,
            0,
            wide("Segoe UI").as_ptr(),
        );
        let previous = if font.is_null() {
            null_mut()
        } else {
            SelectObject(mask.dc, font)
        };
        for &(text, bounds, _) in labels {
            draw_text(mask.dc, text, bounds, 0xffffff);
        }
        if !previous.is_null() {
            SelectObject(mask.dc, previous);
        }
        if !font.is_null() {
            DeleteObject(font);
        }
    }
    let mask = mask.pixels();
    for &(_, bounds, color) in labels {
        for y in bounds.top.max(0)..bounds.bottom.min(height) {
            for x in bounds.left.max(0)..bounds.right.min(width) {
                let index = (y * width + x) as usize;
                let coverage = mask[index][0];
                if coverage > 0 {
                    let mut pixel = [(color >> 16) as u8, (color >> 8) as u8, color as u8, 0];
                    composite::premultiply(&mut pixel, coverage);
                    composite::over(&mut pixels[index], pixel);
                }
            }
        }
    }
}

fn fill(dc: HDC, area: &RECT, color: u32) {
    unsafe {
        let brush = CreateSolidBrush(color);
        FillRect(dc, area, brush);
        DeleteObject(brush);
    }
}

fn rounded(dc: HDC, area: RECT, radius: i32, color: u32, border: u32) {
    unsafe {
        let pen = CreatePen(PS_SOLID, 1, border);
        let brush = CreateSolidBrush(color);
        let old_pen = SelectObject(dc, pen);
        let old_brush = SelectObject(dc, brush);
        let radius = radius
            .min(area.right - area.left)
            .min(area.bottom - area.top);
        if radius > 0 {
            RoundRect(
                dc,
                area.left,
                area.top,
                area.right,
                area.bottom,
                radius,
                radius,
            );
        } else {
            Rectangle(dc, area.left, area.top, area.right, area.bottom);
        }
        SelectObject(dc, old_brush);
        SelectObject(dc, old_pen);
        DeleteObject(brush);
        DeleteObject(pen);
    }
}

fn draw_text(dc: HDC, text: &str, mut area: RECT, color: u32) {
    let text = wide(text);
    unsafe {
        SetTextColor(dc, color);
        DrawTextW(
            dc,
            text.as_ptr(),
            (text.len() - 1) as i32,
            &mut area,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS | DT_NOPREFIX,
        );
    }
}

fn draw_icon(
    pixels: &mut [Pixel],
    width: usize,
    height: usize,
    item: &Item,
    area: RECT,
    size: usize,
) {
    let Some(source) = &item.image else { return };
    let mut cached = item.rendered_icon.borrow_mut();
    if cached.as_ref().map(|(size, _)| *size) != Some(size) {
        *cached = Some((size, source.fit(size)));
    }
    let Some((_, icon)) = cached.as_ref() else {
        return;
    };
    let left = (area.left + area.right - icon.width as i32) / 2;
    let top = (area.top + area.bottom - icon.height as i32) / 2;
    for (y, row) in icon.bgra.chunks_exact(icon.width * 4).enumerate() {
        let dest_y = top + y as i32;
        if !(0..height as i32).contains(&dest_y) {
            continue;
        }
        for (x, source) in row.chunks_exact(4).enumerate() {
            let dest_x = left + x as i32;
            if (0..width as i32).contains(&dest_x) {
                let index = dest_y as usize * width + dest_x as usize;
                composite::over(
                    &mut pixels[index],
                    [source[0], source[1], source[2], source[3]],
                );
            }
        }
    }
}

fn exe_for_window(window: u32) -> Option<String> {
    unsafe {
        let mut pid = 0;
        GetWindowThreadProcessId(window as usize as HWND, &mut pid);
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            return None;
        }
        let mut name = vec![0u16; 32_768];
        let mut len = name.len() as u32;
        let result = QueryFullProcessImageNameW(process, 0, name.as_mut_ptr(), &mut len);
        CloseHandle(process);
        (result != 0).then(|| String::from_utf16_lossy(&name[..len as usize]).to_lowercase())
    }
}

fn send_click(item: &Item, button: u32) {
    if item.callback < WM_USER {
        return;
    }
    unsafe {
        let window = item.window as usize as HWND;
        if IsWindow(window) == 0 {
            return;
        }
        let mut pid = 0;
        GetWindowThreadProcessId(window, &mut pid);
        AllowSetForegroundWindow(pid);
        let mut point: POINT = zeroed();
        GetCursorPos(&mut point);
        let pair = |low, high| (low as u16 as usize) | ((high as u16 as usize) << 16);
        let (down, semantic) = match button {
            WM_LBUTTONUP => (WM_LBUTTONDOWN, Some(WM_USER)),
            WM_RBUTTONUP => (WM_RBUTTONDOWN, Some(WM_CONTEXTMENU)),
            WM_MBUTTONUP => (WM_MBUTTONDOWN, None),
            _ => (0, None),
        };
        let messages = [
            Some(down).filter(|&id| id != 0),
            Some(button),
            semantic.filter(|_| item.version >= 3),
        ];
        for message in messages.into_iter().flatten() {
            if item.version >= 4 {
                SendNotifyMessageW(
                    window,
                    item.callback,
                    pair(point.x, point.y),
                    pair(message as i32, item.id as i32) as LPARAM,
                );
            } else {
                SendNotifyMessageW(window, item.callback, item.id as WPARAM, message as LPARAM);
            }
        }
    }
}
