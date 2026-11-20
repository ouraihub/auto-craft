//! Offline regressions for configured WASM plugins through the analysis service.

use super::*;
use autocraft_core::{
    AnalysisMode, AnalysisService, Availability, CallerDto, CancelFlag, Door, Error, ErrorKind,
    NewProject, ProjectId, ProjectLocations, ProjectPrefChange, ProjectPrefService, ProjectPrefs,
    Result, Secret, TargetRefDto, ToolCall, ToolReply,
};
use ca_engine::wasm_plugins::WasmPluginConfig;
use ca_testkit::wasm_plugin::{check_plugin_wat, echo_plugin_wat};
use parking_lot::Mutex;
use serde_json::json;
use std::{collections::HashMap, sync::Arc};

#[derive(Default)]
struct Prefs(Mutex<HashMap<ProjectId, ProjectPrefs>>);

impl ProjectPrefService for Prefs {
    fn get(&self, id: &ProjectId) -> Result<ProjectPrefs> {
        Ok(self
            .0
            .lock()
            .entry(id.clone())
            .or_insert(ProjectPrefs {
                backend: None,
                model: None,
                effort: None,
                mode: AnalysisMode::CleanRoom,
                view: None,
                revision: 0,
            })
            .clone())
    }

    fn set(
        &self,
        id: &ProjectId,
        change: ProjectPrefChange,
        revision: u64,
    ) -> Result<ProjectPrefs> {
        let mut value = self.get(id)?;
        if value.revision != revision {
            return Err(Error::conflict("changed"));
        }
        if let ProjectPrefChange::Mode(mode) = change {
            value.mode = mode;
        }
        value.revision += 1;
        self.0.lock().insert(id.clone(), value.clone());
        Ok(value)
    }

    fn remove(&self, id: &ProjectId) -> Result<()> {
        self.0.lock().remove(id);
        Ok(())
    }

    fn copy(&self, from: &ProjectId, to: &ProjectId) -> Result<()> {
        let value = self.get(from)?;
        self.0.lock().insert(to.clone(), value);
        Ok(())
    }
}

struct Keys;
impl AssetKeySource for Keys {
    fn asset_keys(&self) -> Result<Vec<(String, Secret)>> {
        Ok(Vec::new())
    }
}

struct Sink;
impl AnalysisSink for Sink {
    fn publish(&self, _event: AnalysisEvent) {}
}

#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "isolated fixture setup"
)]
fn hub_with_plugin(
    plugin_id: &str,
    plugin_wat: String,
) -> (tempfile::TempDir, Arc<AnalysisHub>, ProjectId) {
    let root = tempfile::tempdir().unwrap();
    let plugin_path = root.path().join(format!("{plugin_id}.wasm"));
    std::fs::write(&plugin_path, plugin_wat).unwrap();
    std::fs::create_dir_all(root.path().join("analysis")).unwrap();
    let mut settings = ca_engine::Settings::default();
    settings.plugins.wasm = vec![WasmPluginConfig {
        id: plugin_id.into(),
        name: None,
        path: plugin_path,
        enabled: true,
    }];
    settings
        .save_to(&root.path().join("analysis/config.toml"))
        .unwrap();
    let hub = AnalysisHub::try_new(AnalysisConfig::new(
        root.path().to_owned(),
        Arc::new(Prefs::default()),
        Arc::new(Keys),
        Arc::new(Sink),
    ))
    .unwrap();
    hub.set_locations(ProjectLocations {
        default_dir: root.path().join("projects"),
        extra: Vec::new(),
    })
    .unwrap();
    let project = hub
        .create(NewProject {
            name: "Plugin fixture".into(),
            request: "Check plugin runtime".into(),
            target: TargetRefDto::None,
            door: Door::MakeMyOwn,
            folder: None,
            existing: None,
            mode: AnalysisMode::CleanRoom,
            builder: None,
        })
        .unwrap();
    (root, hub, project.id)
}

#[test]
fn configured_wasm_tool_flows_through_analysis_registry_and_clean_room_policy() {
    let (_root, hub, project) = hub_with_plugin("wordtools", echo_plugin_wat(false));
    let tools = hub.tools(&project).unwrap();
    let tool = tools
        .iter()
        .find(|tool| tool.id == "wordtools_echo")
        .unwrap();
    assert_eq!(tool.available, Availability::NeedsDeep);

    let denied = hub.call(
        &project,
        ToolCall {
            tool: "wordtools_echo".into(),
            params: json!({"text": "offline"}),
            caller: CallerDto::Ui,
            consented: true,
        },
        &CancelFlag::new(),
    );
    assert_eq!(denied.unwrap_err().kind(), Some(ErrorKind::DeepRequired));

    hub.set_mode(&project, AnalysisMode::Deep).unwrap();
    let reply = hub
        .call(
            &project,
            ToolCall {
                tool: "wordtools_echo".into(),
                params: json!({"text": "offline"}),
                caller: CallerDto::Ui,
                consented: true,
            },
            &CancelFlag::new(),
        )
        .unwrap();
    assert_eq!(reply, ToolReply::Value(json!({"text": "offline"})));
}

#[test]
fn configured_wasm_check_filters_kind_and_preserves_failed_result() {
    let (_root, hub, project) = hub_with_plugin("judge", check_plugin_wat(false));
    let checks = hub.plugin_checks(&project, "game").unwrap();
    assert_eq!(checks[0].id, "judge_looks_fine");
    assert_eq!(checks[0].title, "Looks fine");
    assert_eq!(checks[0].extensions, ["md"]);
    assert!(hub.plugin_checks(&project, "website").unwrap().is_empty());

    let results = hub
        .run_plugin_checks(
            &project,
            "game",
            3,
            json!({"kind": "game", "files": []}),
            &CancelFlag::new(),
        )
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id, "judge_looks_fine");
    assert!(!results[0].passed);
    assert_eq!(results[0].similarity, Some(70.0));

    let cancelled = CancelFlag::new();
    cancelled.cancel();
    assert_eq!(
        hub.run_plugin_checks(&project, "game", 3, json!({}), &cancelled)
            .unwrap_err()
            .kind(),
        Some(ErrorKind::Cancelled)
    );
}

#[test]
fn each_judge_receives_only_its_declared_bodies_and_preserves_listing() {
    let context = json!({"files": [
        {"path": "Player.CS", "size": 15, "text": "class Player {}", "text_status": "complete"},
        {"path": "player.gd", "size": 12, "text": "extends Node", "text_status": "complete"},
        {"path": "scene.tscn", "size": 10, "text_status": "unreadable"},
        {"path": "folder.cs/README", "size": 6, "text": "hidden", "text_status": "complete"}
    ], "truncated": false, "unknown": true});
    let cs = plugins::context(&context, &[".cs".into()]);
    assert_eq!(cs["files"][0]["text"], "class Player {}");
    assert_eq!(cs["files"][1]["path"], "player.gd");
    assert_eq!(cs["files"][1]["size"], 12);
    assert_eq!(cs["files"][1]["text_status"], "not_requested");
    assert!(cs["files"][1].get("text").is_none());
    assert!(cs["files"][3].get("text").is_none());
    let godot = plugins::context(&context, &["GD".into(), "tscn".into()]);
    assert_eq!(godot["files"][1]["text"], "extends Node");
    assert_eq!(godot["files"][2]["text_status"], "unreadable");
    assert_eq!(godot["unknown"], true);
}
