//! Consumers must preserve the pinned engine's nested pin and quick-exit result semantics.
use super::model::{self, Action, Mode, Outcome};
use autocraft_core::{ErrorKind, ToolReply};
use serde_json::json;

#[test]
fn list_displays_nested_pin_kind_round_size_and_session_running() {
    let list = model::listing(ToolReply::Value(json!({
        "count": 2,
        "copied_bytes": 180,
        "pins": [
            {"project": "fixture-project", "project_name": "Fixture", "folder": "not-used",
             "pin": {"version": 2, "id": 1, "name": "Frozen build", "mode": "frozen",
                     "created_ms": 1, "round": 7, "bytes": 140, "shared_bytes": 0},
             "running": true, "pids": [42]},
            {"project": "fixture-project", "project_name": "Fixture", "folder": "not-used",
             "pin": {"version": 2, "id": 2, "name": "Light build", "mode": "light",
                     "created_ms": 2, "bytes": 40, "shared_bytes": 100},
             "running": false, "pids": []}
        ]
    })))
    .unwrap();
    assert_eq!(list.copied_bytes, 180);
    assert_eq!(list.pins[0].pin.name, "Frozen build");
    assert_eq!(list.pins[0].pin.mode, Mode::Frozen);
    assert_eq!(list.pins[0].pin.round, Some(7));
    assert_eq!(list.pins[0].pin.bytes, 140);
    assert!(list.pins[0].running);
    assert_eq!(list.pins[1].pin.mode, Mode::Light);
    assert_eq!(list.pins[1].pin.shared_bytes, 100);
    assert_eq!(list.pins[1].pin.round, None);
    assert!(!list.pins[1].running);
}

#[test]
fn malformed_list_is_not_reported_as_empty_or_not_running() {
    for value in [
        json!({"count": 1, "pins": [], "copied_bytes": 0}),
        json!({"count": 1, "pins": [{"pin": {"id": 1, "name": "Saved", "mode": "frozen",
            "bytes": 0, "shared_bytes": 0}}], "copied_bytes": 0}),
        json!({"count": 1, "pins": [{"pin": {"id": 1, "name": "Saved", "mode": "unknown",
            "bytes": 0, "shared_bytes": 0}, "running": false}], "copied_bytes": 0}),
    ] {
        let error = model::listing(ToolReply::Value(value)).unwrap_err();
        assert_eq!(error.kind(), Some(ErrorKind::Protocol));
    }
}

#[test]
fn estimate_uses_light_bytes_not_shared_bytes_or_a_zero_fallback() {
    let size = model::estimate(ToolReply::Value(json!({"estimate": {
        "bytes": 1_200_000_000_u64, "files": 22, "light_bytes": 60_000_000
    }})))
    .unwrap();
    assert_eq!(size.bytes, 1_200_000_000);
    assert_eq!(size.light_bytes, 60_000_000);
    assert_eq!(size.files, 22);
    assert!(
        model::estimate(ToolReply::Value(json!({"estimate": {
            "bytes": 100, "files": 1
        }})))
        .is_err()
    );
}

#[test]
fn open_distinguishes_reused_process_quick_exit_and_running() {
    let action = Action::Open {
        pin: 3,
        another: false,
    };
    for (value, expected) in [
        (
            json!({"pin": 3, "already_running": true, "pid": 42, "started": "ignored"}),
            Outcome::AlreadyRunning,
        ),
        (
            json!({"pin": 3, "already_running": false, "pid": 42, "closed_at_once": null}),
            Outcome::Running,
        ),
        (
            json!({"pin": 3, "already_running": false, "pid": 42,
            "closed_at_once": {"code": 0, "after_ms": 10}, "last_output": "private output"}),
            Outcome::Closed,
        ),
    ] {
        assert_eq!(
            model::outcome(&action, ToolReply::Value(value)).unwrap(),
            expected
        );
    }
    assert!(
        model::outcome(
            &action,
            ToolReply::Value(json!({"pin": 3,
        "already_running": false}))
        )
        .is_err()
    );
    assert!(
        model::outcome(
            &action,
            ToolReply::Value(json!({"pin": 3,
        "already_running": false, "closed_at_once": {"code": 9}}))
        )
        .is_err()
    );
}

#[test]
fn mutations_only_report_success_for_the_requested_pin() {
    assert_eq!(
        model::outcome(
            &Action::Stop(4),
            ToolReply::Value(json!({
                "pin": 4, "stopped": 2
            }))
        )
        .unwrap(),
        Outcome::Stopped(2)
    );
    assert_eq!(
        model::outcome(
            &Action::Stop(4),
            ToolReply::Value(json!({
                "pin": 4, "stopped": 0
            }))
        )
        .unwrap(),
        Outcome::Stopped(0)
    );
    assert_eq!(
        model::outcome(
            &Action::Delete(4),
            ToolReply::Value(json!({
                "removed": 4
            }))
        )
        .unwrap(),
        Outcome::Deleted
    );
    assert!(model::outcome(&Action::Delete(4), ToolReply::Value(json!({"removed": 5}))).is_err());
    assert!(
        model::outcome(
            &Action::Stop(4),
            ToolReply::Value(json!({"pin": 5, "stopped": 1}))
        )
        .is_err()
    );
    let rename = Action::Rename {
        pin: 4,
        name: "New".to_owned(),
    };
    assert_eq!(
        model::outcome(
            &rename,
            ToolReply::Value(json!({"pin": {
                "id": 4, "name": "New", "mode": "light", "round": 5, "bytes": 10, "shared_bytes": 30
            }}))
        )
        .unwrap(),
        Outcome::Renamed
    );
}

#[test]
fn legacy_live_pins_remain_visible_and_are_not_called_frozen() {
    let list = model::listing(ToolReply::Value(json!({"count": 1, "copied_bytes": 0,
        "pins": [{"pin": {"id": 9, "name": "Legacy", "mode": "live", "bytes": 0,
            "shared_bytes": 0}, "running": false}]
    })))
    .unwrap();
    assert_eq!(list.pins[0].pin.mode, Mode::Live);
    assert_eq!(list.pins[0].pin.name, "Legacy");
}

#[test]
fn create_and_folder_outcomes_do_not_need_or_display_untrusted_paths() {
    let action = Action::Create {
        mode: Mode::Frozen,
        name: String::new(),
    };
    assert_eq!(
        model::outcome(
            &action,
            ToolReply::Value(json!({"pin": {
        "version": 2, "id": 1, "name": "Build", "mode": "frozen", "created_ms": 1,
        "round": 1, "bytes": 12, "shared_bytes": 0
    }, "folder": "private-path"}))
        )
        .unwrap(),
        Outcome::Created
    );
    assert_eq!(
        model::outcome(
            &Action::Folder(1),
            ToolReply::Value(json!({
                "opened": "private-path"
            }))
        )
        .unwrap(),
        Outcome::Folder
    );
    assert!(model::outcome(&action, ToolReply::Value(json!({"folder": "private-path"}))).is_err());
    assert!(
        model::outcome(
            &Action::Folder(1),
            ToolReply::Value(json!({"opened": true}))
        )
        .is_err()
    );
}
