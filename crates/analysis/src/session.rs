//! Constructs isolated upstream sessions without touching upstream user configuration.
use crate::{AssetKeySource, errors};
use autocraft_core::{AnalysisMode, Result};
use ca_engine::{
    Session, Settings,
    inbox::InboxDir,
    project::{Project, Purpose},
    tools::asset_tools::AssetKeys,
    workspace::Workspace,
};
use std::{path::Path, sync::Arc};

pub(crate) fn build(
    data_dir: &Path,
    ws: &Workspace,
    settings: &Settings,
    mode: AnalysisMode,
    keys: &dyn AssetKeySource,
) -> Result<Session> {
    let mut session = Session::isolated(&data_dir.join("analysis"));
    session.settings = settings.clone();
    session.settings.engines.store_dir = Some(data_dir.join("engines"));
    session.settings.ai.clear();
    let root = ws.investigation_dir();
    if root.exists() || !ws.is_read_only() {
        let project = if root.exists() {
            Project::open(&root)
        } else {
            Project::create(&root, &ws.manifest.name, Purpose::default())
        }
        .map_err(|e| errors::convert("project_open", e))?;
        session
            .open_project(project)
            .map_err(|e| errors::convert("project_open", e))?;
    }
    session.ext.insert(Arc::new(InboxDir(ws.inbox_dir())));
    session.ext.insert(Arc::new(AssetKeys(
        keys.asset_keys()?
            .into_iter()
            .map(|(name, secret)| (name, secret.expose().to_owned()))
            .collect(),
    )));
    session.deep_for_run = Some(mode == AnalysisMode::Deep);
    Ok(session)
}
