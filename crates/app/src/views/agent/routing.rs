//! Routing signatures restart idle engines without changing the chat's pinned backend or locator.
use super::{AgentPanel, engine::dispose_engine};
use crate::state::run;
use autocraft_core::{ChatId, ModelChoice, RouteRequest, SessionPhase, ThinkingLevel};
use gpui_kit::Context;
use std::sync::Arc;
impl AgentPanel {
    pub(super) fn on_routing_changed(&mut self, cx: &mut Context<Self>) {
        let retry: Vec<_> = self
            .chats
            .iter()
            .filter(|(_, c)| c.blocked.is_some())
            .map(|(id, _)| id.clone())
            .collect();
        for id in retry {
            self.start(id, None, cx);
        }
        let checks: Vec<_> = self
            .chats
            .iter()
            .filter_map(|(id, chat)| {
                Some((
                    id.clone(),
                    chat.project.clone(),
                    chat.binding.as_ref()?.backend,
                    chat.routing_signature.clone(),
                    chat.live()?.clone(),
                    chat.generation,
                ))
            })
            .collect();
        for (id, project, backend, old, session, generation) in checks {
            self.refresh_catalog(&id, cx);
            let services = self.services(cx);
            let request = RouteRequest {
                chat: id.clone(),
                project: project.clone(),
                backend,
                new_session: false,
                model: None,
                effort: None,
            };
            run(
                cx,
                move || super::engine::launch_signature(&services, &request),
                move |this, result, cx| {
                    let Some(chat) = this.chats.get_mut(&id) else {
                        return;
                    };
                    if chat.generation != generation
                        || !chat
                            .session
                            .as_ref()
                            .is_some_and(|s| Arc::ptr_eq(s, &session))
                    {
                        tracing::debug!("agent.routing.stale_result");
                        return;
                    }
                    let changed = match result {
                        Ok(signature) => old.as_ref() != Some(&signature),
                        Err(error) => {
                            this.report(&error, cx);
                            true
                        }
                    };
                    if !changed {
                        return;
                    }
                    let Some(chat) = this.chats.get_mut(&id) else {
                        return;
                    };
                    chat.restart_pending = true;
                    if chat.phase() == SessionPhase::Idle {
                        this.retire(&id, cx);
                    }
                    cx.notify();
                },
            );
        }
    }
    pub(super) fn retire(&mut self, id: &ChatId, cx: &mut Context<Self>) {
        let services = self.services(cx);
        let Some(chat) = self.chats.get_mut(id) else {
            return;
        };
        chat.closing = true;
        chat.restart_pending = false;
        chat.generation = chat.generation.wrapping_add(1);
        let project = chat.project.clone();
        let session = chat.session.take();
        let lease = chat.lease.take();
        dispose_engine(services, project, id.clone(), session, lease, false, cx);
    }
    pub(super) fn refresh_catalog(&mut self, id: &ChatId, cx: &mut Context<Self>) {
        let Some(chat) = self.chats.get(id) else {
            return;
        };
        let Some(backend) = chat.binding.as_ref().map(|b| b.backend) else {
            return;
        };
        if !self.model_picker.begin_refresh(id) {
            return;
        }
        let generation = chat.generation;
        let services = self.services(cx);
        let id = id.clone();
        run(
            cx,
            move || {
                Ok((
                    services.router.catalog(backend)?,
                    services.prefs.prefs()?,
                    services.providers.providers()?,
                ))
            },
            move |this, result, cx| {
                if !this
                    .chats
                    .get(&id)
                    .is_some_and(|c| c.generation == generation)
                {
                    this.model_picker.cancel_refresh(&id);
                    tracing::debug!("agent.catalog.stale_result");
                    return;
                }
                match result {
                    Ok((catalog, prefs, providers)) => {
                        this.model_picker.finish_refresh(&id, prefs, &providers);
                        if let Some(chat) = this.chats.get_mut(&id) {
                            chat.usage.set_catalog(&catalog, &providers);
                            chat.catalog = catalog;
                        }
                    }
                    Err(error) => {
                        this.model_picker.refresh_failed(&id, &error);
                        this.report(&error, cx);
                    }
                }
                cx.notify();
            },
        );
    }
    pub(super) fn set_model(
        &mut self,
        id: ChatId,
        choice: ModelChoice,
        effort: Option<ThinkingLevel>,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(chat) = self.chats.get(&id) else {
            return false;
        };
        let Some(session) = chat.live().cloned() else {
            return false;
        };
        if chat.closing
            || chat.starting
            || chat.phase() != SessionPhase::Idle
            || !chat
                .snapshot
                .as_ref()
                .is_some_and(|s| s.capabilities.model_switch)
            || effort.is_some_and(|e| !choice.model.efforts.contains(&e))
        {
            return false;
        }
        if !self.model_picker.begin_switch(&id) {
            return false;
        }
        let generation = chat.generation;
        if let Some(chat) = self.chats.get_mut(&id) {
            chat.usage.invalidate();
        }
        run(
            cx,
            move || session.set_model(&choice.route, &choice.model.id, effort),
            move |this, result, cx| {
                this.model_picker.finish_switch(&id, result.as_ref().err());
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
                this.refresh_usage(&id, cx);
                cx.notify();
            },
        );
        true
    }
    pub(super) fn set_effort(
        &mut self,
        id: ChatId,
        effort: ThinkingLevel,
        cx: &mut Context<Self>,
    ) -> bool {
        let choice = self.chats.get(&id).and_then(|chat| {
            let model = chat.snapshot.as_ref()?.model.as_ref()?;
            chat.catalog
                .iter()
                .find(|c| {
                    c.route == model.provider.as_deref().unwrap_or_default()
                        && c.model.id == model.id
                })
                .cloned()
        });
        choice.is_some_and(|choice| self.set_model(id, choice, Some(effort), cx))
    }
}
