//! Reference: craft-anything ai/src/rebuild/tools.rs parameter types (300dba58).
//! Strict source-compatible rebuild parameter contracts.
use schemars::JsonSchema;
use serde::Deserialize;
use std::path::PathBuf;
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StartParams {
    /// What to make (a new project), or the follow-up message (with `project`).
    pub request: String,
    /// For a new project: a web address, an absolute path to an app, game or file, or `-` for none.
    #[serde(default)]
    pub target: Option<String>,
    /// Continue this project folder instead of starting a new one.
    #[serde(default)]
    pub project: Option<PathBuf>,
    /// With `project`: carry on where its last run stopped (paused, or the app or the computer
    /// stopped) instead of sending `request` as a follow-up.
    #[serde(default)]
    pub resume: bool,
    /// Where a new project's folder goes (default: the projects folder from settings).
    #[serde(default)]
    pub projects_dir: Option<PathBuf>,
    /// Start the new project in this folder itself instead (it's added to the project list). If
    /// it already has files, `existing` says what they are.
    #[serde(default)]
    pub project_folder: Option<PathBuf>,
    /// With `project_folder` that has files: `edit` (they're the app, edited in place) or
    /// `start_from` (material to start from; the app is built in app/ beside them).
    #[serde(default)]
    pub existing: Option<ca_engine::workspace::ExistingFiles>,
    /// What to build with: an engine (unity, unreal, godot, rust) or a stack's full name.
    #[serde(default)]
    pub build_with: Option<String>,
    /// For a new game: the builder may generate pictures and 3D models (paid from the person's
    /// asset provider key, within the project's budget). Default false; change it later with
    /// project_assets_allow.
    #[serde(default)]
    pub generate_assets: bool,
    /// The coding agent to use (an enabled backend name), kept with a new project for its later
    /// changes; default: the one set up for building.
    #[serde(default)]
    pub backend: Option<String>,
    /// Refinement rounds after the first build (default 2).
    #[serde(default)]
    pub rounds: Option<u32>,
    /// Pictures of the original (absolute paths): the look the person expects. They're copied
    /// into the project's reference/ folder for the builder.
    #[serde(default)]
    pub pictures: Vec<PathBuf>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CheckParams {
    /// The project's folder; default: the project this tool server was started for.
    #[serde(default)]
    pub project: Option<PathBuf>,
    /// Longest wait for the result, in minutes (1..=60, default 35): a website with several pages
    /// or a game's engine build takes a few minutes, and the run stops a check at 30.
    #[serde(default = "d_wait_minutes")]
    pub wait_minutes: u64,
}

/// Longer than the run's own thirty-minute check limit, so a check
/// that hangs is stopped and reported by the run before the builder stops waiting for it.
fn d_wait_minutes() -> u64 {
    35
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProjectParams {
    /// The project's folder.
    pub project: PathBuf,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ActivityParams {
    /// The project's folder.
    pub project: PathBuf,
    /// How many lines to return (200 by default, at most 2000).
    #[serde(default)]
    pub limit: Option<usize>,
    /// How many lines back from the newest to end at (0 by default).
    #[serde(default)]
    pub offset: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NoteParams {
    /// The project's folder.
    pub project: PathBuf,
    /// What to tell the builder.
    pub text: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WithdrawParams {
    /// The project's folder.
    pub project: PathBuf,
    /// The note's id, as rebuild_note returned it.
    pub id: u64,
}
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct QuestionParams {
    /// The project's folder.
    pub project: PathBuf,
    /// The question, in plain words.
    pub question: String,
}
