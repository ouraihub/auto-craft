//! Official pi CLI over `--mode rpc` (JSONL commands, events and `extension_ui_*` dialogs).

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use autocraft_core::ErrorCode as _;
use autocraft_core::{
    AgentRuntime, AgentSession, BackendKind, Capabilities, Error, InteractionAnswer,
    NativeSessionKey, Result, SessionSink, SessionSnapshot, StartSession, ThinkingLevel,
};
use serde_json::{Value, json};
use tracing::info;

use crate::{
    framing::MAX_FRAME,
    process::{self, Process},
    projection::Projection,
    rpc::{self, Dialogs, Stream},
    session::{self, COMMAND, HANDSHAKE, Live, engine_error, new_id, session_error},
};
mod actions;
mod launch;
use actions::action;
use launch::launch_spec;
#[cfg(test)]
pub(crate) use launch::readonly_args;

/// `compact` summarises the transcript with one model call; the generic command timeout is short.
const COMPACT: Duration = Duration::from_secs(10 * 60);

const CAPABILITIES: Capabilities = Capabilities {
    resume: true,
    history_replay: true,
    token_deltas: true,
    questions: true,
    tool_permissions: false,
    model_switch: true,
    compact: true,
    rename: true,
    steer: true,
};

/// Launches pi `--mode rpc` per chat.
#[derive(Debug, Default)]
pub struct PiRuntime;

impl PiRuntime {
    pub fn new() -> Self {
        Self
    }
}

impl AgentRuntime for PiRuntime {
    fn backend(&self) -> BackendKind {
        BackendKind::Pi
    }

    fn capabilities(&self) -> Capabilities {
        CAPABILITIES
    }

    fn start(
        &self,
        request: StartSession,
        sink: Arc<dyn SessionSink>,
    ) -> Result<Arc<dyn AgentSession>> {
        session::validate(&request, BackendKind::Pi)?;
        let trace = Arc::new(crate::trace::SessionTrace::new(&request));
        let _entered = trace.enter("agent.session.start");
        let command = process::command(&launch_spec(&request)?)?;
        let (process, stdout) = Process::spawn(command, MAX_FRAME, trace)?;
        let projection = Projection::new(session::identity(&request, CAPABILITIES), sink);
        let live = Live::new(process, projection, BackendKind::Pi, &request);
        let pi = Arc::new(PiSession {
            live: live.clone(),
            dialogs: Dialogs::default(),
            fork_file: parking_lot::Mutex::new(None),
        });
        let reader = pi.clone();
        let mut stream = Stream::default();
        session::spawn_reader(live.clone(), stdout, MAX_FRAME, move |frame| {
            reader.on_frame(&mut stream, frame)
        })?;
        let deadline = Instant::now() + HANDSHAKE;
        pi.handshake(deadline)
            .map_err(|error| live.fail_start(error, None))?;
        if request.role == autocraft_core::SessionRole::ReadOnly && request.resume.is_some() {
            pi.fork_readonly(deadline)
                .map_err(|error| live.fail_start(error, None))?;
        }
        info!(chat = %request.chat, backend = "pi", "agent session started");
        Ok(pi)
    }
}

struct PiSession {
    live: Arc<Live>,
    dialogs: Dialogs,
    fork_file: parking_lot::Mutex<Option<std::path::PathBuf>>,
}

