//! Normal-launch window and accessible-control comparison with runtime study evidence.
use super::*;
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
const INTERACTIVE: &[&str] = &[
    "button",
    "splitbutton",
    "togglebutton",
    "menu",
    "menubaritem",
    "menuitem",
    "menubutton",
    "tab",
    "tabitem",
    "checkbox",
    "radiobutton",
    "combobox",
    "popupbutton",
    "edit",
    "textfield",
    "hyperlink",
    "link",
    "slider",
];

pub(super) fn compare(
    cx: &CheckCx<'_>,
    program: &Path,
    args: &[String],
    dir: &Path,
) -> Result<(String, Vec<Claim>, Vec<PathBuf>)> {
    let original = cx.ws.capture_dir().join("window.png");
    if !original.is_file() {
        return Ok((String::new(), Vec::new(), Vec::new()));
    }
    cx.stop_now()?;
    let program = if program.is_absolute() || program.components().count() > 1 {
        std::path::absolute(program).map_err(|_| Error::invalid("启动路径无效"))?
    } else {
        let Some(found) = program.to_str().and_then(ca_platform::which::which) else {
            return Ok((
                " 正常启动对比失败：找不到启动程序。".into(),
                Vec::new(),
                Vec::new(),
            ));
        };
        std::path::absolute(found).map_err(|_| Error::invalid("启动路径无效"))?
    };
    let shot =
        std::path::absolute(dir.join("window.png")).map_err(|_| Error::invalid("截图路径无效"))?;
    let tree = std::path::absolute(dir.join("ui-tree.json"))
        .map_err(|_| Error::invalid("控件树路径无效"))?;
    process::remove_old(&shot)?;
    process::remove_old(&tree)?;
    let original_tree = cx.ws.capture_dir().join("ui-tree.json");
    let left = cx.stop().time_left().unwrap_or(Duration::from_secs(90));
    if left < Duration::from_secs(2) {
        return Err(Error::timeout("窗口对比已达到时限"));
    }
    let wait = left
        .saturating_sub(Duration::from_secs(1))
        .as_secs()
        .min(15);
    let mut params = json!({"program":program,"args":args,"working_directory":std::path::absolute(cx.ws.app_dir()).map_err(|_|Error::invalid("项目路径无效"))?,"wait_secs":wait,"output":shot});
    if original_tree.is_file() {
        params["ui_tree"] = json!(tree);
    }
    match cx.call("native_window_capture", params) {
        Ok(reply) => {
            let body = reply_value(reply);
            if body.get("isError").and_then(Value::as_bool) == Some(true)
                || body.get("error").is_some()
            {
                return Ok((
                    " 正常启动窗口捕获失败，未进行比较。".into(),
                    Vec::new(),
                    Vec::new(),
                ));
            }
        }
        Err(e) if matches!(e.kind(), Some(ErrorKind::Cancelled | ErrorKind::Timeout)) => {
            return Err(e);
        }
        Err(error) => {
            return Ok((
                format!(
                    " 正常启动窗口捕获失败（{}），未进行比较。",
                    autocraft_core::ErrorCode::code(&error)
                ),
                Vec::new(),
                Vec::new(),
            ));
        }
    }
    cx.stop_now()?;
    let mut note = String::new();
    let image = process::read_bounded(&shot, MAX_SCREENSHOT_BYTES as u64)
        .ok()
        .and_then(|b| decode_png(&b).ok());
    if let Some(image) = image {
        let (score, _) = likeness(cx, &image, &[original])?;
        if let Some(score) = score {
            note.push_str(&format!(" 正常启动窗口与原程序相似度为 {score:.0}%。"));
        }
    } else {
        note.push_str(" 正常启动未生成有效窗口图片，无法比较。");
    }
    let mut claims = Vec::new();
    if original_tree.is_file() {
        let read = |p: &Path| {
            process::read_bounded(p, 2 * 1024 * 1024)
                .ok()
                .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        };
        if let (Some(a), Some(b)) = (read(&original_tree), read(&tree)) {
            let (summary, controls) = control_comparison(&a, &b);
            note.push_str(&summary);
            claims = controls;
        } else {
            note.push_str(" 控件树不可读，未比较可访问控件。");
        }
    }
    let artifacts = [&shot, &tree]
        .into_iter()
        .filter(|p| p.is_file())
        .filter_map(|p| relative_artifact(cx.ws, p))
        .collect();
    Ok((note, claims, artifacts))
}
fn labelled(tree: &Value) -> Vec<(String, String, String)> {
    tree.get("nodes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(10_000)
        .filter_map(|n| {
            let shown = n
                .get("title")?
                .as_str()?
                .replace('&', "")
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            let shown = shown.chars().take(200).collect::<String>();
            let key = shown.trim_end_matches(':').trim().to_lowercase();
            let role = n.get("role").and_then(Value::as_str).unwrap_or_default();
            (!key.is_empty()).then(|| {
                (
                    key,
                    shown,
                    role.strip_prefix("AX").unwrap_or(role).to_lowercase(),
                )
            })
        })
        .collect()
}
fn control_comparison(original: &Value, built: &Value) -> (String, Vec<Claim>) {
    let (a, b) = (labelled(original), labelled(built));
    let ours: BTreeSet<&str> = b.iter().map(|(key, ..)| key.as_str()).collect();
    let mut seen = BTreeSet::new();
    let claims = a
        .iter()
        .filter(|(key, _, role)| INTERACTIVE.contains(&role.as_str()) && seen.insert(key))
        .take(20)
        .map(|(key, shown, role)| {
            claim(
                format!("存在原程序的“{shown}”{role}控件"),
                ours.contains(key.as_str()),
                "缺少相同标签的控件",
            )
        })
        .collect();
    let mut counts = BTreeMap::<&str, usize>::new();
    for (key, ..) in &b {
        *counts.entry(key).or_default() += 1;
    }
    let mut shared = 0;
    for (key, ..) in &a {
        if let Some(count) = counts.get_mut(key.as_str())
            && *count > 0
        {
            shared += 1;
            *count -= 1;
        }
    }
    let note = if a.len() >= 3 {
        format!(
            " 控件标签相似度为 {:.0}%。",
            200.0 * shared as f64 / (a.len() + b.len()) as f64
        )
    } else {
        String::new()
    };
    (note, claims)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_interactive_controls_fail_but_mnemonics_and_case_match() {
        let original = json!({"nodes":[{"title":"&Save:","role":"AXButton"},{"title":"Open","role":"button"}]});
        let built = json!({"nodes":[{"title":"save","role":"button"}]});
        let (_, claims) = control_comparison(&original, &built);
        assert!(claims[0].passed);
        assert!(!claims[1].passed);
    }
}
