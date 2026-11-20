//! Generation-guarded service work and consent ownership for the workbench.
use super::{Fill, ToolTask, WorkbenchView, data};
use crate::state::run;
use autocraft_core::{
    AnalysisMode, Availability, CallerDto, CancelFlag, Error, ErrorKind, ToolCall, ToolInfo,
};
use gpui_kit::Context;
use serde_json::Value;
impl WorkbenchView {
    pub(super) fn target_kind<'a>(&'a self, cx: &'a gpui_kit::App) -> &'a str {
        self.state
            .read(cx)
            .analysis_view
            .as_ref()
            .and_then(|v| v.targets.iter().find(|t| Some(&t.id) == v.active.as_ref()))
            .map_or("", |t| t.kind.as_str())
    }
    pub(super) fn target_path(&self, cx: &gpui_kit::App) -> String {
        self.state
            .read(cx)
            .analysis_view
            .as_ref()
            .and_then(|v| v.targets.iter().find(|t| Some(&t.id) == v.active.as_ref()))
            .map(|t| t.path.display().to_string())
            .unwrap_or_default()
    }
    pub(super) fn reload(&mut self, cx: &mut Context<Self>) {
        if self.project == self.state.read(cx).project {
            self.refresh_snapshot(cx);
            return;
        }
        self.generation += 1;
        self.editor_generation += 1;
        self.list_generation += 1;
        for task in &self.tasks {
            task.cancel.cancel();
        }
        self.tasks.clear();
        self.pending = None;
        self.confirm_deep = false;
        self.selection = None;
        self.evidence = None;
        self.fill = Fill::Idle;
        self.list_fill = Fill::Idle;
        self.document = None;
        self.graph = None;
        self.bytes = None;
        self.result_reply = None;
        self.target = None;
        self.project = self.state.read(cx).project.clone();
        self.terminal.update(cx, |terminal, cx| {
            terminal.select_project(self.project.clone(), cx)
        });
        self.table.update(cx, |table, cx| {
            table.delegate_mut().clear();
            cx.notify();
        });
        self.refresh_snapshot(cx);
    }
    pub(super) fn refresh_snapshot(&mut self, cx: &mut Context<Self>) {
        let Some(project) = self.project.clone() else {
            self.tools.clear();
            self.list_fill = Fill::Idle;
            cx.notify();
            return;
        };
        let service = self.state.read(cx).services.analysis.clone();
        let generation = self.generation;
        run(
            cx,
            move || Ok((service.tools(&project)?, service.view(&project)?)),
            move |this, result, cx| {
                if generation != this.generation {
                    return;
                }
                match result {
                    Ok((tools, view)) => {
                        this.tools = tools;
                        for task in &mut this.tasks {
                            if !task.owned && view.busy.as_deref() != Some(task.tool.as_str()) {
                                task.done = true;
                            }
                        }
                        if this.target != view.active {
                            this.target = view.active.clone();
                            this.editor_generation += 1;
                            this.list_generation += 1;
                            this.selection = None;
                            this.fill = Fill::Idle;
                            this.list_fill = Fill::Idle;
                            this.table.update(cx, |table, cx| {
                                table.delegate_mut().clear();
                                cx.notify();
                            });
                        }
                        let kind = view
                            .targets
                            .iter()
                            .find(|target| Some(&target.id) == view.active.as_ref())
                            .map_or("", |target| target.kind.as_str());
                        if !this.list_kind.supports(kind)
                            && let Some(next) = super::data::ListKind::ALL
                                .into_iter()
                                .find(|list| list.supports(kind))
                        {
                            this.list_kind = next;
                        }
                        this.state.update(cx, |state, cx| {
                            state.analysis_view = Some(view);
                            cx.notify();
                        });
                        if matches!(this.list_fill, Fill::Idle) {
                            this.load_list(true, cx);
                        }
                        if matches!(this.fill, Fill::Idle) && this.selection.is_some() {
                            this.load_editor(cx);
                        }
                    }
                    Err(error) => this.report(&error, cx),
                }
                cx.notify();
            },
        );
    }
    pub(super) fn load_list(&mut self, reset: bool, cx: &mut Context<Self>) {
        if !self.list_kind.supports(self.target_kind(cx)) {
            self.list_fill =
                Fill::Unavailable(rust_i18n::t!("app.workbench.target_needed").into_owned());
            cx.notify();
            return;
        }
        if reset {
            self.list_generation += 1;
            self.table.update(cx, |table, cx| {
                table.delegate_mut().clear();
                cx.notify();
            });
        }
        let offset = self.table.read(cx).delegate().next.unwrap_or(0);
        let Some(tool) =
            data::choose(&self.tools, self.list_kind.tools(self.target_kind(cx))).cloned()
        else {
            self.list_fill =
                Fill::Unavailable(rust_i18n::t!("app.workbench.unavailable").into_owned());
            cx.notify();
            return;
        };
        let params = data::params(&tool, None, &self.target_path(cx), offset);
        self.request(tool, params, true, false, cx);
    }
    pub(super) fn load_editor(&mut self, cx: &mut Context<Self>) {
        self.editor_generation += 1;
        let Some(id) = self.tab.tool() else {
            if let Some(reply) = self.result_reply.clone() {
                self.install_reply(reply, cx);
            } else {
                self.fill = Fill::Idle;
            }
            cx.notify();
            return;
        };
        if matches!(
            self.tab,
            super::data::EditorTab::Pseudocode
                | super::data::EditorTab::Disassembly
                | super::data::EditorTab::Hex
                | super::data::EditorTab::Graph
        ) && self.selection.as_ref().and_then(data::address).is_none()
        {
            self.fill = Fill::Idle;
            cx.notify();
            return;
        }
        let Some(tool) = self.tools.iter().find(|tool| tool.id == id).cloned() else {
            self.fill = Fill::Unavailable(rust_i18n::t!("app.workbench.unavailable").into_owned());
            cx.notify();
            return;
        };
        let mut params = data::params(&tool, self.selection.as_ref(), &self.target_path(cx), 0);
        if self.tab == super::data::EditorTab::Source
            && tool
                .input_schema
                .pointer("/properties/output_directory")
                .is_some()
            && let (Some(project), Some(params)) = (&self.project, params.as_object_mut())
        {
            params.insert(
                "output_directory".to_owned(),
                Value::String(
                    std::path::Path::new(project.as_str())
                        .join("reference")
                        .join(format!("restored-{}", uuid::Uuid::new_v4()))
                        .display()
                        .to_string(),
                ),
            );
        }
        self.request(tool, params, false, false, cx);
    }
    pub(super) fn request(
        &mut self,
        tool: ToolInfo,
        params: Value,
        list: bool,
        consented: bool,
        cx: &mut Context<Self>,
    ) {
        let blocked = match &tool.available {
            Availability::Ready => None,
            Availability::NeedsDeep => Some(Fill::DeepNeeded),
            Availability::NeedsTarget => Some(Fill::Unavailable(
                rust_i18n::t!("app.workbench.target_needed").into_owned(),
            )),
            Availability::Missing { engine } => Some(Fill::EngineMissing {
                engine: engine.clone(),
                fix: rust_i18n::t!("app.workbench.engine_fix").into_owned(),
            }),
            Availability::Os => Some(Fill::Unavailable(
                rust_i18n::t!("app.workbench.os_unavailable").into_owned(),
            )),
        };
        if let Some(fill) = blocked {
            if list {
                self.list_fill = fill;
                self.table.update(cx, |table, cx| {
                    table.delegate_mut().loading = false;
                    cx.notify();
                });
            } else {
                self.fill = fill;
            }
            cx.notify();
            return;
        }
        if tool.needs_consent && !consented {
            self.pending = Some((tool, params, list));
            cx.notify();
            return;
        }
        let Some(project) = self.project.clone() else {
            return;
        };
        let service = self.state.read(cx).services.analysis.clone();
        let generation = self.generation;
        let editor_generation = self.editor_generation;
        let list_generation = self.list_generation;
        let cancel = CancelFlag::new();
        let job_cancel = cancel.clone();
        let task_index = self.tasks.len();
        let tool_id = tool.id.clone();
        let source_parent = if !list
            && self.tab == super::data::EditorTab::Source
            && tool_id == "js_sources_restore"
        {
            Some(std::path::Path::new(project.as_str()).join("reference"))
        } else {
            None
        };
        self.tasks.push(ToolTask {
            tool: tool.id.clone(),
            cancel,
            owned: true,
            message: rust_i18n::t!("app.workbench.loading").into_owned(),
            fraction: None,
            done: false,
        });
        if list {
            self.list_fill = Fill::Loading;
            self.table.update(cx, |table, cx| {
                table.delegate_mut().loading = true;
                cx.notify();
            });
        } else {
            self.fill = Fill::Loading;
        }
        run(
            cx,
            move || {
                if let Some(parent) = source_parent {
                    std::fs::create_dir_all(parent).map_err(|source| Error::Io {
                        context: rust_i18n::t!("app.workbench.source_unreadable").into_owned(),
                        source,
                    })?;
                }
                service.call(
                    &project,
                    ToolCall {
                        tool: tool_id,
                        params,
                        caller: CallerDto::Ui,
                        consented,
                    },
                    &job_cancel,
                )
            },
            move |this, result, cx| {
                if generation != this.generation {
                    return;
                }
                if let Some(task) = this.tasks.get_mut(task_index) {
                    task.done = true;
                    task.message = match &result {
                        Ok(_) => rust_i18n::t!("app.workbench.complete").into_owned(),
                        Err(error) => error.to_string(),
                    };
                }
                if (list && list_generation != this.list_generation)
                    || (!list && editor_generation != this.editor_generation)
                {
                    cx.notify();
                    return;
                }
                match result {
                    Ok(reply) => {
                        if list {
                            let rows = data::rows(&reply);
                            let next = data::next_offset(&reply);
                            this.table.update(cx, |table, cx| {
                                table.delegate_mut().append(rows, next);
                                cx.notify();
                            });
                            this.list_fill = Fill::Ready(reply);
                        } else {
                            let source = if this.tab == super::data::EditorTab::Source {
                                data::result(&reply)
                                    .get("output_directory")
                                    .and_then(Value::as_str)
                                    .zip(
                                        data::result(&reply)
                                            .pointer("/outputs/0/path")
                                            .and_then(Value::as_str),
                                    )
                                    .map(|(dir, path)| (dir.to_owned(), path.to_owned()))
                            } else {
                                None
                            };
                            this.install_reply(reply, cx);
                            if let Some((directory, path)) = source {
                                this.load_source(directory, path, cx);
                            }
                        }
                    }
                    Err(error) => {
                        let fill = error_fill(&error);
                        if list {
                            this.list_fill = fill;
                            this.table.update(cx, |table, cx| {
                                table.delegate_mut().loading = false;
                                cx.notify();
                            });
                        } else {
                            this.fill = fill;
                        }
                        this.report(&error, cx);
                    }
                }
                this.refresh_snapshot(cx);
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
        run(
            cx,
            move || service.set_mode(&project, AnalysisMode::Deep),
            move |this, result, cx| {
                if generation != this.generation {
                    return;
                }
                match result {
                    Ok(()) => {
                        this.list_fill = Fill::Idle;
                        this.refresh_snapshot(cx);
                        this.fill = Fill::Idle;
                    }
                    Err(error) => this.report(&error, cx),
                }
                cx.notify();
            },
        );
    }
    pub(super) fn load_source(&mut self, directory: String, path: String, cx: &mut Context<Self>) {
        let generation = self.generation;
        let editor_generation = self.editor_generation;
        run(
            cx,
            move || {
                use std::{fs, io::Read as _};
                let root = fs::canonicalize(directory).map_err(|_| {
                    Error::invalid(rust_i18n::t!("app.workbench.source_unreadable"))
                })?;
                let file = fs::canonicalize(root.join(path)).map_err(|_| {
                    Error::invalid(rust_i18n::t!("app.workbench.source_unreadable"))
                })?;
                if !file.starts_with(&root) {
                    return Err(Error::new(
                        ErrorKind::Forbidden,
                        rust_i18n::t!("app.workbench.source_unreadable"),
                    ));
                }
                let mut text = String::new();
                fs::File::open(file)
                    .and_then(|file| file.take(8 * 1024 * 1024 + 1).read_to_string(&mut text))
                    .map_err(|_| {
                        Error::invalid(rust_i18n::t!("app.workbench.source_unreadable"))
                    })?;
                if text.len() > 8 * 1024 * 1024 {
                    return Err(Error::invalid(rust_i18n::t!("app.workbench.source_large")));
                }
                Ok(text)
            },
            move |this, result, cx| {
                if generation != this.generation || editor_generation != this.editor_generation {
                    return;
                }
                match result {
                    Ok(text) => {
                        this.document = Some(std::rc::Rc::new(
                            autocraft_ui::components::CodeDocument::new(
                                text,
                                autocraft_ui::components::CodeLanguage::JavaScript,
                                Vec::new(),
                            ),
                        ))
                    }
                    Err(error) => {
                        this.fill = error_fill(&error);
                        this.report(&error, cx);
                    }
                }
                cx.notify();
            },
        );
    }
}
fn error_fill(error: &Error) -> Fill {
    match error.kind() {
        Some(ErrorKind::DeepRequired) => Fill::DeepNeeded,
        Some(ErrorKind::EngineMissing) => Fill::EngineMissing {
            engine: error.detail().to_owned(),
            fix: rust_i18n::t!("app.workbench.engine_fix").into_owned(),
        },
        _ => Fill::Error {
            message: error.to_string(),
            fix: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn deep_engine_and_execution_errors_have_distinct_states() {
        assert!(matches!(
            error_fill(&Error::new(ErrorKind::DeepRequired, "deep")),
            Fill::DeepNeeded
        ));
        assert!(
            matches!(error_fill(&Error::new(ErrorKind::EngineMissing,"ghidra")),Fill::EngineMissing { engine,.. } if engine=="ghidra")
        );
        assert!(
            matches!(error_fill(&Error::new(ErrorKind::Cancelled,"cancelled")),Fill::Error { message,.. } if message.contains("cancelled"))
        );
        assert!(
            matches!(error_fill(&Error::new(ErrorKind::ToolFailed,"failed")),Fill::Error { message,.. } if message.contains("failed"))
        );
    }
}
