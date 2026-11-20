//! Global MCP server settings.

use autocraft_core::{McpScope, McpServer, McpServerId, McpServerInput};
use gpui_kit::{
    AppContext as _, Context, Entity, Subscription, Window,
    component::input::{InputEvent, InputState, TextareaState},
};

use crate::state::{AppEvent, AppState, run};

mod editor;
mod form;
mod import;
#[cfg(test)]
mod tests;
mod view;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TransportKind {
    Stdio,
    Http,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ImportPreview {
    valid: Vec<McpServerInput>,
    invalid: usize,
}

struct PairFields {
    key: Entity<InputState>,
    value: Entity<InputState>,
}

struct Editor {
    id: Option<McpServerId>,
    revision: u64,
    name: Entity<InputState>,
    command: Entity<InputState>,
    url: Entity<InputState>,
    args: Entity<TextareaState>,
    env: Vec<PairFields>,
    headers: Vec<PairFields>,
    description: Entity<InputState>,
    transport: TransportKind,
    enabled: bool,
    scope: McpScope,
    saving: bool,
}

pub struct McpPage {
    state: Entity<AppState>,
    global: Vec<McpServer>,
    editor: Option<Editor>,
    import: Entity<TextareaState>,
    import_preview: Option<ImportPreview>,
    loading: bool,
    load_generation: u64,
    importing: bool,
    confirm_delete: Option<(McpServerId, u64)>,
    error: Option<&'static str>,
    _subscriptions: Vec<Subscription>,
}

impl McpPage {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let import = cx.new(|cx| {
            TextareaState::new(window, cx)
                .rows(5)
                .placeholder(crate::i18n::text("app.settings.mcp.import_json"))
        });
        let subscriptions = vec![cx.subscribe(&import, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.import_preview = None;
                cx.notify();
            }
        })];
        let mut page = Self {
            state,
            global: Vec::new(),
            editor: None,
            import,
            import_preview: None,
            loading: false,
            load_generation: 0,
            importing: false,
            confirm_delete: None,
            error: None,
            _subscriptions: subscriptions,
        };
        page.reload(cx);
        page
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        self.load_generation += 1;
        let generation = self.load_generation;
        let service = self.state.read(cx).services.mcp.clone();
        self.loading = true;
        run(
            cx,
            move || service.global(),
            move |this, result, cx| {
                if this.load_generation != generation {
                    return;
                }
                this.loading = false;
                match result {
                    Ok(value) => {
                        this.global = value;
                        this.error = None;
                        cx.notify();
                    }
                    Err(error) => this.state.update(cx, |state, cx| state.report(&error, cx)),
                }
            },
        );
    }

    fn toggle(&mut self, server: McpServer, enabled: bool, cx: &mut Context<Self>) {
        let service = self.state.read(cx).services.mcp.clone();
        run(
            cx,
            move || {
                let mut input = server.input.clone();
                input.enabled = enabled;
                service.update(&server.id, server.revision, input).map(drop)
            },
            |this, result, cx| match result {
                Ok(()) => {
                    this.state.update(cx, |_, cx| cx.emit(AppEvent::Routing));
                    this.reload(cx);
                }
                Err(error) => this.state.update(cx, |state, cx| state.report(&error, cx)),
            },
        );
    }

    fn delete(&mut self, server: McpServer, cx: &mut Context<Self>) {
        if self.confirm_delete.as_ref() != Some(&(server.id.clone(), server.revision)) {
            self.confirm_delete = Some((server.id.clone(), server.revision));
            cx.notify();
            return;
        }
        self.confirm_delete = None;
        let service = self.state.read(cx).services.mcp.clone();
        run(
            cx,
            move || service.delete(&server.id, server.revision),
            |this, result, cx| match result {
                Ok(()) => {
                    this.state.update(cx, |_, cx| cx.emit(AppEvent::Routing));
                    this.reload(cx);
                }
                Err(error) => this.state.update(cx, |state, cx| state.report(&error, cx)),
            },
        );
    }
}
