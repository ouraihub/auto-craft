//! T26 owns disposable data, real adapters, authenticated MCP and loopback-only providers.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "isolated integration fixtures"
)]
#![allow(
    dead_code,
    reason = "shared helpers serve independent integration test binaries"
)]
#![allow(
    missing_debug_implementations,
    reason = "private test owners contain process and service handles"
)]

#[allow(dead_code)]
#[path = "../../src/services.rs"]
pub mod services;

use autocraft_core::*;
use autocraft_testkit::{Engine, Isolation, RecordedRequest, ScriptedUpstream, Turn};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

pub const LIMIT: Duration = Duration::from_secs(120);
pub const KEY: &str = "sk-test-t26-loopback-only";
pub const MODELS: [&str; 2] = ["t26-project-alpha", "t26-project-beta"];

pub fn engine(backend: BackendKind) -> Engine {
    match backend {
        BackendKind::Pi => Engine::Pi,
        BackendKind::Omp => Engine::Omp,
        BackendKind::Dsh => Engine::Dsh,
    }
}
pub fn backend_key(backend: BackendKind) -> &'static str {
    match backend {
        BackendKind::Pi => "pi",
        BackendKind::Omp => "omp",
        BackendKind::Dsh => "dsh",
    }
}

/// Explicit versioned specs, not shell commands or automatic searches of real user profiles.
pub fn launches() -> [Option<Launch>; 3] {
    let path = std::env::var_os("AUTOCRAFT_T26_SPEC")
        .expect("set AUTOCRAFT_T26_SPEC to a version-1 JSON engine launch specification");
    let spec: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    assert_eq!(spec["version"], 1, "unsupported T26 launch specification");
    BackendKind::ALL.map(|backend| {
        let entry = &spec["engines"][backend_key(backend)];
        let executable = PathBuf::from(entry["executable"].as_str().expect("absolute executable"));
        assert!(
            executable.is_absolute() && executable.is_file(),
            "installed official engine executable required"
        );
        let args = entry["args"]
            .as_array()
            .expect("explicit argument array")
            .iter()
            .map(|arg| arg.as_str().expect("string launch argument").to_owned())
            .collect();
        Some(Launch::External { executable, args })
    })
}

