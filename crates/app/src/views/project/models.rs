//! LumaKit builder/model/effort picker backed by revisioned project preferences.
use super::ProjectView;
use crate::state::run;
use autocraft_core::{ModelChoice, ModelRef, ProjectPrefChange, ThinkingLevel};
use gpui_kit::Context;
use lumakit_model_picker::{Effort, Model, PickerData, PickerEvent, Selection};
const LEVELS: [(ThinkingLevel, &str); 7] = [
    (ThinkingLevel::Off, "off"),
    (ThinkingLevel::Minimal, "minimal"),
    (ThinkingLevel::Low, "low"),
    (ThinkingLevel::Medium, "medium"),
    (ThinkingLevel::High, "high"),
    (ThinkingLevel::Xhigh, "xhigh"),
    (ThinkingLevel::Max, "max"),
];
fn key(choice: &ModelChoice) -> String {
    format!(
        "{:?}",
        (
            choice.provider.as_str(),
            choice.route.as_str(),
            choice.model.id.as_str()
        )
    )
}
fn reference(choice: &ModelChoice) -> ModelRef {
    ModelRef {
        provider: choice.provider.clone(),
        model_id: choice.model.id.clone(),
    }
}
impl ProjectView {
    pub(super) fn load_models(&mut self, cx: &mut Context<Self>) {
        let Some(prefs) = &self.prefs else {
            return;
        };
        let backend = prefs.backend;
        let services = self.state.read(cx).services.clone();
        let generation = self.generation;
        let revision = prefs.revision;
        run(
            cx,
            move || {
                let defaults = services.prefs.prefs()?;
                let backend = backend.or(defaults.default_agent);
                let catalog = match backend {
                    Some(backend) => services.router.catalog(backend)?,
                    None => Vec::new(),
                };
                let keyed = services
                    .providers
                    .providers()?
                    .into_iter()
                    .filter(|p| p.keys.iter().any(|k| k.enabled))
                    .map(|p| p.id.to_string())
                    .collect::<Vec<_>>();
                Ok((defaults, catalog, keyed))
            },
            move |this, result, cx| {
                if this.generation != generation
                    || this.prefs.as_ref().map(|p| p.revision) != Some(revision)
                {
                    return;
                }
                match result {
                    Ok((defaults, catalog, keyed)) => {
                        this.catalog = catalog;
                        this.model_prefs = defaults;
                        let Some(prefs) = &this.prefs else {
                            return;
                        };
                        let selection =
                            |saved: Option<&ModelRef>, effort: Option<ThinkingLevel>| {
                                saved
                                    .and_then(|m| this.catalog.iter().find(|c| reference(c) == *m))
                                    .map(|c| Selection {
                                        model: key(c),
                                        effort: effort.and_then(|level| {
                                            LEVELS
                                                .iter()
                                                .find(|(l, _)| *l == level)
                                                .map(|(_, id)| (*id).into())
                                        }),
                                    })
                            };
                        let data = PickerData {
                            models: this
                                .catalog
                                .iter()
                                .map(|c| Model {
                                    id: key(c),
                                    provider_id: c.provider.to_string(),
                                    provider: c.provider_name.clone(),
                                    name: c.model.name.clone(),
                                    detail: c.route.clone(),
                                    has_key: keyed.contains(&c.provider.to_string()),
                                    efforts: LEVELS
                                        .iter()
                                        .filter(|(level, _)| c.model.efforts.contains(level))
                                        .map(|(_, id)| Effort {
                                            id: (*id).into(),
                                            label: rust_i18n::t!(format!("app.home.effort_{id}"))
                                                .to_string(),
                                            hint: String::new(),
                                        })
                                        .collect(),
                                })
                                .collect(),
                            selected: selection(prefs.model.as_ref(), prefs.effort),
                            default: selection(
                                this.model_prefs.default_model.as_ref(),
                                this.model_prefs.default_effort,
                            ),
                            enabled: this.writable() && !this.busy,
                            refreshing: false,
                            notice: if this.running() {
                                rust_i18n::t!("app.project.next_run").to_string()
                            } else {
                                String::new()
                            },
                        };
                        this.picker
                            .update(cx, |picker, cx| picker.set_data(data, cx));
                    }
                    Err(error) => this.report(&error, cx),
                }
                cx.notify();
            },
        );
    }
    pub(super) fn pick_model(&mut self, event: &PickerEvent, cx: &mut Context<Self>) {
        match event {
            PickerEvent::Refresh { .. } => self.load_models(cx),
            PickerEvent::Select(selection) | PickerEvent::SetDefault(selection) => {
                if !self.writable() || self.busy {
                    return;
                }
                let Some(choice) = self
                    .catalog
                    .iter()
                    .find(|choice| key(choice) == selection.model)
                else {
                    return;
                };
                let model = Some(reference(choice));
                let effort = selection
                    .effort
                    .as_ref()
                    .and_then(|id| {
                        LEVELS
                            .iter()
                            .find(|(_, value)| *value == id)
                            .map(|(level, _)| *level)
                    })
                    .filter(|level| choice.model.efforts.contains(level));
                if matches!(event, PickerEvent::SetDefault(_)) {
                    let service = self.state.read(cx).services.prefs.clone();
                    let generation = self.generation;
                    run(
                        cx,
                        move || service.set_default(model, effort),
                        move |this, result, cx| {
                            if this.generation != generation {
                                return;
                            }
                            match result {
                                Ok(defaults) => {
                                    this.model_prefs = defaults;
                                    this.state.update(cx, |_, cx| {
                                        cx.emit(crate::state::AppEvent::Routing)
                                    });
                                    this.load_models(cx);
                                }
                                Err(error) => this.report(&error, cx),
                            }
                        },
                    );
                    return;
                }
                let Some(project) = self.project.clone() else {
                    return;
                };
                let Some(prefs) = &self.prefs else {
                    return;
                };
                let revision = prefs.revision;
                let service = self.state.read(cx).services.project_prefs.clone();
                let generation = self.generation;
                self.busy = true;
                run(
                    cx,
                    move || {
                        let prefs =
                            service.set(&project, ProjectPrefChange::Model(model), revision)?;
                        service.set(&project, ProjectPrefChange::Effort(effort), prefs.revision)
                    },
                    move |this, result, cx| {
                        if this.generation != generation {
                            return;
                        }
                        this.busy = false;
                        match result {
                            Ok(prefs) => {
                                this.prefs = Some(prefs);
                                this.load_models(cx);
                            }
                            Err(error) => {
                                this.report(&error, cx);
                                this.reload(cx);
                            }
                        }
                        cx.notify();
                    },
                );
            }
        }
    }
    pub(super) fn set_pref(&mut self, change: ProjectPrefChange, cx: &mut Context<Self>) {
        if self.busy || !self.writable() {
            return;
        }
        let Some(project) = self.project.clone() else {
            return;
        };
        let Some(prefs) = &self.prefs else {
            return;
        };
        let revision = prefs.revision;
        let service = self.state.read(cx).services.project_prefs.clone();
        let generation = self.generation;
        self.busy = true;
        run(
            cx,
            move || service.set(&project, change, revision),
            move |this, result, cx| {
                if this.generation != generation {
                    return;
                }
                this.busy = false;
                match result {
                    Ok(prefs) => {
                        this.prefs = Some(prefs);
                        this.load_models(cx);
                    }
                    Err(error) => this.report(&error, cx),
                }
                cx.notify();
            },
        );
    }
}
