//! Shared settings layout and controls.
// Removed when a window uses this module.
#![cfg_attr(not(test), allow(dead_code))]

use crate::{api, app::App, model::Settings, ui::theme::Palette};
use gpui_kit::{Div, IntoElement, ParentElement, SharedString, Styled, div, px};

/// A labeled settings section.
pub fn section(title: impl Into<SharedString>, app: &App, cx: &gpui_kit::App) -> Div {
    let p = Palette::current(app, cx);
    div().flex().flex_col().gap_2().child(
        div()
            .px_1()
            .text_xs()
            .text_color(p.muted)
            .whitespace_nowrap()
            .overflow_hidden()
            .child(title.into()),
    )
}

/// A bordered group of settings rows.
pub fn group(app: &App, cx: &gpui_kit::App) -> Div {
    let p = Palette::current(app, cx);
    div().flex().flex_col().rounded_xl().border_1().border_color(p.border).bg(p.panel)
}

/// A placeholder for an unfinished section.
pub fn placeholder(
    title: impl Into<SharedString>,
    app: &App,
    cx: &gpui_kit::App,
) -> impl IntoElement {
    section(title, app, cx).child(
        group(app, cx)
            .child(div().px_3().py_2().text_color(Palette::current(app, cx).muted).child("…")),
    )
}

/// Apply a settings change and return the saved settings or its displayable error.
pub fn update_settings(app: &App, change: impl FnOnce(&mut Settings)) -> Result<Settings, String> {
    let mut settings = api::get_settings(app);
    change(&mut settings);
    api::save_settings(app, settings).map_err(|error| error.to_string())
}

/// An error message in a Kit alert.
pub fn problem(
    id: impl Into<gpui_kit::ElementId>,
    message: impl Into<SharedString>,
) -> impl IntoElement {
    gpui_kit::component::alert::Alert::error(id, message.into())
}

/// A row containing a label and a switch.
pub fn switch_row(
    id: impl Into<gpui_kit::ElementId>,
    label: impl Into<SharedString>,
    checked: bool,
    disabled: bool,
    on_change: impl Fn(&bool, &mut gpui_kit::Window, &mut gpui_kit::App) + 'static,
) -> Div {
    use gpui_kit::component::{Disableable, switch::Switch};
    let label = label.into();
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap_4()
        .px_3()
        .py_2()
        .min_h(px(40.))
        .child(div().min_w_0().overflow_hidden().whitespace_nowrap().child(label.clone()))
        .child(
            Switch::new(id)
                .accessibility_label(label)
                .checked(checked)
                .disabled(disabled)
                .on_change(on_change),
        )
}

/// A row containing a label and a picker.
pub fn picker_row(label: impl Into<SharedString>, picker: impl IntoElement) -> Div {
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap_4()
        .px_3()
        .py_2()
        .min_h(px(40.))
        .child(div().min_w_0().overflow_hidden().whitespace_nowrap().child(label.into()))
        .child(picker)
}

/// One labeled credential input, with its masking configured by the owner.
pub struct CredentialInput {
    pub label: SharedString,
    pub state: gpui_kit::Entity<gpui_kit::component::input::InputState>,
}

/// A controlled credential editor; the owner supplies inputs and save/remove actions.
pub struct CredentialField {
    pub id: &'static str,
    pub label: SharedString,
    pub saved: Option<SharedString>,
    pub editing: bool,
    pub busy: bool,
    pub inputs: Vec<CredentialInput>,
    pub error: Option<SharedString>,
    pub on_edit: Box<dyn Fn(&mut gpui_kit::Window, &mut gpui_kit::App)>,
    pub on_cancel: Box<dyn Fn(&mut gpui_kit::Window, &mut gpui_kit::App)>,
    pub on_save: Box<dyn Fn(Vec<String>, &mut gpui_kit::Window, &mut gpui_kit::App)>,
    pub on_remove: Box<dyn Fn(&mut gpui_kit::Window, &mut gpui_kit::App)>,
}

impl CredentialField {
    /// Render the saved credential or its edit form.
    pub fn render(self, app: &App, cx: &gpui_kit::App) -> Div {
        use gpui_kit::component::{Disableable, button::Button, input::Input};
        let p = Palette::current(app, cx);
        let mut result = div().flex().flex_col().gap_2().px_3().py_2();
        if self.editing {
            let complete =
                self.inputs.iter().all(|input| !input.state.read(cx).value().trim().is_empty());
            let states: Vec<_> = self.inputs.iter().map(|input| input.state.clone()).collect();
            for input in self.inputs {
                result = result.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .text_xs()
                                .text_color(p.muted)
                                .whitespace_nowrap()
                                .child(input.label),
                        )
                        .child(Input::new(&input.state).disabled(self.busy)),
                );
            }
            let cancel = self.on_cancel;
            let save = self.on_save;
            result = result.child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new((self.id, 0usize))
                            .label(t!("settings.cancel"))
                            .disabled(self.busy)
                            .on_click(move |_, window, cx| cancel(window, cx)),
                    )
                    .child(
                        Button::new((self.id, 1usize))
                            .label(t!("settings.save"))
                            .disabled(self.busy || !complete)
                            .on_click(move |_, window, cx| {
                                let values = states
                                    .iter()
                                    .map(|state| state.read(cx).value().to_string())
                                    .collect();
                                save(values, window, cx);
                            }),
                    ),
            );
        } else {
            let edit = self.on_edit;
            let remove = self.on_remove;
            let configured = self.saved.is_some();
            let saved = self.saved.unwrap_or_else(|| t!("settings.holdings.notSet").into());
            let mut row = div()
                .flex()
                .items_center()
                .gap_2()
                .child(div().whitespace_nowrap().child(self.label))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_color(p.muted)
                        .text_xs()
                        .child(saved),
                )
                .child(
                    Button::new((self.id, 2usize))
                        .label(if configured { t!("settings.replace") } else { t!("settings.add") })
                        .disabled(self.busy)
                        .on_click(move |_, window, cx| edit(window, cx)),
                );
            if configured {
                row = row.child(
                    Button::new((self.id, 3usize))
                        .label(t!("settings.remove"))
                        .disabled(self.busy)
                        .on_click(move |_, window, cx| remove(window, cx)),
                );
            }
            result = result.child(row);
        }
        if let Some(error) = self.error {
            result = result.child(problem((self.id, 4usize), error));
        }
        result
    }
}
