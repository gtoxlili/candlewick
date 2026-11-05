//! What the menu bar or taskbar shows, and which colors prices use.

use super::parts;
use crate::{
    api,
    app::{App, Topic},
    model::{ColorScheme, Settings},
    ui::{self, theme::Palette},
};
use gpui_kit::{Context, IntoElement, ParentElement, Render, Styled, Task, Window, div};

pub struct DisplaySection {
    app: App,
    /// Why a change was refused, as the last save reported it.
    error: Option<String>,
    _observers: Vec<Task<()>>,
}

impl DisplaySection {
    pub fn new(app: App, _: &mut Window, cx: &mut Context<Self>) -> Self {
        let observers = vec![
            ui::observe(&app, Topic::Locale, cx, |_, _| {}),
            ui::observe(&app, Topic::Settings, cx, |_, _| {}),
        ];
        Self { app, error: None, _observers: observers }
    }

    fn set(&mut self, change: impl FnOnce(&mut Settings), cx: &mut Context<Self>) {
        self.error = parts::update_settings(&self.app, change).err();
        cx.notify();
    }
}

/// The scheme a "Red for up" switch turned on or off selects.
fn color_scheme(red_up: bool) -> ColorScheme {
    if red_up { ColorScheme::RedUp } else { ColorScheme::GreenUp }
}

impl Render for DisplaySection {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let settings = api::get_settings(&self.app);
        let p = Palette::current(&self.app, cx);
        let this = cx.entity().downgrade();

        let symbol = {
            let this = this.clone();
            parts::switch_row(
                "display-show-symbol",
                t!("settings.display.showSymbol"),
                settings.show_symbol,
                false,
                move |on, _, cx| {
                    let _ = this
                        .update(cx, |this, cx| this.set(|settings| settings.show_symbol = *on, cx));
                },
            )
        };
        let change = {
            let this = this.clone();
            parts::switch_row(
                "display-show-change",
                t!("settings.display.showChange"),
                settings.show_change,
                false,
                move |on, _, cx| {
                    let _ = this
                        .update(cx, |this, cx| this.set(|settings| settings.show_change = *on, cx));
                },
            )
        };
        let colors = {
            let this = this.clone();
            parts::switch_row(
                "display-red-up",
                t!("settings.display.redUp"),
                settings.color_scheme == ColorScheme::RedUp,
                false,
                move |on, _, cx| {
                    let _ = this.update(cx, |this, cx| {
                        this.set(|settings| settings.color_scheme = color_scheme(*on), cx);
                    });
                },
            )
        };

        let divider = |row| div().border_t_1().border_color(p.border).child(row);
        let mut group =
            parts::group(&self.app, cx).child(symbol).child(divider(change)).child(divider(colors));
        if let Some(error) = &self.error {
            group = group
                .child(div().px_3().pb_2().child(parts::problem("display-problem", error.clone())));
        }
        parts::section(t!("settings.display.title"), &self.app, cx).child(group)
    }
}

