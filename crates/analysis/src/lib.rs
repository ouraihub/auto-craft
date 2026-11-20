//! Per-project analysis engines, workspace operations, inbox and transparent MCP serving.
mod calls;
mod catalog;
mod channel;
mod engines;
mod errors;
mod host_exec;
mod inbox;
mod install;
mod plugins;
mod project_dto;
mod projects;
mod session;
mod settings;
mod view;
mod wire;

#[cfg(test)]
mod plugin_tests;
use autocraft_core::*;
use ca_engine::{Settings, ToolSpec};
use parking_lot::Mutex;
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        Arc, Weak,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};
/// Canonical project identity shared with composition callbacks.
/// # Errors
/// Returns NOT_FOUND for missing paths and INVALID for non-Unicode paths.
pub fn project_id(root: &std::path::Path) -> Result<ProjectId> {
    project_dto::id(root)
}
use engines::Loaded;

/// Supplies decrypted asset credentials without coupling the analysis and settings services.
pub trait AssetKeySource: Send + Sync {
    /// # Errors
    /// Returns the credential store's error.
    fn asset_keys(&self) -> Result<Vec<(String, Secret)>>;
}
/// Nonblocking application event consumer. Implementations enqueue rather than call back into engines.
pub trait AnalysisSink: Send + Sync {
    fn publish(&self, event: AnalysisEvent);
}
/// Observable changes; plan updates intentionally carry structured steps to the rebuild feed.
#[derive(Debug, Clone)]
pub enum AnalysisEvent {
    Changed {
        project: ProjectId,
    },
    Progress {
        project: ProjectId,
        tool: String,
        message: String,
        fraction: Option<f32>,
    },
    Inbox {
        project: ProjectId,
        request: InboxRequest,
    },
    InboxWithdrawn {
        project: ProjectId,
        id: InboxRequestId,
    },
    PlanUpdated {
        project: ProjectId,
        steps: serde_json::Value,
    },
    ModeChanged {
        project: ProjectId,
    },
}
/// Delivery to an agent runtime, which decides prompt versus steer/queued prompt.
pub trait LongToolSink: Send + Sync {
    /// # Errors
    /// Returns delivery failure, including a closed chat.
    fn deliver(&self, scope: &ChatScope, text: String) -> Result<()>;
}
/// Rebuild-owned operations that must never execute under an analysis session lock.
pub trait HostRebuild: Send + Sync {
    fn running(&self, project: &ProjectId) -> bool;
    /// # Errors
    /// Returns the active run's answer/self-check failure.
    fn call(
        &self,
        project: &ProjectId,
        tool: &str,
        params: serde_json::Value,
        cancel: &CancelFlag,
    ) -> Result<serde_json::Value>;
    /// # Errors
    /// Returns the run's inbox/feed write failure.
    fn answer(&self, project: &ProjectId, id: &InboxRequestId, answer: InboxAnswer) -> Result<()>;
}
/// A live owner of one shared project engine.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Holder {
    Chat(ChatId),
    Run,
    View,
}
/// Installs higher-layer extension state before the shared engine is created.
pub type SessionInit = Arc<dyn Fn(&mut ca_engine::Session) + Send + Sync>;
/// Explicit composition dependencies. Optional callbacks are rejected when a caller needs them.
pub struct AnalysisConfig {
    pub data_dir: PathBuf,
    pub prefs: Arc<dyn ProjectPrefService>,
    pub asset_keys: Arc<dyn AssetKeySource>,
    pub sink: Arc<dyn AnalysisSink>,
    pub extra_tools: Vec<ToolSpec>,
    pub rebuild: Option<Arc<dyn HostRebuild>>,
    pub long_results: Option<Arc<dyn LongToolSink>>,
    pub session_init: Option<SessionInit>,
}
impl AnalysisConfig {
    pub fn new(
        data_dir: PathBuf,
        prefs: Arc<dyn ProjectPrefService>,
        asset_keys: Arc<dyn AssetKeySource>,
        sink: Arc<dyn AnalysisSink>,
    ) -> Self {
        Self {
            data_dir,
            prefs,
            asset_keys,
            sink,
            extra_tools: Vec::new(),
            rebuild: None,
            long_results: None,
            session_init: None,
        }
    }
}
impl std::fmt::Debug for AnalysisConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AnalysisConfig").finish_non_exhaustive()
    }
}
/// Registry owner. The registry mutex is never held while entering an upstream session.
pub struct AnalysisHub {
    data_dir: PathBuf,
    prefs: Arc<dyn ProjectPrefService>,
    asset_keys: Arc<dyn AssetKeySource>,
    sink: Arc<dyn AnalysisSink>,
    settings: Mutex<Settings>,
    loaded: Mutex<HashMap<ProjectId, Arc<Loaded>>>,
    load_gate: Mutex<()>,
    extra_tools: Vec<ToolSpec>,
    rebuild: Option<Arc<dyn HostRebuild>>,
    long_results: Option<Arc<dyn LongToolSink>>,
    session_init: Option<SessionInit>,
    me: Weak<Self>,
    stopped: AtomicBool,
    next_call: AtomicU64,
}
impl std::fmt::Debug for AnalysisHub {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AnalysisHub")
            .field("loaded", &self.loaded.lock().len())
            .finish_non_exhaustive()
    }
}
impl AnalysisHub {
    /// # Errors
    /// Returns failure to create the private application analysis directory or polling thread.
    pub fn try_new(config: AnalysisConfig) -> Result<Arc<Self>> {
        std::fs::create_dir_all(config.data_dir.join("analysis")).map_err(|source| Error::Io {
            context: "无法创建分析数据目录".into(),
            source,
        })?;
        let settings = settings::load(&config.data_dir);
        let hub = Arc::new_cyclic(|me| Self {
            data_dir: config.data_dir,
            prefs: config.prefs,
            asset_keys: config.asset_keys,
            sink: config.sink,
            settings: Mutex::new(settings),
            loaded: Mutex::new(HashMap::new()),
            load_gate: Mutex::new(()),
            extra_tools: config.extra_tools,
            rebuild: config.rebuild,
            long_results: config.long_results,
            me: me.clone(),
            stopped: AtomicBool::new(false),
            next_call: AtomicU64::new(1),
            session_init: config.session_init,
        });
        inbox::start(&hub)?;
        Ok(hub)
    }
    fn settings_path(&self) -> PathBuf {
        self.data_dir.join("analysis/config.toml")
    }
    fn changed(&self, project: &ProjectId) {
        self.sink.publish(AnalysisEvent::Changed {
            project: project.clone(),
        });
    }
    fn is_running(&self, project: &ProjectId) -> bool {
        self.loaded
            .lock()
            .get(project)
            .is_some_and(|l| l.holders.lock().contains(&Holder::Run))
            || self.rebuild.as_ref().is_some_and(|r| r.running(project))
    }
    /// Cancel background jobs belonging to a stopped chat (request cancellation does not cancel DSH jobs).
    pub fn cancel_chat(&self, chat: &ChatId) {
        for loaded in self.loaded.lock().values() {
            for call in loaded.calls.lock().values() {
                if call.chat.as_ref() == Some(chat) {
                    call.cancel.cancel();
                    call.token.cancel();
                }
            }
        }
    }
    /// Cancel work before releasing run/chat/view owners. Active calls keep their engine alive.
    pub fn shutdown(&self) {
        self.stopped.store(true, Ordering::Release);
        let entries: Vec<_> = self.loaded.lock().values().cloned().collect();
        for loaded in &entries {
            for call in loaded.calls.lock().values() {
                call.cancel.cancel();
                call.token.cancel();
            }
            loaded.holders.lock().clear();
        }
        self.loaded.lock().clear();
    }
}
impl Drop for AnalysisHub {
    fn drop(&mut self) {
        self.shutdown();
    }
}
impl AnalysisService for AnalysisHub {
    fn open(&self, project: &ProjectId) -> Result<()> {
        self.hold(project, Holder::View)
    }
    fn close(&self, project: &ProjectId) -> Result<()> {
        self.release(project, &Holder::View)
    }
    fn view(&self, project: &ProjectId) -> Result<AnalysisView> {
        view::snapshot(self.loaded(project)?.as_ref())
    }
    fn call(&self, project: &ProjectId, call: ToolCall, cancel: &CancelFlag) -> Result<ToolReply> {
        calls::call(self, project, call, cancel)
    }
    fn tools(&self, project: &ProjectId) -> Result<Vec<ToolInfo>> {
        catalog::tools(self.loaded(project)?.as_ref())
    }
    fn evidence(&self, project: &ProjectId, id: &EvidenceId) -> Result<serde_json::Value> {
        let loaded = self.loaded(project)?;
        let ev = loaded
            .engine
            .session()
            .resolve_evidence(id.as_str())
            .map_err(|e| errors::convert("core_evidence_browse", e))?;
        serde_json::to_value(ev).map_err(|_| Error::corrupt("无法读取证据"))
    }
    fn set_mode(&self, project: &ProjectId, mode: AnalysisMode) -> Result<()> {
        engines::set_mode(self, project, mode)
    }
    fn busy(&self, project: &ProjectId) -> Option<String> {
        self.loaded
            .lock()
            .get(project)
            .and_then(|l| l.calls.lock().values().next().map(|c| c.tool.clone()))
    }
    fn settings(&self) -> Result<AnalysisSettings> {
        settings::snapshot(&self.settings.lock())
    }
    fn set_settings(&self, change: AnalysisSettingsChange) -> Result<AnalysisSettings> {
        settings::update(self, change)
    }
    fn install_engines(&self, params: serde_json::Value, cancel: &CancelFlag) -> Result<ToolReply> {
        install::engines(self, params, cancel)
    }
    fn plugin_checks(&self, project: &ProjectId, kind: &str) -> Result<Vec<PluginCheckInfo>> {
        let loaded = self.loaded(project)?;
        let checks = ca_engine::wasm_plugins::checks(&loaded.engine.session());
        Ok(checks
            .into_iter()
            .filter(|check| check.kinds.is_empty() || check.kinds.iter().any(|value| value == kind))
            .map(|check| PluginCheckInfo {
                id: check.id.to_owned(),
                title: check.title.clone(),
                extensions: check.extensions.clone(),
            })
            .collect())
    }
    fn run_plugin_checks(
        &self,
        project: &ProjectId,
        kind: &str,
        _round: u32,
        context: serde_json::Value,
        cancel: &CancelFlag,
    ) -> Result<Vec<PluginCheckResult>> {
        let loaded = self.loaded(project)?;
        let checks = ca_engine::wasm_plugins::checks(&loaded.engine.session());
        let mut results = Vec::new();
        for check in checks
            .into_iter()
            .filter(|check| check.kinds.is_empty() || check.kinds.iter().any(|value| value == kind))
        {
            if cancel.is_cancelled() {
                return Err(Error::new(ErrorKind::Cancelled, "插件检查已取消"));
            }
            let check_context = plugins::context(&context, &check.extensions);
            let value = match check.run(&check_context) {
                Ok(value) => value,
                Err(error) if error.code == ca_evidence::ErrorCode::Cancelled => {
                    return Err(Error::new(ErrorKind::Cancelled, "插件检查已取消"));
                }
                Err(_) => {
                    results.push(PluginCheckResult {
                        id: check.id.to_owned(),
                        title: check.title.clone(),
                        passed: false,
                        similarity: None,
                        note: "分析插件检查执行失败".into(),
                    });
                    continue;
                }
            };
            let Some(passed) = value.get("passed").and_then(serde_json::Value::as_bool) else {
                results.push(PluginCheckResult {
                    id: check.id.to_owned(),
                    title: check.title.clone(),
                    passed: false,
                    similarity: None,
                    note: "分析插件检查返回了无效结果".into(),
                });
                continue;
            };
            let similarity = match value.get("similarity") {
                None | Some(serde_json::Value::Null) => None,
                Some(value) => match value
                    .as_f64()
                    .filter(|value| value.is_finite() && (0.0..=100.0).contains(value))
                {
                    Some(value) => Some(value),
                    None => {
                        results.push(PluginCheckResult {
                            id: check.id.to_owned(),
                            title: check.title.clone(),
                            passed: false,
                            similarity: None,
                            note: "分析插件检查返回了无效相似度".into(),
                        });
                        continue;
                    }
                },
            };
            let note = value
                .get("summary")
                .and_then(serde_json::Value::as_str)
                .filter(|text| !text.trim().is_empty())
                .unwrap_or(if passed {
                    "分析插件检查通过"
                } else {
                    "分析插件检查未通过"
                })
                .to_owned();
            results.push(PluginCheckResult {
                id: check.id.to_owned(),
                title: check.title.clone(),
                passed,
                similarity,
                note,
            });
        }
        Ok(results)
    }
}
