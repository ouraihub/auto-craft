//! Structured call graph canvas; visual viewport state has no backend knowledge.
mod layout;
#[cfg(test)]
mod tests;
use super::{TextStyled as _, label, roles};
use crate::metrics::{radius, space, text};
use gpui_kit::{
    App, Bounds, ElementId, InteractiveElement, IntoElement, MouseButton, ParentElement,
    PathBuilder, Pixels, Point, RenderOnce, ScrollDelta, SharedString, StatefulInteractiveElement,
    Styled, Window, canvas, div, fill, point, px, size,
};
use layout::{NODE_HEIGHT, NODE_WIDTH};
use std::{cell::RefCell, rc::Rc};

/// A function identity supplied by a structured tool response.
#[derive(Debug, Clone)]
pub struct CallGraphNode {
    pub id: SharedString,
    pub name: SharedString,
    pub address: SharedString,
}
/// A directed caller-to-callee edge using node identities, not names.
#[derive(Debug, Clone)]
pub struct CallGraphEdge {
    pub from: SharedString,
    pub to: SharedString,
}
/// Structured graph only: DOT/Mermaid are never parsed by this component.
#[derive(Debug, Clone)]
pub struct CallGraphData {
    pub center: SharedString,
    pub nodes: Vec<CallGraphNode>,
    pub edges: Vec<CallGraphEdge>,
}
/// Localized empty/limit labels and a localized “N more” formatter.
#[derive(Clone)]
pub struct CallGraphLabels {
    pub too_large: SharedString,
    pub missing_center: SharedString,
    pub more: Rc<dyn Fn(usize) -> SharedString>,
}
type CachedLayout = (
    std::rc::Weak<CallGraphData>,
    Result<Rc<layout::Layout>, layout::LayoutError>,
);
#[derive(Debug)]
struct ViewportState {
    pan: Point<Pixels>,
    zoom: f32,
    drag: Option<(Point<Pixels>, Point<Pixels>)>,
    layout: Option<CachedLayout>,
    select_task: Option<gpui_kit::Task<()>>,
}
/// Window-owned, cloneable visual pan/zoom. Never store this in analysis services.
#[derive(Debug, Clone)]
pub struct GraphViewport(Rc<RefCell<ViewportState>>);
impl Default for GraphViewport {
    fn default() -> Self {
        Self(Rc::new(RefCell::new(ViewportState {
            pan: point(px(0.), px(0.)),
            zoom: 1.,
            drag: None,
            layout: None,
            select_task: None,
        })))
    }
}
impl GraphViewport {
    pub fn reset(&self) {
        let mut state = self.0.borrow_mut();
        state.pan = point(px(0.), px(0.));
        state.zoom = 1.;
        state.drag = None;
        state.select_task = None;
    }
}
type NodeHandler = Rc<dyn Fn(&CallGraphNode, &mut Window, &mut App)>;
/// Bounded layered canvas with drag pan, pointer-centered zoom and navigation callbacks.
#[derive(IntoElement)]
pub struct CallGraph {
    id: ElementId,
    data: Rc<CallGraphData>,
    viewport: GraphViewport,
    labels: CallGraphLabels,
    on_select: Option<NodeHandler>,
    on_recenter: Option<NodeHandler>,
}
impl CallGraph {
    pub fn new(
        id: impl Into<ElementId>,
        data: Rc<CallGraphData>,
        viewport: GraphViewport,
        labels: CallGraphLabels,
    ) -> Self {
        Self {
            id: id.into(),
            data,
            viewport,
            labels,
            on_select: None,
            on_recenter: None,
        }
    }
    /// Select this function and open pseudocode in the owning workbench.
    pub fn on_select(
        mut self,
        handler: impl Fn(&CallGraphNode, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_select = Some(Rc::new(handler));
        self
    }
    /// Double-click intent: the owner fetches a new graph centered on this function.
    pub fn on_recenter(
        mut self,
        handler: impl Fn(&CallGraphNode, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_recenter = Some(Rc::new(handler));
        self
    }
}
impl RenderOnce for CallGraph {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let r = roles(cx);
        let mut state = self.viewport.0.borrow_mut();
        let calculated = match &state.layout {
            Some((data, graph)) if data.ptr_eq(&Rc::downgrade(&self.data)) => graph.clone(),
            _ => {
                let graph = layout::layout(&self.data).map(Rc::new);
                state.layout = Some((Rc::downgrade(&self.data), graph.clone()));
                graph
            }
        };
        drop(state);
        let graph = match calculated {
            Ok(graph) => graph,
            Err(error) => {
                return div()
                    .id(self.id)
                    .child(label(
                        text::UI,
                        match error {
                            layout::LayoutError::TooLarge => self.labels.too_large,
                            layout::LayoutError::MissingCenter => self.labels.missing_center,
                        },
                    ))
                    .into_any_element();
            }
        };
        let pan = self.viewport.0.borrow().pan;
        let zoom = self.viewport.0.borrow().zoom;
        let origin = Rc::new(RefCell::new(point(px(0.), px(0.))));
        let paint_graph = graph.clone();
        let canvas_origin = origin.clone();
        let down = self.viewport.clone();
        let movement = self.viewport.clone();
        let up = self.viewport.clone();
        let scroll = self.viewport.clone();
        let scroll_origin = origin.clone();
        let nodes = graph
            .nodes
            .iter()
            .enumerate()
            .map(|(index, placed)| {
                let source = placed.source.map(|source| self.data.nodes[source].clone());
                let title = source
                    .as_ref()
                    .map(|node| node.name.clone())
                    .unwrap_or_else(|| (self.labels.more)(placed.hidden));
                let address = source.as_ref().map(|node| node.address.clone());
                let select = self.on_select.clone();
                let recenter = self.on_recenter.clone();
                let clicks = self.viewport.clone();
                let click_data = Rc::downgrade(&self.data);
                div()
                    .id(index)
                    .absolute()
                    .left(gpui_kit::relative(0.5))
                    .top(gpui_kit::relative(0.5))
                    .ml(pan.x + px((placed.x - NODE_WIDTH / 2.) * zoom))
                    .mt(pan.y + px((placed.y - NODE_HEIGHT / 2.) * zoom))
                    .w(px(NODE_WIDTH * zoom))
                    .h(px(NODE_HEIGHT * zoom))
                    .px(space::S8 * zoom)
                    .py(space::S4 * zoom)
                    .overflow_hidden()
                    .rounded(radius::SM)
                    .cursor_pointer()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(move |event, window, cx| {
                        cx.stop_propagation();
                        clicks.0.borrow_mut().select_task = None;
                        if let Some(source) = &source {
                            if event.click_count() >= 2 {
                                if let Some(callback) = &recenter {
                                    callback(source, window, cx);
                                }
                            } else if let Some(callback) = select.clone() {
                                let source = source.clone();
                                let viewport = Rc::downgrade(&clicks.0);
                                let data = click_data.clone();
                                clicks.0.borrow_mut().select_task =
                                    Some(window.spawn(cx, async move |cx| {
                                        cx.background_executor()
                                            .timer(std::time::Duration::from_millis(500))
                                            .await;
                                        let Some(viewport) = viewport.upgrade() else {
                                            return;
                                        };
                                        let current = viewport
                                            .borrow()
                                            .layout
                                            .as_ref()
                                            .is_some_and(|(current, _)| current.ptr_eq(&data));
                                        if current {
                                            let _ = cx
                                                .update(|window, cx| callback(&source, window, cx));
                                        }
                                    }));
                            }
                        }
                    })
                    .child(
                        div()
                            .text_style(text::UI)
                            .text_size(px(14. * zoom))
                            .truncate()
                            .child(title),
                    )
                    .children(address.map(|address| {
                        div()
                            .text_style(text::CODE_SMALL)
                            .text_size(px(11. * zoom))
                            .truncate()
                            .text_color(r.text_muted)
                            .child(address)
                    }))
            })
            .collect::<Vec<_>>();
        div()
            .id(self.id)
            .relative()
            .size_full()
            .overflow_hidden()
            .bg(r.container_content_bg)
            .text_color(r.text_default)
            .on_mouse_down(MouseButton::Left, move |event, _, _| {
                let mut state = down.0.borrow_mut();
                state.drag = Some((event.position, state.pan));
            })
            .on_mouse_move(move |event, window, _| {
                let mut state = movement.0.borrow_mut();
                if event.dragging() {
                    if let Some((start, pan)) = state.drag {
                        state.pan = pan + event.position - start;
                        window.refresh();
                    }
                } else {
                    state.drag = None;
                }
            })
            .on_mouse_up(MouseButton::Left, move |_, _, _| {
                up.0.borrow_mut().drag = None;
            })
            .on_scroll(move |event, window, cx| {
                let delta = match event.delta {
                    ScrollDelta::Pixels(delta) => f32::from(delta.y),
                    ScrollDelta::Lines(delta) => delta.y * 20.,
                };
                let mut state = scroll.0.borrow_mut();
                let zoom = (state.zoom * (delta * 0.002).exp()).clamp(0.2, 3.);
                let pointer = event.position - *scroll_origin.borrow();
                state.pan = pointer - (pointer - state.pan) * (zoom / state.zoom);
                state.zoom = zoom;
                window.refresh();
                cx.stop_propagation();
            })
            .child(
                canvas(
                    move |bounds: Bounds<Pixels>, _, _| {
                        *canvas_origin.borrow_mut() = bounds.center();
                    },
                    move |bounds, _, window, _| {
                        let at = |x: f32, y: f32| {
                            bounds.center() + pan + point(px(x * zoom), px(y * zoom))
                        };
                        for &(from, to) in &paint_graph.edges {
                            let mut path = PathBuilder::stroke(px(1.));
                            if from == to {
                                let node = &paint_graph.nodes[from];
                                let points = layout::self_loop_path(node);
                                path.move_to(at(points[0].0, points[0].1));
                                for &(x, y) in &points[1..] {
                                    path.line_to(at(x, y));
                                }
                                let end = at(points[4].0, points[4].1);
                                path.move_to(end + point(px(6.), px(-4.)));
                                path.line_to(end);
                                path.line_to(end + point(px(6.), px(4.)));
                            } else {
                                let from = &paint_graph.nodes[from];
                                let to = &paint_graph.nodes[to];
                                let direction = if to.y >= from.y { 1. } else { -1. };
                                let start = at(from.x, from.y + direction * NODE_HEIGHT / 2.);
                                let end = at(to.x, to.y - direction * NODE_HEIGHT / 2.);
                                let middle = (start.y + end.y) / 2.;
                                path.move_to(start);
                                path.line_to(point(start.x, middle));
                                path.line_to(point(end.x, middle));
                                path.line_to(end);
                                path.move_to(end + point(px(-4.), px(-direction * 6.)));
                                path.line_to(end);
                                path.line_to(end + point(px(4.), px(-direction * 6.)));
                            }
                            if let Ok(path) = path.build() {
                                window.paint_path(path, r.border_default);
                            }
                        }
                        for placed in &paint_graph.nodes {
                            let bounds = Bounds::new(
                                at(placed.x - NODE_WIDTH / 2., placed.y - NODE_HEIGHT / 2.),
                                size(px(NODE_WIDTH * zoom), px(NODE_HEIGHT * zoom)),
                            );
                            window.paint_quad(
                                fill(
                                    bounds,
                                    if placed.source.is_some() {
                                        r.control_bg
                                    } else {
                                        r.container_elevated_bg
                                    },
                                )
                                .corner_radii(radius::SM),
                            );
                        }
                    },
                )
                .absolute()
                .size_full(),
            )
            .children(nodes)
            .into_any_element()
    }
}
