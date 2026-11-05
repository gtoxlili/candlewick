//! Header: read the instrument, spec, market and mode; use model commands for choices and links.

use std::time::Duration;

use super::{
    market::{Market, VisibleStats},
    model_state::ChartModel,
};
use crate::{
    api, format,
    market::{Book, FeedState, Onchain, ProviderId, StatsSpan},
    model::Instrument,
    ui::{prefs::Mode, tabular, theme::Palette, widgets},
};
use gpui_kit::{
    Animation, AnimationExt, Context, Entity, FontWeight, Hsla, InteractiveElement, IntoElement,
    ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, TestSupportExt as _,
    WeakEntity, Window, div, px,
};
use gpui_kit::{
    assets::IconName,
    base::Disableable as _,
    component::{
        ActiveTheme as _, Icon, Sizable as _,
        button::{Button, ButtonVariants as _},
        popover::{Popover, PopoverState},
    },
    prelude::FluentBuilder as _,
};

/// What the change covers, as the badge appends it: `24h`, `today`.
fn span_label(span: StatsSpan) -> &'static str {
    match span {
        StatsSpan::Rolling24h => t!("chart.span_rolling24h"),
        StatsSpan::Today => t!("chart.span_today"),
    }
}

/// The change as an amount, signed by the direction of the percentage, as
/// the badge shows it: `+1,024.50`, `−345.67`, `0.00`.
fn change_amount(change_pct: f64, change: f64, decimals: usize) -> String {
    let magnitude = format::price(change.abs(), decimals);
    match format::direction(change_pct) {
        format::Direction::Up => format!("+{magnitude}"),
        format::Direction::Down => format!("\u{2212}{magnitude}"),
        format::Direction::Flat => magnitude,
    }
}

/// `+1.23% · +1,024.50 24h`: the badge line under the price.
fn badge_label(change_pct: f64, change: f64, decimals: usize, span: &str) -> String {
    format!(
        "{} · {} {}",
        format::change_signed(change_pct),
        change_amount(change_pct, change, decimals),
        span,
    )
}

/// Share of the book's size on the bid side, 0–100.
fn bid_share(book: Option<&Book>) -> Option<f64> {
    let book = book?;
    let bids: f64 = book.bids.iter().map(|level| level.qty).sum();
    let asks: f64 = book.asks.iter().map(|level| level.qty).sum();
    (bids + asks > 0.).then(|| bids / (bids + asks) * 100.)
}

/// Share of the day's swaps that bought the token, 0–100.
fn buy_share(onchain: Option<&Onchain>) -> Option<f64> {
    let onchain = onchain?;
    let total = f64::from(onchain.buys + onchain.sells);
    (total > 0.).then(|| f64::from(onchain.buys) / total * 100.)
}

/// One readable line about the stretch on screen, plus, while live, the
/// order book's pressure, or for a pool which way the day's swaps lean.
fn insight_line(
    visible: Option<VisibleStats>,
    book: Option<&Book>,
    onchain: Option<&Onchain>,
) -> Option<String> {
    let stats = visible?;
    let stretch = if stats.live {
        t!("chart.lastSpan", span = format::duration(stats.span)).to_owned()
    } else {
        format::range(stats.from, stats.to)
    };
    let mut parts = vec![
        format!("{stretch} {}", format::change_signed(stats.change_pct)),
        t!("chart.amplitude", value = format!("{:.2}", stats.amplitude_pct)).to_owned(),
    ];
    if stats.live {
        if let Some(share) = bid_share(book) {
            parts.push(if share >= 55. {
                t!("chart.bidsLead", share = format!("{share:.0}")).to_owned()
            } else if share <= 45. {
                t!("chart.asksLead", share = format!("{:.0}", 100. - share)).to_owned()
            } else {
                t!("chart.balanced").to_owned()
            });
        }
        if let Some(share) = buy_share(onchain) {
            parts.push(if share >= 55. {
                t!("chart.buyersLead", share = format!("{share:.0}")).to_owned()
            } else if share <= 45. {
                t!("chart.sellersLead", share = format!("{:.0}", 100. - share)).to_owned()
            } else {
                t!("chart.flowBalanced").to_owned()
            });
        }
    }
    Some(parts.join(" · "))
}

