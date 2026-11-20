//! Toolchain builds, native game-engine recipes, and declared tests.
use super::*;
use ca_engine::toolchains::{self, Toolchain};
use ca_engines::game::Builder;

pub struct BuildCheck {
    pub own: bool,
}
impl Check for BuildCheck {
    fn id(&self) -> &'static str {
        CheckResult::BUILD
    }
    fn run(&self, cx: &mut CheckCx<'_>) -> Result<CheckResult> {
        cx.stop_now()?;
        let app = cx.ws.app_dir();
        let recipe = match recipe(&app, &cx.ws.manifest.stack, cx.editor(), self.own) {
            Ok(r) => r,
            Err(note) => return Ok(error_result(self.id(), cx.round, note)),
        };
        let mut diagnostics = Vec::new();
        for (program, args, limit) in recipe.commands {
            let out = process::run(cx, &program, &args, limit)?;
            let text = format!("{}\n{}", out.stdout_text(), out.stderr_text());
            if text.contains("No valid Unity Editor license") {
                return Ok(error_result(
                    self.id(),
                    cx.round,
                    "Unity 需要许可证：请在 Unity Hub 登录并激活 Personal 许可证后重试。",
                ));
            }
            diagnostics.push(process::diagnostic(cx, &out));
            if !out.build_succeeded(recipe.godot) {
                return Ok(error_result(
                    self.id(),
                    cx.round,
                    format!(
                        "构建失败（代码 {:?}）。\n{}",
                        out.code,
                        diagnostics.join("\n")
                    ),
                ));
            }
        }
        if recipe.packaged && ca_engine::outputs::packaged_program(&app.join("Build")).is_none() {
            return Ok(error_result(
                self.id(),
                cx.round,
                "引擎命令完成，但 Build/ 中没有可运行的打包程序。",
            ));
        }
        Ok(passed_result(self.id(), cx.round, recipe.note))
    }
}

struct Recipe {
    commands: Vec<(PathBuf, Vec<String>, Duration)>,
    godot: bool,
    packaged: bool,
    note: String,
}
fn uproject(app: &Path) -> Option<PathBuf> {
    fs::read_dir(app)
        .ok()?
        .filter_map(std::result::Result::ok)
        .take(256)
        .map(|e| e.path())
        .find(|p| {
            p.extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("uproject"))
        })
}
pub(super) fn actual_engine(app: &Path) -> Option<Builder> {
    if app.join("project.godot").is_file() {
        Some(Builder::Godot)
    } else if app.join("Assets").is_dir() && app.join("ProjectSettings").is_dir() {
        Some(Builder::Unity)
    } else if uproject(app).is_some() {
        Some(Builder::Unreal)
    } else {
        None
    }
}
fn recipe(
    app: &Path,
    stack: &str,
    editor: Option<&Path>,
    own: bool,
) -> std::result::Result<Recipe, String> {
    let tool = toolchains::found_in(app);
    let engine = actual_engine(app).or_else(|| {
        if tool.is_some() || ca_engine::outputs::site_dir(app).is_some() {
            None
        } else {
            Builder::from_stack(stack).filter(|b| *b != Builder::Rust)
        }
    });
    let mut recipe = Recipe {
        commands: Vec::new(),
        godot: false,
        packaged: false,
        note: "构建成功。".into(),
    };
    if let Some(engine) = engine {
        if actual_engine(app) != Some(engine) {
            return Err("未找到所选游戏引擎的项目文件，不能仅凭现有文件通过构建。".into());
        }
        let Some(editor) = editor else {
            return Err("游戏引擎编辑器不可用，请先在引擎设置中准备编辑器。".into());
        };
        let root = std::path::absolute(app).map_err(|_| "项目路径无效。".to_string())?;
        let root = root.display().to_string();
        match engine {
            Builder::Godot => {
                recipe.godot = true;
                recipe.commands.push((
                    editor.into(),
                    vec![
                        "--headless".into(),
                        "--path".into(),
                        root.clone(),
                        "--import".into(),
                    ],
                    Duration::from_secs(60 * 60),
                ));
                recipe.commands.push((
                    editor.into(),
                    vec![
                        "--headless".into(),
                        "--path".into(),
                        root.clone(),
                        "--quit-after".into(),
                        "120".into(),
                    ],
                    Duration::from_secs(10 * 60),
                ));
                // Presets alone do not request packaging; the prepared editor runs the project.
                recipe.note = "Godot 导入与无界面运行检查通过；运行项目使用已准备的编辑器。".into();
            }
            Builder::Unity => {
                let mut args = vec![
                    "-batchmode".into(),
                    "-quit".into(),
                    "-nographics".into(),
                    "-projectPath".into(),
                    root,
                ];
                if !own {
                    args.extend(["-executeMethod".into(), "CraftBuild.Build".into()]);
                    recipe.packaged = true;
                }
                args.extend(["-logFile".into(), "-".into()]);
                recipe
                    .commands
                    .push((editor.into(), args, Duration::from_secs(4 * 60 * 60)));
            }
            Builder::Unreal => {
                let engine_root = editor
                    .ancestors()
                    .nth(4)
                    .ok_or("无法定位 Unreal 构建工具。")?;
                let uat = engine_root
                    .join("Engine/Build/BatchFiles")
                    .join(if cfg!(windows) {
                        "RunUAT.bat"
                    } else {
                        "RunUAT.sh"
                    });
                if !uat.is_file() {
                    return Err("找不到 Unreal RunUAT 构建工具。".into());
                }
                let project = uproject(app).ok_or("找不到 Unreal 项目文件。")?;
                let platform = if cfg!(windows) {
                    "Win64"
                } else if cfg!(target_os = "macos") {
                    "Mac"
                } else {
                    "Linux"
                };
                let args = vec![
                    "BuildCookRun".into(),
                    format!("-project={}", project.display()),
                    format!("-platform={platform}"),
                    "-clientconfig=Development".into(),
                    "-build".into(),
                    "-cook".into(),
                    "-stage".into(),
                    "-pak".into(),
                    "-archive".into(),
                    format!("-archivedirectory={}", app.join("Build").display()),
                    "-unattended".into(),
                    "-utf8output".into(),
                    "-nop4".into(),
                ];
                recipe
                    .commands
                    .push((uat, args, Duration::from_secs(8 * 60 * 60)));
                recipe.packaged = true;
            }
            Builder::Rust => {}
        }
    } else if let Some(tool) = tool {
        if tool.build.is_empty() {
            recipe.note = format!("{} 未声明构建命令，没有运行构建。", tool.name);
        } else {
            let (program, args) = tool
                .command(app, &tool.build)
                .ok_or("工具链没有可执行的构建命令。")?;
            recipe
                .commands
                .push((program, args, Duration::from_secs(2 * 60 * 60)));
        }
    } else if ca_engine::outputs::site_dir(app).is_some_and(|site| {
        fs::read(site.join("index.html")).is_ok_and(|b| b.iter().any(|b| !b.is_ascii_whitespace()))
    }) {
        recipe.note = "静态网站无需构建。".into();
    } else if own {
        recipe.note = "未运行构建：尚不支持此项目的构建工具。".into();
    } else {
        return Err("未找到可构建的项目或有效静态网站。".into());
    }
    Ok(recipe)
}

