//! One bounded process-group owner for builds and automated program options.
use super::*;
use ca_platform::process::{Output, Process};
use std::{collections::VecDeque, io::Read, ops::Deref};

const OUTPUT_CAP: usize = 256 * 1024;

#[derive(Debug)]
pub(super) struct CheckOutput {
    output: Output,
    failure: Option<ca_evidence::ErrorCode>,
    scripts_failed: bool,
}
impl CheckOutput {
    pub(super) fn build_succeeded(&self, godot: bool) -> bool {
        self.success && !(godot && self.scripts_failed)
    }
}
impl Deref for CheckOutput {
    type Target = Output;
    fn deref(&self) -> &Output {
        &self.output
    }
}

#[derive(Default)]
struct Tail {
    bytes: VecDeque<u8>,
    truncated: bool,
    scripts_failed: bool,
}
impl Tail {
    fn line(&mut self, line: &str) {
        self.scripts_failed |= line.contains("SCRIPT ERROR")
            || line.contains("Parse Error")
            || line.trim_start().starts_with("ERROR:");
        self.push(line.as_bytes());
        self.push(b"\n");
    }
    fn push(&mut self, bytes: &[u8]) {
        let keep = bytes.len().min(OUTPUT_CAP);
        let remove = self
            .bytes
            .len()
            .saturating_add(keep)
            .saturating_sub(OUTPUT_CAP);
        self.truncated |= bytes.len() > keep || remove > 0;
        self.bytes.drain(..remove);
        self.bytes.extend(&bytes[bytes.len() - keep..]);
    }
}

fn run_owned(process: Process) -> Result<CheckOutput> {
    let stdout = Arc::new(Mutex::new(Tail::default()));
    let stderr = Arc::new(Mutex::new(Tail::default()));
    let out = stdout.clone();
    let err = stderr.clone();
    // The pinned Process only retains prefixes; its line sinks also receive discarded output.
    let result = process
        .stdout_cap(0)
        .stderr_cap(0)
        .on_stdout_line(move |line| {
            out.lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .line(line);
        })
        .on_stderr_line(move |line| {
            err.lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .line(line);
        })
        .run();
    let stdout = std::mem::take(
        &mut *stdout
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
    );
    let stderr = std::mem::take(
        &mut *stderr
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
    );
    let (mut output, failure) = match result {
        Ok(output) => (output, None),
        Err(error) if error.code == ca_evidence::ErrorCode::Cancelled => {
            return Err(crate::run::source(error));
        }
        Err(error) => (Output::default(), Some(error.code)),
    };
    output.stdout = stdout.bytes.into();
    output.stderr = stderr.bytes.into();
    output.stdout_truncated = stdout.truncated;
    output.stderr_truncated = stderr.truncated;
    Ok(CheckOutput {
        output,
        failure,
        scripts_failed: stdout.scripts_failed || stderr.scripts_failed,
    })
}

pub(super) fn run(
    cx: &CheckCx<'_>,
    program: &Path,
    args: &[String],
    limit: Duration,
) -> Result<CheckOutput> {
    cx.stop_now()?;
    let timeout = cx.stop().time_left().map_or(limit, |left| left.min(limit));
    let stop = cx.stop.clone();
    let selected = if program.components().count() > 1 {
        std::path::absolute(program).map_err(|_| Error::invalid("启动路径无效"))?
    } else {
        program.to_path_buf()
    };
    let result = run_owned(
        Process::new(selected)
            .args(args)
            .cwd(cx.ws.app_dir())
            .label("rebuild check")
            .timeout(timeout)
            .cancel_when(move || stop.is_cancelled()),
    );
    cx.stop_now()?;
    result
}

/// Diagnostics are returned as check evidence, never logged. Remove credentials, terminal controls
/// and absolute workspace/home paths before putting a bounded excerpt in the UI.
pub(super) fn diagnostic(cx: &CheckCx<'_>, output: &CheckOutput) -> String {
    diagnostic_paths(
        output,
        [
            Some(cx.ws.root.clone()),
            std::env::var_os("USERPROFILE").map(PathBuf::from),
            std::env::var_os("HOME").map(PathBuf::from),
        ]
        .into_iter()
        .flatten(),
    )
}

