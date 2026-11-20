//! Compact project policy menus display explicit overrides separately from effective defaults.
use super::ProjectView;
use autocraft_core::{
    ActionKindDto, ApprovalDto, BackendKind, PersonChange, ProjectDetail, ProjectPrefChange,
    QuestionsDto,
};
use autocraft_ui::{
    components::label,
    metrics::{space, text},
};
use gpui_kit::{
    Context, IntoElement, ParentElement as _, Styled as _,
    assets::IconName,
    base::{h_flex, v_flex},
    component::{
        Disableable as _,
        button::Button,
        input::Input,
        menu::{DropdownMenu as _, PopupMenuItem},
        switch::Switch,
    },
};

impl ProjectView {
    pub(super) fn backend_menu(
        &self,
        disabled: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let current = self.prefs.as_ref().and_then(|prefs| prefs.backend);
        let entity = cx.entity();
        let title = current.map_or_else(
            || rust_i18n::t!("app.shell.follow_default").to_string(),
            |backend| backend.label().to_string(),
        );
        Button::new("project-backend")
            .outline()
            .label(title.clone())
            .accessibility_label(format!("{}: {title}", rust_i18n::t!("app.home.builder")))
            .icon(IconName::ChevronDown)
            .disabled(disabled)
            .dropdown_menu(move |menu, _, _| {
                std::iter::once(None)
                    .chain(BackendKind::ALL.into_iter().map(Some))
                    .fold(menu, |menu, value| {
                        let entity = entity.clone();
                        menu.item(
                            PopupMenuItem::new(value.map_or_else(
                                || rust_i18n::t!("app.shell.follow_default").to_string(),
                                |backend| backend.label().to_string(),
                            ))
                            .checked(current == value)
                            .on_click(move |_, _, cx| {
                                entity.update(cx, |this, cx| {
                                    this.set_pref(ProjectPrefChange::Backend(value), cx)
                                })
                            }),
                        )
                    })
            })
    }

