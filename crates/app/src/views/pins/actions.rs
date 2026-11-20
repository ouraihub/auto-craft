//! Permission, deletion and rename dialogs capture the originating project and generation.
use super::{
    PinsView,
    model::{self, Action},
};
use crate::state::run;
use autocraft_core::{CallerDto, CancelFlag, ToolCall};
use gpui_kit::{
    AppContext as _, Context, ParentElement as _, Window,
    component::{
        WindowExt as _,
        input::{Input, InputState},
    },
};

impl PinsView {
    pub(super) fn confirm(&mut self, action: Action, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || !self.ready(action.tool()) {
            return;
        }
        let Some(project) = self.project.clone() else {
            return;
        };
        let generation = self.generation;
        let pin = match &action {
            Action::Open { pin, .. }
            | Action::Stop(pin)
            | Action::Rename { pin, .. }
            | Action::Delete(pin)
            | Action::Folder(pin) => Some(*pin),
            Action::Create { .. } => None,
        };
        let name = pin
            .and_then(|pin| self.rows.iter().find(|row| row.pin.id == pin))
            .map(|row| row.pin.name.clone());
        let view = cx.entity();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let action = action.clone();
            let project = project.clone();
            let view = view.clone();
            alert
                .title(rust_i18n::t!("app.pins.confirm_title"))
                .description(match &name {
                    Some(name) => format!(
                        "{}\n\n{}",
                        rust_i18n::t!("app.pins.target", name = name),
                        rust_i18n::t!(action.confirmation())
                    ),
                    None => rust_i18n::t!(action.confirmation()).into_owned(),
                })
                .show_cancel(true)
                .ok_text(rust_i18n::t!("app.pins.confirm"))
                .on_ok(move |_, _, cx| {
                    view.update(cx, |this, cx| {
                        if !this.current(generation, &project, cx) {
                            return true;
                        }
                        if this.busy {
                            return false;
                        }
                        this.execute(action.clone(), cx);
                        true
                    })
                })
        });
    }
    pub(super) fn rename(&mut self, pin: u32, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || !self.ready("project_pin_rename") {
            return;
        }
        let Some(row) = self.rows.iter().find(|row| row.pin.id == pin) else {
            return;
        };
        let Some(project) = self.project.clone() else {
            return;
        };
        let input = cx.new(|cx| InputState::new(window, cx).default_value(row.pin.name.clone()));
        let generation = self.generation;
        let view = cx.entity();
        window.open_dialog(cx, move |dialog, _, _| {
            let input = input.clone();
            let value = input.clone();
            let view = view.clone();
            let project = project.clone();
            dialog
                .title(rust_i18n::t!("app.pins.rename"))
                .child(Input::new(&input))
                .on_ok(move |_, _, cx| {
                    let name = value.read(cx).value().trim().to_owned();
                    if name.is_empty() {
                        return false;
                    }
                    view.update(cx, |this, cx| {
                        if !this.current(generation, &project, cx) {
                            return true;
                        }
                        if this.busy {
                            return false;
                        }
                        this.execute(Action::Rename { pin, name }, cx);
                        true
                    })
                })
        });
    }
    fn execute(&mut self, action: Action, cx: &mut Context<Self>) {
        if self.busy || !self.ready(action.tool()) {
            return;
        }
        let Some(project) = self.project.clone() else {
            return;
        };
        // Confirmation is scoped to this exact action; never cache permission for later calls.
        self.cancel = CancelFlag::new();
        let cancel = self.cancel.clone();
        self.busy = true;
        self.error = None;
        self.outcome = None;
        let service = self.state.read(cx).services.analysis.clone();
        let generation = self.generation;
        let target = project.clone();
        let requested = action.clone();
        run(
            cx,
            move || {
                model::outcome(
                    &requested,
                    service.call(
                        &target,
                        ToolCall {
                            tool: requested.tool().to_owned(),
                            params: requested.params(target.as_str()),
                            caller: CallerDto::Ui,
                            consented: true,
                        },
                        &cancel,
                    )?,
                )
            },
            move |this, result, cx| {
                if !this.current(generation, &project, cx) {
                    return;
                }
                this.busy = false;
                match result {
                    Ok(outcome) => {
                        this.outcome = Some(outcome);
                        if matches!(action, Action::Create { .. }) {
                            this.estimate = None;
                        }
                    }
                    Err(error) => this.report(&error, cx),
                }
                // Opening can fail after a process was started; always reconcile the true run list.
                this.refresh(cx);
                cx.notify();
            },
        );
        cx.notify();
    }
}
