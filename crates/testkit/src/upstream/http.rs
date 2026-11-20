//! Bounded HTTP/1.1 input and OpenAI Chat SSE output for the loopback fixture.

use std::{
    io::{self, Read, Write},
    net::TcpStream,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

use serde_json::{Value, json};

use super::{RecordedRequest, Turn};

const MAX_HEADERS: usize = 64 * 1024;
const MAX_BODY: usize = 8 * 1024 * 1024;

pub(super) fn read_request(
    stream: &mut TcpStream,
    stopping: &AtomicBool,
) -> io::Result<Option<RecordedRequest>> {
    stream.set_read_timeout(Some(Duration::from_millis(50)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut reader = Input {
        stream,
        stopping,
        deadline,
    };
    let mut head = Vec::new();
    while !head.ends_with(b"\r\n\r\n") {
        if head.len() >= MAX_HEADERS {
            return Err(invalid("HTTP headers too large"));
        }
        let mut byte = [0];
        match reader.fill(&mut byte) {
            Ok(()) => {}
            Err(error)
                if head.is_empty()
                    && matches!(
                        error.kind(),
                        io::ErrorKind::UnexpectedEof
                            | io::ErrorKind::ConnectionReset
                            | io::ErrorKind::ConnectionAborted
                    ) =>
            {
                return Ok(None);
            }
            Err(error) => return Err(error),
        }
        head.push(byte[0]);
    }
    let head = std::str::from_utf8(&head).map_err(|_| invalid("non-UTF8 HTTP headers"))?;
    let mut lines = head.split("\r\n");
    let mut first = lines
        .next()
        .ok_or_else(|| invalid("missing request line"))?
        .split_whitespace();
    let method = first
        .next()
        .ok_or_else(|| invalid("missing method"))?
        .to_owned();
    let path = first
        .next()
        .ok_or_else(|| invalid("missing target"))?
        .to_owned();
    if first.next() != Some("HTTP/1.1") || first.next().is_some() {
        return Err(invalid("expected HTTP/1.1 request"));
    }
    let mut headers = Vec::new();
    for line in lines.filter(|line| !line.is_empty()) {
        let (name, value) = line
            .split_once(':')
            .ok_or_else(|| invalid("malformed header"))?;
        headers.push((name.to_ascii_lowercase(), value.trim().to_owned()));
    }
    let lengths: Vec<_> = headers
        .iter()
        .filter(|(name, _)| name == "content-length")
        .collect();
    let encodings: Vec<_> = headers
        .iter()
        .filter(|(name, _)| name == "transfer-encoding")
        .collect();
    if lengths.len() > 1 || encodings.len() > 1 || (!lengths.is_empty() && !encodings.is_empty()) {
        return Err(invalid("ambiguous HTTP body framing"));
    }
    let body = if let Some((_, encoding)) = encodings.first() {
        if !encoding.eq_ignore_ascii_case("chunked") {
            return Err(invalid("unsupported transfer encoding"));
        }
        reader.chunks()?
    } else {
        let length: usize = lengths
            .first()
            .ok_or_else(|| invalid("missing body length"))?
            .1
            .parse()
            .map_err(|_| invalid("invalid body length"))?;
        if length > MAX_BODY {
            return Err(invalid("HTTP body too large"));
        }
        let mut body = vec![0; length];
        reader.fill(&mut body)?;
        body
    };
    Ok(Some(RecordedRequest {
        method,
        path,
        headers,
        body: serde_json::from_slice(&body).map_err(|_| invalid("invalid request JSON"))?,
    }))
}

struct Input<'a> {
    stream: &'a mut TcpStream,
    stopping: &'a AtomicBool,
    deadline: Instant,
}

impl Input<'_> {
    fn fill(&mut self, mut bytes: &mut [u8]) -> io::Result<()> {
        while !bytes.is_empty() {
            if self.stopping.load(Ordering::Acquire) || Instant::now() >= self.deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "upstream read stopped",
                ));
            }
            match self.stream.read(bytes) {
                Ok(0) => {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "incomplete HTTP request",
                    ));
                }
                Ok(count) => bytes = &mut bytes[count..],
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock
                            | io::ErrorKind::TimedOut
                            | io::ErrorKind::Interrupted
                    ) => {}
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }

    fn line(&mut self) -> io::Result<Vec<u8>> {
        let mut line = Vec::new();
        while !line.ends_with(b"\r\n") {
            if line.len() >= MAX_HEADERS {
                return Err(invalid("HTTP line too large"));
            }
            let mut byte = [0];
            self.fill(&mut byte)?;
            line.push(byte[0]);
        }
        line.truncate(line.len() - 2);
        Ok(line)
    }

    fn chunks(&mut self) -> io::Result<Vec<u8>> {
        let mut body = Vec::new();
        loop {
            let line = self.line()?;
            let size = std::str::from_utf8(&line)
                .map_err(|_| invalid("invalid chunk size"))?
                .split(';')
                .next()
                .ok_or_else(|| invalid("missing chunk size"))?;
            let size = usize::from_str_radix(size.trim(), 16)
                .map_err(|_| invalid("invalid chunk size"))?;
            if size == 0 {
                let mut trailer_size = 0;
                loop {
                    let line = self.line()?;
                    trailer_size += line.len() + 2;
                    if trailer_size > MAX_HEADERS {
                        return Err(invalid("HTTP trailers too large"));
                    }
                    if line.is_empty() {
                        return Ok(body);
                    }
                }
            }
            if size > MAX_BODY - body.len() {
                return Err(invalid("HTTP body too large"));
            }
            let start = body.len();
            body.resize(start + size, 0);
            self.fill(&mut body[start..])?;
            let mut ending = [0; 2];
            self.fill(&mut ending)?;
            if ending != *b"\r\n" {
                return Err(invalid("invalid chunk terminator"));
            }
        }
    }
}

