//! Nonblocking analysis snapshots and the last successfully observed project state.

use std::{io::ErrorKind, time::UNIX_EPOCH};

use autocraft_core::{
    self as core, AnalysisMode, AnalysisView, Confidence, EvidenceSummary, Severity, TargetInfo,
    UnknownDto, UnknownStatus,
};
use ca_engine::{Session, inbox::InboxDir, workspace::Workspace};

use crate::{engines::Loaded, errors::convert};

pub(crate) fn snapshot(loaded: &Loaded) -> core::Result<AnalysisView> {
    loaded.apply_settings();
    let Some(session) = loaded.engine.try_session() else {
        let mut cached = loaded.view.lock().clone();
        cached.stale = true;
        cached.busy = busy(loaded);
        return Ok(cached);
    };
    let workspace = session
        .ext
        .get::<InboxDir>()
        .and_then(|inbox| inbox.0.parent().map(Workspace::open))
        .transpose()
        .map_err(|error| convert("core_session_show", error))?;
    let clean_room = session.clean_room();
    let ledger = session
        .ledger()
        .take(300)
        .map(|evidence| {
            Ok(EvidenceSummary {
                id: evidence.id.clone().into(),
                tool: evidence.operation.clone(),
                summary: evidence.summary.clone(),
                confidence: match evidence.confidence {
                    ca_evidence::Confidence::Observed => Confidence::Observed,
                    ca_evidence::Confidence::Derived => Confidence::Derived,
                    ca_evidence::Confidence::Inferred => Confidence::Inferred,
                },
                created_ms: created_ms(&session, &evidence.id)?,
            })
        })
        .collect::<core::Result<Vec<_>>>()?;
    let targets = session
        .targets()
        .map(|open| {
            let target = &open.target;
            TargetInfo {
                id: target.id.clone().into(),
                name: target.name.clone(),
                kind: target.kind.id().replace('-', "_"),
                path: target.path.clone(),
                sha256: target.sha256.clone(),
                size: target.bytes.len() as u64,
            }
        })
        .collect();
    let unknowns = session.unknowns().map(unknown).collect();
    {
        let mut policy = loaded.policy.lock();
        policy.auto_mode = session.settings.auto_mode;
        policy.clean_room = clean_room;
        policy.questions = session
            .settings
            .questions_in(workspace.as_ref().map(|workspace| &workspace.manifest));
    }
    let busy = busy(loaded);
    let mut cached = loaded.view.lock();
    let next = AnalysisView {
        targets,
        active: session.active_id().cloned().map(Into::into),
        mode: if clean_room {
            AnalysisMode::CleanRoom
        } else {
            AnalysisMode::Deep
        },
        ledger,
        unknowns,
        busy,
        stale: false,
        revision: cached.revision + 1,
    };
    *cached = next.clone();
    Ok(next)
}

fn busy(loaded: &Loaded) -> Option<String> {
    loaded
        .calls
        .lock()
        .values()
        .next()
        .map(|call| call.tool.clone())
}

fn created_ms(session: &Session, id: &str) -> core::Result<u64> {
    let Some(project) = &session.project else {
        return Ok(0);
    };
    // Evidence has no clock field. The immutable store's write time is the only recorded time.
    let shard = id.get(3..5).unwrap_or("00");
    let path = project
        .root
        .join("evidence")
        .join(shard)
        .join(format!("{id}.json"));
    let metadata = match std::fs::metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(0),
        Err(error) => {
            return Err(convert(
                "core_session_show",
                ca_evidence::ToolError::io(path.display(), error),
            ));
        }
    };
    let modified = match metadata.modified() {
        Ok(modified) => modified,
        Err(error) if error.kind() == ErrorKind::Unsupported => return Ok(0),
        Err(error) => {
            return Err(convert(
                "core_session_show",
                ca_evidence::ToolError::io(path.display(), error),
            ));
        }
    };
    Ok(modified
        .duration_since(UNIX_EPOCH)
        .map(|duration| u64::try_from(duration.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0))
}

