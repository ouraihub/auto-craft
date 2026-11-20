//! Reference: craft-anything crates/ai/src/rebuild/kinds/website.rs (300dba58).
//! Website recognition and evidence-driven builder instructions.
use super::*;
fn read_json(path: &Path) -> Option<Value> {
    std::fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
}
#[derive(Debug)]
pub struct WebsiteKind;

impl WebsiteKind {
    pub const ID: &'static str = "website";
}

/// Other pages of the site studied and checked besides the one the person gave, at most (the
/// setting `build.max_pages` chooses how many up to this).
pub const MAX_ROUTES: usize = 12;
/// Where the other pages are listed in the capture folder.
pub const ROUTES_FILE: &str = "routes.json";
/// Where the start page's opened menus, pop-ups and tabs are kept in the capture folder, at the
/// desktop width; [`phone_states_dir`] holds them at the phone width (a menu button often only
/// shows there).
pub const STATES_DIR: &str = "states";

/// The folder for the states opened at a phone `width`: `states-390`.
pub fn phone_states_dir(width: u32) -> String {
    format!("{STATES_DIR}-{width}")
}

/// The phone width among the widths compared (the narrowest under 768 px), if any.
pub fn phone_width(widths: &[u32]) -> Option<u32> {
    widths.iter().copied().filter(|w| *w < 768).min()
}

/// The folders of opened states in a capture folder (`states`, then `states-<width>`).
pub fn state_dirs(capture: &Path) -> Vec<String> {
    let mut dirs: Vec<String> = std::fs::read_dir(capture)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.file_name().to_str().map(String::from))
        .filter(|n| {
            n == STATES_DIR
                || n.strip_prefix("states-")
                    .is_some_and(|w| w.parse::<u32>().is_ok())
        })
        .collect();
    dirs.sort();
    dirs
}

/// The opened states the study pictured, as `states/state-<n>.png (label, kind, width)` (none
/// when there were none).
pub fn pictured_states(ws: &Workspace) -> Vec<String> {
    let mut out = Vec::new();
    for dir in state_dirs(&ws.capture_dir()) {
        let Some(v) = read_json(&ws.capture_dir().join(&dir).join("states.json")) else {
            continue;
        };
        let width = v
            .get("width")
            .and_then(Value::as_u64)
            .map(|w| format!(", {w} px"))
            .unwrap_or_default();
        out.extend(
            v.get("states")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|s| {
                    let file = s.get("file").and_then(Value::as_str)?;
                    let label = s
                        .get("label")
                        .and_then(Value::as_str)
                        .filter(|l| !l.is_empty())
                        .unwrap_or("a control");
                    Some(format!(
                        "{dir}/{file} ({label}, {}{width})",
                        s.get("kind").and_then(Value::as_str).unwrap_or("menu")
                    ))
                }),
        );
    }
    out
}

/// Another page of the site: its path, the folder name its capture uses, and its address.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Route {
    pub path: String,
    pub slug: String,
    pub url: String,
}

/// Up to `max` other pages of the same site, from the links a capture found (`capture.json`'s
/// `links`): same scheme and host, a page rather than a file, each path once, not the start page.
pub fn pick_routes(capture: &Value, start: &str, max: usize) -> Vec<Route> {
    let origin_and_path = |u: &str| -> Option<(String, String)> {
        let (scheme, rest) = u.split_once("://")?;
        if scheme != "http" && scheme != "https" {
            return None;
        }
        let rest = rest.split(['#', '?']).next().unwrap_or_default();
        let (host, path) = rest
            .split_once('/')
            .map(|(h, p)| (h, format!("/{p}")))
            .unwrap_or((rest, "/".into()));
        Some((format!("{scheme}://{}", host.to_ascii_lowercase()), path))
    };
    let Some((origin, start_path)) = origin_and_path(start) else {
        return Vec::new();
    };
    let norm = |p: &str| p.trim_end_matches('/').to_string();
    let mut out: Vec<Route> = Vec::new();
    for link in capture
        .get("links")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if out.len() >= max {
            break;
        }
        let Some((o, path)) = link
            .get("href")
            .and_then(Value::as_str)
            .and_then(origin_and_path)
        else {
            continue;
        };
        let last = path.rsplit('/').next().unwrap_or_default();
        let file = last.contains('.') && !last.ends_with(".html") && !last.ends_with(".htm");
        if o != origin
            || file
            || norm(&path) == norm(&start_path)
            || out.iter().any(|r| norm(&r.path) == norm(&path))
        {
            continue;
        }
        let slug: String = path
            .trim_matches('/')
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() {
                    c.to_ascii_lowercase()
                } else {
                    '-'
                }
            })
            .collect();
        let slug = slug
            .split('-')
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("-");
        if slug.is_empty() || out.iter().any(|r| r.slug == slug) {
            continue;
        }
        out.push(Route {
            url: format!("{origin}{path}"),
            path,
            slug,
        });
    }
    out
}

/// The next level: up to `max` pages linked from already captured ones (`captures`, their
/// `capture.json`s) that aren't in `have` yet, same rules as [`pick_routes`].
pub fn pick_deeper_routes(
    captures: &[Value],
    start: &str,
    have: &[Route],
    max: usize,
) -> Vec<Route> {
    let norm = |p: &str| p.trim_end_matches('/').to_string();
    let mut out: Vec<Route> = Vec::new();
    for c in captures {
        for r in pick_routes(c, start, MAX_ROUTES) {
            if out.len() >= max {
                return out;
            }
            let taken = |x: &Route| norm(&x.path) == norm(&r.path) || x.slug == r.slug;
            if !have.iter().any(taken) && !out.iter().any(taken) {
                out.push(r);
            }
        }
    }
    out
}

