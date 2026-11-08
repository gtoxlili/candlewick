//! The holdings window: one number for everything the exchanges hold,
//! how it splits, and the accounts and positions under it.

pub mod summary;

mod allocation;
mod aside;
mod assets;
mod hero;
mod manual;
mod tiles;

#[cfg(all(target_os = "macos", feature = "test-support"))]
pub mod fixtures;

#[cfg(test)]
mod tests;

use crate::{
    api,
    app::App,
    format,
    portfolio::{self, Portfolio},
};
use gpui_kit::{
    AppContext as _, Context, Entity, Hsla, IntoElement, ParentElement, Render, SharedString,
    Styled, Task, UniformListScrollHandle, Window,
    component::{Sizable as _, TitleBar, button::Button, button::ButtonVariants as _},
    div, px,
};

use super::{observe, observe_title, setup, theme::Palette, widgets};

/// Numbers this recent count as live: the dot by the title and at the end of
/// the allocation bar stays lit.
const LIVE_MS: f64 = 20_000.;

/// Which list the aside shows.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Accounts,
    Positions,
}

/// Everything the window draws that only changes when the portfolio, the
/// language or the list controls change, laid out ahead of rendering.
struct Derived {
    shares: Vec<summary::Share>,
    figures: summary::Figures,
    insight: Option<Vec<summary::InsightPart>>,
    rows: std::sync::Arc<Vec<assets::Row>>,
    /// Any asset reports a cost, so the list has a cost column.
    costed: bool,
    small: SmallAssets,
    several: bool,
}

/// The assets folded away under the list.
#[derive(Clone, Copy, Default)]
struct SmallAssets {
    count: usize,
    value: f64,
}

pub struct HoldingsView {
    app: App,
    portfolio: std::sync::Arc<Portfolio>,
    sort: summary::Sort,
    show_small: bool,
    tab: Tab,
    derived: Derived,
    assets_scroll: UniformListScrollHandle,
    live_dot: Entity<allocation::LiveDot>,
    /// The hero number flashes on every new total; `generation` remounts the
    /// animation, `down` picks its color.
    flash: (usize, Option<bool>),
    _observers: Vec<Task<()>>,
    stale_timer: Option<Task<()>>,
    manual: Entity<manual::ManualView>,
    show_manual: bool,
}

impl HoldingsView {
    pub fn new(app: App, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut observers = setup(&app, window, cx);
        observers.push(observe_title(&app, window, cx, |_, _| t!("window.holdings").to_owned()));
        observers.push(observe(&app, crate::app::Topic::Portfolio, cx, |view, cx| {
            let next = api::get_portfolio(&view.app);
            let moved = match (view.portfolio.total, next.total) {
                _ if view.portfolio.valuation.currency != next.valuation.currency
                    || view.portfolio.valuation.rate != next.valuation.rate =>
                {
                    view.flash = (view.flash.0 + 1, None);
                    None
                }
                (Some(before), Some(now)) if before != now => Some(now < before),
                _ => None,
            };
            if let Some(down) = moved {
                view.flash = (view.flash.0 + 1, Some(down));
            }
            view.portfolio = std::sync::Arc::new(next);
            view.manual.update(cx, |manual, cx| {
                manual.portfolio = view.portfolio.clone();
                cx.notify();
            });
            if !view.show_manual {
                view.recompute();
            }
            view.schedule_stale_check(cx);
        }));
        observers.push(observe(&app, crate::app::Topic::Locale, cx, |view, _| {
            view.recompute();
        }));
        let dot_app = app.clone();
        let live_dot = cx.new(|_| allocation::LiveDot::new(dot_app));
        let portfolio = std::sync::Arc::new(api::get_portfolio(&app));
        let manual =
            cx.new(|cx| manual::ManualView::new(app.clone(), portfolio.clone(), window, cx));
        let mut view = Self {
            manual,
            show_manual: false,
            portfolio,
            sort: summary::Sort::Value,
            show_small: false,
            tab: Tab::Accounts,
            derived: Derived {
                shares: Vec::new(),
                figures: summary::Figures {
                    risk: 0.,
                    cash: 0.,
                    positions_pnl: None,
                    positions_change: 0.,
                },
                insight: None,
                rows: std::sync::Arc::new(Vec::new()),
                costed: false,
                small: SmallAssets::default(),
                several: false,
            },
            assets_scroll: UniformListScrollHandle::new(),
            live_dot,
            flash: (0, None),
            app,
            _observers: observers,
            stale_timer: None,
        };
        view.recompute();
        view.schedule_stale_check(cx);
        view
    }

