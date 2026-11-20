//! Builder action approval and cooperative cancellation.
use serde::{Deserialize, Serialize};
use std::{
    fmt,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

/// Shared cancellation state; cancelling any clone cancels all clones.
#[derive(Debug, Clone, Default)]
pub struct CancelFlag(Arc<AtomicBool>);
impl CancelFlag {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}
/// Normalized engine action. Contains private user data; Debug never prints arguments.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub enum AgentAction {
    WriteFile {
        path: String,
    },
    RunCommand {
        command: String,
        shell: ActionShell,
    },
    ReadFile {
        path: String,
    },
    WebFetch {
        url: String,
    },
    WebSearch {
        query: String,
    },
    Other {
        tool: String,
        input: serde_json::Value,
    },
}
impl fmt::Debug for AgentAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::WriteFile { .. } => "WriteFile",
            Self::RunCommand { .. } => "RunCommand",
            Self::ReadFile { .. } => "ReadFile",
            Self::WebFetch { .. } => "WebFetch",
            Self::WebSearch { .. } => "WebSearch",
            Self::Other { .. } => "Other",
        })
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActionShell {
    Bash,
    PowerShell,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyVerdict {
    Allow,
    Refuse { reason: String },
}
/// Blocks for a policy/user decision; cancellation must return Refuse.
pub trait ActionPolicy: Send + Sync {
    fn decide(&self, action: &AgentAction, cancel: &CancelFlag) -> PolicyVerdict;
}
#[derive(Clone)]
pub struct BuilderHooks {
    pub policy: Arc<dyn ActionPolicy>,
}
impl fmt::Debug for BuilderHooks {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BuilderHooks").finish_non_exhaustive()
    }
}
