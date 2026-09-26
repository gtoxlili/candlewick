//! The shell thread. A hidden window owns the tray icon, the taskbar ticker
//! and the dropdown, and hears what Windows broadcasts: the taskbar coming
//! back after Explorer restarts, theme and display changes, sleep, display
//! power and session switches.
//!
//! It has a thread of its own because the ticker is a child of Explorer's
//! taskbar: the two threads' input queues are joined, so a busy thread here
//! would stall the taskbar. Nothing on it waits for the main thread; window
//! actions go there as tasks.
//!
//! State lives in a thread-local cell, borrowed only for short stretches.
//! Calls that run a message loop inside (the dropdown, above all) happen with
//! nothing borrowed, since the window procedure runs again within them.

use std::{
    cell::RefCell,
    io,
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicBool, AtomicIsize, AtomicU32, Ordering},
        mpsc,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use tauri::{AppHandle, Emitter};
use windows::{
    Win32::{
        Foundation::{HANDLE, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
        System::{
            LibraryLoader::GetModuleHandleW,
            Power::{
                HPOWERNOTIFY, POWERBROADCAST_SETTING, RegisterPowerSettingNotification,
                RegisterSuspendResumeNotification, UnregisterPowerSettingNotification,
                UnregisterSuspendResumeNotification,
            },
            RemoteDesktop::{
                NOTIFY_FOR_THIS_SESSION, WTSRegisterSessionNotification,
                WTSUnRegisterSessionNotification,
            },
            SystemServices::GUID_SESSION_DISPLAY_STATUS,
        },
        UI::{
            HiDpi::{GetDpiForSystem, GetDpiForWindow, GetSystemMetricsForDpi},
            Shell::NIN_SELECT,
            WindowsAndMessaging::{
                ChangeWindowMessageFilterEx, CreateWindowExW, DEVICE_NOTIFY_WINDOW_HANDLE,
                DefWindowProcW, DestroyWindow, DispatchMessageW, FindWindowW, GetMessageW, HMENU,
                KillTimer, MSG, MSGFLT_ALLOW, PBT_APMRESUMEAUTOMATIC, PBT_APMRESUMESUSPEND,
                PBT_APMSUSPEND, PBT_POWERSETTINGCHANGE, PostMessageW, PostQuitMessage,
                RegisterClassExW, RegisterWindowMessageW, SM_CXSMICON, SetCoalescableTimer,
                SetForegroundWindow, TPM_BOTTOMALIGN, TPM_LEFTALIGN, TPM_NONOTIFY, TPM_RETURNCMD,
                TPM_RIGHTBUTTON, TPM_VERTICAL, TPMPARAMS, TrackPopupMenuEx, TranslateMessage,
                WM_APP, WM_CONTEXTMENU, WM_DESTROY, WM_DISPLAYCHANGE, WM_DPICHANGED, WM_ENDSESSION,
                WM_NULL, WM_POWERBROADCAST, WM_SETTINGCHANGE, WM_TIMER, WM_WTSSESSION_CHANGE,
                WNDCLASSEXW, WS_EX_TOOLWINDOW, WS_POPUP, WTS_CONSOLE_CONNECT,
                WTS_CONSOLE_DISCONNECT, WTS_REMOTE_CONNECT, WTS_REMOTE_DISCONNECT,
                WTS_SESSION_LOCK, WTS_SESSION_UNLOCK,
            },
        },
    },
    core::{PCWSTR, w},
};

use super::{
    dark,
    draw::Painter,
    menu::{self, Marks},
    notify::{self, NotifyIcon},
    theme::{Accent, Look},
    ticker::{self, TaskbarTicker},
};
use crate::{
    bar::{self, Action, Ticker, View},
    platform::Pause,
};

pub const CLASS: PCWSTR = w!("Candlewick.Shell");
/// A second launch asks the running instance to reopen.
pub const WM_REOPEN: u32 = WM_APP + 1;
/// The tray icon's callback message.
pub const WM_ICON: u32 = WM_APP + 2;
/// Ticker input, `ticker::input` in wParam.
pub const WM_TICKER: u32 = WM_APP + 3;
const WM_RENDER: u32 = WM_APP + 4;
const WM_STOP: u32 = WM_APP + 5;
/// A system setting, the display or its scale changed; wParam 1 when the
/// colors did.
const WM_LOOK: u32 = WM_APP + 6;
/// `NIN_KEYSELECT`: `NIN_SELECT` from the keyboard (`NINF_KEY`).
const NIN_KEYSELECT: u32 = NIN_SELECT | 1;
const GUARD_TIMER: usize = 1;
/// The taskbar's layout changes without notice (tray icons come and go);
/// the ticker re-checks its place this often.
const GUARD_INTERVAL_MS: u32 = 2000;
/// How late a check may come, so Windows can fold it into other wake-ups.
const GUARD_TOLERANCE_MS: u32 = 500;

/// The hidden window, once created.
static WINDOW: AtomicIsize = AtomicIsize::new(0);
/// The registered "TaskbarCreated" message.
static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);
static THREAD: Mutex<Option<JoinHandle<()>>> = Mutex::new(None);
static ON_PAUSE: OnceLock<Box<dyn Fn(Pause, bool) + Send + Sync>> = OnceLock::new();

