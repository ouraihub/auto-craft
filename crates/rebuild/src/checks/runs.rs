//! Bounded terminal observations compared by exit behavior and normalized output words.
use super::*;
use ca_platform::capture::{self, Limits, SandboxRequest, Spec, Terminal};
use serde_json::json;
use std::collections::BTreeMap;

pub struct RunsCheck;
impl Check for RunsCheck {
    fn id(&self) -> &'static str {
        "app-runs"
    }
    fn applies(&self, cx: &CheckCx<'_>) -> bool {
        cx.ws.capture_dir().join("runs").is_dir()
    }
    fn run(&self, cx: &mut CheckCx<'_>) -> Result<CheckResult> {
        let runs = fs::read_dir(cx.ws.capture_dir().join("runs"))
            .map_err(|_| Error::not_found("命令行研究记录不可读"))?;
        let Some((program, base)) = ca_engine::outputs::launch(&cx.ws.app_dir(), cx.editor())
        else {
            return Ok(error_result(
                self.id(),
                cx.round,
                "没有构建完成且可启动的命令行程序。",
            ));
        };
        let original_name = match &cx.ws.manifest.target {
            ca_engine::workspace::TargetRef::Local { path } => path
                .file_stem()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            ca_engine::workspace::TargetRef::Website { .. }
            | ca_engine::workspace::TargetRef::None => String::new(),
        };
        let names = [
            original_name,
            program
                .file_stem()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
        ];
        let dir = cx
            .ws
            .checks_dir()
            .join(format!("round-{}", cx.round))
            .join("runs");
        process::owned_directory(cx.ws, &dir)?;
        fs::create_dir_all(&dir).map_err(|_| Error::external("无法创建命令行检查目录"))?;
        let (mut claims, mut artifacts) = (Vec::new(), Vec::new());
        let mut files = runs
            .take(20)
            .collect::<std::io::Result<Vec<_>>>()
            .map_err(|_| Error::external("无法枚举命令行记录"))?;
        files.sort_by_key(|e| e.file_name());
        for (index, file) in files.iter().enumerate() {
            cx.stop_now()?;
            let original: Value =
                serde_json::from_slice(&process::read_bounded(&file.path(), 2 * 1024 * 1024)?)
                    .map_err(|_| Error::corrupt("命令行研究记录格式无效"))?;
            let mut args = base.clone();
            let extra = original
                .get("arguments")
                .or_else(|| original.get("args"))
                .and_then(Value::as_array)
                .ok_or_else(|| Error::corrupt("命令行研究记录缺少参数"))?;
            for arg in extra.iter().take(128) {
                args.push(
                    arg.as_str()
                        .ok_or_else(|| Error::corrupt("命令行参数格式无效"))?
                        .to_string(),
                );
            }
            let stop = cx.stop.clone();
            let timeout = cx
                .stop()
                .time_left()
                .map_or(Duration::from_secs(8), |left| {
                    left.min(Duration::from_secs(8))
                });
            let cap = capture::capture(
                Spec {
                    executable: program.as_os_str().into(),
                    args: args.iter().map(Into::into).collect(),
                    cwd: Some(cx.ws.app_dir()),
                    env: Vec::new(),
                    terminal: Terminal::default(),
                    events: Vec::new(),
                    timeout,
                    idle_timeout: timeout,
                    settle: Duration::from_millis(200),
                    finalization: Duration::ZERO,
                    observation_paths: Vec::new(),
                    limits: Limits {
                        output_bytes: 256 * 1024,
                        ..Limits::default()
                    },
                    sandbox: SandboxRequest::Off,
                },
                Arc::new(move || stop.is_cancelled()),
            );
            let cap = match cap {
                Ok(cap) => cap,
                Err(error) if error.code == ca_evidence::ErrorCode::Cancelled => {
                    return Err(crate::run::source(error));
                }
                Err(error) => {
                    return Ok(error_result(
                        self.id(),
                        cx.round,
                        format!(
                            "命令行程序无法运行（{:?}）。请检查构建入口和本地运行环境。",
                            error.code
                        ),
                    ));
                }
            };
            cx.stop_now()?;
            let built = json!({"name":original.get("name"),"arguments":args,"reason":cap.exit.reason,"exit_code":cap.exit.code,"lines":cap.terminal.lines,"truncated":cap.terminal.output_truncated});
            let out = dir.join(format!("run-{index}.json"));
            ca_engine::project::write_atomic(
                &out,
                &serde_json::to_vec_pretty(&built)
                    .map_err(|_| Error::corrupt("命令行结果格式无效"))?,
            )
            .map_err(crate::run::source)?;
            artifacts.extend(relative_artifact(cx.ws, &out));
            claims.extend(run_claims(&original, &built, &names));
        }
        if claims.is_empty() {
            return Ok(error_result(
                self.id(),
                cx.round,
                "没有可比较的命令行研究记录。",
            ));
        }
        Ok(CheckResult {
            artifacts,
            ..result_with_claims(
                self.id(),
                cx.round,
                claims.iter().all(|c| c.passed),
                format!("已比较 {} 个命令行行为断言。", claims.len()),
                claims,
            )
        })
    }
}
fn words(run: &Value, names: &[String]) -> BTreeMap<String, usize> {
    let mut text = run
        .get("lines")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>()
        .join("\n")
        .to_lowercase();
    for name in names.iter().filter(|n| !n.is_empty()) {
        text = text.replace(&name.to_lowercase(), "program");
    }
    let mut words = BTreeMap::new();
    for word in text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty())
    {
        *words.entry(word.to_string()).or_default() += 1;
    }
    words
}
fn exit_reason(v: &Value) -> Option<&str> {
    v.get("reason")
        .and_then(Value::as_str)
        .map(|r| if r == "idle_timeout" { "timeout" } else { r })
}

