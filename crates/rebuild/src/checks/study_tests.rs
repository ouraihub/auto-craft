//! External-only clean-room observation regressions.
use super::*;
#[test]
fn cleanroom_console_observation_preserves_nonzero_exit_and_visible_output() {
    let root = tempfile::tempdir().unwrap();
    let script = root.path().join(if cfg!(windows) {
        "observe.cmd"
    } else {
        "observe"
    });
    fs::write(
        &script,
        if cfg!(windows) {
            "@echo visible output\r\n@exit /b 3\r\n"
        } else {
            "#!/bin/sh\necho visible output\nexit 3\n"
        },
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&script, fs::Permissions::from_mode(0o700)).unwrap();
    }
    let record = observe_console(&script, &[], "plain", CancelFlag::new()).unwrap();
    assert_eq!(record["reason"], "exited");
    assert_eq!(record["exit_code"], 3);
    assert!(
        record["lines"]
            .as_array()
            .unwrap()
            .iter()
            .any(|l| l.as_str() == Some("visible output"))
    );
}

#[test]
fn window_evidence_wins_without_running_banner_help_or_version_again() {
    // A GUI's successful native capture is authoritative even when the program
    // would print a banner, --help or --version. The nonexistent path also proves
    // that no redundant terminal execution is attempted after the window.
    let result = observe_runtime(
        Path::new("nonexistent-gui-with-banner"),
        CancelFlag::new(),
        || {
            Ok(ToolReply::Value(
                json!({"result":{"title":"GUI banner","width":640,"height":480}}),
            ))
        },
        || panic!("GUI capture must not request extra terminal launches"),
    )
    .unwrap();
    let RuntimeObservation::Window(ToolReply::Value(reply)) = result else {
        panic!("visible GUI must retain window evidence rather than console runs");
    };
    assert_eq!(reply["result"]["title"], "GUI banner");
    assert_eq!(reply["result"]["width"], 640);
}

#[test]
fn denied_or_unavailable_window_observation_never_falls_back_to_execution() {
    for kind in [
        ErrorKind::Forbidden,
        ErrorKind::ConsentRequired,
        ErrorKind::Invalid,
        ErrorKind::Cancelled,
    ] {
        let error = observe_runtime(
            Path::new("nonexistent-target"),
            CancelFlag::new(),
            || Err(Error::new(kind, "capture refused")),
            || panic!("capture failure must not request terminal launches"),
        )
        .err()
        .unwrap();
        assert_eq!(error.kind(), Some(kind));
    }
}

#[test]
fn confirmed_absence_of_window_preserves_console_cases_and_nonzero_exit() {
    let root = tempfile::tempdir().unwrap();
    let script = root.path().join(if cfg!(windows) {
        "console.cmd"
    } else {
        "console"
    });
    fs::write(
        &script,
        if cfg!(windows) {
            "@echo visible console %1\r\n@exit /b 3\r\n"
        } else {
            "#!/bin/sh\necho visible console $1\nexit 3\n"
        },
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&script, fs::Permissions::from_mode(0o700)).unwrap();
    }
    let RuntimeObservation::Console(records) = observe_runtime(
        &script,
        CancelFlag::new(),
        || Err(Error::not_found("no visible window")),
        || Ok(()),
    )
    .unwrap() else {
        panic!("console must retain terminal observations");
    };
    assert_eq!(records.len(), 3);
    for (record, arg) in records.iter().zip(["", "--help", "--version"]) {
        assert_eq!(record["exit_code"], 3);
        assert_eq!(record["reason"], "exited");
        assert!(record["lines"].as_array().unwrap().iter().any(|line| {
            line.as_str()
                .is_some_and(|line| line.trim_end() == format!("visible console {arg}").trim_end())
        }));
    }
}

#[test]
fn console_fallback_requires_its_own_approval_and_preserves_cancellation() {
    for kind in [ErrorKind::ConsentRequired, ErrorKind::Cancelled] {
        let error = observe_runtime(
            Path::new("nonexistent-console"),
            CancelFlag::new(),
            || Err(Error::not_found("no visible window")),
            || Err(Error::new(kind, "console execution not approved")),
        )
        .err()
        .unwrap();
        assert_eq!(error.kind(), Some(kind));
    }
}
