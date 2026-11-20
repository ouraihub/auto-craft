//! Project tab loading and engine lifetime; stale starts are disposed before their ids can be reused.
use super::{
    AgentPanel, Chat,
    engine::{StartInput, dispose_engine, start_engine},
};
use crate::state::run;
use autocraft_core::{ChatId, ProjectId};
use gpui_kit::Context;
use tracing::debug;

impl AgentPanel {
    /// Selects the project's retained tabs, without interrupting engines in another project.
    pub fn focus_project(&mut self, project: Option<ProjectId>, cx: &mut Context<Self>) {
        if self.project == project {
            return;
        }
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        self.project = project.clone();
        self.builder_selected = false;
        self.order = self
            .chats
            .iter()
            .filter(|(_, c)| Some(&c.project) == project.as_ref())
            .map(|(id, _)| id.clone())
            .collect();
        self.order
            .sort_by_key(|id| self.chats.get(id).map_or(0, |c| c.ordinal));
        self.active = self.order.first().cloned();
        self.guide = None;
        self.slash = Default::default();
        cx.notify();
        let Some(project) = project else {
            return;
        };
        let services = self.services(cx);
        let scope = project.clone();
        run(
            cx,
            move || services.analysis_chat_bindings(&scope),
            move |this, result, cx| {
                if this.generation != generation || this.project.as_ref() != Some(&project) {
                    debug!("agent.tabs.stale_load");
                    return;
                }
                match result {
                    Ok(bindings) => {
                        for binding in bindings {
                            let id = binding.chat.clone();
                            if this.chats.contains_key(&id) {
                                continue;
                            }
                            let ordinal = this.next_ordinal(&project);
                            this.chats.insert(
                                id.clone(),
                                Chat::new(project.clone(), ordinal, Some(binding)),
                            );
                            this.order.push(id);
                        }
                        if this.active.is_none() {
                            this.active = this.order.first().cloned();
                        }
                        if let Some(id) = this.active.clone() {
                            this.start(id, None, cx);
                        } else {
                            this.new_chat(cx);
                        }
                    }
                    Err(error) => this.report(&error, cx),
                }
                cx.notify();
            },
        );
    }
    fn next_ordinal(&self, project: &ProjectId) -> usize {
        self.chats
            .values()
            .filter(|c| &c.project == project)
            .map(|c| c.ordinal)
            .max()
            .unwrap_or(0)
            + 1
    }
    pub(super) fn new_chat(&mut self, cx: &mut Context<Self>) {
        let Some(project) = self.project.clone() else {
            return;
        };
        let id = ChatId::generate();
        self.chats.insert(
            id.clone(),
            Chat::new(project.clone(), self.next_ordinal(&project), None),
        );
        self.order.push(id.clone());
        self.active = Some(id.clone());
        self.builder_selected = false;
        self.start(id, None, cx);
    }
    pub(super) fn start(&mut self, id: ChatId, prompt: Option<String>, cx: &mut Context<Self>) {
        let services = self.services(cx);
        let draft = prompt.clone();
        let prompt = prompt.map(|text| self.question_prompt(text));
        let Some(chat) = self.chats.get_mut(&id) else {
            return;
        };
        if chat.starting || chat.closing || chat.live().is_some() {
            return;
        }
        chat.starting = true;
        chat.blocked = None;
        chat.generation = chat.generation.wrapping_add(1);
        let generation = chat.generation;
        let project = chat.project.clone();
        let input = StartInput {
            services: services.clone(),
            chat: id.clone(),
            project: project.clone(),
            binding: chat.binding.clone(),
            prompt,
        };
        run(
            cx,
            move || start_engine(input),
            move |this, result, cx| {
                if !this
                    .chats
                    .get(&id)
                    .is_some_and(|chat| chat.generation == generation)
                {
                    debug!("agent.session.stale_start");
                    if let Ok(Ok(started)) = result {
                        dispose_engine(
                            this.services(cx),
                            project,
                            id,
                            Some(started.session),
                            Some(started.lease),
                            true,
                            cx,
                        );
                    } else if let Err(error) = result {
                        this.report(&error, cx);
                    }
                    return;
                }
                let Some(chat) = this.chats.get_mut(&id) else {
                    return;
                };
                chat.starting = false;
                match result {
                    Ok(Ok(started)) => {
                        let snapshot = started.session.snapshot();
                        super::projection::apply(
                            &mut chat.snapshot,
                            autocraft_core::SessionEvent::Snapshot {
                                snapshot: Box::new(snapshot),
                            },
                        );
                        chat.session = Some(started.session);
                        chat.binding = Some(started.binding);
                        chat.lease = Some(started.lease);
                        chat.routing_signature = Some(started.signature);
                        let closed = chat
                            .snapshot
                            .as_ref()
                            .is_some_and(|s| s.phase == autocraft_core::SessionPhase::Closed);
                        if let Some(error) = started.prompt_error {
                            chat.draft = draft.clone().unwrap_or_default();
                            this.report(&error, cx);
                        }
                        this.refresh_catalog(&id, cx);
                        this.refresh_usage(&id, cx);
                        if closed {
                            this.retire(&id, cx);
                        }
                    }
                    Ok(Err(readiness)) => {
                        chat.blocked = Some(readiness);
                        chat.draft = draft.unwrap_or_default();
                        this.guide = Some(readiness);
                    }
                    Err(error) => {
                        chat.draft = draft.unwrap_or_default();
                        this.report(&error, cx);
                    }
                }
                cx.notify();
            },
        );
        cx.notify();
    }
    pub(super) fn close_chat(&mut self, id: &ChatId, cx: &mut Context<Self>) {
        let Some(chat) = self.chats.remove(id) else {
            return;
        };
        self.order.retain(|c| c != id);
        if self.active.as_ref() == Some(id) {
            self.active = self.order.last().cloned();
        }
        self.answers.clear();
        self.picks.clear();
        // A start still in flight owns its cleanup; its stale completion also removes the binding.
        if !chat.starting {
            dispose_engine(
                self.services(cx),
                chat.project,
                id.clone(),
                chat.session,
                chat.lease,
                true,
                cx,
            );
        }
        cx.notify();
    }
}
