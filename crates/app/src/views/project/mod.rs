//! Project conversation container: cached virtual feed, composer and project controls.
mod actions;
mod cards;
mod feed;
mod load;
mod manage;
mod models;
mod sidebar;

use crate::state::{AppEvent, AppState};
use autocraft_core::{
    ActivityLine, FeedEntryDto, FeedPlanStepDto, InboxRequest, ModelChoice, ModelPrefs,
    ProjectDetail, ProjectId, ProjectPrefs, RunStatus,
};
use autocraft_ui::{AutocraftTheme, metrics::space};
use feed::FeedRow;
use gpui_kit::{
    AnyView, AppContext as _, Context, Entity, IntoElement, ParentElement as _, Pixels, Render,
    Size, Styled as _, Subscription, Window,
    base::{VirtualListScrollHandle, h_flex, resizable_panel, v_flex, v_resizable, v_virtual_list},
    component::input::{InputState, TextareaState},
};
use lumakit_model_picker::Picker;
use std::{path::PathBuf, rc::Rc};

pub struct ProjectView {
    state: Entity<AppState>,
    project: Option<ProjectId>,
    detail: Option<ProjectDetail>,
    prefs: Option<ProjectPrefs>,
    status: Option<RunStatus>,
    inbox: Vec<InboxRequest>,
    answering: Vec<autocraft_core::InboxRequestId>,
    rows: Vec<Entity<FeedRow>>,
    sizes: Rc<Vec<Size<Pixels>>>,
    scroll: VirtualListScrollHandle,
    plan: Vec<FeedPlanStepDto>,
    plan_folded: bool,
    input: Entity<TextareaState>,
    input_restore: Option<String>,
    clear_input: bool,
    pictures: Vec<PathBuf>,
    just_ask: bool,
    request_cancel: autocraft_core::CancelFlag,
    busy: bool,
    attaching: bool,
    loading: bool,
    reload_again: bool,
    generation: u64,
    agent: Option<AnyView>,
    agent_open: bool,
    activity: Vec<ActivityLine>,
    activity_offset: usize,
    activity_open: bool,
    picker: Entity<Picker>,
    picker_locale: String,
    catalog: Vec<ModelChoice>,
    model_prefs: ModelPrefs,
    budget: Entity<InputState>,
    budget_restore: Option<String>,
    _subscriptions: Vec<Subscription>,
}

impl ProjectView {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| TextareaState::new(window, cx).rows(3));
        let budget = cx.new(|cx| InputState::new(window, cx));
        let picker = cx.new(|cx| Picker::new(window, cx));
        let subscriptions = vec![
            cx.subscribe(&state, |this, _, event, cx| match event {
                AppEvent::Project => this.select(cx),
                AppEvent::Inbox(id) if this.project.as_ref() == Some(id) => this.reload(cx),
                AppEvent::Routing => this.load_models(cx),
                AppEvent::Projects
                | AppEvent::Analysis(_)
                | AppEvent::Inbox(_)
                | AppEvent::Rebuild(_) => cx.notify(),
            }),
            cx.subscribe(&picker, |this, _, event, cx| this.pick_model(event, cx)),
            cx.subscribe(
                &input,
                |_, _, _: &gpui_kit::component::input::InputEvent, cx| cx.notify(),
            ),
        ];
        let mut view = Self {
            state,
            project: None,
            detail: None,
            prefs: None,
            status: None,
            inbox: Vec::new(),
            answering: Vec::new(),
            rows: Vec::new(),
            sizes: Rc::new(Vec::new()),
            scroll: VirtualListScrollHandle::new(),
            plan: Vec::new(),
            plan_folded: false,
            input,
            input_restore: None,
            clear_input: false,
            pictures: Vec::new(),
            just_ask: false,
            request_cancel: autocraft_core::CancelFlag::new(),
            busy: false,
            attaching: false,
            loading: false,
            reload_again: false,
            generation: 0,
            agent: None,
            agent_open: false,
            activity: Vec::new(),
            activity_offset: 0,
            activity_open: false,
            picker,
            picker_locale: String::new(),
            catalog: Vec::new(),
            model_prefs: ModelPrefs::default(),
            budget,
            budget_restore: None,
            _subscriptions: subscriptions,
        };
        view.select(cx);
        view
    }

    /// Installs the shared read-only builder chat drawer.
    pub fn set_agent(&mut self, agent: AnyView, cx: &mut Context<Self>) {
        self.agent = Some(agent);
        cx.notify();
    }

    fn writable(&self) -> bool {
        self.detail.as_ref().is_some_and(|d| !d.summary.read_only)
    }
    fn running(&self) -> bool {
        self.status
            .as_ref()
            .is_some_and(|s| s.running || s.queued.is_some())
    }
    fn report(&self, error: &autocraft_core::Error, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| state.report(error, cx));
    }

    fn push(&mut self, entry: FeedEntryDto, cx: &mut Context<Self>) {
        if let FeedEntryDto::Plan { steps } = entry {
            self.plan = steps;
            return;
        }
        let index = self.rows.len();
        let owner = cx.entity().downgrade();
        let generation = self.generation;
        self.rows
            .push(cx.new(|_| FeedRow::new(entry, owner, index, generation)));
        Rc::make_mut(&mut self.sizes).push(gpui_kit::size(gpui_kit::px(0.), gpui_kit::px(100.)));
    }
}

impl Drop for ProjectView {
    fn drop(&mut self) {
        self.request_cancel.cancel();
    }
}

impl Render for ProjectView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        crate::i18n::refresh_picker(&self.picker, cx, &mut self.picker_locale);
        if self.clear_input {
            self.clear_input = false;
            self.input
                .update(cx, |input, cx| input.set_value("", window, cx));
        }
        if let Some(value) = self.input_restore.take() {
            self.input
                .update(cx, |input, cx| input.set_value(value, window, cx));
        }
        if let Some(value) = self.budget_restore.take() {
            self.budget
                .update(cx, |input, cx| input.set_value(value, window, cx));
        }
        let r = AutocraftTheme::global(cx).roles;
        let list = v_virtual_list(
            cx.entity(),
            "project-feed",
            self.sizes.clone(),
            |this, range, _, _| {
                range
                    .map(|index| this.rows[index].clone())
                    .collect::<Vec<_>>()
            },
        )
        .track_scroll(&self.scroll)
        .flex_1()
        .min_h_0()
        .w_full();
        let mut body = h_flex()
            .size_full()
            .bg(r.container_content_bg)
            .text_color(r.text_default)
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .gap(space::S8)
                    .child(
                        v_resizable("project-feed-plan")
                            .child(resizable_panel().child(list))
                            .child(
                                resizable_panel()
                                    .size(gpui_kit::px(120.))
                                    .size_range(gpui_kit::px(32.)..gpui_kit::px(400.))
                                    .visible(!self.plan.is_empty())
                                    .child(self.plan_bar(cx)),
                            ),
                    )
                    .child(self.composer(cx)),
            )
            .child(self.sidebar(cx));
        if self.agent_open
            && let Some(agent) = &self.agent
        {
            body = body.child(v_flex().w(gpui_kit::px(420.)).h_full().child(agent.clone()));
        }
        body
    }
}

#[cfg(test)]
mod tests;