fn unknown(source: &ca_evidence::Unknown) -> UnknownDto {
    UnknownDto {
        id: source.id.clone(),
        revision: source.revision,
        question: source.question.clone(),
        severity: match source.severity {
            ca_evidence::Severity::Low => Severity::Low,
            ca_evidence::Severity::Medium => Severity::Medium,
            ca_evidence::Severity::High => Severity::High,
            ca_evidence::Severity::Critical => Severity::Critical,
        },
        domain: source.domain.clone(),
        status: match source.status {
            ca_evidence::UnknownStatus::Open => UnknownStatus::Open,
            ca_evidence::UnknownStatus::Investigating => UnknownStatus::Investigating,
            ca_evidence::UnknownStatus::Blocked => UnknownStatus::Blocked,
            ca_evidence::UnknownStatus::Contradicted => UnknownStatus::Contradicted,
            ca_evidence::UnknownStatus::Resolved => UnknownStatus::Resolved,
        },
        probes: source.probes.clone(),
        evidence_for: source
            .evidence_for
            .iter()
            .cloned()
            .map(Into::into)
            .collect(),
        evidence_against: source
            .evidence_against
            .iter()
            .cloned()
            .map(Into::into)
            .collect(),
        resolution: source.resolution.clone(),
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{Arc, mpsc},
        time::Duration,
    };

    use ca_engine::{
        Engine,
        project::{Project, Purpose},
        target::{Target, TargetKind},
        workspace::{Questions, TargetRef},
    };
    use ca_evidence::{Environment, EvidenceBuilder, Subject, Unknown};

    use super::*;
    use crate::engines::ActiveCall;

    #[test]
    fn snapshots_refresh_targets_recent_evidence_unknowns_and_revision() {
        let directory = tempfile::tempdir().unwrap();
        let mut session = Session::isolated(directory.path());
        session.deep_for_run = Some(true);
        session.add_target(Target {
            id: "t_sample".into(),
            path: directory.path().join("sample"),
            name: "Sample".into(),
            sha256: "a".repeat(64),
            kind: TargetKind::Script,
            bytes: Arc::from(&b"hello"[..]),
            raw: None,
        });
        for index in 0..301 {
            session
                .record(
                    EvidenceBuilder::new("sample", Subject::default(), Environment::builtin(false))
                        .result(serde_json::json!(index))
                        .summary(index.to_string())
                        .build()
                        .unwrap(),
                )
                .unwrap();
        }
        let mut question = Unknown::new("What happens?", "ui", ca_evidence::Severity::Critical);
        question.status = ca_evidence::UnknownStatus::Contradicted;
        question.probes.push("Observe the screen".into());
        session.put_unknown(question).unwrap();
        let loaded = Loaded::new(Engine::shared(session, Vec::new()), AnalysisMode::Deep);
        let first = snapshot(&loaded).unwrap();
        assert_eq!(first.active.as_ref().unwrap().as_str(), "t_sample");
        assert_eq!(first.targets[0].size, 5);
        assert_eq!(first.targets[0].kind, "script");
        assert_eq!(first.mode, AnalysisMode::Deep);
        assert_eq!(first.ledger.len(), 300);
        assert_eq!(first.ledger[0].summary, "300");
        assert_eq!(first.ledger[299].summary, "1");
        assert_eq!(first.ledger[0].created_ms, 0);
        assert_eq!(first.unknowns[0].severity, Severity::Critical);
        assert_eq!(first.unknowns[0].status, UnknownStatus::Contradicted);
        assert!(!first.stale);
        assert_eq!(snapshot(&loaded).unwrap().revision, first.revision + 1);
    }

    #[test]
    fn busy_snapshot_returns_cache_without_waiting_and_uses_earliest_call() {
        let directory = tempfile::tempdir().unwrap();
        let loaded = Arc::new(Loaded::new(
            Engine::shared(Session::isolated(directory.path()), Vec::new()),
            AnalysisMode::CleanRoom,
        ));
        let initial = snapshot(&loaded).unwrap();
        for (key, tool) in [(20, "later"), (10, "earlier")] {
            loaded.calls.lock().insert(
                key,
                ActiveCall::new(
                    tool.into(),
                    core::CancelFlag::default(),
                    ca_engine::CancelToken::default(),
                    None,
                ),
            );
        }
        let guard = loaded.engine.session();
        let (sender, receiver) = mpsc::channel();
        let worker_loaded = Arc::clone(&loaded);
        let worker = std::thread::spawn(move || sender.send(snapshot(&worker_loaded)).unwrap());
        let result = receiver.recv_timeout(Duration::from_secs(2));
        drop(guard);
        worker.join().unwrap();
        let cached = result.unwrap().unwrap();
        assert!(cached.stale);
        assert_eq!(cached.revision, initial.revision);
        assert_eq!(cached.targets, initial.targets);
        assert_eq!(cached.busy.as_deref(), Some("earlier"));
        loaded.calls.lock().clear();
        let refreshed = snapshot(&loaded).unwrap();
        assert!(!refreshed.stale);
        assert_eq!(refreshed.busy, None);
        assert_eq!(refreshed.revision, initial.revision + 1);
    }

    #[test]
    fn stored_evidence_time_is_stable_and_policy_reads_current_manifest() {
        let directory = tempfile::tempdir().unwrap();
        let mut workspace =
            Workspace::create(directory.path(), "Project", "", TargetRef::None).unwrap();
        workspace
            .set_person(|manifest| manifest.questions = Some(Questions::Ask))
            .unwrap();
        let mut session = Session::isolated(directory.path());
        session.settings.auto_mode = true;
        session.deep_for_run = Some(true);
        session
            .ext
            .insert(Arc::new(InboxDir(workspace.inbox_dir())));
        session
            .open_project(
                Project::create(
                    &workspace.root.join("investigation"),
                    "Project",
                    Purpose::default(),
                )
                .unwrap(),
            )
            .unwrap();
        let evidence =
            EvidenceBuilder::new("sample", Subject::default(), Environment::builtin(false))
                .build()
                .unwrap();
        session.record(evidence).unwrap();
        let loaded = Loaded::new(Engine::shared(session, Vec::new()), AnalysisMode::Deep);
        let first = snapshot(&loaded).unwrap();
        assert!(first.ledger[0].created_ms > 0);
        assert!(loaded.policy.lock().auto_mode);
        assert!(!loaded.policy.lock().clean_room);
        assert_eq!(loaded.policy.lock().questions, Questions::Ask);
        workspace
            .set_person(|manifest| manifest.questions = Some(Questions::Decide))
            .unwrap();
        let second = snapshot(&loaded).unwrap();
        assert_eq!(first.ledger[0].created_ms, second.ledger[0].created_ms);
        assert_eq!(loaded.policy.lock().questions, Questions::Decide);
    }
}