thread_local! {
    static SHELL: RefCell<Option<Shell>> = const { RefCell::new(None) };
}

struct Shell {
    app: AppHandle,
    window: HWND,
    notify: NotifyIcon,
    ticker: TaskbarTicker,
    painter: Option<Painter>,
    look: Look,
    accent: Accent,
    view: Option<View>,
    marks: Marks,
    menu: Option<menu::Open>,
    suspend_resume: Option<HPOWERNOTIFY>,
    display_state: Option<HPOWERNOTIFY>,
    /// Shutting down: nothing may come back (a render or the dropdown
    /// closing would otherwise re-attach the ticker).
    closed: bool,
}

/// Starts the shell thread and waits until its window exists.
pub fn start(app: &AppHandle) -> io::Result<()> {
    let (ready, created) = mpsc::sync_channel(1);
    let app = app.clone();
    let handle = thread::Builder::new().name("candlewick-shell".to_owned()).spawn(move || {
        match create(app) {
            Ok(()) => {
                let _ = ready.send(Ok(()));
            }
            Err(e) => {
                let _ = ready.send(Err(e));
                return;
            }
        }
        let mut message = MSG::default();
        // SAFETY: the standard message loop of this thread.
        unsafe {
            while GetMessageW(&mut message, None, 0, 0).0 > 0 {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        SHELL.with_borrow_mut(|shell| *shell = None);
    })?;
    created.recv().map_err(|_| io::Error::other("the shell thread ended early"))??;
    *THREAD.lock().unwrap_or_else(|e| e.into_inner()) = Some(handle);
    Ok(())
}

/// Removes the tray icon and ticker and ends the thread, waiting briefly.
pub fn stop() {
    if !post(WM_STOP, WPARAM(0), LPARAM(0)) {
        return;
    }
    let Some(handle) = THREAD.lock().unwrap_or_else(|e| e.into_inner()).take() else {
        return;
    };
    let deadline = Instant::now() + Duration::from_secs(1);
    while !handle.is_finished() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
}

/// Posts to the shell window; false before it exists or after it's gone.
pub fn post(message: u32, wparam: WPARAM, lparam: LPARAM) -> bool {
    match WINDOW.load(Ordering::Acquire) {
        0 => false,
        // SAFETY: posting to our own window, from any thread.
        window => unsafe { PostMessageW(Some(HWND(window as _)), message, wparam, lparam) }.is_ok(),
    }
}

pub fn request_render() -> bool {
    post(WM_RENDER, WPARAM(0), LPARAM(0))
}

pub fn on_pause(on_change: Box<dyn Fn(Pause, bool) + Send + Sync>) {
    let _ = ON_PAUSE.set(on_change);
}

fn create(app: AppHandle) -> io::Result<()> {
    // SAFETY: registers our classes and creates a hidden top-level window
    // (not message-only: those miss broadcasts such as TaskbarCreated).
    let (instance, window) = unsafe {
        let instance: HINSTANCE = GetModuleHandleW(PCWSTR::null())?.into();
        let class = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(procedure),
            hInstance: instance,
            lpszClassName: CLASS,
            ..Default::default()
        };
        if RegisterClassExW(&class) == 0 {
            return Err(io::Error::last_os_error());
        }
        ticker::register(instance)?;
        let window = CreateWindowExW(
            WS_EX_TOOLWINDOW,
            CLASS,
            w!("Candlewick"),
            WS_POPUP,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(instance),
            None,
        )?;
        let taskbar_created = RegisterWindowMessageW(w!("TaskbarCreated"));
        TASKBAR_CREATED.store(taskbar_created, Ordering::Release);
        // Let these through should this process run elevated (UIPI).
        for message in [taskbar_created, WM_REOPEN] {
            let _ = ChangeWindowMessageFilterEx(window, message, MSGFLT_ALLOW, None);
        }
        (instance, window)
    };
    dark::allow(window);
    dark::refresh();
    // SAFETY: notification registrations for our window, undone in `close`.
    let (suspend_resume, display_state) = unsafe {
        let handle = HANDLE(window.0);
        let _ = WTSRegisterSessionNotification(window, NOTIFY_FOR_THIS_SESSION);
        SetCoalescableTimer(Some(window), GUARD_TIMER, GUARD_INTERVAL_MS, None, GUARD_TOLERANCE_MS);
        (
            RegisterSuspendResumeNotification(handle, DEVICE_NOTIFY_WINDOW_HANDLE).ok(),
            // This session's display (not the physical console's, which a
            // remote session doesn't use); sends the current state right
            // away, then every change.
            RegisterPowerSettingNotification(
                handle,
                &GUID_SESSION_DISPLAY_STATUS,
                DEVICE_NOTIFY_WINDOW_HANDLE,
            )
            .ok(),
        )
    };
    let painter = Painter::new().map_err(|e| log::warn!("no ticker: {e}")).ok();
    let shell = Shell {
        app,
        window,
        notify: NotifyIcon::new(window),
        ticker: TaskbarTicker::new(instance),
        painter,
        look: Look::current(),
        accent: Accent::current(),
        view: None,
        marks: Marks::default(),
        menu: None,
        suspend_resume,
        display_state,
        closed: false,
    };
    SHELL.with_borrow_mut(|cell| *cell = Some(shell));
    WINDOW.store(window.0 as isize, Ordering::Release);
    // The first render draws the icon, so it is added with its picture.
    render();
    with_shell(|shell| shell.notify.add());
    Ok(())
}

/// Runs `f` on the shell state unless it is already borrowed further up
/// this thread's stack (a nested message loop), in which case `None`.
fn with_shell<R>(f: impl FnOnce(&mut Shell) -> R) -> Option<R> {
    SHELL.with(|cell| cell.try_borrow_mut().ok()?.as_mut().map(f))
}

unsafe extern "system" fn procedure(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_RENDER => render(),
        WM_ICON => {
            if !icon_event(wparam, lparam) {
                retry(message, wparam, lparam);
            }
        }
        WM_TICKER => {
            if !ticker_event(wparam.0) {
                retry(message, wparam, lparam);
            }
        }
        WM_REOPEN => {
            let reopened = with_shell(|shell| {
                let app = shell.app.clone();
                on_main(&shell.app, move || {
                    if let Err(e) = crate::window::reopen(&app) {
                        log::error!("cannot reopen a window: {e}");
                    }
                });
            });
            if reopened.is_none() {
                retry(message, wparam, lparam);
            }
        }
        WM_TIMER if wparam.0 == GUARD_TIMER => {
            // Busy: the next tick comes soon enough.
            with_shell(Shell::guard);
        }
        // Broadcasts arrive as sent messages, possibly while this thread waits
        // on Explorer inside a shell call; they're handled from the queue.
        WM_SETTINGCHANGE => {
            // SAFETY: WM_SETTINGCHANGE carries a null or a null-terminated string.
            let colors = lparam.0 != 0
                && unsafe { PCWSTR(lparam.0 as *const u16).to_string() }
                    .is_ok_and(|area| area == "ImmersiveColorSet");
            post(WM_LOOK, WPARAM(usize::from(colors)), LPARAM(0));
        }
        WM_DISPLAYCHANGE | WM_DPICHANGED => {
            post(WM_LOOK, WPARAM(0), LPARAM(0));
        }
        WM_LOOK => {
            if with_shell(|shell| shell.look_changed(wparam.0 != 0)).is_none() {
                retry(message, wparam, lparam);
            }
        }
        WM_POWERBROADCAST => {
            power(wparam.0 as u32, lparam);
            return LRESULT(1);
        }
        WM_WTSSESSION_CHANGE => session(wparam.0 as u32),
        WM_ENDSESSION if wparam.0 != 0 => {
            with_shell(Shell::close);
        }
        WM_STOP => {
            with_shell(Shell::close);
            // SAFETY: our own window; its WM_DESTROY ends the loop.
            unsafe {
                let _ = DestroyWindow(window);
            }
        }
        WM_DESTROY => {
            WINDOW.store(0, Ordering::Release);
            // SAFETY: ends this thread's message loop.
            unsafe { PostQuitMessage(0) };
        }
        message if message != 0 && message == TASKBAR_CREATED.load(Ordering::Acquire) => {
            if with_shell(Shell::taskbar_created).is_none() {
                retry(message, wparam, lparam);
            }
        }
        // SAFETY: the default procedure for our own window.
        _ => return unsafe { DefWindowProcW(window, message, wparam, lparam) },
    }
    LRESULT(0)
}

