//! Reference: craft-anything crates/ai/src/rebuild/self_check.rs (300dba5).
//! In-process self-check requests wait on a queue owned by the active rebuild run.
use super::*;
use std::sync::{Condvar, Mutex};
#[derive(Debug, Clone)]
pub struct SelfCheckRequest {
    pub id: u64,
    pub wait: Duration,
    pub taken: bool,
}
#[derive(Debug, Default)]
struct State {
    next: u64,
    pending: Vec<SelfCheckRequest>,
    answers: std::collections::HashMap<u64, String>,
    stopped: bool,
}
#[derive(Debug, Default)]
pub struct SelfCheckQueue {
    state: Mutex<State>,
    wake: Condvar,
}
impl SelfCheckQueue {
    pub fn enqueue(&self, wait: Duration, cancel: &CancelFlag) -> Result<String> {
        let id = {
            let mut s = self
                .state
                .lock()
                .map_err(|_| Error::external("self-check queue lock failed"))?;
            if s.stopped {
                return Err(Error::new(ErrorKind::SessionClosed, "rebuild run stopped"));
            }
            s.next += 1;
            let id = s.next;
            s.pending.push(SelfCheckRequest {
                id,
                wait,
                taken: false,
            });
            id
        };
        self.wake.notify_all();
        let until = Instant::now() + wait;
        let mut s = self
            .state
            .lock()
            .map_err(|_| Error::external("self-check queue lock failed"))?;
        loop {
            if cancel.is_cancelled() {
                s.pending.retain(|r| r.id != id);
                return Err(Error::new(ErrorKind::Cancelled, "self-check cancelled"));
            }
            if let Some(answer) = s.answers.remove(&id) {
                return Ok(answer);
            }
            if s.stopped {
                return Ok("The rebuild was stopped before the self-check completed.".into());
            }
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Ok("The self-check did not finish within the requested wait.".into());
            }
            let (next, _) = self
                .wake
                .wait_timeout(s, left)
                .map_err(|_| Error::external("self-check queue wait failed"))?;
            s = next;
        }
    }
    pub fn take(&self) -> Option<SelfCheckRequest> {
        let mut s = self.state.lock().ok()?;
        let pos = s.pending.iter().position(|r| !r.taken)?;
        s.pending[pos].taken = true;
        Some(s.pending.remove(pos))
    }
    pub fn answer(&self, id: u64, answer: String) -> bool {
        let Ok(mut s) = self.state.lock() else {
            return false;
        };
        s.answers.insert(id, answer);
        self.wake.notify_all();
        true
    }
    pub fn stop(&self) {
        if let Ok(mut s) = self.state.lock() {
            s.stopped = true;
        }
        self.wake.notify_all();
    }
    pub fn pending(&self) -> usize {
        self.state.lock().map(|s| s.pending.len()).unwrap_or(0)
    }
}
/// Render the check report sent back to the builder's self-check request.
pub fn report(
    checks: &[CheckResult],
    good_enough: f64,
    open: &[autocraft_core::UnknownDto],
) -> String {
    if checks.is_empty() {
        return "The checks did not run.".into();
    }
    let passed = checks.iter().filter(|c| c.good(good_enough)).count();
    let mut text = format!("{} of {} checks passed.", passed, checks.len());
    for check in checks.iter().filter(|c| !c.good(good_enough)) {
        text.push_str(&format!(" {}: {}", check.check, check.note));
    }
    if !open.is_empty() {
        text.push_str(&format!(
            " {} unresolved analysis unknowns remain.",
            open.len()
        ));
    }
    text
}
