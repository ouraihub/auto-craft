//! Reference: craft-anything crates/ai/src/rebuild/study.rs (300dba58).
//! Cancellable target study and capture, respecting the run's immutable clean-room mode.
use crate::{
    RebuildConfig, TargetKind,
    kinds::{self, website},
    run::{Run, source},
};
use autocraft_core::*;
use ca_engine::workspace::{Status, TargetRef, Workspace};
use serde_json::{Value, json};
use std::{fs, path::Path};

pub(crate) struct Study<'a> {
    pub run: &'a Run,
    pub config: &'a RebuildConfig,
    pub settings: &'a ca_engine::Settings,
    pub deep: bool,
}
impl Study<'_> {
    fn call(&self, tool: &str, params: Value) -> Result<ToolReply> {
        self.run.cancelled()?;
        self.config.analysis.call(
            &self.run.project,
            ToolCall {
                tool: tool.into(),
                params,
                consented: false,
                caller: CallerDto::Rebuild,
            },
            &self.run.cancel,
        )
    }
    fn step(&self, ws: &mut Workspace, text: &str) -> Result<()> {
        self.run.step(ws, StepDto::Study, Status::Inspecting, text)
    }
    fn optional(&self, tool: &str, params: Value) -> Result<String> {
        match self.call(tool, params) {
            Ok(reply) => describe(reply),
            Err(e) if e.kind() == Some(ErrorKind::Cancelled) => Err(e),
            Err(e) => Ok(format!("此项研究未完成（{}）。\n", e.code())),
        }
    }
    pub fn reopen(&self, ws: &Workspace, kind: &dyn TargetKind) -> Result<()> {
        if !self.deep {
            return Ok(());
        }
        if let TargetRef::Local { path } = &ws.manifest.target {
            let (program, _) = kinds::locate(path, kind.studies_folder());
            if program.exists() {
                self.optional("core_target_open", json!({"path":program}))?;
            }
        }
        Ok(())
    }
    fn approve_console(&self, ws: &Workspace) -> Result<()> {
        use ca_engine::inbox::{Inbox, RequestKind};
        self.run.cancelled()?;
        let inbox = Inbox::new(ws.inbox_dir());
        let id = inbox.post(
            RequestKind::Approval,
            "允许观察原程序的命令行输出？",
            "刚才未检测到可见窗口。接下来会另外运行原程序三次：无参数、--help 和 --version，并记录输出与退出状态。每次最多等待 8 秒；不会读取或哈希原程序文件。",
            Vec::new(),
            false,
            None,
        ).map_err(source)?;
        let answer = inbox.wait(&id, std::time::Duration::from_secs(30 * 60), &|| {
            self.run.cancel.is_cancelled()
        });
        self.run.cancelled()?;
        match answer {
            None => Err(Error::timeout("命令行观察确认已超时")),
            Some(answer) if answer.allow == Some(true) => Ok(()),
            Some(_) => Err(Error::new(ErrorKind::ConsentRequired, "命令行观察未获批准")),
        }
    }
    pub fn study(&self, ws: &mut Workspace, kind: &dyn TargetKind) -> Result<()> {
        // A failed replacement must not leave a previous study looking complete.
        let capture = ws.capture_dir();
        if capture.exists() {
            fs::remove_dir_all(&capture).map_err(|e| Error::Io {
                context: "无法替换研究记录".into(),
                source: e,
            })?;
        }
        match ws.manifest.target.clone() {
            TargetRef::Website { url } => self.website(ws, &url),
            TargetRef::Local { path } => self.local(ws, &path, kind),
            TargetRef::None => Ok(()),
        }
    }
    fn website(&self, ws: &mut Workspace, url: &str) -> Result<()> {
        self.step(ws, "正在打开原网站…")?;
        self.call("web_site_open", json!({"url":url}))?;
        let capture = ws.capture_dir();
        self.step(ws, "正在拍摄页面并读取布局…")?;
        let widths = self.settings.build.widths();
        self.call(
            "web_site_capture",
            json!({"output_directory":capture,"widths":widths,"noise_pass":true}),
        )?;
        self.step(ws, "正在查看菜单、弹窗和页面状态…")?;
        let mut views = vec![(
            website::STATES_DIR.into(),
            widths.first().copied().unwrap_or(1440),
        )];
        if let Some(w) = website::phone_width(&widths) {
            views.push((website::phone_states_dir(w), w));
        }
        for (dir, width) in views {
            match self.call(
                "web_page_states_capture",
                json!({"output_directory":capture.join(dir),"width":width}),
            ) {
                Ok(_) => {}
                Err(e) if e.kind() == Some(ErrorKind::Cancelled) => return Err(e),
                Err(_) => self.step(ws, "无法记录一个页面状态，已继续其他研究。")?,
            }
        }
        let max = self.settings.build.max_pages.min(website::MAX_ROUTES);
        let first = read_capture(&capture)?;
        let picked = website::pick_routes(&first, url, max);
        let mut kept = self.pages(ws, &picked, &widths)?;
        if kept.len() < max {
            let captures = kept
                .iter()
                .map(|r| read_capture(&capture.join("routes").join(&r.slug)))
                .collect::<Result<Vec<_>>>()?;
            let more = website::pick_deeper_routes(&captures, url, &kept, max - kept.len());
            kept.extend(self.pages(ws, &more, &widths)?);
        }
        if !kept.is_empty() {
            write(
                &capture.join(website::ROUTES_FILE),
                &serde_json::to_vec_pretty(&kept).map_err(|_| Error::corrupt("页面记录无效"))?,
            )?;
        }
        if self.deep {
            self.step(ws, "正在研究网页脚本与运行行为…")?;
            self.call("web_site_open", json!({"url":url}))?;
            let mut text = String::from("# How it works\n\n");
            text.push_str(&self.optional("web_page_js_analyze", json!({"load_source_maps":true}))?);
            text.push_str(&self.optional(
                "web_page_watch",
                json!({"watch_ms":10000,"with_websocket_outlines":true}),
            )?);
            write(&capture.join("how-it-works.md"), text.as_bytes())?;
        }
        write(
            &capture.join("README.md"),
            b"# Website study\n\nSee capture.json, page pictures and captured states.\n",
        )
    }
    fn pages(
        &self,
        ws: &mut Workspace,
        routes: &[website::Route],
        widths: &[u32],
    ) -> Result<Vec<website::Route>> {
        let mut kept = Vec::new();
        if !routes.is_empty() {
            fs::create_dir_all(ws.capture_dir().join("routes")).map_err(|e| Error::Io {
                context: "无法创建页面研究目录".into(),
                source: e,
            })?;
        }
        for route in routes {
            self.step(ws, "正在拍摄其他页面…")?;
            let result=self.call("web_site_open",json!({"url":route.url})).and_then(|_| self.call("web_site_capture",json!({"output_directory":ws.capture_dir().join("routes").join(&route.slug),"include_html":false,"widths":widths,"noise_pass":true})));
            match result {
                Ok(_) => kept.push(route.clone()),
                Err(e) if e.kind() == Some(ErrorKind::Cancelled) => return Err(e),
                Err(_) => self.step(ws, "一个页面无法拍摄，已记录并继续其他页面。")?,
            }
        }
        Ok(kept)
    }
    fn local(&self, ws: &mut Workspace, path: &Path, kind: &dyn TargetKind) -> Result<()> {
        self.step(
            ws,
            if self.deep {
                "正在静态研究原程序…"
            } else {
                "净室模式：不读取原程序内部…"
            },
        )?;
        let capture = ws.capture_dir();
        fs::create_dir_all(&capture).map_err(|e| Error::Io {
            context: "无法创建研究目录".into(),
            source: e,
        })?;
        let mut notes = String::new();
        let program = if self.deep {
            let (program, folder) = kinds::locate(path, kind.studies_folder());
            notes.push_str(&self.optional("core_target_triage", json!({"path":program}))?);
            let js_root = if path.is_dir() {
                Some(path)
            } else {
                program.parent()
            };
            if path.extension().is_some_and(|e| e == "asar")
                || js_root.is_some_and(|p| {
                    p.join("resources/app.asar").exists() || p.join("package.json").exists()
                })
            {
                notes.push_str(&self.optional(
                    "js_app_map",
                    json!({"input_path":js_root.unwrap_or(path),"limit":50}),
                )?);
            }
            if let Some(folder) = folder {
                if let Some(found) = ca_engines::game::detect(&folder) {
                    notes.push_str(&format!(
                        "\n## Made with {}\n{}\n",
                        found.label(),
                        found.clue
                    ));
                }
                notes.push_str(&inventory(&folder)?);
            }
            if program.is_file()
                && !program
                    .parent()
                    .is_some_and(|p| p.join("resources/app.asar").is_file())
                && self.settings.ghidra_launcher().is_some()
            {
                self.step(ws, "正在准备反编译分析…")?;
                notes.push_str(&self.optional("core_target_open", json!({"path":program}))?);
                let bytes = fs::read(&program).map_err(|e| Error::Io {
                    context: "无法读取反编译目标".into(),
                    source: e,
                })?;
                let sha = ca_evidence::sha256_hex(&bytes);
                let settings = self.settings.clone();
                let (tx, rx) = std::sync::mpsc::sync_channel(1);
                let span = tracing::Span::current();
                std::thread::Builder::new()
                    .name("rebuild-ghidra".into())
                    .spawn(move || {
                        let _entered = span.enter();
                        let result =
                            ca_engine::tools::deep::prepare(&settings, &bytes, &sha, &|_| {});
                        if tx.send(result.is_ok()).is_err() {
                            tracing::debug!("rebuild.ghidra.result_detached");
                        }
                    })
                    .map_err(|e| Error::Io {
                        context: "无法启动反编译准备".into(),
                        source: e,
                    })?;
                let prepared = loop {
                    self.run.cancelled()?;
                    match rx.recv_timeout(std::time::Duration::from_millis(120)) {
                        Ok(ok) => break ok,
                        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                            return Err(Error::external("反编译准备意外停止"));
                        }
                    }
                };
                notes.push_str(if prepared {"\nGhidra analysis is ready for the builder.\n"}else{"\nGhidra preparation failed; use the available built-in decompiler and record unknowns.\n"});
            }
            program
        } else {
            notes.push_str("# Clean room\n\nNo original files, folder or code were read. Build from the request, reference pictures and visible behavior.\n");
            path.to_path_buf()
        };
        if self.settings.runtime_study
            && program.is_file()
            && ca_engine::outputs::is_program(&program)
        {
            self.step(ws, "正在运行原程序并记录窗口…")?;
            // Native capture observes only visible behavior (no executable hash), and checks
            // execution approval and OS capture permission before starting the target.
            let observed = crate::checks::observation::observe_runtime(
                &program,
                self.run.cancel.clone(),
                || {
                    self.call("native_window_capture", json!({"program":program,"wait_secs":15,"output":capture.join("window.png"),"ui_tree":capture.join("ui-tree.json")}))
                },
                || self.approve_console(ws),
            )?;
            match observed {
                crate::checks::observation::RuntimeObservation::Window(reply) => {
                    notes.push_str(
                        "\n## Running window\nSee capture/window.png and capture/ui-tree.json.\n",
                    );
                    notes.push_str(&describe(reply)?);
                }
                crate::checks::observation::RuntimeObservation::Console(records) => {
                    let runs = capture.join("runs");
                    fs::create_dir_all(&runs).map_err(|e| Error::Io {
                        context: "无法保存命令行研究".into(),
                        source: e,
                    })?;
                    for record in records {
                        let name = record["name"]
                            .as_str()
                            .ok_or_else(|| Error::corrupt("命令行研究名称无效"))?;
                        write(
                            &runs.join(format!("{name}.json")),
                            &serde_json::to_vec_pretty(&record)
                                .map_err(|_| Error::corrupt("命令行研究格式无效"))?,
                        )?;
                    }
                    notes.push_str("\n## Console behavior\nNo visible window was found. Captured no arguments, --help and --version into capture/runs/. Match printed lines and exit behavior.\n");
                }
            }
        }
        write(&capture.join("README.md"), notes.as_bytes())
    }
}
fn describe(reply: ToolReply) -> Result<String> {
    let value = match reply {
        ToolReply::Value(v) | ToolReply::Evidence { body: v, .. } => v,
    };
    serde_json::to_string_pretty(&value)
        .map(|s| {
            format!(
                "```json\n{}\n```\n",
                s.chars().take(20000).collect::<String>()
            )
        })
        .map_err(|_| Error::corrupt("研究结果格式无效"))
}
fn read_capture(dir: &Path) -> Result<Value> {
    let p = dir.join("capture.json");
    let bytes = fs::read(p).map_err(|e| Error::Io {
        context: "无法读取页面研究记录".into(),
        source: e,
    })?;
    serde_json::from_slice(&bytes).map_err(|_| Error::corrupt("页面研究记录损坏"))
}
fn write(path: &Path, bytes: &[u8]) -> Result<()> {
    ca_engine::project::write_atomic(path, bytes).map_err(source)
}
fn inventory(root: &Path) -> Result<String> {
    let mut pending = vec![(root.to_path_buf(), 0)];
    let mut names = Vec::new();
    let mut visited = 0;
    while let Some((dir, depth)) = pending.pop() {
        for entry in fs::read_dir(dir).map_err(|e| Error::Io {
            context: "无法读取目标资源目录".into(),
            source: e,
        })? {
            let entry = entry.map_err(|e| Error::Io {
                context: "无法读取目标资源".into(),
                source: e,
            })?;
            visited += 1;
            if visited > 20000 {
                return Ok(format!(
                    "\n## Files (first 20000 entries)\n{}\n",
                    names.join("\n")
                ));
            }
            let kind = entry.file_type().map_err(|e| Error::Io {
                context: "无法读取目标资源类型".into(),
                source: e,
            })?;
            if kind.is_symlink() {
                continue;
            }
            if kind.is_dir() && depth < 6 {
                pending.push((entry.path(), depth + 1));
            } else if kind.is_file() && names.len() < 1000 {
                names.push(
                    entry
                        .path()
                        .strip_prefix(root)
                        .map_err(|_| Error::invalid("资源路径无效"))?
                        .display()
                        .to_string(),
                );
            }
        }
    }
    names.sort();
    Ok(format!("\n## Files\n{}\n", names.join("\n")))
}
