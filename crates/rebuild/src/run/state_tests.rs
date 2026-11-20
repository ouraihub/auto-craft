//! Consumer-visible study, retry budgets, notes, reminders and terminal-state regressions.
use super::state_fixture::Fixture;
use super::*;
use ca_engine::workspace::TargetRef;
use std::sync::atomic::Ordering;

fn request(max_rounds: u32) -> RunRequest {
    RunRequest {
        message: None,
        shown: None,
        pictures: vec![],
        continuing: true,
        max_rounds: Some(max_rounds),
        backend: Some(BackendKind::Dsh),
    }
}

#[test]
fn first_study_then_same_mode_continue_preserves_capture_and_native_history() {
    let mut f = Fixture::new("Make a website", false, true).unwrap();
    f.execute(request(0)).unwrap();
    assert!(f.ws.capture_dir().join("README.md").is_file());
    assert_eq!(f.ws.manifest.status, Status::Ready);
    std::fs::write(f.ws.capture_dir().join("README.md"), "retained study").unwrap();
    f.execute(request(0)).unwrap();
    assert_eq!(
        std::fs::read_to_string(f.ws.capture_dir().join("README.md")).unwrap(),
        "retained study"
    );
    assert_eq!(f.ws.manifest.agent_session.as_deref(), Some("state-native"));
    assert_eq!(f.boundary.prompts.load(Ordering::SeqCst), 2);
    assert_eq!(
        f.boundary.resumed.lock().unwrap()[1],
        Some(NativeSessionKey::from("state-native"))
    );
    assert!(matches!(
        f.boundary.events.lock().unwrap().last(),
        Some(RebuildEvent::Finished {
            ok: true,
            paused: false,
            ..
        })
    ));
}

#[test]
fn switching_mode_restudies_even_with_capture_and_a_new_message() {
    let mut f = Fixture::new("Make a website", false, true).unwrap();
    f.execute(request(0)).unwrap();
    std::fs::write(f.ws.capture_dir().join("README.md"), "old mode study").unwrap();
    *f.boundary.mode.lock().unwrap() = AnalysisMode::Deep;
    let mut follow = request(0);
    follow.message = Some("Change the layout".into());
    f.execute(follow).unwrap();
    assert_ne!(
        std::fs::read_to_string(f.ws.capture_dir().join("README.md")).unwrap(),
        "old mode study"
    );
    assert_eq!(f.ws.manifest.agent_session_deep, Some(true));
    assert_eq!(f.boundary.resumed.lock().unwrap()[1], None);
    assert_eq!(f.ws.manifest.status, Status::Ready);
}

#[test]
fn same_mode_retry_replaces_partial_study_instead_of_treating_directory_as_complete() {
    let mut f = Fixture::new("Make a website", false, true).unwrap();
    f.execute(request(0)).unwrap();
    // A cancelled/failed study may have created output but never committed README.
    std::fs::remove_file(f.ws.capture_dir().join("README.md")).unwrap();
    let partial = f.ws.capture_dir().join("runs");
    std::fs::create_dir_all(&partial).unwrap();
    std::fs::write(partial.join("no-arguments.json"), b"partial result").unwrap();
    let mut follow = request(0);
    follow.message = Some("Finish the requested layout".into());
    f.execute(follow).unwrap();
    assert!(f.ws.capture_dir().join("README.md").is_file());
    assert!(
        !partial.exists(),
        "partial console records must not disable visual checks"
    );
    assert_eq!(
        f.boundary.resumed.lock().unwrap()[1],
        Some(NativeSessionKey::from("state-native"))
    );
}

#[test]
fn no_target_followups_and_continue_use_existing_history_without_a_capture_directory() {
    let mut f = Fixture::new("Make a website", false, true).unwrap();
    f.ws.manifest.target = TargetRef::None;
    f.execute(request(0)).unwrap();
    assert!(!f.ws.capture_dir().exists());
    let mut follow = request(0);
    follow.message = Some("Change the colors".into());
    f.execute(follow).unwrap();
    f.execute(request(0)).unwrap();
    let prompts = f.boundary.prompt_text.lock().unwrap();
    assert!(prompts[1].starts_with("The person asks: \"Change the colors\""));
    assert!(prompts[2].starts_with("Your last run stopped before it finished"));
    assert!(!f.ws.capture_dir().exists());
    assert_eq!(
        f.boundary.resumed.lock().unwrap()[2],
        Some(NativeSessionKey::from("state-native"))
    );
}