    /// Schedules a single wake-up at `latest + LIVE_MS` so the freshness indicator
    /// transitions from `Live` to `Stale` without a 1 Hz polling loop when idle.
    fn schedule_stale_check(&mut self, cx: &mut Context<Self>) {
        let now = portfolio::now_ms();
        let latest =
            self.portfolio.accounts.iter().filter_map(|account| account.updated).fold(0., f64::max);
        let live = latest > 0. && now - latest < LIVE_MS;
        if self.live_dot.read(cx).live != live {
            self.live_dot.update(cx, |dot, cx| {
                dot.live = live;
                cx.notify();
            });
        }
        self.stale_timer = if live {
            let remaining_ms = (latest + LIVE_MS - now).max(50.);
            let delay = std::time::Duration::from_secs_f64(remaining_ms / 1000.);
            Some(cx.spawn(async move |view, cx| {
                cx.background_executor().timer(delay).await;
                let _ = view.update(cx, |this, cx| {
                    this.schedule_stale_check(cx);
                    cx.notify();
                });
            }))
        } else {
            None
        };
    }

    /// Lays out what rendering only reads. Runs when the portfolio changes,
    /// when the language does, or when the list controls move.
    fn recompute(&mut self) {
        let portfolio = &self.portfolio;
        let total = portfolio.total.unwrap_or(0.);
        let listed = summary::listed(
            &portfolio.assets,
            self.sort,
            self.show_small,
            portfolio.valuation.rate,
        );
        let several = portfolio.accounts.len() > 1 || !portfolio.manual_holdings.is_empty();
        let series: std::collections::HashMap<&str, summary::Series> =
            summary::series(&portfolio.assets)
                .into_iter()
                .zip(portfolio.assets.iter())
                .map(|(series, asset)| (asset.id.as_str(), series))
                .collect();
        let widest_move = portfolio
            .assets
            .iter()
            .map(|asset| asset.change.unwrap_or(0.).abs())
            .fold(0., f64::max)
            .max(1.);
        let mut rows = Vec::with_capacity(listed.shown.len());
        for asset in &listed.shown {
            let series = series.get(asset.id.as_str()).copied().unwrap_or(summary::Series::Rest);
            rows.push(assets::Row::new(
                asset,
                series,
                total,
                widest_move,
                self.sort,
                several,
                portfolio.valuation.currency.decimals(),
            ));
        }
        self.derived = Derived {
            shares: summary::allocation(&portfolio.assets, total),
            figures: summary::figures(portfolio),
            insight: summary::insight(portfolio),
            rows: std::sync::Arc::new(rows),
            costed: portfolio
                .assets
                .iter()
                .any(|asset| asset.cost.is_some() || asset.pnl.is_some()),
            small: SmallAssets {
                count: listed.small.len(),
                value: listed.small.iter().map(|asset| asset.value.unwrap_or(0.)).sum(),
            },
            several,
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
            let exchange = aside::provider_name(failed.exchange);
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
            Button::new("manage-manual")
                .ghost()
                .small()
                .label(if self.show_manual {
                    t!("holdings.manual.back")
                } else {
                    t!("holdings.manual.title")
                })
                .on_click(cx.listener(|this, _, _, cx| {
                    this.show_manual = !this.show_manual;
                    if !this.show_manual {
                        this.recompute();
                    }
                    cx.notify();
                })),
        );
        bar = bar.child(
            Button::new("manage-keys").ghost().small().label(t!("holdings.manageKeys")).on_click(
                move |_, _, cx| {
                    let _ = crate::window::open_settings(&app);
                    cx.stop_propagation();
                },
            ),
        );

        let body = if self.show_manual {
            div().flex().flex_1().min_h_0().child(self.manual.clone())
        } else if self.portfolio.accounts.is_empty() && self.portfolio.manual_holdings.is_empty() {
            empty_portfolio(self.app.clone(), cx.entity(), palette)
        } else {
            let view = cx.entity();
            let toggle_small = {
                let view = view.clone();
                move |cx: &mut gpui_kit::App| {
                    view.update(cx, |view, cx| {
                        view.show_small = !view.show_small;
                        view.recompute();
                        cx.notify();
                    });
                }
            };
            let flash = self.flash;
            let derived = &self.derived;
            let live = self
                .portfolio
                .accounts
                .iter()
                .filter_map(|account| account.updated)
                .fold(0., f64::max);
            let live = live > 0. && now - live < LIVE_MS;
            if self.live_dot.read(cx).live != live {
                self.live_dot.update(cx, |dot, cx| {
                    dot.live = live;
                    cx.notify();
                });
            }
            div()
                .flex()
                .flex_1()
                .min_h_0()
                .child(
                    div()
                        .flex()
                        .flex_1()
                        .min_w_0()
                        .flex_col()
                        .gap(px(12.))
                        .pb(px(16.))
                        .child(hero::hero(&self.portfolio, &derived.insight, flash, palette))
                        .children((!self.portfolio.manual_holdings.is_empty()).then(|| {
                            div().px(px(16.)).text_size(px(11.)).text_color(palette.muted).child(
                                t!(
                                    "holdings.manual.summary",
                                    count = self.portfolio.manual_holdings.len()
                                ),
                            )
                        }))
                        .child(allocation::allocation(&derived.shares, &self.live_dot, palette))
                        .child(sort_tabs(self.sort, view.clone(), palette))
                        .child(assets::panel(
                            &derived.rows,
                            derived.costed,
                            derived.small.count,
                            derived.small.value,
                            self.portfolio.valuation.currency,
                            self.show_small,
                            &self.assets_scroll,
                            palette,
                            toggle_small,
                        ))
                        .child(tiles::tiles(&self.portfolio, &derived.figures, palette)),
                )
                .child(aside::aside(&self.portfolio, self.tab, derived.several, view, palette))
        };

        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(super::theme::glass(cx))
            .text_color(palette.foreground)
            .child(bar)
            .children(self.portfolio.manual_error.clone().map(|error| {
                div().px(px(16.)).text_size(px(11.)).text_color(palette.red).child(error)
            }))
            .children(self.portfolio.partial.then(|| {
                div()
                    .px(px(16.))
                    .text_size(px(11.))
                    .text_color(palette.amber)
                    .child(t!("holdings.manual.partial"))
            }))
            .child(body)
    }
}

