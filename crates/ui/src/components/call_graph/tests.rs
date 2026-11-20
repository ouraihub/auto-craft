//! Double-clicking a node must not dispatch the delayed single-click navigation.
use super::*;
use crate::{AutocraftTheme, ThemeCatalog};
use gpui_kit::{Context, MouseDownEvent, MouseUpEvent, Render, TestAppContext};
use std::{cell::RefCell, rc::Rc, time::Duration};

struct Graph {
    data: Rc<CallGraphData>,
    viewport: GraphViewport,
    events: Rc<RefCell<Vec<&'static str>>>,
}
impl Render for Graph {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let select = self.events.clone();
        let recenter = self.events.clone();
        CallGraph::new(
            "graph",
            self.data.clone(),
            self.viewport.clone(),
            CallGraphLabels {
                too_large: "large".into(),
                missing_center: "missing".into(),
                more: Rc::new(|_| "more".into()),
            },
        )
        .on_select(move |_, _, _| select.borrow_mut().push("select"))
        .on_recenter(move |_, _, _| recenter.borrow_mut().push("recenter"))
    }
}
#[gpui_kit::test]
fn double_click_cancels_pending_single_click(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        let catalog = ThemeCatalog::bundled().unwrap();
        AutocraftTheme::install(&catalog.resolve("dark", None).theme, None, cx);
    });
    let events = Rc::new(RefCell::new(Vec::new()));
    let captured = events.clone();
    let (_, visual) = cx.add_window_view(move |_, _| Graph {
        data: Rc::new(CallGraphData {
            center: "entry".into(),
            nodes: vec![CallGraphNode {
                id: "entry".into(),
                name: "entry".into(),
                address: "0x10".into(),
            }],
            edges: vec![],
        }),
        viewport: GraphViewport::default(),
        events: captured,
    });
    visual.update(|window, cx| {
        window.draw(cx).clear(cx);
    });
    let position = visual.update(|window, _| {
        let size = window.viewport_size();
        point(size.width / 2., size.height / 2.)
    });
    for click_count in [1, 2] {
        visual.simulate_event(MouseDownEvent {
            position,
            button: MouseButton::Left,
            modifiers: Default::default(),
            click_count,
            first_mouse: false,
        });
        visual.simulate_event(MouseUpEvent {
            position,
            button: MouseButton::Left,
            modifiers: Default::default(),
            click_count,
        });
        visual.run_until_parked();
    }
    visual.executor().advance_clock(Duration::from_secs(1));
    visual.run_until_parked();
    assert_eq!(*events.borrow(), ["recenter"]);
    events.borrow_mut().clear();
    visual.simulate_click(position, Default::default());
    visual.run_until_parked();
    visual.executor().advance_clock(Duration::from_secs(1));
    visual.run_until_parked();
    assert_eq!(*events.borrow(), ["select"]);
}