use crate::ui::ease_out;

/// Between two colors in RGBA space; used to fade the price flash away without hue-rainbowing.
fn mix(from: Hsla, to: Hsla, t: f32) -> Hsla {
    crate::ui::mix_color(from, to, f64::from(t))
}

/// The instrument as the title bar names it: the short name, the dimmed
/// part after it, and where a pool trades.
struct InstrumentName {
    name: String,
    detail: String,
    venue: Option<String>,
}

impl InstrumentName {
    fn of(instrument: &Instrument) -> Self {
        if instrument.provider == ProviderId::DexScreener {
            let venue = instrument.venue.as_ref().map(|venue| {
                let mut parts = vec![venue.chain.clone()];
                if let Some(dex) = &venue.dex {
                    parts.push(dex.clone());
                }
                parts.join(" · ")
            });
            return Self {
                name: instrument.base.clone(),
                detail: instrument.name.as_ref().map(|name| format!(" {name}")).unwrap_or_default(),
                venue,
            };
        }
        if instrument.provider.is_exchange() {
            return Self {
                name: instrument.base.clone(),
                detail: format!("/{}", instrument.quote),
                venue: None,
            };
        }
        // Stocks: US tickers read better than their names, other codes are
        // just numbers.
        let name = instrument.name.clone().unwrap_or_default();
        if instrument.symbol.ends_with(".US") || name.is_empty() {
            Self {
                name: instrument.base.clone(),
                detail: if name.is_empty() { String::new() } else { format!(" {name}") },
                venue: None,
            }
        } else {
            Self { name, detail: format!(" {}", instrument.base), venue: None }
        }
    }
}

pub struct Hero {
    pub model: Entity<ChartModel>,
}

impl Render for Hero {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let app = self.model.read(cx).app().clone();
        let palette = Palette::current(&app, cx);
        let model = self.model.read(cx);
        let stats = model.market().and_then(Market::stats);
        let decimals = format::decimals(
            stats.map_or(0., |stats| stats.last),
            model.instrument().and_then(|instrument| instrument.decimals),
        );
        let span = model.spec().map_or(StatsSpan::Rolling24h, |spec| spec.stats_span);

