use std::{future::Future, path::PathBuf, sync::Arc};

use tokio::{runtime::Handle, sync::watch, task::JoinHandle};

use crate::{agent::Agent, model::Shared};

const IDENTIFIER: &str = "com.influo.candlewick";

#[derive(Clone)]
pub struct App(Arc<Inner>);

struct Inner {
    handle: tauri::AppHandle,
    runtime: Handle,
    shared: Shared,
    agent: Agent,
    topics: [watch::Sender<u64>; 6],
}

#[derive(Clone, Copy)]
pub enum Topic {
    Settings,
    Locale,
    Portfolio,
    Ths,
    Update,
    ChartInstrument,
}

impl App {
    pub(crate) fn new(handle: tauri::AppHandle, runtime: Handle, shared: Shared) -> Self {
        Self(Arc::new(Inner {
            handle,
            runtime,
            shared,
            agent: Agent::default(),
            topics: std::array::from_fn(|_| watch::channel(0).0),
        }))
    }

    pub fn shared(&self) -> &Shared {
        &self.0.shared
    }
    pub fn agent(&self) -> &Agent {
        &self.0.agent
    }

    pub fn spawn<F>(&self, fut: F) -> JoinHandle<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        self.0.runtime.spawn(fut)
    }

    pub fn spawn_blocking<F, R>(&self, f: F) -> JoinHandle<R>
    where
        F: FnOnce() -> R + Send + 'static,
        R: Send + 'static,
    {
        self.0.runtime.spawn_blocking(f)
    }

    /// Wakes subscribers, which then read the current state.
    pub fn notify(&self, topic: Topic) {
        self.0.topics[topic as usize].send_modify(|revision| *revision = revision.wrapping_add(1));
    }

    pub fn subscribe(&self, topic: Topic) -> watch::Receiver<u64> {
        self.0.topics[topic as usize].subscribe()
    }

    pub fn on_main(&self, f: impl FnOnce() + Send + 'static) {
        if let Err(error) = self.0.handle.run_on_main_thread(f) {
            log::error!("cannot reach the main thread: {error}");
        }
    }

    pub fn exit(&self, code: i32) {
        self.0.handle.exit(code);
    }

    /// Shell access for windows, tray, and updater.
    pub(crate) fn handle(&self) -> &tauri::AppHandle {
        &self.0.handle
    }
}

pub fn config_dir() -> PathBuf {
    std::env::var_os("CANDLEWICK_CONFIG_DIR").map(PathBuf::from).unwrap_or_else(|| {
        dirs::config_dir().expect("system config directory unavailable").join(IDENTIFIER)
    })
}
