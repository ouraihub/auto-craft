//! Security contracts of generated per-launch configuration, parsed as consumers parse it.
use crate::testing::Fixture;
use autocraft_core::{BackendKind, EngineRouting, SessionRole};

#[test]
fn readonly_dsh_disables_mutators_and_never_stages_skills() {
    let fixture = Fixture::new();
    let mut request = fixture.request(BackendKind::Dsh, None, false, EngineRouting::default());
    request.role = SessionRole::ReadOnly;
    assert!(super::workflow::stage(&request).unwrap().is_none());
    let patch: Vec<serde_yaml_ng::Value> =
        serde_yaml_ng::from_str(&super::workflow::dsh_patch(&request, None).unwrap()).unwrap();
    let row = |id: &str| {
        patch
            .iter()
            .find(|row| row["id"].as_str() == Some(id))
            .unwrap()
    };
    assert_eq!(row("tool-fs")["config"]["write"].as_bool(), Some(false));
    assert_eq!(row("tool-fs")["config"]["edit"].as_bool(), Some(false));
    for id in [
        "tool-bash",
        "tool-pwsh",
        "tool-subagent",
        "tool-web",
        "tool-skill",
        "tool-workflow",
        "plan-mode",
    ] {
        assert_eq!(row(id)["disabled"].as_bool(), Some(true), "{id}");
    }
    assert!(
        !patch
            .iter()
            .any(|row| row["id"].as_str() == Some("skill-filesystem"))
    );
}

#[test]
fn readonly_pi_tools_and_persistence_are_actual_cli_restrictions() {
    let fixture = Fixture::new();
    let mut request = fixture.request(BackendKind::Pi, None, false, EngineRouting::default());
    request.role = SessionRole::ReadOnly;
    let args = crate::pi::readonly_args(&request);
    assert_eq!(
        args,
        [
            "--no-skills",
            "--tools",
            "read,grep,find,ls",
            "--no-session"
        ]
    );
    request.resume = Some("native-builder.jsonl".into());
    let args = crate::pi::readonly_args(&request);
    assert_eq!(args, ["--no-skills", "--tools", "read,grep,find,ls"]);
    assert!(super::workflow::stage(&request).unwrap().is_none());
}
