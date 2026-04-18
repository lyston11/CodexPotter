//! CodexPotter's project-level app-server.
//!
//! CodexPotter uses a long-lived "control plane" process to encapsulate project state and
//! multi-round orchestration. This module defines that app-server:
//!
//! - **Server** (`server`): a JSON-RPC server that exposes project-level methods like
//!   `project/start`, `project/resume`, and `project/start_rounds`.
//! - **Client** (`client`): a small helper for spawning the server (as a subprocess) and
//!   consuming the event stream.
//! - **Protocol** (`protocol`): request/response and event wire types. The message envelope mirrors
//!   upstream Codex app-server JSON-RPC to keep tooling consistent.
//!
//! Each project round is still executed by the upstream `codex app-server` backend driver
//! (see `crate::app_server::codex_backend`); the Potter app-server is responsible for the
//! higher-level "project lifecycle" and for persisting `potter-rollout.jsonl` via the workflow
//! layer.

pub mod client;
pub mod protocol;
pub mod server;

pub use client::PotterAppServerClient;
pub use protocol::*;
pub use server::PotterAppServerConfig;
pub use server::run_potter_app_server;
