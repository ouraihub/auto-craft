//! Project inbox conversion, writes, and weak-owner 500ms polling lifecycle.
use crate::{AnalysisEvent, AnalysisHub, errors};
use autocraft_core::*;
use ca_engine::inbox::{Answer, Inbox, Request, RequestKind};
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, atomic::Ordering},
    time::Duration,
};

pub(crate) fn request(r: Request) -> InboxRequest {
    InboxRequest {
        id: r.id.into(),
        kind: match r.kind {
            RequestKind::Question => InboxKind::Question,
            RequestKind::Approval => InboxKind::Approval,
        },
        title: r.title,
        detail: r.detail,
        choices: r.choices,
        allow_text: r.allow_text,
        remember: r.action,
        created_ms: r.created_ms,
    }
}
impl InboxService for AnalysisHub {
    fn pending(&self, project: &ProjectId) -> Result<Vec<InboxRequest>> {
        let ws = ca_engine::workspace::Workspace::open(std::path::Path::new(project.as_str()))
            .map_err(|e| errors::convert("project_open", e))?;
        Ok(Inbox::new(ws.inbox_dir())
            .pending()
            .into_iter()
            .map(request)
            .collect())
    }
    fn answer(&self, project: &ProjectId, id: &InboxRequestId, answer: InboxAnswer) -> Result<()> {
        let ws = self.writable(project)?;
        if self.is_running(project) {
            return self
                .rebuild
                .as_ref()
                .ok_or_else(|| Error::new(ErrorKind::BackendUnavailable, "复刻回答服务不可用"))?
                .answer(project, id, answer);
        }
        let inbox = Inbox::new(ws.inbox_dir());
        if !inbox.pending().iter().any(|r| r.id == id.as_str()) {
            return Err(Error::new(
                ErrorKind::InteractionExpired,
                "卡片已撤回或已回答",
            ));
        }
        let answer = match answer {
            InboxAnswer::Choice { choice, text } => Answer {
                choice,
                text,
                ..Answer::default()
            },
            InboxAnswer::Approval {
                allow,
                always,
                text,
            } => Answer {
                allow: Some(allow),
                always,
                text,
                ..Answer::default()
            },
        };
        inbox
            .answer(id.as_str(), &answer)
            .map_err(|e| errors::convert("inbox_answer", e))
    }
}
pub(crate) fn start(hub: &Arc<AnalysisHub>) -> Result<()> {
    let weak = Arc::downgrade(hub);
    let span = tracing::Span::current();
    std::thread::Builder::new()
        .name("analysis-inbox".into())
        .spawn(move || {
            let _entered = span.enter();
            let mut known: HashMap<ProjectId, HashSet<InboxRequestId>> = HashMap::new();
            loop {
                std::thread::sleep(Duration::from_millis(500));
                let Some(hub) = weak.upgrade() else {
                    break;
                };
                if hub.stopped.load(Ordering::Acquire) {
                    break;
                }
                let projects: Vec<_> = hub.loaded.lock().keys().cloned().collect();
                known.retain(|project, _| projects.contains(project));
                for project in projects {
                    if hub.is_running(&project) {
                        known.remove(&project);
                        continue;
                    }
                    let pending = match hub.pending(&project) {
                        Ok(requests) => requests,
                        Err(_) => {
                            tracing::warn!(event = "analysis.inbox.scan_failed", code = "IO");
                            continue;
                        }
                    };
                    let old = known.entry(project.clone()).or_default();
                    let current: HashSet<_> = pending.iter().map(|r| r.id.clone()).collect();
                    for request in pending {
                        if !old.contains(&request.id) {
                            hub.sink.publish(AnalysisEvent::Inbox {
                                project: project.clone(),
                                request,
                            });
                        }
                    }
                    for id in old.difference(&current) {
                        hub.sink.publish(AnalysisEvent::InboxWithdrawn {
                            project: project.clone(),
                            id: id.clone(),
                        });
                    }
                    *old = current;
                }
            }
        })
        .map_err(|source| Error::Io {
            context: "无法启动分析卡片轮询".into(),
            source,
        })?;
    Ok(())
}
