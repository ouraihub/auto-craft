//! Authenticated byte transport regressions; all sockets are isolated per test.
use crate::{
    Bridge,
    bridge::{GuardedReader, MAX_LINE, connect},
};
use autocraft_core::{ChannelHandler, ChannelOptions, ChatPurpose, ChatScope};
use std::{
    io::{self, BufRead, Read, Write},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

struct Echo;
impl ChannelHandler for Echo {
    fn serve(
        &self,
        _: &ChatScope,
        _: ChannelOptions,
        mut reader: Box<dyn BufRead + Send>,
        mut writer: Box<dyn Write + Send>,
    ) -> io::Result<()> {
        writer.write_all(b"{\"method\":\"notifications/progress\"}\n")?;
        writer.flush()?;
        loop {
            let mut bytes = Vec::new();
            if reader.read_until(b'\n', &mut bytes)? == 0 {
                return Ok(());
            }
            writer.write_all(&bytes)?;
            writer.flush()?;
        }
    }
}
fn scope() -> ChatScope {
    ChatScope {
        project: std::env::temp_dir().to_string_lossy().into_owned().into(),
        chat: "test-chat".into(),
        purpose: ChatPurpose::Analysis,
    }
}
#[test]
fn unknown_token_is_closed_without_a_response() {
    let bridge = Bridge::start(Arc::new(Echo)).unwrap();
    let mut client = connect(bridge.pipe_name(), "unknown").unwrap();
    assert_eq!(client.read(&mut [0; 1]).unwrap(), 0);
}
#[test]
fn notifications_and_requests_preserve_every_byte_and_revoke_exact_launch() {
    let bridge = Bridge::start(Arc::new(Echo)).unwrap();
    let token = bridge.register(scope(), ChannelOptions::default());
    let other = bridge.register(scope(), ChannelOptions::default());
    let mut client = connect(bridge.pipe_name(), &token).unwrap();
    let notice = b"{\"method\":\"notifications/progress\"}\n";
    let mut got = vec![0; notice.len()];
    client.read_exact(&mut got).unwrap();
    assert_eq!(got, notice);
    let request = b" {\"id\":7,\"method\":\"tools/call\",\"params\":{}} \r\n";
    client.write_all(request).unwrap();
    let mut got = vec![0; request.len()];
    client.read_exact(&mut got).unwrap();
    assert_eq!(got, request);
    bridge.revoke(&token);
    client.write_all(b"next\n").unwrap();
    assert_eq!(client.read(&mut [0; 1]).unwrap(), 0);
    let mut second = connect(bridge.pipe_name(), &other).unwrap();
    let mut got = vec![0; notice.len()];
    second.read_exact(&mut got).unwrap();
    assert_eq!(got, notice);
}
#[test]
fn sixty_four_live_connections_exclude_the_next() {
    let bridge = Bridge::start(Arc::new(Echo)).unwrap();
    let token = bridge.register(scope(), ChannelOptions::default());
    let notice = b"{\"method\":\"notifications/progress\"}\n";
    let mut clients = Vec::new();
    for _ in 0..64 {
        let mut client = connect(bridge.pipe_name(), &token).unwrap();
        let mut bytes = vec![0; notice.len()];
        client.read_exact(&mut bytes).unwrap();
        clients.push(client);
    }
    // A refused slot may close before authentication is written.
    match connect(bridge.pipe_name(), &token) {
        Ok(mut client) => assert!(matches!(client.read(&mut [0; 1]), Ok(0) | Err(_))),
        Err(error) => assert!(matches!(
            error.kind(),
            io::ErrorKind::BrokenPipe
                | io::ErrorKind::ConnectionReset
                | io::ErrorKind::ConnectionAborted
        )),
    }
    drop(clients);
}
#[test]
fn request_line_budget_and_revocation_are_enforced_without_parsing() {
    let active = Arc::new(AtomicBool::new(true));
    let mut reader = GuardedReader {
        inner: io::Cursor::new(b"ab\ncd"),
        active: active.clone(),
        line: MAX_LINE - 2,
    };
    let mut bytes = [0; 5];
    assert_eq!(
        reader.read(&mut bytes).unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
    let mut reader = GuardedReader {
        inner: io::Cursor::new(b"ab\ncd"),
        active: active.clone(),
        line: MAX_LINE - 3,
    };
    assert_eq!(reader.read(&mut bytes).unwrap(), 5);
    assert_eq!(&bytes, b"ab\ncd");
    active.store(false, Ordering::Release);
    assert_eq!(reader.read(&mut bytes).unwrap(), 0);
}
#[test]
fn stdio_copy_preserves_non_json_bytes() {
    let input = b"\0\xffraw\r\n{\"method\":\"notifications/tools/list_changed\"}\n";
    let mut output = Vec::new();
    crate::mcp::copy_chunks(&mut &input[..], &mut output).unwrap();
    assert_eq!(output, input);
}

#[test]
fn buffered_second_request_cannot_bypass_revocation() {
    let active = Arc::new(AtomicBool::new(true));
    let mut reader = GuardedReader {
        inner: io::Cursor::new(b"first\nsecond\n"),
        active: active.clone(),
        line: 0,
    };
    let mut first = Vec::new();
    reader.read_until(b'\n', &mut first).unwrap();
    assert_eq!(first, b"first\n");
    active.store(false, Ordering::Release);
    let mut second = Vec::new();
    assert_eq!(reader.read_until(b'\n', &mut second).unwrap(), 0);
    assert!(second.is_empty());
}
