//! 字体 group: the preview, the interface font and text
//! weight, the code font, size and ligatures, and the fallback fonts under 高级.

use autocraft_core::{CodeFontSize, FontChoice, FontRole, FontWeightOffset};
use autocraft_ui::{
    AutocraftTheme, Roles,
    components::label,
    metrics::{radius, space, text},
};
use gpui_kit::{
    AnyElement, Context, IntoElement, ParentElement as _, Styled as _, Window,
    assets::IconName,
    base::{h_flex, v_flex},
    component::{
        Disableable as _, Sizable as _,
        button::{Button, ButtonVariants as _},
        slider::Slider,
        switch::Switch,
    },
    prelude::FluentBuilder as _,
};

use super::{
    super::rows::{disclosure, group, notice, row, subheading, with_reset},
    AppearancePage, PickerTarget, preview,
};
use crate::appearance::{AppearanceState, bundled_face, family_name, role_font};

fn weight_text(offset: i16) -> String {
    match offset {
        0 => crate::i18n::text("app.settings.appearance.contrast_default").into_owned(),
        o if o > 0 => format!("+{o}"),
        o => format!("−{}", -o),
    }
}

impl AppearancePage {
    pub(super) fn fonts_group(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let r = AutocraftTheme::global(cx).roles;
        let state = cx.global::<AppearanceState>();
        let prefs = state.prefs.clone();
        let failure = state.error;
        let interface = role_font(&prefs.interface_font, FontRole::Interface, &state.families);
        let code = role_font(&prefs.code_font, FontRole::Code, &state.families);
        let static_faces: Vec<&str> = [&interface.family, &code.family]
            .into_iter()
            .filter(|f| {
                *f == bundled_face(FontRole::Interface) || *f == bundled_face(FontRole::Code)
            })
            .map(String::as_str)
            .collect();
        let mut body = vec![
            preview::card(prefs.code_font_size, &r),
            subheading(
                crate::i18n::text("app.settings.appearance.interface_title"),
                crate::i18n::text("app.settings.appearance.interface_description"),
                &r,
            ),
            self.family_row(FontRole::Interface, &prefs.interface_font, &r, cx),
        ];
        body.extend(self.picker_for(PickerTarget::Family(FontRole::Interface), window, cx));
        body.extend(interface.unavailable.as_deref().map(|name| {
            notice(
                crate::panels::settings::appearance::labels::unavailable(
                    name,
                    bundled_face(FontRole::Interface),
                ),
                true,
                &r,
            )
        }));
        body.push(self.weight_row(prefs.font_weight_offset, &static_faces, &r, cx));
        body.push(subheading(
            crate::i18n::text("app.settings.appearance.code_title"),
            crate::i18n::text("app.settings.appearance.code_description"),
            &r,
        ));
        body.push(self.family_row(FontRole::Code, &prefs.code_font, &r, cx));
        body.extend(self.picker_for(PickerTarget::Family(FontRole::Code), window, cx));
        body.extend(code.unavailable.as_deref().map(|name| {
            notice(
                crate::panels::settings::appearance::labels::unavailable(
                    name,
                    bundled_face(FontRole::Code),
                ),
                true,
                &r,
            )
        }));
        body.push(self.size_row(prefs.code_font_size, &r));
        body.push(self.ligatures_row(prefs.code_ligatures, &r, cx));
        let open = self.advanced_open;
        body.push(disclosure(
            "font-advanced-toggle",
            crate::i18n::text("app.settings.appearance.advanced"),
            crate::i18n::text("app.settings.appearance.advanced_summary").into_owned(),
            open,
            &r,
            {
                let page = cx.entity().downgrade();
                move |_, cx| {
                    if page
                        .update(cx, |page, cx| {
                            page.advanced_open = !page.advanced_open;
                            cx.notify();
                        })
                        .is_err()
                    {
                        tracing::debug!("appearance.fonts.view_closed");
                    }
                }
            },
        ));
        if open {
            for role in [FontRole::Interface, FontRole::Code] {
                body.push(self.fallback_row(role, prefs.font(role), &r, cx));
                body.extend(self.picker_for(PickerTarget::Fallback(role), window, cx));
            }
        }
        group(
            "appearance-fonts",
            crate::i18n::text("app.settings.appearance.fonts_title"),
            Some(crate::i18n::text(
                "app.settings.appearance.fonts_description",
            )),
            &r,
        )
        .children(failure.map(|key| notice(crate::i18n::text(key).into_owned(), true, &r)))
        .gap(space::S4)
        .child(v_flex().pt(space::S12).children(body))
        .into_any_element()
    }

