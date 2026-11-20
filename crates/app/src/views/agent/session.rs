//! Prompt/interaction traffic and monotonic live projections, including deferred long-tool replies.
use super::{
    AgentPanel,
    commands::{self, ModelChange, Submit},
    projection,
};
use crate::state::run;
use autocraft_core::{
    ChatId, ChatScope, Error, InteractionAnswer, RebuildEvent, SessionEvent, SessionPhase,
};
use gpui_kit::{Context, Window};

impl AgentPanel {
    pub(super) fn send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.builder_selected {
            return;
        }
        let text = self.composer.read(cx).value().trim().to_owned();
        if text.is_empty() {
            return;
        }
        let Some(id) = self.active.clone() else {
            return;
        };
        let Some(chat) = self.chats.get(&id) else {
            return;
        };
        if chat.blocked.is_some()
            || chat.starting
            || chat.closing
            || matches!(chat.phase(), SessionPhase::Running | SessionPhase::Waiting)
        {
            return;
        }
        let host = self
            .host_offer(chat, cx)
            .map(|(_, host)| host)
            .unwrap_or_default();
        if let Some(parsed) = commands::parse(&text, &host) {
            let accepted = match parsed {
                Ok(submit) => self.run_host(&id, submit, cx),
                Err(message) => {
                    self.report(&Error::invalid(message), cx);
                    false
                }
            };
            if accepted {
                self.clear_draft(&id, window, cx);
            }
            return;
        }
        let session = chat.live().cloned();
        self.clear_draft(&id, window, cx);
        if let Some(chat) = self.chats.get_mut(&id) {
            chat.usage.invalidate();
        }
        match session {
            Some(session) => {
                let generation = self.chats.get(&id).map(|c| c.generation);
                let sent = self.question_prompt(text.clone());
                run(
                    cx,
                    move || session.prompt(&sent),
                    move |this, result, cx| {
                        if this.chats.get(&id).map(|c| c.generation) != generation {
                            return;
                        }
                        if let Err(error) = result {
                            if let Some(chat) = this.chats.get_mut(&id) {
                                chat.draft = text;
                            }
                            this.report(&error, cx);
                        }
                        this.refresh_usage(&id, cx);
                        cx.notify();
                    },
                );
            }
            None => self.start(id, Some(text), cx),
        }
    }
    fn clear_draft(&mut self, id: &ChatId, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(chat) = self.chats.get_mut(id) {
            chat.draft.clear();
        }
        self.composer
            .update(cx, |input, cx| input.set_value("", window, cx));
    }
    fn run_host(&mut self, id: &ChatId, submit: Submit, cx: &mut Context<Self>) -> bool {
        let Some(chat) = self.chats.get(id) else {
            return false;
        };
        let Some(session) = chat.live().cloned() else {
            self.start(id.clone(), None, cx);
            return false;
        };
        match submit {
            Submit::Compact { instructions } => {
                let id = id.clone();
                if let Some(chat) = self.chats.get_mut(&id) {
                    chat.usage.invalidate();
                    chat.usage.compacting = true;
                }
                run(
                    cx,
                    move || session.compact(instructions.as_deref()),
                    move |this, result, cx| {
                        if let Some(chat) = this.chats.get_mut(&id) {
                            chat.usage.compacting = false;
                        }
                        if let Err(error) = result {
                            this.report(&error, cx);
                        }
                        this.refresh_usage(&id, cx);
                        cx.notify();
                    },
                );
                true
            }
            Submit::Name { title } => {
                run(
                    cx,
                    move || session.rename(&title),
                    |this, result, cx| {
                        if let Err(error) = result {
                            this.report(&error, cx);
                        }
                    },
                );
                true
            }
            Submit::Model { query, effort } => {
                let current = chat.snapshot.as_ref().and_then(|s| s.model.as_ref());
                match commands::resolve_model(&query, effort, &chat.catalog, current) {
                    Ok(ModelChange::Switch { choice, effort }) => {
                        self.set_model(id.clone(), choice, effort, cx)
                    }
                    Ok(ModelChange::Effort(effort)) => self.set_effort(id.clone(), effort, cx),
                    Err(message) => {
                        self.report(&Error::invalid(message), cx);
                        false
                    }
                }
            }
        }
    }
    pub(super) fn stop(&mut self, cx: &mut Context<Self>) {
        if self.builder_selected {
            return;
        }
        let Some(id) = self.active.clone() else {
            return;
        };
        let Some(chat) = self.chats.get_mut(&id) else {
            return;
        };
        chat.pending_results.clear();
        let Some(session) = chat.live().cloned() else {
            return;
        };
        let services = self.services(cx);
        run(
            cx,
            move || {
                services.cancel_chat_tools(&id);
                session.stop()
            },
            |this, result, cx| {
                if let Err(error) = result {
                    this.report(&error, cx);
                }
            },
        );
    }
    pub(super) fn respond(
        &mut self,
        request: String,
        answer: InteractionAnswer,
        cx: &mut Context<Self>,
    ) {
        if self.builder_selected {
            return;
        }
        let Some(session) = self
            .active
            .as_ref()
            .and_then(|id| self.chats.get(id))
            .and_then(|c| c.live().cloned())
        else {
            return;
        };
        self.answers.retain(|key, _| !key.starts_with(&request));
        self.picks.retain(|key, _| !key.starts_with(&request));
        run(
            cx,
            move || session.respond(&request, answer),
            |this, result, cx| {
                if let Err(error) = result {
                    this.report(&error, cx);
                }
            },
        );
    }
    /// Receives the parent's actual engine stream; old revisions never overwrite newer output.
    pub fn on_session_event(&mut self, event: SessionEvent, cx: &mut Context<Self>) {
        let id = event.chat().clone();
        let Some(chat) = self.chats.get_mut(&id) else {
            return;
        };
        let previous = chat.phase();
        let model = chat.snapshot.as_ref().and_then(|s| s.model.clone());
        if !projection::apply(&mut chat.snapshot, event) {
            return;
        }
        let phase = chat.phase();
        let changed = chat.snapshot.as_ref().and_then(|s| s.model.as_ref()) != model.as_ref();
        if changed || (previous == SessionPhase::Idle && phase != previous) {
            chat.usage.invalidate();
        }
        let retire =
            phase == SessionPhase::Closed || (phase == SessionPhase::Idle && chat.restart_pending);
        if retire && (chat.lease.is_some() || chat.session.is_some()) {
            self.retire(&id, cx);
        } else if phase == SessionPhase::Idle {
            self.flush_results(&id, cx);
            if previous != phase || changed {
                self.refresh_usage(&id, cx);
            }
        }
        if self.active.as_ref() == Some(&id) && !self.builder_selected {
            self.scroll.scroll_to_bottom();
        }
        cx.notify();
    }
    /// Builder output is the rebuild service's shared session, never a UI-created runtime.
    pub fn on_rebuild_event(&mut self, event: RebuildEvent, cx: &mut Context<Self>) {
        match event {
            RebuildEvent::Agent { project, event } => {
                let mut snapshot = self.builders.remove(&project);
                if snapshot.as_ref().is_some_and(|s| &s.id != event.chat()) {
                    snapshot = None;
                }
                projection::apply(&mut snapshot, event);
                if let Some(snapshot) = snapshot {
                    self.builders.insert(project, snapshot);
                }
                cx.notify();
            }
            RebuildEvent::Step { .. }
            | RebuildEvent::NoteRead { .. }
            | RebuildEvent::Ask { .. }
            | RebuildEvent::Checked { .. }
            | RebuildEvent::Preview { .. }
            | RebuildEvent::Finished { .. }
            | RebuildEvent::Queue { .. } => (),
        }
    }
    /// Delivers async analysis results in the same chat; unsupported steer waits for idle.
    pub fn on_long_tool_result(&mut self, scope: ChatScope, text: String, cx: &mut Context<Self>) {
        let Some(chat) = self.chats.get_mut(&scope.chat) else {
            return;
        };
        if chat.project != scope.project {
            return;
        }
        let Some(session) = chat.live().cloned() else {
            return;
        };
        let running = matches!(chat.phase(), SessionPhase::Running | SessionPhase::Waiting);
        if running && !chat.snapshot.as_ref().is_some_and(|s| s.capabilities.steer) {
            chat.pending_results.push(text);
            return;
        }
        let id = scope.chat;
        let generation = chat.generation;
        run(
            cx,
            move || {
                if running {
                    session.steer(&text)
                } else {
                    session.prompt(&text)
                }
            },
            move |this, result, cx| {
                if !this
                    .chats
                    .get(&id)
                    .is_some_and(|c| c.generation == generation)
                {
                    return;
                }
                if let Err(error) = result {
                    this.report(&error, cx);
                }
            },
        );
    }
    fn flush_results(&mut self, id: &ChatId, cx: &mut Context<Self>) {
        let Some(chat) = self.chats.get_mut(id) else {
            return;
        };
        if chat.pending_results.is_empty() {
            return;
        }
        let text = std::mem::take(&mut chat.pending_results).join("\n\n");
        let scope = ChatScope {
            project: chat.project.clone(),
            chat: id.clone(),
            purpose: autocraft_core::ChatPurpose::Analysis,
        };
        self.on_long_tool_result(scope, text, cx);
    }
}
