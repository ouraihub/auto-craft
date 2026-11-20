//! Pi tool-call input to the builder action contract. MCP approval stays on its channel.
use autocraft_core::{ActionShell, AgentAction};
use serde_json::Value;

pub(super) fn action(event: &Value) -> Option<AgentAction> {
    let tool = event["toolName"].as_str().unwrap_or("");
    let input = &event["input"];
    let text = |key: &str| input[key].as_str().unwrap_or_default().to_owned();
    match tool {
        "read" | "grep" | "find" | "ls" => None,
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
        name if name.starts_with("mcp__autocraft_analysis__") => None,
        _ => Some(AgentAction::Other {
            tool: tool.into(),
            input: input.clone(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn unknown_tools_are_not_treated_as_readonly() {
        assert!(matches!(
            action(&json!({"toolName":"unknown","input":{"path":"x"}})),
            Some(AgentAction::Other { .. })
        ));
        assert_eq!(
            action(&json!({"toolName":"write","input":{"path":"app/x"}})),
            Some(AgentAction::WriteFile {
                path: "app/x".into()
            })
        );
        assert!(
            action(&json!({"toolName":"mcp__autocraft_analysis__native_functions_list"})).is_none()
        );
    }
}
