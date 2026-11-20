//! Waiting tools execute outside the shared engine session lock, matching upstream values.
use crate::{AnalysisHub, engines::Loaded};
use autocraft_core::{CancelFlag, ProjectId};
use ca_engine::{
    Progress, ToolOutput,
    inbox::{Inbox, RequestKind},
    tool::parse_params,
    tools::interaction_tools::{ApproveParams, AskParams, approve_with},
    workspace::{Questions, Workspace},
};
use ca_evidence::{ErrorCode, ToolError};
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};

const DECIDE_ANSWER: &str = "The person asked you to decide questions yourself in this project. Choose what makes the result closest to what they asked for, then start your next message with \"I decided:\" followed by the question, what you chose and why (so they can see it and change it), and continue.";
const WAIT: Duration = Duration::from_secs(30 * 60);
pub(crate) fn contains(tool: &str) -> bool {
    matches!(
        tool,
        "core_person_ask" | "core_action_approve" | "rebuild_check" | "rebuild_question_ask"
    )
}

#[allow(
    clippy::result_large_err,
    reason = "upstream ToolError boundary preserves host-tool error codes and response messages without boxing"
)]
pub(crate) fn execute(
    hub: &AnalysisHub,
    project: &ProjectId,
    loaded: &Arc<Loaded>,
    tool: &str,
    params: Value,
    cancel: &CancelFlag,
    progress: &ca_engine::tool::ProgressFn,
) -> Result<ToolOutput, ToolError> {
    if cancel.is_cancelled() {
        return Err(ToolError::cancelled());
    }
    let ws = Workspace::open(std::path::Path::new(project.as_str()))?;
    let policy = if let Some(s) = loaded.engine.try_session() {
        let policy = crate::engines::HostPolicy {
            auto_mode: s.settings.auto_mode,
            clean_room: s.clean_room(),
            questions: s.settings.questions_in(Some(&ws.manifest)),
        };
        *loaded.policy.lock() = policy;
        policy
    } else {
        *loaded.policy.lock()
    };
    let inbox = Inbox::new(ws.inbox_dir());
    match tool {
        "core_person_ask" => {
            let p: AskParams = parse_params(params)?;
            let question = p.question.trim();
            if question.is_empty() || question.len() > 2000 {
                return Err(ToolError::invalid("question must be 1..=2000 characters"));
            }
            let choices = p
                .choices
                .iter()
                .map(|c| c.trim().chars().take(120).collect::<String>())
                .filter(|c| !c.is_empty())
                .take(6)
                .collect();
            if policy.questions == Questions::Decide {
                return Ok(ToolOutput::Value(
                    json!({"answered":false,"note":DECIDE_ANSWER}),
                ));
            }
            let id = inbox.post(
                RequestKind::Question,
                question,
                p.context.trim(),
                choices,
                p.allow_text || p.choices.is_empty(),
                None,
            )?;
            progress(Progress {
                fraction: None,
                message: "Waiting for your answer…".into(),
            });
            let answer = inbox.wait(&id, WAIT, &|| cancel.is_cancelled());
            if cancel.is_cancelled() {
                return Err(ToolError::cancelled());
            }
            Ok(ToolOutput::Value(match answer {
                Some(a) => {
                    json!({"answered":true,"answer":a.as_text(),"choice":a.choice,"text":a.text})
                }
                None => {
                    json!({"answered":false,"note":"No answer came. Continue with your best judgement and state the assumption you made."})
                }
            }))
        }
        "core_action_approve" => {
            let p: ApproveParams = parse_params(params)?;
            let result = approve_with(
                Some(inbox),
                policy.auto_mode,
                policy.clean_room,
                &p.tool_name,
                &p.input,
                &|| cancel.is_cancelled(),
                &mut || {
                    progress(Progress {
                        fraction: None,
                        message: "Waiting for your approval…".into(),
                    })
                },
            );
            Ok(ToolOutput::Value(match result {
                Ok(Ok(())) => json!({"behavior":"allow","updatedInput":p.input}),
                Ok(Err(message)) => json!({"behavior":"deny","message":message}),
                Err(e) => {
                    json!({"behavior":"deny","message":format!("The approval check failed: {}",e.message)})
                }
            }))
        }
        "rebuild_check" | "rebuild_question_ask" => {
            let rebuild = hub.rebuild.as_ref().ok_or_else(|| {
                ToolError::new(
                    ErrorCode::FeatureMissing,
                    "The rebuild service is unavailable.",
                )
            })?;
            rebuild
                .call(project, tool, params, cancel)
                .map(ToolOutput::Value)
                .map_err(|e| {
                    let code = match e.kind() {
                        Some(autocraft_core::ErrorKind::Cancelled) => ErrorCode::Cancelled,
                        Some(autocraft_core::ErrorKind::Timeout) => ErrorCode::TimedOut,
                        _ => ErrorCode::FeatureMissing,
                    };
                    ToolError::new(code, e.detail())
                })
        }
        _ => Err(ToolError::internal("Not a host-executed tool")),
    }
}
