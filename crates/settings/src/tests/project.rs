//! Project persistence, revision and secret boundaries.

use super::*;
use autocraft_core::{
    AnalysisMode, AssetKeyService, BackendKind, BuilderChatService, ChatBinding,
    ChatBindingService, ChatId, MainViewPref, NativeSessionKey, ProjectPrefChange,
    ProjectPrefService,
};

#[test]
fn project_preferences_survive_reopen_and_reject_stale_changes() {
    let directory = TempDir::new().unwrap();
    let store = open(&directory);
    let project = ProjectId::from("C:/projects/one");
    assert_eq!(
        ProjectPrefService::get(&store, &project).unwrap().revision,
        0
    );
    let prefs = ProjectPrefService::set(
        &store,
        &project,
        ProjectPrefChange::Backend(Some(BackendKind::Omp)),
        0,
    )
    .unwrap();
    assert_eq!(prefs.backend, Some(BackendKind::Omp));
    kind(
        ProjectPrefService::set(
            &store,
            &project,
            ProjectPrefChange::Mode(AnalysisMode::Deep),
            0,
        ),
        ErrorKind::Conflict,
    );
    assert_eq!(ProjectPrefService::get(&store, &project).unwrap(), prefs);
    let prefs = ProjectPrefService::set(
        &store,
        &project,
        ProjectPrefChange::Mode(AnalysisMode::Deep),
        1,
    )
    .unwrap();
    let prefs = ProjectPrefService::set(
        &store,
        &project,
        ProjectPrefChange::View(Some(MainViewPref::Workbench)),
        prefs.revision,
    )
    .unwrap();
    let prefs = ProjectPrefService::set(
        &store,
        &project,
        ProjectPrefChange::Effort(Some(ThinkingLevel::High)),
        prefs.revision,
    )
    .unwrap();
    drop(store);
    let store = open(&directory);
    assert_eq!(ProjectPrefService::get(&store, &project).unwrap(), prefs);
    let cleared = ProjectPrefService::set(
        &store,
        &project,
        ProjectPrefChange::Backend(None),
        prefs.revision,
    )
    .unwrap();
    assert_eq!(cleared.backend, None);
    assert_eq!(cleared.effort, Some(ThinkingLevel::High));
}

#[test]
fn project_copy_does_not_copy_sessions_and_delete_cleans_local_rows() {
    let directory = TempDir::new().unwrap();
    let store = open(&directory);
    let source = ProjectId::from("C:/projects/source");
    let copy = ProjectId::from("C:/projects/copy");
    let prefs = ProjectPrefService::set(
        &store,
        &source,
        ProjectPrefChange::Mode(AnalysisMode::Deep),
        0,
    )
    .unwrap();
    let builder = store.builder_chat(&source, BackendKind::Pi).unwrap();
    store
        .bind(ChatBinding {
            project: source.clone(),
            chat: builder.chat.clone(),
            backend: BackendKind::Pi,
            native_session: "native-source".into(),
        })
        .unwrap();
    ProjectPrefService::copy(&store, &source, &copy).unwrap();
    assert_eq!(ProjectPrefService::get(&store, &copy).unwrap(), prefs);
    assert!(store.bindings(&copy).unwrap().is_empty());
    let copied_builder = store.builder_chat(&copy, BackendKind::Pi).unwrap();
    assert_ne!(copied_builder.chat, builder.chat);
    kind(
        ProjectPrefService::copy(&store, &source, &copy),
        ErrorKind::Conflict,
    );
    ProjectPrefService::remove(&store, &source).unwrap();
    assert_eq!(
        ProjectPrefService::get(&store, &source).unwrap().revision,
        0
    );
    assert!(store.bindings(&source).unwrap().is_empty());
    assert_ne!(
        store.builder_chat(&source, BackendKind::Pi).unwrap().chat,
        builder.chat
    );
    assert_eq!(
        ProjectPrefService::get(&store, &copy).unwrap().mode,
        AnalysisMode::Deep
    );
}

#[test]
fn deleting_provider_clears_both_project_model_columns_and_invalidates_revision() {
    let directory = TempDir::new().unwrap();
    let store = open(&directory);
    let provider = create(&store, input("relay")).unwrap();
    let project = ProjectId::from("C:/projects/model");
    let model = ModelRef {
        provider: provider.id.clone(),
        model_id: "gpt-test".into(),
    };
    let prefs = ProjectPrefService::set(&store, &project, ProjectPrefChange::Model(Some(model)), 0)
        .unwrap();
    ProviderService::delete(&store, &provider.id, provider.revision).unwrap();
    let current = ProjectPrefService::get(&store, &project).unwrap();
    assert_eq!(current.model, None);
    assert_eq!(current.revision, prefs.revision + 1);
    assert_eq!(store.read(|conn| conn.int("SELECT COUNT(*) FROM project_prefs WHERE project_id = ?1 AND model_provider IS NULL AND model_id IS NULL", (project.as_str(),))).unwrap(), 1);
    kind(
        ProjectPrefService::set(
            &store,
            &project,
            ProjectPrefChange::Mode(AnalysisMode::Deep),
            prefs.revision,
        ),
        ErrorKind::Conflict,
    );
}

