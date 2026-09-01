#![forbid(unsafe_code)]

//! Explicitly unqualified protocol-1.1 announcement probe.
//!
//! This binary exists to produce development evidence before a fresh native
//! and disposable-fort campaign admits any protocol-1.1 tuple. It cannot start
//! MCP, mutate Dwarf Fortress, promote compatibility, or issue an admission
//! ticket.

use std::env;
use std::error::Error;
use std::process::ExitCode;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use dfmcp_adapter::{
    BridgeCredentialsV1_1, LiveConnectionConfig, MAX_ANNOUNCEMENTS_PER_BATCH,
    MAX_CAPSULE_CITIZENS, MAX_V1_1_CITIZENS_PER_PAGE,
    connect_authenticated_live_source_v1_1, derive_live_fortress_id,
    parse_loopback_endpoint, project_live_capsule_v1_1,
    read_complete_observation_v1_1_bounded,
};
use dfmcp_core::{Digest32, ObservationCursor};
use serde_json::json;

const ENABLE_ENV: &str = "DFMCP_ALLOW_UNQUALIFIED_ANNOUNCEMENT_PROBE";

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(failure) => {
            eprintln!("dfmcp-live-announcement-probe: {failure}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    match env::args().nth(1).as_deref() {
        None | Some("help" | "--help" | "-h") => {
            print_help();
            Ok(())
        }
        Some("read") => read(),
        Some(other) => Err(format!("unknown command {other:?}; run with help").into()),
    }
}

fn print_help() {
    println!(
        "\
dfmcp-live-announcement-probe {version}
Unadmitted diagnostic reader for dfmcp bridge protocol 1.1

USAGE:
    dfmcp-live-announcement-probe help
    DFMCP_ALLOW_UNQUALIFIED_ANNOUNCEMENT_PROBE=1 \\
      DFMCP_BRIDGE_TOKEN='<32..256 bytes>' \\
      dfmcp-live-announcement-probe read

ENVIRONMENT:
    DFMCP_ALLOW_UNQUALIFIED_ANNOUNCEMENT_PROBE  Must be exactly 1 for read
    DFMCP_BRIDGE_ENDPOINT                       Numeric loopback IP:port
    DFMCP_BRIDGE_TOKEN                          Shared loopback secret
    DFMCP_BRIDGE_CONNECT_MILLIS                 1..60000, default 2000
    DFMCP_BRIDGE_READ_MILLIS                    1..60000, default 5000
    DFMCP_BRIDGE_WRITE_MILLIS                   1..60000, default 5000
    DFMCP_BRIDGE_PAGE_SIZE                      1..4096, default 4096
    DFMCP_BRIDGE_MAX_CITIZENS                   0..100000, default 100000
    DFMCP_BRIDGE_INCLUDE_NAMES                  true/false, default true
    DFMCP_ANNOUNCEMENT_AFTER_ID                 -1 or nonnegative, default -1
    DFMCP_MAX_ANNOUNCEMENTS                     1..512, default 128

This command emits development evidence only. It does not establish compatibility,
runtime admission, complete report history, or mutation authority.
",
        version = env!("CARGO_PKG_VERSION")
    );
}

