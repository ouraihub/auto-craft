//! Tool execution and native report export through the pinned report tool.
use super::{CommandPalette, PaletteEvent};
use crate::state::{AppEvent, run};
use autocraft_core::{CallerDto, CancelFlag, Error, ToolCall, ToolInfo};
use gpui_kit::Context;
use serde_json::{Value, json};
use std::path::Path;
impl CommandPalette {
    pub(super) fn execute(
        &mut self,
        tool: ToolInfo,
        params: Value,
        consented: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(project) = self.project.clone() else {
            return;
        };
        if self.busy {
            return;
        }
        self.cancel = CancelFlag::new();
        let cancel = self.cancel.clone();
        self.busy = true;
        self.error = None;
        let generation = self.generation;
        let service = self.state.read(cx).services.analysis.clone();
        let event_project = project.clone();
        run(
            cx,
            move || {
                service.call(
                    &project,
                    ToolCall {
                        tool: tool.id,
                        params,
                        caller: CallerDto::Ui,
                        consented,
                    },
                    &cancel,
                )
            },
            move |this, result, cx| {
                if generation != this.generation {
                    return;
                }
                this.busy = false;
                this.confirming = false;
                match result {
                    Ok(reply) => {
                        cx.emit(PaletteEvent::ToolResult(reply));
                        this.state
                            .update(cx, |_, cx| cx.emit(AppEvent::Analysis(event_project)));
                    }
                    Err(error) => {
                        this.error = Some(error.to_string());
                        this.report(&error, cx);
                    }
                }
                cx.notify();
            },
        );
        cx.notify();
    }
    pub(super) fn export(&mut self, cx: &mut Context<Self>) {
        let Some(project) = self.project.clone() else {
            return;
        };
        let Some(tool) = self
            .tools
            .iter()
            .find(|tool| {
                tool.id == "core_report_write"
                    && tool.available == autocraft_core::Availability::Ready
            })
            .cloned()
        else {
            self.error = Some(rust_i18n::t!("app.palette.report_unavailable").into_owned());
            cx.notify();
            return;
        };
        let generation = self.generation;
        let picked = cx.prompt_for_new_path(Path::new(project.as_str()), Some("investigation.md"));
        cx.spawn(async move |this, cx| match picked.await {
            Ok(Ok(Some(path))) => {
                let _ = this.update(cx, |this, cx| {
                    if generation != this.generation {
                        return;
                    }
                    let params = json!({"format":"markdown","path":path,"include_evidence":true});
                    this.pending_export(tool, params, cx);
                });
            }
            Ok(Ok(None)) => {}
            Ok(Err(_)) | Err(_) => {
                let _ = this.update(cx, |this, cx| {
                    this.report(
                        &Error::invalid(rust_i18n::t!("app.palette.picker_failed")),
                        cx,
                    )
                });
            }
        })
        .detach();
    }
    fn pending_export(&mut self, tool: ToolInfo, params: Value, cx: &mut Context<Self>) {
        // Choosing a destination is not consent to a tool's other effects; preserve the same form confirmation.
        self.selected = Some(tool.clone());
        self.fields.clear();
        self.field_subscriptions.clear();
        self.confirming = tool.needs_consent;
        if tool.needs_consent {
            self.export_params = Some(params);
            cx.notify();
        } else {
            self.execute(tool, params, false, cx);
        }
    }
}
