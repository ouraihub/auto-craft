//! Project selection leases and persisted page navigation.
use super::*;

impl AppState {
    pub fn select_project(&mut self, id: ProjectId, cx: &mut Context<Self>) {
        self.select(id, None, cx);
    }
    pub fn select_workbench(&mut self, id: ProjectId, cx: &mut Context<Self>) {
        self.select(id, Some(MainView::Workbench), cx);
    }
    pub fn select_ask(&mut self, id: ProjectId, cx: &mut Context<Self>) {
        self.select(id, Some(MainView::Ask), cx);
    }
    fn select(&mut self, id: ProjectId, requested: Option<MainView>, cx: &mut Context<Self>) {
        self.generation += 1;
        let generation = self.generation;
        let services = self.services.clone();
        let lease = self
            .view_leases
            .pending(services.analysis.clone(), id.clone());
        let target = id.clone();
        run(
            cx,
            move || {
                let detail = services.projects.open(&target)?;
                lease.open()?;
                let view = services.analysis.view(&target)?;
                let prefs = services.project_prefs.get(&target)?;
                let main = requested.unwrap_or(match prefs.view {
                    Some(MainViewPref::Project) => MainView::Project,
                    Some(MainViewPref::Workbench) => MainView::Workbench,
                    Some(MainViewPref::Ask) => MainView::Ask,
                    Some(MainViewPref::GameFiles) => MainView::GameFiles,
                    Some(MainViewPref::Pins) => MainView::Pins,
                    None if detail.summary.request.is_empty() => MainView::Ask,
                    None => MainView::Project,
                });
                Ok((detail, view, main, lease))
            },
            move |this, result, cx| {
                if this.generation != generation {
                    tracing::debug!("app.project.selection_stale");
                    return;
                }
                match result {
                    Ok((detail, view, main, lease)) => {
                        this.view_lease = Some(lease);
                        this.project = Some(id);
                        this.detail = Some(detail);
                        this.analysis_view = Some(view);
                        this.main = main;
                        cx.emit(AppEvent::Project);
                    }
                    Err(error) => this.report(&error, cx),
                }
                cx.notify();
            },
        );
    }
    pub fn navigate(&mut self, view: MainView, cx: &mut Context<Self>) {
        self.main = if self.project.is_none() {
            MainView::Home
        } else {
            view
        };
        let saved_view = match self.main {
            MainView::Home => None,
            MainView::Project => Some(MainViewPref::Project),
            MainView::Workbench => Some(MainViewPref::Workbench),
            MainView::Ask => Some(MainViewPref::Ask),
            MainView::GameFiles => Some(MainViewPref::GameFiles),
            MainView::Pins => Some(MainViewPref::Pins),
        };
        if let (Some(project), Some(view)) = (self.project.clone(), saved_view) {
            let prefs = self.services.project_prefs.clone();
            run(
                cx,
                move || {
                    let saved = prefs.get(&project)?;
                    prefs
                        .set(
                            &project,
                            ProjectPrefChange::View(Some(view)),
                            saved.revision,
                        )
                        .map(drop)
                },
                |this, result, cx| {
                    if let Err(error) = result {
                        this.report(&error, cx);
                    }
                },
            );
        }
        cx.emit(AppEvent::Project);
        cx.notify();
    }
}
