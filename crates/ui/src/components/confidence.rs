//! Evidence confidence presentation; unknown is an absent domain confidence.
use super::{label, roles};
use crate::metrics::{radius, space, text};
use autocraft_core::Confidence;
use gpui_kit::{App, IntoElement, ParentElement, RenderOnce, SharedString, Styled, Window, div};

/// Localized confidence names supplied by the application.
#[derive(Debug, Clone)]
pub struct ConfidenceLabels {
    pub observed: SharedString,
    pub derived: SharedString,
    pub inferred: SharedString,
    pub unknown: SharedString,
}

/// Read-only confidence badge. `None` displays unknown without inventing evidence.
#[derive(Debug, IntoElement)]
pub struct ConfidencePill {
    confidence: Option<Confidence>,
    labels: ConfidenceLabels,
}
impl ConfidencePill {
    pub fn new(confidence: Option<Confidence>, labels: ConfidenceLabels) -> Self {
        Self { confidence, labels }
    }
}
impl RenderOnce for ConfidencePill {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let r = roles(cx);
        let (name, color) = match self.confidence {
            Some(Confidence::Observed) => (self.labels.observed, r.feedback_success),
            Some(Confidence::Derived) => (self.labels.derived, r.feedback_info),
            Some(Confidence::Inferred) => (self.labels.inferred, r.feedback_warning),
            None => (self.labels.unknown, r.text_subtle),
        };
        div()
            .px(space::S6)
            .py(space::S2)
            .rounded(radius::SM)
            .border_1()
            .border_color(color)
            .text_color(color)
            .child(label(text::LABEL_PILL, name))
    }
}
