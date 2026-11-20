//! Offline process evidence regressions using disposable files and OS shell children.
//! Windows commands use cwd-relative filenames: Process forwards normal argv escaping, not cmd quoting.
use super::*;

#[test]
fn noisy_timeout_keeps_stable_cause_and_scrubbed_bounded_excerpt() {
    let command = if cfg!(windows) {
        Process::new("cmd.exe").args([
            "/D",
            "/C",
            "for /L %i in (1,1,30) do @echo noise 1>&2 & echo password=hunter2 1>&2 & ping -n 20 127.0.0.1 >nul",
        ])
    } else {
        Process::new("sh").args([
            "-c",
            "i=0; while [ $i -lt 30 ]; do echo noise >&2; i=$((i+1)); done; echo password=hunter2 >&2; sleep 20",
        ])
    };
    let out = run_owned(command.timeout(Duration::from_secs(2))).unwrap();
    assert!(!out.success);
    assert_eq!(out.failure, Some(ca_evidence::ErrorCode::TimedOut));
    let note = diagnostic_paths(&out, []);
    assert!(note.contains("命令超时并已停止。"));
    assert!(note.contains("分类：TimedOut"));
    assert!(note.contains("noise"));
    assert!(!note.contains("hunter2"));
    assert!(note.lines().count() <= 14);
    assert!(note.chars().count() <= 3700);
}

#[test]
fn capped_streams_keep_the_final_actual_error() {
    let d = tempfile::tempdir().unwrap();
    let fixture = d.path().join("build.log");
    let mut text = "ordinary compiler progress\n".repeat(22000);
    text.push_str("FINAL ACTUAL BUILD ERROR\n");
    fs::write(&fixture, text).unwrap();
    let command = if cfg!(windows) {
        Process::new("cmd.exe")
            .args([
                "/D",
                "/C",
                "type build.log & type build.log 1>&2 & exit /b 7",
            ])
            .cwd(d.path())
    } else {
        Process::new("sh").args([
            "-c".to_string(),
            "cat \"$1\"; cat \"$1\" >&2; exit 7".to_string(),
            "checks-test".to_string(),
            fixture.display().to_string(),
        ])
    };
    let out = run_owned(command.timeout(Duration::from_secs(15))).unwrap();
    assert!(!out.success);
    assert_eq!(out.code, Some(7));
    assert_eq!(out.stdout.len(), OUTPUT_CAP);
    assert_eq!(out.stderr.len(), OUTPUT_CAP);
    assert!(out.stdout_truncated && out.stderr_truncated);
    assert!(out.stdout_text().ends_with("FINAL ACTUAL BUILD ERROR\n"));
    assert!(out.stderr_text().ends_with("FINAL ACTUAL BUILD ERROR\n"));
    let note = diagnostic_paths(&out, []);
    assert!(note.contains("FINAL ACTUAL BUILD ERROR"));
    assert!(note.contains("[output truncated]"));
}

#[test]
fn godot_script_error_survives_discarded_output_even_with_zero_exit() {
    let d = tempfile::tempdir().unwrap();
    let fixture = d.path().join("godot.log");
    let mut text = String::from("SCRIPT ERROR: invalid script\n");
    text.push_str(&"ordinary import progress\n".repeat(22000));
    text.push_str("Godot finished\n");
    fs::write(&fixture, text).unwrap();
    let command = if cfg!(windows) {
        Process::new("cmd.exe")
            .args(["/D", "/C", "type godot.log"])
            .cwd(d.path())
    } else {
        Process::new("cat").arg(&fixture)
    };
    let out = run_owned(command.timeout(Duration::from_secs(15))).unwrap();
    assert!(out.success);
    assert!(out.stdout_truncated);
    assert!(!out.stdout_text().contains("SCRIPT ERROR"));
    assert!(
        !out.build_succeeded(true),
        "zero exit must not erase the script failure"
    );
    assert!(out.build_succeeded(false));
    assert!(out.stdout_text().ends_with("Godot finished\n"));
    assert!(diagnostic_paths(&out, []).contains("检测到脚本或引擎错误。"));
}

#[test]
fn diagnostics_redact_paths_controls_and_credentials_without_unbounded_lines() {
    let out = CheckOutput {
        output: Output {
            stderr: format!(
                "E:/private/project/main.rs password=hunter2\u{1b}\n{}",
                "x".repeat(2000)
            )
            .into_bytes(),
            ..Output::default()
        },
        failure: None,
        scripts_failed: false,
    };
    let note = diagnostic_paths(&out, [PathBuf::from("E:/private/project")]);
    assert!(note.contains("<project>/main.rs"));
    assert!(!note.contains("hunter2"));
    assert!(!note.contains("E:/private/project"));
    assert!(!note.contains('\u{1b}'));
    assert!(note.lines().all(|line| line.chars().count() <= 300));
}
