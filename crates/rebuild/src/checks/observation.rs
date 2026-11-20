//! Visible-only original console observation for clean-room study; never parses or hashes code.
use super::*;
use serde_json::json;
pub(crate) enum RuntimeObservation {
    Window(ToolReply),
    Console(Vec<Value>),
}

pub(crate) fn observe_runtime(
    program: &Path,
    stop: CancelFlag,
    capture: impl FnOnce() -> Result<ToolReply>,
    approve_console: impl FnOnce() -> Result<()>,
) -> Result<RuntimeObservation> {
    match capture() {
        Ok(reply) => return Ok(RuntimeObservation::Window(reply)),
        // Only absence of a window permits terminal observation. Approval failures,
        // unsupported hosts and capture failures must never trigger another execution.
        Err(e) if e.kind() == Some(ErrorKind::NotFound) => {}
        Err(e) => return Err(e),
    }
    approve_console()?;
    if stop.is_cancelled() {
        return Err(Error::new(ErrorKind::Cancelled, "原程序观察已取消"));
    }
    let cases: [(&str, &[&str]); 3] = [
        ("no-arguments", &[]),
        ("help", &["--help"]),
        ("version", &["--version"]),
    ];
    let records = cases
        .into_iter()
        .map(|(name, args)| observe_console(program, args, name, stop.clone()))
        .collect::<Result<Vec<_>>>()?;
    if !records.iter().any(|r| {
        r["reason"] == "exited" || r["lines"].as_array().is_some_and(|lines| !lines.is_empty())
    }) {
        return Err(Error::not_found(
            "原程序既未显示窗口，也未产生可观察的命令行行为",
        ));
    }
    Ok(RuntimeObservation::Console(records))
}

pub(crate) fn observe_console(
    program: &Path,
    args: &[&str],
    name: &str,
    stop: CancelFlag,
) -> Result<Value> {
    if stop.is_cancelled() {
        return Err(Error::new(ErrorKind::Cancelled, "原程序观察已取消"));
    }
    use ca_platform::process::Process;
    let out = Process::new(program)
        .args(args)
        .cwd(program.parent().unwrap_or(Path::new(".")))
        .label("original runtime observation")
        .timeout(std::time::Duration::from_secs(8))
        .stdout_cap(256 * 1024)
        .stderr_cap(256 * 1024)
        .cancel_when(move || stop.is_cancelled())
        .run();
    let (reason, code, text) = match out {
        Ok(out) => (
            "exited",
            out.code,
            format!("{}\n{}", out.stdout_text(), out.stderr_text()),
        ),
        Err(e) if e.code == ca_evidence::ErrorCode::TimedOut => {
            let partial = e.details.as_ref().and_then(|d| d.get("partial"));
            let text = ["stdout_tail", "stderr_tail"]
                .iter()
                .filter_map(|key| partial.and_then(|p| p.get(key)).and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n");
            ("timeout", None, text)
        }
        Err(e) => return Err(crate::run::source(e)),
    };
    let lines = text
        .lines()
        .map(|l| l.trim_end())
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>();
    Ok(json!({"name":name,"arguments":args,"reason":reason,"exit_code":code,"lines":lines}))
}

#[cfg(test)]
#[path = "study_tests.rs"]
mod tests;
