//! Token-authenticated, bidirectional analysis transport. No MCP interpretation lives here.
use autocraft_core::{ChannelHandler, ChannelOptions, ChatScope, Error, ErrorKind, Result};
use interprocess::TryClone;
use interprocess::local_socket::{GenericNamespaced, ListenerOptions, Stream, prelude::*};
use parking_lot::Mutex;
use std::{
    collections::HashMap,
    io::{self, BufRead, BufReader, Read, Write},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
};

pub(crate) const MAX_LINE: usize = 64 * 1024 * 1024;
const MAX_TOKEN: usize = 64 * 1024;
const MAX_CONNECTIONS: usize = 64;
#[derive(Clone)]
struct Registration {
    scope: ChatScope,
    options: ChannelOptions,
    active: Arc<AtomicBool>,
}
type Tokens = Arc<Mutex<HashMap<String, Registration>>>;

/// Process-lifetime authenticated channel listener.
pub struct Bridge {
    pipe: String,
    tokens: Tokens,
}
impl std::fmt::Debug for Bridge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Bridge").finish_non_exhaustive()
    }
}
impl Bridge {
    /// Starts accepting connections; each authenticated stream is owned by the handler.
    /// # Errors
    /// Returns an external error if the listener or accept thread cannot start.
    pub fn start(handler: Arc<dyn ChannelHandler>) -> Result<Self> {
        let pipe = format!("autocraft-{}", uuid::Uuid::new_v4().simple());
        let name = pipe
            .clone()
            .to_ns_name::<GenericNamespaced>()
            .map_err(external)?;
        let listener = ListenerOptions::new()
            .name(name)
            .create_sync()
            .map_err(external)?;
        let tokens: Tokens = Arc::new(Mutex::new(HashMap::new()));
        let accept_tokens = tokens.clone();
        let live = Arc::new(AtomicUsize::new(0));
        let span = tracing::Span::current();
        let dispatch = tracing::dispatcher::get_default(Clone::clone);
        thread::Builder::new()
            .name("analysis-bridge-accept".into())
            .spawn(move || {
                let _dispatch = tracing::dispatcher::set_default(&dispatch);
                let _entered = span.enter();
                for incoming in listener.incoming() {
                    let stream = match incoming {
                        Ok(s) => s,
                        Err(e) => {
                            tracing::warn!(error_kind = ?e.kind(), "bridge.accept.failed");
                            continue;
                        }
                    };
                    if live.fetch_add(1, Ordering::AcqRel) >= MAX_CONNECTIONS {
                        live.fetch_sub(1, Ordering::AcqRel);
                        tracing::warn!(limit = MAX_CONNECTIONS, "bridge.connection.limit");
                        continue;
                    }
                    let (tokens, handler, count) =
                        (accept_tokens.clone(), handler.clone(), live.clone());
                    let span = tracing::Span::current();
                    let dispatch = tracing::dispatcher::get_default(Clone::clone);
                    if let Err(e) = thread::Builder::new()
                        .name("analysis-bridge-connection".into())
                        .spawn(move || {
                            let _slot = Slot(count);
                            let _dispatch = tracing::dispatcher::set_default(&dispatch);
                            let _entered = span.enter();
                            if let Err(e) = serve(stream, &tokens, handler.as_ref()) {
                                tracing::debug!(error_kind = ?e.kind(), "bridge.connection.closed");
                            }
                        })
                    {
                        live.fetch_sub(1, Ordering::AcqRel);
                        tracing::warn!(error_kind = ?e.kind(), "bridge.connection.spawn_failed");
                    }
                }
            })
            .map_err(external)?;
        Ok(Self { pipe, tokens })
    }
    pub fn pipe_name(&self) -> &str {
        &self.pipe
    }
    pub fn register(&self, scope: ChatScope, options: ChannelOptions) -> String {
        let token = format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        self.tokens.lock().insert(
            token.clone(),
            Registration {
                scope,
                options,
                active: Arc::new(AtomicBool::new(true)),
            },
        );
        token
    }
    /// Invalidates this exact launch, even if another token has the same scope.
    pub fn revoke(&self, token: &str) {
        if let Some(entry) = self.tokens.lock().remove(token) {
            entry.active.store(false, Ordering::Release);
        }
    }
}
struct Slot(Arc<AtomicUsize>);
impl Drop for Slot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}
fn external(_: io::Error) -> Error {
    Error::new(ErrorKind::External, "分析通道启动失败")
}

fn serve(stream: Stream, tokens: &Tokens, handler: &dyn ChannelHandler) -> io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut token = Vec::new();
    let n = reader
        .by_ref()
        .take(MAX_TOKEN as u64 + 1)
        .read_until(b'\n', &mut token)?;
    if n == 0 || n > MAX_TOKEN || token.last() != Some(&b'\n') {
        return Ok(());
    }
    token.pop();
    let Ok(token) = std::str::from_utf8(&token) else {
        return Ok(());
    };
    let Some(entry) = tokens.lock().get(token).cloned() else {
        return Ok(());
    };
    let reader = GuardedReader {
        inner: reader,
        active: entry.active,
        line: 0,
    };
    handler.serve(
        &entry.scope,
        entry.options,
        Box::new(reader),
        Box::new(stream),
    )
}

/// Preserves every byte while limiting each request line and checking revocation on every read.
pub(crate) struct GuardedReader<R> {
    pub(crate) inner: R,
    pub(crate) active: Arc<AtomicBool>,
    pub(crate) line: usize,
}
impl<R: Read> Read for GuardedReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if !self.active.load(Ordering::Acquire) {
            return Ok(0);
        }
        let n = self.inner.read(buf)?;
        if !self.active.load(Ordering::Acquire) {
            return Ok(0);
        }
        for byte in &buf[..n] {
            self.line += 1;
            if self.line > MAX_LINE {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "analysis request exceeds line limit",
                ));
            }
            if *byte == b'\n' {
                self.line = 0;
            }
        }
        Ok(n)
    }
}

impl<R: BufRead> BufRead for GuardedReader<R> {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        if !self.active.load(Ordering::Acquire) {
            return Ok(&[]);
        }
        let bytes = self.inner.fill_buf()?;
        if !self.active.load(Ordering::Acquire) {
            return Ok(&[]);
        }
        let remaining = MAX_LINE - self.line;
        let end = bytes
            .iter()
            .position(|b| *b == b'\n')
            .map_or(bytes.len(), |i| i + 1);
        if end > remaining {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "analysis request exceeds line limit",
            ));
        }
        Ok(&bytes[..end])
    }
    fn consume(&mut self, amount: usize) {
        match self.inner.fill_buf() {
            Ok(bytes) => {
                let n = amount.min(bytes.len());
                for byte in &bytes[..n] {
                    self.line += 1;
                    if *byte == b'\n' {
                        self.line = 0;
                    }
                }
                self.inner.consume(n);
            }
            Err(error) => {
                tracing::debug!(error_kind = ?error.kind(), "bridge.reader.consume_failed")
            }
        }
    }
}

pub(crate) fn connect(pipe: &str, token: &str) -> io::Result<Stream> {
    if token.contains(['\r', '\n']) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid channel token",
        ));
    }
    let name = pipe.to_owned().to_ns_name::<GenericNamespaced>()?;
    let mut stream = Stream::connect(name)?;
    stream.write_all(token.as_bytes())?;
    stream.write_all(b"\n")?;
    stream.flush()?;
    Ok(stream)
}
