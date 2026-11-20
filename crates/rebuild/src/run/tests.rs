//! Regressions for mode isolation, interrupted checking and persisted no-progress allowances.
use super::*;
fn request(message: Option<&str>, continuing: bool) -> RunRequest {
    RunRequest {
        message: message.map(str::to_owned),
        shown: None,
        pictures: Vec::new(),
        continuing,
        max_rounds: None,
        backend: None,
    }
}
#[test]
fn mode_switch_never_reuses_capture_as_current_mode_history() {
    let follow = request(Some("change the colors"), false);
    assert!(flow::starts_with_study(&follow, false, true, true));
    assert!(!flow::carrying_on(&follow, false));
    assert!(!flow::starts_with_study(&follow, true, false, true));
    assert!(!flow::starts_with_study(&follow, false, false, true));
}
#[test]
fn paused_check_continues_only_with_history_in_the_same_mode() {
    let resume = request(None, true);
    assert!(flow::carrying_on(&resume, true));
    assert!(!flow::carrying_on(&resume, false));
    assert!(!flow::starts_with_study(&resume, true, false, true));
    assert!(flow::starts_with_study(&resume, true, false, false));
    assert!(flow::starts_with_study(&resume, false, false, true));
}
#[test]
fn plain_retry_keeps_stall_budget_but_external_edit_resets_it() {
    let dir = tempfile::tempdir().unwrap();
    let ws = Workspace::create(
        dir.path(),
        "Stalled",
        "Make an app",
        ca_engine::workspace::TargetRef::None,
    )
    .unwrap();
    std::fs::create_dir_all(ws.app_dir()).unwrap();
    let file = ws.app_dir().join("main.rs");
    std::fs::write(&file, "first").unwrap();
    let token = ca_engine::CancelToken::default();
    let mut progress = ca_engine::workspace::progress::BuildProgress::default();
    let key = ws.app_fingerprint(&token).unwrap();
    progress.before_turn(key.clone());
    progress.after_turn(key.clone());
    progress.after_turn(key);
    progress.save(&ws.root).unwrap();
    let mut resumed = ca_engine::workspace::progress::BuildProgress::load(&ws.root).unwrap();
    assert!(resumed.stalled());
    std::fs::write(file, "a real edit").unwrap();
    resumed.before_turn(ws.app_fingerprint(&token).unwrap());
    assert!(!resumed.stalled());
}

#[test]
fn successful_build_check_keeps_omitted_optional_fields_in_ui_projection() {
    let result = CheckResult::new("build", 1, true, "compiled");
    let projected = super::dto(&result).unwrap();
    assert!(projected.passed);
    assert!(projected.claims.is_empty());
    assert!(projected.pages.is_empty());
    assert!(projected.unknowns.is_empty());
    assert_eq!(projected.good_enough, None);
}
