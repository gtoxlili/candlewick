//! One instrument picker for self-selected watchlists and registered holdings.
//! Owns debouncing, cancellation, stale-result rejection and keyboard selection.

use crate::{
    api,
    app::{App, Topic},
    market::Search,
    model::Instrument,
    ui::{self, theme::Palette},
};
use gpui_kit::{
    AppContext as _, Context, Entity, EventEmitter, Focusable as _, InteractiveElement as _,
    IntoElement, ParentElement, Render, SharedString, Styled, Subscription, Task, Window,
    component::{
        Sizable as _,
        input::{Input, InputEvent, InputState},
        list::ListItem,
        popover::Popover,
        scroll::ScrollableElement as _,
        tag::Tag,
    },
    div, px,
};
use std::{collections::HashSet, time::Duration};

pub struct Selected(pub Instrument);

#[derive(Default)]
struct SearchGeneration(u64);
#[derive(Clone, Copy)]
struct SearchRequest {
    generation: u64,
}
impl SearchGeneration {
    fn invalidate(&mut self) {
        self.0 = self.0.wrapping_add(1);
    }
    fn start(&mut self) -> SearchRequest {
        self.invalidate();
        SearchRequest { generation: self.0 }
    }
    fn accepts(&self, request: SearchRequest) -> bool {
        self.0 == request.generation
    }
}

pub struct InstrumentPicker {
    app: App,
    prefix: &'static str,
    input: Entity<InputState>,
    query: String,
    selected: Option<Instrument>,
    existing: HashSet<String>,
    disabled: bool,
    results: Option<Search>,
    loading: bool,
    active: Option<usize>,
    generation: SearchGeneration,
    search_task: Option<Task<()>>,
    shown_exchange: crate::market::ProviderId,
    _subscriptions: Vec<Subscription>,
    _observers: Vec<Task<()>>,
}
impl EventEmitter<Selected> for InstrumentPicker {}

