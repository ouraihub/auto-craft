//! P05 real adapters + loopback model + notifying MCP subprocess. No recorded result is synthetic.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "ignored real-engine probe assertions"
)]
use autocraft_agents::{DshRuntime, OmpRuntime, PiRuntime};
use autocraft_core::{
    AgentRuntime, AgentSession, BackendKind, DefaultModel, EngineRouting, InteractionAnswer,
    Launch, ProjectedMcpServer, ProjectedMcpTransport, ProjectedModel, ProjectedProvider, Protocol,
    Secret, SessionEvent, SessionPhase, SessionRole, SessionSink, StartSession,
};
use autocraft_testkit::{Engine, Isolation, ScriptedUpstream, Turn};
use serde_json::{Value, json};
use std::{
    fs,
    path::PathBuf,
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

struct Sink;
impl SessionSink for Sink {
    fn publish(&self, _: SessionEvent) {}
}
struct Running(Arc<dyn AgentSession>);
impl Drop for Running {
    fn drop(&mut self) {
        if let Err(error) = self.0.dispose() {
            let _ = std::io::Write::write_fmt(
                &mut std::io::stderr().lock(),
                format_args!("probe cleanup: {error}\n"),
            );
        }
    }
}

fn launch(backend: BackendKind) -> Launch {
    let name = match backend {
        BackendKind::Pi => "AUTOCRAFT_SMOKE_PI",
        BackendKind::Omp => "AUTOCRAFT_SMOKE_OMP",
        BackendKind::Dsh => "AUTOCRAFT_SMOKE_DSH",
    };
    let value =
        std::env::var(name).unwrap_or_else(|_| panic!("set {name}=absolute executable|arguments"));
    let mut parts = value.split('|');
    let executable = PathBuf::from(parts.next().unwrap());
    assert!(executable.is_absolute() && executable.is_file());
    Launch::External {
        executable,
        args: parts.map(str::to_owned).collect(),
    }
}
fn node() -> PathBuf {
    let node = std::env::var_os("AUTOCRAFT_PROBE_NODE")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(
                std::env::var("AUTOCRAFT_SMOKE_PI")
                    .expect("set AUTOCRAFT_PROBE_NODE or AUTOCRAFT_SMOKE_PI")
                    .split('|')
                    .next()
                    .unwrap(),
            )
        });
    assert!(node.is_absolute() && node.is_file());
    plain(node)
}
fn plain(path: PathBuf) -> PathBuf {
    let text = path.to_string_lossy();
    text.strip_prefix(r"\\?\")
        .map(PathBuf::from)
        .unwrap_or(path)
}
fn routing(base_url: String, server: ProjectedMcpServer) -> EngineRouting {
    EngineRouting {
        providers: vec![ProjectedProvider {
            route: "autocraft-p05".into(),
            display_name: "P05 loopback".into(),
            protocol: Protocol::OpenaiChat,
            base_url,
            key_var: "AUTOCRAFT_P05_KEY".into(),
            headers: vec![],
            models: vec![ProjectedModel {
                id: "p05".into(),
                name: "P05".into(),
                vision: false,
                reasoning: false,
                efforts: vec![],
                context_window: 128000,
                max_tokens: 4096,
            }],
        }],
        default_model: Some(DefaultModel {
            route: "autocraft-p05".into(),
            model_id: "p05".into(),
            effort: None,
        }),
        mcp: vec![server],
        env: vec![("AUTOCRAFT_P05_KEY".into(), Secret::new("sk-test-p05-only"))],
        signature: "p05".into(),
    }
}
fn probe(backend: BackendKind, engine: Engine, runtime: &dyn AgentRuntime) {
    let sandbox = Isolation::new().unwrap();
    let fixture = plain(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/list-changed-server.mjs"),
    );
    let log = plain(sandbox.root().join("mcp-requests.jsonl"));
    let prefix = match backend {
        BackendKind::Pi => "mcp__autocraft_p05__",
        BackendKind::Omp => "mcp__autocraft_p05_",
        BackendKind::Dsh => "mcp__autocraft-p05__",
    };
    let enable = format!("{prefix}enable_toolset");
    let native = format!("{prefix}native_functions_list");
    let upstream = ScriptedUpstream::start(vec![
        Turn::ToolCall {
            name: enable.clone(),
            args: json!({"toolset":"native"}),
        },
        Turn::Text("P05_AFTER_ENABLE".into()),
        Turn::ToolCall {
            name: native.clone(),
            args: json!({}),
        },
        Turn::Text("P05_FINISHED".into()),
    ])
    .unwrap();
    let mut installed = launch(backend);
    if backend == BackendKind::Omp {
        let Launch::External { args, .. } = &mut installed;
        args.push("--no-skills".into());
    }
    let request = StartSession {
        chat: "p05".into(),
        project: sandbox.cwd().to_string_lossy().into_owned().into(),
        backend,
        launch: installed,
        config_root: Some(sandbox.config_dir(engine)),
        cwd: sandbox.cwd(),
        resume: None,
        analysis: None,
        builder: None,
        routing: routing(
            upstream.base_url(),
            ProjectedMcpServer {
                name: "autocraft-p05".into(),
                transport: ProjectedMcpTransport::Stdio {
                    command: node().display().to_string(),
                    args: vec![fixture.display().to_string(), log.display().to_string()],
                    env: vec![],
                },
            },
        ),
        support_dir: sandbox.support_dir(),
        role: SessionRole::Worker,
    };
    let session = Running(runtime.start(request, Arc::new(Sink)).unwrap());
    let outcome = (|| {
        session
            .0
            .prompt("Enable the native fixture toolset.")
            .map_err(|error| error.to_string())?;
        idle(&session)?;
        thread::sleep(Duration::from_secs(2));
        session
            .0
            .prompt("Call the native functions fixture now.")
            .map_err(|error| error.to_string())?;
        idle(&session)
    })();
    session.0.dispose().unwrap();
    drop(session);
    let recorded = upstream.requests().unwrap();
    let shutdown = upstream.shutdown();
    if outcome.is_err() || shutdown.is_err() {
        let requests:Vec<_> = recorded.iter().map(|request| json!({"method":request.method,"path":request.path,"model":request.body["model"],"tool_count":request.body["tools"].as_array().map(Vec::len)})).collect();
        let _ = std::io::Write::write_fmt(
            &mut std::io::stderr().lock(),
            format_args!(
                "P05 failure backend={} session={outcome:?} upstream={shutdown:?} requests={}\n",
                backend.label(),
                json!(requests)
            ),
        );
    }
    outcome.expect("real engine scenario must complete; diagnostics above");
    let model = shutdown.expect("upstream must consume exactly its actual scripted model turns");
    let mcp: Vec<Value> = fs::read_to_string(&log)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let initial = mcp
        .iter()
        .position(|frame| frame["method"] == "tools/list")
        .expect("real initial tools/list");
    let notification = mcp
        .iter()
        .position(|frame| frame["emitted"] == "notifications/tools/list_changed")
        .expect("real notification after enabling");
    assert!(initial < notification);
    assert!(
        mcp.iter()
            .any(|frame| frame["method"] == "tools/call"
                && frame["params"]["name"] == "enable_toolset"),
        "engine must actually reach MCP enable tool"
    );
    let refreshed = mcp[notification + 1..]
        .iter()
        .any(|frame| frame["method"] == "tools/list");
    let native_reached = mcp.iter().any(|frame| {
        frame["method"] == "tools/call" && frame["params"]["name"] == "native_functions_list"
    });
    let names = |body: &Value| {
        body["tools"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|tool| tool["function"]["name"].as_str().map(str::to_owned))
            .collect::<Vec<_>>()
    };
    let first = names(&model[0].body);
    assert!(
        first.contains(&enable),
        "initial model must see fixture enable tool: {first:?}"
    );
    let later = model
        .iter()
        .skip(1)
        .any(|request| names(&request.body).contains(&native));
    let result_seen = model.iter().any(|request| {
        request.body["messages"]
            .to_string()
            .contains("P05_NATIVE_RESULT_FROM_MCP")
    });
    let report = json!({"backend":backend.label(),"initial_model_tool_count":first.len(),"initial_model_tools":first,"mcp_requests":mcp,"refreshed_tools_list":refreshed,"native_model_declared":later,"native_call_reached_mcp":native_reached,"native_result_seen_by_model":result_seen,"all_tools_required":!refreshed});
    let root = std::env::var_os("AUTOCRAFT_P05_FIXTURES")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/list-changed")
        });
    fs::create_dir_all(&root).unwrap();
    fs::write(
        root.join(format!("{}.json", backend.label())),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
    std::io::Write::write_fmt(&mut std::io::stdout().lock(), format_args!("{report}\n")).unwrap();
    assert!(
        native_reached && result_seen,
        "engine cannot directly use the newly enabled native tool; report saved"
    );
}
fn idle(session: &Running) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        let state = session.0.snapshot();
        if state.last_error.is_some() || state.phase == SessionPhase::Closed {
            return Err(format!("{:?}", state.last_error));
        }
        for interaction in &state.interactions {
            let choice = interaction
                .options
                .iter()
                .find(|option| option.label == "Approve")
                .expect("fixture MCP approval offers Approve");
            session
                .0
                .respond(
                    &interaction.request_id,
                    InteractionAnswer::Select {
                        option_id: choice.id.clone(),
                    },
                )
                .unwrap();
        }
        if state.phase == SessionPhase::Idle {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err("engine did not settle".into());
        }
        thread::sleep(Duration::from_millis(20));
    }
}
#[test]
#[ignore = "requires installed real pi + node; loopback only"]
fn pi_list_changed() {
    probe(BackendKind::Pi, Engine::Pi, &PiRuntime::new());
}
#[test]
#[ignore = "requires installed real OMP + node; loopback only"]
fn omp_list_changed() {
    probe(BackendKind::Omp, Engine::Omp, &OmpRuntime::new());
}
#[test]
#[ignore = "requires installed real DSH + node; loopback only"]
fn dsh_list_changed() {
    probe(BackendKind::Dsh, Engine::Dsh, &DshRuntime::new());
}
