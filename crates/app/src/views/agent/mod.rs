//! Project-scoped agent tabs; engines remain alive when the same panel changes project/view.
mod commands;
mod engine;
mod interaction;
mod labels;
mod model_picker;
mod projection;
mod routing;
mod session;
mod slash;
mod tabs;
mod transcript;
mod usage;
mod view;
pub use view::AgentPanelEvent;

use crate::{
    services::Services,
    state::{AppEvent, AppState},
};
use autocraft_core::{
    AgentSession, ChatBinding, ChatId, Error, ModelChoice, ProjectId, Readiness, SessionPhase,
    SessionSnapshot,
};
use gpui_kit::{
    AppContext as _, Context, Entity, ScrollHandle, Subscription, Window,
    component::input::{InputEvent, InputState, TextareaState},
};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

struct Chat {
    project: ProjectId,
    binding: Option<ChatBinding>,
    session: Option<Arc<dyn AgentSession>>,
    snapshot: Option<SessionSnapshot>,
    lease: Option<engine::LaunchLease>,
    usage: usage::Usage,
    routing_signature: Option<String>,
    catalog: Vec<ModelChoice>,
    restart_pending: bool,
    starting: bool,
    closing: bool,
    blocked: Option<Readiness>,
    ordinal: usize,
    generation: u64,
    draft: String,
    pending_results: Vec<String>,
}
impl Chat {
    fn new(project: ProjectId, ordinal: usize, binding: Option<ChatBinding>) -> Self {
        Self {
            project,
            ordinal,
            binding,
            session: None,
            snapshot: None,
            lease: None,
            usage: Default::default(),
            routing_signature: None,
            catalog: Vec::new(),
            restart_pending: false,
            starting: false,
            closing: false,
            blocked: None,
            generation: 0,
            draft: String::new(),
            pending_results: Vec::new(),
        }
    }
    fn live(&self) -> Option<&Arc<dyn AgentSession>> {
        self.session.as_ref().filter(|_| {
            !self
                .snapshot
                .as_ref()
                .is_some_and(|s| s.phase == SessionPhase::Closed)
        })
    }
    fn phase(&self) -> SessionPhase {
        self.snapshot
            .as_ref()
            .filter(|_| self.live().is_some())
            .map_or(SessionPhase::Closed, |s| s.phase)
    }
    fn title(&self) -> String {
        let title = self
            .snapshot
            .as_ref()
            .and_then(|s| s.session_name.clone())
            .or_else(|| {
                self.snapshot
                    .as_ref()?
                    .messages
                    .iter()
                    .find(|m| m.role == autocraft_core::Role::User)?
                    .content
                    .iter()
                    .find_map(|b| match b {
                        autocraft_core::ContentBlock::Text { text } => {
                            Some(text.lines().next().unwrap_or_default().to_owned())
                        }
                        autocraft_core::ContentBlock::Thinking { .. }
                        | autocraft_core::ContentBlock::Image { .. }
                        | autocraft_core::ContentBlock::Tool { .. } => None,
                    })
            });
        match title.filter(|s| !s.trim().is_empty()) {
            Some(s) if s.chars().count() > 24 => {
                format!("{}…", s.chars().take(24).collect::<String>())
            }
            Some(s) => s,
            None => format!("{} {}", labels::chat(), self.ordinal),
        }
    }
}

