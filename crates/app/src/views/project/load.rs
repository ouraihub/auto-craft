//! Generation-guarded snapshots of the authoritative rebuild feed.
//! Continuity folds native messages/deltas before publishing events; the UI only refreshes rows.
use super::ProjectView;
use crate::state::run;
use autocraft_core::{FeedEntryDto, RebuildEvent};
use gpui_kit::Context;

impl ProjectView {
    pub(super) fn select(&mut self, cx: &mut Context<Self>) {
        let selected = self.state.read(cx).project.clone();
        if self.project == selected {
            self.reload(cx);
            return;
        }
        self.generation += 1;
        self.request_cancel.cancel();
        self.project = selected;
        self.detail = None;
        self.prefs = None;
        self.status = None;
        self.rows.clear();
        self.sizes = std::rc::Rc::new(Vec::new());
        self.inbox.clear();
        self.answering.clear();
        self.plan.clear();
        self.pictures.clear();
        self.activity.clear();
        self.clear_input = true;
        self.busy = false;
        self.attaching = false;
        self.loading = false;
        self.reload_again = false;
        self.reload(cx);
    }

    pub(super) fn reload(&mut self, cx: &mut Context<Self>) {
        let Some(project) = self.project.clone() else {
            cx.notify();
            return;
        };
        if self.loading {
            self.reload_again = true;
            return;
        }
        self.loading = true;
        let generation = self.generation;
        let services = self.state.read(cx).services.clone();
        run(
            cx,
            move || {
                let detail = services.projects.open(&project)?;
                let prefs = services.project_prefs.get(&project)?;
                let inbox = services.inbox.pending(&project)?;
                let rebuild = match services.rebuild {
                    Some(rebuild) => {
                        Some((rebuild.status(&project)?, rebuild.read_feed(&project)?))
                    }
                    None => None,
                };
                Ok((detail, prefs, inbox, rebuild))
            },
            move |this, result, cx| {
                if this.generation != generation {
                    return;
                }
                this.loading = false;
                match result {
                    Ok((detail, prefs, inbox, rebuild)) => {
                        this.budget_restore = Some(detail.asset_budget_usd.to_string());
                        this.detail = Some(detail);
                        this.prefs = Some(prefs);
                        this.inbox = inbox;
                        let entries = rebuild.map(|(status, entries)| {
                            this.status = Some(status);
                            entries
                        });
                        this.update_feed(entries, cx);
                        this.load_models(cx);
                    }
                    Err(error) => this.report(&error, cx),
                }
                if this.reload_again {
                    this.reload_again = false;
                    this.reload(cx);
                }
                cx.notify();
            },
        );
    }

    /// Only the rebuild service owns feed folding; rows retain their UI state across snapshots.
    pub(super) fn update_feed(
        &mut self,
        entries: Option<Vec<FeedEntryDto>>,
        cx: &mut Context<Self>,
    ) {
        if let Some(entries) = entries {
            let mut index = 0;
            self.plan.clear();
            for entry in entries {
                if let FeedEntryDto::Plan { steps } = entry {
                    self.plan = steps;
                    continue;
                }
                if let Some(row) = self.rows.get(index) {
                    row.update(cx, |row, cx| row.set_entry(entry, cx));
                } else {
                    self.push(entry, cx);
                }
                index += 1;
            }
            self.rows.truncate(index);
            std::rc::Rc::make_mut(&mut self.sizes).truncate(index);
        }
        let missing = self
            .inbox
            .iter()
            .filter(|request| {
                !self.rows.iter().any(|row| {
                    matches!(row.read(cx).entry(), FeedEntryDto::Ask { request: shown, .. }
                        if shown.id == request.id)
                })
            })
            .cloned()
            .collect::<Vec<_>>();
        for request in missing {
            self.push(
                FeedEntryDto::Ask {
                    request,
                    answer: None,
                },
                cx,
            );
            if let Some(row) = self.rows.last() {
                row.update(cx, |row, _| row.set_analysis_source());
            }
        }
        for row in &self.rows {
            row.update(cx, |_, cx| cx.notify());
        }
    }

    /// Receives the real rebuild channel; backend work never runs during rendering.
    pub fn on_rebuild_event(&mut self, event: RebuildEvent, cx: &mut Context<Self>) {
        let project = match &event {
            RebuildEvent::Step { project, .. }
            | RebuildEvent::Agent { project, .. }
            | RebuildEvent::NoteRead { project, .. }
            | RebuildEvent::Ask { project, .. }
            | RebuildEvent::Checked { project, .. }
            | RebuildEvent::Preview { project, .. }
            | RebuildEvent::Finished { project, .. }
            | RebuildEvent::Queue { project, .. } => project,
        };
        if self.project.as_ref() != Some(project) {
            return;
        }
        // Events are invalidations, not a second feed projection. In-flight refreshes coalesce
        // through loading/reload_again, including token deltas and delayed inbox relays.
        self.reload(cx);
    }
}