pub struct TestsCheck;
impl Check for TestsCheck {
    fn id(&self) -> &'static str {
        "own-tests"
    }
    fn run(&self, cx: &mut CheckCx<'_>) -> Result<CheckResult> {
        cx.stop_now()?;
        let app = cx.ws.app_dir();
        let Some(tool) = toolchains::found_in(&app) else {
            return Ok(passed_result(
                self.id(),
                cx.round,
                "没有运行测试：项目工具链未知。",
            ));
        };
        if no_declared_tests(&app, tool) {
            return Ok(passed_result(
                self.id(),
                cx.round,
                "没有运行测试：项目未声明测试，或只有占位测试脚本。",
            ));
        }
        let Some((program, args)) = tool.command(&app, &tool.test) else {
            return Ok(error_result(self.id(), cx.round, "测试命令不可用。"));
        };
        let out = process::run(cx, &program, &args, Duration::from_secs(30 * 60))?;
        let text = format!("{}\n{}", out.stdout_text(), out.stderr_text());
        let none = out.code.is_some()
            && !out.stdout_truncated
            && !out.stderr_truncated
            && tool.test_script.is_none()
            && tool.no_tests.iter().any(|s| text.contains(s));
        Ok(CheckResult::new(
            self.id(),
            cx.round,
            out.success || none,
            if none {
                "没有运行测试：测试工具报告没有测试。".into()
            } else {
                format!(
                    "测试{}（代码 {:?}）。\n{}",
                    if out.success { "通过" } else { "失败" },
                    out.code,
                    process::diagnostic(cx, &out)
                )
            },
        ))
    }
}
fn no_declared_tests(app: &Path, tool: &Toolchain) -> bool {
    tool.test.is_empty()
        || tool.test_script.as_ref().is_some_and(|name| {
            ca_engine::outputs::npm_script(app, name)
                .is_none_or(|s| s.trim().is_empty() || tool.no_tests.iter().any(|n| n == s.trim()))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn real_test_script_is_not_confused_with_child_no_tests_message() {
        let d = tempfile::tempdir().unwrap();
        fs::write(
            d.path().join("package.json"),
            r#"{"scripts":{"test":"node test.js"}}"#,
        )
        .unwrap();
        let tool = toolchains::found_in(d.path()).unwrap();
        assert!(!no_declared_tests(d.path(), tool));
        fs::write(
            d.path().join("package.json"),
            r#"{"scripts":{"test":"echo \"Error: no test specified\" && exit 1"}}"#,
        )
        .unwrap();
        assert!(no_declared_tests(d.path(), tool));
    }
    #[test]
    fn godot_presets_do_not_require_unprepared_export_templates() {
        let d = tempfile::tempdir().unwrap();
        fs::write(d.path().join("project.godot"), "config_version=5").unwrap();
        fs::write(
            d.path().join("export_presets.cfg"),
            "[preset.0]\nname=\"Desktop\"",
        )
        .unwrap();
        let editor = d.path().join("godot-editor");
        let plan = recipe(d.path(), "Godot", Some(&editor), false).unwrap();
        assert!(!plan.packaged);
        assert!(!d.path().join("Build").exists());
        assert_eq!(
            ca_engine::outputs::launch(d.path(), Some(&editor)),
            Some((
                editor,
                vec![
                    "--path".into(),
                    std::path::absolute(d.path()).unwrap().display().to_string()
                ],
            ))
        );
    }
}
