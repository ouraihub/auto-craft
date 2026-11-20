//! Native titlebar controls and navigation.
use super::*;
use gpui_kit::{Div, prelude::FluentBuilder as _};

impl TopBar {
    pub(super) fn controls(&mut self, stacked: bool, cx: &mut Context<Self>) -> Div {
        crate::i18n::refresh_picker(&self.picker, cx, &mut self.picker_locale);
        let r = AutocraftTheme::global(cx).roles;
        let state = self.state.read(cx);
        let selected = state.project.clone();
        let enabled = selected.is_some();
        let projects = state.projects.clone();
        let main = state.main;
        let name = state
            .detail
            .as_ref()
            .map(|d| d.summary.name.clone())
            .unwrap_or_else(|| rust_i18n::t!("app.home.title").into_owned());
        let targets = state
            .analysis_view
            .as_ref()
            .map(|v| v.targets.clone())
            .unwrap_or_default();
        let active = state.analysis_view.as_ref().and_then(|v| v.active.clone());
        let target = targets
            .iter()
            .find(|t| Some(&t.id) == active.as_ref())
            .map(|t| t.name.clone())
            .unwrap_or_else(|| rust_i18n::t!("app.workbench.open_target").into_owned());
        let mode = state
            .detail
            .as_ref()
            .map(|d| d.mode)
            .unwrap_or(AnalysisMode::CleanRoom);
        let usage = state.detail.as_ref().map(|d| {
            let u = &d.usage;
            if u.version == 0 {
                rust_i18n::t!("app.shell.usage_unknown").into_owned()
            } else {
                format!(
                    "↑ {} · ↓ {} · {} · {}m",
                    u.input_tokens,
                    u.output_tokens,
                    if u.priced {
                        format!("${:.4}", u.cost_usd)
                    } else {
                        "—".into()
                    },
                    u.work_ms / 60000
                )
            }
        });
        let entity = cx.entity();
        let target_entity = entity.clone();
        let mode_disabled = !enabled || self.building(cx) || self.changing;
        let nav_state = self.state.clone();
        let mut row = h_flex()
            .w_full()
            .min_w(gpui_kit::px(0.))
            .when(stacked, |row| row.flex_col().items_stretch())
            .when(!stacked, |row| row.h_full())
            // Keep controls out of TitleBar's native drag hitbox; only the spacer drags.
            .occlude()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .gap(space::S8)
            .px(space::S12)
            .child(
                Button::new("home-brand")
                    .ghost()
                    .label("autocraft")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.state
                            .update(cx, |state, cx| state.navigate(MainView::Home, cx))
                    })),
            )
            .child(
                Button::new("project-picker")
                    .ghost()
                    .label(name)
                    .max_w(if stacked {
                        autocraft_ui::metrics::content::FORM_FIELD
                    } else {
                        autocraft_ui::metrics::content::FORM_BUILDER
                    })
                    .dropdown_menu(move |menu, _, _| {
                        let mut sorted = projects.clone();
                        sorted.sort_by_key(|p| std::cmp::Reverse(p.updated_ms));
                        let mut menu = menu.item(
                            PopupMenuItem::new(
                                rust_i18n::t!("app.shell.recent_projects").into_owned(),
                            )
                            .disabled(true),
                        );
                        for project in sorted.iter().take(5) {
                            let id = project.id.clone();
                            let entity = entity.clone();
                            menu = menu.item(
                                PopupMenuItem::new(project.name.clone())
                                    .checked(selected.as_ref() == Some(&id))
                                    .on_click(move |_, _, cx| {
                                        entity.update(cx, |this, cx| {
                                            this.state.update(cx, |state, cx| {
                                                state.select_project(id.clone(), cx)
                                            })
                                        })
                                    }),
                            );
                        }
                        menu = menu.item(
                            PopupMenuItem::new(
                                rust_i18n::t!("app.shell.all_projects").into_owned(),
                            )
                            .disabled(true),
                        );
                        for project in &sorted {
                            let id = project.id.clone();
                            let entity = entity.clone();
                            menu = menu.item(
                                PopupMenuItem::new(project.name.clone())
                                    .checked(selected.as_ref() == Some(&id))
                                    .on_click(move |_, _, cx| {
                                        entity.update(cx, |this, cx| {
                                            this.state.update(cx, |state, cx| {
                                                state.select_project(id.clone(), cx)
                                            })
                                        })
                                    }),
                            );
                        }
                        let entity = entity.clone();
                        menu.item(
                            PopupMenuItem::new(rust_i18n::t!("app.shell.new_project").into_owned())
                                .on_click(move |_, _, cx| {
                                    entity.update(cx, |this, cx| {
                                        this.state.update(cx, |state, cx| {
                                            state.navigate(MainView::Home, cx)
                                        })
                                    })
                                }),
                        )
                    }),
            )
            .child(
                Button::new("target-picker")
                    .ghost()
                    .label(target)
                    .max_w(autocraft_ui::metrics::content::FORM_BUILDER)
                    .disabled(!enabled)
                    .dropdown_menu(move |mut menu, _, _| {
                        for target in &targets {
                            let id = target.id.clone();
                            let entity = target_entity.clone();
                            menu = menu.item(
                                PopupMenuItem::new(target.name.clone())
                                    .checked(active.as_ref() == Some(&id))
                                    .on_click(move |_, _, cx| {
                                        entity.update(cx, |this, cx| {
                                            this.target_call(
                                                "core_target_select",
                                                serde_json::json!({"target_id": id.as_str()}),
                                                cx,
                                            )
                                        })
                                    }),
                            );
                        }
                        let entity = target_entity.clone();
                        menu.item(
                            PopupMenuItem::new(
                                rust_i18n::t!("app.workbench.open_target").into_owned(),
                            )
                            .on_click(move |_, window, cx| {
                                entity.update(cx, |this, cx| this.open_target(window, cx))
                            }),
                        )
                    }),
            )
            .child(
                ChoiceMenu::new(
                    "main-view",
                    [
                        rust_i18n::t!("app.project.title").into_owned(),
                        rust_i18n::t!("app.workbench.title").into_owned(),
                    ],
                    match main {
                        MainView::Project => Some(0),
                        MainView::Workbench => Some(1),
                        _ => None,
                    },
                )
                .ghost()
                .placeholder(rust_i18n::t!("app.shell.view").into_owned())
                .disabled(!enabled)
                .on_select(move |index, _, cx| {
                    let view = if *index == 0 {
                        MainView::Project
                    } else {
                        MainView::Workbench
                    };
                    nav_state.update(cx, |state, cx| state.navigate(view, cx))
                }),
            )
            .child(
                ChoiceMenu::new(
                    "analysis-mode",
                    [
                        rust_i18n::t!("app.home.clean_room").into_owned(),
                        rust_i18n::t!("app.home.deep").into_owned(),
                    ],
                    Some(usize::from(mode == AnalysisMode::Deep)),
                )
                .ghost()
                .disabled(mode_disabled)
                .tooltip(
                    rust_i18n::t!(if mode_disabled && enabled {
                        "app.shell.mode_busy"
                    } else {
                        "app.shell.mode_hint"
                    })
                    .into_owned(),
                )
                .on_select(cx.listener(|this, _: &usize, _, cx| this.toggle_mode(cx))),
            )
            .child(self.backend_menu(cx))
            .when(stacked, |row| {
                row.child(
                    gpui_kit::div()
                        .w_full()
                        .min_w_0()
                        .overflow_hidden()
                        .child(self.picker.clone()),
                )
            })
            .when(!stacked, |row| {
                row.child(
                    gpui_kit::div()
                        .max_w(autocraft_ui::metrics::content::FORM_BUILDER)
                        .min_w(gpui_kit::px(0.))
                        .overflow_hidden()
                        .child(self.picker.clone()),
                )
                .child(
                    gpui_kit::div()
                        .flex_1()
                        .min_w(space::S16)
                        .h_full()
                        .window_control_area(WindowControlArea::Drag),
                )
            });
        if let Some(usage) = usage {
            row = row.child(
                Button::new("project-usage")
                    .ghost()
                    .label(rust_i18n::t!("app.shell.usage").into_owned())
                    .tooltip(usage),
            );
        }
        row = row.child(
            Button::new("open-settings")
                .ghost()
                .label(rust_i18n::t!("app.shell.settings").into_owned())
                .on_click(cx.listener(|this, _, window, cx| {
                    SettingsDialog::open(this.state.clone(), window, cx)
                })),
        );
        if self.confirm_deep {
            row = row
                .child(
                    label(
                        text::METADATA,
                        rust_i18n::t!("app.workbench.deep_warning").into_owned(),
                    )
                    .max_w(autocraft_ui::metrics::content::FORM_FIELD)
                    .text_color(r.feedback_warning),
                )
                .child(
                    Button::new("confirm-deep")
                        .ghost()
                        .disabled(mode_disabled)
                        .label(rust_i18n::t!("app.shell.confirm_deep").into_owned())
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_mode(cx))),
                )
                .child(
                    Button::new("cancel-deep")
                        .ghost()
                        .label(rust_i18n::t!("app.shell.dismiss").into_owned())
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.confirm_deep = false;
                            cx.notify();
                        })),
                );
        }
        row
    }
}
