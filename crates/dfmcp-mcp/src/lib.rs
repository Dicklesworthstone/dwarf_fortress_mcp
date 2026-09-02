#![forbid(unsafe_code)]
//! MCP presentation plane for the Dwarf Fortress semantic control plane.
//!
//! `dfmcp-mcp` binds the frozen 11-tool `fortress.*` narrow waist to the owned
//! [`fastmcp_rust`](https://github.com/Dicklesworthstone/fastmcp_rust) sibling
//! (the `fastmcp-rust` facade) and runs it as a modern-only MCP 2026-07-28
//! server. The plane is deliberately thin: every semantic decision is
//! delegated to `dfmcp-core`, `dfmcp-world`, `dfmcp-intent`, `dfmcp-adapter`,
//! and `dfmcp-lab`. This crate adds transport, session framing, and the
//! authority-free canonical Agent Turn Packet projection. It must never become
//! an authority: no tool here may bypass plan sealing, commit-time
//! revalidation, idempotency, or evidence checks (ADR-013,
//! `docs/FASTMCP_INTEGRATION.md`).

pub mod admission;
pub mod agent_facade;
pub mod agent_turn;
pub mod doctor;
pub mod ee_memory;
pub mod http_transport;
mod live_server;
mod live_server_v1_1;
pub mod server;
pub mod tasks;

pub use admission::{AdmissionProvenance, current_admission_provenance, run_live_stdio};
pub use agent_facade::run_stdio;
pub use agent_turn::{
    AGENT_TURN_SCHEMA, AgentPhase, AgentTurnBuilder, ContinuityStatus, ObservationProfile,
    RecoveryClass, empty_active_work, empty_budget, empty_coverage, recommendation,
    recovery_guidance, uncertainty,
};
pub use doctor::{DoctorDiagnosticReport, DoctorInspector};
pub use ee_memory::{EeMemoryBatch, EeMemoryItem};
pub use http_transport::{
    HttpSessionResumeToken, HttpTransportSessionManager, MAX_HTTP_MESSAGE_BYTES,
    MAX_HTTP_SESSION_BUFFER_BYTES, MAX_HTTP_SESSIONS, MAX_HTTP_TOTAL_BUFFER_BYTES,
    MAX_RESUMPTION_BUFFER_SIZE,
};
pub use live_server_v1_1::run_live_v1_1_development_stdio;
pub use server::validate_localhost_bind;
pub use tasks::{McpTaskProjection, McpTaskStatus, cancel_action_task, project_action_task};
