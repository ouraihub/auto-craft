//! Deterministic bounded caller/callee layers, independent of GPUI state.
use super::CallGraphData;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

pub(super) const NODE_WIDTH: f32 = 184.;
pub(super) const NODE_HEIGHT: f32 = 48.;
const COLUMN: f32 = 208.;
const LAYER: f32 = 112.;
#[derive(Debug)]
pub(super) struct Node {
    pub source: Option<usize>,
    pub hidden: usize,
    pub x: f32,
    pub y: f32,
}
#[derive(Debug)]
pub(super) struct Layout {
    pub nodes: Vec<Node>,
    pub edges: Vec<(usize, usize)>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LayoutError {
    TooLarge,
    MissingCenter,
}
pub(super) fn layout(data: &CallGraphData) -> Result<Layout, LayoutError> {
    if data.nodes.len() > 500 {
        return Err(LayoutError::TooLarge);
    }
    let ids: BTreeMap<&str, usize> = data
        .nodes
        .iter()
        .enumerate()
        .map(|(i, node)| (node.id.as_ref(), i))
        .collect();
    let Some(&center) = ids.get(data.center.as_ref()) else {
        return Err(LayoutError::MissingCenter);
    };
    let mut outgoing = vec![Vec::new(); data.nodes.len()];
    let mut incoming = outgoing.clone();
    let mut edges = BTreeSet::new();
    for edge in &data.edges {
        if let (Some(&from), Some(&to)) = (ids.get(edge.from.as_ref()), ids.get(edge.to.as_ref()))
            && edges.insert((from, to))
        {
            outgoing[from].push(to);
            incoming[to].push(from);
        }
    }
    let callers = distances(center, &incoming);
    let callees = distances(center, &outgoing);
    let mut layers: BTreeMap<i32, Vec<usize>> = BTreeMap::new();
    let mut hidden: BTreeMap<i32, usize> = BTreeMap::new();
    layers.insert(0, vec![center]);
    for index in 0..data.nodes.len() {
        if index == center {
            continue;
        }
        let layer = match (callers[index], callees[index]) {
            (Some(up), Some(down)) => {
                if up <= down {
                    -(up as i32)
                } else {
                    down as i32
                }
            }
            (Some(up), None) => -(up as i32),
            (None, Some(down)) => down as i32,
            (None, None) => continue,
        };
        if layer.abs() > 3 {
            *hidden.entry(layer.signum() * 3).or_default() += 1;
        } else {
            layers.entry(layer).or_default().push(index);
        }
    }
    for (&layer, nodes) in &mut layers {
        nodes.sort_by(|&a, &b| {
            data.nodes[a]
                .name
                .cmp(&data.nodes[b].name)
                .then(data.nodes[a].id.cmp(&data.nodes[b].id))
        });
        let limit = if nodes.len() > 40 || hidden.get(&layer).copied().unwrap_or_default() > 0 {
            39
        } else {
            40
        };
        if nodes.len() > limit {
            *hidden.entry(layer).or_default() += nodes.len() - limit;
            nodes.truncate(limit);
        }
    }
    // One outward sweep: names break equal barycenters deterministically.
    for layer in [-1_i32, -2, -3, 1, 2, 3] {
        let neighbor_layer = layer - layer.signum();
        let positions: BTreeMap<usize, usize> = layers
            .get(&neighbor_layer)
            .into_iter()
            .flatten()
            .enumerate()
            .map(|(rank, &node)| (node, rank))
            .collect();
        if let Some(nodes) = layers.get_mut(&layer) {
            let barycenter = |node: usize| {
                let adjacent = if layer < 0 {
                    &outgoing[node]
                } else {
                    &incoming[node]
                };
                let (sum, count) = adjacent
                    .iter()
                    .filter_map(|id| positions.get(id))
                    .fold((0, 0), |(sum, count), rank| (sum + rank, count + 1));
                if count == 0 {
                    f64::INFINITY
                } else {
                    sum as f64 / count as f64
                }
            };
            nodes.sort_by(|&a, &b| {
                barycenter(a)
                    .total_cmp(&barycenter(b))
                    .then(data.nodes[a].name.cmp(&data.nodes[b].name))
                    .then(data.nodes[a].id.cmp(&data.nodes[b].id))
            });
        }
    }
    let mut result = Layout {
        nodes: Vec::new(),
        edges: Vec::new(),
    };
    let mut visible = BTreeMap::new();
    for layer in -3..=3 {
        let nodes = layers.get(&layer).map(Vec::as_slice).unwrap_or_default();
        let count = hidden.get(&layer).copied().unwrap_or_default();
        let width = nodes.len() + usize::from(count > 0);
        for (column, &index) in nodes.iter().enumerate() {
            visible.insert(index, result.nodes.len());
            result.nodes.push(Node {
                source: Some(index),
                hidden: 0,
                x: (column as f32 - (width.saturating_sub(1)) as f32 / 2.) * COLUMN,
                y: layer as f32 * LAYER,
            });
        }
        if count > 0 {
            result.nodes.push(Node {
                source: None,
                hidden: count,
                x: (width.saturating_sub(1)) as f32 / 2. * COLUMN,
                y: layer as f32 * LAYER,
            });
        }
    }
    for (from, to) in edges {
        if let (Some(&from), Some(&to)) = (visible.get(&from), visible.get(&to)) {
            result.edges.push((from, to));
        }
    }
    Ok(result)
}
fn distances(center: usize, adjacent: &[Vec<usize>]) -> Vec<Option<usize>> {
    let mut result = vec![None; adjacent.len()];
    result[center] = Some(0);
    let mut queue = VecDeque::from([(center, 0)]);
    while let Some((node, depth)) = queue.pop_front() {
        for &next in &adjacent[node] {
            if result[next].is_none() {
                result[next] = Some(depth + 1);
                queue.push_back((next, depth + 1));
            }
        }
    }
    result
}
// Keep recursive calls outside the node fill, which is painted over the edges.
pub(super) fn self_loop_path(node: &Node) -> [(f32, f32); 5] {
    let top = node.y - NODE_HEIGHT / 2.;
    let right = node.x + NODE_WIDTH / 2.;
    [
        (node.x, top),
        (node.x, top - 20.),
        (right + 12., top - 20.),
        (right + 12., node.y),
        (right, node.y),
    ]
}
#[cfg(test)]
mod tests {
    use super::{LayoutError, NODE_HEIGHT, NODE_WIDTH, layout, self_loop_path};
    use crate::components::{CallGraphData, CallGraphEdge, CallGraphNode};
    fn node(index: usize) -> CallGraphNode {
        CallGraphNode {
            id: index.to_string().into(),
            name: format!("f{index:03}").into(),
            address: index.to_string().into(),
        }
    }
    #[test]
    fn caps_layers_and_collapses_deep_cycles() {
        let mut data = CallGraphData {
            center: "0".into(),
            nodes: (0..48).map(node).collect(),
            edges: Vec::new(),
        };
        for index in 1..44 {
            data.edges.push(CallGraphEdge {
                from: "0".into(),
                to: index.to_string().into(),
            });
        }
        for index in 43..47 {
            data.edges.push(CallGraphEdge {
                from: index.to_string().into(),
                to: (index + 1).to_string().into(),
            });
        }
        data.edges.push(CallGraphEdge {
            from: "47".into(),
            to: "0".into(),
        });
        let view = layout(&data).unwrap();
        assert!(view.nodes.iter().all(|node| node.y.abs() <= 336.));
        assert!(view.nodes.iter().any(|node| node.hidden >= 3));
        assert_eq!(
            view.nodes
                .iter()
                .filter(|node| node.y == 112. && node.source.is_some())
                .count(),
            39
        );
        data.nodes = (0..501).map(node).collect();
        assert_eq!(layout(&data).unwrap_err(), LayoutError::TooLarge);
    }
    #[test]
    fn adjacent_layer_barycenter_overrides_name_order() {
        let mut data = CallGraphData {
            center: "0".into(),
            nodes: (0..5).map(node).collect(),
            edges: Vec::new(),
        };
        for (from, to) in [(0, 1), (0, 2), (1, 4), (2, 3)] {
            data.edges.push(CallGraphEdge {
                from: from.to_string().into(),
                to: to.to_string().into(),
            });
        }
        let view = layout(&data).unwrap();
        let x = |source| {
            view.nodes
                .iter()
                .find(|node| node.source == Some(source))
                .unwrap()
                .x
        };
        assert!(x(1) < x(2));
        assert!(x(4) < x(3));
    }
    #[test]
    fn self_recursive_edge_remains_visible_outside_node_fill() {
        let data = CallGraphData {
            center: "0".into(),
            nodes: vec![node(0)],
            edges: vec![CallGraphEdge {
                from: "0".into(),
                to: "0".into(),
            }],
        };
        let view = layout(&data).unwrap();
        assert_eq!(view.edges, vec![(0, 0)]);
        let node = &view.nodes[0];
        assert_eq!((node.x, node.y), (0., 0.));
        let path = self_loop_path(node);
        assert_eq!(path[0], (node.x, node.y - NODE_HEIGHT / 2.));
        assert_eq!(path[4], (node.x + NODE_WIDTH / 2., node.y));
        for segment in path.windows(2) {
            let (x, y) = (
                (segment[0].0 + segment[1].0) / 2.,
                (segment[0].1 + segment[1].1) / 2.,
            );
            assert!(x > node.x + NODE_WIDTH / 2. || y < node.y - NODE_HEIGHT / 2.);
            assert!(segment[0].0 == segment[1].0 || segment[0].1 == segment[1].1);
        }
    }
}
