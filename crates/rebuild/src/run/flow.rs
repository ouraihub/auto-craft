//! Reference: craft-anything crates/ai/src/rebuild/mod.rs::Worker::run (300dba58).
//! Authoritative study/build/notes/check/refinement transition rules.
use super::check::{answer_self_checks, check};
use super::{Run, source, turn, usage::Usage};
use crate::{
    BriefCx, KindRegistry, RebuildConfig, approvals::Policy, brief, builder::backend_id, checks,
    session_copy, study::Study,
};
use autocraft_core::*;
use ca_engine::{
    workspace::progress::{AppFingerprintCache, BuildProgress},
    workspace::{CheckResult, Status, TargetRef, Workspace},
};
use std::{
    path::Path,
    sync::{Arc, PoisonError},
};

pub(super) fn starts_with_study(
    request: &RunRequest,
    in_mode: bool,
    elsewhere: bool,
    capture: bool,
) -> bool {
    !capture || (request.message.is_none() && !in_mode) || (!in_mode && elsewhere)
}
pub(super) fn carrying_on(request: &RunRequest, in_mode: bool) -> bool {
    in_mode && (request.continuing || request.message.is_none())
}
pub(super) fn execute(
    run: &Arc<Run>,
    config: &RebuildConfig,
    ws: &mut Workspace,
    request: RunRequest,
) -> Result<String> {
    let mut progress = BuildProgress::load(&ws.root).map_err(source)?;
    if request
        .message
        .as_deref()
        .is_some_and(|m| !m.trim().is_empty())
    {
        progress.reset();
        progress.save(&ws.root).map_err(source)?;
        ws.manifest.not_working_checks = 0;
        ws.save().map_err(source)?;
    }
    let settings_dto = config.analysis.settings()?;
    let settings = super::settings(&settings_dto)?;
    let deep = config.projects.open(&run.project)?.mode == AnalysisMode::Deep;
    let mut plan = config.host.prepare(&run.project, deep, request.backend)?;
    let backend = plan.request.backend;
    let id = backend_id(backend);
    let sees = plan.sees_pictures;
    std::fs::create_dir_all(ws.app_dir()).map_err(|e| Error::Io {
        context: "无法创建构建目录".into(),
        source: e,
    })?;
    let registry = KindRegistry::default();
    if ws.manifest.stack.is_empty() {
        ws.manifest.stack = registry
            .choose(&ws.manifest.request, &ws.manifest.target, deep)
            .ok_or_else(|| Error::invalid("描述一下要做什么：网站、应用还是游戏"))?
            .1
            .into();
        ws.save().map_err(source)?;
    }
    let kind = registry
        .for_workspace(ws)
        .ok_or_else(|| Error::invalid("描述一下要做什么：网站、应用还是游戏"))?;
    let saved = ws
        .manifest
        .agent_session_for_mode(id, deep)
        .map(str::to_owned);
    let in_mode = saved.is_some();
    let elsewhere = ws.manifest.agent_session.is_some() && !in_mode;
    let complete = matches!(ws.manifest.target, TargetRef::None)
        || ws.capture_dir().join("README.md").is_file();
    let first = starts_with_study(&request, in_mode, elsewhere, complete);
    let continuing = carrying_on(&request, in_mode);
    let study = Study {
        run,
        config,
        settings: &settings,
        deep,
    };
    if first {
        study.study(ws, kind.as_ref())?;
    } else {
        study.reopen(ws, kind.as_ref())?;
    }
    let mut resume = saved.clone().map(NativeSessionKey::from);
    if backend != BackendKind::Dsh && saved.as_ref().is_some_and(|s| !Path::new(s).is_file()) {
        resume = session_copy::restore(&ws.root, id, &config.host.session_dir(backend)?)?;
        if let Some(key) = &resume {
            config.host.bind(&run.project, backend, key)?;
            ws.manifest.agent_session = Some(key.to_string());
            ws.save().map_err(source)?;
        }
    }
    let editor = find_editor(&settings, &ws.manifest.stack)?;
    let cx = BriefCx::new(ws, &settings, deep, editor.as_deref(), plan.assets, sees);
    let now = kind
        .follow_up_sections(&cx)
        .iter()
        .map(|s| format!("\n\n{s}"))
        .collect::<String>()
        + &brief::pictures_reminder(ws, sees);
    let mut prompt = match &request.message {
        Some(m) if !first && resume.is_some() => brief::follow_up(m, kind.as_ref()) + &now,
        None if continuing && !first && resume.is_some() => {
            brief::continue_prompt(kind.as_ref()) + &now
        }
        other => brief::brief(kind.as_ref(), &cx, other.as_deref()),
    };
    if saved.is_some() && resume.is_none() {
        let items = ca_engine::feed::load(ws);
        let handoff = session_copy::handoff(&items);
        prompt = brief::resumed_elsewhere(
            &brief::brief(kind.as_ref(), &cx, request.message.as_deref()),
            &ws.manifest.last_reply,
            Some(&handoff),
        );
    }
    let mut asked = ca_engine::questions::untold(ws, deep);
    prompt.insert_str(0, &brief::just_asked(&asked));
    for picture in run
        .continuity
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .user_pictures()
    {
        prompt.push_str(&format!("\nPicture attached: {}", picture.display()));
    }
    if ws.manifest.not_working_checks >= 10 {
        return Err(not_working());
    }
    run.step(ws, StepDto::Build, Status::Building, "正在准备构建方…")?;
    let project = run.project.clone();
    let sink = config.sink.clone();
    let waiting = Arc::new(move || {
        sink.publish(RebuildEvent::Step {
            project: project.clone(),
            step: StepDto::Build,
            text: "等待你的确认…".into(),
        })
    });
    plan.request.cwd = ws.app_dir();
    plan.request.resume = resume;
    plan.request.builder = Some(BuilderHooks {
        policy: Arc::new(Policy::new(
            ws.root.clone(),
            settings.auto_mode,
            !deep,
            waiting,
        )),
    });
    let events = Arc::new(turn::Events::new(run.clone()));
    let session = config.host.start(plan.request, events.clone())?;
    *run.session.lock().unwrap_or_else(PoisonError::into_inner) = Some(session.clone());
    ws.manifest.agent_session_deep = Some(deep);
    let mut usage = Usage::new(session.as_ref())?;
    let mut fingerprints = AppFingerprintCache::default();
    let mut available = true;
    let mut round = ws
        .manifest
        .checks
        .iter()
        .map(|c| c.round)
        .max()
        .unwrap_or(0);
    let mut refinements = 0;
    let mut reminded = false;
    let max_rounds = request.max_rounds.unwrap_or(settings.build.max_rounds);
    let mut only_check =
        continuing && !first && request.message.is_none() && ws.manifest.awaiting_check;
    let mut kept = checks::KeptChecks::from_saved(&progress, ws, kind.id(), deep);
    kept.set_plugin_ids(
        config
            .analysis
            .plugin_checks(&run.project, kind.id())?
            .into_iter()
            .map(|check| check.id),
    );
    loop {
        run.cancelled()?;
        if !std::mem::take(&mut only_check) {
            progress = BuildProgress::load(&ws.root).map_err(source)?;
            if let Some(key) =
                fingerprint(run, ws, &mut progress, &mut fingerprints, &mut available)?
            {
                progress.before_turn(key);
                if progress.stalled() {
                    return Err(stalled());
                }
                progress.save(&ws.root).map_err(source)?;
            }
            run.step(
                ws,
                StepDto::Build,
                Status::Building,
                if round == 0 {
                    if continuing {
                        "继续构建…"
                    } else {
                        "正在构建第一版…"
                    }
                } else {
                    "改进构建…"
                },
            )?;
            ws.manifest.awaiting_check = false;
            ws.save().map_err(source)?;
            let turn = turn::run(
                config,
                ws,
                session.as_ref(),
                events.as_ref(),
                &prompt,
                &mut usage,
                |current| {
                    answer_self_checks(
                        run,
                        config,
                        current,
                        kind.as_ref(),
                        deep,
                        &settings,
                        &mut kept,
                    )
                },
            );
            if turn.is_err()
                && let Err(reset) = reset_after_notes(run, ws)
            {
                tracing::warn!(code = reset.code(), "rebuild.notes.progress_reset_failed");
            }
            let reply = turn?;
            run.continuity
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .delivered()?;
            progress = reset_after_notes(run, ws)?;
            if !asked.is_empty() {
                ca_engine::questions::mark_told(ws, &asked).map_err(source)?;
                asked.clear();
            }
            if !reply.trim().is_empty() {
                ws.manifest.last_reply = reply.trim().into();
            }
            if ca_engines::game::Builder::from_stack(&ws.manifest.stack).is_none()
                && let Some(stack) = crate::stacks::used_in(&ws.app_dir())
            {
                ws.manifest.stack = stack.name.clone();
            }
            ws.manifest.awaiting_check = true;
            ws.save().map_err(source)?;
            if let Some(key) =
                fingerprint(run, ws, &mut progress, &mut fingerprints, &mut available)?
            {
                progress.after_turn(key);
                progress.save(&ws.root).map_err(source)?;
            }
            if let Some((p, more)) = run
                .continuity
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .added(ws, deep, sees)?
            {
                asked.extend(more);
                progress.reset();
                progress.save(&ws.root).map_err(source)?;
                ws.manifest.not_working_checks = 0;
                prompt = p;
                continue;
            }
            if !reminded
                && deep
                && matches!(ws.manifest.target, TargetRef::Local { .. })
                && !ws.manifest.decompiled
            {
                reminded = true;
                prompt = brief::DECOMPILE_FIRST.into();
                continue;
            }
        }
        round += 1;
        run.step(ws, StepDto::Check, Status::Checking, "正在检查构建结果…")?;
        let results = check(&study, ws, kind.as_ref(), round, &mut kept)?;
        ws.manifest.checks.extend(results.iter().cloned());
        ws.manifest.awaiting_check = false;
        ws.manifest.not_working_checks = if results.iter().all(|c| c.passed) {
            0
        } else {
            ws.manifest.not_working_checks.saturating_add(1)
        };
        ws.save().map_err(source)?;
        progress.checked = kept.saved_for(round);
        progress.save(&ws.root).map_err(source)?;
        for result in &results {
            run.emit(RebuildEvent::Checked {
                project: run.project.clone(),
                result: super::dto(result)?,
            })?;
        }
        if let Some((p, more)) = run
            .continuity
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .added(ws, deep, sees)?
        {
            asked.extend(more);
            progress.reset();
            progress.save(&ws.root).map_err(source)?;
            ws.manifest.not_working_checks = 0;
            prompt = p;
            continue;
        }
        let Some(failed) = results.iter().find(|c| !c.good(settings.build.good_enough)) else {
            break;
        };
        if available
            && progress.stalled()
            && let Some(key) =
                fingerprint(run, ws, &mut progress, &mut fingerprints, &mut available)?
        {
            progress.before_turn(key);
            progress.save(&ws.root).map_err(source)?;
            if progress.stalled() {
                return Err(stalled());
            }
        }
        if ws.manifest.not_working_checks >= 10 {
            return Err(not_working());
        }
        if refinements >= max_rounds {
            break;
        }
        refinements += 1;
        prompt = if app_empty(ws)? {
            brief::NO_PROJECT_PROMPT.into()
        } else {
            kind.refine(failed, sees)
        } + &brief::pictures_reminder(ws, sees)
            + brief::CHECK_BEFORE_REPLYING;
    }
    let latest = ws
        .manifest
        .checks
        .iter()
        .filter(|c| c.round == round)
        .cloned()
        .collect::<Vec<_>>();
    if latest.is_empty() || latest.iter().any(|c| !c.passed) {
        return Err(Error::external("构建还不能用。请补充要求或重试。"));
    }
    let mut summary = summary(&latest);
    let open = config
        .analysis
        .view(&run.project)?
        .unknowns
        .iter()
        .filter(|q| q.status != UnknownStatus::Resolved)
        .count();
    if open > 0 {
        summary.push_str(&format!(
            " {open} questions about the original are still open."
        ));
    }
    run.step(ws, StepDto::Done, Status::Ready, &summary)?;
    if let Some(url) = run.preview.url() {
        run.emit(RebuildEvent::Preview {
            project: run.project.clone(),
            url,
        })?;
    }
    Ok(summary)
}
fn fingerprint(
    run: &Run,
    ws: &Workspace,
    progress: &mut BuildProgress,
    cache: &mut AppFingerprintCache,
    available: &mut bool,
) -> Result<Option<String>> {
    run.cancelled()?;
    if !*available {
        return Ok(None);
    }
    match cache.scan(ws, &run.native_cancel) {
        Ok(key) => Ok(Some(key)),
        Err(e) if e.code == ca_evidence::ErrorCode::Cancelled => Err(source(e)),
        Err(_) => {
            *available = false;
            progress.invalidate();
            progress.save(&ws.root).map_err(source)?;
            run.emit(RebuildEvent::Step {
                project: run.project.clone(),
                step: StepDto::Build,
                text: "无法比较文件改动，本次运行不使用连续无进展限制。".into(),
            })?;
            Ok(None)
        }
    }
}
fn find_editor(settings: &ca_engine::Settings, stack: &str) -> Result<Option<std::path::PathBuf>> {
    let Some(builder) = ca_engines::game::Builder::from_stack(stack)
        .filter(|b| *b != ca_engines::game::Builder::Rust)
    else {
        return Ok(None);
    };
    let inv =
        ca_engines::game::inventory(&ca_engines::game::Places::here(settings.engine_store_dir()));
    match inv.best(builder) {
        Some(i) if inv.ready(builder) => Ok(Some(i.program.clone())),
        _ => Err(Error::new(
            ErrorKind::EngineMissing,
            "所选游戏引擎尚未安装，请在分析设置中安装或选择其他技术栈",
        )),
    }
}
fn app_empty(ws: &Workspace) -> Result<bool> {
    let entries = std::fs::read_dir(ws.app_dir()).map_err(|e| Error::Io {
        context: "无法检查构建目录".into(),
        source: e,
    })?;
    for entry in entries {
        let p = entry
            .map_err(|e| Error::Io {
                context: "无法检查构建文件".into(),
                source: e,
            })?
            .path();
        if !ws.is_own_record(&p) && p.file_name().is_none_or(|n| n != ".git") {
            return Ok(false);
        }
    }
    Ok(true)
}
fn not_working() -> Error {
    Error::external("构建连续十次检查仍然不可用。请补充新的要求后继续。")
}
fn stalled() -> Error {
    Error::external("构建方连续两轮没有改动构建目录里的任何文件。")
}
fn summary(checks: &[CheckResult]) -> String {
    let total = checks.iter().map(|c| c.claims.len()).sum::<usize>();
    let passed = checks
        .iter()
        .flat_map(|c| &c.claims)
        .filter(|c| c.passed)
        .count();
    let claims = if total == 0 {
        String::new()
    } else {
        format!(", {passed} of {total} claims hold")
    };
    let result = if let Some(similarity) = checks.iter().find_map(|c| c.similarity) {
        format!("about {similarity:.0}% like the original")
    } else {
        format!("all {} checks pass", checks.len())
    };
    format!("Ready: {result}{claims}.")
}
fn reset_after_notes(run: &Run, ws: &mut Workspace) -> Result<BuildProgress> {
    let mut continuity = run
        .continuity
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let mut progress = BuildProgress::load(&ws.root).map_err(source)?;
    if continuity.progress_reset() {
        progress.reset();
        progress.save(&ws.root).map_err(source)?;
        ws.manifest.not_working_checks = 0;
        ws.save().map_err(source)?;
        continuity.progress_reset_saved();
    }
    Ok(progress)
}
