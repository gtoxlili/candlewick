//! The exchange API keys section: one read-only key per exchange, confirmed
//! with the exchange before it is stored.

use super::parts::{self, CredentialField, CredentialInput, CredentialLink};
use crate::{
    api,
    app::{App, Topic},
    credentials::ApiKey,
    i18n,
    market::ProviderId,
    ui,
};
use gpui_kit::{
    AppContext as _, Context, Div, Entity, IntoElement, ParentElement, Render, SharedString,
    Styled, Task, Window, component::input::InputState, div, px,
};

/// Which part of an API key an input edits.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum KeyPart {
    Key,
    Secret,
    Passphrase,
}

/// One labeled input of the key form, as the exchange's page names it.
#[derive(Clone, Copy, Debug)]
struct KeyField {
    part: KeyPart,
    label: &'static str,
    /// Dots instead of characters while typing.
    secret: bool,
}

/// What each exchange's API page calls the parts of a key; OKX adds a
/// passphrase.
fn key_fields(exchange: ProviderId) -> Vec<KeyField> {
    let okx = exchange == ProviderId::Okx;
    let mut fields = vec![
        KeyField { part: KeyPart::Key, label: t!("settings.holdings.apiKey"), secret: false },
        KeyField {
            part: KeyPart::Secret,
            label: if okx {
                t!("settings.holdings.secretKey")
            } else {
                t!("settings.holdings.apiSecret")
            },
            secret: true,
        },
    ];
    if okx {
        fields.push(KeyField {
            part: KeyPart::Passphrase,
            label: t!("settings.holdings.passphrase"),
            secret: true,
        });
    }
    fields
}

/// Where the exchange lets one create an API key, in the page's language
/// where the site has it.
fn api_page(exchange: ProviderId, locale: i18n::Locale) -> String {
    use i18n::Locale::{En, Ja, ZhCn};
    debug_assert!(exchange.is_exchange());
    match (exchange, locale) {
        (ProviderId::Binance, ZhCn) => {
            "https://www.binance.com/zh-CN/my/settings/api-management".into()
        }
        (ProviderId::Binance, Ja) => "https://www.binance.com/ja/my/settings/api-management".into(),
        (ProviderId::Binance, En) => "https://www.binance.com/en/my/settings/api-management".into(),
        (ProviderId::Bybit, _) => "https://www.bybit.com/app/user/api-management".into(),
        (ProviderId::Okx, ZhCn) => "https://www.okx.com/zh-hans/account/my-api".into(),
        (ProviderId::Okx, _) => "https://www.okx.com/account/my-api".into(),
        // Only exchanges have an API page; they are checked above.
        _ => String::new(),
    }
}

pub struct ApiKeysSection {
    app: App,
    /// The saved keys, masked, one entry per exchange in `ProviderId::EXCHANGES` order.
    keys: Vec<api::ExchangeKey>,
    /// The exchange whose editor is open.
    editing: Option<ProviderId>,
    /// One input per field of the edited key, in field order.
    inputs: Vec<(KeyPart, Entity<InputState>)>,
    /// The exchange is confirming a key: every row waits with it.
    saving: bool,
    /// Why the exchange refused a key, or could not remove it.
    key_problem: Option<(ProviderId, SharedString)>,
    /// Why the holdings switch or window could not be saved or opened.
    holdings_problem: Option<SharedString>,
    _observers: Vec<Task<()>>,
}

impl ApiKeysSection {
    pub fn new(app: App, _: &mut Window, cx: &mut Context<Self>) -> Self {
        let keys = api::get_exchange_keys(&app);
        let observers = vec![
            ui::observe(&app, Topic::Settings, cx, |_, _| {}),
            ui::observe(&app, Topic::Locale, cx, |_, _| {}),
        ];
        Self {
            app,
            keys,
            editing: None,
            inputs: Vec::new(),
            saving: false,
            key_problem: None,
            holdings_problem: None,
            _observers: observers,
        }
    }

    /// Open the exchange's editor with empty inputs, closing any other.
    fn begin_edit(&mut self, exchange: ProviderId, window: &mut Window, cx: &mut Context<Self>) {
        self.editing = Some(exchange);
        self.key_problem = None;
        self.inputs = key_fields(exchange)
            .into_iter()
            .map(|field| {
                let secret = field.secret;
                (field.part, cx.new(|cx| InputState::new(window, cx).masked(secret)))
            })
            .collect();
        cx.notify();
    }

    /// Close the editor and forget what was typed.
    fn cancel_edit(&mut self, cx: &mut Context<Self>) {
        self.editing = None;
        self.inputs.clear();
        self.key_problem = None;
        cx.notify();
    }

