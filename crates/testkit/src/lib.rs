//! Disposable filesystem roots, a scripted loopback model, and a synthetic PE for tests only.
//! Consumers must depend on this crate through `[dev-dependencies]`.

mod isolation;
mod pe;
mod upstream;

pub use isolation::{Engine, Isolation};
pub use pe::pe64;
pub use upstream::{RecordedRequest, ScriptedUpstream, Turn};
