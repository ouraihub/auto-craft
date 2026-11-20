use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::Result;

/// Explicit shell choice; a missing shell is an error, never a silent fallback.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Shell {
    PowerShell,
    Pwsh,
    Cmd,
    Bash,
}

/// A persisted terminal tab grouped by canonical working directory. The PTY is not persisted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalTab {
    pub cwd: PathBuf,
    pub tab_key: String,
    pub title: String,
    pub runtime: Option<Uuid>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GridSize {
    pub cols: u16,
    pub rows: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachRequest {
    /// Canonical absolute path to an existing directory, supplied explicitly by the caller.
    pub cwd: PathBuf,
    pub tab_key: String,
    pub shell: Shell,
    pub size: GridSize,
}

/// Replay is captured under the same lock that assigns byte offsets, so a client applies live
/// chunks only when `byte_offset >= replay_end_offset` and never duplicates output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attached {
    pub runtime: Uuid,
    pub created: bool,
    pub replay: String,
    pub replay_truncated: bool,
    pub replay_end_offset: u64,
    pub size: GridSize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminalEvent {
    Data {
        runtime: Uuid,
        byte_offset: u64,
        data: String,
    },
    Exit {
        runtime: Uuid,
        exit_code: Option<u32>,
    },
}

/// Receives PTY output. May block briefly: a full queue is the backpressure on the reader.
pub trait TerminalSink: Send + Sync {
    fn publish(&self, event: TerminalEvent);
}

pub trait TerminalService: Send + Sync {
    /// Lists tabs for a canonical absolute directory, including one that has been removed.
    fn tabs(&self, cwd: &Path) -> Result<Vec<TerminalTab>>;
    /// Reserves a tab in an existing canonical absolute working directory.
    fn reserve(&self, cwd: &Path, tab_key: &str, title: &str) -> Result<TerminalTab>;
    /// Reuses a live PTY for the directory/tab pair or starts one in the explicit directory.
    /// Rejects relative, noncanonical, missing and nondirectory paths, even when reattaching.
    fn attach(&self, request: AttachRequest) -> Result<Attached>;
    fn write(&self, runtime: &Uuid, data: &str) -> Result<()>;
    fn resize(&self, runtime: &Uuid, size: GridSize) -> Result<()>;
    /// Ends the shell and forgets the tab, even if its working directory was removed.
    fn close(&self, cwd: &Path, tab_key: &str) -> Result<()>;
    /// Ends every shell; used on app exit.
    fn shutdown(&self);
}