fn render() {
    if with_shell(|shell| shell.apply(bar::view(&shell.app))).is_none() {
        request_render();
    }
}

/// The state was busy further up the stack (a sent message arrived while
/// this thread waited): handle the message again once it returns.
fn retry(message: u32, wparam: WPARAM, lparam: LPARAM) {
    post(message, wparam, lparam);
}

/// Tray icon events, protocol version 4: the event in lParam's low word, the
/// anchor point (the pointer, or the icon for the keyboard) in wParam. False
/// when the state was busy.
fn icon_event(wparam: WPARAM, lparam: LPARAM) -> bool {
    let event = (lparam.0 as u32) & 0xFFFF;
    if !matches!(event, NIN_SELECT | NIN_KEYSELECT | WM_CONTEXTMENU) {
        return true;
    }
    let anchor = POINT {
        x: i32::from(wparam.0 as u16 as i16),
        y: i32::from((wparam.0 >> 16) as u16 as i16),
    };
    let Some(exclude) = with_shell(|shell| shell.notify.rect()) else {
        return false;
    };
    open_menu(anchor, exclude, false)
}

/// False when the state was busy.
fn ticker_event(event: usize) -> bool {
    if event != ticker::input::CLICK {
        return with_shell(|shell| {
            shell.ticker.input(event);
            shell.refresh_ticker();
        })
        .is_some();
    }
    let Some(bounds) = with_shell(|shell| {
        shell.ticker.input(event);
        shell.ticker.screen_rect()
    }) else {
        return false;
    };
    match bounds {
        Some(bounds) => open_menu(POINT { x: bounds.left, y: bounds.top }, Some(bounds), true),
        None => true,
    }
}

