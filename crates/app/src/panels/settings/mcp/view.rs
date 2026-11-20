//! Global MCP server rows, presets and import controls.

use autocraft_core::{McpServer, McpTransport};
use autocraft_ui::{
    AutocraftTheme,
    components::{IconButton, label},
    metrics::{space, text},
};
use gpui_kit::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _, Render, Styled as _, Window,
    assets::IconName,
    base::{h_flex, v_flex},
    component::{
        Disableable as _,
        button::{Button, ButtonVariants as _},
        input::Textarea,
        switch::Switch,
    },
    prelude::FluentBuilder as _,
    px,
};

use super::{McpPage, TransportKind, editor::set_input};

impl McpPage {
    fn row(
        &self,
        server: McpServer,
        enabled: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let r = AutocraftTheme::global(cx).roles;
        let id = server.id.clone();
        let edit = server.clone();
        let toggle = server.clone();
        let delete = server.clone();
        let transport = match &server.input.transport {
            McpTransport::Stdio { command, .. } => format!(
                "{} · {}",
                crate::i18n::text("app.settings.mcp.stdio"),
                command
            ),
            McpTransport::Http { url, .. } => {
                format!("{} · {}", crate::i18n::text("app.settings.mcp.http"), url)
            }
        };
        let confirming =
            self.confirm_delete.as_ref() == Some(&(server.id.clone(), server.revision));
        v_flex()
            .gap(space::S4)
            .py(space::S12)
            .border_b_1()
            .border_color(r.border_muted)
            .child(
                h_flex()
                    .id(format!("mcp-{id}"))
                    .gap(space::S8)
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w(px(0.))
                            .child(
                                h_flex()
                                    .gap(space::S6)
                                    .child(label(text::UI, server.input.name.clone()))
                                    .child(
                                        label(
                                            text::METADATA,
                                            crate::i18n::text("app.settings.mcp.global"),
                                        )
                                        .text_color(r.text_subtle),
                                    ),
                            )
                            .child(label(text::METADATA, transport).text_color(r.text_subtle))
                            .when_some(server.input.description.clone(), |d, description| {
                                d.child(label(text::METADATA, description).text_color(r.text_muted))
                            }),
                    )
                    .child(
                        Switch::new(format!("mcp-enabled-{id}"))
                            .checked(enabled)
                            .color(r.feedback_success)
                            .accessibility_label(server.input.name.clone())
                            .on_click(cx.listener(move |this, enabled: &bool, _, cx| {
                                this.toggle(toggle.clone(), *enabled, cx)
                            })),
                    )
                    .map(|d| {
                        d.child(
                            IconButton::new(format!("mcp-edit-{id}"), IconName::Pencil)
                                .tooltip(crate::i18n::text("app.settings.mcp.edit"))
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.begin_edit(Some(edit.clone()), window, cx)
                                })),
                        )
                        .child(if confirming {
                            Button::new(format!("mcp-confirm-delete-{id}"))
                                .danger()
                                .label(crate::i18n::text("app.settings.mcp.confirm_delete"))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.delete(delete.clone(), cx)
                                }))
                                .into_any_element()
                        } else {
                            IconButton::new(format!("mcp-delete-{id}"), IconName::Trash)
                                .tooltip(crate::i18n::text("app.settings.mcp.delete"))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.delete(delete.clone(), cx)
                                }))
                                .into_any_element()
                        })
                    }),
            )
            .when(confirming, |d| {
                d.child(
                    label(
                        text::METADATA,
                        rust_i18n::t!(
                            "app.settings.mcp.remove_server_warning",
                            name = server.input.name
                        )
                        .into_owned(),
                    )
                    .text_color(r.feedback_warning),
                )
            })
    }
}

