//! Reference: craft-anything ai/src/rebuild/feed.rs (300dba58).
//! Project conversation owner: native engine feed persistence and pending note delivery.
use crate::{SeesPictures, activity::Activity, notes::Notes};
use autocraft_core::*;
use ca_engine::{
    feed::{FeedItem, PlanStatus, PlanStep, UserItem},
    inbox::{Inbox, Request, RequestKind},
    questions::{self, AskedQuestion},
    workspace::{Step, Workspace},
};
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::Arc,
};
#[path = "feed/boundary.rs"]
mod boundary;
mod events;
#[cfg(test)]
mod tests;
pub(crate) use boundary::check_dto;
pub use boundary::{attach_image, attach_pictures, read_feed};

pub(crate) struct Continuity {
    root: PathBuf,
    project: ProjectId,
    sink: Arc<dyn RebuildSink>,
    items: Vec<FeedItem>,
    notes: Notes,
    activity: Activity,
    relayed: HashSet<String>,
    text: HashMap<String, String>,
    tools: HashMap<String, ToolStatus>,
    seeded: bool,
    delivery: Vec<NoteId>,
    progress_reset: bool,
    user_pictures: Vec<PathBuf>,
}
impl Continuity {
    pub(crate) fn new(
        root: PathBuf,
        project: ProjectId,
        sink: Arc<dyn RebuildSink>,
    ) -> Result<Self> {
        let ws = Workspace::open(&root).map_err(|_| Error::external("无法打开项目对话"))?;
        let items = ca_engine::feed::load(&ws);
        let relayed = items
            .iter()
            .filter_map(|i| match i {
                FeedItem::Ask { request, .. } => Some(request.id.clone()),
                _ => None,
            })
            .collect();
        Ok(Self {
            items,
            notes: Notes::new(root.clone()),
            activity: Activity::new(root.clone()),
            root,
            project,
            sink,
            relayed,
            text: HashMap::new(),
            tools: HashMap::new(),
            seeded: false,
            delivery: vec![],
            progress_reset: false,
            user_pictures: vec![],
        })
    }
    fn save(&self) -> Result<()> {
        ca_engine::feed::save(&self.root, &self.items)
            .map_err(|_| Error::external("无法保存项目对话"))
    }
    pub(crate) fn answered(&mut self, id: &str, shown: &str) -> Result<()> {
        if !self
            .items
            .iter()
            .any(|i| matches!(i,FeedItem::Ask {request,..} if request.id==id))
            && let Some(request) = Inbox::new(self.root.join("inbox"))
                .pending()
                .into_iter()
                .find(|r| r.id == id)
        {
            self.items.push(FeedItem::Ask {
                request,
                answer: None,
            });
            self.relayed.insert(id.into());
        }
        ca_engine::feed::answered(&mut self.items, id, shown);
        self.activity.record(LineKind::Note, shown)?;
        self.save()
    }
    pub(crate) fn progress_reset(&self) -> bool {
        self.progress_reset
    }
    pub(crate) fn progress_reset_saved(&mut self) {
        self.progress_reset = false;
    }
    pub(crate) fn user(&mut self, request: &RunRequest) -> Result<()> {
        self.user_pictures.clear();
        let text = request.shown.as_ref().or(request.message.as_ref());
        if text.is_some() || !request.pictures.is_empty() {
            let pictures = attach_pictures(&self.root, request.pictures.clone())?;
            self.user_pictures = pictures.clone();
            self.items.push(FeedItem::User(UserItem::new(
                text.cloned().unwrap_or_default(),
                pictures,
            )));
            self.save()?;
        }
        Ok(())
    }
    pub(crate) fn user_pictures(&self) -> Vec<PathBuf> {
        self.user_pictures.clone()
    }
    pub(crate) fn emit(&mut self, event: &RebuildEvent) -> Result<()> {
        match event {
            RebuildEvent::Step { step, text, .. } => {
                self.end_stream();
                self.items.push(FeedItem::Step {
                    step: source_step(*step),
                    text: text.clone(),
                });
                self.activity.record(LineKind::Stage, text)?;
            }
            RebuildEvent::Agent { event, .. } => self.agent(event)?,
            RebuildEvent::NoteRead {
                shown, pictures, ..
            } => {
                self.progress_reset = true;
                self.end_stream();
                self.items.push(FeedItem::User(UserItem::new(
                    shown.clone(),
                    pictures.clone(),
                )));
                self.activity
                    .record(LineKind::Note, "The builder read the added note.")?;
            }
            RebuildEvent::Ask { request, .. } => {
                self.items.push(FeedItem::Ask {
                    request: source_request(request),
                    answer: None,
                });
            }
            RebuildEvent::Checked { result, .. } => self
                .items
                .push(FeedItem::Check(boundary::source_check(result))),
            RebuildEvent::Finished {
                ok,
                message,
                paused,
                ..
            } => {
                self.end_stream();
                for item in &mut self.items {
                    if let FeedItem::Plan(steps) = item {
                        for step in steps {
                            if step.status == PlanStatus::Doing {
                                step.status = PlanStatus::Stopped;
                            }
                        }
                    }
                }
                if *paused {
                    self.items.push(FeedItem::Paused);
                } else if !ok && !matches!(self.items.last(), Some(FeedItem::Error { .. })) {
                    self.items.push(FeedItem::Error {
                        message: message.clone(),
                        remediation: None,
                        log: None,
                    });
                }
                self.activity.record(
                    if *ok {
                        LineKind::Done
                    } else if *paused {
                        LineKind::Note
                    } else {
                        LineKind::Failed
                    },
                    message,
                )?;
            }
            RebuildEvent::Preview { .. } | RebuildEvent::Queue { .. } => {}
        }
        self.save()?;
        self.sink.publish(event.clone());
        Ok(())
    }
    fn end_stream(&mut self) {
        if let Some(FeedItem::Agent { streaming, .. }) = self.items.last_mut() {
            *streaming = false;
        }
    }
    pub(crate) fn plan(&mut self, value: &Value) -> Result<()> {
        let Some(steps) = plan_from(value) else {
            return Err(Error::invalid("计划步骤格式无效"));
        };
        if self.items.iter().any(|i| matches!(i, FeedItem::Plan(_)))
            && ca_engine::feed::visible_plan(&self.items).is_none()
        {
            self.items
                .retain(|i| !matches!(i, FeedItem::Plan(_) | FeedItem::PlanDismissed));
        }
        if let Some(FeedItem::Plan(old)) = self
            .items
            .iter_mut()
            .find(|i| matches!(i, FeedItem::Plan(_)))
        {
            *old = steps;
        } else {
            self.items.push(FeedItem::Plan(steps));
        }
        self.save()
    }
    pub(crate) fn note(
        &mut self,
        text: &str,
        deep: bool,
        pictures: Vec<PathBuf>,
        session: Option<&dyn AgentSession>,
    ) -> Result<NoteId> {
        let ws = Workspace::open(&self.root).map_err(|_| Error::external("无法打开项目问答"))?;
        let asked = questions::untold(&ws, deep);
        let prefix = crate::brief::just_asked(&asked);
        let (id, read) = self.notes.add(text, &prefix, pictures, session)?;
        if let Some(note) = read {
            questions::mark_told(&ws, &asked).map_err(|_| Error::external("无法标记已告知问答"))?;
            self.emit(&RebuildEvent::NoteRead {
                project: self.project.clone(),
                id: note.id,
                shown: note.text,
                pictures: note.pictures,
            })?;
        }
        Ok(id)
    }
    pub(crate) fn withdraw(&mut self, id: &NoteId) -> Result<Option<String>> {
        self.notes.withdraw(id)
    }
    pub(crate) fn waiting_notes(&self) -> Vec<(NoteId, String)> {
        self.notes
            .waiting()
            .into_iter()
            .map(|n| (n.id, n.text))
            .collect()
    }
    pub(crate) fn added(
        &mut self,
        ws: &Workspace,
        deep: bool,
        _sees: SeesPictures,
    ) -> Result<Option<(String, Vec<AskedQuestion>)>> {
        let notes = self.notes.waiting();
        self.delivery = notes.iter().map(|n| n.id.clone()).collect();
        if notes.is_empty() {
            return Ok(None);
        }
        let asked = questions::untold(ws, deep);
        let mut text = crate::brief::just_asked(&asked);
        text.push_str("The person added this while you were working:\n");
        for note in notes {
            text.push_str(&note.text);
            for pic in note.pictures {
                text.push_str(&format!("\nPicture attached: {}", pic.display()));
            }
            text.push_str("\n\n");
        }
        Ok(Some((text, asked)))
    }
    pub(crate) fn delivered(&mut self) -> Result<()> {
        let ids = std::mem::take(&mut self.delivery);
        for note in self
            .notes
            .waiting()
            .into_iter()
            .filter(|n| ids.contains(&n.id))
        {
            self.notes.mark_read(&note.id)?;
            self.emit(&RebuildEvent::NoteRead {
                project: self.project.clone(),
                id: note.id,
                shown: note.text,
                pictures: note.pictures,
            })?;
        }
        Ok(())
    }
    pub(crate) fn relay(&mut self) -> Result<()> {
        for note in self.notes.relay() {
            self.emit(&RebuildEvent::NoteRead {
                project: self.project.clone(),
                id: note.id,
                shown: note.text,
                pictures: note.pictures,
            })?;
        }
        for request in Inbox::new(self.root.join("inbox")).pending() {
            if self.relayed.insert(request.id.clone()) {
                self.emit(&RebuildEvent::Ask {
                    project: self.project.clone(),
                    request: boundary::request_dto(&request),
                })?;
            }
        }
        Ok(())
    }
    pub(crate) fn activity(&self, offset: usize, limit: usize) -> Result<Vec<ActivityLine>> {
        if self.activity.is_empty() {
            crate::activity::read_lines(&self.root, offset, limit)
        } else {
            Ok(self.activity.lines(offset, limit))
        }
    }
    pub(crate) fn finish(&mut self) -> Result<()> {
        self.end_stream();
        self.save()
    }
    pub(crate) fn read_feed(&self) -> Result<Vec<FeedEntryDto>> {
        let ws = Workspace::open(&self.root).map_err(|_| Error::external("无法打开项目问答"))?;
        boundary::with_questions(&ws, &self.items)
    }
    pub(crate) fn attach_pictures(&self, pictures: Vec<PathBuf>) -> Result<Vec<PathBuf>> {
        attach_pictures(&self.root, pictures)
    }
    pub(crate) fn attach_image(&self, bytes: Vec<u8>, mime: &str) -> Result<PathBuf> {
        attach_image(&self.root, bytes, mime)
    }
}
fn source_step(step: StepDto) -> Step {
    match step {
        StepDto::Study => Step::Study,
        StepDto::Build => Step::Build,
        StepDto::Check => Step::Check,
        StepDto::Done => Step::Done,
    }
}
fn source_request(request: &InboxRequest) -> Request {
    Request {
        id: request.id.to_string(),
        kind: match request.kind {
            InboxKind::Question => RequestKind::Question,
            InboxKind::Approval => RequestKind::Approval,
        },
        title: request.title.clone(),
        detail: request.detail.clone(),
        choices: request.choices.clone(),
        allow_text: request.allow_text,
        action: request.remember.clone(),
        created_ms: request.created_ms,
    }
}
fn plan_from(value: &Value) -> Option<Vec<PlanStep>> {
    value
        .get("steps")?
        .as_array()?
        .iter()
        .map(|s| {
            let title = s.get("title")?.as_str()?.to_string();
            let status = match s.get("status")?.as_str()? {
                "todo" => PlanStatus::Todo,
                "doing" => PlanStatus::Doing,
                "done" => PlanStatus::Done,
                "blocked" => PlanStatus::Stopped,
                _ => return None,
            };
            Some(PlanStep {
                text: title.clone(),
                doing: title,
                status,
            })
        })
        .collect()
}
