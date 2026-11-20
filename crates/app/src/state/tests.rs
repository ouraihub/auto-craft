//! Headless state consumers over isolated real services: notices and project navigation.

use autocraft_core::Error;
use gpui_kit::{Context, Entity, IntoElement, Render, TestAppContext, Window, div};

use super::{AppState, run};
use crate::testing;

struct Host {
    state: Entity<AppState>,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

#[gpui_kit::test]
fn a_failed_background_job_leaves_its_trace_id_on_the_notice(cx: &mut TestAppContext) {
    let (_fixture, host) = testing::open(cx, |state, _, _| Host { state });
    let state = cx.update(|cx| host.read(cx).state.clone());
    cx.run_until_parked();
    cx.update(|cx| {
        state.update(cx, |_, cx| {
            run(
                cx,
                || Err::<(), _>(Error::invalid("boom")),
                |state: &mut AppState, result, cx| {
                    if let Err(error) = result {
                        state.report(&error, cx);
                    }
                },
            )
        })
    });
    cx.run_until_parked();
    let notice = cx
        .update(|cx| state.read(cx).notice.clone())
        .expect("notice shown");
    let trace_id = notice.trace_id.expect("background failure has a TraceId");
    assert_eq!(trace_id.len(), 32);
    assert!(trace_id.bytes().all(|b| b.is_ascii_hexdigit()));

    // Direct failures own a root too, so their notice still has a diagnostic ID.
    cx.update(|cx| state.update(cx, |state, cx| state.report(&Error::invalid("direct"), cx)));
    let direct = cx
        .update(|cx| state.read(cx).notice.clone())
        .expect("notice shown");
    let direct_trace = direct.trace_id.expect("direct failure has a TraceId");
    assert_ne!(direct_trace, trace_id);
}

fn create_project(
    state: &AppState,
    folder: &std::path::Path,
    name: &str,
) -> autocraft_core::Result<autocraft_core::ProjectId> {
    state
        .services
        .projects
        .create(autocraft_core::NewProject {
            name: name.into(),
            request: "Build an isolated static page".into(),
            target: autocraft_core::TargetRefDto::None,
            door: autocraft_core::Door::MakeMyOwn,
            folder: Some(folder.to_path_buf()),
            existing: None,
            mode: autocraft_core::AnalysisMode::CleanRoom,
            builder: None,
        })
        .map(|project| project.id)
}

#[gpui_kit::test]
fn navigation_persists_each_projects_view_and_selection_restores_it(cx: &mut TestAppContext) {
    use super::MainView;
    use autocraft_core::MainViewPref;
    let (_fixture, host) = testing::open(cx, |state, _, _| Host { state });
    let state = cx.update(|cx| host.read(cx).state.clone());
    cx.run_until_parked();
    let folders = tempfile::tempdir().unwrap();
    let (first, second) = cx.update(|cx| {
        let state = state.read(cx);
        (
            create_project(state, &folders.path().join("first"), "First").unwrap(),
            create_project(state, &folders.path().join("second"), "Second").unwrap(),
        )
    });
    cx.update(|cx| state.update(cx, |state, cx| state.select_project(first.clone(), cx)));
    cx.run_until_parked();
    cx.update(|cx| state.update(cx, |state, cx| state.navigate(MainView::Workbench, cx)));
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(state.read(cx).project.as_ref(), Some(&first));
        assert_eq!(
            state
                .read(cx)
                .services
                .project_prefs
                .get(&first)
                .unwrap()
                .view,
            Some(MainViewPref::Workbench)
        );
        state.update(cx, |state, cx| state.select_project(second.clone(), cx));
    });
    cx.run_until_parked();
    cx.update(|cx| state.update(cx, |state, cx| state.navigate(MainView::Project, cx)));
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(
            state
                .read(cx)
                .services
                .project_prefs
                .get(&second)
                .unwrap()
                .view,
            Some(MainViewPref::Project)
        );
        state.update(cx, |state, cx| state.select_project(first.clone(), cx));
    });
    cx.run_until_parked();
    cx.update(|cx| {
        let selected = state.read(cx);
        assert_eq!(selected.project.as_ref(), Some(&first));
        assert_eq!(selected.main, MainView::Workbench);
        assert_eq!(selected.detail.as_ref().unwrap().summary.id, first);
    });
    cx.update(|cx| state.update(cx, |state, cx| state.select_project(second.clone(), cx)));
    cx.run_until_parked();
    cx.update(|cx| {
        let selected = state.read(cx);
        assert_eq!(selected.project.as_ref(), Some(&second));
        assert_eq!(selected.main, MainView::Project);
        assert!(selected.analysis_view.is_some());
        assert!(selected.notice.is_none());
    });
}

