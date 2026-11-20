//! Appearance-specific catalog parameter binding.

use std::borrow::Cow;

pub(in crate::panels::settings) fn fallback_fixed(theme: &str) -> String {
    rust_i18n::t!("app.settings.appearance.fallback_fixed", theme = theme).into_owned()
}

pub(in crate::panels::settings) fn fallback_light(theme: &str) -> String {
    rust_i18n::t!("app.settings.appearance.fallback_light", theme = theme).into_owned()
}

pub(in crate::panels::settings) fn fallback_dark(theme: &str) -> String {
    rust_i18n::t!("app.settings.appearance.fallback_dark", theme = theme).into_owned()
}

pub(in crate::panels::settings) fn preview_editor(size: u8) -> String {
    rust_i18n::t!("app.settings.appearance.preview_editor", size = size).into_owned()
}

pub(in crate::panels::settings) fn preview_terminal(size: u8) -> String {
    rust_i18n::t!("app.settings.appearance.preview_terminal", size = size).into_owned()
}

pub(in crate::panels::settings) fn unavailable(family: &str, fallback: &str) -> String {
    rust_i18n::t!(
        "app.settings.appearance.unavailable",
        family = family,
        fallback = fallback
    )
    .into_owned()
}

pub(in crate::panels::settings) fn weight_steps(family: &str) -> String {
    rust_i18n::t!("app.settings.appearance.weight_steps", family = family).into_owned()
}

pub(in crate::panels::settings) fn size_description(delta: u8) -> String {
    rust_i18n::t!("app.settings.appearance.size_description", delta = delta).into_owned()
}

pub(in crate::panels::settings) fn picker_use(family: &str) -> String {
    rust_i18n::t!("app.settings.appearance.picker_use", family = family).into_owned()
}

pub(in crate::panels::settings) fn custom_fonts_added(families: &str) -> String {
    rust_i18n::t!(
        "app.settings.appearance.custom_fonts_added",
        families = families
    )
    .into_owned()
}

pub(in crate::panels::settings) fn line_width_invalid(min: u16, max: u16) -> String {
    rust_i18n::t!(
        "app.settings.appearance.line_width_invalid",
        min = min,
        max = max
    )
    .into_owned()
}

pub(in crate::panels::settings) fn bundled_theme_name(id: &str) -> Option<Cow<'static, str>> {
    Some(crate::i18n::text(match id {
        "dark" => "app.settings.appearance.bundled.dark",
        "light" => "app.settings.appearance.bundled.light",
        "high-contrast-dark" => "app.settings.appearance.bundled.high-contrast-dark",
        "high-contrast-light" => "app.settings.appearance.bundled.high-contrast-light",
        "minimal-dark" => "app.settings.appearance.bundled.minimal-dark",
        "minimal-light" => "app.settings.appearance.bundled.minimal-light",
        _ => return None,
    }))
}
