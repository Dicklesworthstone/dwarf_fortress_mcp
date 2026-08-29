#![forbid(unsafe_code)]
//! MCP presentation plane for the Dwarf Fortress semantic control plane.
//!
//! `dfmcp-mcp` binds the frozen 11-tool `fortress.*` narrow waist to the owned
//! [`fastmcp_rust`](https://github.com/Dicklesworthstone/fastmcp_rust) sibling
//! (the `fastmcp-rust` facade) and runs it as a modern-only MCP 2026-07-28
//! server. The plane is deliberately thin: every semantic decision is
//! delegated to `dfmcp-core`, `dfmcp-world`, `dfmcp-intent`, `dfmcp-adapter`,
//! and `dfmcp-lab`. This crate adds transport and session framing, and
//! nothing else. It must never become an authority: no tool here may bypass
//! plan sealing, commit-time revalidation, idempotency, or evidence checks
//! (ADR-013, `docs/FASTMCP_INTEGRATION.md`).

pub mod server;

pub use server::run_stdio;
