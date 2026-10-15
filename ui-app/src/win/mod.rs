//! Windows host: a borderless, per-pixel transparent Win32 window (D3D11 +
//! DirectComposition) that draws the kit's shell and widgets.
//!
//! The shell is the whole window: its title bar moves the window (double-click
//! maximises), its corner grip and a thin band around its edge resize it, and the close
//! button quits. Moving and resizing are left to Windows through `WM_NCHITTEST`, so snap
//! layouts and the system menu work as for any other window.

mod gpu;

use core::ffi::c_void;
use core::ptr::{null, null_mut};
use std::cell::RefCell;
use std::time::Instant;

use poly_ui::input::Collector;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Dwm::{
    DWMNCRP_DISABLED, DWMWA_BORDER_COLOR, DWMWA_COLOR_NONE, DWMWA_NCRENDERING_POLICY,
    DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND, DwmSetWindowAttribute,
};
use windows_sys::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow, ScreenToClient,
    ValidateRect,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForWindow, SetProcessDpiAwarenessContext,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    ReleaseCapture, SetCapture, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent,
};
use windows_sys::Win32::UI::WindowsAndMessaging::*;
use windows_sys::core::w;

use crate::hit::{Hit, hit_test};
use crate::page::Page;
use gpu::Gpu;

/// Missing in `windows-sys`.
const WM_MOUSELEAVE: u32 = 0x02A3;
/// Button flags in the wParam of mouse messages (`Win32_System_SystemServices`).
const MK_LBUTTON: usize = 0x1;
const MK_RBUTTON: usize = 0x2;
/// Redraw timer while Windows runs its modal move/resize loop.
const MODAL_TIMER: usize = 1;

struct App {
    hwnd: HWND,
    gpu: Gpu,
    page: Page,
    input: Collector,
    last: Option<Instant>,
    /// Client size in pixels.
    size: [u32; 2],
    minimized: bool,
    tracking: bool,
    error: Option<String>,
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

/// Runs `f` on the app unless it does not exist yet or is busy (a nested message while a
/// handler holds it, e.g. `WM_SIZE` from inside `SetWindowPos`).
fn with_app<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|a| a.try_borrow_mut().ok()?.as_mut().map(f))
}

fn maximized(hwnd: HWND) -> bool {
    // SAFETY: Win32 validates the handle.
    unsafe { IsZoomed(hwnd) != 0 }
}

impl App {
    fn scale(&self) -> f32 {
        self.page.kit.theme.scale
    }

    /// Transparent shadow margin (none while maximised).
    fn margin(&self) -> f32 {
        if maximized(self.hwnd) {
            0.0
        } else {
            self.page.shadow_margin()
        }
    }

    fn render(&mut self) {
        if self.minimized || self.error.is_some() || self.size[0] == 0 || self.size[1] == 0 {
            return;
        }
        let now = Instant::now();
        let dt = self.last.map_or(0.0, |l| (now - l).as_secs_f32());
        self.last = Some(now);
        let mut input = self.input.take();
        input.dt = dt;
        let margin = self.margin();
        let result = self.gpu.resize(self.size).and_then(|()| {
            let (backdrop, list, textures, close) = self.page.frame(input, self.size, margin);
            self.gpu.frame(&textures, backdrop, list).map(|()| close)
        });
        match result {
            // SAFETY: Posting to our own live window; the message carries no pointers.
            Ok(true) => unsafe {
                PostMessageW(self.hwnd, WM_CLOSE, 0, 0);
            },
            Ok(false) => {}
            Err(e) => {
                self.error = Some(e);
                // SAFETY: No arguments besides the exit code.
                unsafe { PostQuitMessage(1) };
            }
        }
    }

    /// `WM_NCHITTEST` for a point in client pixels.
    fn hit_test(&self, p: [f32; 2]) -> u32 {
        let shell = &self.page.shell;
        let size = [self.size[0] as f32, self.size[1] as f32];
        let areas = [shell.window(), shell.caption(), shell.grip()];
        if maximized(self.hwnd) {
            return if areas[1].contains(p) {
                HTCAPTION
            } else {
                HTCLIENT
            };
        }
        match hit_test(p, size, self.margin(), self.scale(), areas) {
            Hit::Nowhere => HTNOWHERE,
            Hit::Client => HTCLIENT,
            Hit::Caption => HTCAPTION,
            Hit::Left => HTLEFT,
            Hit::Right => HTRIGHT,
            Hit::Top => HTTOP,
            Hit::Bottom => HTBOTTOM,
            Hit::TopLeft => HTTOPLEFT,
            Hit::TopRight => HTTOPRIGHT,
            Hit::BottomLeft => HTBOTTOMLEFT,
            Hit::BottomRight => HTBOTTOMRIGHT,
        }
    }

