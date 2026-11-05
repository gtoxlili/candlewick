//! The General group's rows and the settings footer.

use super::parts;
use crate::{
    agent::AgentStatus,
    api,
    app::{App, Topic},
    i18n::{self, Locale},
    model::Settings,
    ui::{self, theme::Palette},
    update::{UpdateState, UpdateView},
};
use gpui_kit::{
    AnyElement, Context, IntoElement, ParentElement, Render, SharedString, Styled, Task, Window,
    component::{
        Sizable as _, button::Button, button::ButtonVariants as _, spinner::Spinner, switch::Switch,
    },
    div, px,
};

/// The General group's rows under the language: opening at login, automatic
/// updates, and agent access with the line about who can read the app's
/// data.
pub struct AgentNote {
    app: App,
    /// The login item as the system reports it, or as just asked for while
    /// the call that changes it runs.
    login_item: bool,
    /// A login item call is in flight; the switch waits for it.
    login_busy: bool,
    login_task: Option<Task<()>>,
    /// What a save or the login item call said, as the group shows it.
    error: Option<String>,
    /// The status the offscreen renderer shows instead of the app's.
    #[cfg(all(target_os = "macos", feature = "test-support"))]
    status: Option<AgentStatus>,
    _observers: Vec<Task<()>>,
}

impl AgentNote {
    pub fn new(app: App, _: &mut Window, cx: &mut Context<Self>) -> Self {
        let login_item = api::get_login_item();
        let observers = vec![
            ui::observe(&app, Topic::Locale, cx, |_, _| {}),
            // Applying agent access ends with this notification.
            ui::observe(&app, Topic::Settings, cx, |_, _| {}),
        ];
        Self {
            app,
            login_item,
            login_busy: false,
            login_task: None,
            error: None,
            #[cfg(all(target_os = "macos", feature = "test-support"))]
            status: None,
            _observers: observers,
        }
    }

    /// Registers or removes the login item off the UI thread, then adopts
    /// the state the system reports back.
    fn toggle_login_item(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.login_item = enabled;
        self.login_busy = true;
        self.error = None;
        let app = self.app.clone();
        let job = ui::run(&app, async move { api::set_login_item(enabled) });
        self.login_task = Some(cx.spawn(async move |view, cx| {
            let result = job.await;
            let _ = view.update(cx, |this, cx| {
                this.login_busy = false;
                match result {
                    Ok(Ok(enabled)) => this.login_item = enabled,
                    Ok(Err(error)) => {
                        this.login_item = api::get_login_item();
                        this.error = Some(error.to_string());
                    }
                    Err(error) => this.error = Some(error.to_string()),
                }
                cx.notify();
            });
        }));
    }

    fn set(&mut self, change: impl FnOnce(&mut Settings), cx: &mut Context<Self>) {
        self.error = parts::update_settings(&self.app, change).err();
        cx.notify();
    }

    /// Whether agents have the skill, and who has it.
    fn status(&self) -> AgentStatus {
        #[cfg(all(target_os = "macos", feature = "test-support"))]
        if let Some(status) = &self.status {
            return status.clone();
        }
        api::get_agent(&self.app)
    }
}

impl Render for AgentNote {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let settings = api::get_settings(&self.app);
        let p = Palette::current(&self.app, cx);
        let this = cx.entity().downgrade();

