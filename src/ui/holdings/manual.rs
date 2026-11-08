//! Register quantities against a selected market instrument. Prices are read
//! from the same feeds as the watchlist, independently of where assets are held.

use crate::{
    api,
    app::{App, Topic},
    format,
    manual::{Holding, Input, Value},
    market::{PriceKind, QuantityUnit},
    model::Instrument,
    ui::{
        self,
        instrument_picker::{InstrumentPicker, Presentation, Selected},
        theme::Palette,
    },
};
use gpui_kit::{
    AppContext as _, Context, Div, Entity, InteractiveElement as _, IntoElement, ParentElement,
    Render, SharedString, Styled, Subscription, Task, Window,
    component::{
        Disableable as _, Sizable as _,
        button::{Button, ButtonVariants as _},
        input::{Input as TextInput, InputEvent, InputState},
        scroll::ScrollableElement as _,
    },
    div, px,
};
use std::sync::Arc;

pub struct ManualView {
    app: App,
    inputs: Vec<Entity<InputState>>,
    picker: Entity<InstrumentPicker>,
    pub(super) portfolio: Arc<crate::portfolio::Portfolio>,
    editing: Option<String>,
    removing: Option<String>,
    problem: Option<SharedString>,
    _subscriptions: Vec<Subscription>,
    _observers: Vec<Task<()>>,
}

impl ManualView {
    pub fn new(
        app: App,
        portfolio: Arc<crate::portfolio::Portfolio>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut subscriptions = Vec::new();
        let inputs = (0..3)
            .map(|_| {
                let input = cx.new(|cx| InputState::new(window, cx));
                subscriptions.push(cx.subscribe(&input, |_, _, event: &InputEvent, cx| {
                    if matches!(event, InputEvent::Change) {
                        cx.notify();
                    }
                }));
                input
            })
            .collect();
        let picker = cx.new(|cx| {
            InstrumentPicker::new(
                app.clone(),
                "manual",
                Presentation::Inline,
                t!("holdings.manual.search").to_owned(),
                window,
                cx,
            )
        });
        subscriptions.push(cx.subscribe(&picker, |_, _, _: &Selected, cx| cx.notify()));
        let observers =
            vec![ui::observe_in(&app, Topic::Locale, window, cx, |this, window, cx| {
                this.picker.update(cx, |picker, cx| {
                    picker.configure(
                        std::iter::empty(),
                        false,
                        t!("holdings.manual.search").to_owned(),
                        window,
                        cx,
                    )
                });
            })];
        Self {
            app,
            inputs,
            picker,
            portfolio,
            editing: None,
            removing: None,
            problem: None,
            _subscriptions: subscriptions,
            _observers: observers,
        }
    }

