//! The aside: each exchange's account, or the open derivatives positions.

use crate::{
    format,
    market::ProviderId,
    portfolio::{AccountView, Portfolio, PositionKind, PositionValue, Wallet},
};
use gpui_kit::{
    FontWeight, InteractiveElement as _, ParentElement, SharedString,
    StatefulInteractiveElement as _, Styled, div, px,
};

use super::summary;
use crate::ui::{theme::Palette, widgets};

pub(crate) fn provider_name(provider: ProviderId) -> SharedString {
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

pub(crate) fn wallet_name(wallet: Wallet) -> SharedString {
    match wallet {
        Wallet::Spot => t!("holdings.wallet.spot"),
        Wallet::Trading => t!("holdings.wallet.trading"),
        Wallet::Funding => t!("holdings.wallet.funding"),
        Wallet::Earn => t!("holdings.wallet.earn"),
        Wallet::UsdFutures => t!("holdings.wallet.usdFutures"),
        Wallet::CoinFutures => t!("holdings.wallet.coinFutures"),
    }
    .into()
}

fn kind_name(kind: PositionKind) -> SharedString {
    match kind {
        PositionKind::UsdtPerpetual => t!("holdings.kind.usdtPerpetual"),
        PositionKind::UsdtFutures => t!("holdings.kind.usdtFutures"),
        PositionKind::UsdcPerpetual => t!("holdings.kind.usdcPerpetual"),
        PositionKind::UsdcFutures => t!("holdings.kind.usdcFutures"),
        PositionKind::CoinPerpetual => t!("holdings.kind.coinPerpetual"),
        PositionKind::CoinFutures => t!("holdings.kind.coinFutures"),
        PositionKind::Option => t!("holdings.kind.option"),
        PositionKind::Margin => t!("holdings.kind.margin"),
        PositionKind::Other => t!("holdings.kind.other"),
    }
    .into()
}

/// The aside: a tabs control over the accounts or the positions.
pub fn aside(
    portfolio: &Portfolio,
    tab: super::Tab,
    several: bool,
    view: gpui_kit::Entity<super::HoldingsView>,
    palette: Palette,
) -> gpui_kit::Div {
    let positions_label = if portfolio.positions.is_empty() {
        t!("common.holdings.positions").to_owned()
    } else {
        t!("holdings.positionsCount", n = portfolio.positions.len())
    };
    let tabs = widgets::pill_tabs(
        "aside-tabs",
        [SharedString::from(t!("common.holdings.accounts")), positions_label.into()],
        if tab == super::Tab::Accounts { 0 } else { 1 },
        move |index, _, cx| {
            view.update(cx, |view, cx| {
                view.tab = if *index == 0 { super::Tab::Accounts } else { super::Tab::Positions };
                cx.notify();
            });
        },
    );
    let content = match tab {
        super::Tab::Accounts => accounts(&portfolio.accounts, palette),
        super::Tab::Positions => positions(&portfolio.positions, several, palette),
    };
    div()
        .w(px(280.))
        .flex_none()
        .flex()
        .flex_col()
        .border_l_1()
        .border_color(palette.border)
        .pt(px(8.))
        .child(div().px(px(12.)).pb(px(10.)).child(tabs))
        .child(content)
}

/// Each exchange: its total, what the day did to it, and where it sits.
fn accounts(accounts: &[AccountView], palette: Palette) -> gpui_kit::Stateful<gpui_kit::Div> {
    div().id("accounts").min_h_0().flex_1().overflow_y_scroll().text_size(px(12.)).children(
        accounts.iter().map(|account| {
            let before = match (account.total, account.change) {
                (Some(total), Some(change)) => Some(total - change),
                _ => None,
            };
            let pct = match (before, account.change) {
                (Some(before), Some(change)) if before > 0. => Some(change / before * 100.),
                _ => None,
            };
            let widest =
                account.wallets.iter().map(|wallet| wallet.value).fold(0., f64::max).max(1.);
            let tone = |value: f64| {
                if format::direction(value) == format::Direction::Down {
                    palette.down
                } else {
                    palette.up
                }
            };
            let total = match account.total {
                Some(total) => format::price(total, 2),
                None => t!("common.holdings.reading").to_owned(),
            };
            let updated = match account.updated {
                Some(ms) => t!("common.holdings.updated", time = format::clock(ms)),
                None => t!("common.holdings.reading").to_owned(),
            };
            let mut section = div()
                .px(px(12.))
                .py(px(10.))
                .flex()
                .flex_col()
                .border_b_1()
                .border_color(palette.border)
                .child(
                    div()
                        .flex()
                        .items_baseline()
                        .justify_between()
                        .gap(px(8.))
                        .child(
                            div()
                                .font_weight(FontWeight::MEDIUM)
                                .child(provider_name(account.exchange)),
                        )
                        .child(crate::ui::tabular(
                            div().font_weight(FontWeight::MEDIUM).whitespace_nowrap().child(total),
                        )),
                )
                .child(
                    div()
                        .mt(px(2.))
                        .flex()
                        .items_baseline()
                        .justify_between()
                        .gap(px(8.))
                        .text_size(px(10.))
                        .text_color(palette.muted)
                        .child(crate::ui::tabular(div().whitespace_nowrap().child(updated)))
                        .children((pct.is_some() && account.change.is_some()).then(|| {
                            crate::ui::tabular(
                                div().whitespace_nowrap().text_color(tone(pct.unwrap())).child(
                                    format!(
                                        "{} · {}",
                                        format::change_signed(pct.unwrap()),
                                        format::signed(account.change.unwrap(), 2),
                                    ),
                                ),
                            )
                        })),
                )
                .child(div().mt(px(8.)).flex().flex_col().gap(px(4.)).children(
                    account.wallets.iter().map(move |wallet| {
                        crate::ui::tabular(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(8.))
                                .text_size(px(10.))
                                .child(
                                    div()
                                        .w(px(88.))
                                        .min_w_0()
                                        .truncate()
                                        .text_color(palette.muted)
                                        .child(wallet_name(wallet.wallet)),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .h(px(4.))
                                        .rounded_full()
                                        .overflow_hidden()
                                        .bg(palette.fill_strong)
                                        .child(
                                            div()
                                                .h_full()
                                                .rounded_full()
                                                .bg(palette.glow)
                                                .opacity(0.7)
                                                .w(gpui_kit::DefiniteLength::Fraction(
                                                    (wallet.value / widest) as f32,
                                                )),
                                        ),
                                )
                                .child(
                                    div().whitespace_nowrap().child(format::price(wallet.value, 2)),
                                ),
                        )
                    }),
                ));
            if let Some(error) = &account.error {
                let text = match account.total {
                    None => error.clone(),
                    Some(_) => t!("holdings.refreshFailed", error = error),
                };
                section = section.child(
                    div()
                        .mt(px(8.))
                        .text_size(px(10.))
                        .text_color(palette.red)
                        .whitespace_nowrap()
                        .child(text),
                );
            }
            section
        }),
    )
}

