//! Reference: craft-anything crates/ai/src/rebuild/mod.rs::agent_spec (300dba58).
//! Composition-root boundary for installed builders, routing, credentials and native bindings.
use crate::SeesPictures;
use autocraft_core::*;
use std::{path::PathBuf, sync::Arc};

/// A launch prepared once with the project's effective backend, model, route and MCP credentials.
pub struct BuilderPlan {
    pub request: StartSession,
    pub sees_pictures: SeesPictures,
    /// At least one supported image-asset provider has usable credentials.
    pub assets: bool,
}
impl std::fmt::Debug for BuilderPlan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BuilderPlan")
            .field("backend", &self.request.backend)
            .field("sees_pictures", &self.sees_pictures)
            .finish_non_exhaustive()
    }
}

/// The application owns concrete agent runtimes, the settings database and authenticated bridge.
pub trait BuilderHost: Send + Sync {
    /// Resolve preferences, verify installation and prepare the stable builder chat launch.
    fn prepare(
        &self,
        project: &ProjectId,
        deep: bool,
        backend: Option<BackendKind>,
    ) -> Result<BuilderPlan>;
    /// Launch the selected runtime and bind its actual native locator.
    fn start(
        &self,
        request: StartSession,
        sink: Arc<dyn SessionSink>,
    ) -> Result<Arc<dyn AgentSession>>;
    /// Start a disposable read-only question session, optionally forked from a builder.
    fn start_question(
        &self,
        project: &ProjectId,
        deep: bool,
        ws_root: PathBuf,
        sink: Arc<dyn SessionSink>,
    ) -> Result<(Arc<dyn AgentSession>, AnsweredFrom)>;
    /// Keep the project's analysis engine alive for this run.
    fn hold(&self, project: &ProjectId) -> Result<()>;
    /// Release the run's analysis-engine hold.
    fn release(&self, project: &ProjectId) -> Result<()>;
    /// Persist a relocated or newly created native builder session.
    fn bind(&self, project: &ProjectId, backend: BackendKind, key: &NativeSessionKey)
    -> Result<()>;
    /// Native transcript directory used when restoring a portable project copy.
    fn session_dir(&self, backend: BackendKind) -> Result<PathBuf>;
}

pub(crate) fn backend_id(backend: BackendKind) -> &'static str {
    match backend {
        BackendKind::Pi => "pi",
        BackendKind::Omp => "omp",
        BackendKind::Dsh => "dsh",
    }
}
