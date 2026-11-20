//! A03/A04: official adapters, real MCP requests, project-scoped execution consent.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "real-process acceptance assertions"
)]
mod common;

use autocraft_core::*;
use autocraft_testkit::{RecordedRequest, ScriptedUpstream, Turn};
use common::*;
use serde_json::{Value, json};
use std::{collections::BTreeSet, fs, path::PathBuf};

fn names(request: &RecordedRequest) -> BTreeSet<String> {
    request.body["tools"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|tool| tool["function"]["name"].as_str())
        .map(str::to_owned)
        .collect()
}

fn mode_matrix(
    app: &App,
    upstream: &ScriptedUpstream,
    project: &ProjectId,
    backend: BackendKind,
) -> Value {
    // Exercise the public full-catalog channel on both sides of the mode switch. This avoids
    // conflating per-connection lazy toolset selection with the independent Clean-room gate.
    let options = ChannelOptions {
        all_tools: true,
        async_long_tools: false,
    };
    app.services
        .analysis
        .set_mode(project, AnalysisMode::CleanRoom)
        .unwrap();
    upstream
        .push(vec![Turn::Text("Clean-room catalog observed".into())])
        .unwrap();
    let clean = app.session_with_options(project, backend, options);
    clean.turn(
        "Acknowledge available capabilities without executing tools.",
        || {},
    );
    let clean_request = upstream.requests().unwrap().pop().unwrap();
    let clean_names = names(&clean_request);
    let catalog = app.services.analysis.tools(project).unwrap();
    let mut expected_clean = BTreeSet::new();
    for tool in &catalog {
        let wire = clean_names.iter().find(|name| name.ends_with(&tool.id));
        if ca_engine::clean_room::allows(&tool.id) {
            expected_clean.insert(
                wire.unwrap_or_else(|| panic!("Clean-room tool absent: {}", tool.id))
                    .clone(),
            );
        } else {
            assert!(
                wire.is_none(),
                "Deep-only tool exposed in Clean room: {}",
                tool.id
            );
        }
    }
    assert_eq!(
        clean_names
            .iter()
            .filter(|name| name.contains("autocraft"))
            .cloned()
            .collect::<BTreeSet<_>>(),
        expected_clean
    );
    let clean_session = clean.session.snapshot().id;
    drop(clean);
    app.services
        .analysis
        .set_mode(project, AnalysisMode::Deep)
        .unwrap();
    assert_eq!(
        app.services.projects.open(project).unwrap().mode,
        AnalysisMode::Deep
    );
    // New native process/session, not a forged tools/list or mutation of the previous request.
    upstream
        .push(vec![Turn::Text(
            "Deep catalog observed after restart".into(),
        )])
        .unwrap();
    let deep = app.session_with_options(project, backend, options);
    assert_ne!(deep.session.snapshot().id, clean_session);
    deep.turn(
        "Acknowledge the restarted Deep capabilities without executing tools.",
        || {},
    );
    let deep_request = upstream.requests().unwrap().pop().unwrap();
    let deep_names = names(&deep_request);
    let expected_deep = catalog
        .iter()
        .map(|tool| tool_name(&deep_request, &tool.id))
        .collect::<BTreeSet<_>>();
    assert_eq!(
        deep_names
            .iter()
            .filter(|name| name.contains("autocraft"))
            .cloned()
            .collect::<BTreeSet<_>>(),
        expected_deep
    );
    assert!(expected_clean.is_subset(&expected_deep));
    assert!(expected_deep.len() > expected_clean.len());
    assert_no_auth(
        &[clean_request, deep_request],
        &[&deep.token],
        &deep.session.snapshot(),
    );
    json!({"backend":backend_key(backend),"clean_tools":expected_clean,"deep_tools":expected_deep,
        "fresh_deep_session":true,"saved_deep_mode":true,"channel_all_tools":true})
}

fn marker_args(project: &ProjectId, marker: &std::path::Path, value: &str) -> Value {
    let executable = PathBuf::from(std::env::var_os("SystemRoot").unwrap())
        .join("System32/WindowsPowerShell/v1.0/powershell.exe");
    json!({"executable":executable,"arguments":["-NoProfile","-Command",format!(
        "[IO.File]::WriteAllText('{}','{value}')",marker.display().to_string().replace('\'', "''"))],
        "working_directory":project.as_str(),"timeout_ms":5000,"idle_timeout_ms":5000,"settle_ms":100})
}

