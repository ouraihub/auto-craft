//! Composer commands and attachment storage through typed rebuild/project boundaries.
use super::ProjectView;
use crate::state::run;
use autocraft_core::{ActionKindDto, ApprovalDto, CancelFlag, Error, PersonChange, RunRequest};
use gpui_kit::{ClipboardEntry, ClipboardItem, Context, PathPromptOptions};

enum Action {
    Send(String, Vec<std::path::PathBuf>),
    Ask(String),
    Stop,
    Continue,
    Done,
    Approval(ActionKindDto, Option<ApprovalDto>),
}
impl ProjectView {
    pub(super) fn command(&mut self, text: &str, cx: &mut Context<Self>) {
        if !self.writable() || (self.busy && text.trim() != "/stop") {
            return;
        }
        let Some(project) = self.project.clone() else {
            return;
        };
        let Some(rebuild) = self.state.read(cx).services.rebuild.clone() else {
            return;
        };
        let action = match parse(text, self.just_ask, self.pictures.clone()) {
            Ok(action) => action,
            Err(error) => {
                self.report(&error, cx);
                return;
            }
        };
        if text.trim() == "/stop" {
            self.request_cancel.cancel();
        } else {
            self.request_cancel = CancelFlag::new();
        }
        let cancel = self.request_cancel.clone();
        self.busy = true;
        let generation = self.generation;
        let running = self.running();
        let projects = self.state.read(cx).services.projects.clone();
        run(
            cx,
            move || match action {
                Action::Send(text, pictures) => {
                    if running {
                        rebuild.note(&project, &text, pictures)?;
                    } else {
                        rebuild.start(
                            &project,
                            RunRequest {
                                message: Some(text),
                                shown: None,
                                pictures,
                                continuing: false,
                                max_rounds: None,
                                backend: None,
                            },
                        )?;
                    }
                    Ok(None)
                }
                Action::Ask(question) => rebuild.ask(&project, &question, &cancel).map(Some),
                Action::Stop => rebuild.stop(&project).map(|()| None),
                Action::Continue => rebuild
                    .start(
                        &project,
                        RunRequest {
                            message: None,
                            shown: None,
                            pictures: Vec::new(),
                            continuing: true,
                            max_rounds: None,
                            backend: None,
                        },
                    )
                    .map(|_| None),
                Action::Done => rebuild.finish(&project).map(|()| None),
                Action::Approval(kind, approval) => projects
                    .set_person(&project, PersonChange::Approval { kind, approval })
                    .map(|_| None),
            },
            move |this, result, cx| {
                if generation != this.generation {
                    return;
                }
                this.busy = false;
                match result {
                    Ok(answer) => {
                        this.clear_input = true;
                        this.pictures.clear();
                        if let Some(answer) = answer
                            && let Some(notice) = answer.builder_notice
                            && let Some(status) = &mut this.status
                        {
                            status.note = notice;
                        }
                        this.reload(cx);
                    }
                    Err(error) => this.report(&error, cx),
                }
                cx.notify();
            },
        );
    }

