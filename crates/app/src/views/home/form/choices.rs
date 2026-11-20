//! Creation draft menus retain enum choices, unavailable options and install actions.
use super::{HomeView, TargetKind};
use autocraft_core::{BackendKind, Door, ExistingFilesDto};
use autocraft_ui::components::ChoiceMenu;
use ca_engines::game::Builder;
use gpui_kit::{
    Context, IntoElement, Styled as _,
    assets::IconName,
    component::{
        button::Button,
        menu::{DropdownMenu as _, PopupMenuItem},
    },
};

impl HomeView {
    pub(super) fn door_menu(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let doors = [Door::MakeMyOwn, Door::WorkOnMine, Door::LookInside];
        ChoiceMenu::new(
            "home-doors",
            doors.map(|door| {
                rust_i18n::t!(match door {
                    Door::MakeMyOwn => "app.home.make",
                    Door::WorkOnMine => "app.home.edit",
                    Door::LookInside => "app.home.inspect",
                })
                .into_owned()
            }),
            doors.iter().position(|door| *door == self.door),
        )
        .full_width(true)
        .on_select(cx.listener(move |this, index: &usize, _, cx| {
            if let Some(&door) = doors.get(*index) {
                this.set_door(door, cx);
            }
        }))
    }

    pub(super) fn target_kind_menu(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let kinds = [
            TargetKind::Website,
            TargetKind::Application,
            TargetKind::Game,
            TargetKind::None,
        ];
        ChoiceMenu::new(
            "home-target-kind",
            kinds.map(|kind| rust_i18n::t!(target_key(kind)).into_owned()),
            kinds.iter().position(|kind| *kind == self.target_kind),
        )
        .name(rust_i18n::t!("app.home.target_kind").into_owned())
        .full_width(true)
        .on_select(cx.listener(move |this, index: &usize, _, cx| {
            if !this.busy
                && let Some(&kind) = kinds.get(*index)
            {
                this.target_kind = kind;
                cx.notify();
            }
        }))
    }

    pub(super) fn backend_menu(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let installed =
            BackendKind::ALL.map(|backend| (backend, self.installed.contains(&backend)));
        let state = self.state.clone();
        let entity = cx.entity();
        ChoiceMenu::new(
            "home-backend",
            installed.map(|(backend, available)| {
                if available {
                    backend.label().to_string()
                } else {
                    format!(
                        "{} · {}",
                        backend.label(),
                        rust_i18n::t!("app.onboarding.not_installed")
                    )
                }
            }),
            self.backend
                .and_then(|current| BackendKind::ALL.iter().position(|b| *b == current)),
        )
        .name(rust_i18n::t!("app.home.builder").into_owned())
        .placeholder(rust_i18n::t!("app.home.choose").into_owned())
        .full_width(true)
        .on_select(move |index, window, cx| {
            let Some(&(backend, available)) = installed.get(*index) else {
                return;
            };
            // Open the dialog only after the view update has finished.
            let open = entity.update(cx, |this, cx| {
                if this.busy {
                    return false;
                }
                if available {
                    this.backend = Some(backend);
                    this.pending_backend = None;
                    this.model = None;
                    this.effort = None;
                } else {
                    this.pending_backend = Some(backend);
                }
                this.load_choices(cx);
                !available
            });
            if open {
                crate::panels::settings::SettingsDialog::open_coding(
                    state.clone(),
                    backend,
                    window,
                    cx,
                );
            }
        })
    }

    pub(super) fn game_stack_menu(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let current = self.game_stack;
        let available = Builder::ALL.map(|builder| {
            (
                Some(builder.stack()),
                self.game_stacks.contains(&builder.stack()),
            )
        });
        let entity = cx.entity();
        let state = self.state.clone();
        let title = current.map_or_else(
            || rust_i18n::t!("app.home.automatic_stack").to_string(),
            str::to_string,
        );
        Button::new("home-game-stack")
            .outline()
            .max_w_full()
            .label(title.clone())
            .accessibility_label(format!("{}: {title}", rust_i18n::t!("app.home.game_stack")))
            .icon(IconName::ChevronDown)
            .dropdown_menu(move |menu, _, _| {
                std::iter::once((None, true)).chain(available).fold(
                    menu,
                    |menu, (value, available)| {
                        let entity = entity.clone();
                        let state = state.clone();
                        menu.item(
                            PopupMenuItem::new(value.map_or_else(
                                || rust_i18n::t!("app.home.automatic_stack").to_string(),
                                |stack| {
                                    if available {
                                        stack.to_owned()
                                    } else {
                                        format!(
                                            "{stack} · {}",
                                            rust_i18n::t!("app.onboarding.not_installed")
                                        )
                                    }
                                },
                            ))
                            .checked(current == value)
                            .on_click(move |_, window, cx| {
                                let open = entity.update(cx, |this, cx| {
                                    if this.busy {
                                        return false;
                                    }
                                    this.game_stack = value;
                                    cx.notify();
                                    !available
                                });
                                if open && let Some(stack) = value {
                                    crate::panels::settings::SettingsDialog::open_analysis_engine(
                                        state.clone(),
                                        stack,
                                        window,
                                        cx,
                                    );
                                }
                            }),
                        )
                    },
                )
            })
    }

    pub(super) fn existing_files_menu(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let current = self.existing;
        let entity = cx.entity();
        let title = rust_i18n::t!(match current {
            Some(ExistingFilesDto::Edit) => "app.home.existing_edit",
            Some(ExistingFilesDto::StartFrom) => "app.home.existing_start",
            None => "app.home.choose",
        })
        .to_string();
        Button::new("home-existing-files")
            .outline()
            .max_w_full()
            .label(title.clone())
            .accessibility_label(format!(
                "{} {title}",
                rust_i18n::t!("app.home.existing_files")
            ))
            .icon(IconName::ChevronDown)
            .dropdown_menu(move |menu, _, _| {
                [
                    (Some(ExistingFilesDto::Edit), "app.home.existing_edit"),
                    (Some(ExistingFilesDto::StartFrom), "app.home.existing_start"),
                    (None, "app.home.cancel"),
                ]
                .into_iter()
                .fold(menu, |menu, (value, key)| {
                    let entity = entity.clone();
                    menu.item(
                        PopupMenuItem::new(rust_i18n::t!(key).to_string())
                            .checked(value.is_some() && current == value)
                            .on_click(move |_, _, cx| {
                                entity.update(cx, |this, cx| {
                                    if this.busy {
                                        return;
                                    }
                                    this.existing = value;
                                    if value.is_none() {
                                        this.folder = None;
                                        this.folder_has_files = false;
                                        this.folder_generation += 1;
                                    }
                                    cx.notify();
                                })
                            }),
                    )
                })
            })
    }
}

fn target_key(target: TargetKind) -> &'static str {
    match target {
        TargetKind::Website => "app.home.website",
        TargetKind::Application => "app.home.application",
        TargetKind::Game => "app.home.game",
        TargetKind::None => "app.home.no_target",
    }
}
