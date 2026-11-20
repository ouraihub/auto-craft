//! Reference: craft-anything crates/ai/src/rebuild/runs.rs (300dba58).
//! Rebuild service, FIFO scheduling and owned worker lifecycle.
pub mod brief;
pub mod kinds;
pub use kinds::{BriefCx, CheckId, DetectCx, KindRegistry, SeesPictures, TargetKind};
mod activity;
mod approvals;
pub mod builder;
pub mod checks;
mod feed;
mod notes;
mod question;
mod run;
mod session_copy;
pub mod stacks;
mod study;
pub mod tools;
use autocraft_core::*;
use brief::*;
use builder::BuilderHost;
use ca_engine::workspace::{CheckResult, Status, TargetRef, Workspace};
use ca_engines::game;
use std::{
    collections::{BTreeMap, VecDeque},
    path::{Path, PathBuf},
    sync::{Arc, Condvar, Mutex, PoisonError},
    thread,
    time::{Duration, Instant},
};

/// Services resolved only at the composition root; rebuild has no concrete service dependency.
#[derive(Clone)]
pub struct RebuildConfig {
    pub projects: Arc<dyn ProjectService>,
    pub analysis: Arc<dyn AnalysisService>,
    pub host: Arc<dyn BuilderHost>,
    pub sink: Arc<dyn RebuildSink>,
}
struct State {
    runs: BTreeMap<ProjectId, Arc<run::Run>>,
    active: Vec<ProjectId>,
    queue: VecDeque<(ProjectId, RunRequest)>,
    shutdown: bool,
}
struct Shared {
    config: RebuildConfig,
    state: Mutex<State>,
    settled: Condvar,
}
/// Project rebuild scheduler. Workers retain the shared state until their terminal write completes.
pub struct RebuildHub {
    shared: Arc<Shared>,
}
impl std::fmt::Debug for RebuildHub {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RebuildHub").finish_non_exhaustive()
    }
}
impl std::fmt::Debug for RebuildConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RebuildConfig").finish_non_exhaustive()
    }
}
impl RebuildHub {
    /// Correct interrupted projects before accepting new work.
    pub fn new(config: RebuildConfig) -> Result<Self> {
        for project in config.projects.list()? {
            if matches!(
                project.status,
                ProjectStatus::Building | ProjectStatus::Checking | ProjectStatus::Inspecting
            ) && !project.read_only
            {
                let mut ws = run::open(Path::new(project.id.as_str()))?;
                ws.set_status(Status::Stopped, "上次没有正常结束。发一条消息继续。")
                    .map_err(run::source)?;
            }
        }
        Ok(Self {
            shared: Arc::new(Shared {
                config,
                state: Mutex::new(State {
                    runs: BTreeMap::new(),
                    active: Vec::new(),
                    queue: VecDeque::new(),
                    shutdown: false,
                }),
                settled: Condvar::new(),
            }),
        })
    }
    fn project_run(&self, project: &ProjectId) -> Result<Arc<run::Run>> {
        let mut state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(run) = state.runs.get(project) {
            return Ok(run.clone());
        }
        let detail = self.shared.config.projects.open(project)?;
        if detail.summary.read_only {
            return Err(Error::new(
                ErrorKind::ReadOnly,
                "项目由更新版本创建，不能修改",
            ));
        }
        let root = PathBuf::from(project.as_str());
        let run = run::Run::new(project.clone(), root, &self.shared.config)?;
        state.runs.insert(project.clone(), run.clone());
        Ok(run)
    }
    /// Wait at most ten seconds, then dispose any remaining native sessions and mark them paused.
    pub fn shutdown(&self) -> Result<()> {
        let (runs, queued) = {
            let mut state = self
                .shared
                .state
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            state.shutdown = true;
            let queued = state.queue.drain(..).map(|(p, _)| p).collect::<Vec<_>>();
            (state.runs.values().cloned().collect::<Vec<_>>(), queued)
        };
        let mut failure = None;
        for run in &runs {
            if let Err(e) = run.stop() {
                failure.get_or_insert(e);
            }
            run.preview.shutdown();
            if queued.contains(&run.project)
                && let Err(e) = run.finish(Err(Error::new(ErrorKind::Cancelled, "应用退出")))
            {
                failure.get_or_insert(e);
            }
        }
        let until = Instant::now() + Duration::from_secs(10);
        let mut state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        while !state.active.is_empty() && Instant::now() < until {
            let result = self
                .shared
                .settled
                .wait_timeout(state, until.saturating_duration_since(Instant::now()))
                .unwrap_or_else(PoisonError::into_inner);
            state = result.0;
        }
        let active = state.active.clone();
        drop(state);
        for project in active {
            let run = self.project_run(&project)?;
            let session = run
                .session
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take();
            if let Some(session) = session
                && let Err(e) = session.dispose()
            {
                failure.get_or_insert(e);
            }
            let mut ws = run::open(&run.root)?;
            ws.set_status(Status::Stopped, "已暂停。发一条消息继续。")
                .map_err(run::source)?;
        }
        if let Some(e) = failure {
            Err(e)
        } else {
            Ok(())
        }
    }
    /// Record only the engine's successful core_plan_update projection.
    pub fn plan(&self, project: &ProjectId, value: &serde_json::Value) -> Result<()> {
        self.project_run(project)?
            .continuity
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .plan(value)
    }
    /// Answer a native engine inbox card using the same persisted project inbox.
    pub fn answer(
        &self,
        project: &ProjectId,
        id: &InboxRequestId,
        answer: InboxAnswer,
    ) -> Result<()> {
        let run = self.project_run(project)?;
        let inbox = ca_engine::inbox::Inbox::new(run.root.join("inbox"));
        if !inbox.pending().iter().any(|r| r.id == id.as_str()) {
            return Err(Error::new(ErrorKind::InteractionExpired, "该确认已结束"));
        }
        let native = match answer {
            InboxAnswer::Choice { choice, text } => ca_engine::inbox::Answer {
                choice,
                text,
                allow: None,
                always: false,
            },
            InboxAnswer::Approval {
                allow,
                always,
                text,
            } => ca_engine::inbox::Answer {
                choice: None,
                text,
                allow: Some(allow),
                always,
            },
        };
        let shown = match native.allow {
            Some(true) if native.always => "Always allowed".into(),
            Some(true) => "Allowed".into(),
            Some(false) => "Not allowed".into(),
            None => native.as_text(),
        };
        run.continuity
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .answered(id.as_str(), &shown)?;
        inbox.answer(id.as_str(), &native).map_err(run::source)
    }
}
fn schedule(shared: &Arc<Shared>) -> Result<()> {
    let parallel = shared.config.analysis.settings()?.build.parallel.max(1);
    loop {
        let next = {
            let mut state = shared.state.lock().unwrap_or_else(PoisonError::into_inner);
            if state.shutdown || state.active.len() >= parallel {
                return Ok(());
            }
            let Some((project, request)) = state.queue.pop_front() else {
                return Ok(());
            };
            let run = state
                .runs
                .get(&project)
                .cloned()
                .ok_or_else(|| Error::corrupt("排队项目没有运行状态"))?;
            state.active.push(project.clone());
            Some((project, request, run))
        };
        let Some((project, request, run)) = next else {
            return Ok(());
        };
        shared.config.sink.publish(RebuildEvent::Queue {
            project: project.clone(),
            position: None,
        });
        let owned = shared.clone();
        let failed_project = project.clone();
        let launched = thread::Builder::new()
            .name("rebuild-run".into())
            .spawn(move || {
                let (_trace, operation) =
                    lumakit_observability::TraceContext::root("rebuild.run", "rebuild");
                let result = {
                    let span = operation.span().clone();
                    let _entered = span.enter();
                    run.execute(&owned.config, request)
                };
                let failure = result.as_ref().err().map(|e| (e.kind(), e.code()));
                let finalized = {
                    let span = operation.span().clone();
                    let _entered = span.enter();
                    run.finish(result)
                };
                match (failure, finalized) {
                    (_, Err(e)) => operation.fail(e.code()),
                    (None, Ok(())) => operation.finish("READY"),
                    (Some((Some(ErrorKind::Cancelled), _)), Ok(())) => operation.cancel("USER"),
                    (Some((_, code)), Ok(())) => operation.fail(code),
                }
                {
                    let mut state = owned.state.lock().unwrap_or_else(PoisonError::into_inner);
                    state.active.retain(|p| p != &project);
                    owned.settled.notify_all();
                }
                if let Err(error) = schedule(&owned) {
                    tracing::error!(code = error.code(), "rebuild.queue.failed");
                }
            });
        if let Err(error) = launched {
            shared
                .state
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .active
                .retain(|p| p != &failed_project);
            return Err(Error::Io {
                context: "无法启动复刻工作线程".into(),
                source: error,
            });
        }
    }
}
