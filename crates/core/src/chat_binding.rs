//! Machine-local native session and stable builder chat persistence boundaries.
use crate::{BackendKind, ChatId, NativeSessionKey, ProjectId, Result};
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatBinding {
    pub project: ProjectId,
    pub chat: ChatId,
    pub backend: BackendKind,
    pub native_session: NativeSessionKey,
}
pub trait ChatBindingService: Send + Sync {
    fn binding(&self, project: &ProjectId, chat: &ChatId) -> Result<Option<ChatBinding>>;
    fn bind(&self, binding: ChatBinding) -> Result<()>;
    fn remove_binding(&self, project: &ProjectId, chat: &ChatId) -> Result<()>;
    fn bindings(&self, project: &ProjectId) -> Result<Vec<ChatBinding>>;
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuilderChat {
    pub project: ProjectId,
    pub chat: ChatId,
    pub backend: BackendKind,
}
pub trait BuilderChatService: Send + Sync {
    /// Reads the stable builder identity without allocating a chat or changing its backend.
    fn find_builder_chat(&self, project: &ProjectId) -> Result<Option<BuilderChat>>;
    /// Returns the stable builder chat, atomically clearing its native binding on backend change.
    fn builder_chat(&self, project: &ProjectId, backend: BackendKind) -> Result<BuilderChat>;
}
