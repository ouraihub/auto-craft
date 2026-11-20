//! Offline consumer regressions with disposable projects and OS shell children only.
use super::*;
use autocraft_core::*;
use ca_engine::workspace::TargetRef;
pub(super) enum UnusedAnalysis<'a> {
    Local,
    Plugins {
        checks: &'a [PluginCheckInfo],
        judge: &'a (dyn Fn(Value) -> Result<Vec<PluginCheckResult>> + Sync),
    },
}
fn unused<T>() -> Result<T> {
    Err(Error::invalid(
        "unexpected analysis operation in local check",
    ))
}
impl AnalysisService for UnusedAnalysis<'_> {
    fn open(&self, _: &ProjectId) -> Result<()> {
        unused()
    }
    fn close(&self, _: &ProjectId) -> Result<()> {
        unused()
    }
    fn view(&self, _: &ProjectId) -> Result<AnalysisView> {
        unused()
    }
    fn call(&self, _: &ProjectId, _: ToolCall, _: &CancelFlag) -> Result<ToolReply> {
        unused()
    }
    fn tools(&self, _: &ProjectId) -> Result<Vec<ToolInfo>> {
        unused()
    }
    fn evidence(&self, _: &ProjectId, _: &EvidenceId) -> Result<Value> {
        unused()
    }
    fn set_mode(&self, _: &ProjectId, _: AnalysisMode) -> Result<()> {
        unused()
    }
    fn busy(&self, _: &ProjectId) -> Option<String> {
        None
    }
    fn settings(&self) -> Result<AnalysisSettings> {
        unused()
    }
    fn set_settings(&self, _: AnalysisSettingsChange) -> Result<AnalysisSettings> {
        unused()
    }
    fn install_engines(&self, _: Value, _: &CancelFlag) -> Result<ToolReply> {
        unused()
    }
    fn run_plugin_checks(
        &self,
        _: &ProjectId,
        _: &str,
        _: u32,
        context: Value,
        _: &CancelFlag,
    ) -> Result<Vec<PluginCheckResult>> {
        match self {
            Self::Local => unused(),
            Self::Plugins { judge, .. } => judge(context),
        }
    }
    fn plugin_checks(&self, _: &ProjectId, _: &str) -> Result<Vec<PluginCheckInfo>> {
        match self {
            Self::Local => Ok(Vec::new()),
            Self::Plugins { checks, .. } => Ok(checks.to_vec()),
        }
    }
}
pub(super) fn with_context(ws: &Workspace, f: impl FnOnce(&mut CheckCx<'_>)) {
    with_analysis(ws, &UnusedAnalysis::Local, f);
}
pub(super) fn with_analysis(
    ws: &Workspace,
    analysis: &dyn AnalysisService,
    f: impl FnOnce(&mut CheckCx<'_>),
) {
    let (settings, project, stop, preview) = (
        Settings::default(),
        ProjectId::from("checks-test"),
        CancelFlag::new(),
        PreviewSlot::new(),
    );
    let mut cx = CheckCx::new(
        ws,
        1,
        false,
        CheckResources {
            settings: &settings,
            analysis,
            project: &project,
            preview: &preview,
            stop: &stop,
        },
    );
    f(&mut cx);
}
#[test]
fn deep_local_checks_never_require_private_evidence_in_clean_room_or_website() {
    let root = tempfile::tempdir().unwrap();
    let mut ws = Workspace::create(root.path(), "modes", "modes", TargetRef::None).unwrap();
    for target in [
        TargetRef::None,
        TargetRef::Website {
            url: "http://127.0.0.1/".into(),
        },
        TargetRef::Local {
            path: root.path().join("original.exe"),
        },
    ] {
        ws.manifest.target = target;
        with_context(&ws, |cx| {
            for id in [CheckId::DeepMechanics, CheckId::DeepCredits] {
                assert!(!make_check(id).applies(cx));
                cx.deep = true;
                assert_eq!(
                    make_check(id).applies(cx),
                    matches!(cx.ws.manifest.target, TargetRef::Local { .. })
                );
                cx.deep = false;
            }
        });
    }
}
#[test]
fn stale_output_is_removed_even_when_no_launch_is_declared() {
    let root = tempfile::tempdir().unwrap();
    let ws = Workspace::create(root.path(), "stale", "stale", TargetRef::None).unwrap();
    let shot = ws.checks_dir().join("round-1/screenshot.png");
    fs::create_dir_all(shot.parent().unwrap()).unwrap();
    fs::write(&shot, b"stale PNG from a previous successful check").unwrap();
    with_context(&ws, |cx| {
        assert!(
            process::option(cx, "--screenshot", &shot, 1024)
                .unwrap()
                .is_err()
        );
        assert!(!shot.exists());
        assert!(!screenshot::ScreenshotCheck.run(cx).unwrap().passed);
    });
}
#[test]
fn process_exit_stderr_and_output_caps_are_preserved() {
    let root = tempfile::tempdir().unwrap();
    let ws = Workspace::create(root.path(), "process", "process", TargetRef::None).unwrap();
    fs::create_dir_all(ws.app_dir()).unwrap();
    let (program, args) = if cfg!(windows) {
        (
            PathBuf::from("cmd.exe"),
            vec![
                "/D".into(),
                "/C".into(),
                "echo failure 1>&2 & exit /b 7".into(),
            ],
        )
    } else {
        (
            PathBuf::from("sh"),
            vec!["-c".into(), "echo failure >&2; exit 7".into()],
        )
    };
    with_context(&ws, |cx| {
        let out = process::run(cx, &program, &args, Duration::from_secs(5)).unwrap();
        assert_eq!(out.code, Some(7));
        assert!(!out.success);
        assert!(process::diagnostic(cx, &out).contains("failure"));
        let missing = process::run(
            cx,
            &root.path().join("missing-command"),
            &[],
            Duration::from_secs(1),
        )
        .unwrap();
        assert!(!missing.success);
        assert!(!process::diagnostic(cx, &missing).is_empty());
    });
}
#[test]
fn process_timeout_and_precancel_are_bounded() {
    let root = tempfile::tempdir().unwrap();
    let ws = Workspace::create(root.path(), "timeout", "timeout", TargetRef::None).unwrap();
    fs::create_dir_all(ws.app_dir()).unwrap();
    let (program, args) = if cfg!(windows) {
        (
            PathBuf::from("cmd.exe"),
            vec!["/D".into(), "/C".into(), "ping -n 20 127.0.0.1 >nul".into()],
        )
    } else {
        (PathBuf::from("sh"), vec!["-c".into(), "sleep 20".into()])
    };
    with_context(&ws, |cx| {
        let started = Instant::now();
        assert!(
            !process::run(cx, &program, &args, Duration::from_millis(150))
                .unwrap()
                .success
        );
        assert!(started.elapsed() < Duration::from_secs(5));
        cx.stop.cancel();
        let error = process::run(cx, &program, &args, Duration::from_secs(5)).unwrap_err();
        assert_eq!(error.kind(), Some(ErrorKind::Cancelled));
    });
}
#[test]
fn process_output_is_capped_without_blocking_a_noisy_child() {
    let root = tempfile::tempdir().unwrap();
    let ws = Workspace::create(root.path(), "noisy", "noisy", TargetRef::None).unwrap();
    fs::create_dir_all(ws.app_dir()).unwrap();
    let (program, args) = if cfg!(windows) {
        (
            PathBuf::from("cmd.exe"),
            vec![
                "/D".into(),
                "/C".into(),
                "for /L %i in (1,1,16000) do @echo 012345678901234567890123456789".into(),
            ],
        )
    } else {
        (PathBuf::from("sh"), vec!["-c".into(), "i=0; while [ $i -lt 16000 ]; do echo 012345678901234567890123456789; i=$((i+1)); done".into()])
    };
    with_context(&ws, |cx| {
        let out = process::run(cx, &program, &args, Duration::from_secs(15)).unwrap();
        assert!(out.success);
        assert_eq!(out.stdout.len(), 256 * 1024);
        assert!(out.stdout_truncated);
    });
}

#[test]
fn screenshot_requires_fresh_decodable_nonblank_output_and_honors_picture_bar() {
    use ca_engine::tools::web_common::{Image, encode_png};
    let root = tempfile::tempdir().unwrap();
    let ws = Workspace::create(root.path(), "draw", "draw", TargetRef::None).unwrap();
    let app = ws.app_dir();
    fs::create_dir_all(&app).unwrap();
    fs::write(app.join("project.godot"), "config_version=5").unwrap();
    let runner = app.join(if cfg!(windows) {
        "fake-editor.cmd"
    } else {
        "fake-editor"
    });
    let script = if cfg!(windows) {
        "@echo off\r\nif not \"%~3\"==\"--\" exit /b 8\r\ncopy /y \"%~dp0fixture.png\" \"%~5\" >nul\r\n"
    } else {
        "#!/bin/sh\n[ \"$3\" = -- ] || exit 8\ncp \"$(dirname \"$0\")/fixture.png\" \"$5\"\n"
    };
    fs::write(&runner, script).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&runner, fs::Permissions::from_mode(0o700)).unwrap();
    }
    let fixture = app.join("fixture.png");
    let mut image = Image {
        width: 64,
        height: 36,
        rgba: vec![255; 64 * 36 * 4],
    };
    with_context(&ws, |cx| {
        cx.editor = Some(runner.clone());
        fs::write(&fixture, b"\x89PNG\r\n\x1a\nmalformed").unwrap();
        assert!(!screenshot::ScreenshotCheck.run(cx).unwrap().passed);
        fs::write(&fixture, encode_png(&image).unwrap()).unwrap();
        assert!(!screenshot::ScreenshotCheck.run(cx).unwrap().passed);
        for (i, p) in image.rgba.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            if i % 64 < 32 {
                p[0] = 0;
                p[1] = 0;
                p[2] = 0;
            }
        }
        fs::write(&fixture, encode_png(&image).unwrap()).unwrap();
        ws.add_references(std::slice::from_ref(&fixture)).unwrap();
        let result = screenshot::ScreenshotCheck.run(cx).unwrap();
        assert!(result.passed);
        assert_eq!(result.similarity, None);
        let mut settings = (*cx.settings).clone();
        settings.build.picture_good_enough = Some(99.0);
        let mut scored = CheckCx::new(
            &ws,
            1,
            false,
            CheckResources {
                settings: &settings,
                analysis: &UnusedAnalysis::Local,
                project: cx.project,
                preview: cx.preview,
                stop: cx.stop,
            },
        );
        scored.editor = Some(runner.clone());
        let result = screenshot::ScreenshotCheck.run(&mut scored).unwrap();
        assert!(result.good(100.0));
        assert!(result.similarity.unwrap() > 99.0);
        // Successful process without a new file cannot reuse the previous valid screenshot.
        fs::write(
            &runner,
            if cfg!(windows) {
                "@exit /b 0\r\n"
            } else {
                "#!/bin/sh\nexit 0\n"
            },
        )
        .unwrap();
        assert!(!screenshot::ScreenshotCheck.run(cx).unwrap().passed);
    });
}