        let price: gpui_kit::AnyElement = match stats {
            Some(stats) => {
                let price = tabular(
                    div()
                        .text_size(px(34.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .line_height(px(34.))
                        .whitespace_nowrap()
                        .child(format::price(stats.last, decimals)),
                );
                let direction = model.last_direction();
                match direction {
                    -1 | 1 => {
                        let color = if direction < 0 { palette.down } else { palette.up };
                        price
                            .with_animation(
                                // The key carries the price, so every new
                                // price replays the flash and ends at rest.
                                SharedString::from(format!(
                                    "price-flash-{direction}-{}",
                                    stats.last
                                )),
                                Animation::new(Duration::from_secs(1)).with_easing(ease_out),
                                move |element, delta| {
                                    element.text_color(mix(color, palette.foreground, delta))
                                },
                            )
                            .into_any_element()
                    }
                    _ => price.text_color(palette.foreground).into_any_element(),
                }
            }
            None => {
                let skeleton = div().w(px(192.)).h(px(34.)).rounded(px(8.)).bg(palette.fill_strong);
                skeleton
                    .with_animation(
                        "price-skeleton",
                        Animation::new(Duration::from_millis(1200)).repeat(),
                        |element, delta| {
                            element.opacity(0.55 + 0.45 * (1. - (2. * delta - 1.).abs()))
                        },
                    )
                    .into_any_element()
            }
        };

        let badge = stats.map(|stats| {
            // A change that rounds to 0.00% reads as flat, never colored.
            let color_by = if format::direction(stats.change_pct) == format::Direction::Flat {
                0.
            } else {
                stats.change_pct
            };
            widgets::change_badge(
                badge_label(stats.change_pct, stats.change, decimals, span_label(span)),
                color_by,
                palette,
            )
            .text_size(px(12.))
            .py(px(2.))
            .font_weight(FontWeight::MEDIUM)
        });

        let has_book = model.spec().is_none_or(|spec| spec.book);
        let book = has_book
            .then(|| {
                let books = model.market().map(Market::books).unwrap_or_default();
                let step = model.book_step().min(books.len().saturating_sub(1));
                books.get(step)
            })
            .flatten();
        let onchain = (!has_book).then(|| stats.and_then(|stats| stats.onchain)).flatten();
        let insight = insight_line(model.visible_stats(), book, onchain.as_ref());

        div()
            .id("Hero")
            .size_full()
            .px(px(20.))
            .pt(px(8.))
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(div().flex().items_baseline().gap(px(12.)).child(price).children(badge))
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .h(px(16.))
                    .text_size(px(12.))
                    .text_color(palette.muted)
                    .whitespace_nowrap()
                    .overflow_hidden()
                    .children(insight),
            )
    }
}

pub struct ModeToggle {
    pub model: Entity<ChartModel>,
}

impl Render for ModeToggle {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let app = self.model.read(cx).app().clone();
        let palette = Palette::current(&app, cx);
        let selected = cx.theme().colors.background;
        let model = self.model.read(cx);
        let mode = model.mode();
        let entity = self.model.clone();
        let button = |line: bool| {
            let (id, icon, label, picked) = if line {
                ("chart-mode-line", IconName::ChartLine, t!("chart.line"), mode == Mode::Line)
            } else {
                (
                    "chart-mode-candle",
                    IconName::ChartCandlestick,
                    t!("chart.candles"),
                    mode == Mode::Candle,
                )
            };
            let entity = entity.clone();
            let next = if line { Mode::Line } else { Mode::Candle };
            Button::new(id)
                .ghost()
                .small()
                .icon(Icon::new(icon))
                .tooltip(label)
                .accessibility_label(label)
                .w(px(32.))
                .h(px(24.))
                .rounded(px(12.))
                .when(picked, |button| button.bg(selected).text_color(palette.foreground))
                .when(!picked, |button| button.text_color(palette.muted))
                .on_click(move |_, _, cx| {
                    entity.update(cx, |model, cx| model.set_mode(next, cx));
                })
        };
        div().id("ModeToggle").size_full().flex().items_center().justify_end().child(
            div()
                .flex()
                .p(px(2.))
                .rounded_full()
                .bg(palette.fill_strong)
                .child(button(true))
                .child(button(false)),
        )
    }
}

pub struct LiveBadge {
    pub model: Entity<ChartModel>,
}