/// The other pages captured for a project (none when only the start page was).
pub fn routes(ws: &Workspace) -> Vec<Route> {
    std::fs::read(ws.capture_dir().join(ROUTES_FILE))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

impl TargetKind for WebsiteKind {
    fn id(&self) -> &'static str {
        Self::ID
    }

    fn words(&self) -> KindWords {
        KindWords {
            button: "Open the website",
            icon: "external-link",
            ready: "Your website is ready",
            change_hint: "Ask for a change, e.g. \"make the header blue\" or \"add a contact form\"",
        }
    }

    /// A website wins when the target already is one, the request names one, or the chosen stack
    /// builds sites. The generic word "web" is not enough: "a web app" is an app.
    fn detect(&self, cx: &DetectCx) -> Option<u32> {
        if matches!(cx.target, TargetRef::Website { .. }) {
            Some(100)
        } else if cx.says(&[
            " website",
            " web page",
            " webpage",
            " landing page",
            " site ",
            " webseite",
            " internetseite",
            " sitio web",
            " página web",
            " pagina web",
            " sito ",
            " web sitesi",
            " internet sitesi",
            " trang web",
            "เว็บไซต์",
            "หน้าเว็บ",
            "网站",
            "網站",
            "网页",
            "網頁",
        ]) || says_site_in_estonian(cx)
        {
            Some(50)
        } else if matches!(cx.target, TargetRef::None)
            && stacks::get(cx.stack).is_some_and(|s| s.site)
        {
            // Built as a site with nothing to recreate: "add a contact form" never says website.
            Some(30)
        } else {
            None
        }
    }

    /// A stack the request doesn't name is the builder's to choose, matching the original's.
    fn default_stack(&self, _cx: &DetectCx) -> &'static str {
        stacks::BUILDERS_CHOICE
    }

    fn goal(&self) -> &'static str {
        "a website that looks and behaves like the original"
    }

    fn self_screenshots(&self) -> bool {
        false
    }

    fn brief_sections(&self, cx: &BriefCx) -> Vec<String> {
        let mut out = Vec::new();
        let routes = routes(cx.ws);
        if !routes.is_empty() {
            let list = routes
                .iter()
                .map(|r| format!("{} (capture/routes/{}/)", r.path, r.slug))
                .collect::<Vec<_>>()
                .join(", ");
            out.push(format!(
                "The site has more pages, captured the same way: {list}. Rebuild each at the same path, with the same navigation between them; they're compared with the original too."
            ));
        }
        let states = pictured_states(cx.ws);
        if !states.is_empty() {
            out.push(format!(
                "Its menus, pop-ups and tabs were opened one at a time and photographed in capture/ (on a computer, and on a phone where its controls differ, such as a menu button): {}. Make them open and look the same way at each width.",
                states.join(", ")
            ));
        }
        out
    }

    fn checks(&self, ws: &Workspace) -> Vec<CheckId> {
        if ws.manifest.own_project() {
            return vec![CheckId::OwnBuild, CheckId::OwnTests];
        }
        // A site made from a request alone has nothing to compare with: it must build.
        if matches!(ws.manifest.target, TargetRef::Website { .. }) {
            vec![CheckId::WebVisual]
        } else {
            vec![CheckId::Build]
        }
    }

    fn refine(&self, check: &CheckResult, sees_pictures: SeesPictures) -> String {
        match check.similarity {
            Some(s) if sees_pictures != SeesPictures::No => format!(
                "Check {}: compared with the original, your build looks {s:.0}% alike ({}). Your screenshots are in checks/round-{}/ (screenshot-<width>.png for each width, and routes/<page>/ for other pages) with the layout measured at each width in its capture.json; the original's are in capture/. Open each pair with your file tools; if you cannot view images, compare the layouts in capture.json instead, fix the biggest visible differences first (layout and section order, spacing, sizes, colours, fonts, missing parts), fix the padding, items per row, sizes, colours and fonts the check names at each width with the exact values it gives, add any missing headings or words it lists, build it again, and reply with 1-2 plain sentences on what you changed.",
                check.round, check.note, check.round
            ),
            // The likeness is measured from the screenshots, but the same measurements are in
            // text: the build's capture.json beside the original's.
            Some(s) => format!(
                "Check {}: compared with the original, your build looks {s:.0}% alike ({}). The check measured your build at each width the same way as the original: compare checks/round-{}/capture.json (and routes/<page>/capture.json for other pages) with capture/capture.json, fix the layout and section order and anything missing first, then the padding, items per row, sizes, colours and fonts the check names at each width with the exact values it gives, add any missing headings or words it lists, build it again, and reply with 1-2 plain sentences on what you changed.",
                check.round, check.note, check.round
            ),
            None => format!(
                "Check {}: {}\n\nFix this in app/, build again until it succeeds, and reply with 1-2 plain sentences on what you changed.",
                check.round, check.note
            ),
        }
    }
}
fn says_site_in_estonian(cx: &DetectCx) -> bool {
    cx.words.split_whitespace().any(is_site_word)
}
