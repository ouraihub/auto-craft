//! Rebuild check presentation with explicit screenshot assets and open intents.
use super::{label, roles};
use crate::metrics::{radius, space, text};
use autocraft_core::CheckResultDto;
use gpui_kit::StyledImage as _;
use gpui_kit::{
    App, ElementId, InteractiveElement, IntoElement, ParentElement, RenderOnce, SharedString,
    StatefulInteractiveElement, Styled, Window, div, img, prelude::FluentBuilder as _,
};
use std::{path::PathBuf, rc::Rc, sync::Arc};

/// Localized labels for the check card; evidence and claim content is not translated.
#[derive(Debug, Clone)]
pub struct CheckCardLabels {
    pub passed: SharedString,
    pub failed: SharedString,
    pub similarity: SharedString,
    pub target: SharedString,
    pub unknowns: SharedString,
}
/// A resolved screenshot, supplied by the application (no artifact path guessing).
#[derive(Debug, Clone)]
pub struct CheckThumbnail {
    pub path: PathBuf,
    pub caption: SharedString,
}
type ScreenshotHandler = Rc<dyn Fn(&CheckThumbnail, &mut Window, &mut App)>;
/// Check result, claims, similarity and screenshot thumbnails.
#[derive(IntoElement)]
pub struct CheckCard {
    id: ElementId,
    result: Arc<CheckResultDto>,
    labels: CheckCardLabels,
    thumbnails: Vec<CheckThumbnail>,
    on_screenshot: Option<ScreenshotHandler>,
}
impl CheckCard {
    pub fn new(
        id: impl Into<ElementId>,
        result: Arc<CheckResultDto>,
        labels: CheckCardLabels,
    ) -> Self {
        Self {
            id: id.into(),
            result,
            labels,
            thumbnails: Vec::new(),
            on_screenshot: None,
        }
    }
    pub fn thumbnails(mut self, thumbnails: Vec<CheckThumbnail>) -> Self {
        self.thumbnails = thumbnails;
        self
    }
    pub fn on_screenshot(
        mut self,
        handler: impl Fn(&CheckThumbnail, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_screenshot = Some(Rc::new(handler));
        self
    }
}
impl RenderOnce for CheckCard {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let r = roles(cx);
        let color = if self.result.passed {
            r.feedback_success
        } else {
            r.feedback_error
        };
        let status = if self.result.passed {
            self.labels.passed
        } else {
            self.labels.failed
        };
        let mut card = div()
            .id(self.id)
            .flex()
            .flex_col()
            .gap(space::S8)
            .p(space::S12)
            .rounded(radius::SM)
            .border_1()
            .border_color(color)
            .bg(r.container_content_bg)
            .text_color(r.text_default)
            .child(
                div()
                    .flex()
                    .gap(space::S8)
                    .child(label(text::TITLE_CARD, self.result.check.clone()))
                    .child(label(text::LABEL_PILL, status).text_color(color)),
            )
            .child(label(text::UI, self.result.note.clone()));
        if let Some(score) = self.result.similarity.filter(|value| value.is_finite()) {
            // The engine reports likeness as a percentage, not a unit interval.
            let fraction = (score / 100.).clamp(0., 1.) as f32;
            card = card
                .child(label(
                    text::METADATA,
                    format!("{}: {score:.1}%", self.labels.similarity),
                ))
                .child(
                    div()
                        .h(space::S4)
                        .w_full()
                        .bg(r.control_bg)
                        .child(div().h_full().w(gpui_kit::relative(fraction)).bg(color)),
                );
        }
        if let Some(target) = self.result.good_enough {
            card = card.child(label(
                text::METADATA,
                format!("{}: {target:.1}%", self.labels.target),
            ));
        }
        for claim in &self.result.claims {
            let color = if claim.passed {
                r.feedback_success
            } else {
                r.feedback_error
            };
            card = card.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(space::S2)
                    .child(
                        label(
                            text::EMPHASIS,
                            format!("{} {}", if claim.passed { "✓" } else { "✗" }, claim.title),
                        )
                        .text_color(color),
                    )
                    .child(label(text::UI, claim.detail.clone())),
            );
        }
        if !self.result.unknowns.is_empty() {
            card = card.child(label(text::EMPHASIS, self.labels.unknowns));
            for unknown in &self.result.unknowns {
                card = card.child(label(text::UI, unknown.clone()).text_color(r.feedback_warning));
            }
        }
        for page in &self.result.pages {
            card = card.child(label(
                text::METADATA,
                format!("{}: {:.1}%", page.name, page.similarity),
            ));
        }
        card.child(
            div().flex().flex_wrap().gap(space::S8).children(
                self.thumbnails
                    .into_iter()
                    .enumerate()
                    .map(|(index, thumbnail)| {
                        let callback = self.on_screenshot.clone();
                        let caption = thumbnail.caption.clone();
                        let path = thumbnail.path.clone();
                        div()
                            .id(index)
                            .flex()
                            .flex_col()
                            .gap(space::S4)
                            .when(callback.is_some(), |d| d.cursor_pointer())
                            .on_click(move |_, window, cx| {
                                if let Some(callback) = &callback {
                                    callback(&thumbnail, window, cx);
                                }
                            })
                            .child(
                                img(path)
                                    .w(space::S64 * 3.)
                                    .h(space::S64 * 2.)
                                    .object_fit(gpui_kit::ObjectFit::Contain),
                            )
                            .child(label(text::METADATA, caption))
                    }),
            ),
        )
    }
}
