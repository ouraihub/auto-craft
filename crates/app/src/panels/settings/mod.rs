//! Settings dialog navigation and page lifetimes.

mod analysis;
mod appearance;
mod assets;
mod coding;
mod language;
mod locations;
mod logs;
mod mcp;
mod models;
mod providers;
mod rows;

use crate::state::AppState;
use autocraft_core::BackendKind;
use autocraft_ui::{
    AutocraftTheme,
    components::label,
    metrics::{content, radius, size, space, text},
};
use gpui_kit::{
    App, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, StatefulInteractiveElement as _, Styled as _, Window,
    assets::IconName,
    base::{h_flex, v_flex},
    component::{
        Icon, WindowExt as _,
        button::Button,
        menu::{DropdownMenu as _, PopupMenuItem},
        scroll::ScrollableElement as _,
    },
    prelude::FluentBuilder as _,
    px,
};
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Page {
    Models,
    Analysis,
    Locations,
    Providers,
    Coding,
    Mcp,
    Logs,
    Appearance,
    Language,
}

/// One sidebar entry: the page, its icon and its title key.
type NavItem = (Page, IconName, &'static str);

/// Sidebar groups in display order; the narrow page menu lists the same pages in the same order.
/// A group without a title key is the Agent group, whose title is not translated.
const NAV: [(Option<&str>, &[NavItem]); 4] = [
    (
        None,
        &[
            (Page::Models, IconName::Brain, "app.settings.models.title"),
            (
                Page::Providers,
                IconName::KeyRound,
                "app.settings.providers.title",
            ),
            (
                Page::Coding,
                IconName::SquareTerminal,
                "app.settings.coding.title",
            ),
            (Page::Mcp, IconName::Plug, "app.settings.mcp.title"),
        ],
    ),
    (
        Some("app.settings.groups.analysis"),
        &[(
            Page::Analysis,
            IconName::Microscope,
            "app.settings.analysis.title",
        )],
    ),
    (
        Some("app.settings.groups.interface"),
        &[
            (
                Page::Appearance,
                IconName::Palette,
                "app.settings.appearance.title",
            ),
            (
                Page::Language,
                IconName::Languages,
                "app.settings.language.title",
            ),
        ],
    ),
    (
        Some("app.settings.groups.system"),
        &[
            (
                Page::Locations,
                IconName::FolderOpen,
                "app.settings.locations.title",
            ),
            (Page::Logs, IconName::ScrollText, "app.settings.logs.title"),
        ],
    ),
];

pub struct SettingsDialog;

struct SettingsView {
    page: Page,
    models: Entity<models::ModelsPage>,
    providers: Entity<providers::ProvidersPage>,
    coding: Entity<coding::CodingPage>,
    mcp: Entity<mcp::McpPage>,
    /// Polls the log file only while it is the page on screen; closing settings drops it.
    logs: Entity<logs::LogsPage>,
    appearance: Entity<appearance::AppearancePage>,
    language: Entity<language::LanguagePage>,
    analysis: Entity<analysis::AnalysisPage>,
    locations: Entity<locations::LocationsPage>,
    _coding: gpui_kit::Subscription,
}

impl SettingsDialog {
    pub fn open(state: Entity<AppState>, window: &mut Window, cx: &mut App) {
        Self::show(state, Page::Models, BackendKind::Pi, None, window, cx);
    }
    pub fn open_coding(
        state: Entity<AppState>,
        backend: BackendKind,
        window: &mut Window,
        cx: &mut App,
    ) {
        Self::show(state, Page::Coding, backend, None, window, cx);
    }

    pub fn open_providers(state: Entity<AppState>, window: &mut Window, cx: &mut App) {
        Self::show(state, Page::Providers, BackendKind::Pi, None, window, cx);
    }
    pub fn open_analysis(state: Entity<AppState>, window: &mut Window, cx: &mut App) {
        Self::show(state, Page::Analysis, BackendKind::Pi, None, window, cx);
    }

    pub fn open_analysis_engine(
        state: Entity<AppState>,
        engine: &'static str,
        window: &mut Window,
        cx: &mut App,
    ) {
        Self::show(
            state,
            Page::Analysis,
            BackendKind::Pi,
            Some(engine),
            window,
            cx,
        );
    }

    fn show(
        state: Entity<AppState>,
        page: Page,
        coding_backend: BackendKind,
        engine: Option<&'static str>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let models = cx.new(|cx| models::ModelsPage::new(state.clone(), window, cx));
        let providers = cx.new(|cx| providers::ProvidersPage::new(state.clone(), window, cx));
        let coding =
            cx.new(|cx| coding::CodingPage::new(state.clone(), coding_backend, window, cx));
        let mcp = cx.new(|cx| mcp::McpPage::new(state.clone(), window, cx));
        let logs = cx.new(|cx| logs::LogsPage::new(state.clone(), window, cx));
        let appearance = cx.new(|cx| appearance::AppearancePage::new(window, cx));
        let language = cx.new(|cx| language::LanguagePage::new(state.clone(), cx));
        let analysis = cx.new(|cx| analysis::AnalysisPage::new(state.clone(), window, cx));
        if let Some(engine) = engine {
            analysis.update(cx, |page, cx| page.focus_engine(engine, cx));
        }
        let locations = cx.new(|cx| locations::LocationsPage::new(state.clone(), window, cx));
        let view = cx.new(|cx| {
            // 编码搭档's footer link: jump to 模型服务.
            let watch = cx.observe(&coding, |this: &mut SettingsView, coding, cx| {
                if coding.update(cx, |page, _| std::mem::take(&mut page.open_providers)) {
                    this.page = Page::Providers;
                    cx.notify();
                }
            });
            SettingsView {
                page,
                models,
                providers,
                coding,
                mcp,
                logs,
                appearance,
                language,
                analysis,
                locations,
                _coding: watch,
            }
        });
        // Enter inside a field must not close settings: the dialog has no OK action.
        window.open_dialog(cx, move |dialog, window, _| {
            dialog
                .title(crate::i18n::text("app.settings.title"))
                .w(px(1040.).min(window.viewport_size().width - space::S32))
                .on_ok(|_, _, _| false)
                .child(view.clone())
        });
    }
}

impl SettingsView {
    fn nav(
        &self,
        page: Page,
        icon: IconName,
        title: std::borrow::Cow<'static, str>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let active = self.page == page;
        let r = AutocraftTheme::global(cx).roles;
        let id = match page {
            Page::Models => "settings-nav-models",
            Page::Analysis => "settings-nav-analysis",
            Page::Locations => "settings-nav-locations",
            Page::Providers => "settings-nav-providers",
            Page::Coding => "settings-nav-coding",
            Page::Mcp => "settings-nav-mcp",
            Page::Logs => "settings-nav-logs",
            Page::Appearance => "settings-nav-appearance",
            Page::Language => "settings-nav-language",
        };
        v_flex()
            .id(id)
            .h(px(28.))
            .px(space::S8)
            .rounded(radius::SM)
            .justify_center()
            .when(active, |d| d.bg(r.primary_subtle))
            .when(!active, |d| {
                d.hover(|s| s.bg(r.control_bg_hovered)).cursor_pointer()
            })
            .child(
                h_flex()
                    .gap(space::S8)
                    .child(Icon::new(icon).size(size::ICON_SMALL))
                    .child(label(text::UI, title).text_color(if active {
                        r.primary
                    } else {
                        r.text_default
                    })),
            )
            .when(!active, |d| {
                d.on_click(cx.listener(move |this, _, window, cx| {
                    if this.page == Page::Providers {
                        this.providers
                            .update(cx, |view, cx| view.clear_sensitive(window, cx));
                    }
                    this.logs
                        .update(cx, |logs, cx| logs.set_active(page == Page::Logs, cx));
                    this.page = page;
                    cx.notify();
                }))
            })
    }
    fn page_menu(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let current = self.page;
        let page = cx.entity();
        let title = match current {
            Page::Models => crate::i18n::text("app.settings.models.title"),
            Page::Analysis => crate::i18n::text("app.settings.analysis.title"),
            Page::Locations => crate::i18n::text("app.settings.locations.title"),
            Page::Providers => crate::i18n::text("app.settings.providers.title"),
            Page::Coding => crate::i18n::text("app.settings.coding.title"),
            Page::Mcp => crate::i18n::text("app.settings.mcp.title"),
            Page::Logs => crate::i18n::text("app.settings.logs.title"),
            Page::Appearance => crate::i18n::text("app.settings.appearance.title"),
            Page::Language => crate::i18n::text("app.settings.language.title"),
        };
        Button::new("settings-page-choice")
            .outline()
            .label(title)
            .icon(IconName::ChevronDown)
            .dropdown_menu(move |menu, _, _| {
                NAV.iter().flat_map(|(_, pages)| pages.iter()).fold(
                    menu,
                    |menu, &(value, _, key)| {
                        let page = page.clone();
                        menu.item(
                            PopupMenuItem::new(crate::i18n::text(key))
                                .checked(current == value)
                                .on_click(move |_, _, cx| {
                                    page.update(cx, |this, cx| {
                                        this.page = value;
                                        cx.notify();
                                    })
                                }),
                        )
                    },
                )
            })
    }
}

impl Render for SettingsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let r = AutocraftTheme::global(cx).roles;
        let narrow = window.viewport_size().width < lumakit_design::metrics::size::NARROW;
        let content = match self.page {
            Page::Models => self.models.clone().into_any_element(),
            Page::Analysis => self.analysis.clone().into_any_element(),
            Page::Locations => self.locations.clone().into_any_element(),
            Page::Providers => self.providers.clone().into_any_element(),
            Page::Coding => self.coding.clone().into_any_element(),
            Page::Mcp => self.mcp.clone().into_any_element(),
            Page::Logs => self.logs.clone().into_any_element(),
            Page::Appearance => self.appearance.clone().into_any_element(),
            Page::Language => self.language.clone().into_any_element(),
        };
        h_flex()
            .id("settings-shell")
            .w_full()
            .min_w(px(0.))
            .when(narrow, |d| d.flex_col())
            .items_start()
            .h(window.viewport_size().height * 0.8 - px(72.))
            .min_h(px(0.))
            .bg(r.container_content_bg)
            .child(
                v_flex()
                    .when(narrow, |d| d.w_full().h(px(52.)).flex_row().items_center())
                    .when(!narrow, |d| d.w(px(192.)).h_full())
                    .flex_none()
                    .gap(space::S4)
                    .p(space::S12)
                    .bg(r.container_elevated_bg)
                    .when(narrow, |d| d.border_b_1().child(self.page_menu(cx)))
                    .when(!narrow, |mut d| {
                        d = d.border_r_1();
                        for (index, (group, pages)) in NAV.iter().enumerate() {
                            let title = group.map_or("AGENT".into(), crate::i18n::text);
                            d = d.child(
                                label(text::LABEL_PILL, title)
                                    .h(px(28.))
                                    .when(index > 0, |l| l.mt(space::S8))
                                    .text_color(r.text_subtle),
                            );
                            for (page, icon, key) in pages.iter() {
                                d = d.child(self.nav(*page, *icon, crate::i18n::text(key), cx));
                            }
                        }
                        d
                    }),
            )
            .child(
                v_flex()
                    .id("settings-content")
                    .flex_1()
                    .when(!narrow, |d| d.h_full())
                    .w_full()
                    .min_w(px(0.))
                    .min_h(px(0.))
                    .overflow_y_scrollbar()
                    .p(space::S16)
                    .child(v_flex().w_full().max_w(content::SETTINGS).child(content)),
            )
    }
}