fn has_consent_error(value: &Value) -> bool {
    match value {
        Value::Object(object) => {
            object
                .get("error")
                .is_some_and(|error| error["code"] == "consent_required")
                || object.values().any(has_consent_error)
        }
        Value::Array(values) => values.iter().any(has_consent_error),
        Value::String(text) => {
            serde_json::from_str::<Value>(text.strip_prefix("Error: ").unwrap_or(text))
                .ok()
                .is_some_and(|value| has_consent_error(&value))
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => false,
    }
}

fn consent_matrix(
    app: &App,
    upstream: &ScriptedUpstream,
    project: &ProjectId,
    backend: BackendKind,
) -> Value {
    let options = ChannelOptions {
        all_tools: true,
        async_long_tools: false,
    };
    let other_provider = app
        .services
        .project_prefs
        .get(project)
        .unwrap()
        .model
        .unwrap()
        .provider;
    let other = app.project("Consent must not leak", backend, &other_provider, MODELS[0]);
    upstream
        .push(vec![Turn::Text("Consent session ready".into())])
        .unwrap();
    let mut session = app.session_with_options(project, backend, options);
    session.turn("Acknowledge the bounded execution capabilities.", || {});
    let mut runtime = tool_name(
        upstream.requests().unwrap().last().unwrap(),
        "runtime_walkthrough_record",
    );
    let marker = PathBuf::from(project.as_str()).join("mode-consent-marker.txt");
    let cases = [
        ("refuse", false, false, true),
        ("allow-once", true, false, true),
        ("ask-again", false, false, true),
        ("always", true, true, true),
        ("remembered", true, false, false),
        ("reopened", true, false, false),
    ];
    let mut observations = Vec::new();
    for (stage, allow, always, expects_card) in cases {
        if marker.exists() {
            fs::remove_file(&marker).unwrap();
        }
        if stage == "reopened" {
            drop(session);
            app.services.analysis.close(project).unwrap();
            app.services.analysis.open(project).unwrap();
            upstream
                .push(vec![Turn::Text("Reopened project ready".into())])
                .unwrap();
            session = app.session_with_options(project, backend, options);
            session.turn("Acknowledge the reopened project.", || {});
            runtime = tool_name(
                upstream.requests().unwrap().last().unwrap(),
                "runtime_walkthrough_record",
            );
        }
        let before = upstream.requests().unwrap().len();
        upstream
            .push(vec![
                tool_call(&runtime, marker_args(project, &marker, stage)),
                Turn::Text("Consent outcome consumed".into()),
            ])
            .unwrap();
        let mut cards = BTreeSet::new();
        session.turn("Run the single bounded local marker command.", || {
            for card in app
                .services
                .inbox
                .pending(project)
                .unwrap()
                .into_iter()
                .filter(|card| card.kind == InboxKind::Approval)
            {
                assert!(
                    expects_card,
                    "Remembered project consent unexpectedly asked again"
                );
                assert!(!marker.exists(), "Execution occurred before approval");
                if cards.insert(card.id.to_string()) {
                    app.services
                        .inbox
                        .answer(
                            project,
                            &card.id,
                            InboxAnswer::Approval {
                                allow,
                                always,
                                text: None,
                            },
                        )
                        .unwrap();
                }
            }
        });
        assert_eq!(
            cards.len(),
            usize::from(expects_card),
            "Unexpected card count for {stage}"
        );
        assert_eq!(
            marker.exists(),
            allow,
            "Incorrect execution side effect for {stage}"
        );
        if allow {
            assert_eq!(fs::read_to_string(&marker).unwrap(), stage);
        }
        let result = results(&upstream.requests().unwrap()[before..]);
        if !allow {
            assert!(
                result.iter().any(has_consent_error),
                "Refusal must return typed consent_required: {result:?}"
            );
        }
        observations.push(json!({"stage":stage,"card_ids":cards,"marker_created":marker.exists(),"results":result}));
    }
    drop(session);
    upstream
        .push(vec![Turn::Text("Other project ready".into())])
        .unwrap();
    let other_session = app.session_with_options(&other, backend, options);
    other_session.turn("Acknowledge independent project capabilities.", || {});
    let other_runtime = tool_name(
        upstream.requests().unwrap().last().unwrap(),
        "runtime_walkthrough_record",
    );
    let other_marker = PathBuf::from(other.as_str()).join("mode-consent-marker.txt");
    let before = upstream.requests().unwrap().len();
    upstream
        .push(vec![
            tool_call(
                &other_runtime,
                marker_args(&other, &other_marker, "must-not-run"),
            ),
            Turn::Text("Other project refusal consumed".into()),
        ])
        .unwrap();
    let mut other_cards = BTreeSet::new();
    other_session.turn(
        "Run the bounded local marker only after project approval.",
        || {
            for card in app
                .services
                .inbox
                .pending(&other)
                .unwrap()
                .into_iter()
                .filter(|card| card.kind == InboxKind::Approval)
            {
                assert!(!other_marker.exists());
                if other_cards.insert(card.id.to_string()) {
                    app.services
                        .inbox
                        .answer(
                            &other,
                            &card.id,
                            InboxAnswer::Approval {
                                allow: false,
                                always: false,
                                text: None,
                            },
                        )
                        .unwrap();
                }
            }
        },
    );
    assert_eq!(
        other_cards.len(),
        1,
        "Always consent leaked across projects"
    );
    assert!(!other_marker.exists());
    let other_results = results(&upstream.requests().unwrap()[before..]);
    assert!(other_results.iter().any(has_consent_error));
    json!({"backend":backend_key(backend),"observations":observations,
        "other_project":{"card_ids":other_cards,"marker_created":false,"results":other_results}})
}

#[test]
#[ignore = "A03/A04: installed official engines from AUTOCRAFT_T26_SPEC; loopback only"]
fn official_mode_and_project_consent_matrix() {
    let logging = autocraft_testkit::Isolation::new().unwrap();
    let log = std::sync::Arc::new(autocraft_observability::init(&logging.data_dir()).unwrap());
    drop(std::panic::take_hook());
    let app = App::new(log, launches());
    let upstream = ScriptedUpstream::start(Vec::new()).unwrap();
    let provider = app.provider(&upstream);
    let mut observed = Vec::new();
    for backend in BackendKind::ALL {
        let project = app.project("Mode and consent", backend, &provider, MODELS[0]);
        app.services.analysis.open(&project).unwrap();
        observed.push(json!({"mode":mode_matrix(&app,&upstream,&project,backend),
            "consent":consent_matrix(&app,&upstream,&project,backend)}));
        app.services.analysis.close(&project).unwrap();
    }
    app.services.shutdown();
    upstream.shutdown().unwrap();
    write_evidence(
        "mode-consent-official",
        &json!({"version":1,"scenarios":["A03","A04"],"observed":observed}),
    );
}
