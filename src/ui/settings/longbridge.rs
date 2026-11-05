//! Longbridge credentials and what the account may quote.

use super::parts;
use crate::{
    api,
    app::{App, Topic},
    credentials::LongbridgeKeys,
    market::longbridge::{Account, MarketAccess},
    ui::{self, theme::Palette},
};
use gpui_kit::{
    AppContext, Context, Div, Entity, IntoElement, ParentElement, Render, SharedString, Styled,
    Task, Window,
    component::{
        Sizable as _,
        button::{Button, ButtonVariants as _},
        input::InputState,
        link::Link,
        spinner::Spinner,
    },
    div, px,
};

/// One quote market the account may see, as one short line.
#[derive(Debug, PartialEq, Eq)]
struct AccessLine {
    /// `US`, `HK` or `CN`.
    market: &'static str,
    detail: AccessDetail,
}

#[derive(Debug, PartialEq, Eq)]
enum AccessDetail {
    /// Quote packages as Longbridge names them, joined for one line.
    Packages(String),
    /// Why there are none, as Longbridge puts it.
    Note(String),
    /// No packages and no reason: the account cannot quote this market.
    None,
}

/// What the account may see, one line per market in Longbridge's order.
fn access_lines(account: &Account) -> Vec<AccessLine> {
    account
        .markets
        .iter()
        .map(|MarketAccess { market, packages, note }| {
            let detail = if packages.is_empty() {
                note.clone()
                    .filter(|note| !note.trim().is_empty())
                    .map(AccessDetail::Note)
                    .unwrap_or(AccessDetail::None)
            } else {
                AccessDetail::Packages(packages.join(" · "))
            };
            AccessLine { market, detail }
        })
        .collect()
}

/// The credentials as the form saves them: whitespace removed, all three
/// required.
fn normalized_keys(app_key: &str, app_secret: &str, access_token: &str) -> Option<LongbridgeKeys> {
    let clean = |value: &str| value.chars().filter(|c| !c.is_whitespace()).collect::<String>();
    let keys = LongbridgeKeys {
        app_key: clean(app_key),
        app_secret: clean(app_secret),
        access_token: clean(access_token),
    };
    let complete =
        !keys.app_key.is_empty() && !keys.app_secret.is_empty() && !keys.access_token.is_empty();
    complete.then_some(keys)
}

fn market_label(market: &str) -> SharedString {
    match market {
        "US" => t!("market.US").into(),
        "HK" => t!("market.HK").into(),
        "CN" => t!("market.CN").into(),
        other => other.to_owned().into(),
    }
}

pub struct LongbridgeSection {
    app: App,
    view: api::LongbridgeView,
    editing: bool,
    checking: bool,
    /// Incremented by every check and by removing the keys, so a reply from
    /// an earlier check is ignored.
    check_seq: u64,
    error: Option<SharedString>,
    app_key: Entity<InputState>,
    app_secret: Entity<InputState>,
    access_token: Entity<InputState>,
    _observers: Vec<Task<()>>,
}

impl LongbridgeSection {
    pub fn new(app: App, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let view = api::get_longbridge(&app);
        let should_check = view.app_key.is_some() && view.account.is_none();
        let mut this = Self::with_view(app, view, window, cx);
        if should_check {
            this.start_check(cx);
        }
        this
    }

    /// The section over an explicit snapshot; the offscreen renderer uses it
    /// to show states no stored credentials can produce.
    pub(crate) fn with_view(
        app: App,
        view: api::LongbridgeView,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let editing = view.app_key.is_none();
        let app_key = cx.new(|cx| InputState::new(window, cx));
        let app_secret = cx.new(|cx| InputState::new(window, cx).masked(true));
        let access_token = cx.new(|cx| InputState::new(window, cx).masked(true));
        let observers = vec![
            ui::observe(&app, Topic::Locale, cx, |_, _| {}),
            ui::observe(&app, Topic::Settings, cx, |this, cx| {
                let view = api::get_longbridge(&this.app);
                let editing = this.editing;
                this.show(view, this.checking, this.error.clone(), cx);
                this.editing = editing;
            }),
        ];
        Self {
            app,
            view,
            editing,
            checking: false,
            check_seq: 0,
            error: None,
            app_key,
            app_secret,
            access_token,
            _observers: observers,
        }
    }

