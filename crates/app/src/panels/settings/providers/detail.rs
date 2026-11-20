//! Native detail intents and route data; LumaKit owns their presentation.

use autocraft_routing::models_url;
use autocraft_ui::{
    components::{IconButton, label},
    metrics::{space, text},
};
use gpui_kit::{
    AnyElement, Context, IntoElement, ParentElement as _, Styled as _,
    assets::IconName,
    base::h_flex,
    component::{
        Sizable as _,
        button::{Button, ButtonVariants as _},
        input::Input,
        menu::{DropdownMenu as _, PopupMenuItem},
        switch::Switch,
    },
};
use lumakit_gpui::providers as ui;

use super::{ProvidersPage, error_text, theme};
use crate::state::run;

mod key;

impl ProvidersPage {
    pub(super) fn render_detail(&self, cx: &mut Context<Self>) -> AnyElement {
        let r = theme::roles(cx);
        let (Some(provider), Some(draft)) = (self.provider().cloned(), self.draft.as_ref()) else {
            return lumakit_design::components::settings::placeholder(
                if self.loaded {
                    crate::i18n::text("app.settings.providers.empty")
                } else {
                    crate::i18n::text("app.settings.providers.loading")
                },
                if self.loaded {
                    crate::i18n::text("app.settings.providers.empty_hint")
                } else {
                    "".into()
                },
                &r,
            );
        };
        let failed = self.failed.clone().map(|error| {
            h_flex()
                .gap(space::S8)
                .child(
                    label(
                        text::METADATA,
                        rust_i18n::t!("app.settings.providers.save_failed_detail", error = error)
                            .into_owned(),
                    )
                    .text_color(r.feedback_error),
                )
                .child(
                    Button::new("provider-retry")
                        .link()
                        .label(crate::i18n::text("app.settings.providers.retry"))
                        .on_click(cx.listener(|this, _, _, cx| this.commit(cx))),
                )
                .into_any_element()
        });
        let settings = IconButton::new("provider-settings", IconName::Settings2)
            .tooltip(crate::i18n::text("app.settings.providers.settings"))
            .on_click(cx.listener({
                let p = provider.clone();
                move |this, _, window, cx| this.open_name_dialog(Some(p.clone()), window, cx)
            }));
        let enabled = Switch::new("provider-enabled")
            .checked(provider.input.enabled)
            .color(r.feedback_success)
            .on_click(cx.listener(|this, value: &bool, _, cx| this.set_enabled(*value, cx)));
        ui::detail(
            ui::detail_header(
                draft.name.clone(),
                settings.into_any_element(),
                enabled.into_any_element(),
            )
            .into_any_element(),
            failed,
            self.render_key(&provider, cx),
            ui::strategy(
                crate::i18n::text("app.settings.providers.key_strategy_round_robin"),
                crate::i18n::text("app.settings.providers.key_strategy_round_robin_desc"),
                &r,
            )
            .into_any_element(),
            self.render_url(cx),
            self.render_models(cx),
        )
        .into_any_element()
    }

    pub(super) fn set_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        let Some(id) = self.selected.clone() else {
            return;
        };
        let service = self.service(cx);
        let target = id.clone();
        run(
            cx,
            move || {
                let provider = service.provider(&id)?;
                let mut input = provider.input;
                input.enabled = enabled;
                service.update(&id, provider.revision, input)
            },
            move |this, result, cx| {
                match result {
                    Ok(provider) => {
                        if this.selected.as_ref() == Some(&target)
                            && let Some(draft) = &mut this.draft
                        {
                            draft.enabled = provider.input.enabled;
                        }
                        this.store_snapshot(provider, cx);
                    }
                    Err(error) => this.report(&error, cx),
                }
                cx.notify();
            },
        );
    }

    fn render_url(&self, cx: &mut Context<Self>) -> AnyElement {
        let r = theme::roles(cx);
        let Some(draft) = self.draft.as_ref() else {
            return gpui_kit::div().into_any_element();
        };
        let can_add = !autocraft_core::free_protocols(draft).is_empty();
        let lines = draft.endpoints.clone();
        let current = self.line;
        let page = cx.entity();
        let picker = (lines.len() > 1).then(|| {
            Button::new("provider-line")
                .outline()
                .w_full()
                .label(format!(
                    "{} · {}",
                    crate::panels::settings::providers::labels::protocol_short(current),
                    self.line_url()
                ))
                .icon(IconName::ChevronDown)
                .dropdown_menu(move |menu, _, _| {
                    lines.iter().fold(menu, |menu, line| {
                        let (protocol, page) = (line.protocol, page.clone());
                        menu.item(
                            PopupMenuItem::new(format!(
                                "{} · {}",
                                crate::panels::settings::providers::labels::protocol_short(
                                    protocol
                                ),
                                line.base_url
                            ))
                            .checked(protocol == current)
                            .on_click(move |_, _, cx| {
                                page.update(cx, |this, cx| {
                                    this.line = protocol;
                                    this.sync_inputs = true;
                                    this.models.reset();
                                    cx.notify();
                                })
                            }),
                        )
                    })
                })
                .into_any_element()
        });
        let url = self.line_url();
        let hint = if self.url_error {
            error_text(
                Some(crate::i18n::text("app.settings.providers.url_invalid").into()),
                cx,
            )
            .map(IntoElement::into_any_element)
        } else {
            (!url.is_empty()).then(|| {
                label(
                    text::CODE_SMALL,
                    crate::panels::settings::providers::labels::probe_get(&models_url(
                        &url, current,
                    )),
                )
                .text_color(r.text_subtle)
                .truncate()
                .into_any_element()
            })
        };
        ui::route_section(
            can_add.then(|| {
                Button::new("provider-add-line")
                    .link()
                    .xsmall()
                    .label(crate::i18n::text("app.settings.providers.add_line"))
                    .on_click(
                        cx.listener(|this, _, window, cx| this.open_line_dialog(None, window, cx)),
                    )
                    .into_any_element()
            }),
            picker,
            Input::new(&self.url).into_any_element(),
            Button::new("provider-line-settings")
                .outline()
                .icon(IconName::Settings2)
                .tooltip(crate::i18n::text(
                    "app.settings.providers.line_settings_tip",
                ))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.open_line_dialog(Some(current), window, cx)
                }))
                .into_any_element(),
            None,
            hint,
            &r,
        )
        .into_any_element()
    }
}