    fn family_row(
        &self,
        role: FontRole,
        choice: &FontChoice,
        r: &Roles,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let shown = choice
            .family
            .as_ref()
            .map_or(bundled_face(role), family_name)
            .to_owned();
        let (key, reset_label) = match role {
            FontRole::Interface => (
                "font-interface",
                crate::i18n::text("app.settings.appearance.reset_interface_font"),
            ),
            FontRole::Code => (
                "font-code",
                crate::i18n::text("app.settings.appearance.reset_code_font"),
            ),
        };
        let trigger = Button::new(key)
            .outline()
            .label(shown)
            .icon(IconName::ChevronDown)
            .on_click(cx.listener(move |this, _, window, cx| {
                this.toggle_picker(PickerTarget::Family(role), window, cx);
            }));
        let control = with_reset(
            match role {
                FontRole::Interface => "font-interface-reset",
                FontRole::Code => "font-code-reset",
            },
            choice.family.is_some(),
            reset_label,
            trigger,
            move |_, cx| {
                AppearanceState::edit("app.settings.appearance.fonts_change_failed", cx, |prefs| {
                    prefs.font_mut(role).family = None;
                });
            },
        );
        row(
            crate::i18n::text("app.settings.appearance.font"),
            None,
            control,
            r,
        )
    }

    fn weight_row(
        &self,
        offset: i16,
        static_faces: &[&str],
        r: &Roles,
        _cx: &mut Context<Self>,
    ) -> AnyElement {
        let reset = with_reset(
            "font-weight-reset",
            offset != FontWeightOffset::DEFAULT,
            crate::i18n::text("app.settings.appearance.reset_weight"),
            label(text::UI, weight_text(offset)),
            |_, cx| {
                AppearanceState::edit("app.settings.appearance.fonts_change_failed", cx, |prefs| {
                    prefs.font_weight_offset = FontWeightOffset::DEFAULT;
                });
            },
        );
        v_flex()
            .gap(space::S8)
            .py(space::S12)
            .border_b_1()
            .border_color(r.border_muted)
            .child(
                h_flex()
                    .items_start()
                    .gap(space::S12)
                    .child(
                        v_flex()
                            .flex_1()
                            .gap(space::S2)
                            .child(label(
                                text::UI,
                                crate::i18n::text("app.settings.appearance.weight"),
                            ))
                            .child(
                                label(
                                    text::METADATA,
                                    crate::i18n::text("app.settings.appearance.weight_description"),
                                )
                                .text_color(r.text_muted),
                            ),
                    )
                    .child(reset),
            )
            .child(Slider::new(&self.weight).horizontal())
            .child(
                h_flex()
                    .justify_between()
                    .child(
                        label(
                            text::METADATA,
                            crate::i18n::text("app.settings.appearance.weight_lighter"),
                        )
                        .text_color(r.text_muted),
                    )
                    .child(
                        label(
                            text::METADATA,
                            crate::i18n::text("app.settings.appearance.weight_heavier"),
                        )
                        .text_color(r.text_muted),
                    ),
            )
            .children(static_faces.iter().map(|family| {
                notice(
                    crate::panels::settings::appearance::labels::weight_steps(family),
                    false,
                    r,
                )
            }))
            .into_any_element()
    }

