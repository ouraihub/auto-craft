//! Observable host/session concurrency and byte-compatible upstream MCP regression coverage.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "Isolated test fixtures and assertions"
)]
use autocraft_analysis::{
    AnalysisConfig, AnalysisEvent, AnalysisHub, AnalysisSink, AssetKeySource, LongToolSink,
};
use autocraft_core::*;
use ca_engine::{
    Engine, Session,
    inbox::InboxDir,
    workspace::{Questions, TargetRef, Workspace},
};
use parking_lot::Mutex;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    io::{self, Cursor, Write},
    sync::{Arc, mpsc},
    time::Duration,
};

#[derive(Default)]
struct Prefs(Mutex<HashMap<ProjectId, ProjectPrefs>>);
impl ProjectPrefService for Prefs {
    fn get(&self, id: &ProjectId) -> Result<ProjectPrefs> {
        Ok(self
            .0
            .lock()
            .entry(id.clone())
            .or_insert(ProjectPrefs {
                backend: None,
                model: None,
                effort: None,
                mode: AnalysisMode::Deep,
                view: None,
                revision: 0,
            })
            .clone())
    }
    fn set(
        &self,
        id: &ProjectId,
        change: ProjectPrefChange,
        revision: u64,
    ) -> Result<ProjectPrefs> {
        let mut value = self.get(id)?;
        if value.revision != revision {
            return Err(Error::conflict("changed"));
        }
        match change {
            ProjectPrefChange::Mode(v) => value.mode = v,
            ProjectPrefChange::Backend(v) => value.backend = v,
            ProjectPrefChange::Model(v) => value.model = v,
            ProjectPrefChange::Effort(v) => value.effort = v,
            ProjectPrefChange::View(v) => value.view = v,
        }
        value.revision += 1;
        self.0.lock().insert(id.clone(), value.clone());
        Ok(value)
    }
    fn remove(&self, id: &ProjectId) -> Result<()> {
        self.0.lock().remove(id);
        Ok(())
    }
    fn copy(&self, from: &ProjectId, to: &ProjectId) -> Result<()> {
        let value = self.get(from)?;
        self.0.lock().insert(to.clone(), value);
        Ok(())
    }
}
struct Keys;
impl AssetKeySource for Keys {
    fn asset_keys(&self) -> Result<Vec<(String, Secret)>> {
        Ok(Vec::new())
    }
}
struct Sink;
impl AnalysisSink for Sink {
    fn publish(&self, _event: AnalysisEvent) {}
}
#[derive(Clone, Default)]
struct Output(Arc<Mutex<Vec<u8>>>);
impl Write for Output {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.lock().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
fn fixture() -> (tempfile::TempDir, Arc<AnalysisHub>, ProjectId) {
    let root = tempfile::tempdir().unwrap();
    let hub = AnalysisHub::try_new(AnalysisConfig::new(
        root.path().to_owned(),
        Arc::new(Prefs::default()),
        Arc::new(Keys),
        Arc::new(Sink),
    ))
    .unwrap();
    hub.set_locations(ProjectLocations {
        default_dir: root.path().join("projects"),
        extra: Vec::new(),
    })
    .unwrap();
    let project = hub
        .create(NewProject {
            name: "Example".into(),
            request: "Build".into(),
            target: TargetRefDto::None,
            door: Door::MakeMyOwn,
            folder: None,
            existing: None,
            mode: AnalysisMode::Deep,
            builder: None,
        })
        .unwrap();
    (root, hub, project.id)
}
fn wait_card(hub: &AnalysisHub, id: &ProjectId) -> InboxRequest {
    let started = std::time::Instant::now();
    loop {
        if let Some(card) = hub.pending(id).unwrap().into_iter().next() {
            return card;
        }
        assert!(started.elapsed() < Duration::from_secs(5));
        std::thread::sleep(Duration::from_millis(10));
    }
}
fn scope(project: ProjectId) -> ChatScope {
    ChatScope {
        project,
        chat: ChatId::generate(),
        purpose: ChatPurpose::Analysis,
    }
}

#[test]
fn same_project_tool_completes_while_person_question_waits() {
    let (_root, hub, id) = fixture();
    hub.set_person(&id, PersonChange::Questions(QuestionsDto::Ask))
        .unwrap();
    let worker_hub = hub.clone();
    let worker_id = id.clone();
    let question = std::thread::spawn(move || {
        worker_hub.call(
            &worker_id,
            ToolCall {
                tool: "core_person_ask".into(),
                params: json!({"question":"Choose","choices":["Yes"]}),
                caller: CallerDto::Ui,
                consented: true,
            },
            &CancelFlag::new(),
        )
    });
    let card = wait_card(&hub, &id);
    let other_hub = hub.clone();
    let other_id = id.clone();
    let (tx, rx) = mpsc::channel();
    let other = std::thread::spawn(move || {
        tx.send(other_hub.call(
            &other_id,
            ToolCall {
                tool: "core_session_show".into(),
                params: json!({}),
                caller: CallerDto::Ui,
                consented: true,
            },
            &CancelFlag::new(),
        ))
        .unwrap()
    });
    let fast = rx.recv_timeout(Duration::from_secs(2));
    hub.answer(
        &id,
        &card.id,
        InboxAnswer::Choice {
            choice: Some("Yes".into()),
            text: Some("details".into()),
        },
    )
    .unwrap();
    let answer = question.join().unwrap().unwrap();
    other.join().unwrap();
    assert!(
        fast.unwrap().is_ok(),
        "a question must not retain the session lock"
    );
    assert_eq!(
        answer,
        ToolReply::Value(
            json!({"answered":true,"answer":"Yes — details","choice":"Yes","text":"details"})
        )
    );
}
#[test]
fn channel_question_wait_allows_second_call_and_observes_both_responses() {
    let (_root, hub, id) = fixture();
    hub.set_person(&id, PersonChange::Questions(QuestionsDto::Ask))
        .unwrap();
    let output = Output::default();
    let worker_output = output.clone();
    let worker_hub = hub.clone();
    let worker_id = id.clone();
    let input=json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"core_person_ask","arguments":{"question":"Choose"}}}).to_string()+"\n"+&json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"core_session_show","arguments":{}}}).to_string()+"\n";
    let worker = std::thread::spawn(move || {
        worker_hub.serve(
            &scope(worker_id),
            ChannelOptions::default(),
            Box::new(Cursor::new(input.into_bytes())),
            Box::new(worker_output),
        )
    });
    let card = wait_card(&hub, &id);
    let started = std::time::Instant::now();
    let completed = loop {
        let lines = output.0.lock().clone();
        if lines
            .split(|b| *b == b'\n')
            .filter(|line| !line.is_empty())
            .any(|line| serde_json::from_slice::<Value>(line).unwrap()["id"] == 2)
        {
            break true;
        }
        if started.elapsed() > Duration::from_secs(2) {
            break false;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    hub.answer(
        &id,
        &card.id,
        InboxAnswer::Choice {
            choice: None,
            text: Some("Proceed".into()),
        },
    )
    .unwrap();
    worker.join().unwrap().unwrap();
    assert!(
        completed,
        "tools/call must continue while a host question waits"
    );
    let bytes = output.0.lock();
    let answers: Vec<Value> = bytes
        .split(|b| *b == b'\n')
        .filter(|line| !line.is_empty())
        .map(|line| serde_json::from_slice(line).unwrap())
        .collect();
    assert_eq!(answers.len(), 2);
    let question = answers.iter().find(|v| v["id"] == 1).unwrap();
    assert_eq!(
        question["result"]["structuredContent"]["result"]["answer"],
        "Proceed"
    );
}
#[test]
fn host_decide_approval_and_parse_error_are_byte_exact_upstream_mcp() {
    let (_root, hub, id) = fixture();
    let mut session = Session::isolated(std::path::Path::new(id.as_str()));

    session.settings.auto_mode = true;
    session.settings.questions = Some(Questions::Decide);
    session.deep_for_run = Some(true);
    session.ext.insert(Arc::new(InboxDir(
        std::path::Path::new(id.as_str()).join("inbox"),
    )));
    let input = "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{\"name\":\"core_person_ask\",\"arguments\":{\"question\":\"Choose\"}}}\n";
    for input in [input.to_owned(),json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"core_action_approve","arguments":{"tool_name":"Write","input":{"file_path":format!("{}/app/a.txt",id.as_str()),"content":"exact"}}}}).to_string()+"\n",json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"core_person_ask","arguments":{"question":""}}}).to_string()+"\n"] {
        let mut upstream=Session::isolated(std::path::Path::new(id.as_str()));upstream.settings=session.settings.clone();upstream.deep_for_run=Some(true);upstream.ext.insert(Arc::new(InboxDir(std::path::Path::new(id.as_str()).join("inbox"))));
        let direct=Output::default();let hosted=Output::default();
        ca_mcp::serve(Engine::shared(upstream,Vec::new()),ca_mcp::ServerOptions {budget_for_client:false,..Default::default()},Cursor::new(input.as_bytes()),Box::new(direct.clone())).unwrap();
        hub.serve(&scope(id.clone()),ChannelOptions::default(),Box::new(Cursor::new(input.into_bytes())),Box::new(hosted.clone())).unwrap();
        assert_eq!(*hosted.0.lock(),*direct.0.lock());
    }
}
#[test]
fn read_only_channel_refuses_real_mutating_tool_without_touching_file() {
    let (_root, hub, id) = fixture();
    let output = Output::default();
    let mut chat = scope(id.clone());
    chat.purpose = ChatPurpose::Question;
    let input=json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"core_target_comment_set","arguments":{}}}).to_string()+"\n";
    // Pick by real effect, not a hard-coded assumption about a tool's side effects.
    let tool = hub
        .tools(&id)
        .unwrap()
        .into_iter()
        .find(|tool| tool.effects.iter().any(|e| e == "MUTATES_TARGET"))
        .unwrap();
    let mut request: Value = serde_json::from_str(input.trim()).unwrap();
    request["params"]["name"] = json!(tool.id);
    hub.serve(
        &chat,
        ChannelOptions::default(),
        Box::new(Cursor::new((request.to_string() + "\n").into_bytes())),
        Box::new(output.clone()),
    )
    .unwrap();
    let reply: Value = serde_json::from_slice(&output.0.lock()).unwrap();
    assert_eq!(reply["result"]["isError"], true);
    assert!(
        reply["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("read-only question session")
    );
}
struct Delivery(mpsc::Sender<String>);
impl LongToolSink for Delivery {
    fn deliver(&self, _scope: &ChatScope, text: String) -> Result<()> {
        self.0.send(text).map_err(|_| Error::external("closed"))
    }
}
#[test]
fn dsh_request_cancellation_does_not_cancel_background_result() {
    let root = tempfile::tempdir().unwrap();
    let (tx, rx) = mpsc::channel();
    let mut config = AnalysisConfig::new(
        root.path().to_owned(),
        Arc::new(Prefs::default()),
        Arc::new(Keys),
        Arc::new(Sink),
    );
    config.long_results = Some(Arc::new(Delivery(tx)));
    let hub = AnalysisHub::try_new(config).unwrap();
    let ws = Workspace::create(
        &root.path().join("projects"),
        "Example",
        "",
        TargetRef::None,
    )
    .unwrap();
    let id = autocraft_analysis::project_id(&ws.root).unwrap();
    let output = Output::default();
    let input=json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"core_target_triage","arguments":{}}}).to_string()+"\n"+&json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":1}}).to_string()+"\n";
    hub.serve(
        &scope(id),
        ChannelOptions {
            all_tools: false,
            async_long_tools: true,
        },
        Box::new(Cursor::new(input.into_bytes())),
        Box::new(output.clone()),
    )
    .unwrap();
    let result = rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(result.starts_with("Result of background job "));
    assert!(result.contains("core_target_triage"));
    assert!(result.contains("isError"));
    let reply: Value = serde_json::from_slice(&output.0.lock()).unwrap();
    assert_eq!(reply["result"]["isError"], false);
}

