//! One genuine upstream MCP server per connection, with atomic shared output and policy reader.
mod policy;
use crate::{AnalysisEvent, AnalysisHub, calls::Registration, engines::Loaded};
use autocraft_core::*;
use ca_engine::CancelToken;
use parking_lot::Mutex;
use serde_json::Value;
use std::{
    collections::HashMap,
    io::{self, BufRead, Write},
    sync::Arc,
};

pub(super) struct Inflight {
    pub(super) tool: String,
    pub(super) progress: Option<Value>,
    pub(super) cancel: CancelFlag,
    pub(super) token: CancelToken,
    pub(super) plan: Option<Value>,
    pub(super) _registration: Option<Registration>,
}
pub(super) struct Connection {
    pub(super) hub: Arc<AnalysisHub>,
    pub(super) scope: ChatScope,
    pub(super) loaded: Arc<Loaded>,
    pub(super) out: Mutex<Box<dyn Write + Send>>,
    pub(super) inflight: Mutex<HashMap<String, Inflight>>,
    failure: Mutex<Option<io::ErrorKind>>,
}
impl Connection {
    pub(super) fn send(&self, message: &Value) -> io::Result<()> {
        let mut bytes = serde_json::to_vec(message)?;
        bytes.push(b'\n');
        self.write_line(&bytes)
    }
    fn write_line(&self, bytes: &[u8]) -> io::Result<()> {
        let mut writer = self.out.lock();
        writer
            .write_all(bytes)
            .and_then(|()| writer.flush())
            .inspect_err(|e| {
                *self.failure.lock() = Some(e.kind());
            })
    }
    fn observe(&self, line: &[u8]) {
        let Ok(value) = serde_json::from_slice::<Value>(line) else {
            return;
        };
        if let Some(id) = value.get("id") {
            self.loaded.apply_settings();
            if let Some(call) = self.inflight.lock().remove(&id.to_string()) {
                if value
                    .get("result")
                    .and_then(|r| r.get("isError"))
                    .and_then(Value::as_bool)
                    == Some(false)
                    && let Some(steps) = call.plan
                {
                    self.hub.sink.publish(AnalysisEvent::PlanUpdated {
                        project: self.scope.project.clone(),
                        steps,
                    });
                }
                self.hub.changed(&self.scope.project);
            }
        } else if value.get("method").and_then(Value::as_str) == Some("notifications/progress") {
            let params = &value["params"];
            let tool = self
                .inflight
                .lock()
                .values()
                .find(|call| call.progress.as_ref() == params.get("progressToken"))
                .map(|c| c.tool.clone());
            if let Some(tool) = tool {
                self.hub.sink.publish(AnalysisEvent::Progress {
                    project: self.scope.project.clone(),
                    tool,
                    message: params["message"].as_str().unwrap_or_default().to_owned(),
                    fraction: None,
                });
            }
        }
    }
}
/// Buffers upstream's potentially split writes until a complete line, without altering bytes.
struct ObservingWriter {
    connection: Arc<Connection>,
    pending: Vec<u8>,
}
impl Write for ObservingWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.pending.extend_from_slice(bytes);
        while let Some(end) = self.pending.iter().position(|b| *b == b'\n') {
            let line: Vec<_> = self.pending.drain(..=end).collect();
            self.connection.write_line(&line)?;
            self.connection.observe(&line);
        }
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        self.connection.out.lock().flush()
    }
}
impl ChannelHandler for AnalysisHub {
    fn serve(
        &self,
        scope: &ChatScope,
        options: ChannelOptions,
        reader: Box<dyn BufRead + Send>,
        writer: Box<dyn Write + Send>,
    ) -> io::Result<()> {
        if options.async_long_tools && self.long_results.is_none() {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "Background result delivery is not configured",
            ));
        }
        let hub = self.me.upgrade().ok_or_else(|| {
            io::Error::new(io::ErrorKind::BrokenPipe, "Analysis service has closed")
        })?;
        let loaded = self.loaded(&scope.project).map_err(|_| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "Analysis project could not be opened",
            )
        })?;
        let connection = Arc::new(Connection {
            hub,
            scope: scope.clone(),
            loaded: loaded.clone(),
            out: Mutex::new(writer),
            inflight: Mutex::new(HashMap::new()),
            failure: Mutex::new(None),
        });
        let policy = policy::PolicyReader::new(reader, connection.clone(), options);
        let observer = ObservingWriter {
            connection: connection.clone(),
            pending: Vec::new(),
        };
        let opts = ca_mcp::ServerOptions {
            all_tools: options.all_tools,
            allow_run: false,
            restricted: false,
            max_response_bytes: ca_mcp::DEFAULT_MAX_RESPONSE_BYTES,
            budget_for_client: false,
            ..Default::default()
        };
        let result = ca_mcp::serve(loaded.engine.clone(), opts, policy, Box::new(observer));
        connection.inflight.lock().clear();
        if let Some(kind) = *connection.failure.lock() {
            return Err(io::Error::new(kind, "Analysis channel write failed"));
        }
        result
    }
}