pub struct App {
    pub services: services::Services,
    pub channels: services::Channels,
    pub home: Isolation,
}
impl App {
    pub fn new(
        log: Arc<autocraft_observability::LogHandle>,
        launches: [Option<Launch>; 3],
    ) -> Self {
        let home = Isolation::new().unwrap();
        fs::write(
            home.config_dir(Engine::Pi).join("settings.json"),
            json!({
                "enableInstallTelemetry": false, "enableAnalytics": false,
                "retry": {"enabled": false, "provider": {"maxRetries": 0}}
            })
            .to_string(),
        )
        .unwrap();
        let config = services::RuntimeConfig {
            launches,
            config_roots: BackendKind::ALL.map(|backend| Some(home.config_dir(engine(backend)))),
            host_executable: Some(PathBuf::from(env!("CARGO_BIN_EXE_autocraft"))),
        };
        let (services, channels) = services::build_with(
            &home.data_dir(),
            &home.data_dir().join("autocraft.db"),
            log,
            config,
        )
        .unwrap();
        services
            .projects
            .set_locations(ProjectLocations {
                default_dir: home.cwd(),
                extra: Vec::new(),
            })
            .unwrap();
        Self {
            services,
            channels,
            home,
        }
    }
    pub fn provider(&self, upstream: &ScriptedUpstream) -> ProviderId {
        let provider = self
            .services
            .providers
            .create(ProviderInput {
                name: "T26 loopback only".into(),
                endpoints: vec![Endpoint {
                    protocol: Protocol::OpenaiChat,
                    base_url: upstream.base_url(),
                }],
                default_protocol: Protocol::OpenaiChat,
                headers: Vec::new(),
                access: Access::Gateway,
                enabled: true,
                models: MODELS
                    .into_iter()
                    .map(|id| ProviderModel {
                        model_id: id.into(),
                        model_type: ModelType::Chat,
                        name: id.into(),
                        protocols: vec![Protocol::OpenaiChat],
                        context_window: Some(128_000),
                        max_tokens: Some(4096),
                        vision: false,
                        reasoning: false,
                        tools: true,
                        efforts: Vec::new(),
                        enabled: true,
                    })
                    .collect(),
            })
            .unwrap();
        self.services
            .providers
            .add_key(
                &provider.id,
                ProviderKeyInput {
                    label: None,
                    key: Secret::new(KEY),
                    enabled: true,
                },
            )
            .unwrap();
        provider.id
    }
    pub fn project(
        &self,
        name: &str,
        backend: BackendKind,
        provider: &ProviderId,
        model: &str,
    ) -> ProjectId {
        self.services
            .projects
            .create(NewProject {
                name: name.into(),
                request: "Build a static website with one HTML page.".into(),
                target: TargetRefDto::None,
                door: Door::MakeMyOwn,
                folder: None,
                existing: None,
                mode: AnalysisMode::Deep,
                builder: Some(ProjectModelChoice {
                    backend: Some(backend),
                    model: Some(ModelRef {
                        provider: provider.clone(),
                        model_id: model.into(),
                    }),
                    effort: None,
                }),
            })
            .unwrap()
            .id
    }
    pub fn session(&self, project: &ProjectId, backend: BackendKind) -> RunningSession {
        self.session_with_options(
            project,
            backend,
            ChannelOptions {
                all_tools: false,
                async_long_tools: false,
            },
        )
    }
    pub fn session_with_options(
        &self,
        project: &ProjectId,
        backend: BackendKind,
        options: ChannelOptions,
    ) -> RunningSession {
        let chat = ChatId::generate();
        self.services.hold_analysis(project, &chat).unwrap();
        let access = self
            .services
            .register_analysis(
                ChatScope {
                    project: project.clone(),
                    chat: chat.clone(),
                    purpose: ChatPurpose::Analysis,
                },
                options,
            )
            .unwrap();
        let request = autocraft_routing::resolve_project_route(
            project,
            &chat,
            &*self.services.project_prefs,
            &*self.services.prefs,
            &*self.services.chats,
        )
        .unwrap();
        let routing =
            autocraft_routing::prepare_project_route(&*self.services.router, &request).unwrap();
        let token = access.token.clone();
        assert!(!format!("{access:?}").contains(&token));
        let session = self
            .services
            .runtime(backend)
            .start(
                StartSession {
                    chat: chat.clone(),
                    project: project.clone(),
                    backend,
                    launch: self
                        .services
                        .launch(backend)
                        .unwrap()
                        .expect("injected official launch"),
                    config_root: Some(self.home.config_dir(engine(backend))),
                    cwd: PathBuf::from(project.as_str()),
                    resume: None,
                    analysis: Some(access),
                    builder: None,
                    routing,
                    support_dir: self.services.support_dir(&chat).unwrap(),
                    role: SessionRole::Worker,
                },
                self.services.session_sink.clone(),
            )
            .unwrap();
        RunningSession {
            session,
            services: self.services.clone(),
            project: project.clone(),
            chat,
            token,
        }
    }
}
impl Drop for App {
    fn drop(&mut self) {
        self.services.shutdown();
    }
}

pub struct RunningSession {
    pub session: Arc<dyn AgentSession>,
    services: services::Services,
    project: ProjectId,
    chat: ChatId,
    pub token: String,
}
impl RunningSession {
    pub fn turn(&self, text: &str, during: impl FnMut()) -> SessionSnapshot {
        let revision = self.session.snapshot().revision;
        self.session.prompt(text).unwrap();
        self.wait(revision, during)
    }
    pub fn wait(&self, revision: u64, mut during: impl FnMut()) -> SessionSnapshot {
        let deadline = Instant::now() + LIMIT;
        loop {
            during();
            for interaction in self.session.snapshot().interactions {
                let answer = match interaction.kind {
                    InteractionKind::Confirm => InteractionAnswer::Confirm { confirmed: true },
                    InteractionKind::Select | InteractionKind::Permission => {
                        let Some(option) = interaction.options.iter().find(|option| {
                            option.label == "Allow once"
                                || option.label == "Allow"
                                || option.label == "Approve"
                        }) else {
                            continue;
                        };
                        InteractionAnswer::Select {
                            option_id: option.id.clone(),
                        }
                    }
                    InteractionKind::Input | InteractionKind::Editor | InteractionKind::Ask => {
                        continue;
                    }
                };
                self.session
                    .respond(&interaction.request_id, answer)
                    .unwrap();
            }
            let snapshot = self.session.snapshot();
            assert!(
                snapshot.last_error.is_none(),
                "real adapter failed: {:?}",
                snapshot.last_error
            );
            assert_ne!(
                snapshot.phase,
                SessionPhase::Closed,
                "official engine closed unexpectedly"
            );
            if snapshot.revision > revision && snapshot.phase == SessionPhase::Idle {
                return snapshot;
            }
            assert!(
                Instant::now() < deadline,
                "official engine did not settle before deadline; interactions: {:?}",
                snapshot.interactions
            );
            thread::sleep(Duration::from_millis(20));
        }
    }
}
impl Drop for RunningSession {
    fn drop(&mut self) {
        let _disposed = self.session.dispose();
        self.services.router.release(&self.chat);
        self.services.revoke_analysis(&self.token);
        let _released = self.services.release_analysis(&self.project, &self.chat);
        let _removed = self.services.remove_support_dir(&self.chat);
    }
}

