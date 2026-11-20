//! Typed drafts for external engine paths and build comparison settings.
use autocraft_core::{
    AnalysisBuildSettings, AnalysisEngineSettings, AnalysisSettings, GhidraSettingsDto,
};
use gpui_kit::{App, AppContext as _, Context, Entity, Window, component::input::InputState};
use std::{collections::BTreeMap, path::PathBuf};

pub(super) const TOOLS: &[&str] = &[
    "adb",
    "jadx",
    "apktool",
    "chromium",
    "frida",
    "gdb",
    "lldb",
    "mitmdump",
    "binwalk",
    "unblob",
    "ilspy",
    "java",
    "tshark",
    "objdump",
    "rizin",
    "rtl433",
    "vineflower",
];
pub(super) const ENGINE_FIELDS: &[&str] = &[
    "ghidra_dir",
    "java_home",
    "startup_timeout",
    "language_id",
    "compiler_spec",
    "memory_gb",
    "store_dir",
];
pub(super) const BUILD_FIELDS: &[&str] = &[
    "good_enough",
    "max_rounds",
    "web_widths",
    "max_pages",
    "picture_good_enough",
    "parallel",
];

pub(super) struct Fields(pub BTreeMap<&'static str, Entity<InputState>>);
impl Fields {
    pub(super) fn new<V: 'static>(window: &mut Window, cx: &mut Context<V>) -> Self {
        Self(
            ENGINE_FIELDS
                .iter()
                .chain(BUILD_FIELDS)
                .chain(TOOLS)
                .map(|&key| (key, cx.new(|cx| InputState::new(window, cx))))
                .collect(),
        )
    }
    pub(super) fn fill(&self, settings: &AnalysisSettings, window: &mut Window, cx: &mut App) {
        let engine = &settings.engines;
        let ghidra = &engine.ghidra;
        let path = |p: &Option<PathBuf>| {
            p.as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_default()
        };
        let build = &settings.build;
        let values = [
            ("ghidra_dir", path(&ghidra.install_dir)),
            ("java_home", path(&ghidra.java_home)),
            (
                "startup_timeout",
                ghidra
                    .startup_timeout_secs
                    .map(|v| v.to_string())
                    .unwrap_or_default(),
            ),
            (
                "language_id",
                ghidra.language_id.clone().unwrap_or_default(),
            ),
            (
                "compiler_spec",
                ghidra.compiler_spec.clone().unwrap_or_default(),
            ),
            (
                "memory_gb",
                ghidra.memory_gb.map(|v| v.to_string()).unwrap_or_default(),
            ),
            ("store_dir", path(&engine.store_dir)),
            ("good_enough", build.good_enough.to_string()),
            ("max_rounds", build.max_rounds.to_string()),
            (
                "web_widths",
                build
                    .web_widths
                    .iter()
                    .map(u32::to_string)
                    .collect::<Vec<_>>()
                    .join(", "),
            ),
            ("max_pages", build.max_pages.to_string()),
            (
                "picture_good_enough",
                build
                    .picture_good_enough
                    .map(|v| v.to_string())
                    .unwrap_or_default(),
            ),
            ("parallel", build.parallel.to_string()),
        ];
        for (key, value) in values {
            self.0[key].update(cx, |input, cx| input.set_value(value, window, cx));
        }
        for &key in TOOLS {
            let value = engine
                .tools
                .get(key)
                .map(|p| p.display().to_string())
                .unwrap_or_default();
            self.0[key].update(cx, |input, cx| input.set_value(value, window, cx));
        }
    }
    pub(super) fn parse(
        &self,
        cx: &App,
    ) -> Result<(AnalysisEngineSettings, AnalysisBuildSettings), &'static str> {
        parse(|key| self.0[key].read(cx).value().trim().to_owned())
    }
}

