//! Rebuild orchestration requests, results and nonblocking events.
use crate::{
    CancelFlag, InboxAnswer, InboxRequest, NoteId, ProjectId, ProjectStatus, Result, SessionEvent,
};
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, time::Duration};
pub trait RebuildService: Send + Sync {
    fn start(&self, project: &ProjectId, request: RunRequest) -> Result<RunStarted>;
    fn stop(&self, project: &ProjectId) -> Result<()>;
    fn note(&self, project: &ProjectId, text: &str, pictures: Vec<PathBuf>) -> Result<NoteId>;
    fn withdraw(&self, project: &ProjectId, id: &NoteId) -> Result<Option<String>>;
    fn status(&self, project: &ProjectId) -> Result<RunStatus>;
    fn self_check(
        &self,
        project: &ProjectId,
        wait: Duration,
        cancel: &CancelFlag,
    ) -> Result<String>;
    fn ask(&self, project: &ProjectId, question: &str, cancel: &CancelFlag) -> Result<Answered>;
    fn finish(&self, project: &ProjectId) -> Result<()>;
    fn preview(&self, project: &ProjectId) -> Result<String>;
    fn play(&self, project: &ProjectId) -> Result<()>;
    fn activity(
        &self,
        project: &ProjectId,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<ActivityLine>>;
    /// Reads the persisted project feed, including completed runs.
    fn read_feed(&self, project: &ProjectId) -> Result<Vec<FeedEntryDto>>;
    /// Copies selected pictures into the project's reference storage.
    fn attach_pictures(&self, project: &ProjectId, pictures: Vec<PathBuf>) -> Result<Vec<PathBuf>>;
    /// Stores clipboard image bytes in the project's reference storage.
    fn attach_image(&self, project: &ProjectId, bytes: Vec<u8>, mime: &str) -> Result<PathBuf>;
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunRequest {
    pub message: Option<String>,
    pub shown: Option<String>,
    pub pictures: Vec<PathBuf>,
    pub continuing: bool,
    /// Per-run override; None uses the configured build limit without changing settings.
    #[serde(default)]
    pub max_rounds: Option<u32>,
    /// Backend override for this run only; never rewrites project preferences.
    #[serde(default)]
    pub backend: Option<crate::BackendKind>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RunStarted {
    Running,
    Queued { position: usize },
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunStatus {
    pub running: bool,
    pub queued: Option<usize>,
    pub step: Option<StepDto>,
    pub status: ProjectStatus,
    pub note: String,
    pub latest_checks: Vec<CheckResultDto>,
    pub waiting_notes: Vec<(NoteId, String)>,
    pub pending_requests: u32,
    pub quiet_for_ms: u64,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StepDto {
    Study,
    Build,
    Check,
    Done,
}
/// UI sinks must enqueue without blocking engine or orchestration threads.
pub trait RebuildSink: Send + Sync {
    fn publish(&self, event: RebuildEvent);
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum RebuildEvent {
    Step {
        project: ProjectId,
        step: StepDto,
        text: String,
    },
    Agent {
        project: ProjectId,
        event: SessionEvent,
    },
    NoteRead {
        project: ProjectId,
        id: NoteId,
        shown: String,
        pictures: Vec<PathBuf>,
    },
    Ask {
        project: ProjectId,
        request: InboxRequest,
    },
    Checked {
        project: ProjectId,
        result: CheckResultDto,
    },
    Preview {
        project: ProjectId,
        url: String,
    },
    Finished {
        project: ProjectId,
        ok: bool,
        message: String,
        paused: bool,
    },
    Queue {
        project: ProjectId,
        position: Option<usize>,
    },
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CheckResultDto {
    pub round: u32,
    pub check: String,
    pub passed: bool,
    pub note: String,
    pub similarity: Option<f64>,
    pub good_enough: Option<f64>,
    #[serde(default)]
    pub claims: Vec<ClaimDto>,
    #[serde(default)]
    pub unknowns: Vec<String>,
    #[serde(default)]
    pub artifacts: Vec<PathBuf>,
    #[serde(default)]
    pub pages: Vec<PageLikenessDto>,
    pub at_ms: u64,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaimDto {
    pub title: String,
    pub passed: bool,
    #[serde(default)]
    pub detail: String,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PageLikenessDto {
    pub name: String,
    pub dir: String,
    pub similarity: f64,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Answered {
    pub answer: String,
    pub from: AnsweredFrom,
    pub backend: String,
    pub cost_usd: Option<f64>,
    pub cost_is_estimate: bool,
    pub builder_notice: Option<String>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AnsweredFrom {
    BuildersSession,
    TheFiles,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivityLine {
    pub at: Duration,
    pub kind: LineKind,
    pub text: String,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LineKind {
    Stage,
    Started,
    Done,
    Failed,
    Said,
    Note,
    Output,
    Problem,
}

/// Persisted feed projection, independent of the analysis source and UI.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum FeedEntryDto {
    User {
        text: String,
        pictures: Vec<PathBuf>,
    },
    Agent {
        text: String,
        streaming: bool,
    },
    Work {
        tools: Vec<FeedToolDto>,
        notes: Vec<String>,
    },
    Ask {
        request: InboxRequest,
        answer: Option<InboxAnswer>,
    },
    /// A completed read-only question, merged from persisted questions at its feed position.
    Question {
        question: String,
        answer: String,
        from_session: bool,
        backend: String,
        at_ms: u64,
    },
    Check {
        result: CheckResultDto,
    },
    Step {
        step: StepDto,
        text: String,
    },
    Error {
        message: String,
        remediation: Option<String>,
    },
    Paused,
    Plan {
        steps: Vec<FeedPlanStepDto>,
    },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeedToolDto {
    pub name: String,
    pub ok: Option<bool>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeedPlanStepDto {
    pub text: String,
    pub doing: String,
    pub status: FeedPlanStatus,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FeedPlanStatus {
    Todo,
    Doing,
    Done,
    Stopped,
}
