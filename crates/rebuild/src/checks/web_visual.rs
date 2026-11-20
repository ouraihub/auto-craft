//! Reference: craft-anything crates/ai/src/rebuild/kinds/website.rs (300dba58).
//! Serve the retained build, compare every studied page and report unmeasured evidence explicitly.
use super::*;
use serde_json::json;
mod parsing;
use crate::kinds::website::{ROUTES_FILE, Route, state_dirs};
use parsing::*;
mod comparisons;
use comparisons::*;

pub struct WebVisualCheck;
impl Check for WebVisualCheck {
    fn id(&self) -> &'static str {
        CheckResult::WEB_VISUAL
    }
    fn run(&self, cx: &mut CheckCx<'_>) -> Result<CheckResult> {
        cx.stop_now()?;
        let (ws, round) = (cx.ws, cx.round);
        let Some(site) = ca_engine::outputs::site_dir(&ws.app_dir()) else {
            return Ok(error_result(
                self.id(),
                round,
                format!(
                    "No built site found in {}. Expected index.html in dist/, build/ or out/, or a static index.html without package.json.",
                    ws.app_place()
                ),
            ));
        };
        contained(&ws.app_dir(), &site)?;
        let base = cx.serve_preview(&site)?;
        let widths = cx.build_settings().widths();
        let out = ws.checks_dir().join(format!("round-{round}"));
        if out.exists() {
            contained(&ws.checks_dir(), &out)?;
            fs::remove_dir_all(&out)
                .map_err(|_| Error::external("could not clear previous visual check output"))?;
        }
        fs::create_dir_all(ws.checks_dir())
            .map_err(|_| Error::external("could not create visual check output parent"))?;
        let mut pages = vec![("start page".to_string(), String::new(), base.clone())];
        let route_file = ws.capture_dir().join(ROUTES_FILE);
        if route_file.exists() {
            contained(&ws.capture_dir(), &route_file)?;
            let routes: Vec<Route> = serde_json::from_value(read_json(&route_file)?)
                .map_err(|_| Error::external("invalid captured routes.json"))?;
            for route in routes {
                validate_route(&route.path, &route.slug)?;
                pages.push((
                    route.path.clone(),
                    format!("routes/{}", route.slug),
                    format!("{}{}", base.trim_end_matches('/'), route.path),
                ));
            }
        }
        let mut scores = Vec::new();
        let mut likeness = Vec::new();
        let mut notes = Vec::new();
        let mut artifacts = Vec::new();
        let mut unknowns = Vec::new();
        let mut claims = Vec::new();
        for (name, dir, url) in &pages {
            cx.stop_now()?;
            let original = ws.capture_dir().join(dir);
            let built = out.join(dir);
            if let Some(parent) = built.parent() {
                fs::create_dir_all(parent)
                    .map_err(|_| Error::external("could not create page capture parent"))?;
            }
            let mut page_scores = Vec::new();
            cx.call("web_site_open", json!({"url":url,"settle_ms":1200}))?;
            let captured = evidence_result(cx.call(
                "web_site_capture",
                json!({"output_directory":built,"include_html":false,"widths":widths}),
            )?)?;
            validate_capture(&captured, &built)?;
            let mut per_width = Vec::new();
            for &width in &widths {
                let a = original.join(format!("screenshot-{width}.png"));
                let b = built.join(format!("screenshot-{width}.png"));
                match picture(cx, &a, &b)? {
                    Some(p) => {
                        page_scores.push(p.score);
                        per_width.push(format!("{width} px: {:.0}% alike (layout {:.0}%, detail {:.0}%; page height {} vs {} px)",p.score,p.layout,p.detail,p.before_height,p.after_height));
                        scores_for_page(&mut artifacts, ws, &b)?;
                    }
                    None if a.is_file() => {
                        unknowns.push(format!("{name} at {width} px couldn't be compared"))
                    }
                    None => {}
                }
            }
            if page_scores.is_empty() {
                notes.push(format!("{name}: couldn't be compared"));
                continue;
            }
            let score = page_scores.iter().sum::<f64>() / page_scores.len() as f64;
            scores.push(score);
            likeness.push(PageLikeness {
                name: name.clone(),
                dir: dir.clone(),
                similarity: score,
            });
            contained(&ws.capture_dir(), &original.join("capture.json"))?;
            contained(&ws.checks_dir(), &built.join("capture.json"))?;
            let o = read_json(&original.join("capture.json"))?;
            let b = read_json(&built.join("capture.json"))?;
            let page = if dir.is_empty() {
                "The start page"
            } else {
                name
            };
            claims.extend(page_claims(page, &o, &b));
            let text = [
                text_likeness(&o, &b).map(|v| v.note()),
                structure_likeness(&o, &b).map(|v| v.note()),
            ]
            .into_iter()
            .flatten()
            .chain(design_notes(&o, &b))
            .chain(health_notes(&o, &b))
            .map(|v| format!("; {v}"))
            .collect::<String>();
            notes.push(format!(
                "{name}: {score:.0}% ({}{text})",
                per_width.join("; ")
            ));
        }
        for dir in state_dirs(&ws.capture_dir()) {
            states(
                cx,
                &base,
                &out,
                &dir,
                &mut notes,
                &mut unknowns,
                &mut claims,
                &mut artifacts,
            )?;
        }
        let failed = claims
            .iter()
            .filter(|c| !c.passed)
            .map(|c| format!("{} ({})", c.title, c.detail))
            .collect::<Vec<_>>();
        if !failed.is_empty() {
            notes.push(format!(
                "{} of {} claims don't hold yet: {}",
                failed.len(),
                claims.len(),
                failed.join("; ")
            ));
        }
        if !unknowns.is_empty() {
            notes.push(format!(
                "not compared, so this can't pass yet: {}",
                unknowns.join(", ")
            ));
        }
        let similarity =
            (!scores.is_empty()).then(|| scores.iter().sum::<f64>() / scores.len() as f64);
        if similarity.is_none() {
            notes.push("The build couldn't be compared with the original.".into());
        }
        Ok(CheckResult {
            similarity,
            artifacts,
            pages: if likeness.len() > 1 {
                likeness
            } else {
                Vec::new()
            },
            unknowns,
            claims,
            good_enough: Some(cx.build_settings().good_enough),
            ..CheckResult::new(self.id(), round, similarity.is_some(), notes.join(" | "))
        })
    }
}