impl PiSession {
    fn fork_readonly(&self, deadline: Instant) -> Result<()> {
        // clone forks at the current leaf; fork defaults to before a user message and
        // rejects assistant/tool entries, which commonly end a completed build.
        let result = self.call("clone", json!({}), session::left(deadline))?;
        if result["cancelled"] == true {
            return Err(Error::invalid("只读会话分叉已取消"));
        }
        let state = self.call("get_state", json!({}), session::left(deadline))?;
        let path = state["sessionFile"]
            .as_str()
            .filter(|path| !path.is_empty())
            .ok_or_else(|| Error::protocol("只读分叉没有返回会话文件"))?;
        let original = self.live.state.lock().snapshot().native_session_key;
        if original.as_ref().is_some_and(|key| key.as_str() == path) {
            return Err(Error::protocol("只读分叉仍指向构建方会话"));
        }
        *self.fork_file.lock() = Some(path.into());
        self.live.state.lock().set_key(NativeSessionKey::from(path));
        Ok(())
    }
    fn handshake(&self, deadline: Instant) -> Result<()> {
        let state = self.call("get_state", json!({}), session::left(deadline))?;
        let key = state["sessionFile"].as_str().filter(|s| !s.is_empty());
        let history = self.call("get_messages", json!({}), session::left(deadline))?;
        let commands = rpc::commands::or_empty(
            self.call("get_commands", json!({}), session::left(deadline))
                .map(|data| rpc::commands::pi_commands(&data)),
        );
        let mut p = self.live.state.lock();
        if let Some(key) = key {
            p.set_key(NativeSessionKey::from(key));
        }
        p.set_model(rpc::model(&state));
        p.set_session_name(rpc::commands::session_name(&state));
        p.set_commands(commands);
        if let Some(messages) = history["messages"].as_array() {
            rpc::replay(&mut p, messages, 0);
        }
        p.set_running(state["isStreaming"] == true);
        if state["isStreaming"] != true {
            p.fail_unfinished_tools();
        }
        p.publish();
        Ok(())
    }

    /// Sends a command and waits for its `response`.
    fn call(&self, kind: &str, mut fields: Value, timeout: Duration) -> Result<Value> {
        let id = new_id();
        fields["type"] = json!(kind);
        fields["id"] = json!(id);
        self.live.request(id, &fields, timeout)
    }

    fn on_frame(&self, stream: &mut Stream, frame: Value) -> Result<()> {
        let kind = frame["type"]
            .as_str()
            .ok_or_else(|| Error::protocol("pi 帧缺少 type"))?;
        match kind {
            "response" => self.on_response(frame),
            "extension_ui_request" => self.on_ui_request(&frame),
            "extension_error" => {
                let error = engine_error(frame["error"].as_str().unwrap_or("扩展执行失败"));
                self.live.state.lock().error(session_error(&error));
                Ok(())
            }
            _ => {
                let mut p = self.live.state.lock();
                match kind {
                    "agent_start" => p.set_running(true),
                    "agent_settled" => {
                        p.fail_unfinished_tools();
                        p.set_running(false);
                    }
                    "message_start" => stream.message_start(&mut p, &frame),
                    "message_update" => stream.message_update(&mut p, &frame),
                    "message_end" => stream.message_end(&mut p, &frame),
                    "tool_execution_start" | "tool_execution_update" | "tool_execution_end" => {
                        rpc::tool_event(&mut p, &frame);
                    }
                    "auto_retry_end" if frame["success"] == false => {
                        let error =
                            engine_error(frame["finalError"].as_str().unwrap_or("自动重试失败"));
                        p.error(session_error(&error));
                    }
                    "thinking_level_changed" => rpc::model_event(&mut p, &frame),
                    "compaction_end" if frame["errorMessage"].is_string() => {
                        let error =
                            engine_error(frame["errorMessage"].as_str().unwrap_or_default());
                        p.error(session_error(&error));
                    }
                    _ => {}
                }
                Ok(())
            }
        }
    }

    fn on_response(&self, frame: Value) -> Result<()> {
        let Some(id) = frame["id"].as_str() else {
            if frame["success"] == false {
                let error = engine_error(frame["error"].as_str().unwrap_or("命令失败"));
                self.live.state.lock().error(session_error(&error));
            }
            return Ok(());
        };
        let result = if frame["success"] == true {
            Ok(frame.get("data").cloned().unwrap_or(Value::Null))
        } else {
            Err(engine_error(frame["error"].as_str().unwrap_or("命令失败")))
        };
        if let Some(Err(error)) = self.live.resolve(id, result) {
            self.live.state.lock().error(session_error(&error));
        }
        Ok(())
    }
    fn on_ui_request(&self, frame: &Value) -> Result<()> {
        if crate::approvals::Approvals::marker(&self.live, frame, action)? {
            return Ok(());
        }
        self.dialogs.on_request(&self.live, frame)
    }
}

impl AgentSession for PiSession {
    fn snapshot(&self) -> SessionSnapshot {
        self.live.state.lock().snapshot()
    }