#[test]
fn repeated_open_shares_session_until_last_holder_releases() {
    use autocraft_analysis::Holder;
    let (root, hub, id) = fixture();
    let target = root.path().join("target.js");
    std::fs::write(&target, "function answer() { return 42; }\n").unwrap();
    AnalysisService::open(hub.as_ref(), &id).unwrap();
    hub.call(
        &id,
        ToolCall {
            tool: "core_target_open".into(),
            params: json!({"path":target}),
            caller: CallerDto::Ui,
            consented: true,
        },
        &CancelFlag::new(),
    )
    .unwrap();
    let active = hub.view(&id).unwrap().active.unwrap();
    let chat = Holder::Chat(ChatId::generate());
    hub.hold(&id, chat.clone()).unwrap();
    hub.hold(&id, Holder::Run).unwrap();
    AnalysisService::open(hub.as_ref(), &id).unwrap();
    assert_eq!(hub.view(&id).unwrap().active, Some(active.clone()));
    assert_eq!(
        AnalysisService::close(hub.as_ref(), &id)
            .unwrap_err()
            .kind(),
        Some(ErrorKind::Busy)
    );
    assert_eq!(
        hub.release(&id, &chat).unwrap_err().kind(),
        Some(ErrorKind::Busy)
    );
    assert_eq!(hub.view(&id).unwrap().active, Some(active));
    hub.release(&id, &Holder::Run).unwrap();
    AnalysisService::open(hub.as_ref(), &id).unwrap();
    assert!(
        hub.view(&id).unwrap().active.is_none(),
        "last release must unload ephemeral session targets"
    );
    AnalysisService::close(hub.as_ref(), &id).unwrap();
}