/// Open derivatives positions, the largest PnL first: a card each.
fn positions(
    positions: &[PositionValue],
    several: bool,
    palette: Palette,
) -> gpui_kit::Stateful<gpui_kit::Div> {
    if positions.is_empty() {
        return div()
            .id("positions-empty")
            .flex_1()
            .flex()
            .items_center()
            .justify_center()
            .text_size(px(12.))
            .text_color(palette.muted)
            .child(t!("holdings.noOpenPositions"));
    }
    div().id("positions").min_h_0().flex_1().overflow_y_scroll().text_size(px(12.)).children(
        positions.iter().map(|held| {
            let position = &held.position;
            let decimals = format::decimals(position.mark, None);
            let settled = position.pnl_asset == "USDT" || position.pnl_asset == "USDC";
            let pnl = held.pnl_usd.or_else(|| settled.then_some(position.pnl));
            let roe = summary::return_on_margin(held);
            let distance = summary::liquidation_distance(held);
            let tone = |value: f64| {
                if format::direction(value) == format::Direction::Down {
                    palette.down
                } else {
                    palette.up
                }
            };
            let pnl_text = match pnl {
                Some(pnl) => format::signed(pnl, 2),
                None => format!("{} {}", format::signed(position.pnl, 6), position.pnl_asset),
            };
            let pnl_color = pnl.map_or(palette.foreground, tone);
            let side = if position.long {
                t!("common.holdings.long")
            } else {
                t!("common.holdings.short")
            };
            let mut facts = vec![
                kind_name(position.kind).to_string(),
                format!(
                    "{side}{}",
                    position
                        .leverage
                        .map(|leverage| format!(" {}x", format::amount(leverage)))
                        .unwrap_or_default()
                ),
                if position.isolated { t!("holdings.isolated") } else { t!("holdings.cross") }
                    .to_owned(),
            ];
            if several {
                facts.push(provider_name(held.exchange).to_string());
            }
            let unit =
                position.size_unit.clone().unwrap_or_else(|| t!("holdings.contracts").to_owned());
            let fact = |label: &'static str, value: String| {
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .text_size(px(10.))
                            .text_color(palette.muted)
                            .whitespace_nowrap()
                            .child(label),
                    )
                    .child(crate::ui::tabular(div().text_size(px(10.)).truncate().child(value)))
            };
            let mut card = div()
                .px(px(12.))
                .py(px(10.))
                .flex()
                .flex_col()
                .border_b_1()
                .border_color(palette.border)
                .child(
                    div()
                        .flex()
                        .items_baseline()
                        .justify_between()
                        .gap(px(8.))
                        .child(
                            div()
                                .min_w_0()
                                .truncate()
                                .font_weight(FontWeight::MEDIUM)
                                .child(position.symbol.clone()),
                        )
                        .child(crate::ui::tabular(
                            div()
                                .font_weight(FontWeight::MEDIUM)
                                .whitespace_nowrap()
                                .text_color(pnl_color)
                                .child(pnl_text),
                        )),
                )
                .child(
                    div()
                        .mt(px(2.))
                        .flex()
                        .items_baseline()
                        .justify_between()
                        .gap(px(8.))
                        .text_size(px(10.))
                        .text_color(palette.muted)
                        .child(div().min_w_0().truncate().child(facts.join(" · ")))
                        .children(roe.map(|roe| {
                            crate::ui::tabular(
                                div()
                                    .whitespace_nowrap()
                                    .text_color(tone(roe))
                                    .child(format::change_signed(roe)),
                            )
                        })),
                )
                // Size, entry, mark.
                .child(
                    div()
                        .mt(px(8.))
                        .flex()
                        .gap(px(8.))
                        .child(fact(
                            t!("holdings.size"),
                            format!("{} {unit}", format::amount(position.size)),
                        ))
                        .child(fact(t!("holdings.entry"), format::price(position.entry, decimals)))
                        .child(fact(t!("holdings.mark"), format::price(position.mark, decimals))),
                );
            if let (Some(liquidation), Some(distance)) = (position.liquidation, distance) {
                let (bar_color, text_color) = if distance < 0.05 {
                    (palette.red, palette.red)
                } else if distance < 0.15 {
                    (palette.amber, palette.amber)
                } else {
                    (palette.glow, palette.muted)
                };
                card = card.child(
                    div()
                        .mt(px(8.))
                        .flex()
                        .flex_col()
                        .gap(px(4.))
                        .child(
                            div()
                                .flex()
                                .justify_between()
                                .text_size(px(10.))
                                .text_color(palette.muted)
                                .child(crate::ui::tabular(div().whitespace_nowrap().child(t!(
                                    "holdings.liquidation",
                                    price = format::price(liquidation, decimals),
                                ))))
                                .child(crate::ui::tabular(
                                    div().whitespace_nowrap().text_color(text_color).child(t!(
                                        "holdings.distance",
                                        pct = format!("{:.1}", distance * 100.)
                                    )),
                                )),
                        )
                        .child(
                            div()
                                .h(px(4.))
                                .rounded_full()
                                .overflow_hidden()
                                .bg(palette.fill_strong)
                                .child(
                                    div()
                                        .h_full()
                                        .rounded_full()
                                        .bg(bar_color)
                                        .opacity(if distance < 0.15 { 1. } else { 0.7 })
                                        .w(gpui_kit::DefiniteLength::Fraction(
                                            (distance * 2.).clamp(0., 1.) as f32,
                                        )),
                                ),
                        ),
                );
            }
            card
        }),
    )
}
