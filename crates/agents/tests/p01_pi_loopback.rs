//! Real official pi process → projected OpenAI Chat route → scripted loopback SSE → host snapshot.
//! Run with AUTOCRAFT_SMOKE_PI='<absolute node.exe>|<absolute pi dist/bundle/cli.js>'.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "ignored real-process smoke assertions"
)]

use std::{
    fs,
    path::PathBuf,
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

use autocraft_agents::PiRuntime;
use autocraft_core::{
    AgentRuntime, AgentSession, BackendKind, ContentBlock, DefaultModel, EngineRouting, Launch,
    ProjectedModel, ProjectedProvider, Protocol, Role, Secret, SessionEvent, SessionPhase,
    SessionRole, SessionSink, StartSession,
};
use autocraft_testkit::{Engine, Isolation, ScriptedUpstream, Turn};
use serde_json::json;

struct Sink;

impl SessionSink for Sink {
    fn publish(&self, _event: SessionEvent) {}
}

// Session lifecycle must end before the server and sandbox, including on a failing assertion.
struct RunningSession(Arc<dyn AgentSession>);

impl Drop for RunningSession {
    fn drop(&mut self) {
        let _cleanup_result = self.0.dispose();
    }
}

fn installed_pi() -> Launch {
    let value = std::env::var("AUTOCRAFT_SMOKE_PI")
        .expect("set AUTOCRAFT_SMOKE_PI='<absolute node.exe>|<absolute pi dist/bundle/cli.js>'");
    let mut parts = value.split('|');
    let executable = PathBuf::from(parts.next().expect("node path"));
    assert!(
        executable.is_absolute() && executable.is_file(),
        "expected installed node executable"
    );
    let entry = PathBuf::from(parts.next().expect("official pi bundle path"));
    assert!(
        entry.is_absolute() && entry.is_file(),
        "expected installed official pi entry point"
    );
    assert!(
        parts.next().is_none(),
        "expected node|pi bundle, not a shell command"
    );
    Launch::External {
        executable,
        args: vec![entry.to_string_lossy().into_owned()],
    }
}

fn routing(base_url: String) -> EngineRouting {
    let route = "autocraft-p01-loopback";
    let model = "p01-scripted-text";
    let key_var = "AUTOCRAFT_P01_LOOPBACK_KEY";
    EngineRouting {
        providers: vec![ProjectedProvider {
            route: route.into(),
            display_name: "P01 loopback fixture".into(),
            protocol: Protocol::OpenaiChat,
            base_url,
            key_var: key_var.into(),
            headers: Vec::new(),
            models: vec![ProjectedModel {
                id: model.into(),
                name: model.into(),
                vision: false,
                reasoning: false,
                efforts: Vec::new(),
                context_window: 128_000,
                max_tokens: 4096,
            }],
        }],
        default_model: Some(DefaultModel {
            route: route.into(),
            model_id: model.into(),
            effort: None,
        }),
        mcp: Vec::new(),
        env: vec![(key_var.into(), Secret::new("sk-test-p01-loopback-only"))],
        signature: "p01-loopback".into(),
    }
}

#[test]
#[ignore = "needs AUTOCRAFT_SMOKE_PI pointing at installed official pi and node; loopback only"]
fn pi_completes_one_text_turn_through_scripted_upstream() {
    let launch = installed_pi();
    let sandbox = Isolation::new().unwrap();
    let config = sandbox.config_dir(Engine::Pi);
    fs::write(
        config.join("settings.json"),
        json!({
            "enableInstallTelemetry": false, "enableAnalytics": false,
            "retry": {"enabled": false, "provider": {"maxRetries": 0}}
        })
        .to_string(),
    )
    .unwrap();
    // The unpredictable fixture marker is returned only by the local server, not by the prompt.
    let answer = format!(
        "P01_LOOPBACK_OK_{}",
        sandbox.root().file_name().unwrap().to_string_lossy()
    );
    let upstream = ScriptedUpstream::start(vec![Turn::Text(answer.clone())]).unwrap();
    let cwd = sandbox.cwd();
    let request = StartSession {
        chat: "p01-text-turn".into(),
        project: cwd.to_string_lossy().into_owned().into(),
        backend: BackendKind::Pi,
        launch,
        config_root: Some(config),
        cwd,
        resume: None,
        analysis: None,
        builder: None,
        routing: routing(upstream.base_url()),
        support_dir: sandbox.support_dir(),
        role: SessionRole::Worker,
    };
    let session = RunningSession(PiRuntime::new().start(request, Arc::new(Sink)).unwrap());
    assert_eq!(session.0.snapshot().phase, SessionPhase::Idle);
    let prompt = "Reply with one short text message. Do not call tools.";
    session.0.prompt(prompt).unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    let snapshot = loop {
        let snapshot = session.0.snapshot();
        assert!(
            snapshot.last_error.is_none(),
            "real pi turn failed: {:?}",
            snapshot.last_error
        );
        assert_ne!(
            snapshot.phase,
            SessionPhase::Closed,
            "pi closed before finishing the turn"
        );
        if snapshot.phase == SessionPhase::Idle {
            break snapshot;
        }
        assert!(
            Instant::now() < deadline,
            "pi did not finish its loopback turn"
        );
        thread::sleep(Duration::from_millis(10));
    };
    let text = snapshot
        .messages
        .iter()
        .filter(|message| message.role == Role::Assistant)
        .flat_map(|message| &message.content)
        .filter_map(|block| match block {
            ContentBlock::Text { text } => Some(text.as_str()),
            ContentBlock::Thinking { .. }
            | ContentBlock::Image { .. }
            | ContentBlock::Tool { .. } => None,
        })
        .collect::<String>();
    assert_eq!(
        text, answer,
        "official pi must project the actual loopback reply"
    );
    session.0.dispose().unwrap();
    assert_eq!(session.0.snapshot().phase, SessionPhase::Closed);
    drop(session);
    let requests = upstream.shutdown().unwrap();
    assert_eq!(
        requests.len(),
        1,
        "one actual model request, no hidden retries"
    );
    assert_eq!(requests[0].method, "POST");
    assert_eq!(requests[0].path, "/v1/chat/completions");
    assert_eq!(requests[0].body["model"], "p01-scripted-text");
    assert_eq!(requests[0].body["stream"], true);
    assert!(
        requests[0].body["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|message| {
                message["role"] == "user" && message["content"].to_string().contains(prompt)
            }),
        "upstream must receive the real user prompt"
    );
    sandbox.close().unwrap();
}
