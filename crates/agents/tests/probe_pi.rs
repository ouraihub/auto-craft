//! P02 real official pi probes. Every model request uses a disposable loopback upstream.
//! AUTOCRAFT_SMOKE_PI=<absolute node.exe>|<absolute official pi dist/bundle/cli.js>.
//! Actual protocol evidence is saved to fixtures/pi (or AUTOCRAFT_PI_PROBE_FIXTURES).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "ignored real-process probe assertions"
)]

use std::{
    env, fs,
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc::{self, Receiver},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use autocraft_testkit::{Engine, Isolation, ScriptedUpstream, Turn};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const EXTENSION: &str = include_str!("fixtures/pi/probe-extension.js");
const TIMEOUT: Duration = Duration::from_secs(40);

fn installation() -> (PathBuf, PathBuf) {
    let value =
        env::var("AUTOCRAFT_SMOKE_PI").expect("set AUTOCRAFT_SMOKE_PI=node.exe|official pi cli.js");
    let mut parts = value.split('|');
    let node = PathBuf::from(parts.next().unwrap());
    let pi = PathBuf::from(parts.next().expect("official pi entry point"));
    assert!(parts.next().is_none());
    assert!(node.is_absolute() && node.is_file());
    assert!(pi.is_absolute() && pi.is_file());
    (node, pi)
}

fn evidence(name: &str, value: &Value) {
    let root = env::var_os("AUTOCRAFT_PI_PROBE_FIXTURES")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pi"));
    fs::create_dir_all(&root).unwrap();
    fs::write(
        root.join(format!("{name}.json")),
        serde_json::to_vec_pretty(value).unwrap(),
    )
    .unwrap();
}

struct Rpc {
    child: Child,
    input: Option<ChildStdin>,
    output: Receiver<Result<Value, String>>,
    reader: Option<JoinHandle<()>>,
    frames: Vec<Value>,
    next_id: u64,
}

impl Rpc {
    fn start(
        sandbox: &Isolation,
        upstream: &ScriptedUpstream,
        session: Option<&Path>,
        readonly: bool,
        approval: bool,
    ) -> Self {
        let (node, pi) = installation();
        let extension = sandbox.support_dir().join("p02-extension.mjs");
        fs::write(&extension, EXTENSION).unwrap();
        fs::write(
            sandbox.config_dir(Engine::Pi).join("settings.json"),
            json!({
                "enableInstallTelemetry": false, "enableAnalytics": false,
                "retry": {"enabled": true, "maxRetries": 1, "baseDelayMs": 25,
                    "provider": {"maxRetries": 0, "timeoutMs": 10000}}
            })
            .to_string(),
        )
        .unwrap();
        let mut command = Command::new(node);
        command
            .arg(pi)
            .args(["--mode", "rpc", "--no-skills", "--no-prompt-templates"])
            .arg("-e")
            .arg(extension)
            .args(["--model", "autocraft-p02-loopback/p02-scripted"])
            .current_dir(sandbox.cwd())
            .env("PI_CODING_AGENT_DIR", sandbox.config_dir(Engine::Pi))
            .env("AUTOCRAFT_P02_BASE_URL", upstream.base_url())
            .env("AUTOCRAFT_P02_KEY", "sk-test-p02-only")
            .env(
                "AUTOCRAFT_P02_TOOLS_REPORT",
                sandbox.root().join("tools.jsonl"),
            )
            .env_remove("AUTOCRAFT_P02_APPROVAL_REPORT")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(fs::File::create(sandbox.root().join("pi-stderr.log")).unwrap());
        if approval {
            command.env(
                "AUTOCRAFT_P02_APPROVAL_REPORT",
                sandbox.root().join("approval.json"),
            );
        }
        if let Some(session) = session {
            command.arg("--session").arg(session);
        }
        if readonly {
            command.args(["--tools", "read,grep,find,ls"]);
        }
        let mut child = command.spawn().expect("launch installed official pi");
        let input = child.stdin.take();
        let stdout = child.stdout.take().unwrap();
        let (sender, output) = mpsc::channel();
        let reader = thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let frame = line.map_err(|error| error.to_string()).and_then(|line| {
                    serde_json::from_str(&line).map_err(|error| error.to_string())
                });
                if sender.send(frame).is_err() {
                    break;
                }
            }
        });
        let mut rpc = Self {
            child,
            input,
            output,
            reader: Some(reader),
            frames: Vec::new(),
            next_id: 0,
        };
        let state = rpc.call("get_state", json!({}));
        assert_eq!(
            state["model"]["id"], "p02-scripted",
            "projected loopback model loaded"
        );
        rpc
    }

    fn send(&mut self, frame: &Value) {
        let input = self.input.as_mut().unwrap();
        writeln!(input, "{frame}").unwrap();
        input.flush().unwrap();
    }

    fn until(&mut self, predicate: impl Fn(&Value) -> bool) -> Value {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let frame = self
                .output
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .unwrap_or_else(|error| {
                    panic!("pi frame deadline/EOF: {error}; frames={:?}", self.frames)
                })
                .expect("valid pi JSONL frame");
            let matched = predicate(&frame);
            self.frames.push(frame.clone());
            if matched {
                return frame;
            }
        }
    }

    fn call(&mut self, kind: &str, mut fields: Value) -> Value {
        self.next_id += 1;
        let id = format!("p02-{}", self.next_id);
        fields["type"] = json!(kind);
        fields["id"] = json!(id);
        self.send(&fields);
        let frame = self.until(|frame| frame["type"] == "response" && frame["id"] == id);
        assert_eq!(frame["success"], true, "RPC {kind}: {frame}");
        frame["data"].clone()
    }

    fn prompt(&mut self, text: &str) {
        self.call("prompt", json!({"message": text}));
    }

    fn end_turn(&mut self) {
        self.until(|frame| frame["type"] == "agent_end");
    }

    fn stop(&mut self) {
        drop(self.input.take());
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                assert!(status.success(), "pi shutdown: {status}");
                break;
            }
            assert!(
                Instant::now() < deadline,
                "pi did not shut down after stdin EOF"
            );
            thread::sleep(Duration::from_millis(10));
        }
        if let Some(reader) = self.reader.take() {
            reader.join().unwrap();
        }
    }
}

