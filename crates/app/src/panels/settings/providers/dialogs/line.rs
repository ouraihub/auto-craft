//! 线路设置对话框（协议、API 地址、请求头）与移除线路确认。

use autocraft_core::{Header, Protocol, free_protocols, remove_line, retarget_line, set_line};
use autocraft_routing::preview;
use autocraft_ui::{
    components::{IconButton, label},
    metrics::{space, text},
};
use gpui_kit::{
    AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, Styled as _, Window,
    base::{h_flex, v_flex},
    component::{
        Disableable as _, Sizable as _, WindowExt as _,
        button::{Button, ButtonVariants as _},
        input::{Input, InputState},
    },
    prelude::FluentBuilder as _,
    px,
};

use super::protocol_menu;
use crate::panels::settings::providers::{ProvidersPage, error_text, open_form, theme, valid_url};
use lumakit_gpui::providers as ui;

/// 线路设置：协议、API 地址、请求头（请求头对整个服务商生效）。`editing` 为 None 时新建线路。
struct LineForm {
    page: Entity<ProvidersPage>,
    provider: autocraft_core::ProviderId,
    editing: Option<Protocol>,
    options: Vec<Protocol>,
    protocol: Protocol,
    is_default: bool,
    can_remove: bool,
    url: Entity<InputState>,
    headers: Vec<(Entity<InputState>, Entity<InputState>)>,
    error: Option<String>,
    saving: bool,
}

impl LineForm {
    fn add_header(&mut self, header: Header, window: &mut Window, cx: &mut Context<Self>) {
        let name = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(crate::i18n::text("app.settings.providers.header_name"))
                .default_value(header.name)
        });
        let value = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(crate::i18n::text("app.settings.providers.header_value"))
                .default_value(header.value)
        });
        self.headers.push((name, value));
    }

    fn result(
        &self,
        cx: &gpui_kit::App,
    ) -> Result<(String, Vec<Header>), std::borrow::Cow<'static, str>> {
        let url = self.url.read(cx).value().trim().to_owned();
        if !valid_url(&url) {
            return Err(crate::i18n::text("app.settings.providers.url_invalid"));
        }
        let mut headers = Vec::new();
        for (name, value) in &self.headers {
            let (name, value) = (
                name.read(cx).value().trim().to_owned(),
                value.read(cx).value().trim().to_owned(),
            );
            if name.is_empty() && value.is_empty() {
                continue;
            }
            if name.is_empty() || value.is_empty() {
                return Err(crate::i18n::text("app.settings.providers.header_invalid"));
            }
            headers.push(Header { name, value });
        }
        Ok((url, headers))
    }
}

impl Render for LineForm {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let r = theme::roles(cx);
        let protocol = protocol_menu(
            "line-protocol",
            self.protocol,
            self.options.clone(),
            cx.entity(),
            |form: &mut LineForm, p| form.protocol = p,
        );
        let url = self.url.read(cx).value().trim().to_owned();
        ui::form_fields([])
            .child(ui::field(
                crate::i18n::text("app.settings.providers.protocol"),
                protocol,
                &r,
            ))
            .child(ui::field(
                crate::i18n::text("app.settings.providers.base_url"),
                Input::new(&self.url),
                &r,
            ))
            .when(valid_url(&url), |d| {
                d.child(
                    label(
                        text::CODE_SMALL,
                        crate::panels::settings::providers::labels::request_post(&preview(
                            &url,
                            self.protocol,
                        )),
                    )
                    .text_color(r.text_subtle),
                )
            })
            .child(
                label(
                    text::METADATA,
                    crate::i18n::text("app.settings.providers.url_hint"),
                )
                .text_color(r.text_subtle),
            )
            .child(ui::field(
                crate::i18n::text("app.settings.providers.headers"),
                v_flex()
                    .gap(space::S6)
                    .children(
                        self.headers
                            .iter()
                            .enumerate()
                            .map(|(index, (name, value))| {
                                h_flex()
                                    .gap(space::S6)
                                    .child(Input::new(name).flex_1())
                                    .child(Input::new(value).flex_1())
                                    .child(
                                        IconButton::new(
                                            ("line-header-remove", index),
                                            gpui_kit::assets::IconName::Minus,
                                        )
                                        .on_click(
                                            cx.listener(move |this, _, _, cx| {
                                                this.headers.remove(index);
                                                cx.notify();
                                            }),
                                        ),
                                    )
                            }),
                    )
                    .child(
                        Button::new("line-header-add")
                            .ghost()
                            .small()
                            .label(crate::i18n::text("app.settings.providers.headers_add"))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.add_header(
                                    Header {
                                        name: String::new(),
                                        value: String::new(),
                                    },
                                    window,
                                    cx,
                                );
                                cx.notify();
                            })),
                    )
                    .child(
                        label(
                            text::METADATA,
                            crate::i18n::text("app.settings.providers.headers_shared"),
                        )
                        .text_color(r.text_subtle),
                    ),
                &r,
            ))
            .children(error_text(self.error.clone(), cx))
            .when(self.editing.is_some(), |d| {
                d.child(
                    h_flex()
                        .gap(space::S8)
                        .when(!self.is_default, |d| {
                            d.child(
                                Button::new("line-default")
                                    .outline()
                                    .small()
                                    .label(crate::i18n::text(
                                        "app.settings.providers.set_default_line",
                                    ))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.is_default = true;
                                        cx.notify();
                                    })),
                            )
                        })
                        .when(self.is_default, |d| {
                            d.child(
                                label(
                                    text::METADATA,
                                    crate::i18n::text("app.settings.providers.default_badge"),
                                )
                                .text_color(r.primary),
                            )
                        })
                        .child(gpui_kit::div().flex_1())
                        .child(
                            Button::new("line-remove")
                                .danger()
                                .small()
                                .label(crate::i18n::text("app.settings.providers.remove_line"))
                                .disabled(!self.can_remove)
                                .tooltip(if self.can_remove {
                                    crate::i18n::text("app.settings.providers.remove_line")
                                } else {
                                    crate::i18n::text("app.settings.providers.last_line")
                                })
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.confirm_remove(window, cx)
                                })),
                        ),
                )
            })
    }
}

