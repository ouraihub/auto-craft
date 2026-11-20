//! Pi launch arguments, generated method extension and native readonly tool allowlist.
use crate::{
    host,
    process::{self, LaunchSpec},
    projection_engine,
};
use autocraft_core::{Result, SessionRole, StartSession};

pub(super) fn launch_spec(request: &StartSession) -> Result<LaunchSpec<'_>> {
    let mut host_args = vec!["--mode".to_owned(), "rpc".to_owned()];
    let mut host_env = Vec::new();
    let extension = crate::workflow::pi_extension(request);
    let path = host::write_support(
        &request.support_dir,
        projection_engine::PI_EXTENSION_FILE,
        &extension,
    )?;
    host_args.extend(["-e".to_owned(), path.display().to_string()]);
    if let Some(skills) = crate::workflow::stage(request)? {
        host_args.extend(["--skill".into(), skills.display().to_string()]);
    }
    host_args.extend(readonly_args(request));
    if let Some(access) = &request.analysis {
        host_env = host::env(access);
    }
    match &request.resume {
        Some(key) => host_args.extend(["--session".to_owned(), key.as_str().to_owned()]),
        None => host_args.extend(projection_engine::pi_model_args(&request.routing)),
    }
    host_env.extend(process::routing_env(&request.routing));
    Ok(LaunchSpec {
        launch: &request.launch,
        config_root: request.config_root.as_deref(),
        host_args,
        cwd: request.cwd.clone(),
        config_var: "PI_CODING_AGENT_DIR",
        host_env,
    })
}

pub(crate) fn readonly_args(request: &StartSession) -> Vec<String> {
    if request.role != SessionRole::ReadOnly {
        return Vec::new();
    }
    let mut args = ["--no-skills", "--tools", "read,grep,find,ls"]
        .map(str::to_owned)
        .to_vec();
    if request.resume.is_none() {
        args.push("--no-session".into());
    }
    args
}
