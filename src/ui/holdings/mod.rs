//! The holdings window: one number for everything the exchanges hold,
//! how it splits, and the accounts and positions under it.

pub mod summary;

mod hero;
mod tiles;

use crate::{
    api,
    app::App,
    format,
    portfolio::{self, Portfolio},
};
use gpui_kit::{
    Context, Hsla, IntoElement, ParentElement, Render, Styled, Task, Window,
    component::{Sizable as _, TitleBar, button::Button, button::ButtonVariants as _},
    div, px,
};

use super::{observe, observe_title, setup, theme::Palette, widgets};

/// Numbers this recent count as live: the dot by the title and at the end of
/// the allocation bar stays lit.
const LIVE_MS: f64 = 20_000.;

/// Everything the window draws that only changes when the portfolio or the
/// language changes, laid out ahead of rendering.
struct Derived {
    figures: summary::Figures,
    insight: Option<Vec<summary::InsightPart>>,
}

pub struct HoldingsView {
    app: App,
    portfolio: Portfolio,
    derived: Derived,
    /// The hero number flashes on every new total; `generation` remounts the
    /// animation, `down` picks its color.
    flash: (usize, Option<bool>),
    _observers: Vec<Task<()>>,
    _ticker: Task<()>,
}

impl HoldingsView {
    pub fn new(app: App, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut observers = setup(&app, window, cx);
        observers.push(observe_title(&app, window, cx, |_, _| t!("window.holdings").to_owned()));
        observers.push(observe(&app, crate::app::Topic::Portfolio, cx, |view, _| {
            let next = api::get_portfolio(&view.app);
            let moved = match (view.portfolio.total, next.total) {
                (Some(before), Some(now)) if before != now => Some(now < before),
                _ => None,
            };
            if let Some(down) = moved {
                view.flash = (view.flash.0 + 1, Some(down));
            }
            view.portfolio = next;
            view.recompute();
        }));
        observers.push(observe(&app, crate::app::Topic::Locale, cx, |view, _| {
            view.recompute();
        }));
        // Freshness is a matter of time, not only of data: the live dot goes
        // out and the clock moves on while nothing else changes.
        let ticker = cx.spawn(async move |view, cx| {
            loop {
                cx.background_executor().timer(std::time::Duration::from_secs(1)).await;
                if view.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
            }
        });
        let mut view = Self {
            portfolio: api::get_portfolio(&app),
            derived: Derived {
                figures: summary::Figures {
                    risk: 0.,
                    cash: 0.,
                    positions_pnl: None,
                    positions_change: 0.,
                },
                insight: None,
            },
            flash: (0, None),
            app,
            _observers: observers,
            _ticker: ticker,
        };
        view.recompute();
        view
    }

    /// Lays out what rendering only reads. Runs when the portfolio or the
    /// language changes.
    fn recompute(&mut self) {
        let portfolio = &self.portfolio;
        self.derived = Derived {
            figures: summary::figures(portfolio),
            insight: summary::insight(portfolio),
        };
    }

    /// The state of the numbers: fresh and moving, being read, failed, or
    /// past their time.
    fn freshness(
        accounts: &[crate::portfolio::AccountView],
        now: f64,
    ) -> (widgets::Status, String) {
        let failed = accounts.iter().find(|account| account.error.is_some());
        let loading =
            accounts.iter().any(|account| account.updated.is_none() && account.error.is_none());
        let latest = accounts.iter().filter_map(|account| account.updated).fold(0., f64::max);
        if let Some(failed) = failed {
            let exchange = provider_name(failed.exchange);
            return (widgets::Status::Error, t!("common.holdings.readFailed", exchange = exchange));
        }
        if loading {
            return (widgets::Status::Busy, t!("common.holdings.reading").to_owned());
        }
        if latest > 0. {
            let live = now - latest < LIVE_MS;
            (
                if live { widgets::Status::Live } else { widgets::Status::Stale },
                t!("common.holdings.updated", time = format::clock(latest)),
            )
        } else {
            (widgets::Status::Stale, String::new())
        }
    }
}

impl Render for HoldingsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = Palette::current(&self.app, cx);
        let now = portfolio::now_ms();
        let mut bar = TitleBar::new()
            .bg(gpui_kit::component::Theme::global(cx).transparent)
            .child(
                div()
                    .text_size(px(13.))
                    .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                    .whitespace_nowrap()
                    .child(t!("holdings.title")),
            )
            .child(div().flex_1());
        if !self.portfolio.accounts.is_empty() {
            let (status, text) = Self::freshness(&self.portfolio.accounts, now);
            bar = bar.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .rounded_full()
                    .px_2()
                    .text_size(px(12.))
                    .text_color(palette.muted)
                    .child(widgets::status_dot(status, palette))
                    .child(super::tabular(div().whitespace_nowrap().child(text))),
            );
        }
        let app = self.app.clone();
        bar = bar.child(
            Button::new("manage-keys").ghost().small().label(t!("holdings.manageKeys")).on_click(
                move |_, _, cx| {
                    let _ = crate::window::open_settings(&app);
                    cx.stop_propagation();
                },
            ),
        );

        let body = if self.portfolio.accounts.is_empty() {
            no_keys(self.app.clone(), palette)
        } else {
            div().flex().flex_1().min_h_0().flex_col().gap(px(12.)).pb(px(16.)).child(
                hero::hero(&self.portfolio, &self.derived.insight, self.flash, palette),
            ).child(div().flex_1()).child(tiles::tiles(&self.portfolio, &self.derived.figures, palette))
        };

        div()
            .size_full()
            .flex()
            .flex_col()
            .text_color(palette.foreground)
            .child(bar)
            .child(body)
    }
}

/// Nothing to show because no exchange has a key yet.
fn no_keys(app: App, palette: Palette) -> gpui_kit::Div {
    div()
        .flex()
        .flex_1()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(16.))
        .px(px(32.))
        .pb(px(32.))
        .text_center()
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(6.))
                .items_center()
                .child(
                    div()
                        .text_size(px(15.))
                        .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                        .child(t!("holdings.noKeys")),
                )
                .child(
                    div()
                        .w(px(320.))
                        .text_size(px(12.))
                        .text_color(palette.muted)
                        .child(t!("holdings.noKeysDetail")),
                ),
        )
        .child(Button::new("open-settings").label(t!("holdings.openSettings")).small().on_click(
            move |_, _, _| {
                let _ = crate::window::open_settings(&app);
            },
        ))
}

/// The exchange's name in the language the app speaks.
fn provider_name(provider: crate::market::ProviderId) -> gpui_kit::SharedString {
    use crate::market::ProviderId;
    match provider {
        ProviderId::Binance => t!("common.provider.binance"),
        ProviderId::Bybit => t!("common.provider.bybit"),
        ProviderId::Okx => t!("common.provider.okx"),
        ProviderId::Longbridge => t!("common.provider.longbridge"),
        ProviderId::Ths => t!("common.provider.ths"),
        ProviderId::DexScreener => t!("common.provider.dexscreener"),
    }
    .into()
}

/// Linear blend between two colors, for the hero number's flash fading back
/// into the text color.
pub(crate) fn mix(from: Hsla, to: Hsla, t: f32) -> Hsla {
    Hsla {
        h: from.h + (to.h - from.h) * t,
        s: from.s + (to.s - from.s) * t,
        l: from.l + (to.l - from.l) * t,
        a: from.a + (to.a - from.a) * t,
    }
}
