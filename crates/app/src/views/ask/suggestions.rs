//! Engine wire kinds select questions and plain-language target descriptions.

pub(super) fn questions(kind: Option<&str>) -> &'static [&'static str] {
    match kind {
        None => &["capabilities", "open", "safe_inspect"],
        Some("elf" | "pe" | "mach_o" | "mach_o_fat" | "dos_com" | "raw_binary") => {
            &["program", "network_data", "safe_run", "files"]
        }
        Some("managed" | "jar" | "java_class") => &["program", "parts", "network"],
        Some("sqlite") => &["stored", "tables", "sensitive"],
        Some("apk" | "dex") => &["app", "permissions", "servers", "tracking"],
        Some("asar" | "script") => &["built", "websites", "window_page"],
        Some("wasm") => &["module", "page_needs"],
        Some("directory") => &["folder", "main_file", "built"],
        Some("zip") => &["archive", "archive_program"],
        Some("network_capture") => &["contacted", "network_data"],
        Some("plist") => &["settings"],
        Some("evm") => &["contract", "funds"],
        Some(_) => &["file_kind", "readable"],
    }
}

pub(super) fn kind_key(kind: &str) -> &'static str {
    match kind {
        "elf" => "app.ask.kind.linux",
        "pe" => "app.ask.kind.windows",
        "mach_o" | "mach_o_fat" => "app.ask.kind.apple",
        "managed" => "app.ask.kind.managed",
        "jar" | "java_class" => "app.ask.kind.java",
        "apk" | "dex" => "app.ask.kind.android",
        "wasm" => "app.ask.kind.wasm",
        "zip" => "app.ask.kind.archive",
        "asar" => "app.ask.kind.electron",
        "script" => "app.ask.kind.script",
        "directory" => "app.ask.kind.folder",
        "network_capture" => "app.ask.kind.network",
        "plist" => "app.ask.kind.settings",
        "evm" => "app.ask.kind.contract",
        "sqlite" => "app.ask.kind.database",
        "dos_com" => "app.ask.kind.dos",
        "raw_binary" => "app.ask.kind.machine",
        _ => "app.ask.kind.unknown",
    }
}

pub(super) fn question_key(key: &str) -> &'static str {
    match key {
        "capabilities" => "app.ask.question.capabilities",
        "open" => "app.ask.question.open",
        "safe_inspect" => "app.ask.question.safe_inspect",
        "program" => "app.ask.question.program",
        "network_data" => "app.ask.question.network_data",
        "safe_run" => "app.ask.question.safe_run",
        "files" => "app.ask.question.files",
        "parts" => "app.ask.question.parts",
        "network" => "app.ask.question.network",
        "stored" => "app.ask.question.stored",
        "tables" => "app.ask.question.tables",
        "sensitive" => "app.ask.question.sensitive",
        "app" => "app.ask.question.app",
        "permissions" => "app.ask.question.permissions",
        "servers" => "app.ask.question.servers",
        "tracking" => "app.ask.question.tracking",
        "built" => "app.ask.question.built",
        "websites" => "app.ask.question.websites",
        "window_page" => "app.ask.question.window_page",
        "module" => "app.ask.question.module",
        "page_needs" => "app.ask.question.page_needs",
        "folder" => "app.ask.question.folder",
        "main_file" => "app.ask.question.main_file",
        "archive" => "app.ask.question.archive",
        "archive_program" => "app.ask.question.archive_program",
        "contacted" => "app.ask.question.contacted",
        "settings" => "app.ask.question.settings",
        "contract" => "app.ask.question.contract",
        "funds" => "app.ask.question.funds",
        "file_kind" => "app.ask.question.file_kind",
        "readable" => "app.ask.question.readable",
        _ => "app.ask.question.readable",
    }
}
