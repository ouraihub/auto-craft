//! Parameter controls generated from the selected tool's actual JSON schema.
use super::{CommandPalette, schema};
use gpui_kit::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _, Styled as _,
    Window,
    base::{h_flex, v_flex},
    component::{
        Disableable as _,
        button::Button,
        checkbox::Checkbox,
        input::Input,
        menu::{DropdownMenu as _, PopupMenuItem},
        scroll::ScrollableElement as _,
    },
};
impl CommandPalette {
    pub(super) fn render_form(&self, _: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(tool) = &self.selected else {
            return v_flex().into_any_element();
        };
        let mut content = v_flex()
            .gap_3()
            .child(tool.title.clone())
            .child(tool.summary.clone());
        for (index, parameter) in self.fields.iter().enumerate() {
            let field = &parameter.field;
            let mut row = v_flex()
                .gap_1()
                .child(format!(
                    "{}{}",
                    field.name,
                    if field.required { " *" } else { "" }
                ))
                .child(field.description.clone());
            let choices = field.choices();
            if !choices.is_empty() {
                let current = parameter.input.read(cx).value().to_string();
                let owner = cx.entity();
                row = row.child(
                    Button::new(format!("field-choice-{index}"))
                        .label(if current.is_empty() {
                            field.name.clone()
                        } else {
                            current.clone()
                        })
                        .accessibility_label(field.name.clone())
                        .icon(gpui_kit::assets::IconName::ChevronDown)
                        .disabled(self.busy)
                        .dropdown_menu(move |menu, _, _| {
                            choices.iter().fold(menu, |menu, choice| {
                                let value = choice
                                    .as_str()
                                    .map(str::to_owned)
                                    .unwrap_or_else(|| choice.to_string());
                                let owner = owner.clone();
                                menu.item(
                                    PopupMenuItem::new(value.clone())
                                        .checked(value == current)
                                        .on_click(move |_, window, cx| {
                                            owner.update(cx, |this, cx| {
                                                if this.busy {
                                                    return;
                                                }
                                                if let Some(parameter) = this.fields.get(index) {
                                                    parameter.input.update(cx, |input, cx| {
                                                        input.set_value(value.clone(), window, cx)
                                                    });
                                                }
                                            });
                                        }),
                                )
                            })
                        }),
                );
            } else if field.kind() == "boolean" {
                row = row.child(
                    Checkbox::new(("field-bool", index))
                        .checked(parameter.input.read(cx).value().as_ref() == "true")
                        .label(field.name.clone())
                        .on_click(cx.listener(move |this, checked: &bool, window, cx| {
                            if let Some(parameter) = this.fields.get(index) {
                                parameter.input.update(cx, |input, cx| {
                                    input.set_value(checked.to_string(), window, cx)
                                });
                            }
                        })),
                );
            } else {
                row = row.child(Input::new(&parameter.input));
                if field.kind() == "array" {
                    row = row.child(rust_i18n::t!("app.palette.array_hint"));
                }
            }
            content = content.child(row);
        }
        if self.confirming {
            content = content.child(
                v_flex()
                    .p_3()
                    .border_1()
                    .child(rust_i18n::t!("app.palette.consent"))
                    .child(tool.effects.join(" · ")),
            );
        }
        let buttons = h_flex()
            .gap_2()
            .child(
                Button::new("tool-back")
                    .label(rust_i18n::t!("app.palette.back"))
                    .disabled(self.busy)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.selected = None;
                        this.confirming = false;
                        cx.notify();
                    })),
            )
            .child(
                Button::new("tool-run")
                    .label(if self.confirming {
                        rust_i18n::t!("app.palette.allow_run")
                    } else {
                        rust_i18n::t!("app.palette.run")
                    })
                    .disabled(self.busy)
                    .on_click(cx.listener(|this, _, _, cx| {
                        let values = if let Some(params) = this.export_params.clone() {
                            Ok(params)
                        } else {
                            schema::build(this.fields.iter().map(|parameter| {
                                (
                                    &parameter.field,
                                    parameter.input.read(cx).value().to_string(),
                                )
                            }))
                        };
                        match values {
                            Ok(params) => {
                                if this
                                    .selected
                                    .as_ref()
                                    .is_some_and(|tool| tool.needs_consent)
                                    && !this.confirming
                                {
                                    this.confirming = true;
                                    cx.notify();
                                } else if let Some(tool) = this.selected.clone() {
                                    this.execute(tool, params, this.confirming, cx);
                                }
                            }
                            Err(error) => {
                                this.error = Some(error);
                                cx.notify();
                            }
                        }
                    })),
            );
        let cancel = self.cancel.clone();
        content = content.child(buttons).children(self.error.clone());
        if self.busy {
            content = content.child(
                Button::new("tool-stop")
                    .label(rust_i18n::t!("app.palette.cancel"))
                    .on_click(move |_, _, _| cancel.cancel()),
            );
        }
        content
            .id("tool-parameters")
            .max_h(gpui_kit::px(620.))
            .overflow_y_scrollbar()
            .into_any_element()
    }
}
