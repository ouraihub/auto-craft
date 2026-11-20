//! Current-project pinned builds; tool calls own the copies and session process registry.
mod actions;
mod model;
mod render;
#[cfg(test)]
mod tests;

use crate::state::{AppEvent, AppState, run};
use autocraft_core::{Availability, CallerDto, CancelFlag, ProjectId, ToolCall, ToolInfo};
use gpui_kit::{
    AppContext as _, Context, Entity, Subscription, Task, Window,
    component::input::{InputEvent, InputState},
};
use model::{Estimate, Mode, Outcome, Row};
use serde_json::json;
use std::time::Duration;

/// Window-owned current-project pin workflow. No filesystem or process state is duplicated here.
pub struct PinsView {
    state: Entity<AppState>,
    project: Option<ProjectId>,
    generation: u64,
    rows: Vec<Row>,
    copied_bytes: u64,
    tools: Vec<ToolInfo>,
    name: Entity<InputState>,
    mode: Mode,
    estimate: Option<Estimate>,
    estimate_error: Option<String>,
    error: Option<String>,
    outcome: Option<Outcome>,
    busy: bool,
    cancel: CancelFlag,
    _subscriptions: Vec<Subscription>,
    _poll: Task<()>,
}
impl PinsView {
    /// Creates a retained view using the selected project's inherited pin tools.
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let name = cx.new(|cx| {
            InputState::new(window, cx).placeholder(rust_i18n::t!("app.pins.name_placeholder"))
        });
        let subscriptions = vec![
            cx.subscribe(&state, |this, _, event, cx| {
                if matches!(event, AppEvent::Project) {
                    this.sync_project(cx);
                }
                if matches!(event, AppEvent::Rebuild(project) if this.project.as_ref() == Some(project))
                    && !this.busy
                {
                    this.estimate = None;
                    this.estimate_error = None;
                    this.refresh(cx);
                }
                // Pin reads also publish Analysis; refreshing on that event would loop forever.
                cx.notify();
            }),
            cx.subscribe(&name, |_, _, _: &InputEvent, cx| cx.notify()),
        ];
        let poll = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(3)).await;
                if this
                    .update(cx, |this, cx| {
                        if !this.busy && this.rows.iter().any(|row| row.running) {
                            this.refresh(cx);
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        let mut this = Self {
            state,
            project: None,
            generation: 0,
            rows: Vec::new(),
            copied_bytes: 0,
            tools: Vec::new(),
            name,
            mode: Mode::Frozen,
            estimate: None,
            estimate_error: None,
            error: None,
            outcome: None,
            busy: false,
            cancel: CancelFlag::new(),
            _subscriptions: subscriptions,
            _poll: poll,
        };
        this.sync_project(cx);
        this
    }
    fn sync_project(&mut self, cx: &mut Context<Self>) {
        let project = self.state.read(cx).project.clone();
        if project == self.project {
            return;
        }
        self.cancel.cancel();
        self.generation += 1;
        self.project = project;
        self.rows.clear();
        self.tools.clear();
        self.copied_bytes = 0;
        self.estimate = None;
        self.estimate_error = None;
        self.error = None;
        self.outcome = None;
        self.busy = false;
        self.mode = Mode::Frozen;
        self.refresh(cx);
        cx.notify();
    }
    fn current(&self, generation: u64, project: &ProjectId, cx: &Context<Self>) -> bool {
        let current = generation == self.generation
            && self.project.as_ref() == Some(project)
            && self.state.read(cx).project.as_ref() == Some(project);
        if !current {
            tracing::debug!(reason = "project_changed", "pins.completion.discarded");
        }
        current
    }
    fn ready(&self, id: &str) -> bool {
        self.tools
            .iter()
            .any(|tool| tool.id == id && tool.available == Availability::Ready)
    }
    fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let Some(project) = self.project.clone() else {
            return;
        };
        self.busy = true;
        self.cancel = CancelFlag::new();
        let cancel = self.cancel.clone();
        let service = self.state.read(cx).services.analysis.clone();
        let generation = self.generation;
        let target = project.clone();
        run(
            cx,
            move || {
                let tools = service.tools(&target)?;
                let list = model::listing(service.call(
                    &target,
                    ToolCall {
                        tool: "project_pins_list".to_owned(),
                        params: json!({"project": target.as_str()}),
                        caller: CallerDto::Ui,
                        consented: false,
                    },
                    &cancel,
                )?)?;
                Ok((tools, list))
            },
            move |this, result, cx| {
                if !this.current(generation, &project, cx) {
                    return;
                }
                this.busy = false;
                match result {
                    Ok((tools, list)) => {
                        this.tools = tools;
                        this.rows = list.pins;
                        this.copied_bytes = list.copied_bytes;
                        if this.estimate.is_none() && this.estimate_error.is_none() {
                            this.load_estimate(cx);
                        }
                    }
                    Err(error) => this.report(&error, cx),
                }
                cx.notify();
            },
        );
        cx.notify();
    }
    fn load_estimate(&mut self, cx: &mut Context<Self>) {
        if self.busy || !self.ready("project_pin_create") {
            return;
        }
        let Some(project) = self.project.clone() else {
            return;
        };
        self.busy = true;
        self.estimate = None;
        self.estimate_error = None;
        self.cancel = CancelFlag::new();
        let cancel = self.cancel.clone();
        let service = self.state.read(cx).services.analysis.clone();
        let target = project.clone();
        let generation = self.generation;
        run(
            cx,
            move || {
                model::estimate(service.call(
                    &target,
                    ToolCall {
                        tool: "project_pin_create".to_owned(),
                        params: json!({"project": target.as_str(), "estimate": true}),
                        caller: CallerDto::Ui,
                        consented: false,
                    },
                    &cancel,
                )?)
            },
            move |this, result, cx| {
                if !this.current(generation, &project, cx) {
                    return;
                }
                this.busy = false;
                match result {
                    Ok(estimate) => this.estimate = Some(estimate),
                    Err(error) => {
                        this.estimate_error = Some(error.to_string());
                        // A fresh project has nothing to pin yet; keep the local note but
                        // don't raise a top-level banner for the expected NOT_FOUND.
                        if error.kind() != Some(autocraft_core::ErrorKind::NotFound) {
                            this.state.update(cx, |state, cx| state.report(&error, cx));
                        }
                    }
                }
                cx.notify();
            },
        );
        cx.notify();
    }
    fn report(&mut self, error: &autocraft_core::Error, cx: &mut Context<Self>) {
        self.error = Some(error.to_string());
        self.state.update(cx, |state, cx| state.report(error, cx));
    }
}
impl Drop for PinsView {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}
