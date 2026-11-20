//! Domain types, service contracts and the error model shared by every autocraft crate.
//!
//! This crate has no I/O and no UI: the service crates implement the traits, `autocraft`
//! consumes them through trait objects.

mod agent;
mod analysis;
mod app_prefs;
mod appearance;
mod asset_keys;
mod channel;
mod chat_binding;
mod commands;
mod engine_setup;
mod error;
mod ids;
mod inbox;
mod mcp;
mod model_catalog;
mod policy;
mod preset;
mod project;
mod provider;
mod provider_edit;
mod rebuild;
mod routing;
mod terminal;
mod usage;

pub use agent::*;
pub use analysis::*;
pub use app_prefs::*;
pub use appearance::*;
pub use asset_keys::*;
pub use channel::*;
pub use chat_binding::*;
pub use commands::*;
pub use engine_setup::*;
pub use error::{Error, ErrorCode, ErrorKind, IoContext, Result};
pub use ids::*;
pub use inbox::*;
pub use mcp::*;
pub use model_catalog::*;
pub use policy::*;
pub use preset::*;
pub use project::*;
pub use provider::*;
pub use provider_edit::*;
pub use rebuild::*;
pub use routing::*;
pub use terminal::*;
pub use usage::*;

#[cfg(test)]
mod contracts_tests;