fn read() -> Result<(), Box<dyn Error>> {
    if env::var(ENABLE_ENV).as_deref() != Ok("1") {
        return Err(format!(
            "{ENABLE_ENV}=1 is required because protocol 1.1 has no admitted compatibility tuple"
        )
        .into());
    }
    let endpoint_text = env_string("DFMCP_BRIDGE_ENDPOINT", "127.0.0.1:5000")?;
    let endpoint = parse_loopback_endpoint(&endpoint_text)?;
    let token = env::var("DFMCP_BRIDGE_TOKEN").map_err(|_| {
        "DFMCP_BRIDGE_TOKEN is required and must match the protocol-1.1 DFHack process"
    })?;
    let nonce = probe_nonce(&endpoint_text)?;
    let credentials = BridgeCredentialsV1_1::new(token.into_bytes(), nonce)?;
    let config = LiveConnectionConfig {
        endpoint,
        connect_timeout: Duration::from_millis(env_u64(
            "DFMCP_BRIDGE_CONNECT_MILLIS",
            2_000,
            1,
            60_000,
        )?),
        read_timeout: Duration::from_millis(env_u64(
            "DFMCP_BRIDGE_READ_MILLIS",
            5_000,
            1,
            60_000,
        )?),
        write_timeout: Duration::from_millis(env_u64(
            "DFMCP_BRIDGE_WRITE_MILLIS",
            5_000,
            1,
            60_000,
        )?),
        client_name: "dfmcp-live-announcement-probe".to_owned(),
        client_version: env!("CARGO_PKG_VERSION").to_owned(),
    };
    let page_size = env_u32(
        "DFMCP_BRIDGE_PAGE_SIZE",
        MAX_V1_1_CITIZENS_PER_PAGE,
        1,
        MAX_V1_1_CITIZENS_PER_PAGE,
    )?;
    let max_citizens_hard = u32::try_from(MAX_CAPSULE_CITIZENS)
        .map_err(|_| "MAX_CAPSULE_CITIZENS does not fit u32")?;
    let max_citizens = env_u32(
        "DFMCP_BRIDGE_MAX_CITIZENS",
        max_citizens_hard,
        0,
        max_citizens_hard,
    )?;
    let include_names = env_bool("DFMCP_BRIDGE_INCLUDE_NAMES", true)?;
    let announcement_after_id = env_i32(
        "DFMCP_ANNOUNCEMENT_AFTER_ID",
        -1,
        -1,
        i32::MAX,
    )?;
    let max_announcements_hard = u32::try_from(MAX_ANNOUNCEMENTS_PER_BATCH)
        .map_err(|_| "MAX_ANNOUNCEMENTS_PER_BATCH does not fit u32")?;
    let max_announcements = env_u32(
        "DFMCP_MAX_ANNOUNCEMENTS",
        128,
        1,
        max_announcements_hard,
    )?;

    let mut source = connect_authenticated_live_source_v1_1(&config, credentials)?;
    let capsule = read_complete_observation_v1_1_bounded(
        &mut source,
        page_size,
        include_names,
        max_citizens,
        announcement_after_id,
        max_announcements,
    )?;
    let fortress_id = derive_live_fortress_id(&capsule.base)?;
    let projection = project_live_capsule_v1_1(
        &capsule,
        fortress_id,
        ObservationCursor::ORIGIN,
    )?;
    projection.validate_against(&capsule)?;

    let suffix = projection
        .receipt
        .coverage()
        .domains
        .get("fortress.announcements.retained_suffix")
        .ok_or("announcement retained-suffix coverage is missing")?;
    let history = projection
        .receipt
        .coverage()
        .domains
        .get("fortress.announcements.history")
        .ok_or("announcement history coverage is missing")?;
    let output = json!({
        "schema": "dfmcp.unqualified-live-announcement-probe/1",
        "status": "development_evidence_unadmitted",
        "claims": {
            "compatibility_admitted": false,
            "runtime_admitted": false,
            "complete_announcement_history": false,
            "mutation_authority": false,
        },
        "endpoint": endpoint_text,
        "bridge": {
            "plugin": "dfmcp_bridge_v1_1",
            "protocol": "1.1",
            "version": capsule.base.bridge.bridge_version,
            "generation": capsule.base.bridge.bridge_generation,
            "dfhack_version": capsule.base.bridge.dfhack_version,
            "dwarf_fortress_version": capsule.base.bridge.df_version,
            "supported_methods": capsule.base.bridge.supported_methods,
            "mutation_methods": [],
        },
        "fortress": {
            "fortress_id": fortress_id.to_string(),
            "site_id": capsule.base.site_id,
            "world_name": capsule.base.world_name,
            "world_folder": capsule.base.world_folder,
            "paused": capsule.base.paused,
            "calendar_year": capsule.base.current_year,
            "year_tick": capsule.base.current_year_tick,
            "citizens": capsule.base.citizen_coverage.total,
        },
        "announcements": {
            "requested_after_id": capsule.announcement_batch.coverage.requested_after_id,
            "oldest_available_id": capsule.announcement_batch.coverage.oldest_available_id,
            "latest_available_id": capsule.announcement_batch.coverage.latest_available_id,
            "returned": capsule.announcement_batch.coverage.returned,
            "next_after_id": capsule.announcement_batch.coverage.next_after_id,
            "complete_through_latest": capsule.announcement_batch.coverage.complete_through_latest,
            "gap_before_retained_window": capsule.announcement_batch.coverage.has_gap(),
            "continuation": projection.receipt.coverage().continuation,
            "suffix_coverage": suffix.status.as_str(),
            "history_coverage": history.status.as_str(),
            "history_reason": history.reason,
            "records": capsule.announcement_batch.announcements.iter().map(|record| json!({
                "report_id": record.report_id,
                "report_type": record.report_type,
                "text": record.text,
                "year": record.year,
                "year_tick": record.year_tick,
                "repeat_count": record.repeat_count,
                "continuation": record.continuation,
                "unconscious": record.unconscious,
                "announcement": record.announcement,
            })).collect::<Vec<_>>(),
        },
        "evidence": {
            "combined_capsule_digest": capsule.content_digest.to_string(),
            "citizen_capsule_digest": capsule.base.content_digest.to_string(),
            "announcement_batch_digest": capsule.announcement_batch.content_digest.to_string(),
            "snapshot_state_hash": projection.snapshot.state_hash.to_string(),
            "projection_schema": projection.receipt.schema(),
            "entities": projection.snapshot.graph.entities.len(),
            "edges": projection.snapshot.graph.edges.len(),
        },
    });
    let client = source.into_inner()?;
    let _stream = client.close()?;
    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}

