//! Project management dialogs and sidebar service operations.
use super::ProjectView;
use crate::state::run;
use autocraft_core::{AnalysisMode, CancelFlag, Error, FeedEntryDto, PersonChange};
use gpui_kit::{
    AppContext as _, Context, ParentElement as _, Window,
    component::{
        WindowExt as _,
        button::Button,
        dialog::{DialogAction, DialogClose, DialogFooter},
        input::{Input, InputState},
    },
};
use std::time::Duration;
impl ProjectView {
    pub(super) fn person(&mut self, change: PersonChange, cx: &mut Context<Self>) {
        if self.busy || !self.writable() {
            return;
        }
        let Some(project) = self.project.clone() else {
            return;
        };
        let service = self.state.read(cx).services.projects.clone();
        let generation = self.generation;
        self.busy = true;
        run(
            cx,
            move || service.set_person(&project, change),
            move |this, result, cx| {
                if this.generation != generation {
                    return;
                }
                this.busy = false;
                match result {
                    Ok(detail) => this.detail = Some(detail),
                    Err(error) => this.report(&error, cx),
                }
                cx.notify();
            },
        );
    }
    pub(super) fn set_mode(&mut self, mode: AnalysisMode, cx: &mut Context<Self>) {
        if self.busy || !self.writable() {
            return;
        }
        let Some(project) = self.project.clone() else {
            return;
        };
        let services = self.state.read(cx).services.clone();
        let generation = self.generation;
        self.busy = true;
        run(
            cx,
            move || {
                services.analysis.set_mode(&project, mode)?;
                services.project_prefs.get(&project)
            },
            move |this, result, cx| {
                if this.generation != generation {
                    return;
                }
                this.busy = false;
                match result {
                    Ok(prefs) => this.prefs = Some(prefs),
                    Err(error) => this.report(&error, cx),
                }
                this.reload(cx);
            },
        );
    }
    pub(super) fn save_budget(&mut self, cx: &mut Context<Self>) {
        let budget = self.budget.read(cx).value().trim().parse::<f64>();
        match budget {
            Ok(budget) if budget.is_finite() && budget >= 0. => {
                self.person(PersonChange::AssetBudget(budget), cx)
            }
            Ok(_) | Err(_) => self.report(
                &Error::invalid(rust_i18n::t!("app.project.budget_invalid")),
                cx,
            ),
        }
    }
    pub(super) fn preview(&mut self, play: bool, cx: &mut Context<Self>) {
        let Some(project) = self.project.clone() else {
            return;
        };
        let Some(service) = self.state.read(cx).services.rebuild.clone() else {
            return;
        };
        let generation = self.generation;
        run(
            cx,
            move || {
                if play {
                    service.play(&project).map(|()| None)
                } else {
                    service.preview(&project).map(Some)
                }
            },
            move |this, result, cx| {
                if this.generation != generation {
                    return;
                }
                match result {
                    Ok(Some(url)) => cx.open_url(&url),
                    Ok(None) => {}
                    Err(error) => this.report(&error, cx),
                }
            },
        );
    }
    pub(super) fn self_check(&mut self, cx: &mut Context<Self>) {
        if self.busy || !self.writable() {
            return;
        }
        let Some(project) = self.project.clone() else {
            return;
        };
        let Some(service) = self.state.read(cx).services.rebuild.clone() else {
            return;
        };
        let generation = self.generation;
        self.busy = true;
        self.request_cancel = CancelFlag::new();
        let cancel = self.request_cancel.clone();
        run(
            cx,
            move || service.self_check(&project, Duration::from_secs(120), &cancel),
            move |this, result, cx| {
                if this.generation != generation {
                    return;
                }
                this.busy = false;
                match result {
                    Ok(text) => this.push(
                        FeedEntryDto::Agent {
                            text,
                            streaming: false,
                        },
                        cx,
                    ),
                    Err(error) => this.report(&error, cx),
                }
                this.reload(cx);
            },
        );
    }
    pub(super) fn load_activity(&mut self, reset: bool, cx: &mut Context<Self>) {
        let Some(project) = self.project.clone() else {
            return;
        };
        let Some(service) = self.state.read(cx).services.rebuild.clone() else {
            return;
        };
        let generation = self.generation;
        let offset = if reset { 0 } else { self.activity_offset };
        run(
            cx,
            move || service.activity(&project, offset, 50),
            move |this, result, cx| {
                if this.generation != generation {
                    return;
                }
                match result {
                    Ok(lines) => {
                        if reset {
                            this.activity.clear();
                        }
                        this.activity_offset = offset + lines.len();
                        this.activity.extend(lines);
                    }
                    Err(error) => this.report(&error, cx),
                }
                cx.notify();
            },
        );
    }
    pub(super) fn open_folder(&mut self, app: bool, cx: &mut Context<Self>) {
        let Some(project) = self.project.clone() else {
            return;
        };
        let services = self.state.read(cx).services.clone();
        let generation = self.generation;
        run(
            cx,
            move || {
                if app {
                    services.projects.open(&project).map(|d| d.app_dir)
                } else {
                    services.project_dir(&project)
                }
            },
            move |this, result, cx| {
                if this.generation != generation {
                    return;
                }
                match result {
                    Ok(path) => cx.reveal_path(&path),
                    Err(error) => this.report(&error, cx),
                }
            },
        );
    }
    pub(super) fn name_dialog(
        &mut self,
        duplicate: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(detail) = &self.detail else {
            return;
        };
        let project = detail.summary.id.clone();
        let initial = if duplicate {
            rust_i18n::t!("app.home.copy_name", name = detail.summary.name).to_string()
        } else {
            detail.summary.name.clone()
        };
        let name = cx.new(|cx| InputState::new(window, cx).default_value(initial));
        let view = cx.entity();
        let generation = self.generation;
        window.open_dialog(cx, move |dialog, _, _| {
            let view = view.clone();
            let name = name.clone();
            let project = project.clone();
            let footer = DialogFooter::new()
                .child(DialogClose::new().trigger(|button| {
                    button.label(rust_i18n::t!("app.project.cancel").to_string())
                }))
                .child(
                    DialogAction::new().child(
                        Button::new("project-name-save")
                            .label(rust_i18n::t!("app.project.save").to_string()),
                    ),
                );
            dialog
                .title(
                    rust_i18n::t!(if duplicate {
                        "app.project.duplicate"
                    } else {
                        "app.project.rename"
                    })
                    .to_string(),
                )
                .child(Input::new(&name))
                .footer(footer)
                .on_ok(move |_, _, cx| {
                    let value = name.read(cx).value().trim().to_owned();
                    if value.is_empty() {
                        return false;
                    }
                    view.update(cx, |this, cx| {
                        if this.generation != generation
                            || (duplicate && this.running())
                            || this.busy
                        {
                            return;
                        }
                        let projects = this.state.read(cx).services.projects.clone();
                        let project = project.clone();
                        this.busy = true;
                        run(
                            cx,
                            move || {
                                if duplicate {
                                    projects.duplicate(&project, &value)
                                } else {
                                    projects.rename(&project, &value)
                                }
                            },
                            move |this, result, cx| {
                                if this.generation != generation {
                                    return;
                                }
                                this.busy = false;
                                match result {
                                    Ok(_) => {
                                        this.state.update(cx, |state, cx| state.reload(cx));
                                        this.reload(cx);
                                    }
                                    Err(error) => this.report(&error, cx),
                                }
                                cx.notify();
                            },
                        );
                    });
                    true
                })
        });
    }
    pub(super) fn confirm_delete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(detail) = &self.detail else {
            return;
        };
        let project = detail.summary.clone();
        let view = cx.entity();
        let generation = self.generation;
        window.open_alert_dialog(cx, move |dialog, _, _| {
            let view = view.clone();
            let project = project.clone();
            dialog
                .title(rust_i18n::t!("app.project.delete").to_string())
                .description(
                    rust_i18n::t!("app.home.delete_confirm", name = project.name).to_string(),
                )
                .show_cancel(true)
                .on_ok(move |_, _, cx| {
                    view.update(cx, |this, cx| {
                        if this.generation != generation
                            || this.running()
                            || this.busy
                            || !this.writable()
                        {
                            return;
                        }
                        let service = this.state.read(cx).services.projects.clone();
                        let project = project.id.clone();
                        this.busy = true;
                        run(
                            cx,
                            move || service.delete(&project),
                            move |this, result, cx| {
                                if this.generation != generation {
                                    return;
                                }
                                this.busy = false;
                                match result {
                                    Ok(()) => this.state.update(cx, |state, cx| state.reload(cx)),
                                    Err(error) => this.report(&error, cx),
                                }
                                cx.notify();
                            },
                        );
                    });
                    true
                })
        });
    }
}