    fn mouse(&mut self, msg: u32, wparam: WPARAM, lparam: LPARAM) {
        if !self.tracking {
            let mut track = TRACKMOUSEEVENT {
                cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                dwFlags: TME_LEAVE,
                hwndTrack: self.hwnd,
                dwHoverTime: 0,
            };
            // SAFETY: The struct is initialised and lives for the call.
            self.tracking = unsafe { TrackMouseEvent(&mut track) } != 0;
        }
        let (x, y) = (
            loword(lparam as usize) as i16,
            hiword(lparam as usize) as i16,
        );
        let pixel = (x.max(0) as u32, y.max(0) as u32);
        self.input.feed(msg, wparam, Some(pixel));
    }
}

fn loword(v: usize) -> u16 {
    (v & 0xffff) as u16
}
fn hiword(v: usize) -> u16 {
    ((v >> 16) & 0xffff) as u16
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        // The client area is the whole window; maximised it is the monitor's work area
        // (Windows places a maximised window partly off-screen by its frame size).
        WM_NCCALCSIZE if wparam != 0 => {
            if maximized(hwnd) {
                // SAFETY: With wParam TRUE, lParam points to a live NCCALCSIZE_PARAMS.
                let params = unsafe { &mut *(lparam as *mut NCCALCSIZE_PARAMS) };
                let mut info = MONITORINFO {
                    cbSize: size_of::<MONITORINFO>() as u32,
                    ..Default::default()
                };
                // SAFETY: The window is live and `info` is writable with its size set.
                if unsafe {
                    GetMonitorInfoW(MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST), &mut info)
                } != 0
                {
                    params.rgrc[0] = info.rcWork;
                }
            }
            return 0;
        }
        WM_NCHITTEST => {
            let mut p = POINT {
                x: loword(lparam as usize) as i16 as i32,
                y: hiword(lparam as usize) as i16 as i32,
            };
            // SAFETY: The window is live and `p` is writable.
            unsafe { ScreenToClient(hwnd, &mut p) };
            if let Some(hit) = with_app(|a| a.hit_test([p.x as f32, p.y as f32])) {
                return hit as LRESULT;
            }
        }
        // Keep Windows from drawing a classic frame on activation changes.
        // SAFETY: Unchanged callback parameters, except lParam -1 (no repaint) as documented.
        WM_NCACTIVATE => return unsafe { DefWindowProcW(hwnd, msg, wparam, -1) },
        WM_GETMINMAXINFO => {
            if let Some(min) = with_app(|a| a.page.min_size()) {
                // SAFETY: lParam points to a live MINMAXINFO for this message.
                let info = unsafe { &mut *(lparam as *mut MINMAXINFO) };
                info.ptMinTrackSize = POINT {
                    x: min[0].ceil() as i32,
                    y: min[1].ceil() as i32,
                };
            }
            return 0;
        }
        WM_SIZE => {
            let size = [
                u32::from(loword(lparam as usize)),
                u32::from(hiword(lparam as usize)),
            ];
            with_app(|a| {
                a.size = size;
                a.minimized = wparam == SIZE_MINIMIZED as usize;
                a.render();
            });
            return 0;
        }
        WM_DPICHANGED => {
            let dpi = f32::from(loword(wparam));
            // SAFETY: lParam points to the suggested window rectangle for this message.
            let r = unsafe { *(lparam as *const RECT) };
            let failed = with_app(|a| a.page.kit.set_scale(dpi / 96.0).err()).flatten();
            if let Some(e) = failed {
                with_app(|a| a.error = Some(e));
                // SAFETY: No arguments besides the exit code.
                unsafe { PostQuitMessage(1) };
            }
            // SAFETY: Our own live window; the rectangle comes from Windows.
            unsafe {
                SetWindowPos(
                    hwnd,
                    null_mut(),
                    r.left,
                    r.top,
                    r.right - r.left,
                    r.bottom - r.top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
            }
            return 0;
        }
        WM_ENTERSIZEMOVE => {
            // SAFETY: Our own live window; no callback, the timer posts WM_TIMER.
            unsafe { SetTimer(hwnd, MODAL_TIMER, 15, None) };
            return 0;
        }
        WM_EXITSIZEMOVE => {
            // SAFETY: Our own live window and timer id.
            unsafe { KillTimer(hwnd, MODAL_TIMER) };
            return 0;
        }
        WM_TIMER if wparam == MODAL_TIMER => {
            with_app(App::render);
            return 0;
        }
        WM_PAINT => {
            // SAFETY: Our own live window; null validates the whole client area.
            unsafe { ValidateRect(hwnd, null()) };
            with_app(App::render);
            return 0;
        }
        WM_ERASEBKGND => return 1,
        WM_MOUSEMOVE | WM_MBUTTONDOWN | WM_MBUTTONUP | WM_XBUTTONDOWN | WM_XBUTTONUP
        | WM_LBUTTONDBLCLK | WM_RBUTTONDBLCLK | WM_MBUTTONDBLCLK | WM_XBUTTONDBLCLK => {
            with_app(|a| a.mouse(msg, wparam, lparam));
            return 0;
        }
        WM_LBUTTONDOWN | WM_RBUTTONDOWN => {
            // Keep receiving the mouse while a slider or the colour field is dragged out.
            // SAFETY: Our own live window.
            unsafe { SetCapture(hwnd) };
            with_app(|a| a.mouse(msg, wparam, lparam));
            return 0;
        }
        WM_LBUTTONUP | WM_RBUTTONUP => {
            with_app(|a| a.mouse(msg, wparam, lparam));
            if wparam & (MK_LBUTTON | MK_RBUTTON) == 0 {
                // SAFETY: No arguments.
                unsafe { ReleaseCapture() };
            }
            return 0;
        }
        WM_MOUSELEAVE => {
            with_app(|a| {
                a.tracking = false;
                a.input.leave();
            });
            return 0;
        }
        WM_MOUSEWHEEL | WM_KEYDOWN | WM_KEYUP | WM_CHAR => {
            with_app(|a| a.input.feed(msg, wparam, None));
            return 0;
        }
        // Alt for the kit, then Windows (Alt+F4, Alt+Space).
        WM_SYSKEYDOWN | WM_SYSKEYUP => {
            with_app(|a| a.input.feed(msg, wparam, None));
        }
        // Held modifiers and buttons do not survive Alt+Tab.
        WM_KILLFOCUS => {
            with_app(|a| {
                a.input = Collector::default();
                a.input.leave();
            });
        }
        WM_CLOSE => {
            // SAFETY: Our own live window.
            unsafe { DestroyWindow(hwnd) };
            return 0;
        }
        WM_DESTROY => {
            // SAFETY: No arguments besides the exit code.
            unsafe { PostQuitMessage(0) };
            return 0;
        }
        _ => {}
    }
    // SAFETY: Unchanged parameters of the active callback.
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