impl InstrumentPicker {
    pub fn new(
        app: App,
        prefix: &'static str,
        placeholder: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
        let subscription = cx.subscribe(&input, |this, input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                let query = input.read(cx).value().to_string();
                if this.selected.as_ref().is_some_and(|item| item.pair_label() == query) {
                    return;
                }
                this.selected = None;
                this.query = query;
                this.search(Duration::from_millis(200), cx);
            }
        });
        let observers = vec![
            ui::observe(&app, Topic::Locale, cx, |_, _| {}),
            ui::observe(&app, Topic::Settings, cx, |this, cx| {
                let exchange = api::get_settings(&this.app).exchange;
                if exchange != this.shown_exchange {
                    this.shown_exchange = exchange;
                    if this.selected.is_none() {
                        this.search(Duration::ZERO, cx);
                    }
                }
            }),
        ];
        Self {
            shown_exchange: api::get_settings(&app).exchange,
            app,
            prefix,
            input,
            query: String::new(),
            selected: None,
            existing: HashSet::new(),
            disabled: false,
            results: None,
            loading: false,
            active: None,
            generation: SearchGeneration::default(),
            search_task: None,
            _subscriptions: vec![subscription],
            _observers: observers,
        }
    }

    pub fn selection(&self) -> Option<&Instrument> {
        self.selected.as_ref()
    }

    pub fn configure(
        &mut self,
        existing: impl Iterator<Item = String>,
        disabled: bool,
        placeholder: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.existing = existing.collect();
        self.disabled = disabled;
        self.input.update(cx, |input, cx| input.set_placeholder(placeholder, window, cx));
        cx.notify();
    }

    pub fn set_selection(
        &mut self,
        selected: Option<Instrument>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.generation.invalidate();
        self.search_task = None;
        self.results = None;
        self.loading = false;
        self.active = None;
        self.query = selected.as_ref().map(Instrument::pair_label).unwrap_or_default();
        self.selected = selected;
        self.input.update(cx, |input, cx| input.set_value(self.query.clone(), window, cx));
        cx.notify();
    }

    fn search(&mut self, delay: Duration, cx: &mut Context<Self>) {
        let request = self.generation.start();
        self.search_task = None;
        self.results = None;
        self.active = None;
        self.loading = !self.disabled && !self.query.trim().is_empty();
        if !self.loading {
            cx.notify();
            return;
        }
        let app = self.app.clone();
        let io = app.clone();
        let query = self.query.clone();
        let timer = cx.background_executor().timer(delay);
        self.search_task = Some(cx.spawn(async move |view, cx| {
            timer.await;
            let result =
                ui::run(&app, async move { api::search_instruments(&io, query).await }).await;
            let _ = view.update(cx, |this, cx| {
                if !this.generation.accepts(request) {
                    return;
                }
                this.loading = false;
                this.results = Some(result.unwrap_or_else(|error| Search {
                    candidates: Vec::new(),
                    notes: vec![error.to_string()],
                }));
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn choose(&mut self, instrument: Instrument, window: &mut Window, cx: &mut Context<Self>) {
        if self.disabled || self.existing.contains(&instrument.id()) {
            return;
        }
        self.set_selection(Some(instrument.clone()), window, cx);
        cx.emit(Selected(instrument));
    }

    fn active(&self) -> Option<usize> {
        self.active.or_else(|| {
            self.results
                .as_ref()?
                .candidates
                .iter()
                .position(|candidate| !self.existing.contains(&candidate.instrument.id()))
        })
    }

    fn key(&mut self, event: &gpui_kit::KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let modifiers = event.keystroke.modifiers;
        if modifiers.control
            || modifiers.alt
            || modifiers.platform
            || modifiers.function
            || self.query.trim().is_empty()
            || self.selected.is_some()
        {
            return;
        }
        match event.keystroke.key.as_str() {
            "escape" => self.set_selection(None, window, cx),
            "enter" => {
                if let Some(instrument) = self
                    .active()
                    .and_then(|index| self.results.as_ref()?.candidates.get(index))
                    .map(|candidate| candidate.instrument.clone())
                {
                    self.choose(instrument, window, cx);
                }
            }
            "down" | "up" => {
                let indices: Vec<_> = self
                    .results
                    .as_ref()
                    .into_iter()
                    .flat_map(|results| results.candidates.iter().enumerate())
                    .filter(|(_, candidate)| !self.existing.contains(&candidate.instrument.id()))
                    .map(|(index, _)| index)
                    .collect();
                if indices.is_empty() {
                    return;
                }
                let at = self
                    .active()
                    .and_then(|active| indices.iter().position(|index| *index == active))
                    .unwrap_or(0);
                let next = if event.keystroke.key == "down" {
                    (at + 1).min(indices.len() - 1)
                } else {
                    at.saturating_sub(1)
                };
                self.active = Some(indices[next]);
                cx.notify();
            }
            _ => return,
        }
        cx.stop_propagation();
    }

    fn list(&self, p: Palette, cx: &mut Context<Self>) -> impl IntoElement {
        let mut list = div()
            .id(SharedString::from(format!("{}-search-results", self.prefix)))
            .max_h(px(260.))
            .overflow_y_scrollbar()
            .p(px(4.))
            .flex()
            .flex_col()
            .gap(px(2.));
        let results = self.results.as_ref();
        if results.is_none_or(|results| results.candidates.is_empty()) {
            let text = if self.loading {
                t!("settings.watchlist.searching")
            } else {
                t!("settings.watchlist.noResults")
            };
            list = list.child(div().px_3().py_2().text_xs().text_color(p.muted).child(text));
        }
        if let Some(results) = results {
            for (index, candidate) in results.candidates.iter().enumerate() {
                let item = candidate.instrument.clone();
                let id = item.id();
                let added = self.existing.contains(&id);
                let detail = if added {
                    Some(t!("settings.watchlist.added").to_owned())
                } else if candidate.manual {
                    Some(t!("settings.watchlist.manual").to_owned())
                } else {
                    candidate.liquidity.map(|value| {
                        t!("settings.watchlist.liquidity", value = crate::format::compact(value))
                    })
                };
                let (name, sub) = candidate_labels(&item);
                list = list.child(
                    ListItem::new(SharedString::from(format!("{}-pick-{id}", self.prefix)))
                        .selected(self.active() == Some(index) && !added)
                        .disabled(added)
                        .px(px(8.))
                        .py(px(6.))
                        .rounded(px(6.))
                        .child(
                            div()
                                .w_full()
                                .min_w_0()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .flex()
                                        .items_baseline()
                                        .gap(px(6.))
                                        .overflow_hidden()
                                        .whitespace_nowrap()
                                        .child(
                                            div()
                                                .text_size(px(13.))
                                                .text_color(p.foreground)
                                                .child(name),
                                        )
                                        .children((!sub.is_empty()).then(|| {
                                            div()
                                                .flex_shrink_0()
                                                .text_xs()
                                                .text_color(p.muted)
                                                .child(sub)
                                        })),
                                )
                                .children(detail.map(|text| {
                                    div()
                                        .flex_shrink_0()
                                        .text_xs()
                                        .text_color(p.muted)
                                        .whitespace_nowrap()
                                        .child(text)
                                }))
                                .child(
                                    div().flex_shrink_0().child(
                                        Tag::secondary()
                                            .small()
                                            .child(candidate.instrument.provider.name()),
                                    ),
                                ),
                        )
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.choose(item.clone(), window, cx)
                        })),
                );
            }
            for note in &results.notes {
                list = list
                    .child(div().px_2().py_1().text_xs().text_color(p.amber).child(note.clone()));
            }
        }
        list
    }
}

fn candidate_labels(item: &Instrument) -> (String, String) {
    use crate::market::ProviderId;
    if item.provider == ProviderId::DexScreener {
        let mut parts = Vec::new();
        if let Some(name) = &item.name {
            parts.push(name.clone());
        }
        if let Some(venue) = &item.venue {
            match &venue.dex {
                Some(dex) => parts.push(format!("{} · {dex}", venue.chain)),
                None => parts.push(venue.chain.clone()),
            }
        }
        return (item.base.clone(), parts.join(" · "));
    }
    let (name, detail) = item.row_label();
    let detail = detail.trim();
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
    let sub = match (!detail.is_empty(), market) {
        (true, Some(m)) => format!("{detail} · {m}"),
        (true, None) => detail.to_owned(),
        (false, Some(m)) => m.to_owned(),
        (false, None) => String::new(),
    };
    (name, sub)
}

impl Render for InstrumentPicker {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Palette::current(&self.app, cx);
        let open = self.selected.is_none() && !self.query.trim().is_empty() && !self.disabled;
        let view = cx.entity();
        let dismiss = cx.entity();
        let body = Popover::new("instrument-results")
            .bg(p.popover.alpha(1.))
            .w(px(420.))
            .p_0()
            .mouse_button(gpui_kit::MouseButton::Middle)
            .overlay_closable(false)
            .track_focus(&self.input.read(cx).focus_handle(cx))
            .trigger(Input::new(&self.input).cleanable(true).disabled(self.disabled))
            .open(open)
            .on_open_change(move |open, window, cx| {
                if !*open {
                    dismiss.update(cx, |this, cx| this.set_selection(None, window, cx));
                }
            })
            .content(move |_, _, cx| {
                view.update(cx, |this, cx| div().w(px(420.)).child(this.list(p, cx)))
            });
        div().w_full().capture_key_down(cx.listener(Self::key)).child(body)
    }
}

