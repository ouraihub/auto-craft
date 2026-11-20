//! Analysis snapshots, tool calls and editable engine settings (no source dependency).
use crate::{CancelFlag, EvidenceId, ProjectId, QuestionsDto, Result, TargetId};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::BTreeMap, path::PathBuf};

pub trait AnalysisService: Send + Sync {
    fn open(&self, project: &ProjectId) -> Result<()>;
    fn close(&self, project: &ProjectId) -> Result<()>;
    /// Returns the last snapshot with stale=true rather than waiting for the engine lock.
    fn view(&self, project: &ProjectId) -> Result<AnalysisView>;
    fn call(&self, project: &ProjectId, call: ToolCall, cancel: &CancelFlag) -> Result<ToolReply>;
    fn tools(&self, project: &ProjectId) -> Result<Vec<ToolInfo>>;
    fn evidence(&self, project: &ProjectId, id: &EvidenceId) -> Result<Value>;
    fn set_mode(&self, project: &ProjectId, mode: AnalysisMode) -> Result<()>;
    fn busy(&self, project: &ProjectId) -> Option<String>;
    fn settings(&self) -> Result<AnalysisSettings>;
    fn set_settings(&self, change: AnalysisSettingsChange) -> Result<AnalysisSettings>;
    /// Installs external engines using global settings without opening a user project.
    /// # Errors
    /// Returns validation, cancellation or installation failures through the analysis boundary.
    fn install_engines(&self, params: Value, cancel: &CancelFlag) -> Result<ToolReply>;

    /// Runs configured plugin judges for this project kind outside the engine session lock.
    /// # Errors
    /// Returns cancellation or failure to load the project/plugin registry.
    fn run_plugin_checks(
        &self,
        project: &ProjectId,
        kind: &str,
        round: u32,
        context: Value,
        cancel: &CancelFlag,
    ) -> Result<Vec<PluginCheckResult>>;

    /// Active judges and the file extensions whose UTF-8 bodies they request.
    /// Stable identities are included in cached check fingerprints.
    /// # Errors
    /// Returns failure to load the project/plugin registry.
    fn plugin_checks(&self, project: &ProjectId, kind: &str) -> Result<Vec<PluginCheckInfo>>;
}

/// Metadata for one configured plugin judge, filtered to the requested project kind.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginCheckInfo {
    pub id: String,
    pub title: String,
    pub extensions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PluginCheckResult {
    pub id: String,
    pub title: String,
    pub passed: bool,
    pub similarity: Option<f64>,
    pub note: String,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnalysisMode {
    CleanRoom,
    Deep,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnalysisView {
    pub targets: Vec<TargetInfo>,
    pub active: Option<TargetId>,
    pub mode: AnalysisMode,
    pub ledger: Vec<EvidenceSummary>,
    pub unknowns: Vec<UnknownDto>,
    pub busy: Option<String>,
    pub stale: bool,
    pub revision: u64,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TargetInfo {
    pub id: TargetId,
    pub name: String,
    pub kind: String,
    pub path: PathBuf,
    pub sha256: String,
    pub size: u64,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceSummary {
    pub id: EvidenceId,
    pub tool: String,
    pub summary: String,
    pub confidence: Confidence,
    pub created_ms: u64,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Confidence {
    Observed,
    Derived,
    Inferred,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    pub tool: String,
    pub params: Value,
    pub caller: CallerDto,
    pub consented: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CallerDto {
    Ui,
    Rebuild,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ToolReply {
    Evidence { id: Option<EvidenceId>, body: Value },
    Value(Value),
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolInfo {
    pub id: String,
    pub toolset: String,
    pub title_key: String,
    pub title: String,
    pub summary: String,
    pub effects: Vec<String>,
    pub needs_consent: bool,
    pub available: Availability,
    pub input_schema: Value,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Availability {
    Ready,
    NeedsDeep,
    NeedsTarget,
    Missing { engine: String },
    Os,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnknownDto {
    pub id: String,
    pub revision: u64,
    pub question: String,
    pub severity: Severity,
    pub domain: String,
    pub status: UnknownStatus,
    pub probes: Vec<String>,
    pub evidence_for: Vec<EvidenceId>,
    pub evidence_against: Vec<EvidenceId>,
    pub resolution: Option<String>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Severity {
    Low,
    Medium,
    High,
    Critical,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum UnknownStatus {
    Open,
    Investigating,
    Blocked,
    Contradicted,
    Resolved,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnalysisSettings {
    pub deep_mode: bool,
    pub auto_mode: bool,
    pub ask_before_running: bool,
    pub questions: QuestionsDto,
    pub runtime_study: bool,
    pub engines: AnalysisEngineSettings,
    pub build: AnalysisBuildSettings,
    pub assets: AnalysisAssetSettings,
}
/// Partial update; absent fields retain their existing values.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AnalysisSettingsChange {
    pub deep_mode: Option<bool>,
    pub auto_mode: Option<bool>,
    pub ask_before_running: Option<bool>,
    pub questions: Option<QuestionsDto>,
    pub runtime_study: Option<bool>,
    pub engines: Option<AnalysisEngineSettings>,
    pub build: Option<AnalysisBuildSettings>,
    pub assets: Option<AnalysisAssetSettings>,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnalysisEngineSettings {
    pub ghidra: GhidraSettingsDto,
    pub tools: BTreeMap<String, PathBuf>,
    pub store_dir: Option<PathBuf>,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GhidraSettingsDto {
    pub install_dir: Option<PathBuf>,
    pub java_home: Option<PathBuf>,
    pub startup_timeout_secs: Option<u64>,
    pub language_id: Option<String>,
    pub compiler_spec: Option<String>,
    pub memory_gb: Option<u64>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnalysisBuildSettings {
    pub good_enough: f64,
    pub max_rounds: u32,
    pub web_widths: Vec<u32>,
    pub max_pages: usize,
    pub picture_good_enough: Option<f64>,
    pub parallel: usize,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnalysisAssetSettings {
    pub budget_usd: f64,
    pub image_model: Option<String>,
}
