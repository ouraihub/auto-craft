//! Isolated service boundaries for exercising the real rebuild state machine without an upstream.
use super::*;
use crate::{
    SeesPictures,
    builder::{BuilderHost, BuilderPlan},
};
use serde_json::{Value, json};
use std::sync::{
    Weak,
    atomic::{AtomicUsize, Ordering},
};

pub(super) struct Boundary {
    pub mode: Mutex<AnalysisMode>,
    pub events: Mutex<Vec<RebuildEvent>>,
    pub prompts: Arc<AtomicUsize>,
    pub prompt_text: Arc<Mutex<Vec<String>>>,
    pub write_files: bool,
    pub note_during_check: Mutex<Option<Weak<Run>>>,
    pub resumed: Mutex<Vec<Option<NativeSessionKey>>>,
}
fn unused<T>() -> Result<T> {
    Err(Error::invalid("unused test boundary operation"))
}
impl ProjectService for Boundary {
    fn open(&self, id: &ProjectId) -> Result<ProjectDetail> {
        Ok(ProjectDetail {
            summary: ProjectSummary {
                id: id.clone(),
                name: "State regression".into(),
                request: String::new(),
                target: TargetRefDto::None,
                stack: String::new(),
                status: ProjectStatus::New,
                note: String::new(),
                updated_ms: 0,
                own_project: false,
                pending_requests: 0,
                read_only: false,
            },
            mode: *self.mode.lock().map_err(|_| Error::corrupt("mode lock"))?,
            approvals: vec![],
            approvals_chosen: vec![],
            questions: QuestionsDto::Default,
            asset_budget_usd: 0.0,
            assets_spent_usd: 0.0,
            generate_assets: false,
            latest_checks: vec![],
            usage: ProjectUsageDto::default(),
            app_dir: Path::new(id.as_str()).join("app"),
        })
    }
    fn list(&self) -> Result<Vec<ProjectSummary>> {
        unused()
    }
    fn create(&self, _: NewProject) -> Result<ProjectSummary> {
        unused()
    }
    fn rename(&self, _: &ProjectId, _: &str) -> Result<ProjectSummary> {
        unused()
    }
    fn duplicate(&self, _: &ProjectId, _: &str) -> Result<ProjectSummary> {
        unused()
    }
    fn delete(&self, _: &ProjectId) -> Result<()> {
        unused()
    }
    fn set_person(&self, _: &ProjectId, _: PersonChange) -> Result<ProjectDetail> {
        unused()
    }
    fn locations(&self) -> Result<ProjectLocations> {
        unused()
    }
    fn set_locations(&self, _: ProjectLocations) -> Result<()> {
        unused()
    }
    fn add_references(&self, _: &ProjectId, _: &[std::path::PathBuf]) -> Result<usize> {
        unused()
    }
}
impl AnalysisService for Boundary {
    fn settings(&self) -> Result<AnalysisSettings> {
        let settings = ca_engine::Settings::default();
        Ok(AnalysisSettings {
            deep_mode: false,
            auto_mode: true,
            ask_before_running: false,
            questions: QuestionsDto::Default,
            runtime_study: false,
            engines: AnalysisEngineSettings::default(),
            build: super::transcode(&settings.build)?,
            assets: super::transcode(&settings.assets)?,
        })
    }
    fn view(&self, _: &ProjectId) -> Result<AnalysisView> {
        Ok(AnalysisView {
            targets: vec![],
            active: None,
            mode: *self.mode.lock().map_err(|_| Error::corrupt("mode lock"))?,
            ledger: vec![],
            unknowns: vec![],
            busy: None,
            stale: false,
            revision: 0,
        })
    }
    fn call(&self, _: &ProjectId, call: ToolCall, _: &CancelFlag) -> Result<ToolReply> {
        if call.tool == "web_site_capture" {
            let output = call.params["output_directory"]
                .as_str()
                .ok_or_else(|| Error::invalid("capture output"))?;
            let output = Path::new(output);
            std::fs::create_dir_all(output).map_err(|source| Error::Io {
                context: "test capture".into(),
                source,
            })?;
            std::fs::write(output.join("capture.json"), b"{}").map_err(|source| Error::Io {
                context: "test capture".into(),
                source,
            })?;
            if output.components().any(|part| part.as_os_str() == "checks") {
                let pending = self
                    .note_during_check
                    .lock()
                    .map_err(|_| Error::corrupt("note lock"))?
                    .take();
                if let Some(run) = pending.and_then(|run| run.upgrade()) {
                    run.continuity
                        .lock()
                        .map_err(|_| Error::corrupt("continuity lock"))?
                        .note("Use the new requirement", false, vec![], None)?;
                }
            }
            return Ok(ToolReply::Value(json!({"result": {
                "output_directory": output, "url": "http://127.0.0.1/", "screenshots": []
            }})));
        }
        Ok(ToolReply::Value(json!({})))
    }
    fn open(&self, _: &ProjectId) -> Result<()> {
        unused()
    }
    fn close(&self, _: &ProjectId) -> Result<()> {
        unused()
    }
    fn tools(&self, _: &ProjectId) -> Result<Vec<ToolInfo>> {
        unused()
    }
    fn evidence(&self, _: &ProjectId, _: &EvidenceId) -> Result<Value> {
        unused()
    }
    fn set_mode(&self, _: &ProjectId, _: AnalysisMode) -> Result<()> {
        unused()
    }
    fn busy(&self, _: &ProjectId) -> Option<String> {
        None
    }
    fn set_settings(&self, _: AnalysisSettingsChange) -> Result<AnalysisSettings> {
        unused()
    }
    fn install_engines(&self, _: Value, _: &CancelFlag) -> Result<ToolReply> {
        unused()
    }
    fn run_plugin_checks(
        &self,
        _: &ProjectId,
        _: &str,
        _: u32,
        _: Value,
        _: &CancelFlag,
    ) -> Result<Vec<PluginCheckResult>> {
        Ok(Vec::new())
    }
    fn plugin_checks(&self, _: &ProjectId, _: &str) -> Result<Vec<PluginCheckInfo>> {
        Ok(Vec::new())
    }
}
impl RebuildSink for Boundary {
    fn publish(&self, event: RebuildEvent) {
        if let Ok(mut events) = self.events.lock() {
            events.push(event);
        }
    }
}
impl BuilderHost for Boundary {
    fn prepare(
        &self,
        project: &ProjectId,
        _: bool,
        backend: Option<BackendKind>,
    ) -> Result<BuilderPlan> {
        let root = std::path::PathBuf::from(project.as_str());
        Ok(BuilderPlan {
            request: StartSession {
                chat: ChatId::from("state-builder"),
                project: project.clone(),
                backend: backend.unwrap_or(BackendKind::Dsh),
                launch: Launch::External {
                    executable: root.join("unused-engine"),
                    args: vec![],
                },
                config_root: None,
                cwd: root.clone(),
                resume: None,
                analysis: None,
                builder: None,
                routing: EngineRouting::default(),
                support_dir: root.join("support"),
                role: SessionRole::Worker,
            },
            sees_pictures: SeesPictures::No,
            assets: false,
        })
    }
    fn start(
        &self,
        request: StartSession,
        _: Arc<dyn SessionSink>,
    ) -> Result<Arc<dyn AgentSession>> {
        self.resumed
            .lock()
            .map_err(|_| Error::corrupt("resume lock"))?
            .push(request.resume);
        Ok(Arc::new(Session {
            cwd: request.cwd,
            backend: request.backend,
            revision: AtomicUsize::new(0),
            write_files: self.write_files,
            prompts: self.prompts.clone(),
            prompt_text: self.prompt_text.clone(),
        }))
    }
    fn start_question(
        &self,
        _: &ProjectId,
        _: bool,
        _: std::path::PathBuf,
        _: Arc<dyn SessionSink>,
    ) -> Result<(Arc<dyn AgentSession>, AnsweredFrom)> {
        unused()
    }
    fn hold(&self, _: &ProjectId) -> Result<()> {
        Ok(())
    }
    fn release(&self, _: &ProjectId) -> Result<()> {
        Ok(())
    }
    fn bind(&self, _: &ProjectId, _: BackendKind, _: &NativeSessionKey) -> Result<()> {
        Ok(())
    }
    fn session_dir(&self, _: BackendKind) -> Result<std::path::PathBuf> {
        unused()
    }
}
struct Session {
    cwd: std::path::PathBuf,
    backend: BackendKind,
    revision: AtomicUsize,
    write_files: bool,
    prompts: Arc<AtomicUsize>,
    prompt_text: Arc<Mutex<Vec<String>>>,
}
impl AgentSession for Session {
    fn snapshot(&self) -> SessionSnapshot {
        SessionSnapshot {
            id: ChatId::from("state-builder"),
            backend: self.backend,
            native_session_key: Some(NativeSessionKey::from("state-native")),
            cwd: self.cwd.clone(),
            phase: SessionPhase::Idle,
            model: None,
            messages: vec![],
            interactions: vec![],
            capabilities: Capabilities::default(),
            output_truncated: false,
            last_error: None,
            commands: vec![],
            session_name: None,
            revision: self.revision.load(Ordering::SeqCst) as u64,
        }
    }
    fn prompt(&self, text: &str) -> Result<()> {
        self.prompt_text
            .lock()
            .map_err(|_| Error::corrupt("prompt lock"))?
            .push(text.to_owned());
        self.prompts.fetch_add(1, Ordering::SeqCst);
        let revision = self.revision.fetch_add(1, Ordering::SeqCst) + 1;
        if self.write_files {
            std::fs::write(self.cwd.join("index.html"), format!("<h1>{revision}</h1>")).map_err(
                |source| Error::Io {
                    context: "test builder edit".into(),
                    source,
                },
            )?;
        }
        Ok(())
    }
    fn stop(&self) -> Result<()> {
        Ok(())
    }
    fn respond(&self, _: &str, _: InteractionAnswer) -> Result<()> {
        unused()
    }
    fn dispose(&self) -> Result<()> {
        Ok(())
    }
    fn set_model(&self, _: &str, _: &str, _: Option<ThinkingLevel>) -> Result<()> {
        unused()
    }
}

