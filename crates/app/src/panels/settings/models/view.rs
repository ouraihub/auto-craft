//! Models page composition: the groups in display order, then the page-level save state,
//! error with retry and empty state.

use autocraft_ui::{
    AutocraftTheme,
    components::label,
    metrics::{space, text},
};
use gpui_kit::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _, Render, Styled as _, Window,
    base::v_flex,
    component::{
        Disableable as _,
        button::{Button, ButtonVariants as _},
    },
    prelude::FluentBuilder as _,
};

use super::{super::rows::group, ModelsPage};

impl Render for ModelsPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let r = AutocraftTheme::global(cx).roles;
        let default_group = self.default_group(&r, window, cx);
        let visibility_group = self.visibility_group(&r, cx);
        let context_group = self.context_group(&r, cx);
        let empty = self
            .snapshot
            .as_ref()
            .is_some_and(|snapshot| snapshot.catalog.is_empty());
        let failure = self.failure.as_ref().map(|failure| failure.message.clone());
        let busy = self.busy();

        v_flex()
            .id("settings-models")
            .gap(space::S24)
            // The first load renders no rows: an empty select would flash before the real one.
            .when(default_group.is_some(), |page| {
                page.child(
                    group(
                        "models-default",
                        crate::i18n::text("app.settings.models.default"),
                        None,
                        &r,
                    )
                    .children(default_group)
                    .children(visibility_group),
                )
            })
            .when_some(context_group, |page, context| {
                page.child(
                    group(
                        "models-context",
                        crate::i18n::text("app.settings.models.context_title"),
                        None,
                        &r,
                    )
                    .child(context),
                )
            })
            .when(empty, |page| {
                page.child(
                    label(
                        text::METADATA,
                        crate::i18n::text("app.settings.models.empty"),
                    )
                    .text_color(r.text_subtle),
                )
            })
            .when(self.saving, |page| {
                page.child(
                    label(
                        text::METADATA,
                        crate::i18n::text("app.settings.models.saving"),
                    )
                    .text_color(r.text_subtle),
                )
            })
            .when_some(failure, |page, message| {
                page.child(
                    v_flex()
                        .gap(space::S8)
                        .child(label(text::METADATA, message).text_color(r.feedback_error))
                        .child(
                            Button::new("models-retry")
                                .ghost()
                                .label(crate::i18n::text("app.settings.models.retry"))
                                .disabled(busy)
                                .on_click(cx.listener(|this, _, _, cx| this.retry(cx))),
                        ),
                )
            })
    }
}
