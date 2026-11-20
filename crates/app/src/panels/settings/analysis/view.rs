//! Analysis settings layout using shared native settings rows.
use super::super::rows::{group, notice, row};
use super::{
    AnalysisPage,
    form::{BUILD_FIELDS, ENGINE_FIELDS, TOOLS},
};
use crate::i18n::text as tr;
use autocraft_core::{AnalysisSettingsChange, QuestionsDto};
use autocraft_ui::{
    AutocraftTheme,
    components::label,
    metrics::{space, text},
};
use ca_engines::game::Builder;
use gpui_kit::{
    AnyElement, Context, IntoElement, ParentElement as _, Styled as _, Window,
    assets::IconName,
    base::{h_flex, v_flex},
    component::{
        Disableable as _, Sizable as _,
        button::{Button, ButtonVariants as _},
        input::Input,
        menu::{DropdownMenu as _, PopupMenuItem},
        switch::Switch,
    },
    px,
};

impl AnalysisPage {
    pub(super) fn content(&mut self, _: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let r = AutocraftTheme::global(cx).roles;
        let mut root = v_flex().gap(space::S16).child(label(
            text::TITLE_SECTION,
            tr("app.settings.analysis.title"),
        ));
        let Some(settings) = self.settings.as_ref() else {
            return root
                .child(
                    Button::new("analysis-load")
                        .outline()
                        .label(tr("app.settings.analysis.reload"))
                        .on_click(cx.listener(|this, _, window, cx| this.reload(window, cx))),
                )
                .into_any_element();
        };
        let disabled = self.saving || self.installing.is_some();
        let switches = [
            ("deep_mode", settings.deep_mode),
            ("auto_mode", settings.auto_mode),
            ("ask_before_running", settings.ask_before_running),
            ("runtime_study", settings.runtime_study),
        ];
        let mut defaults = group(
            "analysis-defaults",
            tr("app.settings.analysis.defaults"),
            Some(tr("app.settings.analysis.defaults_description")),
            &r,
        );
        for (key, checked) in switches {
            let label = tr(field_key(key));
            defaults = defaults.child(row(
                label.clone(),
                None,
                Switch::new(key)
                    .checked(checked)
                    .disabled(disabled)
                    .accessibility_label(label)
                    .on_click(cx.listener(move |this, checked: &bool, _, cx| {
                        let change = match key {
                            "deep_mode" => AnalysisSettingsChange {
                                deep_mode: Some(*checked),
                                ..Default::default()
                            },
                            "auto_mode" => AnalysisSettingsChange {
                                auto_mode: Some(*checked),
                                ..Default::default()
                            },
                            "ask_before_running" => AnalysisSettingsChange {
                                ask_before_running: Some(*checked),
                                ..Default::default()
                            },
                            "runtime_study" => AnalysisSettingsChange {
                                runtime_study: Some(*checked),
                                ..Default::default()
                            },
                            _ => return,
                        };
                        this.change(change, cx);
                    })),
                &r,
            ));
        }
        let selected = match settings.questions {
            QuestionsDto::Ask => 0,
            QuestionsDto::Decide => 1,
            QuestionsDto::Default => 2,
        };
        let question_labels = [
            tr("app.settings.analysis.questions_ask"),
            tr("app.settings.analysis.questions_decide"),
            tr("app.settings.analysis.questions_default"),
        ];
        let question_current = question_labels[selected].clone();
        let page = cx.entity();
        defaults = defaults.child(row(
            tr("app.settings.analysis.questions"),
            None,
            Button::new("analysis-questions")
                .outline()
                .small()
                .label(question_current)
                .icon(IconName::ChevronDown)
                .disabled(disabled)
                .dropdown_menu(move |menu, _, _| {
                    question_labels.clone().into_iter().enumerate().fold(
                        menu,
                        |menu, (index, title)| {
                            let page = page.clone();
                            menu.item(
                                PopupMenuItem::new(title)
                                    .checked(index == selected)
                                    .on_click(move |_, _, cx| {
                                        let questions = match index {
                                            0 => QuestionsDto::Ask,
                                            1 => QuestionsDto::Decide,
                                            2 => QuestionsDto::Default,
                                            _ => return,
                                        };
                                        page.update(cx, |this, cx| {
                                            this.change(
                                                AnalysisSettingsChange {
                                                    questions: Some(questions),
                                                    ..Default::default()
                                                },
                                                cx,
                                            );
                                        });
                                    }),
                            )
                        },
                    )
                }),
            &r,
        ));
        root = root.child(defaults);
        let inventory = self.inventory.as_ref();
        let mut game = group(
            "analysis-game-engines",
            tr("app.settings.analysis.game_engines"),
            Some(tr("app.settings.analysis.game_engines_description")),
            &r,
        );
        for builder in Builder::ALL {
            let installed = inventory.is_some_and(|inv| inv.best(builder).is_some());
            let ready = inventory.is_some_and(|inv| inv.ready(builder));
            let detail = if ready {
                format!(
                    "{} · {}",
                    tr("app.settings.analysis.engine_ready"),
                    builder.needs()
                )
            } else if installed && builder == Builder::Unreal {
                format!(
                    "{} · {}",
                    tr("app.settings.analysis.engine_cpp_missing"),
                    builder.needs()
                )
            } else {
                format!(
                    "{} · {}",
                    tr("app.settings.analysis.engine_missing"),
                    builder.needs()
                )
            };
            let mut control = h_flex().gap(space::S8).child(label(text::UI, detail));
            if builder == Builder::Rust {
                control = control.child(
                    Button::new("engine-rust-page")
                        .outline()
                        .small()
                        .label(tr("app.settings.analysis.engine_open_guide"))
                        .on_click(cx.listener(|_, _, _, cx| {
                            cx.open_url(ca_engines::game::download_page(Builder::Rust))
                        })),
                );
            } else {
                let engine = engine_for_install(builder);
                let action = if ready {
                    tr("app.settings.analysis.engine_refresh")
                } else {
                    tr("app.settings.analysis.engine_install")
                };
                control = control.child(
                    Button::new(format!("install-game-engine-{engine}"))
                        .outline()
                        .small()
                        .disabled(disabled)
                        .label(action)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            if ready {
                                this.reload(window, cx);
                            } else {
                                this.install(engine, window, cx);
                            }
                        })),
                );
            }
            if self.focused_engine == Some(builder) {
                control = control.child(label(
                    text::METADATA,
                    tr("app.settings.analysis.engine_selected"),
                ));
            }
            game = game.child(row(builder.name(), None, control, &r));
        }
        game = game.child(
            Button::new("refresh-game-engines")
                .outline()
                .small()
                .disabled(self.refreshing || disabled)
                .label(tr("app.settings.analysis.refresh_engines"))
                .on_click(cx.listener(|this, _, window, cx| this.reload(window, cx))),
        );
        root = root.child(game);
        let mut engines = group(
            "analysis-engines",
            tr("app.settings.analysis.engines"),
            Some(tr("app.settings.analysis.optional_overrides")),
            &r,
        );
        for &key in ENGINE_FIELDS.iter().chain(TOOLS) {
            let title = if TOOLS.contains(&key) {
                key.to_owned()
            } else {
                tr(field_key(key)).into_owned()
            };
            engines = engines.child(row(
                title.clone(),
                None,
                Input::new(&self.fields.0[key])
                    .small()
                    .w(px(280.))
                    .disabled(disabled)
                    .aria_label(title),
                &r,
            ));
        }
        let mut install = h_flex().flex_wrap().gap(space::S8);
        for engine in ["ghidra", "godot", "adb", "jadx", "vineflower"] {
            install = install.child(
                Button::new(format!("install-{engine}"))
                    .outline()
                    .small()
                    .disabled(disabled)
                    .label(
                        rust_i18n::t!("app.settings.analysis.install_engine", engine = engine)
                            .into_owned(),
                    )
                    .on_click(
                        cx.listener(move |this, _, window, cx| this.install(engine, window, cx)),
                    ),
            );
        }
        engines = engines
            .child(notice(
                tr("app.settings.analysis.install_consent").into_owned(),
                false,
                &r,
            ))
            .child(install);
        if let Some(cancel) = self.installing.clone() {
            engines = engines.child(
                Button::new("cancel-engine-install")
                    .outline()
                    .label(tr("app.settings.analysis.cancel_install"))
                    .on_click(move |_, _, _| cancel.cancel()),
            );
        }
        if self.installed {
            engines = engines.child(notice(
                tr("app.settings.analysis.installed").into_owned(),
                false,
                &r,
            ));
        }
        root = root.child(engines);
        let mut build = group(
            "analysis-build",
            tr("app.settings.analysis.build"),
            Some(tr("app.settings.analysis.build_description")),
            &r,
        );
        for &key in BUILD_FIELDS {
            let title = tr(field_key(key));
            build = build.child(row(
                title.clone(),
                None,
                Input::new(&self.fields.0[key])
                    .small()
                    .w(px(220.))
                    .disabled(disabled)
                    .aria_label(title),
                &r,
            ));
        }
        root = root.child(build);
        if let Some(error) = self.error {
            root = root.child(notice(tr(error).into_owned(), true, &r));
        }
        root.child(
            Button::new("analysis-save")
                .primary()
                .disabled(disabled)
                .label(tr("app.settings.analysis.save"))
                .on_click(cx.listener(|this, _, _, cx| this.save_fields(cx))),
        )
        .child(self.assets.clone())
        .into_any_element()
    }
}

