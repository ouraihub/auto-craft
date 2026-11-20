//! Authenticated analysis-channel scope and transport boundary.
use crate::{ChatId, ProjectId};
use serde::{Deserialize, Serialize};
use std::{
    fmt,
    io::{BufRead, Write},
    path::PathBuf,
};

/// Purpose fixes the permissions of a launch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChatPurpose {
    Analysis,
    Builder,
    Question,
}

/// Identity bound to a launch token; never contains that token.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatScope {
    pub project: ProjectId,
    pub chat: ChatId,
    pub purpose: ChatPurpose,
}
impl fmt::Debug for ChatScope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ChatScope")
            .field("chat", &self.chat)
            .field("purpose", &self.purpose)
            .finish_non_exhaustive()
    }
}

/// Immutable tool-surface options for a launch.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelOptions {
    pub all_tools: bool,
    pub async_long_tools: bool,
}

/// Per-launch process credentials. Tokens are deliberately omitted from Debug.
#[derive(Clone)]
pub struct AnalysisAccess {
    pub executable: PathBuf,
    pub pipe: String,
    pub token: String,
    pub options: ChannelOptions,
}
impl fmt::Debug for AnalysisAccess {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AnalysisAccess")
            .field("options", &self.options)
            .finish_non_exhaustive()
    }
}

/// Analysis-owned server; the bridge only authenticates and transports bytes.
pub trait ChannelHandler: Send + Sync {
    /// Serves a connection until EOF, cancellation, or transport failure.
    /// # Errors
    /// Returns the connection's I/O failure.
    fn serve(
        &self,
        scope: &ChatScope,
        options: ChannelOptions,
        reader: Box<dyn BufRead + Send>,
        writer: Box<dyn Write + Send>,
    ) -> std::io::Result<()>;
}
