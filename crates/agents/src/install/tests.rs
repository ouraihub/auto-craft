//! Managed installation cleanup and opt-in real download coverage.

use super::*;

#[test]
fn rejected_download_is_removed_and_cleanup_failure_is_not_reported_as_deleted() {
    let dir = tempfile::TempDir::new().unwrap();
    let archive = dir.path().join("runtime.zip");
    fs::write(&archive, "untrusted archive").unwrap();
    reject_archive(&archive, "runtime");
    assert!(!archive.exists());
    fs::create_dir(&archive).unwrap();
    fs::write(archive.join("keep"), "still present").unwrap();
    let error = reject_archive(&archive, "runtime");
    assert!(archive.join("keep").is_file());
    assert!(error.to_string().contains("无法删除"));
    assert!(!error.to_string().contains("已删除"));
    assert!(remove_artifact(&archive, false).is_err());
}

#[test]
fn failed_installer_reports_status_without_exposing_process_output() {
    let mut command = hidden(Path::new(if cfg!(windows) { "cmd.exe" } else { "sh" }));
    if cfg!(windows) {
        command.args(["/d", "/c", "echo secret-install-output & exit /b 7"]);
    } else {
        command.args(["-c", "echo secret-install-output; exit 7"]);
    }
    let error = run(command, "安装测试搭档").unwrap_err().to_string();
    assert!(error.contains('7'));
    assert!(error.contains("安装测试搭档"));
    assert!(!error.contains("secret-install-output"));
}

/// Real download and install of all three engines into a temp dir (network, minutes).
/// `cargo test -p autocraft-agents install::tests::installs_all_engines -- --ignored`
#[test]
#[ignore = "downloads Node, Bun and three npm packages"]
fn installs_all_engines() {
    let dir = tempfile::TempDir::new().unwrap();
    let engines = ManagedEngines::new(dir.path());
    for backend in BackendKind::ALL {
        let installed = engines
            .install(backend)
            .unwrap_or_else(|e| panic!("{}: {e}", backend.label()));
        assert_eq!(installed.source, EngineSource::Managed);
        assert_eq!(installed.version.as_deref(), Some(package(backend).version));
        assert_eq!(
            engines.status(backend).unwrap(),
            Some(installed.clone()),
            "status finds the install"
        );
        let Launch::External { executable, args } = installed.launch;
        let out = hidden(&executable)
            .args(&args)
            .arg("--version")
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&out.stdout).into_owned()
            + &String::from_utf8_lossy(&out.stderr);
        println!(
            "{} --version -> {} {}",
            backend.label(),
            out.status,
            text.trim()
        );
    }
}

#[test]
#[ignore = "downloads Node and the pi package"]
fn uninstall_removes_the_managed_engine_directory() {
    let dir = tempfile::TempDir::new().unwrap();
    let engines = ManagedEngines::new(dir.path());
    engines.install(BackendKind::Pi).unwrap();
    assert!(engines.status(BackendKind::Pi).unwrap().is_some());
    engines.uninstall(BackendKind::Pi).unwrap();
    assert!(engines.status(BackendKind::Pi).unwrap().is_none());
    assert!(!dir.path().join("coding/pi").exists());
    // The shared runtime stays so other engines keep working.
    assert!(dir.path().join("coding/runtimes").exists());
}
