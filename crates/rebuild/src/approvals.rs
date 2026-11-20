//! Reference: craft-anything engine/tools/interaction_tools.rs (300dba58).
//! Native builder actions use the engine's single project approval policy.
use autocraft_core::{ActionPolicy, ActionShell, AgentAction, CancelFlag, PolicyVerdict};
use ca_engine::{inbox::Inbox, tools::interaction_tools::approve_with};
use serde_json::{Value, json};
use std::{
    path::{Component, Path, PathBuf},
    sync::Arc,
};

/// A run's fixed modes and its project approval inbox.
pub struct Policy {
    root: PathBuf,
    auto_mode: bool,
    clean_room: bool,
    waiting: Arc<dyn Fn() + Send + Sync>,
}
impl Policy {
    pub fn new(
        root: PathBuf,
        auto_mode: bool,
        clean_room: bool,
        waiting: Arc<dyn Fn() + Send + Sync>,
    ) -> Self {
        Self {
            root,
            auto_mode,
            clean_room,
            waiting,
        }
    }
}
/// Translate native tool actions without reimplementing the seven engine categories.
fn to_claude_shape(action: &AgentAction) -> (&str, Value) {
    match action {
        AgentAction::WriteFile { path } => ("Write", json!({"file_path":path})),
        AgentAction::RunCommand {
            command,
            shell: ActionShell::Bash,
        } => ("Bash", json!({"command":command})),
        // The engine's Bash policy also parses PowerShell commands; its other-tool path does not.
        AgentAction::RunCommand {
            command,
            shell: ActionShell::PowerShell,
        } => ("Bash", json!({"command":command})),
        AgentAction::ReadFile { path } => ("Read", json!({"file_path":path})),
        AgentAction::WebFetch { url } => ("WebFetch", json!({"url":url})),
        AgentAction::WebSearch { query } => ("WebSearch", json!({"query":query})),
        AgentAction::Other { tool, input } => (tool, input.clone()),
    }
}
fn normalized(base: &Path, path: &str) -> PathBuf {
    let path = PathBuf::from(path.replace('\\', "/"));
    let joined = if path.is_absolute() {
        path
    } else {
        base.join(path)
    };
    let mut result = PathBuf::new();
    for part in joined.components() {
        match part {
            Component::ParentDir => {
                result.pop();
            }
            Component::CurDir => {}
            other => result.push(other.as_os_str()),
        }
    }
    result
}
fn protected(root: &Path, tool: &str, input: &Value) -> bool {
    let kept = root.join("builder-session");
    if matches!(tool, "Read" | "Glob" | "Grep" | "WebFetch" | "WebSearch") {
        return false;
    }
    for key in [
        "file_path",
        "path",
        "target",
        "destination",
        "new_path",
        "output_path",
    ] {
        if let Some(path) = input.get(key).and_then(Value::as_str) {
            for base in [root.to_path_buf(), root.join("app")] {
                if ca_engine::workspace::may_reach(&kept, &normalized(&base, path)) {
                    return true;
                }
            }
        }
    }
    // Like the engine's own record guard, shell approvals trust scripts, but never allow a
    // command explicitly naming the host-owned copy (including split shell quotes).
    input
        .get("command")
        .and_then(Value::as_str)
        .is_some_and(|command| {
            let plain: String = command
                .chars()
                .filter(|c| !matches!(c, '\'' | '"' | '`'))
                .collect();
            plain
                .replace('\\', "/")
                .to_ascii_lowercase()
                .contains("builder-session")
        })
}
impl ActionPolicy for Policy {
    fn decide(&self, action: &AgentAction, cancel: &CancelFlag) -> PolicyVerdict {
        let refuse = |reason: &str| PolicyVerdict::Refuse {
            reason: reason.into(),
        };
        if cancel.is_cancelled() {
            return refuse("Stopped.");
        }
        let (tool, input) = to_claude_shape(action);
        if protected(&self.root, tool, &input) {
            return refuse("The builder-session folder is a host-owned backup. Do not change it.");
        }
        let result = approve_with(
            Some(Inbox::new(self.root.join("inbox"))),
            self.auto_mode,
            self.clean_room,
            tool,
            &input,
            &|| cancel.is_cancelled(),
            &mut || (self.waiting)(),
        );
        // approve_with can allow safe actions without inspecting cancellation.
        if cancel.is_cancelled() {
            return refuse("Stopped.");
        }
        match result {
            Ok(Ok(())) => PolicyVerdict::Allow,
            Ok(Err(reason)) => PolicyVerdict::Refuse { reason },
            Err(_) => refuse("The approval check failed. The action was not allowed."),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cancel_never_allows_even_safe_writes() {
        let policy = Policy::new(PathBuf::from("C:/project"), true, false, Arc::new(|| {}));
        let cancel = CancelFlag::new();
        cancel.cancel();
        assert!(matches!(
            policy.decide(
                &AgentAction::WriteFile {
                    path: "app/a.rs".into()
                },
                &cancel
            ),
            PolicyVerdict::Refuse { .. }
        ));
    }
    #[test]
    fn protects_backups_before_auto_approval() {
        let root = std::env::current_dir().unwrap();
        assert!(protected(
            &root,
            "Write",
            &json!({"file_path":"app/../builder-session/pi.jsonl"})
        ));
        assert!(protected(
            &root,
            "Bash",
            &json!({"command":"echo x > ../builder-'session'/pi.jsonl"})
        ));
        assert!(!protected(
            &root,
            "Read",
            &json!({"file_path":"builder-session/pi.jsonl"})
        ));
        let action = AgentAction::RunCommand {
            command: "npm run build".into(),
            shell: ActionShell::PowerShell,
        };
        let (tool, input) = to_claude_shape(&action);
        assert_eq!(tool, "Bash");
        assert_eq!(input["command"], "npm run build");
    }
    #[test]
    fn powershell_uses_real_command_categories_and_cleanroom() {
        use ca_engine::workspace::{ActionKind, TargetRef, Workspace};
        let temp = tempfile::tempdir().unwrap();
        let ws = Workspace::create(
            temp.path(),
            "policy",
            "test",
            TargetRef::Website {
                url: "https://original.example".into(),
            },
        )
        .unwrap();
        std::fs::create_dir_all(ws.app_dir()).unwrap();
        let policy = Policy::new(ws.root.clone(), true, true, Arc::new(|| {}));
        let cancel = CancelFlag::new();
        for shell in [ActionShell::Bash, ActionShell::PowerShell] {
            let action = AgentAction::RunCommand {
                command: "git push origin main".into(),
                shell,
            };
            let (tool, input) = to_claude_shape(&action);
            assert_eq!(
                ca_engine::tools::interaction_tools::kind_of(Some(&ws), tool, &input),
                ActionKind::Publish
            );
            assert!(matches!(
                policy.decide(&action, &cancel),
                PolicyVerdict::Refuse { .. }
            ));
            assert_eq!(
                policy.decide(
                    &AgentAction::RunCommand {
                        command: "npm run build".into(),
                        shell
                    },
                    &cancel
                ),
                PolicyVerdict::Allow
            );
        }
        assert!(matches!(
            policy.decide(
                &AgentAction::WebFetch {
                    url: "https://original.example/page".into()
                },
                &cancel
            ),
            PolicyVerdict::Refuse { .. }
        ));
    }
    #[test]
    fn cancellation_while_waiting_refuses_and_withdraws_card() {
        let temp = tempfile::tempdir().unwrap();
        let ws = ca_engine::workspace::Workspace::create(
            temp.path(),
            "cancel",
            "request",
            ca_engine::workspace::TargetRef::None,
        )
        .unwrap();
        let cancel = CancelFlag::new();
        let waiting_cancel = cancel.clone();
        let policy = Policy::new(
            ws.root.clone(),
            false,
            false,
            Arc::new(move || waiting_cancel.cancel()),
        );
        let verdict = policy.decide(
            &AgentAction::Other {
                tool: "unrecognized_writer".into(),
                input: json!({}),
            },
            &cancel,
        );
        assert!(matches!(verdict, PolicyVerdict::Refuse { .. }));
        assert!(Inbox::new(ws.root.join("inbox")).pending().is_empty());
    }
}
