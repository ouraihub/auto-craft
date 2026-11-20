//! Observable serialization and secret-handling contracts.
use crate::*;
#[test]
fn channel_debug_never_reveals_credentials_or_project_paths() {
    let scope = ChatScope {
        project: "private-project-token".into(),
        chat: "chat".into(),
        purpose: ChatPurpose::Builder,
    };
    let access = AnalysisAccess {
        executable: "private-executable".into(),
        pipe: "private-pipe".into(),
        token: "private-launch-token".into(),
        options: ChannelOptions::default(),
    };
    let debug = format!("{scope:?} {access:?}");
    for sensitive in [
        "private-project-token",
        "private-executable",
        "private-pipe",
        "private-launch-token",
    ] {
        assert!(!debug.contains(sensitive));
    }
}
#[test]
fn action_wire_shape_is_stable_without_debugging_arguments() {
    let action = AgentAction::RunCommand {
        command: "private command".into(),
        shell: ActionShell::PowerShell,
    };
    assert_eq!(
        serde_json::to_value(&action).unwrap(),
        serde_json::json!({"RunCommand":{"command":"private command","shell":"PowerShell"}})
    );
    assert!(!format!("{action:?}").contains("private command"));
}
#[test]
fn cancellation_propagates_across_clones() {
    let flag = CancelFlag::new();
    let clone = flag.clone();
    assert!(!clone.is_cancelled());
    flag.cancel();
    assert!(clone.is_cancelled());
}
#[test]
fn notes_are_monotonic_decimal_strings() {
    let first = NoteId::generate();
    let second = NoteId::generate();
    assert!(first.as_str().parse::<u64>().unwrap() < second.as_str().parse::<u64>().unwrap());
    assert_eq!(
        serde_json::to_value(&first).unwrap(),
        serde_json::Value::String(first.to_string())
    );
}
#[test]
fn old_session_errors_remain_non_retryable() {
    let error: SessionError =
        serde_json::from_value(serde_json::json!({"code":"EXTERNAL","message":"busy"})).unwrap();
    assert_eq!(error.retry, None);
    let error = SessionError {
        retry: Some(RetryHint {
            after_secs: Some(5),
        }),
        ..error
    };
    assert_eq!(
        serde_json::from_value::<SessionError>(serde_json::to_value(&error).unwrap()).unwrap(),
        error
    );
}

#[test]
fn saved_build_checks_accept_omitted_empty_source_fields() {
    let check: CheckResultDto = serde_json::from_value(serde_json::json!({
        "round":1,"check":"build","passed":true,"similarity":null,
        "note":"compiled","artifacts":[],"at_ms":1
    }))
    .unwrap();
    assert!(check.claims.is_empty());
    assert!(check.pages.is_empty());
    assert!(check.unknowns.is_empty());
    let claim: ClaimDto =
        serde_json::from_value(serde_json::json!({"title":"build","passed":true})).unwrap();
    assert!(claim.detail.is_empty());
}
