//! OMP extension tool events carry structured input; approval UI prose is never parsed.
use autocraft_core::{ActionShell, AgentAction};
use serde_json::Value;

pub(super) fn action(event: &Value) -> Option<AgentAction> {
    let tool = event["toolName"].as_str().unwrap_or("");
    let input = &event["input"];
    let text = |key: &str| input[key].as_str().unwrap_or_default().to_owned();
    match tool {
        "read" | "grep" | "find" | "ls" | "glob" => None,
        "write" | "edit" if input["path"].as_str().is_some_and(|path| !path.is_empty()) => {
            Some(AgentAction::WriteFile { path: text("path") })
        }
        "bash" if input["command"].is_string() => Some(AgentAction::RunCommand {
            command: text("command"),
            shell: ActionShell::Bash,
        }),
        "powershell" if input["command"].is_string() => Some(AgentAction::RunCommand {
            command: text("command"),
            shell: ActionShell::PowerShell,
        }),
        name if name.starts_with("mcp__autocraft_analysis_") => None,
        _ => Some(AgentAction::Other {
            tool: tool.into(),
            input: input.clone(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn actual_p03_edit_hook_preserves_the_policy_target() {
        let frames: Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/omp/approval-edit-turn.json"
        ))
        .unwrap();
        let event = frames
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|frame| frame["message"].as_str())
            .find_map(|message| message.strip_prefix("autocraft:tool_call:"))
            .unwrap();
        let event: Value = serde_json::from_str(event).unwrap();
        assert_eq!(
            action(&event),
            Some(AgentAction::WriteFile {
                path: "probe-edit.txt".into()
            })
        );
    }
}
