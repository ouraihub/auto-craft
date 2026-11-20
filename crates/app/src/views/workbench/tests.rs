//! Editor state survives same-project navigation and graph addresses navigate functions.
use super::{EditorTab, Fill, WorkbenchView};
use crate::state::MainView;
use autocraft_core::{AnalysisMode, Door, NewProject, TargetRefDto, ToolReply};
use gpui_kit::{AppContext as _, TestAppContext};
use serde_json::json;

#[gpui_kit::test]
fn navigation_keeps_results_but_project_switch_clears_them(cx: &mut TestAppContext) {
    let (_fixture, view) = crate::testing::open(cx, WorkbenchView::new);
    cx.run_until_parked();
    let folder = tempfile::tempdir().unwrap();
    let state = cx.update(|cx| view.read(cx).state.clone());
    let projects = cx.update(|cx| {
        (0..2)
            .map(|index| {
                state
                    .read(cx)
                    .services
                    .projects
                    .create(NewProject {
                        name: format!("Result {index}"),
                        request: String::new(),
                        target: TargetRefDto::None,
                        door: Door::MakeMyOwn,
                        folder: Some(folder.path().join(index.to_string())),
                        existing: None,
                        mode: AnalysisMode::Deep,
                        builder: None,
                    })
                    .unwrap()
                    .id
            })
            .collect::<Vec<_>>()
    });
    cx.update(|cx| {
        state.update(cx, |state, cx| {
            state.select_project(projects[0].clone(), cx)
        })
    });
    cx.run_until_parked();
    cx.update(|cx| {
        state.update(cx, |state, cx| state.navigate(MainView::Workbench, cx));
        view.update(cx, |view, cx| {
            view.show_result(ToolReply::Value(json!({"format":"pe","machine":"x64"})), cx)
        });
    });
    cx.run_until_parked();
    cx.update(|cx| {
        let view = view.read(cx);
        assert!(matches!(&view.fill, Fill::Ready(ToolReply::Value(body)) if body["format"] == "pe" && body["machine"] == "x64"));
    });
    cx.update(|cx| {
        state.update(cx, |state, cx| {
            state.select_project(projects[1].clone(), cx)
        })
    });
    cx.run_until_parked();
    cx.update(|cx| {
        let view = view.read(cx);
        assert!(view.result_reply.is_none());
        assert!(matches!(view.fill, Fill::Idle));
    });
}

#[gpui_kit::test]
fn graph_navigation_selects_function_and_pseudocode(cx: &mut TestAppContext) {
    let (_fixture, view) = crate::testing::open(cx, WorkbenchView::new);
    cx.run_until_parked();
    cx.update(|cx| {
        view.update(cx, |view, cx| {
            view.table.update(cx, |table, _| {
                table.delegate_mut().append(
                    vec![
                        json!({"address":"0x10", "name":"first"}),
                        json!({"address":"0x20", "name":"callee"}),
                    ],
                    None,
                )
            });
            view.tab = EditorTab::Graph;
            view.jump("0x20", cx);
        })
    });
    cx.run_until_parked();
    cx.update(|cx| {
        let view = view.read(cx);
        assert_eq!(view.tab, EditorTab::Pseudocode);
        assert_eq!(view.table.read(cx).selected_row(), Some(1));
        assert_eq!(view.selection.as_ref().unwrap()["name"], "callee");
    });
}

#[gpui_kit::test]
fn address_editors_without_selection_wait_instead_of_calling_tools(cx: &mut TestAppContext) {
    let (_fixture, view) = crate::testing::open(cx, WorkbenchView::new);
    cx.run_until_parked();
    cx.update(|cx| {
        view.update(cx, |view, cx| {
            for tab in [
                EditorTab::Pseudocode,
                EditorTab::Disassembly,
                EditorTab::Hex,
                EditorTab::Graph,
            ] {
                view.tab = tab;
                view.load_editor(cx);
                assert!(matches!(view.fill, Fill::Idle));
            }
            assert!(view.tasks.is_empty());
        })
    });
}