    fn edit(&mut self, entry: Option<&Holding>, window: &mut Window, cx: &mut Context<Self>) {
        self.editing = entry.map(|entry| entry.id.clone());
        self.removing = None;
        self.problem = None;
        let values = entry.map_or_else(
            || vec![String::new(); 3],
            |entry| {
                vec![
                    entry.input.platform.clone(),
                    entry.input.amount.to_string(),
                    entry.input.cost.map(|cost| cost.to_string()).unwrap_or_default(),
                ]
            },
        );
        for (input, value) in self.inputs.iter().zip(values) {
            input.update(cx, |input, cx| input.set_value(value, window, cx));
        }
        self.picker.update(cx, |picker, cx| {
            picker.set_selection(entry.map(|entry| entry.input.instrument.clone()), window, cx)
        });
        cx.notify();
    }

    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(instrument) = self.picker.read(cx).selection().cloned() else {
            return;
        };
        let values: Vec<_> =
            self.inputs.iter().map(|input| input.read(cx).value().trim().to_owned()).collect();
        let number = |index: usize| {
            values[index].parse::<f64>().map_err(|_| t!("holdings.manual.invalidNumber").to_owned())
        };
        let result = (|| {
            let input = Input {
                platform: values[0].clone(),
                instrument,
                amount: number(1)?,
                cost: if values[2].is_empty() { None } else { Some(number(2)?) },
            };
            match &self.editing {
                Some(id) => api::replace_manual_holding(&self.app, id, input),
                None => api::create_manual_holding(&self.app, input),
            }
            .map_err(|error| error.to_string())
        })();
        match result {
            Ok(_) => self.edit(None, window, cx),
            Err(error) => self.problem = Some(error.into()),
        }
        cx.notify();
    }

    fn remove(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        match api::remove_manual_holding(&self.app, id) {
            Ok(()) => {
                if self.editing.as_deref() == Some(id) {
                    self.edit(None, window, cx);
                }
                self.removing = None;
                self.problem = None;
            }
            Err(error) => self.problem = Some(error.to_string().into()),
        }
        cx.notify();
    }

    fn form(&self, p: Palette, cx: &mut Context<Self>) -> Div {
        let selected = self.picker.read(cx).selection().cloned();
        let currency = selected.as_ref().map(|instrument| instrument.quote.as_str()).unwrap_or("—");
        let metadata = selected.as_ref().map(Instrument::metadata);
        let unit = metadata
            .as_ref()
            .map(|metadata| ui::widgets::quantity_unit(metadata.quantity_unit))
            .unwrap_or(t!("common.quantity.units"));
        let labels = [
            t!("holdings.manual.platform").to_owned(),
            t!("holdings.manual.amount", unit = unit),
            t!("holdings.manual.cost", currency = currency),
        ];
        let mut form = div()
            .w(px(280.))
            .flex_none()
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(div().text_size(px(14.)).font_weight(gpui_kit::FontWeight::SEMIBOLD).child(
                if self.editing.is_some() {
                    t!("holdings.manual.edit")
                } else {
                    t!("holdings.manual.add")
                },
            ))
            .child(div().text_size(px(11.)).text_color(p.muted).child(t!("holdings.manual.search")))
            .child(self.picker.clone());
        if let Some(metadata) = &metadata {
            let hint = match (metadata.price_kind, metadata.quantity_unit) {
                (PriceKind::Nav { .. }, _) => {
                    Some(t!("holdings.manual.navHint", currency = currency))
                }
                (_, QuantityUnit::Grams) => Some(t!("holdings.manual.gramHint").to_owned()),
                _ => None,
            };
            if let Some(hint) = hint {
                form = form.child(div().text_size(px(11.)).text_color(p.muted).child(hint));
            }
        }
        for (label, input) in labels.into_iter().zip(&self.inputs) {
            form = form.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(3.))
                    .child(div().text_size(px(11.)).text_color(p.muted).child(label.clone()))
                    .child(TextInput::new(input).small().aria_label(label)),
            );
        }
        let complete = selected.is_some()
            && self.inputs[..2].iter().all(|input| !input.read(cx).value().trim().is_empty());
        form.child(
            div()
                .flex()
                .gap(px(8.))
                .child(
                    Button::new("manual-save")
                        .small()
                        .label(t!("holdings.manual.save"))
                        .disabled(!complete)
                        .on_click(cx.listener(|this, _, window, cx| this.save(window, cx))),
                )
                .child(
                    Button::new("manual-reset")
                        .ghost()
                        .small()
                        .label(t!("holdings.manual.cancel"))
                        .on_click(cx.listener(|this, _, window, cx| this.edit(None, window, cx))),
                ),
        )
    }

    fn card(
        &self,
        value: Value,
        currency: crate::currency::Currency,
        p: Palette,
        cx: &mut Context<Self>,
    ) -> Div {
        let entry = value.holding;
        let id = entry.id.clone();
        let instrument = &entry.input.instrument;
        let price = value
            .price
            .map(|price| format::price(price, format::decimals(price, instrument.decimals)))
            .unwrap_or_else(|| "—".to_owned());
        let total = value
            .value
            .map(|value| format::price(value, currency.decimals()))
            .unwrap_or_else(|| "—".to_owned());
        let mut card = ui::widgets::panel(div(), p)
            .p(px(12.))
            .flex_none()
            .flex()
            .flex_col()
            .gap(px(6.))
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
                            .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                            .child(format!(
                                "{} · {}",
                                instrument.pair_label(),
                                entry.input.platform
                            )),
                    )
                    .child(ui::tabular(
                        div().whitespace_nowrap().child(format!("{total} {}", currency.code())),
                    )),
            )
            .child(div().text_size(px(11.)).text_color(p.muted).child(format!(
                "{} × {price} {} · {}",
                format::amount(entry.input.amount),
                instrument.quote,
                instrument.provider.name()
            )))
            .children(entry.input.cost.map(|cost| {
                div().text_size(px(11.)).text_color(p.muted).child(format!(
                    "{}: {}",
                    t!("holdings.manual.cost", currency = instrument.quote.clone()),
                    format::amount(cost)
                ))
            }))
            .children(value.price_updated.or(value.received).map(|time| {
                let text = if let PriceKind::Nav { utc_offset } = value.metadata.price_kind {
                    let date = crate::calendar::civil(time as i64 / 1000 + utc_offset);
                    t!(
                        "holdings.manual.navDate",
                        date = format!("{:04}-{:02}-{:02}", date.year, date.month, date.day)
                    )
                } else {
                    t!("holdings.manual.priceUpdated", time = format::timestamp(time))
                };
                div()
                    .text_size(px(10.))
                    .text_color(if value.stale { p.amber } else { p.muted })
                    .child(text)
            }))
            .children(
                value.error.map(|error| div().text_size(px(11.)).text_color(p.amber).child(error)),
            )
            .children(value.stale.then(|| {
                div().text_size(px(10.)).text_color(p.amber).child(t!("holdings.manual.stale"))
            }))
            .child(
                div()
                    .text_size(px(10.))
                    .text_color(p.muted)
                    .child(t!("holdings.manual.updated", time = format::timestamp(entry.updated))),
            );
        if self.removing.as_deref() == Some(&id) {
            let remove_id = id.clone();
            card = card
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(p.amber)
                        .child(t!("holdings.manual.confirmDelete")),
                )
                .child(
                    div()
                        .flex()
                        .gap(px(8.))
                        .child(
                            Button::new(SharedString::from(format!("manual-confirm-{id}")))
                                .small()
                                .label(t!("holdings.manual.delete"))
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.remove(&remove_id, window, cx)
                                })),
                        )
                        .child(
                            Button::new(SharedString::from(format!("manual-keep-{id}")))
                                .small()
                                .ghost()
                                .label(t!("holdings.manual.cancel"))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.removing = None;
                                    cx.notify();
                                })),
                        ),
                );
        } else {
            card = card.child(
                div()
                    .flex()
                    .gap(px(8.))
                    .child(
                        Button::new(SharedString::from(format!("manual-edit-{id}")))
                            .small()
                            .ghost()
                            .label(t!("holdings.manual.edit"))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.edit(Some(&entry), window, cx)
                            })),
                    )
                    .child(
                        Button::new(SharedString::from(format!("manual-delete-{id}")))
                            .small()
                            .ghost()
                            .label(t!("holdings.manual.delete"))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.removing = Some(id.clone());
                                cx.notify();
                            })),
                    ),
            );
        }
        card
    }
}

