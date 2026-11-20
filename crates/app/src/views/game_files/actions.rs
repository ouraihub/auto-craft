//! Generation-guarded service calls, mode and explicit per-operation consent.
use super::{GameFilesView, Operation, Outcome, Request, data};
use crate::state::run;
use autocraft_core::{
    AnalysisMode, Availability, CallerDto, CancelFlag, Error, ErrorKind, ToolCall,
};
use gpui_kit::Context;
use serde_json::json;
use std::path::PathBuf;

impl GameFilesView {
    pub(super) fn discover(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if self.busy || self.mode != AnalysisMode::Deep {
            return;
        }
        self.invalidate();
        self.root = Some(path.clone());
        self.archives.clear();
        self.items.clear();
        self.archive = None;
        self.saved = None;
        self.next = None;
        self.submit(
            Request {
                tool: "game_archives_list".into(),
                params: json!({"path":path}),
                operation: Operation::Discover { preferred: None },
            },
            false,
            cx,
        );
    }
    pub(super) fn list(&mut self, archive: data::Archive, offset: usize, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let filter = if offset == 0 {
            self.filter.read(cx).value().to_string()
        } else {
            self.applied_filter.clone()
        };
        let (tool, params) = match archive.listing(offset, &filter) {
            Ok(request) => request,
            Err(error) => {
                self.failure(error, cx);
                return;
            }
        };
        self.invalidate();
        if offset == 0 {
            self.items.clear();
        }
        self.applied_filter = filter;
        self.next = None;
        self.saved = None;
        self.archive = Some(archive.clone());
        self.submit(
            Request {
                tool: tool.into(),
                params,
                operation: Operation::List { archive, offset },
            },
            false,
            cx,
        );
    }
    pub(super) fn submit(&mut self, request: Request, consented: bool, cx: &mut Context<Self>) {
        if self.busy || self.mode != AnalysisMode::Deep {
            return;
        }
        let Some(project) = self.project.clone() else {
            return;
        };
        self.cancel = CancelFlag::new();
        let cancel = self.cancel.clone();
        let service = self.state.read(cx).services.analysis.clone();
        let generation = self.generation;
        let pending = request.clone();
        let event_project = project.clone();
        self.busy = true;
        self.error = None;
        run(
            cx,
            move || {
                // Recheck authoritative mode and availability, not only the rendered snapshot.
                if service.view(&project)?.mode != AnalysisMode::Deep {
                    return Err(Error::new(
                        ErrorKind::DeepRequired,
                        rust_i18n::t!("app.game_files.deep_required"),
                    ));
                }
                let tool = service
                    .tools(&project)?
                    .into_iter()
                    .find(|t| t.id == request.tool)
                    .ok_or_else(|| Error::not_found(rust_i18n::t!("app.game_files.unsupported")))?;
                match tool.available {
                    Availability::Ready => {}
                    Availability::NeedsDeep => {
                        return Err(Error::new(
                            ErrorKind::DeepRequired,
                            rust_i18n::t!("app.game_files.deep_required"),
                        ));
                    }
                    Availability::NeedsTarget | Availability::Missing { .. } | Availability::Os => {
                        return Err(Error::new(
                            ErrorKind::BackendUnavailable,
                            rust_i18n::t!("app.game_files.unavailable"),
                        ));
                    }
                }
                if tool.needs_consent && !consented {
                    return Ok(None);
                }
                let mut request = request;
                if let Operation::Discover { preferred } = &mut request.operation {
                    let path: PathBuf = serde_json::from_value(request.params["path"].clone())
                        .map_err(|_| {
                            Error::invalid(rust_i18n::t!("app.game_files.invalid_result"))
                        })?;
                    let metadata = std::fs::metadata(&path).map_err(|source| Error::Io {
                        context: rust_i18n::t!("app.game_files.failed").into_owned(),
                        source,
                    })?;
                    if metadata.is_file() {
                        *preferred = Some(path.clone());
                        let parent = path.parent().ok_or_else(|| {
                            Error::invalid(rust_i18n::t!("app.game_files.failed"))
                        })?;
                        request.params["path"] = json!(parent);
                    }
                }
                if let Operation::Export { output } = &request.operation {
                    super::pickers::ensure_new_output(output)?;
                }
                let reply = service.call(
                    &project,
                    ToolCall {
                        tool: request.tool,
                        params: request.params,
                        caller: CallerDto::Ui,
                        consented,
                    },
                    &cancel,
                )?;
                let value = data::result(reply);
                let outcome = match &request.operation {
                    Operation::Discover { preferred } => {
                        let (mut archives, truncated) = data::archives(value)?;
                        if let Some(path) = preferred {
                            archives.retain(|a| std::path::Path::new(&a.path) == path);
                        }
                        Outcome::Discover(archives, truncated)
                    }
                    Operation::List { archive, offset } => {
                        Outcome::List(data::page(archive, value, *offset)?)
                    }
                    Operation::Export { output } => Outcome::Export(data::saved(value, output)?),
                };
                Ok(Some(outcome))
            },
            move |this, result, cx| {
                if generation != this.generation {
                    tracing::debug!(reason = "GENERATION_CHANGED", "game_files.result.discarded");
                    return;
                }
                this.busy = false;
                match result {
                    Ok(None) => this.pending = Some(pending),
                    Ok(Some(Outcome::Discover(archives, truncated))) => {
                        this.archives = archives;
                        this.truncated = truncated;
                        if this.archives.len() == 1 {
                            this.list(this.archives[0].clone(), 0, cx);
                        }
                    }
                    Ok(Some(Outcome::List(page))) => {
                        this.items.extend(page.items);
                        this.next = page.next;
                    }
                    Ok(Some(Outcome::Export(saved))) => this.saved = Some(saved),
                    Err(error) => this.failure(error, cx),
                }
                this.state
                    .update(cx, |state, cx| state.refresh_analysis(event_project, cx));
                cx.notify();
            },
        );
        cx.notify();
    }
    pub(super) fn set_deep(&mut self, cx: &mut Context<Self>) {
        let Some(project) = self.project.clone() else {
            return;
        };
        let service = self.state.read(cx).services.analysis.clone();
        let generation = self.generation;
        self.confirm_deep = false;
        self.busy = true;
        run(
            cx,
            move || service.set_mode(&project, AnalysisMode::Deep),
            move |this, result, cx| {
                if generation != this.generation {
                    tracing::debug!(reason = "GENERATION_CHANGED", "game_files.mode.discarded");
                    return;
                }
                this.busy = false;
                match result {
                    Ok(()) => {
                        if let Some(project) = this.project.clone() {
                            this.state
                                .update(cx, |state, cx| state.refresh_analysis(project, cx));
                        }
                    }
                    Err(error) => this.failure(error, cx),
                }
                cx.notify();
            },
        );
    }
}
