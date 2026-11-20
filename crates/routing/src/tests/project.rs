//! Project launch resolution against the actual settings store and router.

use super::*;
use crate::{prepare_project_route, resolve_project_route};
use autocraft_core::{ChatBinding, ChatBindingService, ProjectPrefChange, ProjectPrefService};

fn resolve(fx: &Fixture, project: &ProjectId, chat: &ChatId) -> RouteRequest {
    resolve_project_route(project, chat, &*fx.store, &*fx.store, &*fx.store).unwrap()
}

#[test]
fn two_projects_prepare_their_own_models_and_clamped_efforts() {
    let fx = fixture();
    let provider = create(&fx, input("project models", Access::Direct), &[]);
    fx.store.set_default_agent(Some(BackendKind::Pi)).unwrap();
    let global = ModelRef {
        provider: provider.id.clone(),
        model_id: "gpt-test".into(),
    };
    fx.store
        .set_default(Some(global.clone()), Some(ThinkingLevel::Low))
        .unwrap();
    let first = ProjectId::from("C:/projects/first");
    let second = ProjectId::from("C:/projects/second");
    ProjectPrefService::set(
        &*fx.store,
        &first,
        ProjectPrefChange::Model(Some(global)),
        0,
    )
    .unwrap();
    let prefs = ProjectPrefService::set(
        &*fx.store,
        &second,
        ProjectPrefChange::Model(Some(ModelRef {
            provider: provider.id.clone(),
            model_id: "claude-test".into(),
        })),
        0,
    )
    .unwrap();
    ProjectPrefService::set(
        &*fx.store,
        &second,
        ProjectPrefChange::Effort(Some(ThinkingLevel::Max)),
        prefs.revision,
    )
    .unwrap();
    let router = router(&fx, Arc::new(FakeGateway::default()));
    let one = prepare_project_route(&router, &resolve(&fx, &first, &ChatId::generate())).unwrap();
    let two = prepare_project_route(&router, &resolve(&fx, &second, &ChatId::generate())).unwrap();
    let one = one.default_model.unwrap();
    let two = two.default_model.unwrap();
    assert_eq!(
        (one.model_id.as_str(), one.effort),
        ("gpt-test", Some(ThinkingLevel::Low))
    );
    assert_eq!(
        (two.model_id.as_str(), two.effort),
        ("claude-test", Some(ThinkingLevel::High))
    );
    assert_ne!(one.route, two.route);
}

#[test]
fn binding_backend_wins_and_resume_reapplies_current_model() {
    let fx = fixture();
    let provider = create(&fx, input("resume models", Access::Direct), &[]);
    let project = ProjectId::from("C:/projects/resume");
    let chat = ChatId::generate();
    fx.store.set_default_agent(Some(BackendKind::Dsh)).unwrap();
    let prefs = ProjectPrefService::set(
        &*fx.store,
        &project,
        ProjectPrefChange::Backend(Some(BackendKind::Omp)),
        0,
    )
    .unwrap();
    ProjectPrefService::set(
        &*fx.store,
        &project,
        ProjectPrefChange::Model(Some(ModelRef {
            provider: provider.id,
            model_id: "claude-test".into(),
        })),
        prefs.revision,
    )
    .unwrap();
    assert_eq!(resolve(&fx, &project, &chat).backend, BackendKind::Omp);
    fx.store
        .bind(ChatBinding {
            project: project.clone(),
            chat: chat.clone(),
            backend: BackendKind::Pi,
            native_session: "saved-native".into(),
        })
        .unwrap();
    let request = resolve(&fx, &project, &chat);
    assert!(request.model.is_some());
    assert_eq!(
        request.model.as_ref().map(|model| model.model_id.as_str()),
        Some("claude-test")
    );
    let routing =
        prepare_project_route(&router(&fx, Arc::new(FakeGateway::default())), &request).unwrap();
    assert_eq!(
        routing
            .default_model
            .as_ref()
            .map(|model| model.model_id.as_str()),
        Some("claude-test")
    );
}

#[test]
fn no_backend_selection_is_invalid_instead_of_implicitly_using_pi() {
    let fx = fixture();
    let project = ProjectId::from("C:/projects/no-agent");
    kind(
        resolve_project_route(
            &project,
            &ChatId::generate(),
            &*fx.store,
            &*fx.store,
            &*fx.store,
        ),
        ErrorKind::Invalid,
    );
    fx.store.set_default_agent(Some(BackendKind::Dsh)).unwrap();
    let request = resolve(&fx, &project, &ChatId::generate());
    assert_eq!(request.backend, BackendKind::Dsh);
    assert_eq!(request.model, None);
    assert_eq!(request.effort, None);
    assert!(request.new_session);
}

