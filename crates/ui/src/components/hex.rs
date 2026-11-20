//! Sixteen-byte virtual hex rows with matched ASCII selection.
use super::{TextStyled as _, roles};
use crate::metrics::{size, space, text};
use gpui_kit::{
    App, ElementId, InteractiveElement, IntoElement, ParentElement, RenderOnce,
    StatefulInteractiveElement, Styled, UniformListScrollHandle, Window, div,
    prelude::FluentBuilder as _, uniform_list,
};
use std::{ops::Range, rc::Rc, sync::Arc};

type SelectionHandler = Rc<dyn Fn(Range<u64>, &mut Window, &mut App)>;
/// Bytes and absolute selection range. Shift-click extends the caller-provided selection.
#[derive(IntoElement)]
pub struct HexView {
    id: ElementId,
    bytes: Arc<[u8]>,
    offset: u64,
    selected: Option<Range<u64>>,
    scroll: Option<UniformListScrollHandle>,
    on_select: Option<SelectionHandler>,
}
impl HexView {
    pub fn new(
        id: impl Into<ElementId>,
        bytes: Arc<[u8]>,
        offset: u64,
        selected: Option<Range<u64>>,
    ) -> Self {
        Self {
            id: id.into(),
            bytes,
            offset,
            selected,
            scroll: None,
            on_select: None,
        }
    }
    pub fn track_scroll(mut self, scroll: &UniformListScrollHandle) -> Self {
        self.scroll = Some(scroll.clone());
        self
    }
    pub fn on_select(
        mut self,
        handler: impl Fn(Range<u64>, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_select = Some(Rc::new(handler));
        self
    }
}
impl RenderOnce for HexView {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let bytes = self.bytes;
        let selected = self.selected;
        let callback = self.on_select;
        uniform_list(self.id, bytes.len().div_ceil(16), move |visible, _, cx| {
            let r = roles(cx);
            visible
                .map(|row| {
                    let start = row * 16;
                    let end = (start + 16).min(bytes.len());
                    let row_offset = self.offset.saturating_add(start as u64);
                    let mut hex = div().flex().gap(space::S2);
                    let mut ascii = div().flex();
                    for column in 0..16 {
                        let index = start + column;
                        if index >= end {
                            hex = hex.child(div().w(space::S24).flex_none());
                            ascii = ascii.child(div().w(space::S8).flex_none());
                            continue;
                        }
                        let byte = bytes[index];
                        let address = self.offset.saturating_add(index as u64);
                        let active = selected
                            .as_ref()
                            .is_some_and(|range| range.contains(&address));
                        let anchor = selected.as_ref().map(|range| range.start);
                        let make_cell = |id, content: String, width| {
                            let callback = callback.clone();
                            div()
                                .id(id)
                                .w(width)
                                .flex_none()
                                .text_center()
                                .when(active, |d| d.bg(r.editor_selection_bg))
                                .when(callback.is_some(), |d| d.cursor_pointer())
                                .on_click(move |event, window, cx| {
                                    if let Some(callback) = &callback {
                                        let start = if event.modifiers().shift {
                                            anchor.unwrap_or(address)
                                        } else {
                                            address
                                        };
                                        callback(
                                            start.min(address)
                                                ..start.max(address).saturating_add(1),
                                            window,
                                            cx,
                                        );
                                    }
                                })
                                .child(content)
                        };
                        hex = hex.child(make_cell(column, format!("{byte:02X}"), space::S24));
                        let character = if byte.is_ascii_graphic() || byte == b' ' {
                            char::from(byte)
                        } else {
                            '.'
                        };
                        ascii =
                            ascii.child(make_cell(column + 16, character.to_string(), space::S8));
                    }
                    div()
                        .id(row)
                        .flex()
                        .h(size::ROW)
                        .gap(space::S12)
                        .text_style(text::CODE_DOCUMENT)
                        .text_color(r.text_default)
                        .child(
                            div()
                                .w(space::S64 * 2.)
                                .flex_none()
                                .text_color(r.text_subtle)
                                .child(format!("{row_offset:016X}")),
                        )
                        .child(hex)
                        .child(ascii)
                })
                .collect()
        })
        .size_full()
        .when_some(self.scroll, |list, scroll| list.track_scroll(&scroll))
    }
}
