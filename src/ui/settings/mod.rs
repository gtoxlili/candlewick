//! The settings window.

mod about;
mod coingecko;
mod display;
pub mod keys;
mod language;
mod longbridge;
pub mod parts;
mod ths;
mod watchlist;

#[cfg(all(target_os = "macos", feature = "test-support"))]
pub(crate) use self::longbridge::LongbridgeSection;
#[cfg(not(all(target_os = "macos", feature = "test-support")))]
use self::longbridge::LongbridgeSection;
/// Samples of the settings window for the offscreen test renderer.
#[cfg(all(target_os = "macos", feature = "test-support"))]
pub use about::samples as settings_samples;
/// Samples of the Display section for the offscreen test renderer.
#[cfg(all(target_os = "macos", feature = "test-support"))]
pub use display::samples as display_samples;

use self::{
    about::{AgentNote, AppFooter},
    coingecko::CoinGeckoSection,
    display::DisplaySection,
    keys::ApiKeysSection,
    language::LanguageRow,
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
    #[cfg(all(target_os = "macos", feature = "test-support"))]
    pub fn keys(&self) -> Entity<ApiKeysSection> {
        self.keys.clone()
    }

    /// The display section, for offscreen render checks.
    #[cfg(all(target_os = "macos", feature = "test-support"))]
    pub fn display(&self) -> Entity<DisplaySection> {
        self.display.clone()
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

    /// The Longbridge section shown in this window.
    pub(crate) fn longbridge(&self) -> Entity<LongbridgeSection> {
        self.longbridge.clone()
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
                    .child(self.longbridge())
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