/// Shows the dropdown with its bottom-left corner at `at`, clear of
/// `exclude` (the icon or ticker it opens from), and acts on the choice.
/// False when the state was busy.
fn open_menu(at: POINT, exclude: Option<RECT>, from_ticker: bool) -> bool {
    let Some(prepared) = with_shell(|shell| shell.prepare_menu(from_ticker)) else {
        return false;
    };
    // Already open, or nothing to show.
    let Some((menu, owner)) = prepared else {
        return true;
    };
    let params = TPMPARAMS {
        cbSize: size_of::<TPMPARAMS>() as u32,
        rcExclude: exclude.unwrap_or(RECT { left: at.x, top: at.y, right: at.x, bottom: at.y }),
    };
    let flags = TPM_RETURNCMD
        | TPM_NONOTIFY
        | TPM_RIGHTBUTTON
        | TPM_LEFTALIGN
        | TPM_BOTTOMALIGN
        | TPM_VERTICAL;
    // SAFETY: the menu stays alive (owned by the shell) until `menu_closed`.
    // A popup only closes on an outside click if its owner is in front, and
    // the null message afterwards is the documented companion to that.
    let command = unsafe {
        let _ = SetForegroundWindow(owner);
        let command = TrackPopupMenuEx(menu, flags.0, at.x, at.y, owner, Some(&raw const params));
        let _ = PostMessageW(Some(owner), WM_NULL, WPARAM(0), LPARAM(0));
        command.0 as u32
    };
    let chosen = with_shell(|shell| shell.menu_closed(command)).flatten();
    if let Some((app, action)) = chosen {
        let handle = app.clone();
        on_main(&app, move || bar::perform(&handle, action));
    }
    true
}

