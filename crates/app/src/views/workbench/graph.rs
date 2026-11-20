//! Project the analysis engine's structured call graph without parsing diagram text.
use super::data;
use autocraft_ui::components::{CallGraphData, CallGraphEdge, CallGraphNode};
use std::collections::BTreeMap;
pub(super) fn graph(value: &serde_json::Value) -> Option<CallGraphData> {
    let root = value.get("root")?;
    let center = data::address(root)?;
    let mut nodes = BTreeMap::new();
    let node = |row: &serde_json::Value| {
        let address = data::address(row).unwrap_or_default();
        let name = data::name(row)
            .or_else(|| data::field(row, &["import"]))
            .unwrap_or_else(|| address.clone());
        let id = if address.is_empty() {
            format!("import:{name}")
        } else {
            address.clone()
        };
        CallGraphNode {
            id: id.into(),
            name: name.into(),
            address: address.into(),
        }
    };
    let root = node(root);
    nodes.insert(root.id.to_string(), root);
    for rows in value
        .get("levels")?
        .as_object()?
        .values()
        .filter_map(|v| v.as_array())
    {
        for row in rows {
            let n = node(row);
            nodes.insert(n.id.to_string(), n);
        }
    }
    let edges = value
        .get("edges")?
        .as_array()?
        .iter()
        .filter_map(|edge| {
            let from = edge.get("from")?.as_str()?.to_owned();
            let to = edge
                .get("to")
                .and_then(|v| v.as_str())
                .map(str::to_owned)
                .or_else(|| {
                    edge.get("to_import")
                        .and_then(|v| v.as_str())
                        .map(|s| format!("import:{s}"))
                })?;
            Some(CallGraphEdge {
                from: from.into(),
                to: to.into(),
            })
        })
        .collect();
    Some(CallGraphData {
        center: center.into(),
        nodes: nodes.into_values().collect(),
        edges,
    })
}
