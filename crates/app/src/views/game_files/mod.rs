//! Project-scoped archive browser. Only an explicitly confirmed export writes pictures.
mod actions;
mod data;
mod pickers;
mod render;

use crate::state::{AppEvent, AppState};
use autocraft_core::{AnalysisMode, CancelFlag, ProjectId, TargetId};
use data::{Archive, Item, Saved};
use gpui_kit::{
    AppContext as _, Context, Entity, Subscription, Window,
    component::input::{InputEvent, InputState},
};
use std::path::PathBuf;

#[derive(Clone)]
struct Request {
    tool: String,
    params: serde_json::Value,
    operation: Operation,
}
#[derive(Clone)]
enum Operation {
    Discover { preferred: Option<PathBuf> },
    List { archive: Archive, offset: usize },
    Export { output: PathBuf },
}
enum Outcome {
    Discover(Vec<Archive>, bool),
    List(data::Page),
    Export(Saved),
}
/// Retained game-files page; the shell owns navigation and locale resources.
pub struct GameFilesView {
    state: Entity<AppState>,
    project: Option<ProjectId>,
    target: Option<TargetId>,
    target_path: Option<PathBuf>,
    mode: AnalysisMode,
    generation: u64,
    cancel: CancelFlag,
    busy: bool,
    root: Option<PathBuf>,
    archives: Vec<Archive>,
    archive: Option<Archive>,
    items: Vec<Item>,
    next: Option<usize>,
    filter: Entity<InputState>,
    applied_filter: String,
    pending: Option<Request>,
    confirm_deep: bool,
    error: Option<String>,
    saved: Option<Saved>,
    truncated: bool,
    _subscriptions: Vec<Subscription>,
}
impl std::fmt::Debug for GameFilesView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GameFilesView")
            .field("busy", &self.busy)
            .finish_non_exhaustive()
    }
}
impl GameFilesView {
    /// Creates a page bound to the current project and selected analysis target.
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let filter = cx.new(|cx| {
            InputState::new(window, cx).placeholder(rust_i18n::t!("app.game_files.filter"))
        });
        let subscriptions = vec![
            cx.observe(&state, |this, _, cx| this.sync(cx)),
            cx.subscribe(&state, |this, _, event, cx| {
                if matches!(event, AppEvent::Project | AppEvent::Analysis(_)) {
                    this.sync(cx);
                }
            }),
            cx.subscribe(&filter, |_, _, _: &InputEvent, cx| cx.notify()),
        ];
        let mut this = Self {
            state,
            project: None,
            target: None,
            target_path: None,
            mode: AnalysisMode::CleanRoom,
            generation: 0,
            cancel: CancelFlag::new(),
            busy: false,
            root: None,
            archives: Vec::new(),
            archive: None,
            items: Vec::new(),
            next: None,
            filter,
            pending: None,
            confirm_deep: false,
            error: None,
            saved: None,
            truncated: false,
            _subscriptions: subscriptions,
            applied_filter: String::new(),
        };
        this.sync(cx);
        this
    }
    fn invalidate(&mut self) {
        self.cancel.cancel();
        self.generation += 1;
        self.busy = false;
        self.pending = None;
        self.confirm_deep = false;
        self.error = None;
    }
    fn sync(&mut self, cx: &mut Context<Self>) {
        let state = self.state.read(cx);
        let project = state.project.clone();
        let view = state.analysis_view.as_ref();
        let target = view.and_then(|view| view.active.clone());
        let target_path = view
            .and_then(|view| view.targets.iter().find(|t| Some(&t.id) == target.as_ref()))
            .map(|t| t.path.clone());
        let mode = view.map_or(AnalysisMode::CleanRoom, |v| v.mode);
        if self.project != project || self.target != target || self.target_path != target_path {
            self.invalidate();
            self.project = project;
            self.target = target;
            self.target_path = target_path;
            self.root = self.target_path.clone();
            self.archives.clear();
            self.archive = None;
            self.items.clear();
            self.next = None;
            self.saved = None;
            self.truncated = false;
        }
        if self.mode != mode {
            self.invalidate();
            self.mode = mode;
            self.saved = None;
        }
        cx.notify();
    }
    fn failure(&mut self, error: autocraft_core::Error, cx: &mut Context<Self>) {
        // run owns the final error log; the view only shows a safe, actionable summary.
        let key = match error.kind() {
            Some(autocraft_core::ErrorKind::DeepRequired) => "app.game_files.deep_required",
            Some(autocraft_core::ErrorKind::ConsentRequired) => "app.game_files.consent",
            Some(autocraft_core::ErrorKind::BackendUnavailable) => "app.game_files.unavailable",
            Some(autocraft_core::ErrorKind::Conflict) => "app.game_files.output_exists",
            _ => "app.game_files.failed",
        };
        self.error = Some(format!(
            "{} ({})",
            rust_i18n::t!(key),
            autocraft_core::ErrorCode::code(&error)
        ));
        cx.notify();
    }
}
impl Drop for GameFilesView {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}
