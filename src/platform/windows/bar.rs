//! The taskbar presence: tray icon, ticker and dropdown, all run by the
//! shell thread (`shell`).

use crate::app::App;

use super::shell;

/// Starts the shell thread, which shows the tray icon and ticker. The app
/// begins in efficiency mode: no window is open yet.
pub fn create(app: &App) -> std::io::Result<()> {
    super::set_efficiency_mode(true);
    shell::start(app)?;
    Ok(())
}

/// Queues a render on the shell thread; false if it isn't running.
pub fn schedule_render(_app: &App) -> bool {
    shell::request_render()
}

/// Removes the tray icon (Windows leaves a dead one behind otherwise) and
/// the ticker.
pub fn shutdown() {
    shell::stop();
}
