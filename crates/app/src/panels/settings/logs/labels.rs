//! Log view labels; raw log messages and event identities remain unchanged.

pub(in crate::panels::settings) fn recent(count: usize) -> String {
    rust_i18n::t!("app.settings.logs.recent", count = count).into_owned()
}

pub(in crate::panels::settings) fn filter_trace(short: &str) -> String {
    rust_i18n::t!("app.settings.logs.filter_trace", short = short).into_owned()
}

pub(in crate::panels::settings) fn event(code: &str) -> std::borrow::Cow<'static, str> {
    crate::i18n::text(match code {
        "ENTRY" => "app.settings.logs.event.entry",
        "EXIT" => "app.settings.logs.event.exit",
        "ERROR" => "app.settings.logs.event.error",
        "CANCEL" => "app.settings.logs.event.cancel",
        "TIMEOUT" => "app.settings.logs.event.timeout",
        "UNFINISHED" => "app.settings.logs.event.unfinished",
        _ => "app.settings.logs.event.other",
    })
}
