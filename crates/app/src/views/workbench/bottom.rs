//! Project tasks/logs and a real VT100 terminal backed by TerminalService.
use super::{BottomTab, WorkbenchView};
use crate::state::{AppState, run};
use autocraft_core::{
    AttachRequest, Attached, Error, GridSize, ProjectId, Shell, TerminalEvent, TerminalTab,
};
use autocraft_ui::components::{TextStyled as _, label};
use autocraft_ui::metrics::text;
use gpui_kit::{
    AnyElement, ClipboardItem, Context, Entity, FocusHandle, InteractiveElement as _, IntoElement,
    KeyDownEvent, ParentElement as _, Render, StatefulInteractiveElement as _, Styled as _, Window,
    base::{h_flex, v_flex},
    component::{button::Button, scroll::ScrollableElement as _},
    div,
};
impl WorkbenchView {
    fn refresh_logs(&mut self, cx: &mut Context<Self>) {
        let Some(project) = self.project.clone() else {
            return;
        };
        let log = self.state.read(cx).services.log.clone();
        let generation = self.generation;
        run(
            cx,
            move || {
                log.tail(2000)
                    .map(|entries| {
                        entries
                            .into_iter()
                            .filter(|entry| entry.raw.contains(project.as_str()))
                            .map(|entry| entry.raw)
                            .collect()
                    })
                    .map_err(|source| Error::Io {
                        context: rust_i18n::t!("app.workbench.logs").into_owned(),
                        source,
                    })
            },
            move |this, result, cx| {
                if generation != this.generation {
                    return;
                }
                match result {
                    Ok(logs) => this.logs = logs,
                    Err(error) => this.report(&error, cx),
                }
                cx.notify();
            },
        );
    }
    pub(super) fn render_bottom(&self, cx: &mut Context<Self>) -> AnyElement {
        let tabs = h_flex().gap_2().children(
            [
                (BottomTab::Tasks, "tasks"),
                (BottomTab::Logs, "logs"),
                (BottomTab::Terminal, "terminal"),
            ]
            .into_iter()
            .map(|(tab, key)| {
                Button::new(key)
                    .label(rust_i18n::t!(format!("app.workbench.{key}")))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.bottom = tab;
                        if tab == BottomTab::Logs {
                            this.refresh_logs(cx);
                        }
                        cx.notify();
                    }))
            }),
        );
        let content = match self.bottom {
            BottomTab::Tasks => v_flex()
                .gap_2()
                .children(self.tasks.iter().enumerate().map(|(index, task)| {
                    let mut row = h_flex()
                        .gap_2()
                        .child(task.tool.clone())
                        .child(task.message.clone());
                    if let Some(fraction) = task.fraction {
                        row = row.child(format!("{:.0}%", fraction * 100.));
                    }
                    if !task.done && task.owned {
                        row = row.child(
                            Button::new(("cancel-task", index))
                                .label(rust_i18n::t!("app.workbench.cancel"))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if let Some(task) = this.tasks.get(index) {
                                        task.cancel.cancel();
                                    }
                                    cx.notify();
                                })),
                        );
                    }
                    row
                }))
                .into_any_element(),
            BottomTab::Logs => v_flex()
                .gap_1()
                .child(
                    Button::new("refresh-logs")
                        .label(rust_i18n::t!("app.workbench.refresh"))
                        .on_click(cx.listener(|this, _, _, cx| this.refresh_logs(cx))),
                )
                .children(
                    self.logs
                        .iter()
                        .map(|line| label(text::CODE_TEXT, line.clone())),
                )
                .into_any_element(),
            BottomTab::Terminal => self.terminal.clone().into_any_element(),
        };
        v_flex()
            .size_full()
            .child(tabs)
            .child(
                v_flex()
                    .id("analysis-bottom")
                    .flex_1()
                    .overflow_y_scrollbar()
                    .child(content),
            )
            .into_any_element()
    }
}
/// Terminal owns the attachment replay boundary and VT state; service owns the PTY lifetime.
pub(super) struct TerminalView {
    state: Entity<AppState>,
    project: Option<ProjectId>,
    generation: u64,
    tabs: Vec<TerminalTab>,
    active: Option<Attached>,
    active_key: Option<String>,
    pending: Vec<TerminalEvent>,
    parser: vt100::Parser,
    focus: FocusHandle,
    loading: bool,
    exited: bool,
}
impl std::fmt::Debug for TerminalView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TerminalView")
            .field("generation", &self.generation)
            .finish_non_exhaustive()
    }
}
impl TerminalView {
    pub fn new(state: Entity<AppState>, _: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            state,
            project: None,
            generation: 0,
            tabs: Vec::new(),
            active: None,
            active_key: None,
            pending: Vec::new(),
            parser: vt100::Parser::default(),
            focus: cx.focus_handle(),
            loading: false,
            exited: false,
        }
    }
    pub fn select_project(&mut self, project: Option<ProjectId>, cx: &mut Context<Self>) {
        if self.project == project {
            return;
        }
        self.project = project;
        self.generation += 1;
        self.active = None;
        self.active_key = None;
        self.pending.clear();
        self.tabs.clear();
        self.parser = vt100::Parser::default();
        self.loading = false;
        let Some(project) = self.project.clone() else {
            cx.notify();
            return;
        };
        let service = self.state.read(cx).services.terminals.clone();
        let generation = self.generation;
        run(
            cx,
            move || {
                let cwd = std::fs::canonicalize(project.as_str()).map_err(|source| Error::Io {
                    context: rust_i18n::t!("app.workbench.terminal_cwd").into_owned(),
                    source,
                })?;
                service.tabs(&cwd)
            },
            move |this, result, cx| {
                if generation != this.generation {
                    return;
                }
                match result {
                    Ok(tabs) => this.tabs = tabs,
                    Err(error) => this.report(&error, cx),
                }
                cx.notify();
            },
        );
    }
    fn report(&self, error: &Error, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| state.report(error, cx));
    }
    fn attach(&mut self, key: String, shell: Shell, cx: &mut Context<Self>) {
        if self.loading {
            return;
        }
        let Some(project) = self.project.clone() else {
            return;
        };
        let service = self.state.read(cx).services.terminals.clone();
        let generation = self.generation;
        self.loading = true;
        self.pending.clear();
        let active_key = key.clone();
        run(
            cx,
            move || {
                let cwd = std::fs::canonicalize(project.as_str()).map_err(|source| Error::Io {
                    context: rust_i18n::t!("app.workbench.terminal_cwd").into_owned(),
                    source,
                })?;
                service.reserve(&cwd, &key, &key)?;
                let attached = service.attach(AttachRequest {
                    cwd: cwd.clone(),
                    tab_key: key,
                    shell,
                    size: GridSize { cols: 80, rows: 24 },
                })?;
                Ok((attached, service.tabs(&cwd)?))
            },
            move |this, result, cx| {
                if generation != this.generation {
                    return;
                }
                this.loading = false;
                match result {
                    Ok((attached, tabs)) => {
                        this.parser = vt100::Parser::default();
                        this.parser.process(attached.replay.as_bytes());
                        this.active = Some(attached);
                        this.active_key = Some(active_key);
                        this.tabs = tabs;
                        this.exited = false;
                        for event in std::mem::take(&mut this.pending) {
                            this.on_event(&event, cx);
                        }
                    }
                    Err(error) => this.report(&error, cx),
                }
                cx.notify();
            },
        );
    }
    pub fn on_event(&mut self, event: &TerminalEvent, cx: &mut Context<Self>) {
        if self.loading {
            self.pending.push(event.clone());
            return;
        }
        let Some(active) = self.active.as_mut() else {
            return;
        };
        match event {
            TerminalEvent::Data {
                runtime,
                byte_offset,
                data,
            } if *runtime == active.runtime => {
                let end = byte_offset.saturating_add(data.len() as u64);
                if end > active.replay_end_offset {
                    let skip = active.replay_end_offset.saturating_sub(*byte_offset) as usize;
                    if let Some(data) = data.get(skip..) {
                        self.parser.process(data.as_bytes());
                        active.replay_end_offset = end;
                    }
                }
            }
            TerminalEvent::Exit { runtime, .. } if *runtime == active.runtime => self.exited = true,
            TerminalEvent::Data { .. } | TerminalEvent::Exit { .. } => return,
        }
        cx.notify();
    }
    fn write(&mut self, data: String, cx: &mut Context<Self>) {
        let Some(active) = &self.active else {
            return;
        };
        let runtime = active.runtime;
        let service = self.state.read(cx).services.terminals.clone();
        let generation = self.generation;
        run(
            cx,
            move || service.write(&runtime, &data),
            move |this, result, cx| {
                if generation == this.generation
                    && let Err(error) = result
                {
                    this.report(&error, cx);
                }
            },
        );
    }
    fn close(&mut self, cx: &mut Context<Self>) {
        let (Some(project), Some(key)) = (self.project.clone(), self.active_key.clone()) else {
            return;
        };
        let service = self.state.read(cx).services.terminals.clone();
        let generation = self.generation;
        run(
            cx,
            move || {
                let cwd = std::fs::canonicalize(project.as_str()).map_err(|source| Error::Io {
                    context: rust_i18n::t!("app.workbench.terminal_cwd").into_owned(),
                    source,
                })?;
                service.close(&cwd, &key)?;
                service.tabs(&cwd)
            },
            move |this, result, cx| {
                if generation != this.generation {
                    return;
                }
                match result {
                    Ok(tabs) => {
                        this.tabs = tabs;
                        this.active = None;
                        this.active_key = None;
                        this.parser = vt100::Parser::default();
                    }
                    Err(error) => this.report(&error, cx),
                }
                cx.notify();
            },
        );
    }
    fn key(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.exited {
            return;
        }
        let stroke = &event.keystroke;
        let data = match stroke.key.as_str() {
            "enter" => "\r".to_owned(),
            "backspace" => "\x7f".to_owned(),
            "tab" => "\t".to_owned(),
            "escape" => "\x1b".to_owned(),
            "up" => "\x1b[A".to_owned(),
            "down" => "\x1b[B".to_owned(),
            "right" => "\x1b[C".to_owned(),
            "left" => "\x1b[D".to_owned(),
            "home" => "\x1b[H".to_owned(),
            "end" => "\x1b[F".to_owned(),
            "delete" => "\x1b[3~".to_owned(),
            _ if stroke.modifiers.control && stroke.key.len() == 1 => {
                let byte = stroke.key.as_bytes()[0].to_ascii_lowercase();
                if byte.is_ascii_lowercase() {
                    char::from(byte - b'a' + 1).to_string()
                } else {
                    return;
                }
            }
            _ => stroke.key_char.clone().unwrap_or_default(),
        };
        if !data.is_empty() {
            self.write(data, cx);
            cx.stop_propagation();
        }
    }
}
impl Render for TerminalView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut tabs = h_flex().gap_2();
        for tab in &self.tabs {
            let key = tab.tab_key.clone();
            tabs = tabs.child(
                Button::new(tab.tab_key.clone())
                    .label(tab.title.clone())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.attach(key.clone(), Shell::PowerShell, cx)
                    })),
            );
        }
        tabs = tabs.child(
            Button::new("new-terminal")
                .label(rust_i18n::t!("app.workbench.new_terminal"))
                .on_click(cx.listener(|this, _, _, cx| {
                    if this.loading {
                        return;
                    }
                    let key = format!("shell-{}", uuid::Uuid::new_v4());
                    this.attach(key, Shell::PowerShell, cx);
                })),
        );
        let screen = self.parser.screen().contents();
        v_flex()
            .size_full()
            .child(tabs)
            .child(
                div()
                    .id("terminal-screen")
                    .track_focus(&self.focus)
                    .tab_stop(true)
                    .on_click(cx.listener(|this, _, window, cx| this.focus.focus(window, cx)))
                    .on_key_down(cx.listener(Self::key))
                    .text_style(text::CODE_DOCUMENT)
                    .child(screen),
            )
            .children(
                self.exited
                    .then(|| rust_i18n::t!("app.workbench.terminal_exited").into_owned()),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("terminal-copy")
                            .label(rust_i18n::t!("app.workbench.copy"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(
                                    this.parser.screen().contents(),
                                ))
                            })),
                    )
                    .child(
                        Button::new("terminal-close")
                            .label(rust_i18n::t!("app.workbench.close_terminal"))
                            .on_click(cx.listener(|this, _, _, cx| this.close(cx))),
                    ),
            )
    }
}
