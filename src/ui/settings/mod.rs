//! The settings window.

mod about;
mod coingecko;
pub mod keys;
mod language;
mod longbridge;
pub mod parts;
mod ths;
mod watchlist;

use self::{
    about::{AgentNote, AppFooter, DisplaySection},
    coingecko::CoinGeckoSection,
    keys::ApiKeysSection,
    language::LanguageRow,
    longbridge::LongbridgeSection,
    ths::ThsSection,
    watchlist::WatchlistSection,
};
use crate::app::App;
use gpui_kit::{
    AppContext, Context, Entity, InteractiveElement, IntoElement, ParentElement, Render, Styled,
    Task, Window, component::scroll::ScrollableElement, div,
};

pub struct SettingsView {
    app: App,
    watchlist: Entity<WatchlistSection>,
    keys: Entity<ApiKeysSection>,
    longbridge: Entity<LongbridgeSection>,
    ths: Entity<ThsSection>,
    coingecko: Entity<CoinGeckoSection>,
    display: Entity<DisplaySection>,
    language: Entity<LanguageRow>,
    agent: Entity<AgentNote>,
    footer: Entity<AppFooter>,
    _observers: Vec<Task<()>>,
}
impl SettingsView {
    /// The exchange keys section, for offscreen render checks.
    #[cfg(test)]
    pub fn keys(&self) -> Entity<ApiKeysSection> {
        self.keys.clone()
    }

    pub fn new(app: App, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut observers = super::setup(&app, window, cx);
        observers
            .push(super::observe_title(&app, window, cx, |_, _| t!("window.settings").to_owned()));
        Self {
            watchlist: cx.new(|cx| WatchlistSection::new(app.clone(), window, cx)),
            keys: cx.new(|cx| ApiKeysSection::new(app.clone(), window, cx)),
            longbridge: cx.new(|cx| LongbridgeSection::new(app.clone(), window, cx)),
            ths: cx.new(|cx| ThsSection::new(app.clone(), window, cx)),
            coingecko: cx.new(|cx| CoinGeckoSection::new(app.clone(), window, cx)),
            display: cx.new(|cx| DisplaySection::new(app.clone(), window, cx)),
            language: cx.new(|cx| LanguageRow::new(app.clone(), window, cx)),
            agent: cx.new(|cx| AgentNote::new(app.clone(), window, cx)),
            footer: cx.new(|cx| AppFooter::new(app.clone(), window, cx)),
            app,
            _observers: observers,
        }
    }
}
impl Render for SettingsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        super::frame(t!("window.settings").to_owned(), cx)
            .child(
                div()
                    .id("settings-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .px_5()
                    .pt_3()
                    .pb_6()
                    .flex()
                    .flex_col()
                    .gap_5()
                    .child(self.watchlist.clone())
                    .child(self.keys.clone())
                    .child(self.longbridge.clone())
                    .child(self.ths.clone())
                    .child(self.coingecko.clone())
                    .child(self.display.clone())
                    .child(
                        parts::section(t!("settings.general.title"), &self.app, cx).child(
                            parts::group(&self.app, cx)
                                .child(self.language.clone())
                                .child(self.agent.clone()),
                        ),
                    ),
            )
            .child(
                div()
                    .px_5()
                    .pt_3()
                    .pb_3()
                    .flex_shrink_0()
                    .border_t_1()
                    .border_color(super::theme::Palette::current(&self.app, cx).border)
                    .child(self.footer.clone()),
            )
    }
}
