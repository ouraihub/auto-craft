//! Installed backend discovery and the shared LumaKit model picker.
use super::HomeView;
use crate::state::{AppEvent, run};
use autocraft_core::{BackendKind, ModelChoice, ModelRef, ThinkingLevel};
use ca_engines::game::{self, Builder, Places};
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
fn key(c: &ModelChoice) -> String {
    format!(
        "{:?}",
        (c.provider.as_str(), c.route.as_str(), c.model.id.as_str())
    )
}
fn reference(c: &ModelChoice) -> ModelRef {
    ModelRef {
        provider: c.provider.clone(),
        model_id: c.model.id.clone(),
    }
}
fn effort_id(level: ThinkingLevel) -> String {
    match level {
        ThinkingLevel::Off => "off",
        ThinkingLevel::Minimal => "minimal",
        ThinkingLevel::Low => "low",
        ThinkingLevel::Medium => "medium",
        ThinkingLevel::High => "high",
        ThinkingLevel::Xhigh => "xhigh",
        ThinkingLevel::Max => "max",
    }
    .into()
}

impl HomeView {
    pub(super) fn load_choices(&mut self, cx: &mut Context<Self>) {
        self.generation += 1;
        let generation = self.generation;
        self.loading = true;
        self.sync_picker(cx);
        cx.notify();
        let services = self.state.read(cx).services.clone();
        let backend = self.backend;
        let pending_backend = self.pending_backend;
        run(
            cx,
            move || {
                let prefs = services.prefs.prefs()?;
                let installed = BackendKind::ALL
                    .into_iter()
                    .map(|b| services.engines.status(b).map(|found| (b, found.is_some())))
                    .collect::<autocraft_core::Result<Vec<_>>>()?
                    .into_iter()
                    .filter_map(|(b, found)| found.then_some(b))
                    .collect::<Vec<_>>();
                let backend =
                    refreshed_backend(backend, pending_backend, prefs.default_agent, &installed);
                let catalog = match backend {
                    Some(b) => services.router.catalog(b)?,
                    None => Vec::new(),
                };
                let settings = services.analysis.settings()?;
                let store = settings
                    .engines
                    .store_dir
                    .ok_or_else(|| autocraft_core::Error::corrupt("分析引擎存储位置未配置"))?;
                let inventory = game::inventory(&Places::here(store));
                let stacks = Builder::ALL
                    .into_iter()
                    .filter(|builder| inventory.ready(*builder))
                    .map(Builder::stack)
                    .collect();
                let providers = services.providers.providers()?;
                Ok((prefs, installed, backend, catalog, stacks, providers))
            },
            move |this, result, cx| {
                if generation != this.generation {
                    tracing::debug!("home.models.stale");
                    return;
                }
                this.loading = false;
                match result {
                    Ok((prefs, installed, backend, catalog, stacks, providers)) => {
                        this.prefs = prefs;
                        this.installed = installed;
                        this.backend = backend;
                        if this.pending_backend == backend {
                            this.pending_backend = None;
                        }
                        this.catalog = catalog;
                        this.game_stacks = stacks;
                        if this
                            .model
                            .as_ref()
                            .is_some_and(|m| !this.catalog.iter().any(|c| reference(c) == *m))
                        {
                            this.model = None;
                            this.effort = None;
                        }
                        this.keyed_providers = providers
                            .iter()
                            .filter(|p| p.keys.iter().any(|k| k.enabled))
                            .map(|p| p.id.to_string())
                            .collect();
                        this.sync_picker(cx);
                    }
                    Err(error) => this.report(&error, cx),
                }
                cx.notify();
            },
        );
    }

