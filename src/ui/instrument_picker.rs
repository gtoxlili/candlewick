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
        Disableable as _, Sizable as _,
        button::{Button, ButtonVariants as _},
        input::{Input, InputEvent, InputState},
        list::ListItem,
        popover::Popover,
        scroll::ScrollableElement as _,
    },
    div, px,
};
use std::{collections::HashSet, time::Duration};

#[derive(Clone, Copy)]
pub enum Presentation {
    Popover,
    Inline,
}
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
    presentation: Presentation,
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
        presentation: Presentation,
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
            presentation,
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
            .max_h(px(240.))
            .overflow_y_scrollbar()
            .flex()
            .flex_col();
        let results = self.results.as_ref();
        if results.is_none_or(|results| results.candidates.is_empty()) {
            let text = if self.loading {
                t!("settings.watchlist.searching")
            } else {
                t!("settings.watchlist.noResults")
            };
            list =
                list.child(div().px_2().py_1().text_size(px(11.)).text_color(p.muted).child(text));
        }
        if let Some(results) = results {
            for (index, candidate) in results.candidates.iter().enumerate() {
                let item = candidate.instrument.clone();
                let id = item.id();
                let added = self.existing.contains(&id);
                let detail = if added {
                    t!("settings.watchlist.added").to_owned()
                } else if candidate.manual {
                    t!("settings.watchlist.manual").to_owned()
                } else {
                    candidate
                        .liquidity
                        .map(|value| {
                            t!(
                                "settings.watchlist.liquidity",
                                value = crate::format::compact(value)
                            )
                        })
                        .unwrap_or_default()
                };
                list = list.child(
                    ListItem::new((self.prefix, index))
                        .selected(self.active() == Some(index) && !added)
                        .child(
                            div()
                                .w_full()
                                .flex()
                                .flex_col()
                                .child(
                                    Button::new(SharedString::from(format!(
                                        "{}-pick-{id}",
                                        self.prefix
                                    )))
                                    .small()
                                    .ghost()
                                    .disabled(added)
                                    .label(item.pair_label())
                                    .on_click(cx.listener(
                                        move |this, _, window, cx| {
                                            this.choose(item.clone(), window, cx)
                                        },
                                    )),
                                )
                                .child(div().px_2().text_size(px(10.)).text_color(p.muted).child(
                                    format!("{} {detail}", candidate.instrument.provider.name()),
                                )),
                        ),
                );
            }
            for note in &results.notes {
                list = list.child(
                    div().px_2().py_1().text_size(px(10.)).text_color(p.amber).child(note.clone()),
                );
            }
        }
        list
    }
}

impl Render for InstrumentPicker {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Palette::current(&self.app, cx);
        let open = self.selected.is_none() && !self.query.trim().is_empty() && !self.disabled;
        let body = match self.presentation {
            Presentation::Inline => div()
                .flex()
                .flex_col()
                .gap_1()
                .child(Input::new(&self.input).small().disabled(self.disabled))
                .children(open.then(|| self.list(p, cx).into_any_element()))
                .into_any_element(),
            Presentation::Popover => {
                let view = cx.entity();
                let dismiss = cx.entity();
                Popover::new("instrument-results")
                    .w_full()
                    .p(px(4.))
                    .mouse_button(gpui_kit::MouseButton::Middle)
                    .overlay_closable(false)
                    .track_focus(&self.input.read(cx).focus_handle(cx))
                    .trigger(Input::new(&self.input).disabled(self.disabled))
                    .open(open)
                    .on_open_change(move |open, window, cx| {
                        if !*open {
                            dismiss.update(cx, |this, cx| this.set_selection(None, window, cx));
                        }
                    })
                    .content(move |_, _, cx| {
                        view.update(cx, |this, cx| div().w(px(390.)).child(this.list(p, cx)))
                    })
                    .into_any_element()
            }
        };
        div().w_full().capture_key_down(cx.listener(Self::key)).child(body)
    }
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
    // UX contract: both presentations select the same source instrument and
    // emit it to their consumer without fetching a portfolio or placing an order.
    #[test]
    fn inline_and_popover_emit_selected_instruments() {
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
        for presentation in [Presentation::Inline, Presentation::Popover] {
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
                                presentation,
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
