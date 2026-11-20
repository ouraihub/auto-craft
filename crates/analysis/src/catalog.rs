//! Engine-backed tool descriptions and host channel effect classifications.

use autocraft_core::{Availability, Result, ToolInfo};
use ca_engine::{
    Session, ToolSpec,
    tool::{Effects, Requires},
};

use crate::engines::Loaded;

pub(crate) fn tools(loaded: &Loaded) -> Result<Vec<ToolInfo>> {
    loaded.apply_settings();
    let session = loaded.engine.session();
    Ok(loaded
        .engine
        .registry()
        .all()
        .iter()
        .map(|spec| ToolInfo {
            id: spec.id.to_owned(),
            toolset: spec.toolset.id().to_owned(),
            title_key: format!("tool.{}.title", spec.id),
            title: spec.title.to_owned(),
            summary: spec.summary.to_owned(),
            effects: spec
                .effects
                .iter_names()
                .map(|(name, _)| name.to_owned())
                .collect(),
            needs_consent: spec.effects.needs_consent(),
            available: availability(loaded, &session, spec),
            input_schema: loaded.engine.registry().input_schema(spec),
        })
        .collect())
}

fn availability(loaded: &Loaded, session: &Session, spec: &ToolSpec) -> Availability {
    if loaded.engine.availability(spec, session).available {
        return Availability::Ready;
    }
    if session.clean_room() && !ca_engine::clean_room::allows(spec.id) {
        return Availability::NeedsDeep;
    }
    match spec.requires {
        Requires::Nothing => Availability::Ready,
        Requires::Target => Availability::NeedsTarget,
        Requires::Ghidra => Availability::NeedsDeep,
        Requires::Engine(engine) => Availability::Missing {
            engine: engine.to_owned(),
        },
        Requires::Os(_) => Availability::Os,
    }
}

pub(crate) fn long_tool(spec: &ToolSpec) -> bool {
    spec.effects.intersects(
        Effects::LAUNCHES_PROCESS | Effects::ACCESSES_NETWORK | Effects::EXECUTES_FOREIGN_CODE,
    ) || matches!(spec.requires, Requires::Ghidra | Requires::Engine(_))
        || matches!(
            spec.id,
            "native_function_pseudocode"
                | "native_call_graph_show"
                | "js_app_map"
                | "core_target_triage"
        )
}

pub(crate) fn read_only_forbidden(spec: &ToolSpec) -> bool {
    spec.effects.intersects(
        Effects::WRITES_FILESYSTEM
            | Effects::LAUNCHES_PROCESS
            | Effects::EXECUTES_FOREIGN_CODE
            | Effects::MUTATES_DEVICE
            | Effects::MUTATES_TARGET,
    )
}

#[cfg(test)]
mod tests {
    use autocraft_core::AnalysisMode;
    use ca_engine::Engine;

    use super::*;

    #[test]
    fn channel_effect_rules_allow_evidence_but_reject_external_mutations() {
        let directory = tempfile::tempdir().unwrap();
        let engine = Engine::new(Session::isolated(directory.path()));
        let mut spec = *engine.registry().get("core_session_show").unwrap();
        spec.id = "test_tool";
        spec.requires = Requires::Nothing;
        for effects in [
            Effects::READ_ONLY,
            Effects::EVIDENCE,
            Effects::CHANGES_UI_STATE,
            Effects::CARRIES_TARGET_CONTENT,
        ] {
            spec.effects = effects;
            assert!(!read_only_forbidden(&spec), "{effects:?}");
            assert!(!long_tool(&spec), "{effects:?}");
        }
        for effect in [
            Effects::WRITES_FILESYSTEM,
            Effects::LAUNCHES_PROCESS,
            Effects::EXECUTES_FOREIGN_CODE,
            Effects::MUTATES_DEVICE,
            Effects::MUTATES_TARGET,
        ] {
            spec.effects = Effects::EVIDENCE | effect;
            assert!(read_only_forbidden(&spec), "{effect:?}");
        }
        for effect in [
            Effects::LAUNCHES_PROCESS,
            Effects::ACCESSES_NETWORK,
            Effects::EXECUTES_FOREIGN_CODE,
        ] {
            spec.effects = Effects::EVIDENCE | effect;
            assert!(long_tool(&spec), "{effect:?}");
        }
        spec.effects = Effects::ACCESSES_NETWORK;
        assert!(!read_only_forbidden(&spec));
        spec.effects = Effects::READ_ONLY;
        for requires in [Requires::Ghidra, Requires::Engine("test-engine")] {
            spec.requires = requires;
            assert!(long_tool(&spec));
        }
        spec.requires = Requires::Nothing;
        for id in [
            "native_function_pseudocode",
            "native_call_graph_show",
            "js_app_map",
            "core_target_triage",
        ] {
            spec.id = id;
            assert!(long_tool(&spec));
        }
    }

    #[test]
    fn catalog_preserves_engine_metadata_and_cleanroom_precedence() {
        let directory = tempfile::tempdir().unwrap();
        let mut session = Session::isolated(directory.path());
        session.settings.engines.only_configured = true;
        session.deep_for_run = Some(false);
        let loaded = Loaded::new(Engine::shared(session, Vec::new()), AnalysisMode::CleanRoom);
        let catalog = tools(&loaded).unwrap();
        let show = catalog
            .iter()
            .find(|tool| tool.id == "core_session_show")
            .unwrap();
        assert_eq!(show.title_key, "tool.core_session_show.title");
        assert_eq!(show.available, Availability::Ready);
        let native = catalog
            .iter()
            .find(|tool| tool.id == "native_function_pseudocode")
            .unwrap();
        assert_eq!(native.available, Availability::NeedsDeep);
        let spec = loaded.engine.registry().get("core_session_show").unwrap();
        assert_eq!(show.input_schema, (spec.input_schema)());
        assert_eq!(show.needs_consent, spec.effects.needs_consent());
        assert_eq!(
            show.effects,
            spec.effects
                .iter_names()
                .map(|(name, _)| name.to_owned())
                .collect::<Vec<_>>()
        );
        let mut spec = *spec;
        let mut session = loaded.engine.session();
        session.deep_for_run = Some(true);
        spec.requires = Requires::Target;
        assert_eq!(
            availability(&loaded, &session, &spec),
            Availability::NeedsTarget
        );
        spec.requires = Requires::Engine("autocraft-test-missing-engine");
        assert_eq!(
            availability(&loaded, &session, &spec),
            Availability::Missing {
                engine: "autocraft-test-missing-engine".into()
            }
        );
        spec.requires = Requires::Os(&["autocraft-test-other-os"]);
        assert_eq!(availability(&loaded, &session, &spec), Availability::Os);
        spec.requires = Requires::Ghidra;
        assert_eq!(availability(&loaded, &session, &spec), Availability::Ready);
    }
}