fn create_window() -> Result<HWND, String> {
    // SAFETY: Null asks for the executable's own module.
    let instance = unsafe { GetModuleHandleW(null()) };
    let class = w!("PolyUiWindow");
    let wc = WNDCLASSEXW {
        cbSize: size_of::<WNDCLASSEXW>() as u32,
        lpfnWndProc: Some(wndproc),
        hInstance: instance,
        // SAFETY: Stock cursor and icon, no module.
        hCursor: unsafe { LoadCursorW(null_mut(), IDC_ARROW) },
        // SAFETY: As above.
        hIcon: unsafe { LoadIconW(null_mut(), IDI_APPLICATION) },
        lpszClassName: class,
        ..Default::default()
    };
    // SAFETY: The class struct and its static name outlive the call.
    if unsafe { RegisterClassExW(&wc) } == 0 {
        return Err(format!(
            "RegisterClassExW: {}",
            std::io::Error::last_os_error()
        ));
    }
    // SAFETY: Registered class, static title, no parent, menu or creation data.
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_NOREDIRECTIONBITMAP | WS_EX_APPWINDOW,
            class,
            w!("poly"),
            WS_OVERLAPPEDWINDOW,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            null_mut(),
            null_mut(),
            instance,
            null(),
        )
    };
    if hwnd.is_null() {
        return Err(format!(
            "CreateWindowExW: {}",
            std::io::Error::last_os_error()
        ));
    }
    // No DWM frame, shadow, rounded corners or border: the shell draws its own.
    let attributes: [(i32, u32); 3] = [
        (DWMWA_NCRENDERING_POLICY, DWMNCRP_DISABLED as u32),
        (DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND as u32),
        (DWMWA_BORDER_COLOR, DWMWA_COLOR_NONE),
    ];
    for (attribute, value) in attributes {
        // SAFETY: The value is a live u32 of the given size; unsupported attributes on
        // older Windows versions just fail.
        unsafe {
            DwmSetWindowAttribute(
                hwnd,
                attribute as u32,
                (&raw const value).cast::<c_void>(),
                size_of::<u32>() as u32,
            )
        };
    }
    Ok(hwnd)
}