#[gpui_kit::test]
fn unknown_probe_uses_the_current_target_path(cx: &mut TestAppContext) {
    use autocraft_core::{CallerDto, CancelFlag, ToolCall};
    use gpui_kit::test::TestWindowExt as _;
    let (fixture, view) = crate::testing::open(cx, WorkbenchView::new);
    drop(std::panic::take_hook());
    cx.run_until_parked();
    let folder = tempfile::tempdir().unwrap();
    let target = folder.path().join("sample.exe");
    std::fs::write(&target, autocraft_testkit::pe64()).unwrap();
    let state = cx.update(|cx| view.read(cx).state.clone());
    let project = cx.update(|cx| {
        state
            .read(cx)
            .services
            .projects
            .create(NewProject {
                name: "Unknown probe".into(),
                request: String::new(),
                target: TargetRefDto::Local {
                    path: target.clone(),
                },
                door: Door::LookInside,
                folder: Some(folder.path().join("project")),
                existing: None,
                mode: AnalysisMode::Deep,
                builder: None,
            })
            .unwrap()
            .id
    });
    cx.update(|cx| state.update(cx, |state, cx| state.select_project(project.clone(), cx)));
    cx.run_until_parked();
    cx.update(|cx| {
        let service = state.read(cx).services.analysis.clone();
        service.call(&project, ToolCall { tool: "core_target_open".into(), params: json!({"path":target}), caller: CallerDto::Ui, consented: false }, &CancelFlag::new()).unwrap();
        service.call(&project, ToolCall { tool: "core_unknown_add".into(), params: json!({"question":"What is this file?", "probes":["core_target_triage"]}), caller: CallerDto::Ui, consented: false }, &CancelFlag::new()).unwrap();
        view.update(cx, |view, cx| { view.right = super::RightTab::Unknowns; view.refresh_snapshot(cx); });
    });
    cx.run_until_parked();
    cx.update_window(fixture.window, |_, window, cx| {
        window.render_frame(cx);
        window.click("core_target_triage", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        let view = view.read(cx);
        let Fill::Ready(reply) = &view.fill else {
            panic!("probe failed: {:?}", view.fill);
        };
        assert_eq!(super::data::result(reply)["kind"], "executable");
    });
}

#[gpui_kit::test]
fn changed_target_refreshes_modules_and_restores_source(cx: &mut TestAppContext) {
    use autocraft_core::{CallerDto, CancelFlag, ToolCall};
    use gpui_kit::component::table::TableDelegate as _;
    let (_fixture, view) = crate::testing::open(cx, WorkbenchView::new);
    drop(std::panic::take_hook());
    cx.run_until_parked();
    let folder = tempfile::tempdir().unwrap();
    let target = folder.path().join("sample.js");
    std::fs::write(&target, "export function answer() { return 42; }\n").unwrap();
    let state = cx.update(|cx| view.read(cx).state.clone());
    let project = cx.update(|cx| {
        state
            .read(cx)
            .services
            .projects
            .create(NewProject {
                name: "Script".into(),
                request: String::new(),
                target: TargetRefDto::Local {
                    path: target.clone(),
                },
                door: Door::LookInside,
                folder: Some(folder.path().join("project")),
                existing: None,
                mode: AnalysisMode::Deep,
                builder: None,
            })
            .unwrap()
            .id
    });
    cx.update(|cx| state.update(cx, |state, cx| state.select_project(project.clone(), cx)));
    cx.run_until_parked();
    cx.update(|cx| {
        let service = state.read(cx).services.analysis.clone();
        service
            .call(
                &project,
                ToolCall {
                    tool: "core_target_open".into(),
                    params: json!({"path":target}),
                    caller: CallerDto::Ui,
                    consented: false,
                },
                &CancelFlag::new(),
            )
            .unwrap();
        state.update(cx, |state, cx| state.refresh_analysis(project.clone(), cx));
    });
    cx.run_until_parked();
    cx.update(|cx| {
        let current = view.read(cx);
        assert_eq!(current.list_kind, super::ListKind::Modules);
        assert_eq!(current.table.read(cx).delegate().rows_count(cx), 1);
        assert_eq!(
            super::data::name(current.table.read(cx).delegate().row(0).unwrap()).as_deref(),
            Some("sample.js")
        );
        view.update(cx, |view, cx| {
            view.tab = EditorTab::Source;
            view.load_editor(cx);
        });
    });
    cx.run_until_parked();
    cx.update(|cx| {
        let view = view.read(cx);
        assert!(
            view.document.is_some(),
            "restored JavaScript must be displayed: {:?}",
            view.fill
        );
        let Fill::Ready(reply) = &view.fill else {
            panic!("restore failed: {:?}", view.fill);
        };
        let result = super::data::result(reply);
        let output = std::path::Path::new(result["output_directory"].as_str().unwrap())
            .join(result["outputs"][0]["path"].as_str().unwrap());
        assert!(
            std::fs::read_to_string(output)
                .unwrap()
                .contains("return 42")
        );
    });
}
