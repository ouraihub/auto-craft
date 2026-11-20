//! Reverse-engineering workbench: project-scoped tables, editors and evidence navigation.
mod bottom;
mod calls;
mod data;
mod editor;
mod graph;
mod inspector;
mod table;
#[cfg(test)]
mod tests;
use crate::state::{AppEvent, AppState};
use autocraft_core::{CancelFlag, EvidenceId, ProjectId, ToolInfo, ToolReply};
use autocraft_ui::components::CodeDocument;
use data::{EditorTab, ListKind};
use gpui_kit::{
    AnyView, AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render,
    Styled as _, Subscription, Window,
    base::{h_resizable, resizable_panel, v_flex, v_resizable},
    component::{
        button::Button,
        input::{Input, InputEvent, InputState},
        menu::{DropdownMenu as _, PopupMenuItem},
        table::{DataTable, TableDelegate as _, TableEvent, TableState},
    },
    px,
};
use std::{collections::BTreeSet, rc::Rc, sync::Arc};
use table::Rows;

/// Every editor uses the same exhaustive state machine.
#[derive(Debug, Clone)]
pub enum Fill {
    Idle,
    Loading,
    Unavailable(String),
    DeepNeeded,
    EngineMissing {
        engine: String,
        fix: String,
    },
    Error {
        message: String,
        fix: Option<String>,
    },
    Ready(ToolReply),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RightTab {
    Agent,
    Inspector,
    Evidence,
    Unknowns,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BottomTab {
    Tasks,
    Logs,
    Terminal,
}
#[derive(Debug)]
struct ToolTask {
    tool: String,
    cancel: CancelFlag,
    owned: bool,
    message: String,
    fraction: Option<f64>,
    done: bool,
}
/// Persistent view entity; owns only UI state, never engine state.
pub struct WorkbenchView {
    state: Entity<AppState>,
    project: Option<ProjectId>,
    target: Option<autocraft_core::TargetId>,
    generation: u64,
    editor_generation: u64,
    list_generation: u64,
    tools: Vec<ToolInfo>,
    list_kind: ListKind,
    tab: EditorTab,
    right: RightTab,
    bottom: BottomTab,
    table: Entity<TableState<Rows>>,
    network: Entity<TableState<Rows>>,
    filter: Entity<InputState>,
    list_fill: Fill,
    fill: Fill,
    selection: Option<serde_json::Value>,
    document: Option<Rc<CodeDocument>>,
    graph: Option<Rc<autocraft_ui::components::CallGraphData>>,
    viewport: autocraft_ui::components::GraphViewport,
    bytes: Option<Arc<[u8]>>,
    byte_offset: u64,
    byte_selection: Option<std::ops::Range<u64>>,
    result_reply: Option<ToolReply>,
    evidence: Option<Arc<serde_json::Value>>,
    expanded: Arc<BTreeSet<String>>,
    agent: Option<AnyView>,
    tasks: Vec<ToolTask>,
    pending: Option<(ToolInfo, serde_json::Value, bool)>,
    confirm_deep: bool,
    bottom_open: bool,
    logs: Vec<String>,
    terminal: Entity<bottom::TerminalView>,
    _subscriptions: Vec<Subscription>,
}
impl std::fmt::Debug for WorkbenchView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkbenchView")
            .field("generation", &self.generation)
            .finish_non_exhaustive()
    }
}
impl WorkbenchView {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let weak = cx.entity().downgrade();
        let table = cx.new(|cx| TableState::new(Rows::new(weak.clone()), window, cx));
        let network = cx.new(|cx| {
            let mut rows = Rows::new(weak);
            rows.network = true;
            TableState::new(rows, window, cx)
        });
        let filter = cx.new(|cx| {
            InputState::new(window, cx).placeholder(rust_i18n::t!("app.workbench.filter"))
        });
        let terminal = cx.new(|cx| bottom::TerminalView::new(state.clone(), window, cx));
        let subscriptions = vec![
            cx.subscribe(&state, |this, _, event, cx| match event {
                AppEvent::Project => this.reload(cx),
                AppEvent::Analysis(project) if this.project.as_ref() == Some(project) => {
                    this.refresh_snapshot(cx)
                }
                _ => {}
            }),
            cx.subscribe(&table, |this, table, event, cx| {
                if let TableEvent::SelectRow(index) = event
                    && let Some(row) = table.read(cx).delegate().row(*index).cloned()
                {
                    this.selection = Some(row);
                    this.right = RightTab::Inspector;
                    this.load_editor(cx);
                }
            }),
            cx.subscribe(&filter, |this, filter, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    let value = filter.read(cx).value().to_string();
                    this.table.update(cx, |table, cx| {
                        table.delegate_mut().filter(&value);
                        cx.notify();
                    });
                }
            }),
        ];
        let mut this = Self {
            state,
            project: None,
            target: None,
            generation: 0,
            editor_generation: 0,
            list_generation: 0,
            tools: Vec::new(),
            list_kind: ListKind::Functions,
            tab: EditorTab::Pseudocode,
            right: RightTab::Inspector,
            bottom: BottomTab::Tasks,
            table,
            network,
            filter,
            list_fill: Fill::Idle,
            fill: Fill::Idle,
            selection: None,
            document: None,
            graph: None,
            viewport: Default::default(),
            bytes: None,
            byte_offset: 0,
            byte_selection: None,
            result_reply: None,
            evidence: None,
            expanded: Arc::new(BTreeSet::from([String::new()])),
            agent: None,
            tasks: Vec::new(),
            pending: None,
            confirm_deep: false,
            bottom_open: true,
            logs: Vec::new(),
            terminal,
            _subscriptions: subscriptions,
        };
        this.reload(cx);
        this
    }
    /// Shares the exact AgentPanel entity with the project view.
    pub fn set_agent(&mut self, agent: AnyView, cx: &mut Context<Self>) {
        self.agent = Some(agent);
        cx.notify();
    }
    /// Navigate from disassembly, graph or an external evidence link.
    pub fn jump(&mut self, address: impl Into<String>, cx: &mut Context<Self>) {
        self.select_address(address.into(), EditorTab::Pseudocode, cx);
    }
    fn select_address(&mut self, address: String, tab: EditorTab, cx: &mut Context<Self>) {
        self.tab = tab;
        self.right = RightTab::Inspector;
        let selected = self.table.update(cx, |table, cx| {
            let index = (0..table.delegate().rows_count(cx)).find(|&index| {
                table
                    .delegate()
                    .row(index)
                    .and_then(data::address)
                    .as_deref()
                    == Some(address.as_str())
            });
            if let Some(index) = index {
                table.set_selected_row(index, cx);
            }
            index.is_some()
        });
        if !selected {
            self.selection = Some(serde_json::json!({"address":address}));
            self.load_editor(cx);
        }
    }
    /// Display a command-palette or Agent tool reply in Results.
    pub fn show_result(&mut self, reply: ToolReply, cx: &mut Context<Self>) {
        self.editor_generation += 1;
        self.tab = EditorTab::Results;
        self.install_reply(reply, cx);
    }
    /// Resolve an actual ledger record on a worker.
    pub fn show_evidence(&mut self, id: EvidenceId, cx: &mut Context<Self>) {
        self.right = RightTab::Evidence;
        self.evidence = None;
        let Some(project) = self.project.clone() else {
            return;
        };
        let service = self.state.read(cx).services.analysis.clone();
        let generation = self.generation;
        crate::state::run(
            cx,
            move || service.evidence(&project, &id),
            move |this, result, cx| {
                if generation != this.generation {
                    return;
                }
                match result {
                    Ok(value) => this.evidence = Some(Arc::new(value)),
                    Err(error) => this.report(&error, cx),
                }
                cx.notify();
            },
        );
    }
    pub fn on_terminal_event(
        &mut self,
        event: &autocraft_core::TerminalEvent,
        cx: &mut Context<Self>,
    ) {
        self.terminal
            .update(cx, |terminal, cx| terminal.on_event(event, cx));
    }
    pub fn on_analysis_progress(
        &mut self,
        tool: &str,
        message: &str,
        fraction: Option<f64>,
        cx: &mut Context<Self>,
    ) {
        if let Some(task) = self
            .tasks
            .iter_mut()
            .rev()
            .find(|task| task.tool == tool && !task.done)
        {
            task.message = message.to_owned();
            task.fraction = fraction;
        } else {
            self.tasks.push(ToolTask {
                tool: tool.to_owned(),
                cancel: CancelFlag::new(),
                owned: false,
                message: message.to_owned(),
                fraction,
                done: false,
            });
        }
        cx.notify();
    }
    fn report(&self, error: &autocraft_core::Error, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| state.report(error, cx));
    }
}
impl Render for WorkbenchView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let placeholder = crate::i18n::text("app.workbench.filter");
        if self.filter.read(cx).presentation().placeholder().as_ref() != placeholder.as_ref() {
            self.filter.update(cx, |input, cx| {
                input.set_placeholder(placeholder, window, cx)
            });
        }
        let kinds: Vec<_> = ListKind::ALL
            .into_iter()
            .filter(|kind| {
                kind.supports(self.target_kind(cx))
                    && data::choose(&self.tools, kind.tools(self.target_kind(cx))).is_some()
            })
            .collect();
        let owner = cx.entity();
        let active = self.list_kind;
        let list_selector = Button::new("workbench-list-kind")
            .label(rust_i18n::t!(format!("app.workbench.{}", active.key())))
            .accessibility_label(rust_i18n::t!("app.workbench.list_kind").into_owned())
            .icon(gpui_kit::assets::IconName::ChevronDown)
            .dropdown_menu(move |menu, _, _| {
                kinds.iter().fold(menu, |menu, &kind| {
                    let owner = owner.clone();
                    menu.item(
                        PopupMenuItem::new(rust_i18n::t!(format!("app.workbench.{}", kind.key())))
                            .checked(active == kind)
                            .on_click(move |_, _, cx| {
                                owner.update(cx, |this, cx| {
                                    this.list_kind = kind;
                                    this.load_list(true, cx);
                                })
                            }),
                    )
                })
            });
        let rail = v_flex()
            .size_full()
            .gap_2()
            .child(list_selector)
            .child(Input::new(&self.filter))
            .child(
                v_flex()
                    .flex_1()
                    .min_h_0()
                    .child(DataTable::new(&self.table).stripe(true).bordered(true)),
            )
            .child(self.render_fill(&self.list_fill, cx));
        let editor = self.render_editor(cx);
        let bottom = self.render_bottom(cx);
        let right = self.render_inspector(cx);
        let center = v_resizable("analysis-center")
            .child(resizable_panel().child(editor))
            .child(
                resizable_panel()
                    .size(px(190.))
                    .size_range(px(32.)..px(600.))
                    .visible(self.bottom_open)
                    .child(bottom),
            );
        h_resizable("analysis-workbench")
            .child(
                resizable_panel()
                    .size(px(290.))
                    .size_range(px(200.)..px(520.))
                    .fixed()
                    .child(rail),
            )
            .child(resizable_panel().child(center))
            .child(
                resizable_panel()
                    .size(px(360.))
                    .size_range(px(320.)..px(560.))
                    .fixed()
                    .child(right),
            )
    }
}
