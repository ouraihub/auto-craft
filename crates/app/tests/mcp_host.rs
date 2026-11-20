//! Exercises the shipped executable's authenticated byte-forwarding subcommand.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "integration assertions use isolated child processes"
)]

use autocraft_bridge::Bridge;
use autocraft_core::{ChannelHandler, ChannelOptions, ChatPurpose, ChatScope};
use std::{
    io::{self, BufRead, Read, Write},
    process::{Child, Command, ExitStatus, Stdio},
    sync::{Arc, mpsc},
    thread,
    time::{Duration, Instant},
};

const NOTICE: &[u8] =
    b"{\"jsonrpc\":\"2.0\",\"method\":\"notifications/progress\",\"params\":{\"progress\":1}}\r\n";
const REQUEST: &[u8] = b" {\"jsonrpc\":\"2.0\",\"id\":17,\"method\":\"tools/list\"} \r\n";
const DEADLINE: Duration = Duration::from_secs(15);

struct ChildGuard(Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        match self.0.try_wait() {
            Ok(Some(_)) => {}
            Ok(None) | Err(_) => {
                // Cleanup is best effort, including on assertion unwinding.
                let _kill = self.0.kill();
                let _reap = self.0.wait();
            }
        }
    }
}
fn command() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_autocraft"));
    command
        .arg("mcp-host")
        .env_remove("AUTOCRAFT_PIPE")
        .env_remove("AUTOCRAFT_TOKEN")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}
fn wait(child: &mut ChildGuard) -> ExitStatus {
    let deadline = Instant::now() + DEADLINE;
    loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            return status;
        }
        assert!(
            Instant::now() < deadline,
            "mcp-host did not exit before the deadline"
        );
        thread::sleep(Duration::from_millis(10));
    }
}
fn capture(mut input: impl Read + Send + 'static) -> mpsc::Receiver<io::Result<Vec<u8>>> {
    let (send, receive) = mpsc::channel();
    thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = input.read_to_end(&mut bytes).map(|_| bytes);
        let _delivered = send.send(result);
    });
    receive
}

struct NotifyAndEcho;
impl ChannelHandler for NotifyAndEcho {
    fn serve(
        &self,
        _: &ChatScope,
        _: ChannelOptions,
        mut reader: Box<dyn BufRead + Send>,
        mut writer: Box<dyn Write + Send>,
    ) -> io::Result<()> {
        // A server-originated notification must pass before the client writes any request.
        writer.write_all(NOTICE)?;
        writer.flush()?;
        let mut bytes = vec![0; REQUEST.len()];
        reader.read_exact(&mut bytes)?;
        writer.write_all(&bytes)?;
        writer.flush()
    }
}

#[test]
fn actual_mcp_host_forwards_notifications_and_requests_byte_for_byte() {
    let project = tempfile::tempdir().unwrap();
    let project_id = project
        .path()
        .canonicalize()
        .unwrap()
        .to_string_lossy()
        .into_owned()
        .into();
    let bridge = Bridge::start(Arc::new(NotifyAndEcho)).unwrap();
    let token = bridge.register(
        ChatScope {
            project: project_id,
            chat: "forwarder-test".into(),
            purpose: ChatPurpose::Analysis,
        },
        ChannelOptions::default(),
    );
    let mut command = command();
    command
        .env("AUTOCRAFT_PIPE", bridge.pipe_name())
        .env("AUTOCRAFT_TOKEN", &token);
    let mut child = ChildGuard(command.spawn().unwrap());
    let mut stdin = child.0.stdin.take().unwrap();
    let mut stdout = child.0.stdout.take().unwrap();
    let stderr = capture(child.0.stderr.take().unwrap());
    let (send, notification) = mpsc::channel();
    let output = thread::spawn(move || {
        let mut notice = vec![0; NOTICE.len()];
        let result = stdout.read_exact(&mut notice).map(|_| notice);
        if send.send(result).is_err() {
            return Vec::new();
        }
        let mut rest = Vec::new();
        stdout.read_to_end(&mut rest).unwrap();
        rest
    });
    assert_eq!(
        notification.recv_timeout(DEADLINE).unwrap().unwrap(),
        NOTICE
    );
    stdin.write_all(REQUEST).unwrap();
    stdin.flush().unwrap();
    // Keep stdin open: server EOF must terminate the process rather than join a blocked stdin reader.
    assert_eq!(wait(&mut child).code(), Some(0));
    drop(stdin);
    assert_eq!(output.join().unwrap(), REQUEST);
    assert!(stderr.recv_timeout(DEADLINE).unwrap().unwrap().is_empty());
    bridge.revoke(&token);
}

#[test]
fn actual_mcp_host_missing_credentials_exits_two_without_protocol_output() {
    for supplied in [None, Some("AUTOCRAFT_PIPE"), Some("AUTOCRAFT_TOKEN")] {
        let mut command = command();
        if let Some(name) = supplied {
            command.env(name, "isolated-test-value");
        }
        let mut child = ChildGuard(command.spawn().unwrap());
        let stdout = capture(child.0.stdout.take().unwrap());
        let stderr = capture(child.0.stderr.take().unwrap());
        assert_eq!(wait(&mut child).code(), Some(2));
        assert!(stdout.recv_timeout(DEADLINE).unwrap().unwrap().is_empty());
        let stderr = stderr.recv_timeout(DEADLINE).unwrap().unwrap();
        assert_eq!(
            stderr
                .split(|byte| *byte == b'\n')
                .filter(|line| !line.is_empty())
                .count(),
            1
        );
        assert!(
            !stderr
                .windows(b"isolated-test-value".len())
                .any(|part| part == b"isolated-test-value")
        );
    }
}
