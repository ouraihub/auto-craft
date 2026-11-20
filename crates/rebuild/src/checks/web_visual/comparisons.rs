//! Reference: craft-anything crates/ai/src/rebuild/kinds/website.rs (300dba58).
use super::*;
/// How alike two captures' visible text is (`capture.json`'s `text` and `headings`).
#[derive(Debug, Clone, PartialEq)]
pub struct TextLikeness {
    /// Word overlap, 0..=100 (Dice over word counts).
    pub similarity: f64,
    /// The original's headings the build doesn't have, in page order (at most 8).
    pub missing_headings: Vec<String>,
    /// The original's most frequent words the build never uses (at most 12).
    pub missing_words: Vec<String>,
}

pub(crate) fn words(text: &str) -> std::collections::BTreeMap<String, usize> {
    let mut m = std::collections::BTreeMap::new();
    for w in text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.chars().count() >= 2)
    {
        *m.entry(w.to_lowercase()).or_insert(0) += 1;
    }
    m
}

/// Compare the visible text of the original's capture with the build's. None when the original
/// has no text to compare.
pub fn text_likeness(original: &Value, built: &Value) -> Option<TextLikeness> {
    let text = |v: &Value| {
        v.get("text")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let (a, b) = (words(&text(original)), words(&text(built)));
    let total_a: usize = a.values().sum();
    if total_a == 0 {
        return None;
    }
    let total_b: usize = b.values().sum();
    let shared: usize = a
        .iter()
        .map(|(w, n)| (*n).min(b.get(w).copied().unwrap_or(0)))
        .sum();
    let similarity = 200.0 * shared as f64 / (total_a + total_b) as f64;
    let norm = |s: &str| {
        s.split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase()
    };
    let built_headings: std::collections::BTreeSet<String> = built
        .get("headings")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|h| h.get("text").and_then(Value::as_str))
        .map(norm)
        .collect();
    let missing_headings = original
        .get("headings")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|h| h.get("text").and_then(Value::as_str))
        .filter(|h| !h.trim().is_empty() && !built_headings.contains(&norm(h)))
        .take(8)
        .map(|h| h.trim().to_string())
        .collect();
    let mut missing: Vec<(&String, &usize)> =
        a.iter().filter(|(w, _)| !b.contains_key(*w)).collect();
    missing.sort_by(|x, y| y.1.cmp(x.1).then_with(|| x.0.cmp(y.0)));
    let missing_words = missing
        .into_iter()
        .take(12)
        .map(|(w, _)| w.clone())
        .collect();
    Some(TextLikeness {
        similarity,
        missing_headings,
        missing_words,
    })
}

