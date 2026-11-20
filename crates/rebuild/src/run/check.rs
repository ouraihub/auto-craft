//! Reference: craft-anything crates/ai/src/rebuild/check.rs and self_check.rs (300dba58).
//! Check reuse, one retry and answers to builder self-check requests.
use super::*;
use crate::checks;
pub(super) fn check(
    study: &crate::study::Study<'_>,
    ws: &Workspace,
    kind: &dyn crate::TargetKind,
    round: u32,
    kept: &mut checks::KeptChecks,
) -> Result<Vec<CheckResult>> {
    let crate::study::Study {
        run,
        config,
        settings,
        deep,
    } = *study;
    if let Some(results) = kept.reuse(ws, round) {
        kept.remember(ws, &results);
        return Ok(results);
    }
    let execute = || {
        checks::execute_checks(
            checks::CheckCx::new(
                ws,
                round,
                deep,
                checks::CheckResources {
                    settings,
                    analysis: config.analysis.as_ref(),
                    project: &run.project,
                    preview: run.preview.as_ref(),
                    stop: &run.cancel,
                },
            ),
            kind,
        )
    };
    let results = match execute() {
        Err(e) if e.kind() != Some(ErrorKind::Cancelled) => {
            run.emit(RebuildEvent::Step {
                project: run.project.clone(),
                step: StepDto::Check,
                text: "检查未完成，正在重试一次…".into(),
            })?;
            execute()?
        }
        other => other?,
    };
    kept.remember(ws, &results);
    Ok(results)
}
pub(super) fn answer_self_checks(
    run: &Run,
    config: &RebuildConfig,
    ws: &Workspace,
    kind: &dyn crate::TargetKind,
    deep: bool,
    settings: &ca_engine::Settings,
    kept: &mut checks::KeptChecks,
) -> Result<()> {
    while let Some(request) = run.self_checks.take() {
        run.emit(RebuildEvent::Step {
            project: run.project.clone(),
            step: StepDto::Build,
            text: "构建方正在对照原作检查…".into(),
        })?;
        let result = check(
            &crate::study::Study {
                run,
                config,
                settings,
                deep,
            },
            ws,
            kind,
            0,
            kept,
        );
        let answer = match result {
            Ok(results) => checks::report(
                &results,
                settings.build.good_enough,
                &config.analysis.view(&run.project)?.unknowns,
            ),
            Err(e) if e.kind() == Some(ErrorKind::Cancelled) => {
                run.self_checks.answer(request.id,"The build was stopped before this check finished, so there is nothing to report.".into());
                return Err(e);
            }
            Err(e) => format!("The check couldn't run: {}", e.detail()),
        };
        run.self_checks.answer(request.id, answer);
    }
    Ok(())
}