impl Render for LiveBadge {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let app = self.model.read(cx).app().clone();
        let palette = Palette::current(&app, cx);
        let model = self.model.read(cx);
        let state = model.market().map_or(FeedState::Connecting, Market::feed);
        let (status, label) = match state {
            FeedState::Live => (widgets::Status::Live, t!("chart.live")),
            FeedState::Connecting => (widgets::Status::Busy, t!("chart.connecting")),
            FeedState::Offline => (widgets::Status::Error, t!("chart.reconnecting")),
        };
        let dot = if state == FeedState::Live {
            // A ring swells out of the dot with each new price and rests until
            // the next one; a repeating ring would keep the whole window
            // redrawing every frame while the feed is idle.
            let price = model.market().and_then(|market| market.stats()).map(|stats| stats.last);
            let ring = div().absolute().rounded_full().bg(palette.mint).with_animation(
                SharedString::from(format!("live-ping-{price:?}")),
                Animation::new(Duration::from_millis(1200)),
                |ring, delta| {
                    ring.size(px(8. + 8. * delta))
                        .top(px(-4. * delta))
                        .left(px(-4. * delta))
                        .opacity(0.6 * (1. - delta))
                },
            );
            div().relative().size(px(8.)).child(ring).child(
                div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(widgets::status_dot(status, palette)),
            )
        } else {
            div()
                .size(px(8.))
                .flex()
                .items_center()
                .justify_center()
                .child(widgets::status_dot(status, palette))
        };
        div()
            .id("LiveBadge")
            .size_full()
            .flex()
            .items_center()
            .justify_end()
            .gap(px(6.))
            .text_size(px(12.))
            .text_color(palette.muted)
            .whitespace_nowrap()
            .child(dot)
            .child(label)
    }
}

pub struct InstrumentPicker {
    pub model: Entity<ChartModel>,
}

impl Render for InstrumentPicker {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let app = self.model.read(cx).app().clone();
        let palette = Palette::current(&app, cx);
        let model = self.model.read(cx);
        let instrument = model.instrument().cloned();
        let current = instrument.as_ref().map(Instrument::id);

        let mut watchlist = api::get_settings(model.app()).watchlist;
        if let Some(instrument) = &instrument
            && !watchlist.iter().any(|entry| entry.id() == instrument.id())
        {
            watchlist.push(instrument.clone());
        }

        let name = instrument.as_ref().map(InstrumentName::of);
        let label = div()
            .flex()
            .items_baseline()
            .gap_1()
            .min_w_0()
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_size(px(13.))
                    .whitespace_nowrap()
                    .child(name.as_ref().map(|name| name.name.clone()).unwrap_or_default()),
            )
            .child(
                div()
                    .text_color(palette.muted)
                    .text_size(px(13.))
                    .whitespace_nowrap()
                    .child(name.as_ref().map(|name| name.detail.clone()).unwrap_or_default()),
            )
            .children(name.as_ref().and_then(|name| {
                name.venue.clone().map(|venue| {
                    div()
                        .ml(px(8.))
                        .text_color(palette.muted)
                        .text_size(px(12.))
                        .whitespace_nowrap()
                        .child(venue)
                })
            }));

        if watchlist.len() < 2 {
            return div()
                .id("InstrumentPicker")
                .size_full()
                .flex()
                .items_center()
                .child(div().h(px(24.)).px(px(8.)).flex().items_center().child(label));
        }

        let entries: Vec<(String, InstrumentName)> =
            watchlist.iter().map(|entry| (entry.id(), InstrumentName::of(entry))).collect();
        let entries = std::rc::Rc::new(entries);
        let current = current.unwrap_or_default();
        let entity = self.model.clone();

        let picker = Popover::new("chart-instrument")
            // The list sits 4px from the edge, as a select's does: the
            // popover's own 12px would put it 16px in.
            .p(px(4.))
            .trigger(
                Button::new("chart-instrument-trigger")
                    .ghost()
                    .compact()
                    .tooltip(t!("chart.switch"))
                    .accessibility_label(t!("chart.switch"))
                    .h(px(24.))
                    .px(px(8.))
                    .rounded(px(12.))
                    .child(label)
                    .child(
                        div()
                            .ml(px(4.))
                            .text_color(palette.muted)
                            .child(Icon::new(IconName::ChevronsUpDown).size(px(12.))),
                    ),
            )
            .content(move |_, _, cx| {
                let popover: WeakEntity<PopoverState> = cx.entity().downgrade();
                let rows = entries.iter().enumerate().map(|(index, (id, name))| {
                    let entity = entity.clone();
                    let pick = id.clone();
                    let selected = *id == current;
                    let popover = popover.clone();
                    div()
                        .id(SharedString::from(format!("instrument-{index}")))
                        .px(px(8.))
                        .py(px(4.))
                        .rounded(px(6.))
                        .flex()
                        .items_baseline()
                        .gap_1()
                        .whitespace_nowrap()
                        .when(selected, |row| row.bg(palette.fill_strong))
                        .when(!selected, |row| row.hover(move |row| row.bg(palette.fill)))
                        .child(
                            div()
                                .font_weight(if selected {
                                    FontWeight::SEMIBOLD
                                } else {
                                    FontWeight::NORMAL
                                })
                                .child(name.name.clone()),
                        )
                        .child(div().text_color(palette.muted).child(name.detail.clone()))
                        .on_click(move |_, window, cx| {
                            let _ = popover.update(cx, |state, cx| state.dismiss(window, cx));
                            entity.update(cx, |model, _| model.select_instrument(pick.clone()));
                        })
                });
                div()
                    .id("chart-instrument-list")
                    .test_support()
                    .flex()
                    .flex_col()
                    .min_w(px(180.))
                    .children(rows)
            });
        div().id("InstrumentPicker").size_full().flex().items_center().child(picker)
    }
}

