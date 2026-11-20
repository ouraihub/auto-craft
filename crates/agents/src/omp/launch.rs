//! OMP launch arguments: CLI flags, generated extension and overlay, and the `mcp.json` sync.

use autocraft_core::{EnvBinding, ProjectedMcpServer, ProjectedMcpTransport, Result, StartSession};

use crate::{
    host, omp_mcp,
    process::{self, LaunchSpec},
    projection_engine,
};

/// Host arguments and environment of one OMP launch: the extension (`-e`: providers), the
/// overlay (`--config`: MCP, default model) and the `autocraft-` entries synced into `mcp.json`.
pub(super) fn launch_spec(request: &StartSession) -> Result<LaunchSpec<'_>> {
    let mut host_args: Vec<String> = ["--mode", "rpc-ui", "--approval-mode", "always-ask"]
        .into_iter()
        .map(Into::into)
        .collect();
    if request.builder.is_some() && request.role == autocraft_core::SessionRole::Worker {
        host_args[3] = "yolo".into();
    }
    let mut servers = request.routing.mcp.clone();
    let mut host_env = Vec::new();
    if let Some(access) = &request.analysis {
        servers.push(ProjectedMcpServer {
            name: "autocraft-analysis".into(),
            transport: ProjectedMcpTransport::Stdio {
                command: process::plain_path(&access.executable)
                    .display()
                    .to_string(),
                args: vec!["mcp-host".into()],
                env: vec![
                    EnvBinding {
                        name: host::PIPE_VAR.into(),
                        var: host::PIPE_VAR.into(),
                    },
                    EnvBinding {
                        name: host::TOKEN_VAR.into(),
                        var: host::TOKEN_VAR.into(),
                    },
                ],
            },
        });
        host_env = host::env(access);
    }
    host_env.extend(process::routing_env(&request.routing));
    omp_mcp::sync(
        &process::config_dir(request.config_root.as_deref(), ".omp/agent")?,
        &servers,
    )?;
    let path = host::write_support(
        &request.support_dir,
        projection_engine::OMP_EXTENSION_FILE,
        &crate::workflow::omp_extension(request),
    )?;
    host_args.extend(["-e".to_owned(), path.display().to_string()]);
    let skills = crate::workflow::stage(request)?;
    if request.role == autocraft_core::SessionRole::ReadOnly {
        host_args.extend(["--no-skills", "--tools", "read"].map(str::to_owned));
    }
    let path = host::write_support(
        &request.support_dir,
        projection_engine::OMP_OVERLAY_FILE,
        &crate::workflow::omp_overlay(request, skills.as_deref()),
    )?;
    host_args.extend(["--config".to_owned(), path.display().to_string()]);
    if let Some(key) = &request.resume {
        host_args.extend(["--session".into(), key.as_str().to_owned()]);
    }
    Ok(LaunchSpec {
        launch: &request.launch,
        config_root: request.config_root.as_deref(),
        host_args,
        cwd: request.cwd.clone(),
        config_var: "PI_CODING_AGENT_DIR",
        host_env,
    })
}
