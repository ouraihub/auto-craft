//! Owns asynchronous builder-policy waits and their engine replies, never the model loop.

use crate::session::{Live, new_id};
use autocraft_core::{
    AgentAction, BuilderHooks, CancelFlag, Error, PolicyVerdict, Result, SessionRole, StartSession,
};
use parking_lot::Mutex;
use serde_json::{Value, json};
use std::{collections::HashMap, sync::Arc};

#[derive(Default)]
struct State {
    cancel: CancelFlag,
    pending: HashMap<String, Value>,
}

#[derive(Default)]
pub(crate) struct Approvals {
    pub(crate) builder: Option<BuilderHooks>,
    pub(crate) readonly: bool,
    state: Mutex<State>,
}

impl Approvals {
    pub(crate) fn new(request: &StartSession) -> Self {
        Self {
            builder: request.builder.clone(),
            readonly: request.role == SessionRole::ReadOnly,
            state: Mutex::default(),
        }
    }

    pub(crate) fn begin_turn(&self) {
        self.state.lock().cancel = CancelFlag::new();
    }

    pub(crate) fn cancel(&self, live: &Live) {
        let mut state = self.state.lock();
        state.cancel.cancel();
        let failed = state
            .pending
            .drain()
            .filter(|(_, reply)| live.process.send(reply).is_err())
            .count();
        if failed != 0 {
            tracing::warn!(
                failed_count = failed,
                "agent.approval.cancel_cleanup_failed"
            );
        }
    }

    /// Dispatch policy outside the reader and projection locks. A cancellation wins over Allow.
    pub(crate) fn decide(
        live: &Arc<Live>,
        action: Option<AgentAction>,
        cancelled_reply: Value,
        reply: impl FnOnce(PolicyVerdict) -> Value + Send + 'static,
    ) -> Result<()> {
        let gate = &live.approvals;
        let hooks = gate.builder.clone();
        let readonly = gate.readonly;
        let key = new_id();
        let cancel = {
            let mut state = gate.state.lock();
            state.pending.insert(key.clone(), cancelled_reply);
            state.cancel.clone()
        };
        let worker = live.clone();
        let failed_key = key.clone();
        std::thread::Builder::new()
            .name("agent-approval".into())
            .spawn(move || {
                let _entered = worker.process.trace.enter("agent.approval.decide");
                let verdict = verdict(action.as_ref(), hooks.as_ref(), readonly, &cancel);
                let mut state = worker.approvals.state.lock();
                if state.pending.remove(&key).is_some()
                    && let Err(error) = worker.process.send(&reply(verdict))
                {
                    worker
                        .state
                        .lock()
                        .error(crate::session::session_error(&error));
                }
            })
            .map(drop)
            .map_err(|error| {
                let mut state = live.approvals.state.lock();
                if let Some(reply) = state.pending.remove(&failed_key)
                    && live.process.send(&reply).is_err()
                {
                    tracing::warn!("agent.approval.failure_reply_failed");
                }
                Error::protocol(format!("无法启动审批线程（{:?}）", error.kind()))
            })
    }

    /// Marker traffic is private only when this exact launch has builder hooks.
    pub(crate) fn marker(
        live: &Arc<Live>,
        frame: &Value,
        translate: fn(&Value) -> Option<AgentAction>,
    ) -> Result<bool> {
        if live.approvals.builder.is_none()
            || frame["method"] != "input"
            || frame["title"] != "autocraft:approve"
        {
            return Ok(false);
        }
        let id = frame["id"]
            .as_str()
            .ok_or_else(|| Error::protocol("审批请求缺少 id"))?
            .to_owned();
        let event = frame["placeholder"]
            .as_str()
            .and_then(|raw| serde_json::from_str::<Value>(raw).ok());
        let Some(event) = event.filter(|event| {
            event["toolName"]
                .as_str()
                .is_some_and(|tool| !tool.is_empty())
                && event["input"].is_object()
        }) else {
            live.process.send(&json!({"type":"extension_ui_response","id":id,"value":json!({"allow":false,"reason":"Invalid approval action."}).to_string()}))?;
            return Ok(true);
        };
        let action = translate(&event);
        let cancelled = json!({"type":"extension_ui_response","id":id,"cancelled":true});
        Self::decide(live, action, cancelled, move |verdict| {
            let value = match verdict {
                PolicyVerdict::Allow => json!({"allow":true}),
                PolicyVerdict::Refuse { reason } => json!({"allow":false,"reason":reason}),
            };
            json!({"type":"extension_ui_response","id":id,"value":value.to_string()})
        })?;
        Ok(true)
    }
}

fn verdict(
    action: Option<&AgentAction>,
    hooks: Option<&BuilderHooks>,
    readonly: bool,
    cancel: &CancelFlag,
) -> PolicyVerdict {
    if cancel.is_cancelled() {
        return PolicyVerdict::Refuse {
            reason: "The session was stopped.".into(),
        };
    }
    let decision = if readonly && action.is_some() {
        PolicyVerdict::Refuse {
            reason: "This is a read-only question session.".into(),
        }
    } else {
        match (action, hooks) {
            (None, _) => PolicyVerdict::Allow,
            (Some(action), Some(hooks)) => hooks.policy.decide(action, cancel),
            (Some(_), None) => PolicyVerdict::Refuse {
                reason: "No builder approval policy is installed.".into(),
            },
        }
    };
    if cancel.is_cancelled() {
        PolicyVerdict::Refuse {
            reason: "The session was stopped.".into(),
        }
    } else {
        decision
    }
}

#[cfg(test)]
#[path = "approvals_tests.rs"]
mod tests;
