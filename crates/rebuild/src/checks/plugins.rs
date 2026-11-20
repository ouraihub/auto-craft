//! Adapts analysis-owned WASM plugin checks to rebuild CheckResult values.
use super::{CheckCx, CheckResult};
use autocraft_core::{Error, ErrorKind, Result};
use serde_json::{Value, json};
use std::{io::Read, path::Path};

const MAX_FILES: usize = 20_000;
const MAX_ENTRIES: usize = 100_000;
const MAX_TEXT: u64 = 256 << 10;
const MAX_TOTAL_TEXT: u64 = 8 << 20;
const MAX_DEPTH: usize = 32;
const SKIP_DIRS: &[&str] = &[
    "node_modules",
    "target",
    ".git",
    "dist",
    "build",
    "Library",
    "Temp",
    "obj",
    "bin",
    ".godot",
    "Intermediate",
    "Saved",
    "DerivedDataCache",
    "__pycache__",
    ".venv",
];

/// Execute configured checks after the built-in checks have established a build.
/// Plugin failures are returned by the analysis boundary as failed check results.
pub(super) fn execute(cx: &CheckCx<'_>, kind: &str) -> Result<Vec<CheckResult>> {
    cx.stop_now()?;
    let checks = cx.analysis.plugin_checks(cx.project, kind)?;
    if checks.is_empty() {
        return Ok(Vec::new());
    }
    let extensions: Vec<_> = checks.iter().flat_map(|check| &check.extensions).collect();
    let context = build_context(cx, kind, &extensions)?;
    cx.analysis
        .run_plugin_checks(cx.project, kind, cx.round, context, cx.stop)?
        .into_iter()
        .map(|result| {
            let autocraft_core::PluginCheckResult {
                id,
                title,
                passed,
                similarity,
                note,
            } = result;
            if id.trim().is_empty() {
                return Err(Error::new(ErrorKind::Corrupt, "分析插件返回了无效检查标识"));
            }
            let mut check = CheckResult::new(&id, cx.round, passed, format!("{title}: {note}"));
            check.similarity = similarity;
            Ok(check)
        })
        .collect()
}

fn build_context(cx: &CheckCx<'_>, kind: &str, extensions: &[&String]) -> Result<Value> {
    let mut listing = FileListing::default();
    let root = cx.ws.app_dir();
    listing.collect(&root, &root, 0, cx)?;
    listing.files.sort_by(|a, b| {
        a.get("path")
            .and_then(Value::as_str)
            .cmp(&b.get("path").and_then(Value::as_str))
    });
    let mut total_text = 0;
    for item in &mut listing.files {
        cx.stop_now()?;
        if item.get("text_status").is_some() {
            continue;
        }
        let Some(relative) = item["path"].as_str() else {
            continue;
        };
        let path = root.join(relative);
        let wanted = path
            .extension()
            .and_then(|value| value.to_str())
            .is_some_and(|extension| {
                extensions.iter().any(|wanted| {
                    wanted
                        .trim_start_matches('.')
                        .eq_ignore_ascii_case(extension)
                })
            });
        if !wanted {
            item["text_status"] = "not_requested".into();
            continue;
        }
        let Some(size) = item["size"].as_u64() else {
            continue;
        };
        let remaining = MAX_TOTAL_TEXT.saturating_sub(total_text);
        let status = if size > MAX_TEXT {
            "size_limit"
        } else if size > remaining || remaining == 0 {
            "budget_limit"
        } else {
            // Bound the read as well as the listing: a file can grow after metadata was read.
            let limit = MAX_TEXT.min(remaining);
            let mut bytes = Vec::new();
            match std::fs::symlink_metadata(&path).and_then(|meta| {
                if !meta.is_file() || meta.file_type().is_symlink() {
                    return Err(std::io::Error::other("not a regular file"));
                }
                std::fs::File::open(&path)?
                    .take(limit + 1)
                    .read_to_end(&mut bytes)
            }) {
                Ok(_) => {
                    total_text += bytes.len().min(limit as usize) as u64;
                    if bytes.len() as u64 > limit {
                        if limit == MAX_TEXT {
                            "size_limit"
                        } else {
                            "budget_limit"
                        }
                    } else {
                        match String::from_utf8(bytes) {
                            Ok(text) => {
                                item["text"] = text.into();
                                "complete"
                            }
                            Err(_) => "non_utf8",
                        }
                    }
                }
                Err(_) => {
                    total_text += bytes.len() as u64;
                    "unreadable"
                }
            }
        };
        item["text_status"] = status.into();
        match status {
            "size_limit" | "budget_limit" => listing.truncated = true,
            "non_utf8" | "unreadable" => listing.unknown = true,
            _ => {}
        }
    }
    Ok(json!({
        "request": cx.ws.manifest.request,
        "kind": kind,
        "stack": cx.ws.manifest.stack,
        "round": cx.round,
        "files": listing.files,
        "truncated": listing.truncated,
        "unknown": listing.unknown,
        "omitted_directories": listing.omitted_directories,
        "excluded_directory_names": SKIP_DIRS,
        "omitted_symlinks": listing.omitted_symlinks,
    }))
}

