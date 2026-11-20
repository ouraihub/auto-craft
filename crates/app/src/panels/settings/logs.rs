//! 后台日志: the 显示后台日志
//! switch and level, and — while shown and this page is open — the newest log lines, refreshed
//! every 2 s, narrowed by 只看问题, event, layer and free text (TraceId, op, chat or module),
//! copyable as raw lines.
//!
//! The reads here bypass `state::run` on purpose: polling must not write an `op` line every 2 s.
pub(super) mod labels;

use std::time::Duration;

use autocraft_core::{AppPrefs, Error, LogLevel};
use autocraft_observability::LogEntry;
use autocraft_ui::{
    AutocraftTheme,
    components::label,
    metrics::{radius, space, text},
};
use gpui_kit::{
    AppContext as _, ClipboardItem, Context, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, StatefulInteractiveElement as _, Styled as _, Subscription, Task,
    Window,
    assets::IconName,
    base::{h_flex, v_flex},
    component::{
        Disableable as _, Sizable as _,
        button::{Button, ButtonVariants as _},
        input::{Input, InputEvent, InputState},
        menu::{DropdownMenu as _, PopupMenuItem},
        switch::Switch,
    },
    div,
    prelude::FluentBuilder as _,
    px,
};

use crate::state::{AppState, run};

const LIMIT: usize = 300;
const EVERY: Duration = Duration::from_secs(2);

pub struct LogsPage {
    state: Entity<AppState>,
    prefs: Option<AppPrefs>,
    entries: Vec<LogEntry>,
    narrow: Narrow,
    filter: Entity<InputState>,
    read_error: Option<String>,
    copied: bool,
    /// Set by the settings shell: the page is the one on screen.
    active: bool,
    /// The polling loop; dropping it stops polling.
    poll: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl LogsPage {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let filter = cx.new(|cx| {
            InputState::new(window, cx).placeholder(crate::i18n::text("app.settings.logs.filter"))
        });
        let subscriptions = vec![cx.subscribe(&filter, |_, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify()
            }
        })];
        let mut page = Self {
            state,
            prefs: None,
            entries: Vec::new(),
            narrow: Narrow::default(),
            filter,
            read_error: None,
            copied: false,
            active: false,
            poll: None,
            _subscriptions: subscriptions,
        };
        page.load_prefs(cx);
        page
    }

    fn report(&mut self, error: &Error, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| state.report(error, cx));
    }

    fn load_prefs(&mut self, cx: &mut Context<Self>) {
        let service = self.state.read(cx).services.app_prefs.clone();
        run(
            cx,
            move || service.app_prefs(),
            |this, result, cx| match result {
                Ok(prefs) => {
                    this.prefs = Some(prefs);
                    this.sync_polling(cx);
                }
                Err(error) => this.report(&error, cx),
            },
        );
    }

    /// The settings shell calls this when the page is shown or left.
    pub fn set_active(&mut self, active: bool, cx: &mut Context<Self>) {
        self.active = active;
        self.sync_polling(cx);
    }

    /// Polls only while the page is open and 显示后台日志 is on; otherwise drops the loop.
    fn sync_polling(&mut self, cx: &mut Context<Self>) {
        let wanted = self.active && self.prefs.as_ref().is_some_and(|p| p.logs_visible);
        if !wanted {
            self.poll = None;
            return;
        }
        if self.poll.is_some() {
            return;
        }
        let log = self.state.read(cx).services.log.clone();
        self.poll = Some(cx.spawn(async move |this, cx| {
            loop {
                let log = log.clone();
                let read = cx
                    .background_executor()
                    .spawn(async move { log.tail(LIMIT) })
                    .await;
                if this.update(cx, |this, cx| this.loaded(read, cx)).is_err() {
                    return;
                }
                cx.background_executor().timer(EVERY).await;
            }
        }));
    }

    fn loaded(&mut self, read: std::io::Result<Vec<LogEntry>>, cx: &mut Context<Self>) {
        match read {
            Ok(entries) => {
                self.entries = entries;
                self.read_error = None;
            }
            Err(error) => {
                if self.read_error.is_none() {
                    tracing::error!(code = "LOG_READ_FAILED", kind = ?error.kind(), "app.logs.read_failed");
                }
                self.read_error =
                    Some(crate::i18n::text("app.settings.logs.read_error").into_owned());
            }
        }
        cx.notify();
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        self.poll = None;
        self.sync_polling(cx);
    }

    fn set_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        let service = self.state.read(cx).services.app_prefs.clone();
        run(
            cx,
            move || service.set_logs_visible(visible),
            |this, result, cx| match result {
                Ok(prefs) => {
                    this.prefs = Some(prefs);
                    this.sync_polling(cx);
                    cx.notify();
                }
                Err(error) => this.report(&error, cx),
            },
        );
    }

    fn set_level(&mut self, level: LogLevel, cx: &mut Context<Self>) {
        let services = self.state.read(cx).services.clone();
        run(
            cx,
            move || {
                let prefs = services.app_prefs.set_log_level(level)?;
                services.log.set_level(level).map_err(|_| {
                    Error::external(crate::i18n::text("app.settings.logs.level_apply_failed"))
                })?;
                Ok(prefs)
            },
            |this, result, cx| match result {
                Ok(prefs) => {
                    this.prefs = Some(prefs);
                    cx.notify();
                }
                Err(error) => this.report(&error, cx),
            },
        );
    }

    /// Newest first, after the event / layer / 只看问题 choices and the filter text.
    fn visible(&self, cx: &Context<Self>) -> Vec<&LogEntry> {
        let query = self.filter.read(cx).value().trim().to_owned();
        self.entries
            .iter()
            .rev()
            .filter(|e| self.narrow.keeps(e, &query))
            .collect()
    }

    fn copy(&mut self, cx: &mut Context<Self>) {
        let mut lines: Vec<&str> = self
            .visible(cx)
            .into_iter()
            .map(|e| e.raw.as_str())
            .collect();
        lines.reverse();
        cx.write_to_clipboard(ClipboardItem::new_string(lines.join("\n")));
        self.copied = true;
        cx.notify();
    }

    /// A TraceId tag selects its whole chain; the same tag again clears the filter.
    fn filter_trace(&mut self, trace_id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let next = if self.filter.read(cx).value().trim() == trace_id {
            String::new()
        } else {
            trace_id.to_owned()
        };
        self.filter
            .update(cx, |input, cx| input.set_value(next, window, cx));
        cx.notify();
    }
}

