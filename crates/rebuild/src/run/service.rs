//! Reference: craft-anything crates/ai/src/rebuild/runs.rs (300dba58).
//! Public rebuild requests and passive project reads.
use crate::*;
impl RebuildService for RebuildHub {
    fn start(&self, project: &ProjectId, request: RunRequest) -> Result<RunStarted> {
        let detail = self.shared.config.projects.open(project)?;
        if detail.summary.read_only {
            return Err(Error::new(ErrorKind::ReadOnly, "项目来自更新版本"));
        }
        let parallel = self
            .shared
            .config
            .analysis
            .settings()?
            .build
            .parallel
            .max(1);
        let started = {
            let mut state = self
                .shared
                .state
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            if state.shutdown {
                return Err(Error::busy("应用正在退出"));
            }
            if state.active.contains(project) || state.queue.iter().any(|(p, _)| p == project) {
                return Err(Error::busy("项目已在复刻或排队"));
            }
            notes::clear(Path::new(project.as_str()))?;
            let run = run::Run::new(
                project.clone(),
                PathBuf::from(project.as_str()),
                &self.shared.config,
            )?;
            run.continuity
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .user(&request)?;
            state.runs.insert(project.clone(), run);
            let position = state.queue.len() + 1;
            let queued = state.active.len() >= parallel;
            state.queue.push_back((project.clone(), request));
            if queued {
                RunStarted::Queued { position }
            } else {
                RunStarted::Running
            }
        };
        if let RunStarted::Queued { position } = started {
            self.shared.config.sink.publish(RebuildEvent::Queue {
                project: project.clone(),
                position: Some(position),
            });
        }
        schedule(&self.shared)?;
        Ok(started)
    }
    fn stop(&self, project: &ProjectId) -> Result<()> {
        let run = self.project_run(project)?;
        let queued = {
            let mut state = self
                .shared
                .state
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            let old = state.queue.len();
            state.queue.retain(|(p, _)| p != project);
            state.queue.len() != old
        };
        run.stop()?;
        if queued {
            run.finish(Err(Error::new(ErrorKind::Cancelled, "复刻已暂停")))?;
            self.shared.config.sink.publish(RebuildEvent::Queue {
                project: project.clone(),
                position: None,
            });
        }
        Ok(())
    }
    fn note(&self, project: &ProjectId, text: &str, pictures: Vec<PathBuf>) -> Result<NoteId> {
        let deep = self.shared.config.projects.open(project)?.mode == AnalysisMode::Deep;
        let run = self.project_run(project)?;
        let session = run
            .session
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        run.continuity
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .note(text, deep, pictures, session.as_deref())
    }
    fn withdraw(&self, project: &ProjectId, id: &NoteId) -> Result<Option<String>> {
        self.project_run(project)?
            .continuity
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .withdraw(id)
    }
    fn status(&self, project: &ProjectId) -> Result<RunStatus> {
        let detail = self.shared.config.projects.open(project)?;
        let state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let active = state.active.contains(project);
        let queued = state
            .queue
            .iter()
            .position(|(p, _)| p == project)
            .map(|n| n + 1);
        let run = state.runs.get(project).cloned();
        drop(state);
        let (step, quiet, notes) = if let Some(run) = run {
            let s = run.state.lock().unwrap_or_else(PoisonError::into_inner);
            let step = s.step;
            let quiet = s
                .touched
                .elapsed()
                .as_millis()
                .try_into()
                .unwrap_or(u64::MAX);
            drop(s);
            let notes = run
                .continuity
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .waiting_notes();
            (step, quiet, notes)
        } else {
            (None, 0, Vec::new())
        };
        Ok(RunStatus {
            running: active,
            queued,
            step,
            status: detail.summary.status,
            note: detail.summary.note,
            latest_checks: detail.latest_checks,
            waiting_notes: notes,
            pending_requests: detail.summary.pending_requests,
            quiet_for_ms: quiet,
        })
    }
    fn self_check(
        &self,
        project: &ProjectId,
        wait: Duration,
        cancel: &CancelFlag,
    ) -> Result<String> {
        let active = self
            .shared
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .active
            .contains(project);
        if !active {
            return Err(Error::not_found(format!(
                "No build is running for this project: nothing took the check in {} minutes. Use this while autocraft is building the project; otherwise build it and finish the turn, and it is checked after.",
                wait.as_secs() / 60
            )));
        }
        self.project_run(project)?.self_checks.enqueue(wait, cancel)
    }
    fn ask(&self, project: &ProjectId, question: &str, cancel: &CancelFlag) -> Result<Answered> {
        let detail = self.shared.config.projects.open(project)?;
        let ws = run::open(Path::new(project.as_str()))?;
        question::ask(
            self.shared.config.host.as_ref(),
            project,
            &ws,
            detail.mode == AnalysisMode::Deep,
            question,
            cancel,
        )
    }
    fn finish(&self, project: &ProjectId) -> Result<()> {
        let state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if state.active.contains(project) || state.queue.iter().any(|(p, _)| p == project) {
            return Err(Error::busy("复刻运行中，不能标记完成"));
        }
        drop(state);
        let run = self.project_run(project)?;
        let mut ws = run::open(&run.root)?;
        let text = ca_engine::tools::project_tools::CALLED_DONE;
        ws.set_status(Status::Ready, text).map_err(run::source)?;
        run.emit(RebuildEvent::Step {
            project: project.clone(),
            step: StepDto::Done,
            text: text.into(),
        })
    }
    fn preview(&self, project: &ProjectId) -> Result<String> {
        let run = self.project_run(project)?;
        if let Some(url) = run.preview.url() {
            return Ok(url);
        }
        let reply = self.shared.config.analysis.call(
            project,
            ToolCall {
                tool: "project_preview".into(),
                params: serde_json::json!({"project":run.root}),
                caller: CallerDto::Rebuild,
                consented: true,
            },
            &CancelFlag::new(),
        )?;
        let value = match reply {
            ToolReply::Value(v) | ToolReply::Evidence { body: v, .. } => v,
        };
        let url = value
            .get("url")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| Error::corrupt("预览没有返回地址"))?
            .to_owned();
        run.emit(RebuildEvent::Preview {
            project: project.clone(),
            url: url.clone(),
        })?;
        Ok(url)
    }
    fn play(&self, project: &ProjectId) -> Result<()> {
        let run = self.project_run(project)?;
        self.shared
            .config
            .analysis
            .call(
                project,
                ToolCall {
                    tool: "project_play".into(),
                    params: serde_json::json!({"project":run.root}),
                    caller: CallerDto::Rebuild,
                    consented: true,
                },
                &CancelFlag::new(),
            )
            .map(|_| ())
    }
    fn activity(
        &self,
        project: &ProjectId,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<ActivityLine>> {
        self.project_run(project)?
            .continuity
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .activity(offset, limit)
    }
    fn read_feed(&self, project: &ProjectId) -> Result<Vec<FeedEntryDto>> {
        self.shared.config.projects.open(project)?;
        let run = self
            .shared
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .runs
            .get(project)
            .cloned();
        if let Some(run) = run {
            run.continuity
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .read_feed()
        } else {
            feed::read_feed(Path::new(project.as_str()))
        }
    }
    fn attach_pictures(&self, project: &ProjectId, pictures: Vec<PathBuf>) -> Result<Vec<PathBuf>> {
        self.project_run(project)?
            .continuity
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .attach_pictures(pictures)
    }
    fn attach_image(&self, project: &ProjectId, bytes: Vec<u8>, mime: &str) -> Result<PathBuf> {
        self.project_run(project)?
            .continuity
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .attach_image(bytes, mime)
    }
}
