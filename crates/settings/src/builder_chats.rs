//! Stable project builder chat identities and atomic backend cutovers.

use autocraft_core::{BackendKind, BuilderChat, BuilderChatService, ChatId, ProjectId, Result};

use crate::{db, store::SqliteSettingsStore};

impl BuilderChatService for SqliteSettingsStore {
    fn find_builder_chat(&self, project: &ProjectId) -> Result<Option<BuilderChat>> {
        self.read(|conn| {
            conn.row(
                "SELECT chat_id, backend FROM builder_chats WHERE project_id = ?1",
                (project.as_str(),),
            )?
            .map(|row| {
                Ok(BuilderChat {
                    project: project.clone(),
                    chat: db::text(&row, 0)?.into(),
                    backend: db::word(&row, 1)?,
                })
            })
            .transpose()
        })
    }

    fn builder_chat(&self, project: &ProjectId, backend: BackendKind) -> Result<BuilderChat> {
        self.write(|conn| {
            let chat = if let Some(row) = conn.row(
                "SELECT chat_id, backend FROM builder_chats WHERE project_id = ?1",
                (project.as_str(),),
            )? {
                let chat = ChatId::from(db::text(&row, 0)?);
                if db::word::<BackendKind>(&row, 1)? != backend {
                    conn.execute(
                        "DELETE FROM chat_bindings WHERE chat_id = ?1",
                        (chat.as_str(),),
                    )?;
                    conn.execute(
                        "UPDATE builder_chats SET backend = ?2 WHERE project_id = ?1",
                        (project.as_str(), db::to_word(&backend)?),
                    )?;
                }
                chat
            } else {
                let chat = ChatId::generate();
                conn.execute(
                    "INSERT INTO builder_chats (project_id, chat_id, backend) VALUES (?1, ?2, ?3)",
                    (project.as_str(), chat.as_str(), db::to_word(&backend)?),
                )?;
                chat
            };
            Ok(BuilderChat {
                project: project.clone(),
                chat,
                backend,
            })
        })
    }
}
