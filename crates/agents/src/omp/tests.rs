use std::fs;

use autocraft_core::{BackendKind, EngineRouting};
use serde_json::{Value, json};

use super::{launch_spec, withdraw::stopped_request};
use crate::{
    host,
    testing::{Fixture, SECRETS, routing_vars, sample_routing, secret_routing},
    workflow,
};

fn user(id: &str, parent: &str) -> Value {
    json!({"type": "message", "id": id, "parentId": parent, "message": {"role": "user", "content": [{"type": "text", "text": "q"}]}})
}

fn assistant(id: &str, parent: &str, stop: &str, content: Value) -> Value {
    json!({"type": "message", "id": id, "parentId": parent, "message": {"role": "assistant", "stopReason": stop, "content": content}})
}

fn thinking() -> Value {
    json!([{"type": "thinking", "thinking": "plan"}])
}

#[test]
fn a_request_stopped_before_any_answer_is_withdrawn() {
    let entries = json!([
        user("u1", "root"),
        assistant("a1", "u1", "stop", json!([{"type": "text", "text": "done"}])),
        user("u2", "a1"),
        assistant("a2", "u2", "aborted", thinking()),
        {"type": "custom", "id": "c", "parentId": "a2"},
    ]);
    assert_eq!(stopped_request(&entries, Some("c")).as_deref(), Some("u2"));
}

#[test]
fn stacked_stopped_requests_are_all_withdrawn() {
    let entries = json!([
        user("u0", "root"),
        assistant(
            "a0",
            "u0",
            "stop",
            json!([{"type": "text", "text": "done"}])
        ),
        user("u1", "a0"),
        assistant("a1", "u1", "aborted", thinking()),
        user("u2", "a1"),
        assistant("a2", "u2", "aborted", json!([])),
    ]);
    assert_eq!(stopped_request(&entries, Some("a2")).as_deref(), Some("u1"));
}

#[test]
fn a_partial_answer_or_tool_call_keeps_the_request() {
    let text =
        json!([{"type": "thinking", "thinking": "p"}, {"type": "text", "text": "half an article"}]);
    let tool = json!([{"type": "toolCall", "id": "t", "name": "write", "arguments": {}}]);
    for content in [text, tool] {
        let entries = json!([
            user("u1", "root"),
            assistant("a1", "u1", "aborted", content)
        ]);
        assert_eq!(stopped_request(&entries, Some("a1")), None);
    }
}

#[test]
fn finished_errored_or_unanswered_turns_are_left_alone() {
    let done = json!([
        user("u1", "root"),
        assistant("a1", "u1", "stop", thinking())
    ]);
    assert_eq!(stopped_request(&done, Some("a1")), None);
    let failed = json!([
        user("u1", "root"),
        assistant("a1", "u1", "error", thinking())
    ]);
    assert_eq!(stopped_request(&failed, Some("a1")), None);
    let pending = json!([user("u1", "root")]);
    assert_eq!(stopped_request(&pending, Some("u1")), None);
    assert_eq!(stopped_request(&done, None), None);
}

/// MCP credentials are inherited by the analysis stdio server, never stored in mcp.json.
#[test]
fn every_launch_has_the_same_arguments_without_routing() {
    let f = Fixture::new();
    let (extension, overlay) = (
        f.support_arg("omp-autocraft.ts"),
        f.support_arg("omp-autocraft.yml"),
    );
    for (resume, host_tools) in [(None, false), (None, true), (Some("s.jsonl"), true)] {
        let request = f.request(
            BackendKind::Omp,
            resume,
            host_tools,
            EngineRouting::default(),
        );
        let spec = launch_spec(&request).unwrap();
        let mut expected = vec![
            "--mode",
            "rpc-ui",
            "--approval-mode",
            "always-ask",
            "-e",
            extension.as_str(),
            "--config",
            overlay.as_str(),
        ];
        expected.extend(resume.map(|key| ["--session", key]).into_iter().flatten());
        assert_eq!(spec.host_args, expected);
        assert_eq!(
            spec.host_env,
            if host_tools {
                host::env(request.analysis.as_ref().unwrap())
            } else {
                Vec::new()
            }
        );
        assert_eq!(
            (spec.cwd, spec.config_var),
            (f.cwd(), "PI_CODING_AGENT_DIR")
        );
    }
    assert_eq!(
        fs::read_to_string(&overlay).unwrap(),
        workflow::omp_overlay(
            &f.request(
                BackendKind::Omp,
                Some("s.jsonl"),
                true,
                EngineRouting::default()
            ),
            Some(&f.support().join("skills"))
        )
    );
    let config = fs::read_to_string(f.config().join("mcp.json")).unwrap();
    let value: Value = serde_json::from_str(&config).unwrap();
    assert_eq!(
        value["mcpServers"]["autocraft-analysis"]["args"],
        json!(["mcp-host"])
    );
    assert!(!config.contains("token-1"));
}

