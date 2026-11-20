//! Missing telemetry must not reset the cumulative-counter baseline.
use super::*;
#[test]
fn missing_snapshot_preserves_previous_counters() {
    let mut usage = Usage {
        previous: Some(SessionUsage {
            input_tokens: Some(100),
            output_tokens: Some(20),
            cost_usd: Some(0.3),
            ..SessionUsage::default()
        }),
        at: Instant::now(),
    };
    usage.accept(None);
    assert_eq!(
        usage.previous.as_ref().and_then(|u| u.input_tokens),
        Some(100)
    );
    usage.accept(Some(SessionUsage {
        output_tokens: Some(25),
        ..SessionUsage::default()
    }));
    assert_eq!(
        usage.previous.as_ref().and_then(|u| u.input_tokens),
        Some(100)
    );
    assert_eq!(
        usage.previous.as_ref().and_then(|u| u.output_tokens),
        Some(25)
    );
    assert_eq!(usage.previous.as_ref().and_then(|u| u.cost_usd), Some(0.3));
}

#[test]
fn approval_wait_records_time_and_defers_counters_until_idle() {
    use autocraft_core::*;
    use std::sync::Mutex;
    struct Telemetry(Mutex<Vec<Result<Option<SessionUsage>>>>);
    impl AgentSession for Telemetry {
        fn snapshot(&self) -> SessionSnapshot {
            panic!("not used by telemetry")
        }
        fn usage_snapshot(&self) -> Result<Option<SessionUsage>> {
            self.0.lock().unwrap().remove(0)
        }
        fn prompt(&self, _: &str) -> Result<()> {
            unreachable!()
        }
        fn stop(&self) -> Result<()> {
            unreachable!()
        }
        fn respond(&self, _: &str, _: InteractionAnswer) -> Result<()> {
            unreachable!()
        }
        fn dispose(&self) -> Result<()> {
            unreachable!()
        }
        fn set_model(&self, _: &str, _: &str, _: Option<ThinkingLevel>) -> Result<()> {
            unreachable!()
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let ws = Workspace::create(
        dir.path(),
        "Usage wait",
        "Make a website",
        ca_engine::workspace::TargetRef::None,
    )
    .unwrap();
    let sample = |input, output, cost| {
        Some(SessionUsage {
            input_tokens: Some(input),
            output_tokens: Some(output),
            cost_usd: Some(cost),
            ..SessionUsage::default()
        })
    };
    let session = Telemetry(Mutex::new(vec![
        Ok(sample(100, 20, 0.3)),
        Err(Error::new(ErrorKind::SessionBusy, "waiting approval")),
        Ok(sample(150, 30, 0.4)),
        Err(Error::external("telemetry failed")),
    ]));
    let mut usage = Usage::new(&session).unwrap();
    usage.at = Instant::now() - std::time::Duration::from_secs(61);
    usage.record(&ws, &session).unwrap();
    let waiting: serde_json::Value =
        serde_json::from_slice(&std::fs::read(ws.root.join("usage.json")).unwrap()).unwrap();
    assert!(waiting["work_ms"].as_u64().unwrap() >= 61_000);
    assert_eq!(waiting["input_tokens"], 0);
    usage.record(&ws, &session).unwrap();
    let settled: serde_json::Value =
        serde_json::from_slice(&std::fs::read(ws.root.join("usage.json")).unwrap()).unwrap();
    assert_eq!(settled["input_tokens"], 50);
    assert_eq!(settled["output_tokens"], 10);
    assert_eq!(
        usage.record(&ws, &session).unwrap_err().kind(),
        Some(ErrorKind::External)
    );
}
