#![forbid(unsafe_code)]

//! Integration tests for WP-MCP-03 Doctor Diagnostics & Telemetry Inspector.

use std::collections::BTreeSet;

use dfmcp_adapter::ipc::IpcTelemetry;
use dfmcp_adapter::{AdapterHealth, AdapterIdentity, CompatibilityLevel, HealthStatus};
use dfmcp_core::Digest32;
use dfmcp_mcp::doctor::DoctorInspector;

#[test]
fn test_doctor_diagnostics_report() {
    let inspector = DoctorInspector;
    let identity = AdapterIdentity {
        name: "test_adapter".to_owned(),
        adapter_version: "0.1.0".to_owned(),
        bridge_protocol_version: "2026-07-28".to_owned(),
        dwarf_fortress_version: "50.14".to_owned(),
        dfhack_version: "50.14-r1".to_owned(),
        compatibility: CompatibilityLevel::Exact,
        capabilities: BTreeSet::new(),
        schema_digest: Digest32::ZERO,
    };

    let health = AdapterHealth {
        status: HealthStatus::Healthy,
        identity,
        fortress_loaded: true,
        paused: Some(false),
        current_anchor: None,
        warnings: Vec::new(),
    };

    let telemetry = IpcTelemetry {
        frames_sent: 50,
        frames_received: 50,
        bytes_sent: 1024,
        bytes_received: 2048,
        crc_errors: 0,
        reconnect_attempts: 0,
    };

    let report = inspector.generate_report(2, Some(&health), Some(&telemetry), 3, 1);

    assert!(report.is_healthy);
    assert_eq!(report.active_sessions_count, 2);
    assert!(report.is_adapter_healthy);
    assert_eq!(report.active_leases_count, 3);
    assert_eq!(report.active_obligations_count, 1);
}
