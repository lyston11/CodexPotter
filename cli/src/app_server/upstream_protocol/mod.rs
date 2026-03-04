//! Upstream `codex app-server` JSON-RPC protocol types.
//!
//! CodexPotter drives the upstream `codex app-server` binary as a subprocess. This module hosts
//! a small subset of the upstream JSON-RPC schema used by `cli/src/app_server/codex_backend.rs`
//! to serialize requests and deserialize responses/notifications.
//!
//! Notes:
//! - The JSON-RPC envelope is implemented in a lightweight form (`jsonrpc_lite`) to match the
//!   upstream wire format (no `"jsonrpc": "2.0"` field).
//! - The protocol is versioned; new upstream fields should be added in a backwards-compatible way
//!   where possible.

mod jsonrpc_lite;
mod protocol;

pub use jsonrpc_lite::*;
pub use protocol::common::*;
pub use protocol::v1::*;
pub use protocol::v2::*;
