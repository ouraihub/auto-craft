//! Dense responsive create form and pin list with secondary actions in a native menu.
use super::{
    PinsView,
    model::{self, Action, Mode, Outcome, Row},
};
use autocraft_ui::{
    AutocraftTheme,
    components::label,
    metrics::{space, text},
};
use gpui_kit::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    Styled as _, Window,
    base::{h_flex, v_flex},
    component::{
        Disableable as _, Sizable as _,
        button::{Button, ButtonVariants as _},
        input::Input,
        menu::{DropdownMenu as _, PopupMenuItem},
        scroll::ScrollableElement as _,
    },
    px,
};

impl Render for PinsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let r = AutocraftTheme::global(cx).roles;
        let mut page = v_flex()
            .size_full()
            .min_w(px(0.))
            .gap(space::S16)
            .p(space::S16)
            .child(
                h_flex()
                    .flex_wrap()
                    .gap(space::S8)
                    .child(label(text::TITLE_CARD, rust_i18n::t!("app.pins.title")))
                    .child(
                        Button::new("pins-refresh")
                            .outline()
                            .small()
                            .label(rust_i18n::t!("app.pins.refresh"))
                            .disabled(self.busy || self.project.is_none())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.estimate = None;
                                this.estimate_error = None;
                                this.refresh(cx);
                            })),
                    ),
            );
        if self.project.is_none() {
            return page
                .child(rust_i18n::t!("app.pins.project_needed"))
                .into_any_element();
        }
        page = page.child(self.create_form(cx));
        if let Some(error) = &self.error {
            page = page.child(label(text::UI, error.clone()).text_color(r.feedback_error));
        }
        if let Some(outcome) = self.outcome {
            let message = match outcome {
                Outcome::Created => rust_i18n::t!("app.pins.created"),
                Outcome::Running => rust_i18n::t!("app.pins.started"),
                Outcome::AlreadyRunning => rust_i18n::t!("app.pins.already_running"),
                Outcome::Closed => rust_i18n::t!("app.pins.closed"),
                Outcome::Stopped(count) => rust_i18n::t!("app.pins.stopped", count = count),
                Outcome::Renamed => rust_i18n::t!("app.pins.renamed"),
                Outcome::Deleted => rust_i18n::t!("app.pins.deleted"),
                Outcome::Folder => rust_i18n::t!("app.pins.folder_opened"),
            };
            page = page.child(label(text::UI, message).text_color(r.text_muted));
        }
        if self.busy {
            let cancel = self.cancel.clone();
            page = page.child(
                h_flex()
                    .flex_wrap()
                    .gap(space::S8)
                    .child(rust_i18n::t!("app.pins.working"))
                    .child(
                        Button::new("pins-cancel")
                            .outline()
                            .small()
                            .label(rust_i18n::t!("app.pins.cancel"))
                            .on_click(move |_, _, _| cancel.cancel()),
                    ),
            );
        }
        let mut list = v_flex()
            .id("pins-list")
            .flex_1()
            .min_h(px(0.))
            .min_w(px(0.))
            .gap(space::S8)
            .overflow_y_scrollbar()
            .child(
                label(
                    text::METADATA,
                    rust_i18n::t!(
                        "app.pins.total",
                        count = self.rows.len(),
                        size = model::size(self.copied_bytes)
                    ),
                )
                .text_color(r.text_subtle),
            );
        if self.rows.is_empty() {
            list = list.child(rust_i18n::t!("app.pins.empty"));
        }
        for row in &self.rows {
            list = list.child(self.row(row, cx));
        }
        page.child(list).into_any_element()
    }
}
impl PinsView {
    fn create_form(&self, cx: &mut Context<Self>) -> AnyElement {
        let r = AutocraftTheme::global(cx).roles;
        let mut form = v_flex()
            .min_w(px(0.))
            .gap(space::S8)
            .child(label(text::UI, rust_i18n::t!("app.pins.intro")).text_color(r.text_muted))
            .child(
                label(text::METADATA, rust_i18n::t!("app.pins.website_note"))
                    .text_color(r.feedback_warning),
            );
        let mut choices = h_flex().flex_wrap().gap(space::S8);
        for mode in [Mode::Frozen, Mode::Light] {
            let mut button = Button::new(format!("pin-mode-{mode:?}"))
                .small()
                .label(rust_i18n::t!(mode.key()))
                .disabled(self.busy);
            button = if self.mode == mode {
                button.primary()
            } else {
                button.outline()
            };
            choices = choices.child(button.on_click(cx.listener(move |this, _, _, cx| {
                this.mode = mode;
                cx.notify();
            })));
        }
        form = form.child(choices).child(
            label(
                text::METADATA,
                rust_i18n::t!(match self.mode {
                    Mode::Frozen => "app.pins.frozen_note",
                    Mode::Light => "app.pins.light_note",
                    Mode::Live => "app.pins.live_note",
                }),
            )
            .text_color(r.text_muted),
        );
        if let Some(estimate) = self.estimate {
            form = form.child(
                label(
                    text::METADATA,
                    rust_i18n::t!(
                        "app.pins.estimate",
                        frozen = model::size(estimate.bytes),
                        light = model::size(estimate.light_bytes),
                        files = estimate.files
                    ),
                )
                .text_color(
                    if estimate.bytes >= 1_000_000_000 && self.mode == Mode::Frozen {
                        r.feedback_warning
                    } else {
                        r.text_subtle
                    },
                ),
            );
        }
        if let Some(error) = &self.estimate_error {
            form = form.child(label(text::UI, error.clone()).text_color(r.feedback_error));
        }
        form.child(
            h_flex()
                .flex_wrap()
                .gap(space::S8)
                .child(
                    v_flex().flex_1().min_w(px(0.)).child(
                        Input::new(&self.name)
                            .small()
                            .disabled(self.busy)
                            .aria_label(rust_i18n::t!("app.pins.name")),
                    ),
                )
                .child(
                    Button::new("pin-create")
                        .primary()
                        .small()
                        .label(rust_i18n::t!("app.pins.create"))
                        .disabled(
                            self.busy
                                || self.estimate.is_none()
                                || !self.ready("project_pin_create"),
                        )
                        .on_click(cx.listener(|this, _, window, cx| {
                            let name = this.name.read(cx).value().trim().to_owned();
                            this.confirm(
                                Action::Create {
                                    mode: this.mode,
                                    name,
                                },
                                window,
                                cx,
                            );
                        })),
                ),
        )
        .into_any_element()
    }
    fn row(&self, row: &Row, cx: &mut Context<Self>) -> AnyElement {
        let r = AutocraftTheme::global(cx).roles;
        let pin = row.pin.id;
        let round = row
            .pin
            .round
            .map_or_else(|| "—".to_owned(), |round| round.to_string());
        let state = rust_i18n::t!(if row.running {
            "app.pins.running"
        } else {
            "app.pins.not_running"
        });
        let metadata = rust_i18n::t!(
            "app.pins.metadata",
            kind = rust_i18n::t!(row.pin.mode.key()),
            round = round,
            size = model::size(row.pin.bytes),
            status = state
        );
        let mut details = v_flex()
            .flex_1()
            .min_w(px(0.))
            .gap(space::S4)
            .child(label(text::UI, row.pin.name.clone()))
            .child(label(text::METADATA, metadata).text_color(r.text_subtle));
        if row.pin.shared_bytes > 0 {
            details = details.child(
                label(
                    text::METADATA,
                    rust_i18n::t!("app.pins.shared", size = model::size(row.pin.shared_bytes)),
                )
                .text_color(r.text_subtle),
            );
        }
        if row.pin.mode == Mode::Live {
            details = details.child(
                label(text::METADATA, rust_i18n::t!("app.pins.live_note"))
                    .text_color(r.feedback_warning),
            );
        }
        let owner = cx.entity();
        let running = row.running;
        let busy = self.busy;
        let folder_ready = self.ready("project_open_folder");
        let open_ready = self.ready("project_pin_open");
        let rename_ready = self.ready("project_pin_rename");
        let delete_ready = self.ready("project_pin_delete");
        let action = if running {
            Action::Stop(pin)
        } else {
            Action::Open {
                pin,
                another: false,
            }
        };
        h_flex()
            .flex_wrap()
            .min_w(px(0.))
            .gap(space::S8)
            .p(space::S12)
            .border_1()
            .border_color(r.border_muted)
            .rounded_md()
            .bg(r.container_elevated_bg)
            .child(details)
            .child(
                h_flex()
                    .gap(space::S4)
                    .child(
                        Button::new(format!("pin-open-{pin}"))
                            .outline()
                            .small()
                            .label(rust_i18n::t!(if running {
                                "app.pins.stop"
                            } else {
                                "app.pins.open"
                            }))
                            .disabled(busy || !self.ready(action.tool()))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.confirm(action.clone(), window, cx);
                            })),
                    )
                    .child(
                        Button::new(format!("pin-more-{pin}"))
                            .outline()
                            .small()
                            .label(rust_i18n::t!("app.pins.more"))
                            .disabled(busy)
                            .dropdown_menu(move |menu, _, _| {
                                let folder = owner.clone();
                                let rename = owner.clone();
                                let delete = owner.clone();
                                let another = owner.clone();
                                let mut menu = menu;
                                if running {
                                    menu = menu.item(
                                        PopupMenuItem::new(rust_i18n::t!("app.pins.another"))
                                            .disabled(!open_ready)
                                            .on_click(move |_, window, cx| {
                                                another.update(cx, |this, cx| {
                                                    this.confirm(
                                                        Action::Open { pin, another: true },
                                                        window,
                                                        cx,
                                                    )
                                                });
                                            }),
                                    );
                                }
                                menu.item(
                                    PopupMenuItem::new(rust_i18n::t!("app.pins.folder"))
                                        .disabled(!folder_ready)
                                        .on_click(move |_, window, cx| {
                                            folder.update(cx, |this, cx| {
                                                this.confirm(Action::Folder(pin), window, cx)
                                            });
                                        }),
                                )
                                .item(
                                    PopupMenuItem::new(rust_i18n::t!("app.pins.rename"))
                                        .disabled(!rename_ready)
                                        .on_click(move |_, window, cx| {
                                            rename.update(cx, |this, cx| {
                                                this.rename(pin, window, cx)
                                            });
                                        }),
                                )
                                .item(
                                    PopupMenuItem::new(rust_i18n::t!("app.pins.delete"))
                                        .disabled(running || !delete_ready)
                                        .on_click(move |_, window, cx| {
                                            delete.update(cx, |this, cx| {
                                                this.confirm(Action::Delete(pin), window, cx)
                                            });
                                        }),
                                )
                            }),
                    ),
            )
            .into_any_element()
    }
}