#[derive(Default)]
struct FileListing {
    files: Vec<Value>,
    entries: usize,
    truncated: bool,
    unknown: bool,
    omitted_directories: usize,
    omitted_symlinks: usize,
}

impl FileListing {
    fn collect(&mut self, root: &Path, dir: &Path, depth: usize, cx: &CheckCx<'_>) -> Result<()> {
        cx.stop_now()?;
        match std::fs::symlink_metadata(dir) {
            Ok(meta) if meta.is_dir() && !meta.file_type().is_symlink() => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound && dir == root => {
                return Ok(());
            }
            _ => {
                self.unknown = true;
                return Ok(());
            }
        }
        let entries = match std::fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(_) => {
                self.unknown = true;
                return Ok(());
            }
        };
        for entry in entries {
            cx.stop_now()?;
            if self.entries >= MAX_ENTRIES || self.files.len() >= MAX_FILES {
                self.truncated = true;
                break;
            }
            self.entries += 1;
            let (entry, file_type) = match entry.and_then(|entry| Ok((entry.file_type()?, entry))) {
                Ok((file_type, entry)) => (entry, file_type),
                Err(_) => {
                    self.unknown = true;
                    continue;
                }
            };
            let path = entry.path();
            if file_type.is_symlink() {
                self.omitted_symlinks += 1;
                continue;
            }
            if file_type.is_dir() {
                let name = entry.file_name();
                if SKIP_DIRS.iter().any(|skip| name.to_str() == Some(skip)) {
                    self.omitted_directories += 1;
                } else if depth >= MAX_DEPTH {
                    self.truncated = true;
                } else {
                    self.collect(root, &path, depth + 1, cx)?;
                }
                continue;
            }
            if !file_type.is_file() {
                self.unknown = true;
                continue;
            }
            let size = match entry.metadata() {
                Ok(meta) => meta.len(),
                Err(_) => {
                    self.unknown = true;
                    self.files.push(json!({
                        "path": path.strip_prefix(root).map_err(|_| Error::corrupt("插件检查路径越界"))?
                            .to_string_lossy().replace('\\', "/"),
                        "size": null,
                        "text_status": "unreadable",
                    }));
                    continue;
                }
            };
            let relative = path
                .strip_prefix(root)
                .map_err(|_| Error::corrupt("插件检查路径越界"))?;
            let Some(relative) = relative.to_str() else {
                self.files.push(json!({
                    "path": relative.to_string_lossy().replace('\\', "/"),
                    "size": size,
                    "text_status": "non_unicode_path",
                }));
                self.unknown = true;
                continue;
            };
            self.files
                .push(json!({ "path": relative.replace('\\', "/"), "size": size }));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checks::tests::{UnusedAnalysis, with_analysis, with_context};
    use autocraft_core::{PluginCheckInfo, PluginCheckResult};
    use ca_engine::workspace::{TargetRef, Workspace};

    #[test]
    fn no_configured_judges_leave_non_utf8_builds_alone() {
        let root = tempfile::tempdir().unwrap();
        let ws = Workspace::create(root.path(), "no judges", "build", TargetRef::None).unwrap();
        std::fs::create_dir_all(ws.app_dir()).unwrap();
        std::fs::write(ws.app_dir().join("legacy.txt"), [0xc4, 0xe3, 0xba, 0xc3]).unwrap();
        with_context(&ws, |cx| assert!(execute(cx, "game").unwrap().is_empty()));
    }

    #[test]
    fn declared_game_scripts_survive_unrelated_text_budget_and_encoding() {
        let root = tempfile::tempdir().unwrap();
        let ws = Workspace::create(root.path(), "scripts", "build", TargetRef::None).unwrap();
        std::fs::create_dir_all(ws.app_dir()).unwrap();
        for index in 0..33 {
            std::fs::write(
                ws.app_dir().join(format!("{index:02}.txt")),
                vec![b'x'; MAX_TEXT as usize],
            )
            .unwrap();
        }
        std::fs::write(ws.app_dir().join("legacy.txt"), [0xc4, 0xe3]).unwrap();
        for (name, text) in [
            ("player.gd", "extends Node"),
            ("scene.tscn", "[gd_scene]"),
            ("Player.CS", "class Player {}"),
        ] {
            std::fs::write(ws.app_dir().join(name), text).unwrap();
        }
        std::fs::create_dir(ws.app_dir().join("target")).unwrap();
        std::fs::write(ws.app_dir().join("target/ignored.cs"), "ignored").unwrap();
        let checks = [PluginCheckInfo {
            id: "game_scripts".into(),
            title: "Game scripts".into(),
            extensions: vec![".GD".into(), "tscn".into(), "cs".into()],
        }];
        let judge = |context: Value| {
            let files = context["files"].as_array().unwrap();
            let find = |path: &str| files.iter().find(|file| file["path"] == path).unwrap();
            let passed = find("player.gd")["text"] == "extends Node"
                && find("scene.tscn")["text"] == "[gd_scene]"
                && find("Player.CS")["text"] == "class Player {}"
                && find("legacy.txt")["text_status"] == "not_requested"
                && find("legacy.txt")["size"] == 2
                && files
                    .iter()
                    .all(|file| !file["path"].as_str().unwrap().starts_with("target/"))
                && context["truncated"] == false
                && context["unknown"] == false
                && context["omitted_directories"] == 1;
            Ok(vec![PluginCheckResult {
                id: "game_scripts".into(),
                title: "Game scripts".into(),
                passed,
                similarity: None,
                note: "Requested scripts are complete".into(),
            }])
        };
        let analysis = UnusedAnalysis::Plugins {
            checks: &checks,
            judge: &judge,
        };
        with_analysis(&ws, &analysis, |cx| {
            let results = execute(cx, "game").unwrap();
            assert_eq!(results.len(), 1);
            assert!(results[0].passed, "{}", results[0].note);
        });
    }

    #[test]
    fn requested_unreadable_encoding_and_limits_are_explicit_not_build_errors() {
        let root = tempfile::tempdir().unwrap();
        let ws = Workspace::create(root.path(), "limits", "build", TargetRef::None).unwrap();
        std::fs::create_dir_all(ws.app_dir()).unwrap();
        std::fs::write(ws.app_dir().join("bad.cs"), [0xc4, 0xe3]).unwrap();
        std::fs::write(
            ws.app_dir().join("large.cs"),
            vec![b'x'; MAX_TEXT as usize + 1],
        )
        .unwrap();
        with_context(&ws, |cx| {
            let extension = "cs".to_owned();
            let context = build_context(cx, "game", &[&extension]).unwrap();
            let files = context["files"].as_array().unwrap();
            let bad = files.iter().find(|file| file["path"] == "bad.cs").unwrap();
            let large = files
                .iter()
                .find(|file| file["path"] == "large.cs")
                .unwrap();
            assert_eq!(bad["text_status"], "non_utf8");
            assert_eq!(large["text_status"], "size_limit");
            assert!(bad.get("text").is_none());
            assert!(large.get("text").is_none());
            assert_eq!(context["unknown"], true);
            assert_eq!(context["truncated"], true);
        });
    }

    #[test]
    fn requested_text_budget_reports_omission_without_unbounded_reads() {
        let root = tempfile::tempdir().unwrap();
        let ws = Workspace::create(root.path(), "budget", "build", TargetRef::None).unwrap();
        std::fs::create_dir_all(ws.app_dir()).unwrap();
        for index in 0..33 {
            std::fs::write(
                ws.app_dir().join(format!("{index:02}.cs")),
                vec![b'x'; MAX_TEXT as usize],
            )
            .unwrap();
        }
        with_context(&ws, |cx| {
            let extension = "cs".to_owned();
            let context = build_context(cx, "game", &[&extension]).unwrap();
            let files = context["files"].as_array().unwrap();
            assert_eq!(
                files
                    .iter()
                    .filter_map(|file| file["text"].as_str())
                    .map(str::len)
                    .sum::<usize>(),
                MAX_TOTAL_TEXT as usize
            );
            let omitted = files.iter().find(|file| file["path"] == "32.cs").unwrap();
            assert_eq!(omitted["text_status"], "budget_limit");
            assert!(omitted.get("text").is_none());
            assert_eq!(context["truncated"], true);
        });
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_files_and_directories_never_expose_outside_text() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret.cs"), "private").unwrap();
        let ws = Workspace::create(root.path(), "links", "build", TargetRef::None).unwrap();
        std::fs::create_dir_all(ws.app_dir()).unwrap();
        std::os::unix::fs::symlink(
            outside.path().join("secret.cs"),
            ws.app_dir().join("secret.cs"),
        )
        .unwrap();
        std::os::unix::fs::symlink(outside.path(), ws.app_dir().join("outside")).unwrap();
        with_context(&ws, |cx| {
            let extension = "cs".to_owned();
            let context = build_context(cx, "game", &[&extension]).unwrap();
            assert!(context["files"].as_array().unwrap().is_empty());
            assert_eq!(context["omitted_symlinks"], 2);
            assert_eq!(context["truncated"], false);
        });
    }

    #[test]
    fn depth_limit_marks_listing_truncated_unlike_excluded_directories() {
        let root = tempfile::tempdir().unwrap();
        let ws = Workspace::create(root.path(), "depth", "build", TargetRef::None).unwrap();
        let mut nested = ws.app_dir();
        for _ in 0..=MAX_DEPTH {
            nested.push("d");
        }
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("main.gd"), "extends Node").unwrap();
        with_context(&ws, |cx| {
            let extension = "gd".to_owned();
            let context = build_context(cx, "game", &[&extension]).unwrap();
            assert_eq!(context["truncated"], true);
            assert_eq!(context["omitted_directories"], 0);
            assert!(context["files"].as_array().unwrap().is_empty());
        });
    }
}
