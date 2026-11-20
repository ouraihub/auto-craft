//! Composition root: concrete services and the channels that connect them to the window.

#[path = "services/builder.rs"]
mod builder;
#[path = "services/rebuild.rs"]
mod rebuild;
#[path = "services/sessions.rs"]
mod sessions;
use anyhow::Context as _;
use autocraft_agents::{DshRuntime, ManagedEngines, OmpRuntime, PiRuntime};
use autocraft_analysis::{
    AnalysisConfig, AnalysisEvent, AnalysisHub, AnalysisSink, AssetKeySource, Holder, LongToolSink,
};
use autocraft_bridge::Bridge;
use autocraft_core::*;
use autocraft_observability::LogHandle;
use autocraft_rebuild::{RebuildConfig, RebuildHub};
use autocraft_routing::{Gateway, Probe, Router};
use autocraft_settings::SqliteSettingsStore;
use autocraft_terminal::PtyTerminalService;
pub use builder::RuntimeConfig;
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock, Weak},
};
use tracing::{debug, warn};

#[derive(Clone)]
pub struct Services {
    pub projects: Arc<dyn ProjectService>,
    pub analysis: Arc<dyn AnalysisService>,
    pub inbox: Arc<dyn InboxService>,
    pub project_prefs: Arc<dyn ProjectPrefService>,
    pub asset_keys: Arc<dyn AssetKeyService>,
    pub chats: Arc<dyn ChatBindingService>,
    pub builder_chats: Arc<dyn BuilderChatService>,
    pub rebuild: Option<Arc<dyn RebuildService>>,
    pub providers: Arc<dyn ProviderService>,
    pub mcp: Arc<dyn McpService>,
    pub prefs: Arc<dyn ModelPrefService>,
    pub router: Arc<dyn ModelRouter>,
    pub probe: Arc<dyn ProviderProbe>,
    pub engines: Arc<dyn EngineSetup>,
    pub app_prefs: Arc<dyn AppPrefService>,
    pub log: Arc<LogHandle>,
    pub terminals: Arc<dyn TerminalService>,
    pub session_sink: Arc<dyn SessionSink>,
    hub: Arc<AnalysisHub>,
    bridge: Arc<Bridge>,
    runtimes: [Arc<dyn AgentRuntime>; 3],
    support: PathBuf,
    builder_host: Arc<builder::AppBuilder>,
    rebuild_hub: Arc<RebuildHub>,
    sessions: Arc<sessions::Registry>,
}
impl std::fmt::Debug for Services {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Services").finish_non_exhaustive()
    }
}
pub struct Channels {
    pub sessions: async_channel::Receiver<(tracing::Span, SessionEvent)>,
    pub analysis: async_channel::Receiver<(tracing::Span, AnalysisEvent)>,
    pub rebuild: async_channel::Receiver<(tracing::Span, RebuildEvent)>,
    pub terminals: async_channel::Receiver<(tracing::Span, TerminalEvent)>,
    pub long_tools: async_channel::Receiver<(tracing::Span, ChatScope, String)>,
}
struct Events {
    sessions: async_channel::Sender<(tracing::Span, SessionEvent)>,
    analysis: async_channel::Sender<(tracing::Span, AnalysisEvent)>,
    rebuild: async_channel::Sender<(tracing::Span, RebuildEvent)>,
    terminals: async_channel::Sender<(tracing::Span, TerminalEvent)>,
    long_tools: async_channel::Sender<(tracing::Span, ChatScope, String)>,
}
impl SessionSink for Events {
    fn publish(&self, event: SessionEvent) {
        if self
            .sessions
            .try_send((tracing::Span::current(), event))
            .is_err()
        {
            debug!("app.session.delivery_closed");
        }
    }
}
impl AnalysisSink for Events {
    fn publish(&self, event: AnalysisEvent) {
        if self
            .analysis
            .try_send((tracing::Span::current(), event))
            .is_err()
        {
            debug!("app.analysis.delivery_closed");
        }
    }
}
impl RebuildSink for Events {
    fn publish(&self, event: RebuildEvent) {
        if self
            .rebuild
            .try_send((tracing::Span::current(), event))
            .is_err()
        {
            debug!("app.rebuild.delivery_closed");
        }
    }
}
impl TerminalSink for Events {
    fn publish(&self, event: TerminalEvent) {
        if self
            .terminals
            .send_blocking((tracing::Span::current(), event))
            .is_err()
        {
            debug!("app.terminal.delivery_closed");
        }
    }
}
impl LongToolSink for Events {
    fn deliver(&self, scope: &ChatScope, text: String) -> Result<()> {
        self.long_tools
            .try_send((tracing::Span::current(), scope.clone(), text))
            .map_err(|_| Error::new(ErrorKind::SessionClosed, "聊天已关闭，无法送达后台工具结果"))
    }
}
struct AssetSource(Arc<dyn AssetKeyService>);
impl AssetKeySource for AssetSource {
    fn asset_keys(&self) -> Result<Vec<(String, Secret)>> {
        self.0.asset_keys()
    }
}
impl Services {
    pub fn runtime(&self, backend: BackendKind) -> Arc<dyn AgentRuntime> {
        self.runtimes[match backend {
            BackendKind::Pi => 0,
            BackendKind::Omp => 1,
            BackendKind::Dsh => 2,
        }]
        .clone()
    }
    pub fn channel_options(&self, backend: BackendKind) -> ChannelOptions {
        autocraft_agents::channel_options(backend)
    }
    pub fn launch(&self, backend: BackendKind) -> Result<Option<Launch>> {
        if let Some(launch) = &self.builder_host.config.launches[builder::index(backend)] {
            return Ok(Some(launch.clone()));
        }
        Ok(self
            .engines
            .status(backend)?
            .map(|installed| installed.launch))
    }
    pub fn readiness(&self) -> Result<(Readiness, Option<Launch>)> {
        let default = self.prefs.prefs()?.default_agent;
        let mut installed = Vec::new();
        let mut launch = None;
        for backend in BackendKind::ALL {
            if let Some(installed_launch) = self.launch(backend)? {
                installed.push(backend);
                if default == Some(backend) {
                    launch = Some(installed_launch);
                }
            }
        }
        Ok((autocraft_core::readiness(default, &installed), launch))
    }
    pub fn project_dir(&self, project: &ProjectId) -> Result<PathBuf> {
        project_dir(project)
    }
    pub fn support_dir(&self, chat: &ChatId) -> Result<PathBuf> {
        support_dir(&self.support, chat)
    }
    pub fn remove_support_dir(&self, chat: &ChatId) -> Result<()> {
        remove_support(&self.support, chat)
    }
    pub fn analysis_chat_bindings(&self, project: &ProjectId) -> Result<Vec<ChatBinding>> {
        let builder = self.builder_chats.find_builder_chat(project)?;
        Ok(self
            .chats
            .bindings(project)?
            .into_iter()
            .filter(|binding| {
                builder
                    .as_ref()
                    .is_none_or(|builder| builder.chat != binding.chat)
            })
            .collect())
    }
    pub fn cancel_chat_tools(&self, chat: &ChatId) {
        self.hub.cancel_chat(chat);
    }
    pub fn hold_analysis(&self, project: &ProjectId, chat: &ChatId) -> Result<()> {
        self.hub.hold(project, Holder::Chat(chat.clone()))
    }
    pub fn release_analysis(&self, project: &ProjectId, chat: &ChatId) -> Result<()> {
        self.hub.cancel_chat(chat);
        retained(self.hub.release(project, &Holder::Chat(chat.clone())))
    }
    pub fn register_analysis(
        &self,
        scope: ChatScope,
        options: ChannelOptions,
    ) -> Result<AnalysisAccess> {
        let executable = match &self.builder_host.config.host_executable {
            Some(path) => path.clone(),
            None => std::env::current_exe().map_err(|source| Error::Io {
                context: "无法定位宿主程序".into(),
                source,
            })?,
        };
        let token = self.bridge.register(scope, options);
        Ok(AnalysisAccess {
            executable,
            pipe: self.bridge.pipe_name().to_owned(),
            token,
            options,
        })
    }
    pub fn revoke_analysis(&self, token: &str) {
        self.bridge.revoke(token);
    }
    pub fn update_plan(&self, project: &ProjectId, steps: &serde_json::Value) -> Result<()> {
        self.rebuild_hub.plan(project, steps)
    }
    pub fn shutdown(&self) {
        if let Err(error) = self.rebuild_hub.shutdown() {
            warn!(error_kind = ?error.kind(), "app.rebuild.shutdown_failed");
        }
        if let Err(error) = self.sessions.shutdown() {
            warn!(error_kind = ?error.kind(), "app.sessions.shutdown_failed");
        }
        if let Err(error) = self.builder_host.shutdown() {
            warn!(error_kind = ?error.kind(), "app.builders.shutdown_failed");
        }
        self.hub.shutdown();
        self.terminals.shutdown();
    }
}
pub fn build(
    data_dir: &Path,
    db_path: &Path,
    log: Arc<LogHandle>,
) -> anyhow::Result<(Services, Channels)> {
    build_with(data_dir, db_path, log, RuntimeConfig::default())
}
pub fn build_with(
    data_dir: &Path,
    db_path: &Path,
    log: Arc<LogHandle>,
    runtime_config: RuntimeConfig,
) -> anyhow::Result<(Services, Channels)> {
    anyhow::Context::context(std::fs::create_dir_all(data_dir), "无法创建数据目录")?;
    let settings = Arc::new(
        SqliteSettingsStore::open(db_path)
            .context("无法打开设置数据库；请使用兼容版本或新的数据目录")?,
    );
    let providers: Arc<dyn ProviderService> = settings.clone();
    let mcp: Arc<dyn McpService> = settings.clone();
    let prefs: Arc<dyn ModelPrefService> = settings.clone();
    let app_prefs: Arc<dyn AppPrefService> = settings.clone();
    let stored = app_prefs.app_prefs()?;
    if log.set_level(stored.log_level).is_err() {
        warn!(code = "LOG_LEVEL_APPLY_FAILED", "app.logs.level_fallback");
    }
    let router = Arc::new(Router::new(
        providers.clone(),
        mcp.clone(),
        prefs.clone(),
        Gateway::new(providers.clone()),
    ));
    let (session_tx, sessions) = async_channel::unbounded();
    let (analysis_tx, analysis) = async_channel::unbounded();
    let (rebuild_tx, rebuild) = async_channel::unbounded();
    let (terminal_tx, terminals) = async_channel::bounded(256);
    let (long_tx, long_tools) = async_channel::unbounded();
    let events = Arc::new(Events {
        sessions: session_tx,
        analysis: analysis_tx,
        rebuild: rebuild_tx,
        terminals: terminal_tx,
        long_tools: long_tx,
    });
    let asset_keys: Arc<dyn AssetKeyService> = settings.clone();
    let mut config = AnalysisConfig::new(
        data_dir.to_path_buf(),
        settings.clone(),
        Arc::new(AssetSource(asset_keys.clone())),
        events.clone(),
    );
    let rebuild_slot: Arc<OnceLock<Weak<RebuildHub>>> = Arc::new(OnceLock::new());
    config.extra_tools = autocraft_rebuild::tools::specs();
    let handle_slot: Arc<OnceLock<autocraft_rebuild::tools::RebuildHandle>> =
        Arc::new(OnceLock::new());
    config.rebuild = Some(Arc::new(rebuild::Host {
        service: rebuild_slot.clone(),
        handle: handle_slot.clone(),
    }));
    let init_handle = handle_slot.clone();
    config.session_init = Some(Arc::new(move |session| {
        if let Some(handle) = init_handle.get() {
            session.ext.insert(Arc::new(handle.clone()));
        }
    }));
    config.long_results = Some(events.clone());
    let hub = AnalysisHub::try_new(config)?;
    let bridge = Arc::new(Bridge::start(hub.clone())?);
    let terminals_service = Arc::new(PtyTerminalService::new(data_dir, events.clone()));
    let engines: Arc<dyn EngineSetup> = Arc::new(ManagedEngines::new(data_dir));
    let runtimes: [Arc<dyn AgentRuntime>; 3] = [
        Arc::new(PiRuntime::new()),
        Arc::new(OmpRuntime::new()),
        Arc::new(DshRuntime::new()),
    ];
    let support = data_dir.join("engines/support");
    let sessions_registry = Arc::new(sessions::Registry::default());
    let runtimes: [Arc<dyn AgentRuntime>; 3] = std::array::from_fn(|index| {
        Arc::new(sessions::Runtime {
            inner: runtimes[index].clone(),
            registry: sessions_registry.clone(),
            config_root: runtime_config.config_roots[index].clone(),
        }) as Arc<dyn AgentRuntime>
    });
    let builder_host = Arc::new(builder::AppBuilder {
        hub: hub.clone(),
        bridge: bridge.clone(),
        projects: settings.clone(),
        prefs: prefs.clone(),
        chats: settings.clone(),
        builders: settings.clone(),
        router: router.clone(),
        engines: engines.clone(),
        asset_keys: asset_keys.clone(),
        runtimes: runtimes.clone(),
        config: runtime_config,
        support: support.clone(),
        pending: Mutex::new(std::collections::HashMap::new()),
        sessions: Mutex::new(Vec::new()),
    });
    let rebuild_hub = Arc::new(RebuildHub::new(RebuildConfig {
        projects: hub.clone(),
        analysis: hub.clone(),
        host: Arc::new(builder::Host(builder_host.clone())),
        sink: events.clone(),
    })?);
    handle_slot
        .set(rebuild::handle(
            &rebuild_hub,
            &hub,
            settings.clone(),
            builder_host.clone(),
        ))
        .map_err(|_| anyhow::anyhow!("复刻工具重复初始化"))?;
    rebuild_slot
        .set(Arc::downgrade(&rebuild_hub))
        .map_err(|_| anyhow::anyhow!("复刻服务重复初始化"))?;
    let services = Services {
        projects: hub.clone(),
        analysis: hub.clone(),
        inbox: hub.clone(),
        project_prefs: settings.clone(),
        asset_keys,
        chats: settings.clone(),
        builder_chats: settings,
        rebuild: Some(rebuild_hub.clone()),
        providers: providers.clone(),
        mcp,
        prefs,
        router,
        probe: Arc::new(Probe::new(providers)),
        engines,
        app_prefs,
        log,
        terminals: terminals_service,
        session_sink: events,
        hub,
        bridge,
        runtimes,
        support,
        builder_host,
        rebuild_hub,
        sessions: sessions_registry,
    };
    Ok((
        services,
        Channels {
            sessions,
            analysis,
            rebuild,
            terminals,
            long_tools,
        },
    ))
}

