//! Reference: craft-anything crates/ai/src/rebuild/kept_check.rs (300dba58).
//! Check fingerprints use the engine's content scanner and include comparison inputs.
use super::*;
use ca_engine::{
    CancelToken,
    workspace::progress::{AppFingerprintCache, BuildProgress, CheckedBuild},
};
#[derive(Default)]
pub struct KeptChecks {
    fingerprint: Option<String>,
    results: Vec<CheckResult>,
    kind: Option<String>,
    deep: bool,
    plugin_ids: Vec<String>,
    cache: AppFingerprintCache,
}
impl std::fmt::Debug for KeptChecks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeptChecks")
            .field("has_fingerprint", &self.fingerprint.is_some())
            .field("result_count", &self.results.len())
            .field("kind", &self.kind)
            .field("deep", &self.deep)
            .finish()
    }
}
impl KeptChecks {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn from_saved(progress: &BuildProgress, ws: &Workspace, kind_id: &str, deep: bool) -> Self {
        let kept = progress.checked.as_ref().and_then(|c| {
            let last = ws.manifest.checks.last()?.round;
            let results: Vec<_> = ws
                .manifest
                .checks
                .iter()
                .filter(|r| r.round == c.round)
                .cloned()
                .collect();
            (last == c.round && all_worked(&results)).then_some((c.key.clone(), results))
        });
        let (fingerprint, results) =
            kept.map_or((None, Vec::new()), |(key, values)| (Some(key), values));
        Self {
            fingerprint,
            results,
            kind: Some(kind_id.to_owned()),
            deep,
            plugin_ids: Vec::new(),
            cache: AppFingerprintCache::default(),
        }
    }
    pub fn saved_for(&self, round: u32) -> Option<CheckedBuild> {
        self.fingerprint
            .as_ref()
            .filter(|_| self.results.first().is_some_and(|r| r.round == round))
            .map(|key| CheckedBuild {
                key: key.clone(),
                round,
            })
    }
    pub fn reuse(&mut self, ws: &Workspace, round: u32) -> Option<Vec<CheckResult>> {
        let key = self.current_key(ws)?;
        if self.fingerprint.as_deref() != Some(&key) {
            self.fingerprint = None;
            self.results.clear();
            return None;
        }
        Some(
            self.results
                .iter()
                .cloned()
                .map(|mut result| {
                    result.round = round;
                    result
                })
                .collect(),
        )
    }
    pub fn remember(&mut self, ws: &Workspace, results: &[CheckResult]) {
        if all_worked(results) {
            self.fingerprint = self.current_key(ws);
            self.results = results.to_vec();
        } else {
            self.fingerprint = None;
            self.results.clear();
        }
    }
    pub fn remember_for(
        &mut self,
        ws: &Workspace,
        kind_id: &str,
        deep: bool,
        results: &[CheckResult],
    ) {
        self.kind = Some(kind_id.to_owned());
        self.deep = deep;
        self.remember(ws, results);
    }
    /// Include active analysis plugin ids so removing a check cannot reuse old results.
    /// The saved fingerprint already includes this set; compare it during reuse before invalidating.
    pub fn set_plugin_ids(&mut self, ids: impl IntoIterator<Item = String>) {
        let mut next: Vec<_> = ids.into_iter().collect();
        next.sort_unstable();
        next.dedup();
        self.plugin_ids = next;
    }
    fn current_key(&mut self, ws: &Workspace) -> Option<String> {
        let kind = self.kind.as_deref()?;
        let mode = format!("deep={}", self.deep);
        let extra = [
            ws.root.to_string_lossy().to_string(),
            kind.to_owned(),
            self.plugin_ids.join(","),
            ws.manifest.stack.clone(),
            mode,
        ];
        let refs: Vec<&str> = extra.iter().map(String::as_str).collect();
        ws.check_fingerprint(&mut self.cache, &refs, &CancelToken::default())
            .ok()
    }
}
fn all_worked(results: &[CheckResult]) -> bool {
    !results.is_empty() && results.iter().all(|r| r.passed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ca_engine::workspace::TargetRef;

    #[test]
    fn restored_results_reuse_only_the_same_normalized_plugin_set() {
        let root = tempfile::tempdir().unwrap();
        let mut ws = Workspace::create(root.path(), "cache", "build", TargetRef::None).unwrap();
        std::fs::create_dir_all(ws.app_dir()).unwrap();
        std::fs::write(ws.app_dir().join("main.cs"), "class Main {}").unwrap();
        let results = vec![CheckResult::new("plugin_judge", 1, true, "Scripts passed")];
        ws.manifest.checks = results.clone();
        let mut kept = KeptChecks::new();
        kept.set_plugin_ids(["plugin_judge".into(), "plugin_other".into()]);
        kept.remember_for(&ws, "game", false, &results);
        let progress = BuildProgress {
            checked: kept.saved_for(1),
            ..BuildProgress::default()
        };
        assert!(progress.checked.is_some());

        let mut restored = KeptChecks::from_saved(&progress, &ws, "game", false);
        restored.set_plugin_ids([
            "plugin_other".into(),
            "plugin_judge".into(),
            "plugin_judge".into(),
        ]);
        let reused = restored.reuse(&ws, 2).unwrap();
        assert_eq!(reused[0].check, "plugin_judge");
        assert_eq!(reused[0].round, 2);
        assert!(reused[0].passed);

        for ids in [
            vec![],
            vec!["plugin_judge".to_owned()],
            vec!["new_judge".to_owned()],
        ] {
            let mut restored = KeptChecks::from_saved(&progress, &ws, "game", false);
            restored.set_plugin_ids(ids);
            assert!(restored.reuse(&ws, 2).is_none());
            assert!(restored.saved_for(1).is_none());
        }

        restored.set_plugin_ids(["plugin_judge".into()]);
        assert!(restored.reuse(&ws, 3).is_none());
    }
}
