//! ACP `session/request_permission`: permission cards, their replies and turn-cancel answers.

use autocraft_core::{
    ContentBlock, Error, Interaction, InteractionAnswer, InteractionKind, InteractionOption,
    Result, SessionSnapshot,
};
use serde_json::{Value, json};
use tracing::warn;

use super::{DshSession, str_of};
use crate::session::new_id;

impl DshSession {
    pub(super) fn on_permission(&self, id: Value, params: &Value) -> Result<()> {
        if params["sessionId"] != *self.key.lock() {
            let cancelled = json!({"outcome": {"outcome": "cancelled"}});
            return self
                .live
                .process
                .send(&json!({"jsonrpc": "2.0", "id": id, "result": cancelled}));
        }
        let options = params["options"]
            .as_array()
            .filter(|o| !o.is_empty())
            .ok_or_else(|| Error::protocol("权限请求缺少 options"))?;
        if self.live.approvals.builder.is_some() || self.live.approvals.readonly {
            let options = params["options"].clone();
            let mut call = params["toolCall"].clone();
            let call_id = call["toolCallId"].as_str().unwrap_or_default();
            let snapshot = self.live.state.lock().snapshot();
            if let Some((name, input)) = snapshot
                .messages
                .iter()
                .flat_map(|message| &message.content)
                .find_map(|block| match block {
                    ContentBlock::Tool {
                        tool_call_id,
                        name,
                        input,
                        ..
                    } if tool_call_id == call_id => Some((name, input)),
                    _ => None,
                })
            {
                call["title"] = json!(name);
                if let Some(input) = input {
                    call["rawInput"] = input.clone();
                }
            }
            let action = super::actions::action(&call);
            let cancelled =
                json!({"jsonrpc":"2.0","id":id,"result":{"outcome":{"outcome":"cancelled"}}});
            return crate::approvals::Approvals::decide(
                &self.live,
                action,
                cancelled,
                move |verdict| json!({"jsonrpc":"2.0","id":id,"result":{"outcome":super::actions::outcome(&options, verdict)}}),
            );
        }
        let tool_call_id = params["toolCall"]["toolCallId"]
            .as_str()
            .unwrap_or_default();
        let title = {
            let p = self.live.state.lock();
            p.tool_owner(tool_call_id)
                .and_then(|owner| tool_name(&p.snapshot(), &owner, tool_call_id))
        }
        .unwrap_or_default();
        let interaction = Interaction {
            request_id: new_id(),
            kind: InteractionKind::Permission,
            title,
            message: String::new(),
            options: options
                .iter()
                .map(|o| InteractionOption {
                    id: str_of(&o["optionId"]),
                    label: str_of(&o["name"]),
                })
                .collect(),
            detail: params.clone(),
        };
        self.permissions
            .lock()
            .insert(interaction.request_id.clone(), id);
        self.live.state.lock().add_interaction(interaction);
        Ok(())
    }

    pub(super) fn reply_permission(&self, rpc_id: Value, outcome: Value) -> Result<()> {
        self.live
            .process
            .send(&json!({"jsonrpc": "2.0", "id": rpc_id, "result": {"outcome": outcome}}))
    }

    /// ACP: a client cancelling a turn answers its open permission requests `cancelled` first.
    pub(super) fn cancel_permissions(&self) {
        let open: Vec<Value> = self.permissions.lock().drain().map(|(_, id)| id).collect();
        let had = !self.live.state.lock().take_interactions().is_empty();
        let failed = open
            .into_iter()
            .map(|rpc_id| self.reply_permission(rpc_id, json!({"outcome": "cancelled"})))
            .filter(Result::is_err)
            .count();
        if failed != 0 {
            warn!(
                failed_count = failed,
                "agent.permission.cancel_cleanup_failed"
            );
        }
        if had {
            self.live.state.lock().publish();
        }
    }
}

fn tool_name(snapshot: &SessionSnapshot, owner: &str, tool_call_id: &str) -> Option<String> {
    snapshot
        .messages
        .iter()
        .find(|m| m.id == owner)?
        .content
        .iter()
        .find_map(|block| match block {
            ContentBlock::Tool {
                tool_call_id: id,
                name,
                ..
            } if id == tool_call_id => Some(name.clone()),
            ContentBlock::Tool { .. }
            | ContentBlock::Text { .. }
            | ContentBlock::Thinking { .. }
            | ContentBlock::Image { .. } => None,
        })
}

/// The ACP permission `outcome` for a user's answer; only offered options or a cancel are valid.
pub(super) fn permission_outcome(
    interaction: &Interaction,
    answer: &InteractionAnswer,
) -> Result<Value> {
    match answer {
        InteractionAnswer::Cancel => Ok(json!({"outcome": "cancelled"})),
        InteractionAnswer::Select { option_id }
            if interaction.options.iter().any(|o| &o.id == option_id) =>
        {
            Ok(json!({"outcome": "selected", "optionId": option_id}))
        }
        InteractionAnswer::Select { .. }
        | InteractionAnswer::Confirm { .. }
        | InteractionAnswer::Text { .. }
        | InteractionAnswer::Answers { .. } => {
            Err(Error::invalid("权限请求只能选择给出的选项或取消"))
        }
    }
}
