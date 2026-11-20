//! Authenticated transparent analysis channel and the `mcp-host` byte forwarder.
mod bridge;
mod mcp;
#[cfg(test)]
mod tests;
pub use bridge::Bridge;
pub use mcp::run_mcp_stdio;
