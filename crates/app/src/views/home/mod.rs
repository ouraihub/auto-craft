//! Home owns creation drafts and delegates persistence to the project services.
mod actions;
mod form;
mod models;
mod projects;
mod references;

use crate::state::{AppEvent, AppState};
use autocraft_core::{
    AnalysisMode, BackendKind, Door, ExistingFilesDto, ModelChoice, ModelPrefs, ModelRef,
    ThinkingLevel,
};
use autocraft_ui::{
    AutocraftTheme,
    components::label,
    metrics::{content, space, text},
};
use gpui_kit::{
    AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, Styled as _,
    Subscription, Window,
    base::v_flex,
    component::{
        input::{InputEvent, InputState, TextareaState},
        scroll::ScrollableElement as _,
    },
};
use lumakit_model_picker::Picker;
use std::path::PathBuf;

#[derive(Clone, Copy, PartialEq, Eq)]
enum TargetKind {
    Website,
    Application,
    Game,
    None,
}

pub struct HomeView {
    state: Entity<AppState>,
    door: Door,
    target_kind: TargetKind,
    mode: AnalysisMode,
    name: Entity<InputState>,
    target: Entity<InputState>,
    picked_target: Option<PathBuf>,
    request: Entity<TextareaState>,
    references: Vec<gpui_kit::Image>,
    budget: Entity<InputState>,
    folder: Option<PathBuf>,
    existing: Option<ExistingFilesDto>,
    folder_has_files: bool,
    folder_loading: bool,
    backend: Option<BackendKind>,
    pending_backend: Option<BackendKind>,
    installed: Vec<BackendKind>,
    catalog: Vec<ModelChoice>,
    keyed_providers: Vec<String>,
    prefs: ModelPrefs,
    model: Option<ModelRef>,
    effort: Option<ThinkingLevel>,
    picker: Entity<Picker>,
    picker_locale: String,
    game_stack: Option<&'static str>,
    game_stacks: Vec<&'static str>,
    generate_assets: bool,
    loading: bool,
    busy: bool,
    generation: u64,
    folder_generation: u64,
    error: Option<String>,
    _subscriptions: Vec<Subscription>,
}

impl HomeView {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let name = cx.new(|cx| InputState::new(window, cx));
        let target = cx.new(|cx| InputState::new(window, cx));
        let request = cx.new(|cx| TextareaState::new(window, cx).rows(4));
        let budget = cx.new(|cx| InputState::new(window, cx).default_value("0"));
        let picker = cx.new(|cx| Picker::new(window, cx));
        let subscriptions = vec![
            cx.subscribe(&state, |this, _, event, cx| {
                if matches!(event, AppEvent::Routing) {
                    this.load_choices(cx);
                }
                cx.notify();
            }),
            cx.subscribe(&picker, |this, _, event, cx| this.pick_model(event, cx)),
            cx.subscribe(&name, |_, _, _: &InputEvent, cx| cx.notify()),
            cx.subscribe(&target, |_, _, _: &InputEvent, cx| cx.notify()),
            cx.subscribe(&request, |_, _, _: &InputEvent, cx| cx.notify()),
            cx.subscribe(&budget, |_, _, _: &InputEvent, cx| cx.notify()),
        ];
        let mut view = Self {
            state,
            door: Door::MakeMyOwn,
            target_kind: TargetKind::Website,
            mode: AnalysisMode::CleanRoom,
            name,
            target,
            picked_target: None,
            request,
            references: Vec::new(),
            budget,
            folder: None,
            existing: None,
            folder_has_files: false,
            folder_loading: false,
            backend: None,
            pending_backend: None,
            installed: Vec::new(),
            catalog: Vec::new(),
            keyed_providers: Vec::new(),
            prefs: ModelPrefs::default(),
            model: None,
            effort: None,
            picker,
            picker_locale: String::new(),
            game_stack: None,
            game_stacks: Vec::new(),
            generate_assets: false,
            loading: false,
            busy: false,
            generation: 0,
            folder_generation: 0,
            error: None,
            _subscriptions: subscriptions,
        };
        view.load_choices(cx);
        view
    }

    fn set_door(&mut self, door: Door, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.door = door;
        self.folder_generation += 1;
        self.folder = None;
        self.folder_has_files = false;
        self.folder_loading = false;
        self.existing = None;
        self.picked_target = None;
        self.mode = if door == Door::LookInside {
            AnalysisMode::Deep
        } else {
            AnalysisMode::CleanRoom
        };
        self.error = None;
        cx.notify();
    }

    fn report(&self, error: &autocraft_core::Error, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| state.report(error, cx));
    }
}

impl Render for HomeView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        crate::i18n::refresh_picker(&self.picker, cx, &mut self.picker_locale);
        if let Some(path) = self.picked_target.take() {
            self.target.update(cx, |input, cx| {
                input.set_value(path.display().to_string(), window, cx)
            });
        }
        let r = AutocraftTheme::global(cx).roles;
        let padding = if window.viewport_size().width < lumakit_design::metrics::size::NARROW {
            space::S16
        } else {
            space::S24
        };
        v_flex()
            .size_full()
            .bg(r.container_content_bg)
            .text_color(r.text_default)
            .overflow_y_scrollbar()
            .p(padding)
            .gap(space::S24)
            .child(
                v_flex()
                    .w_full()
                    .max_w(content::FORM_WIDE)
                    .self_center()
                    .gap(space::S24)
                    .child(label(
                        text::HEADING_XL,
                        rust_i18n::t!("app.home.title").to_string(),
                    ))
                    .child(self.form(
                        (window.viewport_size().width - padding * 2.).min(content::FORM_WIDE),
                        cx,
                    ))
                    .child(self.projects(cx)),
            )
    }
}
