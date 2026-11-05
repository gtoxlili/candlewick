use std::{collections::HashMap, io};

use gpui_kit::{
    AnyWindowHandle, AppContext, Global, WindowBackgroundAppearance, WindowBounds, WindowOptions,
    px, size,
};

use crate::{
    app::{App, Topic},
    market::{ProviderId, ths},
    model::Instrument,
    platform, ui,
};

const SETTINGS: &str = "settings";
const CHART: &str = "chart";
const HOLDINGS: &str = "holdings";

#[derive(Default)]
struct Windows(HashMap<&'static str, AnyWindowHandle>);
impl Global for Windows {}

pub(crate) fn init(app: &App, cx: &mut gpui_kit::App) {
    cx.set_global(Windows::default());
    let app = app.clone();
    cx.on_window_closed(move |cx, id| {
        let label = cx
            .global::<Windows>()
            .0
            .iter()
            .find_map(|(label, window)| (window.window_id() == id).then_some(*label));
        if let Some(label) = label {
            cx.global_mut::<Windows>().0.remove(label);
            app.set_window_count(cx.global::<Windows>().0.len());
            on_destroyed(&app, label);
            if cx.global::<Windows>().0.is_empty() {
                platform::window::did_close_all(cx);
            }
        }
    })
    .detach();
}

pub fn open_settings(app: &App) -> io::Result<()> {
    enqueue_open(app, SETTINGS, t!("window.settings").to_owned())
}

pub fn open_holdings(app: &App) -> io::Result<()> {
    enqueue_open(app, HOLDINGS, t!("window.holdings").to_owned())
}

pub fn open_chart(app: &App, id: &str) -> io::Result<()> {
    let title = {
        let mut model = app.shared().model();
        let Some(instrument) = model.instrument(id) else { return Ok(()) };
        let title = chart_title(instrument);
        model.chart = Some(id.to_owned());
        title
    };
    let handle = app.clone();
    dispatch(app, move |cx| {
        let existed = cx.global::<Windows>().0.contains_key(CHART);
        open_on_main(&handle, cx, CHART, title);
        if existed {
            handle.notify(Topic::ChartInstrument);
        }
    })
}

fn chart_title(instrument: &Instrument) -> String {
    t!("window.chart", pair = instrument.pair_label())
}

pub fn relabel(app: &App) {
    let handle = app.clone();
    app.on_gpui(move |cx| {
        let chart = {
            let model = handle.shared().model();
            model.chart.as_deref().and_then(|id| model.instrument(id)).map(chart_title)
        };
        for (label, title) in [
            (SETTINGS, Some(t!("window.settings").to_owned())),
            (CHART, chart),
            (HOLDINGS, Some(t!("window.holdings").to_owned())),
        ] {
            if let Some((window, title)) = cx.global::<Windows>().0.get(label).copied().zip(title) {
                let _ = window.update(cx, |_, window, _| window.set_window_title(&title));
            }
        }
        handle.notify(Topic::Locale);
        platform::window::relabel(cx);
    });
}

pub fn reopen(app: &App) -> io::Result<()> {
    let handle = app.clone();
    dispatch(app, move |cx| {
        let open: Vec<_> = [CHART, HOLDINGS, SETTINGS]
            .into_iter()
            .filter_map(|label| cx.global::<Windows>().0.get(label).copied())
            .collect();
        if open.is_empty() {
            open_on_main(&handle, cx, SETTINGS, t!("window.settings").to_owned());
        } else if !platform::window::ACTIVATION_RAISES_WINDOWS
            || !open.iter().any(|window| {
                window.update(cx, |_, w, _| platform::window::is_unminimized(w)).unwrap_or(false)
            })
        {
            raise(cx, open[0]);
        }
    })
}

pub fn sync_chart(app: &App) {
    let handle = app.clone();
    app.on_gpui(move |cx| {
        let Some(window) = cx.global::<Windows>().0.get(CHART).copied() else { return };
        let replacement = {
            let mut model = handle.shared().model();
            if model.chart.as_deref().is_some_and(|id| model.instrument(id).is_some()) {
                return;
            }
            let next = model
                .settings
                .watchlist
                .first()
                .map(|instrument| (instrument.id(), chart_title(instrument)));
            if let Some((id, _)) = &next {
                model.chart = Some(id.clone());
            }
            next
        };
        if let Some((_, title)) = replacement {
            let _ = window.update(cx, |_, window, _| window.set_window_title(&title));
            handle.notify(Topic::ChartInstrument);
        } else {
            let _ = window.update(cx, |_, window, _| window.remove_window());
        }
    });
}

fn dispatch(app: &App, f: impl FnOnce(&mut gpui_kit::App) + Send + 'static) -> io::Result<()> {
    if app.on_gpui(f) { Ok(()) } else { Err(io::Error::other("main thread unavailable")) }
}

fn enqueue_open(app: &App, label: &'static str, title: String) -> io::Result<()> {
    let handle = app.clone();
    dispatch(app, move |cx| open_on_main(&handle, cx, label, title))
}

fn open_on_main(app: &App, cx: &mut gpui_kit::App, label: &'static str, title: String) {
    platform::window::will_show(app, cx, label);
    if let Some(window) = cx.global::<Windows>().0.get(label).copied() {
        let _ = window.update(cx, |_, window, _| window.set_window_title(&title));
        raise(cx, window);
        return;
    }
    let options = window_options(cx, label, title);
    let handle = app.clone();
    let result = match label {
        SETTINGS => gpui_kit::open_window(options, cx, |window, cx| {
            cx.new(|cx| ui::settings::SettingsView::new(handle, window, cx))
        })
        .map(|(w, _)| w),
        HOLDINGS => gpui_kit::open_window(options, cx, |window, cx| {
            cx.new(|cx| ui::holdings::HoldingsView::new(handle, window, cx))
        })
        .map(|(w, _)| w),
        CHART => gpui_kit::open_window(options, cx, |window, cx| {
            cx.new(|cx| ui::chart::ChartView::new(handle, window, cx))
        })
        .map(|(w, _)| w),
        _ => unreachable!(),
    };
    match result {
        Ok(window) => {
            cx.global_mut::<Windows>().0.insert(label, window);
            app.set_window_count(cx.global::<Windows>().0.len());
            if label == HOLDINGS {
                set_holdings_open(app, true);
            }
            raise(cx, window);
        }
        Err(error) => {
            log::error!("cannot open {label}: {error}");
            if cx.global::<Windows>().0.is_empty() {
                platform::window::did_close_all(cx);
            }
        }
    }
}

pub(crate) fn window_options(cx: &mut gpui_kit::App, label: &str, title: String) -> WindowOptions {
    let (width, height, min_width, min_height, resizable) = match label {
        SETTINGS => (460., 640., 460., 640., false),
        HOLDINGS => (980., 640., 800., 500., true),
        CHART => (980., 640., 760., 480., true),
        _ => unreachable!(),
    };
    WindowOptions {
        window_bounds: Some(WindowBounds::centered(size(px(width), px(height)), cx)),
        titlebar: Some(gpui_kit::TitlebarOptions {
            title: Some(title.into()),
            ..gpui_kit::component::TitleBar::title_bar_options()
        }),
        window_background: if cfg!(target_os = "windows") {
            WindowBackgroundAppearance::MicaBackdrop
        } else {
            WindowBackgroundAppearance::Blurred
        },
        window_min_size: Some(size(px(min_width), px(min_height))),
        is_resizable: resizable,
        ..gpui_kit::component::TitleBar::window_options()
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn raise_label(cx: &mut gpui_kit::App, label: &str) {
    if let Some(window) = cx.global::<Windows>().0.get(label).copied() {
        raise(cx, window);
    }
}

fn raise(cx: &mut gpui_kit::App, window: AnyWindowHandle) {
    let _ = window.update(cx, |_, window, _| platform::window::bring_to_front(window));
}

fn set_holdings_open(app: &App, open: bool) {
    app.shared().control.send_if_modified(|control| {
        let changed = control.holdings_open != open;
        control.holdings_open = open;
        changed
    });
}

pub fn on_destroyed(app: &App, label: &str) {
    match label {
        CHART => app.shared().streams.lock().unwrap_or_else(|e| e.into_inner()).clear(),
        SETTINGS => {
            for provider in ProviderId::ALL {
                provider.provider().drop_search_cache();
            }
            ths::cancel_sign_in();
        }
        HOLDINGS => set_holdings_open(app, false),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::{TestAppContext, VisualTestContext};

    #[test]
    fn windows_reuse_retarget_relabel_and_cleanup() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let cx = TestAppContext::single();
        cx.update(gpui_kit::init);
        let (shared, control) = crate::model::Shared::new(
            crate::model::Settings::default(),
            Default::default(),
            Default::default(),
            Default::default(),
        );
        let app = cx.update(|cx| {
            let app = App::new(cx, runtime.handle().clone(), shared);
            init(&app, cx);
            app
        });
        let initial_locale = crate::i18n::current();
        let ids: Vec<_> = app.shared().model().settings.watchlist.iter().map(|i| i.id()).collect();
        let background = app.clone();
        std::thread::spawn(move || open_settings(&background).unwrap()).join().unwrap();
        open_holdings(&app).unwrap();
        open_chart(&app, &ids[0]).unwrap();
        cx.run_until_parked();
        let handles = cx.read(|cx| cx.global::<Windows>().0.clone());
        assert_eq!(handles.len(), 3);
        assert!(control.borrow().holdings_open);
        assert!(app.windows_open());
        open_settings(&app).unwrap();
        open_holdings(&app).unwrap();
        let chart_changes = app.subscribe(Topic::ChartInstrument);
        open_chart(&app, &ids[1]).unwrap();
        cx.run_until_parked();
        assert!(chart_changes.has_changed().unwrap());
        assert_eq!(cx.read(|cx| cx.global::<Windows>().0.clone()), handles);
        assert_eq!(app.shared().model().chart.as_deref(), Some(ids[1].as_str()));
        let chart = handles[CHART];
        let mut chart_cx = VisualTestContext::from_window(chart, &cx);
        let title = {
            let model = app.shared().model();
            chart_title(model.instrument(&ids[1]).unwrap())
        };
        assert_eq!(chart_cx.window_title().as_deref(), Some(title.as_str()));

        let locale_changes = app.subscribe(Topic::Locale);
        crate::i18n::set(crate::i18n::Locale::Ja);
        relabel(&app);
        cx.run_until_parked();
        assert!(locale_changes.has_changed().unwrap());
        let mut settings_cx = VisualTestContext::from_window(handles[SETTINGS], &cx);
        assert_eq!(settings_cx.window_title().as_deref(), Some(t!("window.settings")));
        let title = {
            let model = app.shared().model();
            chart_title(model.instrument(&ids[1]).unwrap())
        };
        assert_eq!(chart_cx.window_title().as_deref(), Some(title.as_str()));
        crate::i18n::set(initial_locale);

        let (sender, mut receiver) = tokio::sync::oneshot::channel();
        app.shared().streams.lock().unwrap().insert(1, sender);
        app.shared().model().settings.watchlist.retain(|item| item.id() != ids[1]);
        sync_chart(&app);
        cx.run_until_parked();
        assert_eq!(app.shared().model().chart.as_deref(), Some(ids[0].as_str()));
        app.shared().model().settings.watchlist.clear();
        sync_chart(&app);
        cx.run_until_parked();
        assert!(matches!(
            receiver.try_recv(),
            Err(tokio::sync::oneshot::error::TryRecvError::Closed)
        ));
        assert!(!cx.read(|cx| cx.global::<Windows>().0.contains_key(CHART)));
        for label in [SETTINGS, HOLDINGS] {
            cx.update(|cx| {
                handles[label].update(cx, |_, window, _| window.remove_window()).unwrap()
            });
        }
        assert!(!control.borrow().holdings_open);
        assert!(!app.windows_open());
        assert!(cx.read(|cx| cx.global::<Windows>().0.is_empty()));
        reopen(&app).unwrap();
        cx.run_until_parked();
        assert_eq!(
            cx.read(|cx| cx.global::<Windows>().0.keys().copied().collect::<Vec<_>>()),
            [SETTINGS]
        );
    }
}
