//! P04 real DSH ACP probes. Run with --ignored --nocapture; fixtures retain raw local evidence.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "probe assertions and owned fixtures"
)]

use std::{
    fs,
    io::{BufRead, BufReader, Write},
    path::PathBuf,
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc::{self, Receiver},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use autocraft_testkit::{Engine, Isolation, ScriptedUpstream, Turn};
use serde_json::{Value, json};

struct Acp {
    child: Child,
    stdin: ChildStdin,
    frames: Receiver<Value>,
    transcript: Vec<Value>,
    started: Instant,
    readers: Vec<thread::JoinHandle<()>>,
    stderr: std::sync::Arc<std::sync::Mutex<Vec<u8>>>,
    evidence_name: String,
}

impl Acp {
    fn start(isolation: &Isolation, base_url: &str, evidence_name: &str) -> Self {
        let spec = std::env::var("AUTOCRAFT_SMOKE_DSH")
            .expect("set AUTOCRAFT_SMOKE_DSH to executable|argument|argument");
        let mut parts = spec.split('|');
        let executable = parts.next().expect("DSH executable");
        let args: Vec<_> = parts.collect();
        let patch = plain_path(&isolation.support_dir().join("probe.patch.yml"));
        // Same provider/acp projection shape as agents/src/projection_engine.rs.
        // Preserve the real engine's default retry policy (five retries), not a host retry.
        fs::write(&patch, format!(
            "- id: llm-pi-ai\n  config:\n    providers:\n      probe:\n        api: openai-completions\n        baseURL: {base_url}\n        apiKeyEnv: AUTOCRAFT_PROBE_KEY\n        models:\n          - id: probe-model\n            name: Probe\n            contextWindow: 128000\n            maxTokens: 4096\n            input: [text]\n            reasoningEfforts: false\n- id: acp\n  config:\n    provider: probe\n    model: probe-model\n"
        )).unwrap();
        let mut command = Command::new(executable);
        command
            .args(args)
            .args(["--profile", "acp", "--patch"])
            .arg(patch)
            .current_dir(plain_path(&isolation.config_dir(Engine::Dsh)))
            .env_clear();
        // Only OS/runtime prerequisites survive. No inherited provider keys or user config.
        for name in ["PATH", "SystemRoot", "WINDIR", "TEMP", "TMP", "COMSPEC"] {
            if let Some(value) = std::env::var_os(name) {
                command.env(name, value);
            }
        }
        command
            .env("DSH_HOME", plain_path(&isolation.config_dir(Engine::Dsh)))
            .env("HOME", plain_path(isolation.root()))
            .env("USERPROFILE", plain_path(isolation.root()))
            .env("DSH_AGENTS_HOME", plain_path(&isolation.support_dir()))
            .env("AUTOCRAFT_PROBE_KEY", "sk-test-dsh")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command.spawn().expect("launch real DSH");
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        let (sender, frames) = mpsc::channel();
        let out = thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let line = line.expect("read DSH stdout");
                let frame = serde_json::from_str(&line)
                    .unwrap_or_else(|_| json!({"non_json_stdout": line}));
                if sender.send(frame).is_err() {
                    break;
                }
            }
        });
        let captured = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let capture = captured.clone();
        let err = thread::spawn(move || {
            use std::io::Read as _;
            let mut stderr = BufReader::new(stderr);
            let mut bytes = [0; 4096];
            loop {
                let count = stderr.read(&mut bytes).unwrap();
                if count == 0 {
                    break;
                }
                capture.lock().unwrap().extend_from_slice(&bytes[..count]);
            }
        });
        Self {
            child,
            stdin,
            frames,
            transcript: Vec::new(),
            started: Instant::now(),
            readers: vec![out, err],
            stderr: captured,
            evidence_name: evidence_name.to_owned(),
        }
    }

    fn send(&mut self, frame: Value) {
        serde_json::to_writer(&mut self.stdin, &frame).unwrap();
        self.stdin.write_all(b"\n").unwrap();
        self.stdin.flush().unwrap();
    }

    fn call(&mut self, id: u64, method: &str, params: Value, seconds: u64) -> Value {
        self.send(json!({"jsonrpc":"2.0", "id": id, "method": method, "params": params}));
        let deadline = Instant::now() + Duration::from_secs(seconds);
        loop {
            let frame = self
                .frames
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .unwrap_or_else(|error| {
                    panic!(
                        "DSH {method} did not settle: {error}; frames={}; stderr={}",
                        json!(self.transcript),
                        String::from_utf8_lossy(&self.stderr.lock().unwrap())
                    )
                });
            self.transcript.push(json!({"elapsed_ms":self.started.elapsed().as_millis(), "at_ms":now_ms(), "frame":frame}));
            if frame["method"] == "session/request_permission" {
                let option = frame["params"]["options"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|option| option["kind"] == "allow_once")
                    .expect("allow_once permission option");
                self.send(json!({"jsonrpc":"2.0", "id":frame["id"], "result":{"outcome":{"outcome":"selected", "optionId":option["optionId"]}}}));
            } else if frame["id"] == id {
                return frame;
            }
        }
    }

    fn session(&mut self, isolation: &Isolation, mcp: Vec<Value>) -> String {
        let init = self.call(1, "initialize", json!({"protocolVersion":1,"clientCapabilities":{},"clientInfo":{"name":"autocraft-probe","version":"1"}}), 60);
        assert_eq!(init["result"]["protocolVersion"], 1, "{init}");
        let reply = self.call(
            2,
            "session/new",
            json!({"cwd":plain_path(&isolation.cwd()),"mcpServers":mcp}),
            60,
        );
        reply["result"]["sessionId"]
            .as_str()
            .unwrap_or_else(|| panic!("session/new: {reply}"))
            .to_owned()
    }

    fn prompt(&mut self, session: &str, seconds: u64) -> Value {
        self.call(3, "session/prompt", json!({"sessionId":session,"prompt":[{"type":"text","text":"Execute the scripted request and report its result."}]}), seconds)
    }

    fn stop(&mut self) {
        if self.readers.is_empty() {
            return;
        }
        if self.child.try_wait().expect("query DSH process").is_none() {
            self.child.kill().expect("kill DSH process");
        }
        self.child.wait().expect("reap DSH process");
        for reader in self.readers.drain(..) {
            reader.join().expect("join DSH reader");
        }
        for frame in self.frames.try_iter() {
            self.transcript
                .push(json!({"received_after_stop":true,"frame":frame}));
        }
        // Synthetic local upstream only; preserve fields and stderr verbatim, including on panic.
        evidence(
            &format!("{}.acp", self.evidence_name),
            &json!(self.transcript),
        );
        fs::write(
            evidence_root().join(format!("{}.stderr.log", self.evidence_name)),
            &*self.stderr.lock().unwrap(),
        )
        .unwrap();
    }
}