/// Window work belongs to the main thread; this one never waits for it.
fn on_main(app: &AppHandle, task: impl FnOnce() + Send + 'static) {
    if let Err(e) = app.run_on_main_thread(task) {
        log::error!("cannot reach the main thread: {e}");
    }
}

fn power(event: u32, lparam: LPARAM) {
    let pause = |reason, paused| {
        if let Some(on_pause) = ON_PAUSE.get() {
            on_pause(reason, paused);
        }
    };
    match event {
        PBT_APMSUSPEND => pause(Pause::SystemSleep, true),
        PBT_APMRESUMEAUTOMATIC | PBT_APMRESUMESUSPEND => pause(Pause::SystemSleep, false),
        PBT_POWERSETTINGCHANGE if lparam.0 != 0 => {
            let setting = lparam.0 as *const POWERBROADCAST_SETTING;
            // SAFETY: for this event lParam points at a POWERBROADCAST_SETTING
            // followed by its DataLength bytes; the display status is a DWORD.
            let (guid, state) = unsafe {
                let data = (&raw const (*setting).Data).cast::<u32>();
                (
                    (*setting).PowerSetting,
                    ((*setting).DataLength >= 4).then(|| data.read_unaligned()),
                )
            };
            // 0 off, 1 on, 2 dimmed (still readable).
            if guid == GUID_SESSION_DISPLAY_STATUS
                && let Some(state) = state
            {
                pause(Pause::DisplaySleep, state == 0);
            }
        }
        _ => {}
    }
}

/// Locked, or switched away from (fast user switching, a remote session
/// ending). Tracked apart: coming back reconnects before it unlocks.
fn session(event: u32) {
    static LOCKED: AtomicBool = AtomicBool::new(false);
    static DISCONNECTED: AtomicBool = AtomicBool::new(false);
    match event {
        WTS_SESSION_LOCK => LOCKED.store(true, Ordering::Relaxed),
        WTS_SESSION_UNLOCK => LOCKED.store(false, Ordering::Relaxed),
        WTS_CONSOLE_DISCONNECT | WTS_REMOTE_DISCONNECT => {
            DISCONNECTED.store(true, Ordering::Relaxed);
        }
        WTS_CONSOLE_CONNECT | WTS_REMOTE_CONNECT => DISCONNECTED.store(false, Ordering::Relaxed),
        _ => return,
    }
    let paused = LOCKED.load(Ordering::Relaxed) || DISCONNECTED.load(Ordering::Relaxed);
    if let Some(on_pause) = ON_PAUSE.get() {
        on_pause(Pause::SessionInactive, paused);
    }
}

impl Shell {
    fn apply(&mut self, view: View) {
        if self.closed {
            return;
        }
        self.ticker.set(view.ticker.clone().map(|ticker| (ticker, view.scheme)));
        if let Some(menu) = &mut self.menu {
            menu.update(&view, &mut self.marks);
        }
        self.view = Some(view);
        self.redraw();
    }

    /// Everything the view and the look decide: the icon, its tooltip, the
    /// ticker.
    fn redraw(&mut self) {
        if self.closed {
            return;
        }
        let palette = self.look.taskbar_palette();
        if let Some(view) = &self.view {
            let dot = match &view.ticker {
                Some(ticker) if ticker.stale => palette.secondary,
                Some(Ticker { change: Some((_, direction)), .. }) => {
                    bar::hue(*direction, view.scheme).map_or(palette.text, |hue| palette.hue(hue))
                }
                _ => palette.text,
            };
            let look = notify::Look { size: self.icon_size(), ink: palette.text, dot };
            self.notify.update(look, &tooltip(view));
        }
        self.refresh_ticker();
    }

    fn refresh_ticker(&mut self) {
        self.ticker.refresh(&self.look, self.painter.as_ref());
    }

