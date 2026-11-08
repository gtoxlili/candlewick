//! Register quantities against a selected market instrument. Prices are read
//! from the same feeds as the watchlist, independently of where assets are held.

use crate::{
    api,
    app::{App, Topic},
    format,
    manual::{Holding, Input, Value},
    model::Instrument,
    ui::{
        self,
        instrument_picker::{InstrumentPicker, Selected},
        settings::parts,
        theme::Palette,
    },
};
use gpui_kit::{
    AppContext as _, Context, Div, Entity, InteractiveElement as _, IntoElement, ParentElement,
    Render, SharedString, Styled, Subscription, Task, TestSupportExt as _, Window,
    component::{
        Disableable as _, Sizable as _,
        button::{Button, ButtonVariants as _},
        input::{Input as TextInput, InputEvent, InputState},
        scroll::ScrollableElement as _,
        tag::Tag,
    },
    div, px,
};
use std::sync::Arc;

pub struct ManualView {
    app: App,
    portfolio: Arc<crate::portfolio::Portfolio>,
    picker: Entity<InstrumentPicker>,
    inputs: [Entity<InputState>; 3],
    editing: Option<String>,
    removing: Option<String>,
    problem: Option<SharedString>,
    _subscriptions: Vec<Subscription>,
    _observers: Vec<Task<()>>,
}

