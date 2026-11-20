//! Viewport-based titlebar composition keeps native controls and a draggable spacer visible.
use super::*;
use gpui_kit::{
    StatefulInteractiveElement as _, assets::IconName, base::v_flex, component::popover::Popover,
    prelude::FluentBuilder as _,
};
impl Render for TopBar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let viewport = window.viewport_size();
        let narrow = viewport.width < lumakit_design::metrics::size::NARROW;
        let compact =
            viewport.width < lumakit_design::metrics::size::NARROW * 2. || self.confirm_deep;
        let r = AutocraftTheme::global(cx).roles;
        let entity = cx.entity();
        let row = if compact {
            let state = self.state.read(cx);
            let enabled = state.project.is_some();
            let main = state.main;
            let mut row = h_flex()
                .h_full()
                .gap(space::S8)
                .px(space::S8)
                .occlude()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation());
            if !narrow {
                // Without a project only Home is reachable, so the other views are not offered.
                let views: Vec<(MainView, &str)> = [
                    (MainView::Home, "app.home.title"),
                    (MainView::Project, "app.project.title"),
                    (MainView::Workbench, "app.workbench.title"),
                ]
                .into_iter()
                .filter(|(view, _)| enabled || *view == MainView::Home)
                .collect();
                let state = self.state.clone();
                let targets: Vec<MainView> = views.iter().map(|(view, _)| *view).collect();
                row = row.child(
                    ChoiceMenu::new(
                        "compact-main-view",
                        views
                            .iter()
                            .map(|(_, key)| rust_i18n::t!(*key).into_owned()),
                        targets.iter().position(|view| *view == main),
                    )
                    .ghost()
                    .placeholder(rust_i18n::t!("app.shell.view").into_owned())
                    .on_select(move |index, _, cx| {
                        if let Some(&view) = targets.get(*index) {
                            state.update(cx, |state, cx| state.navigate(view, cx))
                        }
                    }),
                );
            }
            row.child(
                Popover::new("compact-topbar-controls")
                    .when(self.confirm_deep, |popover| popover.open(true))
                    .on_open_change(cx.listener(|this, open: &bool, _, cx| {
                        if !open && this.confirm_deep {
                            this.confirm_deep = false;
                            cx.notify();
                        }
                    }))
                    .trigger(
                        Button::new("compact-topbar-menu")
                            .ghost()
                            .icon(IconName::ChevronDown)
                            .accessibility_label(
                                rust_i18n::t!("app.shell.controls_menu").into_owned(),
                            )
                            .tooltip(rust_i18n::t!("app.shell.controls_menu").into_owned()),
                    )
                    .content(move |_, window, cx| {
                        let viewport = window.viewport_size();
                        v_flex()
                            .id("compact-topbar-scroll")
                            .w((viewport.width - space::S64)
                                .min(autocraft_ui::metrics::content::FORM_FIELD))
                            .max_h(viewport.height - size::TOP_BAR - space::S64)
                            .overflow_y_scroll()
                            .child(entity.update(cx, |this, cx| this.controls(true, cx)))
                    }),
            )
            .child(
                gpui_kit::div()
                    .flex_1()
                    .min_w(space::S16)
                    .h(size::TOP_BAR)
                    .window_control_area(WindowControlArea::Drag),
            )
            .child(
                Button::new("compact-open-settings")
                    .ghost()
                    .icon(IconName::Settings)
                    .accessibility_label(rust_i18n::t!("app.shell.settings").into_owned())
                    .tooltip(rust_i18n::t!("app.shell.settings").into_owned())
                    .on_click(cx.listener(|this, _, window, cx| {
                        SettingsDialog::open(this.state.clone(), window, cx)
                    })),
            )
        } else {
            self.controls(false, cx)
        };
        // The row takes the space TitleBar leaves beside its own window controls and may shrink:
        // a fixed width pushes the native minimize/maximize/close buttons out of the window.
        TitleBar::new()
            .h(size::TOP_BAR)
            .bg(r.container_header_bg)
            .child(row.flex_1().min_w(gpui_kit::px(0.)).overflow_hidden())
    }
}