#[gpui_kit::test]
fn newer_selection_rejects_an_older_inflight_project_result(cx: &mut TestAppContext) {
    let (_fixture, host) = testing::open(cx, |state, _, _| Host { state });
    let state = cx.update(|cx| host.read(cx).state.clone());
    cx.run_until_parked();
    let folders = tempfile::tempdir().unwrap();
    let (first, second) = cx.update(|cx| {
        let state = state.read(cx);
        (
            create_project(state, &folders.path().join("older"), "Older").unwrap(),
            create_project(state, &folders.path().join("newer"), "Newer").unwrap(),
        )
    });
    // Both jobs are issued before either completion can be delivered to this entity.
    cx.update(|cx| {
        state.update(cx, |state, cx| {
            state.select_project(first.clone(), cx);
            state.select_project(second.clone(), cx);
        })
    });
    cx.run_until_parked();
    cx.update(|cx| {
        let selected = state.read(cx);
        assert_eq!(selected.project.as_ref(), Some(&second));
        assert_eq!(selected.detail.as_ref().unwrap().summary.id, second);
        assert!(selected.analysis_view.is_some());
        assert!(selected.notice.is_none());
        // A discarded selection must not retain an analysis View holder and make deletion BUSY.
        selected.services.projects.delete(&first).unwrap();
    });
}

#[gpui_kit::test]
fn repeated_selection_retains_current_projects_analysis_holder(cx: &mut TestAppContext) {
    let (_fixture, host) = testing::open(cx, |state, _, _| Host { state });
    let state = cx.update(|cx| host.read(cx).state.clone());
    cx.run_until_parked();
    let folder = tempfile::tempdir().unwrap();
    let project = cx.update(|cx| {
        create_project(state.read(cx), &folder.path().join("shared"), "Shared").unwrap()
    });
    cx.update(|cx| {
        state.update(cx, |state, cx| {
            state.select_project(project.clone(), cx);
            state.select_project(project.clone(), cx);
        })
    });
    cx.run_until_parked();
    cx.update(|cx| {
        let selected = state.read(cx);
        assert_eq!(selected.project.as_ref(), Some(&project));
        assert_eq!(
            selected
                .services
                .projects
                .delete(&project)
                .unwrap_err()
                .kind(),
            Some(autocraft_core::ErrorKind::Busy)
        );
        assert!(selected.services.analysis.view(&project).is_ok());
    });
}

#[gpui_kit::test]
fn refreshed_analysis_updates_mode_for_every_project_surface(cx: &mut TestAppContext) {
    use autocraft_core::AnalysisMode;
    let (_fixture, host) = testing::open(cx, |state, _, _| Host { state });
    let state = cx.update(|cx| host.read(cx).state.clone());
    cx.run_until_parked();
    let folder = tempfile::tempdir().unwrap();
    let project = cx
        .update(|cx| create_project(state.read(cx), &folder.path().join("mode"), "Mode").unwrap());
    cx.update(|cx| state.update(cx, |state, cx| state.select_project(project.clone(), cx)));
    cx.run_until_parked();
    cx.update(|cx| {
        state
            .read(cx)
            .services
            .analysis
            .set_mode(&project, AnalysisMode::Deep)
            .unwrap();
        state.update(cx, |state, cx| state.refresh_analysis(project.clone(), cx));
    });
    cx.run_until_parked();
    cx.update(|cx| {
        let state = state.read(cx);
        assert_eq!(state.detail.as_ref().unwrap().mode, AnalysisMode::Deep);
        assert_eq!(
            state.analysis_view.as_ref().unwrap().mode,
            AnalysisMode::Deep
        );
    });
}
