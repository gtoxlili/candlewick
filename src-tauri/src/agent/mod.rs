//! AI agents' access to Candlewick (settings → General → AI Agent Access): while on,
//! a read-only HTTP API on this machine (`api.rs`) and a skill that tells
//! Claude Code, Codex and OpenCode how to call it (`skill.rs`). The skill
//! exists exactly while the API runs: both start when access is turned on or
//! the app launches with it on, and both go when it is turned off or the app
//! quits.

mod api;
mod skill;

use std::{
    io,
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
        return failed(t!("error.noConfigDir").to_owned());
    };
    let token_path = config.join("agent-token");
    let running = agent
        .server
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .map(|s| (s.port, s.token.clone()));
    let started = match running {
        Some((port, token)) => {
            credentials::write_private(&token_path, token.as_bytes()).map(|()| port)
        }
        None => start_server(app, agent, &token_path).await,
    };
    let port = match started {
        Ok(port) => port,
        Err(e) => {
            // A skill from before would point agents at nothing, or at someone else.
            skill::uninstall(&home);
            return failed(t!("error.agentServer", error = e));
        }
    };
    match skill::install(&home, &format!("http://127.0.0.1:{port}"), &token_path) {
        Ok(agents) => AgentStatus { on: true, agents, error: None },
        Err(e) => failed(t!("error.agentSkill", error = e)),
    }
}

/// A server with a token of its own (one taken from an earlier run is
/// worthless), on the usual port or any free one. Returns its port.
async fn start_server(app: &AppHandle, agent: &Agent, token_path: &Path) -> io::Result<u16> {
    let mut bytes = [0u8; 32];
    SystemRandom::new().fill(&mut bytes).map_err(|_| io::Error::other("no randomness"))?;
    let token = sign::hex(&bytes);
    credentials::write_private(token_path, token.as_bytes())?;
    let listener = listen().await?;
    Ok(serve(app, agent, listener, token))
}

fn stop(app: &AppHandle, agent: &Agent) -> AgentStatus {
    // Dropping the sender stops the server.
    agent.server.lock().unwrap_or_else(|e| e.into_inner()).take();
    if let Ok(home) = app.path().home_dir() {
        skill::uninstall(&home);
    }
    AgentStatus::default()
}

/// The app is quitting: the skill goes with the API, so agents only ever
/// see it while it can be called. The next launch puts it back.
pub fn shutdown(app: &AppHandle) {
    let running = app.state::<Agent>().server.lock().unwrap_or_else(|e| e.into_inner()).take();
    if running.is_some()
        && let Ok(home) = app.path().home_dir()
    {
        skill::uninstall(&home);
    }
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
fn serve(app: &AppHandle, agent: &Agent, listener: TcpListener, token: String) -> u16 {
    let port = listener.local_addr().map_or(PORT, |a| a.port());
    let router = api::router(app.clone(), port, &token);
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
        Some(Server { port, token, _stop: stop });
    port
}