#[test]
fn website_study_writes_completion_record_and_reuses_it_for_same_mode_followup() {
    let mut f = Fixture::new("Make a website", false, true).unwrap();
    f.ws.manifest.target = TargetRef::Website {
        url: "http://127.0.0.1/".into(),
    };
    // This fixture captures study metadata, not comparable screenshots; the build check fails.
    assert_eq!(
        f.execute(request(0)).unwrap_err().kind(),
        Some(ErrorKind::External)
    );
    assert!(f.ws.manifest.checks.iter().any(|check| {
        check.check == ca_engine::workspace::CheckResult::WEB_VISUAL && !check.passed
    }));
    let complete = f.ws.capture_dir().join("README.md");
    assert!(complete.is_file());
    std::fs::write(&complete, "completed website study").unwrap();
    assert_eq!(
        f.execute(request(0)).unwrap_err().kind(),
        Some(ErrorKind::External)
    );
    assert_eq!(
        std::fs::read_to_string(complete).unwrap(),
        "completed website study"
    );
    assert_eq!(f.boundary.prompts.load(Ordering::SeqCst), 2);
    assert_eq!(
        f.boundary.resumed.lock().unwrap()[1],
        Some(NativeSessionKey::from("state-native"))
    );
}

#[test]
fn no_target_mode_switch_still_starts_with_a_full_brief_and_fresh_history() {
    let mut f = Fixture::new("Make a website", false, true).unwrap();
    f.ws.manifest.target = TargetRef::None;
    f.execute(request(0)).unwrap();
    *f.boundary.mode.lock().unwrap() = AnalysisMode::Deep;
    let mut follow = request(0);
    follow.message = Some("Change the colors".into());
    f.execute(follow).unwrap();
    assert_eq!(f.boundary.resumed.lock().unwrap()[1], None);
    assert!(!f.boundary.prompt_text.lock().unwrap()[1].starts_with("The person asks:"));
    assert_eq!(f.ws.manifest.agent_session_deep, Some(true));
    assert!(!f.ws.capture_dir().exists());
}

#[test]
fn switching_agent_starts_its_own_history_instead_of_the_saved_agents_history() {
    let mut f = Fixture::new("Make a website", false, true).unwrap();
    std::fs::create_dir_all(f.ws.capture_dir()).unwrap();
    f.ws.manifest.agent_session = Some("other-agents-native-history".into());
    f.ws.manifest.agent_session_by = Some("pi".into());
    f.ws.manifest.agent_session_deep = Some(false);
    f.execute(request(0)).unwrap();
    assert_eq!(f.ws.manifest.agent_session_by.as_deref(), Some("dsh"));
    assert_eq!(f.ws.manifest.agent_session.as_deref(), Some("state-native"));
    assert_eq!(f.boundary.resumed.lock().unwrap()[0], None);
    assert_eq!(f.ws.manifest.status, Status::Ready);
}

#[test]
fn tenth_unusable_check_stops_before_another_refinement_and_new_requirement_resets_it() {
    let mut f = Fixture::new("Make an app", false, true).unwrap();
    f.ws.manifest.not_working_checks = 9;
    assert_eq!(
        f.execute(request(100)).unwrap_err().kind(),
        Some(ErrorKind::External)
    );
    assert_eq!(f.ws.manifest.not_working_checks, 10);
    assert_eq!(f.ws.manifest.status, Status::NeedsYou);
    assert_eq!(f.boundary.prompts.load(Ordering::SeqCst), 1);
    assert!(f.execute(request(100)).is_err());
    assert_eq!(f.boundary.prompts.load(Ordering::SeqCst), 1);
    let mut follow = request(0);
    follow.message = Some("Use this new requirement".into());
    assert!(f.execute(follow).is_err());
    assert_eq!(f.ws.manifest.not_working_checks, 1);
    assert_eq!(f.boundary.prompts.load(Ordering::SeqCst), 2);
}

