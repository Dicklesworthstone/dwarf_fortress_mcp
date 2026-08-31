#![forbid(unsafe_code)]

//! Doctor Diagnostics, Health Probes & Live Telemetry Inspector.
//!
//! WP-MCP-03: Gathers deep diagnostic telemetry (sessions, leases, obligations,
//! adapter status, IPC throughput, WAL size) and produces structured health audits.

use dfmcp_adapter::{AdapterHealth, HealthStatus, IpcTelemetry};

/// Comprehensive system health and diagnostic audit report.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DoctorDiagnosticReport {
    pub server_version: String,
    pub active_sessions_count: usize,
    pub is_adapter_healthy: bool,
    pub adapter_details: String,
    pub ipc_telemetry: IpcTelemetry,
    pub active_leases_count: usize,
    pub active_obligations_count: usize,
    pub findings: Vec<String>,
    pub is_healthy: bool,
}

/// Telemetry and Diagnostics Inspector for `fortress_doctor`.
#[derive(Clone, Debug, Default)]
pub struct DoctorInspector;

impl DoctorInspector {
    /// Perform a full system health audit.
    #[must_use]
    pub fn generate_report(
        &self,
        active_sessions_count: usize,
        adapter_health: Option<&AdapterHealth>,
        ipc_telemetry: Option<&IpcTelemetry>,
        active_leases_count: usize,
        active_obligations_count: usize,
    ) -> DoctorDiagnosticReport {
        let mut findings = Vec::new();
        let mut is_healthy = true;

        let (is_adapter_healthy, adapter_details) = match adapter_health {
            Some(health) => {
                let healthy = health.status == HealthStatus::Healthy;
                if !healthy {
                    is_healthy = false;
                    findings.push(format!("adapter health degraded: {:?}", health.status));
                }
                (
                    healthy,
                    format!(
                        "status={:?}, loaded={}, warnings={:?}",
                        health.status, health.fortress_loaded, health.warnings
                    ),
                )
            }
            None => {
                findings.push(
                    "no live adapter connected (running in pure simulation lab mode)".to_owned(),
                );
                (true, "simulation lab".to_owned())
            }
        };

        let telemetry = ipc_telemetry.cloned().unwrap_or_default();
        if telemetry.crc_errors > 0 {
            findings.push(format!(
                "detected {} CRC32 framing errors on IPC stream",
                telemetry.crc_errors
            ));
        }

        if active_sessions_count == 0 {
            findings.push("no active client sessions open".to_owned());
        }

        DoctorDiagnosticReport {
            server_version: "0.1.0".to_owned(),
            active_sessions_count,
            is_adapter_healthy,
            adapter_details,
            ipc_telemetry: telemetry,
            active_leases_count,
            active_obligations_count,
            findings,
            is_healthy,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_doctor_inspector_health_audit() {
        let inspector = DoctorInspector;
        let report = inspector.generate_report(1, None, None, 2, 0);

        assert_eq!(report.server_version, "0.1.0");
        assert_eq!(report.active_sessions_count, 1);
        assert!(report.is_healthy);
        assert_eq!(report.active_leases_count, 2);
    }
}