pub(super) fn respond(
    stream: &mut TcpStream,
    turn: Turn,
    model: &str,
    sequence: u64,
) -> io::Result<()> {
    let completion_id = format!("chatcmpl-autocraft-{sequence}");
    let (delta, finish) = match turn {
        Turn::Text(text) => (json!({"content": text}), "stop"),
        Turn::ToolCall { name, args } => (
            json!({"tool_calls": [{
                "index": 0, "id": format!("call_autocraft_{sequence}"), "type": "function",
                "function": {"name": name, "arguments": args.to_string()}
            }]}),
            "tool_calls",
        ),
        Turn::HttpError {
            status,
            retry_after,
        } => return error(stream, status, retry_after, "scripted upstream error"),
    };
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n"
    )?;
    for (delta, reason) in [
        (json!({"role": "assistant"}), Value::Null),
        (delta, Value::Null),
        (json!({}), json!(finish)),
    ] {
        let chunk = json!({
            "id": completion_id, "object": "chat.completion.chunk",
            "created": 0, "model": model,
            "choices": [{"index": 0, "delta": delta, "finish_reason": reason}]
        });
        write!(stream, "data: {chunk}\n\n")?;
        stream.flush()?;
    }
    let usage = json!({
        "id": completion_id, "object": "chat.completion.chunk",
        "created": 0, "model": model, "choices": [],
        "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2}
    });
    write!(stream, "data: {usage}\n\ndata: [DONE]\n\n")?;
    stream.flush()
}

pub(super) fn error(
    stream: &mut TcpStream,
    status: u16,
    retry_after: Option<u64>,
    message: &str,
) -> io::Result<()> {
    let body = json!({"error": {"message": message, "type": "scripted_error", "code": status}})
        .to_string();
    write!(
        stream,
        "HTTP/1.1 {status} Scripted Error\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    )?;
    if let Some(seconds) = retry_after {
        write!(stream, "Retry-After: {seconds}\r\n")?;
    }
    write!(stream, "\r\n{body}")?;
    stream.flush()
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