/// The sort control above the list.
fn sort_tabs(
    sort: summary::Sort,
    view: gpui_kit::Entity<HoldingsView>,
    palette: Palette,
) -> gpui_kit::Div {
    let _ = palette;
    div().h(px(28.)).flex().items_center().gap(px(8.)).px(px(16.)).child(widgets::pill_tabs(
        "sort",
        [SharedString::from(t!("holdings.byValue")), SharedString::from(t!("holdings.byChange"))],
        if sort == summary::Sort::Value { 0 } else { 1 },
        move |index, _, cx| {
            view.update(cx, |view, cx| {
                view.sort = if *index == 0 { summary::Sort::Value } else { summary::Sort::Change };
                view.recompute();
                cx.notify();
            });
        },
    ))
}

/// Neither a synced account nor a registered holding exists.
fn empty_portfolio(app: App, view: Entity<HoldingsView>, palette: Palette) -> gpui_kit::Div {
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
                        .child(t!("holdings.emptyTitle")),
                )
                .child(
                    div()
                        .w(px(320.))
                        .text_size(px(12.))
                        .text_color(palette.muted)
                        .child(t!("holdings.emptyDetail")),
                ),
        )
        .child(Button::new("add-manual").label(t!("holdings.manual.add")).small().on_click(
            move |_, _, cx| {
                view.update(cx, |this, cx| {
                    this.show_manual = true;
                    cx.notify();
                })
            },
        ))
        .child(
            Button::new("open-settings")
                .ghost()
                .label(t!("holdings.openSettings"))
                .small()
                .on_click(move |_, _, _| {
                    let _ = crate::window::open_settings(&app);
                }),
        )
}

/// The color a series of the allocation is drawn with, in the list's share
/// bars and the legend alike.
pub(crate) fn series_color(series: summary::Series, palette: Palette) -> Hsla {
    let index = match series {
        summary::Series::One => 0,
        summary::Series::Two => 1,
        summary::Series::Three => 2,
        summary::Series::Four => 3,
        summary::Series::Five => 4,
        summary::Series::Rest => 5,
        summary::Series::Cash => 6,
    };
    palette.series[index]
}

/// Linear blend between two colors in RGBA space, for the hero number's flash
/// fading back into the text color without hue-rainbowing.
pub(crate) fn mix(from: Hsla, to: Hsla, t: f32) -> Hsla {
    crate::ui::mix_color(from, to, f64::from(t))
}
