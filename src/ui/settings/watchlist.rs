//! Watchlist and instrument search.

use std::collections::HashSet;

use super::parts;
use crate::{
    api,
    app::{App, Topic},
    market::ProviderId,
    model::{Instrument, MAX_INSTRUMENTS},
    ui::{
        self,
        instrument_picker::{InstrumentPicker, Presentation, Selected},
    },
};
use gpui_kit::{
    AppContext, Context, Entity, IntoElement, ParentElement, Render, SharedString, Styled,
    Subscription, Task, Window,
    component::{
        Disableable, IndexPath, Sizable,
        button::{Button, ButtonVariants},
        select::{Select, SelectEvent, SelectItem, SelectState},
        switch::Switch,
    },
    div, px,
};

#[derive(Clone)]
struct ExchangeChoice(ProviderId);
impl SelectItem for ExchangeChoice {
    type Value = ProviderId;
    fn title(&self) -> SharedString {
        self.0.name().into()
    }
    fn value(&self) -> &ProviderId {
        &self.0
    }
}
fn exchanges() -> Vec<ExchangeChoice> {
    ProviderId::EXCHANGES.into_iter().map(ExchangeChoice).collect()
}
fn exchange_index(value: ProviderId) -> usize {
    ProviderId::EXCHANGES.iter().position(|v| *v == value).unwrap_or_default()
}
fn shared_symbols<'a>(items: impl IntoIterator<Item = &'a Instrument>) -> HashSet<String> {
    let mut seen = HashSet::new();
    let mut shared = HashSet::new();
    for item in items {
        if !seen.insert(item.symbol.clone()) {
            shared.insert(item.symbol.clone());
        }
    }
    shared
}
fn add_instrument(items: &mut Vec<Instrument>, mut item: Instrument) -> bool {
    if items.len() >= MAX_INSTRUMENTS || items.iter().any(|i| i.id() == item.id()) {
        return false;
    }
    item.pinned = items.is_empty();
    items.push(item);
    true
}
fn remove_instrument(items: &mut Vec<Instrument>, id: &str) {
    items.retain(|item| item.id() != id);
}
pub struct WatchlistSection {
    app: App,
    picker: Entity<InstrumentPicker>,
    exchange: Entity<SelectState<Vec<ExchangeChoice>>>,
    shown_exchange: ProviderId,
    shown_locale: crate::i18n::Locale,
    error: Option<String>,
    _observers: Vec<Task<()>>,
    _subscriptions: Vec<Subscription>,
}
impl WatchlistSection {
    pub fn new(app: App, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let settings = api::get_settings(&app);
        let exchange_value = settings.exchange;
        let placeholder = if settings.watchlist.len() >= MAX_INSTRUMENTS {
            t!("settings.watchlist.full", max = MAX_INSTRUMENTS)
        } else {
            t!("settings.watchlist.search").to_owned()
        };
        let picker = cx.new(|cx| {
            InstrumentPicker::new(
                app.clone(),
                "watchlist",
                Presentation::Popover,
                placeholder,
                window,
                cx,
            )
        });
        let exchange = cx.new(|cx| {
            SelectState::new(
                exchanges(),
                Some(IndexPath::new(exchange_index(exchange_value))),
                window,
                cx,
            )
        });
        let picker_subscription =
            cx.subscribe_in(&picker, window, |this, _, event: &Selected, window, cx| {
                this.add(event.0.clone(), window, cx)
            });
        let exchange_subscription = cx.subscribe_in(
            &exchange,
            window,
            |this, _, event: &SelectEvent<Vec<ExchangeChoice>>, window, cx| {
                if let SelectEvent::Confirm(Some(value)) = event {
                    this.error =
                        parts::update_settings(&this.app, |settings| settings.exchange = *value)
                            .err();
                    if this.error.is_none() && this.shown_exchange != *value {
                        this.cancel_search(window, cx);
                    }
                    this.shown_exchange = *value;
                    this.sync(window, cx);
                    cx.notify();
                }
            },
        );
        let observers = vec![
            ui::observe_in(&app, Topic::Locale, window, cx, Self::sync),
            ui::observe_in(&app, Topic::Settings, window, cx, Self::sync),
        ];
        let mut view = Self {
            app,
            picker,
            exchange,
            shown_exchange: exchange_value,
            shown_locale: api::get_locale(),
            error: None,
            _observers: observers,
            _subscriptions: vec![picker_subscription, exchange_subscription],
        };
        view.sync(window, cx);
        view
    }
    fn cancel_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.picker.update(cx, |picker, cx| picker.set_selection(None, window, cx));
    }

    fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let settings = api::get_settings(&self.app);
        let locale = api::get_locale();
        if settings.exchange != self.shown_exchange || locale != self.shown_locale {
            if settings.exchange != self.shown_exchange {
                self.cancel_search(window, cx);
            }
            self.exchange.update(cx, |select, cx| {
                select.set_items(exchanges(), window, cx);
                select.set_selected_index(
                    Some(IndexPath::new(exchange_index(settings.exchange))),
                    window,
                    cx,
                );
            });
            self.shown_exchange = settings.exchange;
            self.shown_locale = locale;
        }
        let full = settings.watchlist.len() >= MAX_INSTRUMENTS;
        let placeholder = if full {
            t!("settings.watchlist.full", max = MAX_INSTRUMENTS)
        } else {
            t!("settings.watchlist.search").to_owned()
        };
        self.picker.update(cx, |picker, cx| {
            picker.configure(
                settings.watchlist.iter().map(Instrument::id),
                full,
                placeholder,
                window,
                cx,
            )
        });
    }
    fn add(&mut self, item: Instrument, window: &mut Window, cx: &mut Context<Self>) {
        let mut added = false;
        self.error = parts::update_settings(&self.app, |settings| {
            added = add_instrument(&mut settings.watchlist, item);
            if added && settings.watchlist.iter().any(|item| item.pinned) {
                settings.holdings_in_bar = false;
            }
        })
        .err();
        if added && self.error.is_none() {
            self.cancel_search(window, cx);
        }
        cx.notify();
    }
}