#[test]
fn bounded_read_rejects_oversized_files_and_missing_launch_is_a_failure() {
    let root = tempfile::tempdir().unwrap();
    let output = root.path().join("output.json");
    fs::write(&output, vec![b'x'; 1025]).unwrap();
    assert!(process::read_bounded(&output, 1024).is_err());
    let ws = Workspace::create(root.path(), "missing", "missing", TargetRef::None).unwrap();
    with_context(&ws, |cx| {
        assert!(!self_test::SelfTestCheck.run(cx).unwrap().passed)
    });
}

#[test]
fn game_build_reports_engine_script_errors_and_requires_packaged_output() {
    let root = tempfile::tempdir().unwrap();
    let mut ws = Workspace::create(root.path(), "engine", "engine", TargetRef::None).unwrap();
    let app = ws.app_dir();
    fs::create_dir_all(&app).unwrap();
    let runner = app.join(if cfg!(windows) {
        "engine.cmd"
    } else {
        "engine"
    });
    fs::write(
        &runner,
        if cfg!(windows) {
            "@echo SCRIPT ERROR: broken gameplay\r\n@exit /b 0\r\n"
        } else {
            "#!/bin/sh\necho 'SCRIPT ERROR: broken gameplay'\nexit 0\n"
        },
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&runner, fs::Permissions::from_mode(0o700)).unwrap();
    }
    fs::write(app.join("project.godot"), "config_version=5").unwrap();
    ws.manifest.stack = "Godot".into();
    with_context(&ws, |cx| {
        cx.editor = None;
        assert!(!build::BuildCheck { own: false }.run(cx).unwrap().passed);
        cx.editor = Some(runner.clone());
        let result = build::BuildCheck { own: false }.run(cx).unwrap();
        assert!(!result.passed);
        assert!(result.note.contains("broken gameplay"));
    });
    fs::remove_file(app.join("project.godot")).unwrap();
    fs::create_dir_all(app.join("Assets")).unwrap();
    fs::create_dir_all(app.join("ProjectSettings")).unwrap();
    fs::write(
        &runner,
        if cfg!(windows) {
            "@exit /b 0\r\n"
        } else {
            "#!/bin/sh\nexit 0\n"
        },
    )
    .unwrap();
    ws.manifest.stack = "Unity".into();
    with_context(&ws, |cx| {
        cx.editor = Some(runner.clone());
        assert!(!build::BuildCheck { own: false }.run(cx).unwrap().passed);
        assert!(build::BuildCheck { own: true }.run(cx).unwrap().passed);
    });
    fs::remove_dir(app.join("Assets")).unwrap();
    fs::remove_dir(app.join("ProjectSettings")).unwrap();
    fs::write(app.join("readme.txt"), "not a game project").unwrap();
    with_context(&ws, |cx| {
        assert!(!build::BuildCheck { own: false }.run(cx).unwrap().passed)
    });
}

