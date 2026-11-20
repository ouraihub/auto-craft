//! Application-owned launch transaction for rebuild workers and read-only questions.
use autocraft_analysis::{AnalysisHub, Holder};
use autocraft_bridge::Bridge;
use autocraft_core::*;
use autocraft_rebuild::{
    SeesPictures,
    builder::{BuilderHost, BuilderPlan},
};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, PoisonError, Weak,
        atomic::{AtomicBool, Ordering},
    },
};

#[derive(Default)]
pub struct RuntimeConfig {
    pub launches: [Option<Launch>; 3],
    pub config_roots: [Option<PathBuf>; 3],
    pub host_executable: Option<PathBuf>,
}
pub(super) fn index(backend: BackendKind) -> usize {
    match backend {
        BackendKind::Pi => 0,
        BackendKind::Omp => 1,
        BackendKind::Dsh => 2,
    }
}
pub(super) struct AppBuilder {
    pub hub: Arc<AnalysisHub>,
    pub bridge: Arc<Bridge>,
    pub projects: Arc<dyn ProjectPrefService>,
    pub prefs: Arc<dyn ModelPrefService>,
    pub chats: Arc<dyn ChatBindingService>,
    pub builders: Arc<dyn BuilderChatService>,
    pub asset_keys: Arc<dyn AssetKeyService>,
    pub router: Arc<dyn ModelRouter>,
    pub engines: Arc<dyn EngineSetup>,
    pub runtimes: [Arc<dyn AgentRuntime>; 3],
    pub config: RuntimeConfig,
    pub support: PathBuf,
    pub pending: Mutex<HashMap<ChatId, Lease>>,
    pub sessions: Mutex<Vec<Weak<ManagedSession>>>,
}
pub(super) struct Host(pub Arc<AppBuilder>);
impl std::ops::Deref for Host {
    type Target = AppBuilder;
    fn deref(&self) -> &AppBuilder {
        &self.0
    }
}
pub(super) struct Lease {
    project: ProjectId,
    chat: ChatId,
    token: Option<String>,
    held: bool,
    new_session: bool,
    purpose: ChatPurpose,
}
impl AppBuilder {
    fn backend(&self, project: &ProjectId) -> Result<BackendKind> {
        self.projects
            .get(project)?
            .backend
            .or(self.prefs.prefs()?.default_agent)
            .ok_or_else(|| Error::invalid("请先选择默认 Agent，或在项目设置中选择 Agent。"))
    }
    pub(super) fn launch(&self, backend: BackendKind) -> Result<Launch> {
        if let Some(launch) = &self.config.launches[index(backend)] {
            return Ok(launch.clone());
        }
        self.engines
            .status(backend)?
            .map(|installed| installed.launch)
            .ok_or_else(|| Error::new(ErrorKind::BackendUnavailable, "项目选择的 Agent 尚未安装"))
    }
    fn cleanup(&self, lease: Lease) -> Result<()> {
        self.router.release(&lease.chat);
        let mut result = super::remove_support(&self.support, &lease.chat);
        if let Some(token) = lease.token {
            self.bridge.revoke(&token);
        }
        if lease.held {
            self.hub.cancel_chat(&lease.chat);
            super::first(
                &mut result,
                super::retained(self.hub.release(&lease.project, &Holder::Chat(lease.chat))),
            );
        }
        result
    }
    fn prepare_chat(
        &self,
        project: &ProjectId,
        chat: ChatId,
        backend: BackendKind,
        resume: Option<NativeSessionKey>,
        purpose: ChatPurpose,
    ) -> Result<BuilderPlan> {
        let launch = self.launch(backend)?;
        let prefs = self.projects.get(project)?;
        let route = RouteRequest {
            chat: chat.clone(),
            project: project.clone(),
            backend,
            new_session: resume.is_none(),
            model: prefs.model,
            effort: prefs.effort,
        };
        let mut lease = Lease {
            project: project.clone(),
            chat: chat.clone(),
            token: None,
            held: false,
            new_session: resume.is_none(),
            purpose,
        };
        let result = (|| {
            self.hub.hold(project, Holder::Chat(chat.clone()))?;
            lease.held = true;
            let routing = autocraft_routing::prepare_project_route(self.router.as_ref(), &route)?;
            let sees_pictures = routing
                .default_model
                .as_ref()
                .and_then(|default| {
                    routing
                        .providers
                        .iter()
                        .find(|provider| provider.route == default.route)
                        .and_then(|provider| {
                            provider
                                .models
                                .iter()
                                .find(|model| model.id == default.model_id)
                        })
                })
                .map_or(SeesPictures::Unknown, |model| {
                    if model.vision {
                        SeesPictures::Yes
                    } else {
                        SeesPictures::No
                    }
                });
            let analysis = if purpose == ChatPurpose::Question {
                None
            } else {
                let executable = match &self.config.host_executable {
                    Some(path) => path.clone(),
                    None => std::env::current_exe().map_err(|source| Error::Io {
                        context: "无法定位宿主程序".into(),
                        source,
                    })?,
                };
                let options = autocraft_agents::channel_options(backend);
                let token = self.bridge.register(
                    ChatScope {
                        project: project.clone(),
                        chat: chat.clone(),
                        purpose,
                    },
                    options,
                );
                lease.token = Some(token.clone());
                Some(AnalysisAccess {
                    executable,
                    pipe: self.bridge.pipe_name().to_owned(),
                    token,
                    options,
                })
            };
            // The pinned built-in asset catalogue uses fal; arbitrary custom keys do not qualify.
            let assets = self
                .asset_keys
                .asset_key_summaries()?
                .iter()
                .any(|key| key.provider == "fal");
            Ok(BuilderPlan {
                request: StartSession {
                    chat: chat.clone(),
                    project: project.clone(),
                    backend,
                    launch,
                    config_root: self.config.config_roots[index(backend)].clone(),
                    cwd: super::project_dir(project)?,
                    resume,
                    analysis,
                    builder: None,
                    routing,
                    support_dir: super::support_dir(&self.support, &chat)?,
                    role: if purpose == ChatPurpose::Question {
                        SessionRole::ReadOnly
                    } else {
                        SessionRole::Worker
                    },
                },
                sees_pictures,
                assets,
            })
        })();
        match result {
            Ok(plan) => {
                self.pending
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .insert(chat, lease);
                Ok(plan)
            }
            Err(error) => {
                if self.cleanup(lease).is_err() {
                    tracing::warn!("rebuild.launch.cleanup_failed");
                }
                Err(error)
            }
        }
    }
    pub fn shutdown(&self) -> Result<()> {
        let mut result = Ok(());
        let sessions: Vec<_> = self
            .sessions
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .drain(..)
            .filter_map(|session| session.upgrade())
            .collect();
        for session in sessions {
            super::first(&mut result, session.dispose());
        }
        let pending: Vec<_> = self
            .pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .drain()
            .map(|(_, lease)| lease)
            .collect();
        for lease in pending {
            super::first(&mut result, self.cleanup(lease));
        }
        result
    }
}
impl BuilderHost for Host {
    fn prepare(
        &self,
        project: &ProjectId,
        deep: bool,
        backend: Option<BackendKind>,
    ) -> Result<BuilderPlan> {
        let backend = match backend {
            Some(backend) => backend,
            None => self.backend(project)?,
        };
        let builder = self.builders.builder_chat(project, backend)?;
        let ws = ca_engine::workspace::Workspace::open(Path::new(project.as_str()))
            .map_err(|error| Error::external(error.to_string()))?;
        let id = match backend {
            BackendKind::Pi => "pi",
            BackendKind::Omp => "omp",
            BackendKind::Dsh => "dsh",
        };
        let resume = ws
            .manifest
            .agent_session_for_mode(id, deep)
            .map(NativeSessionKey::from);
        self.prepare_chat(project, builder.chat, backend, resume, ChatPurpose::Builder)
    }
    fn start(
        &self,
        mut request: StartSession,
        sink: Arc<dyn SessionSink>,
    ) -> Result<Arc<dyn AgentSession>> {
        let resume_model = request.routing.default_model.clone();
        let lease = self
            .pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&request.chat)
            .ok_or_else(|| Error::invalid("构建方启动缺少已准备的租约"))?;
        if lease.new_session != request.resume.is_none() {
            self.router.release(&request.chat);
            let prefs = self.projects.get(&request.project);
            let routing = prefs.and_then(|prefs| {
                autocraft_routing::prepare_project_route(
                    self.router.as_ref(),
                    &RouteRequest {
                        chat: request.chat.clone(),
                        project: request.project.clone(),
                        backend: request.backend,
                        new_session: request.resume.is_none(),
                        model: prefs.model,
                        effort: prefs.effort,
                    },
                )
            });
            match routing {
                Ok(routing) => request.routing = routing,
                Err(error) => {
                    let _ = self.cleanup(lease);
                    return Err(error);
                }
            }
        }
        let binding = (request.role == SessionRole::Worker).then(|| {
            (
                request.project.clone(),
                request.chat.clone(),
                request.backend,
            )
        });
        match self.runtimes[index(request.backend)].start(request, sink) {
            Ok(inner) => {
                if !lease.new_session
                    && let Some(model) = &resume_model
                {
                    inner.set_model(&model.route, &model.model_id, model.effort)?;
                }
                if let Some((project, chat, backend)) = binding {
                    let result = inner
                        .snapshot()
                        .native_session_key
                        .ok_or_else(|| Error::new(ErrorKind::Protocol, "Agent 未返回真实会话标识"))
                        .and_then(|native_session| {
                            self.chats.bind(ChatBinding {
                                project,
                                chat,
                                backend,
                                native_session,
                            })
                        });
                    if let Err(error) = result {
                        let _ = inner.dispose();
                        let _ = self.cleanup(lease);
                        return Err(error);
                    }
                }
                let session = Arc::new(ManagedSession {
                    inner,
                    host: self.0.clone(),
                    lease: Mutex::new(Some(lease)),
                    disposed: AtomicBool::new(false),
                });
                let mut sessions = self.sessions.lock().unwrap_or_else(PoisonError::into_inner);
                sessions.retain(|session| session.strong_count() != 0);
                sessions.push(Arc::downgrade(&session));
                Ok(session)
            }
            Err(error) => {
                if self.cleanup(lease).is_err() {
                    tracing::warn!("rebuild.launch.cleanup_failed");
                }
                Err(error)
            }
        }
    }
    fn start_question(
        &self,
        project: &ProjectId,
        deep: bool,
        ws_root: PathBuf,
        sink: Arc<dyn SessionSink>,
    ) -> Result<(Arc<dyn AgentSession>, AnsweredFrom)> {
        let stored = self.builders.find_builder_chat(project)?;
        let backend = match stored.as_ref() {
            Some(builder) => builder.backend,
            None => self.backend(project)?,
        };
        let ws = ca_engine::workspace::Workspace::open(&ws_root)
            .map_err(|error| Error::external(error.to_string()))?;
        let resume = if backend == BackendKind::Pi {
            ws.manifest
                .agent_session_for_mode("pi", deep)
                .filter(|key| Path::new(key).is_file())
                .map(NativeSessionKey::from)
        } else {
            None
        };
        let from = if resume.is_some() {
            AnsweredFrom::BuildersSession
        } else {
            AnsweredFrom::TheFiles
        };
        let mut plan = self.prepare_chat(
            project,
            ChatId::generate(),
            backend,
            resume,
            ChatPurpose::Question,
        )?;
        plan.request.cwd = ws_root;
        self.start(plan.request, sink)
            .map(|session| (session, from))
    }
    fn hold(&self, project: &ProjectId) -> Result<()> {
        self.hub.hold(project, Holder::Run)
    }
    fn release(&self, project: &ProjectId) -> Result<()> {
        let mut pending = self.pending.lock().unwrap_or_else(PoisonError::into_inner);
        let chats: Vec<_> = pending
            .iter()
            .filter(|(_, lease)| &lease.project == project && lease.purpose == ChatPurpose::Builder)
            .map(|(chat, _)| chat.clone())
            .collect();
        let leases: Vec<_> = chats
            .iter()
            .filter_map(|chat| pending.remove(chat))
            .collect();
        drop(pending);
        let mut result = Ok(());
        for lease in leases {
            super::first(&mut result, self.cleanup(lease));
        }
        super::first(
            &mut result,
            super::retained(self.hub.release(project, &Holder::Run)),
        );
        result
    }
    fn bind(
        &self,
        project: &ProjectId,
        backend: BackendKind,
        key: &NativeSessionKey,
    ) -> Result<()> {
        let builder = self.builders.builder_chat(project, backend)?;
        self.chats.bind(ChatBinding {
            project: project.clone(),
            chat: builder.chat,
            backend,
            native_session: key.clone(),
        })
    }
    fn session_dir(&self, backend: BackendKind) -> Result<PathBuf> {
        let root = match &self.config.config_roots[index(backend)] {
            Some(root) => root.clone(),
            None => {
                let home = std::env::var_os("USERPROFILE")
                    .or_else(|| std::env::var_os("HOME"))
                    .ok_or_else(|| {
                        Error::new(ErrorKind::BackendUnavailable, "无法定位 Agent 配置目录")
                    })?;
                Path::new(&home).join(match backend {
                    BackendKind::Pi => ".pi/agent",
                    BackendKind::Omp => ".omp/agent",
                    BackendKind::Dsh => ".dsh",
                })
            }
        };
        Ok(root.join("sessions"))
    }
}

pub(super) struct ManagedSession {
    inner: Arc<dyn AgentSession>,
    host: Arc<AppBuilder>,
    lease: Mutex<Option<Lease>>,
    disposed: AtomicBool,
}
impl AgentSession for ManagedSession {
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
        let mut result = self.inner.dispose();
        if let Some(lease) = self
            .lease
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        {
            super::first(&mut result, self.host.cleanup(lease));
        }
        result
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
impl Drop for ManagedSession {
    fn drop(&mut self) {
        if let Err(error) = self.dispose() {
            tracing::warn!(error_kind = ?error.kind(), "rebuild.session.cleanup_failed");
        }
    }
}
