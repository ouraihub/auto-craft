//! T26 S1/S2/S3/S5 and A06: official adapters → shipped mcp-host → Bridge → shared AnalysisHub.
//! AUTOCRAFT_T26_SPEC is {"version":1,"engines":{"pi":{"executable":"…","args":[]},
//! "omp":{"executable":"…","args":[]},"dsh":{"executable":"…","args":[]}}}.
//! These ignored tests use synthetic loopback auth only. Paid/live acceptance still needs consent.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "real-process acceptance assertions"
)]
mod common;

use autocraft_core::*;
use autocraft_testkit::{Isolation, ScriptedUpstream, Turn};
use common::*;
use serde_json::json;
use std::{fs, path::PathBuf, sync::Arc};

fn assert_tool_result(requests: &[autocraft_testkit::RecordedRequest], evidence: &str) {
    assert!(
        results(requests)
            .iter()
            .any(|result| result.to_string().contains(evidence)),
        "model must consume the genuine engine evidence id, not just receive a scripted completion"
    );
}

#[test]
#[ignore = "T26: installed pi/OMP/DSH in AUTOCRAFT_T26_SPEC; loopback only, no paid provider"]
fn s1_s2_s3_s5_shared_analysis_and_a06_project_models() {
    let logging = Isolation::new().unwrap();
    let log = Arc::new(autocraft_observability::init(&logging.data_dir()).unwrap());
    // Keep test assertion locations visible; production panic handling is unchanged.
    drop(std::panic::take_hook());
    let app = App::new(log, launches());
    let fixture = app.home.cwd().join("sample.exe");
    fs::write(&fixture, autocraft_testkit::pe64()).unwrap();
    let upstream = ScriptedUpstream::start(vec![Turn::Text("discovery complete".into())]).unwrap();
    let provider = app.provider(&upstream);
    let project = app.project("Shared analysis", BackendKind::Pi, &provider, MODELS[0]);
    app.services.analysis.open(&project).unwrap();
    let mut retained = Vec::<EvidenceId>::new();
    let mut observations = Vec::new();
    let mut tokens = Vec::new();

    for backend in BackendKind::ALL {
        let prefs = app.services.project_prefs.get(&project).unwrap();
        app.services
            .project_prefs
            .set(
                &project,
                ProjectPrefChange::Backend(Some(backend)),
                prefs.revision,
            )
            .unwrap();
        if backend != BackendKind::Pi {
            upstream
                .push(vec![Turn::Text("discovery complete".into())])
                .unwrap();
        }
        let session = app.session(&project, backend);
        tokens.push(session.token.clone());
        let initial = session.turn(
            "List available analysis capabilities, without calling tools.",
            || {},
        );
        assert_eq!(initial.model.as_ref().unwrap().id, MODELS[0]);
        let requests = upstream.requests().unwrap();
        let advertised = requests.last().unwrap();
        let open = tool_name(advertised, "core_target_open");
        let triage = tool_name(advertised, "core_target_triage");
        let enable = tool_name(advertised, "enable_toolset");
        let _list = tool_name(advertised, "list_toolsets");
        for core_tool in app
            .services
            .analysis
            .tools(&project)
            .unwrap()
            .into_iter()
            .filter(|tool| tool.toolset == "core")
        {
            tool_name(advertised, &core_tool.id);
        }
        assert!(
            open.contains("autocraft"),
            "MCP server must be named autocraft"
        );
        let before = app.services.analysis.view(&project).unwrap();
        assert_eq!(
            before.mode,
            AnalysisMode::Deep,
            "native analysis requires the saved project Deep mode"
        );
        for id in &retained {
            assert!(before.ledger.iter().any(|entry| &entry.id == id));
        }
        let turn_start = requests.len();
        upstream
            .push(vec![
                tool_call(&open, json!({"path": fixture})),
                tool_call(&triage, json!({"path": fixture})),
                tool_call(&enable, json!({"toolset":"native"})),
                Turn::Text("native enabled".into()),
            ])
            .unwrap();
        session.turn(
            "Open and triage the sample, then enable native tools.",
            || {},
        );
        let view = app.services.analysis.view(&project).unwrap();
        assert_eq!(view.targets.len(), 1);
        assert_eq!(view.targets[0].kind, "pe");
        assert_eq!(view.targets[0].size, autocraft_testkit::pe64().len() as u64);
        let triage_id = view
            .ledger
            .iter()
            .find(|entry| entry.tool == "core_target_triage")
            .unwrap()
            .id
            .clone();
        let evidence = app
            .services
            .analysis
            .evidence(&project, &triage_id)
            .unwrap();
        assert!(
            evidence["result"]["plain_english"]
                .as_str()
                .unwrap()
                .contains("Windows")
        );
        assert_tool_result(
            &upstream.requests().unwrap()[turn_start..],
            triage_id.as_str(),
        );
        retained.push(triage_id);

        let refreshed = upstream.requests().unwrap();
        write_evidence(
            "analysis-native-discovery",
            &json!({
                "backend": backend_key(backend),
                "tools": refreshed.last().unwrap().body["tools"].as_array().unwrap().iter()
                    .map(|tool| tool["function"]["name"].clone()).collect::<Vec<_>>(),
                "results": results(&refreshed[turn_start..]),
            }),
        );
        let native = await_tool(&upstream, &session, "native_functions_list");
        upstream
            .push(vec![
                tool_call(&native, json!({"limit":50})),
                tool_call(&enable, json!({"toolset":"runtime"})),
                Turn::Text("functions inspected".into()),
            ])
            .unwrap();
        session.turn("List native functions, then enable runtime tools.", || {});
        let view = app.services.analysis.view(&project).unwrap();
        let functions_id = view
            .ledger
            .iter()
            .find(|entry| entry.tool == "native_functions_list")
            .unwrap()
            .id
            .clone();
        let functions = app
            .services
            .analysis
            .evidence(&project, &functions_id)
            .unwrap();
        assert!(
            functions["result"]["functions"]
                .as_array()
                .unwrap()
                .iter()
                .any(|function| {
                    function.to_string().contains("helper")
                        || function.to_string().contains("140001000")
                }),
            "real PE entry/helper must be observable in function inventory"
        );
        retained.push(functions_id);
        assert_tool_result(
            &upstream.requests().unwrap()[refreshed.len()..],
            retained.last().unwrap().as_str(),
        );

        let runtime = await_tool(&upstream, &session, "runtime_walkthrough_record");
        let marker =
            PathBuf::from(project.as_str()).join(format!("consent-{}.txt", backend_key(backend)));
        #[cfg(windows)]
        let (executable, arguments) = (
            PathBuf::from(std::env::var_os("SystemRoot").unwrap())
                .join("System32/WindowsPowerShell/v1.0/powershell.exe")
                .display()
                .to_string(),
            vec![
                "-NoProfile".into(),
                "-Command".into(),
                format!(
                    "[IO.File]::WriteAllText('{}','consent-granted')",
                    marker.display().to_string().replace('\'', "''")
                ),
            ],
        );
        #[cfg(not(windows))]
        let (executable, arguments) = (
            "/bin/sh".to_owned(),
            vec![
                "-c".to_owned(),
                format!("printf consent-granted > '{}'", marker.display()),
            ],
        );
        let args = json!({"executable":executable,"arguments":arguments,"working_directory":project.as_str(),
            "timeout_ms":5000,"idle_timeout_ms":5000,"settle_ms":100});
        let mut approvals = Vec::new();
        for allow in [false, true] {
            upstream
                .push(vec![
                    tool_call(&runtime, args.clone()),
                    Turn::Text("consent outcome consumed".into()),
                ])
                .unwrap();
            let before = upstream.requests().unwrap().len();
            let mut answered = false;
            session.turn("Run the bounded consent check once.", || {
                if answered {
                    return;
                }
                if let Some(card) = app
                    .services
                    .inbox
                    .pending(&project)
                    .unwrap()
                    .into_iter()
                    .find(|card| card.kind == InboxKind::Approval)
                {
                    assert!(!marker.exists(), "execution must wait for user approval");
                    app.services
                        .inbox
                        .answer(
                            &project,
                            &card.id,
                            InboxAnswer::Approval {
                                allow,
                                always: false,
                                text: None,
                            },
                        )
                        .unwrap();
                    approvals.push(card.id);
                    answered = true;
                }
            });
            assert!(
                answered,
                "consent tool must produce actual inbox approval card"
            );
            let requests = upstream.requests().unwrap();
            let returned = results(&requests[before..])
                .into_iter()
                .map(|value| value.to_string())
                .collect::<String>();
            write_evidence(
                "analysis-consent-result",
                &json!({"backend":backend_key(backend),"allow":allow,"result":returned}),
            );
            if allow {
                assert!(
                    marker.exists(),
                    "approved execution must create its marker; actual tool result: {returned}"
                );
                assert!(
                    fs::read_to_string(&marker)
                        .unwrap()
                        .contains("consent-granted")
                );
                let view = app.services.analysis.view(&project).unwrap();
                assert!(
                    view.ledger
                        .iter()
                        .any(|entry| entry.tool == "runtime_walkthrough_record")
                );
            } else {
                assert!(!marker.exists());
                assert!(
                    returned.contains("consent_required"),
                    "refusal must return typed consent_required to the real model"
                );
            }
        }
        if backend == BackendKind::Dsh {
            #[cfg(windows)]
            {
                upstream
                    .push(vec![Turn::Text("async session ready".into())])
                    .unwrap();
                let long_session = app.session_with_options(
                    &project,
                    backend,
                    app.services.channel_options(backend),
                );
                long_session.turn("Acknowledge the available tools.", || {});
                let long_enable = tool_name(
                    upstream.requests().unwrap().last().unwrap(),
                    "enable_toolset",
                );
                upstream
                    .push(vec![
                        tool_call(&long_enable, json!({"toolset":"runtime"})),
                        Turn::Text("runtime ready".into()),
                    ])
                    .unwrap();
                long_session.turn("Enable runtime observation capabilities.", || {});
                let long_runtime =
                    await_tool(&upstream, &long_session, "runtime_walkthrough_record");
                let program = PathBuf::from(std::env::var_os("SystemRoot").unwrap())
                    .join("System32/WindowsPowerShell/v1.0/powershell.exe");
                let long_args = json!({"executable":program,"arguments":["-NoProfile","-Command","Start-Sleep -Seconds 70; Write-Output S4_COMPLETED"],"working_directory":project.as_str(),"timeout_ms":90000,"idle_timeout_ms":90000,"settle_ms":100});
                upstream
                    .push(vec![
                        tool_call(&long_runtime, long_args),
                        Turn::Text("background job acknowledged".into()),
                    ])
                    .unwrap();
                let started = std::time::Instant::now();
                long_session.turn("Run the long local observation once.", || {
                    for card in app.services.inbox.pending(&project).unwrap() {
                        if card.kind == InboxKind::Approval {
                            app.services
                                .inbox
                                .answer(
                                    &project,
                                    &card.id,
                                    InboxAnswer::Approval {
                                        allow: true,
                                        always: false,
                                        text: None,
                                    },
                                )
                                .unwrap();
                        }
                    }
                });
                assert!(started.elapsed() < std::time::Duration::from_secs(60));
                let deadline = started + std::time::Duration::from_secs(100);
                let completion = loop {
                    if let Ok((_, scope, text)) = app.channels.long_tools.try_recv() {
                        assert_eq!(scope.project, project);
                        break text;
                    }
                    for card in app.services.inbox.pending(&project).unwrap() {
                        if card.kind == InboxKind::Approval {
                            app.services
                                .inbox
                                .answer(
                                    &project,
                                    &card.id,
                                    InboxAnswer::Approval {
                                        allow: true,
                                        always: false,
                                        text: None,
                                    },
                                )
                                .unwrap();
                        }
                    }
                    assert!(
                        std::time::Instant::now() < deadline,
                        "long tool completion was not delivered"
                    );
                    std::thread::sleep(std::time::Duration::from_millis(20));
                };
                assert!(started.elapsed() >= std::time::Duration::from_secs(60));
                assert!(completion.contains("S4_COMPLETED"));
                upstream
                    .push(vec![Turn::Text("long observation consumed".into())])
                    .unwrap();
                long_session.turn(&completion, || {});
                write_evidence(
                    "dsh-long-tool",
                    &json!({"elapsed_ms":started.elapsed().as_millis(),"background_ack_before_60s":true,"real_completion_consumed":true}),
                );
            }
        }
        let requests = upstream.requests().unwrap();
        assert_no_auth(&requests, &[&session.token], &session.session.snapshot());
        observations.push(json!({"backend":backend_key(backend),"native_tools_refreshed":true,
            "tools_list_requery":"not observed here; see separate P05 adapter probe evidence",
            "evidence_ids":retained.iter().map(EvidenceId::as_str).collect::<Vec<_>>(),"approval_cards":approvals.len()}));
        drop(session);
    }

    // A06: a distinct real project's saved choice must reach both native snapshot and wire request.
    let second = app.project("Other model", BackendKind::Pi, &provider, MODELS[1]);
    upstream
        .push(vec![Turn::Text("second project selected its model".into())])
        .unwrap();
    let second_session = app.session(&second, BackendKind::Pi);
    tokens.push(second_session.token.clone());
    let snapshot = second_session.turn("Reply once without tools.", || {});
    assert_eq!(snapshot.model.as_ref().unwrap().id, MODELS[1]);
    assert_eq!(
        upstream.requests().unwrap().last().unwrap().body["model"],
        MODELS[1]
    );
    assert_eq!(
        app.services
            .project_prefs
            .get(&project)
            .unwrap()
            .model
            .unwrap()
            .model_id,
        MODELS[0]
    );
    assert!(
        app.services
            .analysis
            .view(&second)
            .unwrap()
            .ledger
            .is_empty(),
        "separate projects must not share evidence"
    );
    assert_no_auth(
        &upstream.requests().unwrap(),
        &[&second_session.token],
        &snapshot,
    );
    drop(second_session);
    app.services.analysis.close(&project).unwrap();
    app.services.shutdown();
    upstream.shutdown().unwrap();
    assert_log_auth(&app.services.log, &tokens);
    write_evidence(
        "analysis",
        &json!({"version":1,"scenarios":["S1","S2","S3","S5","A06"],
        "observed":observations,"live_paid_acceptance":"pending user approval"}),
    );
}
