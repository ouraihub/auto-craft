//! Model display formatting and semantic catalog parameter binding.

use autocraft_core::{BackendKind, EngineNote, ModelStatus, ThinkingLevel};
use std::borrow::Cow;

pub(in crate::panels::settings) fn effort_label(value: ThinkingLevel) -> Cow<'static, str> {
    crate::i18n::text(match value {
        ThinkingLevel::Off => "app.settings.models.effort.off",
        ThinkingLevel::Minimal => "app.settings.models.effort.minimal",
        ThinkingLevel::Low => "app.settings.models.effort.low",
        ThinkingLevel::Medium => "app.settings.models.effort.medium",
        ThinkingLevel::High => "app.settings.models.effort.high",
        ThinkingLevel::Xhigh => "app.settings.models.effort.xhigh",
        ThinkingLevel::Max => "app.settings.models.effort.max",
    })
}

pub(in crate::panels::settings) fn format_context(tokens: u32) -> String {
    if tokens >= 1_000_000 && tokens.is_multiple_of(1_000_000) {
        format!("{}M", tokens / 1_000_000)
    } else {
        format!("{}K", tokens / 1000)
    }
}

pub(in crate::panels::settings) fn model_label(provider: &str, name: &str, id: &str) -> String {
    if name == id {
        format!("{provider} · {name}")
    } else {
        format!("{provider} · {name} ({id})")
    }
}

pub(in crate::panels::settings) fn model_meta(
    provider: &str,
    context: Option<u32>,
    reasoning: bool,
) -> String {
    let context = context.map_or_else(
        || crate::i18n::text("app.settings.models.default_context").into_owned(),
        format_context,
    );
    let key = if reasoning {
        "app.settings.models.metadata_reasoning"
    } else {
        "app.settings.models.metadata"
    };
    rust_i18n::t!(key, provider = provider, context = context).into_owned()
}

pub(in crate::panels::settings) fn visibility_summary(hidden: usize) -> String {
    if hidden == 0 {
        crate::i18n::text("app.settings.models.new_models_visible").into_owned()
    } else {
        rust_i18n::t!("app.settings.models.visibility_summary", hidden = hidden).into_owned()
    }
}

pub(in crate::panels::settings) fn unavailable(status: &ModelStatus) -> Option<String> {
    Some(match status {
        ModelStatus::Available => return None,
        ModelStatus::ProviderMissing => {
            crate::i18n::text("app.settings.models.provider_missing").into_owned()
        }
        ModelStatus::ProviderDisabled { provider } => {
            rust_i18n::t!("app.settings.models.provider_disabled", provider = provider).into_owned()
        }
        ModelStatus::ModelMissing => {
            crate::i18n::text("app.settings.models.model_missing").into_owned()
        }
        ModelStatus::ModelUnusable { model } => {
            rust_i18n::t!("app.settings.models.model_unusable", model = model).into_owned()
        }
    })
}

pub(in crate::panels::settings) fn engine_note(backend: BackendKind, note: &EngineNote) -> String {
    let engine = backend.label();
    match note {
        EngineNote::Official => {
            rust_i18n::t!("app.settings.models.engine_official", engine = engine).into_owned()
        }
        EngineNote::OnlyProvider { provider } => rust_i18n::t!(
            "app.settings.models.engine_provider",
            engine = engine,
            provider = provider
        )
        .into_owned(),
        EngineNote::ProviderUnavailable => {
            rust_i18n::t!("app.settings.models.engine_unavailable", engine = engine).into_owned()
        }
    }
}

pub(in crate::panels::settings) fn context_available(count: usize) -> String {
    rust_i18n::t!("app.settings.models.context_available", count = count).into_owned()
}

pub(in crate::panels::settings) fn context_model(provider: &str, tokens: u32) -> String {
    rust_i18n::t!(
        "app.settings.models.context_model",
        provider = provider,
        context = format_context(tokens)
    )
    .into_owned()
}

pub(in crate::panels::settings) fn load_error(error: &str) -> String {
    rust_i18n::t!("app.settings.models.load_error", error = error).into_owned()
}

pub(in crate::panels::settings) fn save_error(error: &str) -> String {
    rust_i18n::t!("app.settings.models.save_error", error = error).into_owned()
}
