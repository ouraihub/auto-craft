//! Behavioral regressions for source feed persistence and note batch ownership.
use super::*;
struct Sink;
impl RebuildSink for Sink {
    fn publish(&self, _: RebuildEvent) {}
}
#[test]
fn deferred_delivery_does_not_claim_later_notes() {
    let temp = tempfile::tempdir().unwrap();
    let ws = Workspace::create(
        temp.path(),
        "batch",
        "request",
        ca_engine::workspace::TargetRef::None,
    )
    .unwrap();
    let project = ProjectId::from(ws.root.to_string_lossy().into_owned());
    let mut feed = Continuity::new(ws.root.clone(), project, Arc::new(Sink)).unwrap();
    let first = feed.note("first", false, vec![], None).unwrap();
    let prompt = feed.added(&ws, false, SeesPictures::No).unwrap().unwrap();
    assert!(prompt.0.contains("first"));
    let second = feed.note("second", false, vec![], None).unwrap();
    feed.delivered().unwrap();
    assert_eq!(
        feed.waiting_notes(),
        vec![(second.clone(), "second".into())]
    );
    assert_eq!(feed.withdraw(&first).unwrap(), None);
    assert_eq!(feed.withdraw(&second).unwrap(), Some("second".into()));
    assert!(feed.progress_reset());
    feed.progress_reset_saved();
    assert!(!feed.progress_reset());
}
#[test]
fn source_plan_stops_doing_but_does_not_mark_done() {
    let temp = tempfile::tempdir().unwrap();
    let ws = Workspace::create(
        temp.path(),
        "plan",
        "request",
        ca_engine::workspace::TargetRef::None,
    )
    .unwrap();
    let project = ProjectId::from(ws.root.to_string_lossy().into_owned());
    let mut feed = Continuity::new(ws.root.clone(), project.clone(), Arc::new(Sink)).unwrap();
    feed.plan(&serde_json::json!({"steps":[{"title":"Build","status":"doing"},{"title":"Read","status":"done"}]})).unwrap();
    feed.emit(&RebuildEvent::Finished {
        project,
        ok: false,
        message: "paused".into(),
        paused: true,
    })
    .unwrap();
    let saved = ca_engine::feed::load(&ws);
    let plan = saved
        .iter()
        .find_map(|item| match item {
            FeedItem::Plan(steps) => Some(steps),
            _ => None,
        })
        .expect("saved checklist must remain visible after pausing");
    assert_eq!(plan[0].status, PlanStatus::Stopped);
    assert_eq!(plan[1].status, PlanStatus::Done);
    assert!(matches!(saved.last(), Some(FeedItem::Paused)));
}
#[test]
fn pictures_only_request_keeps_copied_paths_for_builder() {
    let temp = tempfile::tempdir().unwrap();
    let ws = Workspace::create(
        temp.path(),
        "picture",
        "request",
        ca_engine::workspace::TargetRef::None,
    )
    .unwrap();
    let original = temp.path().join("original.png");
    image::DynamicImage::new_rgba8(2, 2)
        .save(&original)
        .unwrap();
    let mut feed =
        Continuity::new(ws.root.clone(), ProjectId::from("test"), Arc::new(Sink)).unwrap();
    feed.user(&RunRequest {
        message: None,
        shown: None,
        pictures: vec![original.clone()],
        continuing: false,
        backend: None,
        max_rounds: None,
    })
    .unwrap();
    let copied = feed.user_pictures();
    assert_eq!(copied.len(), 1);
    assert_ne!(copied[0], original);
    assert!(copied[0].starts_with(ws.attachments_dir()));
    assert!(
        matches!(ca_engine::feed::load(&ws).last(),Some(FeedItem::User(user)) if user.pictures()==copied)
    );
}
