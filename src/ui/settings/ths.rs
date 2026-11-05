//! Tonghuashun credentials and account settings.

use std::sync::Arc;

use base64::{Engine, engine::general_purpose::STANDARD};
use gpui_kit::{
    AppContext, Context, Entity, Image, ImageFormat, IntoElement, ParentElement, Render, Styled,
    Task, Window,
    component::{button::Button, input::InputState, spinner::Spinner},
    div, img, px, white,
};

use super::parts::{self, CredentialField, CredentialInput};
use crate::{
    api,
    app::{App, Topic},
    market::ths::{AccountView, SignIn, SignInState, SyncState},
    ui::{self, theme::Palette},
};

pub struct ThsSection {
    app: App,
    key: Entity<InputState>,
    editing: bool,
    busy: bool,
    error: Option<String>,
    qr: Option<(String, Arc<Image>)>,
    _observers: Vec<Task<()>>,
}

#[derive(Clone, Copy)]
enum AccountAction {
    SignIn,
    Cancel,
    SignOut,
}

impl ThsSection {
    pub fn new(app: App, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let observers = [Topic::Ths, Topic::Settings, Topic::Locale]
            .into_iter()
            .map(|topic| ui::observe(&app, topic, cx, |_, _| {}))
            .collect();
        let key = cx.new(|cx| InputState::new(window, cx).masked(true));
        Self { app, key, editing: false, busy: false, error: None, qr: None, _observers: observers }
    }