#[test]
fn cancelling_a_running_check_ends_its_process_group() {
    let root = tempfile::tempdir().unwrap();
    let ws = Workspace::create(root.path(), "cancel", "cancel", TargetRef::None).unwrap();
    fs::create_dir_all(ws.app_dir()).unwrap();
    let (program, args) = if cfg!(windows) {
        (
            PathBuf::from("cmd.exe"),
            vec!["/D".into(), "/C".into(), "ping -n 20 127.0.0.1 >nul".into()],
        )
    } else {
        (
            PathBuf::from("sh"),
            vec!["-c".into(), "sleep 20 & wait".into()],
        )
    };
    with_context(&ws, |cx| {
        let stop = cx.stop.clone();
        let cancel = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            stop.cancel();
        });
        let started = Instant::now();
        let error = process::run(cx, &program, &args, Duration::from_secs(20)).unwrap_err();
        assert_eq!(error.kind(), Some(ErrorKind::Cancelled));
        assert!(started.elapsed() < Duration::from_secs(5));
        cancel.join().unwrap();
    });
}

#[test]
fn selftest_extra_failures_and_stale_reports_fail_the_real_consumer() {
    let root = tempfile::tempdir().unwrap();
    let ws = Workspace::create(root.path(), "play", "play", TargetRef::None).unwrap();
    let app = ws.app_dir();
    fs::create_dir_all(&app).unwrap();
    fs::write(app.join("project.godot"), "config_version=5").unwrap();
    let runner = app.join(if cfg!(windows) {
        "selftest.cmd"
    } else {
        "selftest"
    });
    fs::write(
        &runner,
        if cfg!(windows) {
            "@echo off\r\ncopy /y \"%~dp0fixture.json\" \"%~5\" >nul\r\n"
        } else {
            "#!/bin/sh\ncp \"$(dirname \"$0\")/fixture.json\" \"$5\"\n"
        },
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&runner, fs::Permissions::from_mode(0o700)).unwrap();
    }
    let mut report = serde_json::json!({"version":1,"steps":[{"step":"move","passed":true},{"step":"act","passed":true},{"step":"hud","passed":true},{"step":"save","passed":false,"detail":"save failed"}]});
    fs::write(app.join("fixture.json"), report.to_string()).unwrap();
    with_context(&ws, |cx| {
        cx.editor = Some(runner.clone());
        let result = self_test::SelfTestCheck.run(cx).unwrap();
        assert!(!result.passed);
        assert_eq!(result.claims.len(), 4);
        report["steps"][3]["passed"] = serde_json::json!(true);
        fs::write(app.join("fixture.json"), report.to_string()).unwrap();
        assert!(self_test::SelfTestCheck.run(cx).unwrap().passed);
        fs::write(
            &runner,
            if cfg!(windows) {
                "@exit /b 0\r\n"
            } else {
                "#!/bin/sh\nexit 0\n"
            },
        )
        .unwrap();
        assert!(!self_test::SelfTestCheck.run(cx).unwrap().passed);
    });
}