impl LineForm {
    fn confirm_remove(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(protocol) = self.editing else { return };
        let page = self.page.clone();
        let provider = self.provider.clone();
        window.close_dialog(cx);
        page.update(cx, |this, cx| {
            this.open_remove_line(provider, protocol, window, cx)
        });
    }
}

impl ProvidersPage {
    pub(in crate::panels::settings::providers) fn open_line_dialog(
        &mut self,
        editing: Option<Protocol>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(draft) = self.draft.as_ref() else {
            return;
        };
        let options: Vec<Protocol> = editing.into_iter().chain(free_protocols(draft)).collect();
        let Some(&protocol) = options.first() else {
            return;
        };
        let url = editing.map(|_| self.line_url()).unwrap_or_default();
        let headers = draft.headers.clone();
        let is_default = editing == Some(draft.default_protocol);
        let can_remove = draft.endpoints.len() > 1;
        let page = cx.entity();
        let Some(target) = self.selected.clone() else {
            return;
        };
        let form = cx.new(|cx| {
            let mut form = LineForm {
                provider: target.clone(),
                page: page.clone(),
                editing,
                options,
                protocol,
                is_default,
                can_remove,
                url: cx.new(|cx| {
                    InputState::new(window, cx)
                        .placeholder(crate::i18n::text("app.settings.providers.url_placeholder"))
                        .default_value(url)
                }),
                headers: Vec::new(),
                error: None,
                saving: false,
            };
            for header in headers {
                form.add_header(header, window, cx);
            }
            form
        });
        let title = if editing.is_some() {
            crate::i18n::text("app.settings.providers.line_settings")
        } else {
            crate::i18n::text("app.settings.providers.add_line")
        };
        open_form(
            window,
            cx,
            title,
            px(520.),
            form,
            crate::i18n::text("app.settings.providers.confirm"),
            move |form, window, cx| {
                if form.saving {
                    return false;
                }
                match form.result(cx) {
                    Ok((url, headers)) => {
                        let (from, to, is_default) = (form.editing, form.protocol, form.is_default);
                        let id = target.clone();
                        form.saving = true;
                        crate::panels::settings::providers::forms::save(
                            page.clone(),
                            window,
                            cx,
                            move |service| {
                                let provider = service.provider(&id)?;
                                let mut input = provider.input;
                                if let Some(from) = from {
                                    retarget_line(&mut input, from, to);
                                }
                                set_line(&mut input, to, url);
                                input.headers = headers;
                                if is_default {
                                    input.default_protocol = to;
                                }
                                service.update(&id, provider.revision, input)
                            },
                            false,
                            |form, error| {
                                form.saving = false;
                                form.error = Some(error);
                            },
                        );
                        false
                    }
                    Err(error) => {
                        form.error = Some(error.into());
                        cx.notify();
                        false
                    }
                }
            },
        );
    }

    fn open_remove_line(
        &mut self,
        provider: autocraft_core::ProviderId,
        protocol: Protocol,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let page = cx.entity();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let page = page.clone();
            let provider = provider.clone();
            alert
                .title(crate::i18n::text("app.settings.providers.remove_line"))
                .description(
                    crate::panels::settings::providers::labels::remove_line_confirm(protocol),
                )
                .ok_text(crate::i18n::text("app.settings.providers.delete"))
                .show_cancel(true)
                .on_ok(move |_, _, cx| {
                    let id = provider.clone();
                    page.update(cx, |this, cx| {
                        super::super::write(this, cx, move |service| {
                            let provider = service.provider(&id)?;
                            let mut input = provider.input;
                            remove_line(&mut input, protocol);
                            service.update(&id, provider.revision, input)
                        });
                    });
                    true
                })
        });
    }
}
