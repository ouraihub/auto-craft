//! Project navigation and project-scoped controls in the native title bar.
mod models;
mod responsive;
mod view;
use crate::{
    panels::settings::SettingsDialog,
    state::{AppEvent, AppState, MainView, run},
};
use autocraft_core::{AnalysisMode, CallerDto, CancelFlag, ProjectPrefs, ProjectStatus, ToolCall};
use autocraft_ui::{
    AutocraftTheme,
    components::{ChoiceMenu, label},
    metrics::{size, space, text},
};
use gpui_kit::{
    AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, MouseButton,
    ParentElement as _, Render, Styled as _, Subscription, Window, WindowControlArea,
    base::h_flex,
    component::{
        Disableable as _, TitleBar,
        button::{Button, ButtonVariants as _},
        menu::{DropdownMenu as _, PopupMenuItem},
    },
};
use lumakit_model_picker::Picker;

pub struct TopBar {
    state: Entity<AppState>,
    confirm_deep: bool,
    changing: bool,
    generation: u64,
    prefs: Option<ProjectPrefs>,
    picker: Entity<Picker>,
    picker_locale: String,
    catalog: Vec<autocraft_core::ModelChoice>,
    backend: Option<autocraft_core::BackendKind>,
    _subscriptions: Vec<Subscription>,
}
impl TopBar {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let picker = cx.new(|cx| Picker::new(window, cx));
        let subscriptions = vec![
            cx.observe(&state, |_, _, cx| cx.notify()),
            cx.subscribe(&state, |this, _, event, cx| {
                if matches!(event, AppEvent::Project | AppEvent::Routing) {
                    this.confirm_deep = false;
                    this.load_models(cx);
                }
                cx.notify();
            }),
            cx.subscribe(&picker, |this, _, event, cx| this.pick_model(event, cx)),
        ];
        let mut this = Self {
            state,
            picker,
            picker_locale: String::new(),
            confirm_deep: false,
            changing: false,
            generation: 0,
            prefs: None,
            catalog: Vec::new(),
            backend: None,
            _subscriptions: subscriptions,
        };
        this.load_models(cx);
        this
    }
    fn building(&self, cx: &Context<Self>) -> bool {
        self.state.read(cx).detail.as_ref().is_some_and(|d| {
            matches!(
                d.summary.status,
                ProjectStatus::Building | ProjectStatus::Checking
            )
        })
    }
    pub fn toggle_mode(&mut self, cx: &mut Context<Self>) {
        if self.building(cx) || self.changing {
            return;
        }
        let state = self.state.read(cx);
        let Some(project) = state.project.clone() else {
            return;
        };
        let mode = state
            .detail
            .as_ref()
            .map(|d| d.mode)
            .unwrap_or(AnalysisMode::CleanRoom);
        if mode == AnalysisMode::CleanRoom && !self.confirm_deep {
            self.confirm_deep = true;
            cx.notify();
            return;
        }
        self.confirm_deep = false;
        self.changing = true;
        let services = state.services.clone();
        let generation = self.generation;
        run(
            cx,
            move || {
                services.analysis.set_mode(
                    &project,
                    if mode == AnalysisMode::CleanRoom {
                        AnalysisMode::Deep
                    } else {
                        AnalysisMode::CleanRoom
                    },
                )?;
                Ok((
                    project.clone(),
                    services.projects.open(&project)?,
                    services.analysis.view(&project)?,
                ))
            },
            move |this, result, cx| {
                if this.generation != generation {
                    return;
                }
                this.changing = false;
                match result {
                    Ok((project, detail, view)) => this.state.update(cx, |state, cx| {
                        if state.project.as_ref() == Some(&project) {
                            state.detail = Some(detail);
                            state.analysis_view = Some(view);
                            cx.notify();
                        }
                    }),
                    Err(error) => this.state.update(cx, |state, cx| state.report(&error, cx)),
                }
                cx.notify();
            },
        );
    }
    fn target_call(
        &mut self,
        tool: &'static str,
        params: serde_json::Value,
        cx: &mut Context<Self>,
    ) {
        let state = self.state.read(cx);
        let Some(project) = state.project.clone() else {
            return;
        };
        let service = state.services.analysis.clone();
        let generation = self.generation;
        run(
            cx,
            move || {
                service.call(
                    &project,
                    ToolCall {
                        tool: tool.into(),
                        params,
                        caller: CallerDto::Ui,
                        consented: true,
                    },
                    &CancelFlag::new(),
                )?;
                Ok((project.clone(), service.view(&project)?))
            },
            move |this, result, cx| {
                if this.generation != generation {
                    return;
                }
                match result {
                    Ok((project, view)) => this.state.update(cx, |state, cx| {
                        if state.project.as_ref() == Some(&project) {
                            state.analysis_view = Some(view);
                            cx.notify();
                        }
                    }),
                    Err(error) => this.state.update(cx, |state, cx| state.report(&error, cx)),
                }
                cx.notify();
            },
        );
    }
    fn open_target(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        let generation = self.generation;
        let picked = cx.prompt_for_paths(gpui_kit::PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: None,
        });
        cx.spawn(async move |this, cx| {
            let result = picked.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                match result {
                    Ok(Ok(Some(paths))) => {
                        if let Some(path) = paths.into_iter().next() {
                            this.target_call(
                                "core_target_open",
                                serde_json::json!({"path": path}),
                                cx,
                            );
                        }
                    }
                    Ok(Ok(None)) => {}
                    Ok(Err(_)) | Err(_) => this.state.update(cx, |state, cx| {
                        state.report(
                            &autocraft_core::Error::invalid(rust_i18n::t!(
                                "app.errors.FILE_PICKER_FAILED"
                            )),
                            cx,
                        )
                    }),
                }
            });
        })
        .detach();
    }
}