pub(super) struct Fixture {
    pub _dir: tempfile::TempDir,
    pub ws: Workspace,
    pub boundary: Arc<Boundary>,
    pub config: RebuildConfig,
    pub run: Arc<Run>,
}
impl Fixture {
    pub fn new(request: &str, deep: bool, write_files: bool) -> Result<Self> {
        let dir = tempfile::tempdir().map_err(|source| Error::Io {
            context: "test workspace".into(),
            source,
        })?;
        let target = dir.path().join("original.bin");
        std::fs::write(&target, b"isolated target").map_err(|source| Error::Io {
            context: "test target".into(),
            source,
        })?;
        let mut ws = Workspace::create(
            dir.path(),
            "State regression",
            request,
            ca_engine::workspace::TargetRef::Local { path: target },
        )
        .map_err(source)?;
        ws.manifest.stack = "HTML + CSS + JavaScript".into();
        ws.save().map_err(source)?;
        let boundary = Arc::new(Boundary {
            mode: Mutex::new(if deep {
                AnalysisMode::Deep
            } else {
                AnalysisMode::CleanRoom
            }),
            events: Mutex::new(vec![]),
            prompts: Arc::new(AtomicUsize::new(0)),
            prompt_text: Arc::new(Mutex::new(vec![])),
            write_files,
            note_during_check: Mutex::new(None),
            resumed: Mutex::new(vec![]),
        });
        let config = RebuildConfig {
            projects: boundary.clone(),
            analysis: boundary.clone(),
            host: boundary.clone(),
            sink: boundary.clone(),
        };
        let project = ProjectId::from(ws.root.to_string_lossy().into_owned());
        let run = Run::new(project, ws.root.clone(), &config)?;
        Ok(Self {
            _dir: dir,
            ws,
            boundary,
            config,
            run,
        })
    }
    pub fn execute(&mut self, request: RunRequest) -> Result<()> {
        self.ws.save().map_err(source)?;
        let result = self.run.execute(&self.config, request);
        let failure = result
            .as_ref()
            .err()
            .map(|e| Error::new(e.kind().unwrap_or(ErrorKind::External), e.detail()));
        self.run.finish(result)?;
        self.ws = Workspace::open(&self.ws.root).map_err(source)?;
        failure.map_or(Ok(()), Err)
    }
}
