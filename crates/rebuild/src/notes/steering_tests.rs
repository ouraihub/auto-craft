//! Behavioral tests for native steering acceptance and pending file ownership.
use super::*;
use autocraft_core::*;
use std::sync::Mutex;
struct Session {
    accept: bool,
    seen: Mutex<String>,
}
impl AgentSession for Session {
    fn snapshot(&self) -> SessionSnapshot {
        SessionSnapshot {
            id: ChatId::from("test"),
            backend: BackendKind::Pi,
            native_session_key: None,
            cwd: PathBuf::new(),
            phase: SessionPhase::Running,
            model: None,
            messages: vec![],
            interactions: vec![],
            capabilities: Capabilities {
                steer: true,
                ..Capabilities::default()
            },
            output_truncated: false,
            last_error: None,
            commands: vec![],
            session_name: None,
            revision: 1,
        }
    }
    fn prompt(&self, _: &str) -> Result<()> {
        Err(Error::invalid("unexpected prompt"))
    }
    fn steer(&self, text: &str) -> Result<()> {
        *self.seen.lock().map_err(|_| Error::external("test lock"))? = text.into();
        if self.accept {
            Ok(())
        } else {
            Err(Error::external("engine rejected steering"))
        }
    }
    fn stop(&self) -> Result<()> {
        Err(Error::invalid("unexpected stop"))
    }
    fn respond(&self, _: &str, _: InteractionAnswer) -> Result<()> {
        Err(Error::invalid("unexpected response"))
    }
    fn dispose(&self) -> Result<()> {
        Err(Error::invalid("unexpected dispose"))
    }
    fn set_model(&self, _: &str, _: &str, _: Option<ThinkingLevel>) -> Result<()> {
        Err(Error::invalid("unexpected model change"))
    }
}
#[test]
fn accepted_steer_claims_file_and_failed_steer_really_queues() {
    let temp = tempfile::tempdir().unwrap();
    let ws = Workspace::create(
        temp.path(),
        "steer",
        "request",
        ca_engine::workspace::TargetRef::None,
    )
    .unwrap();
    let mut notes = Notes::new(ws.root.clone());
    let accepted = Session {
        accept: true,
        seen: Mutex::new(String::new()),
    };
    let (id, read) = notes
        .add(
            "make it blue",
            "Earlier answer: app/\n",
            vec![],
            Some(&accepted),
        )
        .unwrap();
    assert!(read.is_some());
    assert!(notes.path(&id, true).is_file());
    assert!(!notes.path(&id, false).exists());
    assert_eq!(notes.withdraw(&id).unwrap(), None);
    assert!(
        accepted
            .seen
            .lock()
            .unwrap()
            .starts_with("Earlier answer: app/\n")
    );
    let rejected = Session {
        accept: false,
        seen: Mutex::new(String::new()),
    };
    let (id, read) = notes
        .add("make it green", "", vec![], Some(&rejected))
        .unwrap();
    assert!(read.is_none());
    assert!(notes.path(&id, false).is_file());
    assert_eq!(notes.waiting().len(), 1);
    assert_eq!(notes.withdraw(&id).unwrap(), Some("make it green".into()));
}