mod filter;
mod menus;
mod row;

use filter::Narrow;
use row::row;

impl Render for LogsPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let placeholder = crate::i18n::text("app.settings.logs.filter");
        if self.filter.read(cx).presentation().placeholder().as_ref() != placeholder.as_ref() {
            self.filter.update(cx, |input, cx| {
                input.set_placeholder(placeholder, window, cx)
            });
        }
        let r = AutocraftTheme::global(cx).roles;
        let prefs = self.prefs.clone();
        let shown = prefs.as_ref().is_some_and(|p| p.logs_visible);
        let level = prefs.as_ref().map_or(LogLevel::Info, |p| p.log_level);
        let pinned = self.state.read(cx).services.log.pinned();
        let page = cx.entity();
        let visible = self.visible(cx);
        let empty = if self.entries.is_empty() {
            crate::i18n::text("app.settings.logs.empty")
        } else if self.narrow
            == (Narrow {
                only_problems: true,
                ..Narrow::default()
            })
        {
            crate::i18n::text("app.settings.logs.no_problems")
        } else {
            crate::i18n::text("app.settings.logs.no_matches")
        };
        let rows: Vec<_> = visible.iter().map(|e| row(e, &r, cx)).collect();
        let count = visible.len();
        let setting = |title: std::borrow::Cow<'static, str>,
                       hint: std::borrow::Cow<'static, str>,
                       control: gpui_kit::AnyElement| {
            h_flex()
                .gap(space::S16)
                .py(space::S12)
                .border_b_1()
                .border_color(r.border_muted)
                .child(
                    v_flex()
                        .flex_1()
                        .gap(space::S2)
                        .child(label(text::UI, title))
                        .child(label(text::METADATA, hint).text_color(r.text_subtle)),
                )
                .child(control)
        };
        v_flex()
            .id("settings-logs")
            .gap(space::S12)
            .child(setting(
                crate::i18n::text("app.settings.logs.show"),
                crate::i18n::text("app.settings.logs.show_hint"),
                Switch::new("logs-show")
                    .checked(shown)
                    .disabled(prefs.is_none())
                    .accessibility_label(crate::i18n::text("app.settings.logs.show"))
                    .on_click(
                        cx.listener(|this, checked: &bool, _, cx| this.set_visible(*checked, cx)),
                    )
                    .into_any_element(),
            ))
            .child(setting(
                crate::i18n::text("app.settings.logs.level"),
                if pinned {
                    crate::i18n::text("app.settings.logs.level_pinned")
                } else {
                    crate::i18n::text("app.settings.logs.level_hint")
                },
                Button::new("logs-level")
                    .ghost()
                    .small()
                    .label(if level == LogLevel::Debug {
                        crate::i18n::text("app.settings.logs.debug")
                    } else {
                        crate::i18n::text("app.settings.logs.info")
                    })
                    .icon(IconName::ChevronDown)
                    .dropdown_menu(move |menu, _, _| {
                        [
                            (LogLevel::Info, crate::i18n::text("app.settings.logs.info")),
                            (
                                LogLevel::Debug,
                                crate::i18n::text("app.settings.logs.debug"),
                            ),
                        ]
                        .into_iter()
                        .fold(menu, |menu, (value, title)| {
                            let page = page.clone();
                            menu.item(PopupMenuItem::new(title).checked(value == level).on_click(
                                move |_, _, cx| {
                                    page.update(cx, |this, cx| this.set_level(value, cx))
                                },
                            ))
                        })
                    })
                    .into_any_element(),
            ))
            .when(shown, |d| {
                d.child(
                    v_flex()
                        .gap(space::S8)
                        .p(space::S12)
                        .rounded(radius::LG)
                        .border_1()
                        .border_color(r.border_muted)
                        .child(
                            h_flex()
                                .gap(space::S8)
                                .items_center()
                                .child(
                                    v_flex()
                                        .flex_1()
                                        .child(label(
                                            text::TITLE_SECTION,
                                            crate::i18n::text("app.settings.logs.title"),
                                        ))
                                        .child(
                                            label(
                                                text::METADATA,
                                                crate::panels::settings::logs::labels::recent(
                                                    count,
                                                ),
                                            )
                                            .text_color(r.text_subtle),
                                        ),
                                )
                                .child(
                                    Switch::new("logs-problems")
                                        .checked(self.narrow.only_problems)
                                        .label(
                                            crate::i18n::text("app.settings.logs.only_problems")
                                                .into_owned(),
                                        )
                                        .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                            this.narrow.only_problems = *checked;
                                            cx.notify();
                                        })),
                                )
                                .child(
                                    Button::new("logs-refresh")
                                        .outline()
                                        .small()
                                        .label(crate::i18n::text("app.settings.logs.refresh"))
                                        .on_click(cx.listener(|this, _, _, cx| this.refresh(cx))),
                                )
                                .child(
                                    Button::new("logs-copy")
                                        .outline()
                                        .small()
                                        .label(crate::i18n::text("app.settings.logs.copy"))
                                        .disabled(count == 0)
                                        .on_click(cx.listener(|this, _, _, cx| this.copy(cx))),
                                )
                                .child(
                                    Button::new("logs-open-dir")
                                        .outline()
                                        .small()
                                        .label(crate::i18n::text("app.settings.logs.open_dir"))
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            let dir =
                                                this.state.read(cx).services.log.dir().to_owned();
                                            cx.reveal_path(&dir);
                                        })),
                                ),
                        )
                        .child(
                            h_flex()
                                .gap(space::S8)
                                .items_center()
                                .child(self.event_menu(cx))
                                .child(self.layer_menu(cx))
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .child(Input::new(&self.filter).small()),
                                ),
                        )
                        .when(self.copied, |d| {
                            d.child(
                                label(
                                    text::METADATA,
                                    crate::i18n::text("app.settings.logs.copied"),
                                )
                                .text_color(r.feedback_success),
                            )
                        })
                        .children(self.read_error.clone().map(|e| {
                            label(
                                text::METADATA,
                                rust_i18n::t!("app.settings.logs.read_failed_detail", error = e)
                                    .into_owned(),
                            )
                            .text_color(r.feedback_error)
                        }))
                        .child(
                            div()
                                .id("logs-rows")
                                .max_h(px(520.))
                                .overflow_y_scroll()
                                .child(v_flex().children(rows))
                                .when(count == 0, |d| {
                                    d.child(
                                        label(text::METADATA, empty)
                                            .text_color(r.text_subtle)
                                            .p(space::S8),
                                    )
                                }),
                        ),
                )
            })
    }
}
