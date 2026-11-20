//! The shared panel surface: project tabs, live Markdown, interaction cards and composer.
use super::{AgentPanel, labels as s, transcript};
use autocraft_core::{ChatId, Readiness, SessionPhase, SessionSnapshot};
use autocraft_ui::{
    AutocraftTheme, Roles,
    components::{IconButton, Status, StatusDot, Tab, TabStrip, label},
    metrics::{size, space, text},
};
use gpui_kit::{
    AnyElement, Context, EventEmitter, InteractiveElement as _, IntoElement, ParentElement as _,
    Render, StatefulInteractiveElement as _, Styled as _, Window,
    assets::IconName,
    base::{h_flex, v_flex},
    component::input::Textarea,
    div, px,
};
use std::rc::Rc;

/// Navigation intent; the workbench owns the evidence inspector and may switch to it.
pub enum AgentPanelEvent {
    Evidence {
        project: autocraft_core::ProjectId,
        id: autocraft_core::EvidenceId,
    },
}
impl EventEmitter<AgentPanelEvent> for AgentPanel {}
impl AgentPanel {
    fn tab_strip(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut strip = TabStrip::new();
        for id in &self.order {
            let Some(chat) = self.chats.get(id) else {
                continue;
            };
            let select = id.clone();
            let close = id.clone();
            strip = strip.tab(
                Tab::new(format!("agent-tab-{id}"), chat.title())
                    .icon(IconName::MessageSquare)
                    .active(!self.builder_selected && self.active.as_ref() == Some(id))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.select_tab(select.clone(), window, cx);
                    }))
                    .on_close(cx.listener(move |this, _, _, cx| this.close_chat(&close, cx))),
            );
        }
        if self
            .project
            .as_ref()
            .is_some_and(|id| self.builders.contains_key(id))
        {
            strip = strip.tab(
                Tab::new(
                    "agent-builder-tab",
                    rust_i18n::t!("app.agent.builder").to_string(),
                )
                .active(self.builder_selected)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.builder_selected = true;
                    cx.notify();
                })),
            );
        }
        strip.action(
            IconButton::new("agent-new", IconName::MessageCirclePlus)
                .tooltip(s::new_chat())
                .disabled(self.project.is_none())
                .on_click(cx.listener(|this, _, window, cx| {
                    this.new_chat(cx);
                    if let Some(id) = this.active.clone() {
                        this.select_tab(id, window, cx);
                    }
                })),
        )
    }
    fn select_tab(&mut self, id: ChatId, window: &mut Window, cx: &mut Context<Self>) {
        self.active = Some(id.clone());
        self.builder_selected = false;
        let draft = self
            .chats
            .get(&id)
            .map(|c| c.draft.clone())
            .unwrap_or_default();
        self.composer
            .update(cx, |input, cx| input.set_value(draft, window, cx));
        self.slash = Default::default();
        self.scroll.scroll_to_bottom();
        if self.chats.get(&id).is_some_and(|c| c.snapshot.is_none()) {
            self.start(id, None, cx);
        }
        cx.notify();
    }
    fn transcript(
        &mut self,
        snapshot: Option<&SessionSnapshot>,
        readonly: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let r = AutocraftTheme::global(cx).roles;
        let entity = cx.entity().downgrade();
        let toggle: transcript::Toggle = Rc::new(move |key, _, cx| {
            if let Some(panel) = entity.upgrade() {
                panel.update(cx, |this, cx| {
                    if !this.open.remove(&key) {
                        this.open.insert(key);
                    }
                    cx.notify();
                });
            }
        });
        let entity = cx.entity().downgrade();
        let evidence: transcript::Evidence = Rc::new(move |id, _, cx| {
            if let Some(panel) = entity.upgrade() {
                panel.update(cx, |this, cx| {
                    if let Some(project) = this.project.clone() {
                        cx.emit(AgentPanelEvent::Evidence {
                            project,
                            id: id.into(),
                        });
                    }
                });
            }
        });
        let mut body = v_flex()
            .id("agent-transcript")
            .flex_1()
            .min_h(px(0.))
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .p(space::S12)
            .gap(space::S16);
        if let Some(snapshot) = snapshot {
            if snapshot.output_truncated {
                body = body.child(label(text::METADATA, s::truncated()).text_color(r.text_subtle));
            }
            body = body.children(
                snapshot
                    .messages
                    .iter()
                    .map(|m| transcript::message(m, &self.open, &toggle, &evidence, &r)),
            );
            if matches!(snapshot.phase, SessionPhase::Running) {
                body = body.child(
                    label(
                        text::METADATA,
                        rust_i18n::t!("app.agent.streaming").to_string(),
                    )
                    .text_color(r.text_subtle),
                );
            }
            if let Some(error) = &snapshot.last_error {
                body = body.child(
                    label(
                        text::METADATA,
                        format!("{} · {}", error.code, error.message),
                    )
                    .text_color(r.feedback_error),
                );
                if let Some(trace) = &error.trace_id {
                    let trace = trace.clone();
                    body = body.child(
                        gpui_kit::component::button::Button::new("agent-copy-trace")
                            .label(rust_i18n::t!("app.agent.copy_trace").to_string())
                            .on_click(move |_, _, cx| {
                                cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(
                                    trace.clone(),
                                ));
                            }),
                    );
                }
            }
            if !readonly {
                let interactions: Vec<_> = snapshot
                    .interactions
                    .iter()
                    .cloned()
                    .map(|i| self.interaction_card(i, window, cx))
                    .collect();
                body = body.children(interactions);
            }
        }
        body.into_any_element()
    }
    fn chat_body(
        &mut self,
        id: &ChatId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.sync_model_picker(id, cx);
        self.sync_usage_view(id, cx);
        let Some(chat) = self.chats.get(id) else {
            return div().into_any_element();
        };
        let phase = chat.phase();
        let starting = chat.starting || chat.closing;
        let blocked = chat.blocked;
        let snapshot = chat.snapshot.clone();
        let backend = chat.binding.as_ref().map(|b| b.backend);
        let r = AutocraftTheme::global(cx).roles;
        let (status, phase_label) = match phase {
            SessionPhase::Idle => (Status::Success, s::phase_idle()),
            SessionPhase::Running => (Status::Info, s::phase_running()),
            SessionPhase::Waiting => (Status::Warning, s::phase_waiting()),
            SessionPhase::Closed => (Status::Idle, s::phase_closed()),
        };
        let busy = matches!(phase, SessionPhase::Running | SessionPhase::Waiting);
        let action = if busy {
            IconButton::new("agent-stop", IconName::Square)
                .tooltip(s::stop())
                .on_click(cx.listener(|this, _, _, cx| this.stop(cx)))
        } else {
            IconButton::new("agent-send", IconName::ArrowUp)
                .tooltip(s::send())
                .disabled(starting || blocked.is_some())
                .on_click(cx.listener(|this, _, window, cx| this.send(window, cx)))
        };
        let textarea = self
            .slash_keys(Textarea::new(&self.composer).appearance(false), cx)
            .into_any_element();
        let menu = self.slash_menu(cx);
        let mut content = v_flex()
            .size_full()
            .child(
                h_flex()
                    .h(size::PANEL_HEADER)
                    .px(space::S12)
                    .gap(space::S8)
                    .children(
                        backend.map(|b| label(text::LABEL_PILL, b.label()).text_color(r.primary)),
                    )
                    .child(StatusDot::new(status).label(phase_label)),
            )
            .child(self.transcript(snapshot.as_ref(), false, window, cx));
        if starting {
            content = content.child(label(text::METADATA, s::starting()).px(space::S12));
        } else if let Some(readiness) = blocked {
            content = content.child(empty(readiness_label(readiness), &r));
        } else if phase == SessionPhase::Closed {
            content = content.child(label(text::METADATA, s::closed_hint()).px(space::S12));
        }
        content
            .child(
                div()
                    .relative()
                    .flex_shrink_0()
                    .p(space::S8)
                    .border_t_1()
                    .border_color(r.border_muted)
                    .child(
                        v_flex().gap(space::S6).child(textarea).child(
                            h_flex()
                                .flex_wrap()
                                .gap(space::S8)
                                .child(self.model_picker.view())
                                .child(div().flex_1().min_w(px(0.)).child(self.usage_view.clone()))
                                .child(action),
                        ),
                    )
                    .children(menu),
            )
            .into_any_element()
    }
}
impl Render for AgentPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let r = AutocraftTheme::global(cx).roles;
        // Draft is project/tab-owned; selecting a project externally must not reuse another draft.
        if !self.builder_selected {
            let draft = self
                .active
                .as_ref()
                .and_then(|id| self.chats.get(id))
                .map(|c| c.draft.clone())
                .unwrap_or_default();
            if self.composer.read(cx).value().as_ref() != draft.as_str() {
                self.composer
                    .update(cx, |input, cx| input.set_value(draft, window, cx));
            }
        }
        let strip = self.tab_strip(cx).into_any_element();
        let body = if self.project.is_none() {
            empty(s::no_workspace(), &r)
        } else if self.builder_selected {
            let snapshot = self
                .project
                .as_ref()
                .and_then(|id| self.builders.get(id))
                .cloned();
            v_flex()
                .size_full()
                .min_h(px(0.))
                .child(
                    label(
                        text::METADATA,
                        rust_i18n::t!("app.agent.builder_readonly").to_string(),
                    )
                    .p(space::S8),
                )
                .child(self.transcript(snapshot.as_ref(), true, window, cx))
                .into_any_element()
        } else if let Some(id) = self.active.clone() {
            self.chat_body(&id, window, cx)
        } else {
            empty(self.guide.map_or_else(s::no_chats, readiness_label), &r)
        };
        v_flex()
            .size_full()
            .min_h(px(0.))
            .bg(r.container_workspace_bg)
            .child(strip)
            .child(div().flex_1().min_h(px(0.)).child(body))
    }
}
fn empty(message: String, r: &Roles) -> AnyElement {
    div()
        .p(space::S16)
        .child(label(text::METADATA, message).text_color(r.text_subtle))
        .into_any_element()
}
fn readiness_label(readiness: Readiness) -> String {
    match readiness {
        Readiness::Ready(backend) => backend.label().to_owned(),
        Readiness::NoneInstalled => rust_i18n::t!("app.agent.none_installed").to_string(),
        Readiness::NoDefault(_) => rust_i18n::t!("app.agent.no_default").to_string(),
        Readiness::DefaultMissing(backend) => {
            rust_i18n::t!("app.agent.backend_missing", backend = backend.label()).to_string()
        }
    }
}
