//! Asset defaults and revision-checked fal.ai credentials; plaintext is never cached for display.
use super::rows::{group, notice, row};
use crate::{
    i18n::text as tr,
    state::{AppState, run},
};
use autocraft_core::{AnalysisAssetSettings, AnalysisSettingsChange, AssetKeySummary, Secret};
use autocraft_ui::{AutocraftTheme, metrics::space};
use gpui_kit::{
    AppContext as _, ClipboardItem, Context, Entity, IntoElement, ParentElement as _, Render,
    Styled as _, Subscription, Window,
    base::{h_flex, v_flex},
    component::{
        Disableable as _, Sizable as _,
        button::{Button, ButtonVariants as _},
        input::{Input, InputEvent, InputState},
    },
    px,
};

pub(super) struct AssetsPage {
    state: Entity<AppState>,
    budget: Entity<InputState>,
    model: Entity<InputState>,
    key: Entity<InputState>,
    summary: Option<AssetKeySummary>,
    loaded: bool,
    saving: bool,
    key_busy: bool,
    generation: u64,
    key_generation: u64,
    error: Option<&'static str>,
    _subscriptions: Vec<Subscription>,
}
impl AssetsPage {
    pub(super) fn new(
        state: Entity<AppState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let budget = cx.new(|cx| InputState::new(window, cx));
        let model = cx.new(|cx| InputState::new(window, cx));
        let key = cx.new(|cx| InputState::new(window, cx).masked(true));
        let mut subscriptions = Vec::new();
        for input in [&budget, &model] {
            subscriptions.push(cx.subscribe(
                input,
                |this: &mut Self, _, event: &InputEvent, cx| {
                    if matches!(event, InputEvent::Change) {
                        this.generation = this.generation.wrapping_add(1);
                        this.error = None;
                        cx.notify();
                    }
                },
            ));
        }
        subscriptions.push(
            cx.subscribe(&key, |this: &mut Self, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.key_generation = this.key_generation.wrapping_add(1);
                    this.error = None;
                    cx.notify();
                }
            }),
        );
        let mut page = Self {
            state,
            budget,
            model,
            key,
            summary: None,
            loaded: false,
            saving: false,
            key_busy: false,
            generation: 0,
            key_generation: 0,
            error: None,
            _subscriptions: subscriptions,
        };
        page.load(window, cx);
        page
    }
    fn load(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let analysis = self.state.read(cx).services.analysis.clone();
        let keys = self.state.read(cx).services.asset_keys.clone();
        let handle = window.window_handle();
        let generation = self.generation;
        run(
            cx,
            move || Ok((analysis.settings()?.assets, keys.asset_key_summaries()?)),
            move |this, result, cx| {
                match result {
                    Ok((assets, summaries)) => {
                        this.summary = summaries.into_iter().find(|s| s.provider == "fal");
                        this.loaded = true;
                        if this.generation == generation {
                            if handle
                                .update(cx, |_, window, cx| {
                                    this.budget.update(cx, |input, cx| {
                                        input.set_value(assets.budget_usd.to_string(), window, cx)
                                    });
                                    this.model.update(cx, |input, cx| {
                                        input.set_value(
                                            assets.image_model.unwrap_or_default(),
                                            window,
                                            cx,
                                        )
                                    });
                                })
                                .is_err()
                            {
                                tracing::debug!("settings.assets.window_closed");
                                return;
                            }
                        } else {
                            tracing::debug!("settings.assets.draft_preserved");
                        }
                    }
                    Err(error) => this.state.update(cx, |state, cx| state.report(&error, cx)),
                }
                cx.notify();
            },
        );
    }
    fn save_defaults(&mut self, cx: &mut Context<Self>) {
        if self.saving || !self.loaded {
            return;
        }
        let budget = self.budget.read(cx).value();
        let Some(budget_usd) = parse_budget(&budget) else {
            self.error = Some("app.settings.assets.invalid_budget");
            cx.notify();
            return;
        };
        let model = self.model.read(cx).value().trim().to_owned();
        let assets = AnalysisAssetSettings {
            budget_usd,
            image_model: (!model.is_empty()).then_some(model),
        };
        let service = self.state.read(cx).services.analysis.clone();
        self.saving = true;
        run(
            cx,
            move || {
                service.set_settings(AnalysisSettingsChange {
                    assets: Some(assets),
                    ..Default::default()
                })
            },
            |this, result, cx| {
                this.saving = false;
                if let Err(error) = result {
                    this.state.update(cx, |state, cx| state.report(&error, cx));
                }
                cx.notify();
            },
        );
        cx.notify();
    }
    fn save_key(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.key_busy || !self.loaded {
            return;
        }
        let value = self.key.read(cx).value().trim().to_owned();
        if value.is_empty() {
            self.error = Some("app.settings.assets.key_empty");
            cx.notify();
            return;
        }
        let revision = self.summary.as_ref().map_or(0, |s| s.revision);
        let generation = self.key_generation;
        let handle = window.window_handle();
        let service = self.state.read(cx).services.asset_keys.clone();
        self.key_busy = true;
        run(
            cx,
            move || service.set_asset_key("fal", Secret::new(value), revision),
            move |this, result, cx| {
                this.key_busy = false;
                match result {
                    Ok(summary) => {
                        this.summary = Some(summary);
                        if this.key_generation == generation {
                            if handle
                                .update(cx, |_, window, cx| {
                                    this.key
                                        .update(cx, |input, cx| input.set_value("", window, cx))
                                })
                                .is_err()
                            {
                                tracing::debug!("settings.assets.window_closed");
                                return;
                            }
                        } else {
                            tracing::debug!("settings.assets.key_draft_preserved");
                        }
                    }
                    Err(error) => this.state.update(cx, |state, cx| state.report(&error, cx)),
                }
                cx.notify();
            },
        );
        cx.notify();
    }
    fn remove_key(&mut self, cx: &mut Context<Self>) {
        if self.key_busy {
            return;
        }
        let Some(summary) = &self.summary else {
            return;
        };
        let revision = summary.revision;
        let service = self.state.read(cx).services.asset_keys.clone();
        self.key_busy = true;
        run(
            cx,
            move || service.remove_asset_key("fal", revision),
            |this, result, cx| {
                this.key_busy = false;
                match result {
                    Ok(()) => this.summary = None,
                    Err(error) => this.state.update(cx, |state, cx| state.report(&error, cx)),
                }
                cx.notify();
            },
        );
        cx.notify();
    }
    fn copy_key(&mut self, cx: &mut Context<Self>) {
        if self.key_busy || self.summary.is_none() {
            return;
        }
        let service = self.state.read(cx).services.asset_keys.clone();
        run(
            cx,
            move || service.asset_key("fal"),
            |this, result, cx| match result {
                Ok(secret) => {
                    cx.write_to_clipboard(ClipboardItem::new_string(secret.expose().to_owned()))
                }
                Err(error) => this.state.update(cx, |state, cx| state.report(&error, cx)),
            },
        );
    }
}
fn parse_budget(value: &str) -> Option<f64> {
    value
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|v| v.is_finite() && *v >= 0.0)
}