    /// Replaces the snapshot the section shows, with the status that goes
    /// with it.
    pub(crate) fn show(
        &mut self,
        view: api::LongbridgeView,
        checking: bool,
        error: Option<SharedString>,
        cx: &mut Context<Self>,
    ) {
        self.view = view;
        self.checking = checking;
        self.error = error;
        self.editing = self.view.app_key.is_none();
        cx.notify();
    }

    fn clear_inputs(&self, window: &mut Window, cx: &mut Context<Self>) {
        for input in [&self.app_key, &self.app_secret, &self.access_token] {
            let input = input.clone();
            input.update(cx, |input, cx| input.set_value("", window, cx));
        }
    }

    fn begin_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.editing = true;
        self.error = None;
        self.clear_inputs(window, cx);
        cx.notify();
    }

    fn cancel_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.editing = false;
        self.clear_inputs(window, cx);
        cx.notify();
    }

    fn save(&mut self, values: Vec<String>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(keys) = normalized_keys(
            values.first().map(String::as_str).unwrap_or_default(),
            values.get(1).map(String::as_str).unwrap_or_default(),
            values.get(2).map(String::as_str).unwrap_or_default(),
        ) else {
            self.error = Some(t!("error.longbridgeIncomplete").into());
            cx.notify();
            return;
        };
        match api::set_longbridge(&self.app, Some(keys)) {
            Ok(view) => {
                self.view = view;
                self.editing = false;
                self.error = None;
                self.clear_inputs(window, cx);
                self.start_check(cx);
            }
            Err(error) => {
                self.error = Some(error.to_string().into());
                cx.notify();
            }
        }
    }

    fn remove(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.check_seq += 1;
        self.checking = false;
        match api::set_longbridge(&self.app, None) {
            Ok(view) => {
                self.view = view;
                self.editing = true;
                self.error = None;
                self.clear_inputs(window, cx);
            }
            Err(error) => self.error = Some(error.to_string().into()),
        }
        cx.notify();
    }

    fn start_check(&mut self, cx: &mut Context<Self>) {
        self.check_seq += 1;
        let seq = self.check_seq;
        self.checking = true;
        self.error = None;
        cx.notify();
        let app = self.app.clone();
        let task = ui::run(&app, async move { api::check_longbridge().await });
        cx.spawn(async move |view, cx| {
            let result = task.await;
            let _ = view.update(cx, |this, cx| {
                if this.check_seq != seq {
                    return;
                }
                this.checking = false;
                match result {
                    Ok(Ok(account)) => this.view.account = Some(account),
                    Ok(Err(error)) => this.error = Some(error.to_string().into()),
                    Err(error) => this.error = Some(error.to_string().into()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn status(&self, cx: &mut Context<Self>) -> Div {
        let p = Palette::current(&self.app, cx);
        let mut block = div()
            .flex()
            .flex_col()
            .px_3()
            .pt_1()
            .pb_2()
            .text_xs()
            .border_t_1()
            .border_color(p.border);
        if self.checking {
            let source: SharedString = t!("common.provider.longbridge").into();
            return block.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .py_1()
                    .text_color(p.muted)
                    .child(Spinner::new().xsmall().color(p.muted))
                    .child(
                        div()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .child(t!("settings.stocks.signingIn", source = source)),
                    ),
            );
        }
        if let Some(account) = &self.view.account {
            for line in access_lines(account) {
                let detail: SharedString = match line.detail {
                    AccessDetail::Packages(text) | AccessDetail::Note(text) => text.into(),
                    AccessDetail::None => t!("settings.stocks.noAccess").into(),
                };
                block = block.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_3()
                        .py_1()
                        .child(
                            div()
                                .w(px(64.))
                                .flex_shrink_0()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_color(p.muted)
                                .child(market_label(line.market)),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .child(detail),
                        ),
                );
            }
            return block;
        }
        block.child(
            div().flex().justify_start().py_1().child(
                Button::new("longbridge-check")
                    .label(t!("settings.stocks.checkAccess"))
                    .link()
                    .w_auto()
                    .on_click(cx.listener(|this, _, _, cx| this.start_check(cx))),
            ),
        )
    }
}

impl Render for LongbridgeSection {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let source: SharedString = t!("common.provider.longbridge").into();
        let configured = self.view.app_key.is_some();
        let edit = cx.entity().downgrade();
        let cancel = edit.clone();
        let save = edit.clone();
        let remove = edit.clone();
        let field = parts::CredentialField {
            id: "longbridge",
            label: t!("settings.stocks.appKey").into(),
            saved: self.view.app_key.clone().map(Into::into),
            editing: self.editing,
            busy: false,
            inputs: vec![
                parts::CredentialInput {
                    label: t!("settings.stocks.appKey").into(),
                    state: self.app_key.clone(),
                },
                parts::CredentialInput {
                    label: t!("settings.stocks.appSecret").into(),
                    state: self.app_secret.clone(),
                },
                parts::CredentialInput {
                    label: t!("settings.stocks.accessToken").into(),
                    state: self.access_token.clone(),
                },
            ],
            error: None,
            on_edit: Box::new(move |window, cx| {
                let _ = edit.update(cx, |this, cx| this.begin_edit(window, cx));
            }),
            on_cancel: Box::new(move |window, cx| {
                let _ = cancel.update(cx, |this, cx| this.cancel_edit(window, cx));
            }),
            on_save: Box::new(move |values, window, cx| {
                let _ = save.update(cx, |this, cx| this.save(values, window, cx));
            }),
            on_remove: Box::new(move |window, cx| {
                let _ = remove.update(cx, |this, cx| this.remove(window, cx));
            }),
        }
        .render(&self.app, cx);
        let mut group = parts::group(&self.app, cx).child(field);
        if configured && !self.editing {
            group = group.child(self.status(cx));
        }
        let mut section = parts::section(source.clone(), &self.app, cx).child(group);
        if self.editing {
            section = section.child(
                div().px_1().text_xs().text_color(Palette::current(&self.app, cx).muted).child(
                    Link::new("longbridge-keys")
                        .href("https://open.longbridge.com/")
                        .whitespace_nowrap()
                        .overflow_hidden()
                        .child(t!("settings.stocks.getKeys", source = source)),
                ),
            );
        }
        if !configured || self.editing {
            section = section.child(
                div()
                    .px_1()
                    .text_xs()
                    .text_color(Palette::current(&self.app, cx).muted)
                    .whitespace_nowrap()
                    .overflow_hidden()
                    .child(t!("settings.stocks.footnote")),
            );
        }
        if let Some(error) = &self.error {
            section = section.child(parts::problem("longbridge-error", error.clone()));
        }
        section
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account(markets: Vec<MarketAccess>) -> Account {
        Account { markets }
    }

    #[test]
    fn each_market_gets_one_line_of_its_packages() {
        let account = account(vec![
            MarketAccess {
                market: "US",
                packages: vec!["LV1 Real-time Quotes".into(), "LV2 Real-time Quotes".into()],
                note: None,
            },
            MarketAccess {
                market: "HK",
                packages: vec!["LV1 Real-time Quotes".into()],
                note: Some("ignored while there are packages".into()),
            },
        ]);
        assert_eq!(
            access_lines(&account),
            vec![
                AccessLine {
                    market: "US",
                    detail: AccessDetail::Packages(
                        "LV1 Real-time Quotes · LV2 Real-time Quotes".into()
                    ),
                },
                AccessLine {
                    market: "HK",
                    detail: AccessDetail::Packages("LV1 Real-time Quotes".into()),
                },
            ]
        );
    }

    #[test]
    fn an_empty_market_falls_back_to_its_note_then_to_no_access() {
        let account = account(vec![
            MarketAccess {
                market: "US",
                packages: Vec::new(),
                note: Some("The quote plan expired".into()),
            },
            MarketAccess { market: "CN", packages: Vec::new(), note: Some("  ".into()) },
            MarketAccess { market: "HK", packages: Vec::new(), note: None },
        ]);
        assert_eq!(
            access_lines(&account),
            vec![
                AccessLine {
                    market: "US",
                    detail: AccessDetail::Note("The quote plan expired".into()),
                },
                AccessLine { market: "CN", detail: AccessDetail::None },
                AccessLine { market: "HK", detail: AccessDetail::None },
            ]
        );
    }

    #[test]
    fn keys_lose_whitespace_before_they_are_saved() {
        assert_eq!(
            normalized_keys(" LB-1234\n", " secret\t", " token "),
            Some(LongbridgeKeys {
                app_key: "LB-1234".into(),
                app_secret: "secret".into(),
                access_token: "token".into(),
            })
        );
        assert_eq!(normalized_keys("a b", "c", "d").unwrap().app_key, "ab");
    }

    #[test]
    fn all_three_keys_are_required() {
        assert_eq!(normalized_keys("key", "secret", ""), None);
        assert_eq!(normalized_keys("key", " \n\t", "token"), None);
        assert_eq!(normalized_keys("", "secret", "token"), None);
    }
}
