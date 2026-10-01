//! AI agents' access to Candlewick (settings → 通用 → AI 助手接入): while on,
//! a read-only HTTP API on this machine (`api.rs`) and a skill that tells
//! Claude Code, Codex and OpenCode how to call it (`skill.rs`). Turning it on
//! starts both; turning it off stops the API and takes the skill away.

mod api;
mod skill;

use std::{
    fs, io,
    net::{Ipv4Addr, SocketAddr},
    path::Path,
    sync::Mutex,
};

use ring::rand::{SecureRandom, SystemRandom};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tokio::{net::TcpListener, sync::oneshot};

use crate::{credentials, model::Shared, sign, window};

/// Where the API listens unless another program holds it.
const PORT: u16 = 52733;
const STATUS_EVENT: &str = "agent";

/// What the settings window shows under the switch.
#[derive(Debug, Clone, Default, Serialize)]
pub struct AgentStatus {
    /// Access has been turned on (the switch may lead it by a moment).
    on: bool,
    /// The agents that have the skill.
    agents: Vec<&'static str>,
    error: Option<String>,
}

#[derive(Default)]
pub struct Agent {
    server: Mutex<Option<Server>>,
    status: Mutex<AgentStatus>,
    /// One change at a time.
    applying: tokio::sync::Mutex<()>,
}

struct Server {
    port: u16,
    /// The token it takes.
    token: String,
    /// Stops it when sent or dropped.
    _stop: oneshot::Sender<()>,
}

impl Agent {
    pub fn status(&self) -> AgentStatus {
        self.status.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

/// Starts or stops access to follow the settings.
pub fn sync(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move { apply(&app).await });
}

async fn apply(app: &AppHandle) {
    let agent = app.state::<Agent>();
    let _applying = agent.applying.lock().await;
    let on = app.state::<Shared>().model().settings.agent_access;
    let status = if on { start(app, &agent).await } else { stop(app, &agent) };
    *agent.status.lock().unwrap_or_else(|e| e.into_inner()) = status.clone();
    if app.get_webview_window(window::SETTINGS).is_some() {
        let _ = app.emit_to(window::SETTINGS, STATUS_EVENT, &status);
    }
}

async fn start(app: &AppHandle, agent: &Agent) -> AgentStatus {
    let failed = |error: String| AgentStatus { on: true, agents: Vec::new(), error: Some(error) };
    let (Ok(config), Ok(home)) = (app.path().app_config_dir(), app.path().home_dir()) else {
        return failed("找不到配置目录".to_owned());
    };
    let token_path = config.join("agent-token");
    let token = match token(&token_path) {
        Ok(token) => token,
        Err(e) => return failed(format!("无法保存访问令牌：{e}")),
    };
    let running = {
        let mut server = agent.server.lock().unwrap_or_else(|e| e.into_inner());
        // A token made anew since (its file was removed) needs a new server.
        if server.as_ref().is_some_and(|s| s.token != token) {
            server.take();
        }
        server.as_ref().map(|s| s.port)
    };
    let port = match running {
        Some(port) => port,
        None => match listen().await {
            Ok(listener) => serve(app, agent, listener, &token),
            Err(e) => return failed(format!("无法启动本机接口：{e}")),
        },
    };
    // Rewritten on each start: the port may differ, and the skill's text with
    // the app's version.
    match skill::install(&home, &format!("http://127.0.0.1:{port}"), &token_path) {
        Ok(agents) => AgentStatus { on: true, agents, error: None },
        Err(e) => failed(format!("无法写入 skill：{e}")),
    }
}

fn stop(app: &AppHandle, agent: &Agent) -> AgentStatus {
    // Dropping the sender stops the server.
    agent.server.lock().unwrap_or_else(|e| e.into_inner()).take();
    if let Ok(home) = app.path().home_dir() {
        skill::uninstall(&home);
    }
    AgentStatus::default()
}

/// The usual port, or any free one if that is taken.
async fn listen() -> io::Result<TcpListener> {
    let at = |port| SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    match TcpListener::bind(at(PORT)).await {
        Ok(listener) => Ok(listener),
        Err(e) => {
            log::warn!("port {PORT} unavailable ({e}), taking another");
            TcpListener::bind(at(0)).await
        }
    }
}

/// Serves the API on `listener` until stopped; returns its port.
fn serve(app: &AppHandle, agent: &Agent, listener: TcpListener, token: &str) -> u16 {
    let port = listener.local_addr().map_or(PORT, |a| a.port());
    let router = api::router(app.clone(), port, token);
    let (stop, stopped) = oneshot::channel::<()>();
    tauri::async_runtime::spawn(async move {
        let serving = axum::serve(listener, router).with_graceful_shutdown(async {
            let _ = stopped.await;
        });
        if let Err(e) = serving.await {
            log::error!("agent API stopped: {e}");
        }
    });
    log::info!("agent API on 127.0.0.1:{port}");
    *agent.server.lock().unwrap_or_else(|e| e.into_inner()) =
        Some(Server { port, token: token.to_owned(), _stop: stop });
    port
}

/// The token requests must carry: kept in a file only the user can read,
/// made once.
fn token(path: &Path) -> io::Result<String> {
    if let Ok(saved) = fs::read_to_string(path)
        && saved.len() == 64
        && saved.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Ok(saved);
    }
    let mut bytes = [0u8; 32];
    SystemRandom::new().fill(&mut bytes).map_err(|_| io::Error::other("no randomness"))?;
    let token = sign::hex(&bytes);
    credentials::write_private(path, token.as_bytes())?;
    Ok(token)
}
