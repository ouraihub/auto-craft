use std::{collections::HashMap, path::Path, sync::Arc};

use autocraft_core::{
    AttachRequest, Attached, Error, ErrorCode, GridSize, Result, TerminalService, TerminalSink,
    TerminalTab,
};
use parking_lot::Mutex;
use tracing::{info, warn};
use uuid::Uuid;

use crate::{
    catalog::{Catalog, MAX_TITLE_BYTES, StoredTab, validate_tab},
    runtime::{Runtime, SpawnRequest},
};

/// [`TerminalService`] over portable-pty. Tabs persist in `<data>/terminals.json`; PTYs never do.
pub struct PtyTerminalService {
    catalog: Catalog,
    sink: Arc<dyn TerminalSink>,
    runtimes: Mutex<HashMap<Uuid, Arc<Runtime>>>,
}

impl std::fmt::Debug for PtyTerminalService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PtyTerminalService")
            .field("runtimes", &self.runtimes.lock().len())
            .finish_non_exhaustive()
    }
}

impl PtyTerminalService {
    /// Creates the PTY service with persistent tab storage and an output sink.
    pub fn new(data_dir: &Path, sink: Arc<dyn TerminalSink>) -> Self {
        Self {
            catalog: Catalog::new(data_dir.to_path_buf()),
            sink,
            runtimes: Mutex::new(HashMap::new()),
        }
    }

    /// Live runtime for a tab; a shell that exited on its own is dropped here, never returned.
    fn live(&self, cwd: &Path, tab_key: &str) -> Option<Arc<Runtime>> {
        let mut runtimes = self.runtimes.lock();
        runtimes.retain(|_, runtime| !runtime.is_exited());
        runtimes
            .values()
            .find(|r| r.cwd == cwd && r.tab_key == tab_key)
            .cloned()
    }

    fn runtime(&self, id: &Uuid) -> Result<Arc<Runtime>> {
        self.runtimes
            .lock()
            .get(id)
            .filter(|r| !r.is_exited())
            .cloned()
            .ok_or_else(|| Error::not_found("终端"))
    }
}

impl TerminalService for PtyTerminalService {
    fn tabs(&self, cwd: &Path) -> Result<Vec<TerminalTab>> {
        if !cwd.is_absolute() {
            return Err(Error::invalid("终端工作目录须为绝对路径"));
        }
        let tabs = self.catalog.lock()?.read()?;
        Ok(tabs
            .into_iter()
            .filter(|tab| tab.cwd == cwd)
            .map(|tab| {
                let runtime = self.live(&tab.cwd, &tab.tab_key).map(|r| r.id);
                TerminalTab {
                    cwd: tab.cwd,
                    tab_key: tab.tab_key,
                    title: tab.title,
                    runtime,
                }
            })
            .collect())
    }

    fn reserve(&self, cwd: &Path, tab_key: &str, title: &str) -> Result<TerminalTab> {
        validate_tab(cwd, tab_key)?;
        validate_cwd(cwd)?;
        if title.len() > MAX_TITLE_BYTES {
            return Err(Error::invalid("终端标题过长"));
        }
        let locked = self.catalog.lock()?;
        let mut tabs = locked.read()?;
        let stored = match tabs.iter().find(|t| t.cwd == cwd && t.tab_key == tab_key) {
            Some(existing) => existing.clone(),
            None => {
                let tab = StoredTab {
                    cwd: cwd.to_path_buf(),
                    tab_key: tab_key.into(),
                    title: title.into(),
                };
                tabs.push(tab.clone());
                locked.save(&tabs)?;
                tab
            }
        };
        let runtime = self.live(cwd, tab_key).map(|r| r.id);
        Ok(TerminalTab {
            cwd: stored.cwd,
            tab_key: stored.tab_key,
            title: stored.title,
            runtime,
        })
    }

    fn attach(&self, request: AttachRequest) -> Result<Attached> {
        validate_tab(&request.cwd, &request.tab_key)?;
        validate_cwd(&request.cwd)?;
        crate::runtime::validate_size(request.size)?;
        if let Some(runtime) = self.live(&request.cwd, &request.tab_key) {
            return Ok(runtime.attached(false));
        }
        let locked = self.catalog.lock()?;
        let mut tabs = locked.read()?;
        // Re-check under the catalog lock so two attaches cannot start two shells for one tab.
        if let Some(runtime) = self.live(&request.cwd, &request.tab_key) {
            return Ok(runtime.attached(false));
        }
        let runtime = Arc::new(Runtime::spawn(SpawnRequest {
            cwd: request.cwd.clone(),
            tab_key: request.tab_key.clone(),
            shell: request.shell,
            size: request.size,
            sink: self.sink.clone(),
        })?);
        if !tabs
            .iter()
            .any(|t| t.cwd == request.cwd && t.tab_key == request.tab_key)
        {
            tabs.push(StoredTab {
                cwd: request.cwd,
                tab_key: request.tab_key,
                title: "终端".into(),
            });
            locked.save(&tabs)?;
        }
        self.runtimes.lock().insert(runtime.id, runtime.clone());
        Ok(runtime.attached(true))
    }

    fn write(&self, runtime: &Uuid, data: &str) -> Result<()> {
        self.runtime(runtime)?.write(data)
    }

    fn resize(&self, runtime: &Uuid, size: GridSize) -> Result<()> {
        self.runtime(runtime)?.resize(size)
    }

    fn close(&self, cwd: &Path, tab_key: &str) -> Result<()> {
        validate_tab(cwd, tab_key)?;
        let removed = {
            let mut runtimes = self.runtimes.lock();
            let id = runtimes
                .iter()
                .find(|(_, r)| r.cwd == cwd && r.tab_key == tab_key)
                .map(|(id, _)| *id);
            id.and_then(|id| runtimes.remove(&id))
        };
        if let Some(runtime) = removed {
            runtime.shutdown()?;
        }
        let locked = self.catalog.lock()?;
        let mut tabs = locked.read()?;
        let before = tabs.len();
        tabs.retain(|t| !(t.cwd == cwd && t.tab_key == tab_key));
        if tabs.len() != before {
            locked.save(&tabs)?;
        }
        info!("terminal.tab.closed");
        Ok(())
    }

    fn shutdown(&self) {
        let runtimes: Vec<_> = self.runtimes.lock().drain().map(|(_, r)| r).collect();
        let mut failed = 0;
        let mut code = None;
        for runtime in runtimes {
            if let Err(error) = runtime.shutdown() {
                failed += 1;
                code.get_or_insert(error.code());
            }
        }
        if failed > 0 {
            warn!(count = failed, code, "terminal.runtimes.shutdown_failed");
        }
    }
}

impl Drop for PtyTerminalService {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Canonical directory identity prevents relative paths and aliases from splitting tab groups.
fn validate_cwd(cwd: &Path) -> Result<()> {
    let canonical = cwd
        .canonicalize()
        .map_err(|_| Error::invalid("终端工作目录不存在或无法访问"))?;
    if !cwd.is_absolute() || canonical.as_os_str() != cwd.as_os_str() || !canonical.is_dir() {
        return Err(Error::invalid("终端工作目录须为已存在目录的规范绝对路径"));
    }
    Ok(())
}
