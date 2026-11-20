//! Embedded UI translations: English source, complete Chinese, visible missing-key markers.
//! Agent replies, protocol identities and external command output are never translated here.

use std::borrow::Cow;

use autocraft_core::{AppPrefService, Result};
use gpui_kit::App;

mod shared;

/// Relabels cached home/project snapshots without catalog I/O or changing the active selection.
pub(crate) fn refresh_picker(
    picker: &gpui_kit::Entity<lumakit_model_picker::Picker>,
    cx: &mut App,
    applied_locale: &mut String,
) {
    let locale = rust_i18n::locale();
    if applied_locale == &*locale {
        return;
    }
    applied_locale.clear();
    applied_locale.push_str(&locale);
    const IDS: [&str; 7] = ["off", "minimal", "low", "medium", "high", "xhigh", "max"];
    let labels = IDS.map(|id| {
        text(match id {
            "off" => "app.home.effort_off",
            "minimal" => "app.home.effort_minimal",
            "low" => "app.home.effort_low",
            "medium" => "app.home.effort_medium",
            "high" => "app.home.effort_high",
            "xhigh" => "app.home.effort_xhigh",
            _ => "app.home.effort_max",
        })
    });
    let labels = std::array::from_fn::<_, 7, _>(|index| (IDS[index], labels[index].as_ref()));
    picker.update(cx, |picker, cx| picker.set_effort_labels(&labels, cx));
}

/// Resolves a stored preference without rewriting unknown preferences in the database.
/// The first-run preference is `zh-CN`; `system` uses the Windows UI language.
pub(crate) fn resolve(preference: &str, system: &str) -> &'static str {
    let requested = if preference == "system" {
        system
    } else {
        preference
    };
    let base = requested.split(['-', '_']).next().unwrap_or_default();
    if base.eq_ignore_ascii_case("en") {
        "en"
    } else if base.eq_ignore_ascii_case("zh") {
        "zh-CN"
    } else {
        "en"
    }
}

fn system_locale() -> String {
    sys_locale::get_locale().unwrap_or_default()
}

/// Applies the saved language before the first window. Storage failures belong to the caller's
/// startup error boundary; they are not silently replaced with default preferences.
pub(crate) fn init(service: &dyn AppPrefService, cx: &mut App) -> Result<()> {
    let prefs = service.app_prefs()?;
    apply(&prefs.locale, cx);
    Ok(())
}

/// Refreshes every open window, including the GPUI Kit widgets' own translations.
pub(crate) fn apply(preference: &str, cx: &mut App) {
    let locale = resolve(preference, &system_locale());
    let pseudo = cfg!(debug_assertions)
        && std::env::var_os("AUTOCRAFT_PSEUDO_LOCALE").is_some_and(|value| value == "1");
    gpui_kit::component::set_locale(locale);
    // Kit and the application share rust-i18n's process-global locale.
    // Apply the developer override last instead of immediately overwriting it.
    rust_i18n::set_locale(if pseudo { "en-XA" } else { locale });
    shared::install(if pseudo { "en-XA" } else { locale });
    cx.refresh_windows();
}

/// A borrowed built-in label when available; no allocation on the ordinary rendering path.
pub(crate) fn text(key: &'static str) -> Cow<'static, str> {
    text_for(&rust_i18n::locale(), key)
}

fn text_for(locale: &str, key: &'static str) -> Cow<'static, str> {
    crate::_rust_i18n_try_translate(locale, key)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| format!("⟦{key}⟧").into())
}

/// Generated pseudo translations cover every catalog consumer, including direct `t!` calls.
/// Only developer mode selects `en-XA`; the stored language preference is never overwritten.
#[derive(Debug)]
pub(crate) struct PseudoBackend;

impl rust_i18n::Backend for PseudoBackend {
    fn available_locales(&self) -> Vec<Cow<'_, str>> {
        vec![Cow::Borrowed("en-XA")]
    }

    fn translate(&self, locale: &str, key: &str) -> Option<Cow<'_, str>> {
        if locale != "en-XA" {
            return None;
        }
        static MESSAGES: std::sync::LazyLock<std::collections::HashMap<String, String>> =
            std::sync::LazyLock::new(|| {
                crate::_rust_i18n_backend()
                    .messages_for_locale("en")
                    .unwrap_or_default()
                    .into_iter()
                    .map(|(key, source)| (key.into_owned(), pseudo(&source)))
                    .collect()
            });
        MESSAGES
            .get(key)
            .map(|message| Cow::Borrowed(message.as_str()))
    }

    fn messages_for_locale(&self, _locale: &str) -> Option<Vec<(Cow<'_, str>, Cow<'_, str>)>> {
        None
    }
}

fn pseudo(source: &str) -> String {
    let mut output = String::with_capacity(source.len() * 2 + 12);
    output.push_str("⟦ ");
    let mut remaining = source;
    while let Some(start) = remaining.find("%{") {
        pseudo_text(&remaining[..start], &mut output);
        let parameter = &remaining[start..];
        let Some(end) = parameter.find('}') else {
            output.push_str(parameter);
            remaining = "";
            break;
        };
        output.push_str(&parameter[..=end]);
        remaining = &parameter[end + 1..];
    }
    pseudo_text(remaining, &mut output);
    output.push_str(" ⟧");
    output
}

fn pseudo_text(text: &str, output: &mut String) {
    for letter in text.chars() {
        output.push_str(match letter {
            'a' => "áá",
            'e' => "ëë",
            'i' => "ïï",
            'o' => "ôô",
            'u' => "üü",
            'A' => "ÁÁ",
            'E' => "ËË",
            'I' => "ÏÏ",
            'O' => "ÔÔ",
            'U' => "ÜÜ",
            _ => {
                output.push(letter);
                continue;
            }
        });
    }
}

#[cfg(test)]
mod tests;
