//! IPC surface for the settings window.

use serde::{Serialize, Serializer};
use tauri::{AppHandle, State, WebviewWindow};

use crate::{
    macos,
    model::{self, Settings, Shared},
    tray,
    window::StatusView,
};

#[derive(Debug, thiserror::Error)]
pub enum CommandError {
    #[error("{0}")]
    Invalid(String),
    #[error("无法保存设置：{0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    LoginItem(String),
    #[error(transparent)]
    Tauri(#[from] tauri::Error),
}

// The webview only needs a readable message.
impl Serialize for CommandError {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

type CmdResult<T> = Result<T, CommandError>;

#[tauri::command]
pub fn get_settings(shared: State<'_, Shared>) -> Settings {
    shared.model().settings.clone()
}

/// Validates, persists and applies settings; returns them normalized.
#[tauri::command]
pub fn save_settings(
    app: AppHandle,
    shared: State<'_, Shared>,
    settings: Settings,
) -> CmdResult<Settings> {
    let settings = settings.validated().map_err(CommandError::Invalid)?;
    model::save(&shared.settings_path, &settings)?;
    {
        let mut model = shared.model();
        model.quotes.retain(|symbol, _| settings.coins.iter().any(|c| &c.symbol == symbol));
        model.settings = settings.clone();
    }
    let symbols = settings.symbols();
    shared.control.send_if_modified(|control| {
        let changed = control.symbols != symbols;
        control.symbols = symbols;
        changed
    });
    tray::request_render(&app);
    Ok(settings)
}

#[tauri::command]
pub fn get_status(shared: State<'_, Shared>) -> StatusView {
    StatusView::from(&shared.model().status)
}

#[tauri::command]
pub fn get_login_item() -> bool {
    macos::login_item_enabled()
}

#[tauri::command]
pub fn set_login_item(enabled: bool) -> CmdResult<bool> {
    macos::set_login_item(enabled).map_err(CommandError::LoginItem)?;
    Ok(macos::login_item_enabled())
}

/// The page has painted; now the window can appear without a blank flash.
#[tauri::command]
pub fn settings_ready(window: WebviewWindow) -> CmdResult<()> {
    window.show()?;
    window.set_focus()?;
    Ok(())
}
