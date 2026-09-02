#![forbid(unsafe_code)]

use std::error::Error;
use std::process::Command;

fn server_command() -> Command {
    Command::new(env!("CARGO_BIN_EXE_dfmcp-live-v1-1-dev-server"))
}

fn clean_admission_environment(command: &mut Command) {
    for name in [
        "DFMCP_ADMISSION_TICKET",
        "DFMCP_ADMITTED_BRIDGE_PROTOCOL",
        "DFMCP_COMPATIBILITY_ENTRY_ID",
        "DFMCP_COMPATIBILITY_DECISION_DIGEST",
        "DFMCP_COMPATIBILITY_REGISTRY_DIGEST",
        "DFMCP_COMPATIBILITY_FLOOR_DIGEST",
        "DFMCP_COMPATIBILITY_FLOOR_FILE_SHA256",
        "DFMCP_COMPATIBILITY_FLOOR_SEQUENCE",
        "DFMCP_SERVER_RECEIPT_DIGEST",
        "DFMCP_ADMITTED_LAUNCH_DIGEST",
    ] {
        command.env_remove(name);
    }
}

#[test]
fn protocol_1_1_development_server_requires_exact_opt_in() -> Result<(), Box<dyn Error>> {
    let mut command = server_command();
    clean_admission_environment(&mut command);
    let output = command
        .env_remove("DFMCP_ALLOW_UNADMITTED_LIVE_V1_1")
        .output()?;
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr)?;
    assert!(stderr.contains("DFMCP_ALLOW_UNADMITTED_LIVE_V1_1=1 is required"));
    assert!(stderr.contains("explicitly unadmitted"));
    assert!(!stderr.contains("DFMCP_BRIDGE_TOKEN is required"));
    Ok(())
}

#[test]
fn protocol_1_1_development_server_rejects_production_admission_state() -> Result<(), Box<dyn Error>> {
    let mut command = server_command();
    clean_admission_environment(&mut command);
    let output = command
        .env("DFMCP_ALLOW_UNADMITTED_LIVE_V1_1", "1")
        .env("DFMCP_COMPATIBILITY_ENTRY_ID", "0".repeat(64))
        .output()?;
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr)?;
    assert!(stderr.contains("refuses production admission environment"));
    assert!(stderr.contains("DFMCP_COMPATIBILITY_ENTRY_ID"));
    assert!(!stderr.contains("DFMCP_BRIDGE_TOKEN is required"));
    Ok(())
}

#[test]
fn protocol_1_1_development_server_rejects_protocol_bound_admission_state() -> Result<(), Box<dyn Error>> {
    let mut command = server_command();
    clean_admission_environment(&mut command);
    let output = command
        .env("DFMCP_ALLOW_UNADMITTED_LIVE_V1_1", "1")
        .env("DFMCP_ADMITTED_BRIDGE_PROTOCOL", "1.1")
        .output()?;
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr)?;
    assert!(stderr.contains("refuses production admission environment"));
    assert!(stderr.contains("DFMCP_ADMITTED_BRIDGE_PROTOCOL"));
    assert!(!stderr.contains("DFMCP_BRIDGE_TOKEN is required"));
    Ok(())
}

#[test]
fn near_miss_opt_in_values_fail_before_bridge_configuration() -> Result<(), Box<dyn Error>> {
    for value in ["true", "01", "yes", "1 "] {
        let mut command = server_command();
        clean_admission_environment(&mut command);
        let output = command
            .env("DFMCP_ALLOW_UNADMITTED_LIVE_V1_1", value)
            .output()?;
        assert!(!output.status.success(), "near-miss opt-in {value:?} was accepted");
        let stderr = String::from_utf8(output.stderr)?;
        assert!(stderr.contains("DFMCP_ALLOW_UNADMITTED_LIVE_V1_1=1 is required"));
        assert!(!stderr.contains("DFMCP_BRIDGE_TOKEN is required"));
    }
    Ok(())
}