    pub(super) fn sync_picker(&mut self, cx: &mut Context<Self>) {
        let selection = |saved: Option<&ModelRef>, effort: Option<ThinkingLevel>| {
            saved
                .and_then(|m| self.catalog.iter().find(|c| reference(c) == *m))
                .map(|c| Selection {
                    model: key(c),
                    effort: effort.map(effort_id),
                })
        };
        let data = PickerData {
            models: self
                .catalog
                .iter()
                .map(|c| Model {
                    id: key(c),
                    provider_id: c.provider.to_string(),
                    provider: c.provider_name.clone(),
                    name: c.model.name.clone(),
                    detail: c.route.clone(),
                    has_key: self.keyed_providers.contains(&c.provider.to_string()),
                    efforts: LEVELS
                        .iter()
                        .filter(|(level, _)| c.model.efforts.contains(level))
                        .map(|(_, id)| Effort {
                            id: (*id).into(),
                            label: rust_i18n::t!(format!("app.home.effort_{id}")).to_string(),
                            hint: String::new(),
                        })
                        .collect(),
                })
                .collect(),
            selected: selection(self.model.as_ref(), self.effort),
            default: selection(self.prefs.default_model.as_ref(), self.prefs.default_effort),
            enabled: !self.busy && !self.loading && self.backend.is_some(),
            refreshing: self.loading,
            notice: String::new(),
        };
        self.picker
            .update(cx, |picker, cx| picker.set_data(data, cx));
    }

    pub(super) fn pick_model(&mut self, event: &PickerEvent, cx: &mut Context<Self>) {
        match event {
            PickerEvent::Refresh { force: _ } => self.load_choices(cx),
            PickerEvent::Select(selection) | PickerEvent::SetDefault(selection) => {
                if self.busy {
                    return;
                }
                let Some(choice) = self.catalog.iter().find(|c| key(c) == selection.model) else {
                    return;
                };
                let model = Some(reference(choice));
                let effort = selection
                    .effort
                    .as_ref()
                    .and_then(|id| {
                        LEVELS
                            .iter()
                            .find(|(_, value)| *value == id.as_str())
                            .map(|(level, _)| *level)
                    })
                    .filter(|level| choice.model.efforts.contains(level));
                if matches!(event, PickerEvent::SetDefault(_)) {
                    let prefs = self.state.read(cx).services.prefs.clone();
                    run(
                        cx,
                        move || prefs.set_default(model, effort),
                        |this, result, cx| match result {
                            Ok(prefs) => {
                                this.prefs = prefs;
                                this.state.update(cx, |_, cx| cx.emit(AppEvent::Routing));
                            }
                            Err(error) => this.report(&error, cx),
                        },
                    );
                } else {
                    self.model = model;
                    self.effort = effort;
                    self.sync_picker(cx);
                    cx.notify();
                }
            }
        }
    }
}

fn refreshed_backend(
    current: Option<BackendKind>,
    pending: Option<BackendKind>,
    default: Option<BackendKind>,
    installed: &[BackendKind],
) -> Option<BackendKind> {
    [pending, current, default]
        .into_iter()
        .flatten()
        .find(|backend| installed.contains(backend))
}

#[cfg(test)]
mod tests {
    use super::refreshed_backend;
    use autocraft_core::BackendKind::{Dsh, Omp, Pi};

    #[test]
    fn pending_install_preserves_current_until_available() {
        assert_eq!(
            refreshed_backend(Some(Pi), Some(Omp), Some(Dsh), &[Pi, Dsh]),
            Some(Pi)
        );
        assert_eq!(
            refreshed_backend(Some(Pi), Some(Omp), Some(Dsh), &[Pi, Omp, Dsh]),
            Some(Omp)
        );
        assert_eq!(
            refreshed_backend(Some(Pi), None, Some(Dsh), &[Pi, Dsh]),
            Some(Pi)
        );
        assert_eq!(
            refreshed_backend(None, Some(Omp), Some(Dsh), &[Dsh]),
            Some(Dsh)
        );
        assert_eq!(refreshed_backend(None, Some(Omp), Some(Dsh), &[]), None);
    }
}
