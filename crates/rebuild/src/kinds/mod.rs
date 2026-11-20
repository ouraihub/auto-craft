//! Reference: craft-anything crates/ai/src/rebuild/kinds/mod.rs (300dba58).
//! Target registry, score ordering and ordered check contracts.
mod app;
pub(crate) mod contracts;
mod game;
mod local;
mod own;
pub mod website;
use super::*;
pub use app::{AppKind, RUNS_BRIEF};
use contracts::*;
pub use game::GameKind;
use local::{captured, game_folder};
pub(crate) use local::{is_console, locate};
pub use own::OwnKind;
use serde_json::Value;
use std::sync::Arc;
pub use website::WebsiteKind;

/// Whether the selected model can inspect image content.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeesPictures {
    Yes,
    No,
    Unknown,
}

/// Ordered check identifiers; execution belongs to the check runner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckId {
    Build,
    WebVisual,
    Runs,
    AppScreenshot,
    SelfTest,
    Mechanics,
    DeepMechanics,
    DeepCredits,
    OwnBuild,
    OwnTests,
}

/// What a kind is recognised from: the person's words, the target and the stack (when chosen).
pub struct DetectCx<'a> {
    /// Lower-case request padded with spaces (`" clone this website "`), for phrase matching.
    pub request: String,
    /// The request as whole lower-case words padded with spaces, punctuation removed.
    pub words: String,
    pub target: &'a TargetRef,
    /// The project's stack, or empty before one is chosen.
    pub stack: &'a str,
    /// Deep mode: the kind may look inside a local target (its files) to pick a stack.
    pub deep: bool,
}

impl std::fmt::Debug for DetectCx<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DetectCx")
            .field("deep", &self.deep)
            .finish_non_exhaustive()
    }
}

impl<'a> DetectCx<'a> {
    pub fn new(request: &str, target: &'a TargetRef, stack: &'a str) -> DetectCx<'a> {
        let words = stacks::words(request);
        let request = format!(" {} ", request.to_lowercase());
        DetectCx {
            request,
            words,
            target,
            stack,
            deep: false,
        }
    }

    /// Any of these phrases appears in the request.
    pub fn says(&self, phrases: &[&str]) -> bool {
        phrases.iter().any(|p| self.request.contains(p))
    }
}

/// Estonian website words: *veebileht*, *koduleht* and *veebisait* with their inflected forms.
/// *veebilehitseja* (a browser) is not a website.
fn is_site_word(w: &str) -> bool {
    (w.starts_with("veebileh") || w.starts_with("koduleh") || w.starts_with("veebisai"))
        && !w.starts_with("veebilehitseja")
}

/// Whether an Estonian word is in a case that marks a complement (source, means, place or topic)
/// rather than the head of a noun phrase: elative (-st), comitative (-ga), adessive (-l),
/// allative (-le), ablative (-lt), inessive (-s), illative (-sse), translative (-ks),
/// terminative (-ni), abessive (-ta) or essive (-na). The longer endings come first.
fn is_complement_case(w: &str) -> bool {
    const ENDINGS: &[&str] = &[
        "sse", "st", "ga", "le", "lt", "ks", "ni", "ta", "na", "l", "s",
    ];
    ENDINGS.iter().any(|e| w.ends_with(e))
}

/// What the brief is written from.
pub struct BriefCx<'a> {
    pub ws: &'a Workspace,
    /// Deep mode: decompiling the target is allowed (and required for programs).
    pub deep: bool,
    /// Ghidra is installed, so decompiling uses it.
    pub ghidra: bool,
    /// The game engine's editor for engine stacks.
    pub editor: Option<&'a Path>,
    /// An asset provider's key is set up; assets are offered only if the project also allows them
    /// (`Manifest::generate_assets`).
    pub assets: bool,
    /// The picture model chosen in Settings (an asset-table id); `None` = the table's default.
    pub image_model: Option<&'a str>,
    /// The builder can look at pictures (an agent app opens them with its Read tool). One that
    /// can't (the API loop) is never sent to open a screenshot; it's told what to use instead.
    pub sees_pictures: SeesPictures,
}

