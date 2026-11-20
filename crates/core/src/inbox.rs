//! Project question and approval cards.
use crate::{InboxRequestId, ProjectId, Result};
use serde::{Deserialize, Serialize};
pub trait InboxService: Send + Sync {
    fn pending(&self, project: &ProjectId) -> Result<Vec<InboxRequest>>;
    fn answer(&self, project: &ProjectId, id: &InboxRequestId, answer: InboxAnswer) -> Result<()>;
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InboxRequest {
    pub id: InboxRequestId,
    pub kind: InboxKind,
    pub title: String,
    pub detail: String,
    pub choices: Vec<String>,
    pub allow_text: bool,
    pub remember: Option<String>,
    pub created_ms: u64,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum InboxKind {
    Question,
    Approval,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum InboxAnswer {
    Choice {
        choice: Option<String>,
        text: Option<String>,
    },
    Approval {
        allow: bool,
        always: bool,
        text: Option<String>,
    },
}
