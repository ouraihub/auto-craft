//! Provider routing for engine launches: route resolution and signatures, the loopback gateway
//! and provider probes.

mod gateway;
mod probe;
mod project;
mod router;
mod runtime;
mod signature;
mod url;

pub use gateway::Gateway;
pub use probe::Probe;
pub use project::{prepare_project_route, resolve_project_route};
pub use router::{GatewayRoutes, Router};
pub use url::{models_url, preview};

#[cfg(test)]
mod tests;
