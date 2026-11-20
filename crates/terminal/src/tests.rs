use std::{
    fs,
    path::PathBuf,
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

use autocraft_core::{
    AttachRequest, ErrorKind, GridSize, Shell, TerminalEvent, TerminalService, TerminalSink,
};

use crate::PtyTerminalService;

struct ChannelSink(async_channel::Sender<TerminalEvent>);

impl TerminalSink for ChannelSink {
    fn publish(&self, event: TerminalEvent) {
        let _ = self.0.send_blocking(event);
    }
}

fn service(
    dir: &std::path::Path,
    capacity: usize,
) -> (PtyTerminalService, async_channel::Receiver<TerminalEvent>) {
    let (tx, rx) = async_channel::bounded(capacity);
    let svc = PtyTerminalService::new(dir, Arc::new(ChannelSink(tx)));
    (svc, rx)
}

#[test]
fn reservations_reload_under_lock_and_reject_corruption() {
    let directory = tempfile::tempdir().unwrap();
    let (first, _a) = service(directory.path(), 8);
    let (second, _b) = service(directory.path(), 8);
    let cwd = directory.path().canonicalize().unwrap();
    first.reserve(&cwd, "a", "A").unwrap();
    second.reserve(&cwd, "b", "B").unwrap();
    first.close(&cwd, "a").unwrap();
    let tabs = second.tabs(&cwd).unwrap();
    assert_eq!(tabs.len(), 1);
    assert_eq!(tabs[0].tab_key, "b");
    assert_eq!(tabs[0].cwd, cwd);
    assert!(tabs[0].runtime.is_none());
    fs::write(directory.path().join("terminals.json"), b"[broken").unwrap();
    let error = first.reserve(&cwd, "c", "C").unwrap_err();
    assert_eq!(error.kind(), Some(ErrorKind::Corrupt));
    assert_eq!(
        fs::read(directory.path().join("terminals.json")).unwrap(),
        b"[broken"
    );
}

#[test]
fn legacy_catalog_reports_unsupported_version_without_generic_parse_error() {
    let directory = tempfile::tempdir().unwrap();
    let (svc, _rx) = service(directory.path(), 8);
    let cwd = directory.path().canonicalize().unwrap();
    fs::write(
        directory.path().join("terminals.json"),
        br#"{"schemaVersion":1,"tabs":[{"workspace":"legacy","tabKey":"t","title":"\\u7ec8\\u7aef 1"}]}"#,
    )
    .unwrap();

    let error = svc.reserve(&cwd, "new", "New").unwrap_err();
    assert_eq!(error.kind(), Some(ErrorKind::Corrupt));
    assert_eq!(error.detail(), "终端配置版本 1 不受支持");
}

#[test]
fn invalid_identifiers_and_sizes_are_rejected() {
    let directory = tempfile::tempdir().unwrap();
    let (svc, _rx) = service(directory.path(), 8);
    let cwd = directory.path().canonicalize().unwrap();
    assert_eq!(
        svc.reserve(&cwd, "", "x").unwrap_err().kind(),
        Some(ErrorKind::Invalid)
    );
    assert_eq!(
        svc.reserve(&cwd, &"k".repeat(501), "x").unwrap_err().kind(),
        Some(ErrorKind::Invalid)
    );
    let zero = AttachRequest {
        cwd,
        tab_key: "t".into(),
        shell: Shell::Cmd,
        size: GridSize { cols: 0, rows: 24 },
    };
    assert_eq!(
        svc.attach(zero).unwrap_err().kind(),
        Some(ErrorKind::Invalid)
    );
}

#[test]
fn attach_and_reserve_require_an_existing_canonical_absolute_directory() {
    let directory = tempfile::tempdir().unwrap();
    let cwd = directory.path().canonicalize().unwrap();
    let (svc, _rx) = service(directory.path(), 8);
    let file = cwd.join("file");
    fs::write(&file, b"not a directory").unwrap();
    fs::create_dir(cwd.join("child")).unwrap();
    // Windows normalizes `..` when joining a verbatim path; construct the raw alias instead.
    let mut alias = cwd.clone().into_os_string();
    alias.push(format!("{0}child{0}..", std::path::MAIN_SEPARATOR));
    for invalid in [
        PathBuf::new(),
        PathBuf::from("relative"),
        cwd.join("missing"),
        file,
        PathBuf::from(alias),
    ] {
        assert_eq!(
            svc.reserve(&invalid, "t", "T").unwrap_err().kind(),
            Some(ErrorKind::Invalid)
        );
        let request = AttachRequest {
            cwd: invalid,
            tab_key: "t".into(),
            shell: Shell::Cmd,
            size: GridSize { cols: 80, rows: 24 },
        };
        assert_eq!(
            svc.attach(request).unwrap_err().kind(),
            Some(ErrorKind::Invalid)
        );
    }
    assert!(svc.tabs(&cwd).unwrap().is_empty());
}

#[test]
fn directory_groups_are_independent_and_removed_directories_can_be_closed() {
    let directory = tempfile::tempdir().unwrap();
    let cwd = directory.path().canonicalize().unwrap();
    let other = cwd.join("other");
    fs::create_dir(&other).unwrap();
    let other = other.canonicalize().unwrap();
    let (svc, _rx) = service(directory.path(), 8);
    svc.reserve(&cwd, "t", "Root").unwrap();
    svc.reserve(&other, "t", "Other").unwrap();
    assert_eq!(svc.tabs(&cwd).unwrap()[0].title, "Root");
    assert_eq!(svc.tabs(&other).unwrap()[0].title, "Other");
    fs::remove_dir(&other).unwrap();
    assert_eq!(svc.tabs(&other).unwrap()[0].cwd, other);
    svc.close(&other, "t").unwrap();
    assert!(svc.tabs(&other).unwrap().is_empty());
    assert_eq!(svc.tabs(&cwd).unwrap()[0].title, "Root");
}

#[cfg(windows)]
#[test]
#[ignore = "Starts disposable real Windows ConPTY cmd.exe processes"]
fn windows_pty_lifecycle_smoke() {
    let directory = tempfile::tempdir().unwrap();
    let (svc, rx) = service(directory.path(), 8);
    let cwd = directory.path().canonicalize().unwrap();
    let request = || AttachRequest {
        cwd: cwd.clone(),
        tab_key: "cmd".into(),
        shell: Shell::Cmd,
        size: GridSize { cols: 80, rows: 24 },
    };
    let attached = svc.attach(request()).unwrap();
    assert!(attached.created);
    let id = attached.runtime;
    let unique = uuid::Uuid::new_v4().simple().to_string();
    let (left, right) = unique.split_at(16);
    let marker = format!("AUTOCRAFT_{left}{right}");
    fs::write(cwd.join(&marker), b"cwd marker").unwrap();
    svc.write(
        &id,
        &format!("set \"A=AUTOCRAFT_{left}\"\rset \"B={right}\"\rif exist %A%%B% echo %A%%B%\r"),
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    let (mut output, mut offset) = (String::new(), 0u64);
    while !output.contains(&marker) {
        assert!(
            Instant::now() < deadline,
            "no shell output marker: {output:?}"
        );
        match rx.try_recv() {
            Ok(TerminalEvent::Data {
                runtime,
                byte_offset,
                data,
            }) => {
                assert_eq!(runtime, id);
                assert_eq!(byte_offset, offset);
                offset += data.len() as u64;
                output.push_str(&data);
            }
            Ok(TerminalEvent::Exit { .. }) => panic!("shell exited early"),
            Err(_) => thread::sleep(Duration::from_millis(5)),
        }
    }
    svc.resize(
        &id,
        GridSize {
            cols: 101,
            rows: 37,
        },
    )
    .unwrap();
    let replay = svc.attach(request()).unwrap();
    assert!(!replay.created);
    assert_eq!(replay.runtime, id);
    assert!(replay.replay.contains(&marker));
    let mut invalid_reattach = request();
    let mut alias = cwd.clone().into_os_string();
    alias.push(format!("{}..", std::path::MAIN_SEPARATOR));
    invalid_reattach.cwd = PathBuf::from(alias);
    assert_eq!(
        svc.attach(invalid_reattach).unwrap_err().kind(),
        Some(ErrorKind::Invalid)
    );
    assert!(replay.replay_end_offset >= offset);
    assert_eq!(
        replay.size,
        GridSize {
            cols: 101,
            rows: 37
        }
    );
    svc.write(&id, "exit 7\r").unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        assert!(Instant::now() < deadline, "no natural exit");
        match rx.try_recv() {
            Ok(TerminalEvent::Exit { runtime, exit_code }) => {
                assert_eq!(runtime, id);
                assert_eq!(exit_code, Some(7));
                break;
            }
            Ok(_) => {}
            Err(_) => thread::sleep(Duration::from_millis(5)),
        }
    }
    thread::sleep(Duration::from_millis(100));
    assert!(svc.tabs(&cwd).unwrap()[0].runtime.is_none());
    assert_eq!(
        svc.write(&id, "echo dead\r").unwrap_err().kind(),
        Some(ErrorKind::NotFound)
    );
    let fresh = svc.attach(request()).unwrap();
    assert_ne!(fresh.runtime, id);
    svc.close(&cwd, "cmd").unwrap();
    assert!(svc.tabs(&cwd).unwrap().is_empty());
    let busy = svc.attach(request()).unwrap();
    svc.write(
        &busy.runtime,
        "for /L %i in (1,1,10000) do @echo BACKPRESSURE_%i\r",
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    while rx.len() < 8 {
        assert!(Instant::now() < deadline, "bounded queue never filled");
        thread::sleep(Duration::from_millis(5));
    }
    let closing = Instant::now();
    svc.shutdown();
    assert!(
        closing.elapsed() < Duration::from_secs(10),
        "shutdown stalled under backpressure"
    );
    let (restarted, _r) = service(directory.path(), 8);
    assert!(restarted.tabs(&cwd).unwrap()[0].runtime.is_none());
}

/// The catalog must not follow a link out of its directory (symlink on Unix, junction on
/// Windows, which needs no administrator rights).
#[test]
fn catalog_refuses_a_linked_catalog_file() {
    let directory = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let link = directory.path().join("terminals.json");
    #[cfg(windows)]
    {
        let status = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&link)
            .arg(outside.path())
            .output()
            .unwrap()
            .status;
        assert!(status.success(), "creating the test junction failed");
    }
    #[cfg(unix)]
    std::os::unix::fs::symlink(outside.path(), &link).unwrap();
    let (svc, _rx) = service(directory.path(), 8);
    let error = svc
        .tabs(&directory.path().canonicalize().unwrap())
        .unwrap_err();
    assert_eq!(error.kind(), Some(ErrorKind::Forbidden));
}
