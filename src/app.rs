use std::{
    future::Future,
    path::PathBuf,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicUsize, Ordering},
    },
};

use tokio::{runtime::Handle, sync::watch, task::JoinHandle};

use crate::{agent::Agent, model::Shared};

pub(crate) const IDENTIFIER: &str = "com.influo.candlewick";

#[derive(Clone)]
pub struct App(Arc<Inner>);

type MainTask = Box<dyn FnOnce(&mut gpui_kit::App) + Send>;

struct Inner {
    main: tokio::sync::mpsc::UnboundedSender<MainTask>,
    windows: AtomicUsize,
    main_thread: std::thread::ThreadId,
    runtime: Handle,
    shared: Shared,
    agent: Agent,
    topics: [watch::Sender<u64>; 6],
    updater: OnceLock<Arc<crate::update::Updater>>,
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
    pub(crate) fn new(cx: &mut gpui_kit::App, runtime: Handle, shared: Shared) -> Self {
        let (main, mut receiver) = tokio::sync::mpsc::unbounded_channel::<MainTask>();
        cx.spawn(async move |cx| {
            while let Some(task) = receiver.recv().await {
                cx.update(task);
            }
        })
        .detach();
        Self(Arc::new(Inner {
            main,
            windows: AtomicUsize::new(0),
            main_thread: std::thread::current().id(),
            runtime,
            shared,
            agent: Agent::default(),
            topics: std::array::from_fn(|_| watch::channel(0).0),
            updater: OnceLock::new(),
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

    pub fn on_main(&self, f: impl FnOnce() + Send + 'static) -> bool {
        if std::thread::current().id() == self.0.main_thread {
            f();
            true
        } else {
            self.on_gpui(move |_| f())
        }
    }

    pub(crate) fn on_gpui(&self, f: impl FnOnce(&mut gpui_kit::App) + Send + 'static) -> bool {
        self.0.main.send(Box::new(f)).is_ok()
    }

    pub fn exit(&self) {
        self.on_gpui(|cx| cx.quit());
    }

    pub(crate) fn set_updater(&self, updater: Arc<crate::update::Updater>) {
        assert!(self.0.updater.set(updater).is_ok(), "updater already started");
    }

    pub(crate) fn updater(&self) -> Option<Arc<crate::update::Updater>> {
        self.0.updater.get().cloned()
    }

    pub(crate) fn windows_open(&self) -> bool {
        self.0.windows.load(Ordering::Acquire) != 0
    }

    pub(crate) fn set_window_count(&self, count: usize) {
        self.0.windows.store(count, Ordering::Release);
    }

    #[cfg(target_os = "macos")]
    pub(crate) fn request_restart(&self) {
        self.on_gpui(|cx| cx.restart());
    }
}

pub fn config_dir() -> PathBuf {
    std::env::var_os("CANDLEWICK_CONFIG_DIR").map(PathBuf::from).unwrap_or_else(|| {
        dirs::config_dir().expect("system config directory unavailable").join(IDENTIFIER)
    })
}
