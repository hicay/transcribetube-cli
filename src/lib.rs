//! TranscribeTube CLI and MCP server.
//!
//! `tube` (src/main.rs) and `transcribetube-mcp` (src/bin) are thin entry
//! points over this library so the CLI command and the MCP tools share one
//! implementation.

pub mod api;
pub mod cli;
pub mod config;
pub mod format;
pub mod mcp;
pub mod output;
pub mod transcribe;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
