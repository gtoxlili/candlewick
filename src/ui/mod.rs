//! The native windows: settings, chart and holdings.

use std::{future::Future, sync::Arc};

use gpui_kit::{Context, FontFeatures, Styled, Task, Window, WindowAppearance};

use crate::app::{App, Topic};

pub mod chart;
pub mod holdings;
pub mod settings;
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

/// Update the native title on locale changes. Hold the task in the view.
pub fn observe_title<V: 'static>(
    app: &App,
    window: &Window,
    cx: &mut Context<V>,
    mut title: impl FnMut(&mut V, &mut Context<V>) -> String + 'static,
) -> Task<()> {
    let mut receiver = app.subscribe(Topic::Locale);
    cx.spawn_in(window, async move |view, cx| {
        while receiver.changed().await.is_ok() {
            if view
                .update_in(cx, |view, window, cx| {
                    window.set_window_title(&title(view, cx));
                    cx.notify();
                })
                .is_err()
            {
                break;
            }
        }
    })
}

/// Run an API future on Tokio. Await it in `cx.spawn`, then use `view.update(cx, ...)` and `cx.notify()`.
pub fn run<F>(app: &App, future: F) -> tokio::task::JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    app.spawn(future)
}

/// Keep the system font while making digits equal width.
pub fn tabular<T: Styled>(element: T) -> T {
    element.font_features(FontFeatures(Arc::new(vec![("tnum".into(), 1)])))
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
