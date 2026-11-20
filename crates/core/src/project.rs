//! Analysis-project metadata and machine-local project preferences.
use crate::{
    AnalysisMode, BackendKind, CheckResultDto, ModelRef, ProjectId, Result, ThinkingLevel,
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub trait ProjectService: Send + Sync {
    fn list(&self) -> Result<Vec<ProjectSummary>>;
    fn open(&self, id: &ProjectId) -> Result<ProjectDetail>;
    fn create(&self, input: NewProject) -> Result<ProjectSummary>;
    fn rename(&self, id: &ProjectId, name: &str) -> Result<ProjectSummary>;
    fn duplicate(&self, id: &ProjectId, name: &str) -> Result<ProjectSummary>;
    /// Deletes generated content only; BUSY while rebuilding.
    fn delete(&self, id: &ProjectId) -> Result<()>;
    fn set_person(&self, id: &ProjectId, change: PersonChange) -> Result<ProjectDetail>;
    fn locations(&self) -> Result<ProjectLocations>;
    fn set_locations(&self, locations: ProjectLocations) -> Result<()>;
    /// Adds selected files to project reference material and returns the imported count.
    /// # Errors
    /// Returns validation, read-only project or filesystem failures.
    fn add_references(&self, project: &ProjectId, files: &[PathBuf]) -> Result<usize>;
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectSummary {
    pub id: ProjectId,
    pub name: String,
    pub request: String,
    pub target: TargetRefDto,
    pub stack: String,
    pub status: ProjectStatus,
    pub note: String,
    pub updated_ms: u64,
    pub own_project: bool,
    pub pending_requests: u32,
    pub read_only: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectStatus {
    New,
    Inspecting,
    Building,
    Checking,
    NeedsYou,
    Ready,
    Stopped,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TargetRefDto {
    Website { url: String },
    Local { path: PathBuf },
    None,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectDetail {
    pub summary: ProjectSummary,
    pub mode: AnalysisMode,
    pub approvals: Vec<(ActionKindDto, ApprovalDto)>,
    pub approvals_chosen: Vec<ActionKindDto>,
    pub questions: QuestionsDto,
    pub asset_budget_usd: f64,
    pub assets_spent_usd: f64,
    pub generate_assets: bool,
    pub latest_checks: Vec<CheckResultDto>,
    pub usage: ProjectUsageDto,
    pub app_dir: PathBuf,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActionKindDto {
    Web,
    Commands,
    OutsideFiles,
    Publish,
    DeleteOutside,
    System,
    OtherTools,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ApprovalDto {
    Allow,
    Ask,
    Refuse,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum QuestionsDto {
    Ask,
    Decide,
    Default,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewProject {
    pub name: String,
    pub request: String,
    pub target: TargetRefDto,
    pub door: Door,
    pub folder: Option<PathBuf>,
    pub existing: Option<ExistingFilesDto>,
    pub mode: AnalysisMode,
    pub builder: Option<ProjectModelChoice>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Door {
    MakeMyOwn,
    WorkOnMine,
    LookInside,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExistingFilesDto {
    Edit,
    StartFrom,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PersonChange {
    Approval {
        kind: ActionKindDto,
        approval: Option<ApprovalDto>,
    },
    Questions(QuestionsDto),
    AssetBudget(f64),
    GenerateAssets(bool),
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectLocations {
    pub default_dir: PathBuf,
    pub extra: Vec<PathBuf>,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ProjectUsageDto {
    pub version: u32,
    pub work_ms: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd: f64,
    pub api_billed_usd: f64,
    pub priced: bool,
}

pub trait ProjectPrefService: Send + Sync {
    fn get(&self, project: &ProjectId) -> Result<ProjectPrefs>;
    fn set(
        &self,
        project: &ProjectId,
        change: ProjectPrefChange,
        expect_revision: u64,
    ) -> Result<ProjectPrefs>;
    fn remove(&self, project: &ProjectId) -> Result<()>;
    fn copy(&self, from: &ProjectId, to: &ProjectId) -> Result<()>;
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectPrefs {
    pub backend: Option<BackendKind>,
    pub model: Option<ModelRef>,
    pub effort: Option<ThinkingLevel>,
    pub mode: AnalysisMode,
    pub view: Option<MainViewPref>,
    pub revision: u64,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectModelChoice {
    pub backend: Option<BackendKind>,
    pub model: Option<ModelRef>,
    pub effort: Option<ThinkingLevel>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MainViewPref {
    Project,
    Workbench,
    Ask,
    GameFiles,
    Pins,
}
/// Each variant changes exactly one preference; None clears an explicit choice.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ProjectPrefChange {
    Backend(Option<BackendKind>),
    Model(Option<ModelRef>),
    Effort(Option<ThinkingLevel>),
    Mode(AnalysisMode),
    View(Option<MainViewPref>),
}
