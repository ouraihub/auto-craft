//! Application navigation and mounting of persistent page entities.
use super::*;

impl Render for Workbench {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let r = AutocraftTheme::global(cx).roles;
        let main = self.state.read(cx).main;
        if self.last_main != main {
            self.agent.update(cx, |agent, cx| {
                agent.set_novice(false, cx);
                if main == MainView::Ask {
                    agent.show_analysis(cx);
                }
            });
            self.last_main = main;
        }
        let content = if self.show_onboarding {
            self.onboarding.clone().into_any_element()
        } else {
            match main {
                MainView::Home => self.home.clone().into_any_element(),
                MainView::Project => self.project.clone().into_any_element(),
                MainView::Workbench => self.analysis.clone().into_any_element(),
                MainView::Ask => self.ask.clone().into_any_element(),
                MainView::GameFiles => self.game_files.clone().into_any_element(),
                MainView::Pins => self.pins.clone().into_any_element(),
            }
        };
        let activity = v_flex()
            .w(size::ACTIVITY_BAR)
            .h_full()
            .bg(r.container_sidebar_bg)
            .children(
                [
                    (
                        MainView::Home,
                        "activity-home",
                        IconName::House,
                        "app.home.title",
                    ),
                    (
                        MainView::Ask,
                        "activity-ask",
                        IconName::MessageSquare,
                        "app.ask.title",
                    ),
                    (
                        MainView::Project,
                        "activity-project",
                        IconName::Folder,
                        "app.project.title",
                    ),
                    (
                        MainView::Workbench,
                        "activity-workbench",
                        IconName::Search,
                        "app.workbench.title",
                    ),
                    (
                        MainView::GameFiles,
                        "activity-game-files",
                        IconName::FolderOpen,
                        "app.game_files.title",
                    ),
                    (
                        MainView::Pins,
                        "activity-pins",
                        IconName::Star,
                        "app.pins.title",
                    ),
                ]
                .into_iter()
                .map(|(view, id, icon, key)| {
                    ActivityItem::new(id, icon, crate::i18n::text(key))
                        .active(main == view)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.state.update(cx, |state, cx| state.navigate(view, cx));
                        }))
                }),
            );
        v_flex()
            .id("workbench")
            .size_full()
            .bg(r.container_workspace_bg)
            .text_color(r.text_default)
            .text_style(text::UI)
            .child(self.topbar.clone())
            .children(self.notice(cx))
            .child(
                h_flex()
                    .flex_1()
                    .min_h(px(0.))
                    .child(activity)
                    .child(div().flex_1().min_w(px(0.)).h_full().child(content)),
            )
            .child(self.statusline.clone())
    }
}
