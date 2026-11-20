//! Public fixture contracts: isolated ownership, actual HTTP requests and scripted wire responses.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test fixtures and assertions"
)]

use std::{
    fs,
    io::{Read, Write},
    net::TcpStream,
    time::{Duration, Instant},
};

use autocraft_testkit::{Engine, Isolation, ScriptedUpstream, Turn, pe64};
use serde_json::{Value, json};

fn connect(server: &ScriptedUpstream) -> TcpStream {
    let authority = server
        .base_url()
        .trim_start_matches("http://")
        .trim_end_matches("/v1")
        .to_owned();
    let stream = TcpStream::connect(authority).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    stream
}

fn request(server: &ScriptedUpstream, body: &Value, chunked: bool) -> String {
    let mut stream = connect(server);
    let body = body.to_string();
    write!(stream, "POST /v1/chat/completions HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer sk-test-only\r\nContent-Type: application/json\r\n").unwrap();
    if chunked {
        write!(
            stream,
            "Transfer-Encoding: chunked\r\n\r\n{:x}\r\n{body}\r\n0\r\n\r\n",
            body.len()
        )
        .unwrap();
    } else {
        write!(stream, "Content-Length: {}\r\n\r\n{body}", body.len()).unwrap();
    }
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    response
}

fn deltas(response: &str) -> Vec<Value> {
    response
        .split("\n\n")
        .filter_map(|event| event.strip_prefix("data: "))
        .filter(|event| *event != "[DONE]")
        .map(|event| serde_json::from_str(event).unwrap())
        .collect()
}

#[test]
fn isolated_dirs_are_independent_and_removed_without_touching_another_sandbox() {
    let first = Isolation::new().unwrap();
    let second = Isolation::new().unwrap();
    let root = first.root().to_owned();
    for dir in [
        first.data_dir(),
        first.cwd(),
        first.support_dir(),
        first.config_dir(Engine::Pi),
        first.config_dir(Engine::Omp),
        first.config_dir(Engine::Dsh),
    ] {
        assert!(dir.is_dir());
        assert!(dir.starts_with(&root));
        fs::write(dir.join("owned"), b"isolated").unwrap();
    }
    assert_ne!(first.config_dir(Engine::Pi), first.config_dir(Engine::Omp));
    assert_eq!(Engine::Omp.config_var(), "PI_CODING_AGENT_DIR");
    assert_eq!(Engine::Dsh.config_var(), "DSH_HOME");
    first.close().unwrap();
    assert!(!root.exists());
    assert!(second.data_dir().is_dir());
    second.close().unwrap();
}

#[test]
fn scripts_errors_tool_calls_and_text_and_records_actual_messages_in_order() {
    let server = ScriptedUpstream::start(vec![
        Turn::HttpError {
            status: 429,
            retry_after: Some(2),
        },
        Turn::HttpError {
            status: 500,
            retry_after: None,
        },
        Turn::ToolCall {
            name: "core_target_open".into(),
            args: json!({"path": "样本.exe"}),
        },
        Turn::Text("tool result received ✓".into()),
    ])
    .unwrap();
    let body = json!({"model": "fixture", "stream": true, "messages": [{"role": "user", "content": "open sample"}]});
    let response = request(&server, &body, false);
    assert!(response.starts_with("HTTP/1.1 429 "));
    assert!(response.contains("Retry-After: 2\r\n"));
    assert!(request(&server, &body, false).starts_with("HTTP/1.1 500 "));
    let response = request(&server, &body, true);
    let chunks = deltas(response.split_once("\r\n\r\n").unwrap().1);
    let call = &chunks[1]["choices"][0]["delta"]["tool_calls"][0];
    assert_eq!(call["function"]["name"], "core_target_open");
    assert_eq!(
        serde_json::from_str::<Value>(call["function"]["arguments"].as_str().unwrap()).unwrap(),
        json!({"path": "样本.exe"})
    );
    assert_eq!(chunks[2]["choices"][0]["finish_reason"], "tool_calls");
    let result = json!({"model": "fixture", "stream": true, "messages": [{"role": "tool", "tool_call_id": call["id"], "content": "evidence-1"}]});
    let response = request(&server, &result, false);
    let chunks = deltas(response.split_once("\r\n\r\n").unwrap().1);
    assert_eq!(
        chunks[1]["choices"][0]["delta"]["content"],
        "tool result received ✓"
    );
    assert_eq!(chunks[2]["choices"][0]["finish_reason"], "stop");
    assert!(response.ends_with("data: [DONE]\n\n"));
    let recorded = server.shutdown().unwrap();
    assert_eq!(
        recorded
            .iter()
            .map(|request| &request.body)
            .collect::<Vec<_>>(),
        vec![&body, &body, &body, &result]
    );
    assert_eq!(recorded[0].method, "POST");
    assert_eq!(recorded[0].path, "/v1/chat/completions");
    assert!(
        recorded[0]
            .headers
            .contains(&("authorization".into(), "Bearer sk-test-only".into()))
    );
}

