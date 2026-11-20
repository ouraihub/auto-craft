//! Default Agent and host preferences through public storage contracts.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "integration test helpers fail loudly"
)]

use autocraft_core::{
    AppPrefService, AppPrefs, AppearancePrefs, BackendKind, LogLevel, ModelPrefService,
};
use autocraft_settings::SqliteSettingsStore;
use tempfile::TempDir;

#[test]
fn default_agent_is_stored_and_cleared() {
    let dir = TempDir::new().unwrap();
    let store = SqliteSettingsStore::open(&dir.path().join("autocraft.db")).unwrap();
    assert_eq!(store.prefs().unwrap().default_agent, None);
    assert_eq!(
        store
            .set_default_agent(Some(BackendKind::Omp))
            .unwrap()
            .default_agent,
        Some(BackendKind::Omp)
    );
    assert_eq!(store.prefs().unwrap().default_agent, Some(BackendKind::Omp));
    assert_eq!(store.set_default_agent(None).unwrap().default_agent, None);
}

#[test]
fn app_preferences_default_and_survive_reopen() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("autocraft.db");
    let store = SqliteSettingsStore::open(&path).unwrap();
    assert_eq!(store.app_prefs().unwrap(), AppPrefs::default());
    assert_eq!(AppPrefs::default().log_level, LogLevel::Info);
    assert_eq!(store.app_prefs().unwrap().locale, "zh-CN");
    store.set_locale("en").unwrap();
    assert!(!store.app_prefs().unwrap().onboarding_done);
    store.set_onboarding_done(true).unwrap();
    store.set_logs_visible(true).unwrap();
    store.set_log_level(LogLevel::Debug).unwrap();
    let appearance = AppearancePrefs {
        theme: "minimal-light".into(),
        code_font_size: 15,
        code_ligatures: true,
        ..AppearancePrefs::default()
    };
    let expected = AppPrefs {
        logs_visible: true,
        log_level: LogLevel::Debug,
        appearance: appearance.clone(),
        locale: "en".into(),
        onboarding_done: true,
    };
    assert_eq!(store.set_appearance(&appearance).unwrap(), expected);
    drop(store);
    let store = SqliteSettingsStore::open(&path).unwrap();
    assert_eq!(store.app_prefs().unwrap(), expected);
}

#[test]
fn unknown_locale_is_preserved_without_overwriting_other_preferences() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("autocraft.db");
    let store = SqliteSettingsStore::open(&path).unwrap();
    store.set_logs_visible(true).unwrap();
    store.set_onboarding_done(true).unwrap();
    let saved = store.set_locale("future-locale").unwrap();
    assert_eq!(saved.locale, "future-locale");
    assert!(saved.logs_visible);
    assert!(saved.onboarding_done);
    drop(store);
    let reopened = SqliteSettingsStore::open(&path).unwrap();
    assert_eq!(reopened.app_prefs().unwrap(), saved);
}