fn scores_for_page(artifacts: &mut Vec<PathBuf>, ws: &Workspace, path: &Path) -> Result<()> {
    contained(&ws.root, path)?;
    artifacts.push(
        relative_artifact(ws, path)
            .ok_or_else(|| Error::external("visual artifact escapes workspace"))?,
    );
    Ok(())
}

fn picture(cx: &CheckCx<'_>, original: &Path, built: &Path) -> Result<Option<Picture>> {
    cx.stop_now()?;
    if !original.is_file() || !built.is_file() {
        return Ok(None);
    }
    contained(&cx.ws.capture_dir(), original)?;
    contained(&cx.ws.checks_dir(), built)?;
    let mut params = json!({"before":{"path":original},"after":{"path":built},"common_area":true,"tolerance":24});
    if let Some(width) = original
        .file_name()
        .and_then(|v| v.to_str())
        .and_then(|v| v.strip_prefix("screenshot-"))
    {
        let noise = original.with_file_name(format!("noise-{width}"));
        if noise.is_file() {
            contained(&cx.ws.capture_dir(), &noise)?;
            params["ignore"] = json!({"path":noise});
        }
    }
    parse_picture(&evidence_result(cx.call("web_pixels_compare", params)?)?).map(Some)
}

#[allow(clippy::too_many_arguments)]
fn states(
    cx: &CheckCx<'_>,
    base: &str,
    out: &Path,
    dir: &str,
    notes: &mut Vec<String>,
    unknowns: &mut Vec<String>,
    claims: &mut Vec<Claim>,
    artifacts: &mut Vec<PathBuf>,
) -> Result<()> {
    let theirs = cx.ws.capture_dir().join(dir);
    let file = theirs.join("states.json");
    if !file.exists() {
        return Ok(());
    }
    contained(&cx.ws.capture_dir(), &file)?;
    let original = read_json(&file)?;
    validate_states(&original)?;
    if pair_states(&original, &Value::Null).is_empty() {
        return Ok(());
    }
    let width = original["width"]
        .as_u64()
        .ok_or_else(|| Error::external("states.json has no width"))?;
    let ours = out.join(dir);
    cx.call("web_site_open", json!({"url":base,"settle_ms":1200}))?;
    match cx.call(
        "web_page_states_capture",
        json!({"output_directory":ours,"max_states":8,"width":width}),
    ) {
        Ok(reply) => {
            evidence_result(reply)?;
        }
        Err(e) if matches!(e.kind(), Some(ErrorKind::Cancelled | ErrorKind::Timeout)) => {
            return Err(e);
        }
        Err(e) => {
            unknowns.push(format!(
                "menus and pop-ups at {width} px couldn't be opened"
            ));
            notes.push(format!("menus and pop-ups couldn't be opened: {e}"));
            return Ok(());
        }
    }
    contained(&cx.ws.checks_dir(), &ours.join("states.json"))?;
    let built = read_json(&ours.join("states.json"))?;
    validate_states(&built)?;
    let mut parts = Vec::new();
    for (o, b) in pair_states(&original, &built) {
        let label = o["label"]
            .as_str()
            .ok_or_else(|| Error::external("state label missing"))?;
        let kind = o["kind"]
            .as_str()
            .ok_or_else(|| Error::external("state kind missing"))?;
        let name = if label.is_empty() {
            format!("a {kind}")
        } else {
            format!("the \"{label}\" {kind}")
        };
        claims.push(Claim::new(
            format!("{name} opens in the build at {width} px"),
            b.is_some(),
            "no such control opens",
        ));
        let measured = match b {
            Some(b) => picture(cx, &theirs.join(state_file(o)?), &ours.join(state_file(b)?))?,
            None => None,
        };
        match (b, measured) {
            (Some(b), Some(p)) => {
                parts.push(format!("{name} {:.0}% alike", p.score));
                scores_for_page(artifacts, cx.ws, &ours.join(state_file(b)?))?;
            }
            (Some(_), None) => {
                unknowns.push(format!("{name} at {width} px couldn't be compared"));
                parts.push(format!("{name} couldn't be compared"));
            }
            (None, _) => parts.push(format!("{name} doesn't open in the build")),
        }
    }
    notes.push(format!(
        "opened menus and pop-ups at {width} px: {}",
        parts.join(", ")
    ));
    Ok(())
}
