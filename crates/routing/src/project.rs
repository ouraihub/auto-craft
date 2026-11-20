//! Shared startup resolution for analysis chats, builders and read-only questions.

use autocraft_core::{
    ChatBindingService, ChatId, EngineNote, Error, ModelPrefService, ModelRouter, ModelStatus,
    ProjectId, ProjectPrefService, Result, RouteRequest,
};

/// Resolves binding → project → global backend precedence without choosing an implicit Agent.
/// A resumed native session keeps its binding and re-applies the current project/global model
/// through the engine's session switch after the native session is restored.
/// The caller presents readiness when no default Agent has been selected.
pub fn resolve_project_route(
    project: &ProjectId,
    chat: &ChatId,
    projects: &dyn ProjectPrefService,
    defaults: &dyn ModelPrefService,
    bindings: &dyn ChatBindingService,
) -> Result<RouteRequest> {
    let binding = bindings.binding(project, chat)?;
    let prefs = projects.get(project)?;
    let backend = match binding
        .as_ref()
        .map(|binding| binding.backend)
        .or(prefs.backend)
    {
        Some(backend) => backend,
        None => defaults.prefs()?.default_agent.ok_or_else(|| {
            Error::invalid("请先在编码搭档中选择默认 Agent，或在项目设置中选择 Agent。")
        })?,
    };
    let new_session = binding.is_none();
    Ok(RouteRequest {
        chat: chat.clone(),
        project: project.clone(),
        backend,
        new_session,
        model: prefs.model,
        effort: prefs.effort,
    })
}

/// Prepares the current project model for both new and resumed sessions. Resumed engines receive
/// it through `AgentSession::set_model` after their native session is restored.
///
/// Reasons for unavailable providers/models come from `model_status`; engine restrictions come
/// from actual preparation and the engine notes used by the project model selector.
///
/// # Errors
/// Returns `INVALID` for an unroutable explicit project model (never retries another model),
/// otherwise propagates routing failures.
pub fn prepare_project_route(
    router: &dyn ModelRouter,
    request: &RouteRequest,
) -> Result<autocraft_core::EngineRouting> {
    if let Some(model) = &request.model {
        let reason = match router.model_status(model)? {
            ModelStatus::Available => None,
            ModelStatus::ProviderMissing => Some("提供方已被删除".to_owned()),
            ModelStatus::ProviderDisabled { provider } => {
                Some(format!("提供方「{provider}」已停用"))
            }
            ModelStatus::ModelMissing => Some("模型已被删除".to_owned()),
            ModelStatus::ModelUnusable { model } => {
                Some(format!("模型「{model}」已停用或不是聊天模型"))
            }
        };
        if let Some(reason) = reason {
            return Err(unavailable(&model.model_id, &reason));
        }
        return match router.prepare(request) {
            Err(error) if error.kind() == Some(autocraft_core::ErrorKind::Invalid) => {
                let reason = router
                    .engine_notes()?
                    .into_iter()
                    .find(|(backend, _)| *backend == request.backend)
                    .map(|(_, note)| match note {
                        EngineNote::Official => "编码搭档选择了官方配置".to_owned(),
                        EngineNote::OnlyProvider { provider } => {
                            format!("编码搭档只允许提供方「{provider}」")
                        }
                        EngineNote::ProviderUnavailable => {
                            "编码搭档选择的提供方已删除或停用".to_owned()
                        }
                    })
                    .unwrap_or_else(|| error.detail().to_owned());
                Err(unavailable(&model.model_id, &reason))
            }
            result => result,
        };
    }
    router.prepare(request)
}

fn unavailable(model: &str, reason: &str) -> Error {
    Error::invalid(format!(
        "项目选择的模型「{model}」不可用：{reason}。请前往项目设置。"
    ))
}
