//! Five-step first-run flow; completion is persisted before the shell is dismissed.
use crate::{
    panels::settings::SettingsDialog,
    state::{AppEvent, AppState, run},
};
use autocraft_core::{BackendKind, ModelPrefs};
use autocraft_ui::{
    AutocraftTheme,
    components::label,
    metrics::{space, text},
};
use gpui_kit::{
    Context, Entity, EventEmitter, IntoElement, ParentElement as _, Render, Styled as _,
    Subscription, Window,
    assets::IconName,
    base::{h_flex, v_flex},
    component::{
        Disableable as _,
        button::{Button, ButtonVariants as _},
        menu::{DropdownMenu as _, PopupMenuItem},
    },
    prelude::FluentBuilder as _,
};

#[derive(Clone, Copy)]
pub enum OnboardingEvent {
    Completed,
}

pub struct OnboardingView {
    state: Entity<AppState>,
    step: usize,
    installed: Vec<BackendKind>,
    prefs: ModelPrefs,
    selected_agent: Option<BackendKind>,
    installing: Option<BackendKind>,
    busy: bool,
    generation: u64,
    model: Option<String>,
    reload_pending: bool,
    _routing: Subscription,
}
impl EventEmitter<OnboardingEvent> for OnboardingView {}
impl OnboardingView {
    pub fn new(state: Entity<AppState>, _: &mut Window, cx: &mut Context<Self>) -> Self {
        let routing = cx.subscribe(&state, |this: &mut Self, _, event: &AppEvent, cx| {
            if matches!(event, AppEvent::Routing) {
                if this.busy {
                    this.reload_pending = true;
                } else {
                    this.reload(cx);
                }
            }
        });
        let mut view = Self {
            state,
            step: 0,
            installed: Vec::new(),
            prefs: ModelPrefs::default(),
            selected_agent: None,
            installing: None,
            busy: false,
            generation: 0,
            model: None,
            reload_pending: false,
            _routing: routing,
        };
        view.reload(cx);
        view
    }
    fn reload(&mut self, cx: &mut Context<Self>) {
        self.busy = true;
        self.generation += 1;
        let generation = self.generation;
        let services = self.state.read(cx).services.clone();
        run(
            cx,
            move || {
                let mut installed = Vec::new();
                for backend in BackendKind::ALL {
                    if services.engines.status(backend)?.is_some() {
                        installed.push(backend);
                    }
                }
                let prefs = services.prefs.prefs()?;
                let model = services
                    .router
                    .settings_catalog()?
                    .into_iter()
                    .find(|choice| {
                        prefs.default_model.as_ref().is_some_and(|model| {
                            model.provider == choice.provider && model.model_id == choice.model.id
                        })
                    })
                    .map(|choice| format!("{} · {}", choice.provider_name, choice.model.name));
                Ok((installed, prefs, model))
            },
            move |this, result, cx| {
                if this.generation != generation {
                    tracing::debug!("onboarding.load.stale");
                    return;
                }
                match result {
                    Ok((installed, prefs, model)) => {
                        this.model = model;
                        this.installed = installed;
                        this.prefs = prefs;
                        if !this.installed.is_empty() {
                            this.selected_agent = this
                                .prefs
                                .default_agent
                                .filter(|backend| this.installed.contains(backend))
                                .or_else(|| {
                                    this.selected_agent
                                        .filter(|backend| this.installed.contains(backend))
                                });
                        }
                    }
                    Err(error) => this.state.update(cx, |state, cx| state.report(&error, cx)),
                }
                this.busy = false;
                if std::mem::take(&mut this.reload_pending) {
                    this.reload(cx);
                }
                cx.notify();
            },
        );
    }
    fn install(&mut self, backend: BackendKind, cx: &mut Context<Self>) {
        if self.busy || self.installing.is_some() {
            return;
        }
        self.installing = Some(backend);
        cx.notify();
        let engines = self.state.read(cx).services.engines.clone();
        run(
            cx,
            move || engines.install(backend),
            move |this, result, cx| {
                this.installing = None;
                match result {
                    Ok(_) => {
                        if !this.installed.contains(&backend) {
                            this.installed.push(backend);
                        }
                        if this.selected_agent == Some(backend) {
                            this.choose_default(backend, cx);
                        }
                    }
                    Err(error) => this.state.update(cx, |state, cx| state.report(&error, cx)),
                }
                cx.notify();
            },
        );
    }
    fn choose_default(&mut self, backend: BackendKind, cx: &mut Context<Self>) {
        if self.busy || !self.installed.contains(&backend) {
            return;
        }
        self.busy = true;
        self.selected_agent = Some(backend);
        let prefs = self.state.read(cx).services.prefs.clone();
        run(
            cx,
            move || prefs.set_default_agent(Some(backend)),
            |this, result, cx| this.saved(result, cx),
        );
    }
    fn saved(&mut self, result: autocraft_core::Result<ModelPrefs>, cx: &mut Context<Self>) {
        self.busy = false;
        match result {
            Ok(prefs) => {
                self.prefs = prefs;
                self.state.update(cx, |_, cx| cx.emit(AppEvent::Routing));
            }
            Err(error) => {
                self.selected_agent = self.prefs.default_agent;
                self.state.update(cx, |state, cx| state.report(&error, cx));
            }
        }
        cx.notify();
    }
    fn advance(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        if self.step < 4 {
            self.step += 1;
            cx.notify();
        } else {
            self.finish(cx);
        }
    }
    fn finish(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.busy = true;
        let prefs = self.state.read(cx).services.app_prefs.clone();
        run(
            cx,
            move || prefs.set_onboarding_done(true),
            |this, result, cx| {
                this.busy = false;
                match result {
                    Ok(_) => cx.emit(OnboardingEvent::Completed),
                    Err(error) => this.state.update(cx, |state, cx| state.report(&error, cx)),
                }
                cx.notify();
            },
        );
    }