#[test]
fn routing_env_is_the_whole_launch_environment() {
    let f = Fixture::new();
    let routing = secret_routing();
    let request = f.request(BackendKind::Omp, None, true, routing.clone());
    let spec = launch_spec(&request).unwrap();
    let mut expected = host::env(request.analysis.as_ref().unwrap());
    expected.extend(routing_vars(&routing));
    assert_eq!(spec.host_env, expected);
    f.assert_no_secret_written();
}

#[test]
fn routing_adds_the_extension_overlay_and_mcp_entries() {
    let f = Fixture::new();
    let routing = sample_routing();
    fs::write(
        f.config().join("mcp.json"),
        r#"{"mcpServers": {"mine": {"command": "x"}}}"#,
    )
    .unwrap();
    let request = f.request(BackendKind::Omp, None, true, routing.clone());
    let spec = launch_spec(&request).unwrap();
    let (extension, overlay) = (
        f.support_arg("omp-autocraft.ts"),
        f.support_arg("omp-autocraft.yml"),
    );
    assert_eq!(
        spec.host_args,
        [
            "--mode",
            "rpc-ui",
            "--approval-mode",
            "always-ask",
            "-e",
            extension.as_str(),
            "--config",
            overlay.as_str()
        ]
    );
    let mut expected_env = host::env(request.analysis.as_ref().unwrap());
    expected_env.extend(routing_vars(&routing));
    assert_eq!(spec.host_env, expected_env);
    let mcp: Value =
        serde_json::from_str(&fs::read_to_string(f.config().join("mcp.json")).unwrap()).unwrap();
    assert_eq!(
        mcp["mcpServers"]["mine"],
        json!({"command":"x"}),
        "host projection must preserve the user's server configuration"
    );
    f.assert_no_secret_written();
    let text = fs::read_to_string(f.config().join("mcp.json")).unwrap();
    assert!(SECRETS.iter().all(|s| !text.contains(s)));

    let resumed = EngineRouting {
        default_model: None,
        mcp: Vec::new(),
        ..routing
    };
    let request = f.request(BackendKind::Omp, Some("s.jsonl"), false, resumed);
    let spec = launch_spec(&request).unwrap();
    assert_eq!(
        spec.host_args[4..],
        [
            "-e",
            extension.as_str(),
            "--config",
            overlay.as_str(),
            "--session",
            "s.jsonl"
        ]
    );
    let mcp: Value =
        serde_json::from_str(&fs::read_to_string(f.config().join("mcp.json")).unwrap()).unwrap();
    assert_eq!(
        mcp,
        json!({"mcpServers": {"mine": {"command": "x"}}}),
        "stale autocraft- entries are removed"
    );
}

/// Without a config root `mcp.json` goes to OMP's own default dir; resolved, never written here.
#[test]
fn mcp_without_a_config_root_targets_the_omp_default_dir() {
    let home = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .unwrap();
    let dir = crate::process::config_dir(None, ".omp/agent").unwrap();
    assert_eq!(dir, std::path::PathBuf::from(home).join(".omp/agent"));
    let root = std::path::Path::new("C:/isolated");
    assert_eq!(
        crate::process::config_dir(Some(root), ".omp/agent").unwrap(),
        root
    );
}
