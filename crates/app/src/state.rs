//! Shared application services and user-facing failure notices.
mod navigation;

use autocraft_core::{
    AnalysisService, AnalysisView, Error, ErrorCode, MainViewPref, ProjectDetail, ProjectId,
    ProjectPrefChange, ProjectSummary, Result,
};
use gpui_kit::{Context, EventEmitter};
use tracing::debug;

use crate::services::Services;

/// A slot serializes the last close with a replacement lease, never with engine loading.
type ViewSlot = std::sync::Arc<std::sync::Mutex<ViewSlotState>>;
#[derive(Default)]
struct ViewSlotState {
    lease: std::sync::Weak<ViewLease>,
    opened: std::sync::Arc<std::sync::Mutex<bool>>,
}
#[derive(Default)]
struct ViewLeases(std::sync::Mutex<std::collections::HashMap<ProjectId, ViewSlot>>);
impl ViewLeases {
    fn pending(
        &self,
        analysis: std::sync::Arc<dyn AnalysisService>,
        project: ProjectId,
    ) -> std::sync::Arc<ViewLease> {
        let slot = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .entry(project.clone())
            .or_default()
            .clone();
        let mut current = slot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(lease) = current.lease.upgrade() {
            return lease;
        }
        let lease = std::sync::Arc::new(ViewLease {
            analysis,
            project,
            opened: current.opened.clone(),
            slot: slot.clone(),
        });
        current.lease = std::sync::Arc::downgrade(&lease);
        lease
    }
}
struct ViewLease {
    analysis: std::sync::Arc<dyn AnalysisService>,
    project: ProjectId,
    opened: std::sync::Arc<std::sync::Mutex<bool>>,
    slot: ViewSlot,
}
impl ViewLease {
    fn open(&self) -> Result<()> {
        let mut opened = self
            .opened
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !*opened {
            self.analysis.open(&self.project)?;
            *opened = true;
        }
        Ok(())
    }
}
impl Drop for ViewLease {
    fn drop(&mut self) {
        // If a replacement won the slot before this destructor, it inherits the same open bit.
        let slot = self
            .slot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !std::ptr::eq(slot.lease.as_ptr(), self) {
            return;
        }
        let mut opened = self
            .opened
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if *opened {
            *opened = false;
            match self.analysis.close(&self.project) {
                Ok(()) => {}
                Err(error) if error.kind() == Some(autocraft_core::ErrorKind::Busy) => {}
                Err(error) => {
                    tracing::warn!(code = error.code(), "app.project.view_release_failed")
                }
            }
        }
    }
}

/// Runs blocking service work on the background executor and hands the result back to the view.
/// Each call is one `ui.background` root: ENTRY when issued and one job terminal after the
/// completion runs. A closed view cancels delivery. The job and completion share the root span;
/// [`AppState::report`] attaches that TraceId to any notice it shows. Panics propagate unchanged.
#[track_caller]
pub fn run<V: 'static, T: Send + 'static>(
    cx: &mut Context<V>,
    job: impl FnOnce() -> Result<T> + Send + 'static,
    done: impl FnOnce(&mut V, Result<T>, &mut Context<V>) + 'static,
) {
    let caller = std::panic::Location::caller();
    let (trace, root) = autocraft_observability::TraceContext::root("ui.background", "ui");
    let span = root.span().clone();
    span.in_scope(|| debug!(at = %caller, "ui.background.issued"));
    let job_span = span.clone();
    let task = cx
        .background_executor()
        .spawn(async move { job_span.in_scope(job) });
    cx.spawn(async move |this, cx| {
        let result = task.await;
        let _entered = span.enter();
        let code = result.as_ref().err().map(ErrorCode::code);
        let _current = CurrentTrace::set(trace.trace_id());
        let applied = this.update(cx, |view, cx| done(view, result, cx)).is_ok();
        if !applied {
            debug!(at = %caller, "ui.background.view_closed");
        }
        match code {
            Some(code) => root.fail(code),
            None if !applied => root.cancel("VIEW_CLOSED"),
            None => root.finish("SUCCESS"),
        }
    })
    .detach();
}

