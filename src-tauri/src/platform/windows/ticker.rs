//! The ticker in the taskbar: the pinned entry, drawn the way the macOS menu
//! bar shows it, just left of the notification area. It is a child window of
//! the taskbar (`Shell_TrayWnd`), so it hides, moves and auto-hides with it,
//! and a layered one with per-pixel alpha, so it sits on the taskbar's own
//! material. Windows has no supported way to extend the taskbar: this
//! follows the taskbar's window layout, re-checked every couple of seconds,
//! and when that layout isn't there the tray icon alone carries the price.

use std::{
    cell::Cell,
    time::{Duration, Instant},
};

use windows::{
    Win32::{
        Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM},
        Graphics::Gdi::{AC_SRC_ALPHA, AC_SRC_OVER, BLENDFUNCTION, MapWindowPoints},
        UI::{
            HiDpi::GetDpiForWindow,
            Input::KeyboardAndMouse::{TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent},
            WindowsAndMessaging::{
                CreateWindowExW, DefWindowProcW, DestroyWindow, FindWindowExW, FindWindowW,
                GW_CHILD, GetClientRect, GetParent, GetWindow, GetWindowRect, HWND_TOP, IDC_ARROW,
                IsWindow, IsWindowVisible, LoadCursorW, MA_NOACTIVATE, RegisterClassExW, SW_HIDE,
                SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SWP_SHOWWINDOW, SetWindowPos,
                ShowWindow, ULW_ALPHA, UpdateLayeredWindow, WM_LBUTTONDOWN, WM_LBUTTONUP,
                WM_MOUSEACTIVATE, WM_MOUSEMOVE, WM_RBUTTONUP, WNDCLASSEXW, WS_CHILD,
                WS_CLIPSIBLINGS, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_NOPARENTNOTIFY,
                WS_EX_TOOLWINDOW,
            },
        },
    },
    core::{PCWSTR, w},
};

use super::{
    draw::{Frame, Painter, Plate, Surface},
    shell,
    theme::Look,
};
use crate::{bar::Ticker, model::ColorScheme};

const CLASS: PCWSTR = w!("Candlewick.Ticker");
/// `WM_MOUSELEAVE`, from the Controls headers.
const WM_MOUSELEAVE: u32 = 0x02A3;
/// The widest the Widgets button gets beside the tray (with the weather).
const WIDGETS_WIDTH: f32 = 160.0;
/// Where the tray starts when the taskbar doesn't say: its usual width with a
/// few icons and the clock.
const TRAY_WIDTH: f32 = 280.0;
/// After the taskbar refused the ticker window, how long before trying again.
const RETRY_ATTACH: Duration = Duration::from_secs(60);

/// What the ticker window reports to the shell thread (`shell::WM_TICKER`).
pub mod input {
    pub const ENTER: usize = 1;
    pub const LEAVE: usize = 2;
    pub const PRESS: usize = 3;
    pub const CLICK: usize = 4;
}

thread_local! {
    /// Whether a leave notification is armed for the pointer over the ticker.
    static TRACKING: Cell<bool> = const { Cell::new(false) };
}

pub fn register(instance: HINSTANCE) -> windows::core::Result<()> {
    let class = WNDCLASSEXW {
        cbSize: size_of::<WNDCLASSEXW>() as u32,
        lpfnWndProc: Some(procedure),
        hInstance: instance,
        // SAFETY: a stock system cursor.
        hCursor: unsafe { LoadCursorW(None, IDC_ARROW) }.unwrap_or_default(),
        lpszClassName: CLASS,
        ..Default::default()
    };
    // SAFETY: a class with a static procedure and name.
    if unsafe { RegisterClassExW(&class) } == 0 {
        return Err(windows::core::Error::from_win32());
    }
    Ok(())
}

pub struct TaskbarTicker {
    instance: HINSTANCE,
    window: Option<HWND>,
    taskbar: Option<HWND>,
    content: Option<(Ticker, ColorScheme)>,
    hovering: bool,
    pressed: bool,
    open: bool,
    drawn: Option<Frame>,
    placed: Option<RECT>,
    surface: Option<Surface>,
    /// Creating the window failed for this taskbar, at this time; not
    /// retried every tick.
    refused: Option<(HWND, Instant)>,
}

impl TaskbarTicker {
    pub fn new(instance: HINSTANCE) -> Self {
        Self {
            instance,
            window: None,
            taskbar: None,
            content: None,
            hovering: false,
            pressed: false,
            open: false,
            drawn: None,
            placed: None,
            surface: None,
            refused: None,
        }
    }

    /// What to show; `None` (nothing pinned) hides the ticker.
    pub fn set(&mut self, content: Option<(Ticker, ColorScheme)>) {
        self.content = content;
    }

