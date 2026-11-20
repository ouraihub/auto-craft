//! Track actual engine sessions without retaining a conversation or creating an ownership cycle.
use autocraft_core::*;
use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex, PoisonError, Weak,
        atomic::{AtomicBool, Ordering},
    },
};
#[derive(Default)]
pub(super) struct Registry(Mutex<RegistryState>);
#[derive(Default)]
struct RegistryState {
    stopped: bool,
    sessions: Vec<Weak<Tracked>>,
}
impl Registry {
    pub fn shutdown(&self) -> Result<()> {
        let sessions: Vec<_> = {
            let mut state = self.0.lock().unwrap_or_else(PoisonError::into_inner);
            state.stopped = true;
            state
                .sessions
                .drain(..)
                .filter_map(|session| session.upgrade())
                .collect()
        };
        let mut result = Ok(());
        for session in sessions {
            super::first(&mut result, session.dispose());
        }
        result
    }
}
pub(super) struct Runtime {
    pub inner: Arc<dyn AgentRuntime>,
    pub registry: Arc<Registry>,
    pub config_root: Option<PathBuf>,
}
impl AgentRuntime for Runtime {
    fn backend(&self) -> BackendKind {
        self.inner.backend()
    }
    fn capabilities(&self) -> Capabilities {
        self.inner.capabilities()
    }
    fn start(
        &self,
        mut request: StartSession,
        sink: Arc<dyn SessionSink>,
    ) -> Result<Arc<dyn AgentSession>> {
        if self
            .registry
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .stopped
        {
            return Err(Error::new(
                ErrorKind::SessionClosed,
                "应用正在退出，无法启动 Agent",
            ));
        }
        if request.config_root.is_none() {
            request.config_root = self.config_root.clone();
        }
        let inner = self.inner.start(request, sink)?;
        let session = Arc::new(Tracked {
            inner,
            disposed: AtomicBool::new(false),
        });
        let mut state = self
            .registry
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if state.stopped {
            drop(state);
            if session.dispose().is_err() {
                tracing::warn!("app.session.shutdown_cleanup_failed");
            }
            return Err(Error::new(
                ErrorKind::SessionClosed,
                "应用正在退出，Agent 已关闭",
            ));
        }
        state.sessions.retain(|session| session.strong_count() != 0);
        state.sessions.push(Arc::downgrade(&session));
        Ok(session)
    }
}
struct Tracked {
    inner: Arc<dyn AgentSession>,
    disposed: AtomicBool,
}
impl AgentSession for Tracked {
    fn snapshot(&self) -> SessionSnapshot {
        self.inner.snapshot()
    }
    fn usage_snapshot(&self) -> Result<Option<SessionUsage>> {
        self.inner.usage_snapshot()
    }
    fn prompt(&self, text: &str) -> Result<()> {
        self.inner.prompt(text)
    }
    fn steer(&self, text: &str) -> Result<()> {
        self.inner.steer(text)
    }
    fn stop(&self) -> Result<()> {
        self.inner.stop()
    }
    fn respond(&self, id: &str, answer: InteractionAnswer) -> Result<()> {
        self.inner.respond(id, answer)
    }
    fn dispose(&self) -> Result<()> {
        if self.disposed.swap(true, Ordering::AcqRel) {
            return Ok(());
        }
        self.inner.dispose()
    }
    fn set_model(&self, provider: &str, model: &str, effort: Option<ThinkingLevel>) -> Result<()> {
        self.inner.set_model(provider, model, effort)
    }
    fn compact(&self, instructions: Option<&str>) -> Result<()> {
        self.inner.compact(instructions)
    }
    fn rename(&self, name: &str) -> Result<()> {
        self.inner.rename(name)
    }
}
impl Drop for Tracked {
    fn drop(&mut self) {
        if let Err(error) = self.dispose() {
            tracing::warn!(error_kind = ?error.kind(), "app.session.cleanup_failed");
        }
    }
}
