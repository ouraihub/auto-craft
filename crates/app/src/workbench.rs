//! Native window routing; services own state and views retain their UI entities across switches.
mod layout;
use crate::{
    panels::settings::SettingsDialog,
    services::{Channels, Services},
    state::{AppEvent, AppState, MainView, run},
    statusline::StatusLine,
    topbar::TopBar,
    views::{
        agent::{AgentPanel, AgentPanelEvent},
        ask::{AskEvent, AskView},
        game_files::GameFilesView,
        home::HomeView,
        onboarding::{OnboardingEvent, OnboardingView},
        palette::{CommandPalette, PaletteEvent},
        pins::PinsView,
        project::ProjectView,
        workbench::WorkbenchView,
    },
};
use autocraft_analysis::AnalysisEvent;
use autocraft_core::{CallerDto, CancelFlag, RebuildEvent, ToolCall};
use autocraft_ui::{
    AutocraftTheme,
    components::{ActivityItem, IconButton, TextStyled as _, label},
    metrics::{size, space, text},
};
use gpui_kit::{
    AppContext as _, ClipboardItem, Context, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, Styled as _, Subscription, Window,
    assets::IconName,
    base::{h_flex, v_flex},
    component::{
        Sizable as _, WindowExt as _,
        button::{Button, ButtonVariants as _},
    },
    div,
    prelude::FluentBuilder as _,
    px,
};
use rust_i18n::t;
use std::time::Duration;

