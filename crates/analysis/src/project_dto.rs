//! Workspace-to-core projections and canonical project identity.
use crate::{AnalysisHub, errors::convert};
use autocraft_core::*;
use ca_engine::workspace::{self, Workspace};
use std::path::Path;

pub(crate) fn id(root: &Path) -> Result<ProjectId> {
    let canonical = std::fs::canonicalize(root).map_err(|_| Error::not_found("项目目录不存在"))?;
    let text = canonical
        .to_str()
        .ok_or_else(|| Error::invalid("项目路径编码无效"))?;
    Ok(ProjectId::from(normalize(text)))
}
fn normalize(text: &str) -> String {
    if cfg!(windows) {
        let text = text
            .strip_prefix(r"\\?\UNC\")
            .map(|s| format!(r"\\{s}"))
            .unwrap_or_else(|| text.strip_prefix(r"\\?\").unwrap_or(text).to_owned());
        let mut normalized = text.replace('/', "\\");
        if normalized.as_bytes().get(1) == Some(&b':') {
            normalized.replace_range(..1, &normalized[..1].to_ascii_uppercase());
        }
        normalized
    } else {
        text.to_owned()
    }
}
pub(crate) fn workspace(id: &ProjectId) -> Result<Workspace> {
    let path = Path::new(id.as_str());
    if !path.is_absolute() {
        return Err(Error::invalid("项目标识必须为绝对路径"));
    }
    let canonical = self::id(path)?;
    if canonical != *id {
        return Err(Error::invalid("项目标识未规范化"));
    }
    Workspace::open(path).map_err(|e| convert("project", e))
}
pub(crate) fn summary(ws: &Workspace) -> Result<ProjectSummary> {
    let m = &ws.manifest;
    Ok(ProjectSummary {
        id: id(&ws.root)?,
        name: m.name.clone(),
        request: m.request.clone(),
        target: target(&m.target),
        stack: m.stack.clone(),
        status: match m.status {
            workspace::Status::New => ProjectStatus::New,
            workspace::Status::Inspecting => ProjectStatus::Inspecting,
            workspace::Status::Building => ProjectStatus::Building,
            workspace::Status::Checking => ProjectStatus::Checking,
            workspace::Status::NeedsYou => ProjectStatus::NeedsYou,
            workspace::Status::Ready => ProjectStatus::Ready,
            workspace::Status::Stopped => ProjectStatus::Stopped,
        },
        note: m.note.clone(),
        updated_ms: m.updated_ms,
        own_project: m.own_project(),
        pending_requests: ca_engine::inbox::Inbox::new(ws.inbox_dir())
            .pending()
            .len()
            .try_into()
            .unwrap_or(u32::MAX),
        read_only: ws.is_read_only(),
    })
}
pub(crate) fn detail(hub: &AnalysisHub, ws: &Workspace) -> Result<ProjectDetail> {
    let summary = summary(ws)?;
    let settings = hub.settings.lock().clone();
    let approvals = workspace::ACTION_KINDS
        .iter()
        .map(|row| {
            (
                action(row.kind),
                approval(ws.manifest.approval(row.kind, settings.auto_mode)),
            )
        })
        .collect();
    let latest_round = ws.manifest.checks.iter().map(|c| c.round).max();
    Ok(ProjectDetail {
        mode: hub.prefs.get(&summary.id)?.mode,
        summary,
        approvals,
        approvals_chosen: ws.manifest.approvals.keys().copied().map(action).collect(),
        questions: questions(ws.manifest.questions),
        asset_budget_usd: ws.manifest.asset_budget_usd,
        assets_spent_usd: ws.assets_spent(),
        generate_assets: ws.manifest.generate_assets,
        latest_checks: ws
            .manifest
            .checks
            .iter()
            .filter(|c| Some(c.round) == latest_round)
            .map(crate::settings::transcode)
            .collect::<Result<_>>()?,
        usage: crate::settings::transcode(&ws.usage())?,
        app_dir: ws.app_dir(),
    })
}
pub(crate) fn target(value: &workspace::TargetRef) -> TargetRefDto {
    match value {
        workspace::TargetRef::Website { url } => TargetRefDto::Website { url: url.clone() },
        workspace::TargetRef::Local { path } => TargetRefDto::Local { path: path.clone() },
        workspace::TargetRef::None => TargetRefDto::None,
    }
}
pub(crate) fn source_target(value: TargetRefDto) -> workspace::TargetRef {
    match value {
        TargetRefDto::Website { url } => workspace::TargetRef::Website { url },
        TargetRefDto::Local { path } => workspace::TargetRef::Local { path },
        TargetRefDto::None => workspace::TargetRef::None,
    }
}
pub(crate) fn questions(value: Option<workspace::Questions>) -> QuestionsDto {
    match value {
        Some(workspace::Questions::Ask) => QuestionsDto::Ask,
        Some(workspace::Questions::Decide) => QuestionsDto::Decide,
        None => QuestionsDto::Default,
    }
}
pub(crate) fn source_questions(value: QuestionsDto) -> Option<workspace::Questions> {
    match value {
        QuestionsDto::Ask => Some(workspace::Questions::Ask),
        QuestionsDto::Decide => Some(workspace::Questions::Decide),
        QuestionsDto::Default => None,
    }
}
pub(crate) fn action(value: workspace::ActionKind) -> ActionKindDto {
    match value {
        workspace::ActionKind::Web => ActionKindDto::Web,
        workspace::ActionKind::Commands => ActionKindDto::Commands,
        workspace::ActionKind::OutsideFiles => ActionKindDto::OutsideFiles,
        workspace::ActionKind::Publish => ActionKindDto::Publish,
        workspace::ActionKind::DeleteOutside => ActionKindDto::DeleteOutside,
        workspace::ActionKind::System => ActionKindDto::System,
        workspace::ActionKind::OtherTools => ActionKindDto::OtherTools,
    }
}
pub(crate) fn source_action(value: ActionKindDto) -> workspace::ActionKind {
    match value {
        ActionKindDto::Web => workspace::ActionKind::Web,
        ActionKindDto::Commands => workspace::ActionKind::Commands,
        ActionKindDto::OutsideFiles => workspace::ActionKind::OutsideFiles,
        ActionKindDto::Publish => workspace::ActionKind::Publish,
        ActionKindDto::DeleteOutside => workspace::ActionKind::DeleteOutside,
        ActionKindDto::System => workspace::ActionKind::System,
        ActionKindDto::OtherTools => workspace::ActionKind::OtherTools,
    }
}
pub(crate) fn approval(value: workspace::Approval) -> ApprovalDto {
    match value {
        workspace::Approval::Allow => ApprovalDto::Allow,
        workspace::Approval::Ask => ApprovalDto::Ask,
        workspace::Approval::Refuse => ApprovalDto::Refuse,
    }
}
pub(crate) fn source_approval(value: ApprovalDto) -> workspace::Approval {
    match value {
        ApprovalDto::Allow => workspace::Approval::Allow,
        ApprovalDto::Ask => workspace::Approval::Ask,
        ApprovalDto::Refuse => workspace::Approval::Refuse,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn identity_resolves_directory_aliases() {
        let dir = tempfile::tempdir().unwrap();
        let child = dir.path().join("child");
        std::fs::create_dir(&child).unwrap();
        assert_eq!(id(dir.path()).unwrap(), id(&child.join("..")).unwrap());
    }
}
