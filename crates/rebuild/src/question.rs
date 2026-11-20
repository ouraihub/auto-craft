//! Reference: craft-anything ai/src/rebuild/question.rs (300dba58).
//! Bounded, disposable readonly questions using the injected real agent runtime.
use crate::builder::BuilderHost;
use autocraft_core::*;
use ca_engine::{
    questions::{self, AskedQuestion},
    workspace::{ProjectUsage, Workspace},
};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime},
};
pub const WAIT: Duration = Duration::from_secs(10 * 60);
const PREAMBLE: &str = "You're answering a question the person asked about the project you built inside autocraft. This isn't a task: answer it and stop. Don't create, change or delete any file, don't run anything, and don't carry on the build; if the answer is that something should change, say what you'd change and that they can ask for it as a change. Answer in plain words for someone who doesn't code, briefly, with full paths when they ask where something is. You're a short-lived copy with fewer tools than the builder, and the build may still be running beside you. So a tool you can't see here is not a tool the build has lost, and a call with no result here may still be running: never report tools as disconnected or work as cut off because of what you see from here.\n\n";
const SKIPPED: &[&str] = &[
    "node_modules",
    "target",
    ".godot",
    "Library",
    "Temp",
    "obj",
    ".venv",
    "__pycache__",
    ".next",
    ".turbo",
    ".git",
];
#[derive(Default)]
struct AnswerSink(Mutex<Vec<SessionEvent>>);
impl SessionSink for AnswerSink {
    fn publish(&self, event: SessionEvent) {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(event);
    }
}
/// Launches a real readonly session; only the composition root may assert verified fork support.
pub(crate) fn ask(
    host: &dyn BuilderHost,
    project: &ProjectId,
    ws: &Workspace,
    deep: bool,
    question: &str,
    cancel: &CancelFlag,
) -> Result<Answered> {
    let question = question.trim();
    validate(question)?;
    if cancel.is_cancelled() {
        return Err(Error::new(ErrorKind::Cancelled, "问题已停止"));
    }
    let sink = Arc::new(AnswerSink::default());
    let (session, from) = host.start_question(project, deep, ws.root.clone(), sink.clone())?;
    let baseline = session.snapshot().messages;
    let mut reported_usage = None;
    let result = (|| {
        let baseline_usage = session.usage_snapshot()?;
        let mut prompt = String::from(PREAMBLE);
        prompt.push_str(&context(ws)?);
        if from == AnsweredFrom::TheFiles {
            if !ws.manifest.last_reply.is_empty() {
                prompt.push_str("\nLast builder reply:\n");
                prompt.extend(ws.manifest.last_reply.chars().take(4000));
            }
            prompt.push_str("\nRecent conversation:\n");
            let feed = ca_engine::feed::load(ws);
            let tail = &feed[feed.len().saturating_sub(20)..];
            prompt.push_str(&crate::session_copy::handoff(tail));
        }
        prompt.push_str("\n\nQuestion: ");
        prompt.push_str(question);
        session.prompt(&prompt)?;
        let start = Instant::now();
        let mut idle_since = None;
        loop {
            if cancel.is_cancelled() {
                session.stop()?;
                return Err(Error::new(ErrorKind::Cancelled, "问题已停止"));
            }
            if start.elapsed() >= WAIT {
                session.stop()?;
                return Err(Error::timeout("等待回答超时，请缩短问题后重试"));
            }
            let events: Vec<_> = sink
                .0
                .lock()
                .map_err(|_| Error::external("无法读取问题回答"))?
                .drain(..)
                .collect();
            for event in events {
                if let SessionEvent::Error { .. } = event {
                    return Err(Error::external("回答问题的 Agent 失败，请重试"));
                }
            }
            let snapshot = session.snapshot();
            if snapshot.last_error.is_some() {
                return Err(Error::external("回答问题的 Agent 失败，请重试"));
            }
            match snapshot.phase {
                SessionPhase::Running | SessionPhase::Waiting => idle_since = None,
                SessionPhase::Idle | SessionPhase::Closed => {
                    if idle_since.get_or_insert_with(Instant::now).elapsed()
                        >= Duration::from_secs(2)
                    {
                        let answer = snapshot
                            .messages
                            .iter()
                            .rev()
                            .find(|m| {
                                m.role == Role::Assistant && !baseline.iter().any(|old| old == *m)
                            })
                            .map(|m| {
                                m.content
                                    .iter()
                                    .filter_map(|b| match b {
                                        ContentBlock::Text { text } => Some(text.as_str()),
                                        ContentBlock::Thinking { .. }
                                        | ContentBlock::Image { .. }
                                        | ContentBlock::Tool { .. } => None,
                                    })
                                    .collect::<String>()
                            })
                            .unwrap_or_default();
                        if answer.trim().is_empty() {
                            return Err(Error::external("AI 已停止，但没有回答问题"));
                        }
                        let mut usage = session.usage_snapshot()?;
                        if let (Some(after), Some(before)) =
                            (usage.as_mut(), baseline_usage.as_ref())
                        {
                            after.input_tokens = after
                                .input_tokens
                                .map(|v| v.saturating_sub(before.input_tokens.unwrap_or(0)));
                            after.output_tokens = after
                                .output_tokens
                                .map(|v| v.saturating_sub(before.output_tokens.unwrap_or(0)));
                            after.cost_usd = after
                                .cost_usd
                                .map(|v| (v - before.cost_usd.unwrap_or(0.0)).max(0.0));
                        }
                        let (cost, estimate) = usage
                            .as_ref()
                            .map(|u| (u.cost_usd, u.estimate))
                            .unwrap_or((None, false));
                        reported_usage = usage;
                        let backend = match snapshot.backend {
                            BackendKind::Pi => "pi",
                            BackendKind::Omp => "omp",
                            BackendKind::Dsh => "dsh",
                        }
                        .to_string();
                        return Ok(Answered {
                            answer: answer.trim().into(),
                            from,
                            backend,
                            cost_usd: cost,
                            cost_is_estimate: estimate,
                            builder_notice: None,
                        });
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    })();
    let disposed = session.dispose();
    let answered = match (result, disposed) {
        (Ok(answer), Ok(())) => answer,
        (Err(error), Ok(())) => return Err(error),
        (Ok(_), Err(error)) => return Err(error),
        (Err(error), Err(_)) => {
            tracing::warn!(event = "rebuild.question.cleanup_failed");
            return Err(error);
        }
    };
    let after = ca_engine::feed::load(ws).len();
    let asked = AskedQuestion {
        deep,
        ..AskedQuestion::new(
            question.into(),
            answered.answer.clone(),
            answered.from == AnsweredFrom::BuildersSession,
            answered.backend.clone(),
            answered.cost_usd,
            after,
        )
    };
    questions::add(ws, asked).map_err(|_| Error::external("回答已完成，但保存问答失败"))?;
    ProjectUsage::update(&ws.root, |u| {
        let (input, output) = reported_usage
            .as_ref()
            .map(|u| (u.input_tokens.unwrap_or(0), u.output_tokens.unwrap_or(0)))
            .unwrap_or((0, 0));
        u.add_turn(input, output, answered.cost_usd, answered.cost_is_estimate)
    })
    .map_err(|_| Error::external("回答已完成，但保存问题用量失败"))?;
    Ok(answered)
}
fn validate(question: &str) -> Result<()> {
    if !(1..=4000).contains(&question.chars().count()) {
        return Err(Error::invalid("问题长度必须为 1–4000 字"));
    }
    Ok(())
}
fn context(ws: &Workspace) -> Result<String> {
    let m = &ws.manifest;
    let mut text = format!(
        "Project: {}\nWhat the person asked for: {}\nProject folder: {}\nThe app's folder: {}\nStatus: {}\nTarget: {:?}\nBuilt with: {}",
        m.name,
        m.request,
        ws.root.display(),
        ws.app_dir().display(),
        m.status.label(),
        m.target,
        m.stack
    );
    for path in recent_files(&ws.app_dir())? {
        let ago = path
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| SystemTime::now().duration_since(t).ok())
            .map(|d| d.as_secs() / 60)
            .unwrap_or(0);
        text.push_str(&format!("\n- {} ({ago} min ago)", path.display()));
    }
    Ok(text)
}
fn recent_files(app: &Path) -> Result<Vec<PathBuf>> {
    let mut stack = vec![(app.to_path_buf(), 0)];
    let mut files = Vec::new();
    let mut scanned = 0;
    while let Some((dir, depth)) = stack.pop() {
        if scanned >= 20_000 {
            break;
        }
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => {
                return Err(Error::Io {
                    context: "无法扫描最近修改的项目文件".into(),
                    source: e,
                });
            }
        };
        for entry in entries {
            let entry = entry.context("无法读取项目文件条目")?;
            scanned += 1;
            if scanned > 20_000 {
                break;
            }
            let kind = entry.file_type().context("无法读取项目文件类型")?;
            if kind.is_dir()
                && depth < 12
                && !entry
                    .file_name()
                    .to_str()
                    .is_some_and(|n| SKIPPED.contains(&n))
            {
                stack.push((entry.path(), depth + 1));
            } else if kind.is_file() {
                let modified = entry
                    .metadata()
                    .and_then(|m| m.modified())
                    .context("无法读取项目文件修改时间")?;
                files.push((modified, entry.path()));
            }
        }
    }
    files.sort_by_key(|(at, _)| std::cmp::Reverse(*at));
    Ok(files.into_iter().take(25).map(|(_, p)| p).collect())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn character_not_byte_boundary() {
        assert!(validate("").is_err());
        assert!(validate(&"界".repeat(4000)).is_ok());
        assert!(validate(&"界".repeat(4001)).is_err());
    }
    #[test]
    fn recent_scan_skips_generated_and_links() {
        let root = std::env::temp_dir().join(format!(
            "autocraft-recent-{}",
            ca_engine::inbox::next_counter()
        ));
        std::fs::create_dir_all(root.join("node_modules")).unwrap();
        std::fs::write(root.join("main.rs"), b"x").unwrap();
        std::fs::write(root.join("node_modules/generated"), b"x").unwrap();
        let found = recent_files(&root).unwrap();
        assert_eq!(found, vec![root.join("main.rs")]);
        std::fs::remove_dir_all(root).unwrap();
    }
}