        let login = {
            let this = this.clone();
            parts::switch_row(
                "general-login-item",
                t!("settings.general.loginItem"),
                self.login_item,
                self.login_busy,
                move |enabled, _, cx| {
                    let _ = this.update(cx, |this, cx| this.toggle_login_item(*enabled, cx));
                },
            )
        };
        let auto_update = {
            let this = this.clone();
            parts::switch_row(
                "general-auto-update",
                t!("settings.general.autoUpdate"),
                settings.auto_update,
                false,
                move |enabled, _, cx| {
                    let _ = this.update(cx, |this, cx| {
                        this.set(|settings| settings.auto_update = *enabled, cx);
                    });
                },
            )
        };
        // The line under the switch spans the row, where the longest
        // translation still fits.
        let status = self.status();
        let agents = (settings.agent_access && status.on).then(|| agent_note(&status, &p));
        let access_label: SharedString = t!("settings.general.agentAccess").into();
        let access = {
            let this = this.clone();
            let label = access_label.clone();
            let switch = Switch::new("general-agent-access")
                .accessibility_label(label.clone())
                .checked(settings.agent_access)
                .on_change(move |enabled, _, cx| {
                    let _ = this.update(cx, |this, cx| {
                        this.set(|settings| settings.agent_access = *enabled, cx);
                    });
                });
            div()
                .flex()
                .flex_col()
                .gap_1()
                .px_3()
                .py_2()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_4()
                        .min_h(px(24.))
                        .child(div().min_w_0().whitespace_nowrap().overflow_hidden().child(label))
                        .child(switch),
                )
                .children(agents)
        };

        let divider = |row: AnyElement| {
            div().border_t_1().border_color(p.border).child(row).into_any_element()
        };
        let mut rows = div()
            .flex()
            .flex_col()
            .child(divider(login.into_any_element()))
            .child(divider(auto_update.into_any_element()))
            .child(divider(access.into_any_element()));
        if let Some(error) = &self.error {
            rows = rows
                .child(div().px_3().pb_2().child(parts::problem("general-problem", error.clone())));
        }
        rows
    }
}

/// Who can read the app's data now, as a line under the agent access switch.
fn agent_note(status: &AgentStatus, p: &Palette) -> AnyElement {
    let (text, color) = match &status.error {
        Some(error) => (error.clone(), p.red),
        None if status.agents.is_empty() => (t!("settings.general.noAgents").to_owned(), p.muted),
        None => (
            t!(
                "settings.general.agentsCanRead",
                agents = agent_list(&status.agents, i18n::current())
            ),
            p.muted,
        ),
    };
    div()
        .text_xs()
        .whitespace_nowrap()
        .overflow_hidden()
        .text_color(color)
        .child(text)
        .into_any_element()
}

/// The agents with the skill as one phrase: "Claude Code, Codex, and
/// OpenCode".
fn agent_list(agents: &[&'static str], locale: Locale) -> String {
    let Some((last, rest)) = agents.split_last() else {
        return String::new();
    };
    if rest.is_empty() {
        return (*last).to_owned();
    }
    match locale {
        // "A and B", "A, B, and C".
        Locale::En if rest.len() == 1 => format!("{} and {last}", rest[0]),
        Locale::En => format!("{}, and {last}", rest.join(", ")),
        Locale::ZhCn => format!("{}和{last}", rest.join("、")),
        Locale::Ja => format!("{}、{last}", rest.join("、")),
    }
}

/// The footer under the scroll area: the running version, the update state
/// as the button that acts on it, and the line that explains the state. The
/// line keeps its height whether or not it has something to say, so the
/// footer never moves.
pub struct AppFooter {
    app: App,
    /// The update view the offscreen renderer shows instead of the app's.
    #[cfg(all(target_os = "macos", feature = "test-support"))]
    preview: Option<UpdateView>,
    _observers: Vec<Task<()>>,
}

impl AppFooter {
    pub fn new(app: App, _: &mut Window, cx: &mut Context<Self>) -> Self {
        let observers = vec![
            ui::observe(&app, Topic::Update, cx, |_, _| {}),
            ui::observe(&app, Topic::Locale, cx, |_, _| {}),
        ];
        Self {
            app,
            #[cfg(all(target_os = "macos", feature = "test-support"))]
            preview: None,
            _observers: observers,
        }
    }

    fn view(&self) -> UpdateView {
        #[cfg(all(target_os = "macos", feature = "test-support"))]
        if let Some(preview) = &self.preview {
            return preview.clone();
        }
        api::get_update(&self.app)
    }
}

impl Render for AppFooter {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Palette::current(&self.app, cx);
        let view = self.view();
        let state = &view.state;
        let label = update_label(state);
        let note = update_note(state);
        let busy = matches!(
            state,
            UpdateState::Checking
                | UpdateState::Downloading { .. }
                | UpdateState::Restarting { .. }
        );
        let action: AnyElement = if matches!(state, UpdateState::Disabled) {
            div().text_color(p.muted).whitespace_nowrap().child(label).into_any_element()
        } else {
            let hint: Option<SharedString> = match state {
                UpdateState::Idle { checked: true } => {
                    Some(t!("settings.update.checkAgain").into())
                }
                UpdateState::Idle { .. }
                | UpdateState::Unreachable
                | UpdateState::Failed { .. } => Some(t!("settings.update.checkForNew").into()),
                _ => None,
            };
            let mut button =
                Button::new("update-action").small().ghost().label(label).loading(busy);
            if busy {
                button = button.icon(Spinner::new());
            }
            if matches!(state, UpdateState::Ready { .. }) {
                button = button.text_color(p.glow);
            }
            if let Some(hint) = hint {
                button = button.tooltip(hint);
            }
            let app = self.app.clone();
            let state = state.clone();
            button.on_click(move |_, _, _| act(&app, &state)).into_any_element()
        };
        let version = ui::tabular(div()).text_color(p.muted).child(format!("v{}", view.current));
        div()
            .flex()
            .flex_col()
            .text_size(px(FOOTER_TEXT))
            .child(
                div()
                    .h(px(FOOTER_ROW))
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_4()
                    .child(
                        div()
                            .flex()
                            .items_baseline()
                            .gap_1()
                            .whitespace_nowrap()
                            .child("Candlewick")
                            .child(version),
                    )
                    .child(div().flex_shrink_0().child(action)),
            )
            .children(note.map(|note| {
                div().pb_2().whitespace_nowrap().overflow_hidden().text_color(p.muted).child(note)
            }))
    }
}

