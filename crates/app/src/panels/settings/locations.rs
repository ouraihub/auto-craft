//! Project discovery folders: native folder pickers and editable, persisted directory rows.
use super::rows::{group, notice, row};
use crate::{
    i18n::text as tr,
    state::{AppState, run},
};
use autocraft_core::{Error, ProjectLocations};
use autocraft_ui::{AutocraftTheme, metrics::space};
use gpui_kit::{
    AppContext as _, Context, Entity, IntoElement, ParentElement as _, PathPromptOptions, Render,
    Styled as _, Subscription, Window,
    base::{h_flex, v_flex},
    component::{
        Disableable as _, Sizable as _,
        button::{Button, ButtonVariants as _},
        input::{Input, InputEvent, InputState},
    },
    px,
};
use std::path::PathBuf;

pub(super) struct LocationsPage {
    state: Entity<AppState>,
    default: Entity<InputState>,
    extra: Vec<Entity<InputState>>,
    loaded: bool,
    saving: bool,
    generation: u64,
    picker_generation: u64,
    error: Option<&'static str>,
    _subscriptions: Vec<Subscription>,
}
impl LocationsPage {
    pub(super) fn new(
        state: Entity<AppState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let default = cx.new(|cx| InputState::new(window, cx));
        let subscription = cx.subscribe(&default, |this: &mut Self, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.edited(cx);
            }
        });
        let mut page = Self {
            state,
            default,
            extra: Vec::new(),
            loaded: false,
            saving: false,
            generation: 0,
            picker_generation: 0,
            error: None,
            _subscriptions: vec![subscription],
        };
        page.load(window, cx);
        page
    }
    fn edited(&mut self, cx: &mut Context<Self>) {
        self.generation = self.generation.wrapping_add(1);
        self.error = None;
        cx.notify();
    }
    fn append(&mut self, value: String, window: &mut Window, cx: &mut Context<Self>) {
        let input = cx.new(|cx| InputState::new(window, cx).default_value(value));
        self._subscriptions.push(cx.subscribe(
            &input,
            |this: &mut Self, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.edited(cx);
                }
            },
        ));
        self.extra.push(input);
        self.edited(cx);
    }
    fn show(&mut self, locations: ProjectLocations, window: &mut Window, cx: &mut gpui_kit::App) {
        self.default.update(cx, |input, cx| {
            input.set_value(locations.default_dir.display().to_string(), window, cx)
        });
        self.extra = locations
            .extra
            .into_iter()
            .map(|path| {
                cx.new(|cx| InputState::new(window, cx).default_value(path.display().to_string()))
            })
            .collect();
        self.loaded = true;
    }
    fn watch_extras(&mut self, cx: &mut Context<Self>) {
        self._subscriptions.truncate(1);
        for input in &self.extra {
            self._subscriptions.push(cx.subscribe(
                input,
                |this: &mut Self, _, event: &InputEvent, cx| {
                    if matches!(event, InputEvent::Change) {
                        this.edited(cx);
                    }
                },
            ));
        }
    }
    fn load(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let service = self.state.read(cx).services.projects.clone();
        let generation = self.generation;
        let handle = window.window_handle();
        run(
            cx,
            move || service.locations(),
            move |this, result, cx| {
                match result {
                    Ok(locations) => {
                        if this.generation != generation {
                            tracing::debug!("settings.locations.stale_load");
                            return;
                        }
                        if handle
                            .update(cx, |_, window, cx| this.show(locations, window, cx))
                            .is_err()
                        {
                            tracing::debug!("settings.locations.window_closed");
                            return;
                        }
                        this.watch_extras(cx);
                    }
                    Err(error) => this.state.update(cx, |state, cx| state.report(&error, cx)),
                }
                cx.notify();
            },
        );
    }
    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.saving || !self.loaded {
            return;
        }
        let default = self.default.read(cx).value().trim().to_owned();
        let extra: Vec<String> = self
            .extra
            .iter()
            .map(|i| i.read(cx).value().trim().to_owned())
            .collect();
        let locations = match locations(&default, &extra) {
            Ok(locations) => locations,
            Err(key) => {
                self.error = Some(key);
                cx.notify();
                return;
            }
        };
        let service = self.state.read(cx).services.projects.clone();
        let generation = self.generation;
        let handle = window.window_handle();
        self.saving = true;
        run(
            cx,
            move || {
                service.set_locations(locations)?;
                service.locations()
            },
            move |this, result, cx| {
                this.saving = false;
                match result {
                    Ok(locations) => {
                        if this.generation == generation {
                            if handle
                                .update(cx, |_, window, cx| this.show(locations, window, cx))
                                .is_err()
                            {
                                tracing::debug!("settings.locations.window_closed");
                                return;
                            }
                            this.watch_extras(cx);
                        } else {
                            tracing::debug!("settings.locations.draft_preserved");
                        }
                        this.state.update(cx, |state, cx| state.reload(cx));
                    }
                    Err(error) => this.state.update(cx, |state, cx| state.report(&error, cx)),
                }
                cx.notify();
            },
        );
        cx.notify();
    }
    fn pick(&mut self, input: Entity<InputState>, window: &mut Window, cx: &mut Context<Self>) {
        if self.saving {
            return;
        }
        self.picker_generation = self.picker_generation.wrapping_add(1);
        let picker_generation = self.picker_generation;
        let generation = self.generation;
        let handle = window.window_handle();
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: None,
        });
        cx.spawn(async move |this, cx| {
            let result = picked.await;
            if this
                .update(cx, |this, cx| {
                    if this.picker_generation != picker_generation || this.generation != generation
                    {
                        tracing::debug!("settings.locations.stale_picker");
                        return;
                    }
                    match result {
                        Ok(Ok(Some(paths))) => {
                            if let Some(path) = paths.into_iter().next()
                                && handle
                                    .update(cx, |_, window, cx| {
                                        input.update(cx, |input, cx| {
                                            input.set_value(path.display().to_string(), window, cx)
                                        })
                                    })
                                    .is_err()
                            {
                                tracing::debug!("settings.locations.window_closed");
                            }
                        }
                        Ok(Ok(None)) => {}
                        Ok(Err(_)) | Err(_) => run(
                            cx,
                            || {
                                Err::<(), _>(Error::external(tr(
                                    "app.settings.locations.picker_failed",
                                )))
                            },
                            |this, result, cx| {
                                if let Err(error) = result {
                                    this.state.update(cx, |state, cx| state.report(&error, cx));
                                }
                            },
                        ),
                    }
                })
                .is_err()
            {
                tracing::debug!("settings.locations.view_closed");
            }
        })
        .detach();
    }
}
fn locations(default: &str, extra: &[String]) -> Result<ProjectLocations, &'static str> {
    let default_dir = PathBuf::from(default);
    if !default_dir.is_absolute() || extra.iter().any(|p| !PathBuf::from(p).is_absolute()) {
        return Err("app.settings.locations.absolute_required");
    }
    let extra: Vec<PathBuf> = extra.iter().map(PathBuf::from).collect();
    if extra
        .iter()
        .enumerate()
        .any(|(index, path)| path == &default_dir || extra[..index].contains(path))
    {
        return Err("app.settings.locations.duplicate");
    }
    Ok(ProjectLocations { default_dir, extra })
}
impl Render for LocationsPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let r = AutocraftTheme::global(cx).roles;
        let mut root = v_flex().gap(space::S16);
        if !self.loaded {
            return root
                .child(
                    Button::new("locations-load")
                        .outline()
                        .label(tr("app.settings.analysis.reload"))
                        .on_click(cx.listener(|this, _, window, cx| this.load(window, cx))),
                )
                .into_any_element();
        }
        let default = self.default.clone();
        let mut folders = group(
            "project-locations",
            tr("app.settings.locations.title"),
            Some(tr("app.settings.locations.description")),
            &r,
        )
        .child(row(
            tr("app.settings.locations.default"),
            None,
            h_flex()
                .gap(space::S8)
                .child(
                    Input::new(&self.default)
                        .small()
                        .w(px(300.))
                        .disabled(self.saving)
                        .aria_label(tr("app.settings.locations.default")),
                )
                .child(
                    Button::new("locations-default-pick")
                        .outline()
                        .small()
                        .disabled(self.saving)
                        .label(tr("app.settings.locations.choose"))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.pick(default.clone(), window, cx)
                        })),
                ),
            &r,
        ));
        for (index, input) in self.extra.iter().enumerate() {
            let chosen = input.clone();
            folders = folders.child(row(
                tr("app.settings.locations.extra"),
                None,
                h_flex()
                    .gap(space::S8)
                    .child(
                        Input::new(input)
                            .small()
                            .w(px(250.))
                            .disabled(self.saving)
                            .aria_label(tr("app.settings.locations.extra")),
                    )
                    .child(
                        Button::new(format!("locations-pick-{index}"))
                            .outline()
                            .small()
                            .disabled(self.saving)
                            .label(tr("app.settings.locations.choose"))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.pick(chosen.clone(), window, cx)
                            })),
                    )
                    .child(
                        Button::new(format!("locations-remove-{index}"))
                            .outline()
                            .small()
                            .disabled(self.saving)
                            .label(tr("app.settings.locations.remove"))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.extra.remove(index);
                                drop(this._subscriptions.remove(index + 1));
                                this.edited(cx);
                            })),
                    ),
                &r,
            ));
        }
        folders = folders.child(
            Button::new("locations-add")
                .outline()
                .disabled(self.saving)
                .label(tr("app.settings.locations.add"))
                .on_click(
                    cx.listener(|this, _, window, cx| this.append(String::new(), window, cx)),
                ),
        );
        root = root.child(folders);
        if let Some(error) = self.error {
            root = root.child(notice(tr(error).into_owned(), true, &r));
        }
        root.child(
            Button::new("locations-save")
                .primary()
                .disabled(self.saving)
                .label(tr("app.settings.locations.save"))
                .on_click(cx.listener(|this, _, window, cx| this.save(window, cx))),
        )
        .into_any_element()
    }
}
#[cfg(test)]
mod tests {
    use super::locations;
    #[test]
    fn locations_require_absolute_distinct_directories() {
        let root = std::env::temp_dir();
        let main = root.join("autocraft-main").display().to_string();
        let extra = root.join("autocraft-extra").display().to_string();
        assert_eq!(
            locations(&main, std::slice::from_ref(&extra))
                .unwrap()
                .extra,
            [std::path::PathBuf::from(&extra)]
        );
        assert!(locations("relative", &[]).is_err());
        assert!(locations(&main, std::slice::from_ref(&main)).is_err());
        assert!(locations(&main, &[extra.clone(), extra]).is_err());
        assert!(locations(&main, &[String::new()]).is_err());
    }
}