fn probe_nonce(endpoint: &str) -> Result<Vec<u8>, Box<dyn Error>> {
    let elapsed = SystemTime::now().duration_since(UNIX_EPOCH)?;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"dfmcp-unqualified-announcement-probe-v1\0");
    bytes.extend_from_slice(&elapsed.as_nanos().to_be_bytes());
    bytes.extend_from_slice(&std::process::id().to_be_bytes());
    bytes.extend_from_slice(endpoint.as_bytes());
    Ok(Digest32::of_bytes(&bytes).as_bytes().to_vec())
}

fn env_string(name: &str, default: &str) -> Result<String, Box<dyn Error>> {
    match env::var(name) {
        Ok(value) if !value.is_empty() => Ok(value),
        Ok(_) => Err(format!("{name} must not be empty").into()),
        Err(env::VarError::NotPresent) => Ok(default.to_owned()),
        Err(env::VarError::NotUnicode(_)) => Err(format!("{name} must be valid UTF-8").into()),
    }
}

fn parse_u64(
    name: &str,
    raw: &str,
    minimum: u64,
    maximum: u64,
) -> Result<u64, Box<dyn Error>> {
    let value = raw
        .parse::<u64>()
        .map_err(|_| format!("{name} must be a decimal u64"))?;
    if value < minimum || value > maximum {
        return Err(format!("{name} must be in {minimum}..={maximum}, got {value}").into());
    }
    Ok(value)
}

fn env_u64(
    name: &str,
    default: u64,
    minimum: u64,
    maximum: u64,
) -> Result<u64, Box<dyn Error>> {
    match env::var(name) {
        Ok(raw) => parse_u64(name, &raw, minimum, maximum),
        Err(env::VarError::NotPresent) => Ok(default),
        Err(env::VarError::NotUnicode(_)) => Err(format!("{name} must be valid UTF-8").into()),
    }
}

fn parse_u32(
    name: &str,
    raw: &str,
    minimum: u32,
    maximum: u32,
) -> Result<u32, Box<dyn Error>> {
    let value = parse_u64(name, raw, u64::from(minimum), u64::from(maximum))?;
    Ok(u32::try_from(value).map_err(|_| format!("{name} does not fit u32"))?)
}

fn env_u32(
    name: &str,
    default: u32,
    minimum: u32,
    maximum: u32,
) -> Result<u32, Box<dyn Error>> {
    match env::var(name) {
        Ok(raw) => parse_u32(name, &raw, minimum, maximum),
        Err(env::VarError::NotPresent) => Ok(default),
        Err(env::VarError::NotUnicode(_)) => Err(format!("{name} must be valid UTF-8").into()),
    }
}

fn parse_i32(
    name: &str,
    raw: &str,
    minimum: i32,
    maximum: i32,
) -> Result<i32, Box<dyn Error>> {
    let value = raw
        .parse::<i32>()
        .map_err(|_| format!("{name} must be a decimal i32"))?;
    if value < minimum || value > maximum {
        return Err(format!("{name} must be in {minimum}..={maximum}, got {value}").into());
    }
    Ok(value)
}

fn env_i32(
    name: &str,
    default: i32,
    minimum: i32,
    maximum: i32,
) -> Result<i32, Box<dyn Error>> {
    match env::var(name) {
        Ok(raw) => parse_i32(name, &raw, minimum, maximum),
        Err(env::VarError::NotPresent) => Ok(default),
        Err(env::VarError::NotUnicode(_)) => Err(format!("{name} must be valid UTF-8").into()),
    }
}

fn parse_bool(name: &str, raw: &str) -> Result<bool, Box<dyn Error>> {
    match raw {
        "1" | "true" | "TRUE" | "yes" | "YES" => Ok(true),
        "0" | "false" | "FALSE" | "no" | "NO" => Ok(false),
        _ => Err(format!("{name} must be true/false or 1/0").into()),
    }
}

fn env_bool(name: &str, default: bool) -> Result<bool, Box<dyn Error>> {
    match env::var(name) {
        Ok(raw) => parse_bool(name, &raw),
        Err(env::VarError::NotPresent) => Ok(default),
        Err(env::VarError::NotUnicode(_)) => Err(format!("{name} must be valid UTF-8").into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_numeric_parsers_fail_closed() {
        assert!(parse_u32("value", "0", 1, 10).is_err());
        assert!(parse_u32("value", "11", 1, 10).is_err());
        assert!(parse_u32("value", "not-a-number", 1, 10).is_err());
        assert_eq!(parse_u32("value", "7", 1, 10).ok(), Some(7));

        assert!(parse_i32("cursor", "-2", -1, 10).is_err());
        assert!(parse_i32("cursor", "11", -1, 10).is_err());
        assert_eq!(parse_i32("cursor", "-1", -1, 10).ok(), Some(-1));
    }

    #[test]
    fn boolean_parser_accepts_only_documented_forms() {
        assert_eq!(parse_bool("flag", "true").ok(), Some(true));
        assert_eq!(parse_bool("flag", "0").ok(), Some(false));
        assert!(parse_bool("flag", "maybe").is_err());
    }
}
