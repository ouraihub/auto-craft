//! Revisioned project model choices using the shared LumaKit picker.
use super::TopBar;
use crate::state::run;
use autocraft_core::{BackendKind, ModelChoice, ModelRef, ProjectPrefChange, ThinkingLevel};
use gpui_kit::{
    Context, IntoElement,
    component::{
        Disableable as _,
        button::{Button, ButtonVariants as _},
        menu::{DropdownMenu as _, PopupMenuItem},
    },
};
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
impl TopBar {
    pub(super) fn load_models(&mut self, cx: &mut Context<Self>) {
        self.generation += 1;
        self.changing = false;
        self.prefs = None;
        self.picker.update(cx, |picker, cx| {
            picker.set_data(
                PickerData {
                    models: Vec::new(),
                    selected: None,
                    default: None,
                    enabled: false,
                    refreshing: false,
                    notice: String::new(),
                },
                cx,
            )
        });
        let state = self.state.read(cx);
        let Some(project) = state.project.clone() else {
            self.backend = None;
            return;
        };
        let services = state.services.clone();
        let generation = self.generation;
        run(
            cx,
            move || {
                let prefs = services.project_prefs.get(&project)?;
                let defaults = services.prefs.prefs()?;
                let backend = prefs.backend.or(defaults.default_agent);
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
                Ok((prefs, defaults, backend, catalog, keyed))
            },
            move |this, result, cx| {
                if this.generation != generation {
                    return;
                }
                match result {
                    Ok((prefs, defaults, backend, catalog, keyed)) => {
                        this.catalog = catalog;
                        this.backend = backend;
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
                                                .into_owned(),
                                            hint: String::new(),
                                        })
                                        .collect(),
                                })
                                .collect(),
                            selected: selection(prefs.model.as_ref(), prefs.effort),
                            default: selection(
                                defaults.default_model.as_ref(),
                                defaults.default_effort,
                            ),
                            enabled: !this
                                .state
                                .read(cx)
                                .detail
                                .as_ref()
                                .is_some_and(|d| d.summary.read_only),
                            refreshing: false,
                            notice: String::new(),
                        };
                        this.prefs = Some(prefs);
                        this.picker
                            .update(cx, |picker, cx| picker.set_data(data, cx));
                    }
                    Err(error) => this.state.update(cx, |state, cx| state.report(&error, cx)),
                }
                cx.notify();
            },
        );
    }
    pub(super) fn pick_model(&mut self, event: &PickerEvent, cx: &mut Context<Self>) {
        match event {
            PickerEvent::Refresh { .. } => self.load_models(cx),
            PickerEvent::Select(selection) | PickerEvent::SetDefault(selection) => {
                let Some(choice) = self.catalog.iter().find(|c| key(c) == selection.model) else {
                    return;
                };
                let model = reference(choice);
                let effort = selection
                    .effort
                    .as_ref()
                    .and_then(|id| LEVELS.iter().find(|(_, v)| *v == id).map(|(l, _)| *l))
                    .filter(|l| choice.model.efforts.contains(l));
                self.save_choices(
                    vec![
                        ProjectPrefChange::Model(Some(model)),
                        ProjectPrefChange::Effort(effort),
                    ],
                    cx,
                );
            }
        }
    }
    fn save_choices(&mut self, changes: Vec<ProjectPrefChange>, cx: &mut Context<Self>) {
        if self.changing
            || self
                .state
                .read(cx)
                .detail
                .as_ref()
                .is_some_and(|d| d.summary.read_only)
        {
            return;
        }
        let state = self.state.read(cx);
        let Some(project) = state.project.clone() else {
            return;
        };
        let Some(prefs) = &self.prefs else {
            return;
        };
        let mut revision = prefs.revision;
        let service = state.services.project_prefs.clone();
        let generation = self.generation;
        self.changing = true;
        run(
            cx,
            move || {
                for change in changes {
                    revision = service.set(&project, change, revision)?.revision;
                }
                Ok(())
            },
            move |this, result, cx| {
                if this.generation != generation {
                    return;
                }
                this.changing = false;
                if let Err(error) = result {
                    this.state.update(cx, |state, cx| state.report(&error, cx));
                }
                this.load_models(cx);
                cx.notify();
            },
        );
    }
    pub(super) fn backend_menu(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let entity = cx.entity();
        let backend = self.backend;
        let enabled = self.prefs.is_some()
            && !self.changing
            && !self
                .state
                .read(cx)
                .detail
                .as_ref()
                .is_some_and(|d| d.summary.read_only);
        Button::new("shell-agent")
            .ghost()
            .disabled(!enabled)
            .label(backend.map_or("Agent", BackendKind::label))
            .dropdown_menu(move |menu, _, _| {
                std::iter::once(None)
                    .chain(BackendKind::ALL.into_iter().map(Some))
                    .fold(menu, |menu, value| {
                        let entity = entity.clone();
                        menu.item(
                            PopupMenuItem::new(value.map_or_else(
                                || rust_i18n::t!("app.shell.follow_default").into_owned(),
                                |b| b.label().into(),
                            ))
                            .checked(backend == value)
                            .on_click(move |_, _, cx| {
                                entity.update(cx, |this, cx| {
                                    this.save_choices(
                                        vec![
                                            ProjectPrefChange::Backend(value),
                                            ProjectPrefChange::Model(None),
                                            ProjectPrefChange::Effort(None),
                                        ],
                                        cx,
                                    )
                                })
                            }),
                        )
                    })
            })
    }
}
