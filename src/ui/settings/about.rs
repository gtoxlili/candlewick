//! Display, general, agent access and update settings.

use super::parts;
use crate::{
    app::{App, Topic},
    ui,
};
use gpui_kit::{Context, IntoElement, Render, Task, Window};

macro_rules! placeholder {
    ($name:ident, $title:tt) => {
        pub struct $name {
            app: App,
            _observers: Vec<Task<()>>,
        }
        impl $name {
            pub fn new(app: App, _: &mut Window, cx: &mut Context<Self>) -> Self {
                let observers = vec![ui::observe(&app, Topic::Locale, cx, |_, _| {})];
                Self { app, _observers: observers }
            }
        }
        impl Render for $name {
            fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
                parts::placeholder(t!($title), &self.app, cx)
            }
        }
    };
}
placeholder!(DisplaySection, "settings.display.title");
placeholder!(AgentNote, "settings.general.agentAccess");
placeholder!(AppFooter, "settings.update.check");