fn field_key(key: &'static str) -> &'static str {
    match key {
        "deep_mode" => "app.settings.analysis.deep_mode",
        "auto_mode" => "app.settings.analysis.auto_mode",
        "ask_before_running" => "app.settings.analysis.ask_before_running",
        "runtime_study" => "app.settings.analysis.runtime_study",
        "ghidra_dir" => "app.settings.analysis.ghidra_dir",
        "java_home" => "app.settings.analysis.java_home",
        "startup_timeout" => "app.settings.analysis.startup_timeout",
        "language_id" => "app.settings.analysis.language_id",
        "compiler_spec" => "app.settings.analysis.compiler_spec",
        "memory_gb" => "app.settings.analysis.memory_gb",
        "store_dir" => "app.settings.analysis.store_dir",
        "good_enough" => "app.settings.analysis.good_enough",
        "max_rounds" => "app.settings.analysis.max_rounds",
        "web_widths" => "app.settings.analysis.web_widths",
        "max_pages" => "app.settings.analysis.max_pages",
        "picture_good_enough" => "app.settings.analysis.picture_good_enough",
        "parallel" => "app.settings.analysis.parallel",
        other => other,
    }
}

fn engine_for_install(builder: Builder) -> &'static str {
    match builder {
        Builder::Unity => "unity",
        Builder::Unreal => "unreal",
        Builder::Godot => "godot",
        Builder::Rust => "rust",
    }
}