/// The footer's one action: a ready update restarts into it, anything else
/// checks. The outcome arrives as `Topic::Update`.
fn act(app: &App, state: &UpdateState) {
    if matches!(state, UpdateState::Ready { .. }) {
        api::restart_to_update(app);
    } else {
        api::check_update(app);
    }
}

/// The footer's single row, a line under which the border makes 32px; the
/// small type of a status line (12px on Windows, as there).
const FOOTER_ROW: f32 = 31.;
const FOOTER_TEXT: f32 = if cfg!(target_os = "windows") { 12. } else { 11. };

/// What the footer's button says for each update state.
fn update_label(state: &UpdateState) -> SharedString {
    match state {
        UpdateState::Disabled => t!("settings.update.unavailable").into(),
        UpdateState::Idle { checked } if *checked => t!("settings.update.upToDate").into(),
        UpdateState::Idle { .. } => t!("settings.update.check").into(),
        UpdateState::Checking => t!("settings.update.checking").into(),
        UpdateState::Downloading { .. } => t!("settings.update.downloading").into(),
        UpdateState::Ready { .. } => t!("settings.update.restartToUpdate").into(),
        UpdateState::Restarting { .. } => t!("settings.update.restarting").into(),
        UpdateState::Unreachable => t!("settings.update.checkFailed").into(),
        UpdateState::Failed { .. } => t!("settings.update.updateFailed").into(),
    }
}

/// What the footer says about the update state, under the version line, for
/// the states whose button cannot say it all: which release is on its way,
/// why a check failed, or that this copy never updates itself. Up to date,
/// checking and restarting are their button's own words, and say nothing
/// twice.
fn update_note(state: &UpdateState) -> Option<String> {
    match state {
        UpdateState::Disabled => Some(t!("settings.update.noteDisabled").to_owned()),
        UpdateState::Idle { .. } | UpdateState::Checking | UpdateState::Restarting { .. } => None,
        UpdateState::Unreachable => Some(t!("settings.update.noteUnreachable").to_owned()),
        UpdateState::Downloading { version } => {
            Some(t!("settings.update.noteDownloading", version = version))
        }
        UpdateState::Ready { version } => Some(t!("settings.update.noteReady", version = version)),
        UpdateState::Failed { version } => {
            Some(t!("settings.update.noteFailed", version = version))
        }
    }
}

