//! Machine-local project choices and optimistic revision writes in the shared transaction.

use autocraft_core::{
    AnalysisMode, Error, ModelRef, ProjectId, ProjectPrefChange, ProjectPrefService, ProjectPrefs,
    Result,
};

use crate::{
    db::{self, Conn},
    store::SqliteSettingsStore,
};

fn defaults() -> ProjectPrefs {
    ProjectPrefs {
        backend: None,
        model: None,
        effort: None,
        mode: AnalysisMode::CleanRoom,
        view: None,
        revision: 0,
    }
}

fn load(conn: &Conn, project: &ProjectId) -> Result<ProjectPrefs> {
    let Some(row) = conn.row(
        "SELECT backend, model_provider, model_id, effort, mode, view, revision FROM project_prefs WHERE project_id = ?1",
        (project.as_str(),),
    )? else {
        return Ok(defaults());
    };
    let model = match (db::optional_text(&row, 1)?, db::optional_text(&row, 2)?) {
        (Some(provider), Some(model_id)) => Some(ModelRef {
            provider: provider.into(),
            model_id,
        }),
        (None, None) => None,
        (Some(_), None) | (None, Some(_)) => return Err(Error::corrupt("项目模型设置已损坏。")),
    };
    Ok(ProjectPrefs {
        backend: db::optional_text(&row, 0)?
            .map(|_| db::word(&row, 0))
            .transpose()?,
        model,
        effort: db::optional_text(&row, 3)?
            .map(|_| db::word(&row, 3))
            .transpose()?,
        mode: db::word(&row, 4)?,
        view: db::optional_text(&row, 5)?
            .map(|_| db::word(&row, 5))
            .transpose()?,
        revision: db::revision(&row, 6)?,
    })
}

fn save(conn: &Conn, project: &ProjectId, prefs: &ProjectPrefs) -> Result<()> {
    conn.execute(
        "INSERT INTO project_prefs (project_id, backend, model_provider, model_id, effort, mode, view, revision) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8) ON CONFLICT (project_id) DO UPDATE SET \
         backend = excluded.backend, model_provider = excluded.model_provider, model_id = excluded.model_id, \
         effort = excluded.effort, mode = excluded.mode, view = excluded.view, revision = excluded.revision",
        (
            project.as_str(),
            prefs.backend.as_ref().map(db::to_word).transpose()?,
            prefs.model.as_ref().map(|model| model.provider.as_str()),
            prefs.model.as_ref().map(|model| model.model_id.as_str()),
            prefs.effort.as_ref().map(db::to_word).transpose()?,
            db::to_word(&prefs.mode)?,
            prefs.view.as_ref().map(db::to_word).transpose()?,
            i64::try_from(prefs.revision).map_err(|_| Error::conflict("项目设置版本已超出范围。"))?,
        ),
    )?;
    Ok(())
}

impl ProjectPrefService for SqliteSettingsStore {
    fn get(&self, project: &ProjectId) -> Result<ProjectPrefs> {
        self.read(|conn| load(conn, project))
    }

    fn set(
        &self,
        project: &ProjectId,
        change: ProjectPrefChange,
        expect_revision: u64,
    ) -> Result<ProjectPrefs> {
        self.write(|conn| {
            let mut prefs = load(conn, project)?;
            if prefs.revision != expect_revision {
                return Err(Error::conflict("项目设置已被修改，请重新读取后再保存。"));
            }
            match change {
                ProjectPrefChange::Backend(backend) => prefs.backend = backend,
                ProjectPrefChange::Model(model) => {
                    if let Some(model) = &model
                        && conn.row(
                            "SELECT 1 FROM provider_models WHERE provider_id = ?1 AND model_id = ?2",
                            (model.provider.as_str(), model.model_id.as_str()),
                        )?.is_none()
                    {
                        return Err(Error::invalid("项目选择的模型不存在。"));
                    }
                    prefs.model = model;
                }
                ProjectPrefChange::Effort(effort) => prefs.effort = effort,
                ProjectPrefChange::Mode(mode) => prefs.mode = mode,
                ProjectPrefChange::View(view) => prefs.view = view,
            }
            prefs.revision = prefs.revision.checked_add(1)
                .ok_or_else(|| Error::conflict("项目设置版本已超出范围。"))?;
            save(conn, project, &prefs)?;
            Ok(prefs)
        })
    }

    fn remove(&self, project: &ProjectId) -> Result<()> {
        self.write(|conn| {
            conn.execute(
                "DELETE FROM project_prefs WHERE project_id = ?1",
                (project.as_str(),),
            )?;
            conn.execute(
                "DELETE FROM chat_bindings WHERE project_id = ?1",
                (project.as_str(),),
            )?;
            conn.execute(
                "DELETE FROM builder_chats WHERE project_id = ?1",
                (project.as_str(),),
            )?;
            Ok(())
        })
    }

    fn copy(&self, from: &ProjectId, to: &ProjectId) -> Result<()> {
        self.write(|conn| {
            if from == to || load(conn, to)?.revision != 0 {
                return Err(Error::conflict("目标项目已有设置。"));
            }
            let mut prefs = load(conn, from)?;
            prefs.revision = 1;
            save(conn, to, &prefs)
        })
    }
}
