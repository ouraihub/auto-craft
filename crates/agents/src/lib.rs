//! pi / OMP / DSH official process protocols: one `AgentRuntime` per backend.
//!
//! `process`, `framing`, `projection` and `session` are shared by all three; `rpc` is shared by the
//! pi-family backends (pi, OMP); `projection_engine` renders `EngineRouting` into each engine's
//! files and parameters, and `omp_mcp` merges OMP's `mcp.json`. Backend modules never call each
//! other.

mod approvals;
mod dsh;
mod framing;
mod host;
mod install;
mod omp;
mod omp_mcp;
mod pi;
mod process;
mod projection;
mod projection_engine;
mod rpc;
mod session;
#[cfg(test)]
mod smoke;
#[cfg(test)]
mod testing;
mod trace;
mod workflow;
#[cfg(test)]
mod workflow_tests;

/// P05 observed real notifications and tools/list refresh for pi, OMP and DSH.
pub const ALL_TOOLS: [(autocraft_core::BackendKind, bool); 3] = [
    (autocraft_core::BackendKind::Pi, false),
    (autocraft_core::BackendKind::Omp, false),
    (autocraft_core::BackendKind::Dsh, false),
];

/// Analysis-channel policy for a new native launch, backed by the P04/P05 observations.
pub fn channel_options(backend: autocraft_core::BackendKind) -> autocraft_core::ChannelOptions {
    let index = match backend {
        autocraft_core::BackendKind::Pi => 0,
        autocraft_core::BackendKind::Omp => 1,
        autocraft_core::BackendKind::Dsh => 2,
    };
    autocraft_core::ChannelOptions {
        all_tools: ALL_TOOLS[index].1,
        async_long_tools: backend == autocraft_core::BackendKind::Dsh,
    }
}

pub use dsh::DshRuntime;
pub use install::ManagedEngines;
pub use omp::OmpRuntime;
pub use pi::PiRuntime;