impl Drop for Acp {
    fn drop(&mut self) {
        self.stop();
    }
}

fn evidence_root() -> PathBuf {
    let root = std::env::var_os("AUTOCRAFT_DSH_PROBE_FIXTURES")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/dsh"));
    fs::create_dir_all(&root).unwrap();
    root
}

fn evidence(name: &str, value: &Value) {
    fs::write(
        evidence_root().join(format!("{name}.json")),
        serde_json::to_string_pretty(value).unwrap() + "\n",
    )
    .unwrap();
}

fn model_requests(requests: &[autocraft_testkit::RecordedRequest]) -> Vec<Value> {
    // Headers are unnecessary evidence and deliberately excluded, even for fake credentials.
    requests
        .iter()
        .map(|request| json!({"method":request.method,"path":request.path,"body":request.body}))
        .collect()
}

fn assistant_text(acp: &Acp) -> String {
    acp.transcript
        .iter()
        .filter_map(|entry| {
            let update = &entry["frame"]["params"]["update"];
            (update["sessionUpdate"] == "agent_message_chunk")
                .then(|| update["content"]["text"].as_str())
                .flatten()
        })
        .collect()
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis()
}

fn plain_path(path: &std::path::Path) -> std::path::PathBuf {
    let text = path.to_string_lossy();
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        std::path::PathBuf::from(format!(r"\\{rest}"))
    } else {
        std::path::PathBuf::from(text.strip_prefix(r"\\?\").unwrap_or(&text))
    }
}

#[test]
#[ignore = "needs dsh"]
fn o04_upstream_errors() {
    for status in [429, 500] {
        let isolation = Isolation::new().unwrap();
        // Initial request plus the real provider's five default retries.
        let upstream = ScriptedUpstream::start(
            (0..6)
                .map(|_| Turn::HttpError {
                    status,
                    retry_after: Some(1),
                })
                .collect(),
        )
        .unwrap();
        let name = format!("o04-http-{status}");
        let mut acp = Acp::start(&isolation, &upstream.base_url(), &name);
        let session = acp.session(&isolation, vec![]);
        let response = acp.prompt(&session, 90);
        acp.stop();
        let requests = upstream.requests().unwrap();
        evidence(
            &name,
            &json!({"probe":"DSH.O04","http_status_script":status,"retry_after_seconds_script":1,"response":response,"requests":model_requests(&requests)}),
        );
        assert_eq!(requests.len(), 6, "provider must exhaust its five retries");
        assert_eq!(response["jsonrpc"], "2.0");
        assert_eq!(response["id"], 3);
        assert_eq!(response["error"]["code"], -32603, "{response}");
        assert!(response["error"]["message"].is_string(), "{response}");
        assert!(
            response.get("result").is_none(),
            "failure cannot be success"
        );
        // Installed dsh-acp maps failed turns to internalError(undefined, detail).
        // Neither HTTP status nor Retry-After survives as typed error data.
        assert!(response["error"].get("data").is_none(), "{response}");
        upstream.shutdown().unwrap();
        isolation.close().unwrap();
    }
}

#[test]
#[ignore = "needs dsh"]
fn o10_usage_fields() {
    let isolation = Isolation::new().unwrap();
    let upstream =
        ScriptedUpstream::start(vec![Turn::Text("usage probe response".into())]).unwrap();
    let mut acp = Acp::start(&isolation, &upstream.base_url(), "o10-usage");
    let session = acp.session(&isolation, vec![]);
    let response = acp.prompt(&session, 45);
    acp.stop();
    assert_eq!(response["result"]["stopReason"], "end_turn", "{response}");
    assert_eq!(assistant_text(&acp), "usage probe response");
    let updates: Vec<_> = acp
        .transcript
        .iter()
        .filter(|entry| entry["frame"]["method"] == "session/update")
        .collect();
    let usage: Vec<_> = updates
        .iter()
        .filter(|entry| entry["frame"]["params"]["update"]["sessionUpdate"] == "usage_update")
        .collect();
    println!(
        "{}",
        json!({"probe":"DSH.O10","response":response,"usage_updates":usage,"usage_update_present":!usage.is_empty(),"frames":updates})
    );
    drop(acp);
    assert_eq!(upstream.shutdown().unwrap().len(), 1);
    isolation.close().unwrap();
}

#[test]
#[ignore = "needs dsh; takes 70 seconds"]
fn o07_long_mcp_progress() {
    let isolation = Isolation::new().unwrap();
    let upstream = ScriptedUpstream::start(vec![
        Turn::ToolCall {
            name: "mcp__probe__slow".into(),
            args: json!({}),
        },
        Turn::Text("long tool observed".into()),
    ])
    .unwrap();
    let script = plain_path(&isolation.support_dir().join("slow-mcp.cjs"));
    fs::write(&script, include_str!("fixtures/dsh/slow-mcp.cjs")).unwrap();
    let events = plain_path(&evidence_root().join("o07-long-mcp.mcp.jsonl"));
    fs::write(&events, "").unwrap();
    let node = std::env::var("AUTOCRAFT_SMOKE_NODE")
        .expect("O07 requires AUTOCRAFT_SMOKE_NODE pointing to the absolute Node executable");
    assert!(
        std::path::Path::new(&node).is_absolute(),
        "AUTOCRAFT_SMOKE_NODE must be absolute"
    );
    let mut acp = Acp::start(&isolation, &upstream.base_url(), "o07-long-mcp");
    let session = acp.session(
        &isolation,
        vec![json!({"name":"probe","command":node,"args":[script,events],"env":[]})],
    );
    let started = now_ms();
    // Seventy seconds work plus at most thirty seconds extra, including final model turn.
    let response = acp.prompt(&session, 100);
    let settled = now_ms();
    acp.stop();
    let requests = upstream.requests().unwrap();
    let tool_results: Vec<_> = requests
        .iter()
        .flat_map(|request| request.body["messages"].as_array().into_iter().flatten())
        .filter(|message| message["role"] == "tool")
        .collect();
    let mcp_events: Vec<Value> = fs::read_to_string(&events)
        .expect("real MCP tool executed")
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let call = mcp_events.iter().find(|event| event["event"] == "call");
    let token_present = call.map(|event| &event["progress_token_present"]);
    evidence(
        "o07-long-mcp",
        &json!({"probe":"DSH.O07","prompt_started_at_ms":started,"prompt_settled_at_ms":settled,"duration_ms":settled-started,"response":response,"tool_results":tool_results,"mcp_events":mcp_events,"requests":model_requests(&requests),"progress_token_present":token_present,"valid_progress_timeout_reset":"not established when client omits progress token"}),
    );
    let call = call.expect("MCP slow call must actually execute");
    let progress: Vec<_> = mcp_events
        .iter()
        .filter(|event| {
            event["event"] == "send" && event["frame"]["method"] == "notifications/progress"
        })
        .collect();
    for event in &progress {
        assert_eq!(
            event["frame"]["params"]["progressToken"], call["params"]["_meta"]["progressToken"],
            "progress must use exactly the client-supplied token"
        );
    }
    // Pin the installed rc.5 behavior, not an invented valid-progress experiment.
    // A newer engine supplying a token must be investigated with the retained raw frames.
    assert_eq!(call["progress_token_present"], false, "{call}");
    assert_eq!(progress.len(), 0, "never invent a fallback progress token");
    assert!(mcp_events.iter().any(|event| {
        event["event"] == "progress_unavailable" && event["reason"] == "missing_progress_token"
    }));
    let cancelled = mcp_events
        .iter()
        .find(|event| event["event"] == "cancelled")
        .expect("MCP client must cancel the timed-out request");
    assert_eq!(cancelled["params"]["requestId"], call["request_id"]);
    let elapsed = cancelled["at_ms"].as_u64().unwrap() - call["at_ms"].as_u64().unwrap();
    assert!(
        (55_000..70_000).contains(&elapsed),
        "cancel after {elapsed}ms"
    );
    let failed: Vec<_> = acp
        .transcript
        .iter()
        .map(|entry| &entry["frame"]["params"]["update"])
        .filter(|update| {
            update["sessionUpdate"] == "tool_call_update" && update["status"] == "failed"
        })
        .collect();
    assert_eq!(failed.len(), 1, "ACP client must see the failed tool");
    assert_eq!(
        tool_results.len(),
        1,
        "model must consume the failed tool result"
    );
    let client_text: String = failed[0]["content"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|block| block["content"]["text"].as_str())
        .collect();
    assert_eq!(tool_results[0]["content"], client_text);
    assert_eq!(response["result"]["stopReason"], "end_turn", "{response}");
    assert_eq!(assistant_text(&acp), "long tool observed");
    assert_eq!(
        requests.len(),
        2,
        "model must finish after observing failure"
    );
    upstream.shutdown().unwrap();
    isolation.close().unwrap();
}
