//! Provider-specific labels and parameter binding; protocol identities stay stable.

use autocraft_core::{ModelType, Protocol};
use std::borrow::Cow;

pub(in crate::panels::settings) fn protocol_short(protocol: Protocol) -> &'static str {
    match protocol {
        Protocol::OpenaiChat => "OpenAI",
        Protocol::OpenaiResponses => "Responses",
        Protocol::AnthropicMessages => "Anthropic",
    }
}

pub(in crate::panels::settings) fn protocol(value: Protocol) -> Cow<'static, str> {
    crate::i18n::text(match value {
        Protocol::OpenaiChat => "app.settings.providers.protocols.openai_chat",
        Protocol::OpenaiResponses => "app.settings.providers.protocols.openai_responses",
        Protocol::AnthropicMessages => "app.settings.providers.protocols.anthropic_messages",
    })
}

pub(in crate::panels::settings) fn model_type(value: ModelType) -> Cow<'static, str> {
    crate::i18n::text(match value {
        ModelType::Chat => "app.settings.providers.types.chat",
        ModelType::Image => "app.settings.providers.types.image",
        ModelType::Video => "app.settings.providers.types.video",
        ModelType::Tts => "app.settings.providers.types.tts",
        ModelType::Stt => "app.settings.providers.types.stt",
        ModelType::Embedding => "app.settings.providers.types.embedding",
        ModelType::Rerank => "app.settings.providers.types.rerank",
        ModelType::Unknown => "app.settings.providers.types.unknown",
    })
}

pub(in crate::panels::settings) fn probe_get(url: &str) -> String {
    rust_i18n::t!("app.settings.providers.probe_get", url = url).into_owned()
}

pub(in crate::panels::settings) fn request_post(url: &str) -> String {
    rust_i18n::t!("app.settings.providers.request_post", url = url).into_owned()
}

pub(in crate::panels::settings) fn keys_enabled(enabled: usize, total: usize) -> String {
    rust_i18n::t!(
        "app.settings.providers.keys_enabled",
        enabled = enabled,
        total = total
    )
    .into_owned()
}

pub(in crate::panels::settings) fn check_ok(count: usize) -> String {
    rust_i18n::t!("app.settings.providers.check_ok", count = count).into_owned()
}

pub(in crate::panels::settings) fn picked_count(count: usize) -> String {
    rust_i18n::t!("app.settings.providers.picked_count", count = count).into_owned()
}

pub(in crate::panels::settings) fn delete_confirm(name: &str) -> String {
    rust_i18n::t!("app.settings.providers.delete_confirm", name = name).into_owned()
}

pub(in crate::panels::settings) fn remove_line_confirm(protocol: Protocol) -> String {
    rust_i18n::t!(
        "app.settings.providers.remove_line_confirm",
        protocol = protocol_short(protocol)
    )
    .into_owned()
}
