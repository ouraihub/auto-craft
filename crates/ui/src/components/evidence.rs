//! Evidence projections and controlled, independently expandable JSON trees.
use super::{ConfidenceLabels, ConfidencePill, Handler, TreeRow, label, roles};
use crate::metrics::{radius, space, text};
use autocraft_core::Confidence;
use gpui_kit::{
    App, ElementId, InteractiveElement, IntoElement, ParentElement, RenderOnce, SharedString,
    StatefulInteractiveElement, Styled, Window, div, prelude::FluentBuilder as _,
};
use serde_json::Value;
use std::{collections::BTreeSet, rc::Rc, sync::Arc};

type ExpandHandler = Rc<dyn Fn(&str, bool, &mut Window, &mut App)>;
/// Display-only evidence projection. Labels and metadata are already localized by the caller.
#[derive(Debug, Clone)]
pub struct EvidenceCardProps {
    pub summary: SharedString,
    pub confidence: Option<Confidence>,
    pub confidence_labels: ConfidenceLabels,
    /// Coverage, location, limitations, source tool and mode, in presentation order.
    pub fields: Vec<(SharedString, SharedString)>,
    pub json: Arc<Value>,
    pub expanded: bool,
    pub toggle_label: SharedString,
    /// Expanded JSON pointers; the empty pointer refers to the root.
    pub expanded_json: Arc<BTreeSet<String>>,
}
/// Controlled evidence card: owners retain expansion state and backend access.
#[derive(IntoElement)]
pub struct EvidenceCard {
    id: ElementId,
    props: EvidenceCardProps,
    on_toggle: Option<Handler>,
    on_json_expand: Option<ExpandHandler>,
}
impl EvidenceCard {
    pub fn new(id: impl Into<ElementId>, props: EvidenceCardProps) -> Self {
        Self {
            id: id.into(),
            props,
            on_toggle: None,
            on_json_expand: None,
        }
    }
    pub fn on_toggle(
        mut self,
        handler: impl Fn(&gpui_kit::ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_toggle = Some(Box::new(handler));
        self
    }
    pub fn on_json_expand(
        mut self,
        handler: impl Fn(&str, bool, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_json_expand = Some(Rc::new(handler));
        self
    }
}
impl RenderOnce for EvidenceCard {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let r = roles(cx);
        let props = self.props;
        let mut card = div()
            .id(self.id.clone())
            .flex()
            .flex_col()
            .gap(space::S8)
            .p(space::S12)
            .rounded(radius::SM)
            .border_1()
            .border_color(r.border_default)
            .bg(r.container_content_bg)
            .text_color(r.text_default)
            .child(label(text::TITLE_CARD, props.summary))
            .child(ConfidencePill::new(
                props.confidence,
                props.confidence_labels,
            ));
        for (name, value) in props.fields {
            card = card.child(
                div()
                    .flex()
                    .gap(space::S8)
                    .child(label(text::METADATA, name).text_color(r.text_subtle))
                    .child(label(text::UI, value)),
            );
        }
        card = card.child(
            div()
                .id((self.id.clone(), "expand"))
                .cursor_pointer()
                .text_color(r.text_link)
                .child(label(text::ACTION, props.toggle_label))
                .when_some(self.on_toggle, |d, handler| d.on_click(handler)),
        );
        if props.expanded {
            let mut tree = JsonTree::new((self.id, "json"), props.json, props.expanded_json);
            tree.on_expand = self.on_json_expand;
            card = card.child(tree);
        }
        card
    }
}
/// JSON tree using RFC 6901 pointers as stable expansion identities.
#[derive(IntoElement)]
pub struct JsonTree {
    id: ElementId,
    value: Arc<Value>,
    expanded: Arc<BTreeSet<String>>,
    on_expand: Option<ExpandHandler>,
}
impl JsonTree {
    pub fn new(
        id: impl Into<ElementId>,
        value: Arc<Value>,
        expanded: Arc<BTreeSet<String>>,
    ) -> Self {
        Self {
            id: id.into(),
            value,
            expanded,
            on_expand: None,
        }
    }
    pub fn on_expand(
        mut self,
        handler: impl Fn(&str, bool, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_expand = Some(Rc::new(handler));
        self
    }
}
impl RenderOnce for JsonTree {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let mut rows = Vec::new();
        append_rows(&self.value, "", "", 0, &self.expanded, &mut rows);
        div()
            .id(self.id.clone())
            .flex()
            .flex_col()
            .children(rows.into_iter().enumerate().map(|(index, row)| {
                let callback = self.on_expand.clone();
                let path = row.path;
                let open = row.open;
                TreeRow::new((self.id.clone(), index.to_string()), row.label)
                    .depth(row.depth)
                    .expanded(open)
                    .on_click(move |_, window, cx| {
                        if let (Some(callback), Some(open)) = (&callback, open) {
                            callback(&path, !open, window, cx);
                        }
                    })
            }))
    }
}
struct JsonRow {
    path: String,
    label: SharedString,
    depth: usize,
    open: Option<bool>,
}
fn append_rows(
    value: &Value,
    path: &str,
    key: &str,
    depth: usize,
    expanded: &BTreeSet<String>,
    rows: &mut Vec<JsonRow>,
) {
    let (suffix, count) = match value {
        Value::Array(values) => ("[]".to_owned(), Some(values.len())),
        Value::Object(values) => ("{}".to_owned(), Some(values.len())),
        other => (other.to_string(), None),
    };
    let open = count.map(|_| expanded.contains(path));
    let label = if let Some(count) = count {
        format!("{key} {suffix} ({count})")
    } else if key.is_empty() {
        suffix
    } else {
        format!("{key}: {suffix}")
    };
    rows.push(JsonRow {
        path: path.to_owned(),
        label: label.into(),
        depth,
        open,
    });
    if open != Some(true) {
        return;
    }
    match value {
        Value::Array(values) => {
            for (index, value) in values.iter().enumerate() {
                append_rows(
                    value,
                    &format!("{path}/{index}"),
                    &index.to_string(),
                    depth + 1,
                    expanded,
                    rows,
                );
            }
        }
        Value::Object(values) => {
            for (key, value) in values {
                let escaped = key.replace('~', "~0").replace('/', "~1");
                append_rows(
                    value,
                    &format!("{path}/{escaped}"),
                    key,
                    depth + 1,
                    expanded,
                    rows,
                );
            }
        }
        _ => {}
    }
}