/// Install source results after input events settle, so render checks stay offline.
#[cfg(all(target_os = "macos", feature = "test-support"))]
pub(crate) fn sample_results(
    cx: &mut gpui_kit::HeadlessAppContext,
    window: gpui_kit::AnyWindowHandle,
    picker: &Entity<InstrumentPicker>,
    query: &str,
    results: Search,
) {
    cx.update_window(window, |_, window, cx| {
        let input = picker.read(cx).input.clone();
        input.update(cx, |input, cx| input.set_value(query.to_owned(), window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        picker.update(cx, |picker, cx| {
            picker.generation.invalidate();
            picker.search_task = None;
            picker.query = query.to_owned();
            picker.results = Some(results);
            picker.loading = false;
            cx.notify();
        })
    });
}

#[cfg(test)]
mod tests {
    use super::*;
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
    // UX contract: the result row selects the source instrument and
    // emit it to their consumer without fetching a portfolio or placing an order.
    #[test]
    fn popover_emits_selected_instruments() {
        use crate::{
            market::{Candidate, ProviderId},
            model::{Settings, Shared},
        };
        use gpui_kit::{
            Bounds, TestAppContext, WindowBounds, WindowOptions, size, test::TestWindowExt as _,
        };
        struct Stage {
            picker: Entity<InstrumentPicker>,
            selected: Option<Instrument>,
            _subscription: Subscription,
        }
        impl Render for Stage {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                div().size_full().p_4().child(self.picker.clone())
            }
        }
        let dir =
            tempfile::tempdir_in(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target"))
                .unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let cx = TestAppContext::single();
        cx.update(gpui_kit::init);
        let (shared, _control) = Shared::new(
            Settings::default(),
            dir.path().join("settings.json"),
            Default::default(),
            dir.path().join("credentials.json"),
        );
        let app = cx.update(|cx| App::new(cx, runtime.handle().clone(), shared));
        {
            let mut stage = None;
            let window = cx.update(|cx| {
                gpui_kit::open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                            Default::default(),
                            size(px(460.), px(400.)),
                        ))),
                        ..Default::default()
                    },
                    cx,
                    |window, cx| {
                        let picker = cx.new(|cx| {
                            InstrumentPicker::new(
                                app.clone(),
                                "sample",
                                "Search".into(),
                                window,
                                cx,
                            )
                        });
                        let view = cx.new(|cx| {
                            let subscription = cx.subscribe(
                                &picker,
                                |this: &mut Stage, _, event: &Selected, cx| {
                                    this.selected = Some(event.0.clone());
                                    cx.notify();
                                },
                            );
                            Stage { picker, selected: None, _subscription: subscription }
                        });
                        stage = Some(view.clone());
                        view
                    },
                )
                .unwrap()
                .0
            });
            let stage = stage.unwrap();
            cx.update(|cx| {
                let picker = stage.read(cx).picker.clone();
                picker.update(cx, |picker, cx| {
                    picker.query = "AU9999".into();
                    picker.results = Some(Search {
                        candidates: vec![Candidate {
                            instrument: ProviderId::Eastmoney
                                .provider()
                                .bare("118.AU9999")
                                .unwrap(),
                            manual: false,
                            liquidity: None,
                        }],
                        notes: Vec::new(),
                    });
                    cx.notify();
                });
                window
                    .update(cx, |_, window, cx| {
                        window.click(SharedString::from("sample-pick-eastmoney:118.AU9999"), cx)
                    })
                    .unwrap();
            });
            cx.run_until_parked();
            cx.update(|cx| {
                assert_eq!(stage.read(cx).selected.as_ref().unwrap().id(), "eastmoney:118.AU9999")
            });
        }
    }
}
