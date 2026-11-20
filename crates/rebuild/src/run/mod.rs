//! Reference: craft-anything crates/ai/src/rebuild/mod.rs::start/run_outcome (300dba58).
//! One owned run, its cancellation, session and terminal state.
mod check;
mod flow;
mod service;
#[cfg(test)]
mod state_fixture;
#[cfg(test)]
mod state_tests;
#[cfg(test)]
mod tests;
mod turn;
mod usage;
use crate::{
    RebuildConfig,
    checks::{PreviewSlot, SelfCheckQueue},
    feed::Continuity,
};
use autocraft_core::*;
use ca_engine::workspace::{CheckResult, Status, Workspace};
use std::{
    path::Path,
    sync::{Arc, Mutex, PoisonError},
    time::Instant,
};

pub(crate) struct Run {
    pub project: ProjectId,
    pub root: std::path::PathBuf,
    pub cancel: CancelFlag,
    pub native_cancel: ca_engine::CancelToken,
    pub session: Mutex<Option<Arc<dyn AgentSession>>>,
    pub continuity: Mutex<Continuity>,
    pub preview: Arc<PreviewSlot>,
    pub self_checks: SelfCheckQueue,
    pub state: Mutex<RunState>,
}
pub(crate) struct RunState {
    pub step: Option<StepDto>,
    pub touched: Instant,
    pub done: bool,
}
impl Run {
    pub fn new(
        project: ProjectId,
        root: std::path::PathBuf,
        config: &RebuildConfig,
    ) -> Result<Arc<Self>> {
        let continuity = Mutex::new(Continuity::new(
            root.clone(),
            project.clone(),
            config.sink.clone(),
        )?);
        let preview = Arc::new(PreviewSlot::new());
        Ok(Arc::new(Self {
            project,
            root,
            cancel: CancelFlag::new(),
            native_cancel: ca_engine::CancelToken::default(),
            session: Mutex::new(None),
            continuity,
            preview,
            self_checks: SelfCheckQueue::default(),
            state: Mutex::new(RunState {
                step: None,
                touched: Instant::now(),
                done: false,
            }),
        }))
    }
    pub fn stop(&self) -> Result<()> {
        self.cancel.cancel();
        self.native_cancel.cancel();
        let session = self
            .session
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        if let Some(session) = session {
            session.stop()?;
        }
        Ok(())
    }
    pub fn cancelled(&self) -> Result<()> {
        if self.cancel.is_cancelled() {
            Err(Error::new(ErrorKind::Cancelled, "复刻已暂停"))
        } else {
            Ok(())
        }
    }
    pub fn emit(&self, event: RebuildEvent) -> Result<()> {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .touched = Instant::now();
        self.continuity
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .emit(&event)
    }
    pub fn step(
        &self,
        ws: &mut Workspace,
        step: StepDto,
        status: Status,
        text: &str,
    ) -> Result<()> {
        self.cancelled()?;
        ws.set_status(status, text).map_err(source)?;
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .step = Some(step);
        self.emit(RebuildEvent::Step {
            project: self.project.clone(),
            step,
            text: text.into(),
        })
    }
    pub fn execute(
        self: &Arc<Self>,
        config: &RebuildConfig,
        request: RunRequest,
    ) -> Result<String> {
        let mut ws = Workspace::open(&self.root).map_err(source)?;
        config.host.hold(&self.project)?;
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            flow::execute(self, config, &mut ws, request)
        }));
        let failed =
            outcome.as_ref().is_err() || outcome.as_ref().is_ok_and(|result| result.is_err());
        let session = self
            .session
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        let cleanup = session.map_or(Ok(()), |s| {
            let stopped = if failed { s.stop() } else { Ok(()) };
            let disposed = s.dispose();
            stopped.and(disposed)
        });
        let released = config.host.release(&self.project);
        let result = match outcome {
            Ok(result) => result,
            Err(payload) => {
                if cleanup.is_err() || released.is_err() {
                    tracing::warn!("rebuild.cleanup.failed");
                }
                std::panic::resume_unwind(payload);
            }
        };
        match result {
            Ok(summary) => {
                cleanup?;
                released?;
                Ok(summary)
            }
            Err(error) => {
                if cleanup.is_err() || released.is_err() {
                    tracing::warn!("rebuild.cleanup.failed");
                }
                Err(error)
            }
        }
    }
    pub fn finish(&self, result: Result<String>) -> Result<()> {
        let mut ws = Workspace::open(&self.root).map_err(source)?;
        let paused = self.cancel.is_cancelled()
            || result
                .as_ref()
                .err()
                .is_some_and(|e| e.kind() == Some(ErrorKind::Cancelled));
        let ok = result.is_ok() && !paused;
        let text = if paused {
            "已暂停。发一条消息继续。".into()
        } else {
            match result {
                Ok(s) => s,
                Err(e) => e.detail().to_owned(),
            }
        };
        ws.set_status(
            if paused {
                Status::Stopped
            } else if ok {
                Status::Ready
            } else {
                Status::NeedsYou
            },
            &text,
        )
        .map_err(source)?;
        self.self_checks.stop();
        self.emit(RebuildEvent::Finished {
            project: self.project.clone(),
            ok,
            message: text,
            paused,
        })?;
        self.continuity
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .finish()?;
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .done = true;
        Ok(())
    }
}

pub(crate) fn source(error: ca_evidence::ToolError) -> Error {
    use ca_evidence::ErrorCode as C;
    let kind = match error.code {
        C::Cancelled => ErrorKind::Cancelled,
        C::NotSupported => ErrorKind::ReadOnly,
        C::NotFound => ErrorKind::NotFound,
        C::BadSettings | C::BadInput => ErrorKind::Invalid,
        C::TimedOut => ErrorKind::Timeout,
        other => {
            let _ = other;
            ErrorKind::External
        }
    };
    Error::new(kind, "项目数据或分析操作失败，请查看项目状态后重试")
}
pub(crate) fn dto(result: &CheckResult) -> Result<CheckResultDto> {
    Ok(crate::feed::check_dto(result))
}
pub(crate) fn transcode<T: serde::Serialize, U: serde::de::DeserializeOwned>(v: &T) -> Result<U> {
    serde_json::to_value(v)
        .and_then(serde_json::from_value)
        .map_err(|_| Error::corrupt("项目数据格式不兼容"))
}
pub(crate) fn settings(value: &AnalysisSettings) -> Result<ca_engine::Settings> {
    let mut s = ca_engine::Settings {
        deep_mode: value.deep_mode,
        auto_mode: value.auto_mode,
        ask_before_running: value.ask_before_running,
        runtime_study: value.runtime_study,
        build: transcode(&value.build)?,
        assets: transcode(&value.assets)?,
        ..Default::default()
    };
    s.engines.ghidra = transcode(&value.engines.ghidra)?;
    s.engines.store_dir = value.engines.store_dir.clone();
    for (id, path) in s.engines.tools.entries_mut() {
        *path = value.engines.tools.get(id).cloned();
    }
    Ok(s)
}
pub(crate) fn open(root: &Path) -> Result<Workspace> {
    Workspace::open(root).map_err(source)
}
