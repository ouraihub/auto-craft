//! The 事件 and 层级 menus beside the log filter: each narrows the list to one value or shows all.

use gpui_kit::{
    Context, IntoElement,
    assets::IconName,
    component::{
        Sizable as _,
        button::{Button, ButtonVariants as _},
        menu::{DropdownMenu as _, PopupMenuItem},
    },
};

use super::{LogsPage, filter};

impl LogsPage {
    /// 事件 menu: all, or one lifecycle event (ENTRY, EXIT, ERROR…).
    pub(super) fn event_menu(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let current = self.narrow.event;
        let page = cx.entity();
        Button::new("logs-event")
            .ghost()
            .small()
            .label(current.map_or(
                crate::i18n::text("app.settings.logs.all_events"),
                crate::panels::settings::logs::labels::event,
            ))
            .accessibility_label(crate::i18n::text("app.settings.logs.event_menu"))
            .icon(IconName::ChevronDown)
            .dropdown_menu(move |menu, _, _| {
                std::iter::once(None)
                    .chain(filter::EVENTS.into_iter().map(Some))
                    .fold(menu, |menu, value| {
                        let page = page.clone();
                        menu.item(
                            PopupMenuItem::new(value.map_or(
                                crate::i18n::text("app.settings.logs.all_events"),
                                crate::panels::settings::logs::labels::event,
                            ))
                            .checked(current == value)
                            .on_click(move |_, _, cx| {
                                page.update(cx, |this, cx| {
                                    this.narrow.event = value;
                                    cx.notify();
                                })
                            }),
                        )
                    })
            })
    }

    /// 层级 menu: all, or one of the layers present in the lines read.
    pub(super) fn layer_menu(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let current = self.narrow.layer.clone();
        let layers = filter::layers(&self.entries);
        let page = cx.entity();
        Button::new("logs-layer")
            .ghost()
            .small()
            .label(
                current.clone().unwrap_or_else(|| {
                    crate::i18n::text("app.settings.logs.all_layers").into_owned()
                }),
            )
            .accessibility_label(crate::i18n::text("app.settings.logs.layer_menu"))
            .icon(IconName::ChevronDown)
            .dropdown_menu(move |menu, _, _| {
                std::iter::once(None)
                    .chain(layers.iter().cloned().map(Some))
                    .fold(menu, |menu, value| {
                        let page = page.clone();
                        let title = value.clone().unwrap_or_else(|| {
                            crate::i18n::text("app.settings.logs.all_layers").into_owned()
                        });
                        let checked = current == value;
                        menu.item(PopupMenuItem::new(title).checked(checked).on_click(
                            move |_, _, cx| {
                                let value = value.clone();
                                page.update(cx, |this, cx| {
                                    this.narrow.layer = value;
                                    cx.notify();
                                })
                            },
                        ))
                    })
            })
    }
}
