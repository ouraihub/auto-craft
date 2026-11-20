//! Native selection is identity-guarded; choosing output never grants extraction consent.
use super::{GameFilesView, Operation, Request, data};
use autocraft_core::{AnalysisMode, Error};
use gpui_kit::{Context, PathPromptOptions};
use std::path::{Path, PathBuf};

impl GameFilesView {
    pub(super) fn pick_path(&mut self, directory: bool, cx: &mut Context<Self>) {
        if self.busy || self.mode != AnalysisMode::Deep {
            return;
        }
        self.invalidate();
        let generation = self.generation;
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: !directory,
            directories: directory,
            multiple: false,
            prompt: None,
        });
        cx.spawn(async move |this, cx| {
            let result = picked.await;
            if this
                .update(cx, |this, cx| {
                    if generation != this.generation {
                        tracing::debug!(
                            reason = "GENERATION_CHANGED",
                            "game_files.picker.discarded"
                        );
                        return;
                    }
                    match result {
                        Ok(Ok(Some(paths))) => {
                            if let Some(path) = paths.into_iter().next() {
                                this.discover(path, cx);
                            }
                        }
                        Ok(Ok(None)) => {}
                        Ok(Err(_)) | Err(_) => this.failure(
                            Error::invalid(rust_i18n::t!("app.game_files.picker_failed")),
                            cx,
                        ),
                    }
                })
                .is_err()
            {
                tracing::debug!("game_files.picker.view_closed");
            }
        })
        .detach();
    }
    pub(super) fn pick_output(
        &mut self,
        archive: data::Archive,
        item: data::Item,
        cx: &mut Context<Self>,
    ) {
        if self.busy || self.mode != AnalysisMode::Deep {
            return;
        }
        let Some(picture) = item.picture else {
            return;
        };
        self.invalidate();
        let Some(project) = self.project.clone() else {
            return;
        };
        let generation = self.generation;
        let suggested = data::suggested_name(&item.name);
        let directory = match self.state.read(cx).services.project_dir(&project) {
            Ok(directory) => directory,
            Err(error) => {
                self.failure(error, cx);
                return;
            }
        };
        let picked = cx.prompt_for_new_path(&directory, Some(&suggested));
        cx.spawn(async move |this, cx| {
            let result = picked.await;
            if this
                .update(cx, |this, cx| {
                    if generation != this.generation {
                        tracing::debug!(
                            reason = "GENERATION_CHANGED",
                            "game_files.output.discarded"
                        );
                        return;
                    }
                    match result {
                        Ok(Ok(Some(path))) => {
                            let path = match png_destination(path) {
                                Ok(path) => path,
                                Err(error) => {
                                    this.failure(error, cx);
                                    return;
                                }
                            };
                            let (tool, params) = picture.export(&archive, &path);
                            this.pending = Some(Request {
                                tool: tool.into(),
                                params,
                                operation: Operation::Export { output: path },
                            });
                            cx.notify();
                        }
                        Ok(Ok(None)) => {}
                        Ok(Err(_)) | Err(_) => this.failure(
                            Error::invalid(rust_i18n::t!("app.game_files.picker_failed")),
                            cx,
                        ),
                    }
                })
                .is_err()
            {
                tracing::debug!("game_files.output.view_closed");
            }
        })
        .detach();
    }
}

/// Validate the final extension, not the pre-normalized save-dialog selection.
fn png_destination(mut path: PathBuf) -> autocraft_core::Result<PathBuf> {
    path.set_extension("png");
    ensure_new_output(&path)?;
    Ok(path)
}

pub(super) fn ensure_new_output(path: &Path) -> autocraft_core::Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Err(Error::conflict(rust_i18n::t!(
            "app.game_files.output_exists"
        ))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(Error::Io {
            context: rust_i18n::t!("app.game_files.picker_failed").into_owned(),
            source,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalized_png_destination_never_overwrites_existing_output() {
        let root = tempfile::tempdir().unwrap();
        let png = root.path().join("texture.png");
        std::fs::write(&png, b"original picture").unwrap();
        let error = png_destination(root.path().join("texture.jpg")).unwrap_err();
        assert_eq!(error.kind(), Some(autocraft_core::ErrorKind::Conflict));
        assert_eq!(std::fs::read(&png).unwrap(), b"original picture");
        let selected = png_destination(root.path().join("another.jpg")).unwrap();
        assert_eq!(selected, root.path().join("another.png"));
        assert!(!selected.exists());
    }
}
