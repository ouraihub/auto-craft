//! Controlled four-stage progress and target-kind choice cards.
use super::{Handler, label, roles};
use crate::metrics::{radius, size, space, text};
use autocraft_core::StepDto;
use gpui_kit::{
    App, ClickEvent, ElementId, InteractiveElement, IntoElement, ParentElement, RenderOnce,
    SharedString, StatefulInteractiveElement, Styled, Window, assets::IconName, component::Icon,
    div, prelude::FluentBuilder as _,
};
use std::rc::Rc;

/// Presentation status supplied explicitly, never derived from backend run state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepState {
    Pending,
    Active,
    Complete,
    Failed,
}
type StepHandler = Rc<dyn Fn(StepDto, &mut Window, &mut App)>;
/// Four ordered segments: study, build, check, ready.
#[derive(IntoElement)]
pub struct StepTracker {
    id: ElementId,
    current: Option<StepDto>,
    states: [StepState; 4],
    labels: [SharedString; 4],
    on_select: Option<StepHandler>,
}
impl StepTracker {
    pub fn new(
        id: impl Into<ElementId>,
        current: Option<StepDto>,
        states: [StepState; 4],
        labels: [SharedString; 4],
    ) -> Self {
        Self {
            id: id.into(),
            current,
            states,
            labels,
            on_select: None,
        }
    }
    pub fn on_select(mut self, handler: impl Fn(StepDto, &mut Window, &mut App) + 'static) -> Self {
        self.on_select = Some(Rc::new(handler));
        self
    }
}
impl RenderOnce for StepTracker {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let r = roles(cx);
        let steps = [
            StepDto::Study,
            StepDto::Build,
            StepDto::Check,
            StepDto::Done,
        ];
        div()
            .id(self.id)
            .flex()
            .gap(space::S4)
            .children(self.labels.into_iter().enumerate().map(|(index, name)| {
                let step = steps[index];
                let color = match self.states[index] {
                    StepState::Pending => r.text_subtle,
                    StepState::Active => r.primary,
                    StepState::Complete => r.feedback_success,
                    StepState::Failed => r.feedback_error,
                };
                let marker = match self.states[index] {
                    StepState::Pending | StepState::Active => (index + 1).to_string(),
                    StepState::Complete => "✓".to_owned(),
                    StepState::Failed => "!".to_owned(),
                };
                let callback = self.on_select.clone();
                div()
                    .id(index)
                    .flex_1()
                    .flex()
                    .flex_col()
                    .gap(space::S4)
                    .p(space::S8)
                    .border_b_2()
                    .border_color(color)
                    .text_color(color)
                    .when(self.current == Some(step), |d| d.bg(r.control_bg_selected))
                    .when(callback.is_some(), |d| d.cursor_pointer())
                    .on_click(move |_, window, cx| {
                        if let Some(callback) = &callback {
                            callback(step, window, cx);
                        }
                    })
                    .child(label(text::EMPHASIS, marker))
                    .child(label(text::UI, name))
            }))
    }
}
/// Selectable target-kind tile with its caller-owned selected state.
#[derive(IntoElement)]
pub struct KindTile {
    id: ElementId,
    icon: IconName,
    title: SharedString,
    description: SharedString,
    selected: bool,
    on_click: Option<Handler>,
}
impl KindTile {
    pub fn new(
        id: impl Into<ElementId>,
        icon: IconName,
        title: impl Into<SharedString>,
        description: impl Into<SharedString>,
        selected: bool,
    ) -> Self {
        Self {
            id: id.into(),
            icon,
            title: title.into(),
            description: description.into(),
            selected,
            on_click: None,
        }
    }
    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Box::new(handler));
        self
    }
}
impl RenderOnce for KindTile {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let r = roles(cx);
        div()
            .id(self.id)
            .flex()
            .flex_col()
            .gap(space::S8)
            .p(space::S16)
            .rounded(radius::LG)
            .border_1()
            .border_color(if self.selected {
                r.control_border_active
            } else {
                r.control_border_default
            })
            .bg(if self.selected {
                r.control_bg_selected
            } else {
                r.control_bg
            })
            .cursor_pointer()
            .hover(|s| s.bg(r.control_bg_hovered))
            .when_some(self.on_click, |d, handler| d.on_click(handler))
            .child(Icon::new(self.icon).size(size::ICON).text_color(r.primary))
            .child(label(text::TITLE_CARD, self.title).text_color(r.text_default))
            .child(label(text::UI, self.description).text_color(r.text_muted))
    }
}