/// One shared Entity hosts both analysis chats and the orchestration-owned builder projection.
pub struct AgentPanel {
    state: Entity<AppState>,
    chats: HashMap<ChatId, Chat>,
    order: Vec<ChatId>,
    active: Option<ChatId>,
    project: Option<ProjectId>,
    generation: u64,
    builder_selected: bool,
    builders: HashMap<ProjectId, SessionSnapshot>,
    composer: Entity<TextareaState>,
    open: HashSet<String>,
    answers: HashMap<String, Entity<InputState>>,
    picks: HashMap<String, Vec<String>>,
    scroll: ScrollHandle,
    slash: slash::SlashMenu,
    model_picker: model_picker::ModelPicker,
    usage_view: Entity<lumakit_usage_status::ContextStatusView>,
    guide: Option<Readiness>,
    novice: bool,
    _subscriptions: Vec<Subscription>,
}
impl AgentPanel {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let composer = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(3, 8)
                .submit_on_enter(true)
                .placeholder(labels::placeholder())
        });
        let subscriptions = vec![
            cx.subscribe_in(
                &state,
                window,
                |this, _, event: &AppEvent, _, cx| match event {
                    AppEvent::Project => {
                        let project = this.state.read(cx).project.clone();
                        this.focus_project(project, cx);
                    }
                    AppEvent::Routing => this.on_routing_changed(cx),
                    AppEvent::Projects
                    | AppEvent::Analysis(_)
                    | AppEvent::Inbox(_)
                    | AppEvent::Rebuild(_) => (),
                },
            ),
            cx.subscribe_in(
                &composer,
                window,
                |this, _, event: &InputEvent, window, cx| match event {
                    InputEvent::PressEnter { shift: false, .. } => this.send(window, cx),
                    InputEvent::Change => {
                        if !this.builder_selected
                            && let Some(chat) =
                                this.active.as_ref().and_then(|id| this.chats.get_mut(id))
                        {
                            chat.draft = this.composer.read(cx).value().to_string();
                        }
                        this.on_draft_changed(cx);
                    }
                    InputEvent::PressEnter { shift: true, .. }
                    | InputEvent::Focus
                    | InputEvent::Blur => (),
                },
            ),
        ];
        let model_picker = Self::new_model_picker(window, cx);
        let usage_view = cx.new(|cx| lumakit_usage_status::ContextStatusView::new(window, cx));
        let project = state.read(cx).project.clone();
        let mut panel = Self {
            state,
            composer,
            model_picker,
            usage_view,
            _subscriptions: subscriptions,
            chats: HashMap::new(),
            order: Vec::new(),
            active: None,
            project: None,
            generation: 0,
            builder_selected: false,
            builders: HashMap::new(),
            open: HashSet::new(),
            answers: HashMap::new(),
            picks: HashMap::new(),
            scroll: ScrollHandle::new(),
            slash: Default::default(),
            guide: None,
            novice: false,
        };
        panel.focus_project(project, cx);
        panel
    }
    /// Adds a suggested question without discarding a partially written draft.
    /// Returns false while no analysis chat is available.
    pub fn set_question(
        &mut self,
        question: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if question.trim().is_empty() {
            return false;
        }
        let Some(chat) = self.active.as_ref().and_then(|id| self.chats.get_mut(id)) else {
            return false;
        };
        self.builder_selected = false;
        chat.draft = question_draft(&chat.draft, question);
        let draft = chat.draft.clone();
        self.composer
            .update(cx, |input, cx| input.set_value(draft, window, cx));
        self.on_draft_changed(cx);
        cx.notify();
        true
    }

    /// Controls plain-language guidance for subsequent questions, not session history.
    pub fn set_novice(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.novice != enabled {
            self.novice = enabled;
            cx.notify();
        }
    }
    /// Returns from the orchestration-owned builder projection to the retained analysis chat.
    pub fn show_analysis(&mut self, cx: &mut Context<Self>) {
        if self.builder_selected {
            self.builder_selected = false;
            cx.notify();
        }
    }
    fn question_prompt(&self, text: String) -> String {
        if self.novice {
            format!(
                "{}\n\n{}",
                text,
                rust_i18n::t!("app.ask.novice_instruction")
            )
        } else {
            text
        }
    }
    fn services(&self, cx: &Context<Self>) -> Services {
        self.state.read(cx).services.clone()
    }
    fn report(&self, error: &Error, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| state.report(error, cx));
    }
}

fn question_draft(draft: &str, question: &str) -> String {
    if draft.trim().is_empty() {
        question.trim().to_owned()
    } else {
        format!("{}\n\n{}", draft.trim_end(), question.trim())
    }
}

#[cfg(test)]
mod question_tests {
    use super::question_draft;

    #[test]
    fn suggestion_preserves_an_unsubmitted_multiline_question() {
        let draft = "Please compare these:\n  first\n  second  ";
        assert_eq!(
            question_draft(draft, "What permissions does it ask for?"),
            "Please compare these:\n  first\n  second\n\nWhat permissions does it ask for?"
        );
        assert_eq!(question_draft(" \n ", "  Explain this  "), "Explain this");
    }
}
