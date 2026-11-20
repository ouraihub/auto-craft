//! Reference: craft-anything crates/ai/src/rebuild/kinds/own.rs (300dba58).
//! Existing projects retain their own build and test contracts.
use super::*;
/// A project being edited in place, without an original to reconstruct.
#[derive(Debug)]
pub struct OwnKind;
impl TargetKind for OwnKind {
    fn id(&self) -> &'static str {
        "own"
    }
    fn words(&self) -> KindWords {
        AppKind.words()
    }
    fn detect(&self, _: &DetectCx) -> Option<u32> {
        None
    }
    fn default_stack(&self, _: &DetectCx) -> &'static str {
        stacks::BUILDERS_CHOICE
    }
    fn goal(&self) -> &'static str {
        "what their request asks of the project in this folder"
    }
    fn self_screenshots(&self) -> bool {
        false
    }
    fn checks(&self, _: &Workspace) -> Vec<CheckId> {
        vec![CheckId::OwnBuild, CheckId::OwnTests]
    }
}
