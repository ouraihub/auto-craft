//! Ctrl-K command discovery and schema-generated, consent-aware analysis forms.
mod execute;
mod form;
mod schema;
use crate::state::{AppEvent, AppState};
use autocraft_core::{Availability, CancelFlag, ProjectId, ToolInfo, ToolReply};
use gpui_kit::{
    AppContext as _, Context, Entity, EventEmitter, IntoElement, ParentElement as _, Render,
    Styled as _, Subscription, Window,
    base::v_flex,
    component::{
        Disableable as _, WindowExt as _,
        command::{Command, CommandGroup, CommandItem, CommandState},
        input::{InputEvent, InputState},
    },
};
use std::collections::BTreeMap;
/// The root owns application navigation; tool execution stays in the palette.
#[derive(Debug, Clone)]
pub enum PaletteEvent {
    ToolResult(ToolReply),
    Home,
    Project,
    Workbench,
    Settings,
    OpenFile,
    OpenFolder,
    ToggleMode,
}
#[derive(Debug, Clone, Copy)]
enum AppCommand {
    OpenFile,
    OpenFolder,
    Project,
    Workbench,
    ToggleMode,
    Settings,
    Export,
    Home,
}
impl AppCommand {
    const ALL: [Self; 8] = [
        Self::OpenFile,
        Self::OpenFolder,
        Self::Project,
        Self::Workbench,
        Self::ToggleMode,
        Self::Settings,
        Self::Export,
        Self::Home,
    ];
    fn key(self) -> &'static str {
        match self {
            Self::OpenFile => "open_file",
            Self::OpenFolder => "open_folder",
            Self::Project => "project",
            Self::Workbench => "workbench",
            Self::ToggleMode => "toggle_mode",
            Self::Settings => "settings",
            Self::Export => "export",
            Self::Home => "home",
        }
    }
}
struct Parameter {
    field: schema::Field,
    input: Entity<InputState>,
}
/// Window-owned palette; project changes invalidate forms and asynchronous replies.
pub struct CommandPalette {
    state: Entity<AppState>,
    command: Entity<CommandState>,
    project: Option<ProjectId>,
    generation: u64,
    tools: Vec<ToolInfo>,
    selected: Option<ToolInfo>,
    fields: Vec<Parameter>,
    groups: Vec<Vec<String>>,
    commands: Vec<AppCommand>,
    query: String,
    error: Option<String>,
    confirming: bool,
    busy: bool,
    cancel: CancelFlag,
    export_params: Option<serde_json::Value>,
    _subscriptions: Vec<Subscription>,
    field_subscriptions: Vec<Subscription>,
}
impl std::fmt::Debug for CommandPalette {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CommandPalette")
            .field("generation", &self.generation)
            .finish_non_exhaustive()
    }
}
impl EventEmitter<PaletteEvent> for CommandPalette {}
impl CommandPalette {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let command = cx.new(|cx| CommandState::new(window, cx));
        let subscriptions = vec![cx.subscribe(&state, |this, _, event, cx| {
            if matches!(event, AppEvent::Project | AppEvent::Analysis(_)) {
                this.reload(cx);
            }
        })];
        let mut this = Self {
            state,
            command,
            project: None,
            generation: 0,
            tools: Vec::new(),
            selected: None,
            fields: Vec::new(),
            groups: Vec::new(),
            commands: Vec::new(),
            query: String::new(),
            error: None,
            confirming: false,
            busy: false,
            cancel: CancelFlag::new(),
            export_params: None,
            _subscriptions: subscriptions,
            field_subscriptions: Vec::new(),
        };
        this.reload(cx);
        this
    }
    /// Root binds Ctrl-K globally and opens this retained palette entity.
    pub fn open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.selected = None;
        self.error = None;
        self.confirming = false;
        self.query.clear();
        self.export_params = None;
        self.command.update(cx, |command, cx| {
            command.set_query("", window, cx);
            command.focus(window, cx);
        });
        self.reload(cx);
        let view = cx.entity();
        window.open_dialog(cx, move |dialog, _, _| {
            dialog
                .title(rust_i18n::t!("app.palette.title"))
                .w(gpui_kit::px(720.))
                .on_ok(|_, _, _| false)
                .child(view.clone())
        });
    }

    pub fn export_report(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open(window, cx);
        self.export(cx);
    }
    fn reload(&mut self, cx: &mut Context<Self>) {
        let project = self.state.read(cx).project.clone();
        if self.project != project {
            self.cancel.cancel();
            self.generation += 1;
            self.project = project.clone();
            self.selected = None;
            self.fields.clear();
            self.busy = false;
        }
        let Some(project) = project else {
            self.tools.clear();
            cx.notify();
            return;
        };
        let service = self.state.read(cx).services.analysis.clone();
        let generation = self.generation;
        crate::state::run(
            cx,
            move || service.tools(&project),
            move |this, result, cx| {
                if generation != this.generation {
                    return;
                }
                match result {
                    Ok(tools) => this.tools = tools,
                    Err(error) => this.report(&error, cx),
                }
                cx.notify();
            },
        );
    }
    fn report(&self, error: &autocraft_core::Error, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| state.report(error, cx));
    }
    fn choose(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tool) = self.tools.iter().find(|tool| tool.id == id).cloned() else {
            return;
        };
        if tool.available != Availability::Ready {
            return;
        }
        self.export_params = None;
        self.fields.clear();
        self.field_subscriptions.clear();
        for field in schema::fields(&tool.input_schema) {
            let default = if field.kind() == "boolean" && field.default_text().is_empty() {
                "false".to_owned()
            } else {
                field.default_text()
            };
            let input = cx.new(|cx| InputState::new(window, cx).default_value(default));
            self.field_subscriptions
                .push(cx.subscribe(&input, |_, _, _: &InputEvent, cx| cx.notify()));
            self.fields.push(Parameter { field, input });
        }
        self.selected = Some(tool);
        self.error = None;
        self.confirming = false;
        cx.notify();
    }
    fn app_command(&mut self, command: AppCommand, window: &mut Window, cx: &mut Context<Self>) {
        if let AppCommand::Export = command {
            self.export(cx);
            return;
        }
        let event = match command {
            AppCommand::OpenFile => PaletteEvent::OpenFile,
            AppCommand::OpenFolder => PaletteEvent::OpenFolder,
            AppCommand::Project => PaletteEvent::Project,
            AppCommand::Workbench => PaletteEvent::Workbench,
            AppCommand::ToggleMode => PaletteEvent::ToggleMode,
            AppCommand::Settings => PaletteEvent::Settings,
            AppCommand::Home => PaletteEvent::Home,
            AppCommand::Export => return,
        };
        window.close_dialog(cx);
        cx.emit(event);
    }
}
impl Render for CommandPalette {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.selected.is_some() {
            return self.render_form(window, cx);
        }
        let project = self.project.is_some();
        self.commands = AppCommand::ALL
            .into_iter()
            .filter(|command| {
                fuzzy(
                    &rust_i18n::t!(format!("app.palette.{}", command.key())),
                    &self.query,
                )
            })
            .collect();
        let app = CommandGroup::new()
            .label(rust_i18n::t!("app.palette.application"))
            .items(self.commands.iter().copied().map(|command| {
                CommandItem::new()
                    .label(rust_i18n::t!(format!("app.palette.{}", command.key())))
                    .disabled(
                        !project
                            && matches!(
                                command,
                                AppCommand::Project
                                    | AppCommand::Workbench
                                    | AppCommand::ToggleMode
                                    | AppCommand::Export
                            ),
                    )
            }));
        let mut grouped: BTreeMap<String, Vec<&ToolInfo>> = BTreeMap::new();
        for tool in &self.tools {
            let translated = rust_i18n::t!(&tool.title_key);
            if fuzzy(&tool.title, &self.query)
                || fuzzy(&translated, &self.query)
                || fuzzy(&tool.id, &self.query)
            {
                grouped.entry(tool.toolset.clone()).or_default().push(tool);
            }
        }
        self.groups = grouped
            .values()
            .map(|tools| tools.iter().map(|tool| tool.id.clone()).collect())
            .collect();
        let query_owner = cx.entity();
        let mut command = Command::new(&self.command)
            .placeholder(rust_i18n::t!("app.palette.search"))
            .filterable(false)
            .on_query(move |query, _, cx| {
                query_owner.update(cx, |this, cx| {
                    this.query = query.to_owned();
                    cx.notify();
                })
            })
            .group(app);
        for (name, tools) in grouped {
            command = command.group(CommandGroup::new().label(name).items(tools.into_iter().map(
                |tool| {
                    let reason = availability(&tool.available);
                    let translated = rust_i18n::t!(&tool.title_key).into_owned();
                    let title = if translated == tool.title_key {
                        tool.title.clone()
                    } else {
                        translated
                    };
                    let label = if reason.is_empty() {
                        title
                    } else {
                        format!("{title} — {reason}")
                    };
                    CommandItem::new()
                        .label(label)
                        .keywords([tool.id.clone(), tool.summary.clone()])
                        .disabled(tool.available != Availability::Ready)
                },
            )));
        }
        let entity = cx.entity();
        command = command.on_confirm(move |index, window, cx| {
            entity.update(cx, |this, cx| {
                if index.section == 0 {
                    if let Some(command) = this.commands.get(index.row) {
                        this.app_command(*command, window, cx);
                    }
                } else if let Some(id) = this
                    .groups
                    .get(index.section - 1)
                    .and_then(|group| group.get(index.row))
                    .cloned()
                {
                    this.choose(&id, window, cx);
                }
            })
        });
        v_flex()
            .gap_2()
            .child(command)
            .children(self.error.clone())
            .into_any_element()
    }
}
fn availability(value: &Availability) -> String {
    match value {
        Availability::Ready => String::new(),
        Availability::NeedsDeep => rust_i18n::t!("app.palette.needs_deep").into_owned(),
        Availability::NeedsTarget => rust_i18n::t!("app.palette.needs_target").into_owned(),
        Availability::Missing { engine } => {
            rust_i18n::t!("app.palette.missing_engine", engine = engine).into_owned()
        }
        Availability::Os => rust_i18n::t!("app.palette.os_unavailable").into_owned(),
    }
}
fn fuzzy(text: &str, query: &str) -> bool {
    let lowered = text.to_lowercase();
    let mut text = lowered.chars();
    query
        .to_lowercase()
        .chars()
        .filter(|c| !c.is_whitespace())
        .all(|needle| text.by_ref().any(|character| character == needle))
}
#[cfg(test)]
mod tests {
    #[test]
    fn fuzzy_titles_keep_character_order() {
        assert!(super::fuzzy("Native function pseudocode", "nfp"));
        assert!(!super::fuzzy("Native function pseudocode", "pfn"));
    }
}
