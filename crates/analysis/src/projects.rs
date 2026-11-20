//! Project lifecycle backed exclusively by upstream workspaces.
use crate::{AnalysisHub, errors::convert, project_dto as dto};
use autocraft_core::*;
use ca_engine::{
    Session,
    workspace::{self, Workspace},
};

impl ProjectService for AnalysisHub {
    fn list(&self) -> Result<Vec<ProjectSummary>> {
        let locations = self.settings.lock().project_locations();
        let mut seen = std::collections::HashSet::new();
        Workspace::list_in(&locations)
            .iter()
            .map(dto::summary)
            .filter(|r| match r {
                Ok(s) => seen.insert(s.id.clone()),
                Err(_) => true,
            })
            .collect()
    }
    fn open(&self, id: &ProjectId) -> Result<ProjectDetail> {
        dto::detail(self, &dto::workspace(id)?)
    }
    fn create(&self, input: NewProject) -> Result<ProjectSummary> {
        if input.name.trim().is_empty() {
            return Err(Error::invalid("项目名称不能为空"));
        }
        if matches!(input.door, Door::WorkOnMine) && input.folder.is_none() {
            return Err(Error::invalid("请选择自己的项目目录"));
        }
        if matches!(input.door, Door::LookInside) && matches!(input.target, TargetRefDto::None) {
            return Err(Error::invalid("请选择要分析的目标"));
        }
        let settings = self.settings.lock().clone();
        let target = dto::source_target(input.target);
        let existing = match input.existing {
            Some(ExistingFilesDto::Edit) => Some(workspace::ExistingFiles::Edit),
            Some(ExistingFilesDto::StartFrom) => Some(workspace::ExistingFiles::StartFrom),
            None => None,
        };
        let mut ws = if let Some(folder) = &input.folder {
            if !folder.is_absolute() {
                return Err(Error::invalid("项目目录必须是绝对路径"));
            }
            Workspace::create_at(
                folder,
                &input.name,
                &input.request,
                target,
                if matches!(input.door, Door::WorkOnMine) {
                    Some(workspace::ExistingFiles::Edit)
                } else {
                    existing
                },
            )
        } else {
            Workspace::create(
                &settings.projects_dir(),
                &input.name,
                &input.request,
                target,
            )
        }
        .map_err(|e| convert("project_create", e))?;
        let own = matches!(input.door, Door::WorkOnMine);
        ws.set_person(|m| {
            m.asset_budget_usd = settings.assets.budget();
            if own {
                m.app_in_root = true;
            }
        })
        .map_err(|e| convert("project_create", e))?;
        let id = dto::id(&ws.root)?;
        let mut prefs = self.prefs.get(&id)?;
        prefs = self
            .prefs
            .set(&id, ProjectPrefChange::Mode(input.mode), prefs.revision)?;
        if let Some(builder) = input.builder {
            prefs = self.prefs.set(
                &id,
                ProjectPrefChange::Backend(builder.backend),
                prefs.revision,
            )?;
            prefs = self
                .prefs
                .set(&id, ProjectPrefChange::Model(builder.model), prefs.revision)?;
            self.prefs.set(
                &id,
                ProjectPrefChange::Effort(builder.effort),
                prefs.revision,
            )?;
        }
        if input.folder.is_some() {
            remember(self, &ws)?;
        }
        if matches!(input.door, Door::LookInside) {
            AnalysisService::open(self, &id)?;
            let call = |tool: &str, params| {
                AnalysisService::call(
                    self,
                    &id,
                    ToolCall {
                        tool: tool.to_owned(),
                        params,
                        caller: CallerDto::Ui,
                        consented: true,
                    },
                    &CancelFlag::default(),
                )
            };
            match &ws.manifest.target {
                workspace::TargetRef::Website { url } => {
                    call("web_site_open", serde_json::json!({"url": url}))?;
                }
                workspace::TargetRef::Local { path } => {
                    call("core_target_open", serde_json::json!({"path": path}))?;
                    call("core_target_triage", serde_json::json!({"path": path}))?;
                }
                workspace::TargetRef::None => {}
            }
        }
        self.changed(&id);
        dto::summary(&ws)
    }
    fn rename(&self, id: &ProjectId, name: &str) -> Result<ProjectSummary> {
        let mut ws = self.writable(id)?;
        ws.rename(name).map_err(|e| convert("project_rename", e))?;
        self.changed(id);
        dto::summary(&ws)
    }
    fn duplicate(&self, id: &ProjectId, name: &str) -> Result<ProjectSummary> {
        if name.trim().is_empty() {
            return Err(Error::invalid("项目名称不能为空"));
        }
        if self.is_running(id) {
            return Err(Error::busy("项目正在运行"));
        }
        let source = self.writable(id)?;
        let ws = source
            .duplicate(name)
            .map_err(|e| convert("project_duplicate", e))?;
        let copy_id = dto::id(&ws.root)?;
        self.prefs.copy(id, &copy_id)?;
        remember(self, &ws)?;
        self.changed(&copy_id);
        dto::summary(&ws)
    }
    fn delete(&self, id: &ProjectId) -> Result<()> {
        let _creating = self.load_gate.lock();
        let ws = self.writable(id)?;
        if self.is_running(id) {
            return Err(Error::busy("项目正在运行"));
        }
        let entry = self.loaded.lock().get(id).cloned();
        if let Some(entry) = entry
            && (!entry.holders.lock().is_empty() || !entry.calls.lock().is_empty())
        {
            return Err(Error::busy("项目仍在使用中"));
        }
        ws.delete().map_err(|e| convert("project_delete", e))?;
        self.prefs.remove(id)?;
        self.loaded.lock().remove(id);
        self.changed(id);
        Ok(())
    }
    fn set_person(&self, id: &ProjectId, change: PersonChange) -> Result<ProjectDetail> {
        let mut ws = self.writable(id)?;
        if let PersonChange::AssetBudget(v) = &change
            && (!v.is_finite() || *v < 0.0)
        {
            return Err(Error::invalid("素材预算无效"));
        }
        ws.set_person(|m| match change {
            PersonChange::Approval { kind, approval } => {
                let kind = dto::source_action(kind);
                if let Some(value) = approval {
                    m.approvals.insert(kind, dto::source_approval(value));
                } else {
                    m.approvals.remove(&kind);
                }
            }
            PersonChange::Questions(value) => m.questions = dto::source_questions(value),
            PersonChange::AssetBudget(value) => m.asset_budget_usd = value,
            PersonChange::GenerateAssets(value) => m.generate_assets = value,
        })
        .map_err(|e| convert("project_person", e))?;
        let questions = self.settings.lock().questions_in(Some(&ws.manifest));
        let loaded = self.loaded.lock().get(id).cloned();
        if let Some(loaded) = loaded {
            loaded.policy.lock().questions = questions;
        }
        self.changed(id);
        dto::detail(self, &ws)
    }
    fn locations(&self) -> Result<ProjectLocations> {
        let settings = self.settings.lock();
        Ok(ProjectLocations {
            default_dir: settings.projects_dir(),
            extra: settings.project_folders.clone(),
        })
    }
    fn set_locations(&self, locations: ProjectLocations) -> Result<()> {
        if !locations.default_dir.is_absolute() || locations.extra.iter().any(|p| !p.is_absolute())
        {
            return Err(Error::invalid("项目位置必须是绝对路径"));
        }
        let mut settings = self.settings.lock();
        let mut next = settings.clone();
        next.projects_dir = Some(locations.default_dir);
        next.project_folders = locations.extra;
        next.save_to(&self.settings_path())
            .map_err(|e| convert("project_locations", e))?;
        *settings = next;
        Ok(())
    }
    fn add_references(&self, project: &ProjectId, files: &[std::path::PathBuf]) -> Result<usize> {
        let ws = self.writable(project)?;
        let added = ws
            .add_references(files)
            .map_err(|e| convert("project_references", e))?;
        self.changed(project);
        Ok(added)
    }
}
fn remember(hub: &AnalysisHub, ws: &Workspace) -> Result<()> {
    let mut settings = hub.settings.lock();
    let mut session = Session::isolated(&hub.data_dir.join("analysis"));
    session.settings = settings.clone();
    session
        .remember_project(&ws.root)
        .map_err(|e| convert("project_locations", e))?;
    *settings = session.settings;
    Ok(())
}