impl std::fmt::Debug for BriefCx<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BriefCx")
            .field("deep", &self.deep)
            .field("ghidra", &self.ghidra)
            .field("assets", &self.assets)
            .field("sees_pictures", &self.sees_pictures)
            .finish_non_exhaustive()
    }
}

impl<'a> BriefCx<'a> {
    /// Build prompt context from the shared analysis settings and selected builder capability.
    pub fn new(
        ws: &'a Workspace,
        settings: &'a ca_engine::Settings,
        deep: bool,
        editor: Option<&'a Path>,
        assets: bool,
        sees_pictures: SeesPictures,
    ) -> Self {
        Self {
            ws,
            deep,
            ghidra: settings.ghidra_launcher().is_some(),
            editor,
            assets,
            image_model: settings.assets.image_model.as_deref(),
            sees_pictures,
        }
    }
}

/// The words the app uses for a kind's result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KindWords {
    pub button: &'static str,
    pub icon: &'static str,
    pub ready: &'static str,
    pub change_hint: &'static str,
}

/// A kind of thing autocraft can make.
pub trait TargetKind: Send + Sync {
    /// Stable id, stored with the project: `"website"`, `"app"`, `"game"`.
    fn id(&self) -> &'static str;
    /// How the app talks about the result.
    fn words(&self) -> KindWords;
    /// How strongly the request, target and stack say "this kind" (higher wins); `None` if not.
    fn detect(&self, cx: &DetectCx) -> Option<u32>;
    /// The stack used when the person names none.
    fn default_stack(&self, cx: &DetectCx) -> &'static str;
    /// A stack the request names that only this kind understands ("in Godot" for a game).
    fn named_stack(&self, _cx: &DetectCx) -> Option<&'static str> {
        None
    }
    /// For a folder target: also study the folder around the program (a game's content).
    fn studies_folder(&self) -> bool {
        false
    }
    /// "What to build" in the brief.
    fn goal(&self) -> &'static str;
    /// Extra brief sections after the shared steps and rules.
    fn brief_sections(&self, _cx: &BriefCx) -> Vec<String> {
        Vec::new()
    }
    /// Sections added to a follow-up message (a builder that resumes its session sees only that),
    /// for what may have changed since the brief: the project's settings as they are now.
    fn follow_up_sections(&self, _cx: &BriefCx) -> Vec<String> {
        Vec::new()
    }
    /// The result can't be photographed from outside, so the builder screenshots its own.
    fn self_screenshots(&self) -> bool {
        true
    }
    /// How a follow-up tells the builder to rebuild.
    fn rebuild_words(&self) -> &'static str {
        "build it again"
    }
    /// The checks that judge a build of this kind for this project, in order.
    fn checks(&self, ws: &Workspace) -> Vec<CheckId>;
    /// The next round's message after a check that wasn't good enough, for a builder that can
    /// look at pictures or one that can't ([`BriefCx::sees_pictures`]).
    fn refine(&self, check: &CheckResult, _sees_pictures: SeesPictures) -> String {
        format!(
            "Check {}: {}\n\nFix this in app/, build again until it succeeds, and reply with 1-2 plain sentences on what you changed.",
            check.round, check.note
        )
    }
}
/// The kinds a run can choose from.
#[derive(Clone)]
pub struct KindRegistry {
    kinds: Vec<Arc<dyn TargetKind>>,
}

impl std::fmt::Debug for KindRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list()
            .entries(self.kinds.iter().map(|k| k.id()))
            .finish()
    }
}

impl Default for KindRegistry {
    fn default() -> Self {
        KindRegistry::built_in()
    }
}

impl KindRegistry {
    /// Detect an existing project without mistaking its requested change for a new target.
    pub fn for_workspace(&self, ws: &Workspace) -> Option<Arc<dyn TargetKind>> {
        if ws.manifest.own_project() {
            self.get("own")
        } else {
            self.detect(
                &ws.manifest.request,
                &ws.manifest.target,
                &ws.manifest.stack,
            )
        }
    }
    /// Website, game and app.
    pub fn built_in() -> KindRegistry {
        KindRegistry {
            kinds: vec![
                Arc::new(OwnKind),
                Arc::new(WebsiteKind),
                Arc::new(GameKind),
                Arc::new(AppKind),
            ],
        }
    }

