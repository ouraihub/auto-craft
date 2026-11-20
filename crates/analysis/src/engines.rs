//! Shared engine registry, explicit holders, and nonblocking policy cache ownership.
use crate::{AnalysisEvent, AnalysisHub, Holder, session};
use autocraft_core::*;
use ca_engine::{
    CancelToken, Engine,
    workspace::{Questions, Workspace},
};
use parking_lot::Mutex;
use std::{
    collections::{BTreeMap, HashSet},
    sync::{Arc, atomic::Ordering},
};

#[derive(Debug, Clone, Copy)]
pub(crate) struct HostPolicy {
    pub auto_mode: bool,
    pub clean_room: bool,
    pub questions: Questions,
}
pub(crate) struct ActiveCall {
    pub tool: String,
    pub chat: Option<ChatId>,
    pub cancel: CancelFlag,
    pub token: CancelToken,
}
impl ActiveCall {
    pub(crate) fn new(
        tool: String,
        cancel: CancelFlag,
        token: CancelToken,
        chat: Option<ChatId>,
    ) -> Self {
        Self {
            tool,
            chat,
            cancel,
            token,
        }
    }
}
pub(crate) struct Loaded {
    pub engine: Arc<Engine>,
    pub holders: Mutex<HashSet<Holder>>,
    pub view: Mutex<AnalysisView>,
    pub policy: Mutex<HostPolicy>,
    pub calls: Mutex<BTreeMap<u64, ActiveCall>>,
    pub pending_settings: Mutex<Option<ca_engine::Settings>>,
}
impl Loaded {
    pub(crate) fn new(engine: Arc<Engine>, mode: AnalysisMode) -> Self {
        let policy = {
            let s = engine.session();
            HostPolicy {
                auto_mode: s.settings.auto_mode,
                clean_room: s.clean_room(),
                questions: s.settings.questions(),
            }
        };
        Self {
            engine,
            holders: Mutex::new(HashSet::new()),
            policy: Mutex::new(policy),
            calls: Mutex::new(BTreeMap::new()),
            pending_settings: Mutex::new(None),
            view: Mutex::new(AnalysisView {
                targets: Vec::new(),
                active: None,
                mode,
                ledger: Vec::new(),
                unknowns: Vec::new(),
                busy: None,
                stale: false,
                revision: 0,
            }),
        }
    }
    pub(crate) fn apply_settings(&self) {
        if let Some(mut session) = self.engine.try_session()
            && let Some(mut settings) = self.pending_settings.lock().take()
        {
            settings.ai.clear();
            // Runtime plugin routes are owned by this Engine; preserve them while applying
            // non-plugin settings updates.
            settings.plugins = session.settings.plugins.clone();
            session.settings = settings;
        }
    }
}
impl AnalysisHub {
    pub(crate) fn loaded(&self, project: &ProjectId) -> Result<Arc<Loaded>> {
        if self.stopped.load(Ordering::Acquire) {
            return Err(Error::new(ErrorKind::SessionClosed, "分析服务已关闭"));
        }
        if let Some(loaded) = self.loaded.lock().get(project).cloned() {
            return Ok(loaded);
        }
        // Only creation is serialized; the registry mutex never spans filesystem or engine calls.
        let _creating = self.load_gate.lock();
        if let Some(loaded) = self.loaded.lock().get(project).cloned() {
            return Ok(loaded);
        }
        let ws = crate::project_dto::workspace(project)?;
        let mode = self.prefs.get(project)?.mode;
        let mut session = session::build(
            &self.data_dir,
            &ws,
            &self.settings.lock(),
            mode,
            self.asset_keys.as_ref(),
        )?;
        if let Some(init) = &self.session_init {
            init(&mut session);
        }
        let runtime_plugins = ca_engine::plugins::install(&mut session);
        let loaded = Arc::new(Loaded::new(
            Engine::shared_with(session, self.extra_tools.clone(), runtime_plugins),
            mode,
        ));
        crate::view::snapshot(&loaded)?;
        self.loaded.lock().insert(project.clone(), loaded.clone());
        Ok(loaded)
    }
    /// # Errors
    /// Returns failure to load the project or its engine.
    pub fn hold(&self, project: &ProjectId, holder: Holder) -> Result<()> {
        loop {
            let loaded = self.loaded(project)?;
            let table = self.loaded.lock();
            if table
                .get(project)
                .is_some_and(|entry| Arc::ptr_eq(entry, &loaded))
            {
                loaded.holders.lock().insert(holder);
                return Ok(());
            }
        }
    }
    /// # Errors
    /// Returns BUSY while other owners or calls remain, after removing this holder.
    pub fn release(&self, project: &ProjectId, holder: &Holder) -> Result<()> {
        let mut table = self.loaded.lock();
        let Some(loaded) = table.get(project) else {
            return Ok(());
        };
        loaded.holders.lock().remove(holder);
        if !loaded.holders.lock().is_empty() || !loaded.calls.lock().is_empty() {
            return Err(Error::busy("项目仍在使用中"));
        }
        table.remove(project);
        Ok(())
    }
    pub(crate) fn writable(&self, project: &ProjectId) -> Result<Workspace> {
        let ws = crate::project_dto::workspace(project)?;
        if ws.is_read_only() {
            return Err(Error::new(
                ErrorKind::ReadOnly,
                "项目由更新版本创建，只能查看",
            ));
        }
        Ok(ws)
    }
}
pub(crate) fn set_mode(hub: &AnalysisHub, project: &ProjectId, mode: AnalysisMode) -> Result<()> {
    hub.writable(project)?;
    if hub.is_running(project) {
        return Err(Error::busy("复刻运行期间不能切换分析模式"));
    }
    let loaded = hub.loaded(project)?;
    let mut session = loaded
        .engine
        .try_session()
        .ok_or_else(|| Error::busy("工具运行期间不能切换分析模式"))?;
    let prefs = hub.prefs.get(project)?;
    hub.prefs
        .set(project, ProjectPrefChange::Mode(mode), prefs.revision)?;
    session.deep_for_run = Some(mode == AnalysisMode::Deep);
    loaded.policy.lock().clean_room = mode == AnalysisMode::CleanRoom;
    drop(session);
    hub.changed(project);
    hub.sink.publish(AnalysisEvent::ModeChanged {
        project: project.clone(),
    });
    Ok(())
}