#[test]
fn asset_keys_are_masked_revision_checked_and_explicitly_readable() {
    let directory = TempDir::new().unwrap();
    let store = open(&directory);
    let summary = store
        .set_asset_key("fal", Secret::new("sk-private-value-1234"), 0)
        .unwrap();
    assert_eq!(summary.masked_key, "sk****1234");
    assert_eq!(summary.revision, 1);
    assert!(!format!("{summary:?}").contains("private-value"));
    kind(
        store.set_asset_key("fal", Secret::new("replacement"), 0),
        ErrorKind::Conflict,
    );
    assert_eq!(
        store.asset_key("fal").unwrap().expose(),
        "sk-private-value-1234"
    );
    drop(store);
    let store = open(&directory);
    assert_eq!(store.asset_key_summaries().unwrap(), vec![summary]);
    let keys = store.asset_keys().unwrap();
    assert_eq!(keys[0].0, "fal");
    assert_eq!(keys[0].1.expose(), "sk-private-value-1234");
    let summary = store.set_asset_key("fal", Secret::new("short"), 1).unwrap();
    assert_eq!(summary.masked_key, "••••••••");
    kind(store.remove_asset_key("fal", 1), ErrorKind::Conflict);
    store.remove_asset_key("fal", 2).unwrap();
    assert!(store.asset_keys().unwrap().is_empty());
    kind(store.asset_key("fal"), ErrorKind::NotFound);
    kind(
        store.set_asset_key("fal", Secret::new(" "), 0),
        ErrorKind::Invalid,
    );
}

#[test]
fn builder_chat_is_stable_but_backend_cutover_cannot_resume_old_native_session() {
    let directory = TempDir::new().unwrap();
    let store = open(&directory);
    let project = ProjectId::from("C:/projects/builder");
    assert_eq!(store.find_builder_chat(&project).unwrap(), None);
    assert_eq!(
        store
            .read(|conn| conn.int("SELECT COUNT(*) FROM builder_chats", ()))
            .unwrap(),
        0
    );
    let builder = store.builder_chat(&project, BackendKind::Pi).unwrap();
    let binding = ChatBinding {
        project: project.clone(),
        chat: builder.chat.clone(),
        backend: BackendKind::Pi,
        native_session: NativeSessionKey::from("native-pi"),
    };
    store.bind(binding.clone()).unwrap();
    drop(store);
    let store = open(&directory);
    assert_eq!(
        store.builder_chat(&project, BackendKind::Pi).unwrap(),
        builder
    );
    assert_eq!(
        store.binding(&project, &builder.chat).unwrap(),
        Some(binding.clone())
    );
    assert_eq!(
        store.find_builder_chat(&project).unwrap(),
        Some(builder.clone())
    );
    assert_eq!(
        store.binding(&project, &builder.chat).unwrap(),
        Some(binding.clone())
    );
    let changed = store.builder_chat(&project, BackendKind::Dsh).unwrap();
    assert_eq!(changed.chat, builder.chat);
    assert_eq!(changed.backend, BackendKind::Dsh);
    assert_eq!(store.find_builder_chat(&project).unwrap(), Some(changed));
    assert_eq!(store.binding(&project, &builder.chat).unwrap(), None);
    kind(store.bind(binding), ErrorKind::Conflict);
    store
        .bind(ChatBinding {
            project: project.clone(),
            chat: builder.chat.clone(),
            backend: BackendKind::Dsh,
            native_session: "native-dsh".into(),
        })
        .unwrap();
    kind(
        store.binding(&ProjectId::from("C:/other"), &builder.chat),
        ErrorKind::Invalid,
    );
    assert_eq!(
        store.bindings(&project).unwrap()[0].native_session.as_str(),
        "native-dsh"
    );
    store.remove_binding(&project, &builder.chat).unwrap();
    assert!(store.bindings(&project).unwrap().is_empty());
}

#[test]
fn ordinary_chat_bindings_cannot_move_projects_or_backends() {
    let directory = TempDir::new().unwrap();
    let store = open(&directory);
    let project = ProjectId::from("C:/projects/chat");
    let chat = ChatId::generate();
    let binding = ChatBinding {
        project: project.clone(),
        chat: chat.clone(),
        backend: BackendKind::Pi,
        native_session: "native-one".into(),
    };
    store.bind(binding.clone()).unwrap();
    let mut other = binding.clone();
    other.project = ProjectId::from("C:/projects/other");
    kind(store.bind(other), ErrorKind::Conflict);
    let mut other = binding.clone();
    other.backend = BackendKind::Omp;
    kind(store.bind(other), ErrorKind::Conflict);
    let mut updated = binding;
    updated.native_session = "native-two".into();
    store.bind(updated.clone()).unwrap();
    assert_eq!(store.binding(&project, &chat).unwrap(), Some(updated));
}

#[test]
fn shipped_schema_six_is_refused_without_changing_database_bytes() {
    let directory = TempDir::new().unwrap();
    let store = open(&directory);
    store
        .write(|conn| conn.execute("UPDATE schema_meta SET version = 6", ()))
        .unwrap();
    drop(store);
    let data = directory.path().join("data");
    let before = snapshot(&data);
    kind(
        SqliteSettingsStore::open(&data.join(DB)),
        ErrorKind::Corrupt,
    );
    assert_eq!(snapshot(&data), before);
}
