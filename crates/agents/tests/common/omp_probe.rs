//! Owned real OMP process and bounded raw RPC recorder for the P03 probes.

use std::{
    collections::BTreeMap,
    env, fs,
    io::{self, BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc::{self, Receiver},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use autocraft_testkit::{Engine, Isolation, RecordedRequest};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::{Value, json};

pub const WAIT: Duration = Duration::from_secs(90);

type PendingChunks = Option<(String, u64, u64, usize, Vec<u8>)>;

#[derive(Debug)]
pub struct Omp {
    child: Child,
    stdin: Option<ChildStdin>,
    output: Receiver<io::Result<Value>>,
    readers: Vec<JoinHandle<()>>,
    stderr_path: PathBuf,
    pub frames: Vec<Value>,
    sequence: u64,
}

impl Omp {
    pub fn start(home: &Isolation, base_url: &str, tools: &str, overlay: &str) -> Self {
        let config = plain_path(&home.config_dir(Engine::Omp));
        fs::write(
            config.join("models.yml"),
            format!(
                "providers:\n  autocraft-probe:\n    baseUrl: {base_url}\n    api: openai-completions\n    auth: none\n    models:\n      - id: probe-model\n        name: Probe\n        contextWindow: 128000\n        maxTokens: 1024\n"
            ),
        )
        .unwrap();
        let overlay_path = plain_path(&home.support_dir().join("probe.yml"));
        fs::write(
            &overlay_path,
            format!(
                "modelRoles:\n  default: autocraft-probe/probe-model\nenabledProviders: [autocraft-probe]\nmcp:\n  enableProjectConfig: false\nedit:\n  mode: replace\n  enforceSeenLines: false\n{overlay}"
            ),
        )
        .unwrap();
        let extension = plain_path(&home.support_dir().join("observe.ts"));
        fs::write(
            &extension,
            r#"export default function (pi) {
  pi.on("tool_call", (event, ctx) => {
    ctx.ui.notify("autocraft:tool_call:" + JSON.stringify({ toolName: event.toolName, input: event.input }), "info");
  });
}
"#,
        )
        .unwrap();
        let spec = env::var("AUTOCRAFT_SMOKE_OMP").expect("set AUTOCRAFT_SMOKE_OMP=executable|arg");
        let mut parts = spec.split('|');
        let mut command = Command::new(parts.next().filter(|p| !p.is_empty()).unwrap());
        command.args(parts);
        // Whitelist OS launch variables, never provider keys, profile selectors or user config.
        command.env_clear();
        for key in [
            "SystemRoot",
            "WINDIR",
            "PATH",
            "PATHEXT",
            "COMSPEC",
            "TEMP",
            "TMP",
        ] {
            if let Some(value) = env::var_os(key) {
                command.env(key, value);
            }
        }
        for key in [
            "HOME",
            "USERPROFILE",
            "APPDATA",
            "LOCALAPPDATA",
            "XDG_CONFIG_HOME",
            "XDG_DATA_HOME",
            "XDG_CACHE_HOME",
            "XDG_STATE_HOME",
        ] {
            command.env(key, plain_path(home.root()));
        }
        command
            .env("PI_CODING_AGENT_DIR", &config)
            .env("PI_CODING_AGENT_SESSION_DIR", config.join("sessions"))
            .env("PI_EDIT_VARIANT", "replace")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env(
                "GIT_CONFIG_GLOBAL",
                plain_path(&home.support_dir().join("empty-gitconfig")),
            )
            .current_dir(plain_path(&home.cwd()))
            .args([
                "--mode",
                "rpc-ui",
                "--approval-mode",
                "always-ask",
                "--no-skills",
                "--no-rules",
                "--no-context-files",
                "--no-extensions",
                "--no-lsp",
                "--no-pty",
                "--provider",
                "autocraft-probe",
                "--model",
                "probe-model",
                "--tools",
                tools,
                "--config",
            ])
            .arg(&overlay_path)
            .arg("-e")
            .arg(extension)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command.spawn().expect("launch real OMP");
        let stdin = child.stdin.take();
        let stdout = child.stdout.take().unwrap();
        let mut stderr = child.stderr.take().unwrap();
        let (send, output) = mpsc::channel();
        let stdout_reader = thread::spawn(move || {
            let mut pending: PendingChunks = None;
            let mut reader = BufReader::new(stdout);
            loop {
                let result = next_frame(&mut reader, &mut pending);
                match result {
                    Ok(Some(frame)) => {
                        if send.send(Ok(frame)).is_err() {
                            break;
                        }
                    }
                    Ok(None) => break,
                    Err(error) => {
                        let _ = send.send(Err(error));
                        break;
                    }
                }
            }
        });
        let stderr_path = home.support_dir().join("omp-stderr.txt");
        let capture_path = stderr_path.clone();
        let stderr_reader = thread::spawn(move || {
            let mut file = fs::File::create(capture_path).unwrap();
            io::copy(&mut stderr, &mut file).unwrap();
        });
        let mut omp = Self {
            child,
            stdin,
            output,
            readers: vec![stdout_reader, stderr_reader],
            stderr_path,
            frames: Vec::new(),
            sequence: 0,
        };
        let ready = omp.until(Instant::now() + WAIT, |frame| frame["type"] == "ready");
        assert!(
            ready["supportedProtocolVersions"]
                .as_array()
                .unwrap()
                .contains(&json!(2))
        );
        let reply = omp.call("negotiate_protocol", json!({"protocolVersion": 2}));
        assert_eq!(reply["success"], true, "{reply}");
        let reply = omp.call(
            "set_event_filter",
            json!({"events": null, "messageUpdates": "delta"}),
        );
        assert_eq!(reply["success"], true, "{reply}");
        omp
    }

    pub fn send(&mut self, mut frame: Value) -> String {
        self.sequence += 1;
        let id = format!("probe-{}", self.sequence);
        frame["id"] = json!(id);
        self.reply(frame);
        id
    }

    pub fn reply(&mut self, frame: Value) {
        let stdin = self.stdin.as_mut().unwrap();
        serde_json::to_writer(&mut *stdin, &frame).unwrap();
        stdin.write_all(b"\n").unwrap();
        stdin.flush().unwrap();
    }

    pub fn call(&mut self, kind: &str, mut fields: Value) -> Value {
        fields["type"] = json!(kind);
        let id = self.send(fields);
        self.until(Instant::now() + WAIT, |frame| {
            frame["type"] == "response" && frame["id"] == id
        })
    }

    pub fn next(&mut self, deadline: Instant) -> Value {
        let received = self
            .output
            .recv_timeout(deadline.saturating_duration_since(Instant::now()));
        let frame = match received {
            Ok(Ok(frame)) => frame,
            error => {
                // Join stderr before the sandbox unwinds, so startup diagnostics cannot disappear.
                self.stop();
                let stderr = fs::read_to_string(&self.stderr_path).unwrap();
                panic!(
                    "OMP RPC deadline/EOF/protocol failure: {error:?}; stderr={stderr}; frames={}",
                    json!(self.frames)
                );
            }
        };
        self.frames.push(frame.clone());
        frame
    }

    pub fn until(&mut self, deadline: Instant, predicate: impl Fn(&Value) -> bool) -> Value {
        loop {
            let frame = self.next(deadline);
            if predicate(&frame) {
                return frame;
            }
        }
    }

    pub fn deny(&mut self, frame: &Value) {
        let mut response = json!({"type": "extension_ui_response", "id": frame["id"]});
        match frame["method"].as_str() {
            Some("select") => {
                assert!(
                    frame["options"]
                        .as_array()
                        .unwrap()
                        .contains(&json!("Deny")),
                    "{frame}"
                );
                response["value"] = json!("Deny");
            }
            Some("confirm") => response["confirmed"] = json!(false),
            _ => response["cancelled"] = json!(true),
        }
        self.reply(response);
    }

    pub fn finish_turn(&mut self, id: &str, deadline: Instant) -> Value {
        loop {
            let frame = self.next(deadline);
            if is_dialog(&frame) {
                self.deny(&frame);
            }
            if frame["type"] == "prompt_result" && frame["id"] == id {
                return frame;
            }
        }
    }

    pub fn close(mut self) {
        self.stop();
    }

    fn stop(&mut self) {
        // Abort first so pending dialogs/tools get cancellation; bounded kill also handles failures.
        if let Some(mut stdin) = self.stdin.take() {
            if let Err(error) = stdin.write_all(b"{\"type\":\"abort\"}\n") {
                assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
            }
            drop(stdin);
        }
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            if self.child.try_wait().unwrap().is_some() {
                break;
            }
            if Instant::now() >= deadline {
                self.child.kill().unwrap();
                self.child.wait().unwrap();
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
        for reader in self.readers.drain(..) {
            reader.join().unwrap();
        }
    }
}

impl Drop for Omp {
    fn drop(&mut self) {
        self.stop();
    }
}

fn plain_path(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        PathBuf::from(format!(r"\\{rest}"))
    } else {
        PathBuf::from(text.strip_prefix(r"\\?\").unwrap_or(&text))
    }
}

fn next_frame(reader: &mut impl BufRead, pending: &mut PendingChunks) -> io::Result<Option<Value>> {
    loop {
        let mut line = Vec::new();
        let count = (&mut *reader)
            .take(1024 * 1024 + 1)
            .read_until(b'\n', &mut line)?;
        if count == 0 {
            if pending.is_some() {
                return Err(io::Error::other("truncated OMP chunk"));
            }
            return Ok(None);
        }
        if count > 1024 * 1024 || !line.ends_with(b"\n") {
            return Err(io::Error::other(
                "oversized/unterminated OMP physical frame",
            ));
        }
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let frame: Value = serde_json::from_slice(&line)?;
        if frame["type"] != "rpc_chunk" {
            if pending.is_some() {
                return Err(io::Error::other("interrupted OMP chunk"));
            }
            return Ok(Some(frame));
        }
        let id = frame["chunkId"]
            .as_str()
            .ok_or_else(|| io::Error::other("chunk id"))?;
        let index = frame["index"]
            .as_u64()
            .ok_or_else(|| io::Error::other("chunk index"))?;
        let count = frame["count"]
            .as_u64()
            .ok_or_else(|| io::Error::other("chunk count"))?;
        let size = frame["byteLength"]
            .as_u64()
            .ok_or_else(|| io::Error::other("chunk size"))?;
        if size > 64 * 1024 * 1024 || count == 0 {
            return Err(io::Error::other("chunk bounds"));
        }
        let state =
            pending.get_or_insert_with(|| (id.to_owned(), count, 0, size as usize, Vec::new()));
        if state.0 != id || state.1 != count || state.2 != index || state.3 != size as usize {
            return Err(io::Error::other("chunk sequence mismatch"));
        }
        let bytes = STANDARD
            .decode(
                frame["data"]
                    .as_str()
                    .ok_or_else(|| io::Error::other("chunk data"))?,
            )
            .map_err(io::Error::other)?;
        if state.4.len() + bytes.len() > state.3 {
            return Err(io::Error::other("chunk overflow"));
        }
        state.4.extend(bytes);
        state.2 += 1;
        if state.2 == count {
            let (_, _, _, size, bytes) = pending.take().unwrap();
            if bytes.len() != size {
                return Err(io::Error::other("chunk length mismatch"));
            }
            return serde_json::from_slice(&bytes).map(Some).map_err(Into::into);
        }
    }
}

pub fn is_dialog(frame: &Value) -> bool {
    frame["type"] == "extension_ui_request"
        && matches!(
            frame["method"].as_str(),
            Some("select" | "confirm" | "input" | "editor" | "ask")
        )
}

pub fn tools(request: &RecordedRequest) -> Vec<String> {
    request.body["tools"]
        .as_array()
        .expect("actual model tools list")
        .iter()
        .map(|tool| tool["function"]["name"].as_str().unwrap().to_owned())
        .collect()
}

pub fn fixture(name: &str, value: &Value, home: &Isolation, base_url: &str) {
    // Only synthetic probe input is used; normalize nondeterministic sandbox paths and loopback port.
    let encoded = serde_json::to_string_pretty(value).unwrap();
    let mut replacements = BTreeMap::new();
    for path in [
        home.root().to_string_lossy().into_owned(),
        home.root().to_string_lossy().replace('\\', "/"),
    ] {
        let escaped = serde_json::to_string(&path).unwrap();
        replacements.insert(escaped[1..escaped.len() - 1].to_owned(), "<ISOLATED_ROOT>");
    }
    let mut encoded = encoded.replace(base_url, "http://127.0.0.1:<PORT>/v1");
    for (from, to) in replacements {
        encoded = encoded.replace(&from, to);
    }
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/omp");
    fs::create_dir_all(&directory).unwrap();
    fs::write(
        directory.join(format!("{name}.json")),
        format!("{encoded}\n"),
    )
    .unwrap();
}
