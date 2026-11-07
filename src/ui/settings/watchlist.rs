//! Watchlist and instrument search.

use std::{collections::HashSet, time::Duration};

use super::parts;
use crate::{
    api,
    app::{App, Topic},
    market::{ProviderId, Search},
    model::{Instrument, MAX_INSTRUMENTS},
    ui,
};
use gpui_kit::{
    AppContext, Context, Entity, Focusable, InteractiveElement, IntoElement, ParentElement, Render,
    SharedString, Styled, Subscription, Task, Window,
    component::{
        Disableable, IndexPath, Sizable,
        button::{Button, ButtonVariants},
        input::{Input, InputEvent, InputState},
        list::ListItem,
        popover::Popover,
        scroll::ScrollableElement,
        select::{Select, SelectEvent, SelectItem, SelectState},
        switch::Switch,
        tag::Tag,
    },
    div, px,
};

const SEARCH_DELAY: Duration = Duration::from_millis(200);

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
#[derive(Default)]
struct SearchGeneration(u64);
#[derive(Clone, Copy)]
struct SearchRequest {
    generation: u64,
}
impl SearchGeneration {
    fn invalidate(&mut self) {
        self.0 += 1;
    }
    fn start(&mut self) -> SearchRequest {
        self.invalidate();
        SearchRequest { generation: self.0 }
    }
    fn accepts(&self, request: SearchRequest) -> bool {
        self.0 == request.generation
    }
}

