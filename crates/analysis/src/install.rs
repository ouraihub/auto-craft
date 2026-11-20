//! Project-independent, explicitly consented installation into the application engine store.
use crate::{AnalysisHub, errors};
use autocraft_core::*;
use ca_engine::{CallOptions, Caller, CancelToken, Consent, Engine, Session, ToolOutput};
use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

pub(crate) fn engines(
    hub: &AnalysisHub,
    params: serde_json::Value,
    cancel: &CancelFlag,
) -> Result<ToolReply> {
    if cancel.is_cancelled() {
        return Err(Error::new(ErrorKind::Cancelled, "已取消安装"));
    }
    let mut session = Session::isolated(&hub.data_dir.join("analysis"));
    session.settings = hub.settings.lock().clone();
    session.settings.ai.clear();
    session.deep_for_run = Some(true);
    let engine = Engine::new(session);
    let token = CancelToken::default();
    let done = AtomicBool::new(false);
    let result = std::thread::scope(|scope| {
        let bridge = scope.spawn(|| {
            while !done.load(Ordering::Acquire) {
                if cancel.is_cancelled() {
                    token.cancel();
                    break;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        });
        let result = engine.call(
            "engines_install",
            params,
            CallOptions::new(Caller::Ui)
                .with_cancel(token.clone())
                .with_consent(Consent::Granted),
        );
        done.store(true, Ordering::Release);
        if let Err(payload) = bridge.join() {
            std::panic::resume_unwind(payload);
        }
        result.map_err(|e| errors::convert("engines_install", e))
    })?;
    match result {
        ToolOutput::Value(value) => Ok(ToolReply::Value(value)),
        ToolOutput::Evidence(ev) => Ok(ToolReply::Evidence {
            id: Some(ev.id.clone().into()),
            body: serde_json::to_value(ev).map_err(|_| Error::corrupt("无法读取安装结果"))?,
        }),
    }
}
