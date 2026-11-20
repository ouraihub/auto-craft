//! P03: real OMP approval/steering/retry/todo probes against a loopback scripted model.
//! Run ignored tests with AUTOCRAFT_SMOKE_OMP=executable|arg and --nocapture.
//! JSON fixtures are observations written only after real frames/requests arrive, never mock echoes.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "real-engine probe assertions"
)]
#![allow(
    clippy::print_stdout,
    reason = "ignored probes print observed capabilities"
)]

#[path = "common/omp_probe.rs"]
mod omp_probe;

use std::time::Instant;

use autocraft_testkit::{Isolation, ScriptedUpstream, Turn};
use omp_probe::{Omp, WAIT, fixture, is_dialog, tools};
use serde_json::{Value, json};

const NOTE: &str = "AUTOCRAFT_STEERING_NOTE_PLACEHOLDER";
const HOOK_MARK: &str = "autocraft:tool_call:";

#[test]
#[ignore = "P03/O02: needs real AUTOCRAFT_SMOKE_OMP; records actual approval frames"]
fn omp_o02_always_ask_tool_frames() {
    for (label, tool, args) in [
        (
            "write",
            "write",
            json!({"path": "probe-write.txt", "content": "PROBE_CONTENT_PLACEHOLDER\n"}),
        ),
        (
            "edit",
            "edit",
            json!({"path": "probe-edit.txt", "old_string": "PROBE_BEFORE_PLACEHOLDER", "new_string": "PROBE_AFTER_PLACEHOLDER"}),
        ),
        (
            "shell",
            "bash",
            json!({"command": "echo PROBE_SHELL_PLACEHOLDER"}),
        ),
        // A URL read is OMP's built-in webpage-reading tool; deny before network I/O.
        (
            "web",
            "read",
            json!({"path": "http://127.0.0.1:9/PROBE_WEB_PLACEHOLDER"}),
        ),
    ] {
        let home = Isolation::new().unwrap();
        std::fs::write(
            home.cwd().join("probe-edit.txt"),
            "PROBE_BEFORE_PLACEHOLDER\n",
        )
        .unwrap();
        let upstream = ScriptedUpstream::start(vec![
            Turn::ToolCall {
                name: tool.into(),
                args: args.clone(),
            },
            Turn::Text("PROBE_TOOL_TURN_FINISHED_PLACEHOLDER".into()),
        ])
        .unwrap();
        let base_url = upstream.base_url();
        let mut omp = Omp::start(&home, &base_url, "read,write,edit,bash", "");
        let first_frame = omp.frames.len();
        let id = omp.send(json!({"type": "prompt", "message": "Exercise the requested probe tool once; its approval will be denied."}));
        let outcome = omp.finish_turn(&id, Instant::now() + WAIT);
        let frames = &omp.frames[first_frame..];
        let approvals: Vec<&Value> = frames.iter().filter(|frame| is_dialog(frame)).collect();
        let hook = frames.iter().any(|frame| {
            frame["type"] == "extension_ui_request"
                && frame["method"] == "notify"
                && frame["message"].as_str().is_some_and(|message| {
                    message.strip_prefix(HOOK_MARK).is_some_and(|payload| {
                        let event: Value = serde_json::from_str(payload).unwrap();
                        event["toolName"] == tool && event["input"] == args
                    })
                })
        });
        fixture(
            &format!("approval-{label}"),
            &json!(approvals),
            &home,
            &base_url,
        );
        fixture(
            &format!("approval-{label}-turn"),
            &json!(frames),
            &home,
            &base_url,
        );
        println!(
            "O02 {label}: extension_ui_request={} tool_call_hook={} outcome={}",
            if approvals.is_empty() {
                "absent"
            } else {
                "present"
            },
            if hook { "present" } else { "absent" },
            outcome
        );
        assert_eq!(outcome["status"], "completed", "{outcome}");
        let requests = upstream.requests().unwrap();
        assert_eq!(
            requests.len(),
            2,
            "real engine must consume the tool result in its next model request"
        );
        assert!(
            tools(&requests[0]).iter().any(|name| name == tool),
            "tool must actually be advertised"
        );
        let execution_seen = frames.iter().any(|frame| {
            frame["toolName"] == tool
                && matches!(
                    frame["type"].as_str(),
                    Some("tool_execution_start" | "tool_execution_end")
                )
        });
        assert!(
            execution_seen || hook,
            "actual model-issued tool invocation must reach OMP"
        );
        let result_messages: Vec<&Value> = requests[1].body["messages"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|message| message["role"] == "tool")
            .collect();
        assert!(
            !result_messages.is_empty(),
            "actual tool result must reach next request"
        );
        fixture(
            &format!("approval-{label}-result"),
            &json!({
                "tool": tool, "tool_call_hook": hook, "approval_request_present": !approvals.is_empty(),
                "result_messages": result_messages,
            }),
            &home,
            &base_url,
        );
        // A captured denial must stop mutation; missing approval remains an explicit observation.
        if !approvals.is_empty() {
            assert!(!home.cwd().join("probe-write.txt").exists());
            assert_eq!(
                std::fs::read_to_string(home.cwd().join("probe-edit.txt")).unwrap(),
                "PROBE_BEFORE_PLACEHOLDER\n"
            );
        }
        omp.close();
        upstream.shutdown().unwrap();
        home.close().unwrap();
    }
}