impl ManualView {
    pub fn new(app: App, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let portfolio = Arc::new(api::get_portfolio(&app));
        let mut subscriptions = Vec::new();
        let inputs = [
            t!("holdings.manual.platformPlaceholder"),
            t!("holdings.manual.amountPlaceholder"),
            t!("holdings.manual.costPlaceholder"),
        ]
        .map(|placeholder| {
            let input = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
            subscriptions.push(cx.subscribe(&input, |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }));
            input
        });
        let picker = cx.new(|cx| {
            InstrumentPicker::new(
                app.clone(),
                "manual",
                t!("holdings.manual.search").to_owned(),
                window,
                cx,
            )
        });
        subscriptions.push(cx.subscribe_in(
            &picker,
            window,
            |this: &mut Self, _, _: &Selected, window, cx| {
                if this.editing.take().is_some() {
                    this.clear_inputs(window, cx);
                }
                this.removing = None;
                this.problem = None;
                cx.notify();
            },
        ));
        let mut observers = ui::setup(&app, window, cx);
        observers.push(ui::observe(&app, Topic::Portfolio, cx, |this: &mut Self, _| {
            this.portfolio = Arc::new(api::get_portfolio(&this.app));
        }));
        observers.push(ui::observe_in(
            &app,
            Topic::Locale,
            window,
            cx,
            |this: &mut Self, window, cx| {
                this.portfolio = Arc::new(api::get_portfolio(&this.app));
                for (input, placeholder) in this.inputs.iter().zip([
                    t!("holdings.manual.platformPlaceholder"),
                    t!("holdings.manual.amountPlaceholder"),
                    t!("holdings.manual.costPlaceholder"),
                ]) {
                    input.update(cx, |input, cx| input.set_placeholder(placeholder, window, cx));
                }
                this.picker.update(cx, |picker, cx| {
                    picker.configure(
                        std::iter::empty(),
                        false,
                        t!("holdings.manual.search").to_owned(),
                        window,
                        cx,
                    )
                });
            },
        ));
        observers.push(ui::observe_title(&app, window, cx, |_: &mut Self, _| {
            t!("holdings.manual.title").to_owned()
        }));
        Self {
            app,
            portfolio,
            picker,
            inputs,
            editing: None,
            removing: None,
            problem: None,
            _subscriptions: subscriptions,
            _observers: observers,
        }
    }

    #[cfg(any(test, all(target_os = "macos", feature = "test-support")))]
    pub(crate) fn picker(&self) -> Entity<InstrumentPicker> {
        self.picker.clone()
    }

    pub(crate) fn begin_edit(
        &mut self,
        entry: &Holding,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.editing = Some(entry.id.clone());
        self.removing = None;
        self.problem = None;
        self.picker.update(cx, |picker, cx| picker.set_selection(None, window, cx));
        let values = [
            entry.input.platform.clone(),
            format::amount(entry.input.amount),
            entry.input.cost.map(format::amount).unwrap_or_default(),
        ];
        for (input, value) in self.inputs.iter().zip(values) {
            input.update(cx, |input, cx| input.set_value(value, window, cx));
        }
        cx.notify();
    }

    fn clear_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for input in &self.inputs {
            input.update(cx, |input, cx| input.set_value(String::new(), window, cx));
        }
    }

    fn reset(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.editing = None;
        self.problem = None;
        self.picker.update(cx, |picker, cx| picker.set_selection(None, window, cx));
        self.clear_inputs(window, cx);
        cx.notify();
    }

    fn target_instrument(&self, cx: &gpui_kit::App) -> Option<Instrument> {
        if let Some(id) = &self.editing {
            self.portfolio
                .manual_holdings
                .iter()
                .find(|v| &v.holding.id == id)
                .map(|v| v.holding.input.instrument.clone())
        } else {
            self.picker.read(cx).selection().cloned()
        }
    }

    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(instrument) = self.target_instrument(cx) else {
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
            Ok(_) => {
                self.portfolio = Arc::new(api::get_portfolio(&self.app));
                self.reset(window, cx);
            }
            Err(error) => {
                self.problem = Some(error.into());
                cx.notify();
            }
        }
    }

    fn remove(&mut self, id: &str, cx: &mut Context<Self>) {
        match api::remove_manual_holding(&self.app, id) {
            Ok(()) => {
                self.removing = None;
                self.problem = None;
                self.portfolio = Arc::new(api::get_portfolio(&self.app));
            }
            Err(error) => self.problem = Some(error.to_string().into()),
        }
        cx.notify();
    }

    fn form_fields(&self, instrument: &Instrument, p: Palette, cx: &mut Context<Self>) -> Div {
        let metadata = instrument.metadata();
        let unit = ui::widgets::quantity_unit(metadata.quantity_unit);
        let currency = instrument.quote.as_str();
        let field = |label: String, input: &Entity<InputState>| {
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(div().text_xs().text_color(p.muted).whitespace_nowrap().child(label.clone()))
                .child(TextInput::new(input).small().aria_label(label))
        };
        let complete =
            self.inputs[..2].iter().all(|input| !input.read(cx).value().trim().is_empty());
        let mut body = div()
            .px_3()
            .pt_1()
            .pb_3()
            .flex()
            .flex_col()
            .gap_2()
            .child(field(t!("holdings.manual.platform").to_owned(), &self.inputs[0]))
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(div().flex_1().min_w_0().child(field(
                        t!("holdings.manual.amount", unit = unit),
                        &self.inputs[1],
                    )))
                    .child(div().flex_1().min_w_0().child(field(
                        t!("holdings.manual.cost", currency = currency),
                        &self.inputs[2],
                    ))),
            );
        if let Some(error) = &self.problem {
            body = body.child(
                div()
                    .id("manual-problem")
                    .text_xs()
                    .text_color(p.red)
                    .whitespace_normal()
                    .child(error.clone()),
            );
        }
        body.child(
            div()
                .flex()
                .items_center()
                .justify_end()
                .gap_2()
                .pt_0p5()
                .child(
                    Button::new("manual-reset")
                        .small()
                        .label(t!("holdings.manual.cancel"))
                        .on_click(cx.listener(|this, _, window, cx| this.reset(window, cx))),
                )
                .child(
                    Button::new("manual-save")
                        .primary()
                        .small()
                        .label(t!("holdings.manual.save"))
                        .disabled(!complete)
                        .on_click(cx.listener(|this, _, window, cx| this.save(window, cx))),
                ),
        )
    }

    fn row(
        &self,
        value: &Value,
        currency: crate::currency::Currency,
        p: Palette,
        cx: &mut Context<Self>,
    ) -> Div {
        let entry = &value.holding;
        let id = entry.id.clone();
        let instrument = &entry.input.instrument;
        if self.editing.as_deref() == Some(&id) {
            return div()
                .flex()
                .flex_col()
                .child(
                    div()
                        .h(px(40.))
                        .px_3()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_2()
                        .child(instrument_title(instrument, p))
                        .child(Tag::secondary().small().child(instrument.provider.name())),
                )
                .child(self.form_fields(instrument, p, cx));
        }

        let unit = ui::widgets::quantity_unit(value.metadata.quantity_unit);
        let mut detail = format!(
            "{} · {} {}",
            entry.input.platform,
            format::amount(entry.input.amount),
            unit
        );
        if let Some(cost) = entry.input.cost {
            let cost_str = format::price(cost, format::decimals(cost, instrument.decimals));
            detail.push_str(&format!(" · {}", t!("holdings.manual.costShort", value = cost_str)));
        }

        let right = if self.removing.as_deref() == Some(&id) {
            let remove_id = id.clone();
            div()
                .flex_shrink_0()
                .flex()
                .items_center()
                .gap_1()
                .child(
                    Button::new(SharedString::from(format!("manual-keep-{id}")))
                        .ghost()
                        .small()
                        .label(t!("holdings.manual.cancel"))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.removing = None;
                            cx.notify();
                        })),
                )
                .child(
                    Button::new(SharedString::from(format!("manual-confirm-{id}")))
                        .danger()
                        .small()
                        .label(t!("holdings.manual.delete"))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.remove(&remove_id, cx);
                        })),
                )
        } else {
            let total = value
                .value
                .map(|v| format!("{} {}", format::price(v, currency.decimals()), currency.code()))
                .unwrap_or_else(|| "—".to_owned());
            let quote = value
                .price
                .map(|pr| {
                    format!(
                        "{} {}",
                        format::price(pr, format::decimals(pr, instrument.decimals)),
                        instrument.quote
                    )
                })
                .unwrap_or_else(|| t!("holdings.manual.noQuote").to_owned());
            let edit_entry = entry.clone();
            let delete_id = id.clone();
            div()
                .flex_shrink_0()
                .flex()
                .items_center()
                .gap_2()
                .child(ui::tabular(
                    div()
                        .flex()
                        .flex_col()
                        .items_end()
                        .gap(px(1.))
                        .child(
                            div()
                                .text_size(px(13.))
                                .font_weight(gpui_kit::FontWeight::MEDIUM)
                                .whitespace_nowrap()
                                .child(total),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(p.muted)
                                .whitespace_nowrap()
                                .child(quote),
                        ),
                ))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(2.))
                        .child(
                            Button::new(SharedString::from(format!("manual-edit-{id}")))
                                .ghost()
                                .small()
                                .label(t!("holdings.manual.edit"))
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.begin_edit(&edit_entry, window, cx);
                                })),
                        )
                        .child(
                            Button::new(SharedString::from(format!("manual-delete-{id}")))
                                .ghost()
                                .small()
                                .label("×")
                                .accessibility_label(t!("holdings.manual.delete"))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.removing = Some(delete_id.clone());
                                    cx.notify();
                                })),
                        ),
                )
        };

        div()
            .px_3()
            .py(px(10.))
            .min_h(px(52.))
            .flex()
            .items_center()
            .justify_between()
            .gap_3()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(instrument_title(instrument, p))
                    .child(ui::tabular(
                        div().text_xs().text_color(p.muted).truncate().child(detail),
                    )),
            )
            .child(right)
    }
}