impl Render for AssetsPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let r = AutocraftTheme::global(cx).roles;
        let mut body = group(
            "analysis-assets",
            tr("app.settings.assets.title"),
            Some(tr("app.settings.assets.description")),
            &r,
        );
        if !self.loaded {
            return body
                .child(
                    Button::new("assets-load")
                        .outline()
                        .label(tr("app.settings.analysis.reload"))
                        .on_click(cx.listener(|this, _, window, cx| this.load(window, cx))),
                )
                .into_any_element();
        }
        body = body
            .child(row(
                tr("app.settings.assets.budget"),
                None,
                Input::new(&self.budget)
                    .small()
                    .w(px(220.))
                    .disabled(self.saving)
                    .aria_label(tr("app.settings.assets.budget")),
                &r,
            ))
            .child(row(
                tr("app.settings.assets.image_model"),
                None,
                Input::new(&self.model)
                    .small()
                    .w(px(280.))
                    .disabled(self.saving)
                    .aria_label(tr("app.settings.assets.image_model")),
                &r,
            ))
            .child(
                Button::new("asset-defaults-save")
                    .outline()
                    .disabled(self.saving)
                    .label(tr("app.settings.assets.save_defaults"))
                    .on_click(cx.listener(|this, _, _, cx| this.save_defaults(cx))),
            );
        let mask = self
            .summary
            .as_ref()
            .map(|s| s.masked_key.clone())
            .unwrap_or_else(|| tr("app.settings.assets.not_configured").into_owned());
        body = body.child(row(
            tr("app.settings.assets.fal_key"),
            Some(mask),
            v_flex()
                .gap(space::S8)
                .child(
                    Input::new(&self.key)
                        .small()
                        .w(px(280.))
                        .disabled(self.key_busy)
                        .aria_label(tr("app.settings.assets.fal_key")),
                )
                .child(
                    h_flex()
                        .gap(space::S8)
                        .child(
                            Button::new("asset-key-save")
                                .primary()
                                .disabled(self.key_busy)
                                .label(tr("app.settings.assets.save_key"))
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.save_key(window, cx)),
                                ),
                        )
                        .child(
                            Button::new("asset-key-copy")
                                .outline()
                                .disabled(self.key_busy || self.summary.is_none())
                                .label(tr("app.settings.assets.copy_key"))
                                .on_click(cx.listener(|this, _, _, cx| this.copy_key(cx))),
                        )
                        .child(
                            Button::new("asset-key-remove")
                                .outline()
                                .disabled(self.key_busy || self.summary.is_none())
                                .label(tr("app.settings.assets.remove_key"))
                                .on_click(cx.listener(|this, _, _, cx| this.remove_key(cx))),
                        ),
                ),
            &r,
        ));
        body = body.child(
            Button::new("assets-reload")
                .outline()
                .disabled(self.saving || self.key_busy)
                .label(tr("app.settings.analysis.reload"))
                .on_click(cx.listener(|this, _, window, cx| this.load(window, cx))),
        );
        if let Some(error) = self.error {
            body = body.child(notice(tr(error).into_owned(), true, &r));
        }
        body.into_any_element()
    }
}
#[cfg(test)]
mod tests {
    use super::parse_budget;
    #[test]
    fn asset_budget_rejects_nonfinite_and_negative_spend() {
        assert_eq!(parse_budget(" 0 "), Some(0.0));
        assert_eq!(parse_budget("12.50"), Some(12.5));
        for value in ["", "-1", "NaN", "inf", "five"] {
            assert_eq!(parse_budget(value), None);
        }
    }
}