    pub(super) fn policy_controls(
        &self,
        detail: &ProjectDetail,
        disabled: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let mut policies = v_flex().gap(space::S8).child(label(
            text::EMPHASIS,
            rust_i18n::t!("app.project.approvals").to_string(),
        ));
        for kind in [
            ActionKindDto::Web,
            ActionKindDto::Commands,
            ActionKindDto::OutsideFiles,
            ActionKindDto::Publish,
            ActionKindDto::DeleteOutside,
            ActionKindDto::System,
            ActionKindDto::OtherTools,
        ] {
            let effective = detail
                .approvals
                .iter()
                .find(|(value, _)| *value == kind)
                .map(|(_, approval)| *approval);
            let current = if detail.approvals_chosen.contains(&kind) {
                effective
            } else {
                None
            };
            let entity = cx.entity();
            let category = rust_i18n::t!(action_key(kind)).to_string();
            let title = rust_i18n::t!(approval_key(current)).to_string();
            let mut button = Button::new(format!("approval-{}", action_key(kind)))
                .outline()
                .label(title.clone())
                .accessibility_label(format!("{category}: {title}"))
                .icon(IconName::ChevronDown)
                .disabled(disabled);
            if current.is_none()
                && let Some(effective) = effective
            {
                button = button.tooltip(format!(
                    "{}: {}",
                    rust_i18n::t!("app.project.effective"),
                    rust_i18n::t!(approval_key(Some(effective)))
                ));
            }
            policies = policies.child(
                h_flex()
                    .gap(space::S8)
                    .child(label(text::METADATA, category).flex_1())
                    .child(button.dropdown_menu(move |menu, _, _| {
                        [
                            None,
                            Some(ApprovalDto::Allow),
                            Some(ApprovalDto::Ask),
                            Some(ApprovalDto::Refuse),
                        ]
                        .into_iter()
                        .fold(menu, |menu, approval| {
                            let entity = entity.clone();
                            menu.item(
                                PopupMenuItem::new(
                                    rust_i18n::t!(approval_key(approval)).to_string(),
                                )
                                .checked(current == approval)
                                .on_click(move |_, _, cx| {
                                    entity.update(cx, |this, cx| {
                                        this.person(PersonChange::Approval { kind, approval }, cx)
                                    })
                                }),
                            )
                        })
                    })),
            );
        }
        policies
            .child(label(
                text::EMPHASIS,
                rust_i18n::t!("app.project.questions").to_string(),
            ))
            .child(self.questions_menu(detail.questions, disabled, cx))
            .child(
                Switch::new("project-assets")
                    .checked(detail.generate_assets)
                    .label(rust_i18n::t!("app.home.assets_on").to_string())
                    .accessibility_label(rust_i18n::t!("app.home.assets_on").to_string())
                    .disabled(disabled)
                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                        this.person(PersonChange::GenerateAssets(*checked), cx)
                    })),
            )
            .child(label(
                text::METADATA,
                rust_i18n::t!("app.project.budget").to_string(),
            ))
            .child(Input::new(&self.budget))
            .child(
                Button::new("project-save-budget")
                    .outline()
                    .label(rust_i18n::t!("app.project.save").to_string())
                    .disabled(disabled)
                    .on_click(cx.listener(|this, _, _, cx| this.save_budget(cx))),
            )
            .child(label(
                text::METADATA,
                rust_i18n::t!(
                    "app.project.spent",
                    amount = format!("{:.2}", detail.assets_spent_usd)
                )
                .to_string(),
            ))
            .child(label(
                text::METADATA,
                rust_i18n::t!(
                    "app.project.usage",
                    input = detail.usage.input_tokens,
                    output = detail.usage.output_tokens,
                    amount = format!("{:.2}", detail.usage.cost_usd)
                )
                .to_string(),
            ))
    }

    fn questions_menu(
        &self,
        current: QuestionsDto,
        disabled: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let entity = cx.entity();
        let title = rust_i18n::t!(questions_key(current)).to_string();
        Button::new("project-questions")
            .outline()
            .label(title.clone())
            .accessibility_label(format!(
                "{}: {title}",
                rust_i18n::t!("app.project.questions")
            ))
            .icon(IconName::ChevronDown)
            .disabled(disabled)
            .dropdown_menu(move |menu, _, _| {
                [
                    QuestionsDto::Default,
                    QuestionsDto::Ask,
                    QuestionsDto::Decide,
                ]
                .into_iter()
                .fold(menu, |menu, value| {
                    let entity = entity.clone();
                    menu.item(
                        PopupMenuItem::new(rust_i18n::t!(questions_key(value)).to_string())
                            .checked(current == value)
                            .on_click(move |_, _, cx| {
                                entity.update(cx, |this, cx| {
                                    this.person(PersonChange::Questions(value), cx)
                                })
                            }),
                    )
                })
            })
    }
}

fn approval_key(approval: Option<ApprovalDto>) -> &'static str {
    match approval {
        None => "app.project.default",
        Some(ApprovalDto::Allow) => "app.project.allow",
        Some(ApprovalDto::Ask) => "app.project.ask",
        Some(ApprovalDto::Refuse) => "app.project.refuse",
    }
}

fn questions_key(questions: QuestionsDto) -> &'static str {
    match questions {
        QuestionsDto::Default => "app.project.default",
        QuestionsDto::Ask => "app.project.ask_me",
        QuestionsDto::Decide => "app.project.decide",
    }
}

fn action_key(kind: ActionKindDto) -> &'static str {
    match kind {
        ActionKindDto::Web => "app.project.approval_web",
        ActionKindDto::Commands => "app.project.approval_commands",
        ActionKindDto::OutsideFiles => "app.project.approval_outside_files",
        ActionKindDto::Publish => "app.project.approval_publish",
        ActionKindDto::DeleteOutside => "app.project.approval_delete_outside",
        ActionKindDto::System => "app.project.approval_system",
        ActionKindDto::OtherTools => "app.project.approval_other_tools",
    }
}