/// The title bar's link to the instrument's page on its provider's site.
pub(super) fn link_button(model: &Entity<ChartModel>, cx: &gpui_kit::App) -> Button {
    let model_ref = model.read(cx);
    let label =
        model_ref.spec().map(|spec| t!("chart.openOn", source = spec.source.name()).to_owned());
    let enabled = model_ref.spec().is_some_and(|spec| spec.link.is_some());
    let entity = model.clone();
    Button::new("chart-open-link")
        .ghost()
        .small()
        .disabled(!enabled)
        .accessibility_label(label.clone().unwrap_or_default())
        .tooltip(label.unwrap_or_default())
        .child(Icon::new(IconName::ExternalLink).size(px(14.)))
        .on_click(move |_, _, cx| {
            entity.update(cx, |model, _| model.open_link());
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::market::Level;

    #[test]
    fn interval_labels_use_the_largest_whole_unit() {
        use crate::ui::chart::interval_label;

        assert_eq!(interval_label(60), "1m");
        assert_eq!(interval_label(300), "5m");
        assert_eq!(interval_label(3_600), "1h");
        assert_eq!(interval_label(14_400), "4h");
        assert_eq!(interval_label(86_400), "1D");
        assert_eq!(interval_label(1_209_600), "2W");
        // Not a whole minute: seconds.
        assert_eq!(interval_label(45), "45s");
        assert_eq!(interval_label(90), "90s");
    }

    #[test]
    fn badge_reads_percent_then_amount_then_span() {
        assert_eq!(badge_label(1.23, 1024.5, 2, "24h"), "+1.23% · +1,024.50 24h");
        assert_eq!(badge_label(-0.41, -345.67, 2, "today"), "−0.41% · −345.67 today");
        // The sign comes from the percentage: an amount that rounds to zero
        // takes none of its own.
        assert_eq!(badge_label(-0.004, -0.001, 2, "24h"), "0.00% · 0.00 24h");
        assert_eq!(badge_label(1.0, 0.004, 2, "24h"), "+1.00% · +0.00 24h");
        assert_eq!(change_amount(1.23, 1024.5, 2), "+1,024.50");
        assert_eq!(change_amount(-1.23, 1024.5, 2), "−1,024.50");
        assert_eq!(change_amount(0.0, -1024.5, 2), "1,024.50");
    }

    #[test]
    fn instrument_names_follow_the_provider() {
        let exchange = Instrument {
            provider: ProviderId::Binance,
            symbol: "BTCUSDT".into(),
            base: "BTC".into(),
            quote: "USDT".into(),
            name: None,
            decimals: Some(2),
            pinned: false,
            venue: None,
        };
        let name = InstrumentName::of(&exchange);
        assert_eq!((name.name.as_str(), name.detail.as_str(), name.venue), ("BTC", "/USDT", None));

        let pool = Instrument {
            provider: ProviderId::DexScreener,
            symbol: "pool".into(),
            base: "$WIF".into(),
            quote: "SOL".into(),
            name: Some("dogwifhat".into()),
            decimals: None,
            pinned: false,
            venue: Some(crate::model::Venue {
                chain: "Solana".into(),
                dex: Some("Raydium".into()),
            }),
        };
        let name = InstrumentName::of(&pool);
        assert_eq!(name.name, "$WIF");
        assert_eq!(name.detail, " dogwifhat");
        assert_eq!(name.venue.as_deref(), Some("Solana · Raydium"));

        let us = Instrument {
            provider: ProviderId::Longbridge,
            symbol: "AAPL.US".into(),
            base: "AAPL".into(),
            quote: "USD".into(),
            name: Some("Apple".into()),
            decimals: Some(2),
            pinned: false,
            venue: None,
        };
        let name = InstrumentName::of(&us);
        assert_eq!((name.name.as_str(), name.detail.as_str()), ("AAPL", " Apple"));

        let hk = Instrument {
            provider: ProviderId::Longbridge,
            symbol: "700.HK".into(),
            base: "700".into(),
            quote: "HKD".into(),
            name: Some("Tencent".into()),
            decimals: Some(2),
            pinned: false,
            venue: None,
        };
        let name = InstrumentName::of(&hk);
        assert_eq!((name.name.as_str(), name.detail.as_str()), ("Tencent", " 700"));
    }

    #[test]
    fn book_pressure_names_the_leading_side() {
        let book = |bids: f64, asks: f64| Book {
            bids: vec![Level { price: 1., qty: bids }],
            asks: vec![Level { price: 2., qty: asks }],
        };
        let live = || {
            Some(VisibleStats {
                live: true,
                span: 3600.,
                from: 0.,
                to: 0.,
                change_pct: 1.23,
                amplitude_pct: 3.68,
            })
        };
        assert_eq!(
            insight_line(live(), Some(&book(60., 40.)), None),
            Some("Last 1 hour +1.23% · Range 3.68% · Bids lead 60%".to_owned())
        );
        assert_eq!(
            insight_line(live(), Some(&book(30., 70.)), None),
            Some("Last 1 hour +1.23% · Range 3.68% · Asks lead 70%".to_owned())
        );
        assert_eq!(
            insight_line(live(), Some(&book(52., 48.)), None),
            Some("Last 1 hour +1.23% · Range 3.68% · Balanced book".to_owned())
        );
        assert_eq!(bid_share(Some(&book(0., 0.))), None);
        assert_eq!(bid_share(None), None);
    }

    #[test]
    fn pool_swaps_lean_the_way_the_day_did() {
        let visible = VisibleStats {
            live: true,
            span: 3600.,
            from: 0.,
            to: 0.,
            change_pct: -2.5,
            amplitude_pct: 5.,
        };
        let onchain = Onchain {
            liquidity: Some(1.2e7),
            market_cap: None,
            fdv: Some(3.4e8),
            buys: 70,
            sells: 30,
        };
        assert_eq!(
            insight_line(Some(visible), None, Some(&onchain)),
            Some("Last 1 hour −2.50% · Range 5.00% · Buyers lead 70%".to_owned())
        );
        let split = Onchain { buys: 45, sells: 55, ..onchain };
        assert_eq!(
            insight_line(Some(visible), None, Some(&split)),
            Some("Last 1 hour −2.50% · Range 5.00% · Sellers lead 55%".to_owned())
        );
        let even = Onchain { buys: 50, sells: 50, ..onchain };
        assert_eq!(
            insight_line(Some(visible), None, Some(&even)),
            Some("Last 1 hour −2.50% · Range 5.00% · Buys and sells even".to_owned())
        );
    }
}
