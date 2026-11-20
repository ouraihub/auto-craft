//! Coding-agent display names and catalog parameters.

use autocraft_core::BackendKind;

pub(in crate::panels::settings) fn name(backend: BackendKind) -> &'static str {
    match backend {
        BackendKind::Pi => "Pi",
        BackendKind::Omp => "Oh My Pi",
        BackendKind::Dsh => "DeepSeek Harness",
    }
}

pub(in crate::panels::settings) fn official(backend: BackendKind) -> String {
    rust_i18n::t!("app.settings.coding.official", engine = name(backend)).into_owned()
}

pub(in crate::panels::settings) fn version(version: &str) -> String {
    rust_i18n::t!("app.settings.coding.version", version = version).into_owned()
}
