use crate::{
    hook::{self, Event, Inbox, Instance, Monitor, wide},
    model::{Edge, Item, Settings, Tray},
    protocol::IconImage,
};
use std::{
    cell::RefCell,
    fs, io,
    mem::{size_of, zeroed},
    path::PathBuf,
    ptr::{null, null_mut},
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
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

const BACK: u32 = rgb(23, 26, 33);
const BORDER: u32 = rgb(51, 58, 70);
const HOVER: u32 = rgb(43, 50, 64);
const TEXT: u32 = rgb(220, 230, 243);
const SUBTLE: u32 = rgb(163, 177, 197);
const OVERFLOW_COLUMNS: usize = 6;
const WM_MOUSELEAVE: u32 = 0x02A3;
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
                WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
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
            WM_PAINT => paint(
                hwnd,
                if is_popup {
                    &ui.popup_view
                } else {
                    &ui.dock_view
                },
                &ui.tray,
            ),
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
            unsafe { InvalidateRect(hwnd, null(), 0) };
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
        unsafe { InvalidateRect(hwnd, null(), 0) };
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
        let cell = scaled(34);
        let strip = scaled(11);
        let pad = scaled(3);
        let gap = scaled(8);
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
            scaled(18),
        );
        if !self.popup_open {
            unsafe { ShowWindow(self.popup, SW_HIDE) };
            return;
        }

        let columns =
            OVERFLOW_COLUMNS.min(((work.right - work.left - 4 * gap) / cell).max(1) as usize);
        let rows = (((work.bottom - work.top) / 3 / cell).max(1) as usize).min(6);
        let page_size = columns * rows;
        let pages = overflow.len().div_ceil(page_size).max(1);
        self.page = self.page.min(pages - 1);
        self.popup_view.slots.clear();
        let page = &overflow[(self.page * page_size).min(overflow.len())
            ..((self.page + 1) * page_size).min(overflow.len())];
        let footer = scaled(28);
        let pop_width = columns as i32 * cell + 2 * pad;
        let row_count = page.len().div_ceil(columns).max(1) as i32;
        let pop_height = row_count * cell + footer + 2 * pad;
        for (offset, &index) in page.iter().enumerate() {
            let left = (offset % columns) as i32 * cell + pad;
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
            scaled(18),
        );
        unsafe { ShowWindow(self.popup, SW_SHOWNOACTIVATE) };
    }
}

