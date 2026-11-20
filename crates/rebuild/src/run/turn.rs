//! Reference: craft-anything crates/ai/src/rebuild/mod.rs::turn_with_wait (300dba58).
//! Actual session completion, bounded structured retries and polling-time continuity.
use super::{Run, usage::Usage};
use crate::{RebuildConfig, builder::backend_id, session_copy};
use autocraft_core::*;
use ca_engine::workspace::Workspace;
use std::{
    collections::HashSet,
    sync::{Arc, Mutex, PoisonError},
    thread,
    time::{Duration, Instant},
};
pub(super) struct Events {
    run: Arc<Run>,
    pending: Mutex<Vec<SessionEvent>>,
}
impl Events {
    pub fn new(run: Arc<Run>) -> Self {
        Self {
            run,
            pending: Mutex::new(Vec::new()),
        }
    }
    pub fn drain(&self) -> Result<()> {
        let pending =
            std::mem::take(&mut *self.pending.lock().unwrap_or_else(PoisonError::into_inner));
        for event in pending {
            self.run.emit(RebuildEvent::Agent {
                project: self.run.project.clone(),
                event,
            })?;
        }
        Ok(())
    }
}
impl SessionSink for Events {
    fn publish(&self, event: SessionEvent) {
        self.pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(event);
    }
}
pub(super) fn run<F>(
    config: &RebuildConfig,
    ws: &mut Workspace,
    session: &dyn AgentSession,
    events: &Events,
    prompt: &str,
    usage: &mut Usage,
    mut self_checks: F,
) -> Result<String>
where
    F: FnMut(&Workspace) -> Result<()>,
{
    let run = events.run.as_ref();
    let mut attempt = 0;
    let mut text = prompt.to_owned();
    loop {
        run.cancelled()?;
        let before = session.snapshot();
        let old: HashSet<_> = before.messages.iter().map(|m| m.id.clone()).collect();
        session.prompt(&text)?;
        let outcome = loop {
            if run.cancel.is_cancelled() {
                session.stop()?;
                break Err(SessionError {
                    code: "CANCELLED".into(),
                    message: "复刻已暂停".into(),
                    trace_id: None,
                    retry: None,
                });
            }
            events.drain()?;
            run.continuity
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .relay()?;
            self_checks(ws)?;
            if usage.due() {
                usage.record(ws, session)?;
            }
            let snapshot = session.snapshot();
            if snapshot.revision <= before.revision {
                thread::sleep(Duration::from_millis(120));
                continue;
            }
            if let Some(error) = snapshot.last_error.clone() {
                break Err(error);
            }
            match snapshot.phase {
                SessionPhase::Idle => break Ok(snapshot),
                SessionPhase::Closed => {
                    break Err(SessionError {
                        code: "SESSION_CLOSED".into(),
                        message: "构建方连接已关闭".into(),
                        trace_id: None,
                        retry: None,
                    });
                }
                SessionPhase::Running | SessionPhase::Waiting => {
                    thread::sleep(Duration::from_millis(120))
                }
            }
        };
        events.drain()?;
        usage.record(ws, session)?;
        let saved = session.snapshot();
        if let Some(key) = saved.native_session_key.as_ref() {
            let backend = backend_id(saved.backend);
            ws.manifest.agent_session = Some(key.as_str().into());
            ws.manifest.agent_session_by = Some(backend.into());
            config.host.bind(&run.project, saved.backend, key)?;
            if !session_copy::save(&ws.root, backend, key)? && saved.backend != BackendKind::Dsh {
                run.emit(RebuildEvent::Step {
                    project: run.project.clone(),
                    step: StepDto::Build,
                    text: "构建方会话超过 64 MB，未复制到项目；原会话仍然保留。".into(),
                })?;
            }
            ws.save().map_err(super::source)?;
        }
        match outcome {
            Ok(snapshot) => {
                let (text, decompiled) = reply(&snapshot.messages, &old);
                ws.manifest.decompiled |= decompiled;
                return Ok(text);
            }
            Err(error) => {
                if run.cancel.is_cancelled() {
                    return Err(Error::new(ErrorKind::Cancelled, "复刻已暂停"));
                }
                let Some(hint) = error.retry else {
                    return Err(Error::external(error.message));
                };
                if saved.backend == BackendKind::Pi || attempt >= 3 {
                    return Err(Error::external(error.message));
                }
                attempt += 1;
                let seconds = hint.after_secs.unwrap_or(30 * attempt).min(900);
                run.emit(RebuildEvent::Step {
                    project: run.project.clone(),
                    step: StepDto::Build,
                    text: format!("AI 服务暂时不可用，{seconds} 秒后第 {attempt}/3 次重试…"),
                })?;
                let until = Instant::now() + Duration::from_secs(seconds);
                while Instant::now() < until {
                    run.cancelled()?;
                    run.continuity
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .relay()?;
                    thread::sleep(Duration::from_millis(120));
                }
                text="Your last turn was cut off. Carry on from where you stopped: look at what is already in app/ and your checklist, then finish the task.".into();
            }
        }
    }
}
fn reply(messages: &[Message], old: &HashSet<String>) -> (String, bool) {
    let mut replies = Vec::new();
    let mut decompiled = false;
    for message in messages.iter().filter(|m| !old.contains(&m.id)) {
        for block in &message.content {
            match block {
                ContentBlock::Text { text } if message.role == Role::Assistant => {
                    replies.push(text.as_str())
                }
                ContentBlock::Tool {
                    name,
                    status: ToolStatus::Completed,
                    ..
                } if name.rsplit("__").next() == Some("native_function_pseudocode") => {
                    decompiled = true
                }
                ContentBlock::Text { .. }
                | ContentBlock::Thinking { .. }
                | ContentBlock::Image { .. }
                | ContentBlock::Tool { .. } => {}
            }
        }
    }
    (replies.join("\n"), decompiled)
}
#[cfg(test)]
#[path = "turn_tests.rs"]
mod tests;