/// Centres a window of `size` pixels on the work area of the window's monitor.
fn place(hwnd: HWND, size: [f32; 2]) {
    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    // SAFETY: The window is live and `info` is writable with its size set.
    unsafe { GetMonitorInfoW(MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST), &mut info) };
    let work = info.rcWork;
    let w = (size[0].ceil() as i32).min(work.right - work.left);
    let h = (size[1].ceil() as i32).min(work.bottom - work.top);
    let x = work.left + (work.right - work.left - w) / 2;
    let y = work.top + (work.bottom - work.top - h) / 2;
    // SAFETY: Our own live window.
    unsafe { SetWindowPos(hwnd, null_mut(), x, y, w, h, SWP_NOZORDER | SWP_NOACTIVATE) };
}

pub fn run() -> Result<(), String> {
    // SAFETY: Process-wide setting before any window exists; failure (already set by a
    // manifest) is harmless.
    unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
    let hwnd = create_window()?;
    // SAFETY: The window is live.
    let scale = unsafe { GetDpiForWindow(hwnd) }.max(96) as f32 / 96.0;
    let page = Page::new(scale)?;
    let margin = 2.0 * page.shadow_margin();
    let initial = page.shell.state.size;
    place(
        hwnd,
        [initial[0] * scale + margin, initial[1] * scale + margin],
    );
    let mut client = RECT::default();
    // SAFETY: The window is live and `client` is writable.
    unsafe { GetClientRect(hwnd, &mut client) };
    let size = [
        (client.right - client.left).max(1) as u32,
        (client.bottom - client.top).max(1) as u32,
    ];
    let gpu = Gpu::new(windows::Win32::Foundation::HWND(hwnd), size)?;
    let mut input = Collector::default();
    input.leave();
    APP.with(|a| {
        *a.borrow_mut() = Some(App {
            hwnd,
            gpu,
            page,
            input,
            last: None,
            size,
            minimized: false,
            tracking: false,
            error: None,
        })
    });
    // SAFETY: Our own live window.
    unsafe { ShowWindow(hwnd, SW_SHOW) };

    let mut msg = MSG::default();
    'run: loop {
        // SAFETY: `msg` is writable; messages of every window of this thread.
        while unsafe { PeekMessageW(&mut msg, null_mut(), 0, 0, PM_REMOVE) } != 0 {
            if msg.message == WM_QUIT {
                break 'run;
            }
            // SAFETY: `msg` was just filled by PeekMessageW.
            unsafe {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        // Present(1) paces the loop at the display's refresh rate.
        match with_app(|a| (!a.minimized).then(|| a.render())) {
            Some(Some(())) => {}
            // SAFETY: No arguments; returns when a message arrives.
            _ => unsafe {
                WaitMessage();
            },
        }
    }
    match APP.with(|a| a.borrow_mut().take()).and_then(|a| a.error) {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

/// Shows a fatal error (the release build has no console).
pub fn error_box(e: &str) {
    let text: Vec<u16> = e.encode_utf16().chain([0]).collect();
    // SAFETY: NUL-terminated text and a static caption; no owner window.
    unsafe { MessageBoxW(null_mut(), text.as_ptr(), w!("poly"), MB_ICONERROR | MB_OK) };
}
