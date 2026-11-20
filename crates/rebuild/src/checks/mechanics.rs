//! Reference: craft-anything crates/ai/src/rebuild/kinds/mechanics.rs (300dba58).
//! Mechanics sheets are checked against numeric literals in the built source.
use super::*;
use ca_engine::mechanics::{self, MechanicValue, SheetProblem};
use std::collections::HashSet;
const MAX_FILE: u64 = 2 * 1024 * 1024;
const MAX_CODE: u64 = 32 * 1024 * 1024;
const MAX_DEPTH: usize = 16;
const CODE: &[&str] = &[
    "rs", "js", "mjs", "ts", "tsx", "jsx", "py", "cs", "cpp", "cc", "c", "h", "hpp", "gd", "go",
    "java", "kt", "swift", "lua", "glsl", "tscn", "tres", "cfg", "json", "toml", "yaml", "yml",
    "xml", "ini",
];
pub struct MechanicsCheck;
impl Check for MechanicsCheck {
    fn id(&self) -> &'static str {
        "mechanics"
    }
    fn run(&self, cx: &mut CheckCx<'_>) -> Result<CheckResult> {
        let sheet = match mechanics::read(&cx.ws.mechanics_path()) {
            Ok(s) => s,
            Err(p) => {
                return Ok(error_result(
                    self.id(),
                    cx.round,
                    format!(
                        "{} Keep app/mechanics.json with version 1, values and source or estimated.",
                        problem_note(&p)
                    ),
                ));
            }
        };
        let found = numbers_in(&cx.ws.app_dir(), &|| cx.stop_now())?;
        let estimated = sheet.values.iter().filter(|v| v.estimated).count();
        let sourced = sheet
            .values
            .iter()
            .filter(|v| !v.source.trim().is_empty())
            .count();
        let missing: Vec<String> = sheet
            .values
            .iter()
            .filter_map(|v| match v.value {
                MechanicValue::Number(n) if !v.estimated && !found.contains(&key(n)) => {
                    Some(format!("{} ({n})", v.name))
                }
                _ => None,
            })
            .collect();
        let mut note = format!(
            "app/mechanics.json has {} values ({sourced} with a source, {estimated} estimated).",
            sheet.values.len()
        );
        if !missing.is_empty() {
            note.push_str(&format!(
                " Numbers from the original aren't in the code as written: {}.",
                missing
                    .iter()
                    .take(8)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        let claims = sheet
            .values
            .iter()
            .filter_map(|v| match v.value {
                MechanicValue::Number(n) if !v.estimated => Some(claim(
                    format!("{} ({n}) is used in the code", v.name),
                    found.contains(&key(n)),
                    "not in the code as written",
                )),
                _ => None,
            })
            .take(60)
            .collect();
        Ok(result_with_claims(
            self.id(),
            cx.round,
            missing.is_empty(),
            note,
            claims,
        ))
    }
}
fn problem_note(p: &SheetProblem) -> String {
    match p {
        SheetProblem::Missing => "There's no app/mechanics.json yet.".into(),
        SheetProblem::TooLarge => "app/mechanics.json is too large.".into(),
        SheetProblem::Unreadable(_) => "app/mechanics.json isn't readable.".into(),
        SheetProblem::Newer(_) => "app/mechanics.json was written for a newer version.".into(),
        SheetProblem::Empty => "app/mechanics.json has no values.".into(),
    }
}
fn key(n: f64) -> String {
    format!("{:.6e}", n.abs())
}
fn numbers_in(dir: &Path, check: &dyn Fn() -> Result<()>) -> Result<HashSet<String>> {
    let mut out = HashSet::new();
    let (mut budget, mut entries) = (MAX_CODE, 8192);
    walk(dir, 0, &mut budget, &mut entries, &mut out, check)?;
    Ok(out)
}
fn walk(
    dir: &Path,
    depth: usize,
    budget: &mut u64,
    remaining: &mut usize,
    found: &mut HashSet<String>,
    check: &dyn Fn() -> Result<()>,
) -> Result<()> {
    check()?;
    if depth > MAX_DEPTH {
        return Err(Error::invalid(
            "机制检查源代码目录过深，无法确认代码使用情况",
        ));
    }
    let entries = fs::read_dir(dir).map_err(|_| Error::external("无法读取机制检查源代码目录"))?;
    for entry in entries {
        check()?;
        if *remaining == 0 {
            return Err(Error::invalid("机制检查目录项过多，无法确认代码使用情况"));
        }
        *remaining -= 1;
        let entry = entry.map_err(|_| Error::external("无法读取机制检查目录项"))?;
        let kind = entry
            .file_type()
            .map_err(|_| Error::external("无法读取机制检查文件类型"))?;
        if kind.is_symlink() {
            continue;
        }
        let p = entry.path();
        if kind.is_dir() {
            let skip = [
                "target",
                "node_modules",
                "build",
                "dist",
                "bin",
                "obj",
                "library",
                "temp",
                "shots",
                "addons",
                "binaries",
                "intermediate",
                "saved",
                "deriveddatacache",
                ".git",
                ".venv",
                ".godot",
                "__pycache__",
                ".next",
                ".turbo",
            ];
            if !skip
                .iter()
                .any(|s| entry.file_name().to_string_lossy().eq_ignore_ascii_case(s))
            {
                walk(&p, depth + 1, budget, remaining, found, check)?;
            }
        } else if kind.is_file()
            && !entry
                .file_name()
                .to_string_lossy()
                .eq_ignore_ascii_case("mechanics.json")
            && p.extension()
                .and_then(|x| x.to_str())
                .is_some_and(|x| CODE.iter().any(|c| c.eq_ignore_ascii_case(x)))
        {
            let size = entry
                .metadata()
                .map_err(|_| Error::external("无法读取机制检查文件大小"))?
                .len();
            if size > MAX_FILE || size > *budget {
                return Err(Error::invalid(
                    "机制检查源代码超出大小限制，无法确认代码使用情况",
                ));
            }
            let bytes = super::process::read_bounded(&p, MAX_FILE.min(*budget))?;
            *budget -= bytes.len() as u64;
            literals(&bytes, found);
        }
    }
    Ok(())
}
fn literals(text: &[u8], found: &mut HashSet<String>) {
    let mut i = 0;
    while i < text.len() {
        let starts = text[i].is_ascii_digit()
            || (text[i] == b'.' && text.get(i + 1).is_some_and(u8::is_ascii_digit));
        let in_word =
            i > 0 && (text[i - 1].is_ascii_alphanumeric() || matches!(text[i - 1], b'_' | b'.'));
        if !starts || in_word {
            i += 1;
            continue;
        }
        let start = i;
        while text
            .get(i)
            .is_some_and(|c| c.is_ascii_digit() || *c == b'_')
        {
            i += 1;
        }
        if text.get(i) == Some(&b'.') && text.get(i + 1) != Some(&b'.') {
            i += 1;
            while text
                .get(i)
                .is_some_and(|c| c.is_ascii_digit() || *c == b'_')
            {
                i += 1;
            }
        }
        if text.get(i).is_some_and(|c| matches!(c, b'e' | b'E')) {
            let mut end = i + 1;
            if text.get(end).is_some_and(|c| matches!(c, b'+' | b'-')) {
                end += 1;
            }
            if text.get(end).is_some_and(u8::is_ascii_digit) {
                i = end + 1;
                while text
                    .get(i)
                    .is_some_and(|c| c.is_ascii_digit() || *c == b'_')
                {
                    i += 1;
                }
            }
        }
        let value = String::from_utf8_lossy(&text[start..i]).replace('_', "");
        let mut suffix_end = i;
        while text
            .get(suffix_end)
            .is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'_')
        {
            suffix_end += 1;
        }
        let suffix = &text[i..suffix_end];
        let valid_suffix = suffix.is_empty()
            || [
                b"f".as_slice(),
                b"f32",
                b"f64",
                b"u8",
                b"u16",
                b"u32",
                b"u64",
                b"usize",
                b"i8",
                b"i16",
                b"i32",
                b"i64",
                b"isize",
                b"L",
                b"UL",
                b"d",
                b"m",
            ]
            .contains(&suffix);
        if valid_suffix
            && let Ok(n) = value.parse::<f64>()
            && n.is_finite()
        {
            found.insert(key(n));
        }
        i = suffix_end.max(start + 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sheet_and_mechanics_notes_cannot_validate_their_own_numbers() {
        let d = tempfile::tempdir().unwrap();
        fs::write(
            d.path().join("mechanics.json"),
            r#"{"values":[{"value":42.75}]}"#,
        )
        .unwrap();
        fs::write(d.path().join("MECHANICS.md"), "42.75").unwrap();
        fs::write(d.path().join("notes.txt"), "42.75").unwrap();
        assert!(
            !numbers_in(d.path(), &|| Ok(()))
                .unwrap()
                .contains(&key(42.75))
        );
        fs::write(d.path().join("main.rs"), "const SPEED:f32=42.75;").unwrap();
        assert!(
            numbers_in(d.path(), &|| Ok(()))
                .unwrap()
                .contains(&key(42.75))
        );
    }
    #[test]
    fn supported_source_and_data_extensions_count_but_vendor_directories_do_not() {
        let d = tempfile::tempdir().unwrap();
        for ext in [
            "go", "java", "kt", "swift", "lua", "mjs", "cc", "glsl", "tscn", "tres", "cfg",
        ] {
            let file = d.path().join(format!("main.{ext}"));
            fs::write(&file, "speed = 42.75").unwrap();
            assert!(
                numbers_in(d.path(), &|| Ok(()))
                    .unwrap()
                    .contains(&key(42.75)),
                "{ext}"
            );
            fs::remove_file(file).unwrap();
        }
        for name in [".venv", ".godot", "__pycache__", ".next", ".turbo"] {
            fs::create_dir(d.path().join(name)).unwrap();
            fs::write(d.path().join(name).join("cached.py"), "speed = 42.75").unwrap();
        }
        assert!(
            !numbers_in(d.path(), &|| Ok(()))
                .unwrap()
                .contains(&key(42.75))
        );
    }
    #[test]
    fn omitted_relevant_source_cannot_produce_a_successful_scan() {
        let d = tempfile::tempdir().unwrap();
        fs::write(d.path().join("main.rs"), "const SPEED:f32=42.75;").unwrap();
        let oversized = fs::File::create(d.path().join("large.go")).unwrap();
        oversized.set_len(MAX_FILE + 1).unwrap();
        assert!(numbers_in(d.path(), &|| Ok(())).is_err());

        fs::remove_file(d.path().join("large.go")).unwrap();
        let mut found = HashSet::new();
        let (mut budget, mut entries) = (1, 8192);
        assert!(
            walk(d.path(), 0, &mut budget, &mut entries, &mut found, &|| Ok(
                ()
            ))
            .is_err()
        );
        let (mut budget, mut entries) = (MAX_CODE, 0);
        assert!(
            walk(d.path(), 0, &mut budget, &mut entries, &mut found, &|| Ok(
                ()
            ))
            .is_err()
        );
        let (mut budget, mut entries) = (MAX_CODE, 8192);
        assert!(
            walk(
                d.path(),
                MAX_DEPTH + 1,
                &mut budget,
                &mut entries,
                &mut found,
                &|| Ok(())
            )
            .is_err()
        );
    }
    #[test]
    fn numeric_tokens_handle_exponents_arithmetic_suffixes_and_identifiers() {
        let mut found = HashSet::new();
        literals(
            b"vec3 x2 14players 4.5f 1e-3 1E+2 1_000 .5 7+8 9-10",
            &mut found,
        );
        for n in [4.5, 0.001, 100.0, 1000.0, 0.5, 7.0, 8.0, 9.0, 10.0] {
            assert!(found.contains(&key(n)), "{n}");
        }
        assert!(!found.contains(&key(3.0)));
        assert!(!found.contains(&key(2.0)));
        assert!(!found.contains(&key(14.0)));
    }
}
