//! 自定义字体 and 自定义主题: each folder with its files and what is wrong with them.

use autocraft_core::FontFamily;
use autocraft_ui::{
    AutocraftTheme, Roles,
    components::label,
    metrics::{space, text},
    theme::UserThemeIssueKind,
};
use gpui_kit::{
    AnyElement, Context, IntoElement, ParentElement as _, PathPromptOptions, Styled as _,
    assets::IconName,
    base::{h_flex, v_flex},
    component::{Sizable as _, button::Button},
    prelude::FluentBuilder as _,
};

use super::{
    super::rows::{group, notice},
    AppearancePage,
};
use crate::appearance::{AppearanceState, FileProblem, FontProblem, family_name};

fn font_problem(problem: &FontProblem) -> String {
    match problem {
        FontProblem::File(FileProblem::TooLarge) => {
            crate::i18n::text("app.settings.appearance.font_too_large").into_owned()
        }
        FontProblem::File(FileProblem::TooMany) => {
            crate::i18n::text("app.settings.appearance.font_too_many").into_owned()
        }
        FontProblem::File(FileProblem::Unreadable(detail)) => rust_i18n::t!(
            "app.settings.appearance.problem_detail",
            message = crate::i18n::text("app.settings.appearance.font_unreadable"),
            detail = detail
        )
        .into_owned(),
        FontProblem::File(FileProblem::Collection) => {
            crate::i18n::text("app.settings.appearance.font_collection").into_owned()
        }
        FontProblem::File(FileProblem::Unsupported) => {
            crate::i18n::text("app.settings.appearance.font_unsupported").into_owned()
        }
        FontProblem::Invalid(detail) => rust_i18n::t!(
            "app.settings.appearance.problem_detail",
            message = crate::i18n::text("app.settings.appearance.font_invalid"),
            detail = detail
        )
        .into_owned(),
    }
}

fn theme_problem(kind: UserThemeIssueKind) -> std::borrow::Cow<'static, str> {
    match kind {
        UserThemeIssueKind::TooLarge => {
            crate::i18n::text("app.settings.appearance.theme_too_large")
        }
        UserThemeIssueKind::TooMany => crate::i18n::text("app.settings.appearance.theme_too_many"),
        UserThemeIssueKind::Unreadable => {
            crate::i18n::text("app.settings.appearance.theme_unreadable")
        }
        UserThemeIssueKind::InvalidJson => {
            crate::i18n::text("app.settings.appearance.theme_invalid_json")
        }
        UserThemeIssueKind::InvalidTheme => {
            crate::i18n::text("app.settings.appearance.theme_invalid")
        }
        UserThemeIssueKind::IdTooLong => {
            crate::i18n::text("app.settings.appearance.theme_id_too_long")
        }
        UserThemeIssueKind::DuplicateId => {
            crate::i18n::text("app.settings.appearance.theme_duplicate_id")
        }
    }
}

/// One file line: its name, then what it holds or what is wrong.
fn file_line(name: &str, detail: AnyElement, r: &Roles) -> AnyElement {
    h_flex()
        .gap(space::S12)
        .py(space::S8)
        .border_b_1()
        .border_color(r.border_muted)
        .child(label(text::CODE_SMALL, name.to_owned()).flex_none())
        .child(detail)
        .into_any_element()
}

