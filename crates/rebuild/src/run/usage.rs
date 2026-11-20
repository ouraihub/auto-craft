//! Reference: craft-anything crates/ai/src/rebuild/mod.rs::usage (300dba58).
//! Delta-only persistence of optional native telemetry and elapsed work.
use super::source;
use autocraft_core::{AgentSession, ErrorKind, Result, SessionUsage};
use ca_engine::workspace::{ProjectUsage, Workspace};
use std::time::Instant;
pub(super) struct Usage {
    previous: Option<SessionUsage>,
    at: Instant,
}
impl Usage {
    pub fn new(session: &dyn AgentSession) -> Result<Self> {
        Ok(Self {
            previous: session.usage_snapshot()?,
            at: Instant::now(),
        })
    }
    pub fn due(&self) -> bool {
        self.at.elapsed().as_secs() >= 60
    }
    pub fn record(&mut self, ws: &Workspace, session: &dyn AgentSession) -> Result<()> {
        let now = match session.usage_snapshot() {
            Ok(now) => now,
            // RPC telemetry requires an idle engine. Keep elapsed work and the counter baseline
            // while a turn or approval is pending; sample cumulative counters after it settles.
            Err(error) if error.kind() == Some(ErrorKind::SessionBusy) => None,
            Err(error) => return Err(error),
        };
        let elapsed = self.at.elapsed().as_millis().try_into().unwrap_or(u64::MAX);
        let previous = self.previous.as_ref();
        let input = now
            .as_ref()
            .and_then(|u| u.input_tokens)
            .map(|n| n.saturating_sub(previous.and_then(|p| p.input_tokens).unwrap_or(0)))
            .unwrap_or(0);
        let output = now
            .as_ref()
            .and_then(|u| u.output_tokens)
            .map(|n| n.saturating_sub(previous.and_then(|p| p.output_tokens).unwrap_or(0)))
            .unwrap_or(0);
        let cost = now
            .as_ref()
            .and_then(|u| u.cost_usd)
            .map(|n| (n - previous.and_then(|p| p.cost_usd).unwrap_or(0.0)).max(0.0));
        ProjectUsage::update(&ws.root, |u| {
            u.work_ms = u.work_ms.saturating_add(elapsed);
            u.add_turn(input, output, cost, true);
        })
        .map_err(source)?;
        self.accept(now);
        self.at = Instant::now();
        Ok(())
    }
    fn accept(&mut self, now: Option<SessionUsage>) {
        if let Some(now) = now {
            let previous = self.previous.get_or_insert_with(SessionUsage::default);
            if now.input_tokens.is_some() {
                previous.input_tokens = now.input_tokens;
            }
            if now.output_tokens.is_some() {
                previous.output_tokens = now.output_tokens;
            }
            if now.cost_usd.is_some() {
                previous.cost_usd = now.cost_usd;
            }
        }
    }
}
#[cfg(test)]
#[path = "usage_tests.rs"]
mod tests;
