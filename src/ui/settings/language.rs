//! Language selection and persistence.

use super::parts;
use crate::{
    api,
    app::{App, Topic},
    i18n::{Language, Locale},
    ui,
};
use gpui_kit::{
    AppContext, Context, Entity, IntoElement, ParentElement, Render, SharedString, Styled,
    Subscription, Task, Window,
    component::{
        IndexPath,
        select::{Select, SelectEvent, SelectItem, SelectState},
    },
};

#[derive(Clone)]
struct Choice {
    label: SharedString,
    value: Language,
}
impl SelectItem for Choice {
    type Value = Language;
    fn title(&self) -> SharedString {
        self.label.clone()
    }
    fn value(&self) -> &Language {
        &self.value
    }
}
fn choices() -> Vec<Choice> {
    [
        (t!("settings.general.system"), Language::System),
        (t!("settings.general.english"), Language::Only(Locale::En)),
        (t!("settings.general.chinese"), Language::Only(Locale::ZhCn)),
        (t!("settings.general.japanese"), Language::Only(Locale::Ja)),
    ]
    .into_iter()
    .map(|(label, value)| Choice { label: label.into(), value })
    .collect()
}
fn index(language: Language) -> usize {
    match language {
        Language::System => 0,
        Language::Only(Locale::En) => 1,
        Language::Only(Locale::ZhCn) => 2,
        Language::Only(Locale::Ja) => 3,
    }
}

pub struct LanguageRow {
    app: App,
    select: Entity<SelectState<Vec<Choice>>>,
    shown_language: Language,
    shown_locale: Locale,
    error: Option<String>,
    _observers: Vec<Task<()>>,
    _subscription: Subscription,
}
impl LanguageRow {
    pub fn new(app: App, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let language = api::get_settings(&app).language;
        let select = cx.new(|cx| {
            SelectState::new(choices(), Some(IndexPath::new(index(language))), window, cx)
        });
        let subscription =
            cx.subscribe_in(&select, window, |this, _, event: &SelectEvent<Vec<Choice>>, _, cx| {
                if let SelectEvent::Confirm(Some(language)) = event {
                    this.error =
                        parts::update_settings(&this.app, |settings| settings.language = *language)
                            .err();
                    // Force synchronization on failure as the picker already selected the rejected value.
                    this.shown_language = *language;
                    cx.notify();
                }
            });
        let observers = vec![
            ui::observe(&app, Topic::Locale, cx, |_, _| {}),
            ui::observe(&app, Topic::Settings, cx, |_, _| {}),
        ];
        Self {
            app,
            select,
            shown_language: language,
            shown_locale: api::get_locale(),
            error: None,
            _observers: observers,
            _subscription: subscription,
        }
    }
}
impl Render for LanguageRow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let language = api::get_settings(&self.app).language;
        let locale = api::get_locale();
        if locale != self.shown_locale || language != self.shown_language {
            self.select.update(cx, |select, cx| {
                select.set_items(choices(), window, cx);
                select.set_selected_index(Some(IndexPath::new(index(language))), window, cx);
            });
            self.shown_locale = locale;
            self.shown_language = language;
        }
        let label = t!("settings.general.language");
        let mut row = gpui_kit::div().flex().flex_col().child(parts::picker_row(
            label,
            Select::new(&self.select)
                .id("language-select")
                .accessibility_label(label)
                .w(gpui_kit::px(170.)),
        ));
        if let Some(error) = &self.error {
            row = row.child(parts::problem("language-error", error.clone()));
        }
        row
    }
}
