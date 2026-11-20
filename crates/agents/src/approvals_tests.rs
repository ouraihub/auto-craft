//! Policy verdict tests; cancellation cannot be turned into a successful engine reply.
use super::*;
use autocraft_core::{AgentAction, BuilderHooks, CancelFlag, PolicyVerdict};
struct Allow;
impl autocraft_core::ActionPolicy for Allow {
    fn decide(&self, _: &AgentAction, _: &CancelFlag) -> PolicyVerdict {
        PolicyVerdict::Allow
    }
}
#[test]
fn cancelled_or_readonly_launch_never_uses_an_allow_verdict() {
    let action = AgentAction::WriteFile {
        path: "app/main.rs".into(),
    };
    let cancel = CancelFlag::new();
    let hooks = BuilderHooks {
        policy: Arc::new(Allow),
    };
    assert_eq!(
        verdict(Some(&action), Some(&hooks), false, &cancel),
        PolicyVerdict::Allow
    );
    assert!(matches!(
        verdict(Some(&action), Some(&hooks), true, &cancel),
        PolicyVerdict::Refuse { .. }
    ));
    cancel.cancel();
    assert!(matches!(
        verdict(Some(&action), Some(&hooks), false, &cancel),
        PolicyVerdict::Refuse { .. }
    ));
    assert!(matches!(
        verdict(None, None, false, &cancel),
        PolicyVerdict::Refuse { .. }
    ));
}
