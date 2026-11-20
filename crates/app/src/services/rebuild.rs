//! Weak analysis-to-rebuild callbacks: no service graph owns its own ancestor.
use super::builder::AppBuilder;
use autocraft_analysis::{AnalysisHub, HostRebuild};
use autocraft_core::*;
use autocraft_rebuild::{
    RebuildHub,
    tools::{RebuildHandle, StartParams},
};
use autocraft_settings::SqliteSettingsStore;
use ca_engine::workspace::{TargetRef, Workspace};
use serde_json::{Value, json};
use std::{
    path::Path,
    sync::{Arc, OnceLock, Weak},
};

pub(super) struct Host {
    pub service: Arc<OnceLock<Weak<RebuildHub>>>,
    pub handle: Arc<OnceLock<RebuildHandle>>,
}
impl HostRebuild for Host {
    fn running(&self, project: &ProjectId) -> bool {
        self.service
            .get()
            .and_then(Weak::upgrade)
            .is_some_and(|hub| hub.status(project).is_ok_and(|status| status.running))
    }
    fn call(
        &self,
        project: &ProjectId,
        tool: &str,
        mut params: Value,
        cancel: &CancelFlag,
    ) -> Result<Value> {
        let handle = self.handle.get().ok_or_else(closed)?;
        if let Some(params) = params.as_object_mut() {
            params
                .entry("project")
                .or_insert_with(|| Value::String(project.as_str().to_owned()));
        }
        autocraft_rebuild::tools::dispatch_outside(handle, tool, params, cancel)
    }
    fn answer(&self, project: &ProjectId, id: &InboxRequestId, answer: InboxAnswer) -> Result<()> {
        self.service
            .get()
            .and_then(Weak::upgrade)
            .ok_or_else(closed)?
            .answer(project, id, answer)
    }
}
fn closed() -> Error {
    Error::new(ErrorKind::SessionClosed, "复刻服务已关闭")
}
fn source(error: impl std::fmt::Display) -> Error {
    Error::external(error.to_string())
}

pub(super) fn handle(
    hub: &Arc<RebuildHub>,
    analysis: &Arc<AnalysisHub>,
    settings: Arc<SqliteSettingsStore>,
    builder: Arc<AppBuilder>,
) -> RebuildHandle {
    let service: Arc<dyn RebuildService> = hub.clone();
    let weak = Arc::downgrade(&service);
    let start_service = Arc::downgrade(hub);
    let analysis = Arc::downgrade(analysis);
    let builder = Arc::downgrade(&builder);
    RebuildHandle::new(
        weak,
        Arc::new(|root: &Path| autocraft_analysis::project_id(root)),
        Arc::new(move |params, deep| {
            let service = start_service.upgrade().ok_or_else(closed)?;
            let analysis = analysis.upgrade().ok_or_else(closed)?;
            let builder = builder.upgrade().ok_or_else(closed)?;
            start(
                &service,
                &analysis,
                settings.as_ref(),
                builder.as_ref(),
                params,
                deep,
            )
        }),
    )
}
fn start(
    service: &RebuildHub,
    analysis: &AnalysisHub,
    settings: &SqliteSettingsStore,
    builder: &AppBuilder,
    params: StartParams,
    deep: bool,
) -> Result<Value> {
    let backend = params
        .backend
        .as_deref()
        .map(|name| match name {
            "pi" => Ok(BackendKind::Pi),
            "omp" => Ok(BackendKind::Omp),
            "dsh" => Ok(BackendKind::Dsh),
            _ => Err(Error::invalid("未知 Agent 名称")),
        })
        .transpose()?;
    // Validate explicit Agent/stack choices before creating folders or attaching pictures.
    if let Some(backend) = backend {
        builder.launch(backend)?;
    }
    let stack = params
        .build_with
        .as_deref()
        .map(|name| {
            autocraft_rebuild::stacks::resolve(name).ok_or_else(|| Error::invalid("未知构建技术栈"))
        })
        .transpose()?;
    if params.project.is_some() && (stack.is_some() || params.generate_assets) {
        return Err(Error::invalid(
            "build_with 和 generate_assets 仅适用于新项目",
        ));
    }
    if params.project.is_none() && params.resume {
        return Err(Error::invalid("resume 需要现有项目目录"));
    }
    if params.project_folder.is_some() && params.projects_dir.is_some() {
        return Err(Error::invalid(
            "不能同时指定 projects_dir 和 project_folder",
        ));
    }
    let existing = params.project.is_some();
    let mut ws = match &params.project {
        Some(root) => Workspace::open(root).map_err(source)?,
        None => {
            let target = params.target.as_deref().unwrap_or("-");
            let target = if target.trim() == "-" {
                TargetRef::None
            } else {
                let parsed = TargetRef::parse(target);
                if parsed == TargetRef::None {
                    return Err(Error::invalid("目标必须为网址、现有绝对路径或 -"));
                }
                parsed
            };
            let name = ca_engine::workspace::suggest_name(&params.request, &target);
            match &params.project_folder {
                Some(folder) if folder.join(ca_engine::workspace::MANIFEST).is_file() => {
                    Workspace::open(folder).map_err(source)?
                }
                Some(folder) => {
                    Workspace::create_at(folder, &name, &params.request, target, params.existing)
                        .map_err(source)?
                }
                None => {
                    let locations = analysis.locations()?;
                    let parent = params
                        .projects_dir
                        .as_ref()
                        .unwrap_or(&locations.default_dir);
                    Workspace::create(parent, &name, &params.request, target).map_err(source)?
                }
            }
        }
    };
    let project = autocraft_analysis::project_id(&ws.root)?;
    if !existing {
        if let Some(stack) = stack {
            ws.manifest.stack = stack.name.clone();
        }
        let budget = analysis.settings()?.assets.budget_usd;
        ws.set_person(|manifest| {
            manifest.generate_assets = params.generate_assets;
            manifest.asset_budget_usd = budget;
        })
        .map_err(source)?;
        let prefs = ProjectPrefService::get(settings, &project)?;
        ProjectPrefService::set(
            settings,
            &project,
            ProjectPrefChange::Mode(if deep {
                AnalysisMode::Deep
            } else {
                AnalysisMode::CleanRoom
            }),
            prefs.revision,
        )?;
        if let Some(backend) = backend {
            let prefs = ProjectPrefService::get(settings, &project)?;
            ProjectPrefService::set(
                settings,
                &project,
                ProjectPrefChange::Backend(Some(backend)),
                prefs.revision,
            )?;
        }
    }
    if params.project_folder.is_some() || params.projects_dir.is_some() || existing {
        let mut locations = analysis.locations()?;
        if ws.root.parent() != Some(locations.default_dir.as_path())
            && !locations.extra.contains(&ws.root)
        {
            locations.extra.push(ws.root.clone());
            analysis.set_locations(locations)?;
        }
    }
    let pictures = if params.pictures.is_empty() {
        Vec::new()
    } else {
        service.attach_pictures(&project, params.pictures)?
    };
    let message = (existing && !params.resume).then_some(params.request);
    let started = service.start(
        &project,
        RunRequest {
            shown: message.clone(),
            message,
            pictures,
            continuing: params.resume,
            max_rounds: Some(params.rounds.unwrap_or(2)),
            backend,
        },
    )?;
    Ok(match started {
        RunStarted::Running => json!({"project": project.as_str(), "started": true}),
        RunStarted::Queued { position } => {
            json!({"project": project.as_str(), "started": false, "queued": position})
        }
    })
}