    fn usage_snapshot(&self) -> Result<Option<autocraft_core::SessionUsage>> {
        self.live.ensure_idle()?;
        let state = self.call("get_state", json!({}), COMMAND)?;
        let stats = self.call("get_session_stats", json!({}), COMMAND)?;
        let usage = rpc::usage::parse(&state, &stats);
        self.live.ensure_idle()?;
        Ok(Some(usage))
    }

    fn prompt(&self, text: &str) -> Result<()> {
        self.live.ensure_idle()?;
        self.live.approvals.begin_turn();
        self.live.state.lock().set_running(true);
        match self.call("prompt", json!({"message": text}), COMMAND) {
            // `handled`: an extension command completed locally; a run it starts reports agent_start.
            Ok(data) if data["disposition"] == "handled" => {
                self.live.state.lock().set_running(false);
                Ok(())
            }
            Ok(_) => Ok(()),
            Err(error) => {
                self.live.state.lock().finish_turn_error(error.code());
                self.live.state.lock().set_running(false);
                Err(error)
            }
        }
    }

    fn steer(&self, text: &str) -> Result<()> {
        self.live.ensure_open()?;
        self.call("steer", json!({"message":text}), COMMAND)
            .map(drop)
    }

    fn stop(&self) -> Result<()> {
        self.live.ensure_open()?;
        let _entered = self.live.process.trace.enter("agent.session.stop");
        info!("agent.session.stop_requested");
        self.live.state.lock().cancel_turn("user_cancelled");
        self.live.approvals.cancel(&self.live);
        self.call("abort", json!({}), COMMAND).map(drop)
    }

    fn respond(&self, request_id: &str, answer: InteractionAnswer) -> Result<()> {
        self.dialogs.respond(&self.live, request_id, &answer)
    }

    fn dispose(&self) -> Result<()> {
        let _entered = self.live.process.trace.enter("agent.session.dispose");
        if self.live.begin_dispose() {
            self.dialogs.cancel_all(&self.live);
            self.live.shutdown();
            if let Some(path) = self.fork_file.lock().take() {
                match std::fs::remove_file(path) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => {
                        return Err(Error::protocol(format!(
                            "无法删除只读分叉会话（{:?}）",
                            error.kind()
                        )));
                    }
                }
            }
        }
        Ok(())
    }

    fn set_model(
        &self,
        provider: &str,
        model_id: &str,
        effort: Option<ThinkingLevel>,
    ) -> Result<()> {
        rpc::switch_model(
            &self.live,
            |kind, fields| self.call(kind, fields, COMMAND),
            provider,
            model_id,
            effort,
        )
    }

    fn compact(&self, instructions: Option<&str>) -> Result<()> {
        self.live.ensure_idle()?;
        self.live.state.lock().set_running(true);
        let mut fields = json!({});
        if let Some(text) = instructions.filter(|s| !s.trim().is_empty()) {
            fields["customInstructions"] = json!(text);
        }
        let result = self.call("compact", fields, COMPACT);
        let mut p = self.live.state.lock();
        p.set_running(false);
        let text = match result {
            Ok(data) => match (
                data["tokensBefore"].as_u64(),
                data["estimatedTokensAfter"].as_u64(),
            ) {
                (Some(before), Some(after)) => {
                    format!("上下文已压缩（约 {before} → {after} tokens）")
                }
                _ => "上下文已压缩".to_owned(),
            },
            // pi refused (e.g. "Nothing to compact"): the session is fine, so this is a turn
            // outcome in the transcript, as in the original, not an app error.
            Err(error) if error.kind() == Some(autocraft_core::ErrorKind::External) => {
                format!("压缩失败。{}", error.detail())
            }
            Err(error) => return Err(error),
        };
        rpc::commands::note(&mut p, text);
        p.publish();
        Ok(())
    }

    fn rename(&self, name: &str) -> Result<()> {
        self.live.ensure_open()?;
        self.call("set_session_name", json!({"name": name}), COMMAND)?;
        self.live
            .state
            .lock()
            .set_session_name(Some(name.to_owned()));
        Ok(())
    }
}

#[cfg(test)]
mod tests;
