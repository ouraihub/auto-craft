//! Fresh, bounded game self-test reports: required core loop and every extra step.
use super::*;
use ca_engine::self_test::{SELF_TEST_VERSION, SelfTestReport};
const CORE: &[(&str, &str)] = &[
    ("move", "玩家移动"),
    ("act", "主要动作"),
    ("hud", "屏幕信息"),
];
pub struct SelfTestCheck;
impl Check for SelfTestCheck {
    fn id(&self) -> &'static str {
        "game-self-test"
    }
    fn run(&self, cx: &mut CheckCx<'_>) -> Result<CheckResult> {
        let out = cx
            .ws
            .checks_dir()
            .join(format!("round-{}", cx.round))
            .join("self-test.json");
        let saved = match process::option(cx, "--self-test", &out, 2 * 1024 * 1024)? {
            Ok(saved) => saved,
            Err(note) => return Ok(error_result(self.id(), cx.round, note)),
        };
        let claims = match report_claims(&saved.bytes) {
            Ok(claims) => claims,
            Err(note) => return Ok(error_result(self.id(), cx.round, note)),
        };
        Ok(CheckResult {
            artifacts: relative_artifact(cx.ws, &out).into_iter().collect(),
            ..result_with_claims(
                self.id(),
                cx.round,
                claims.iter().all(|c| c.passed),
                format!("自测已比较 {} 个核心及附加步骤。", claims.len()),
                claims,
            )
        })
    }
}
fn report_claims(bytes: &[u8]) -> std::result::Result<Vec<Claim>, String> {
    let report: SelfTestReport = serde_json::from_slice(bytes)
        .map_err(|_| "自测报告不是有效 JSON 或步骤格式无效。".to_string())?;
    if report.version > SELF_TEST_VERSION {
        return Err("自测报告版本高于当前支持的版本 1。".into());
    }
    let mut claims = Vec::new();
    for (name, meaning) in CORE {
        let steps = report
            .steps
            .iter()
            .filter(|s| s.step.trim().eq_ignore_ascii_case(name))
            .collect::<Vec<_>>();
        let ok = !steps.is_empty() && steps.iter().all(|s| s.passed);
        let detail = steps
            .iter()
            .map(|s| s.detail.trim())
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("; ");
        claims.push(claim(
            format!("游戏中{meaning}正常"),
            ok,
            if detail.is_empty() {
                "未测试".into()
            } else {
                detail.chars().take(1000).collect::<String>()
            },
        ));
    }
    // All steps affect readiness, including failures after the display's bounded claim list.
    let extras = report
        .steps
        .iter()
        .filter(|s| {
            !CORE
                .iter()
                .any(|(name, _)| s.step.trim().eq_ignore_ascii_case(name))
        })
        .collect::<Vec<_>>();
    for step in extras.iter().take(20) {
        claims.push(claim(
            format!(
                "自测步骤“{}”",
                step.step.trim().chars().take(120).collect::<String>()
            ),
            step.passed,
            step.detail.trim().chars().take(1000).collect::<String>(),
        ));
    }
    if extras.len() > 20 {
        claims.push(claim(
            "其余附加自测步骤全部通过",
            extras[20..].iter().all(|s| s.passed),
            format!("另有 {} 个步骤", extras.len() - 20),
        ));
    }
    Ok(claims)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn extra_and_duplicate_failures_cannot_be_hidden_by_core_success() {
        let bytes = br#"{"steps":[{"step":"move","passed":true},{"step":"act","passed":true},{"step":"hud","passed":true},{"step":"save","passed":false,"detail":"save failed"}]}"#;
        let claims = report_claims(bytes).unwrap();
        assert_eq!(claims.len(), 4);
        assert!(!claims[3].passed);
        assert!(report_claims(br#"{"version":2,"steps":[]}"#).is_err());
        assert!(
            report_claims(br#"{"steps":[]}"#)
                .unwrap()
                .iter()
                .all(|c| !c.passed)
        );
    }
}