thread_local! {
    static CURRENT_TRACE: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

/// The TraceId of the `run` completion on this (UI) thread; restored when dropped.
struct CurrentTrace(Option<String>);

impl CurrentTrace {
    fn set(trace_id: &str) -> Self {
        Self(CURRENT_TRACE.with(|c| c.replace(Some(trace_id.to_owned()))))
    }

    fn get() -> Option<String> {
        CURRENT_TRACE.with(|c| c.borrow().clone())
    }
}

impl Drop for CurrentTrace {
    fn drop(&mut self) {
        CURRENT_TRACE.with(|c| *c.borrow_mut() = self.0.take());
    }
}

/// A user-facing failure: the stable code, the human message and, when it came from a
/// background job, the TraceId that finds its log lines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub code: &'static str,
    pub message: String,
    pub trace_id: Option<String>,
}

impl Notice {
    pub fn from_error(error: &Error) -> Self {
        Self {
            code: error.code(),
            message: error.to_string(),
            trace_id: CurrentTrace::get(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MainView {
    Home,
    Project,
    Workbench,
    Ask,
    GameFiles,
    Pins,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppEvent {
    Projects,
    Project,
    Routing,
    Analysis(ProjectId),
    Inbox(ProjectId),
    Rebuild(ProjectId),
}

pub struct AppState {
    pub services: Services,
    pub projects: Vec<ProjectSummary>,
    pub project: Option<ProjectId>,
    pub main: MainView,
    pub analysis_view: Option<AnalysisView>,
    pub detail: Option<ProjectDetail>,
    pub notice: Option<Notice>,
    generation: u64,
    view_leases: ViewLeases,
    view_lease: Option<std::sync::Arc<ViewLease>>,
}

impl EventEmitter<AppEvent> for AppState {}

impl std::fmt::Debug for AppState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppState").finish_non_exhaustive()
    }
}

impl AppState {
    pub fn new(services: Services, cx: &mut Context<Self>) -> Self {
        let mut state = Self {
            services,
            projects: Vec::new(),
            project: None,
            main: MainView::Home,
            analysis_view: None,
            detail: None,
            notice: None,
            generation: 0,
            view_leases: ViewLeases::default(),
            view_lease: None,
        };
        state.reload(cx);
        state
    }

    pub fn reload(&mut self, cx: &mut Context<Self>) {
        let projects = self.services.projects.clone();
        run(
            cx,
            move || projects.list(),
            |this, result, cx| {
                match result {
                    Ok(projects) => {
                        this.projects = projects;
                        if this
                            .project
                            .as_ref()
                            .is_some_and(|id| !this.projects.iter().any(|p| &p.id == id))
                        {
                            this.project = None;
                            this.view_lease = None;
                            this.detail = None;
                            this.analysis_view = None;
                            this.main = MainView::Home;
                            this.generation += 1;
                            cx.emit(AppEvent::Project);
                        }
                        cx.emit(AppEvent::Projects);
                    }
                    Err(error) => this.report(&error, cx),
                }
                cx.notify();
            },
        );
    }

    pub fn refresh_analysis(&mut self, project: ProjectId, cx: &mut Context<Self>) {
        if self.project.as_ref() != Some(&project) {
            return;
        }
        let service = self.services.analysis.clone();
        let projects = self.services.projects.clone();
        let generation = self.generation;
        let target = project.clone();
        run(
            cx,
            move || Ok((service.view(&target)?, projects.open(&target)?)),
            move |this, result, cx| {
                if this.generation != generation || this.project.as_ref() != Some(&project) {
                    return;
                }
                match result {
                    Ok((view, detail)) => {
                        this.analysis_view = Some(view);
                        this.detail = Some(detail);
                    }
                    Err(error) => this.report(&error, cx),
                }
                cx.emit(AppEvent::Analysis(project));
                cx.notify();
            },
        );
    }

    /// Shows a failure; the background root owns its log, or a direct report creates one.
    pub fn report(&mut self, error: &Error, cx: &mut Context<Self>) {
        self.notice = Some(if CurrentTrace::get().is_some() {
            Notice::from_error(error)
        } else {
            let (trace, root) = autocraft_observability::TraceContext::root("ui.notice", "ui");
            let _current = CurrentTrace::set(trace.trace_id());
            let notice = Notice::from_error(error);
            root.fail(error.code());
            notice
        });
        cx.notify();
    }

    pub fn dismiss_notice(&mut self, cx: &mut Context<Self>) {
        self.notice = None;
        cx.notify();
    }
}

#[cfg(test)]
mod tests;