impl TextLikeness {
    /// One line for the check's note, e.g. `text 72% alike; missing headings: "Pricing"; missing words: faq, plans`.
    pub fn note(&self) -> String {
        let mut s = format!("text {:.0}% alike", self.similarity);
        if !self.missing_headings.is_empty() {
            s.push_str(&format!(
                "; missing headings: {}",
                self.missing_headings
                    .iter()
                    .map(|h| format!("\"{h}\""))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if !self.missing_words.is_empty() {
            s.push_str(&format!(
                "; missing words: {}",
                self.missing_words.join(", ")
            ));
        }
        s
    }
}

/// The parts of a page the structure is compared by, as (element, plain name). Elements with
/// the same name count as one part (`ul` and `ol` are both a list).
const PARTS: &[(&str, &str)] = &[
    ("header", "header"),
    ("nav", "navigation bar"),
    ("main", "main area"),
    ("footer", "footer"),
    ("section", "section"),
    ("article", "article"),
    ("aside", "sidebar"),
    ("form", "form"),
    ("h1", "top heading"),
    ("h2", "subheading"),
    ("h3", "subheading"),
    ("h4", "subheading"),
    ("ul", "list"),
    ("ol", "list"),
    ("img", "image"),
    ("video", "video"),
    ("button", "button"),
    ("input", "form field"),
    ("select", "form field"),
    ("textarea", "form field"),
];

/// The page's parts in document order, from `capture.json`'s outline (a role counts as its element).
fn skeleton(capture: &Value) -> Vec<&'static str> {
    capture
        .get("outline")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| {
            let tag = match item.get("role").and_then(Value::as_str) {
                Some("banner") => "header",
                Some("navigation") => "nav",
                Some("main") => "main",
                Some("contentinfo") => "footer",
                Some("complementary") => "aside",
                Some("form" | "search") => "form",
                Some("button") => "button",
                Some("img") => "img",
                _ => item.get("tag").and_then(Value::as_str)?,
            };
            PARTS.iter().find(|(t, _)| *t == tag).map(|(_, name)| *name)
        })
        .collect()
}

/// Length of the longest sequence both have in the same order.
fn common_len(a: &[&str], b: &[&str]) -> usize {
    let mut row = vec![0usize; b.len() + 1];
    for x in a {
        let mut diag = 0;
        for (j, y) in b.iter().enumerate() {
            let up = row[j + 1];
            row[j + 1] = if x == y { diag + 1 } else { up.max(row[j]) };
            diag = up;
        }
    }
    row[b.len()]
}

/// How alike two captures' page structure is: their landmarks, headings, lists, media and
/// controls, in order.
#[derive(Debug, Clone, PartialEq)]
pub struct StructureLikeness {
    /// 0..=100: the parts both have in the same order, over all parts of both.
    pub similarity: f64,
    /// Parts the original has more of, as (plain name, how many more), most first (at most 6).
    pub missing: Vec<(&'static str, usize)>,
}

/// Compare the original's page structure with the build's. None when the original's outline has
/// none of the parts.
pub fn structure_likeness(original: &Value, built: &Value) -> Option<StructureLikeness> {
    let (a, b) = (skeleton(original), skeleton(built));
    if a.is_empty() {
        return None;
    }
    let similarity = 200.0 * common_len(&a, &b) as f64 / (a.len() + b.len()) as f64;
    let count = |s: &[&str], name: &str| s.iter().filter(|p| **p == name).count();
    let mut missing: Vec<(&'static str, usize)> = Vec::new();
    for (_, name) in PARTS {
        if !missing.iter().any(|(n, _)| n == name) {
            let more = count(&a, name).saturating_sub(count(&b, name));
            if more > 0 {
                missing.push((name, more));
            }
        }
    }
    missing.sort_by_key(|m| std::cmp::Reverse(m.1));
    missing.truncate(6);
    Some(StructureLikeness {
        similarity,
        missing,
    })
}

impl StructureLikeness {
    /// One line for the check's note, e.g. `structure 80% alike; missing parts: 2 lists, a form`.
    pub fn note(&self) -> String {
        let mut s = format!("structure {:.0}% alike", self.similarity);
        if !self.missing.is_empty() {
            let parts = self
                .missing
                .iter()
                .map(|(name, n)| {
                    if *n == 1 {
                        format!("a {name}")
                    } else {
                        format!("{n} {name}s")
                    }
                })
                .collect::<Vec<_>>();
            s.push_str(&format!("; missing parts: {}", parts.join(", ")));
        }
        s
    }
}

/// What the build gets wrong that the original doesn't, from both captures' `health` and `a11y`:
/// script and console errors, requests that fail (compared by path, since the build is served
/// from elsewhere), and controls without a name, pictures without alt text or a missing page
/// language. Problems the original has too aren't the build's to fix.
pub fn health_notes(original: &Value, built: &Value) -> Vec<String> {
    let mut out = Vec::new();
    let new_errors = new_errors(original, built);
    if !new_errors.is_empty() {
        let shown: Vec<String> = new_errors
            .iter()
            .take(3)
            .map(|t| format!("\"{t}\""))
            .collect();
        out.push(format!(
            "the build logs {} error(s) the original doesn't: {}",
            new_errors.len(),
            shown.join("; ")
        ));
    }
    let failing = new_failures(original, built);
    if !failing.is_empty() {
        out.push(format!(
            "{} request(s) fail in the build: {}",
            failing.len(),
            failing
                .iter()
                .take(5)
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    let n = |v: &Value, k: &str| {
        v.pointer(&format!("/a11y/{k}"))
            .and_then(Value::as_u64)
            .unwrap_or(0)
    };
    if built.get("a11y").is_some() {
        for (k, what) in [
            ("unnamed_controls", "controls without a name"),
            ("images_without_alt", "pictures without alt text"),
        ] {
            if n(built, k) > n(original, k) {
                out.push(format!(
                    "{} {what} (the original has {})",
                    n(built, k),
                    n(original, k)
                ));
            }
        }
        let lang = |v: &Value| {
            v.pointer("/a11y/lang")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string()
        };
        if lang(built).is_empty() && !lang(original).is_empty() {
            out.push(format!(
                "the page language isn't set (the original's is {})",
                lang(original)
            ));
        }
    }
    out
}

fn health_list(v: &Value, p: &str) -> Vec<Value> {
    v.pointer(p)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

/// The build's error texts the original doesn't log.
fn new_errors(original: &Value, built: &Value) -> Vec<String> {
    let text = |e: &Value| {
        e.get("text")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let theirs: Vec<String> = health_list(original, "/health/errors")
        .iter()
        .map(text)
        .collect();
    health_list(built, "/health/errors")
        .iter()
        .map(text)
        .filter(|t| !t.is_empty() && !theirs.contains(t))
        .collect()
}

/// The build's failing requests the original doesn't have, by path, as `/logo.png (404)`.
fn new_failures(original: &Value, built: &Value) -> Vec<String> {
    let path = |f: &Value| {
        let url = f.get("url").and_then(Value::as_str).unwrap_or_default();
        let rest = url.split_once("://").map_or(url, |(_, r)| r);
        rest.find('/').map_or("/".to_string(), |i| {
            rest.get(i..).unwrap_or("/").to_string()
        })
    };
    let theirs: Vec<String> = health_list(original, "/health/failed_requests")
        .iter()
        .map(path)
        .collect();
    health_list(built, "/health/failed_requests")
        .iter()
        .filter(|f| !theirs.contains(&path(f)))
        .map(|f| {
            let why = f
                .get("status")
                .and_then(Value::as_u64)
                .map(|s| s.to_string())
                .or_else(|| f.get("error").and_then(Value::as_str).map(String::from))
                .unwrap_or_default();
            format!("{} ({why})", path(f))
        })
        .collect()
}

/// What `page` of the build must do, each judged on its own from the two captures: not scroll
/// sideways at any width where the original doesn't, log no errors and fail no requests the
/// original doesn't, and have the original's landmarks. A claim is made only when both captures
/// hold what it needs (older captures make none).
pub fn page_claims(page: &str, original: &Value, built: &Value) -> Vec<Claim> {
    let mut out = Vec::new();
    let n = |v: &Value, k: &str| v.get(k).and_then(Value::as_u64).unwrap_or(0);
    let built_layouts = built
        .get("layouts")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for theirs in original
        .get("layouts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let w = n(theirs, "width");
        if n(theirs, "page_w") > w + 1 {
            continue;
        }
        if let Some(ours) = built_layouts.iter().find(|l| n(l, "width") == w) {
            let pw = n(ours, "page_w");
            out.push(Claim::new(
                format!("{page} doesn't scroll sideways at {w} px"),
                pw <= w + 1,
                format!("it's {pw} px wide"),
            ));
        }
    }
    if original.get("health").is_some() && built.get("health").is_some() {
        let errors = new_errors(original, built);
        out.push(Claim::new(
            format!("{page} logs no errors the original doesn't"),
            errors.is_empty(),
            errors.first().cloned().unwrap_or_default(),
        ));
        let failing = new_failures(original, built);
        out.push(Claim::new(
            format!("{page} loads every file it asks for"),
            failing.is_empty(),
            failing
                .iter()
                .take(5)
                .cloned()
                .collect::<Vec<_>>()
                .join(", "),
        ));
    }
    let landmarks = |v: &Value| -> Vec<String> {
        v.pointer("/a11y/landmarks")
            .and_then(Value::as_object)
            .map(|m| m.keys().cloned().collect())
            .unwrap_or_default()
    };
    if built.get("a11y").is_some() && !landmarks(original).is_empty() {
        let ours = landmarks(built);
        let missing: Vec<String> = landmarks(original)
            .into_iter()
            .filter(|l| !ours.contains(l))
            .collect();
        out.push(Claim::new(
            format!("{page} has the original's landmarks"),
            missing.is_empty(),
            format!("missing: {}", missing.join(", ")),
        ));
    }
    out
}

/// A `#rrggbb` or `#rrggbbaa` colour as (r, g, b, a).
fn rgba(c: &str) -> Option<[u8; 4]> {
    let h = c.strip_prefix('#')?;
    let byte = |i: usize| h.get(i..i + 2).and_then(|x| u8::from_str_radix(x, 16).ok());
    match h.len() {
        6 => Some([byte(0)?, byte(2)?, byte(4)?, 255]),
        8 => Some([byte(0)?, byte(2)?, byte(4)?, byte(6)?]),
        _ => None,
    }
}
/// The same colour to the eye: each channel within 3 (rounding in the browser), or the same text.
fn same_colour(a: &str, b: &str) -> bool {
    match (rgba(a), rgba(b)) {
        (Some(x), Some(y)) => x.iter().zip(y).all(|(p, q)| p.abs_diff(q) <= 3),
        _ => a.eq_ignore_ascii_case(b),
    }
}

/// The first family of a CSS font stack, as a person names it: `"__Figtree_a1b2c3", sans-serif`
/// (how next/font renames a font) reads `Figtree`.
fn family(stack: &str) -> String {
    let first = stack
        .split(',')
        .next()
        .unwrap_or_default()
        .trim()
        .trim_matches(['"', '\'']);
    first
        .trim_start_matches('_')
        .split('_')
        .next()
        .unwrap_or(first)
        .to_string()
}

/// The values of a `design` list (`[{value, count}]`), most used first.
fn design_values<'a>(capture: &'a Value, key: &str) -> Vec<&'a str> {
    capture
        .pointer(&format!("/design/{key}"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|x| x.get("value").and_then(Value::as_str))
        .collect()
}

/// What a person sees differ between the original's capture and the build's, for the builder:
/// colours, the main font, text sizes and spacing the build doesn't use, and at each width a
/// page that scrolls sideways, heading sizes, and sections whose padding, items per row or
/// content column differ (sections are paired by tag and heading, in order). Both are
/// `capture.json` documents; empty when nothing differs or there's nothing to compare.
pub fn design_notes(original: &Value, built: &Value) -> Vec<String> {
    let mut out = Vec::new();
    let mut colours = Vec::new();
    for (key, what) in [("backgrounds", "background"), ("text_colors", "text")] {
        let ours = design_values(built, key);
        colours.extend(
            design_values(original, key)
                .into_iter()
                .take(6)
                .filter(|c| !ours.iter().any(|o| same_colour(c, o)))
                .map(|c| format!("{c} ({what})")),
        );
    }
    if !colours.is_empty() {
        out.push(format!(
            "colours the original uses that the build doesn't: {}",
            colours.join(", ")
        ));
    }
    if let (Some(a), Some(b)) = (
        design_values(original, "fonts").first(),
        design_values(built, "fonts").first(),
    ) && !family(a).eq_ignore_ascii_case(&family(b))
    {
        out.push(format!(
            "main font: {} in the original, {} in the build",
            family(a),
            family(b)
        ));
    }
    for (key, what) in [("font_sizes", "text sizes"), ("spacing", "spacing")] {
        let ours = design_values(built, key);
        let missing: Vec<&str> = design_values(original, key)
            .into_iter()
            .take(8)
            .filter(|v| !ours.contains(v))
            .collect();
        if !missing.is_empty() {
            out.push(format!(
                "{what} the original uses that the build doesn't: {}",
                missing.join(", ")
            ));
        }
    }
    let n = |v: &Value, k: &str| v.get(k).and_then(Value::as_u64).unwrap_or(0);
    let layouts = |v: &Value| {
        v.get("layouts")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
    };
    let built_layouts = layouts(built);
    for theirs in layouts(original) {
        let w = n(&theirs, "width");
        let Some(ours) = built_layouts.iter().find(|l| n(l, "width") == w) else {
            continue;
        };
        let mut diffs = Vec::new();
        if n(ours, "page_w") > w + 1 && n(&theirs, "page_w") <= w + 1 {
            diffs.push(format!(
                "the page scrolls sideways ({} px wide)",
                n(ours, "page_w")
            ));
        }
        for (k, name) in [
            ("h1", "top heading"),
            ("h2", "subheadings"),
            ("body", "body text"),
        ] {
            let size = |l: &Value| {
                l.pointer(&format!("/type/{k}"))
                    .and_then(Value::as_u64)
                    .unwrap_or(0)
            };
            let (a, b) = (size(&theirs), size(ours));
            if a > 0 && b > 0 && a.abs_diff(b) > 2 {
                diffs.push(format!("{name} {b} px, the original's {a} px"));
            }
        }
        diffs.extend(block_diffs(&theirs, ours).into_iter().take(4));
        if !diffs.is_empty() {
            out.push(format!("at {w} px: {}", diffs.join("; ")));
        }
    }
    out
}

/// The sections of one width whose padding, items per row or content column differ. Sections are
/// paired first by tag and heading in order (the n-th `section "Plans"` with the n-th), then,
/// because the builder writes its own words, the rest of each tag in page order (the original's
/// "Why people switch" with the build's "Why people like it"). Items per row is a grid's measure,
/// so it isn't compared for a `nav` or `header` (a row of links, checked by the pictures).
fn block_diffs(original: &Value, built: &Value) -> Vec<String> {
    let blocks = |v: &Value| {
        v.get("blocks")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
    };
    let key = |b: &Value| {
        let heading = b
            .get("heading")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase();
        (
            b.get("tag")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            heading,
        )
    };
    let nums = |b: &Value, k: &str| -> Vec<u64> {
        b.get(k)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(|x| x.as_u64().unwrap_or(0))
            .collect()
    };
    let n = |v: &Value, k: &str| v.get(k).and_then(Value::as_u64).unwrap_or(0);
    let column = |b: &Value, k: &str| b.get("column").map(|c| n(c, k));
    let (theirs, ours) = (blocks(original), blocks(built));
    // The build's block paired with each of the original's, if any.
    let mut partner: Vec<Option<usize>> = vec![None; theirs.len()];
    let mut taken = vec![false; ours.len()];
    let mut pair = |same: &dyn Fn(&Value, &Value) -> bool, partner: &mut Vec<Option<usize>>| {
        for (i, a) in theirs.iter().enumerate() {
            if partner[i].is_some() {
                continue;
            }
            if let Some(j) = ours
                .iter()
                .enumerate()
                .position(|(j, b)| !taken[j] && same(a, b))
            {
                taken[j] = true;
                partner[i] = Some(j);
            }
        }
    };
    pair(&|a, b| key(a) == key(b), &mut partner);
    pair(&|a, b| key(a).0 == key(b).0, &mut partner);
    let mut out = Vec::new();
    for (i, a) in theirs.iter().enumerate() {
        let k = key(a);
        let Some(b) = partner[i].and_then(|j| ours.get(j)) else {
            continue;
        };
        let mut what = Vec::new();
        let (pa, pb) = (nums(a, "pad"), nums(b, "pad"));
        if pa.len() == pb.len() && pa.iter().zip(&pb).any(|(x, y)| x.abs_diff(*y) > 4) {
            let show = |p: &[u64]| p.iter().map(u64::to_string).collect::<Vec<_>>().join(" ");
            what.push(format!(
                "padding {} vs the original's {}",
                show(&pb),
                show(&pa)
            ));
        }
        let grid = !matches!(k.0.as_str(), "nav" | "header");
        if grid && n(a, "per_row") != n(b, "per_row") {
            what.push(format!(
                "{} per row vs {}",
                n(b, "per_row"),
                n(a, "per_row")
            ));
        }
        if let (Some(x), Some(y)) = (column(a, "max_w"), column(b, "max_w"))
            && x.abs_diff(y) > 8
        {
            what.push(format!("content column {y} px vs {x} px"));
        } else if column(a, "max_w").is_some() && column(b, "max_w").is_none() {
            what.push(format!(
                "no content column (the original's is {} px)",
                column(a, "max_w").unwrap_or(0)
            ));
        }
        if !what.is_empty() {
            let name = if k.1.is_empty() {
                format!("the {}", k.0)
            } else {
                format!(
                    "the \"{}\" {}",
                    a.get("heading").and_then(Value::as_str).unwrap_or_default(),
                    k.0
                )
            };
            out.push(format!("{name}: {}", what.join(", ")));
        }
    }
    out
}

/// Pair each state the study pictured with the build's: the same kind of control with the same
/// label, ignoring case and spacing. The build's half is None when it has no such control or
/// couldn't picture it. Both are `states.json` documents.
pub fn pair_states<'a>(
    original: &'a Value,
    built: &'a Value,
) -> Vec<(&'a Value, Option<&'a Value>)> {
    let rows = |v: &'a Value| {
        v.get("states")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|s| s.get("file").is_some())
    };
    let key = |s: &Value| {
        let label = s
            .get("label")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase();
        (
            s.get("kind")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            label,
        )
    };
    rows(original)
        .map(|o| (o, rows(built).find(|b| key(b) == key(o))))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_control_is_not_paired_with_an_unrelated_menu() {
        let original =
            json!({"states":[{"kind":"menu","label":" Account  settings ","file":"state-1.png"}]});
        let built =
            json!({"states":[{"kind":"menu","label":"account SETTINGS","file":"state-2.png"}]});
        assert_eq!(
            pair_states(&original, &built)[0].1.unwrap()["file"],
            "state-2.png"
        );
        let wrong =
            json!({"states":[{"kind":"tab","label":"account settings","file":"state-2.png"}]});
        assert!(pair_states(&original, &wrong)[0].1.is_none());
    }
    #[test]
    fn page_claims_expose_new_overflow_and_request_failures() {
        let original = json!({"layouts":[{"width":390,"page_w":390}],"health":{"errors":[],"failed_requests":[]}});
        let built = json!({"layouts":[{"width":390,"page_w":500}],"health":{"errors":[],"failed_requests":[{"url":"http://localhost/logo.png","status":404}]}});
        let claims = page_claims("Start", &original, &built);
        assert_eq!(claims.iter().filter(|c| !c.passed).count(), 2);
        assert!(
            claims
                .iter()
                .any(|c| !c.passed && c.detail.contains("/logo.png (404)"))
        );
    }
    #[test]
    fn text_counts_repetitions_and_structure_preserves_order() {
        let original =
            json!({"text":"hello hello world","outline":[{"tag":"header"},{"tag":"main"}]});
        let built = json!({"text":"hello world","outline":[{"tag":"main"},{"tag":"header"}]});
        assert_eq!(text_likeness(&original, &built).unwrap().similarity, 80.0);
        assert_eq!(
            structure_likeness(&original, &built).unwrap().similarity,
            50.0
        );
    }
}
