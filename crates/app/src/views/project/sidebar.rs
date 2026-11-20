//! Right sidebar presents authoritative project policy, progress and run controls.
mod policies;

use super::ProjectView;
use autocraft_core::{AnalysisMode, ProjectPrefChange, ProjectStatus, StepDto};
use autocraft_ui::{
    AutocraftTheme,
    components::{ChoiceMenu, StepState, StepTracker, label},
    metrics::{space, text},
};
use gpui_kit::{
    AnyElement, Context, IntoElement, ParentElement as _, Styled as _,
    base::{h_flex, v_flex},
    component::{Disableable as _, button::Button, scroll::ScrollableElement as _},
};
impl ProjectView {
    pub(super) fn sidebar(&self, cx: &mut Context<Self>) -> AnyElement {
        let r = AutocraftTheme::global(cx).roles;
        let mut bar = v_flex()
            .w(gpui_kit::px(300.))
            .h_full()
            .overflow_y_scrollbar()
            .p(space::S12)
            .gap(space::S16)
            .border_l_1()
            .border_color(r.border_default);
        let Some(detail) = &self.detail else {
            return bar.into_any_element();
        };
        let disabled = self.busy || !self.writable();
        let current = self.status.as_ref().and_then(|s| s.step);
        let position = current.map(step_index);
        let states = std::array::from_fn(|index| match position {
            Some(current) if index < current => StepState::Complete,
            Some(current) if index == current => StepState::Active,
            Some(_) | None => StepState::Pending,
        });
        bar = bar
            .child(label(text::TITLE_CARD, detail.summary.name.clone()))
            .child(label(
                text::METADATA,
                rust_i18n::t!(status_key(
                    self.status
                        .as_ref()
                        .map_or(detail.summary.status, |s| s.status)
                ))
                .to_string(),
            ))
            .child(label(
                text::UI,
                self.status
                    .as_ref()
                    .map_or_else(|| detail.summary.note.clone(), |s| s.note.clone()),
            ))
            .child(
                Button::new("project-rename")
                    .outline()
                    .label(rust_i18n::t!("app.project.rename").to_string())
                    .disabled(disabled)
                    .on_click(
                        cx.listener(|this, _, window, cx| this.name_dialog(false, window, cx)),
                    ),
            )
            .child(StepTracker::new(
                "project-steps",
                current,
                states,
                [
                    StepDto::Study,
                    StepDto::Build,
                    StepDto::Check,
                    StepDto::Done,
                ]
                .map(|step| rust_i18n::t!(step_key(step)).into()),
            ));
        if self.state.read(cx).services.rebuild.is_some() {
            bar = bar
                .child(
                    h_flex()
                        .gap(space::S8)
                        .child(
                            Button::new("project-preview")
                                .outline()
                                .label(rust_i18n::t!("app.project.preview").to_string())
                                .on_click(cx.listener(|this, _, _, cx| this.preview(false, cx))),
                        )
                        .child(
                            Button::new("project-play")
                                .outline()
                                .label(rust_i18n::t!("app.project.run").to_string())
                                .disabled(disabled)
                                .on_click(cx.listener(|this, _, _, cx| this.preview(true, cx))),
                        ),
                )
                .child(
                    Button::new("project-pins")
                        .outline()
                        .label(crate::i18n::text("app.pins.title"))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.state.update(cx, |state, cx| {
                                state.navigate(crate::state::MainView::Pins, cx)
                            });
                        })),
                )
                .child(
                    Button::new("project-continue")
                        .outline()
                        .label(rust_i18n::t!("app.project.continue").to_string())
                        .disabled(disabled || self.running())
                        .on_click(cx.listener(|this, _, _, cx| this.command("/continue", cx))),
                )
                .child(
                    Button::new("project-done")
                        .outline()
                        .label(rust_i18n::t!("app.project.done").to_string())
                        .disabled(disabled)
                        .on_click(cx.listener(|this, _, _, cx| this.command("/done", cx))),
                );
        }
        bar = bar.child(label(
            text::EMPHASIS,
            rust_i18n::t!("app.project.builder").to_string(),
        ));
        if let Some(prefs) = &self.prefs {
            bar = bar
                .child(self.backend_menu(disabled, cx))
                .child(
                    Button::new("project-default-model")
                        .outline()
                        .label(rust_i18n::t!("app.project.follow_default").to_string())
                        .disabled(disabled)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.set_pref(ProjectPrefChange::Model(None), cx)
                        })),
                )
                .child(self.picker.clone())
                .child(
                    ChoiceMenu::new(
                        "project-analysis-mode",
                        [
                            rust_i18n::t!("app.project.clean_room").to_string(),
                            rust_i18n::t!("app.project.deep").to_string(),
                        ],
                        Some(usize::from(prefs.mode == AnalysisMode::Deep)),
                    )
                    .full_width(true)
                    .disabled(disabled)
                    .on_select(cx.listener(|this, index: &usize, _, cx| {
                        this.set_mode(
                            if *index == 0 {
                                AnalysisMode::CleanRoom
                            } else {
                                AnalysisMode::Deep
                            },
                            cx,
                        )
                    })),
                );
        }
        if self.running() {
            bar = bar.child(
                label(
                    text::METADATA,
                    rust_i18n::t!("app.project.next_run").to_string(),
                )
                .text_color(r.text_subtle),
            );
        }
        bar = bar.child(self.policy_controls(detail, disabled, cx));
        let checks = self
            .status
            .as_ref()
            .map_or(detail.latest_checks.as_slice(), |s| {
                s.latest_checks.as_slice()
            });
        bar = bar
            .child(label(
                text::EMPHASIS,
                rust_i18n::t!("app.project.checks").to_string(),
            ))
            .children(checks.iter().map(|check| {
                label(
                    text::METADATA,
                    format!(
                        "{} {} · {}",
                        if check.passed { "✓" } else { "✗" },
                        check.check,
                        check.note
                    ),
                )
            }));
        bar = bar
            .child(
                Button::new("project-self-check")
                    .outline()
                    .label(rust_i18n::t!("app.project.self_check").to_string())
                    .disabled(disabled)
                    .on_click(cx.listener(|this, _, _, cx| this.self_check(cx))),
            )
            .child(
                Button::new("project-activity")
                    .outline()
                    .label(rust_i18n::t!("app.project.activity").to_string())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.activity_open = !this.activity_open;
                        if this.activity_open {
                            this.load_activity(true, cx);
                        }
                        cx.notify();
                    })),
            );
        if self.activity_open {
            bar = bar
                .children(self.activity.iter().map(|line| {
                    label(
                        text::METADATA,
                        format!("{}s · {}", line.at.as_secs(), line.text),
                    )
                }))
                .child(
                    Button::new("project-more-activity")
                        .outline()
                        .label(rust_i18n::t!("app.project.more").to_string())
                        .on_click(cx.listener(|this, _, _, cx| this.load_activity(false, cx))),
                );
        }
        if self.agent.is_some() {
            bar = bar.child(
                Button::new("project-agent-drawer")
                    .outline()
                    .label(rust_i18n::t!("app.project.agent").to_string())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.agent_open = !this.agent_open;
                        cx.notify();
                    })),
            );
        }
        bar.child(
            Button::new("project-folder")
                .outline()
                .label(rust_i18n::t!("app.project.open_folder").to_string())
                .on_click(cx.listener(|this, _, _, cx| this.open_folder(false, cx))),
        )
        .child(
            Button::new("project-app-folder")
                .outline()
                .label(rust_i18n::t!("app.project.open_app").to_string())
                .on_click(cx.listener(|this, _, _, cx| this.open_folder(true, cx))),
        )
        .child(
            Button::new("project-duplicate")
                .outline()
                .label(rust_i18n::t!("app.project.duplicate").to_string())
                .disabled(self.busy || self.running())
                .on_click(cx.listener(|this, _, window, cx| this.name_dialog(true, window, cx))),
        )
        .child(
            Button::new("project-delete")
                .outline()
                .label(rust_i18n::t!("app.project.delete").to_string())
                .disabled(disabled || self.running())
                .on_click(cx.listener(|this, _, window, cx| this.confirm_delete(window, cx))),
        )
        .into_any_element()
    }
}
fn step_index(step: StepDto) -> usize {
    match step {
        StepDto::Study => 0,
        StepDto::Build => 1,
        StepDto::Check => 2,
        StepDto::Done => 3,
    }
}
pub(super) fn step_key(step: StepDto) -> &'static str {
    match step {
        StepDto::Study => "app.project.study",
        StepDto::Build => "app.project.build",
        StepDto::Check => "app.project.check",
        StepDto::Done => "app.project.ready",
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
