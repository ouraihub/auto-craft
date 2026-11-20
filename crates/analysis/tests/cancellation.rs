//! Cancellation withdraws host inbox waits instead of reporting a successful unanswered question.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "Isolated test fixture assertions"
)]
use autocraft_analysis::{
    AnalysisConfig, AnalysisEvent, AnalysisHub, AnalysisSink, AssetKeySource,
};
use autocraft_core::*;
use parking_lot::Mutex;
use serde_json::json;
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

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
                mode: AnalysisMode::Deep,
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
        let mut p = self.get(id)?;
        if p.revision != revision {
            return Err(Error::conflict("changed"));
        }
        match change {
            ProjectPrefChange::Mode(v) => p.mode = v,
            ProjectPrefChange::Backend(v) => p.backend = v,
            ProjectPrefChange::Model(v) => p.model = v,
            ProjectPrefChange::Effort(v) => p.effort = v,
            ProjectPrefChange::View(v) => p.view = v,
        }
        p.revision += 1;
        self.0.lock().insert(id.clone(), p.clone());
        Ok(p)
    }
    fn copy(&self, from: &ProjectId, to: &ProjectId) -> Result<()> {
        let p = self.get(from)?;
        self.0.lock().insert(to.clone(), p);
        Ok(())
    }
    fn remove(&self, id: &ProjectId) -> Result<()> {
        self.0.lock().remove(id);
        Ok(())
    }
}
struct Dependencies;
impl AnalysisSink for Dependencies {
    fn publish(&self, _event: AnalysisEvent) {}
}
impl AssetKeySource for Dependencies {
    fn asset_keys(&self) -> Result<Vec<(String, Secret)>> {
        Ok(Vec::new())
    }
}
#[test]
fn cancelling_question_withdraws_request_and_clears_busy() {
    let directory = tempfile::tempdir().unwrap();
    let hub = AnalysisHub::try_new(AnalysisConfig::new(
        directory.path().to_owned(),
        Arc::new(Prefs(Mutex::new(HashMap::new()))),
        Arc::new(Dependencies),
        Arc::new(Dependencies),
    ))
    .unwrap();
    hub.set_locations(ProjectLocations {
        default_dir: directory.path().join("projects"),
        extra: Vec::new(),
    })
    .unwrap();
    let id = hub
        .create(NewProject {
            name: "Cancel".into(),
            request: String::new(),
            target: TargetRefDto::None,
            door: Door::MakeMyOwn,
            folder: None,
            existing: None,
            mode: AnalysisMode::Deep,
            builder: None,
        })
        .unwrap()
        .id;
    hub.set_person(&id, PersonChange::Questions(QuestionsDto::Ask))
        .unwrap();
    let cancel = CancelFlag::new();
    let worker_cancel = cancel.clone();
    let worker_hub = hub.clone();
    let worker_id = id.clone();
    let worker = std::thread::spawn(move || {
        worker_hub.call(
            &worker_id,
            ToolCall {
                tool: "core_person_ask".into(),
                params: json!({"question":"Wait for me"}),
                caller: CallerDto::Ui,
                consented: true,
            },
            &worker_cancel,
        )
    });
    let start = Instant::now();
    while hub.pending(&id).unwrap().is_empty() {
        assert!(start.elapsed() < Duration::from_secs(5));
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(hub.busy(&id).as_deref(), Some("core_person_ask"));
    cancel.cancel();
    let error = worker.join().unwrap().unwrap_err();
    assert_eq!(error.kind(), Some(ErrorKind::Cancelled));
    assert!(hub.pending(&id).unwrap().is_empty());
    assert!(hub.busy(&id).is_none());
}