    pub(super) fn choose_pictures(&mut self, cx: &mut Context<Self>) {
        if self.attaching || !self.writable() {
            return;
        }
        let generation = self.generation;
        let picker = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: None,
        });
        cx.spawn(async move |this, cx| match picker.await {
            Ok(Ok(Some(paths))) => {
                if this
                    .update(cx, |this, cx| {
                        if this.generation == generation {
                            this.attach(paths, cx);
                        }
                    })
                    .is_err()
                {
                    tracing::debug!("project.attach.view_closed");
                }
            }
            Ok(Ok(None)) => {}
            Ok(Err(_)) | Err(_) => {
                if this
                    .update(cx, |this, cx| {
                        this.report(
                            &Error::invalid(rust_i18n::t!("app.project.picker_failed")),
                            cx,
                        )
                    })
                    .is_err()
                {
                    tracing::debug!("project.attach.view_closed");
                }
            }
        })
        .detach();
    }

    fn attach(&mut self, paths: Vec<std::path::PathBuf>, cx: &mut Context<Self>) {
        let Some(project) = self.project.clone() else {
            return;
        };
        let Some(rebuild) = self.state.read(cx).services.rebuild.clone() else {
            return;
        };
        self.attaching = true;
        let generation = self.generation;
        run(
            cx,
            move || rebuild.attach_pictures(&project, paths),
            move |this, result, cx| {
                if generation != this.generation {
                    return;
                }
                this.attaching = false;
                match result {
                    Ok(paths) => this.pictures.extend(paths),
                    Err(error) => this.report(&error, cx),
                }
                cx.notify();
            },
        );
    }

    pub(super) fn paste(&mut self, item: &ClipboardItem, cx: &mut Context<Self>) -> bool {
        let images = item
            .entries
            .iter()
            .filter_map(|entry| match entry {
                ClipboardEntry::Image(image) => {
                    Some((image.bytes().to_vec(), image.format().mime_type()))
                }
                ClipboardEntry::String(_) | ClipboardEntry::ExternalPaths(_) => None,
            })
            .collect::<Vec<_>>();
        if images.is_empty() {
            return false;
        }
        if self.attaching || !self.writable() {
            return true;
        }
        let Some(project) = self.project.clone() else {
            return true;
        };
        let Some(rebuild) = self.state.read(cx).services.rebuild.clone() else {
            return true;
        };
        self.attaching = true;
        let generation = self.generation;
        run(
            cx,
            move || {
                images
                    .into_iter()
                    .map(|(bytes, mime)| rebuild.attach_image(&project, bytes, mime))
                    .collect::<autocraft_core::Result<Vec<_>>>()
            },
            move |this, result, cx| {
                if generation != this.generation {
                    return;
                }
                this.attaching = false;
                match result {
                    Ok(paths) => this.pictures.extend(paths),
                    Err(error) => this.report(&error, cx),
                }
                cx.notify();
            },
        );
        true
    }

    pub(super) fn withdraw(&mut self, id: autocraft_core::NoteId, cx: &mut Context<Self>) {
        let Some(project) = self.project.clone() else {
            return;
        };
        let Some(rebuild) = self.state.read(cx).services.rebuild.clone() else {
            return;
        };
        let generation = self.generation;
        run(
            cx,
            move || rebuild.withdraw(&project, &id),
            move |this, result, cx| {
                if this.generation != generation {
                    return;
                }
                match result {
                    Ok(text) => this.input_restore = text,
                    Err(error) => this.report(&error, cx),
                }
                this.reload(cx);
            },
        );
    }
}
fn parse(
    text: &str,
    ask: bool,
    pictures: Vec<std::path::PathBuf>,
) -> autocraft_core::Result<Action> {
    let text = text.trim();
    if text.is_empty() {
        return Err(Error::invalid(rust_i18n::t!(
            "app.project.message_required"
        )));
    }
    if !text.starts_with('/') {
        return Ok(if ask {
            Action::Ask(text.into())
        } else {
            Action::Send(text.into(), pictures)
        });
    }
    let mut words = text.split_whitespace();
    let action = match words.next() {
        Some("/stop") if words.next().is_none() => Action::Stop,
        Some("/continue") if words.next().is_none() => Action::Continue,
        Some("/done") if words.next().is_none() => Action::Done,
        Some("/btw") if !text[4..].trim().is_empty() => Action::Ask(text[4..].trim().into()),
        Some("/approvals") => {
            let kind = match words.next() {
                Some("web") => ActionKindDto::Web,
                Some("commands") => ActionKindDto::Commands,
                Some("outside_files") => ActionKindDto::OutsideFiles,
                Some("publish") => ActionKindDto::Publish,
                Some("delete_outside") => ActionKindDto::DeleteOutside,
                Some("system") => ActionKindDto::System,
                Some("other_tools") => ActionKindDto::OtherTools,
                _ => return Err(command_error()),
            };
            let approval = match words.next() {
                Some("allow") => Some(ApprovalDto::Allow),
                Some("ask") => Some(ApprovalDto::Ask),
                Some("refuse") => Some(ApprovalDto::Refuse),
                Some("default") => None,
                _ => return Err(command_error()),
            };
            if words.next().is_some() {
                return Err(command_error());
            }
            Action::Approval(kind, approval)
        }
        _ => return Err(command_error()),
    };
    Ok(action)
}
fn command_error() -> Error {
    Error::invalid(rust_i18n::t!("app.project.command_help"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn commands_require_explicit_valid_policy() {
        assert!(matches!(
            parse("/approvals web default", false, Vec::new()),
            Ok(Action::Approval(ActionKindDto::Web, None))
        ));
        assert!(parse("/approvals web unknown", false, Vec::new()).is_err());
        assert!(parse("/stop extra", false, Vec::new()).is_err());
        assert!(
            matches!(parse("/btw why now?", false, Vec::new()), Ok(Action::Ask(question)) if question == "why now?")
        );
    }
}
