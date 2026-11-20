//! Reference: craft-anything engine tools/{browser_tools,website_tools}.rs (300dba58).
use super::*;

pub(super) struct Picture {
    pub score: f64,
    pub layout: f64,
    pub detail: f64,
    pub before_height: u64,
    pub after_height: u64,
}
fn invalid(message: &str) -> Error {
    Error::external(format!("Invalid visual check evidence: {message}"))
}
pub(super) fn evidence_result(reply: ToolReply) -> Result<Value> {
    let value = reply_value(reply);
    value
        .get("result")
        .filter(|v| v.is_object())
        .cloned()
        .ok_or_else(|| invalid("missing result object"))
}
pub(super) fn parse_picture(value: &Value) -> Result<Picture> {
    let unit = |key: &str| {
        value
            .get(key)
            .and_then(Value::as_f64)
            .filter(|v| v.is_finite() && (0.0..=1.0).contains(v))
            .ok_or_else(|| invalid(&format!("missing or out-of-range {key}")))
    };
    let height = |side: &str| {
        value
            .pointer(&format!("/sizes/{side}/height"))
            .and_then(Value::as_u64)
            .filter(|h| *h > 0)
            .ok_or_else(|| invalid("missing positive screenshot height"))
    };
    let layout = unit("layout_similarity")?;
    let detail = unit("perceptual_similarity")?;
    Ok(Picture {
        score: (layout + detail) * 50.0,
        layout: layout * 100.0,
        detail: detail * 100.0,
        before_height: height("before")?,
        after_height: height("after")?,
    })
}
pub(super) fn contained(root: &Path, path: &Path) -> Result<()> {
    let root = fs::canonicalize(root).map_err(|_| invalid("evidence root cannot be resolved"))?;
    let path = fs::canonicalize(path).map_err(|_| invalid("evidence path cannot be resolved"))?;
    if !path.starts_with(root) {
        return Err(invalid("evidence path escapes its owning directory"));
    }
    Ok(())
}
fn component(value: &str) -> bool {
    !value.is_empty() && value != "." && value != ".." && !value.contains(['/', '\\', ':', '\0'])
}
pub(super) fn validate_route(path: &str, slug: &str) -> Result<()> {
    if !component(slug)
        || !path.starts_with('/')
        || path.starts_with("//")
        || path.contains(['\\', '\0', '#'])
    {
        return Err(invalid("unsafe captured route"));
    }
    // Browser URL normalization must not escape a captured route's intended local path.
    let bare = path
        .split('?')
        .next()
        .ok_or_else(|| invalid("route path missing"))?;
    let lower = bare.to_ascii_lowercase();
    if lower.contains("%2e")
        || lower.contains("%2f")
        || lower.contains("%5c")
        || bare.split('/').any(|v| matches!(v, "." | ".."))
    {
        return Err(invalid("captured route contains traversal"));
    }
    Ok(())
}
pub(super) fn read_json(path: &Path) -> Result<Value> {
    let bytes = fs::read(path)
        .map_err(|_| invalid("expected capture JSON file is missing or unreadable"))?;
    serde_json::from_slice(&bytes).map_err(|_| invalid("capture JSON file is malformed"))
}
pub(super) fn validate_capture(value: &Value, output: &Path) -> Result<()> {
    let folder = value
        .get("output_directory")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("capture output_directory missing"))?;
    if value
        .get("url")
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty())
        .is_none()
        || value.get("screenshots").and_then(Value::as_array).is_none()
    {
        return Err(invalid("capture url or screenshots missing"));
    }
    let actual =
        fs::canonicalize(folder).map_err(|_| invalid("capture output directory is missing"))?;
    let expected =
        fs::canonicalize(output).map_err(|_| invalid("requested capture directory is missing"))?;
    if actual != expected {
        return Err(invalid(
            "capture output_directory does not match requested folder",
        ));
    }
    Ok(())
}
pub(super) fn state_file(value: &Value) -> Result<&str> {
    value
        .get("file")
        .and_then(Value::as_str)
        .filter(|v| component(v))
        .ok_or_else(|| invalid("state screenshot must be a local filename"))
}
pub(super) fn validate_states(value: &Value) -> Result<()> {
    if value
        .get("width")
        .and_then(Value::as_u64)
        .filter(|v| (320..=3840).contains(v))
        .is_none()
    {
        return Err(invalid("state width missing or invalid"));
    }
    let rows = value
        .get("states")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("states array missing"))?;
    for row in rows {
        if row.get("label").and_then(Value::as_str).is_none()
            || row.get("kind").and_then(Value::as_str).is_none()
        {
            return Err(invalid("state label or kind missing"));
        }
        if row.get("file").is_some() {
            state_file(row)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn pixels() -> Value {
        json!({"layout_similarity":0.8,"perceptual_similarity":0.2,"sizes":{"before":{"height":1200},"after":{"height":900}}})
    }
    #[test]
    fn score_uses_both_metrics_without_rounding() {
        let p = parse_picture(&pixels()).unwrap();
        assert_eq!(p.score, 50.0);
        assert_eq!(p.before_height, 1200);
        assert_eq!(p.after_height, 900);
    }
    #[test]
    fn missing_metrics_are_not_perfect_or_zero_scores() {
        let mut v = pixels();
        v.as_object_mut().unwrap().remove("perceptual_similarity");
        assert!(parse_picture(&v).is_err());
        v["perceptual_similarity"] = json!(1.01);
        assert!(parse_picture(&v).is_err());
        v["perceptual_similarity"] = json!(-0.01);
        assert!(parse_picture(&v).is_err());
    }
    #[test]
    fn score_endpoints_are_valid() {
        for n in [0.0, 1.0] {
            let mut v = pixels();
            v["layout_similarity"] = json!(n);
            v["perceptual_similarity"] = json!(n);
            assert_eq!(parse_picture(&v).unwrap().score, n * 100.0);
        }
    }
    #[test]
    fn routes_keep_queries_but_cannot_change_origin_or_directory() {
        assert!(validate_route("/plans?annual=1", "plans").is_ok());
        for (path, slug) in [
            ("//evil.test/", "x"),
            ("/a/../b", "x"),
            ("/%2e%2e/private", "x"),
            ("/ok", "../outside"),
            ("/ok", "C:outside"),
        ] {
            assert!(validate_route(path, slug).is_err());
        }
    }
    #[test]
    fn states_reject_external_screenshot_paths() {
        let mut v =
            json!({"width":390,"states":[{"label":"Menu","kind":"button","file":"state-1.png"}]});
        assert!(validate_states(&v).is_ok());
        v["states"][0]["file"] = json!("../secret.png");
        assert!(validate_states(&v).is_err());
    }
    #[test]
    fn containment_rejects_sibling_and_accepts_owned_file() {
        let dir = tempfile::tempdir().unwrap();
        let owned = dir.path().join("owned");
        fs::create_dir(&owned).unwrap();
        let file = owned.join("capture.json");
        fs::write(&file, b"{}").unwrap();
        assert!(contained(&owned, &file).is_ok());
        let sibling = dir.path().join("secret");
        fs::write(&sibling, b"x").unwrap();
        assert!(contained(&owned, &sibling).is_err());
    }
}
