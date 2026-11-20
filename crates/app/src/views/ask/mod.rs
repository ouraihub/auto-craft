//! Beginner target questions reuse the window's single analysis conversation.
mod suggestions;
mod view;

use crate::{
    state::{AppEvent, AppState},
    views::agent::AgentPanel,
};
use gpui_kit::{Context, Entity, EventEmitter, Subscription, Window};

/// Navigation and export remain owned by the application shell.
#[derive(Debug, Clone, Copy)]
pub enum AskEvent {
    Back,
    Workbench,
    /// Open the shared palette's native report export and consent flow.
    ExportReport,
}

/// Read-only target context beside the existing, editable AgentPanel.
pub struct AskView {
    state: Entity<AppState>,
    agent: Option<Entity<AgentPanel>>,
    novice: bool,
    error: Option<String>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<AskEvent> for AskView {}

impl AskView {
    /// Creates target context; the shell supplies its existing panel with `set_agent`.
    pub fn new(state: Entity<AppState>, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        let subscription = cx.subscribe(&state, |this, _, event: &AppEvent, cx| {
            if matches!(event, AppEvent::Project) {
                this.error = None;
            }
            cx.notify();
        });
        let observation = cx.observe(&state, |_, _, cx| cx.notify());
        Self {
            state,
            agent: None,
            novice: true,
            error: None,
            _subscriptions: vec![subscription, observation],
        }
    }

    /// Embeds the shell-owned panel, never creating an engine or another chat history.
    pub fn set_agent(&mut self, agent: Entity<AgentPanel>, cx: &mut Context<Self>) {
        agent.update(cx, |agent, cx| {
            agent.show_analysis(cx);
            agent.set_novice(true, cx);
        });
        self.agent = Some(agent);
        cx.notify();
    }

    fn suggest(&mut self, question: &str, window: &mut Window, cx: &mut Context<Self>) {
        let accepted = self.agent.as_ref().is_some_and(|agent| {
            agent.update(cx, |agent, cx| agent.set_question(question, window, cx))
        });
        self.error = (!accepted).then(|| tr("app.ask.chat_unavailable"));
        cx.notify();
    }
}

fn tr(key: &'static str) -> String {
    crate::i18n::text(key).into_owned()
}
