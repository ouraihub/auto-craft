//! Intercepts only policy-rejected, host-waiting and DSH-background tool calls.
use super::{Connection, Inflight};
use crate::{AnalysisEvent, calls, catalog, host_exec, wire};
use autocraft_core::{CancelFlag, ChannelOptions, ChatPurpose};
use ca_engine::{Caller, CancelToken, Progress};
use ca_evidence::{ErrorCode, ToolError};
use parking_lot::Mutex;
use serde_json::{Value, json};
use std::{
    io::{self, BufRead, Read},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

const MAX_LINE: usize = 64 * 1024 * 1024;
static NEXT_JOB: AtomicU64 = AtomicU64::new(1);
pub(super) struct PolicyReader {
    reader: Box<dyn BufRead + Send>,
    connection: Arc<Connection>,
    options: ChannelOptions,
    buffer: Vec<u8>,
    offset: usize,
    workers: Vec<JoinHandle<()>>,
    eof: bool,
}
impl PolicyReader {
    pub(super) fn new(
        reader: Box<dyn BufRead + Send>,
        connection: Arc<Connection>,
        options: ChannelOptions,
    ) -> Self {
        Self {
            reader,
            connection,
            options,
            buffer: Vec::new(),
            offset: 0,
            workers: Vec::new(),
            eof: false,
        }
    }
    fn intercept(&mut self, message: &Value) -> io::Result<bool> {
        let method = message.get("method").and_then(Value::as_str);
        if method == Some("notifications/cancelled") {
            if let Some(id) = message["params"].get("requestId")
                && let Some(call) = self.connection.inflight.lock().remove(&id.to_string())
            {
                call.cancel.cancel();
                call.token.cancel();
                self.connection.hub.changed(&self.connection.scope.project);
            }
            return Ok(false);
        }
        self.connection.loaded.apply_settings();
        if method != Some("tools/call") {
            return Ok(false);
        }
        let Some(id) = message.get("id").cloned() else {
            return Ok(false);
        };
        let params = &message["params"];
        let Some(name) = params.get("name").and_then(Value::as_str) else {
            return Ok(false);
        };
        let Some(spec) = self.connection.loaded.engine.registry().get(name).copied() else {
            return Ok(false);
        };
        if self.connection.scope.purpose == ChatPurpose::Question
            && catalog::read_only_forbidden(&spec)
        {
            let error = ToolError::new(
                ErrorCode::NoPermission,
                "This is a read-only question session. Tools that write files, launch processes, execute code or mutate the device or target are not allowed.",
            );
            self.connection
                .send(&wire::response(id, wire::error_body(&error)))?;
            return Ok(true);
        }
        if crate::project_dto::workspace(&self.connection.scope.project)
            .map_err(|_| io::Error::new(io::ErrorKind::NotFound, "Project unavailable"))?
            .is_read_only()
        {
            self.connection.send(&wire::response(
                id,
                wire::error_body(&ToolError::new(
                    ErrorCode::NoPermission,
                    "This project was written by a newer version and is read-only.",
                )),
            ))?;
            return Ok(true);
        }
        let background = self.options.async_long_tools
            && catalog::long_tool(&spec)
            && !host_exec::contains(spec.id);
        let cancel = CancelFlag::new();
        let token = CancelToken::default();
        let registration = self.connection.hub.register(
            &self.connection.loaded,
            spec.id.into(),
            cancel.clone(),
            token.clone(),
            Some(self.connection.scope.chat.clone()),
        );
        if !host_exec::contains(spec.id) && !background {
            self.connection.inflight.lock().insert(
                id.to_string(),
                Inflight {
                    tool: spec.id.into(),
                    progress: params
                        .get("_meta")
                        .and_then(|m| m.get("progressToken"))
                        .cloned(),
                    plan: (spec.id == "core_plan_update").then(|| {
                        params
                            .get("arguments")
                            .cloned()
                            .unwrap_or_else(|| json!({}))
                    }),
                    cancel,
                    token,
                    _registration: Some(registration),
                },
            );
            return Ok(false);
        }
        let job = NEXT_JOB.fetch_add(1, Ordering::Relaxed);
        if background {
            self.connection.send(&wire::response(id.clone(),json!({"content":[{"type":"text","text":format!("Started {} in the background (job {job}). Its result will arrive as your next message; continue with other work or wait.",spec.id)}],"isError":false})))?;
        }
        let args = params
            .get("arguments")
            .cloned()
            .unwrap_or_else(|| json!({}));
        let progress_token = params
            .get("_meta")
            .and_then(|m| m.get("progressToken"))
            .cloned();
        if !background {
            // The worker owns registration; the map only supplies cancellation and busy correlation.
            self.connection.inflight.lock().insert(
                id.to_string(),
                Inflight {
                    tool: spec.id.into(),
                    progress: progress_token.clone(),
                    plan: None,
                    cancel: cancel.clone(),
                    token: token.clone(),
                    _registration: None,
                },
            );
        }
        let connection = self.connection.clone();
        let span = tracing::Span::current();
        let worker = std::thread::Builder::new().name("analysis-host-call".into()).spawn(move || {
            let _span = span.enter();
            let _registration = registration;
            let last = Mutex::new(None::<Instant>);
            let sequence = Arc::new(AtomicU64::new(0));
            let progress_connection = connection.clone();
            let progress_sequence = sequence.clone();
            let pt = progress_token.clone();
            let progress = move |p: Progress| {
                let mut last = last.lock();
                if last.is_some_and(|t|t.elapsed()<Duration::from_millis(100)) { return; }
                *last = Some(Instant::now());
                progress_connection.hub.sink.publish(AnalysisEvent::Progress {project:progress_connection.scope.project.clone(),tool:spec.id.into(),message:p.message.clone(),fraction:p.fraction});
                if let Some(pt) = &pt {
                    let message = match p.fraction { Some(f)=>format!("{} ({:.0}%)",p.message,f64::from(f.clamp(0.0,1.0))*100.0),None=>p.message };
                    send(&progress_connection,&json!({"jsonrpc":"2.0","method":"notifications/progress","params":{"progressToken":pt,"progress":progress_sequence.fetch_add(1,Ordering::Relaxed)+1,"message":message}}));
                }
            };
            let result = calls::execute(&connection.hub,&connection.scope.project,&connection.loaded,spec.id,args,Caller::Mcp,false,&cancel,&token,&progress);
            let body = wire::body(spec.id,result,spec.effects,&id);
            if let Some(pt) = &progress_token { send(&connection,&json!({"jsonrpc":"2.0","method":"notifications/progress","params":{"progressToken":pt,"progress":sequence.fetch_add(1,Ordering::Relaxed)+1,"message":"done"}})); }
            drop(_registration);
            connection.hub.changed(&connection.scope.project);
            if background {
                if !cancel.is_cancelled()
                    && let Some(sink) = &connection.hub.long_results
                    && let Err(e) = sink.deliver(&connection.scope,format!("Result of background job {job} ({}):\n{body}",spec.id))
                {
                    tracing::warn!(event="analysis.background.delivery_failed",code=autocraft_core::ErrorCode::code(&e));
                }
            } else {
                connection.inflight.lock().remove(&id.to_string());
                if !cancel.is_cancelled() { send(&connection,&wire::response(id,body)); }
            }
        })?;
        if !background {
            self.workers.push(worker);
        }
        Ok(true)
    }
}
fn send(connection: &Connection, value: &Value) {
    if connection.send(value).is_err() {
        tracing::debug!(event = "analysis.channel.disconnected");
    }
}
impl Read for PolicyReader {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        let available = self.fill_buf()?;
        let n = available.len().min(bytes.len());
        bytes[..n].copy_from_slice(&available[..n]);
        self.consume(n);
        Ok(n)
    }
}
impl BufRead for PolicyReader {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        if self.eof {
            return Ok(&[]);
        }
        while self.offset == self.buffer.len() {
            self.buffer.clear();
            self.offset = 0;
            loop {
                let bytes = self.reader.fill_buf()?;
                if bytes.is_empty() {
                    break;
                }
                let n = bytes
                    .iter()
                    .position(|b| *b == b'\n')
                    .map_or(bytes.len(), |n| n + 1);
                if self.buffer.len().saturating_add(n) > MAX_LINE {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "MCP request too large",
                    ));
                }
                self.buffer.extend_from_slice(&bytes[..n]);
                let done = bytes[n - 1] == b'\n';
                self.reader.consume(n);
                if done {
                    break;
                }
            }
            if self.buffer.is_empty() {
                // Return through serve before joining host workers; ca_mcp waits its own calls.
                self.eof = true;
                return Ok(&[]);
            }
            if let Ok(message) = serde_json::from_slice::<Value>(&self.buffer)
                && self.intercept(&message)?
            {
                self.buffer.clear();
                continue;
            }
            break;
        }
        Ok(&self.buffer[self.offset..])
    }
    fn consume(&mut self, n: usize) {
        self.offset = (self.offset + n).min(self.buffer.len());
    }
}
impl Drop for PolicyReader {
    fn drop(&mut self) {
        for worker in self.workers.drain(..) {
            if let Err(payload) = worker.join() {
                std::panic::resume_unwind(payload);
            }
        }
    }
}
