//! Cached text/hex/graph projections and exhaustive editor-state rendering.
use super::{
    Fill, WorkbenchView,
    data::{self, EditorTab},
};
use autocraft_core::{Confidence, ToolReply};
use autocraft_ui::components::{
    AddressRange, CallGraph, CallGraphLabels, CodeDocument, CodeLanguage, CodeView, ConfidencePill,
    HexView,
};
use gpui_kit::{
    AnyElement, Context, IntoElement, ParentElement as _, Styled as _,
    base::{h_flex, v_flex},
    component::{
        button::Button,
        menu::{DropdownMenu as _, PopupMenuItem},
        table::DataTable,
    },
};
use std::{rc::Rc, sync::Arc};
impl WorkbenchView {
    pub(super) fn install_reply(&mut self, reply: ToolReply, cx: &mut Context<Self>) {
        if self.tab == EditorTab::Results {
            self.result_reply = Some(reply.clone());
        }
        self.document = data::code(&reply, self.tab).map(|source| {
            let language = match self.tab {
                EditorTab::Pseudocode => CodeLanguage::C,
                EditorTab::Disassembly => CodeLanguage::Disassembly,
                EditorTab::Source => CodeLanguage::JavaScript,
                EditorTab::Hex | EditorTab::Graph | EditorTab::Network | EditorTab::Results => {
                    CodeLanguage::Plain
                }
            };
            let addresses = if self.tab == EditorTab::Disassembly {
                source
                    .lines()
                    .enumerate()
                    .filter_map(|(line, text)| {
                        let start = text.len() - text.trim_start().len();
                        let end = text[start..].find(':').map(|end| start + end)?;
                        let address = text.get(start..end)?;
                        address
                            .trim_start_matches("0x")
                            .chars()
                            .all(|c| c.is_ascii_hexdigit())
                            .then(|| AddressRange {
                                line,
                                bytes: start..end,
                                address: address.to_owned().into(),
                            })
                    })
                    .collect()
            } else {
                Vec::new()
            };
            Rc::new(CodeDocument::new(source, language, addresses))
        });
        self.graph = super::graph::graph(data::result(&reply)).map(Rc::new);
        self.bytes = data::result(&reply)
            .get("hex")
            .and_then(|v| v.as_str())
            .and_then(decode_hex)
            .map(Arc::from);
        self.byte_offset = data::result(&reply)
            .get("address")
            .and_then(|v| v.as_str())
            .and_then(|s| u64::from_str_radix(s.trim_start_matches("0x"), 16).ok())
            .unwrap_or(0);
        self.byte_selection = None;
        let rows = data::rows(&reply);
        self.network.update(cx, |table, cx| {
            table.delegate_mut().clear();
            table.delegate_mut().append(rows, None);
            cx.notify();
        });
        self.fill = Fill::Ready(reply);
        cx.notify();
    }
    pub(super) fn render_fill(&self, fill: &Fill, cx: &mut Context<Self>) -> AnyElement {
        match fill {
            Fill::Idle => v_flex()
                .p_3()
                .child(rust_i18n::t!("app.workbench.select_item"))
                .into_any_element(),
            Fill::Loading => v_flex()
                .p_3()
                .child(rust_i18n::t!("app.workbench.loading"))
                .into_any_element(),
            Fill::Unavailable(reason) => v_flex().p_3().child(reason.clone()).into_any_element(),
            Fill::DeepNeeded => v_flex()
                .p_3()
                .gap_2()
                .child(rust_i18n::t!("app.workbench.deep_needed"))
                .child(
                    Button::new("request-deep")
                        .label(rust_i18n::t!("app.workbench.switch_deep"))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.confirm_deep = true;
                            cx.notify();
                        })),
                )
                .into_any_element(),
            Fill::EngineMissing { engine, fix } => v_flex()
                .p_3()
                .child(engine.clone())
                .child(fix.clone())
                .into_any_element(),
            Fill::Error { message, fix } => v_flex()
                .p_3()
                .child(message.clone())
                .children(fix.clone())
                .child(
                    Button::new("retry-editor")
                        .label(rust_i18n::t!("app.workbench.retry"))
                        .on_click(cx.listener(|this, _, _, cx| this.load_editor(cx))),
                )
                .into_any_element(),
            Fill::Ready(_) => v_flex().into_any_element(),
        }
    }
    pub(super) fn render_editor(&self, cx: &mut Context<Self>) -> AnyElement {
        let tabs = h_flex()
            .h(gpui_kit::px(28.))
            .overflow_hidden()
            .children(EditorTab::ALL.into_iter().map(|tab| {
                Button::new(tab.key())
                    .label(rust_i18n::t!(format!("app.workbench.{}", tab.key())))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.tab = tab;
                        this.load_editor(cx);
                    }))
            }))
            .child(
                Button::new("toggle-bottom")
                    .label(rust_i18n::t!("app.workbench.toggle_bottom"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.bottom_open = !this.bottom_open;
                        cx.notify();
                    })),
            );
        let mut body = v_flex().size_full().gap_2().child(tabs);
        if let Some((tool, _, _)) = &self.pending {
            body = body.child(
                v_flex()
                    .p_3()
                    .gap_2()
                    .child(rust_i18n::t!("app.workbench.consent"))
                    .child(tool.title.clone())
                    .child(tool.summary.clone())
                    .child(tool.effects.join(" · "))
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Button::new("consent-run")
                                    .label(rust_i18n::t!("app.workbench.allow_once"))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        if let Some((tool, params, list)) = this.pending.take() {
                                            this.request(tool, params, list, true, cx);
                                        }
                                    })),
                            )
                            .child(
                                Button::new("consent-cancel")
                                    .label(rust_i18n::t!("app.workbench.cancel"))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.pending = None;
                                        cx.notify();
                                    })),
                            ),
                    ),
            );
        }
        if self.confirm_deep {
            body = body.child(
                v_flex()
                    .p_3()
                    .gap_2()
                    .child(rust_i18n::t!("app.workbench.deep_warning"))
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Button::new("confirm-deep")
                                    .label(rust_i18n::t!("app.workbench.switch_deep"))
                                    .on_click(cx.listener(|this, _, _, cx| this.set_deep(cx))),
                            )
                            .child(
                                Button::new("cancel-deep")
                                    .label(rust_i18n::t!("app.workbench.cancel"))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.confirm_deep = false;
                                        cx.notify();
                                    })),
                            ),
                    ),
            );
        }
        if let Fill::Ready(reply) = &self.fill {
            let value = data::body(reply);
            let confidence =
                value
                    .get("confidence")
                    .and_then(|v| v.as_str())
                    .and_then(|v| match v {
                        "observed" => Some(Confidence::Observed),
                        "derived" => Some(Confidence::Derived),
                        "inferred" => Some(Confidence::Inferred),
                        _ => None,
                    });
            let mut banner = h_flex()
                .gap_2()
                .min_w_0()
                .child(ConfidencePill::new(
                    confidence,
                    super::inspector::confidence_labels(),
                ))
                .child(
                    v_flex().flex_1().min_w_0().overflow_hidden().child(
                        value
                            .get("summary")
                            .and_then(|v| v.as_str())
                            .unwrap_or_default()
                            .to_owned(),
                    ),
                )
                .child(
                    value
                        .get("operation")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_owned(),
                );
            if let ToolReply::Evidence { id: Some(id), .. } = reply {
                let id = id.clone();
                banner = banner.child(
                    Button::new("result-evidence")
                        .label(id.as_str().to_owned())
                        .on_click(
                            cx.listener(move |this, _, _, cx| this.show_evidence(id.clone(), cx)),
                        ),
                );
            }
            body = body.child(banner);
            if self.tab == EditorTab::Source
                && let Some(directory) = data::result(reply)
                    .get("output_directory")
                    .and_then(|v| v.as_str())
                && let Some(outputs) = data::result(reply)
                    .get("outputs")
                    .and_then(|v| v.as_array())
            {
                let directory = directory.to_owned();
                let paths: Vec<_> = outputs
                    .iter()
                    .filter_map(|output| output.get("path").and_then(|v| v.as_str()))
                    .map(str::to_owned)
                    .collect();
                let owner = cx.entity();
                body = body.child(
                    Button::new("restored-source")
                        .label(rust_i18n::t!("app.workbench.source"))
                        .icon(gpui_kit::assets::IconName::ChevronDown)
                        .dropdown_menu(move |menu, _, _| {
                            paths.iter().fold(menu, |menu, path| {
                                let (owner, directory, path) =
                                    (owner.clone(), directory.clone(), path.clone());
                                menu.item(PopupMenuItem::new(path.clone()).on_click(
                                    move |_, _, cx| {
                                        owner.update(cx, |this, cx| {
                                            this.load_source(directory.clone(), path.clone(), cx)
                                        });
                                    },
                                ))
                            })
                        }),
                );
            }
            let code_owner = cx.entity();
            let hex_owner = cx.entity();
            let graph_owner = cx.entity();
            let recenter_owner = cx.entity();
            let content = if let Some(document) = self
                .document
                .as_ref()
                .filter(|_| self.tab != EditorTab::Graph || self.graph.is_none())
            {
                CodeView::new("analysis-code", document.clone())
                    .on_address(move |address, _, cx| {
                        code_owner.update(cx, |this, cx| this.jump(address, cx))
                    })
                    .into_any_element()
            } else if self.tab == EditorTab::Hex {
                if let Some(bytes) = &self.bytes {
                    HexView::new(
                        "analysis-hex",
                        bytes.clone(),
                        self.byte_offset,
                        self.byte_selection.clone(),
                    )
                    .on_select(move |range, _, cx| {
                        hex_owner.update(cx, |this, cx| {
                            this.byte_selection = Some(range);
                            cx.notify();
                        })
                    })
                    .into_any_element()
                } else {
                    self.json(data::result(reply).clone(), cx)
                }
            } else if self.tab == EditorTab::Graph {
                if let Some(graph) = &self.graph {
                    CallGraph::new(
                        "analysis-graph",
                        graph.clone(),
                        self.viewport.clone(),
                        CallGraphLabels {
                            too_large: rust_i18n::t!("app.workbench.graph_large")
                                .into_owned()
                                .into(),
                            missing_center: rust_i18n::t!("app.workbench.graph_missing")
                                .into_owned()
                                .into(),
                            more: Rc::new(|count| {
                                rust_i18n::t!("app.workbench.graph_more", count = count)
                                    .into_owned()
                                    .into()
                            }),
                        },
                    )
                    .on_select(move |node, _, cx| {
                        graph_owner.update(cx, |this, cx| {
                            if !node.address.is_empty() {
                                this.jump(node.address.as_ref(), cx);
                            }
                        })
                    })
                    .on_recenter(move |node, _, cx| {
                        recenter_owner.update(cx, |this, cx| {
                            if !node.address.is_empty() {
                                this.viewport.reset();
                                this.select_address(node.address.to_string(), EditorTab::Graph, cx);
                            }
                        })
                    })
                    .into_any_element()
                } else {
                    self.json(data::result(reply).clone(), cx)
                }
            } else if self.tab == EditorTab::Network {
                DataTable::new(&self.network)
                    .stripe(true)
                    .into_any_element()
            } else {
                self.json(data::result(reply).clone(), cx)
            };
            body = body.child(v_flex().flex_1().min_h_0().child(content));
        } else {
            body = body.child(self.render_fill(&self.fill, cx));
        }
        body.into_any_element()
    }
}
fn decode_hex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    text.as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| {
            let a = char::from(pair[0]).to_digit(16)?;
            let b = char::from(pair[1]).to_digit(16)?;
            Some((a * 16 + b) as u8)
        })
        .collect()
}