    /// Add a kind, or replace the one with the same id.
    pub fn register(&mut self, kind: Arc<dyn TargetKind>) {
        self.kinds.retain(|k| k.id() != kind.id());
        self.kinds.push(kind);
    }

    pub fn get(&self, id: &str) -> Option<Arc<dyn TargetKind>> {
        self.kinds.iter().find(|k| k.id() == id).cloned()
    }

    pub fn ids(&self) -> Vec<&'static str> {
        self.kinds.iter().map(|k| k.id()).collect()
    }

    /// The kind that claims the project most strongly; ties go to the one registered first.
    pub fn detect(
        &self,
        request: &str,
        target: &TargetRef,
        stack: &str,
    ) -> Option<Arc<dyn TargetKind>> {
        let cx = DetectCx::new(request, target, stack);
        let mut best: Option<(u32, &Arc<dyn TargetKind>)> = None;
        for k in &self.kinds {
            if let Some(score) = k.detect(&cx)
                && best.is_none_or(|(b, _)| score > b)
            {
                best = Some((score, k));
            }
        }
        best.map(|(_, k)| k.clone())
    }

    /// The kind and stack for a new project: a stack the person names wins, else the kind's. In
    /// Deep mode (`deep`) the kind's default may come from looking inside the target.
    pub fn choose(
        &self,
        request: &str,
        target: &TargetRef,
        deep: bool,
    ) -> Option<(Arc<dyn TargetKind>, &'static str)> {
        let kind = self.detect(request, target, "")?;
        let cx = DetectCx {
            deep,
            ..DetectCx::new(request, target, "")
        };
        let stack = kind
            .named_stack(&cx)
            .or_else(|| named_stack(&cx))
            .unwrap_or_else(|| kind.default_stack(&cx));
        Some((kind, stack))
    }

    /// The stack the request names, if it names one any kind knows ("in Godot", "use Electron").
    pub fn named(&self, request: &str, target: &TargetRef) -> Option<&'static str> {
        let cx = DetectCx::new(request, target, "");
        self.kinds
            .iter()
            .find_map(|k| k.named_stack(&cx))
            .or_else(|| named_stack(&cx))
    }
}

/// Stacks any kind can be asked for by name (`asked_by` in `stacks.toml`).
fn named_stack(cx: &DetectCx) -> Option<&'static str> {
    stacks::asked_for(&cx.words).map(|s| s.name.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detection_preserves_scores_and_multilingual_disambiguation() {
        let registry = KindRegistry::built_in();
        for (request, expected) in [
            ("a music player", "app"),
            ("a single player app for music", "app"),
            ("a 2-player puzzle", "game"),
            ("a low-level editor", "app"),
            ("Tee sellest mängust veebileht", "website"),
            ("Loo mäng nimega Veebileht", "game"),
            ("un juego de herramientas", "app"),
            ("做一个网站", "website"),
            ("做一个游戏", "game"),
        ] {
            assert_eq!(
                registry.detect(request, &TargetRef::None, "").unwrap().id(),
                expected,
                "{request}"
            );
        }
        let site = TargetRef::Website {
            url: "https://example.test".into(),
        };
        assert_eq!(
            registry.detect("a game", &site, "").unwrap().id(),
            "website"
        );
        assert_eq!(
            registry.detect("a website", &site, "Godot").unwrap().id(),
            "game"
        );
    }

    #[test]
    fn explicit_stack_overrides_kind_default() {
        let registry = KindRegistry::built_in();
        for (request, expected) in [
            ("a game with Godot", "Godot"),
            ("a website using Next.js", "Next.js + TypeScript"),
            ("a tool in Python", "Python"),
            ("a desktop game using Python tkinter", "Python"),
            ("a counter app: use Python", "Python"),
            ("a game using Pythonista", "Unity"),
            ("a website", stacks::BUILDERS_CHOICE),
        ] {
            assert_eq!(
                registry.choose(request, &TargetRef::None, false).unwrap().1,
                expected
            );
        }
    }
}
