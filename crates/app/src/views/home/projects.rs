//! Project cards use the authoritative app list and native menus/dialogs.
use super::HomeView;
use crate::state::run;
use autocraft_core::{ProjectId, ProjectStatus, ProjectSummary, TargetRefDto};
use autocraft_ui::{
    AutocraftTheme,
    components::label,
    metrics::{radius, space, text},
};
use gpui_kit::{
    AnyElement, AppContext as _, Context, InteractiveElement as _, IntoElement, ParentElement as _,
    Styled as _, Window,
    base::{h_flex, v_flex},
    component::{
        WindowExt as _,
        button::{Button, ButtonVariants as _},
        input::{Input, InputState},
        menu::{ContextMenuExt as _, PopupMenuItem},
    },
};

impl HomeView {
    pub(super) fn projects(&self, cx: &mut Context<Self>) -> AnyElement {
        let r = AutocraftTheme::global(cx).roles;
        let cards = self
            .state
            .read(cx)
            .projects
            .clone()
            .into_iter()
            .map(|project| {
                let id = project.id.clone();
                let card = v_flex()
                    .id(format!("home-project-{id}"))
                    .w(gpui_kit::px(280.))
                    .max_w_full()
                    .min_w_0()
                    .p(space::S16)
                    .gap(space::S8)
                    .rounded(radius::LG)
                    .border_1()
                    .border_color(r.border_default)
                    .bg(r.container_elevated_bg)
                    .child(label(text::TITLE_CARD, project.name.clone()).truncate())
                    .child(
                        label(text::METADATA, target_label(&project.target))
                            .text_color(r.text_subtle)
                            .truncate(),
                    )
                    .child(
                        h_flex()
                            .flex_wrap()
                            .gap(space::S8)
                            .child(label(
                                text::METADATA,
                                format!("● {}", rust_i18n::t!(status_key(project.status))),
                            ))
                            .child(
                                label(text::METADATA, updated(project.updated_ms))
                                    .text_color(r.text_subtle),
                            ),
                    )
                    .child(
                        Button::new(format!("home-open-{id}"))
                            .outline()
                            .label(rust_i18n::t!("app.home.open").to_string())
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.state
                                    .update(cx, |state, cx| state.select_project(id.clone(), cx));
                            })),
                    );
                let view = cx.entity();
                card.context_menu(move |menu, _, _| {
                    let open = view.clone();
                    let open_id = project.id.clone();
                    let rename = view.clone();
                    let rename_project = project.clone();
                    let duplicate = view.clone();
                    let duplicate_project = project.clone();
                    let delete = view.clone();
                    let delete_project = project.clone();
                    menu.item(
                        PopupMenuItem::new(rust_i18n::t!("app.home.open_folder").to_string())
                            .on_click(move |_, _, cx| {
                                open.update(cx, |this, cx| this.open_folder(open_id.clone(), cx))
                            }),
                    )
                    .item(
                        PopupMenuItem::new(rust_i18n::t!("app.home.rename").to_string())
                            .disabled(project.read_only)
                            .on_click(move |_, window, cx| {
                                rename.update(cx, |this, cx| {
                                    this.name_dialog(rename_project.clone(), false, window, cx)
                                })
                            }),
                    )
                    .item(
                        PopupMenuItem::new(rust_i18n::t!("app.home.duplicate").to_string())
                            .on_click(move |_, window, cx| {
                                duplicate.update(cx, |this, cx| {
                                    this.name_dialog(duplicate_project.clone(), true, window, cx)
                                })
                            }),
                    )
                    .item(
                        PopupMenuItem::new(rust_i18n::t!("app.home.delete").to_string())
                            .disabled(project.read_only)
                            .on_click(move |_, window, cx| {
                                delete.update(cx, |this, cx| {
                                    this.confirm_delete(delete_project.clone(), window, cx)
                                })
                            }),
                    )
                })
                .into_any_element()
            })
            .collect::<Vec<_>>();
        v_flex()
            .gap(space::S12)
            .child(
                h_flex()
                    .gap(space::S12)
                    .child(label(
                        text::HEADING_SM,
                        rust_i18n::t!("app.home.projects").to_string(),
                    ))
                    .child(
                        Button::new("home-refresh-projects")
                            .outline()
                            .label(rust_i18n::t!("app.home.refresh").to_string())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.state.update(cx, |state, cx| state.reload(cx))
                            })),
                    ),
            )
            .child(h_flex().flex_wrap().gap(space::S16).children(cards))
            .into_any_element()
    }

    fn open_folder(&mut self, id: ProjectId, cx: &mut Context<Self>) {
        let projects = self.state.read(cx).services.projects.clone();
        run(
            cx,
            move || projects.open(&id).map(|p| p.app_dir),
            |this, result, cx| match result {
                Ok(path) => cx.reveal_path(&path),
                Err(error) => this.report(&error, cx),
            },
        );
    }

    fn name_dialog(
        &mut self,
        project: ProjectSummary,
        duplicate: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let initial = if duplicate {
            rust_i18n::t!("app.home.copy_name", name = project.name).to_string()
        } else {
            project.name
        };
        let name = cx.new(|cx| InputState::new(window, cx).default_value(initial));
        let view = cx.entity();
        window.open_dialog(cx, move |dialog, _, _| {
            let view = view.clone();
            let name = name.clone();
            let id = project.id.clone();
            let save_view = view.clone();
            let save_name = name.clone();
            let save_id = id.clone();
            let footer = h_flex()
                .gap(space::S8)
                .child(
                    Button::new("home-name-cancel")
                        .outline()
                        .label(rust_i18n::t!("app.home.cancel").to_string())
                        .on_click(|_, window, cx| window.close_dialog(cx)),
                )
                .child(
                    Button::new("home-name-save")
                        .primary()
                        .label(rust_i18n::t!("app.home.save").to_string())
                        .on_click(move |_, window, cx| {
                            let value = save_name.read(cx).value().trim().to_owned();
                            if value.is_empty() {
                                return;
                            }
                            let id = save_id.clone();
                            save_view.update(cx, |this, cx| {
                                let projects = this.state.read(cx).services.projects.clone();
                                run(
                                    cx,
                                    move || {
                                        if duplicate {
                                            projects.duplicate(&id, &value)
                                        } else {
                                            projects.rename(&id, &value)
                                        }
                                    },
                                    |this, result, cx| {
                                        if let Err(error) = result {
                                            this.report(&error, cx);
                                        }
                                        this.state.update(cx, |state, cx| state.reload(cx));
                                    },
                                );
                            });
                            window.close_dialog(cx);
                        }),
                );
            dialog
                .title(
                    rust_i18n::t!(if duplicate {
                        "app.home.duplicate"
                    } else {
                        "app.home.rename"
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
                        let projects = this.state.read(cx).services.projects.clone();
                        let id = id.clone();
                        run(
                            cx,
                            move || {
                                if duplicate {
                                    projects.duplicate(&id, &value)
                                } else {
                                    projects.rename(&id, &value)
                                }
                            },
                            |this, result, cx| {
                                if let Err(error) = result {
                                    this.report(&error, cx);
                                }
                                this.state.update(cx, |state, cx| state.reload(cx));
                            },
                        );
                    });
                    true
                })
        });
    }

    fn confirm_delete(
        &mut self,
        project: ProjectSummary,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let view = cx.entity();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let view = view.clone();
            let id = project.id.clone();
            alert
                .title(rust_i18n::t!("app.home.delete").to_string())
                .description(
                    rust_i18n::t!("app.home.delete_confirm", name = project.name).to_string(),
                )
                .show_cancel(true)
                .ok_text(rust_i18n::t!("app.home.delete").to_string())
                .on_ok(move |_, _, cx| {
                    let id = id.clone();
                    view.update(cx, |this, cx| {
                        let projects = this.state.read(cx).services.projects.clone();
                        run(
                            cx,
                            move || projects.delete(&id),
                            |this, result, cx| {
                                if let Err(error) = result {
                                    this.report(&error, cx);
                                }
                                this.state.update(cx, |state, cx| state.reload(cx));
                            },
                        );
                    });
                    true
                })
        });
    }
}
fn status_key(status: ProjectStatus) -> &'static str {
    match status {
        ProjectStatus::New => "app.home.status_new",
        ProjectStatus::Inspecting => "app.home.status_inspecting",
        ProjectStatus::Building => "app.home.status_building",
        ProjectStatus::Checking => "app.home.status_checking",
        ProjectStatus::NeedsYou => "app.home.status_needs_you",
        ProjectStatus::Ready => "app.home.status_ready",
        ProjectStatus::Stopped => "app.home.status_stopped",
    }
}
fn target_label(target: &TargetRefDto) -> String {
    match target {
        TargetRefDto::Website { url } => url.clone(),
        TargetRefDto::Local { path } => path.display().to_string(),
        TargetRefDto::None => rust_i18n::t!("app.home.no_target").to_string(),
    }
}
fn updated(ms: u64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let minutes = now.saturating_sub(ms / 1000) / 60;
    rust_i18n::t!("app.home.updated_minutes", count = minutes).to_string()
}