fn instrument_name(
    item: &Instrument,
    show_source: bool,
    p: ui::theme::Palette,
) -> impl IntoElement {
    let (name, detail, venue) = if item.provider == ProviderId::DexScreener {
        let detail = item.name.as_ref().map(|name| format!(" {name}")).unwrap_or_default();
        let venue = item.venue.as_ref().map(|venue| match &venue.dex {
            Some(dex) => format!("{} · {dex}", venue.chain),
            None => venue.chain.clone(),
        });
        (item.base.clone(), detail, venue)
    } else {
        let (name, detail) = item.row_label();
        let market = if item.provider == ProviderId::Ths
            || item.symbol.ends_with(".SH")
            || item.symbol.ends_with(".SZ")
        {
            Some(t!("market.CN"))
        } else if item.provider == ProviderId::Longbridge && item.symbol.ends_with(".US") {
            Some(t!("market.US"))
        } else if item.provider == ProviderId::Longbridge && item.symbol.ends_with(".HK") {
            Some(t!("market.HK"))
        } else {
            None
        };
        (name, detail, market.map(str::to_owned))
    };
    let mut suffix = venue.unwrap_or_default();
    if show_source {
        if !suffix.is_empty() {
            suffix.push_str(" · ");
        }
        suffix.push_str(item.provider.name());
    }
    if !suffix.is_empty() {
        suffix.insert(0, ' ');
    }
    div()
        .flex_1()
        .min_w_0()
        .overflow_hidden()
        .whitespace_nowrap()
        .text_ellipsis()
        .child(format!("{name}{detail}{suffix}"))
        .text_color(p.foreground)
}

