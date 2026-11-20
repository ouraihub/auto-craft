//! Virtual source viewer with one parsed document and caller-supplied address links.
use super::{TextStyled as _, disassembly, roles};
use crate::metrics::{size, space, text};
use gpui_kit::{
    App, ElementId, HighlightStyle, InteractiveText, IntoElement, ParentElement, RenderOnce,
    SharedString, Styled, StyledText, UniformListScrollHandle, Window,
    base::input::Rope,
    component::{ActiveTheme as _, highlighter::SyntaxHighlighter},
    div,
    prelude::FluentBuilder as _,
    uniform_list,
};
use std::{ops::Range, rc::Rc};

/// Languages supported by the shared source viewer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodeLanguage {
    C,
    JavaScript,
    Disassembly,
    Plain,
}
/// A clickable UTF-8 byte range in one line and its address payload.
#[derive(Debug, Clone)]
pub struct AddressRange {
    pub line: usize,
    pub bytes: Range<usize>,
    pub address: SharedString,
}
/// Immutable source plus parsed syntax; build once when a tool result changes, not each frame.
pub struct CodeDocument {
    source: SharedString,
    lines: Vec<Range<usize>>,
    language: CodeLanguage,
    syntax: Option<SyntaxHighlighter>,
    addresses: Vec<Vec<AddressRange>>,
}
impl std::fmt::Debug for CodeDocument {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CodeDocument")
            .field("language", &self.language)
            .field("lines", &self.lines.len())
            .finish_non_exhaustive()
    }
}
impl CodeDocument {
    /// Invalid or overlapping links are omitted, never passed to GPUI's text shaper.
    pub fn new(
        source: impl Into<SharedString>,
        language: CodeLanguage,
        addresses: Vec<AddressRange>,
    ) -> Self {
        let source = source.into();
        let mut start = 0;
        let mut lines = Vec::new();
        for line in source.split_inclusive('\n') {
            let end = start + line.trim_end_matches(['\r', '\n']).len();
            lines.push(start..end);
            start += line.len();
        }
        if source.is_empty() || source.ends_with('\n') {
            lines.push(source.len()..source.len());
        }
        let mut links = vec![Vec::new(); lines.len()];
        for link in addresses {
            if let Some(range) = lines.get(link.line) {
                let line = &source[range.clone()];
                if link.bytes.start < link.bytes.end && line.get(link.bytes.clone()).is_some() {
                    links[link.line].push(link);
                }
            }
        }
        for line in &mut links {
            line.sort_by_key(|link| link.bytes.start);
            let mut end = 0;
            line.retain(|link| {
                if link.bytes.start < end {
                    false
                } else {
                    end = link.bytes.end;
                    true
                }
            });
        }
        let syntax = match language {
            CodeLanguage::C | CodeLanguage::JavaScript => {
                let mut syntax = SyntaxHighlighter::new(if language == CodeLanguage::C {
                    "c"
                } else {
                    "javascript"
                });
                syntax.update(None, &Rope::from_str(&source), None);
                Some(syntax)
            }
            CodeLanguage::Disassembly | CodeLanguage::Plain => None,
        };
        Self {
            source,
            lines,
            language,
            syntax,
            addresses: links,
        }
    }
    pub fn line_count(&self) -> usize {
        self.lines.len()
    }
}
type AddressHandler = Rc<dyn Fn(&str, &mut Window, &mut App)>;
/// Virtual lines, fixed line-number gutter, syntax highlighting and address navigation intents.
#[derive(IntoElement)]
pub struct CodeView {
    id: ElementId,
    document: Rc<CodeDocument>,
    first_line: usize,
    scroll: Option<UniformListScrollHandle>,
    on_address: Option<AddressHandler>,
}
impl CodeView {
    pub fn new(id: impl Into<ElementId>, document: Rc<CodeDocument>) -> Self {
        Self {
            id: id.into(),
            document,
            first_line: 1,
            scroll: None,
            on_address: None,
        }
    }
    pub fn first_line(mut self, first_line: usize) -> Self {
        self.first_line = first_line;
        self
    }
    pub fn track_scroll(mut self, scroll: &UniformListScrollHandle) -> Self {
        self.scroll = Some(scroll.clone());
        self
    }
    pub fn on_address(mut self, handler: impl Fn(&str, &mut Window, &mut App) + 'static) -> Self {
        self.on_address = Some(Rc::new(handler));
        self
    }
}
impl RenderOnce for CodeView {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let document = self.document;
        let callback = self.on_address;
        uniform_list(self.id, document.lines.len(), move |visible, _, cx| {
            let r = roles(cx);
            let theme = cx.theme().highlight_theme.clone();
            visible
                .map(|index| {
                    let range = &document.lines[index];
                    let source = &document.source[range.clone()];
                    let highlights = if let Some(syntax) = &document.syntax {
                        syntax
                            .styles(range, theme.as_ref())
                            .into_iter()
                            .map(|(span, style)| {
                                (span.start - range.start..span.end - range.start, style)
                            })
                            .collect()
                    } else if document.language == CodeLanguage::Disassembly {
                        disassembly::highlights(source, r)
                    } else {
                        Vec::new()
                    };
                    let highlights: Vec<(Range<usize>, HighlightStyle)> = highlights;
                    let styled = StyledText::new(source.to_owned()).with_highlights(highlights);
                    let links = document.addresses[index].clone();
                    let ranges = links.iter().map(|link| link.bytes.clone()).collect();
                    let callback = callback.clone();
                    div()
                        .flex()
                        .h(size::ROW)
                        .text_style(text::CODE_DOCUMENT)
                        .text_color(r.text_default)
                        .child(
                            div()
                                .w(space::S64)
                                .flex_none()
                                .pr(space::S12)
                                .text_right()
                                .text_color(r.text_subtle)
                                .child(self.first_line.saturating_add(index).to_string()),
                        )
                        .child(InteractiveText::new(index, styled).on_click(
                            ranges,
                            move |link, window, cx| {
                                if let (Some(callback), Some(link)) = (&callback, links.get(link)) {
                                    callback(&link.address, window, cx);
                                }
                            },
                        ))
                })
                .collect()
        })
        .size_full()
        .when_some(self.scroll, |list, scroll| list.track_scroll(&scroll))
    }
}
