//! T26 S8: every official builder writes a real static site; real build check reaches Ready.
//! Uses AUTOCRAFT_T26_SPEC version 1 (analysis_smoke.rs). No external providers or credentials.
//! This is loopback acceptance infrastructure, not the paid website/app/game acceptance run.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "real-process acceptance assertions"
)]
mod common;

use autocraft_core::*;
use autocraft_testkit::{Isolation, ScriptedUpstream, Turn};
use common::*;
use serde_json::{Value, json};
use std::{fs, path::Path, sync::Arc, thread, time::Instant};

fn write_call(request: &autocraft_testkit::RecordedRequest, path: &str, content: &str) -> Turn {
    let tool = request.body["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| tool["function"]["name"] == "write")
        .expect("official engine must advertise its actual write tool");
    let function = &tool["function"];
    let properties = &function["parameters"]["properties"];
    let path_key = if properties.get("file_path").is_some() {
        "file_path"
    } else {
        assert!(
            properties.get("path").is_some(),
            "official write schema has no supported path field"
        );
        "path"
    };
    let mut args = json!({"content": content});
    args[path_key] = json!(path);
    tool_call(function["name"].as_str().unwrap(), args)
}

fn project_show(root: &Path) -> Value {
    // The exact source API behind CLI project-show, not an independent JSON interpretation.
    let engine = ca_engine::Engine::new(ca_engine::Session::isolated(root));
    let shown = engine
        .call(
            "project_show",
            json!({"project":root}),
            ca_engine::CallOptions::new(ca_engine::Caller::Cli),
        )
        .unwrap();
    match shown {
        ca_engine::ToolOutput::Value(value) => value,
        ca_engine::ToolOutput::Evidence(evidence) => panic!(
            "project_show unexpectedly returned evidence {}",
            evidence.id
        ),
    }
}

#[test]
#[ignore = "T26 S8: official pi/OMP/DSH in AUTOCRAFT_T26_SPEC and node/npm on PATH; loopback only"]
fn s8_all_builders_create_static_site_and_persist_ready_craft_project() {
    let logging = Isolation::new().unwrap();
    let log = Arc::new(autocraft_observability::init(&logging.data_dir()).unwrap());
    drop(std::panic::take_hook());
    let app = App::new(log, launches());
    let upstream =
        ScriptedUpstream::start(vec![Turn::Text("official tools discovered".into())]).unwrap();
    let provider = app.provider(&upstream);
    app.services
        .analysis
        .set_settings(AnalysisSettingsChange {
            auto_mode: Some(true),
            ..Default::default()
        })
        .unwrap();
    let mut evidence = Vec::new();
    for backend in BackendKind::ALL {
        if backend != BackendKind::Pi {
            upstream
                .push(vec![Turn::Text("official tools discovered".into())])
                .unwrap();
        }
        let discovery = app.project(
            &format!("Discover {}", backend_key(backend)),
            backend,
            &provider,
            MODELS[0],
        );
        let discover_session = app.session(&discovery, backend);
        discover_session.turn("Reply without invoking tools.", || {});
        let advertised = upstream.requests().unwrap().last().unwrap().clone();
        write_evidence(
            &format!("builder-tools-{}", backend_key(backend)),
            &advertised.body["tools"],
        );
        drop(discover_session);

        let project = app.project(
            &format!("Static site {}", backend_key(backend)),
            backend,
            &provider,
            MODELS[0],
        );
        let root = Path::new(project.as_str());
        let app_dir = app.services.projects.open(&project).unwrap().app_dir;
        let marker = format!(
            "T26_{}_{}",
            backend_key(backend),
            root.file_name().unwrap().to_string_lossy()
        );
        let html = format!(
            "<!doctype html><html lang=\"en\"><meta charset=\"utf-8\"><title>{marker}</title><main><h1>{marker}</h1><p>Built through the official agent file tool.</p></main></html>"
        );
        let package =
            json!({"name":"autocraft-loopback-static-site","version":"1.0.0","private":true,
            "scripts":{"build":"node build.cjs"}})
            .to_string();
        let build = "const fs = require('node:fs');\nconst page = fs.readFileSync('index.html', 'utf8');\nif (!page.startsWith('<!doctype html>') || !page.includes('<h1>')) process.exit(2);\nfs.mkdirSync('dist', { recursive: true });\nfs.writeFileSync('dist/index.html', page);\nconsole.log('T26_STATIC_SITE_BUILD_OK');\n";
        let outside = app
            .home
            .cwd()
            .join(format!("outside-{}.txt", backend_key(backend)));
        let shell = advertised.body["tools"]
            .as_array()
            .unwrap()
            .iter()
            .find(|tool| {
                matches!(
                    tool["function"]["name"].as_str(),
                    Some("bash" | "powershell" | "pwsh")
                )
            })
            .expect("official shell tool must be advertised")["function"]["name"]
            .as_str()
            .unwrap()
            .to_owned();
        upstream
            .push(vec![
                write_call(&advertised, "index.html", &html),
                write_call(&advertised, "package.json", &package),
                write_call(&advertised, "build.cjs", build),
                write_call(&advertised, outside.to_str().unwrap(), "S6_OUTSIDE_ALLOWED"),
                tool_call(&shell, json!({"command":"git push origin main","description":"Verify host policy rejects publishing code"})),
                Turn::Text(
                    "The static website files are complete. Run the real build check.".into(),
                ),
            ])
            .unwrap();
        let rebuild = app
            .services
            .rebuild
            .as_ref()
            .expect("real rebuild service configured");
        assert_eq!(
            rebuild
                .start(
                    &project,
                    RunRequest {
                        message: None,
                        shown: None,
                        pictures: Vec::new(),
                        continuing: false,
                        max_rounds: Some(0),
                        backend: Some(backend)
                    }
                )
                .unwrap(),
            RunStarted::Running
        );
        let deadline = Instant::now() + LIMIT;
        let mut steps = Vec::new();
        let mut checks = Vec::new();
        let mut finished = None;
        let mut snapshots = Vec::new();
        while finished.is_none() {
            while let Ok((_span, event)) = app.channels.rebuild.try_recv() {
                match event {
                    RebuildEvent::Step {
                        project: id, step, ..
                    } if id == project => steps.push(step),
                    RebuildEvent::Checked {
                        project: id,
                        result,
                    } if id == project => checks.push(result),
                    RebuildEvent::Finished {
                        project: id,
                        ok,
                        paused,
                        ..
                    } if id == project => finished = Some((ok, paused)),
                    RebuildEvent::Agent {
                        project: id,
                        event: SessionEvent::Snapshot { snapshot },
                    } if id == project => snapshots.push(snapshot),
                    other => {
                        let _ = other;
                    }
                }
            }
            assert!(
                Instant::now() < deadline,
                "rebuild did not finish; last state {:?}",
                rebuild.status(&project).unwrap()
            );
            thread::sleep(std::time::Duration::from_millis(20));
        }
        assert_eq!(
            finished,
            Some((true, false)),
            "run must complete, not pause or report failure"
        );
        assert!(
            !steps.contains(&StepDto::Study),
            "no-target static site must skip target research"
        );
        assert!(
            steps.contains(&StepDto::Build)
                && steps.contains(&StepDto::Check)
                && steps.contains(&StepDto::Done)
        );
        assert!(
            checks.iter().any(|check| check.check == "build"
                && check.passed
                && check.note.contains("T26_STATIC_SITE_BUILD_OK")),
            "real npm build must execute, not merely report nonempty output directory"
        );
        assert_eq!(fs::read_to_string(&outside).unwrap(), "S6_OUTSIDE_ALLOWED");
        let tool_results = results(&upstream.requests().unwrap())
            .into_iter()
            .map(|result| result.to_string())
            .collect::<String>();
        assert!(
            tool_results.contains("publish") || tool_results.contains("push"),
            "publish refusal must reach the model"
        );
        assert_eq!(
            fs::read_to_string(app_dir.join("index.html")).unwrap(),
            html
        );
        assert_eq!(
            fs::read_to_string(app_dir.join("dist/index.html")).unwrap(),
            html
        );
        assert_eq!(
            app.services.projects.open(&project).unwrap().summary.status,
            ProjectStatus::Ready
        );
        assert_eq!(
            rebuild.status(&project).unwrap().status,
            ProjectStatus::Ready
        );
        let workspace = ca_engine::workspace::Workspace::open(root).unwrap();
        assert_eq!(
            workspace.manifest.status,
            ca_engine::workspace::Status::Ready
        );
        let shown = project_show(root);
        assert_eq!(shown["status"], "ready");
        assert_eq!(shown["target"]["kind"], "none");
        assert!(
            shown["checks"]
                .as_array()
                .unwrap()
                .iter()
                .any(|check| check["check"] == "build" && check["passed"] == true)
        );
        assert!(
            snapshots.iter().any(|snapshot| snapshot
                .model
                .as_ref()
                .is_some_and(|model| model.id == MODELS[0])),
            "builder native snapshot must use the project's chosen model"
        );
        assert_no_auth(
            &upstream.requests().unwrap(),
            &[],
            snapshots.last().unwrap(),
        );
        let before_page = fs::read(app_dir.join("index.html")).unwrap();
        let before_build = fs::read(app_dir.join("dist/index.html")).unwrap();
        upstream
            .push(vec![Turn::Text(
                "This project contains a static HTML page and a real Node build script.".into(),
            )])
            .unwrap();
        let answer = rebuild
            .ask(&project, "What does this project do?", &CancelFlag::new())
            .unwrap();
        assert!(answer.answer.contains("static HTML page"));
        assert_eq!(fs::read(app_dir.join("index.html")).unwrap(), before_page);
        assert_eq!(
            fs::read(app_dir.join("dist/index.html")).unwrap(),
            before_build
        );
        if backend != BackendKind::Dsh {
            let original = workspace.manifest.agent_session.as_ref().unwrap();
            fs::remove_file(original).unwrap();
            upstream
                .push(vec![Turn::Text(
                    "The restored static website remains complete.".into(),
                )])
                .unwrap();
            rebuild
                .start(
                    &project,
                    RunRequest {
                        message: Some("Confirm the existing page without edits.".into()),
                        shown: None,
                        pictures: Vec::new(),
                        continuing: true,
                        max_rounds: Some(0),
                        backend: Some(backend),
                    },
                )
                .unwrap();
            let deadline = Instant::now() + LIMIT;
            loop {
                if let Ok((
                    _,
                    RebuildEvent::Finished {
                        project: id,
                        ok,
                        paused,
                        ..
                    },
                )) = app.channels.rebuild.try_recv()
                    && id == project
                {
                    assert!(ok && !paused, "portable session resume must complete");
                    break;
                }
                assert!(
                    Instant::now() < deadline,
                    "portable session restore did not finish"
                );
                thread::sleep(std::time::Duration::from_millis(20));
            }
            let restored = ca_engine::workspace::Workspace::open(root).unwrap();
            let key = restored.manifest.agent_session.unwrap();
            assert_ne!(&key, original);
            assert!(!Path::new(&key).starts_with(root.join("builder-session")));
            assert!(Path::new(&key).is_file());
            assert_eq!(fs::read(app_dir.join("index.html")).unwrap(), before_page);
        }
        app.services
            .projects
            .set_person(
                &project,
                PersonChange::Approval {
                    kind: ActionKindDto::OutsideFiles,
                    approval: Some(ApprovalDto::Ask),
                },
            )
            .unwrap();
        let ask_file = app
            .home
            .cwd()
            .join(format!("asked-{}.txt", backend_key(backend)));
        upstream
            .push(vec![
                write_call(&advertised, ask_file.to_str().unwrap(), "S6B_APPROVED"),
                Turn::Text("The approved change is complete.".into()),
            ])
            .unwrap();
        rebuild
            .start(
                &project,
                RunRequest {
                    message: Some(
                        "Write the requested external note only after permission.".into(),
                    ),
                    shown: None,
                    pictures: Vec::new(),
                    continuing: true,
                    max_rounds: Some(0),
                    backend: Some(backend),
                },
            )
            .unwrap();
        let deadline = Instant::now() + LIMIT;
        let mut approved = false;
        let note_text = format!(
            "S7_NOTE_{} keep the current static page",
            backend_key(backend)
        );
        let mut note_sent = false;
        loop {
            if let Some(card) = app
                .services
                .inbox
                .pending(&project)
                .unwrap()
                .into_iter()
                .find(|card| card.kind == InboxKind::Approval)
            {
                assert!(!ask_file.exists());
                let reply = app
                    .services
                    .analysis
                    .call(
                        &project,
                        ToolCall {
                            tool: "core_target_triage".into(),
                            params: json!({"path":app_dir.join("index.html")}),
                            caller: CallerDto::Ui,
                            consented: true,
                        },
                        &CancelFlag::new(),
                    )
                    .unwrap();
                assert!(matches!(reply, ToolReply::Evidence { .. }));
                if !note_sent {
                    if backend == BackendKind::Dsh {
                        upstream
                            .push(vec![Turn::Text(
                                "The additional note was applied without changing the page.".into(),
                            )])
                            .unwrap();
                    }
                    let note = rebuild.note(&project, &note_text, Vec::new()).unwrap();
                    if backend == BackendKind::Dsh {
                        let withdrawn = rebuild
                            .note(&project, "S7_WITHDRAW_THIS", Vec::new())
                            .unwrap();
                        assert_eq!(
                            rebuild.withdraw(&project, &withdrawn).unwrap(),
                            Some("S7_WITHDRAW_THIS".into())
                        );
                    } else {
                        assert_eq!(
                            rebuild.withdraw(&project, &note).unwrap(),
                            None,
                            "accepted steer is already read"
                        );
                    }
                    note_sent = true;
                }
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
                approved = true;
            }
            if let Ok((
                _,
                RebuildEvent::Finished {
                    project: id,
                    ok,
                    paused,
                    ..
                },
            )) = app.channels.rebuild.try_recv()
                && id == project
            {
                assert!(ok && !paused);
                break;
            }
            assert!(
                Instant::now() < deadline,
                "approval waiting run did not finish"
            );
            thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(note_sent);
        assert!(
            upstream
                .requests()
                .unwrap()
                .iter()
                .any(|request| request.body["messages"].to_string().contains(&note_text)),
            "builder must consume the actual added note"
        );
        write_evidence(
            &format!("approval-wait-{}", backend_key(backend)),
            &json!({"approved":approved,"file_exists":ask_file.exists(),"results":results(&upstream.requests().unwrap())}),
        );
        assert!(
            approved,
            "project policy must ask instead of inheriting Auto allowance"
        );
        assert_eq!(fs::read_to_string(ask_file).unwrap(), "S6B_APPROVED");
        evidence.push(
            json!({"backend":backend_key(backend),"steps":steps,"checks":checks,
            "status":shown["status"],"built_marker":marker}),
        );
    }
    app.services.shutdown();
    upstream.shutdown().unwrap();
    assert_log_auth(&app.services.log, &[]);
    write_evidence(
        "rebuild",
        &json!({"version":1,"scenario":"S8","observed":evidence,
        "live_paid_website_app_game_acceptance":"pending user approval"}),
    );
}