fn diagnostic_paths(output: &CheckOutput, paths: impl IntoIterator<Item = PathBuf>) -> String {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let mut lines = stdout
        .lines()
        .chain(stderr.lines())
        .filter(|line| !line.trim().is_empty())
        .rev()
        .take(12)
        .map(str::to_owned)
        .collect::<Vec<_>>();
    for path in paths {
        let path = path.to_string_lossy();
        for line in &mut lines {
            *line = line
                .replace(path.as_ref(), "<project>")
                .replace(&path.replace('\\', "/"), "<project>");
        }
    }
    let mut note = String::new();
    if let Some(code) = output.failure {
        note.push_str(if code == ca_evidence::ErrorCode::TimedOut {
            "命令超时并已停止。\n"
        } else {
            "无法运行检查命令。\n"
        });
        note.push_str(&format!("分类：{code:?}\n"));
    }
    if output.scripts_failed {
        note.push_str("检测到脚本或引擎错误。\n");
    }
    for line in lines.into_iter().rev() {
        if !note.is_empty() && !note.ends_with('\n') {
            note.push('\n');
        }
        note.extend(
            ca_engine::journal::scrubbed_line(&line)
                .chars()
                .filter(|c| !c.is_control())
                .take(300),
        );
    }
    if output.stdout_truncated || output.stderr_truncated {
        note.push_str("\n[output truncated]");
    }
    note
}

pub(super) fn remove_old(file: &Path) -> Result<()> {
    match fs::remove_file(file) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(Error::external("无法清除上次检查输出")),
    }
}

pub(super) fn read_bounded(file: &Path, max: u64) -> Result<Vec<u8>> {
    let meta = fs::symlink_metadata(file).map_err(|_| Error::not_found("检查输出文件不可读"))?;
    if !meta.is_file() || meta.file_type().is_symlink() {
        return Err(Error::invalid("检查输出必须为普通文件"));
    }
    let mut bytes = Vec::new();
    fs::File::open(file)
        .map_err(|_| Error::not_found("检查输出文件不可读"))?
        .take(max + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| Error::external("检查输出读取失败"))?;
    if bytes.len() as u64 > max {
        return Err(Error::invalid("检查输出超过大小限制"));
    }
    Ok(bytes)
}

pub(super) struct Saved {
    pub bytes: Vec<u8>,
    pub program: PathBuf,
    pub args: Vec<String>,
}
pub(super) fn option(
    cx: &CheckCx<'_>,
    name: &str,
    file: &Path,
    max: u64,
) -> Result<std::result::Result<Saved, String>> {
    cx.stop_now()?;
    if let Some(parent) = file.parent() {
        owned_directory(cx.ws, parent)?;
        fs::create_dir_all(parent).map_err(|_| Error::external("无法创建检查目录"))?;
    }
    remove_old(file)?;
    let app = cx.ws.app_dir();
    let Some((program, args)) = ca_engine::outputs::launch(&app, cx.editor()) else {
        return Ok(Err("没有可启动的构建程序，请先声明并构建入口。".into()));
    };
    let mut with = args.clone();
    if app.join("project.godot").is_file() {
        with.push("--".into());
    }
    with.extend([
        name.into(),
        std::path::absolute(file)
            .map_err(|_| Error::invalid("检查输出路径无效"))?
            .display()
            .to_string(),
    ]);
    let output = run(cx, &program, &with, Duration::from_secs(90))?;
    if !output.success {
        return Ok(Err(format!(
            "程序退出失败（代码 {:?}）。{}",
            output.code,
            diagnostic(cx, &output)
        )));
    }
    match read_bounded(file, max) {
        Ok(bytes) => Ok(Ok(Saved {
            bytes,
            program,
            args,
        })),
        Err(_) => Ok(Err(format!(
            "程序未保存有效的 {name} 输出。{}",
            diagnostic(cx, &output)
        ))),
    }
}

pub(super) fn owned_directory(ws: &Workspace, dir: &Path) -> Result<()> {
    let root = fs::canonicalize(&ws.root).map_err(|_| Error::invalid("项目目录不可读"))?;
    let absolute = std::path::absolute(dir).map_err(|_| Error::invalid("输出目录无效"))?;
    let mut existing = absolute.as_path();
    while !existing.exists() {
        existing = existing
            .parent()
            .ok_or_else(|| Error::invalid("输出目录无效"))?;
    }
    let actual = fs::canonicalize(existing).map_err(|_| Error::invalid("输出目录不可读"))?;
    if !actual.starts_with(root)
        || absolute
            .components()
            .any(|c| c == std::path::Component::ParentDir)
    {
        return Err(Error::invalid("检查输出目录必须位于项目中"));
    }
    Ok(())
}

#[cfg(test)]
#[path = "process_tests.rs"]
mod tests;