#[test]
fn append_uses_tool_name_discovered_from_actual_first_request() {
    let server = ScriptedUpstream::start(vec![Turn::Text("discovered".into())]).unwrap();
    let discovery = json!({"model": "fixture", "stream": true, "messages": [],
        "tools": [{"type": "function", "function": {"name": "mcp__actual_server__open", "parameters": {"type": "object"}}}]});
    assert!(request(&server, &discovery, false).contains("discovered"));
    let recorded = server.requests().unwrap();
    let tool_name = recorded[0].body["tools"][0]["function"]["name"]
        .as_str()
        .unwrap();
    assert!(
        server
            .push(vec![Turn::HttpError {
                status: 200,
                retry_after: None
            }])
            .is_err()
    );
    server
        .push(vec![Turn::ToolCall {
            name: tool_name.into(),
            args: json!({"path": "actual.exe"}),
        }])
        .unwrap();
    let next = json!({"model": "fixture", "stream": true, "messages": [{"role": "user", "content": "open actual.exe"}]});
    let response = request(&server, &next, false);
    let chunks = deltas(response.split_once("\r\n\r\n").unwrap().1);
    let call = &chunks[1]["choices"][0]["delta"]["tool_calls"][0];
    assert_eq!(call["function"]["name"], tool_name);
    assert_eq!(call["function"]["arguments"], "{\"path\":\"actual.exe\"}");
    let recorded = server.shutdown().unwrap();
    assert_eq!(recorded[0].body, discovery);
    assert_eq!(recorded[1].body, next);
}

#[test]
fn shutdown_joins_even_with_no_request_or_a_partial_request() {
    ScriptedUpstream::start(vec![]).unwrap().shutdown().unwrap();
    let server = ScriptedUpstream::start(vec![]).unwrap();
    let mut partial = connect(&server);
    partial
        .write_all(b"POST /v1/chat/completions HTTP/1.1\r\n")
        .unwrap();
    let started = Instant::now();
    server.shutdown().unwrap();
    assert!(started.elapsed() < Duration::from_secs(3));
    let server = ScriptedUpstream::start(vec![Turn::Text("unconsumed".into())]).unwrap();
    assert!(server.shutdown().is_err());
}

#[test]
fn exhausted_script_fails_instead_of_repeating_a_success() {
    let server = ScriptedUpstream::start(vec![]).unwrap();
    let body = json!({"model": "fixture", "stream": true, "messages": []});
    assert!(request(&server, &body, false).starts_with("HTTP/1.1 409 "));
    assert!(server.shutdown().is_err());
}

#[test]
fn unused_speculative_connection_does_not_consume_script_or_fail_server() {
    let server = ScriptedUpstream::start(vec![Turn::Text("real response".into())]).unwrap();
    drop(connect(&server));
    let body = json!({"model": "fixture", "stream": true, "messages": []});
    let response = request(&server, &body, false);
    assert!(response.contains("real response"));
    let recorded = server.shutdown().unwrap();
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].body, body);
}

#[test]
fn closed_partial_request_is_still_a_protocol_failure() {
    let server = ScriptedUpstream::start(vec![]).unwrap();
    let mut partial = connect(&server);
    partial
        .write_all(b"POST /v1/chat/completions HTTP/1.1\r\n")
        .unwrap();
    partial.shutdown(std::net::Shutdown::Write).unwrap();
    let mut closed = [0; 1];
    assert_eq!(partial.read(&mut closed).unwrap(), 0);
    assert_eq!(
        server.shutdown().unwrap_err().kind(),
        std::io::ErrorKind::UnexpectedEof
    );
}

#[test]
fn pe_fixture_resolves_entry_import_and_export_inside_mapped_sections() {
    let pe = pe64();
    let word = |offset| u16::from_le_bytes(pe[offset..offset + 2].try_into().unwrap());
    let dword = |offset| u32::from_le_bytes(pe[offset..offset + 4].try_into().unwrap());
    assert_eq!(&pe[..2], b"MZ");
    assert_eq!(dword(0x3c), 0x80);
    assert_eq!(&pe[0x80..0x84], b"PE\0\0");
    assert_eq!(word(0x84), 0x8664);
    assert_eq!(word(0x98), 0x20b);
    let sections = 0x98 + usize::from(word(0x84 + 16));
    let file_offset = |rva: u32| {
        (0..usize::from(word(0x86)))
            .find_map(|index| {
                let header = sections + index * 40;
                let start = dword(header + 12);
                (rva >= start && rva - start < dword(header + 16))
                    .then(|| (dword(header + 20) + rva - start) as usize)
            })
            .expect("RVA maps to initialized section bytes")
    };
    let entry = file_offset(dword(0x98 + 16));
    assert_eq!(&pe[entry..entry + 4], &[0x48, 0x83, 0xec, 0x28]);
    let import = file_offset(dword(0x98 + 112 + 8));
    let dll = file_offset(dword(import + 12));
    assert_eq!(&pe[dll..dll + 11], b"msvcrt.dll\0");
    let lookup = file_offset(dword(import));
    let name = file_offset(dword(lookup)) + 2;
    assert_eq!(&pe[name..name + 5], b"puts\0");
    let export = file_offset(dword(0x98 + 112));
    let names = file_offset(dword(export + 32));
    let name = file_offset(dword(names));
    assert_eq!(&pe[name..name + 7], b"helper\0");
    let functions = file_offset(dword(export + 28));
    let helper = file_offset(dword(functions));
    assert_eq!(&pe[helper..helper + 6], &[0xb8, 42, 0, 0, 0, 0xc3]);
}
