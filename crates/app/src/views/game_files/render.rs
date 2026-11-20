//! Archive rows, paged contents, explicit consent and the exported PNG preview.
use super::GameFilesView;
use autocraft_core::AnalysisMode;
use autocraft_ui::{
    AutocraftTheme,
    components::label,
    metrics::{space, text},
};
use gpui_kit::{
    AnyElement, Context, IntoElement, ParentElement as _, Render, Styled as _, Window,
    base::{h_flex, v_flex},
    component::{Disableable as _, button::Button, input::Input, scroll::ScrollableElement as _},
    px,
};
impl GameFilesView {
    fn archive_rows(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut body = v_flex().min_w_0().gap(space::S8).child(label(
            text::HEADING_MD,
            rust_i18n::t!("app.game_files.archives").to_string(),
        ));
        if self.archives.is_empty() && self.root.is_some() && !self.busy {
            body = body.child(label(
                text::METADATA,
                rust_i18n::t!("app.game_files.no_archives").to_string(),
            ));
        }
        for (index, archive) in self.archives.iter().enumerate() {
            let archive = archive.clone();
            body = body.child(
                h_flex()
                    .min_w_0()
                    .gap(space::S8)
                    .child(label(text::UI, archive.name.clone()).min_w_0().flex_1())
                    .child(label(
                        text::METADATA,
                        rust_i18n::t!("app.game_files.bytes", count = archive.size).to_string(),
                    ))
                    .child(
                        Button::new(format!("archive-{index}"))
                            .outline()
                            .label(rust_i18n::t!("app.game_files.open").to_string())
                            .disabled(self.busy)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.filter
                                    .update(cx, |input, cx| input.set_value("", window, cx));
                                this.list(archive.clone(), 0, cx);
                            })),
                    ),
            );
        }
        if self.truncated {
            body = body.child(label(
                text::METADATA,
                rust_i18n::t!("app.game_files.truncated").to_string(),
            ));
        }
        body.into_any_element()
    }
    fn item_rows(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(archive) = self.archive.clone() else {
            return label(
                text::METADATA,
                rust_i18n::t!("app.game_files.choose_archive").to_string(),
            )
            .into_any_element();
        };
        let filter_archive = archive.clone();
        let mut body = v_flex()
            .min_w_0()
            .gap(space::S8)
            .child(label(text::HEADING_MD, archive.name.clone()))
            .child(
                h_flex()
                    .min_w_0()
                    .gap(space::S8)
                    .child(Input::new(&self.filter).flex_1().min_w_0())
                    .child(
                        Button::new("apply-filter")
                            .outline()
                            .disabled(self.busy)
                            .label(rust_i18n::t!("app.game_files.search").to_string())
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.list(filter_archive.clone(), 0, cx)
                            })),
                    ),
            );
        let hint = if archive.supports_pictures() {
            "app.game_files.picture_hint"
        } else {
            "app.game_files.browse_only"
        };
        body = body.child(label(text::METADATA, rust_i18n::t!(hint).to_string()));
        if self.items.is_empty() && !self.busy {
            body = body.child(label(
                text::METADATA,
                rust_i18n::t!("app.game_files.no_items").to_string(),
            ));
        }
        for (index, item) in self.items.iter().enumerate() {
            let mut row = h_flex()
                .min_w_0()
                .gap(space::S8)
                .child(label(text::UI, item.name.clone()).min_w_0().flex_1());
            if let Some(size) = item.size {
                row = row.child(label(
                    text::METADATA,
                    rust_i18n::t!("app.game_files.bytes", count = size).to_string(),
                ));
            }
            if item.picture.is_some() && archive.supports_pictures() {
                let archive = archive.clone();
                let item = item.clone();
                row = row.child(
                    Button::new(format!("save-{index}"))
                        .outline()
                        .disabled(self.busy)
                        .label(rust_i18n::t!("app.game_files.save").to_string())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.pick_output(archive.clone(), item.clone(), cx)
                        })),
                );
            }
            body = body.child(row);
        }
        if let Some(offset) = self.next {
            body = body.child(
                Button::new("more")
                    .outline()
                    .disabled(self.busy)
                    .label(rust_i18n::t!("app.game_files.more").to_string())
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.list(archive.clone(), offset, cx)),
                    ),
            );
        }
        body.into_any_element()
    }
    fn confirmation(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut body = v_flex().min_w_0().gap(space::S8);
        if self.confirm_deep {
            body = body
                .child(label(
                    text::UI,
                    rust_i18n::t!("app.game_files.deep_confirm").to_string(),
                ))
                .child(
                    Button::new("confirm-deep")
                        .disabled(self.busy)
                        .label(rust_i18n::t!("app.game_files.confirm").to_string())
                        .on_click(cx.listener(|this, _, _, cx| this.set_deep(cx))),
                );
        }
        if let Some(pending) = &self.pending {
            body = body.child(label(
                text::UI,
                rust_i18n::t!("app.game_files.consent").to_string(),
            ));
            if let super::Operation::Export { output } = &pending.operation {
                body = body.child(label(text::METADATA, output.display().to_string()));
            }
            body = body.child(
                Button::new("allow-once")
                    .disabled(self.busy)
                    .label(rust_i18n::t!("app.game_files.allow").to_string())
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Some(request) = this.pending.take() {
                            this.submit(request, true, cx);
                        }
                    })),
            );
        }
        if self.confirm_deep || self.pending.is_some() {
            body = body.child(
                Button::new("cancel-confirmation")
                    .outline()
                    .label(rust_i18n::t!("app.game_files.cancel").to_string())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.confirm_deep = false;
                        this.pending = None;
                        cx.notify();
                    })),
            );
        }
        body.into_any_element()
    }
}
impl Render for GameFilesView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let roles = AutocraftTheme::global(cx).roles;
        let deep = self.mode == AnalysisMode::Deep;
        let blocked = self.busy || self.project.is_none() || !deep;
        let mut body = v_flex()
            .size_full()
            .min_w_0()
            .bg(roles.container_content_bg)
            .text_color(roles.text_default)
            .overflow_y_scrollbar()
            .p(space::S24)
            .gap(space::S16)
            .child(label(
                text::HEADING_XL,
                rust_i18n::t!("app.game_files.title").to_string(),
            ))
            .child(label(
                text::UI,
                rust_i18n::t!("app.game_files.description").to_string(),
            ));
        if self.project.is_none() {
            return body.child(label(
                text::UI,
                rust_i18n::t!("app.game_files.project_required").to_string(),
            ));
        }
        if !deep {
            body = body
                .child(label(
                    text::UI,
                    rust_i18n::t!("app.game_files.deep_required").to_string(),
                ))
                .child(
                    Button::new("enable-deep")
                        .disabled(self.busy)
                        .label(rust_i18n::t!("app.game_files.enable_deep").to_string())
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.confirm_deep = true;
                            cx.notify();
                        })),
                );
        }
        body = body.child(
            h_flex()
                .flex_wrap()
                .gap(space::S8)
                .child(
                    Button::new("pick-folder")
                        .disabled(blocked)
                        .label(rust_i18n::t!("app.game_files.pick_folder").to_string())
                        .on_click(cx.listener(|this, _, _, cx| this.pick_path(true, cx))),
                )
                .child(
                    Button::new("pick-archive")
                        .outline()
                        .disabled(blocked)
                        .label(rust_i18n::t!("app.game_files.pick_archive").to_string())
                        .on_click(cx.listener(|this, _, _, cx| this.pick_path(false, cx))),
                )
                .child(
                    Button::new("use-target")
                        .outline()
                        .disabled(blocked || self.target_path.is_none())
                        .label(rust_i18n::t!("app.game_files.use_target").to_string())
                        .on_click(cx.listener(|this, _, _, cx| {
                            if let Some(path) = this.target_path.clone() {
                                this.discover(path, cx);
                            }
                        })),
                ),
        );
        if let Some(path) = &self.root {
            body = body.child(label(text::METADATA, path.display().to_string()).min_w_0());
        }
        if self.busy {
            body = body.child(label(
                text::METADATA,
                rust_i18n::t!("app.game_files.loading").to_string(),
            ));
        }
        if let Some(error) = &self.error {
            body = body.child(label(text::UI, error.clone()).text_color(roles.feedback_error));
        }
        body = body.child(self.confirmation(cx));
        if deep {
            body = body.child(self.archive_rows(cx)).child(self.item_rows(cx));
        }
        if let Some(saved) = &self.saved {
            let (width, height) = saved.dimensions();
            body = body
                .child(label(
                    text::UI,
                    rust_i18n::t!("app.game_files.saved", width = width, height = height)
                        .to_string(),
                ))
                .child(label(text::METADATA, saved.output.clone()).min_w_0())
                .child(
                    gpui_kit::img(std::path::PathBuf::from(&saved.output))
                        .w_full()
                        .h(px(360.)),
                );
        }
        body
    }
}