impl Render for McpPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let placeholder = crate::i18n::text("app.settings.mcp.import_json");
        if self.import.read(cx).presentation().placeholder().as_ref() != placeholder.as_ref() {
            self.import.update(cx, |input, cx| {
                input.set_placeholder(placeholder, window, cx)
            });
        }
        let r = AutocraftTheme::global(cx).roles;
        let rows: Vec<_> = self
            .global
            .clone()
            .into_iter()
            .map(|server| {
                let enabled = server.input.enabled;
                self.row(server, enabled, cx).into_any_element()
            })
            .collect();
        let empty = rows.is_empty();
        v_flex()
            .id("settings-mcp")
            .gap(space::S12)
            .child(
                h_flex()
                    .child(
                        label(
                            text::HEADING_MD,
                            crate::i18n::text("app.settings.mcp.title"),
                        )
                        .flex_1(),
                    )
                    .child(
                        Button::new("mcp-add")
                            .primary()
                            .icon(IconName::Plus)
                            .label(crate::i18n::text("app.settings.mcp.add"))
                            .on_click(
                                cx.listener(|this, _, window, cx| {
                                    this.begin_edit(None, window, cx)
                                }),
                            ),
                    ),
            )
            .child(
                label(
                    text::METADATA,
                    crate::i18n::text("app.settings.mcp.description"),
                )
                .text_color(r.text_subtle),
            )
            .when(self.editor.is_some(), |d| d.child(self.editor(cx)))
            .children(rows)
            .when(empty && !self.loading, |d| {
                d.child(
                    label(text::METADATA, crate::i18n::text("app.settings.mcp.empty"))
                        .text_color(r.text_subtle),
                )
            })
            .child(
                h_flex().gap(space::S8).children(
                    [
                        (
                            crate::i18n::text("app.settings.mcp.context7"),
                            "https://mcp.context7.com/mcp",
                            "context7",
                        ),
                        (
                            crate::i18n::text("app.settings.mcp.deepwiki"),
                            "https://mcp.deepwiki.com/mcp",
                            "deepwiki",
                        ),
                    ]
                    .into_iter()
                    .map(|(title, url, name)| {
                        Button::new(format!("mcp-preset-{name}"))
                            .ghost()
                            .label(title)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.begin_edit(None, window, cx);
                                if let Some(editor) = &mut this.editor {
                                    editor.transport = TransportKind::Http;
                                    set_input(&editor.name, name.to_owned(), window, cx);
                                    set_input(&editor.url, url.to_owned(), window, cx);
                                }
                                cx.notify();
                            }))
                    }),
                ),
            )
            .child(
                v_flex()
                    .gap(space::S8)
                    .border_t_1()
                    .border_color(r.border_muted)
                    .pt(space::S12)
                    .child(label(
                        text::TITLE_SECTION,
                        crate::i18n::text("app.settings.mcp.import_json"),
                    ))
                    .child(Textarea::new(&self.import))
                    .child(
                        h_flex()
                            .gap(space::S8)
                            .child(
                                Button::new("mcp-preview")
                                    .ghost()
                                    .label(crate::i18n::text("app.settings.mcp.preview"))
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.preview_import(cx)),
                                    ),
                            )
                            .child(
                                Button::new("mcp-import")
                                    .primary()
                                    .label(crate::i18n::text("app.settings.mcp.import"))
                                    .disabled(self.importing)
                                    .on_click(cx.listener(|this, _, _, cx| this.import_valid(cx))),
                            ),
                    )
                    .when_some(self.import_preview.as_ref(), |d, preview| {
                        d.child(label(
                            text::METADATA,
                            rust_i18n::t!(
                                "app.settings.mcp.import_summary",
                                valid = preview.valid.len(),
                                invalid = preview.invalid
                            )
                            .into_owned(),
                        ))
                        .children(
                            preview
                                .valid
                                .iter()
                                .map(|input| label(text::METADATA, input.name.clone())),
                        )
                    }),
            )
            .when_some(self.error, |d, error| {
                d.child(
                    label(text::METADATA, crate::i18n::text(error)).text_color(r.feedback_error),
                )
            })
    }
}
