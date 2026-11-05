//! The native windows: settings, chart and holdings.

use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, LazyLock},
    task::{Context as TaskContext, Poll},
};

use gpui_kit::{Context, FontFeatures, Hsla, Rgba, Styled, Task, Window, WindowAppearance};

use crate::app::{App, Topic};

pub mod assets;
pub mod chart;
pub mod holdings;
pub mod prefs;
pub mod settings;
#[cfg(all(target_os = "macos", feature = "test-support"))]
pub mod testing;
pub mod theme;
pub mod widgets;

/// Hold the returned task in the view; dropping it stops observation.
pub fn observe<V: 'static>(
    app: &App,
    topic: Topic,
    cx: &mut Context<V>,
    mut changed: impl FnMut(&mut V, &mut Context<V>) + 'static,
) -> Task<()> {
    let mut receiver = app.subscribe(topic);
    cx.spawn(async move |view, cx| {
        while receiver.changed().await.is_ok() {
            if view
                .update(cx, |view, cx| {
                    changed(view, cx);
                    cx.notify();
                })
                .is_err()
            {
                break;
            }
        }
    })
}

/// Hold the returned task in the view; dropping it stops observation.
pub fn observe_in<V: 'static>(
    app: &App,
    topic: Topic,
    window: &Window,
    cx: &mut Context<V>,
    mut changed: impl FnMut(&mut V, &mut Window, &mut Context<V>) + 'static,
) -> Task<()> {
    let mut receiver = app.subscribe(topic);
    cx.spawn_in(window, async move |view, cx| {
        while receiver.changed().await.is_ok() {
            if view
                .update_in(cx, |view, window, cx| {
                    changed(view, window, cx);
                    cx.notify();
                })
                .is_err()
            {
                break;
            }
        }
    })
}

/// Update the native title on locale changes. Hold the task in the view.
pub fn observe_title<V: 'static>(
    app: &App,
    window: &Window,
    cx: &mut Context<V>,
    mut title: impl FnMut(&mut V, &mut Context<V>) -> String + 'static,
) -> Task<()> {
    observe_in(app, Topic::Locale, window, cx, move |view, window, cx| {
        window.set_window_title(&title(view, cx));
    })
}

/// Future wrapper that aborts the underlying Tokio task if the GPUI `Task` is dropped.
pub struct RunTask<T>(tokio::task::JoinHandle<T>);

impl<T> Drop for RunTask<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}

impl<T> Future for RunTask<T> {
    type Output = Result<T, tokio::task::JoinError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<Self::Output> {
        Pin::new(&mut self.0).poll(cx)
    }
}

/// Run an API future on Tokio. Await it in `cx.spawn`, then use `view.update(cx, ...)` and `cx.notify()`.
/// Dropping the enclosing GPUI `Task` aborts the Tokio task automatically.
pub fn run<F>(app: &App, future: F) -> RunTask<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    RunTask(app.spawn(future))
}

/// Shared OpenType `tnum` feature list without per-call heap allocations.
pub fn tabular_features() -> FontFeatures {
    static TABULAR: LazyLock<FontFeatures> =
        LazyLock::new(|| FontFeatures(Arc::new(vec![("tnum".into(), 1)])));
    TABULAR.clone()
}

/// Keep the system font while making digits equal width.
pub fn tabular<T: Styled>(element: T) -> T {
    element.font_features(tabular_features())
}

/// Cubic ease-out on `0..=1`.
pub fn ease_out(t: f32) -> f32 {
    1. - (1. - t.clamp(0., 1.)).powi(3)
}

/// Interpolate two colors in RGBA space so contrasting hues never rainbow through intermediate hues.
pub fn mix_color(a: Hsla, b: Hsla, t: f64) -> Hsla {
    let t = t.clamp(0., 1.) as f32;
    let a: Rgba = a.into();
    let b: Rgba = b.into();
    Rgba {
        r: a.r + (b.r - a.r) * t,
        g: a.g + (b.g - a.g) * t,
        b: a.b + (b.b - a.b) * t,
        a: a.a + (b.a - a.a) * t,
    }
    .into()
}

fn dark(appearance: WindowAppearance) -> bool {
    matches!(appearance, WindowAppearance::Dark | WindowAppearance::VibrantDark)
}

/// Install appearance and settings observers; keep the returned tasks in the view.
pub fn setup<V: 'static>(app: &App, window: &mut Window, cx: &mut Context<V>) -> Vec<Task<()>> {
    theme::apply(app, dark(window.appearance()), cx);
    let handle = app.clone();
    cx.observe_window_appearance(window, move |_, window, cx| {
        theme::apply(&handle, dark(window.appearance()), cx);
        cx.notify();
    })
    .detach();
    let handle = app.clone();
    vec![observe(app, Topic::Settings, cx, move |_, cx| {
        let is_dark = gpui_kit::component::Theme::global(cx).is_dark();
        theme::apply(&handle, is_dark, cx);
    })]
}

/// Draw the common window title bar over the system material.
pub fn frame(title: String, cx: &gpui_kit::App) -> gpui_kit::Div {
    use gpui_kit::{ParentElement, component::TitleBar, div};
    div().size_full().flex().flex_col().child(
        TitleBar::new()
            .bg(gpui_kit::component::Theme::global(cx).transparent)
            .child(div().whitespace_nowrap().child(title)),
    )
}
