//! Blocking launch transaction: project routing, analysis lease, bridge token and native binding.
use super::AgentPanel;
use crate::{services::Services, state::run};
use autocraft_core::{
    AgentSession, ChatBinding, ChatId, ChatPurpose, ChatScope, Error, ErrorCode as _, ProjectId,
    Readiness, Result, RouteRequest, SessionRole, StartSession,
};
use std::sync::Arc;
use tracing::{info, warn};

/// The token and analysis hold are owned by exactly one live engine, including across view switches.
pub(super) struct LaunchLease {
    project: ProjectId,
    chat: ChatId,
    token: Option<String>,
    held: bool,
    owner: Option<Services>,
    session: Option<Arc<dyn AgentSession>>,
}
impl Drop for LaunchLease {
    fn drop(&mut self) {
        let Some(services) = self.owner.take() else {
            return;
        };
        let mut lease = LaunchLease {
            project: self.project.clone(),
            chat: self.chat.clone(),
            token: self.token.take(),
            held: self.held,
            owner: None,
            session: None,
        };
        let session = self.session.take();
        let span = tracing::Span::current();
        // Release may block on process exit; even an undelivered state::run result owns cleanup.
        std::thread::spawn(move || {
            span.in_scope(|| {
                if let Err(error) = cleanup(&services, &mut lease, session, false) {
                    warn!(code = error.code(), "agent.abandoned.cleanup_failed");
                }
            })
        });
    }
}
pub(super) struct Started {
    pub session: Arc<dyn AgentSession>,
    pub binding: ChatBinding,
    pub signature: String,
    pub lease: LaunchLease,
    pub prompt_error: Option<Error>,
}
pub(super) struct StartInput {
    pub services: Services,
    pub chat: ChatId,
    pub project: ProjectId,
    pub binding: Option<ChatBinding>,
    pub prompt: Option<String>,
}

pub(super) fn start_engine(input: StartInput) -> Result<std::result::Result<Started, Readiness>> {
    let services = &input.services;
    let prefs = services.project_prefs.get(&input.project)?;
    if input.binding.is_none()
        && prefs.backend.is_none()
        && services.prefs.prefs()?.default_agent.is_none()
    {
        return Ok(Err(services.readiness()?.0));
    }
    let route = autocraft_routing::resolve_project_route(
        &input.project,
        &input.chat,
        services.project_prefs.as_ref(),
        services.prefs.as_ref(),
        services.chats.as_ref(),
    )?;
    let backend = route.backend;
    let Some(launch) = services.launch(backend)? else {
        return Ok(Err(Readiness::DefaultMissing(backend)));
    };
    let mut lease = LaunchLease {
        project: input.project.clone(),
        chat: input.chat.clone(),
        token: None,
        held: false,
        owner: Some(services.clone()),
        session: None,
    };
    let prepared = (|| {
        let request = route;
        let routing = autocraft_routing::prepare_project_route(services.router.as_ref(), &request)?;
        let current_model = routing.default_model.clone();
        let signature = launch_signature(
            services,
            &RouteRequest {
                new_session: false,
                ..request
            },
        )?;
        let support_dir = services.support_dir(&input.chat)?;
        let cwd = services.project_dir(&input.project)?;
        services.analysis.open(&input.project)?;
        services.hold_analysis(&input.project, &input.chat)?;
        lease.held = true;
        let options = services.channel_options(backend);
        let access = services.register_analysis(
            ChatScope {
                project: input.project.clone(),
                chat: input.chat.clone(),
                purpose: ChatPurpose::Analysis,
            },
            options,
        )?;
        lease.token = Some(access.token.clone());
        let session = services.runtime(backend).start(
            StartSession {
                chat: input.chat.clone(),
                project: input.project.clone(),
                backend,
                launch,
                config_root: None,
                cwd,
                resume: input.binding.as_ref().map(|b| b.native_session.clone()),
                analysis: Some(access),
                builder: None,
                routing,
                support_dir,
                role: SessionRole::Worker,
            },
            services.session_sink.clone(),
        )?;
        if input.binding.is_some()
            && let Some(model) = &current_model
        {
            session.set_model(&model.route, &model.model_id, model.effort)?;
        }
        let binding_result = match input.binding {
            Some(binding) => Ok(binding),
            None => session
                .snapshot()
                .native_session_key
                .ok_or_else(|| Error::protocol(rust_i18n::t!("app.agent.missing_locator")))
                .and_then(|native_session| {
                    let binding = ChatBinding {
                        project: input.project.clone(),
                        chat: input.chat.clone(),
                        backend,
                        native_session,
                    };
                    services.chats.bind(binding.clone())?;
                    Ok(binding)
                }),
        };
        match binding_result {
            Ok(binding) => Ok((session, binding, signature)),
            Err(error) => {
                if let Err(cleanup) = session.dispose() {
                    warn!(code = cleanup.code(), "agent.launch.dispose_failed");
                }
                Err(error)
            }
        }
    })();
    match prepared {
        Ok((session, binding, signature)) => {
            lease.session = Some(session.clone());
            let prompt_error = input.prompt.and_then(|text| session.prompt(&text).err());
            info!(chat_id = %input.chat, backend = backend.label(), "agent.session.started");
            Ok(Ok(Started {
                session,
                binding,
                signature,
                lease,
                prompt_error,
            }))
        }
        Err(error) => {
            if let Err(cleanup) = cleanup(services, &mut lease, None, false) {
                warn!(code = cleanup.code(), "agent.launch.cleanup_failed");
            }
            Err(error)
        }
    }
}

fn first(result: &mut Result<()>, next: Result<()>) {
    if let Err(error) = next {
        if result.is_ok() {
            *result = Err(error);
        } else {
            warn!(code = error.code(), "agent.cleanup.secondary_failure");
        }
    }
}
fn cleanup(
    services: &Services,
    lease: &mut LaunchLease,
    session: Option<Arc<dyn AgentSession>>,
    unbind: bool,
) -> Result<()> {
    lease.owner = None;
    lease.session = None;
    let mut result = session.map_or(Ok(()), |session| session.dispose());
    services.router.release(&lease.chat);
    first(&mut result, services.remove_support_dir(&lease.chat));
    if let Some(token) = &lease.token {
        services.revoke_analysis(token);
    }
    if lease.held {
        first(
            &mut result,
            services.release_analysis(&lease.project, &lease.chat),
        );
    }
    if unbind {
        first(
            &mut result,
            services.chats.remove_binding(&lease.project, &lease.chat),
        );
    }
    result
}

pub(super) fn dispose_engine(
    services: Services,
    project: ProjectId,
    chat: ChatId,
    session: Option<Arc<dyn AgentSession>>,
    lease: Option<LaunchLease>,
    unbind: bool,
    cx: &mut gpui_kit::Context<AgentPanel>,
) {
    let mut lease = lease.unwrap_or_else(|| LaunchLease {
        project,
        chat: chat.clone(),
        token: None,
        held: false,
        owner: None,
        session: None,
    });
    run(
        cx,
        move || cleanup(&services, &mut lease, session, unbind),
        move |this, result, cx| {
            if let Some(chat) = this.chats.get_mut(&chat) {
                chat.closing = false;
            }
            if let Err(error) = result {
                this.report(&error, cx);
            }
            cx.notify();
        },
    );
}

pub(super) fn launch_signature(services: &Services, request: &RouteRequest) -> Result<String> {
    let signature = services.router.signature(request)?;
    let mode = services.project_prefs.get(&request.project)?.mode;
    Ok(format!("{signature}/{mode:?}"))
}