    fn save(&mut self, key: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        self.busy = true;
        self.error = None;
        let app = self.app.clone();
        let io_app = app.clone();
        let task = ui::run(&app, async move { api::set_ths(&io_app, key).await });
        cx.spawn_in(window, async move |view, cx| {
            let result = task.await;
            let _ = view.update_in(cx, |this, window, cx| {
                this.busy = false;
                match result {
                    Ok(Ok(_)) => {
                        this.editing = false;
                        this.key.update(cx, |input, cx| input.set_value("", window, cx));
                    }
                    Ok(Err(error)) => this.error = Some(error.to_string()),
                    Err(error) => this.error = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn qr_image(&mut self, data: Option<&str>) -> Option<Arc<Image>> {
        let data = data?;
        if self.qr.as_ref().is_none_or(|(cached, _)| cached != data) {
            self.qr = decode_qr(data).map(|image| (data.to_owned(), image));
        }
        self.qr.as_ref().map(|(_, image)| image.clone())
    }

    fn account_action(&mut self, action: AccountAction, cx: &mut Context<Self>) {
        self.error = None;
        match action {
            AccountAction::SignIn => api::ths_sign_in(&self.app),
            AccountAction::Cancel => api::ths_sign_in_cancel(&self.app),
            AccountAction::SignOut => {
                api::ths_sign_out(&self.app);
            }
        }
        cx.notify();
    }
}

fn decode_qr(data: &str) -> Option<Arc<Image>> {
    let encoded = data.strip_prefix("data:image/png;base64,")?;
    let bytes = STANDARD.decode(encoded).ok()?;
    Some(Arc::new(Image::from_bytes(ImageFormat::Png, bytes)))
}

fn sign_in_detail(sign_in: &SignIn) -> String {
    match sign_in.state {
        SignInState::Waiting => t!("settings.ths.scan").to_owned(),
        SignInState::Expired => t!("settings.ths.qrExpired").to_owned(),
        SignInState::Failed => {
            sign_in.error.clone().unwrap_or_else(|| t!("settings.ths.qrExpired").to_owned())
        }
    }
}

fn account_detail(account: &AccountView) -> String {
    match account.state {
        SyncState::Reading => t!("settings.ths.reading").to_owned(),
        SyncState::SignedOut => t!("settings.ths.signedOut").to_owned(),
        SyncState::Failed => {
            t!("settings.ths.readFailed", error = account.error.as_deref().unwrap_or("")).to_owned()
        }
        SyncState::Read => {
            let count = account.count;
            let mut parts = vec![if count == 1 {
                t!("settings.ths.countOne", count = count).to_owned()
            } else {
                t!("settings.ths.count", count = count).to_owned()
            }];
            let count = account.groups;
            if count > 0 {
                parts.push(if count == 1 {
                    t!("settings.ths.groupsOne", count = count).to_owned()
                } else {
                    t!("settings.ths.groups", count = count).to_owned()
                });
            }
            let count = account.skipped;
            if count > 0 {
                parts.push(t!("settings.ths.skipped", count = count).to_owned());
            }
            parts.join(" · ")
        }
    }
}

impl Render for ThsSection {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = api::get_ths(&self.app);
        let p = Palette::current(&self.app, cx);
        let edit = cx.entity().downgrade();
        let cancel = edit.clone();
        let save = edit.clone();
        let remove = edit.clone();
        let configured = state.key.is_some() && !self.editing;
        let field = CredentialField {
            id: "ths-key",
            label: "API Key".into(),
            saved: state.key.clone().map(Into::into),
            editing: self.editing || state.key.is_none(),
            busy: self.busy,
            inputs: vec![CredentialInput { label: "API Key".into(), state: self.key.clone() }],
            error: self.error.clone().map(Into::into),
            link: None,
            on_edit: Box::new(move |_, cx| {
                let _ = edit.update(cx, |this, cx| {
                    this.editing = true;
                    this.error = None;
                    cx.notify();
                });
            }),
            on_cancel: Box::new(move |window, cx| {
                let _ = cancel.update(cx, |this, cx| {
                    this.editing = false;
                    this.error = None;
                    this.key.update(cx, |input, cx| input.set_value("", window, cx));
                    cx.notify();
                });
            }),
            on_save: Box::new(move |values, window, cx| {
                let _ =
                    save.update(cx, |this, cx| this.save(values.into_iter().next(), window, cx));
            }),
            on_remove: Box::new(move |window, cx| {
                let _ = remove.update(cx, |this, cx| this.save(None, window, cx));
            }),
        }
        .render(&self.app, cx);
        let mut group = parts::group(&self.app, cx).child(field);
        if configured {
            let account = state.account.as_ref();
            let sign_in = state.sign_in.as_ref();
            let detail = sign_in
                .map(sign_in_detail)
                .or_else(|| account.map(account_detail))
                .unwrap_or_else(|| t!("settings.ths.watchlistDetail").to_owned());
            let is_problem = sign_in.is_some_and(|s| matches!(s.state, SignInState::Failed))
                || account
                    .is_some_and(|a| matches!(a.state, SyncState::SignedOut | SyncState::Failed));
            let mut buttons = div().flex().gap_2();
            if let Some(sign_in) = sign_in {
                if !matches!(sign_in.state, SignInState::Waiting) {
                    let view = cx.entity().downgrade();
                    buttons = buttons.child(
                        Button::new("ths-new-code")
                            .compact()
                            .label(t!("settings.ths.newCode"))
                            .on_click(move |_, _, cx| {
                                let _ = view.update(cx, |this, cx| {
                                    this.account_action(AccountAction::SignIn, cx)
                                });
                            }),
                    );
                }
                let view = cx.entity().downgrade();
                buttons = buttons.child(
                    Button::new("ths-cancel").compact().label(t!("settings.cancel")).on_click(
                        move |_, _, cx| {
                            let _ = view.update(cx, |this, cx| {
                                this.account_action(AccountAction::Cancel, cx)
                            });
                        },
                    ),
                );
            } else {
                if account.is_none_or(|a| matches!(a.state, SyncState::SignedOut)) {
                    let view = cx.entity().downgrade();
                    buttons = buttons.child(
                        Button::new("ths-sign-in")
                            .compact()
                            .label(if account.is_some() {
                                t!("settings.ths.signInAgain")
                            } else {
                                t!("settings.ths.signIn")
                            })
                            .on_click(move |_, _, cx| {
                                let _ = view.update(cx, |this, cx| {
                                    this.account_action(AccountAction::SignIn, cx)
                                });
                            }),
                    );
                }
                if account.is_some() {
                    let view = cx.entity().downgrade();
                    buttons = buttons.child(
                        Button::new("ths-sign-out")
                            .compact()
                            .label(t!("settings.ths.signOut"))
                            .on_click(move |_, _, cx| {
                                let _ = view.update(cx, |this, cx| {
                                    this.account_action(AccountAction::SignOut, cx)
                                });
                            }),
                    );
                }
            }
            let mut row = div()
                .flex()
                .flex_col()
                .gap_1()
                .px_3()
                .py_2()
                .border_t_1()
                .border_color(p.border)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_2()
                        .child(div().min_w_0().overflow_hidden().whitespace_nowrap().child(
                            if account.is_some() {
                                t!("settings.ths.account")
                            } else {
                                t!("settings.ths.watchlist")
                            },
                        ))
                        .child(buttons),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(if is_problem { p.red } else { p.muted })
                        .whitespace_nowrap()
                        .overflow_hidden()
                        .child(detail),
                );
            if sign_in.is_some_and(|s| matches!(s.state, SignInState::Waiting)) {
                let image = self.qr_image(sign_in.and_then(|s| s.qr.as_deref()));
                let mut box_ = div()
                    .size(px(176.))
                    .p_4()
                    .rounded_lg()
                    .bg(white())
                    .flex()
                    .items_center()
                    .justify_center();
                if let Some(image) = image {
                    box_ = box_.child(img(image).size_full());
                } else {
                    box_ = box_.child(Spinner::new().color(gpui_kit::black()));
                }
                row = row.child(div().flex().justify_center().py_2().child(box_));
            }
            group = group.child(row);
        }
        let mut section = parts::section(t!("common.provider.ths"), &self.app, cx).child(group);
        if !configured {
            section = section.child(
                div()
                    .px_1()
                    .text_xs()
                    .text_color(p.muted)
                    .whitespace_nowrap()
                    .overflow_hidden()
                    .child(t!("settings.ths.footnote")),
            );
            section = section.child(
                Button::new("ths-get-key")
                    .compact()
                    .label(t!("settings.ths.getKey"))
                    .on_click(|_, _, cx| cx.open_url("https://fuyao.aicubes.cn/admin")),
            );
        }
        section
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_in_reports_scan_expiry_and_failure() {
        let mut sign_in = SignIn { qr: None, state: SignInState::Waiting, error: None };
        assert_eq!(sign_in_detail(&sign_in), t!("settings.ths.scan"));
        sign_in.state = SignInState::Expired;
        assert_eq!(sign_in_detail(&sign_in), t!("settings.ths.qrExpired"));
        sign_in.state = SignInState::Failed;
        sign_in.error = Some("connection failed".into());
        assert_eq!(sign_in_detail(&sign_in), "connection failed");
        sign_in.error = None;
        assert_eq!(sign_in_detail(&sign_in), t!("settings.ths.qrExpired"));
    }

    #[test]
    fn account_summary_includes_nonzero_groups_and_skipped() {
        let mut account =
            AccountView { count: 1, groups: 0, skipped: 0, state: SyncState::Read, error: None };
        assert_eq!(account_detail(&account), t!("settings.ths.countOne", count = 1));
        account.count = 7;
        account.groups = 2;
        account.skipped = 3;
        assert_eq!(
            account_detail(&account),
            format!(
                "{} · {} · {}",
                t!("settings.ths.count", count = 7),
                t!("settings.ths.groups", count = 2),
                t!("settings.ths.skipped", count = 3)
            )
        );
        account.state = SyncState::SignedOut;
        assert_eq!(account_detail(&account), t!("settings.ths.signedOut"));
    }

    #[test]
    fn qr_data_url_decodes_png_bytes() {
        let image = decode_qr("data:image/png;base64,iVBORw0KGgo=").unwrap();
        assert_eq!(image.bytes, b"\x89PNG\r\n\x1a\n");
        assert!(decode_qr("https://example.com/qr.png").is_none());
        assert!(decode_qr("data:image/png;base64,!!!").is_none());
    }
}