fn instrument_title(instrument: &Instrument, p: Palette) -> Div {
    let (name, code) = instrument.row_label();
    let code = code.trim().to_owned();
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
                .font_weight(gpui_kit::FontWeight::MEDIUM)
                .text_color(p.foreground)
                .child(name),
        )
        .children((!code.is_empty()).then(|| {
            div().flex_shrink_0().text_xs().text_color(p.muted).child(code)
        }))
}

impl Render for ManualView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Palette::current(&self.app, cx);
        let selected =
            if self.editing.is_none() { self.picker.read(cx).selection().cloned() } else { None };
        let mut add_section =
            parts::section(t!("holdings.manual.add"), &self.app, cx).child(self.picker.clone());
        if let Some(instrument) = &selected {
            let group = parts::group(&self.app, cx).child(
                div()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .h(px(40.))
                            .px_3()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap_2()
                            .child(instrument_title(instrument, p))
                            .child(Tag::secondary().small().child(instrument.provider.name())),
                    )
                    .child(self.form_fields(instrument, p, cx)),
            );
            add_section = add_section.child(group);
        }

        let mut list_group = parts::group(&self.app, cx);
        if self.portfolio.manual_holdings.is_empty() {
            list_group = list_group.child(
                div()
                    .px_3()
                    .py_4()
                    .text_color(p.muted)
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .child(t!("holdings.manual.empty")),
            );
        } else {
            let holdings = self.portfolio.manual_holdings.clone();
            let currency = self.portfolio.valuation.currency;
            for (index, value) in holdings.iter().enumerate() {
                let mut row = self.row(value, currency, p, cx);
                if index > 0 {
                    row = row.border_t_1().border_color(p.border);
                }
                list_group = list_group.child(row);
            }
        }
        let mut list_section =
            parts::section(t!("holdings.manual.title"), &self.app, cx).child(list_group);
        if let Some(error) = &self.portfolio.manual_error {
            list_section = list_section.child(parts::problem("manual-load-error", error.clone()));
        }

        ui::frame(t!("holdings.manual.title").to_owned(), cx).child(
            div()
                .id("manual-scroll")
                .test_support()
                .flex_1()
                .min_h_0()
                .overflow_y_scrollbar()
                .px_5()
                .pt_3()
                .pb_6()
                .flex()
                .flex_col()
                .gap_5()
                .child(add_section)
                .child(list_section),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Settings, Shared};
    use gpui_kit::{
        Bounds, TestAppContext, WindowBounds, WindowOptions, size, test::TestWindowExt as _,
    };

    // UX contract: ManualView adds, edits inline, cancels without mutating,
    // and supports cancelable deletion in a single Settings-styled window.
    #[test]
    fn manual_view_adds_edits_cancels_and_deletes_inline() {
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
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                Default::default(),
                size(px(460.), px(640.)),
            ))),
            ..Default::default()
        };
        let mut view_entity = None;
        let window = cx.update(|cx| {
            gpui_kit::open_window(options, cx, |window, cx| {
                let view = cx.new(|cx| ManualView::new(app.clone(), window, cx));
                view_entity = Some(view.clone());
                view
            })
            .unwrap()
            .0
        });
        let view = view_entity.unwrap();
        let instrument = crate::market::ProviderId::Ths.provider().bare("510300.SH").unwrap();
        cx.update(|cx| {
            window
                .update(cx, |_, window, cx| {
                    view.update(cx, |this, cx| {
                        this.picker.update(cx, |picker, cx| {
                            picker.set_selection(Some(instrument.clone()), window, cx)
                        })
                    });
                    let inputs = view.read(cx).inputs.clone();
                    for (input, value) in inputs.iter().zip(["Broker", "100", "3.5"]) {
                        input.update(cx, |input, cx| input.set_value(value, window, cx));
                    }
                    window.click("manual-save", cx);
                })
                .unwrap()
        });
        cx.run_until_parked();
        let entry = api::get_manual_holdings(&app).unwrap().remove(0);
        assert_eq!((entry.input.amount, entry.input.cost), (100., Some(3.5)));

        // Begin inline edit and cancel: saved quantity stays intact.
        cx.update(|cx| {
            window
                .update(cx, |_, window, cx| {
                    window.click(SharedString::from(format!("manual-edit-{}", entry.id)), cx);
                    let amount = view.read(cx).inputs[1].clone();
                    amount.update(cx, |input, cx| input.set_value("200", window, cx));
                    window.click("manual-reset", cx);
                })
                .unwrap()
        });
        assert_eq!(
            api::get_manual_holdings(&app).unwrap()[0].input.amount,
            100.,
            "cancel leaves the saved quantity intact"
        );

        // Begin inline edit and save.
        cx.update(|cx| {
            window
                .update(cx, |_, window, cx| {
                    window.click(SharedString::from(format!("manual-edit-{}", entry.id)), cx);
                    let amount = view.read(cx).inputs[1].clone();
                    amount.update(cx, |input, cx| input.set_value("200", window, cx));
                    window.click("manual-save", cx);
                })
                .unwrap()
        });
        cx.run_until_parked();
        assert_eq!(api::get_manual_holdings(&app).unwrap()[0].input.amount, 200.);

        // Delete with cancel, then confirm.
        cx.update(|cx| {
            window
                .update(cx, |_, window, cx| {
                    window.click(SharedString::from(format!("manual-delete-{}", entry.id)), cx);
                    window.click(SharedString::from(format!("manual-keep-{}", entry.id)), cx);
                })
                .unwrap()
        });
        assert_eq!(api::get_manual_holdings(&app).unwrap().len(), 1);
        cx.update(|cx| {
            window
                .update(cx, |_, window, cx| {
                    window.click(SharedString::from(format!("manual-delete-{}", entry.id)), cx);
                    window.click(SharedString::from(format!("manual-confirm-{}", entry.id)), cx);
                })
                .unwrap()
        });
        assert!(api::get_manual_holdings(&app).unwrap().is_empty());
    }
}
