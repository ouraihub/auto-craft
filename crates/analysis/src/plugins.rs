//! Per-judge visibility of the bounded rebuild context.
pub(super) fn context(context: &serde_json::Value, extensions: &[String]) -> serde_json::Value {
    let mut filtered = context.clone();
    let Some(files) = filtered
        .get_mut("files")
        .and_then(serde_json::Value::as_array_mut)
    else {
        return filtered;
    };
    for file in files {
        let wanted = file
            .get("path")
            .and_then(serde_json::Value::as_str)
            .and_then(|path| std::path::Path::new(path).extension())
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| {
                extensions.iter().any(|wanted| {
                    wanted
                        .trim_start_matches('.')
                        .eq_ignore_ascii_case(extension)
                })
            });
        if !wanted && let Some(object) = file.as_object_mut() {
            object.remove("text");
            object.insert("text_status".into(), "not_requested".into());
        }
    }
    filtered
}