pub struct WatchlistSection {
    app: App,
    input: Entity<InputState>,
    exchange: Entity<SelectState<Vec<ExchangeChoice>>>,
    shown_exchange: ProviderId,
    shown_locale: crate::i18n::Locale,
    query: String,
    active: Option<usize>,
    results: Option<Search>,
    loading: bool,
    generation: SearchGeneration,
    error: Option<String>,
    placeholder: String,
    search_task: Option<Task<()>>,
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
        let input = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder.clone()));
        let exchange = cx.new(|cx| {
            SelectState::new(
                exchanges(),
                Some(IndexPath::new(exchange_index(exchange_value))),
                window,
                cx,
            )
        });
        let input_subscription =
            cx.subscribe_in(&input, window, |this, input, event: &InputEvent, _, cx| match event {
                InputEvent::Change => {
                    this.query = input.read(cx).value().to_string();
                    this.active = None;
                    this.search(SEARCH_DELAY, cx);
                }
                InputEvent::Focus if this.results.is_none() && !this.loading => {
                    this.search(Duration::ZERO, cx)
                }
                _ => {}
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
        Self {
            app,
            input,
            exchange,
            shown_exchange: exchange_value,
            shown_locale: api::get_locale(),
            query: String::new(),
            active: None,
            results: None,
            loading: false,
            generation: SearchGeneration::default(),
            error: None,
            placeholder,
            search_task: None,
            _observers: observers,
            _subscriptions: vec![input_subscription, exchange_subscription],
        }
    }
    fn cancel_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.generation.invalidate();
        self.search_task = None;
        self.query.clear();
        self.results = None;
        self.loading = false;
        self.input.update(cx, |input, cx| input.set_value("", window, cx));
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
        if placeholder != self.placeholder {
            self.input
                .update(cx, |input, cx| input.set_placeholder(placeholder.clone(), window, cx));
            self.placeholder = placeholder;
        }
    }
    fn search(&mut self, delay: Duration, cx: &mut Context<Self>) {
        let request = self.generation.start();
        self.loading = true;
        self.results = None;
        let query = self.query.clone();
        let app = self.app.clone();
        let io_app = app.clone();
        let timer = cx.background_executor().timer(delay);
        self.search_task = Some(cx.spawn(async move |view, cx| {
            timer.await;
            let result =
                ui::run(&app, async move { api::search_instruments(&io_app, query).await }).await;
            let _ = view.update(cx, |this, cx| {
                if this.generation.accepts(request) {
                    this.loading = false;
                    this.results = Some(result.unwrap_or_else(|error| Search {
                        candidates: vec![],
                        notes: vec![error.to_string()],
                    }));
                    cx.notify();
                }
            });
        }));
        cx.notify();
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
        let full = settings.watchlist.len() >= MAX_INSTRUMENTS;
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
        let results = self.results.clone().unwrap_or_default();
        let loading = self.loading;
        let result_view = cx.entity().downgrade();
        let show_results = !self.query.trim().is_empty() && !full;
        let active = self.active.or_else(|| {
            results.candidates.iter().position(|candidate| {
                !existing.iter().any(|item| item.id() == candidate.instrument.id())
            })
        });
        let dismiss_view = cx.entity().downgrade();
        let search = Popover::new("watchlist-search-results")
            .w_full()
            // The results sit 4px from the edge, as the instrument picker's does.
            .p(px(4.))
            // The input handles left clicks; its query controls the popover.
            .mouse_button(gpui_kit::MouseButton::Middle)
            .overlay_closable(false)
            .track_focus(&self.input.read(cx).focus_handle(cx))
            .trigger(Input::new(&self.input).disabled(full))
            .open(show_results)
            .on_open_change(move |open, window, cx| {
                if !*open {
                    let _ = dismiss_view.update(cx, |this, cx| {
                        this.cancel_search(window, cx);
                        cx.notify();
                    });
                }
            })
            .content(move |_, _, _| {
                let mut list = div()
                    .id("watchlist-results-list")
                    .w(px(390.))
                    .max_h(px(280.))
                    .overflow_y_scrollbar()
                    .flex()
                    .flex_col();
                if results.candidates.is_empty() {
                    let notes = if loading {
                        vec![t!("settings.watchlist.searching").to_owned()]
                    } else if results.notes.is_empty() {
                        vec![t!("settings.watchlist.noResults").to_owned()]
                    } else {
                        results.notes.clone()
                    };
                    for note in notes {
                        list = list.child(
                            div()
                                .px_2()
                                .py_1()
                                .text_color(p.muted)
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .child(note),
                        );
                    }
                } else {
                    for (index, candidate) in results.candidates.iter().enumerate() {
                        let added =
                            existing.iter().any(|item| item.id() == candidate.instrument.id());
                        let item = candidate.instrument.clone();
                        let view = result_view.clone();
                        let detail = if added {
                            t!("settings.watchlist.added").to_owned()
                        } else if candidate.manual {
                            t!("settings.watchlist.manual").to_owned()
                        } else if let Some(value) = candidate.liquidity {
                            t!(
                                "settings.watchlist.liquidity",
                                value = crate::format::compact(value)
                            )
                        } else {
                            String::new()
                        };
                        list = list.child(
                            ListItem::new(("watchlist-result", index))
                                .selected(active == Some(index) && !added)
                                .accessibility_label(item.pair_label())
                                .px(px(8.))
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap_2()
                                        .min_w_0()
                                        .w_full()
                                        .child(instrument_name(&item, false, p))
                                        .child(Tag::secondary().small().child(item.provider.name()))
                                        .child(
                                            div()
                                                .text_xs()
                                                .text_color(p.muted)
                                                .whitespace_nowrap()
                                                .child(detail),
                                        )
                                        .child(
                                            Button::new(("watchlist-add", index))
                                                .label(t!("settings.add"))
                                                .small()
                                                .disabled(added)
                                                .on_click(move |_, window, cx| {
                                                    let _ = view.update(cx, |this, cx| {
                                                        this.add(item.clone(), window, cx)
                                                    });
                                                }),
                                        ),
                                ),
                        );
                    }
                }
                list
            });
        let mut watchlist = parts::section(t!("settings.watchlist.title"), &self.app, cx)
            .child(
                div()
                    .capture_key_down(cx.listener(
                        |this, event: &gpui_kit::KeyDownEvent, window, cx| {
                            let m = event.keystroke.modifiers;
                            if m.control || m.alt || m.platform || m.function {
                                return;
                            }
                            if this.query.trim().is_empty() {
                                return;
                            }
                            if event.keystroke.key == "escape" {
                                this.cancel_search(window, cx);
                                cx.stop_propagation();
                                cx.notify();
                                return;
                            }
                            if event.keystroke.key == "enter" {
                                let existing = api::get_settings(&this.app).watchlist;
                                let item = this.results.as_ref().and_then(|results| {
                                    let index = this.active.or_else(|| {
                                        results.candidates.iter().position(|candidate| {
                                            !existing
                                                .iter()
                                                .any(|i| i.id() == candidate.instrument.id())
                                        })
                                    })?;
                                    results.candidates.get(index).map(|c| c.instrument.clone())
                                });
                                if let Some(item) = item {
                                    this.add(item, window, cx);
                                }
                                cx.stop_propagation();
                                return;
                            }
                            let Some(results) = &this.results else {
                                return;
                            };
                            let count = results.candidates.len();
                            if count == 0 {
                                return;
                            }
                            let existing = api::get_settings(&this.app).watchlist;
                            let current = this
                                .active
                                .or_else(|| {
                                    results.candidates.iter().position(|candidate| {
                                        !existing
                                            .iter()
                                            .any(|i| i.id() == candidate.instrument.id())
                                    })
                                })
                                .unwrap_or(0);
                            let next = match event.keystroke.key.as_str() {
                                "down" => current.saturating_add(1).min(count - 1),
                                "up" => current.saturating_sub(1),
                                _ => return,
                            };
                            this.active = Some(next);
                            cx.stop_propagation();
                            cx.notify();
                        },
                    ))
                    .child(search),
            )
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
    fn debounce_restarts_and_rejects_stale_results() {
        let mut gate = SearchGeneration::default();
        let first = gate.start();
        let second = gate.start();
        assert!(!gate.accepts(first));
        assert!(gate.accepts(second));
        gate.invalidate();
        assert!(!gate.accepts(second));
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
