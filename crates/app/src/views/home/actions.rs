//! Native selection and project creation; blocking I/O always runs off the UI thread.
use super::{HomeView, TargetKind};
use crate::state::run;
use autocraft_core::{
    Door, Error, ExistingFilesDto, NewProject, PersonChange, ProjectModelChoice, RunRequest,
    TargetRefDto,
};
use gpui_kit::{Context, PathPromptOptions};
use std::path::PathBuf;

impl HomeView {
    pub(super) fn choose_path(&mut self, save: bool, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.folder_generation += 1;
        let generation = self.folder_generation;
        let own = self.door == Door::WorkOnMine;
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: !save && !own,
            directories: true,
            multiple: false,
            prompt: None,
        });
        cx.spawn(async move |this, cx| {
            match picked.await {
                Ok(Ok(Some(paths))) => {
                    if let Some(path) = paths.into_iter().next() {
                        let result = this.update(cx, |this, cx| {
                            if generation != this.folder_generation {
                                tracing::debug!("home.picker.stale");
                                return;
                            }
                            if save {
                                this.inspect_folder(path, generation, cx);
                            } else {
                                // Input changes need a Window; defer the native selection through the next render.
                                this.picked_target = Some(path.clone());
                                if own {
                                    this.folder = Some(path);
                                    this.existing = Some(ExistingFilesDto::Edit);
                                }
                                cx.notify();
                            }
                        });
                        if result.is_err() {
                            tracing::debug!("home.picker.view_closed");
                        }
                    }
                }
                Ok(Ok(None)) => {}
                Ok(Err(_)) | Err(_) => {
                    if this
                        .update(cx, |this, cx| {
                            this.report(
                                &Error::invalid(rust_i18n::t!("app.home.picker_failed")),
                                cx,
                            )
                        })
                        .is_err()
                    {
                        tracing::debug!("home.picker.view_closed");
                    }
                }
            }
        })
        .detach();
    }

    fn inspect_folder(&mut self, path: PathBuf, generation: u64, cx: &mut Context<Self>) {
        self.folder_loading = true;
        cx.notify();
        run(
            cx,
            move || {
                let mut entries = std::fs::read_dir(&path)
                    .map_err(|_| Error::invalid(rust_i18n::t!("app.home.folder_failed")))?;
                let has_files = entries
                    .next()
                    .transpose()
                    .map_err(|_| Error::invalid(rust_i18n::t!("app.home.folder_failed")))?
                    .is_some();
                Ok((path, has_files))
            },
            move |this, result, cx| {
                if generation != this.folder_generation {
                    tracing::debug!("home.folder.stale");
                    return;
                }
                this.folder_loading = false;
                match result {
                    Ok((path, has_files)) => {
                        this.folder = Some(path);
                        this.folder_has_files = has_files;
                        this.existing = None;
                    }
                    Err(error) => this.report(&error, cx),
                }
                cx.notify();
            },
        );
    }

    pub(super) fn create(&mut self, cx: &mut Context<Self>) {
        if self.busy || self.loading || self.folder_loading {
            return;
        }
        let rebuilding = self.door != Door::LookInside;
        let services = self.state.read(cx).services.clone();
        if rebuilding && services.rebuild.is_none() {
            return;
        }
        let name = self.name.read(cx).value().trim().to_owned();
        let target_text = self.target.read(cx).value().trim().to_owned();
        let mut request = if rebuilding {
            self.request.read(cx).value().trim().to_owned()
        } else {
            String::new()
        };
        let target = if self.door == Door::WorkOnMine {
            TargetRefDto::None
        } else {
            match self.target_kind {
                TargetKind::Website => TargetRefDto::Website {
                    url: target_text.clone(),
                },
                TargetKind::Application | TargetKind::Game => TargetRefDto::Local {
                    path: PathBuf::from(&target_text),
                },
                TargetKind::None => TargetRefDto::None,
            }
        };
        let invalid = if name.is_empty() {
            Some("app.home.name_required")
        } else if rebuilding && request.is_empty() {
            Some("app.home.request_required")
        } else if self.door == Door::WorkOnMine && target_text.is_empty() {
            Some("app.home.folder_required")
        } else if (self.door != Door::WorkOnMine
            && self.target_kind != TargetKind::None
            && target_text.is_empty())
            || (self.door == Door::LookInside && self.target_kind == TargetKind::None)
        {
            Some("app.home.target_required")
        } else if self.door != Door::WorkOnMine && self.folder_has_files && self.existing.is_none()
        {
            Some("app.home.choose_existing")
        } else {
            None
        };
        if let Some(key) = invalid {
            self.error = Some(rust_i18n::t!(key).to_string());
            cx.notify();
            return;
        }
        let game =
            rebuilding && self.target_kind == TargetKind::Game && self.door != Door::WorkOnMine;
        let budget = if game && self.generate_assets {
            match self.budget.read(cx).value().trim().parse::<f64>() {
                Ok(value) if value.is_finite() && value >= 0.0 => value,
                Ok(_) | Err(_) => {
                    self.error = Some(rust_i18n::t!("app.home.invalid_budget").to_string());
                    cx.notify();
                    return;
                }
            }
        } else {
            0.0
        };
        if game
            && rebuilding
            && let Some(stack) = self.game_stack
        {
            request.push_str(&format!("\n\n{stack}"));
        }
        let folder = if self.door == Door::WorkOnMine {
            Some(PathBuf::from(target_text))
        } else {
            self.folder.clone()
        };
        let input = NewProject {
            name,
            request,
            target,
            door: self.door,
            folder,
            existing: if self.door == Door::WorkOnMine {
                Some(ExistingFilesDto::Edit)
            } else {
                self.existing
            },
            mode: self.mode,
            builder: rebuilding.then(|| ProjectModelChoice {
                backend: self.backend,
                model: self.model.clone(),
                effort: self.effort,
            }),
        };
        let assets = game && self.generate_assets;
        let references = if rebuilding {
            self.references.clone()
        } else {
            Vec::new()
        };
        self.busy = true;
        self.error = None;
        self.sync_picker(cx);
        run(
            cx,
            move || {
                let project = services.projects.create(input)?;
                super::references::store(services.projects.as_ref(), &project.id, &references)?;
                if game {
                    services
                        .projects
                        .set_person(&project.id, PersonChange::GenerateAssets(assets))?;
                    services
                        .projects
                        .set_person(&project.id, PersonChange::AssetBudget(budget))?;
                }
                if rebuilding && let Some(rebuild) = services.rebuild {
                    rebuild.start(
                        &project.id,
                        RunRequest {
                            message: None,
                            shown: None,
                            pictures: Vec::new(),
                            continuing: false,
                            max_rounds: None,
                            backend: None,
                        },
                    )?;
                }
                Ok(project.id)
            },
            move |this, result, cx| {
                this.busy = false;
                this.sync_picker(cx);
                this.state.update(cx, |state, cx| state.reload(cx));
                match result {
                    Ok(id) => {
                        this.references.clear();
                        this.state.update(cx, |state, cx| {
                            if state.main != crate::state::MainView::Home {
                                tracing::debug!("home.create.navigation_stale");
                                return;
                            }
                            if rebuilding {
                                state.select_project(id, cx);
                            } else {
                                state.select_ask(id, cx);
                            }
                        });
                    }
                    Err(error) => this.report(&error, cx),
                }
                cx.notify();
            },
        );
    }
}
