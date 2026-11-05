//! Analysis toolbar: the drawing tools, the indicators menu, and the buttons
//! that undo or clear what is on the chart. It reads and changes the chart's
//! `Analysis` through the model, then calls `analysis_changed`.

use gpui_kit::{
    Anchor, App, AppContext as _, Context, Entity, Hsla, InteractiveElement as _, IntoElement,
    ParentElement, Render, SharedString, Styled, TestSupportExt as _, TextAlign, Window,
    assets::IconName,
    component::{
        ActiveTheme as _, Icon, Sizable as _, Size,
        button::{Button, ButtonVariants as _, Toggle},
        h_flex,
        input::{Input, InputEvent, InputState},
        popover::Popover,
        switch::Switch,
    },
    div,
    prelude::FluentBuilder as _,
    px,
};

use super::{
    analysis::Tool,
    indicators::{IndicatorKind, IndicatorSpec, MIN_PERIOD},
    model::DrawingKind,
    model_state::ChartModel,
};
use crate::ui::theme::Palette;

/// What the toolbar offers besides its tools, and when each is shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct BarActions {
    /// Undo brings back what the last removal took, so it appears only after a
    /// removal.
    undo: bool,
    /// The eraser removes every drawing, so it appears while there are any.
    clear: bool,
}

impl BarActions {
    fn new(can_undo: bool, drawings: usize) -> Self {
        Self { undo: can_undo, clear: drawings > 0 }
    }
}

/// The tools in the pill, cursor first.
fn tools() -> [Tool; 5] {
    [
        Tool::Cursor,
        Tool::Drawing(DrawingKind::Level),
        Tool::Drawing(DrawingKind::Trend),
        Tool::Drawing(DrawingKind::Fib),
        Tool::Drawing(DrawingKind::Measure),
    ]
}

fn tool_icon(tool: Tool) -> IconName {
    match tool {
        Tool::Cursor => IconName::MousePointer2,
        Tool::Drawing(DrawingKind::Level) => IconName::Minus,
        Tool::Drawing(DrawingKind::Trend) => IconName::TrendingUp,
        // Stock Lucide has no Fibonacci symbol; the staggered levels read the
        // way the retracement draws.
        Tool::Drawing(DrawingKind::Fib) => IconName::ChartNoAxesGantt,
        Tool::Drawing(DrawingKind::Measure) => IconName::Ruler,
    }
}

fn tool_label(tool: Tool) -> SharedString {
    match tool {
        Tool::Cursor => t!("chart.tool.cursor").into(),
        Tool::Drawing(DrawingKind::Level) => t!("chart.tool.level").into(),
        Tool::Drawing(DrawingKind::Trend) => t!("chart.tool.trend").into(),
        Tool::Drawing(DrawingKind::Fib) => t!("chart.tool.fib").into(),
        Tool::Drawing(DrawingKind::Measure) => t!("chart.tool.measure").into(),
    }
}

fn indicator_label(kind: IndicatorKind) -> SharedString {
    match kind {
        IndicatorKind::Simple => t!("chart.indicator.sma").into(),
        IndicatorKind::Exponential => t!("chart.indicator.ema").into(),
        IndicatorKind::Bollinger => t!("chart.indicator.boll").into(),
    }
}

/// The color the chart paints an indicator with, by the slot it fills: three
/// moving averages, then the exponential one and the bands.
pub(crate) fn indicator_color(spec: &IndicatorSpec, palette: Palette) -> Hsla {
    match spec.id.as_str() {
        "ma1" => palette.violet,
        "ma2" => palette.amber,
        "ma3" => palette.mint,
        "boll" => palette.glow,
        _ => palette.foreground,
    }
}

/// The period the field's text asks for, or `None` while it is not a period
/// the indicators take. The value is clamped, so the model and the field agree
/// once the field is left.
fn accepted_period(text: &str) -> Option<usize> {
    let period: usize = text.trim().parse().ok()?;
    (period >= MIN_PERIOD).then(|| IndicatorSpec::clamped_period(period))
}

/// The period editor of one indicator.
struct PeriodField {
    id: String,
    input: Entity<InputState>,
}

pub struct AnalysisBar {
    pub model: Entity<ChartModel>,
    periods: Vec<PeriodField>,
}

impl AnalysisBar {
    pub fn new(model: Entity<ChartModel>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let indicators = model.read(cx).analysis().indicators.clone();
        let mut periods = Vec::with_capacity(indicators.len());
        for spec in indicators {
            let id = spec.id.clone();
            let input = cx.new(|cx| {
                InputState::new(window, cx)
                    .default_value(spec.period.to_string())
                    .validate(|text, _| text.len() <= 3 && text.bytes().all(|b| b.is_ascii_digit()))
            });
            let subscribed = id.clone();
            cx.subscribe_in(&input, window, move |bar, input, event, window, cx| {
                bar.period_event(&subscribed, input, event, window, cx);
            })
            .detach();
            periods.push(PeriodField { id, input });
        }
        cx.observe(&model, |_, _, cx| cx.notify()).detach();
        Self { model, periods }
    }

