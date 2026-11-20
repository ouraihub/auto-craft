//! Three creation doors share one draft while exposing only applicable controls.
mod choices;

use super::{HomeView, TargetKind};
use autocraft_core::{AnalysisMode, Door};
use autocraft_ui::{
    AutocraftTheme,
    components::{ChoiceMenu, label},
    metrics::{content, space, text},
};
use gpui_kit::{
    AnyElement, Context, IntoElement, ParentElement as _, Pixels, Styled as _,
    base::{h_flex, v_flex},
    component::{
        Disableable as _,
        button::{Button, ButtonVariants as _},
        input::{Input, Textarea},
        switch::Switch,
    },
    prelude::FluentBuilder as _,
};
impl HomeView {
    pub(super) fn form(&self, width: Pixels, cx: &mut Context<Self>) -> AnyElement {
        let r = AutocraftTheme::global(cx).roles;
        let rebuilding = self.door != Door::LookInside;
        let own = self.door == Door::WorkOnMine;
        let game = self.target_kind == TargetKind::Game && !own;
        let stacked = width < lumakit_design::metrics::size::NARROW;
        let mut form = v_flex()
            .w_full()
            .min_w_0()
            .gap(space::S12)
            .child(
                v_flex()
                    .w(content::FORM_FIELD)
                    .max_w_full()
                    .child(self.door_menu(cx)),
            )
            .child(
                v_flex()
                    .gap(space::S8)
                    .child(label(text::UI, rust_i18n::t!("app.home.name").to_string()))
                    .child(Input::new(&self.name).w(content::FORM_FIELD).max_w_full()),
            );
        if own {
            form = form.child(
                v_flex()
                    .gap(space::S8)
                    .child(label(
                        text::UI,
                        rust_i18n::t!("app.home.source_folder").to_string(),
                    ))
                    .child(
                        h_flex()
                            .gap(space::S8)
                            .min_w_0()
                            .child(Input::new(&self.target).flex_1().min_w_0())
                            .child(
                                Button::new("home-select-target")
                                    .outline()
                                    .flex_shrink_0()
                                    .label(rust_i18n::t!("app.home.choose").to_string())
                                    .disabled(self.busy)
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.choose_path(false, cx)),
                                    ),
                            ),
                    ),
            );
        } else {
            form = form.child(
                h_flex()
                    .gap(space::S12)
                    .items_start()
                    .when(stacked, |row| row.flex_col())
                    .children([
                        v_flex()
                            .gap(space::S8)
                            .flex_none()
                            .when(stacked, |column| column.w_full())
                            .when(!stacked, |column| column.w(content::FORM_SELECT))
                            .child(label(
                                text::UI,
                                rust_i18n::t!("app.home.target_kind").to_string(),
                            ))
                            .child(self.target_kind_menu(cx)),
                        v_flex()
                            .gap(space::S8)
                            .when(stacked, |column| column.w_full())
                            .when(!stacked, |column| column.flex_1())
                            .min_w_0()
                            .child(label(
                                text::UI,
                                rust_i18n::t!("app.home.target").to_string(),
                            ))
                            .child(
                                h_flex()
                                    .gap(space::S8)
                                    .min_w_0()
                                    .child(Input::new(&self.target).flex_1().min_w_0())
                                    .when(self.target_kind != TargetKind::Website, |row| {
                                        row.child(
                                            Button::new("home-select-target")
                                                .outline()
                                                .flex_shrink_0()
                                                .label(rust_i18n::t!("app.home.choose").to_string())
                                                .disabled(self.busy)
                                                .on_click(cx.listener(|this, _, _, cx| {
                                                    this.choose_path(false, cx)
                                                })),
                                        )
                                    }),
                            ),
                    ]),
            );
        }
        if !own {
            form = form
                .child(
                    v_flex()
                        .gap(space::S8)
                        .when(stacked, |column| column.w_full())
                        .when(!stacked, |column| column.w(content::FORM_SELECT))
                        .child(label(
                            text::UI,
                            rust_i18n::t!("app.home.analysis_mode").to_string(),
                        ))
                        .child(
                            ChoiceMenu::new(
                                "home-mode",
                                [
                                    rust_i18n::t!("app.home.clean_room").to_string(),
                                    rust_i18n::t!("app.home.deep").to_string(),
                                ],
                                Some(usize::from(self.mode == AnalysisMode::Deep)),
                            )
                            .full_width(true)
                            .disabled(self.busy)
                            .on_select(cx.listener(
                                |this, index: &usize, _, cx| {
                                    if !this.busy {
                                        this.mode = if *index == 0 {
                                            AnalysisMode::CleanRoom
                                        } else {
                                            AnalysisMode::Deep
                                        };
                                        cx.notify();
                                    }
                                },
                            )),
                        ),
                )
                .child(
                    label(
                        text::METADATA,
                        rust_i18n::t!(if self.mode == AnalysisMode::Deep {
                            "app.home.deep_hint"
                        } else {
                            "app.home.clean_hint"
                        })
                        .to_string(),
                    )
                    .text_color(r.text_subtle),
                );
        }
        if rebuilding {
            let view = cx.entity();
            form = form
                .child(label(
                    text::UI,
                    rust_i18n::t!("app.home.request").to_string(),
                ))
                .child(Textarea::new(&self.request).disabled(self.busy).on_paste(
                    move |clipboard, _, cx| {
                        view.update(cx, |this, cx| this.paste_references(clipboard, cx))
                    },
                ))
                .child(
                    label(
                        text::METADATA,
                        rust_i18n::t!("app.home.reference_count", count = self.references.len())
                            .to_string(),
                    )
                    .text_color(r.text_subtle),
                )
                .when(!self.references.is_empty(), |form| {
                    form.child(
                        Button::new("home-clear-references")
                            .outline()
                            .label(rust_i18n::t!("app.home.clear_references").to_string())
                            .disabled(self.busy)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.references.clear();
                                cx.notify();
                            })),
                    )
                });
            let default_name = self
                .prefs
                .default_model
                .as_ref()
                .and_then(|model| {
                    self.catalog.iter().find(|choice| {
                        choice.provider == model.provider && choice.model.id == model.model_id
                    })
                })
                .map(|choice| choice.model.name.as_str())
                .unwrap_or("—");
            form = form.child(
                h_flex()
                    .gap(space::S12)
                    .items_start()
                    .when(stacked, |row| row.flex_col())
                    .children([
                        v_flex()
                            .gap(space::S8)
                            .when(stacked, |column| column.w_full())
                            .when(!stacked, |column| column.w(content::FORM_BUILDER))
                            .flex_none()
                            .child(label(
                                text::UI,
                                rust_i18n::t!("app.home.builder").to_string(),
                            ))
                            .child(self.backend_menu(cx)),
                        v_flex()
                            .gap(space::S8)
                            .when(stacked, |column| column.w_full())
                            .when(!stacked, |column| column.flex_1())
                            .min_w_0()
                            .child(label(text::UI, rust_i18n::t!("app.home.model").to_string()))
                            .child(
                                h_flex()
                                    .w_full()
                                    .min_w_0()
                                    .gap(space::S8)
                                    .flex_wrap()
                                    .child(
                                        Button::new("home-follow-default")
                                            .outline()
                                            .max_w_full()
                                            .label(
                                                rust_i18n::t!(
                                                    "app.home.follow_default",
                                                    name = default_name
                                                )
                                                .to_string(),
                                            )
                                            .disabled(self.busy)
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.model = None;
                                                this.effort = None;
                                                this.load_choices(cx);
                                            })),
                                    )
                                    .child(
                                        v_flex()
                                            .min_w_0()
                                            .max_w_full()
                                            .overflow_hidden()
                                            .child(self.picker.clone()),
                                    ),
                            ),
                    ]),
            );
        }
        if game && rebuilding {
            form = form
                .child(label(
                    text::UI,
                    rust_i18n::t!("app.home.game_stack").to_string(),
                ))
                .child(self.game_stack_menu(cx))
                .child(
                    label(
                        text::METADATA,
                        rust_i18n::t!("app.home.stack_unavailable").to_string(),
                    )
                    .text_color(r.text_subtle),
                )
                .child(
                    Switch::new("home-assets")
                        .checked(self.generate_assets)
                        .label(rust_i18n::t!("app.home.assets_on").to_string())
                        .accessibility_label(rust_i18n::t!("app.home.assets_on").to_string())
                        .disabled(self.busy)
                        .on_click(cx.listener(|this, checked: &bool, _, cx| {
                            if !this.busy {
                                this.generate_assets = *checked;
                                cx.notify();
                            }
                        })),
                )
                .when(self.generate_assets, |form| {
                    form.child(label(
                        text::UI,
                        rust_i18n::t!("app.home.budget").to_string(),
                    ))
                    .child(Input::new(&self.budget))
                });
        }
        if !own {
            let path = self.folder.as_ref().map_or_else(
                || rust_i18n::t!("app.home.default_folder").to_string(),
                |p| p.display().to_string(),
            );
            form = form
                .child(label(
                    text::UI,
                    rust_i18n::t!("app.home.save_folder").to_string(),
                ))
                .child(
                    h_flex()
                        .gap(space::S8)
                        .min_w_0()
                        .when(stacked, |row| row.flex_col().items_start())
                        .child(
                            label(text::METADATA, path)
                                .min_w_0()
                                .max_w_full()
                                .truncate(),
                        )
                        .child(
                            Button::new("home-save-folder")
                                .outline()
                                .flex_shrink_0()
                                .label(rust_i18n::t!("app.home.change").to_string())
                                .disabled(self.busy)
                                .on_click(cx.listener(|this, _, _, cx| this.choose_path(true, cx))),
                        ),
                );
            if self.folder_has_files {
                form = form
                    .child(label(
                        text::UI,
                        rust_i18n::t!("app.home.existing_files").to_string(),
                    ))
                    .child(self.existing_files_menu(cx));
            }
        }
        if !rebuilding || self.state.read(cx).services.rebuild.is_some() {
            form = form.child(
                h_flex()
                    .justify_end()
                    .gap(space::S12)
                    .when(stacked, |row| row.flex_col().items_end())
                    .min_w_0()
                    .children(self.error.as_ref().map(|error| {
                        label(text::UI, error.clone())
                            .min_w_0()
                            .max_w_full()
                            .text_color(r.feedback_error)
                    }))
                    .child(
                        Button::new("home-start")
                            .primary()
                            .w(gpui_kit::px(168.))
                            .flex_shrink_0()
                            .label(
                                rust_i18n::t!(if self.busy {
                                    "app.home.starting"
                                } else {
                                    "app.home.start"
                                })
                                .to_string(),
                            )
                            .disabled(
                                self.busy
                                    || self.loading
                                    || self.folder_loading
                                    || (rebuilding && self.backend.is_none())
                                    || (!own && self.folder_has_files && self.existing.is_none()),
                            )
                            .on_click(cx.listener(|this, _, _, cx| this.create(cx))),
                    ),
            );
        } else {
            form = form.child(
                label(
                    text::METADATA,
                    rust_i18n::t!("app.home.rebuild_unavailable").to_string(),
                )
                .text_color(r.text_subtle),
            );
        }
        form.into_any_element()
    }
}