impl Drop for Rpc {
    fn drop(&mut self) {
        drop(self.input.take());
        let _kill_result = self.child.kill();
        let _wait_result = self.child.wait();
        if let Some(reader) = self.reader.take() {
            let _join_result = reader.join();
        }
    }
}

fn request_bodies(upstream: &ScriptedUpstream) -> Vec<Value> {
    upstream
        .requests()
        .unwrap()
        .into_iter()
        .map(|request| request.body)
        .collect()
}

fn tool_results(body: &Value) -> Vec<&Value> {
    body["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|message| message["role"] == "tool")
        .collect()
}

#[test]
#[ignore = "needs installed official pi via AUTOCRAFT_SMOKE_PI; local upstream only"]
fn o01_ui_input_cancel_and_block_reason_are_observable() {
    let sandbox = Isolation::new().unwrap();
    let target = sandbox.cwd().join("p02-must-not-be-written.txt");
    let upstream = ScriptedUpstream::start(vec![
        Turn::ToolCall {
            name: "write".into(),
            args: json!({"path": target, "content": "forbidden"}),
        },
        Turn::Text("P02_CANCEL_FINISHED".into()),
    ])
    .unwrap();
    let mut rpc = Rpc::start(&sandbox, &upstream, None, false, true);
    rpc.prompt("Write the requested probe file.");
    let dialog =
        rpc.until(|frame| frame["type"] == "extension_ui_request" && frame["method"] == "input");
    assert_eq!(dialog["title"], "autocraft:approve");
    let action: Value = serde_json::from_str(dialog["placeholder"].as_str().unwrap()).unwrap();
    assert_eq!(action["tool"], "write");
    assert!(!target.exists());
    // A real round-trip while the hook is suspended proves RPC remains responsive without executing write.
    let state = rpc.call("get_state", json!({}));
    assert_eq!(state["isStreaming"], true);
    thread::sleep(Duration::from_millis(250));
    assert!(
        !target.exists(),
        "tool must not execute while ctx.ui.input is pending"
    );
    assert_eq!(
        upstream.requests().unwrap().len(),
        1,
        "model loop must await the approval"
    );
    rpc.send(&json!({"type": "extension_ui_response", "id": dialog["id"], "cancelled": true}));
    rpc.end_turn();
    assert_eq!(
        rpc.call("get_last_assistant_text", json!({}))["text"],
        "P02_CANCEL_FINISHED"
    );
    rpc.stop();
    let report: Value =
        serde_json::from_slice(&fs::read(sandbox.root().join("approval.json")).unwrap()).unwrap();
    assert_eq!(report["returnedUndefined"], true);
    assert!(
        !target.exists(),
        "blocked write must have no filesystem side effect"
    );
    let bodies = request_bodies(&upstream);
    assert_eq!(bodies.len(), 2);
    let results = tool_results(&bodies[1]);
    assert!(
        results.iter().any(|result| result["content"]
            .to_string()
            .contains("P02_APPROVAL_CANCELLED_REASON")),
        "model must receive exact block reason"
    );
    evidence(
        "o01-approval",
        &json!({"成立": true, "方案": "首选 ctx.ui.input", "waiting_state": state, "dialog": dialog, "cancel_report": report, "model_tool_results": results, "frames": rpc.frames}),
    );
    upstream.shutdown().unwrap();
    sandbox.close().unwrap();
}

#[test]
#[ignore = "needs installed official pi via AUTOCRAFT_SMOKE_PI; local upstream only"]
fn o04_http_429_records_retry_frames() {
    let sandbox = Isolation::new().unwrap();
    let upstream = ScriptedUpstream::start(vec![
        Turn::HttpError {
            status: 429,
            retry_after: Some(1),
        },
        Turn::Text("P02_RETRY_RECOVERED".into()),
    ])
    .unwrap();
    let mut rpc = Rpc::start(&sandbox, &upstream, None, false, false);
    rpc.prompt("Reply with one text message.");
    let start = rpc.until(|frame| frame["type"] == "auto_retry_start");
    let end = rpc.until(|frame| frame["type"] == "auto_retry_end");
    assert_eq!(start["attempt"], 1);
    assert_eq!(start["maxAttempts"], 1);
    assert!(start["delayMs"].is_number());
    assert_eq!(end["success"], true);
    assert_eq!(end["attempt"], 1);
    rpc.end_turn();
    assert_eq!(
        rpc.call("get_last_assistant_text", json!({}))["text"],
        "P02_RETRY_RECOVERED"
    );
    rpc.stop();
    let requests = upstream.shutdown().unwrap();
    assert_eq!(
        requests.len(),
        2,
        "agent retry rather than invisible provider retry"
    );
    evidence(
        "o04-retry",
        &json!({"成立": true, "方案": "pi 自动重试", "retry_start": start, "retry_end": end, "actual_request_count": requests.len(), "frames": rpc.frames}),
    );
    sandbox.close().unwrap();
}

fn sha256(path: &Path) -> String {
    Sha256::digest(fs::read(path).unwrap())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[test]
#[ignore = "needs installed official pi via AUTOCRAFT_SMOKE_PI; local upstream only"]
fn o05_native_session_readonly_fork_preserves_source_and_rejects_write_bash() {
    let sandbox = Isolation::new().unwrap();
    let upstream = ScriptedUpstream::start(vec![
        Turn::Text("P02_ORIGINAL_FIRST".into()),
        Turn::Text("P02_ORIGINAL_SECOND".into()),
        Turn::ToolCall {
            name: "write".into(),
            args: json!({"path": "p02-forbidden-write.txt", "content": "forbidden"}),
        },
        Turn::Text("P02_WRITE_REJECTED".into()),
        Turn::ToolCall {
            name: "bash".into(),
            args: json!({"command": "echo forbidden > p02-forbidden-bash.txt"}),
        },
        Turn::Text("P02_BASH_REJECTED".into()),
    ])
    .unwrap();
    let mut original = Rpc::start(&sandbox, &upstream, None, false, false);
    for prompt in [
        "First native history message.",
        "Second native history message.",
    ] {
        original.prompt(prompt);
        original.end_turn();
    }
    let state = original.call("get_state", json!({}));
    let source = PathBuf::from(
        state["sessionFile"]
            .as_str()
            .expect("real persisted native history"),
    );
    original.stop();
    assert!(source.is_file());
    let before = sha256(&source);
    let mut readonly = Rpc::start(&sandbox, &upstream, Some(&source), true, false);
    let entries = readonly.call("get_entries", json!({}));
    let entry = entries["entries"]
        .as_array()
        .unwrap()
        .iter()
        .rev()
        .find(|entry| entry["type"] == "message" && entry["message"]["role"] == "user")
        .expect("actual user history entry to fork");
    let fork = readonly.call("fork", json!({"entryId": entry["id"]}));
    assert_eq!(fork["cancelled"], false);
    let fork_state = readonly.call("get_state", json!({}));
    let fork_path = PathBuf::from(fork_state["sessionFile"].as_str().unwrap());
    assert_ne!(source, fork_path);
    for prompt in [
        "Try writing the probe file.",
        "Try running the probe shell command.",
    ] {
        readonly.prompt(prompt);
        readonly.end_turn();
    }
    readonly.stop();
    assert_eq!(
        before,
        sha256(&source),
        "restore, fork and read-only turns must not mutate builder history"
    );
    assert!(!sandbox.cwd().join("p02-forbidden-write.txt").exists());
    assert!(!sandbox.cwd().join("p02-forbidden-bash.txt").exists());
    let bodies = request_bodies(&upstream);
    assert_eq!(bodies.len(), 6);
    for body in &bodies[2..] {
        let tools = body["tools"]
            .as_array()
            .expect("actual read-only model tool list");
        assert!(
            tools
                .iter()
                .all(|tool| tool["function"]["name"] != "write"
                    && tool["function"]["name"] != "bash")
        );
    }
    let failed_tools: Vec<_> = readonly
        .frames
        .iter()
        .filter(|frame| frame["type"] == "tool_execution_end" && frame["isError"] == true)
        .collect();
    assert!(
        failed_tools
            .iter()
            .any(|frame| frame["toolName"] == "write"),
        "actual model write call must fail"
    );
    assert!(
        failed_tools.iter().any(|frame| frame["toolName"] == "bash"),
        "actual model bash call must fail"
    );
    assert!(!tool_results(&bodies[3]).is_empty());
    assert!(!tool_results(&bodies[5]).is_empty());
    let tools: Vec<Value> = fs::read_to_string(sandbox.root().join("tools.jsonl"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    evidence(
        "o05-readonly-fork",
        &json!({"成立": true, "方案": "只读恢复后立即 fork", "original_sha256_before": before, "original_sha256_after": sha256(&source), "original_state": state, "fork_state": fork_state, "fork_response": fork, "tool_snapshots": tools, "tool_failures": failed_tools, "frames": readonly.frames}),
    );
    upstream.shutdown().unwrap();
    fs::remove_file(fork_path).unwrap();
    sandbox.close().unwrap();
}

#[cfg(windows)]
#[test]
#[ignore = "needs Windows and node executable from AUTOCRAFT_SMOKE_PI"]
fn o14_generic_namespaced_pipe_node_roundtrip() {
    use interprocess::local_socket::{
        GenericNamespaced, ListenerNonblockingMode, ListenerOptions, prelude::*,
    };

    let sandbox = Isolation::new().unwrap();
    let (node, _) = installation();
    let name = format!(
        "autocraft-p02-{}-{}",
        std::process::id(),
        sandbox.root().file_name().unwrap().to_string_lossy()
    );
    let listener = ListenerOptions::new()
        .name(name.clone().to_ns_name::<GenericNamespaced>().unwrap())
        .nonblocking(ListenerNonblockingMode::Both)
        .create_sync()
        .unwrap();
    let wire_path = format!(r"\\.\pipe\{name}");
    let script = r#"const net = require('node:net');
const socket = net.connect(process.argv[1]);
socket.setTimeout(5000, () => { console.error('pipe timeout'); socket.destroy(); process.exitCode = 2; });
socket.on('error', error => { console.error(error.message); process.exitCode = 3; });
socket.on('connect', () => socket.write('P02_PIPE_TOKEN\n' + JSON.stringify({id: 14, method: 'ping'}) + '\n'));
let input = '';
socket.on('data', chunk => {
  input += chunk;
  if (!input.includes('\n')) return;
  const response = JSON.parse(input.slice(0, input.indexOf('\n')));
  if (response.id !== 14 || response.result !== 'P02_PIPE_PONG') { process.exitCode = 4; socket.destroy(); return; }
  console.log(JSON.stringify({connected: true, response}));
  socket.end();
});"#;
    let mut child = Command::new(node)
        .args(["-e", script, &wire_path])
        .current_dir(sandbox.cwd())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(8);
    let mut stream = loop {
        match listener.accept() {
            Ok(stream) => break stream,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                if Instant::now() >= deadline {
                    child.kill().unwrap();
                    child.wait().unwrap();
                    panic!("Node did not connect to GenericNamespaced pipe");
                }
                thread::sleep(Duration::from_millis(5));
            }
            Err(error) => {
                child.kill().unwrap();
                child.wait().unwrap();
                panic!("pipe accept: {error}");
            }
        }
    };
    let mut input = Vec::new();
    while input.iter().filter(|byte| **byte == b'\n').count() < 2 {
        let mut bytes = [0; 1024];
        match stream.read(&mut bytes) {
            Ok(0) => panic!("Node closed pipe before sending token/request"),
            Ok(count) => input.extend_from_slice(&bytes[..count]),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(5))
            }
            Err(error) => panic!("pipe read: {error}"),
        }
        assert!(Instant::now() < deadline, "pipe read deadline");
    }
    let input = String::from_utf8(input).unwrap();
    let mut lines = input.lines();
    assert_eq!(lines.next(), Some("P02_PIPE_TOKEN"));
    let request: Value = serde_json::from_str(lines.next().unwrap()).unwrap();
    assert_eq!(request, json!({"id": 14, "method": "ping"}));
    stream
        .write_all(b"{\"id\":14,\"result\":\"P02_PIPE_PONG\"}\n")
        .unwrap();
    drop(stream);
    while child.try_wait().unwrap().is_none() {
        if Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("Node pipe exit deadline");
        }
        thread::sleep(Duration::from_millis(5));
    }
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "Node pipe error: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let response: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(response["connected"], true);
    evidence(
        "o14-node-pipe",
        &json!({"成立": true, "namespace": "GenericNamespaced", "node_path": wire_path, "received_request": request, "node_output": response}),
    );
    drop(listener);
    sandbox.close().unwrap();
}
