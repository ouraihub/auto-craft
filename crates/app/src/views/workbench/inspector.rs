//! Inspector, evidence ledger and severity-ordered unknowns.
use super::{RightTab, WorkbenchView};
use autocraft_core::{Severity, UnknownStatus};
use autocraft_ui::components::{ConfidenceLabels, EvidenceCard, EvidenceCardProps, JsonTree};
use gpui_kit::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _, Styled as _,
    base::{h_flex, v_flex},
    component::{button::Button, scroll::ScrollableElement as _},
};
use std::sync::Arc;
pub(super) fn confidence_labels() -> ConfidenceLabels {
    ConfidenceLabels {
        observed: rust_i18n::t!("app.workbench.observed").into_owned().into(),
        derived: rust_i18n::t!("app.workbench.derived").into_owned().into(),
        inferred: rust_i18n::t!("app.workbench.inferred").into_owned().into(),
        unknown: rust_i18n::t!("app.workbench.unknown").into_owned().into(),
    }
}
impl WorkbenchView {
    pub(super) fn json(&self, value: serde_json::Value, cx: &mut Context<Self>) -> AnyElement {
        let entity = cx.entity();
        JsonTree::new("workbench-json", Arc::new(value), self.expanded.clone())
            .on_expand(move |path, open, _, cx| {
                entity.update(cx, |this, cx| {
                    let set = Arc::make_mut(&mut this.expanded);
                    if open {
                        set.insert(path.to_owned());
                    } else {
                        set.remove(path);
                    }
                    cx.notify();
                })
            })
            .into_any_element()
    }
    pub(super) fn render_inspector(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut tabs = h_flex().flex_wrap();
        for (tab, key) in [
            (RightTab::Agent, "agent"),
            (RightTab::Inspector, "inspector"),
            (RightTab::Evidence, "evidence"),
            (RightTab::Unknowns, "unknowns"),
        ] {
            if tab == RightTab::Agent && self.agent.is_none() {
                continue;
            }
            tabs = tabs.child(
                Button::new(key)
                    .label(rust_i18n::t!(format!("app.workbench.{key}")))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.right = tab;
                        cx.notify();
                    })),
            );
        }
        let mut content = v_flex().gap_2().p_3();
        match self.right {
            RightTab::Agent => {
                if let Some(agent) = &self.agent {
                    return v_flex()
                        .size_full()
                        .child(tabs)
                        .child(agent.clone())
                        .into_any_element();
                }
            }
            RightTab::Inspector => {
                if let Some(selection) = &self.selection {
                    content = content.child(self.json(selection.clone(), cx));
                } else {
                    content = content.child(rust_i18n::t!("app.workbench.select_item"));
                }
            }
            RightTab::Evidence => {
                let ledger = self
                    .state
                    .read(cx)
                    .analysis_view
                    .as_ref()
                    .map(|v| v.ledger.clone())
                    .unwrap_or_default();
                for summary in ledger {
                    let id = summary.id.clone();
                    content = content.child(
                        Button::new(summary.id.as_str().to_owned())
                            .label(format!("{} · {}", summary.id.as_str(), summary.summary))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.show_evidence(id.clone(), cx)
                            })),
                    );
                }
                if let Some(value) = &self.evidence {
                    let confidence =
                        value
                            .get("confidence")
                            .and_then(|v| v.as_str())
                            .and_then(|v| match v {
                                "observed" => Some(autocraft_core::Confidence::Observed),
                                "derived" => Some(autocraft_core::Confidence::Derived),
                                "inferred" => Some(autocraft_core::Confidence::Inferred),
                                _ => None,
                            });
                    let fields = [
                        "coverage",
                        "locations",
                        "limitations",
                        "operation",
                        "environment",
                    ]
                    .into_iter()
                    .filter_map(|key| {
                        value.get(key).map(|v| {
                            (
                                rust_i18n::t!(format!("app.workbench.{key}"))
                                    .into_owned()
                                    .into(),
                                super::data::scalar(v).into(),
                            )
                        })
                    })
                    .collect();
                    let entity = cx.entity();
                    content = content.child(
                        EvidenceCard::new(
                            "ledger-evidence",
                            EvidenceCardProps {
                                summary: value
                                    .get("summary")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or_default()
                                    .to_owned()
                                    .into(),
                                confidence,
                                confidence_labels: confidence_labels(),
                                fields,
                                json: value.clone(),
                                expanded: true,
                                toggle_label: rust_i18n::t!("app.workbench.json")
                                    .into_owned()
                                    .into(),
                                expanded_json: self.expanded.clone(),
                            },
                        )
                        .on_json_expand(move |path, open, _, cx| {
                            entity.update(cx, |this, cx| {
                                let set = Arc::make_mut(&mut this.expanded);
                                if open {
                                    set.insert(path.to_owned());
                                } else {
                                    set.remove(path);
                                }
                                cx.notify();
                            })
                        }),
                    );
                }
            }
            RightTab::Unknowns => {
                let mut unknowns = self
                    .state
                    .read(cx)
                    .analysis_view
                    .as_ref()
                    .map(|v| v.unknowns.clone())
                    .unwrap_or_default();
                unknowns.sort_by_key(|unknown| match unknown.severity {
                    Severity::Critical => 0,
                    Severity::High => 1,
                    Severity::Medium => 2,
                    Severity::Low => 3,
                });
                for unknown in unknowns {
                    let severity = match unknown.severity {
                        Severity::Critical => "critical",
                        Severity::High => "high",
                        Severity::Medium => "medium",
                        Severity::Low => "low",
                    };
                    let status = match unknown.status {
                        UnknownStatus::Open => "open",
                        UnknownStatus::Investigating => "investigating",
                        UnknownStatus::Blocked => "blocked",
                        UnknownStatus::Contradicted => "contradicted",
                        UnknownStatus::Resolved => "resolved",
                    };
                    let mut card = v_flex()
                        .gap_2()
                        .p_3()
                        .border_1()
                        .child(format!(
                            "{} · {}",
                            rust_i18n::t!(format!("app.workbench.{severity}")),
                            rust_i18n::t!(format!("app.workbench.{status}"))
                        ))
                        .child(unknown.question)
                        .child(unknown.domain)
                        .children(unknown.resolution);
                    for id in unknown
                        .evidence_for
                        .into_iter()
                        .chain(unknown.evidence_against)
                    {
                        card = card.child(
                            Button::new(id.as_str().to_owned())
                                .label(id.as_str().to_owned())
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.show_evidence(id.clone(), cx)
                                })),
                        );
                    }
                    for probe in unknown.probes {
                        if let Some(tool) = self.tools.iter().find(|tool| tool.id == probe).cloned()
                        {
                            let params = super::data::params(
                                &tool,
                                self.selection.as_ref(),
                                &self.target_path(cx),
                                0,
                            );
                            card =
                                card.child(Button::new(probe).label(tool.title.clone()).on_click(
                                    cx.listener(move |this, _, _, cx| {
                                        this.tab = super::data::EditorTab::Results;
                                        this.request(
                                            tool.clone(),
                                            params.clone(),
                                            false,
                                            false,
                                            cx,
                                        );
                                    }),
                                ));
                        }
                    }
                    content = content.child(card);
                }
            }
        }
        v_flex()
            .size_full()
            .child(tabs)
            .child(
                content
                    .id("analysis-inspector")
                    .flex_1()
                    .overflow_y_scrollbar(),
            )
            .into_any_element()
    }
}
