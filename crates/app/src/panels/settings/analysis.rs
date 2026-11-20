//! Global analysis preferences; source settings and live engine updates remain service-owned.
mod form;
mod game_engines;
mod view;

use super::assets::AssetsPage;
use crate::state::{AppState, run};
use autocraft_core::{AnalysisSettings, AnalysisSettingsChange, CancelFlag};
use ca_engines::game::{Builder, Inventory};
use gpui_kit::{
    AppContext as _, Context, Entity, IntoElement, Render, Subscription, Window,
    component::input::InputEvent,
};

pub(super) struct AnalysisPage {
    state: Entity<AppState>,
    settings: Option<AnalysisSettings>,
    fields: form::Fields,
    assets: Entity<AssetsPage>,
    saving: bool,
    installing: Option<CancelFlag>,
    generation: u64,
    error: Option<&'static str>,
    installed: bool,
    inventory: Option<Inventory>,
    focused_engine: Option<Builder>,
    refreshing: bool,
    _subscriptions: Vec<Subscription>,
}

impl AnalysisPage {
    pub(super) fn new(
        state: Entity<AppState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let fields = form::Fields::new(window, cx);
        let subscriptions = fields
            .0
            .values()
            .map(|input| {
                cx.subscribe(input, |this: &mut Self, _, event: &InputEvent, cx| {
                    if matches!(event, InputEvent::Change) {
                        this.generation = this.generation.wrapping_add(1);
                        this.error = None;
                        cx.notify();
                    }
                })
            })
            .collect();
        let assets = cx.new(|cx| AssetsPage::new(state.clone(), window, cx));
        let mut page = Self {
            state,
            settings: None,
            fields,
            assets,
            saving: false,
            installing: None,
            generation: 0,
            error: None,
            installed: false,
            inventory: None,
            focused_engine: None,
            refreshing: false,
            _subscriptions: subscriptions,
        };
        page.reload(window, cx);
        page
    }
    pub(super) fn focus_engine(&mut self, engine: &'static str, cx: &mut Context<Self>) {
        self.focused_engine = Builder::from_stack(engine);
        cx.notify();
    }
    fn reload(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let service = self.state.read(cx).services.analysis.clone();
        if self.refreshing {
            return;
        }
        self.refreshing = true;
        let handle = window.window_handle();
        let generation = self.generation;
        run(
            cx,
            move || {
                let settings = service.settings()?;
                let inventory = game_engines::discover(&settings)?;
                Ok((settings, inventory))
            },
            move |this, result, cx| {
                this.refreshing = false;
                match result {
                    Ok((settings, inventory)) => {
                        this.inventory = Some(inventory);
                        if this.generation == generation {
                            if handle
                                .update(cx, |_, window, cx| this.fields.fill(&settings, window, cx))
                                .is_err()
                            {
                                tracing::debug!("settings.analysis.window_closed");
                                return;
                            }
                        } else {
                            tracing::debug!("settings.analysis.draft_preserved");
                        }
                        this.settings = Some(settings);
                    }
                    Err(error) => this.state.update(cx, |state, cx| state.report(&error, cx)),
                }
                cx.notify();
            },
        );
    }
    fn change(&mut self, change: AnalysisSettingsChange, cx: &mut Context<Self>) {
        if self.saving || self.settings.is_none() || self.installing.is_some() {
            return;
        }
        self.saving = true;
        let service = self.state.read(cx).services.analysis.clone();
        run(
            cx,
            move || {
                let settings = service.set_settings(change)?;
                let inventory = game_engines::discover(&settings)?;
                Ok((settings, inventory))
            },
            |this, result, cx| {
                this.saving = false;
                match result {
                    Ok((settings, inventory)) => {
                        this.settings = Some(settings);
                        this.inventory = Some(inventory);
                    }
                    Err(error) => this.state.update(cx, |state, cx| state.report(&error, cx)),
                }
                cx.notify();
            },
        );
        cx.notify();
    }
    fn save_fields(&mut self, cx: &mut Context<Self>) {
        match self.fields.parse(cx) {
            Ok((engines, build)) => self.change(
                AnalysisSettingsChange {
                    engines: Some(engines),
                    build: Some(build),
                    ..Default::default()
                },
                cx,
            ),
            Err(key) => {
                self.error = Some(key);
                cx.notify();
            }
        }
    }
    fn install(&mut self, engine: &'static str, window: &mut Window, cx: &mut Context<Self>) {
        if self.installing.is_some() || self.saving || self.settings.is_none() {
            return;
        }
        let cancel = CancelFlag::new();
        self.installing = Some(cancel.clone());
        self.installed = false;
        let service = self.state.read(cx).services.analysis.clone();
        let handle = window.window_handle();
        let generation = self.generation;
        run(
            cx,
            move || {
                service.install_engines(serde_json::json!({"engine": engine}), &cancel)?;
                let settings = service.settings()?;
                let inventory = game_engines::discover(&settings)?;
                Ok((settings, inventory))
            },
            move |this, result, cx| {
                this.installing = None;
                match result {
                    Ok((settings, inventory)) => {
                        this.inventory = Some(inventory);
                        this.installed = true;
                        if this.generation == generation {
                            if handle
                                .update(cx, |_, window, cx| this.fields.fill(&settings, window, cx))
                                .is_err()
                            {
                                tracing::debug!("settings.analysis.window_closed");
                                return;
                            }
                        } else {
                            tracing::debug!("settings.analysis.install_draft_preserved");
                        }
                        this.settings = Some(settings);
                        this.state
                            .update(cx, |_, cx| cx.emit(crate::state::AppEvent::Routing));
                    }
                    Err(error) => this.state.update(cx, |state, cx| state.report(&error, cx)),
                }
                cx.notify();
            },
        );
        cx.notify();
    }
}
impl Drop for AnalysisPage {
    fn drop(&mut self) {
        if let Some(cancel) = &self.installing {
            cancel.cancel();
        }
    }
}
impl Render for AnalysisPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.content(window, cx)
    }
}
