//! A one-of-many choice shown as its current value; the options open in a dropdown menu.

use std::rc::Rc;

use gpui_kit::{
    App, ElementId, IntoElement, ParentElement as _, RenderOnce, SharedString, Styled as _, Window,
    assets::IconName,
    base::h_flex,
    component::{
        Disableable as _, Icon, Sizable as _,
        button::{Button, ButtonVariants as _},
        menu::{DropdownMenu as _, PopupMenuItem},
    },
    div,
    prelude::FluentBuilder as _,
};

use super::{SelectHandler, label};
use crate::metrics::{space, text};

/// At most one option is active and checked in the menu. Picking the active option again does
/// not call the handler, so toggling handlers only run on a real change.
#[derive(IntoElement)]
pub struct ChoiceMenu {
    id: ElementId,
    options: Vec<SharedString>,
    active: Option<usize>,
    placeholder: SharedString,
    name: Option<SharedString>,
    ghost: bool,
    full_width: bool,
    disabled: bool,
    tooltip: Option<SharedString>,
    on_select: Option<SelectHandler>,
}

impl ChoiceMenu {
    pub fn new(
        id: impl Into<ElementId>,
        options: impl IntoIterator<Item = impl Into<SharedString>>,
        active: Option<usize>,
    ) -> Self {
        Self {
            id: id.into(),
            options: options.into_iter().map(Into::into).collect(),
            active,
            placeholder: SharedString::default(),
            name: None,
            ghost: false,
            full_width: false,
            disabled: false,
            tooltip: None,
            on_select: None,
        }
    }

    /// The title while no option is active.
    pub fn placeholder(mut self, placeholder: impl Into<SharedString>) -> Self {
        self.placeholder = placeholder.into();
        self
    }

    /// What the choice sets; screen readers hear "name: value".
    pub fn name(mut self, name: impl Into<SharedString>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// A borderless trigger for title bars; forms keep the outlined default.
    pub fn ghost(mut self) -> Self {
        self.ghost = true;
        self
    }

    pub fn full_width(mut self, full_width: bool) -> Self {
        self.full_width = full_width;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn tooltip(mut self, tooltip: impl Into<SharedString>) -> Self {
        self.tooltip = Some(tooltip.into());
        self
    }

    pub fn on_select(mut self, handler: impl Fn(&usize, &mut Window, &mut App) + 'static) -> Self {
        self.on_select = Some(Box::new(handler));
        self
    }
}

impl RenderOnce for ChoiceMenu {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let title = self
            .active
            .and_then(|index| self.options.get(index).cloned())
            .unwrap_or(self.placeholder);
        let handler = self.on_select.map(Rc::new);
        let (options, active, ghost) = (self.options, self.active, self.ghost);
        let accessible = match &self.name {
            Some(name) => SharedString::from(format!("{name}: {title}")),
            None => title.clone(),
        };
        // A select look: value left, chevron right (a labelled Button centers label + icon).
        let trigger = h_flex()
            .w_full()
            .gap(space::S4)
            .child(label(text::UI, title.clone()).truncate())
            .child(div().flex_1())
            .child(Icon::new(IconName::ChevronDown).small());
        Button::new(self.id)
            .map(|button| {
                if ghost {
                    button.ghost()
                } else {
                    button.outline()
                }
            })
            .when(self.full_width, |button| button.w_full())
            .min_w_0()
            .child(trigger)
            .accessibility_label(accessible)
            .disabled(self.disabled)
            .when_some(self.tooltip, |button, tooltip| button.tooltip(tooltip))
            .dropdown_menu(move |menu, _, _| {
                options
                    .iter()
                    .enumerate()
                    .fold(menu, |menu, (index, option)| {
                        let handler = handler.clone();
                        menu.item(
                            PopupMenuItem::new(option.clone())
                                .checked(active == Some(index))
                                .on_click(move |_, window, cx| {
                                    if active != Some(index)
                                        && let Some(handler) = &handler
                                    {
                                        handler(&index, window, cx);
                                    }
                                }),
                        )
                    })
            })
    }
}