    pub fn input(&mut self, event: usize) {
        match event {
            input::ENTER => self.hovering = true,
            input::LEAVE => {
                self.hovering = false;
                self.pressed = false;
            }
            input::PRESS => self.pressed = true,
            input::CLICK => self.pressed = false,
            _ => {}
        }
    }

    /// The dropdown opened from the ticker, or closed.
    pub fn set_open(&mut self, open: bool) {
        self.open = open;
    }

    /// The ticker's bounds on screen, while it shows.
    pub fn screen_rect(&self) -> Option<RECT> {
        let window = self.window.filter(|_| self.placed.is_some())?;
        let mut rect = RECT::default();
        // SAFETY: our own window.
        unsafe { GetWindowRect(window, &mut rect) }.ok()?;
        Some(rect)
    }

    /// Brings the ticker in line with its content, the taskbar's layout and
    /// `look`: attaches to the taskbar if needed, redraws when anything it
    /// shows changed, and moves when the tray did.
    pub fn refresh(&mut self, look: &Look, painter: Option<&Painter>) {
        let (Some((ticker, scheme)), Some(painter)) = (self.content.clone(), painter) else {
            self.hide();
            return;
        };
        let Some((window, taskbar)) = self.attach() else {
            return;
        };
        let mut client = RECT::default();
        // SAFETY: the taskbar window, alive as checked by `attach`.
        if unsafe { GetClientRect(taskbar, &mut client) }.is_err() || client.right <= client.bottom
        {
            // Gone, or docked to a side: a row of text has nowhere to go.
            self.hide();
            return;
        }
        // SAFETY: a plain query on a window handle.
        let dpi = match unsafe { GetDpiForWindow(taskbar) } {
            0 => 96,
            dpi => dpi,
        };
        let plate = match (self.open, self.pressed, self.hovering) {
            (true, _, _) => Plate::Open,
            (_, true, _) => Plate::Pressed,
            (_, _, true) => Plate::Hover,
            _ => Plate::None,
        };
        let palette = look.taskbar_palette();
        let frame = Frame { ticker, scheme, palette, dpi, height: client.bottom, plate };
        let redraw = self.drawn.as_ref() != Some(&frame) || self.surface.is_none();
        if redraw {
            if let Err(e) = painter.ticker(&frame, &mut self.surface) {
                log::warn!("cannot draw the ticker: {e}");
                self.drawn = None;
                self.hide();
                return;
            }
            self.drawn = Some(frame);
        }
        let Some((width, height, dc)) = self.surface.as_ref().map(|s| (s.width, s.height, s.dc))
        else {
            return;
        };
        let Some(bounds) = place(taskbar, width, client, dpi, look.widgets_beside_tray) else {
            self.hide();
            return;
        };
        // SAFETY: our own window; the surface's DC holds the drawn bitmap.
        unsafe {
            if redraw
                || self.placed.map(|p| (p.right - p.left, p.bottom - p.top))
                    != Some((width, height))
            {
                let blend = BLENDFUNCTION {
                    BlendOp: AC_SRC_OVER as u8,
                    BlendFlags: 0,
                    SourceConstantAlpha: 255,
                    AlphaFormat: AC_SRC_ALPHA as u8,
                };
                let size = SIZE { cx: width, cy: height };
                let origin = POINT::default();
                if let Err(e) = UpdateLayeredWindow(
                    window,
                    None,
                    None,
                    Some(&raw const size),
                    Some(dc),
                    Some(&raw const origin),
                    Default::default(),
                    Some(&raw const blend),
                    ULW_ALPHA,
                ) {
                    log::warn!("cannot show the ticker: {e}");
                }
            }
            let visible = IsWindowVisible(window).as_bool();
            if self.placed != Some(bounds) || !visible {
                let _ = SetWindowPos(
                    window,
                    None,
                    bounds.left,
                    bounds.top,
                    bounds.right - bounds.left,
                    bounds.bottom - bounds.top,
                    SWP_NOZORDER | SWP_NOACTIVATE | SWP_SHOWWINDOW,
                );
                self.placed = Some(bounds);
            }
            // The taskbar's content is a XAML island covering all of it; stay
            // above it. Checked first: moving for nothing makes Explorer
            // repaint that strip of the taskbar.
            if GetWindow(taskbar, GW_CHILD).ok() != Some(window) {
                let _ = SetWindowPos(
                    window,
                    Some(HWND_TOP),
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                );
            }
        }
    }

