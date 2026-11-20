//! Reference: craft-anything crates/ai/src/rebuild/kinds/mod.rs (300dba58).
//! Check contracts, ordered execution, preview serving and kept-result reuse.
use crate::{CheckId, TargetKind};
use autocraft_core::{
    AnalysisService, CallerDto, CancelFlag, Error, ErrorKind, ProjectId, Result, ToolCall,
    ToolReply,
};
use ca_engine::{
    settings::Settings,
    workspace::{CheckResult, Claim, PageLikeness, Workspace},
};
use serde_json::Value;
use std::{
    fmt, fs,
    net::{IpAddr, Ipv4Addr, TcpListener},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

mod build;
mod credits;
mod kept_check;
mod mechanics;
pub(crate) mod observation;
mod plugins;
mod preview;
mod process;
mod runs;
mod screenshot;
mod self_check;
mod self_test;
mod web_visual;
pub use preview::{PreviewServer, PreviewSink, PreviewSlot};

pub use kept_check::KeptChecks;
pub use self_check::{SelfCheckQueue, SelfCheckRequest, report};

/// Borrowed service and cancellation dependencies for one check round.
pub struct CheckResources<'a> {
    pub settings: &'a Settings,
    pub analysis: &'a dyn AnalysisService,
    pub project: &'a ProjectId,
    pub preview: &'a dyn PreviewSink,
    pub stop: &'a CancelFlag,
}
impl fmt::Debug for CheckResources<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CheckResources").finish_non_exhaustive()
    }
}

pub struct CheckCx<'a> {
    pub ws: &'a Workspace,
    pub round: u32,
    pub deep: bool,
    until: Option<Instant>,
    analysis: &'a dyn AnalysisService,
    project: &'a ProjectId,
    settings: &'a Settings,
    preview: &'a dyn PreviewSink,
    stop: &'a CancelFlag,
    editor: Option<PathBuf>,
}
impl fmt::Debug for CheckCx<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CheckCx")
            .field("round", &self.round)
            .field("deep", &self.deep)
            .field("limited", &self.until.is_some())
            .finish()
    }
}
impl<'a> CheckCx<'a> {
    pub fn new(ws: &'a Workspace, round: u32, deep: bool, resources: CheckResources<'a>) -> Self {
        let CheckResources {
            settings,
            analysis,
            project,
            preview,
            stop,
        } = resources;
        let editor = build::actual_engine(&ws.app_dir())
            .or_else(|| ca_engines::game::Builder::from_stack(&ws.manifest.stack))
            .and_then(|builder| {
                (builder != ca_engines::game::Builder::Rust).then(|| {
                    let store = settings.engine_store_dir();
                    let inv = ca_engines::game::inventory(&ca_engines::game::Places::here(store));
                    inv.best(builder)
                        .filter(|_| inv.ready(builder))
                        .map(|item| item.program.clone())
                })
            })
            .flatten();
        Self {
            ws,
            round,
            deep,
            until: None,
            analysis,
            project,
            settings,
            preview,
            stop,
            editor,
        }
    }
    pub fn limited(mut self, limit: Duration) -> Self {
        self.until = Some(Instant::now() + limit);
        self
    }
    pub fn call(&self, tool: &str, params: Value) -> Result<ToolReply> {
        self.stop_now()?;
        let reply = self.analysis.call(
            self.project,
            ToolCall {
                tool: tool.into(),
                params,
                caller: CallerDto::Rebuild,
                consented: true,
            },
            self.stop,
        )?;
        self.stop_now()?;
        Ok(reply)
    }
    pub fn serve_preview(&self, dir: &Path) -> Result<String> {
        let server = PreviewServer::start(dir)?;
        let url = server.url();
        self.preview.publish(&url);
        self.preview.retain(server);
        Ok(url)
    }
    pub fn build_settings(&self) -> &ca_engine::settings::BuildSettings {
        &self.settings.build
    }
    pub fn editor(&self) -> Option<&Path> {
        self.editor.as_deref()
    }
    pub fn stop(&self) -> CheckStop<'_> {
        CheckStop {
            until: self.until,
            stop: self.stop,
        }
    }
    fn stop_now(&self) -> Result<()> {
        if self.stop.is_cancelled() {
            return Err(Error::new(ErrorKind::Cancelled, "check cancelled"));
        }
        if self.until.is_some_and(|t| Instant::now() >= t) {
            return Err(Error::timeout("check time limit reached"));
        }
        Ok(())
    }
}
pub struct CheckStop<'a> {
    until: Option<Instant>,
    stop: &'a CancelFlag,
}
impl fmt::Debug for CheckStop<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CheckStop")
            .field("cancelled", &self.cancelled())
            .field("limited", &self.until.is_some())
            .finish()
    }
}
impl CheckStop<'_> {
    pub fn cancelled(&self) -> bool {
        self.stop.is_cancelled() || self.until.is_some_and(|t| Instant::now() >= t)
    }
    pub fn time_left(&self) -> Option<Duration> {
        self.until
            .map(|t| t.saturating_duration_since(Instant::now()))
    }
}

