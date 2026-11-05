//! The CoinGecko API key, its plan and the pace it allows.

use super::parts::{self, CredentialField, CredentialInput};
use crate::{
    api::{self, CoinGeckoView},
    app::{App, Topic},
    format,
    i18n::{self, Locale},
    market::dexscreener::Access,
    ui::{self, theme::Palette},
};
use gpui_kit::{
    AppContext as _, Context, Div, Entity, IntoElement, ParentElement, Render, SharedString,
    Styled, Subscription, Task, Window,
    component::{
        Sizable, Size,
        input::{InputEvent, InputState},
        link::Link,
        spinner::Spinner,
    },
    div,
};

const DASHBOARD: &str = "https://www.coingecko.com/en/developers/dashboard";

pub struct CoinGeckoSection {
    app: App,
    view: CoinGeckoView,
    editing: bool,
    key: Entity<InputState>,
    busy: bool,
    error: Option<SharedString>,
    task: Option<Task<()>>,
    _subscription: Subscription,
    _observers: Vec<Task<()>>,
}

impl CoinGeckoSection {
    pub fn new(app: App, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let key = cx.new(|cx| InputState::new(window, cx).masked(true));
        let subscription = cx.subscribe(&key, |_, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        });
        let observers = vec![
            ui::observe(&app, Topic::Settings, cx, |this, _| {
                this.view = api::get_coingecko(&this.app);
            }),
            ui::observe(&app, Topic::Locale, cx, |_, _| {}),
        ];
        let mut this = Self {
            view: api::get_coingecko(&app),
            app,
            editing: false,
            key,
            busy: false,
            error: None,
            task: None,
            _subscription: subscription,
            _observers: observers,
        };
        if this.view.key.is_some() && this.view.access.is_none() {
            this.check(window, cx);
        }
        this
    }

    /// Ask CoinGecko what the saved key is now; a paid plan's usage moves.
    fn check(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.error = None;
        let app = self.app.clone();
        let io = app.clone();
        let task = ui::run(&app, async move { api::check_coingecko(&io).await });
        self.task = Some(cx.spawn_in(window, async move |view, cx| {
            let result = task.await;
            let _ = view.update_in(cx, |this, _, cx| {
                this.busy = false;
                match result {
                    Ok(Ok(access)) => this.view.access = Some(access),
                    Ok(Err(error)) => this.error = Some(error.to_string().into()),
                    Err(_) => this.error = Some(t!("settings.onchain.failed").into()),
                }
                cx.notify();
            });
        }));
    }

    /// Store the key, or with `None` remove it, once CoinGecko takes it.
    fn store(&mut self, key: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.error = None;
        let app = self.app.clone();
        let io = app.clone();
        let task = ui::run(&app, async move { api::set_coingecko(&io, key).await });
        self.task = Some(cx.spawn_in(window, async move |view, cx| {
            let result = task.await;
            let _ = view.update_in(cx, |this, window, cx| {
                this.busy = false;
                match result {
                    Ok(Ok(saved)) => {
                        this.view = saved;
                        this.editing = false;
                        let input = this.key.clone();
                        input.update(cx, |state, cx| state.set_value("", window, cx));
                    }
                    Ok(Err(error)) => this.error = Some(error.to_string().into()),
                    Err(_) => this.error = Some(t!("settings.onchain.failed").into()),
                }
                cx.notify();
            });
        }));
    }

    fn check_link(&self, p: Palette, cx: &mut Context<Self>) -> impl IntoElement {
        let view = cx.entity().downgrade();
        Link::new("coingecko-check")
            .text_xs()
            .text_color(p.muted)
            .whitespace_nowrap()
            .on_click(move |_, _window, cx| {
                let _ = view.update_in(cx, |this, window, cx| this.check(window, cx));
            })
            .child(t!("settings.onchain.check"))
    }

    /// The saved key's row, or the form that replaces or adds one.
    fn credential(&self, cx: &mut Context<Self>) -> CredentialField {
        let edit = cx.entity().downgrade();
        let cancel = edit.clone();
        let save = edit.clone();
        let remove = edit.clone();
        CredentialField {
            id: "coingecko-key",
            label: t!("settings.onchain.key").into(),
            saved: self.view.key.clone().map(Into::into),
            editing: self.editing,
            busy: self.busy,
            inputs: vec![CredentialInput {
                label: t!("settings.onchain.field").into(),
                state: self.key.clone(),
            }],
            error: None,
            link: None,
            on_edit: Box::new(move |_, cx| {
                let _ = edit.update(cx, |this, cx| {
                    this.editing = true;
                    cx.notify();
                });
            }),
            on_cancel: Box::new(move |_, cx| {
                let _ = cancel.update(cx, |this, cx| {
                    this.editing = false;
                    cx.notify();
                });
            }),
            on_save: Box::new(move |values, _window, cx| {
                if let Some(key) = values.into_iter().next() {
                    let _ =
                        save.update_in(cx, |this, window, cx| this.store(Some(key), window, cx));
                }
            }),
            on_remove: Box::new(move |_window, cx| {
                let _ = remove.update_in(cx, |this, window, cx| this.store(None, window, cx));
            }),
        }
    }

    /// What the saved key is doing right now: checking, its plan, or a prompt
    /// to check.
    fn status(&self, p: Palette, cx: &mut Context<Self>) -> Div {
        let row = div().flex().flex_col().gap_1().px_3().py_2().text_xs();
        if self.busy {
            return row.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_color(p.muted)
                    .child(Spinner::new().with_size(Size::Small))
                    .child(div().whitespace_nowrap().child(t!("settings.onchain.checking"))),
            );
        }
        let Some(access) = self.view.access.as_ref() else {
            return row.child(self.check_link(p, cx));
        };
        let [plan, limits, pace] = access_lines(access, i18n::current());
        row.text_color(p.foreground)
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .child(
                        div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().child(plan),
                    )
                    .child(div().flex_shrink_0().child(self.check_link(p, cx))),
            )
            .child(
                div()
                    .text_color(p.muted)
                    .child(ui::tabular(div().whitespace_nowrap().overflow_hidden().child(limits))),
            )
            .child(div().text_color(p.muted).whitespace_nowrap().overflow_hidden().child(pace))
    }
}