#[test]
#[ignore = "P03/O03: needs real AUTOCRAFT_SMOKE_OMP; steers while an approval holds the turn"]
fn omp_o03_steer_reaches_next_model_request() {
    let home = Isolation::new().unwrap();
    let upstream = ScriptedUpstream::start(vec![
        Turn::ToolCall {
            name: "write".into(),
            args: json!({"path": "probe-steer.txt", "content": "PROBE_PLACEHOLDER"}),
        },
        Turn::Text("PROBE_STEER_FINISHED_PLACEHOLDER".into()),
    ])
    .unwrap();
    let base_url = upstream.base_url();
    let mut omp = Omp::start(&home, &base_url, "read,write", "");
    let prompt = omp
        .send(json!({"type": "prompt", "message": "Make one write call for the steering probe."}));
    let deadline = Instant::now() + WAIT;
    let approval = omp.until(deadline, is_dialog);
    assert_eq!(
        upstream.requests().unwrap().len(),
        1,
        "approval keeps the first turn in flight"
    );
    let steer = omp.send(json!({"type": "steer", "message": NOTE}));
    let response = omp.until(deadline, |frame| {
        frame["type"] == "response" && frame["id"] == steer
    });
    omp.deny(&approval);
    let outcome = omp.finish_turn(&prompt, deadline);
    let requests = upstream.requests().unwrap();
    assert_eq!(
        requests.len(),
        2,
        "probe must see the request immediately after the held tool"
    );
    let next_messages = &requests[1].body["messages"];
    let seen =
        next_messages.as_array().unwrap().iter().any(|message| {
            message["role"] == "user" && message["content"].to_string().contains(NOTE)
        });
    let supported = response["success"] == true && seen;
    println!(
        "O03 steer: {} (command_success={}, next_model_request_sees_note={seen})",
        if supported { "present" } else { "absent" },
        response["success"]
    );
    fixture(
        "steer",
        &json!({
            "response": response, "outcome": outcome, "next_request_messages": next_messages,
            "command_success": response["success"], "next_request_sees_note": seen,
            "capability": if supported { "present" } else { "absent" },
            "frames": omp.frames,
        }),
        &home,
        &base_url,
    );
    // A successful acknowledgment without next-request delivery is explicitly absent, not support.
    assert_eq!(outcome["status"], "completed", "{outcome}");
    assert!(!home.cwd().join("probe-steer.txt").exists());
    omp.close();
    upstream.shutdown().unwrap();
    home.close().unwrap();
}

