//! Reference: craft-anything ai/src/rebuild/tools.rs (300dba58).
//! Source-compatible tool schemas with weak, injected real service dispatch.
use autocraft_core::{CancelFlag, Error, ErrorKind, ProjectId, RebuildService};
use ca_engine::tool::{
    Ctx, Effects, Requires, ToolOutput, ToolSpec, Toolset, parse_params, schema_of,
};
use ca_evidence::{ErrorCode, ToolError};
use serde_json::{Value, json};
use std::{
    path::Path,
    sync::{Arc, Weak},
    time::Duration,
};
pub const QUESTION_TOOL: &str = "rebuild_question_ask";
mod params;
mod specs;
pub use params::{
    ActivityParams, CheckParams, NoteParams, ProjectParams, QuestionParams, StartParams,
    WithdrawParams,
};
pub use specs::specs;

/// Resolve a canonical workspace folder to an actual host project identity.
pub type ProjectResolver = Arc<dyn Fn(&Path) -> autocraft_core::Result<ProjectId> + Send + Sync>;
/// The composition root owns source-compatible project creation and saved builder preferences.
pub type Starter = Arc<dyn Fn(StartParams, bool) -> autocraft_core::Result<Value> + Send + Sync>;
/// Weak ownership prevents AnalysisHub -> engine -> rebuild -> AnalysisHub cycles.
#[derive(Clone)]
pub struct RebuildHandle {
    service: Weak<dyn RebuildService>,
    resolve: ProjectResolver,
    start: Starter,
}
impl std::fmt::Debug for RebuildHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RebuildHandle").finish_non_exhaustive()
    }
}
impl RebuildHandle {
    pub fn new(
        service: Weak<dyn RebuildService>,
        resolve: ProjectResolver,
        start: Starter,
    ) -> Self {
        Self {
            service,
            resolve,
            start,
        }
    }
    fn service(&self) -> autocraft_core::Result<Arc<dyn RebuildService>> {
        self.service
            .upgrade()
            .ok_or_else(|| Error::new(ErrorKind::SessionClosed, "复刻服务已关闭"))
    }
    fn project(&self, path: &Path) -> autocraft_core::Result<ProjectId> {
        (self.resolve)(path)
    }
}
fn handle(ctx: &Ctx<'_>) -> Option<RebuildHandle> {
    ctx.session.ext.get::<RebuildHandle>().map(|h| (*h).clone())
}
fn unavailable() -> ToolError {
    ToolError::new(
        ErrorCode::FeatureMissing,
        "Building isn't available in this engine.",
    )
}
fn tool_error(error: autocraft_core::Error) -> ToolError {
    let code = match error.kind() {
        Some(ErrorKind::Invalid) => ErrorCode::BadInput,
        Some(ErrorKind::NotFound) => ErrorCode::NotFound,
        Some(ErrorKind::Cancelled) => ErrorCode::Cancelled,
        Some(ErrorKind::Timeout) => ErrorCode::TimedOut,
        Some(
            ErrorKind::Forbidden
            | ErrorKind::ReadOnly
            | ErrorKind::ConsentRequired
            | ErrorKind::DeepRequired,
        ) => ErrorCode::NoPermission,
        Some(
            ErrorKind::Conflict
            | ErrorKind::Busy
            | ErrorKind::SessionBusy
            | ErrorKind::SessionClosed
            | ErrorKind::InteractionExpired
            | ErrorKind::Corrupt
            | ErrorKind::External
            | ErrorKind::BackendUnavailable
            | ErrorKind::Protocol
            | ErrorKind::ToolFailed
            | ErrorKind::EngineMissing,
        )
        | None => ErrorCode::ProgramFailed,
    };
    ToolError::new(code, error.detail())
}
#[allow(
    clippy::result_large_err,
    reason = "upstream ToolError boundary: ToolSpec requires this exact dispatch signature"
)]
fn rebuild_start(ctx: &mut Ctx<'_>, params: Value) -> Result<ToolOutput, ToolError> {
    let p: StartParams = parse_params(params)?;
    let handle = handle(ctx).ok_or_else(unavailable)?;
    let value = (handle.start)(p, !ctx.session.clean_room()).map_err(tool_error)?;
    serde_json::to_value(value)
        .map(ToolOutput::Value)
        .map_err(|_| ToolError::internal("Cannot encode rebuild response."))
}
#[allow(
    clippy::result_large_err,
    reason = "upstream ToolError boundary: ToolSpec requires this exact dispatch signature"
)]
fn rebuild_status(ctx: &mut Ctx<'_>, params: Value) -> Result<ToolOutput, ToolError> {
    let p: ProjectParams = parse_params(params)?;
    let h = handle(ctx).ok_or_else(unavailable)?;
    let project = h.project(&p.project).map_err(tool_error)?;
    let status = h
        .service()
        .map_err(tool_error)?
        .status(&project)
        .map_err(tool_error)?;
    let ws = ca_engine::workspace::Workspace::open(&p.project)?;
    Ok(ToolOutput::Value(
        json!({"project":ws.root,"running":status.running,"queued":status.queued,"status":ws.manifest.status,"status_label":ws.manifest.status.label(),"note":status.note,"last_check":ws.manifest.checks.last(),"waiting":status.pending_requests}),
    ))
}
#[allow(
    clippy::result_large_err,
    reason = "upstream ToolError boundary: ToolSpec requires this exact dispatch signature"
)]
fn rebuild_activity_show(ctx: &mut Ctx<'_>, params: Value) -> Result<ToolOutput, ToolError> {
    let p: ActivityParams = parse_params(params)?;
    let h = handle(ctx).ok_or_else(unavailable)?;
    let project = h.project(&p.project).map_err(tool_error)?;
    h.service()
        .map_err(tool_error)?
        .activity(
            &project,
            p.offset.unwrap_or(0),
            p.limit.unwrap_or(200).min(2000),
        )
        .map_err(tool_error)?;
    let (lines, total) =
        crate::activity::read_log(&p.project, p.offset.unwrap_or(0), p.limit.unwrap_or(200))
            .map_err(tool_error)?;
    Ok(ToolOutput::Value(
        json!({"project":p.project,"log":p.project.join(ca_engine::workspace::ACTIVITY_LOG),"lines":lines,"total_lines_read":total}),
    ))
}
#[allow(
    clippy::result_large_err,
    reason = "upstream ToolError boundary: ToolSpec requires this exact dispatch signature"
)]
fn rebuild_note(ctx: &mut Ctx<'_>, params: Value) -> Result<ToolOutput, ToolError> {
    let p: NoteParams = parse_params(params)?;
    let h = handle(ctx).ok_or_else(unavailable)?;
    let project = h.project(&p.project).map_err(tool_error)?;
    let service = h.service().map_err(tool_error)?;
    let id = service
        .note(&project, &p.text, vec![])
        .map_err(tool_error)?;
    let status = service.status(&project).map_err(tool_error)?;
    let number = id
        .as_str()
        .parse::<u64>()
        .map_err(|_| ToolError::internal("Invalid note identity."))?;
    Ok(ToolOutput::Value(
        json!({"id":number,"queued":status.waiting_notes.iter().any(|(n,_)|n==&id)}),
    ))
}
#[allow(
    clippy::result_large_err,
    reason = "upstream ToolError boundary: ToolSpec requires this exact dispatch signature"
)]
fn rebuild_note_withdraw(ctx: &mut Ctx<'_>, params: Value) -> Result<ToolOutput, ToolError> {
    let p: WithdrawParams = parse_params(params)?;
    let h = handle(ctx).ok_or_else(unavailable)?;
    let project = h.project(&p.project).map_err(tool_error)?;
    let service = h.service().map_err(tool_error)?;
    let id = service
        .status(&project)
        .map_err(tool_error)?
        .waiting_notes
        .into_iter()
        .find(|(id, _)| id.as_str() == p.id.to_string())
        .map(|(id, _)| id);
    if let Some(id) = id
        && let Some(text) = service.withdraw(&project, &id).map_err(tool_error)?
    {
        return Ok(ToolOutput::Value(json!({"withdrawn":text})));
    }
    Err(ToolError::new(
        ErrorCode::BadInput,
        format!(
            "Note {} isn't waiting: the agent has read it, or it isn't from this session.",
            p.id
        ),
    ))
}
#[allow(
    clippy::result_large_err,
    reason = "upstream ToolError boundary: ToolSpec requires this exact dispatch signature"
)]
fn rebuild_stop(ctx: &mut Ctx<'_>, params: Value) -> Result<ToolOutput, ToolError> {
    let p: ProjectParams = parse_params(params)?;
    let h = handle(ctx).ok_or_else(unavailable)?;
    let project = h.project(&p.project).map_err(tool_error)?;
    let service = h.service().map_err(tool_error)?;
    let status = service.status(&project).map_err(tool_error)?;
    service.stop(&project).map_err(tool_error)?;
    Ok(ToolOutput::Value(if status.queued.is_some() {
        json!({"unqueued":p.project})
    } else {
        json!({"stopping":p.project})
    }))
}
#[allow(
    clippy::result_large_err,
    reason = "upstream ToolError boundary: ToolSpec requires this exact dispatch signature"
)]
fn rebuild_questions_list(ctx: &mut Ctx<'_>, params: Value) -> Result<ToolOutput, ToolError> {
    let p: ProjectParams = parse_params(params)?;
    let h = handle(ctx).ok_or_else(unavailable)?;
    h.project(&p.project).map_err(tool_error)?;
    let ws = ca_engine::workspace::Workspace::open(&p.project)?;
    Ok(ToolOutput::Value(
        json!({"project":ws.root,"questions":ca_engine::questions::load(&ws)}),
    ))
}
#[allow(
    clippy::result_large_err,
    reason = "upstream ToolError boundary: ToolSpec requires this exact dispatch signature"
)]
fn rebuild_question_ask(_ctx: &mut Ctx<'_>, _params: Value) -> Result<ToolOutput, ToolError> {
    Err(ToolError::new(
        ErrorCode::FeatureMissing,
        "Questions about a build aren't available in this engine.",
    )
    .with_remediation("Use the app or `autocraft` outside the engine lock."))
}
#[allow(
    clippy::result_large_err,
    reason = "upstream ToolError boundary: ToolSpec requires this exact dispatch signature"
)]
fn rebuild_check(_ctx: &mut Ctx<'_>, _params: Value) -> Result<ToolOutput, ToolError> {
    Err(ToolError::new(
        ErrorCode::FeatureMissing,
        "This builder can't check its own work while it builds.",
    )
    .with_remediation("Finish the turn: the app checks the build after it."))
}
/// Long-running host dispatch. The caller MUST release the project engine lock first.
pub fn dispatch_outside(
    handle: &RebuildHandle,
    id: &str,
    params: Value,
    cancel: &CancelFlag,
) -> autocraft_core::Result<Value> {
    match id {
        "rebuild_question_ask" => {
            let p: QuestionParams =
                serde_json::from_value(params).map_err(|_| Error::invalid("问题参数无效"))?;
            let project = handle.project(&p.project)?;
            let answered = handle.service()?.ask(&project, &p.question, cancel)?;
            Ok(
                json!({"project":p.project,"question":p.question.trim(),"answered":answered,"not_kept":Value::Null}),
            )
        }
        "rebuild_check" => {
            let p: CheckParams =
                serde_json::from_value(params).map_err(|_| Error::invalid("自查参数无效"))?;
            if !(1..=60).contains(&p.wait_minutes) {
                return Err(Error::invalid("wait_minutes must be 1..=60"));
            }
            let path = p
                .project
                .ok_or_else(|| Error::invalid("No project given."))?;
            let project = handle.project(&path)?;
            let report = handle.service()?.self_check(
                &project,
                Duration::from_secs(p.wait_minutes * 60),
                cancel,
            )?;
            Ok(json!({"project":path,"report":report}))
        }
        _ => Err(Error::invalid("此工具不是锁外复刻工具")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_unknown_parameters() {
        assert!(
            parse_params::<NoteParams>(json!({"project":"x","text":"ok","extra":true})).is_err()
        );
    }
}
