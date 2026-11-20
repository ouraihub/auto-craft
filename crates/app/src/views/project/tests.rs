//! Project preferences and authoritative feed refreshes in a real GPUI test window.
use super::ProjectView;
use autocraft_core::{
    AnalysisMode, BackendKind, Capabilities, ContentBlock, Door, FeedEntryDto, Message, NewProject,
    RebuildEvent, Role, SessionEvent, SessionPhase, SessionSnapshot, TargetRefDto,
};
use ca_engine::{
    feed::{FeedItem, save},
    inbox::{Inbox, RequestKind},
};
use gpui_kit::{AppContext as _, TestAppContext, test::TestWindowExt as _};

#[gpui_kit::test]
fn changing_project_mode_updates_preferences_without_conflict(cx: &mut TestAppContext) {
    let (_fixture, view) = crate::testing::open(cx, ProjectView::new);
    cx.run_until_parked();
    let folder = tempfile::tempdir().unwrap();
    let project = cx.update(|cx| {
        let state = view.read(cx).state.clone();
        let project = state
            .read(cx)
            .services
            .projects
            .create(NewProject {
                name: "Mode ownership".into(),
                request: "Build a page".into(),
                target: TargetRefDto::None,
                door: Door::MakeMyOwn,
                folder: Some(folder.path().join("project")),
                existing: None,
                mode: AnalysisMode::CleanRoom,
                builder: None,
            })
            .unwrap()
            .id;
        state.update(cx, |state, cx| state.select_project(project.clone(), cx));
        project
    });
    cx.run_until_parked();
    cx.update(|cx| view.update(cx, |view, cx| view.set_mode(AnalysisMode::Deep, cx)));
    cx.run_until_parked();
    cx.update(|cx| {
        let view = view.read(cx);
        assert_eq!(view.detail.as_ref().unwrap().mode, AnalysisMode::Deep);
        assert_eq!(
            view.state
                .read(cx)
                .services
                .project_prefs
                .get(&project)
                .unwrap()
                .mode,
            AnalysisMode::Deep
        );
        assert!(view.state.read(cx).notice.is_none());
    });
}

#[gpui_kit::test]
fn inbox_before_repeated_ask_refreshes_one_builder_card_and_cannot_reopen_answer(
    cx: &mut TestAppContext,
) {
    let (fixture, view) = crate::testing::open(cx, ProjectView::new);
    cx.run_until_parked();
    let folder = tempfile::tempdir().unwrap();
    let (project, request, rebuild) = cx.update(|cx| {
        let state = view.read(cx).state.clone();
        let project = state
            .read(cx)
            .services
            .projects
            .create(NewProject {
                name: "Inbox relay".into(),
                request: "Build a page".into(),
                target: TargetRefDto::None,
                door: Door::MakeMyOwn,
                folder: Some(folder.path().join("project")),
                existing: None,
                mode: AnalysisMode::CleanRoom,
                builder: None,
            })
            .unwrap()
            .id;
        save(std::path::Path::new(project.as_str()), &[]).unwrap();
        let inbox = Inbox::new(std::path::Path::new(project.as_str()).join("inbox"));
        inbox
            .post(
                RequestKind::Approval,
                "Write outside project",
                "Permission required",
                vec![],
                false,
                None,
            )
            .unwrap();
        let request = state
            .read(cx)
            .services
            .inbox
            .pending(&project)
            .unwrap()
            .remove(0);
        let rebuild = state.update(cx, |state, cx| {
            let rebuild = state.services.rebuild.take();
            state.select_project(project.clone(), cx);
            rebuild
        });
        (project, request, rebuild)
    });
    cx.run_until_parked();
    let row = cx.update(|cx| {
        let row = view.read(cx).rows[0].clone();
        assert!(row.read(cx).analysis_source());
        row
    });
    cx.update_window(fixture.window, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find_all("allow_once").len(), 1);
    })
    .unwrap();
    cx.update(|cx| {
        let state = view.read(cx).state.clone();
        state.update(cx, |state, _| state.services.rebuild = rebuild);
        view.update(cx, |view, cx| {
            view.reload(cx);
            for _ in 0..2 {
                view.on_rebuild_event(
                    RebuildEvent::Ask {
                        project: project.clone(),
                        request: request.clone(),
                    },
                    cx,
                );
            }
            assert!(view.reload_again);
            assert_eq!(
                view.rows.len(),
                1,
                "Ask invalidations never append a duplicate card"
            );
        });
    });
    cx.run_until_parked();
    cx.update(|cx| {
        let view = view.read(cx);
        assert_eq!(view.rows.len(), 1);
        assert_eq!(view.rows[0], row, "the card keeps its retained UI state");
        assert!(
            !row.read(cx).analysis_source(),
            "the authoritative builder card replaces the inbox fallback"
        );
    });
    cx.update_window(fixture.window, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find_all("allow_once").len(), 1);
        window.click("allow_once", cx);
    })
    .unwrap();
    cx.run_until_parked();
    // A queued relay may still arrive after the person has already answered.
    cx.update(|cx| {
        view.update(cx, |view, cx| {
            view.on_rebuild_event(RebuildEvent::Ask { project, request }, cx)
        })
    });
    cx.run_until_parked();
    cx.update_window(fixture.window, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find_all("allow_once").is_empty());
        assert!(window.find_all("always_allow").is_empty());
        assert!(window.find_all("refuse").is_empty());
        assert!(window.find_all("cancel-request").is_empty());
    })
    .unwrap();
    cx.update(|cx| assert!(view.read(cx).inbox.is_empty()));
}

