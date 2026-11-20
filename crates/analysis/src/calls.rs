//! Tool execution, cancellation bridging, consent cards and active-call lifecycle.
use crate::{
    AnalysisEvent, AnalysisHub,
    engines::{ActiveCall, Loaded},
    errors, host_exec,
};
use autocraft_core::ErrorCode as _;
use autocraft_core::*;
use ca_engine::{
    CallOptions, Caller, CancelToken, Consent, Progress, ToolOutput,
    inbox::{Inbox, RequestKind},
    workspace::Workspace,
};
use ca_evidence::{ErrorCode, ToolError};
use parking_lot::Mutex;
use serde_json::Value;
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

pub(crate) struct Registration {
    loaded: Arc<Loaded>,
    id: u64,
}
impl Drop for Registration {
    fn drop(&mut self) {
        self.loaded.calls.lock().remove(&self.id);
    }
}
impl AnalysisHub {
    pub(crate) fn register(
        &self,
        loaded: &Arc<Loaded>,
        tool: String,
        cancel: CancelFlag,
        token: CancelToken,
        chat: Option<ChatId>,
    ) -> Registration {
        let id = self.next_call.fetch_add(1, Ordering::Relaxed);
        loaded
            .calls
            .lock()
            .insert(id, ActiveCall::new(tool, cancel, token, chat));
        Registration {
            loaded: loaded.clone(),
            id,
        }
    }
}
pub(crate) fn call(
    hub: &AnalysisHub,
    project: &ProjectId,
    call: ToolCall,
    cancel: &CancelFlag,
) -> Result<ToolReply> {
    let (_trace, operation) =
        lumakit_observability::TraceContext::root("analysis.tool", "analysis");
    let result = {
        let span = operation.span().clone();
        let _entered = span.enter();
        call_inner(hub, project, call, cancel)
    };
    match &result {
        Ok(_) => operation.finish("SUCCESS"),
        Err(error) if error.kind() == Some(ErrorKind::Cancelled) => operation.cancel("USER"),
        Err(error) if error.kind() == Some(ErrorKind::Timeout) => operation.timeout(0),
        Err(error) => operation.fail(error.code()),
    }
    result
}
fn call_inner(
    hub: &AnalysisHub,
    project: &ProjectId,
    call: ToolCall,
    cancel: &CancelFlag,
) -> Result<ToolReply> {
    hub.writable(project)?;
    let loaded = hub.loaded(project)?;
    let last = Mutex::new(None::<Instant>);
    let project_event = project.clone();
    let tool_event = call.tool.clone();
    let sink = hub.sink.clone();
    let progress = move |p: Progress| {
        let mut last = last.lock();
        if last.is_some_and(|at| at.elapsed() < Duration::from_millis(100)) {
            return;
        }
        *last = Some(Instant::now());
        sink.publish(AnalysisEvent::Progress {
            project: project_event.clone(),
            tool: tool_event.clone(),
            message: p.message,
            fraction: p.fraction,
        });
    };
    let token = CancelToken::default();
    let _registered = hub.register(
        &loaded,
        call.tool.clone(),
        cancel.clone(),
        token.clone(),
        None,
    );
    let plan = (loaded.engine.registry().canonical(&call.tool) == Some("core_plan_update"))
        .then(|| call.params.clone());
    let result = execute(
        hub,
        project,
        &loaded,
        &call.tool,
        call.params,
        Caller::Ui,
        call.consented,
        cancel,
        &token,
        &progress,
    );
    loaded.apply_settings();
    if result.is_ok()
        && let Some(steps) = plan
    {
        hub.sink.publish(AnalysisEvent::PlanUpdated {
            project: project.clone(),
            steps,
        });
    }
    hub.sink.publish(AnalysisEvent::Progress {
        project: project.clone(),
        tool: call.tool.clone(),
        message: "done".into(),
        fraction: Some(1.0),
    });
    hub.changed(project);
    match result.map_err(|e| errors::convert(&call.tool, e))? {
        ToolOutput::Evidence(ev) => Ok(ToolReply::Evidence {
            id: Some(EvidenceId::from(ev.id.clone())),
            body: serde_json::to_value(ev).map_err(|_| Error::corrupt("证据无法序列化"))?,
        }),
        ToolOutput::Value(value) => Ok(ToolReply::Value(value)),
    }
}
#[allow(
    clippy::result_large_err,
    reason = "upstream ToolError boundary preserves structured engine and MCP errors without boxing"
)]
#[allow(
    clippy::too_many_arguments,
    reason = "Matches the upstream call boundary plus project-scoped host context"
)]
pub(crate) fn execute(
    hub: &AnalysisHub,
    project: &ProjectId,
    loaded: &Arc<Loaded>,
    name: &str,
    args: Value,
    caller: Caller,
    consented: bool,
    cancel: &CancelFlag,
    token: &CancelToken,
    progress: &ca_engine::tool::ProgressFn,
) -> Result<ToolOutput, ToolError> {
    token.check()?;
    if cancel.is_cancelled() {
        return Err(ToolError::cancelled());
    }
    loaded.apply_settings();
    let canonical = loaded.engine.registry().canonical(name).unwrap_or(name);
    let ws = Workspace::open(std::path::Path::new(project.as_str()))?;
    // Even ostensibly read-only evidence calls journal and persist records upstream.
    if ws.is_read_only() {
        return Err(ToolError::new(
            ErrorCode::NoPermission,
            "This project was written by a newer version and is read-only.",
        ));
    }
    if host_exec::contains(canonical) {
        return host_exec::execute(hub, project, loaded, canonical, args, cancel, progress);
    }
    let done = AtomicBool::new(false);
    std::thread::scope(|scope| {
        let bridge = scope.spawn(|| {
            while !done.load(Ordering::Acquire) {
                if cancel.is_cancelled() {
                    token.cancel();
                    return;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        });
        let opts = |consent| {
            CallOptions::new(caller)
                .with_consent(consent)
                .with_cancel(token.clone())
                .with_progress(progress)
        };
        let consent = if consented {
            Consent::Granted
        } else {
            Consent::NotAsked
        };
        let result = match loaded.engine.call(name, args.clone(), opts(consent)) {
            Err(e) if e.code == ErrorCode::ConsentRequired && !consented => {
                match ask_consent(loaded, &ws, canonical, &e.message, token) {
                    Ok(true) => loaded.engine.call(name, args, opts(Consent::Granted)),
                    Ok(false) if token.is_cancelled() => Err(ToolError::cancelled()),
                    Ok(false) => Err(ToolError::new(
                        ErrorCode::ConsentRequired,
                        "the person declined this action",
                    )
                    .with_remediation(
                        "Continue without it, or explain why it's needed and ask again.",
                    )),
                    Err(error) => Err(error),
                }
            }
            result => result,
        };
        done.store(true, Ordering::Release);
        if let Err(payload) = bridge.join() {
            std::panic::resume_unwind(payload);
        }
        result
    })
}
#[allow(
    clippy::result_large_err,
    reason = "upstream ToolError boundary forwards inbox and workspace errors unchanged to the engine call"
)]
fn ask_consent(
    loaded: &Loaded,
    ws: &Workspace,
    name: &str,
    why: &str,
    token: &CancelToken,
) -> Result<bool, ToolError> {
    let mut ws = ws.clone();
    let remember = format!("tool:{name}");
    if ws.manifest.allowed_actions.contains(&remember) {
        return Ok(true);
    }
    let title = loaded.engine.registry().get(name).map_or(name, |s| s.title);
    let inbox = Inbox::new(ws.inbox_dir());
    let id = inbox.post(
        RequestKind::Approval,
        &format!("Allow: {title}"),
        why,
        Vec::new(),
        false,
        Some(remember.clone()),
    )?;
    let answer = inbox.wait(&id, Duration::from_secs(30 * 60), &|| token.is_cancelled());
    let allowed = answer.as_ref().and_then(|a| a.allow).unwrap_or(false);
    if allowed && answer.is_some_and(|a| a.always) {
        ws.set_person(|m| {
            if !m.allowed_actions.contains(&remember) {
                m.allowed_actions.push(remember);
            }
        })?;
    }
    Ok(allowed)
}