pub fn tool_name(request: &RecordedRequest, tool: &str) -> String {
    let tools = request.body["tools"]
        .as_array()
        .expect("actual advertised model tools");
    tools
        .iter()
        .filter_map(|entry| entry["function"]["name"].as_str())
        .find(|name| name.ends_with(tool))
        .unwrap_or_else(|| panic!("missing real MCP tool {tool}"))
        .into()
}
pub fn tool_call(name: &str, args: Value) -> Turn {
    Turn::ToolCall {
        name: name.into(),
        args,
    }
}
/// MCP clients refresh asynchronously; observe a subsequent real request, never mutate tools.
pub fn await_tool(upstream: &ScriptedUpstream, session: &RunningSession, tool: &str) -> String {
    let deadline = Instant::now() + LIMIT;
    loop {
        let requests = upstream.requests().unwrap();
        if let Some(name) = requests
            .last()
            .and_then(|request| request.body["tools"].as_array())
            .into_iter()
            .flatten()
            .filter_map(|entry| entry["function"]["name"].as_str())
            .find(|name| name.ends_with(tool))
        {
            return name.into();
        }
        assert!(
            Instant::now() < deadline,
            "client never exposed enabled MCP tool {tool}"
        );
        upstream
            .push(vec![Turn::Text("tool refresh observed".into())])
            .unwrap();
        session.turn(
            "Acknowledge the refreshed capabilities without calling tools.",
            || {},
        );
    }
}
pub fn results(requests: &[RecordedRequest]) -> Vec<Value> {
    requests
        .iter()
        .flat_map(|request| request.body["messages"].as_array().unwrap())
        .filter(|message| message["role"] == "tool")
        .map(|message| match &message["content"] {
            Value::String(text) => {
                serde_json::from_str(text).unwrap_or_else(|_| Value::String(text.clone()))
            }
            content => content.clone(),
        })
        .collect()
}
pub fn assert_no_auth(requests: &[RecordedRequest], tokens: &[&str], snapshot: &SessionSnapshot) {
    let observable = format!(
        "{}{}",
        serde_json::to_string(snapshot).unwrap(),
        requests
            .iter()
            .map(|request| request.body.to_string())
            .collect::<String>()
    );
    for secret in tokens.iter().copied().chain([KEY]) {
        assert!(
            !observable.contains(secret),
            "credential leaked into model/session content"
        );
    }
    for request in requests {
        assert_eq!(request.method, "POST");
        assert_eq!(request.path, "/v1/chat/completions");
        assert!(
            request
                .headers
                .iter()
                .any(|(key, value)| key == "authorization" && value == &format!("Bearer {KEY}")),
            "only synthetic fixture auth may reach loopback upstream"
        );
    }
}
pub fn assert_log_auth(log: &autocraft_observability::LogHandle, tokens: &[String]) {
    let output = log
        .tail(10_000)
        .unwrap()
        .into_iter()
        .map(|entry| entry.raw)
        .collect::<String>();
    assert!(
        !output.contains(KEY),
        "synthetic provider credential leaked into diagnostic logs"
    );
    for token in tokens {
        assert!(
            !output.contains(token),
            "analysis launch token leaked into diagnostic logs"
        );
    }
}
pub fn write_evidence(scenario: &str, value: &Value) {
    if let Some(root) = std::env::var_os("AUTOCRAFT_T26_EVIDENCE") {
        let root = Path::new(&root);
        fs::create_dir_all(root).unwrap();
        fs::write(
            root.join(format!("{scenario}.json")),
            serde_json::to_vec_pretty(value).unwrap(),
        )
        .unwrap();
    }
}
