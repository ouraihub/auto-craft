//! The UI copy of one chat's `SessionSnapshot`, kept current from `SessionEvent`s. Revisions only
//! move forward: a late event never overwrites a newer state.

use autocraft_core::{ContentBlock, SessionEvent, SessionSnapshot};

/// Applies `event` to the chat's projection. Returns whether anything changed.
pub fn apply(current: &mut Option<SessionSnapshot>, event: SessionEvent) -> bool {
    match event {
        SessionEvent::Snapshot { snapshot } => {
            if current
                .as_ref()
                .is_some_and(|c| c.revision > snapshot.revision)
            {
                return false;
            }
            *current = Some(*snapshot);
            true
        }
        SessionEvent::Message {
            revision, message, ..
        } => {
            let Some(snapshot) = fresh(current, revision) else {
                return false;
            };
            match snapshot.messages.iter_mut().find(|m| m.id == message.id) {
                Some(existing) => *existing = message,
                None => snapshot.messages.push(message),
            }
            true
        }
        SessionEvent::Delta {
            revision,
            message_id,
            block,
            text,
            ..
        } => {
            let Some(snapshot) = fresh(current, revision) else {
                return false;
            };
            let target = snapshot
                .messages
                .iter_mut()
                .find(|m| m.id == message_id)
                .and_then(|m| m.content.get_mut(block));
            match target {
                Some(
                    ContentBlock::Text { text: existing }
                    | ContentBlock::Thinking { text: existing },
                ) => {
                    existing.push_str(&text);
                    true
                }
                _ => false,
            }
        }
        SessionEvent::Error {
            revision, error, ..
        } => {
            let Some(snapshot) = fresh(current, revision) else {
                return false;
            };
            snapshot.last_error = Some(error);
            true
        }
    }
}

/// The projection to update when `revision` is newer; bumps it.
fn fresh(current: &mut Option<SessionSnapshot>, revision: u64) -> Option<&mut SessionSnapshot> {
    let snapshot = current.as_mut()?;
    if revision < snapshot.revision {
        return None;
    }
    snapshot.revision = revision;
    Some(snapshot)
}

#[cfg(test)]
#[path = "projection_tests.rs"]
mod tests;
