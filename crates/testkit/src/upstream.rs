//! An owned loopback HTTP server; one accepted model request consumes one scripted turn.

use std::{
    collections::VecDeque,
    fmt, io,
    net::{SocketAddr, TcpListener},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use serde_json::Value;

mod http;

/// A single OpenAI Chat completion response, including retries as separate requests.
#[derive(Debug, Clone, PartialEq)]
pub enum Turn {
    /// A complete assistant text response.
    Text(String),
    /// A single function call; a subsequent turn can consume its actual tool result.
    ToolCall {
        /// Name exposed to the real model by the engine.
        name: String,
        /// Structured function arguments, serialized on the SSE wire.
        args: Value,
    },
    /// A scripted upstream failure; the server never retries or repeats a turn itself.
    HttpError {
        /// HTTP error status in 400..=599 (including 429 and 500).
        status: u16,
        /// Optional Retry-After delay in seconds.
        retry_after: Option<u64>,
    },
}

/// Actual request received on the socket. Debug intentionally omits headers and body.
#[derive(Clone, PartialEq)]
pub struct RecordedRequest {
    /// HTTP request method.
    pub method: String,
    /// Request target, including a query if supplied.
    pub path: String,
    /// Lowercase header names and their actual values; test credentials only.
    pub headers: Vec<(String, String)>,
    /// Parsed request JSON, including the actual messages and tool declarations.
    pub body: Value,
}

impl fmt::Debug for RecordedRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RecordedRequest")
            .field("method", &self.method)
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

/// Owns the listener and worker. Explicit shutdown reports server/protocol errors and joins it.
#[derive(Debug)]
pub struct ScriptedUpstream {
    address: SocketAddr,
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
    turns: Arc<Mutex<VecDeque<Turn>>>,
    stopping: Arc<AtomicBool>,
    worker: Option<JoinHandle<io::Result<usize>>>,
}

impl ScriptedUpstream {
    /// Binds an OS-assigned loopback port and starts serving the supplied finite script.
    ///
    /// # Errors
    /// Returns bind/thread errors or an invalid scripted HTTP status.
    pub fn start(turns: Vec<Turn>) -> io::Result<Self> {
        validate_turns(&turns)?;
        let turns = Arc::new(Mutex::new(turns.into()));
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))?;
        listener.set_nonblocking(true)?;
        let address = listener.local_addr()?;
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stopping = Arc::new(AtomicBool::new(false));
        let recorded = Arc::clone(&requests);
        let stop = Arc::clone(&stopping);
        let script = Arc::clone(&turns);
        let worker = thread::Builder::new()
            .name("autocraft-test-upstream".into())
            .spawn(move || serve(listener, &script, &recorded, &stop))?;
        Ok(Self {
            address,
            requests,
            turns,
            stopping,
            worker: Some(worker),
        })
    }

    /// Provider base URL as required by the real OpenAI Chat adapter.
    pub fn base_url(&self) -> String {
        format!("http://{}/v1", self.address)
    }

    /// Appends finite responses after observing actual model tool names in an earlier request.
    /// Call before sending the next prompt: an empty queue still fails immediately, never waits.
    ///
    /// # Errors
    /// Returns an error for invalid HTTP statuses, a stopped worker or a poisoned script queue.
    pub fn push(&self, turns: Vec<Turn>) -> io::Result<()> {
        validate_turns(&turns)?;
        if self.stopping.load(Ordering::Acquire)
            || self.worker.as_ref().is_none_or(JoinHandle::is_finished)
        {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "upstream worker stopped",
            ));
        }
        self.turns
            .lock()
            .map_err(|_| io::Error::other("upstream script poisoned"))?
            .extend(turns);
        Ok(())
    }

    /// A snapshot of requests received so far, in wire order.
    ///
    /// # Errors
    /// Returns an error if the worker panicked while recording a request.
    pub fn requests(&self) -> io::Result<Vec<RecordedRequest>> {
        self.requests
            .lock()
            .map(|requests| requests.clone())
            .map_err(|_| io::Error::other("upstream request recorder poisoned"))
    }

    /// Stops even an idle or partially connected server, joins it and returns requests.
    /// Drop the engine first. Unconsumed turns are an error, not synthetic successful replies.
    ///
    /// # Errors
    /// Reports HTTP/worker errors or an incompletely consumed script.
    pub fn shutdown(mut self) -> io::Result<Vec<RecordedRequest>> {
        let remaining = self.finish()?;
        if remaining != 0 {
            return Err(io::Error::other(format!(
                "{remaining} upstream turns unconsumed"
            )));
        }
        self.requests()
    }

    fn finish(&mut self) -> io::Result<usize> {
        self.stopping.store(true, Ordering::Release);
        match self.worker.take() {
            Some(worker) => worker
                .join()
                .map_err(|_| io::Error::other("upstream worker panicked"))?,
            None => Ok(0),
        }
    }
}

impl Drop for ScriptedUpstream {
    fn drop(&mut self) {
        // Drop cannot return cleanup errors; callers use shutdown to assert the server result.
        let _cleanup_result = self.finish();
    }
}

fn serve(
    listener: TcpListener,
    turns: &Mutex<VecDeque<Turn>>,
    requests: &Mutex<Vec<RecordedRequest>>,
    stopping: &AtomicBool,
) -> io::Result<usize> {
    let mut sequence = 0u64;
    while !stopping.load(Ordering::Acquire) {
        match listener.accept() {
            Ok((mut stream, _)) => {
                let request = match http::read_request(&mut stream, stopping) {
                    Ok(Some(request)) => request,
                    Ok(None) => continue,
                    Err(_) if stopping.load(Ordering::Acquire) => break,
                    Err(error) => return Err(error),
                };
                let model = request
                    .body
                    .get("model")
                    .and_then(Value::as_str)
                    .unwrap_or("test-model")
                    .to_owned();
                let valid = request.method == "POST" && request.path == "/v1/chat/completions";
                requests
                    .lock()
                    .map_err(|_| io::Error::other("request recorder poisoned"))?
                    .push(request);
                if !valid {
                    http::error(&mut stream, 404, None, "expected POST /v1/chat/completions")?;
                    return Err(io::Error::other("unexpected upstream route"));
                }
                let turn = turns
                    .lock()
                    .map_err(|_| io::Error::other("upstream script poisoned"))?
                    .pop_front();
                let Some(turn) = turn else {
                    http::error(&mut stream, 409, None, "script exhausted")?;
                    return Err(io::Error::other("upstream script exhausted"));
                };
                sequence += 1;
                http::respond(&mut stream, turn, &model, sequence)?;
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(5));
            }
            Err(error) => return Err(error),
        }
    }
    Ok(turns
        .lock()
        .map_err(|_| io::Error::other("upstream script poisoned"))?
        .len())
}

fn validate_turns(turns: &[Turn]) -> io::Result<()> {
    if turns
        .iter()
        .any(|turn| matches!(turn, Turn::HttpError { status, .. } if !(400..=599).contains(status)))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "expected HTTP error status",
        ));
    }
    Ok(())
}