/// Images of the Display section alone and inside the settings window, for
/// the offscreen renderer: two setting combinations and a refused save, in
/// both themes and both checked languages. `render` draws and saves one
/// frame.
#[cfg(all(target_os = "macos", feature = "test-support"))]
pub fn samples(
    cx: &mut gpui_kit::HeadlessAppContext,
    app: &App,
    render: &dyn Fn(&mut gpui_kit::HeadlessAppContext, gpui_kit::AnyWindowHandle, &str),
) {
    use crate::i18n::Locale;
    use gpui_kit::{
        AppContext, Bounds, Entity, IntoElement, ParentElement, Render, Styled, Window,
        WindowBounds, WindowOptions, div, px, size,
    };

    struct Stage {
        app: App,
        section: Entity<DisplaySection>,
    }
    impl Render for Stage {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let p = Palette::current(&self.app, cx);
            div()
                .size_full()
                .bg(p.background)
                .p_5()
                .text_color(p.foreground)
                .child(self.section.clone())
        }
    }
    struct SettingsWindow {
        app: App,
        view: Entity<super::SettingsView>,
    }
    impl Render for SettingsWindow {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            div()
                .size_full()
                .bg(Palette::current(&self.app, cx).background)
                .child(self.view.clone())
        }
    }

    let stage_window = |cx: &mut gpui_kit::HeadlessAppContext| {
        let handle = app.clone();
        let mut section = None;
        let window = cx.update(|cx| {
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: Default::default(),
                    size: size(px(460.), px(640.)),
                })),
                focus: false,
                show: false,
                ..Default::default()
            };
            gpui_kit::open_window(options, cx, |window, cx| {
                let view = cx.new(|cx| DisplaySection::new(handle.clone(), window, cx));
                section = Some(view.clone());
                cx.new(|_| Stage { app: handle.clone(), section: view })
            })
            .unwrap()
            .0
        });
        (window, section.unwrap())
    };
    // Taller than the real window only so the section, last but one in the
    // scroll area, is in the picture at the real 460-pixel width.
    let settings_window = |cx: &mut gpui_kit::HeadlessAppContext| {
        let handle = app.clone();
        let mut section = None;
        let window = cx.update(|cx| {
            let mut options =
                crate::window::window_options(cx, "settings", t!("window.settings").to_owned());
            options.window_bounds = Some(WindowBounds::Windowed(Bounds {
                origin: Default::default(),
                size: size(px(460.), px(1800.)),
            }));
            options.focus = false;
            options.show = false;
            let (window, _) = gpui_kit::open_window(options, cx, |window, cx| {
                let view = cx.new(|cx| super::SettingsView::new(handle.clone(), window, cx));
                section = Some(view.read(cx).display());
                cx.new(|_| SettingsWindow { app: handle.clone(), view })
            })
            .unwrap();
            window
        });
        (window, section.unwrap())
    };

    let original = api::get_settings(app);
    let (stage, stage_section) = stage_window(cx);
    let (window, window_section) = settings_window(cx);
    let sections = [&stage_section, &window_section];
    let set = |cx: &mut gpui_kit::HeadlessAppContext,
               symbol: bool,
               change: bool,
               scheme: ColorScheme,
               problem: Option<&str>| {
        {
            let mut model = app.shared().model();
            model.settings.show_symbol = symbol;
            model.settings.show_change = change;
            model.settings.color_scheme = scheme;
        }
        app.notify(Topic::Settings);
        cx.run_until_parked();
        for section in sections {
            section.update(cx, |section, cx| {
                section.error = problem.map(str::to_owned);
                cx.notify();
            });
        }
        cx.run_until_parked();
    };

    let states = [
        ("default", true, false, ColorScheme::GreenUp, None),
        ("reversed", false, true, ColorScheme::RedUp, None),
        (
            "problem",
            true,
            true,
            ColorScheme::RedUp,
            Some("Couldn't save settings: Permission denied (os error 13)"),
        ),
    ];
    for (tag, locale) in [("en", Locale::En), ("ja", Locale::Ja)] {
        cx.update(|_| crate::i18n::set(locale));
        app.notify(Topic::Locale);
        cx.run_until_parked();
        for (name, symbol, change, scheme, problem) in states {
            set(cx, symbol, change, scheme, problem);
            for (theme, dark) in [("light", false), ("dark", true)] {
                cx.update(|cx| ui::theme::apply(app, dark, cx));
                render(cx, stage, &format!("display-{name}-{theme}-{tag}"));
                render(cx, window, &format!("settings-display-{name}-{theme}-{tag}"));
            }
        }
    }
    {
        let mut model = app.shared().model();
        model.settings = original;
    }
    app.notify(Topic::Settings);
    cx.update(|_| crate::i18n::set(Locale::En));
    app.notify(Topic::Locale);
    cx.run_until_parked();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_switch_selects_the_scheme_it_names() {
        assert_eq!(color_scheme(true), ColorScheme::RedUp);
        assert_eq!(color_scheme(false), ColorScheme::GreenUp);
    }
}