/// Every state the footer draws, with the name its sample images use.
#[cfg(any(test, all(target_os = "macos", feature = "test-support")))]
fn update_states() -> Vec<(&'static str, UpdateState)> {
    use UpdateState::*;
    vec![
        ("idle", Idle { checked: false }),
        ("up-to-date", Idle { checked: true }),
        ("checking", Checking),
        ("unreachable", Unreachable),
        ("downloading", Downloading { version: "1.2.0".into() }),
        ("ready", Ready { version: "1.2.0".into() }),
        ("restarting", Restarting { version: "1.2.0".into() }),
        ("failed", Failed { version: "1.2.0".into() }),
        ("disabled", Disabled),
    ]
}

/// Images of the settings window with every update state, and of the
/// General group with the agent note, for the offscreen renderer. `render`
/// draws and saves one frame.
#[cfg(all(target_os = "macos", feature = "test-support"))]
pub fn samples(
    cx: &mut gpui_kit::HeadlessAppContext,
    app: &App,
    render: &dyn Fn(&mut gpui_kit::HeadlessAppContext, gpui_kit::AnyWindowHandle, &str),
) {
    use gpui_kit::{AppContext, Bounds, Entity, WindowBounds, WindowOptions, size};
    use std::{cell::RefCell, rc::Rc};

    let original = crate::i18n::current();

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
    struct GeneralGroup {
        app: App,
        note: Entity<AgentNote>,
    }
    impl Render for GeneralGroup {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let p = Palette::current(&self.app, cx);
            div().size_full().bg(p.background).p_5().text_color(p.foreground).child(
                parts::section(t!("settings.general.title"), &self.app, cx)
                    .child(parts::group(&self.app, cx).child(self.note.clone())),
            )
        }
    }

    let settings_window = |cx: &mut gpui_kit::HeadlessAppContext| {
        let app = app.clone();
        let created: Rc<RefCell<Option<Entity<super::SettingsView>>>> = Rc::new(RefCell::new(None));
        let slot = created.clone();
        let window = cx.update(|cx| {
            let mut options =
                crate::window::window_options(cx, "settings", t!("window.settings").to_owned());
            options.focus = false;
            options.show = false;
            gpui_kit::open_window(options, cx, |window, cx| {
                let view = cx.new(|cx| super::SettingsView::new(app.clone(), window, cx));
                *slot.borrow_mut() = Some(view.clone());
                cx.new(|_| SettingsWindow { app: app.clone(), view })
            })
            .unwrap()
            .0
        });
        (window, created.borrow_mut().take().unwrap())
    };
    let row_window = |cx: &mut gpui_kit::HeadlessAppContext| {
        let app = app.clone();
        let created: Rc<RefCell<Option<Entity<AgentNote>>>> = Rc::new(RefCell::new(None));
        let slot = created.clone();
        let window = cx.update(|cx| {
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: Default::default(),
                    size: size(px(460.), px(300.)),
                })),
                focus: false,
                show: false,
                ..Default::default()
            };
            gpui_kit::open_window(options, cx, |window, cx| {
                let note = cx.new(|cx| AgentNote::new(app.clone(), window, cx));
                *slot.borrow_mut() = Some(note.clone());
                cx.new(|_| GeneralGroup { app: app.clone(), note })
            })
            .unwrap()
            .0
        });
        (window, created.borrow_mut().take().unwrap())
    };
    let show =
        |cx: &mut gpui_kit::HeadlessAppContext, window: gpui_kit::AnyWindowHandle, name: &str| {
            for (theme, dark) in [("light", false), ("dark", true)] {
                cx.update(|cx| ui::theme::apply(app, dark, cx));
                render(cx, window, &format!("{name}-{theme}"));
            }
        };
    let locale = |cx: &mut gpui_kit::HeadlessAppContext, locale: Locale| {
        cx.update(|_| crate::i18n::set(locale));
        app.notify(Topic::Locale);
        cx.run_until_parked();
    };

    let (window, view) = settings_window(cx);
    let version = env!("CARGO_PKG_VERSION").to_owned();
    for (tag, tag_locale) in [("en", Locale::En), ("ja", Locale::Ja)] {
        locale(cx, tag_locale);
        for (name, state) in update_states() {
            view.update(cx, |view, cx| {
                view.footer.update(cx, |footer, cx| {
                    footer.preview =
                        Some(UpdateView { current: version.clone(), state: state.clone() });
                    cx.notify();
                });
            });
            show(cx, window, &format!("settings-update-{name}-{tag}"));
        }
    }
    view.update(cx, |view, cx| {
        view.footer.update(cx, |footer, cx| {
            footer.preview = None;
            cx.notify();
        });
    });

    // The agent note says who has the skill; its longest line fills the row.
    app.shared().model().settings.agent_access = true;
    let (window, note) = row_window(cx);
    let installed =
        AgentStatus { on: true, agents: vec!["Claude Code", "Codex", "OpenCode"], error: None };
    let failed = AgentStatus {
        on: true,
        agents: Vec::new(),
        error: Some("The operation couldn't be completed. (SMAppService error 1.)".to_owned()),
    };
    let unsaved = "Couldn't save settings: Permission denied (os error 13)".to_owned();
    for (tag, tag_locale) in [("en", Locale::En), ("ja", Locale::Ja)] {
        locale(cx, tag_locale);
        for (name, status, error) in [
            ("agents", &installed, None),
            ("failed", &failed, None),
            ("problem", &installed, Some(unsaved.clone())),
        ] {
            note.update(cx, |note, cx| {
                note.status = Some(status.clone());
                note.error = error.clone();
                cx.notify();
            });
            show(cx, window, &format!("settings-general-{name}-{tag}"));
        }
    }
    app.shared().model().settings.agent_access = false;
    cx.update(|_| crate::i18n::set(original));
    app.notify(Topic::Locale);
    cx.run_until_parked();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_state_has_a_label() {
        for (name, state) in update_states() {
            assert!(!update_label(&state).is_empty(), "{name}");
        }
    }

    // Each state names itself, so a mistaken arm shows up here.
    #[test]
    fn every_state_has_its_own_label() {
        let labels: Vec<_> =
            update_states().into_iter().map(|(name, state)| (name, update_label(&state))).collect();
        for (index, (name, label)) in labels.iter().enumerate() {
            for (other, other_label) in &labels[index + 1..] {
                assert_ne!(label, other_label, "{name} and {other} read the same");
            }
        }
    }

    // Only a copy that never checked has nothing to add under the version.
    #[test]
    fn every_other_state_has_a_note() {
        for (name, state) in update_states() {
            let quiet = matches!(
                state,
                UpdateState::Idle { .. } | UpdateState::Checking | UpdateState::Restarting { .. }
            );
            assert_eq!(update_note(&state).is_none(), quiet, "{name}");
        }
    }

    // Every state that names a release carries the version into its line.
    #[test]
    fn notes_about_a_release_name_it() {
        for (name, state) in update_states() {
            let Some(note) = update_note(&state) else {
                continue;
            };
            match &state {
                UpdateState::Downloading { .. }
                | UpdateState::Ready { .. }
                | UpdateState::Failed { .. } => {
                    assert!(note.contains("1.2.0"), "{name}: {note}");
                }
                _ => {}
            }
        }
    }

    #[test]
    fn agent_names_read_as_a_list_in_each_language() {
        let three = ["Claude Code", "Codex", "OpenCode"];
        assert_eq!(agent_list(&three, Locale::En), "Claude Code, Codex, and OpenCode");
        assert_eq!(agent_list(&three[..2], Locale::En), "Claude Code and Codex");
        assert_eq!(agent_list(&three[..1], Locale::En), "Claude Code");
        assert_eq!(agent_list(&three, Locale::ZhCn), "Claude Code、Codex和OpenCode");
        assert_eq!(agent_list(&three[..2], Locale::ZhCn), "Claude Code和Codex");
        assert_eq!(agent_list(&three, Locale::Ja), "Claude Code、Codex、OpenCode");
        assert_eq!(agent_list(&[], Locale::En), "");
    }
}
