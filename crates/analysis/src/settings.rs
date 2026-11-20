//! Isolated upstream settings persistence and public settings projection.
use crate::{AnalysisHub, errors::convert};
use autocraft_core::*;
use ca_engine::Settings;
use std::path::Path;

pub(crate) fn load(data_dir: &Path) -> Settings {
    let mut settings = Settings::load_from(&data_dir.join("analysis/config.toml"));
    settings.projects_dir.get_or_insert_with(|| {
        ca_engine::workspace::default_projects_dir()
            .parent()
            .unwrap_or(Path::new("."))
            .join("autocraft")
    });
    settings
        .engines
        .store_dir
        .get_or_insert_with(|| data_dir.join("engines"));
    settings
}

pub(crate) fn snapshot(settings: &Settings) -> Result<AnalysisSettings> {
    let mut tools = settings.engines.tools.clone();
    Ok(AnalysisSettings {
        deep_mode: settings.deep_mode,
        auto_mode: settings.auto_mode,
        ask_before_running: settings.ask_before_running,
        questions: crate::project_dto::questions(settings.questions),
        runtime_study: settings.runtime_study,
        engines: AnalysisEngineSettings {
            ghidra: transcode(&settings.engines.ghidra)?,
            tools: tools
                .entries_mut()
                .into_iter()
                .filter_map(|(id, path)| path.clone().map(|p| (id.to_owned(), p)))
                .collect(),
            store_dir: settings.engines.store_dir.clone(),
        },
        build: transcode(&settings.build)?,
        assets: transcode(&settings.assets)?,
    })
}

pub(crate) fn transcode<T: serde::Serialize, U: serde::de::DeserializeOwned>(
    value: &T,
) -> Result<U> {
    serde_json::to_value(value)
        .and_then(serde_json::from_value)
        .map_err(|_| Error::corrupt("分析数据格式不兼容"))
}

pub(crate) fn update(
    hub: &AnalysisHub,
    change: AnalysisSettingsChange,
) -> Result<AnalysisSettings> {
    let mut guard = hub.settings.lock();
    if guard.version > ca_engine::settings::SETTINGS_VERSION {
        return Err(Error::new(
            ErrorKind::ReadOnly,
            "设置来自更新版本，不能修改",
        ));
    }
    let mut next = guard.clone();
    if let Some(v) = change.deep_mode {
        next.deep_mode = v;
    }
    if let Some(v) = change.auto_mode {
        next.auto_mode = v;
    }
    if let Some(v) = change.ask_before_running {
        next.ask_before_running = v;
    }
    if let Some(v) = change.questions {
        next.questions = crate::project_dto::source_questions(v);
    }
    if let Some(v) = change.runtime_study {
        next.runtime_study = v;
    }
    if let Some(v) = change.engines {
        next.engines.ghidra = transcode(&v.ghidra)?;
        let known: Vec<_> = next
            .engines
            .tools
            .entries_mut()
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        if v.tools.keys().any(|id| !known.contains(&id.as_str())) {
            return Err(Error::invalid("未知的分析工具"));
        }
        for (id, path) in next.engines.tools.entries_mut() {
            *path = v.tools.get(id).cloned();
        }
        next.engines.store_dir = Some(v.store_dir.unwrap_or_else(|| hub.data_dir.join("engines")));
    }
    if let Some(v) = change.build {
        if !v.good_enough.is_finite()
            || !(0.0..=100.0).contains(&v.good_enough)
            || v.parallel == 0
            || v.web_widths.is_empty()
            || v.web_widths.len() > 4
            || v.web_widths.iter().any(|w| !(320..=3840).contains(w))
            || v.picture_good_enough
                .is_some_and(|p| !p.is_finite() || !(0.0..=100.0).contains(&p))
        {
            return Err(Error::invalid("构建检查设置无效"));
        }
        next.build = transcode(&v)?;
    }
    if let Some(v) = change.assets {
        if !v.budget_usd.is_finite() || v.budget_usd < 0.0 {
            return Err(Error::invalid("素材预算无效"));
        }
        next.assets = transcode(&v)?;
    }
    next.save_to(&hub.settings_path())
        .map_err(|e| convert("settings", e))?;
    let result = snapshot(&next)?;
    *guard = next.clone();
    drop(guard);
    let loaded: Vec<_> = hub
        .loaded
        .lock()
        .iter()
        .map(|(id, loaded)| (id.clone(), loaded.clone()))
        .collect();
    for (id, loaded) in loaded {
        let ws = crate::project_dto::workspace(&id)?;
        let plugin_changed = loaded
            .engine
            .try_session()
            .is_some_and(|session| session.settings.plugins != next.plugins);
        if plugin_changed && loaded.holders.lock().is_empty() && loaded.calls.lock().is_empty() {
            // Runtime plugin clients/routes belong to this Engine. Dropping an idle entry
            // tears them down; the next request installs the changed registry.
            hub.loaded.lock().remove(&id);
            continue;
        }
        *loaded.pending_settings.lock() = Some(next.clone());
        loaded.apply_settings();
        let mut policy = loaded.policy.lock();
        policy.auto_mode = next.auto_mode;
        policy.questions = next.questions_in(Some(&ws.manifest));
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn isolated_defaults_and_newer_settings_are_preserved() {
        let dir = tempfile::tempdir().unwrap();
        let settings = load(dir.path());
        assert_eq!(settings.engines.store_dir, Some(dir.path().join("engines")));
        let path = dir.path().join("analysis/config.toml");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "version = 999\ndeep_mode = true\n").unwrap();
        let newer = load(dir.path());
        assert!(newer.deep_mode);
        assert!(newer.save_to(&path).is_err());
        assert_eq!(
            std::fs::read_to_string(path).unwrap(),
            "version = 999\ndeep_mode = true\n"
        );
    }
}
