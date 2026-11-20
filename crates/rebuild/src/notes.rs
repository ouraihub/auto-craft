//! Reference: craft-anything ai/src/rebuild/notes.rs (300dba58).
//! Atomic pending/read note ownership and engine-independent steering fallback.
use autocraft_core::{AgentSession, Error, IoContext, NoteId, Result};
use ca_engine::workspace::Workspace;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};
#[derive(Clone)]
pub(crate) struct Note {
    pub id: NoteId,
    pub text: String,
    pub pictures: Vec<PathBuf>,
}
pub(crate) struct Notes {
    root: PathBuf,
    pending: BTreeMap<u64, Note>,
}
impl Notes {
    pub(crate) fn new(root: PathBuf) -> Self {
        Self {
            root,
            pending: BTreeMap::new(),
        }
    }
    fn path(&self, id: &NoteId, read: bool) -> PathBuf {
        self.root.join("inbox/notes").join(format!(
            "{}{}.json",
            id.as_str(),
            if read { ".read" } else { "" }
        ))
    }
    pub(crate) fn add(
        &mut self,
        text: &str,
        prefix: &str,
        pictures: Vec<PathBuf>,
        session: Option<&dyn AgentSession>,
    ) -> Result<(NoteId, Option<Note>)> {
        if text.trim().is_empty() && pictures.is_empty() {
            return Err(Error::invalid("补充说明不能为空"));
        }
        let ws = Workspace::open(&self.root).map_err(|_| Error::external("无法打开项目"))?;
        let pictures = ws
            .add_attachments(&pictures)
            .map_err(|_| Error::external("无法保存说明图片"))?;
        let id = NoteId::generate();
        let number = id
            .as_str()
            .parse::<u64>()
            .map_err(|_| Error::corrupt("说明编号无效"))?;
        let note = Note {
            id: id.clone(),
            text: text.into(),
            pictures,
        };
        let prompt = format!("{prefix}{}", prompt(&note));
        std::fs::create_dir_all(self.root.join("inbox/notes")).context("无法创建补充说明目录")?;
        let bytes = serde_json::to_vec(&serde_json::json!({"id":number,"text":prompt}))
            .map_err(|_| Error::corrupt("无法编码补充说明"))?;
        ca_engine::project::write_atomic(&self.path(&id, false), &bytes)
            .map_err(|_| Error::external("无法保存补充说明"))?;
        self.pending.insert(number, note.clone());
        if let Some(session) = session.filter(|s| s.snapshot().capabilities.steer) {
            match session.steer(&prompt) {
                Ok(()) => {
                    self.mark_read(&id)?;
                    return Ok((id, Some(note)));
                }
                // Accepted=false: the file stays pending and the ordinary Notes step delivers it.
                Err(_) => return Ok((id, None)),
            }
        }
        Ok((id, None))
    }
    pub(crate) fn mark_read(&mut self, id: &NoteId) -> Result<()> {
        std::fs::rename(self.path(id, false), self.path(id, true)).context("无法标记已读说明")?;
        if let Ok(number) = id.as_str().parse::<u64>() {
            self.pending.remove(&number);
        }
        Ok(())
    }
    pub(crate) fn withdraw(&mut self, id: &NoteId) -> Result<Option<String>> {
        let number = id
            .as_str()
            .parse::<u64>()
            .map_err(|_| Error::invalid("说明编号无效"))?;
        if !self.pending.contains_key(&number) {
            return Ok(None);
        }
        match std::fs::remove_file(self.path(id, false)) {
            Ok(()) => Ok(self.pending.remove(&number).map(|n| n.text)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                self.pending.remove(&number);
                Ok(None)
            }
            Err(e) => Err(autocraft_core::Error::Io {
                context: "无法撤回补充说明".into(),
                source: e,
            }),
        }
    }
    pub(crate) fn waiting(&self) -> Vec<Note> {
        self.pending.values().cloned().collect()
    }
    pub(crate) fn relay(&mut self) -> Vec<Note> {
        let ids: Vec<_> = self
            .pending
            .iter()
            .filter(|(_, n)| self.path(&n.id, true).is_file())
            .map(|(id, _)| *id)
            .collect();
        ids.into_iter()
            .filter_map(|id| self.pending.remove(&id))
            .collect()
    }
}
fn prompt(note: &Note) -> String {
    let mut text = note.text.clone();
    for path in &note.pictures {
        text.push_str(&format!("\nPicture attached: {}", path.display()));
    }
    text
}
pub(crate) fn clear(root: &Path) -> Result<()> {
    match std::fs::remove_dir_all(root.join("inbox/notes")) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(autocraft_core::Error::Io {
            context: "无法清理上次补充说明".into(),
            source: e,
        }),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn withdraw_only_before_read_and_propagates_bad_directory() {
        let root = std::env::temp_dir().join(format!(
            "autocraft-notes-{}",
            ca_engine::inbox::next_counter()
        ));
        std::fs::create_dir_all(root.join("inbox/notes")).unwrap();
        let mut notes = Notes::new(root.clone());
        let id = NoteId::generate();
        let n = id.as_str().parse().unwrap();
        std::fs::write(notes.path(&id, false), b"{\"text\":\"hello\"}").unwrap();
        notes.pending.insert(
            n,
            Note {
                id: id.clone(),
                text: "hello".into(),
                pictures: vec![],
            },
        );
        assert_eq!(notes.withdraw(&id).unwrap(), Some("hello".into()));
        assert_eq!(notes.withdraw(&id).unwrap(), None);
        let id2 = NoteId::generate();
        assert!(id2.as_str().parse::<u64>().unwrap() > n);
        std::fs::write(notes.path(&id2, false), b"{}").unwrap();
        notes.pending.insert(
            id2.as_str().parse().unwrap(),
            Note {
                id: id2.clone(),
                text: "read".into(),
                pictures: vec![],
            },
        );
        notes.mark_read(&id2).unwrap();
        assert_eq!(notes.withdraw(&id2).unwrap(), None);
        assert!(notes.path(&id2, true).exists());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn writes_source_shape_and_keeps_pending_on_no_steer() {
        let temp = tempfile::tempdir().unwrap();
        let ws = Workspace::create(
            temp.path(),
            "notes",
            "request",
            ca_engine::workspace::TargetRef::None,
        )
        .unwrap();
        let mut notes = Notes::new(ws.root.clone());
        let (id, read) = notes
            .add("please keep the blue button", "", vec![], None)
            .unwrap();
        assert!(read.is_none());
        let saved: serde_json::Value =
            serde_json::from_slice(&std::fs::read(notes.path(&id, false)).unwrap()).unwrap();
        assert_eq!(saved["id"], id.as_str().parse::<u64>().unwrap());
        assert_eq!(saved["text"], "please keep the blue button");
        assert_eq!(
            notes.withdraw(&id).unwrap(),
            Some("please keep the blue button".into())
        );
        assert!(!notes.path(&id, false).exists());
        assert!(notes.add(" ", "", vec![], None).is_err());
        let blocked = Notes::new(temp.path().join("missing-workspace"));
        let mut blocked = blocked;
        assert!(blocked.add("not saved", "", vec![], None).is_err());
    }
}
#[cfg(test)]
#[path = "notes/steering_tests.rs"]
mod steering_tests;