#[test]
fn disabled_project_model_returns_model_status_reason_without_global_fallback() {
    let fx = fixture();
    let mut configured = input("disabled project provider", Access::Direct);
    let provider = create(&fx, configured.clone(), &[]);
    let model = ModelRef {
        provider: provider.id.clone(),
        model_id: "claude-test".into(),
    };
    fx.store.set_default_agent(Some(BackendKind::Pi)).unwrap();
    fx.store
        .set_default(
            Some(ModelRef {
                provider: provider.id.clone(),
                model_id: "gpt-test".into(),
            }),
            None,
        )
        .unwrap();
    let project = ProjectId::from("C:/projects/unavailable");
    ProjectPrefService::set(
        &*fx.store,
        &project,
        ProjectPrefChange::Model(Some(model.clone())),
        0,
    )
    .unwrap();
    configured.models[1].enabled = false;
    ProviderService::update(&*fx.store, &provider.id, provider.revision, configured).unwrap();
    let router = router(&fx, Arc::new(FakeGateway::default()));
    assert_eq!(
        router.model_status(&model).unwrap(),
        ModelStatus::ModelUnusable {
            model: "CLAUDE-TEST".into()
        }
    );
    let request = resolve(&fx, &project, &ChatId::generate());
    let error = prepare_project_route(&router, &request).unwrap_err();
    assert_eq!(error.kind(), Some(ErrorKind::Invalid));
    assert!(error.detail().contains("CLAUDE-TEST"));
    assert!(error.detail().contains("已停用或不是聊天模型"));
    kind(router.prepare(&request), ErrorKind::Invalid);
}

#[test]
fn official_engine_choice_is_invalid_for_pinned_project_model() {
    let fx = fixture();
    let provider = create(&fx, input("official conflict", Access::Direct), &[]);
    fx.store.set_default_agent(Some(BackendKind::Omp)).unwrap();
    fx.store
        .set_engine_provider(BackendKind::Omp, EngineProvider::Official)
        .unwrap();
    let project = ProjectId::from("C:/projects/official");
    ProjectPrefService::set(
        &*fx.store,
        &project,
        ProjectPrefChange::Model(Some(ModelRef {
            provider: provider.id,
            model_id: "gpt-test".into(),
        })),
        0,
    )
    .unwrap();
    let router = router(&fx, Arc::new(FakeGateway::default()));
    let request = resolve(&fx, &project, &ChatId::generate());
    let error = prepare_project_route(&router, &request).unwrap_err();
    assert_eq!(error.kind(), Some(ErrorKind::Invalid));
    assert!(error.detail().contains("官方配置"));
    kind(router.prepare(&request), ErrorKind::Invalid);
}

#[test]
fn hidden_saved_model_still_routes_and_global_default_routes_without_project_override() {
    let fx = fixture();
    let provider = create(&fx, input("hidden model", Access::Direct), &[]);
    let model = ModelRef {
        provider: provider.id,
        model_id: "gpt-test".into(),
    };
    fx.store.set_default_agent(Some(BackendKind::Pi)).unwrap();
    fx.store
        .set_default(Some(model.clone()), Some(ThinkingLevel::High))
        .unwrap();
    fx.store.set_hidden(&model, true).unwrap();
    let project = ProjectId::from("C:/projects/hidden");
    let router = router(&fx, Arc::new(FakeGateway::default()));
    let global_request = resolve(&fx, &project, &ChatId::generate());
    assert_eq!(global_request.model, None);
    let global = prepare_project_route(&router, &global_request)
        .unwrap()
        .default_model
        .unwrap();
    assert_eq!(global.model_id, "gpt-test");
    ProjectPrefService::set(
        &*fx.store,
        &project,
        ProjectPrefChange::Model(Some(model)),
        0,
    )
    .unwrap();
    let pinned = prepare_project_route(&router, &resolve(&fx, &project, &ChatId::generate()))
        .unwrap()
        .default_model
        .unwrap();
    assert_eq!(pinned, global);
}

#[test]
fn disabled_provider_reason_comes_from_status_and_selected_other_provider_cannot_fallback() {
    let fx = fixture();
    let mut configured = input("project provider", Access::Direct);
    let provider = create(&fx, configured.clone(), &[]);
    let other = create(&fx, input("other provider", Access::Direct), &[]);
    let model = ModelRef {
        provider: provider.id.clone(),
        model_id: "gpt-test".into(),
    };
    fx.store.set_default_agent(Some(BackendKind::Pi)).unwrap();
    let project = ProjectId::from("C:/projects/provider-choice");
    ProjectPrefService::set(
        &*fx.store,
        &project,
        ProjectPrefChange::Model(Some(model.clone())),
        0,
    )
    .unwrap();
    fx.store
        .set_engine_provider(BackendKind::Pi, EngineProvider::Provider(other.id))
        .unwrap();
    let router = router(&fx, Arc::new(FakeGateway::default()));
    let request = resolve(&fx, &project, &ChatId::generate());
    let error = prepare_project_route(&router, &request).unwrap_err();
    assert_eq!(error.kind(), Some(ErrorKind::Invalid));
    assert!(error.detail().contains("other provider"));
    configured.enabled = false;
    ProviderService::update(&*fx.store, &provider.id, provider.revision, configured).unwrap();
    assert_eq!(
        router.model_status(&model).unwrap(),
        ModelStatus::ProviderDisabled {
            provider: "project provider".into()
        }
    );
    let error = prepare_project_route(&router, &request).unwrap_err();
    assert_eq!(error.kind(), Some(ErrorKind::Invalid));
    assert!(error.detail().contains("提供方「project provider」已停用"));
}
