//! Schema-aware projections of the pinned analysis engine's evidence outputs.
use autocraft_core::{Availability, ToolInfo, ToolReply};
use serde_json::{Value, json};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ListKind {
    Functions,
    Strings,
    Imports,
    Sections,
    Classes,
    Modules,
    Resources,
}
impl ListKind {
    pub const ALL: [Self; 7] = [
        Self::Functions,
        Self::Strings,
        Self::Imports,
        Self::Sections,
        Self::Classes,
        Self::Modules,
        Self::Resources,
    ];
    pub fn key(self) -> &'static str {
        match self {
            Self::Functions => "functions",
            Self::Strings => "strings",
            Self::Imports => "imports",
            Self::Sections => "sections",
            Self::Classes => "classes",
            Self::Modules => "modules",
            Self::Resources => "resources",
        }
    }
    pub fn supports(self, kind: &str) -> bool {
        match self {
            Self::Functions | Self::Strings | Self::Imports | Self::Sections => matches!(
                kind,
                "elf" | "pe" | "mach_o" | "mach_o_fat" | "dos_com" | "raw_binary" | "managed"
            ),
            Self::Classes => matches!(
                kind,
                "managed" | "jar" | "java_class" | "apk" | "dex" | "mach_o" | "mach_o_fat"
            ),
            Self::Modules => matches!(kind, "script" | "directory" | "asar"),
            Self::Resources => matches!(kind, "pe" | "managed"),
        }
    }
    pub fn tools(self, kind: &str) -> &'static [&'static str] {
        match self {
            Self::Functions => &["native_functions_list"],
            Self::Strings => &["native_strings_browse"],
            Self::Imports => &["native_named_addresses_list"],
            Self::Sections => &["native_sections_list"],
            Self::Classes => match kind {
                "managed" => &["managed_members_list"],
                "jar" | "java_class" => &["java_classes_find"],
                "apk" | "dex" => &["android_classes_find"],
                _ => &["apple_objc_classes_list"],
            },
            Self::Modules => &["js_app_map"],
            Self::Resources => &["native_pe_resources_list"],
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum EditorTab {
    Pseudocode,
    Disassembly,
    Hex,
    Graph,
    Source,
    Network,
    Results,
}
impl EditorTab {
    pub const ALL: [Self; 7] = [
        Self::Pseudocode,
        Self::Disassembly,
        Self::Hex,
        Self::Graph,
        Self::Source,
        Self::Network,
        Self::Results,
    ];
    pub fn key(self) -> &'static str {
        match self {
            Self::Pseudocode => "pseudocode",
            Self::Disassembly => "disassembly",
            Self::Hex => "hex",
            Self::Graph => "graph",
            Self::Source => "source",
            Self::Network => "network",
            Self::Results => "results",
        }
    }
    pub fn tool(self) -> Option<&'static str> {
        match self {
            Self::Pseudocode => Some("native_function_pseudocode"),
            Self::Disassembly => Some("native_function_disassemble"),
            Self::Hex => Some("native_bytes_show"),
            Self::Graph => Some("native_call_graph_show"),
            Self::Source => Some("js_sources_restore"),
            Self::Network => Some("web_page_watch"),
            Self::Results => None,
        }
    }
}
pub(super) fn body(reply: &ToolReply) -> &Value {
    match reply {
        ToolReply::Evidence { body, .. } | ToolReply::Value(body) => body,
    }
}
pub(super) fn result(reply: &ToolReply) -> &Value {
    let value = body(reply);
    match reply {
        ToolReply::Evidence { .. } => value.get("result").unwrap_or(value),
        ToolReply::Value(_) => value,
    }
}
pub(super) fn scalar(value: &Value) -> String {
    value
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| value.to_string())
}
pub(super) fn field(row: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| row.get(*key).filter(|v| !v.is_null()).map(scalar))
}
pub(super) fn address(row: &Value) -> Option<String> {
    field(row, &["address", "start", "entry", "va", "rva", "offset"])
}
pub(super) fn name(row: &Value) -> Option<String> {
    field(
        row,
        &[
            "name",
            "full_name",
            "symbol",
            "text",
            "module",
            "path",
            "class",
            "id",
        ],
    )
}
pub(super) fn params(
    tool: &ToolInfo,
    selection: Option<&Value>,
    path: &str,
    offset: usize,
) -> Value {
    let selected_address = selection
        .and_then(address)
        .or_else(|| selection.and_then(name));
    let selected_name = selection.and_then(name);
    let mut wanted = vec![
        ("offset", json!(offset)),
        ("limit", json!(500)),
        ("input_path", json!(path)),
        ("path", json!(path)),
    ];
    if let Some(address) = selected_address {
        wanted.extend([("address", json!(address)), ("function", json!(address))]);
    }
    if let Some(name) = selected_name {
        wanted.extend([("name", json!(name)), ("module", json!(name))]);
    }
    let properties = tool
        .input_schema
        .get("properties")
        .and_then(Value::as_object);
    Value::Object(
        wanted
            .into_iter()
            .filter(|(key, _)| properties.is_some_and(|p| p.contains_key(*key)))
            .map(|(k, v)| (k.to_owned(), v))
            .collect(),
    )
}
pub(super) fn choose<'a>(tools: &'a [ToolInfo], ids: &[&str]) -> Option<&'a ToolInfo> {
    ids.iter()
        .find_map(|id| {
            tools
                .iter()
                .find(|tool| tool.id == *id && tool.available == Availability::Ready)
        })
        .or_else(|| {
            ids.iter()
                .find_map(|id| tools.iter().find(|tool| tool.id == *id))
        })
}
pub(super) fn rows(reply: &ToolReply) -> Vec<Value> {
    let value = result(reply);
    if let Some(rows) = value.as_array() {
        return rows.clone();
    }
    // All named collections below occur in the pinned native/managed/Java/Android/JS outputs.
    for key in [
        "functions",
        "strings",
        "names",
        "sections",
        "classes",
        "types",
        "modules",
        "resources",
        "entries",
        "nodes",
        "outputs",
        "requests",
    ] {
        if let Some(rows) = value.get(key).and_then(Value::as_array) {
            return rows.clone();
        }
    }
    if let Some(rows) = value.pointer("/network/requests").and_then(Value::as_array) {
        return rows.clone();
    }
    Vec::new()
}
pub(super) fn next_offset(reply: &ToolReply) -> Option<usize> {
    body(reply)
        .pointer("/coverage/next_offset")
        .and_then(Value::as_u64)
        .and_then(|n| usize::try_from(n).ok())
}
pub(super) fn code(reply: &ToolReply, tab: EditorTab) -> Option<String> {
    let value = result(reply);
    let keys: &[&str] = match tab {
        EditorTab::Pseudocode => &["pseudo_code", "pseudo_c"],
        EditorTab::Disassembly => &["assembly"],
        EditorTab::Source => &["source"],
        EditorTab::Graph => &["dot", "mermaid"],
        EditorTab::Results => &["report"],
        EditorTab::Hex | EditorTab::Network => &[],
    };
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_str).map(str::to_owned))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn normalized_target_kinds_choose_their_available_lists() {
        assert!(ListKind::Classes.supports("java_class"));
        assert_eq!(
            ListKind::Classes.tools("java_class"),
            &["java_classes_find"]
        );
        assert!(ListKind::Functions.supports("mach_o"));
        assert!(ListKind::Classes.supports("mach_o_fat"));
        assert!(ListKind::Functions.supports("raw_binary"));
        assert!(!ListKind::Modules.supports("java_class"));
    }
    #[test]
    fn pinned_output_fields_are_not_confused_with_counts() {
        let reply = ToolReply::Evidence {
            id: None,
            body: json!({"result":{"assembly":"0x10 push rbp", "instructions":1}, "coverage":{"next_offset":500}}),
        };
        assert_eq!(
            code(&reply, EditorTab::Disassembly).as_deref(),
            Some("0x10 push rbp")
        );
        assert!(rows(&reply).is_empty());
        assert_eq!(next_offset(&reply), Some(500));
    }
}