#[test]
#[ignore = "P03/O04: needs real AUTOCRAFT_SMOKE_OMP; observes HTTP429 retry frames"]
fn omp_o04_http429_retry_frames() {
    let home = Isolation::new().unwrap();
    let upstream = ScriptedUpstream::start(vec![
        Turn::HttpError {
            status: 429,
            retry_after: Some(0),
        },
        Turn::Text("PROBE_RETRY_RECOVERED_PLACEHOLDER".into()),
    ])
    .unwrap();
    let base_url = upstream.base_url();
    let mut omp = Omp::start(&home, &base_url, "read", "");
    let first_frame = omp.frames.len();
    let id = omp.send(json!({"type": "prompt", "message": "Probe the scripted local rate limit."}));
    let outcome = omp.finish_turn(&id, Instant::now() + WAIT);
    let requests = upstream.requests().unwrap();
    assert!(
        !requests.is_empty(),
        "the real model endpoint must receive HTTP429"
    );
    assert!(
        requests.len() <= 2,
        "upstream must never fabricate repeated responses"
    );
    let retry_frames: Vec<&Value> = omp.frames[first_frame..]
        .iter()
        .filter(|frame| {
            matches!(
                frame["type"].as_str(),
                Some("auto_retry_start" | "auto_retry_end")
            )
        })
        .collect();
    println!(
        "O04 retry frames: {}; model_requests={}; outcome={}",
        if retry_frames.is_empty() {
            "absent"
        } else {
            "present"
        },
        requests.len(),
        outcome
    );
    fixture(
        "retry-http429",
        &json!({
            "http_status_script": 429, "retry_after_script": 0, "requests_received": requests.len(),
            "retry_frames_present": !retry_frames.is_empty(), "retry_frames": retry_frames,
            "outcome": outcome, "frames": omp.frames[first_frame..],
        }),
        &home,
        &base_url,
    );
    if requests.len() == 2 {
        assert_eq!(outcome["status"], "completed", "{outcome}");
    } else {
        assert_eq!(
            outcome["status"], "error",
            "absence must be a real failed prompt"
        );
    }
    omp.close();
    let shutdown = upstream.shutdown();
    if requests.len() == 2 {
        shutdown.unwrap();
    } else {
        let error = shutdown.expect_err("the unused recovery turn must be reported, not swallowed");
        println!("O04 recovery absent; shutdown reports unused script: {error}");
    }
    home.close().unwrap();
}

#[test]
#[ignore = "P03/O06: needs real AUTOCRAFT_SMOKE_OMP; compares actual model tools before/after CLI exclusion"]
fn omp_o06_todo_ids_and_disable_method() {
    for (label, selected_tools) in [("enabled", "read,todo"), ("cli-disabled", "read")] {
        let home = Isolation::new().unwrap();
        let upstream =
            ScriptedUpstream::start(vec![Turn::Text("PROBE_TOOL_LIST_PLACEHOLDER".into())])
                .unwrap();
        let base_url = upstream.base_url();
        let mut omp = Omp::start(&home, &base_url, selected_tools, "");
        let id = omp.send(json!({"type": "prompt", "message": "Reply with the probe marker without invoking tools."}));
        let outcome = omp.finish_turn(&id, Instant::now() + WAIT);
        assert_eq!(outcome["status"], "completed", "{outcome}");
        let requests = upstream.requests().unwrap();
        assert_eq!(requests.len(), 1);
        let names = tools(&requests[0]);
        assert!(
            names.iter().any(|name| name == "read"),
            "positive control: tools must be advertised"
        );
        let todo_ids: Vec<&str> = names
            .iter()
            .map(String::as_str)
            .filter(|name| name.contains("todo"))
            .collect();
        if label == "enabled" {
            assert_eq!(
                todo_ids,
                vec!["todo"],
                "exact built-in todo ID from real request"
            );
        } else {
            assert!(
                todo_ids.is_empty(),
                "--tools read must exclude every todo ID from the model request"
            );
        }
        println!(
            "O06 {label}: --tools {selected_tools}; todo_ids={todo_ids:?}; model_tools={names:?}"
        );
        let evidence = json!({"selection": label, "cli_tools": selected_tools, "todo_ids": todo_ids, "model_tools": names});
        fixture(&format!("todo-{label}"), &evidence, &home, &base_url);
        omp.close();
        upstream.shutdown().unwrap();
        home.close().unwrap();
    }
}