    /// A typed period reaches the model as soon as it is a whole window;
    /// leaving the field shows the stored, clamped period again.
    fn period_event(
        &mut self,
        id: &str,
        input: &Entity<InputState>,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::Change => {
                let text = input.read(cx).value();
                let Some(period) = accepted_period(&text) else { return };
                let id = id.to_owned();
                self.model.update(cx, |model, cx| {
                    model.analysis_mut().set_indicator(&id, None, Some(period));
                    model.analysis_changed(cx);
                });
            }
            InputEvent::Blur | InputEvent::PressEnter { .. } => {
                let period = {
                    let model = self.model.read(cx);
                    model
                        .analysis()
                        .indicators
                        .iter()
                        .find(|spec| spec.id == id)
                        .map(|spec| spec.period)
                };
                if let Some(period) = period {
                    let text = period.to_string();
                    input.update(cx, |state, cx| state.set_value(text, window, cx));
                }
            }
            _ => {}
        }
    }

    fn period(&self, id: &str) -> Option<&Entity<InputState>> {
        self.periods.iter().find(|field| field.id == id).map(|field| &field.input)
    }

    fn action_button(
        &self,
        id: &'static str,
        icon: IconName,
        label: SharedString,
        palette: Palette,
        action: impl Fn(&mut ChartModel, &mut Context<ChartModel>) + 'static,
    ) -> Button {
        let model = self.model.clone();
        Button::new(id)
            .ghost()
            .icon(icon)
            .with_size(Size::Small)
            .w(px(28.))
            .text_color(palette.muted)
            .tooltip(label.clone())
            .accessibility_label(label)
            .on_click(move |_, _, cx| {
                model.update(cx, |model, cx| action(model, cx));
            })
    }

    /// A tool of the pill. Pressed, it lifts off the pill's fill the way the
    /// chart-type toggle does.
    fn pill_button(
        &self,
        index: usize,
        tool: Tool,
        pressed: bool,
        palette: Palette,
        cx: &App,
    ) -> Toggle {
        Toggle::new(("chart-tool", index))
            .checked(pressed)
            .icon(Icon::new(tool_icon(tool)).with_size(Size::Small))
            .tooltip(tool_label(tool))
            .with_size(Size::Small)
            .w(px(28.))
            .rounded_full()
            .text_color(if pressed { palette.foreground } else { palette.muted })
            .when(pressed, |this| this.bg(cx.theme().background).shadow_xs())
            .on_click({
                let model = self.model.clone();
                move |_, _, cx| {
                    model.update(cx, |model, cx| {
                        model.analysis_mut().set_tool(tool);
                        model.analysis_changed(cx);
                    });
                }
            })
    }

    fn pill(&self, tool: Tool, any_on: bool, palette: Palette, cx: &App) -> gpui_kit::Div {
        let mut pill = div()
            .flex()
            .items_center()
            .p(px(2.))
            .rounded_full()
            .bg(palette.fill_strong)
            .whitespace_nowrap();
        for (index, entry) in tools().into_iter().enumerate() {
            pill = pill.child(self.pill_button(index, entry, entry == tool, palette, cx));
        }
        pill.child(div().w(px(1.)).h(px(14.)).mx(px(4.)).bg(palette.border)).child(
            Popover::new("chart-indicators")
                .anchor(Anchor::TopRight)
                .offset(px(6.))
                .trigger(self.menu_button(any_on, palette, cx))
                .child(self.menu(palette, cx)),
        )
    }

    /// The menu trigger. It is a button rather than a toggle because a toggle
    /// takes the pointer down for itself, and the popover opens on that event.
    fn menu_button(&self, on: bool, palette: Palette, cx: &App) -> Button {
        let button = Button::new("chart-indicators-button")
            .ghost()
            .icon(Icon::new(IconName::Sigma).with_size(Size::Small))
            .with_size(Size::Small)
            .w(px(28.))
            .rounded_full()
            .tooltip(t!("chart.indicators"))
            .accessibility_label(t!("chart.indicators"));
        if on {
            button.bg(cx.theme().background).shadow_xs().text_color(palette.foreground)
        } else {
            button.text_color(palette.muted)
        }
    }

    /// The indicators, each with the color the chart draws it in, its period,
    /// and its switch. The width holds the longest indicator name in the
    /// catalogs on one line.
    fn menu(&self, palette: Palette, cx: &App) -> impl IntoElement + use<> {
        let indicators = self.model.read(cx).analysis().indicators.clone();
        div()
            .id("chart-indicator-menu")
            .test_support()
            .flex()
            .flex_col()
            .w(px(268.))
            .child(
                div()
                    .px(px(6.))
                    .pb(px(6.))
                    .text_xs()
                    .text_color(palette.muted)
                    .whitespace_nowrap()
                    .child(t!("chart.indicators")),
            )
            .children(
                indicators.iter().map(|spec| self.indicator_row(spec, palette).into_any_element()),
            )
    }

    fn indicator_row(&self, spec: &IndicatorSpec, palette: Palette) -> impl IntoElement + use<> {
        let label = indicator_label(spec.kind);
        let period = self.period(&spec.id);
        let model = self.model.clone();
        let id = spec.id.clone();
        let switch_label = spec.label();
        let period_label = t!("chart.periodOf", label = switch_label);
        h_flex()
            .h(px(32.))
            .px(px(6.))
            .gap_2()
            .rounded_lg()
            .hover(|this| this.bg(palette.fill))
            .child(div().size(px(8.)).rounded_full().bg(indicator_color(spec, palette)))
            .child(div().flex_1().min_w_0().whitespace_nowrap().child(label))
            .children(period.map(|input| {
                Input::new(input)
                    .id(SharedString::from(format!("chart-indicator-{}-period", spec.id)))
                    .with_size(Size::Small)
                    .w(px(46.))
                    .text_align(TextAlign::Center)
                    .aria_label(period_label)
            }))
            .child(
                Switch::new(format!("chart-indicator-{}", spec.id))
                    .with_size(Size::Small)
                    .checked(spec.on)
                    .accessibility_label(switch_label)
                    .on_change(move |on, _, cx| {
                        model.update(cx, |model, cx| {
                            model.analysis_mut().set_indicator(&id, Some(*on), None);
                            model.analysis_changed(cx);
                        });
                    }),
            )
    }
}