impl Render for CoinGeckoSection {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Palette::current(&self.app, cx);
        let configured = !self.editing && self.view.key.is_some();
        let group = parts::group(&self.app, cx);
        let group = if configured {
            group.child(self.credential(cx).render(&self.app, cx)).child(self.status(p, cx))
        } else {
            group
                .child(
                    div().px_3().pt_2().child(
                        Link::new("coingecko-get-key")
                            .href(DASHBOARD)
                            .text_xs()
                            .text_color(p.muted)
                            .whitespace_nowrap()
                            .child(t!("settings.onchain.getKey")),
                    ),
                )
                .child(self.credential(cx).render(&self.app, cx))
        };
        let mut section = parts::section(t!("settings.onchain.title"), &self.app, cx).child(group);
        if let Some(error) = self.error.clone() {
            section = section.child(parts::problem("coingecko-error", error));
        }
        if !configured {
            section = section.child(
                div()
                    .px_1()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .text_xs()
                    .text_color(p.muted)
                    .child(
                        div()
                            .whitespace_nowrap()
                            .overflow_hidden()
                            .child(t!("settings.onchain.footnote")),
                    )
                    .child(
                        div()
                            .whitespace_nowrap()
                            .overflow_hidden()
                            .child(t!("settings.onchain.footnote2")),
                    ),
            );
        }
        section
    }
}

/// The plan, its limits and what they mean for the chart, as three short lines
/// that fit the settings window without wrapping.
fn access_lines(access: &Access, locale: Locale) -> [String; 3] {
    match access {
        Access::Demo => [
            t!("settings.onchain.demo").to_owned(),
            t!("settings.onchain.demoLimits").to_owned(),
            t!("settings.onchain.paceDemo").to_owned(),
        ],
        Access::Paid(plan) => [
            t!("settings.onchain.paid", plan = plan.name.as_str()),
            t!(
                "settings.onchain.paidLimits",
                per_minute = count(plan.per_minute, locale),
                remaining = count(plan.remaining, locale),
                monthly = count(plan.monthly, locale)
            ),
            t!("settings.onchain.pacePaid").to_owned(),
        ],
    }
}

/// A call count with grouped thousands; English, Simplified Chinese and
/// Japanese all group them with commas.
fn count(value: u32, _: Locale) -> String {
    format::price(f64::from(value), 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Restore the process-wide language even when an assertion fails.
    struct Speaks(Locale);
    impl Drop for Speaks {
        fn drop(&mut self) {
            i18n::set(self.0);
        }
    }

    /// A paid plan as CoinGecko's `/key` describes it; its type is only ever
    /// made by that response, so the test makes it the same way.
    fn plan(json: &str) -> Access {
        Access::Paid(serde_json::from_str(json).expect("a /key plan"))
    }

    #[test]
    fn access_lines_name_the_plan_its_limits_and_the_pace() {
        let _language = Speaks(i18n::current());
        let paid = plan(
            r#"{"plan":"Analyst","rate_limit_request_per_minute":500,
                "monthly_call_credit":500000,"current_remaining_monthly_calls":499998}"#,
        );
        let cases: [(Locale, [&str; 3], [&str; 3]); 3] = [
            (
                Locale::En,
                [
                    "Demo plan",
                    "100 calls a minute · 10,000 a month",
                    "Candles load reliably; trades refresh as without a key.",
                ],
                [
                    "Analyst plan",
                    "500 calls a minute · 499,998 of 500,000 left this month",
                    "Trades refresh every 10 s, the day's range every 5 min.",
                ],
            ),
            (
                Locale::ZhCn,
                [
                    "Demo 套餐",
                    "每分钟 100 次 · 每月 10,000 次",
                    "K 线能稳定加载；成交刷新频率和不填 Key 时相同。",
                ],
                [
                    "Analyst 套餐",
                    "每分钟 500 次 · 本月剩余 499,998 / 500,000",
                    "成交每 10 秒刷新，24 小时区间每 5 分钟。",
                ],
            ),
            (
                Locale::Ja,
                [
                    "Demo プラン",
                    "毎分 100 回 · 毎月 10,000 回",
                    "ローソク足は安定。約定の更新はキーなしと同じ。",
                ],
                [
                    "Analyst プラン",
                    "毎分 500 回 · 今月の残り 499,998 / 500,000",
                    "約定は 10 秒ごと、24 時間レンジは 5 分ごとに更新されます。",
                ],
            ),
        ];
        for (locale, demo, paid_lines) in cases {
            i18n::set(locale);
            assert_eq!(access_lines(&Access::Demo, locale), demo);
            assert_eq!(access_lines(&paid, locale), paid_lines);
        }
    }
}