    /// SM_CXSMICON at the taskbar's DPI: 16, 20, 24 or 32 px at 100–200%.
    fn icon_size(&self) -> u32 {
        // SAFETY: plain queries.
        let dpi = unsafe {
            FindWindowW(w!("Shell_TrayWnd"), PCWSTR::null())
                .ok()
                .map(|taskbar| GetDpiForWindow(taskbar))
                .filter(|dpi| *dpi != 0)
                .unwrap_or_else(|| GetDpiForSystem())
        };
        // SAFETY: a plain query.
        let size = unsafe { GetSystemMetricsForDpi(SM_CXSMICON, dpi) };
        u32::try_from(size).ok().filter(|size| *size > 0).unwrap_or(16)
    }

    /// Every couple of seconds: the icon comes back if the notification area
    /// lost it, and both follow the taskbar's layout and scale, which change
    /// without notice.
    fn guard(&mut self) {
        if self.notify.added() {
            self.redraw();
        } else {
            // The icon only: the ticker follows a new taskbar by itself, and
            // re-creating it here would flicker on every tick until the
            // notification area takes the icon.
            self.add_icon();
        }
    }

    /// Explorer came back: a new icon, and the ticker attached afresh.
    fn taskbar_created(&mut self) {
        self.ticker.reset();
        self.add_icon();
    }

    /// Adds the icon anew, drawn first so it's added with its picture.
    fn add_icon(&mut self) {
        // A timer tick or broadcast queued before closing.
        if self.closed {
            return;
        }
        self.notify.remove();
        self.redraw();
        self.notify.add();
    }

    /// A setting, the display or its scale changed: read the look again and
    /// redraw (tray icons, the taskbar's alignment or its scale may have
    /// changed too). `colors` when the light, dark or accent colors did.
    fn look_changed(&mut self, colors: bool) {
        if colors {
            dark::refresh();
            let accent = Accent::current();
            if accent != self.accent {
                self.accent = accent;
                let _ = self.app.emit("system-colors", &self.accent);
            }
        }
        self.look = Look::current();
        self.redraw();
    }

    /// Builds the dropdown for the current view; `None` while one is open.
    fn prepare_menu(&mut self, from_ticker: bool) -> Option<(HMENU, HWND)> {
        if self.menu.is_some() {
            return None;
        }
        let view = self.view.as_ref()?;
        // SAFETY: plain queries on our own window.
        let dpi = match unsafe { GetDpiForWindow(self.window) } {
            0 => 96,
            dpi => dpi,
        };
        let size = (16 * dpi).div_ceil(96);
        self.marks.prepare(size, self.look.apps, self.look.menu_palette());
        let open = menu::Open::build(view, &mut self.marks)
            .map_err(|e| log::error!("cannot build the dropdown: {e}"))
            .ok()?;
        let handle = open.menu;
        self.menu = Some(open);
        if from_ticker {
            self.ticker.set_open(true);
            self.refresh_ticker();
        }
        Some((handle, self.window))
    }

    fn menu_closed(&mut self, command: u32) -> Option<(AppHandle, Action)> {
        let open = self.menu.take()?;
        self.ticker.set_open(false);
        self.refresh_ticker();
        let action = open.action(command)?;
        Some((self.app.clone(), action))
    }

    fn close(&mut self) {
        self.closed = true;
        self.ticker.set(None);
        self.notify.remove();
        self.ticker.detach();
        // An open dropdown still shows them; they go with the thread.
        if self.menu.is_none() {
            self.marks.clear();
        }
        // SAFETY: undoes the registrations made in `create`.
        unsafe {
            let _ = KillTimer(Some(self.window), GUARD_TIMER);
            let _ = WTSUnRegisterSessionNotification(self.window);
            if let Some(handle) = self.suspend_resume.take() {
                let _ = UnregisterSuspendResumeNotification(handle);
            }
            if let Some(handle) = self.display_state.take() {
                let _ = UnregisterPowerSettingNotification(handle);
            }
        }
    }
}

/// `Candlewick`, then the pinned entry and a feed's trouble, if any.
fn tooltip(view: &View) -> String {
    let mut lines = vec!["Candlewick".to_owned()];
    if let Some(ticker) = &view.ticker {
        lines.push(ticker.spoken());
    }
    if !view.caption.is_empty() {
        lines.push(view.caption.clone());
    }
    lines.join("\n")
}
