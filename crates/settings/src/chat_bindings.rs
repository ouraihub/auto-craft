//! Project-scoped native-session bindings; a chat never resumes a different project's history.

use autocraft_core::{ChatBinding, ChatBindingService, ChatId, Error, ProjectId, Result};
use turso::Row;

use crate::{
    db::{self, Conn},
    store::SqliteSettingsStore,
};

fn decode(row: &Row) -> Result<ChatBinding> {
    Ok(ChatBinding {
        project: db::text(row, 0)?.into(),
        chat: db::text(row, 1)?.into(),
        backend: db::word(row, 2)?,
        native_session: db::text(row, 3)?.into(),
    })
}

pub(crate) fn load(conn: &Conn, chat: &ChatId) -> Result<Option<ChatBinding>> {
    conn.row(
        "SELECT project_id, chat_id, backend, native_session FROM chat_bindings WHERE chat_id = ?1",
        (chat.as_str(),),
    )?
    .as_ref()
    .map(decode)
    .transpose()
}

impl ChatBindingService for SqliteSettingsStore {
    fn binding(&self, project: &ProjectId, chat: &ChatId) -> Result<Option<ChatBinding>> {
        self.read(|conn| {
            let binding = load(conn, chat)?;
            if binding
                .as_ref()
                .is_some_and(|binding| &binding.project != project)
            {
                return Err(Error::invalid("聊天不属于当前项目。"));
            }
            Ok(binding)
        })
    }

    fn bind(&self, binding: ChatBinding) -> Result<()> {
        if binding.native_session.as_str().trim().is_empty() {
            return Err(Error::invalid("原生会话标识不能为空。"));
        }
        self.write(|conn| {
            if let Some(existing) = load(conn, &binding.chat)?
                && (existing.project != binding.project || existing.backend != binding.backend)
            {
                return Err(Error::conflict("聊天已绑定到其他项目或 Agent。"));
            }
            if let Some(row) = conn.row(
                "SELECT project_id, backend FROM builder_chats WHERE chat_id = ?1",
                (binding.chat.as_str(),),
            )? && (db::text(&row, 0)? != binding.project.as_str()
                || db::word::<autocraft_core::BackendKind>(&row, 1)? != binding.backend)
            {
                return Err(Error::conflict("构建方聊天的项目或 Agent 已改变。"));
            }
            conn.execute(
                "INSERT INTO chat_bindings (project_id, chat_id, backend, native_session) VALUES (?1, ?2, ?3, ?4) \
                 ON CONFLICT (chat_id) DO UPDATE SET native_session = excluded.native_session",
                (binding.project.as_str(), binding.chat.as_str(), db::to_word(&binding.backend)?, binding.native_session.as_str()),
            )?;
            Ok(())
        })
    }

    fn remove_binding(&self, project: &ProjectId, chat: &ChatId) -> Result<()> {
        self.write(|conn| {
            conn.execute(
                "DELETE FROM chat_bindings WHERE project_id = ?1 AND chat_id = ?2",
                (project.as_str(), chat.as_str()),
            )?;
            Ok(())
        })
    }

    fn bindings(&self, project: &ProjectId) -> Result<Vec<ChatBinding>> {
        self.read(|conn| {
            conn.rows(
                "SELECT project_id, chat_id, backend, native_session FROM chat_bindings WHERE project_id = ?1 ORDER BY chat_id",
                (project.as_str(),),
            )?.iter().map(decode).collect()
        })
    }
}
