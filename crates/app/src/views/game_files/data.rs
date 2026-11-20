//! Typed projections of the pinned archive tools; unsupported kinds stay browse-only.
use autocraft_core::{Error, Result, ToolReply};
use serde::Deserialize;
use serde_json::{Value, json};
use std::path::Path;

#[derive(Clone, Debug, Deserialize)]
pub(super) struct Archive {
    pub path: String,
    pub name: String,
    pub kind: String,
    pub size: u64,
    pub list_tool: String,
}
#[derive(Deserialize)]
struct Archives {
    archives: Vec<Archive>,
    truncated: bool,
}
#[derive(Clone, Debug)]
pub(super) struct Item {
    pub name: String,
    pub size: Option<u64>,
    pub picture: Option<Picture>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Picture {
    Unreal { file: String },
    Unity { file: Option<String>, path_id: i64 },
}
#[derive(Deserialize)]
struct FileRow {
    path: String,
    size: Option<u64>,
}
#[derive(Deserialize)]
struct UnityRow {
    path_id: i64,
    class: String,
    name: Option<String>,
    size: u64,
    file: Option<String>,
}
#[derive(Deserialize)]
struct Files {
    files: Vec<FileRow>,
    matching: Option<usize>,
    offset: usize,
}
#[derive(Deserialize)]
struct Objects {
    objects: Vec<UnityRow>,
    matching: usize,
    offset: usize,
}
#[derive(Deserialize)]
struct Packed {
    entries: Vec<FileRow>,
    matching: usize,
}
pub(super) struct Page {
    pub items: Vec<Item>,
    pub next: Option<usize>,
}
#[derive(Debug, Deserialize)]
pub(super) struct Saved {
    pub output: String,
    pub width: u32,
    pub height: u32,
    saved: Option<Dimensions>,
}
#[derive(Debug, Deserialize)]
struct Dimensions {
    width: u32,
    height: u32,
}
impl Saved {
    pub fn dimensions(&self) -> (u32, u32) {
        self.saved
            .as_ref()
            .map_or((self.width, self.height), |d| (d.width, d.height))
    }
}
fn decode<T: serde::de::DeserializeOwned>(value: Value) -> Result<T> {
    serde_json::from_value(value)
        .map_err(|_| Error::protocol(rust_i18n::t!("app.game_files.invalid_result")))
}
pub(super) fn result(reply: ToolReply) -> Value {
    match reply {
        ToolReply::Evidence { body, .. } => body.get("result").cloned().unwrap_or(Value::Null),
        ToolReply::Value(value) => value,
    }
}
pub(super) fn archives(value: Value) -> Result<(Vec<Archive>, bool)> {
    let parsed: Archives = decode(value)?;
    Ok((parsed.archives, parsed.truncated))
}
impl Archive {
    fn effective_tool(&self) -> Result<&'static str> {
        match self.kind.as_str() {
            "unreal_pak" => Ok("game_pak_list"),
            "unreal_iostore" => Ok("game_iostore_list"),
            "unity_assets" | "unity_bundle" => Ok("game_unity_list"),
            "godot_pck" => Ok("core_packed_files_list"),
            _ if self.list_tool == "game_unity_bundle_list" => Ok("game_unity_bundle_list"),
            _ => Err(Error::invalid(rust_i18n::t!("app.game_files.unsupported"))),
        }
    }
    pub fn listing(&self, offset: usize, filter: &str) -> Result<(&'static str, Value)> {
        let tool = self.effective_tool()?;
        let mut params = json!({"path":self.path,"offset":offset,"limit":400});
        params[if tool == "core_packed_files_list" {
            "contains"
        } else {
            "filter"
        }] = json!(filter);
        Ok((tool, params))
    }
    pub fn supports_pictures(&self) -> bool {
        matches!(
            self.kind.as_str(),
            "unreal_pak" | "unity_assets" | "unity_bundle"
        )
    }
}
pub(super) fn page(archive: &Archive, value: Value, offset: usize) -> Result<Page> {
    let (items, matching, start): (Vec<Item>, Option<usize>, usize) =
        match archive.effective_tool()? {
            "game_unity_list" => {
                let rows: Objects = decode(value)?;
                (
                    rows.objects
                        .into_iter()
                        .map(|r| Item {
                            name: r
                                .name
                                .filter(|s| !s.is_empty())
                                .unwrap_or_else(|| format!("#{}", r.path_id)),
                            size: Some(r.size),
                            picture: (r.class == "Texture2D").then_some(Picture::Unity {
                                file: r.file,
                                path_id: r.path_id,
                            }),
                        })
                        .collect(),
                    Some(rows.matching),
                    rows.offset,
                )
            }
            "core_packed_files_list" => {
                let rows: Packed = decode(value)?;
                (
                    rows.entries
                        .into_iter()
                        .map(|r| Item {
                            name: r.path,
                            size: r.size,
                            picture: None,
                        })
                        .collect(),
                    Some(rows.matching),
                    offset,
                )
            }
            "game_pak_list" | "game_iostore_list" | "game_unity_bundle_list" => {
                let rows: Files = decode(value)?;
                (
                    rows.files
                        .into_iter()
                        .map(|r| Item {
                            picture: (archive.kind == "unreal_pak"
                                && r.path.to_lowercase().ends_with(".uasset"))
                            .then(|| Picture::Unreal {
                                file: r.path.clone(),
                            }),
                            name: r.path,
                            size: r.size,
                        })
                        .collect(),
                    rows.matching,
                    rows.offset,
                )
            }
            _ => return Err(Error::invalid(rust_i18n::t!("app.game_files.unsupported"))),
        };
    if start != offset {
        return Err(Error::protocol(rust_i18n::t!(
            "app.game_files.invalid_result"
        )));
    }
    let end = start + items.len();
    let next = matching
        .filter(|&total| !items.is_empty() && end < total)
        .map(|_| end);
    Ok(Page { items, next })
}
pub(super) fn saved(value: Value, expected: &Path) -> Result<Saved> {
    let saved: Saved = decode(value)?;
    let (width, height) = saved.dimensions();
    if Path::new(&saved.output) != expected || width == 0 || height == 0 {
        return Err(Error::protocol(rust_i18n::t!(
            "app.game_files.invalid_result"
        )));
    }
    Ok(saved)
}
impl Picture {
    pub fn export(&self, archive: &Archive, output: &Path) -> (&'static str, Value) {
        match self {
            Self::Unreal { file } => (
                "game_unreal_texture_export",
                json!({"path":archive.path,"file":file,"output":output,"permission":true}),
            ),
            Self::Unity { file, path_id } => (
                "game_unity_asset_export",
                json!({"path":archive.path,"file":file,"path_id":path_id,"output":output}),
            ),
        }
    }
}
pub(super) fn suggested_name(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or("picture");
    let stem = base.strip_suffix(".uasset").unwrap_or(base);
    let safe: String = stem
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .take(80)
        .collect();
    format!("{}.png", if safe.is_empty() { "picture" } else { &safe })
}
#[cfg(test)]
#[path = "data_tests.rs"]
mod tests;
