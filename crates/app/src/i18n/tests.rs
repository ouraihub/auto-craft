//! Catalog integrity and locale/fallback behavior, independent of process-global locale state.

use super::{PseudoBackend, pseudo, resolve, text_for};
use serde_yaml::Value;
use std::collections::{BTreeMap, BTreeSet};

type Catalog = BTreeMap<String, BTreeMap<String, String>>;

fn catalog() -> Result<Catalog, String> {
    fn visit(value: &Value, prefix: &str, output: &mut Catalog) -> Result<(), String> {
        let object = value
            .as_mapping()
            .ok_or_else(|| format!("catalog node must be a mapping: {prefix}"))?;
        if object.contains_key(Value::String("zh-CN".into()))
            || object.contains_key(Value::String("en".into()))
        {
            let mut values = BTreeMap::new();
            for (locale, message) in object {
                let locale = locale.as_str().ok_or("locale must be a string")?;
                if !matches!(locale, "zh-CN" | "en") {
                    return Err(format!(
                        "unexpected locale or mixed subtree at {prefix}: {locale}"
                    ));
                }
                let message = message
                    .as_str()
                    .filter(|text| !text.trim().is_empty())
                    .ok_or_else(|| format!("invalid translation: {prefix}/{locale}"))?;
                values.insert(locale.to_owned(), message.to_owned());
            }
            output.insert(prefix.to_owned(), values);
        } else {
            for (key, child) in object {
                let key = key.as_str().ok_or("semantic keys must be strings")?;
                if key == "_version" {
                    continue;
                }
                let path = if prefix.is_empty() {
                    key.to_owned()
                } else {
                    format!("{prefix}.{key}")
                };
                visit(child, &path, output)?;
            }
        }
        Ok(())
    }
    let value: Value = serde_yaml::from_str(include_str!("../../locales/ui.yml"))
        .map_err(|error| error.to_string())?;
    let mut output = BTreeMap::new();
    visit(&value, "", &mut output)?;
    Ok(output)
}

fn parameters(text: &str) -> Result<BTreeSet<&str>, String> {
    text.split("%{")
        .skip(1)
        .map(|part| {
            part.split_once('}')
                .map(|(parameter, _)| parameter)
                .ok_or_else(|| "unclosed parameter".to_owned())
        })
        .collect()
}

#[test]
fn english_source_and_chinese_are_complete_and_parameters_match() {
    for (key, translations) in catalog().expect("catalog must parse") {
        assert!(key.starts_with("app."), "unexpected namespace: {key}");
        let chinese = translations
            .get("zh-CN")
            .expect("every semantic key must have complete Chinese");
        let english = translations
            .get("en")
            .expect("every semantic key must have complete English source");
        assert_eq!(
            parameters(english).unwrap(),
            parameters(chinese).unwrap(),
            "source parameter mismatch: {key}"
        );
        for (locale, translation) in &translations {
            assert_eq!(
                parameters(chinese).unwrap(),
                parameters(translation).unwrap(),
                "parameter mismatch: {key}/{locale}"
            );
            assert_eq!(
                crate::_rust_i18n_try_translate(locale, &key).as_deref(),
                Some(translation.as_str()),
                "compiled catalog mismatch: {key}/{locale}"
            );
        }
    }
}

#[test]
fn missing_locale_uses_english_source_then_visible_key() {
    assert_eq!(
        text_for("missing", "app.settings.language.title"),
        "Language"
    );
    assert_eq!(text_for("en", "app.settings.language.title"), "Language");
    assert_eq!(
        text_for("zh-CN", "app.settings.mcp.name_required"),
        "请填写服务器名称。"
    );
    assert_eq!(
        text_for("en", "app.settings.mcp.name_required"),
        "Enter a server name."
    );
    assert_eq!(text_for("missing", "app.no_such_key"), "⟦app.no_such_key⟧");
    assert_eq!(
        rust_i18n::t!(
            "app.settings.models.context_available",
            locale = "zh-CN",
            count = 7
        ),
        "7 个可用模型"
    );
}

#[test]
fn locale_selection_preserves_chinese_default_and_matches_system_tags() {
    assert_eq!(resolve("zh-CN", "en-US"), "zh-CN");
    assert_eq!(resolve("en", "zh-CN"), "en");
    assert_eq!(resolve("system", "en-US"), "en");
    assert_eq!(resolve("system", "EN_gb"), "en");
    assert_eq!(resolve("system", "zh-Hans-CN"), "zh-CN");
    assert_eq!(resolve("system", "zh"), "zh-CN");
    assert_eq!(resolve("system", "fr-FR"), "en");
    assert_eq!(resolve("future-locale", "en-US"), "en");
}

#[test]
fn pseudo_locale_expands_text_without_changing_parameters_or_identities() {
    use rust_i18n::Backend as _;
    assert_eq!(
        pseudo("Open %{name} (%{count})"),
        "⟦ ÔÔpëën %{name} (%{count}) ⟧"
    );
    let backend = PseudoBackend;
    assert_eq!(backend.translate("en", "app.settings.title"), None);
    assert_eq!(backend.translate("en-XA", "app.no_such_key"), None);
    for (key, translations) in catalog().unwrap() {
        let english = &translations["en"];
        let expanded = backend
            .translate("en-XA", &key)
            .expect("every English key must support pseudo locale");
        assert!(expanded.starts_with("⟦ ") && expanded.ends_with(" ⟧"));
        assert!(expanded.chars().count() > english.chars().count());
        assert_eq!(
            parameters(english).unwrap(),
            parameters(&expanded).unwrap(),
            "pseudo parameter mismatch: {key}"
        );
    }
    assert_eq!(
        rust_i18n::t!(
            "app.settings.models.context_available",
            locale = "en-XA",
            count = 7
        ),
        "⟦ 7 áávááïïlááblëë môôdëëls ⟧"
    );
}
