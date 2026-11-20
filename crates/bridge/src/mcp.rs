//! `mcp-host` forwards opaque bytes; it never parses or manufactures JSON-RPC.
use crate::bridge::connect;
use interprocess::TryClone;
use std::{
    io::{self, Read, Write},
    sync::mpsc,
    thread,
};

/// Connects using launch-only credentials, then returns when either direction closes.
/// The subcommand process owns the two forwarding threads and exits immediately on return.
/// # Errors
/// Missing environment or a failed connection is a startup error (exit code 2 at the app boundary).
pub fn run_mcp_stdio() -> io::Result<()> {
    let pipe = std::env::var("AUTOCRAFT_PIPE")
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "缺少 AUTOCRAFT_PIPE"))?;
    let token = std::env::var("AUTOCRAFT_TOKEN")
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "缺少 AUTOCRAFT_TOKEN"))?;
    let stream = connect(&pipe, &token)?;
    let mut outgoing = stream.try_clone()?;
    let mut incoming = stream;
    let (finished, wait) = mpsc::channel();
    let sent = finished.clone();
    thread::Builder::new()
        .name("mcp-host-input".into())
        .spawn(move || {
            let result = copy_chunks(&mut io::stdin().lock(), &mut outgoing);
            if sent.send(result).is_err() {
                tracing::debug!("bridge.forwarder.already_closed");
            }
        })?;
    thread::Builder::new()
        .name("mcp-host-output".into())
        .spawn(move || {
            let result = copy_chunks(&mut incoming, &mut io::stdout().lock());
            if finished.send(result).is_err() {
                tracing::debug!("bridge.forwarder.already_closed");
            }
        })?;
    wait.recv()
        .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "分析通道转发线程已关闭"))?
}

pub(crate) fn copy_chunks(input: &mut impl Read, output: &mut impl Write) -> io::Result<()> {
    let mut buf = [0; 16 * 1024];
    loop {
        let n = match input.read(&mut buf) {
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            result => result?,
        };
        if n == 0 {
            return Ok(());
        }
        output.write_all(&buf[..n])?;
        output.flush()?;
    }
}