#[gpui_kit::test]
fn delta_snapshot_and_persisted_reload_render_one_reply_without_losing_tool_fold(
    cx: &mut TestAppContext,
) {
    let (fixture, view) = crate::testing::open(cx, ProjectView::new);
    cx.run_until_parked();
    let folder = tempfile::tempdir().unwrap();
    let project = cx.update(|cx| {
        let state = view.read(cx).state.clone();
        let project = state
            .read(cx)
            .services
            .projects
            .create(NewProject {
                name: "Feed continuity".into(),
                request: "Build a page".into(),
                target: TargetRefDto::None,
                door: Door::MakeMyOwn,
                folder: Some(folder.path().join("project")),
                existing: None,
                mode: AnalysisMode::CleanRoom,
                builder: None,
            })
            .unwrap()
            .id;
        state.update(cx, |state, cx| state.select_project(project.clone(), cx));
        project
    });
    let root = std::path::Path::new(project.as_str());
    let work = FeedItem::Work {
        tools: vec![("write".into(), Some(true))],
        notes: vec![],
    };
    save(
        root,
        &[
            work.clone(),
            FeedItem::Agent {
                text: "Hel".into(),
                streaming: true,
            },
        ],
    )
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        view.update(cx, |view, cx| {
            // The live Continuity exposes streaming DTOs; disk reloads below are settled.
            view.update_feed(
                Some(vec![
                    FeedEntryDto::Work {
                        tools: vec![autocraft_core::FeedToolDto {
                            name: "write".into(),
                            ok: Some(true),
                        }],
                        notes: vec![],
                    },
                    FeedEntryDto::Agent {
                        text: "Hel".into(),
                        streaming: true,
                    },
                ]),
                cx,
            );
        });
        assert!(matches!(
            view.read(cx).rows[1].read(cx).entry(),
            FeedEntryDto::Agent { text, streaming: true } if text == "Hel"
        ));
    });
    cx.update_window(fixture.window, |_, window, cx| {
        window.render_frame(cx);
        window.click("tools-fold", cx);
    })
    .unwrap();
    let row = cx.update(|cx| view.read(cx).rows[1].clone());
    let message = |text: &str| Message {
        id: "assistant-1".into(),
        role: Role::Assistant,
        content: vec![ContentBlock::Text { text: text.into() }],
    };
    cx.update(|cx| {
        view.update(cx, |view, cx| {
            view.on_rebuild_event(
                RebuildEvent::Agent {
                    project: project.clone(),
                    event: SessionEvent::Message {
                        chat: "builder".into(),
                        revision: 1,
                        message: message("Hel"),
                    },
                },
                cx,
            );
            let replies = view
                .rows
                .iter()
                .filter_map(|row| {
                    if let FeedEntryDto::Agent { text, .. } = row.read(cx).entry() {
                        Some(text.as_str())
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>();
            assert_eq!(
                replies,
                ["Hel"],
                "a delayed Message must not append a second reply"
            );
        })
    });
    cx.run_until_parked();
    save(
        root,
        &[
            work,
            FeedItem::Agent {
                text: "Hello".into(),
                streaming: false,
            },
        ],
    )
    .unwrap();
    cx.update(|cx| {
        view.update(cx, |view, cx| {
            view.reload(cx);
            view.on_rebuild_event(
                RebuildEvent::Agent {
                    project: project.clone(),
                    event: SessionEvent::Delta {
                        chat: "builder".into(),
                        revision: 2,
                        message_id: "assistant-1".into(),
                        block: 0,
                        text: "lo".into(),
                    },
                },
                cx,
            );
            view.on_rebuild_event(
                RebuildEvent::Agent {
                    project: project.clone(),
                    event: SessionEvent::Snapshot {
                        snapshot: Box::new(SessionSnapshot {
                            id: "builder".into(),
                            backend: BackendKind::Omp,
                            native_session_key: None,
                            cwd: root.into(),
                            phase: SessionPhase::Idle,
                            model: None,
                            messages: vec![message("Hello")],
                            interactions: vec![],
                            capabilities: Capabilities::default(),
                            output_truncated: false,
                            last_error: None,
                            commands: vec![],
                            session_name: None,
                            revision: 3,
                        }),
                    },
                },
                cx,
            );
        })
    });
    cx.run_until_parked();
    cx.update(|cx| {
        let view = view.read(cx);
        let replies = view
            .rows
            .iter()
            .filter_map(|row| {
                if let FeedEntryDto::Agent { text, streaming } = row.read(cx).entry() {
                    Some((text.as_str(), *streaming))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        assert_eq!(replies, [("Hello", false)]);
        assert_eq!(
            view.rows[1], row,
            "refreshes retain markdown selection state"
        );
        assert!(
            !view.rows[0].read(cx).folded(),
            "tool expansion survives refreshes"
        );
    });
    cx.update_window(fixture.window, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find_all("tools-fold").len(), 1);
    })
    .unwrap();
}
