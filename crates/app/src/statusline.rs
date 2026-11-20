//! Read-only projection of selected-project analysis and rebuild activity.
use crate::state::AppState;
use autocraft_core::{ProjectId, RebuildEvent};
use autocraft_ui::{
    AutocraftTheme,
    components::label,
    metrics::{space, text},
};
use gpui_kit::{
    Context, Entity, IntoElement, ParentElement as _, Render, Styled as _, Subscription, Window,
    base::h_flex,
};
use std::collections::HashMap;
#[derive(Default)]
struct RebuildStatus {
    busy: Option<String>,
    queue: Option<usize>,
    checks: usize,
    passed: usize,
}
pub struct StatusLine {
    state: Entity<AppState>,
    progress: Option<(ProjectId, String, Option<f32>)>,
    rebuilds: HashMap<ProjectId, RebuildStatus>,
    observation: Option<Subscription>,
}
impl StatusLine {
    pub fn new(state: Entity<AppState>) -> Self {
        Self {
            state,
            progress: None,
            rebuilds: HashMap::new(),
            observation: None,
        }
    }
    pub fn progress(&mut self, message: String, fraction: Option<f32>, cx: &mut Context<Self>) {
        self.progress = self
            .state
            .read(cx)
            .project
            .clone()
            .map(|project| (project, message, fraction));
        cx.notify();
    }
    pub fn rebuild(&mut self, event: &RebuildEvent, cx: &mut Context<Self>) {
        match event {
            RebuildEvent::Step { project, text, .. } => {
                let status = self.rebuilds.entry(project.clone()).or_default();
                if status.busy.is_none() {
                    status.checks = 0;
                    status.passed = 0;
                }
                status.busy = Some(text.clone());
                status.queue = None;
            }
            RebuildEvent::Checked { project, result } => {
                let status = self.rebuilds.entry(project.clone()).or_default();
                status.checks += 1;
                status.passed += usize::from(result.passed);
            }
            RebuildEvent::Queue { project, position } => {
                self.rebuilds.entry(project.clone()).or_default().queue = *position
            }
            RebuildEvent::Finished { project, .. } => {
                self.rebuilds.entry(project.clone()).or_default().busy = None;
                if self
                    .progress
                    .as_ref()
                    .is_some_and(|(id, _, _)| id == project)
                {
                    self.progress = None;
                }
            }
            _ => {}
        }
        cx.notify();
    }
}
impl Render for StatusLine {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.observation.is_none() {
            self.observation = Some(cx.observe(&self.state, |_, _, cx| cx.notify()));
        }
        let r = AutocraftTheme::global(cx).roles;
        let state = self.state.read(cx);
        let rebuild = state.project.as_ref().and_then(|id| self.rebuilds.get(id));
        let busy = rebuild
            .and_then(|s| s.busy.clone())
            .or_else(|| state.analysis_view.as_ref().and_then(|v| v.busy.clone()))
            .unwrap_or_else(|| rust_i18n::t!("app.shell.ready").into_owned());
        let progress = self
            .progress
            .as_ref()
            .filter(|(id, _, _)| state.project.as_ref() == Some(id))
            .map(|(_, m, f)| match f {
                Some(f) => format!("{m} {:.0}%", f.clamp(0., 1.) * 100.),
                None => m.clone(),
            })
            .unwrap_or_default();
        let tasks = rebuild
            .filter(|s| s.checks > 0)
            .map(|s| {
                rust_i18n::t!("app.shell.checks", passed = s.passed, total = s.checks).into_owned()
            })
            .unwrap_or_default();
        let queue = rebuild
            .and_then(|s| s.queue)
            .map(|position| rust_i18n::t!("app.shell.queue", position = position).into_owned())
            .unwrap_or_default();
        let pending = state
            .detail
            .as_ref()
            .filter(|d| d.summary.pending_requests > 0)
            .map(|d| {
                rust_i18n::t!("app.shell.pending", count = d.summary.pending_requests).into_owned()
            })
            .unwrap_or_default();
        h_flex()
            .px(space::S12)
            .py(space::S4)
            .gap(space::S16)
            .bg(r.container_header_bg)
            .border_t_1()
            .border_color(r.border_muted)
            .child(label(text::METADATA, busy).text_color(r.text_subtle))
            .child(label(text::METADATA, progress).text_color(r.text_muted))
            .child(label(text::METADATA, tasks).text_color(r.text_muted))
            .child(label(text::METADATA, queue).text_color(r.text_subtle))
            .child(label(text::METADATA, pending).text_color(r.text_subtle))
    }
}