    fn size_row(&self, size: u8, r: &Roles) -> AnyElement {
        let step = |delta: i8| {
            move |_: &gpui_kit::ClickEvent, _: &mut Window, cx: &mut gpui_kit::App| {
                AppearanceState::edit("app.settings.appearance.fonts_change_failed", cx, |prefs| {
                    prefs.code_font_size = prefs
                        .code_font_size
                        .saturating_add_signed(delta)
                        .clamp(CodeFontSize::MIN, CodeFontSize::MAX);
                });
            }
        };
        let stepper = h_flex()
            .gap(space::S4)
            .items_center()
            .child(
                Button::new("font-size-smaller")
                    .outline()
                    .small()
                    .icon(IconName::Minus)
                    .tooltip(crate::i18n::text("app.settings.appearance.smaller"))
                    .accessibility_label(crate::i18n::text("app.settings.appearance.smaller"))
                    .disabled(size <= CodeFontSize::MIN)
                    .on_click(step(-1)),
            )
            .child(
                label(text::UI, format!("{size} px"))
                    .min_w(gpui_kit::px(48.))
                    .text_center()
                    .px(space::S8)
                    .py(space::S4)
                    .rounded(radius::SM)
                    .border_1()
                    .border_color(r.control_border_default),
            )
            .child(
                Button::new("font-size-larger")
                    .outline()
                    .small()
                    .icon(IconName::Plus)
                    .tooltip(crate::i18n::text("app.settings.appearance.larger"))
                    .accessibility_label(crate::i18n::text("app.settings.appearance.larger"))
                    .disabled(size >= CodeFontSize::MAX)
                    .on_click(step(1)),
            );
        row(
            crate::i18n::text("app.settings.appearance.size"),
            Some(
                crate::panels::settings::appearance::labels::size_description(
                    CodeFontSize::EDITOR_DELTA,
                ),
            ),
            with_reset(
                "font-size-reset",
                size != CodeFontSize::DEFAULT,
                crate::i18n::text("app.settings.appearance.reset_size"),
                stepper,
                |_, cx| {
                    AppearanceState::edit(
                        "app.settings.appearance.fonts_change_failed",
                        cx,
                        |prefs| {
                            prefs.code_font_size = CodeFontSize::DEFAULT;
                        },
                    );
                },
            ),
            r,
        )
    }

    fn ligatures_row(&self, on: bool, r: &Roles, cx: &mut Context<Self>) -> AnyElement {
        let switch = Switch::new("font-ligatures")
            .checked(on)
            .accessibility_label(crate::i18n::text("app.settings.appearance.ligatures"))
            .on_click(cx.listener(|_, checked: &bool, _, cx| {
                let checked = *checked;
                AppearanceState::edit("app.settings.appearance.fonts_change_failed", cx, |prefs| {
                    prefs.code_ligatures = checked;
                });
            }));
        row(
            crate::i18n::text("app.settings.appearance.ligatures"),
            Some(crate::i18n::text("app.settings.appearance.ligatures_description").into_owned()),
            with_reset(
                "font-ligatures-reset",
                on,
                crate::i18n::text("app.settings.appearance.reset_ligatures"),
                switch,
                |_, cx| {
                    AppearanceState::edit(
                        "app.settings.appearance.fonts_change_failed",
                        cx,
                        |prefs| {
                            prefs.code_ligatures = false;
                        },
                    );
                },
            ),
            r,
        )
    }

    fn fallback_row(
        &self,
        role: FontRole,
        choice: &FontChoice,
        r: &Roles,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (title, description) = match role {
            FontRole::Interface => (
                crate::i18n::text("app.settings.appearance.interface_fallbacks"),
                crate::i18n::text("app.settings.appearance.interface_fallbacks_description"),
            ),
            FontRole::Code => (
                crate::i18n::text("app.settings.appearance.code_fallbacks"),
                crate::i18n::text("app.settings.appearance.code_fallbacks_description"),
            ),
        };
        let chips = choice
            .fallbacks
            .iter()
            .enumerate()
            .map(|(index, fallback)| {
                h_flex()
                    .gap(space::S4)
                    .items_center()
                    .pl(space::S8)
                    .rounded(radius::SM)
                    .border_1()
                    .border_color(r.border_default)
                    .child(label(text::METADATA, family_name(fallback).to_owned()))
                    .child(
                        Button::new(format!("font-{role:?}-fallback-remove-{index}"))
                            .ghost()
                            .xsmall()
                            .icon(IconName::X)
                            .on_click(move |_, _, cx| {
                                AppearanceState::edit(
                                    "app.settings.appearance.fonts_change_failed",
                                    cx,
                                    |prefs| {
                                        let fallbacks = &mut prefs.font_mut(role).fallbacks;
                                        if index < fallbacks.len() {
                                            fallbacks.remove(index);
                                        }
                                    },
                                );
                            }),
                    )
            });
        let full = choice.fallbacks.len() >= autocraft_core::FONT_FALLBACKS_MAX;
        let control = h_flex()
            .flex_wrap()
            .gap(space::S4)
            .max_w(gpui_kit::px(360.))
            .children(chips)
            .when(!full, |d| {
                d.child(
                    Button::new(match role {
                        FontRole::Interface => "font-interface-fallback-add",
                        FontRole::Code => "font-code-fallback-add",
                    })
                    .outline()
                    .small()
                    .icon(IconName::Plus)
                    .label(crate::i18n::text("app.settings.appearance.add_fallback"))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.toggle_picker(PickerTarget::Fallback(role), window, cx);
                    })),
                )
            });
        row(title, Some(description.into_owned()), control, r)
    }
}
