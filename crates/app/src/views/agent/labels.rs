//! Localized retained chat labels.
pub(super) fn new_chat() -> String {
    rust_i18n::t!("app.agent.new_chat").to_string()
}
pub(super) fn chat() -> String {
    rust_i18n::t!("app.agent.chat").to_string()
}
pub(super) fn no_workspace() -> String {
    rust_i18n::t!("app.agent.no_workspace").to_string()
}
pub(super) fn no_chats() -> String {
    rust_i18n::t!("app.agent.no_chats").to_string()
}
pub(super) fn starting() -> String {
    rust_i18n::t!("app.agent.starting").to_string()
}
pub(super) fn closed_hint() -> String {
    rust_i18n::t!("app.agent.closed_hint").to_string()
}
pub(super) fn truncated() -> String {
    rust_i18n::t!("app.agent.truncated").to_string()
}
pub(super) fn phase_idle() -> String {
    rust_i18n::t!("app.agent.phase_idle").to_string()
}
pub(super) fn phase_running() -> String {
    rust_i18n::t!("app.agent.phase_running").to_string()
}
pub(super) fn phase_waiting() -> String {
    rust_i18n::t!("app.agent.phase_waiting").to_string()
}
pub(super) fn phase_closed() -> String {
    rust_i18n::t!("app.agent.phase_closed").to_string()
}
pub(super) fn thinking() -> String {
    rust_i18n::t!("app.agent.thinking").to_string()
}
pub(super) fn tool_input() -> String {
    rust_i18n::t!("app.agent.tool_input").to_string()
}
pub(super) fn tool_output() -> String {
    rust_i18n::t!("app.agent.tool_output").to_string()
}
pub(super) fn image() -> String {
    rust_i18n::t!("app.agent.image").to_string()
}
pub(super) fn placeholder() -> String {
    rust_i18n::t!("app.agent.placeholder").to_string()
}
pub(super) fn send() -> String {
    rust_i18n::t!("app.agent.send").to_string()
}
pub(super) fn stop() -> String {
    rust_i18n::t!("app.agent.stop").to_string()
}
pub(super) fn yes() -> String {
    rust_i18n::t!("app.agent.yes").to_string()
}
pub(super) fn no() -> String {
    rust_i18n::t!("app.agent.no").to_string()
}
pub(super) fn submit() -> String {
    rust_i18n::t!("app.agent.submit").to_string()
}
pub(super) fn cancel() -> String {
    rust_i18n::t!("app.agent.cancel").to_string()
}
pub(super) fn permission() -> String {
    rust_i18n::t!("app.agent.permission").to_string()
}
pub(super) fn question() -> String {
    rust_i18n::t!("app.agent.question").to_string()
}
pub(super) fn other() -> String {
    rust_i18n::t!("app.agent.other").to_string()
}
pub(super) fn cmd_compact() -> String {
    rust_i18n::t!("app.agent.cmd_compact").to_string()
}
pub(super) fn cmd_name() -> String {
    rust_i18n::t!("app.agent.cmd_name").to_string()
}
pub(super) fn cmd_model() -> String {
    rust_i18n::t!("app.agent.cmd_model").to_string()
}
pub(super) fn cmd_builtin() -> String {
    rust_i18n::t!("app.agent.cmd_builtin").to_string()
}
pub(super) fn name_empty() -> String {
    rust_i18n::t!("app.agent.name_empty").to_string()
}
pub(super) fn model_usage() -> String {
    rust_i18n::t!("app.agent.model_usage").to_string()
}
pub(super) fn model_unknown() -> String {
    rust_i18n::t!("app.agent.model_unknown").to_string()
}
pub(super) fn builtin_source(backend: &str) -> String {
    format!("{backend} / {}", cmd_builtin())
}
pub(super) fn model_none(query: &str) -> String {
    rust_i18n::t!("app.agent.model_none", query = query).to_string()
}
pub(super) fn model_many(candidates: &[String]) -> String {
    rust_i18n::t!("app.agent.model_many", candidates = candidates.join("、")).to_string()
}
pub(super) fn effort_unsupported(effort: &str) -> String {
    rust_i18n::t!("app.agent.effort_unsupported", effort = effort).to_string()
}