#[test]
fn max_rounds_limits_refinements_not_the_initial_build_and_never_marks_failed_checks_ready() {
    for (max_rounds, turns) in [(0, 1), (1, 2)] {
        let mut f = Fixture::new("Make an app", false, true).unwrap();
        assert!(f.execute(request(max_rounds)).is_err());
        assert_eq!(f.boundary.prompts.load(Ordering::SeqCst), turns);
        assert_eq!(
            f.ws.manifest.checks.iter().map(|c| c.round).max(),
            Some(turns as u32)
        );
        assert_eq!(f.ws.manifest.status, Status::NeedsYou);
        assert!(matches!(
            f.boundary.events.lock().unwrap().last(),
            Some(RebuildEvent::Finished {
                ok: false,
                paused: false,
                ..
            })
        ));
        assert!(
            !f.boundary
                .events
                .lock()
                .unwrap()
                .iter()
                .any(|event| matches!(
                    event,
                    RebuildEvent::Step {
                        step: StepDto::Done,
                        ..
                    }
                ))
        );
    }
}

#[test]
fn two_unchanged_turns_stop_without_spending_the_remaining_round_budget() {
    let mut f = Fixture::new("Make a website", false, false).unwrap();
    assert!(f.execute(request(100)).is_err());
    assert_eq!(f.boundary.prompts.load(Ordering::SeqCst), 2);
    assert_eq!(f.ws.manifest.not_working_checks, 2);
    assert_eq!(f.ws.manifest.status, Status::NeedsYou);
    assert!(!f.ws.app_dir().join("index.html").exists());
}

#[test]
fn note_arriving_during_check_returns_to_build_even_when_max_rounds_is_zero() {
    let mut f = Fixture::new("Make a website", false, true).unwrap();
    f.ws.manifest.target = TargetRef::Website {
        url: "http://127.0.0.1/original".into(),
    };
    *f.boundary.note_during_check.lock().unwrap() = Some(Arc::downgrade(&f.run));
    assert!(f.execute(request(0)).is_err());
    assert_eq!(f.boundary.prompts.load(Ordering::SeqCst), 2);
    assert_eq!(f.ws.manifest.not_working_checks, 1);
    assert!(f.run.continuity.lock().unwrap().waiting_notes().is_empty());
    let events = f.boundary.events.lock().unwrap();
    let check = events
        .iter()
        .position(|event| matches!(event, RebuildEvent::Checked { .. }))
        .unwrap();
    let rebuilt = events.iter().skip(check + 1).any(|event| {
        matches!(
            event,
            RebuildEvent::Step {
                step: StepDto::Build,
                ..
            }
        )
    });
    assert!(rebuilt);
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, RebuildEvent::NoteRead { .. }))
            .count(),
        1
    );
    assert_eq!(f.ws.manifest.checks.iter().map(|c| c.round).max(), Some(2));
}

#[test]
fn deep_reminder_is_one_extra_turn_not_an_unbounded_loop() {
    let mut f = Fixture::new("Make an app", true, true).unwrap();
    assert!(f.execute(request(1)).is_err());
    assert!(!f.ws.manifest.decompiled);
    assert_eq!(f.boundary.prompts.load(Ordering::SeqCst), 3);
    assert_eq!(f.ws.manifest.checks.iter().map(|c| c.round).max(), Some(2));
    assert_eq!(f.ws.manifest.status, Status::NeedsYou);
}

#[test]
fn cancellation_wins_over_a_late_success_and_finishes_as_stopped() {
    let f = Fixture::new("Make a website", false, true).unwrap();
    f.run.stop().unwrap();
    f.run.finish(Ok("late result".into())).unwrap();
    let ws = Workspace::open(&f.ws.root).unwrap();
    assert_eq!(ws.manifest.status, Status::Stopped);
    assert!(f.run.state.lock().unwrap().done);
    assert!(matches!(
        f.boundary.events.lock().unwrap().last(),
        Some(RebuildEvent::Finished {
            ok: false,
            paused: true,
            ..
        })
    ));
}