impl Render for AnalysisBar {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (app, tool, actions, any_on) = {
            let model = self.model.read(cx);
            let analysis = model.analysis();
            (
                model.app().clone(),
                analysis.tool,
                BarActions::new(analysis.can_undo(), analysis.drawings.len()),
                analysis.indicators.iter().any(|spec| spec.on),
            )
        };
        let palette = Palette::current(&app, cx);

        let mut bar = div().flex().items_center().gap_1().whitespace_nowrap();
        if actions.undo {
            bar = bar.child(self.action_button(
                "chart-undo",
                IconName::Undo2,
                t!("chart.undo").into(),
                palette,
                |model, cx| {
                    model.analysis_mut().undo();
                    model.analysis_changed(cx);
                },
            ));
        }
        if actions.clear {
            bar = bar.child(self.action_button(
                "chart-clear",
                IconName::Eraser,
                t!("chart.clearDrawings").into(),
                palette,
                |model, cx| {
                    model.analysis_mut().clear_all();
                    model.analysis_changed(cx);
                },
            ));
        }
        bar.child(self.pill(tool, any_on, palette, cx))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pill_lists_the_tools_and_shows_only_the_actions_that_apply() {
        assert_eq!(
            tools(),
            [
                Tool::Cursor,
                Tool::Drawing(DrawingKind::Level),
                Tool::Drawing(DrawingKind::Trend),
                Tool::Drawing(DrawingKind::Fib),
                Tool::Drawing(DrawingKind::Measure),
            ]
        );
        // Nothing drawn: no eraser. Nothing removed: no undo.
        assert_eq!(BarActions::new(false, 0), BarActions { undo: false, clear: false });
        // Drawings on the chart: the eraser. A removed drawing: also undo.
        assert_eq!(BarActions::new(false, 2), BarActions { undo: false, clear: true });
        assert_eq!(BarActions::new(true, 1), BarActions { undo: true, clear: true });
        // Undo outlives the drawings it brings back.
        assert_eq!(BarActions::new(true, 0), BarActions { undo: true, clear: false });
    }

    #[test]
    fn a_period_needs_two_candles_and_is_clamped_before_it_reaches_the_model() {
        assert_eq!(accepted_period(""), None);
        assert_eq!(accepted_period("0"), None);
        assert_eq!(accepted_period("1"), None);
        assert_eq!(accepted_period("-5"), None);
        assert_eq!(accepted_period("abc"), None);
        assert_eq!(accepted_period("2"), Some(MIN_PERIOD));
        assert_eq!(accepted_period("12"), Some(12));
        assert_eq!(accepted_period(" 99 "), Some(99));
        assert_eq!(accepted_period("999"), Some(IndicatorSpec::clamped_period(999)));
        assert_eq!(accepted_period("999"), Some(500));
    }

    #[test]
    fn every_tool_has_its_own_icon_and_name() {
        let icons: Vec<_> = tools().into_iter().map(tool_icon).collect();
        for (index, icon) in icons.iter().enumerate() {
            assert!(!icons[..index].contains(icon), "two tools share an icon");
        }
        assert!(tools().into_iter().all(|tool| !tool_label(tool).is_empty()));
    }
}