pub trait Check: Send + Sync {
    fn id(&self) -> &'static str;
    fn applies(&self, _cx: &CheckCx<'_>) -> bool {
        true
    }
    fn run(&self, cx: &mut CheckCx<'_>) -> Result<CheckResult>;
}
pub fn execute_checks(cx: CheckCx<'_>, kind: &dyn TargetKind) -> Result<Vec<CheckResult>> {
    let mut cx = if cx.round == 0 {
        cx.limited(Duration::from_secs(30 * 60))
    } else {
        cx
    };
    let mut out = Vec::new();
    for id in kind.checks(cx.ws) {
        cx.stop_now()?;
        let check = make_check(id);
        if !check.applies(&cx) {
            continue;
        }
        let result = check.run(&mut cx)?;
        let failed = !result.passed;
        out.push(result);
        if failed {
            break;
        }
    }
    if out.iter().all(|result| result.passed) {
        cx.stop_now()?;
        out.extend(plugins::execute(&cx, kind.id())?);
    }
    Ok(out)
}

fn deep_local(deep: bool, target: &ca_engine::workspace::TargetRef) -> bool {
    deep && matches!(target, ca_engine::workspace::TargetRef::Local { .. })
}

struct DeepOnly<C>(C);
impl<C: Check> Check for DeepOnly<C> {
    fn id(&self) -> &'static str {
        self.0.id()
    }
    fn applies(&self, cx: &CheckCx<'_>) -> bool {
        deep_local(cx.deep, &cx.ws.manifest.target) && self.0.applies(cx)
    }
    fn run(&self, cx: &mut CheckCx<'_>) -> Result<CheckResult> {
        self.0.run(cx)
    }
}

fn make_check(id: CheckId) -> Box<dyn Check> {
    match id {
        CheckId::Build => Box::new(build::BuildCheck { own: false }),
        CheckId::OwnBuild => Box::new(build::BuildCheck { own: true }),
        CheckId::WebVisual => Box::new(web_visual::WebVisualCheck),
        CheckId::Runs => Box::new(runs::RunsCheck),
        CheckId::AppScreenshot => Box::new(screenshot::ScreenshotCheck),
        CheckId::SelfTest => Box::new(self_test::SelfTestCheck),
        CheckId::Mechanics => Box::new(mechanics::MechanicsCheck),
        CheckId::DeepMechanics => Box::new(DeepOnly(mechanics::MechanicsCheck)),
        CheckId::DeepCredits => Box::new(DeepOnly(credits::CreditsCheck)),
        CheckId::OwnTests => Box::new(build::TestsCheck),
    }
}

fn reply_value(reply: ToolReply) -> Value {
    match reply {
        ToolReply::Value(v) | ToolReply::Evidence { body: v, .. } => v,
    }
}
fn claim(title: impl Into<String>, passed: bool, detail: impl Into<String>) -> Claim {
    Claim::new(title, passed, detail)
}
fn relative_artifact(ws: &Workspace, p: &Path) -> Option<PathBuf> {
    p.strip_prefix(&ws.root)
        .ok()
        .map(Path::to_path_buf)
        .filter(|r| !r.components().any(|c| c == std::path::Component::ParentDir))
}

pub(crate) fn error_result(id: &'static str, round: u32, note: impl Into<String>) -> CheckResult {
    CheckResult::new(id, round, false, note)
}
pub(crate) fn passed_result(id: &'static str, round: u32, note: impl Into<String>) -> CheckResult {
    CheckResult::new(id, round, true, note)
}
pub(crate) fn result_with_claims(
    id: &'static str,
    round: u32,
    passed: bool,
    note: impl Into<String>,
    claims: Vec<Claim>,
) -> CheckResult {
    CheckResult {
        claims,
        ..CheckResult::new(id, round, passed, note)
    }
}

#[cfg(test)]
mod tests;
