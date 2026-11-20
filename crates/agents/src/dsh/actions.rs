//! ACP permission action translation and exact option-kind selection.
use autocraft_core::{ActionShell, AgentAction, PolicyVerdict};
use serde_json::{Value, json};

pub(super) fn action(call: &Value) -> Option<AgentAction> {
    let input = &call["rawInput"];
    let text = |key: &str| input[key].as_str().unwrap_or_default().to_owned();
    match call["title"].as_str() {
        Some("write" | "edit") if input["file_path"].is_string() => {
            return Some(AgentAction::WriteFile {
                path: text("file_path"),
            });
        }
        Some("pwsh" | "powershell") if input["command"].is_string() => {
            return Some(AgentAction::RunCommand {
                command: text("command"),
                shell: ActionShell::PowerShell,
            });
        }
        Some("bash") if input["command"].is_string() => {
            return Some(AgentAction::RunCommand {
                command: text("command"),
                shell: ActionShell::Bash,
            });
        }
        Some("web_fetch") if input["url"].is_string() => {
            return Some(AgentAction::WebFetch { url: text("url") });
        }
        Some("web_search") if input["queries"].is_array() => {
            return Some(AgentAction::WebSearch {
                query: input["queries"].to_string(),
            });
        }
        _ => {}
    }
    match call["kind"].as_str().unwrap_or("") {
        "read" | "search" | "think" | "switch_mode" => None,
        "edit" | "delete" | "move" => {
            let path = call["locations"][0]["path"]
                .as_str()
                .or_else(|| input["path"].as_str())
                .or_else(|| input["file_path"].as_str())
                .or_else(|| input["abs_path"].as_str())
                .unwrap_or_default();
            if path.is_empty() {
                return Some(AgentAction::Other {
                    tool: call["title"].as_str().unwrap_or_default().into(),
                    input: input.clone(),
                });
            }
            Some(AgentAction::WriteFile { path: path.into() })
        }
        "execute" => {
            let command = match &input["command"] {
                Value::Array(parts) if parts.iter().all(Value::is_string) => parts
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(" "),
                Value::String(command) => command.clone(),
                _ => {
                    return Some(AgentAction::Other {
                        tool: "execute".into(),
                        input: input.clone(),
                    });
                }
            };
            Some(AgentAction::RunCommand {
                command,
                shell: ActionShell::Bash,
            })
        }
        "fetch" if input["url"].is_string() => Some(AgentAction::WebFetch { url: text("url") }),
        _ => Some(AgentAction::Other {
            tool: call["title"].as_str().unwrap_or_default().into(),
            input: input.clone(),
        }),
    }
}

pub(super) fn outcome(options: &Value, verdict: PolicyVerdict) -> Value {
    let kind = match verdict {
        PolicyVerdict::Allow => "allow_once",
        PolicyVerdict::Refuse { .. } => "reject_once",
    };
    match options
        .as_array()
        .and_then(|options| options.iter().find(|option| option["kind"] == kind))
        .and_then(|option| option["optionId"].as_str())
    {
        Some(id) => json!({"outcome":"selected","optionId":id}),
        None => json!({"outcome":"cancelled"}),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_tool_start_fields_define_policy_action() {
        assert_eq!(
            action(
                &json!({"title":"write","kind":"other","rawInput":{"file_path":"C:/outside/file"}})
            ),
            Some(AgentAction::WriteFile {
                path: "C:/outside/file".into()
            })
        );
        assert_eq!(
            action(
                &json!({"title":"pwsh","kind":"other","rawInput":{"command":"git push origin main"}})
            ),
            Some(AgentAction::RunCommand {
                command: "git push origin main".into(),
                shell: ActionShell::PowerShell
            })
        );
    }
    #[test]
    fn policy_never_chooses_an_allow_always_or_unoffered_option() {
        let options = json!([{"kind":"allow_always","optionId":"always"},{"kind":"reject_once","optionId":"no"}]);
        assert_eq!(
            outcome(&options, PolicyVerdict::Allow),
            json!({"outcome":"cancelled"})
        );
        assert_eq!(
            outcome(
                &options,
                PolicyVerdict::Refuse {
                    reason: "no".into()
                }
            ),
            json!({"outcome":"selected","optionId":"no"})
        );
        assert_eq!(
            action(&json!({"kind":"execute","rawInput":{"command":["echo","safe"]}})),
            Some(AgentAction::RunCommand {
                command: "echo safe".into(),
                shell: ActionShell::Bash
            })
        );
        assert_eq!(
            action(
                &json!({"kind":"move","locations":[{"path":"checks/a"}],"rawInput":{"path":"other"}})
            ),
            Some(AgentAction::WriteFile {
                path: "checks/a".into()
            })
        );
    }
}