fn show_menu(dock: HWND, edge: Edge) {
    const FIRST_EDGE: usize = 1;
    const REFRESH: usize = 10;
    const EXIT: usize = 11;
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
    let width = bounds.right - bounds.left;
    let height = bounds.bottom - bounds.top;
    let geometry = (bounds.left, bounds.top, width, height, radius);
    unsafe {
        if view.geometry == Some(geometry) {
            InvalidateRect(window, null(), 0);
            return;
        }
        view.geometry = Some(geometry);
        let region = CreateRoundRectRgn(0, 0, width + 1, height + 1, radius, radius);
        if !region.is_null() && SetWindowRgn(window, region, 1) == 0 {
            DeleteObject(region);
        }
        SetWindowPos(
            window,
            HWND_TOPMOST,
            bounds.left,
            bounds.top,
            width,
            height,
            SWP_NOACTIVATE,
        );
        InvalidateRect(window, null(), 0);
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

const fn rgb(red: u32, green: u32, blue: u32) -> u32 {
    red | green << 8 | blue << 16
}

fn paint(hwnd: HWND, view: &View, tray: &Tray) {
    unsafe {
        let mut paint: PAINTSTRUCT = zeroed();
        let screen = BeginPaint(hwnd, &mut paint);
        if screen.is_null() {
            return;
        }
        let mut area: RECT = zeroed();
        GetClientRect(hwnd, &mut area);
        let width = area.right;
        let height = area.bottom;
        let dpi = GetDpiForWindow(hwnd).max(96) as i32;
        let scaled = |value| value * dpi / 96;
        let buffer = CreateCompatibleDC(screen);
        let bitmap = CreateCompatibleBitmap(screen, width, height);
        let buffered = !buffer.is_null() && !bitmap.is_null();
        let previous = if buffered {
            SelectObject(buffer, bitmap)
        } else {
            null_mut()
        };
        let dc = if buffered { buffer } else { screen };
        fill(dc, &area, BACK);
        rounded(dc, area, scaled(18), BACK, BORDER);
        SetBkMode(dc, TRANSPARENT as i32);
        let font = CreateFontW(
            -scaled(13),
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
            CLEARTYPE_QUALITY as u32,
            0,
            wide("Segoe UI").as_ptr(),
        );
        let old_font = if font.is_null() {
            null_mut()
        } else {
            SelectObject(dc, font)
        };

        for slot in &view.slots {
            let hover = view.hover == Some(slot.target);
            if hover {
                let mut bounds = slot.bounds;
                InflateRect(&mut bounds, -scaled(1), -scaled(1));
                rounded(dc, bounds, scaled(10), HOVER, HOVER);
            }
            if let Some(image) = tray
                .items
                .get(slot.target)
                .and_then(|item| item.image.as_ref())
            {
                let size = scaled(20);
                let left = (slot.bounds.left + slot.bounds.right - size) / 2;
                let top = (slot.bounds.top + slot.bounds.bottom - size) / 2;
                draw_icon(dc, image, left, top, size, if hover { HOVER } else { BACK });
            } else if slot.target == usize::MAX && view.slots.len() == 1 {
                draw_text(dc, "+", slot.bounds, TEXT);
            } else if slot.target == NEXT || slot.target == PREVIOUS {
                draw_text(
                    dc,
                    if slot.target == NEXT { "›" } else { "‹" },
                    slot.bounds,
                    TEXT,
                );
            }
        }
        if !view.status.is_empty() {
            let footer = rect(
                scaled(31),
                height - scaled(31),
                (width - scaled(62)).max(1),
                scaled(28),
            );
            draw_text(dc, &view.status, footer, SUBTLE);
        }
        if !old_font.is_null() {
            SelectObject(dc, old_font);
        }
        if !font.is_null() {
            DeleteObject(font);
        }
        if buffered {
            BitBlt(screen, 0, 0, width, height, dc, 0, 0, SRCCOPY);
            SelectObject(buffer, previous);
        }
        if !bitmap.is_null() {
            DeleteObject(bitmap);
        }
        if !buffer.is_null() {
            DeleteDC(buffer);
        }
        EndPaint(hwnd, &paint);
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
        RoundRect(
            dc,
            area.left,
            area.top,
            area.right,
            area.bottom,
            radius,
            radius,
        );
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

fn draw_icon(dc: HDC, image: &IconImage, x: i32, y: i32, size: i32, background: u32) {
    let mut pixels = Vec::with_capacity(image.rgba.len());
    for pixel in image.rgba.chunks_exact(4) {
        let alpha = pixel[3] as u32;
        let blend = |component: u8, shift: u32| {
            ((component as u32 * alpha + ((background >> shift) & 255u32) * (255 - alpha) + 127)
                / 255) as u8
        };
        pixels.extend_from_slice(&[
            blend(pixel[2], 16),
            blend(pixel[1], 8),
            blend(pixel[0], 0),
            0,
        ]);
    }
    unsafe {
        let mut bitmap: BITMAPINFO = zeroed();
        bitmap.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
        bitmap.bmiHeader.biWidth = image.width as i32;
        bitmap.bmiHeader.biHeight = -(image.height as i32);
        bitmap.bmiHeader.biPlanes = 1;
        bitmap.bmiHeader.biBitCount = 32;
        bitmap.bmiHeader.biCompression = BI_RGB;
        SetStretchBltMode(dc, HALFTONE);
        SetBrushOrgEx(dc, 0, 0, null_mut());
        StretchDIBits(
            dc,
            x,
            y,
            size,
            size,
            0,
            0,
            image.width as i32,
            image.height as i32,
            pixels.as_ptr().cast(),
            &bitmap,
            DIB_RGB_COLORS,
            SRCCOPY,
        );
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