fn project_dir(project: &ProjectId) -> Result<PathBuf> {
    let path = PathBuf::from(project.as_str());
    if !path.is_absolute() || !path.is_dir() {
        return Err(Error::invalid("项目目录不存在"));
    }
    Ok(path)
}
fn support_path(root: &Path, chat: &ChatId) -> Result<PathBuf> {
    let id = chat.as_str();
    if id.is_empty() || !id.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-') {
        return Err(Error::invalid("聊天标识无效"));
    }
    Ok(root.join(id))
}
fn support_dir(root: &Path, chat: &ChatId) -> Result<PathBuf> {
    let path = support_path(root, chat)?;
    std::fs::create_dir_all(&path).map_err(|source| Error::Io {
        context: "无法创建聊天辅助目录".into(),
        source,
    })?;
    Ok(path)
}
fn remove_support(root: &Path, chat: &ChatId) -> Result<()> {
    match std::fs::remove_dir_all(support_path(root, chat)?) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(Error::Io {
            context: "无法删除聊天辅助目录".into(),
            source,
        }),
    }
}
fn retained(result: Result<()>) -> Result<()> {
    match result {
        Err(error) if error.kind() == Some(ErrorKind::Busy) => Ok(()),
        other => other,
    }
}
fn first(result: &mut Result<()>, next: Result<()>) {
    if result.is_ok() {
        *result = next;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retained_analysis_is_not_cleanup_failure() {
        assert!(retained(Err(Error::busy("view still owns engine"))).is_ok());
        let error = retained(Err(Error::new(ErrorKind::SessionClosed, "closed"))).unwrap_err();
        assert_eq!(error.kind(), Some(ErrorKind::SessionClosed));
    }

    #[test]
    fn support_cleanup_never_creates_or_escapes_a_chat_directory() {
        let root = tempfile::tempdir().unwrap();
        let support = root.path().join("support");
        let chat = ChatId::generate();
        remove_support(&support, &chat).unwrap();
        assert!(!support.exists());
        assert!(support_path(&support, &ChatId::from("../outside")).is_err());
        let path = support_dir(&support, &chat).unwrap();
        assert!(path.is_dir());
        remove_support(&support, &chat).unwrap();
        assert!(!path.exists());
    }
}