    fn agent_menu(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let current = self.selected_agent;
        let installed =
            BackendKind::ALL.map(|backend| (backend, self.installed.contains(&backend)));
        let busy = self.busy;
        let entity = cx.entity();
        let title = current.map_or_else(
            || rust_i18n::t!("app.home.choose").to_string(),
            |backend| {
                format!(
                    "{} · {}",
                    backend.label(),
                    rust_i18n::t!(if self.installed.contains(&backend) {
                        "app.onboarding.installed"
                    } else {
                        "app.onboarding.not_installed"
                    })
                )
            },
        );
        h_flex()
            .flex_wrap()
            .gap(space::S12)
            .child(
                Button::new("onboarding-agent")
                    .outline()
                    .label(title.clone())
                    .accessibility_label(format!(
                        "{}: {title}",
                        rust_i18n::t!("app.onboarding.agents")
                    ))
                    .icon(IconName::ChevronDown)
                    .dropdown_menu(move |menu, _, _| {
                        installed
                            .into_iter()
                            .fold(menu, |menu, (backend, available)| {
                                let entity = entity.clone();
                                menu.item(
                                    PopupMenuItem::new(format!(
                                        "{} · {}",
                                        backend.label(),
                                        rust_i18n::t!(if available {
                                            "app.onboarding.installed"
                                        } else {
                                            "app.onboarding.not_installed"
                                        })
                                    ))
                                    .checked(current == Some(backend))
                                    .disabled(
                                        busy || (!available
                                            && installed.iter().any(|(_, found)| *found)),
                                    )
                                    .on_click(
                                        move |_, _, cx| {
                                            entity.update(cx, |this, cx| {
                                                if this.busy {
                                                    return;
                                                }
                                                if this.installed.contains(&backend) {
                                                    this.choose_default(backend, cx);
                                                } else {
                                                    this.selected_agent = Some(backend);
                                                }
                                                cx.notify();
                                            })
                                        },
                                    ),
                                )
                            })
                    }),
            )
            .children(
                self.selected_agent
                    .filter(|_| self.installed.is_empty())
                    .map(|backend| {
                        let installing = self.installing == Some(backend);
                        Button::new("onboarding-install")
                            .outline()
                            .label(if installing {
                                rust_i18n::t!("app.settings.coding.installing").to_string()
                            } else {
                                format!(
                                    "{} {}",
                                    rust_i18n::t!("app.settings.coding.install"),
                                    backend.label()
                                )
                            })
                            .disabled(self.busy || self.installing.is_some())
                            .on_click(cx.listener(move |this, _, _, cx| this.install(backend, cx)))
                    }),
            )
    }
}
impl Render for OnboardingView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let r = AutocraftTheme::global(cx).roles;
        let title = [
            "app.onboarding.welcome",
            "app.onboarding.agents",
            "app.onboarding.models",
            "app.onboarding.analysis",
            "app.onboarding.ready",
        ][self.step];
        let hint = [
            "app.onboarding.welcome_hint",
            if self.installed.is_empty() {
                "app.onboarding.agents_hint"
            } else {
                "app.onboarding.agents_ready"
            },
            if self.model.is_none() {
                "app.onboarding.models_hint"
            } else {
                "app.onboarding.models_ready"
            },
            "app.onboarding.analysis_hint",
            "app.onboarding.ready_hint",
        ][self.step];
        let mut body = v_flex()
            .size_full()
            .p(space::S32)
            .gap(space::S24)
            .bg(r.container_content_bg)
            .text_color(r.text_default)
            .child(label(
                text::METADATA,
                rust_i18n::t!("app.onboarding.step", current = self.step + 1).to_string(),
            ))
            .child(label(text::HEADING_XL, rust_i18n::t!(title).to_string()))
            .child(label(text::READING, rust_i18n::t!(hint).to_string()).text_color(r.text_muted));
        if self.step == 1 {
            body = body.child(self.agent_menu(cx));
            if let Some(backend) = self.installing {
                body = body.child(
                    label(
                        text::METADATA,
                        rust_i18n::t!("app.onboarding.installing", agent = backend.label())
                            .to_string(),
                    )
                    .text_color(r.text_muted),
                );
            }
            if self
                .selected_agent
                .is_some_and(|_| self.installed.is_empty())
                && self.installing.is_none()
            {
                body = body.child(
                    label(
                        text::METADATA,
                        rust_i18n::t!("app.onboarding.install_selected_hint").to_string(),
                    )
                    .text_color(r.text_muted),
                );
            }
        }
        if self.step == 2 {
            if let Some(model) = &self.model {
                body = body.child(label(text::UI, model.clone()));
            } else {
                body = body.child(
                    h_flex()
                        .flex_wrap()
                        .gap(space::S12)
                        .child(
                            Button::new("onboarding-providers")
                                .outline()
                                .label(rust_i18n::t!("app.onboarding.add_provider").to_string())
                                .disabled(self.busy)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    SettingsDialog::open_providers(this.state.clone(), window, cx)
                                })),
                        )
                        .child(
                            Button::new("onboarding-models")
                                .outline()
                                .label(rust_i18n::t!("app.onboarding.select_model").to_string())
                                .disabled(self.busy)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    SettingsDialog::open(this.state.clone(), window, cx)
                                })),
                        ),
                );
            }
        }
        if self.step == 3 {
            body = body.child(
                Button::new("onboarding-analysis-settings")
                    .outline()
                    .label(rust_i18n::t!("app.onboarding.configure_analysis").to_string())
                    .disabled(self.busy)
                    .on_click(cx.listener(|this, _, window, cx| {
                        SettingsDialog::open_analysis(this.state.clone(), window, cx)
                    })),
            );
        }
        body.child(
            h_flex()
                .gap(space::S12)
                .when(self.step > 0, |row| {
                    row.child(
                        Button::new("onboarding-back")
                            .outline()
                            .label(rust_i18n::t!("app.onboarding.back").to_string())
                            .disabled(self.busy)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.step = this.step.saturating_sub(1);
                                cx.notify();
                            })),
                    )
                })
                .child(
                    Button::new("onboarding-skip")
                        .outline()
                        .label(rust_i18n::t!("app.onboarding.skip").to_string())
                        .disabled(self.busy)
                        .on_click(cx.listener(|this, _, _, cx| this.advance(cx))),
                )
                .child(
                    Button::new("onboarding-next")
                        .primary()
                        .label(
                            rust_i18n::t!(if self.step == 4 {
                                "app.onboarding.finish"
                            } else {
                                "app.onboarding.next"
                            })
                            .to_string(),
                        )
                        .disabled(self.busy)
                        .on_click(cx.listener(|this, _, _, cx| this.advance(cx))),
                ),
        )
    }
}