    /// The window, a child of the current taskbar; created or re-created as
    /// needed (Explorer restarts take their children with them).
    fn attach(&mut self) -> Option<(HWND, HWND)> {
        // SAFETY: plain window queries.
        unsafe {
            let taskbar = FindWindowW(w!("Shell_TrayWnd"), PCWSTR::null()).ok()?;
            if let (Some(window), Some(current)) = (self.window, self.taskbar)
                && current == taskbar
                && IsWindow(Some(window)).as_bool()
                && GetParent(window).ok() == Some(taskbar)
            {
                return Some((window, taskbar));
            }
            self.detach();
            if let Some((refused, at)) = self.refused
                && refused == taskbar
                && at.elapsed() < RETRY_ATTACH
            {
                return None;
            }
            match CreateWindowExW(
                // No parent notifications: the parent is Explorer's, and they
                // would be sent across processes on every click.
                WS_EX_LAYERED | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_NOPARENTNOTIFY,
                CLASS,
                w!("Candlewick"),
                WS_CHILD | WS_CLIPSIBLINGS,
                0,
                0,
                0,
                0,
                Some(taskbar),
                None,
                Some(self.instance),
                None,
            ) {
                Ok(window) => {
                    log::debug!("ticker attached to the taskbar");
                    self.window = Some(window);
                    self.taskbar = Some(taskbar);
                    Some((window, taskbar))
                }
                Err(e) => {
                    log::warn!("cannot attach the ticker to the taskbar: {e}");
                    self.refused = Some((taskbar, Instant::now()));
                    None
                }
            }
        }
    }

    pub fn hide(&mut self) {
        if let Some(window) = self.window
            && self.placed.take().is_some()
        {
            // SAFETY: our own window.
            unsafe {
                let _ = ShowWindow(window, SW_HIDE);
            }
        }
    }

    pub fn detach(&mut self) {
        if let Some(window) = self.window.take() {
            // SAFETY: our own window; gone already if the taskbar went away.
            unsafe {
                if IsWindow(Some(window)).as_bool() {
                    let _ = DestroyWindow(window);
                }
            }
        }
        self.taskbar = None;
        self.placed = None;
        self.drawn = None;
        TRACKING.set(false);
    }

    /// The taskbar was re-created (Explorer restarted): attach afresh.
    pub fn reset(&mut self) {
        self.detach();
        self.refused = None;
        self.hovering = false;
        self.pressed = false;
    }
}

/// Where the ticker goes, in the taskbar's client coordinates: its full
/// height, ending where the notification area starts (and before the Widgets
/// button, which Windows 11 puts there on a left-aligned taskbar).
fn place(taskbar: HWND, width: i32, client: RECT, dpi: u32, widgets: bool) -> Option<RECT> {
    let px = |dip: f32| (dip * dpi as f32 / 96.0).round() as i32;
    // SAFETY: plain window queries on the taskbar and its child.
    let tray = unsafe {
        FindWindowExW(Some(taskbar), None, w!("TrayNotifyWnd"), PCWSTR::null()).ok().and_then(
            |tray| {
                let mut rect = RECT::default();
                GetWindowRect(tray, &mut rect).ok()?;
                let mut corners =
                    [POINT { x: rect.left, y: rect.top }, POINT { x: rect.right, y: rect.bottom }];
                MapWindowPoints(None, Some(taskbar), &mut corners);
                Some(corners[0].x)
            },
        )
    };
    let tray_left = tray.filter(|&x| x > 0).unwrap_or(client.right - px(TRAY_WIDTH));
    let widgets = if widgets { px(WIDGETS_WIDTH) } else { 0 };
    let left = tray_left - widgets - width;
    (left > 0).then_some(RECT { left, top: 0, right: left + width, bottom: client.bottom })
}

/// Input only: everything else is Explorer's business or the default. Events
/// go to the shell thread as posted messages, so none is handled while the
/// shell is in the middle of something.
unsafe extern "system" fn procedure(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let report = |event: usize| shell::post(shell::WM_TICKER, WPARAM(event), LPARAM(0));
    match message {
        // Clicking the ticker must not take focus from the app in front.
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        WM_MOUSEMOVE => {
            if !TRACKING.get() {
                let mut track = TRACKMOUSEEVENT {
                    cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: window,
                    dwHoverTime: 0,
                };
                // SAFETY: tracks our own window.
                if unsafe { TrackMouseEvent(&mut track) }.is_ok() {
                    TRACKING.set(true);
                    report(input::ENTER);
                }
            }
            LRESULT(0)
        }
        WM_MOUSELEAVE => {
            TRACKING.set(false);
            report(input::LEAVE);
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            report(input::PRESS);
            LRESULT(0)
        }
        // Taskbar buttons act on release; either button opens the dropdown.
        WM_LBUTTONUP | WM_RBUTTONUP => {
            report(input::CLICK);
            LRESULT(0)
        }
        // SAFETY: the default procedure for our own window.
        _ => unsafe { DefWindowProcW(window, message, wparam, lparam) },
    }
}
