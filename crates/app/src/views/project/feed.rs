//! Stateful feed rows preserve markdown selection, tool folding and measured heights.
use super::ProjectView;
use autocraft_core::{FeedEntryDto, InboxAnswer};
use autocraft_ui::{
    AutocraftTheme,
    components::{CheckCard, CheckCardLabels, CheckThumbnail, label, markdown},
    metrics::{radius, space, text},
};
use gpui_kit::{
    AnyElement, Context, IntoElement, ParentElement as _, Render, Styled as _, WeakEntity, Window,
    base::{h_flex, v_flex},
    component::{Disableable as _, button::Button},
    prelude::FluentBuilder as _,
};

pub(super) struct FeedRow {
    entry: FeedEntryDto,
    owner: WeakEntity<ProjectView>,
    index: usize,
    generation: u64,
    folded: bool,
    analysis_source: bool,
}
impl FeedRow {
    pub(super) fn new(
        entry: FeedEntryDto,
        owner: WeakEntity<ProjectView>,
        index: usize,
        generation: u64,
    ) -> Self {
        Self {
            entry,
            owner,
            index,
            generation,
            folded: true,
            analysis_source: false,
        }
    }
    pub(super) fn set_entry(&mut self, entry: FeedEntryDto, cx: &mut Context<Self>) {
        self.analysis_source = false;
        if self.entry != entry {
            self.entry = entry;
            cx.notify();
        }
    }
    pub(super) fn set_analysis_source(&mut self) {
        self.analysis_source = true;
    }
    pub(super) fn entry(&self) -> &FeedEntryDto {
        &self.entry
    }
    #[cfg(test)]
    pub(super) fn analysis_source(&self) -> bool {
        self.analysis_source
    }
    #[cfg(test)]
    pub(super) fn folded(&self) -> bool {
        self.folded
    }
}
impl Render for FeedRow {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let r = AutocraftTheme::global(cx).roles;
        let body: AnyElement = match &self.entry {
            FeedEntryDto::User {
                text: content,
                pictures,
            } => v_flex()
                .p(space::S12)
                .gap(space::S8)
                .rounded(radius::LG)
                .bg(r.control_bg_selected)
                .child(label(
                    text::EMPHASIS,
                    rust_i18n::t!("app.project.user").to_string(),
                ))
                .child(markdown("user-markdown", content.clone()))
                .child(
                    h_flex()
                        .flex_wrap()
                        .gap(space::S8)
                        .children(pictures.iter().map(|path| image(path.clone()))),
                )
                .into_any_element(),
            FeedEntryDto::Agent {
                text: content,
                streaming,
            } => v_flex()
                .gap(space::S4)
                .child(markdown("agent-markdown", content.clone()))
                .when(*streaming, |d| d.child(label(text::METADATA, "▍")))
                .into_any_element(),
            FeedEntryDto::Question {
                question,
                answer,
                backend,
                ..
            } => v_flex()
                .gap(space::S8)
                .p(space::S12)
                .child(label(
                    text::EMPHASIS,
                    rust_i18n::t!("app.project.just_ask").to_string(),
                ))
                .child(markdown("question-markdown", question.clone()))
                .child(markdown("answer-markdown", answer.clone()))
                .child(label(text::METADATA, backend.clone()).text_color(r.text_subtle))
                .into_any_element(),
            FeedEntryDto::Work { tools, notes } => {
                let mut group = v_flex().gap(space::S4).child(
                    Button::new("tools-fold")
                        .outline()
                        .label(rust_i18n::t!("app.project.tools", count = tools.len()).to_string())
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.folded = !this.folded;
                            cx.notify();
                        })),
                );
                if !self.folded {
                    group = group.children(tools.iter().map(|tool| {
                        label(
                            text::UI,
                            format!(
                                "{} {}",
                                match tool.ok {
                                    None => "◌",
                                    Some(true) => "✓",
                                    Some(false) => "✗",
                                },
                                tool.name
                            ),
                        )
                    }));
                    group =
                        group.children(notes.iter().enumerate().map(|(index, note)| {
                            markdown(format!("tool-note-{index}"), note.clone())
                        }));
                }
                group.into_any_element()
            }
            FeedEntryDto::Ask { request, answer } => {
                let owner = self.owner.upgrade();
                let pending = owner
                    .as_ref()
                    .is_some_and(|view| view.read(cx).inbox.iter().any(|r| r.id == request.id));
                let mut card = v_flex()
                    .gap(space::S8)
                    .p(space::S12)
                    .border_1()
                    .border_color(r.feedback_warning)
                    .rounded(radius::SM)
                    .child(label(
                        text::METADATA,
                        rust_i18n::t!(if self.analysis_source {
                            "app.project.source_analysis"
                        } else {
                            "app.project.source_builder"
                        })
                        .to_string(),
                    ))
                    .child(label(text::EMPHASIS, request.title.clone()))
                    .child(markdown("request-detail", request.detail.clone()));
                if answer.is_some() || !pending {
                    card = card.child(label(
                        text::METADATA,
                        rust_i18n::t!("app.project.answered").to_string(),
                    ));
                } else if let Some(owner) = owner {
                    match request.kind {
                        autocraft_core::InboxKind::Approval => {
                            for (key, allow, always) in [
                                ("allow_once", true, false),
                                ("always_allow", true, true),
                                ("refuse", false, false),
                            ] {
                                let owner = owner.clone();
                                let request = request.clone();
                                card = card.child(
                                    Button::new(key)
                                        .outline()
                                        .label(
                                            rust_i18n::t!(format!("app.project.{key}")).to_string(),
                                        )
                                        .disabled(!owner.read(cx).writable())
                                        .on_click(move |_, window, cx| {
                                            owner.update(cx, |this, cx| {
                                                if allow {
                                                    this.answer(
                                                        request.clone(),
                                                        InboxAnswer::Approval {
                                                            allow,
                                                            always,
                                                            text: None,
                                                        },
                                                        cx,
                                                    );
                                                } else {
                                                    this.answer_dialog(
                                                        request.clone(),
                                                        true,
                                                        window,
                                                        cx,
                                                    );
                                                }
                                            })
                                        }),
                                );
                            }
                        }
                        autocraft_core::InboxKind::Question => {
                            for (index, choice) in request.choices.iter().take(6).enumerate() {
                                let owner = owner.clone();
                                let request = request.clone();
                                let choice = choice.clone();
                                card = card.child(
                                    Button::new(format!("choice-{index}"))
                                        .outline()
                                        .label(choice.clone())
                                        .disabled(!owner.read(cx).writable())
                                        .on_click(move |_, _, cx| {
                                            owner.update(cx, |this, cx| {
                                                this.answer(
                                                    request.clone(),
                                                    InboxAnswer::Choice {
                                                        choice: Some(choice.clone()),
                                                        text: None,
                                                    },
                                                    cx,
                                                )
                                            })
                                        }),
                                );
                            }
                            if request.allow_text {
                                let owner = owner.clone();
                                let request = request.clone();
                                card = card.child(
                                    Button::new("answer-text")
                                        .outline()
                                        .label(rust_i18n::t!("app.project.answer_text").to_string())
                                        .on_click(move |_, window, cx| {
                                            owner.update(cx, |this, cx| {
                                                this.answer_dialog(
                                                    request.clone(),
                                                    false,
                                                    window,
                                                    cx,
                                                )
                                            })
                                        }),
                                );
                            }
                        }
                    }
                    let request = request.clone();
                    card = card.child(
                        Button::new("cancel-request")
                            .outline()
                            .label(rust_i18n::t!("app.project.cancel").to_string())
                            .on_click(move |_, _, cx| {
                                owner
                                    .update(cx, |this, cx| this.cancel_request(request.clone(), cx))
                            }),
                    );
                }
                card.into_any_element()
            }
            FeedEntryDto::Check { result } => {
                let project_root = self
                    .owner
                    .upgrade()
                    .and_then(|owner| owner.read(cx).project.clone())
                    .map(|project| std::path::PathBuf::from(project.as_str()));
                let thumbnails = result
                    .artifacts
                    .iter()
                    .map(|path| {
                        let resolved = if path.is_absolute() {
                            path.clone()
                        } else {
                            project_root
                                .as_ref()
                                .map_or_else(|| path.clone(), |root| root.join(path))
                        };
                        CheckThumbnail {
                            path: resolved,
                            caption: path
                                .file_name()
                                .map(|n| n.to_string_lossy().into_owned())
                                .unwrap_or_default()
                                .into(),
                        }
                    })
                    .collect();
                CheckCard::new(
                    "check",
                    std::sync::Arc::new(result.clone()),
                    CheckCardLabels {
                        passed: rust_i18n::t!("app.project.passed").into(),
                        failed: rust_i18n::t!("app.project.failed").into(),
                        similarity: rust_i18n::t!("app.project.similarity").into(),
                        target: rust_i18n::t!("app.project.target").into(),
                        unknowns: rust_i18n::t!("app.project.unknowns").into(),
                    },
                )
                .thumbnails(thumbnails)
                .on_screenshot(|thumbnail, window, cx| {
                    use gpui_kit::component::WindowExt as _;
                    let path = thumbnail.path.clone();
                    window.open_dialog(cx, move |dialog, _, _| {
                        dialog.child(gpui_kit::img(path.clone()).w_full().h(gpui_kit::px(600.)))
                    });
                })
                .into_any_element()
            }
            FeedEntryDto::Step {
                step,
                text: content,
            } => label(
                text::EMPHASIS,
                format!(
                    "{} · {}",
                    rust_i18n::t!(super::sidebar::step_key(*step)),
                    content
                ),
            )
            .into_any_element(),
            FeedEntryDto::Error {
                message,
                remediation,
            } => {
                let owner = self.owner.clone();
                v_flex()
                    .gap(space::S8)
                    .p(space::S12)
                    .border_1()
                    .border_color(r.feedback_error)
                    .child(label(text::UI, message.clone()))
                    .children(remediation.iter().map(|hint| label(text::UI, hint.clone())))
                    .child(
                        Button::new("error-logs")
                            .outline()
                            .label(rust_i18n::t!("app.project.logs").to_string())
                            .on_click(move |_, _, cx| {
                                if let Some(owner) = owner.upgrade() {
                                    cx.reveal_path(
                                        owner.read(cx).state.read(cx).services.log.dir(),
                                    );
                                }
                            }),
                    )
                    .into_any_element()
            }
            FeedEntryDto::Paused => {
                let owner = self.owner.clone();
                h_flex()
                    .gap(space::S8)
                    .child(label(
                        text::UI,
                        rust_i18n::t!("app.project.paused").to_string(),
                    ))
                    .child(
                        Button::new("continue-paused")
                            .outline()
                            .label(rust_i18n::t!("app.project.continue").to_string())
                            .on_click(move |_, _, cx| {
                                if let Some(owner) = owner.upgrade() {
                                    owner.update(cx, |this, cx| this.command("/continue", cx));
                                }
                            }),
                    )
                    .into_any_element()
            }
            FeedEntryDto::Plan { .. } => v_flex().into_any_element(),
        };
        let owner = self.owner.clone();
        let index = self.index;
        let generation = self.generation;
        v_flex()
            .relative()
            .w_full()
            .p(space::S12)
            .child(body)
            .child(
                gpui_kit::canvas(
                    move |bounds, _, cx| {
                        let height = bounds.size.height;
                        cx.defer(move |cx| {
                            if let Some(owner) = owner.upgrade() {
                                owner.update(cx, |this, cx| {
                                    if this.generation != generation {
                                        return;
                                    }
                                    if let Some(size) =
                                        std::rc::Rc::make_mut(&mut this.sizes).get_mut(index)
                                        && (size.height - height).abs() > gpui_kit::px(1.)
                                    {
                                        size.height = height;
                                        cx.notify();
                                    }
                                });
                            }
                        });
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full()
                .top_0()
                .left_0(),
            )
    }
}
fn image(path: std::path::PathBuf) -> AnyElement {
    use gpui_kit::{InteractiveElement as _, component::WindowExt as _};
    let open = path.clone();
    gpui_kit::div()
        .on_mouse_down(gpui_kit::MouseButton::Left, move |_, window, cx| {
            let open = open.clone();
            window.open_dialog(cx, move |dialog, _, _| {
                dialog.child(gpui_kit::img(open.clone()).w_full().h(gpui_kit::px(600.)))
            });
        })
        .child(
            gpui_kit::img(path)
                .w(gpui_kit::px(128.))
                .h(gpui_kit::px(96.)),
        )
        .into_any_element()
}