#[test]
fn door_modes_and_overrides_survive_engine_unload_and_hub_recreation() {
    let root = tempfile::tempdir().unwrap();
    let prefs = Arc::new(Prefs::default());
    let config = || {
        AnalysisConfig::new(
            root.path().to_owned(),
            prefs.clone(),
            Arc::new(Keys),
            Arc::new(Sink),
        )
    };
    let hub = AnalysisHub::try_new(config()).unwrap();
    hub.set_locations(ProjectLocations {
        default_dir: root.path().join("projects"),
        extra: Vec::new(),
    })
    .unwrap();
    let target = root.path().join("reference.js");
    std::fs::write(&target, "function sample() { return 7; }\n").unwrap();
    let own = root.path().join("own");
    std::fs::create_dir(&own).unwrap();
    std::fs::write(own.join("keep.txt"), "user-owned content").unwrap();
    let mut projects = Vec::new();
    for (door, mode, folder, target_ref) in [
        (
            Door::MakeMyOwn,
            AnalysisMode::CleanRoom,
            None,
            TargetRefDto::None,
        ),
        (
            Door::WorkOnMine,
            AnalysisMode::CleanRoom,
            Some(own.clone()),
            TargetRefDto::None,
        ),
        (
            Door::LookInside,
            AnalysisMode::Deep,
            None,
            TargetRefDto::Local {
                path: target.clone(),
            },
        ),
    ] {
        let summary = hub
            .create(NewProject {
                name: format!("{door:?}"),
                request: "A real project".into(),
                target: target_ref,
                door,
                folder,
                existing: Some(ExistingFilesDto::Edit),
                mode,
                builder: None,
            })
            .unwrap();
        let detail = ProjectService::open(hub.as_ref(), &summary.id).unwrap();
        assert_eq!(detail.mode, mode);
        assert_eq!(hub.view(&summary.id).unwrap().mode, mode);
        if door == Door::WorkOnMine {
            assert!(summary.own_project);
            assert_eq!(detail.app_dir, own);
        }
        if door == Door::LookInside {
            assert!(hub.view(&summary.id).unwrap().active.is_some());
        }
        projects.push((summary.id, mode));
    }
    assert_eq!(
        std::fs::read_to_string(own.join("keep.txt")).unwrap(),
        "user-owned content"
    );
    let changed = projects[0].0.clone();
    hub.set_mode(&changed, AnalysisMode::Deep).unwrap();
    assert_eq!(
        ProjectService::open(hub.as_ref(), &changed).unwrap().mode,
        AnalysisMode::Deep
    );
    hub.hold(&changed, autocraft_analysis::Holder::Run).unwrap();
    assert_eq!(
        hub.set_mode(&changed, AnalysisMode::CleanRoom)
            .unwrap_err()
            .kind(),
        Some(ErrorKind::Busy)
    );
    assert_eq!(prefs.get(&changed).unwrap().mode, AnalysisMode::Deep);
    hub.release(&changed, &autocraft_analysis::Holder::Run)
        .unwrap();
    hub.shutdown();
    let reopened = AnalysisHub::try_new(config()).unwrap();
    for (id, original_mode) in projects {
        let expected = if id == changed {
            AnalysisMode::Deep
        } else {
            original_mode
        };
        assert_eq!(
            ProjectService::open(reopened.as_ref(), &id).unwrap().mode,
            expected
        );
        assert_eq!(reopened.view(&id).unwrap().mode, expected);
    }
}
