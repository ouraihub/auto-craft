//! Selectable Markdown, collapsed reasoning/tool JSON and links to the project's evidence ledger.
use super::labels as s;
use autocraft_core::{ContentBlock, Message, Role, ToolStatus};
use autocraft_ui::{
    Roles,
    components::{ConfidenceLabels, EvidenceCard, EvidenceCardProps, JsonTree, label, markdown},
    metrics::{radius, space, text},
};
use gpui_kit::{
    AnyElement, App, ElementId, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, Window,
    base::{h_flex, v_flex},
    div,
    prelude::FluentBuilder as _,
};
use serde_json::Value;
use std::{
    collections::{BTreeSet, HashSet},
    rc::Rc,
    sync::Arc,
};
pub(super) type Toggle = Rc<dyn Fn(String, &mut Window, &mut App)>;
pub(super) type Evidence = Rc<dyn Fn(String, &mut Window, &mut App)>;

pub(super) fn message(
    m: &Message,
    open: &HashSet<String>,
    toggle: &Toggle,
    evidence: &Evidence,
    r: &Roles,
) -> AnyElement {
    let body = v_flex()
        .gap(space::S8)
        .children(m.content.iter().enumerate().map(|(index, block)| {
            let key = format!("{}/{index}", m.id);
            match block {
                ContentBlock::Text { text: body } => markdown(
                    ElementId::Name(format!("markdown-{key}").into()),
                    body.clone(),
                )
                .into_any_element(),
                ContentBlock::Thinking { text: body } => {
                    let is_open = open.contains(&key);
                    v_flex()
                        .gap(space::S4)
                        .child(disclosure(
                            &key,
                            format!("{} · {}", s::thinking(), body.chars().count()),
                            is_open,
                            toggle,
                            r,
                        ))
                        .when(is_open, |d| {
                            d.child(markdown(
                                ElementId::Name(format!("thinking-{key}").into()),
                                body.clone(),
                            ))
                        })
                        .into_any_element()
                }
                ContentBlock::Image { mime_type } => {
                    label(text::METADATA, format!("{} · {mime_type}", s::image()))
                        .text_color(r.text_subtle)
                        .into_any_element()
                }
                ContentBlock::Tool {
                    name,
                    status,
                    input,
                    output,
                    ..
                } => {
                    let is_open = open.contains(&key);
                    let color = match status {
                        ToolStatus::Running => r.feedback_info,
                        ToolStatus::Completed => r.feedback_success,
                        ToolStatus::Failed => r.feedback_error,
                    };
                    let parsed = output
                        .as_deref()
                        .and_then(|s| serde_json::from_str::<Value>(s).ok());
                    let record = parsed.as_ref().and_then(evidence_record);
                    let mut card = v_flex()
                        .gap(space::S6)
                        .p(space::S8)
                        .rounded(radius::MD)
                        .border_1()
                        .border_color(color)
                        .bg(r.container_elevated_bg)
                        .child(disclosure(&key, name.clone(), is_open, toggle, r));
                    if let Some((id, record)) = record {
                        let summary = record
                            .get("summary")
                            .and_then(Value::as_str)
                            .unwrap_or(name)
                            .to_owned();
                        let confidence = match record.get("confidence").and_then(Value::as_str) {
                            Some("Observed" | "observed") => {
                                Some(autocraft_core::Confidence::Observed)
                            }
                            Some("Derived" | "derived") => {
                                Some(autocraft_core::Confidence::Derived)
                            }
                            Some("Inferred" | "inferred") => {
                                Some(autocraft_core::Confidence::Inferred)
                            }
                            _ => None,
                        };
                        let go = evidence.clone();
                        let link_id = id.clone();
                        let props = EvidenceCardProps {
                            summary: summary.into(),
                            confidence,
                            confidence_labels: ConfidenceLabels {
                                observed: rust_i18n::t!("app.agent.observed").into_owned().into(),
                                derived: rust_i18n::t!("app.agent.derived").into_owned().into(),
                                inferred: rust_i18n::t!("app.agent.inferred").into_owned().into(),
                                unknown: rust_i18n::t!("app.agent.unknown").into_owned().into(),
                            },
                            fields: vec![(
                                rust_i18n::t!("app.agent.evidence").into_owned().into(),
                                id.into(),
                            )],
                            json: Arc::new(record),
                            expanded: is_open,
                            toggle_label: rust_i18n::t!("app.agent.open_evidence")
                                .into_owned()
                                .into(),
                            expanded_json: Arc::new(BTreeSet::from([String::new()])),
                        };
                        card = card.child(
                            EvidenceCard::new(
                                ElementId::Name(format!("evidence-{key}").into()),
                                props,
                            )
                            .on_toggle(move |_, window, cx| go(link_id.clone(), window, cx)),
                        );
                    }
                    if is_open {
                        if let Some(input) = input {
                            card = card
                                .child(label(text::CAPTION, s::tool_input()))
                                .child(json_tree(&format!("{key}/in"), input, open, toggle));
                        }
                        if let Some(output) = output {
                            card = card.child(label(text::CAPTION, s::tool_output()));
                            card = if let Some(parsed) = parsed {
                                card.child(json_tree(&format!("{key}/out"), &parsed, open, toggle))
                            } else {
                                card.child(label(text::CODE_SMALL, output.clone()))
                            };
                        }
                    }
                    card.into_any_element()
                }
            }
        }));
    match m.role {
        Role::User => h_flex()
            .justify_end()
            .child(
                div()
                    .max_w_4_5()
                    .p(space::S12)
                    .rounded(radius::LG)
                    .bg(r.bubble_user_bg)
                    .border_1()
                    .border_color(r.bubble_user_border)
                    .child(body),
            )
            .into_any_element(),
        Role::Assistant | Role::Tool => body.into_any_element(),
        Role::System => div()
            .text_color(r.text_muted)
            .child(body)
            .into_any_element(),
    }
}
fn disclosure(
    key: &str,
    title: String,
    open: bool,
    toggle: &Toggle,
    r: &Roles,
) -> impl IntoElement {
    let action = toggle.clone();
    let key = key.to_owned();
    div()
        .id(ElementId::Name(format!("toggle-{key}").into()))
        .cursor_pointer()
        .text_color(r.text_muted)
        .child(label(
            text::METADATA,
            format!("{} {title}", if open { "▾" } else { "▸" }),
        ))
        .on_click(move |_, window, cx| action(key.clone(), window, cx))
}
fn json_tree(key: &str, value: &Value, open: &HashSet<String>, toggle: &Toggle) -> JsonTree {
    let prefix = format!("json/{key}");
    let expanded = open
        .iter()
        .filter_map(|entry| entry.strip_prefix(&prefix).map(str::to_owned))
        .chain(std::iter::once(String::new()))
        .collect();
    let action = toggle.clone();
    JsonTree::new(
        ElementId::Name(prefix.clone().into()),
        Arc::new(value.clone()),
        Arc::new(expanded),
    )
    .on_expand(move |path, _, window, cx| action(format!("{prefix}{path}"), window, cx))
}
fn evidence_record(value: &Value) -> Option<(String, Value)> {
    if let Some(content) = value.get("structuredContent") {
        return evidence_record(content);
    }
    if let Some(id) = value
        .get("evidence_id")
        .or_else(|| value.pointer("/saved_evidence/evidence_id"))
        .or_else(|| {
            value
                .get("id")
                .filter(|id| id.as_str().is_some_and(|s| s.starts_with("ev-")))
        })
        .and_then(Value::as_str)
    {
        return Some((
            id.to_owned(),
            value
                .get("record")
                .or_else(|| value.get("evidence_summary"))
                .unwrap_or(value)
                .clone(),
        ));
    }
    if let Some(content) = value.get("content").and_then(Value::as_array) {
        for block in content {
            if let Some(body) = block
                .get("text")
                .and_then(Value::as_str)
                .and_then(|s| serde_json::from_str::<Value>(s).ok())
                && let Some(record) = evidence_record(&body)
            {
                return Some(record);
            }
        }
    }
    None
}