fn run_claims(original: &Value, built: &Value, names: &[String]) -> Vec<Claim> {
    let reason = exit_reason;
    let exited = reason(original) == Some("exited");
    let exit_ok = reason(original).is_some()
        && reason(original) == reason(built)
        && (!exited
            || original.get("exit_code").and_then(Value::as_i64)
                == built.get("exit_code").and_then(Value::as_i64));
    let (a, b) = (words(original, names), words(built, names));
    let total: usize = a.values().sum::<usize>() + b.values().sum::<usize>();
    let shared: usize = a
        .iter()
        .map(|(word, n)| (*n).min(b.get(word).copied().unwrap_or(0)))
        .sum();
    let score = if total == 0 {
        100.0
    } else {
        200.0 * shared as f64 / total as f64
    };
    let name = original
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("输入");
    vec![
        claim(
            format!("{name} 的退出行为与原程序一致"),
            exit_ok,
            format!(
                "退出原因 {:?}，代码 {:?}",
                reason(built),
                built.get("exit_code")
            ),
        ),
        claim(
            format!("{name} 的输出与原程序一致"),
            score >= 80.0,
            format!("输出词语匹配度 {score:.0}%"),
        ),
    ]
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn multiline_stderr_and_nonzero_exit_compare_without_requiring_success() {
        let a = json!({"name":"help","reason":"exited","exit_code":2,"lines":["usage original --help", "invalid option"]});
        let b = json!({"reason":"exited","exit_code":2,"lines":["usage rebuilt --help\ninvalid option"]});
        let names = ["original".into(), "rebuilt".into()];
        assert!(run_claims(&a, &b, &names).iter().all(|c| c.passed));
        let mut wrong = b;
        wrong["exit_code"] = json!(0);
        assert!(!run_claims(&a, &wrong, &names)[0].passed);
        wrong["lines"] = json!([]);
        assert!(!run_claims(&a, &wrong, &names)[1].passed);
    }
}