impl Render for WatchlistSection {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let settings = api::get_settings(&self.app);
        let p = ui::theme::Palette::current(&self.app, cx);
        let existing = settings.watchlist.clone();
        let shared = shared_symbols(existing.iter());
        let mut group = parts::group(&self.app, cx);
        if existing.is_empty() {
            group = group.child(
                div()
                    .px_3()
                    .py_4()
                    .text_color(p.muted)
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .child(t!("settings.watchlist.empty")),
            );
        }
        for (index, item) in existing.iter().enumerate() {
            let id = item.id();
            let pin_id = id.clone();
            let remove_id = id.clone();
            let weak = cx.entity().downgrade();
            let pin_view = weak.clone();
            let remove_view = weak.clone();
            let mut row = div()
                .flex()
                .items_center()
                .gap_1()
                .px_3()
                .h(px(40.))
                .child(instrument_name(item, shared.contains(&item.symbol), p))
                .child(
                    div()
                        .text_xs()
                        .text_color(p.muted)
                        .whitespace_nowrap()
                        .child(t!("settings.watchlist.bar")),
                )
                .child(
                    Switch::new(SharedString::from(format!("watchlist-pin:{id}")))
                        .accessibility_label(t!("settings.watchlist.bar"))
                        .checked(item.pinned)
                        .on_change(move |checked, _, cx| {
                            let _ = pin_view.update(cx, |this, cx| {
                                this.error = parts::update_settings(&this.app, |settings| {
                                    if *checked {
                                        settings.holdings_in_bar = false;
                                    }
                                    for item in &mut settings.watchlist {
                                        if item.id() == pin_id {
                                            item.pinned = *checked;
                                        } else if *checked {
                                            item.pinned = false;
                                        }
                                    }
                                })
                                .err();
                                cx.notify();
                            });
                        }),
                );
            for (delta, label, disabled) in
                [(-1isize, "↑", index == 0), (1, "↓", index + 1 == existing.len())]
            {
                let move_id = id.clone();
                let move_view = weak.clone();
                let key = if delta < 0 {
                    format!("watchlist-up:{id}")
                } else {
                    format!("watchlist-down:{id}")
                };
                row = row.child(
                    Button::new(SharedString::from(key))
                        .label(label)
                        .accessibility_label(if delta < 0 {
                            t!("settings.watchlist.moveUp")
                        } else {
                            t!("settings.watchlist.moveDown")
                        })
                        .ghost()
                        .small()
                        .disabled(disabled)
                        .on_click(move |_, _, cx| {
                            let _ = move_view.update(cx, |this, cx| {
                                this.error = parts::update_settings(&this.app, |settings| {
                                    if let Some(i) =
                                        settings.watchlist.iter().position(|v| v.id() == move_id)
                                        && let Some(j) = i
                                            .checked_add_signed(delta)
                                            .filter(|j| *j < settings.watchlist.len())
                                    {
                                        settings.watchlist.swap(i, j);
                                    }
                                })
                                .err();
                                cx.notify();
                            });
                        }),
                );
            }
            row = row.child(
                Button::new(SharedString::from(format!("watchlist-remove:{id}")))
                    .label("×")
                    .ghost()
                    .small()
                    .accessibility_label(t!("settings.watchlist.remove", name = item.short_label()))
                    .on_click(move |_, _, cx| {
                        let _ = remove_view.update(cx, |this, cx| {
                            this.error = parts::update_settings(&this.app, |settings| {
                                remove_instrument(&mut settings.watchlist, &remove_id)
                            })
                            .err();
                            cx.notify();
                        });
                    }),
            );
            group = group.child(row);
        }
        let mut watchlist = parts::section(t!("settings.watchlist.title"), &self.app, cx)
            .child(self.picker.clone())
            .child(group)
            .child(
                div()
                    .px_1()
                    .text_xs()
                    .text_color(p.muted)
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .child(t!("settings.watchlist.footnote")),
            );
        if let Some(error) = &self.error {
            watchlist = watchlist.child(parts::problem("watchlist-error", error.clone()));
        }
        let crypto = parts::section(t!("settings.crypto.title"), &self.app, cx)
            .child(parts::group(&self.app, cx).child(parts::picker_row(
                t!("settings.crypto.exchange"),
                Select::new(&self.exchange).id("exchange-select").w(px(150.)),
            )))
            .child(
                div()
                    .px_1()
                    .text_xs()
                    .text_color(p.muted)
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .child(t!("settings.crypto.footnote")),
            );
        div().flex().flex_col().gap_5().child(watchlist).child(crypto)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn item(provider: ProviderId, symbol: &str) -> Instrument {
        Instrument::bare(provider, symbol)
    }
    #[test]
    fn list_preserves_order_rejects_duplicates_and_caps_size() {
        let mut list = vec![];
        assert!(add_instrument(&mut list, item(ProviderId::Binance, "BTC")));
        assert!(list[0].pinned);
        assert!(!add_instrument(&mut list, item(ProviderId::Binance, "BTC")));
        assert!(add_instrument(&mut list, item(ProviderId::Bybit, "BTC")));
        assert_eq!(shared_symbols(list.iter()).len(), 1);
        for n in 2..MAX_INSTRUMENTS {
            assert!(add_instrument(&mut list, item(ProviderId::Binance, &n.to_string())));
        }
        assert!(!add_instrument(&mut list, item(ProviderId::Binance, "overflow")));
        remove_instrument(&mut list, "binance:BTC");
        assert_eq!(list[0].provider, ProviderId::Bybit);
        assert!(add_instrument(&mut list, item(ProviderId::Binance, "overflow")));
    }
}