impl AppearancePage {
    pub(super) fn custom_fonts(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let r = AutocraftTheme::global(cx).roles;
        let state = cx.global::<AppearanceState>();
        let prefs = &state.prefs;
        let in_use = |family: &str| {
            [&prefs.interface_font, &prefs.code_font]
                .iter()
                .any(|choice| {
                    choice
                        .family
                        .iter()
                        .chain(choice.fallbacks.iter())
                        .any(|f| {
                            matches!(f, FontFamily::Name(_))
                                && family_name(f).eq_ignore_ascii_case(family)
                        })
                })
        };
        let files: Vec<AnyElement> = state
            .fonts
            .iter()
            .map(|font| {
                let detail = match &font.face {
                    Ok(face) => h_flex()
                        .gap(space::S8)
                        .child(label(text::UI, face.family.clone()))
                        .when(face.monospace, |d| {
                            d.child(
                                label(
                                    text::CAPTION,
                                    crate::i18n::text("app.settings.appearance.picker_mono"),
                                )
                                .text_color(r.text_subtle),
                            )
                        })
                        .when(in_use(&face.family), |d| {
                            d.child(
                                label(
                                    text::CAPTION,
                                    crate::i18n::text(
                                        "app.settings.appearance.custom_fonts_in_use",
                                    ),
                                )
                                .text_color(r.primary),
                            )
                        })
                        .into_any_element(),
                    Err(problem) => label(text::METADATA, font_problem(problem))
                        .text_color(r.feedback_error)
                        .into_any_element(),
                };
                file_line(&font.file_name, detail, &r)
            })
            .collect();
        let added: Vec<AnyElement> = state
            .added
            .iter()
            .map(|added| match &added.outcome {
                Ok(_) => notice(
                    crate::panels::settings::appearance::labels::custom_fonts_added(
                        &added.file_name,
                    ),
                    false,
                    &r,
                ),
                Err(problem) => notice(
                    rust_i18n::t!(
                        "app.settings.appearance.file_problem",
                        file = added.file_name,
                        problem = font_problem(problem)
                    )
                    .into_owned(),
                    true,
                    &r,
                ),
            })
            .collect();
        let fonts_dir = state.fonts_dir.clone();
        let choose = Button::new("custom-fonts-choose")
            .outline()
            .small()
            .icon(IconName::Plus)
            .label(crate::i18n::text(
                "app.settings.appearance.custom_fonts_choose",
            ))
            .on_click(|_, _, cx| {
                let picked = cx.prompt_for_paths(PathPromptOptions {
                    files: true,
                    directories: false,
                    multiple: true,
                    prompt: None,
                });
                cx.spawn(async move |cx| match picked.await {
                    Ok(Ok(Some(paths))) => cx.update(|cx| AppearanceState::add_fonts(paths, cx)),
                    Ok(Ok(None)) => {}
                    Ok(Err(_)) | Err(_) => {
                        tracing::error!(
                            code = "FONT_PICKER_FAILED",
                            "appearance.fonts.picker_failed"
                        );
                        cx.update(|cx| {
                            cx.global_mut::<AppearanceState>().error =
                                Some("app.settings.appearance.picker_failed");
                            cx.refresh_windows();
                        });
                    }
                })
                .detach();
            });
        group(
            "appearance-custom-fonts",
            crate::i18n::text("app.settings.appearance.custom_fonts_title"),
            Some(crate::i18n::text(
                "app.settings.appearance.custom_fonts_description",
            )),
            &r,
        )
        .child(
            h_flex()
                .gap(space::S8)
                .items_center()
                .py(space::S12)
                .child(choose)
                .child(
                    label(
                        text::METADATA,
                        crate::i18n::text("app.settings.appearance.custom_fonts_hint"),
                    )
                    .text_color(r.text_subtle),
                ),
        )
        .children(added)
        .children(files)
        .child(self.folder_row(
            "fonts",
            crate::i18n::text("app.settings.appearance.custom_fonts_folder"),
            fonts_dir,
            &r,
            cx,
        ))
        .child(
            label(
                text::METADATA,
                crate::i18n::text("app.settings.appearance.custom_fonts_remove_hint"),
            )
            .text_color(r.text_subtle)
            .pt(space::S8),
        )
        .into_any_element()
    }

    pub(super) fn custom_themes(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let r = AutocraftTheme::global(cx).roles;
        let state = cx.global::<AppearanceState>();
        let issues: Vec<AnyElement> = state
            .catalog
            .user_issues()
            .iter()
            .map(|issue| {
                let detail = v_flex()
                    .child(
                        label(text::METADATA, theme_problem(issue.kind))
                            .text_color(r.feedback_error),
                    )
                    .children(
                        issue
                            .detail
                            .clone()
                            .map(|d| label(text::CODE_SMALL, d).text_color(r.text_subtle)),
                    )
                    .into_any_element();
                file_line(&issue.file_name, detail, &r)
            })
            .collect();
        let themes_dir = state.themes_dir.clone();
        group(
            "appearance-custom-themes",
            crate::i18n::text("app.settings.appearance.custom_themes_title"),
            None,
            &r,
        )
        .child(
            label(
                text::METADATA,
                crate::i18n::text("app.settings.appearance.custom_themes_steps"),
            )
            .text_color(r.text_muted)
            .py(space::S12),
        )
        .child(self.folder_row(
            "themes",
            crate::i18n::text("app.settings.appearance.custom_themes_folder"),
            themes_dir,
            &r,
            cx,
        ))
        .children(issues)
        .into_any_element()
    }
}