pub struct Workbench {
    state: Entity<AppState>,
    home: Entity<HomeView>,
    project: Entity<ProjectView>,
    analysis: Entity<WorkbenchView>,
    ask: Entity<AskView>,
    game_files: Entity<GameFilesView>,
    pins: Entity<PinsView>,
    agent: Entity<AgentPanel>,
    onboarding: Entity<OnboardingView>,
    show_onboarding: bool,
    last_main: MainView,
    topbar: Entity<TopBar>,
    statusline: Entity<StatusLine>,
    _palette: Entity<CommandPalette>,
    _subscriptions: Vec<Subscription>,
}
impl Workbench {
    pub fn new(
        services: Services,
        channels: Channels,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let state = cx.new(|cx| AppState::new(services.clone(), cx));
        let home = cx.new(|cx| HomeView::new(state.clone(), window, cx));
        let project = cx.new(|cx| ProjectView::new(state.clone(), window, cx));
        let analysis = cx.new(|cx| WorkbenchView::new(state.clone(), window, cx));
        let agent = cx.new(|cx| AgentPanel::new(state.clone(), window, cx));
        analysis.update(cx, |view, cx| view.set_agent(agent.clone().into(), cx));
        project.update(cx, |view, cx| view.set_agent(agent.clone().into(), cx));
        let ask = cx.new(|cx| AskView::new(state.clone(), window, cx));
        ask.update(cx, |view, cx| view.set_agent(agent.clone(), cx));
        let game_files = cx.new(|cx| GameFilesView::new(state.clone(), window, cx));
        let pins = cx.new(|cx| PinsView::new(state.clone(), window, cx));
        let onboarding = cx.new(|cx| OnboardingView::new(state.clone(), window, cx));
        let topbar = cx.new(|cx| TopBar::new(state.clone(), window, cx));
        let statusline = cx.new(|_| StatusLine::new(state.clone()));
        let palette = cx.new(|cx| CommandPalette::new(state.clone(), window, cx));
        let mut subscriptions = vec![
            cx.observe(&state, |_, _, cx| cx.notify()),
            cx.subscribe(&onboarding, |this, _, _: &OnboardingEvent, cx| {
                this.show_onboarding = false;
                cx.notify();
            }),
            cx.subscribe_in(
                &palette,
                window,
                |this, _, event: &PaletteEvent, window, cx| this.palette_event(event, window, cx),
            ),
            cx.subscribe_in(
                &ask,
                window,
                |this, _, event: &AskEvent, window, cx| match event {
                    AskEvent::Back => this
                        .state
                        .update(cx, |state, cx| state.navigate(MainView::Home, cx)),
                    AskEvent::Workbench => this
                        .state
                        .update(cx, |state, cx| state.navigate(MainView::Workbench, cx)),
                    AskEvent::ExportReport => this
                        ._palette
                        .update(cx, |palette, cx| palette.export_report(window, cx)),
                },
            ),
            cx.subscribe(&agent, |this, _, event: &AgentPanelEvent, cx| match event {
                AgentPanelEvent::Evidence { project, id } => {
                    this.state
                        .update(cx, |state, cx| state.select_workbench(project.clone(), cx));
                    this.analysis
                        .update(cx, |view, cx| view.show_evidence(id.clone(), cx));
                }
            }),
        ];
        subscriptions.extend(crate::appearance::watch(window, cx));
        let keys = palette.downgrade();
        let window_id = window.window_handle();
        subscriptions.push(cx.intercept_keystrokes(move |event, window, cx| {
            if window.window_handle() != window_id {
                return;
            }
            let key = &event.keystroke;
            if key.key == "k" && key.modifiers.control && !key.modifiers.alt {
                cx.stop_propagation();
                if let Err(error) = keys.update(cx, |palette, cx| palette.open(window, cx)) {
                    tracing::debug!(error_kind=?error,"app.palette.closed");
                }
            }
        }));
        let mut this = Self {
            state,
            home,
            project,
            analysis,
            ask,
            game_files,
            pins,
            agent,
            onboarding,
            show_onboarding: false,
            last_main: MainView::Home,
            topbar,
            statusline,
            _palette: palette,
            _subscriptions: subscriptions,
        };
        let prefs = services.app_prefs.clone();
        run(
            cx,
            move || prefs.app_prefs(),
            |this: &mut Self, result, cx| {
                match result {
                    Ok(prefs) => this.show_onboarding = !prefs.onboarding_done,
                    Err(error) => this.state.update(cx, |state, cx| state.report(&error, cx)),
                }
                cx.notify();
            },
        );
        this.pump(channels, cx);
        this
    }
    fn pump(&mut self, channels: Channels, cx: &mut Context<Self>) {
        let Channels {
            sessions,
            analysis,
            rebuild,
            terminals,
            long_tools,
        } = channels;
        cx.spawn(async move |this, cx| {
            while let Ok((span, event)) = sessions.recv().await {
                if this
                    .update(cx, |this, cx| {
                        span.in_scope(|| {
                            this.agent
                                .update(cx, |panel, cx| panel.on_session_event(event, cx))
                        })
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        cx.spawn(async move |this, cx| {
            while let Ok((span, event)) = terminals.recv().await {
                if this
                    .update(cx, |this, cx| {
                        span.in_scope(|| {
                            this.analysis
                                .update(cx, |view, cx| view.on_terminal_event(&event, cx))
                        })
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        cx.spawn(async move |this, cx| {
            while let Ok((span, scope, text)) = long_tools.recv().await {
                if this
                    .update(cx, |this, cx| {
                        span.in_scope(|| {
                            this.agent
                                .update(cx, |panel, cx| panel.on_long_tool_result(scope, text, cx))
                        })
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        cx.spawn(async move |this, cx| {
            while let Ok((span, event)) = rebuild.recv().await {
                if this
                    .update(cx, |this, cx| {
                        span.in_scope(|| {
                            let id = match &event {
                                RebuildEvent::Step { project, .. }
                                | RebuildEvent::Agent { project, .. }
                                | RebuildEvent::NoteRead { project, .. }
                                | RebuildEvent::Ask { project, .. }
                                | RebuildEvent::Checked { project, .. }
                                | RebuildEvent::Preview { project, .. }
                                | RebuildEvent::Finished { project, .. }
                                | RebuildEvent::Queue { project, .. } => project.clone(),
                            };
                            this.statusline
                                .update(cx, |line, cx| line.rebuild(&event, cx));
                            this.agent
                                .update(cx, |panel, cx| panel.on_rebuild_event(event.clone(), cx));
                            this.project
                                .update(cx, |view, cx| view.on_rebuild_event(event, cx));
                            this.state.update(cx, |state, cx| {
                                state.refresh_analysis(id.clone(), cx);
                                cx.emit(AppEvent::Rebuild(id));
                            });
                        })
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        cx.spawn(async move |this, cx| {
            while let Ok((span, event)) = analysis.recv().await {
                if this
                    .update(cx, |this, cx| {
                        span.in_scope(|| this.analysis_event(event, cx))
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        let weak = cx.entity().downgrade();
        cx.spawn(async move |_, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(1500))
                    .await;
                let alive = weak.update(cx, |this, cx| {
                    let project = {
                        let state = this.state.read(cx);
                        state
                            .analysis_view
                            .as_ref()
                            .filter(|v| v.stale)
                            .and(state.project.clone())
                    };
                    if let Some(project) = project {
                        this.state
                            .update(cx, |state, cx| state.refresh_analysis(project, cx));
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
        })
        .detach();
    }
    fn analysis_event(&mut self, event: AnalysisEvent, cx: &mut Context<Self>) {
        match event {
            AnalysisEvent::Changed { project } => self
                .state
                .update(cx, |state, cx| state.refresh_analysis(project, cx)),
            AnalysisEvent::ModeChanged { project } => self.state.update(cx, |state, cx| {
                state.refresh_analysis(project, cx);
                cx.emit(AppEvent::Routing);
            }),
            AnalysisEvent::Inbox { project, .. }
            | AnalysisEvent::InboxWithdrawn { project, .. } => self
                .state
                .update(cx, |_, cx| cx.emit(AppEvent::Inbox(project))),
            AnalysisEvent::Progress {
                project,
                tool,
                message,
                fraction,
            } => {
                if self.state.read(cx).project.as_ref() == Some(&project) {
                    self.analysis.update(cx, |view, cx| {
                        view.on_analysis_progress(&tool, &message, fraction.map(f64::from), cx)
                    });
                    self.statusline
                        .update(cx, |line, cx| line.progress(message, fraction, cx));
                }
            }
            AnalysisEvent::PlanUpdated { project, steps } => {
                let services = self.state.read(cx).services.clone();
                run(
                    cx,
                    move || services.update_plan(&project, &steps),
                    |this, result, cx| {
                        if let Err(error) = result {
                            this.state.update(cx, |state, cx| state.report(&error, cx));
                        }
                    },
                );
            }
        }
    }
    fn palette_event(&mut self, event: &PaletteEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            PaletteEvent::ToolResult(reply) => {
                window.close_dialog(cx);
                self.state
                    .update(cx, |state, cx| state.navigate(MainView::Workbench, cx));
                self.analysis
                    .update(cx, |view, cx| view.show_result(reply.clone(), cx));
            }
            PaletteEvent::Home => self
                .state
                .update(cx, |state, cx| state.navigate(MainView::Home, cx)),
            PaletteEvent::Project => self
                .state
                .update(cx, |state, cx| state.navigate(MainView::Project, cx)),
            PaletteEvent::Workbench => self
                .state
                .update(cx, |state, cx| state.navigate(MainView::Workbench, cx)),
            PaletteEvent::Settings => SettingsDialog::open(self.state.clone(), window, cx),
            PaletteEvent::OpenFile => self.open_target(false, window, cx),
            PaletteEvent::OpenFolder => self.open_target(true, window, cx),
            PaletteEvent::ToggleMode => self.topbar.update(cx, |bar, cx| bar.toggle_mode(cx)),
        }
    }
    fn open_target(&mut self, directory: bool, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(project) = self.state.read(cx).project.clone() else {
            self.state
                .update(cx, |state, cx| state.navigate(MainView::Home, cx));
            return;
        };
        let picked = cx.prompt_for_paths(gpui_kit::PathPromptOptions {
            files: !directory,
            directories: directory,
            multiple: false,
            prompt: None,
        });
        cx.spawn(async move |this, cx| {
            let result = picked.await;
            if this
                .update(cx, |this, cx| match result {
                    Ok(Ok(Some(paths))) => {
                        if let Some(path) = paths.into_iter().next() {
                            let service = this.state.read(cx).services.analysis.clone();
                            run(
                                cx,
                                move || {
                                    service.call(
                                        &project,
                                        ToolCall {
                                            tool: "core_target_open".into(),
                                            params: serde_json::json!({"path":path}),
                                            caller: CallerDto::Ui,
                                            consented: true,
                                        },
                                        &CancelFlag::new(),
                                    )
                                },
                                |this, result, cx| match result {
                                    Ok(reply) => this
                                        .analysis
                                        .update(cx, |view, cx| view.show_result(reply, cx)),
                                    Err(error) => {
                                        this.state.update(cx, |state, cx| state.report(&error, cx))
                                    }
                                },
                            );
                        }
                    }
                    Ok(Ok(None)) => {}
                    Ok(Err(_)) | Err(_) => this.state.update(cx, |state, cx| {
                        state.report(
                            &autocraft_core::Error::invalid(t!("app.errors.FILE_PICKER_FAILED")),
                            cx,
                        )
                    }),
                })
                .is_err()
            {
                tracing::debug!("app.target.view_closed");
            }
        })
        .detach();
    }
    fn notice(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let n = self.state.read(cx).notice.clone()?;
        let r = AutocraftTheme::global(cx).roles;
        Some(
            h_flex()
                .px(space::S12)
                .py(space::S6)
                .gap(space::S8)
                .bg(r.feedback_error_subtle)
                .child(label(text::UI, n.message).flex_1())
                .child(label(text::METADATA, n.code))
                .when_some(n.trace_id, |row, id| {
                    row.child(
                        Button::new("copy-trace")
                            .link()
                            .xsmall()
                            .label(t!("app.shell.copy_trace").into_owned())
                            .on_click(move |_, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(id.clone()))
                            }),
                    )
                })
                .child(
                    IconButton::new("dismiss-notice", IconName::X)
                        .tooltip(t!("app.shell.dismiss").into_owned())
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.state.update(cx, |state, cx| state.dismiss_notice(cx))
                        })),
                ),
        )
    }
}
