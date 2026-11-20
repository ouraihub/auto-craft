//! Event-driven telemetry cache and UI adapter; all engine I/O runs through background jobs.

use std::{collections::HashMap, sync::Arc};

use gpui_kit::Context;
use lumakit_usage_status::{ContextStatus, StatusState};

use super::{AgentPanel, Chat};
use crate::state::run;
use autocraft_core::{ChatId, SessionPhase, SessionUsage};

#[derive(Default)]
pub(super) struct Usage {
    snapshot: Option<SessionUsage>,
    generation: u64,
    in_flight: Option<u64>,
    pending: bool,
    pub(super) compacting: bool,
    catalog_windows: HashMap<(String, String), u64>,
}

impl Usage {
    pub(super) fn set_catalog(
        &mut self,
        catalog: &[autocraft_core::ModelChoice],
        providers: &[autocraft_core::Provider],
    ) {
        self.catalog_windows = catalog
            .iter()
            .filter_map(|choice| {
                let model = providers
                    .iter()
                    .find(|provider| provider.id == choice.provider)?
                    .input
                    .models
                    .iter()
                    .find(|model| model.model_id == choice.model.id)?;
                let window = model.context_window.filter(|window| *window > 0)?;
                Some((
                    (choice.route.clone(), choice.model.id.clone()),
                    u64::from(window),
                ))
            })
            .collect();
    }

    /// A new prompt/model makes the previous context occupancy obsolete, not the session total.
    pub(super) fn invalidate(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.pending = false;
        if let Some(usage) = self.snapshot.as_mut() {
            usage.used_tokens = None;
            usage.context_window = None;
            usage.percent = None;
        }
    }

    fn finish(&mut self, generation: u64, current: bool, usage: Option<SessionUsage>) -> bool {
        if self.in_flight != Some(generation) {
            return false;
        }
        self.in_flight = None;
        if current && self.generation == generation {
            self.snapshot = usage;
        }
        std::mem::take(&mut self.pending)
    }
}

impl AgentPanel {
    pub(super) fn refresh_usage(&mut self, id: &ChatId, cx: &mut Context<Self>) {
        let Some(chat) = self.chats.get_mut(id) else {
            return;
        };
        if chat.starting || chat.closing || chat.phase() != SessionPhase::Idle {
            return;
        }
        let Some(session) = chat.live().cloned() else {
            return;
        };
        if chat.usage.in_flight.is_some() {
            chat.usage.pending = true;
            return;
        }
        let generation = chat.usage.generation;
        chat.usage.in_flight = Some(generation);
        let model = chat.snapshot.as_ref().and_then(|s| s.model.clone());
        let worker = session.clone();
        let id = id.clone();
        run(
            cx,
            move || worker.usage_snapshot(),
            move |this, result, cx| {
                let Some(chat) = this.chats.get_mut(&id) else {
                    return;
                };
                let current = chat.live().is_some_and(|live| Arc::ptr_eq(live, &session))
                    && chat.phase() == SessionPhase::Idle
                    && !chat.closing
                    && chat.snapshot.as_ref().and_then(|s| s.model.as_ref()) == model.as_ref();
                // Query failure stays unknown; state::run records the failed boundary.
                let (usage, error) = match result {
                    Ok(usage) => (usage, None),
                    Err(error) => (None, Some(error)),
                };
                let pending = chat.usage.finish(generation, current, usage);
                if let Some(error) = error {
                    this.report(&error, cx);
                }
                if pending {
                    this.refresh_usage(&id, cx);
                }
                cx.notify();
            },
        );
    }

    pub(super) fn sync_usage_view(&mut self, id: &ChatId, cx: &mut Context<Self>) {
        let Some(chat) = self.chats.get(id) else {
            return;
        };
        let status = status(chat);
        self.usage_view
            .update(cx, |view, cx| view.set_status(status, cx));
    }
}

fn status(chat: &Chat) -> ContextStatus {
    let usage = chat.usage.snapshot.as_ref();
    let model = chat.snapshot.as_ref().and_then(|s| s.model.as_ref());
    let catalog_window = model.and_then(|model| {
        chat.usage
            .catalog_windows
            .get(&(model.provider.clone().unwrap_or_default(), model.id.clone()))
            .copied()
    });
    let context_window = usage.and_then(|u| u.context_window).or(catalog_window);
    let source = if usage.and_then(|u| u.context_window).is_some() {
        rust_i18n::t!("app.agent.usage_engine")
    } else if catalog_window.is_some() {
        rust_i18n::t!("app.agent.usage_catalog")
    } else {
        rust_i18n::t!("app.agent.unknown")
    };
    let mut detail = rust_i18n::t!(
        "app.agent.usage_detail",
        used = number(usage.and_then(|u| u.used_tokens)),
        window = number(context_window),
        source = source,
        total = number(usage.and_then(|u| u.total_tokens))
    )
    .to_string();
    if let Some(u) = usage {
        detail.push_str(&rust_i18n::t!(
            "app.agent.usage_counters",
            input = number(u.input_tokens),
            output = number(u.output_tokens),
            cache_read = number(u.cache_read_tokens),
            cache_write = number(u.cache_write_tokens),
            reasoning = number(u.reasoning_tokens),
            source = if u.estimate {
                rust_i18n::t!("app.agent.usage_estimate")
            } else {
                rust_i18n::t!("app.agent.usage_reported")
            }
        ));
    } else {
        detail.push_str(&rust_i18n::t!("app.agent.usage_unavailable"));
    }
    ContextStatus {
        used_tokens: usage.and_then(|u| u.used_tokens),
        context_window,
        percent: usage.and_then(|u| u.percent),
        estimate: usage.is_some_and(|u| u.estimate),
        cumulative_tokens: usage.and_then(|u| u.total_tokens),
        cumulative_cost: usage
            .and_then(|u| u.cost_usd)
            .map(|amount| lumakit_usage_status::Money {
                currency: "USD".into(),
                amount,
                estimated: true,
                partial: false,
            }),
        auto_compact: usage.is_some_and(|u| u.auto_compact),
        state: if chat.usage.compacting {
            StatusState::Compacting
        } else {
            match chat.phase() {
                SessionPhase::Running | SessionPhase::Waiting => StatusState::Streaming,
                SessionPhase::Idle if usage.is_some() => StatusState::Ready,
                SessionPhase::Idle | SessionPhase::Closed => StatusState::Unknown,
            }
        },
        detail,
    }
}

fn number(value: Option<u64>) -> String {
    value.map_or_else(
        || rust_i18n::t!("app.agent.unknown").to_string(),
        |value| value.to_string(),
    )
}