fn parse(
    value: impl Fn(&str) -> String,
) -> Result<(AnalysisEngineSettings, AnalysisBuildSettings), &'static str> {
    let optional = |key| {
        let v = value(key);
        (!v.is_empty()).then_some(v)
    };
    let whole = |key| {
        value(key)
            .parse::<u64>()
            .map_err(|_| "app.settings.analysis.invalid")
    };
    let positive = |key| {
        whole(key).and_then(|v| {
            if v > 0 {
                Ok(v)
            } else {
                Err("app.settings.analysis.invalid")
            }
        })
    };
    let optional_positive = |key| {
        if value(key).is_empty() {
            Ok(None)
        } else {
            positive(key).map(Some)
        }
    };
    let ratio = |key| {
        value(key)
            .parse::<f64>()
            .ok()
            .filter(|v| v.is_finite() && (0.0..=100.0).contains(v))
            .ok_or("app.settings.analysis.invalid")
    };
    let widths: Vec<u32> = value("web_widths")
        .split(',')
        .map(|v| {
            v.trim()
                .parse::<u32>()
                .ok()
                .filter(|v| (320..=3840).contains(v))
                .ok_or("app.settings.analysis.invalid")
        })
        .collect::<Result<_, _>>()?;
    if widths.len() > 4 {
        return Err("app.settings.analysis.invalid");
    }
    let memory_gb = optional_positive("memory_gb")?;
    if memory_gb.is_some_and(|v| !(4..=31).contains(&v)) {
        return Err("app.settings.analysis.invalid");
    }
    let engines = AnalysisEngineSettings {
        ghidra: GhidraSettingsDto {
            install_dir: optional("ghidra_dir").map(PathBuf::from),
            java_home: optional("java_home").map(PathBuf::from),
            startup_timeout_secs: optional_positive("startup_timeout")?,
            language_id: optional("language_id"),
            compiler_spec: optional("compiler_spec"),
            memory_gb,
        },
        store_dir: optional("store_dir").map(PathBuf::from),
        tools: TOOLS
            .iter()
            .filter_map(|&key| optional(key).map(|v| (key.to_owned(), PathBuf::from(v))))
            .collect(),
    };
    let build = AnalysisBuildSettings {
        good_enough: ratio("good_enough")?,
        max_rounds: u32::try_from(whole("max_rounds")?)
            .map_err(|_| "app.settings.analysis.invalid")?,
        web_widths: widths,
        max_pages: usize::try_from(whole("max_pages")?)
            .map_err(|_| "app.settings.analysis.invalid")?,
        picture_good_enough: if value("picture_good_enough").is_empty() {
            None
        } else {
            Some(ratio("picture_good_enough")?)
        },
        parallel: usize::try_from(positive("parallel")?)
            .map_err(|_| "app.settings.analysis.invalid")?,
    };
    Ok((engines, build))
}

#[cfg(test)]
mod tests {
    use super::parse;
    fn draft(key: &str) -> String {
        match key {
            "good_enough" => "85",
            "max_rounds" => "8",
            "web_widths" => "390, 1440",
            "max_pages" => "5",
            "parallel" => "2",
            "adb" => "C:/tools/adb.exe",
            _ => "",
        }
        .into()
    }
    #[test]
    fn typed_drafts_preserve_paths_widths_and_optional_overrides() {
        let (engine, build) = parse(draft).unwrap();
        assert_eq!(
            engine.tools["adb"],
            std::path::PathBuf::from("C:/tools/adb.exe")
        );
        assert_eq!(build.web_widths, [390, 1440]);
        assert_eq!(engine.ghidra.memory_gb, None);
        assert_eq!(build.picture_good_enough, None);
        assert!(
            parse(|k| if k == "max_pages" || k == "max_rounds" {
                "0".into()
            } else {
                draft(k)
            })
            .is_ok()
        );
        for (key, bad) in [
            ("good_enough", "NaN"),
            ("parallel", "0"),
            ("max_rounds", "4294967296"),
            ("web_widths", "390,,1440"),
            ("memory_gb", "-1"),
            ("picture_good_enough", "100.1"),
        ] {
            assert!(
                parse(|k| if k == key { bad.into() } else { draft(k) }).is_err(),
                "{key}"
            );
        }
    }
}