    /// The typed values, in field order, as an API key.
    fn key_from_values(&self, values: &[String]) -> ApiKey {
        let mut key = ApiKey { key: String::new(), secret: String::new(), passphrase: None };
        for ((part, _), value) in self.inputs.iter().zip(values) {
            match part {
                KeyPart::Key => key.key = value.clone(),
                KeyPart::Secret => key.secret = value.clone(),
                KeyPart::Passphrase => key.passphrase = Some(value.clone()),
            }
        }
        key
    }

    /// Save (or with `None`, remove) the exchange's key. The exchange confirms
    /// the key first; until then the editor stays open.
    fn start_save(&mut self, exchange: ProviderId, key: Option<ApiKey>, cx: &mut Context<Self>) {
        self.saving = true;
        self.key_problem = None;
        let io_app = self.app.clone();
        let check =
            ui::run(&self.app, async move { api::set_exchange_key(&io_app, exchange, key).await });
        cx.spawn(async move |view, cx| {
            let outcome = check.await;
            let _ = view.update(cx, |this, cx| {
                this.saving = false;
                match outcome {
                    Ok(Ok(keys)) => {
                        this.keys = keys;
                        this.editing = None;
                        this.inputs.clear();
                    }
                    Ok(Err(error)) => this.key_problem = Some((exchange, error.to_string().into())),
                    Err(error) => this.key_problem = Some((exchange, error.to_string().into())),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn set_holdings_in_bar(&mut self, on: bool, cx: &mut Context<Self>) {
        self.holdings_problem =
            parts::update_settings(&self.app, |settings| settings.holdings_in_bar = on)
                .err()
                .map(SharedString::from);
        cx.notify();
    }

    /// One exchange's row: its name and masked key, and its editor.
    fn exchange_field(
        &self,
        exchange: ProviderId,
        saved: Option<String>,
        locale: i18n::Locale,
        cx: &mut Context<Self>,
    ) -> Div {
        let editing = self.editing == Some(exchange);
        let inputs = if editing {
            key_fields(exchange)
                .into_iter()
                .zip(&self.inputs)
                .map(|(field, (_, state))| CredentialInput {
                    label: field.label.into(),
                    state: state.clone(),
                })
                .collect()
        } else {
            Vec::new()
        };
        let weak = cx.entity().downgrade();
        let edit = weak.clone();
        let cancel = weak.clone();
        let save = weak.clone();
        let remove = weak.clone();
        CredentialField {
            id: exchange.key(),
            label: exchange.name().into(),
            saved: saved.map(SharedString::from),
            editing,
            busy: self.saving,
            inputs,
            error: match &self.key_problem {
                Some((at, message)) if *at == exchange => Some(message.clone()),
                _ => None,
            },
            link: editing.then(|| CredentialLink {
                label: t!("settings.holdings.create", exchange = exchange.name()).into(),
                url: api_page(exchange, locale),
            }),
            on_edit: Box::new(move |window, cx| {
                let _ = edit.update(cx, |this, cx| this.begin_edit(exchange, window, cx));
            }),
            on_cancel: Box::new(move |_, cx| {
                let _ = cancel.update(cx, |this, cx| this.cancel_edit(cx));
            }),
            on_save: Box::new(move |values, _, cx| {
                let _ = save.update(cx, |this, cx| {
                    let key = this.key_from_values(&values);
                    this.start_save(exchange, Some(key), cx);
                });
            }),
            on_remove: Box::new(move |_, cx| {
                let _ = remove.update(cx, |this, cx| this.start_save(exchange, None, cx));
            }),
        }
        .render(&self.app, cx)
    }

    /// The show-total switch, with its explanatory line under the row so the
    /// line has the full width in every language.
    fn holdings_row(&self, checked: bool, cx: &mut Context<Self>) -> Div {
        use gpui_kit::component::switch::Switch;
        let p = crate::ui::theme::Palette::current(&self.app, cx);
        let weak = cx.entity().downgrade();
        let label = t!("settings.holdings.inBar");
        div()
            .flex()
            .flex_col()
            .gap_1()
            .px_3()
            .py_2()
            .min_h(px(40.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_4()
                    .child(div().min_w_0().overflow_hidden().whitespace_nowrap().child(label))
                    .child(
                        Switch::new("holdings-in-bar")
                            .accessibility_label(label)
                            .checked(checked)
                            .on_change(move |on, _, cx| {
                                let _ =
                                    weak.update(cx, |this, cx| this.set_holdings_in_bar(*on, cx));
                            }),
                    ),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(p.muted)
                    .whitespace_nowrap()
                    .overflow_hidden()
                    .child(t!("settings.holdings.inBarDetail")),
            )
    }

    /// The row that opens the holdings window.
    fn open_row(&self, cx: &mut Context<Self>) -> Div {
        use gpui_kit::component::{button::Button, button::ButtonVariants as _};
        let app = self.app.clone();
        let weak = cx.entity().downgrade();
        div()
            .flex()
            .items_center()
            .justify_between()
            .gap_4()
            .px_3()
            .py_2()
            .min_h(px(40.))
            .child(
                div()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .child(t!("settings.holdings.window")),
            )
            .child(
                Button::new("open-holdings").label(t!("settings.holdings.open")).ghost().on_click(
                    move |_, _, cx| {
                        if let Err(error) = api::open_holdings(&app) {
                            let _ = weak.update(cx, |this, cx| {
                                this.holdings_problem = Some(error.to_string().into());
                                cx.notify();
                            });
                        }
                    },
                ),
            )
    }

    /// Put the section into a given editing, busy and problem state; for
    /// offscreen renders.
    #[cfg(test)]
    pub fn testing_state(
        &mut self,
        editing: Option<ProviderId>,
        saving: bool,
        problem: Option<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(exchange) = editing {
            self.begin_edit(exchange, window, cx);
        }
        self.saving = saving;
        self.key_problem = problem.map(|message| (editing.unwrap_or(ProviderId::Binance), message));
        cx.notify();
    }
}

impl Render for ApiKeysSection {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let locale = api::get_locale();
        let settings = api::get_settings(&self.app);
        let configured = self.keys.iter().any(|entry| entry.key.is_some());
        let mut group = parts::group(&self.app, cx);
        let entries: Vec<(ProviderId, Option<String>)> =
            self.keys.iter().map(|entry| (entry.exchange, entry.key.clone())).collect();
        for (exchange, saved) in entries {
            group = group.child(self.exchange_field(exchange, saved, locale, cx));
        }
        if configured {
            group = group
                .child(self.holdings_row(settings.holdings_in_bar, cx))
                .child(self.open_row(cx));
        }
        if let Some(error) = self.holdings_problem.clone() {
            group = group.child(parts::problem("holdings-problem", error));
        }
        let p = crate::ui::theme::Palette::current(&self.app, cx);
        parts::section(t!("settings.holdings.title"), &self.app, cx).child(group).child(
            div()
                .flex()
                .flex_col()
                .px_1()
                .text_xs()
                .text_color(p.muted)
                .child(footnote_line(t!("settings.holdings.footnote")))
                .child(footnote_line(t!("settings.holdings.footnoteKeys"))),
        )
    }
}

/// One deliberate line of the footnote: it must not wrap when the window is
/// as narrow as it gets.
fn footnote_line(text: &'static str) -> Div {
    div().whitespace_nowrap().overflow_hidden().child(text)
}

#[cfg(test)]
mod tests {
    use super::{KeyPart::*, api_page, key_fields};
    use crate::{
        i18n::Locale::{En, Ja, ZhCn},
        market::ProviderId,
    };

    #[test]
    fn fields_per_exchange() {
        let parts = |exchange: ProviderId| {
            key_fields(exchange).iter().map(|field| (field.part, field.secret)).collect::<Vec<_>>()
        };
        assert_eq!(
            parts(ProviderId::Binance),
            vec![(Key, false), (Secret, true)],
            "no passphrase on Binance"
        );
        assert_eq!(
            parts(ProviderId::Bybit),
            vec![(Key, false), (Secret, true)],
            "no passphrase on Bybit"
        );
        assert_eq!(
            parts(ProviderId::Okx),
            vec![(Key, false), (Secret, true), (Passphrase, true)],
            "OKX adds a passphrase"
        );
    }

    #[test]
    fn api_pages_by_locale() {
        assert_eq!(
            api_page(ProviderId::Binance, En),
            "https://www.binance.com/en/my/settings/api-management"
        );
        assert_eq!(
            api_page(ProviderId::Binance, ZhCn),
            "https://www.binance.com/zh-CN/my/settings/api-management"
        );
        assert_eq!(
            api_page(ProviderId::Binance, Ja),
            "https://www.binance.com/ja/my/settings/api-management"
        );
        for locale in [En, ZhCn, Ja] {
            assert_eq!(
                api_page(ProviderId::Bybit, locale),
                "https://www.bybit.com/app/user/api-management",
                "Bybit has one page"
            );
        }
        assert_eq!(api_page(ProviderId::Okx, En), "https://www.okx.com/account/my-api");
        assert_eq!(
            api_page(ProviderId::Okx, Ja),
            "https://www.okx.com/account/my-api",
            "OKX's Japanese page is the default one"
        );
        assert_eq!(api_page(ProviderId::Okx, ZhCn), "https://www.okx.com/zh-hans/account/my-api");
    }
}