impl Render for ManualView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Palette::current(&self.app, cx);
        let portfolio = self.portfolio.clone();
        let mut list = div()
            .id("manual-records")
            .flex_1()
            .min_w_0()
            .min_h_0()
            .overflow_y_scrollbar()
            .flex()
            .flex_col()
            .gap(px(10.));
        if portfolio.manual_holdings.is_empty() {
            list = list.child(
                div().text_size(px(12.)).text_color(p.muted).child(t!("holdings.manual.empty")),
            );
        }
        for value in portfolio.manual_holdings.iter().cloned() {
            list = list.child(self.card(value, portfolio.valuation.currency, p, cx));
        }
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .gap(px(12.))
            .p(px(16.))
            .child(div().text_size(px(12.)).text_color(p.muted).child(t!("holdings.manual.detail")))
            .children(
                portfolio
                    .manual_error
                    .clone()
                    .map(|error| div().text_size(px(12.)).text_color(p.red).child(error)),
            )
            .children(self.problem.clone().map(|error| {
                div().id("manual-problem").text_size(px(12.)).text_color(p.red).child(error)
            }))
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .gap(px(20.))
                    .child(
                        div()
                            .id("manual-form-scroll")
                            .w(px(280.))
                            .flex_none()
                            .min_h_0()
                            .overflow_y_scrollbar()
                            .child(self.form(p, cx)),
                    )
                    .child(list),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        market::ProviderId,
        model::{Settings, Shared},
    };
    use gpui_kit::{
        Bounds, TestAppContext, WindowBounds, WindowOptions, size, test::TestWindowExt as _,
    };

    // UX contract: without any synced account the user can select a quoted
    // instrument, save/edit quantity and cost, and cancel or confirm deletion.
    #[test]
    fn register_edit_and_confirm_delete_without_an_exchange_key() {
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
        let mut root = None;
        let window = cx.update(|cx| {
            gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                        Default::default(),
                        size(px(800.), px(500.)),
                    ))),
                    ..Default::default()
                },
                cx,
                |window, cx| {
                    let view =
                        cx.new(|cx| super::super::HoldingsView::new(app.clone(), window, cx));
                    root = Some(view.clone());
                    view
                },
            )
            .unwrap()
            .0
        });
        let root = root.unwrap();
        cx.update(|cx| window.update(cx, |_, window, cx| window.click("add-manual", cx)).unwrap());
        let editor = cx.update(|cx| root.read(cx).manual.clone());
        let instrument = Instrument {
            provider: ProviderId::Ths,
            symbol: "510300.SH".into(),
            base: "510300".into(),
            quote: "CNY".into(),
            name: Some("沪深300ETF".into()),
            decimals: Some(3),
            pinned: false,
            venue: None,
        };
        cx.update(|cx| {
            window
                .update(cx, |_, window, cx| {
                    editor.update(cx, |view, cx| {
                        view.picker.update(cx, |picker, cx| {
                            picker.set_selection(Some(instrument), window, cx)
                        });
                        cx.notify();
                    });
                    let inputs = editor.read(cx).inputs.clone();
                    for (input, value) in inputs.iter().zip(["Broker", "100", "3.5"]) {
                        input.update(cx, |input, cx| input.set_value(value, window, cx));
                    }
                    window.click("manual-save", cx);
                })
                .unwrap();
        });
        cx.run_until_parked();
        let entries = api::get_manual_holdings(&app).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].input.amount, 100.);
        assert_eq!(entries[0].input.cost, Some(3.5));
        let id = &entries[0].id;
        cx.update(|cx| {
            window
                .update(cx, |_, window, cx| {
                    window.click(SharedString::from(format!("manual-edit-{id}")), cx);
                    let amount = editor.read(cx).inputs[1].clone();
                    amount.update(cx, |input, cx| input.set_value("200", window, cx));
                    window.click("manual-save", cx);
                })
                .unwrap()
        });
        cx.run_until_parked();
        assert_eq!(api::get_manual_holdings(&app).unwrap()[0].input.amount, 200.);
        cx.update(|cx| {
            window
                .update(cx, |_, window, cx| {
                    window.click(SharedString::from(format!("manual-delete-{id}")), cx);
                    window.click(SharedString::from(format!("manual-keep-{id}")), cx);
                })
                .unwrap()
        });
        assert_eq!(api::get_manual_holdings(&app).unwrap().len(), 1, "cancel keeps the record");
        cx.update(|cx| {
            window
                .update(cx, |_, window, cx| {
                    window.click(SharedString::from(format!("manual-delete-{id}")), cx);
                    window.click(SharedString::from(format!("manual-confirm-{id}")), cx);
                })
                .unwrap()
        });
        cx.run_until_parked();
        assert!(api::get_manual_holdings(&app).unwrap().is_empty());
    }
}
