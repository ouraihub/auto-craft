//! Project composer and inbox answers; dismissing a card sends a real refusal/cancellation.
use super::ProjectView;
use crate::state::run;
use autocraft_core::{FeedPlanStatus, InboxAnswer, InboxKind, InboxRequest};
use autocraft_ui::{
    AutocraftTheme,
    components::{ChoiceMenu, label},
    metrics::{space, text},
};
use gpui_kit::{
    AnyElement, AppContext as _, Context, IntoElement, ParentElement as _, Styled as _, Window,
    assets::IconName,
    base::{h_flex, v_flex},
    component::{
        Disableable as _, WindowExt as _,
        button::{Button, ButtonVariants as _},
        dialog::{DialogAction, DialogClose, DialogFooter},
        input::{Input, InputState, Textarea},
        menu::{DropdownMenu as _, PopupMenuItem},
    },
};
impl ProjectView {
    pub(super) fn composer(&self, cx: &mut Context<Self>) -> AnyElement {
        let owner = cx.entity();
        let r = AutocraftTheme::global(cx).roles;
        let mut composer = v_flex()
            .p(space::S12)
            .gap(space::S8)
            .border_t_1()
            .border_color(r.border_default);
        if let Some(status) = &self.status {
            for (id, content) in &status.waiting_notes {
                let id = id.clone();
                composer =
                    composer.child(
                        h_flex()
                            .gap(space::S8)
                            .child(label(text::METADATA, content.clone()))
                            .child(
                                Button::new(format!("withdraw-{id}"))
                                    .outline()
                                    .label(rust_i18n::t!("app.project.withdraw").to_string())
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.withdraw(id.clone(), cx)
                                    })),
                            ),
                    );
            }
        }
        composer = composer
            .child(
                h_flex().child(
                    ChoiceMenu::new(
                        "project-input-mode",
                        [
                            rust_i18n::t!("app.project.modify").to_string(),
                            rust_i18n::t!("app.project.just_ask").to_string(),
                        ],
                        Some(usize::from(self.just_ask)),
                    )
                    .on_select(cx.listener(|this, index: &usize, _, cx| {
                        this.just_ask = *index == 1;
                        cx.notify();
                    })),
                ),
            )
            .child(
                Textarea::new(&self.input)
                    .on_paste(move |item, _, cx| owner.update(cx, |this, cx| this.paste(item, cx))),
            )
            .child(h_flex().flex_wrap().gap(space::S8).children(
                self.pictures.iter().enumerate().map(|(index, path)| {
                    v_flex()
                        .gap(space::S4)
                        .child(
                            gpui_kit::img(path.clone())
                                .w(gpui_kit::px(80.))
                                .h(gpui_kit::px(64.)),
                        )
                        .child(
                            Button::new(format!("remove-picture-{index}"))
                                .outline()
                                .label(rust_i18n::t!("app.project.remove").to_string())
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if index < this.pictures.len() {
                                        this.pictures.remove(index);
                                        cx.notify();
                                    }
                                })),
                        )
                }),
            ))
            .child(
                label(
                    text::METADATA,
                    rust_i18n::t!("app.project.command_help").to_string(),
                )
                .text_color(r.text_subtle),
            );
        if self.input.read(cx).value().trim_start().starts_with('/') {
            let owner = cx.entity();
            composer = composer.child(
                h_flex().child(
                    Button::new("slash-commands")
                        .outline()
                        .label(rust_i18n::t!("app.project.insert_command").to_string())
                        .icon(IconName::ChevronDown)
                        .dropdown_menu(move |menu, _, _| {
                            ["/approvals ", "/btw ", "/stop", "/continue", "/done"]
                                .into_iter()
                                .fold(menu, |menu, command| {
                                    let owner = owner.clone();
                                    menu.item(PopupMenuItem::new(command.trim_end()).on_click(
                                        move |_, _, cx| {
                                            owner.update(cx, |this, cx| {
                                                this.input_restore = Some(command.into());
                                                cx.notify();
                                            })
                                        },
                                    ))
                                })
                        }),
                ),
            );
        }
        if self.state.read(cx).services.rebuild.is_some() {
            composer = composer.child(
                h_flex()
                    .gap(space::S8)
                    .child(
                        Button::new("project-attach")
                            .outline()
                            .label(rust_i18n::t!("app.project.attach").to_string())
                            .disabled(self.busy || self.attaching || !self.writable())
                            .on_click(cx.listener(|this, _, _, cx| this.choose_pictures(cx))),
                    )
                    .child(
                        Button::new("project-send")
                            .primary()
                            .label(rust_i18n::t!("app.project.send").to_string())
                            .disabled(self.busy || self.attaching || !self.writable())
                            .on_click(cx.listener(|this, _, _, cx| {
                                let text = this.input.read(cx).value().to_string();
                                this.command(&text, cx);
                            })),
                    )
                    .child(
                        Button::new("project-stop")
                            .outline()
                            .label(rust_i18n::t!("app.project.stop").to_string())
                            .disabled((!self.running() && !self.busy) || !self.writable())
                            .on_click(cx.listener(|this, _, _, cx| this.command("/stop", cx))),
                    ),
            );
        }
        composer.into_any_element()
    }
    pub(super) fn plan_bar(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut plan = v_flex().gap(space::S4).px(space::S12);
        if !self.plan.is_empty() {
            plan = plan.child(
                Button::new("project-plan-fold")
                    .outline()
                    .label(rust_i18n::t!("app.project.plan").to_string())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.plan_folded = !this.plan_folded;
                        cx.notify();
                    })),
            );
            if !self.plan_folded {
                plan = plan.children(self.plan.iter().map(|step| {
                    label(
                        text::METADATA,
                        format!(
                            "{} {}",
                            match step.status {
                                FeedPlanStatus::Todo => "○",
                                FeedPlanStatus::Doing => "◉",
                                FeedPlanStatus::Done => "✓",
                                FeedPlanStatus::Stopped => "■",
                            },
                            if step.status == FeedPlanStatus::Doing {
                                &step.doing
                            } else {
                                &step.text
                            }
                        ),
                    )
                }));
            }
        }
        plan.into_any_element()
    }
    pub(super) fn cancel_request(&mut self, request: InboxRequest, cx: &mut Context<Self>) {
        let answer = match request.kind {
            InboxKind::Approval => InboxAnswer::Approval {
                allow: false,
                always: false,
                text: None,
            },
            InboxKind::Question => InboxAnswer::Choice {
                choice: None,
                text: None,
            },
        };
        self.answer(request, answer, cx);
    }
    pub(super) fn answer(
        &mut self,
        request: InboxRequest,
        answer: InboxAnswer,
        cx: &mut Context<Self>,
    ) {
        if !self.writable()
            || !self.inbox.iter().any(|r| r.id == request.id)
            || self.answering.contains(&request.id)
        {
            return;
        }
        let Some(project) = self.project.clone() else {
            return;
        };
        let service = self.state.read(cx).services.inbox.clone();
        let generation = self.generation;
        let id = request.id.clone();
        self.answering.push(id.clone());
        run(
            cx,
            move || service.answer(&project, &request.id, answer),
            move |this, result, cx| {
                if this.generation != generation {
                    return;
                }
                this.answering.retain(|pending| pending != &id);
                match result {
                    Ok(()) => {
                        this.inbox.retain(|r| r.id != id);
                        this.reload(cx);
                    }
                    Err(error) => this.report(&error, cx),
                }
                cx.notify();
            },
        );
    }
    pub(super) fn answer_dialog(
        &mut self,
        request: InboxRequest,
        refusal: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = cx.new(|cx| InputState::new(window, cx));
        let view = cx.entity();
        let cancel_view = view.clone();
        let cancel_request = request.clone();
        window.open_dialog(cx, move |dialog, _, _| {
            let input = input.clone();
            let view = view.clone();
            let request = request.clone();
            let cancel_view = cancel_view.clone();
            let cancel_request = cancel_request.clone();
            let footer = DialogFooter::new()
                .child(DialogClose::new().trigger(|button| {
                    button.label(rust_i18n::t!("app.project.cancel").to_string())
                }))
                .child(
                    DialogAction::new().child(
                        Button::new("project-answer-save")
                            .primary()
                            .label(rust_i18n::t!("app.project.save").to_string()),
                    ),
                );
            dialog
                .title(request.title.clone())
                .child(Input::new(&input))
                .footer(footer)
                .on_ok(move |_, _, cx| {
                    let text = input.read(cx).value().trim().to_owned();
                    if text.is_empty() && !refusal {
                        return false;
                    }
                    view.update(cx, |this, cx| {
                        this.answer(
                            request.clone(),
                            if refusal {
                                InboxAnswer::Approval {
                                    allow: false,
                                    always: false,
                                    text: (!text.is_empty()).then_some(text),
                                }
                            } else {
                                InboxAnswer::Choice {
                                    choice: None,
                                    text: Some(text),
                                }
                            },
                            cx,
                        )
                    });
                    true
                })
                .on_cancel(move |_, _, cx| {
                    cancel_view.update(cx, |this, cx| {
                        this.cancel_request(cancel_request.clone(), cx)
                    });
                    true
                })
        });
    }
}