#[test]
fn console_replay_accepts_matching_nonzero_exit_and_rejects_wrong_output() {
    let root = tempfile::tempdir().unwrap();
    let ws = Workspace::create(root.path(), "console", "console", TargetRef::None).unwrap();
    let packaged = ws.app_dir().join("Build");
    fs::create_dir_all(&packaged).unwrap();
    let source = if cfg!(windows) {
        PathBuf::from(std::env::var_os("SystemRoot").unwrap()).join("System32/cmd.exe")
    } else {
        PathBuf::from("/bin/sh")
    };
    fs::copy(
        source,
        packaged.join(format!("replayed{}", std::env::consts::EXE_SUFFIX)),
    )
    .unwrap();
    let captured = ws.capture_dir().join("runs");
    fs::create_dir_all(&captured).unwrap();
    let args = if cfg!(windows) {
        vec!["/D", "/C", "echo expected message & exit /b 2"]
    } else {
        vec!["-c", "echo expected message; exit 2"]
    };
    let mut record = serde_json::json!({"name":"nonzero","arguments":args,"reason":"exited","exit_code":2,"lines":["expected message"]});
    fs::write(captured.join("case.json"), record.to_string()).unwrap();
    with_context(&ws, |cx| {
        assert!(runs::RunsCheck.run(cx).unwrap().passed);
        record["lines"] = serde_json::json!(["completely unrelated words"]);
        fs::write(captured.join("case.json"), record.to_string()).unwrap();
        let result = runs::RunsCheck.run(cx).unwrap();
        assert!(!result.passed);
        assert!(result.claims[0].passed);
        assert!(!result.claims[1].passed);
    });
}

#[test]
fn mechanics_consumer_does_not_accept_the_sheet_as_implementation_evidence() {
    let root = tempfile::tempdir().unwrap();
    let ws = Workspace::create(root.path(), "mechanics", "mechanics", TargetRef::None).unwrap();
    fs::create_dir_all(ws.app_dir()).unwrap();
    fs::write(
        ws.mechanics_path(),
        r#"{"version":1,"values":[{"name":"walk speed","value":42.75,"source":"observed"}]}"#,
    )
    .unwrap();
    with_context(&ws, |cx| {
        assert!(!mechanics::MechanicsCheck.run(cx).unwrap().passed);
        fs::write(ws.app_dir().join("main.rs"), "const SPEED:f32=42.75;").unwrap();
        assert!(mechanics::MechanicsCheck.run(cx).unwrap().passed);
    });
}
