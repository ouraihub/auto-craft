//! Responsive conversation and compact authoritative target context.
use super::{AskEvent, AskView, suggestions, tr};
use autocraft_core::AnalysisMode;
use autocraft_ui::{
    AutocraftTheme,
    components::label,
    metrics::{content, radius, space, text},
};
use gpui_kit::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    Styled as _, Window,
    base::{h_flex, v_flex},
    component::{
        Disableable as _,
        button::{Button, ButtonVariants as _},
        scroll::ScrollableElement as _,
        switch::Switch,
    },
    div, px,
};

impl AskView {
    fn context_card(&self, cx: &mut Context<Self>) -> AnyElement {
        let r = AutocraftTheme::global(cx).roles;
        let state = self.state.read(cx);
        let snapshot = state.analysis_view.as_ref();
        let target = snapshot.and_then(|view| {
            view.active
                .as_ref()
                .and_then(|id| view.targets.iter().find(|target| &target.id == id))
        });
        let mut card = v_flex()
            .min_w(px(0.))
            .gap(space::S8)
            .p(space::S12)
            .rounded(radius::MD)
            .border_1()
            .border_color(r.border_muted)
            .bg(r.container_content_bg)
            .child(label(text::EYEBROW, tr("app.ask.target")));
        if let Some(target) = target {
            card = card
                .child(label(text::TITLE_CARD, target.name.clone()).truncate())
                .child(label(text::UI, tr(suggestions::kind_key(&target.kind))))
                .child(
                    label(
                        text::METADATA,
                        rust_i18n::t!("app.ask.bytes", count = target.size).into_owned(),
                    )
                    .text_color(r.text_subtle),
                );
        } else {
            card = card
                .child(label(text::TITLE_CARD, tr("app.ask.no_target")))
                .child(label(text::UI, tr("app.ask.no_target_hint")));
        }
        if let Some(snapshot) = snapshot {
            card = card
                .child(label(
                    text::LABEL_PILL,
                    tr(match snapshot.mode {
                        AnalysisMode::CleanRoom => "app.home.clean_room",
                        AnalysisMode::Deep => "app.home.deep",
                    }),
                ))
                .child(label(
                    text::METADATA,
                    tr(match snapshot.mode {
                        AnalysisMode::CleanRoom => "app.ask.clean_hint",
                        AnalysisMode::Deep => "app.ask.deep_hint",
                    }),
                ))
                .child(label(
                    text::METADATA,
                    rust_i18n::t!("app.ask.findings", count = snapshot.ledger.len()).into_owned(),
                ));
            if snapshot.stale {
                card = card.child(label(text::METADATA, tr("app.ask.stale")));
            }
        } else if state.project.is_some() {
            card = card.child(label(text::METADATA, tr("app.ask.snapshot_pending")));
        }
        if let Some(notice) = &state.notice {
            card = card.child(label(
                text::METADATA,
                rust_i18n::t!("app.ask.operation_failed", code = notice.code).into_owned(),
            ));
        }
        let questions = suggestions::questions(target.map(|target| target.kind.as_str()));
        let available = self.agent.is_some() && state.project.is_some();
        let can_export = snapshot.is_some();
        let mut choices = v_flex()
            .min_w(px(0.))
            .gap(space::S6)
            .child(label(text::EYEBROW, tr("app.ask.try_asking")));
        for (index, key) in questions.iter().enumerate() {
            let question = tr(suggestions::question_key(key));
            choices = choices.child(
                Button::new(("ask-suggestion", index))
                    .outline()
                    .w_full()
                    .label(question.clone())
                    .tooltip(question.clone())
                    .disabled(!available)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.suggest(&question, window, cx);
                    })),
            );
        }
        v_flex()
            .gap(space::S12)
            .min_w(px(0.))
            .child(card)
            .child(choices)
            .child(label(text::METADATA, tr("app.ask.draft_hint")).text_color(r.text_subtle))
            .child(
                Switch::new("ask-novice")
                    .checked(self.novice)
                    .label(tr("app.ask.novice"))
                    .accessibility_label(tr("app.ask.novice"))
                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                        this.novice = *checked;
                        if let Some(agent) = &this.agent {
                            agent.update(cx, |agent, cx| agent.set_novice(*checked, cx));
                        }
                        cx.notify();
                    })),
            )
            .child(label(text::METADATA, tr("app.ask.novice_hint")).text_color(r.text_subtle))
            .child(
                Button::new("ask-export")
                    .outline()
                    .label(tr("app.ask.export"))
                    .disabled(!can_export)
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(AskEvent::ExportReport))),
            )
            .children(
                self.error
                    .as_ref()
                    .map(|error| label(text::METADATA, error.clone())),
            )
            .into_any_element()
    }
}

impl Render for AskView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let r = AutocraftTheme::global(cx).roles;
        if let Some(agent) = &self.agent {
            agent.update(cx, |agent, cx| agent.set_novice(self.novice, cx));
        }
        let narrow = window.viewport_size().width < lumakit_design::metrics::size::NARROW;
        let context = self.context_card(cx);
        let conversation = div()
            .flex_1()
            .min_w(px(0.))
            .min_h(px(0.))
            .max_w(content::FORM_WIDE)
            .w_full()
            .h_full()
            .children(self.agent.clone())
            .children(
                self.agent
                    .is_none()
                    .then(|| label(text::UI, tr("app.ask.chat_unavailable")).p(space::S16)),
            );
        let body = if narrow {
            v_flex()
                .flex_1()
                .min_h(px(0.))
                .min_w(px(0.))
                .gap(space::S8)
                .child(
                    div()
                        .id("ask-context-stacked")
                        .w_full()
                        .max_h(px(220.))
                        .flex_shrink_0()
                        .overflow_y_scrollbar()
                        .child(context),
                )
                .child(conversation)
                .into_any_element()
        } else {
            h_flex()
                .items_start()
                .flex_1()
                .min_h(px(0.))
                .min_w(px(0.))
                .gap(space::S16)
                .child(conversation)
                .child(
                    div()
                        .id("ask-context-side")
                        .w(px(300.))
                        .h_full()
                        .flex_shrink_0()
                        .overflow_y_scrollbar()
                        .child(context),
                )
                .into_any_element()
        };
        v_flex()
            .size_full()
            .min_w(px(0.))
            .min_h(px(0.))
            .p(space::S12)
            .gap(space::S8)
            .bg(r.container_workspace_bg)
            .child(
                h_flex()
                    .flex_wrap()
                    .gap(space::S8)
                    .child(label(text::HEADING_SM, tr("app.ask.title")))
                    .child(div().flex_1())
                    .child(
                        Button::new("ask-back")
                            .ghost()
                            .label(tr("app.ask.back"))
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(AskEvent::Back))),
                    )
                    .child(
                        Button::new("ask-workbench")
                            .outline()
                            .label(tr("app.ask.workbench"))
                            .disabled(self.state.read(cx).project.is_none())
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(AskEvent::Workbench))),
                    ),
            )
            .child(body)
    }
}
