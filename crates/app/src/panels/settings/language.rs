//! Language preference: persist first, then apply globally and refresh existing windows.

use autocraft_ui::{
    AutocraftTheme,
    components::label,
    metrics::{space, text},
};
use gpui_kit::base::v_flex;
use gpui_kit::component::button::Button;
use gpui_kit::{
    Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _, Render, Styled as _,
    Window,
    component::menu::{DropdownMenu as _, PopupMenuItem},
};

use crate::{
    i18n,
    state::{AppState, run},
};

const LOCALES: [&str; 3] = ["zh-CN", "en", "system"];

pub(super) struct LanguagePage {
    state: Entity<AppState>,
    preference: Option<String>,
    saving: bool,
}

impl LanguagePage {
    pub(super) fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let service = state.read(cx).services.app_prefs.clone();
        run(
            cx,
            move || service.app_prefs(),
            |this: &mut Self, result, cx| {
                match result {
                    Ok(prefs) => this.preference = Some(prefs.locale),
                    Err(error) => this.state.update(cx, |state, cx| state.report(&error, cx)),
                }
                cx.notify();
            },
        );
        Self {
            state,
            preference: None,
            saving: false,
        }
    }
    fn choose(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(locale) = LOCALES.get(index).copied() else {
            return;
        };
        if self.saving || self.preference.is_none() || self.preference.as_deref() == Some(locale) {
            return;
        }
        self.saving = true;
        let service = self.state.read(cx).services.app_prefs.clone();
        run(
            cx,
            move || service.set_locale(locale),
            |this, result, cx| {
                this.saving = false;
                match result {
                    Ok(prefs) => {
                        i18n::apply(&prefs.locale, cx);
                        this.preference = Some(prefs.locale);
                    }
                    Err(error) => this.state.update(cx, |state, cx| state.report(&error, cx)),
                }
                cx.notify();
            },
        );
        cx.notify();
    }
}

impl Render for LanguagePage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let r = AutocraftTheme::global(cx).roles;
        let active = self
            .preference
            .as_deref()
            .and_then(|locale| LOCALES.iter().position(|candidate| *candidate == locale));
        let current = match active {
            Some(0) => i18n::text("app.settings.language.chinese"),
            Some(1) => i18n::text("app.settings.language.english"),
            Some(2) => i18n::text("app.settings.language.system"),
            _ => "".into(),
        };
        let menu_labels = [
            i18n::text("app.settings.language.chinese"),
            i18n::text("app.settings.language.english"),
            i18n::text("app.settings.language.system"),
        ];
        let page = cx.entity();
        let control = Button::new("locale-choice")
            .outline()
            .label(current)
            .dropdown_menu(move |menu, _, _| {
                menu_labels
                    .clone()
                    .into_iter()
                    .enumerate()
                    .fold(menu, |menu, (index, title)| {
                        let page = page.clone();
                        menu.item(
                            PopupMenuItem::new(title)
                                .checked(active == Some(index))
                                .on_click(move |_, _, cx| {
                                    page.update(cx, |this, cx| this.choose(index, cx))
                                }),
                        )
                    })
            });
        v_flex()
            .id("settings-language")
            .gap(space::S16)
            .child(label(
                text::TITLE_SECTION,
                i18n::text("app.settings.language.title"),
            ))
            .child(
                label(
                    text::METADATA,
                    i18n::text("app.settings.language.description"),
                )
                .text_color(r.text_subtle),
            )
            .child(control)
    }
}

#[cfg(test)]
mod tests;
