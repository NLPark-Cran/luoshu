//! 洛书 Luoshu — local device bridge.
//!
//! A loopback-only MCP (Model Context Protocol) server that exposes local
//! device capabilities (system info, sandboxed filesystem, shell, screenshot,
//! embedded-browser control) to cloud agents running on the Cran Code
//! platform (https://crys.tt2.li).
//!
//! Design notes:
//! - **Transport**: MCP "Streamable HTTP" subset — a single `POST /mcp`
//!   endpoint speaking JSON-RPC 2.0 (`initialize`, `ping`, `tools/list`,
//!   `tools/call`). No SSE streams in v1; see docs/003-mvp-architecture.md.
//! - **Auth**: every request requires `Authorization: Bearer <token>` with a
//!   per-run random token written (0600) to `~/.luoshu/bridge.json`.
//! - **Safety**: filesystem sandbox (allowed roots + sensitive-pattern
//!   refusal), dangerous-command approval flow with one-time tokens.

pub mod approval;
pub mod auth;
pub mod browser;
pub mod config;
pub mod eval_relay;
pub mod mcp;
pub mod sandbox;
pub mod server;
pub mod tools;
pub mod tunnel;
