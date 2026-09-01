#![forbid(unsafe_code)]

use std::env;
use std::error::Error;
use std::net::{SocketAddr, TcpStream};
use std::process::ExitCode;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use dfmcp_adapter::{
    BRIDGE_PROTOCOL_MAJOR, BRIDGE_PROTOCOL_MINOR, BridgeCredentials, DfHackProbeClient,
    LiveConnectionConfig, LiveObservationSource, MAX_CAPSULE_CITIZENS,
    MAX_CITIZENS_PER_PAGE, ObservationAssembler, ProbeHandshakeRequest,
    ProbeObservationRequest, connect_authenticated_live_source, derive_live_fortress_id,
    parse_loopback_endpoint, project_live_capsule,
};
use dfmcp_core::{Digest32, ErrorCode, ObservationCursor};
use serde_json::{Value as JsonValue, json};

const DEFAULT_ENDPOINT: &str = "127.0.0.1:5000";
const DEFAULT_CONNECT_MILLIS: u64 = 2_000;
const DEFAULT_READ_MILLIS: u64 = 5_000;
const DEFAULT_WRITE_MILLIS: u64 = 5_000;
const MAX_TIMEOUT_MILLIS: u64 = 60_000;
const VALID_NONCE_BYTES: usize = 32;
const SHORT_NONCE_BYTES: usize = 15;
const LONG_NONCE_BYTES: usize = 65;
const SHORT_TOKEN_BYTES: usize = 31;
const LONG_TOKEN_BYTES: usize = 257;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(failure) => {
            eprintln!("dfmcp-live-probe: {failure}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let mut arguments = env::args().skip(1);
    let command = arguments.next().unwrap_or_else(|| "help".to_owned());
    match command.as_str() {
        "help" | "--help" | "-h" => print_help(),
        "handshake-case" => {
            let case = arguments
                .next()
                .ok_or("handshake-case requires one normative R2 case name")?;
            reject_extra_arguments(arguments)?;
            run_handshake_case(&case)?;
        }
        "observation-case" => {
            let case = arguments
                .next()
                .ok_or("observation-case requires one normative R3/R4 case name")?;
            reject_extra_arguments(arguments)?;
            run_observation_case(&case)?;
        }
        "capsule" => {
            let page_size = parse_u32_argument(arguments.next(), "page_size")?;
            let include_names = parse_bool_argument(arguments.next(), "include_names")?;
            reject_extra_arguments(arguments)?;
            run_capsule(page_size, include_names)?;
        }
        "agent-turn" => {
            reject_extra_arguments(arguments)?;
            run_agent_turn()?;
        }
        other => {
            return Err(format!("unknown command {other:?}; run with --help").into());
        }
    }
    Ok(())
}

fn reject_extra_arguments(mut arguments: impl Iterator<Item = String>) -> Result<(), Box<dyn Error>> {
    if let Some(extra) = arguments.next() {
        return Err(format!("unexpected extra argument {extra:?}").into());
    }
    Ok(())
}

fn print_help() {
    println!(
        "\
dfmcp-live-probe {version}
Bounded, read-only disposable-fort acceptance probe

USAGE:
    dfmcp-live-probe handshake-case <R2_CASE>
    dfmcp-live-probe observation-case <R3_OR_R4_CASE>
    dfmcp-live-probe capsule <PAGE_SIZE> <INCLUDE_NAMES>
    dfmcp-live-probe agent-turn

R2 CASES:
    missing_token configured_token_short configured_token_long
    presented_token_short presented_token_long wrong_token correct_token
    nonce_short nonce_long nonce_mismatch protocol_mismatch

OBSERVATION CASES:
    offset_at_total offset_beyond_total oversize_request
    running_multipage_rejected world_unloaded non_fortress_mode

ENVIRONMENT:
    DFMCP_BRIDGE_ENDPOINT       Numeric loopback IP:port (default 127.0.0.1:5000)
    DFMCP_BRIDGE_TOKEN          Secret inherited by the disposable DF/DFHack process
    DFMCP_BRIDGE_CONNECT_MILLIS 1..60000 (default 2000)
    DFMCP_BRIDGE_READ_MILLIS    1..60000 (default 5000)
    DFMCP_BRIDGE_WRITE_MILLIS   1..60000 (default 5000)
    DFMCP_BRIDGE_MAX_CITIZENS   0..100000 for capsule mode

The probe never prints the bearer token. Typed bridge rejection is emitted as
successful JSON output so the evidence journal can distinguish a proved
rejection from transport or decoder failure. Configured-token cases require the
operator to restart the disposable DFHack process with the named invalid server
configuration. World-mode and restart cases likewise require explicit operator
state changes; this binary never forwards DFHack commands or Lua.
",
        version = env!("CARGO_PKG_VERSION")
    );
}

fn parse_u32_argument(value: Option<String>, name: &str) -> Result<u32, Box<dyn Error>> {
    let raw = value.ok_or_else(|| format!("missing {name}"))?;
    raw.parse::<u32>()
        .map_err(|_| format!("{name} must be a decimal u32").into())
}

fn parse_bool_argument(value: Option<String>, name: &str) -> Result<bool, Box<dyn Error>> {
    match value.as_deref() {
        Some("true" | "1") => Ok(true),
        Some("false" | "0") => Ok(false),
        Some(_) => Err(format!("{name} must be true/false or 1/0").into()),
        None => Err(format!("missing {name}").into()),
    }
}

fn endpoint_text() -> Result<String, Box<dyn Error>> {
    match env::var("DFMCP_BRIDGE_ENDPOINT") {
        Ok(value) => Ok(value),
        Err(env::VarError::NotPresent) => Ok(DEFAULT_ENDPOINT.to_owned()),
        Err(env::VarError::NotUnicode(_)) => {
            Err("DFMCP_BRIDGE_ENDPOINT must be valid UTF-8".into())
        }
    }
}

fn configured_token() -> Result<Vec<u8>, Box<dyn Error>> {
    match env::var("DFMCP_BRIDGE_TOKEN") {
        Ok(value) => Ok(value.into_bytes()),
        Err(env::VarError::NotPresent) => Err("DFMCP_BRIDGE_TOKEN is required for this case".into()),
        Err(env::VarError::NotUnicode(_)) => {
            Err("DFMCP_BRIDGE_TOKEN must be valid UTF-8".into())
        }
    }
}

fn bounded_env_u64(
    name: &str,
    default: u64,
    minimum: u64,
    maximum: u64,
) -> Result<u64, Box<dyn Error>> {
    let value = match env::var(name) {
        Ok(raw) => raw
            .parse::<u64>()
            .map_err(|_| format!("{name} must be a decimal u64"))?,
        Err(env::VarError::NotPresent) => default,
        Err(env::VarError::NotUnicode(_)) => {
            return Err(format!("{name} must be valid UTF-8").into());
        }
    };
    if value < minimum || value > maximum {
        return Err(format!("{name} must be in {minimum}..={maximum}, got {value}").into());
    }
    Ok(value)
}

fn bounded_env_u32(
    name: &str,
    default: u32,
    minimum: u32,
    maximum: u32,
) -> Result<u32, Box<dyn Error>> {
    let value = bounded_env_u64(
        name,
        u64::from(default),
        u64::from(minimum),
        u64::from(maximum),
    )?;
    u32::try_from(value).map_err(|_| format!("{name} does not fit u32").into())
}

fn timeouts() -> Result<(Duration, Duration, Duration), Box<dyn Error>> {
    Ok((
        Duration::from_millis(bounded_env_u64(
            "DFMCP_BRIDGE_CONNECT_MILLIS",
            DEFAULT_CONNECT_MILLIS,
            1,
            MAX_TIMEOUT_MILLIS,
        )?),
        Duration::from_millis(bounded_env_u64(
            "DFMCP_BRIDGE_READ_MILLIS",
            DEFAULT_READ_MILLIS,
            1,
            MAX_TIMEOUT_MILLIS,
        )?),
        Duration::from_millis(bounded_env_u64(
            "DFMCP_BRIDGE_WRITE_MILLIS",
            DEFAULT_WRITE_MILLIS,
            1,
            MAX_TIMEOUT_MILLIS,
        )?),
    ))
}

fn open_probe_client() -> Result<(String, DfHackProbeClient<TcpStream>), Box<dyn Error>> {
    let endpoint = endpoint_text()?;
    let address = parse_loopback_endpoint(&endpoint)?;
    let (connect_timeout, read_timeout, write_timeout) = timeouts()?;
    let stream = TcpStream::connect_timeout(&address, connect_timeout)?;
    stream.set_read_timeout(Some(read_timeout))?;
    stream.set_write_timeout(Some(write_timeout))?;
    stream.set_nodelay(true)?;
    Ok((endpoint, DfHackProbeClient::negotiate_transport(stream)?))
}

fn fresh_nonce(address: &str, domain: &[u8]) -> Result<Vec<u8>, Box<dyn Error>> {
    let elapsed = SystemTime::now().duration_since(UNIX_EPOCH)?;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(domain);
    bytes.extend_from_slice(&elapsed.as_nanos().to_be_bytes());
    bytes.extend_from_slice(&std::process::id().to_be_bytes());
    bytes.extend_from_slice(address.as_bytes());
    bytes.extend_from_slice(env!("CARGO_PKG_VERSION").as_bytes());
    Ok(Digest32::of_bytes(&bytes).as_bytes().to_vec())
}

fn case_token(case: &str) -> Result<Vec<u8>, Box<dyn Error>> {
    match case {
        "missing_token" => Ok(Vec::new()),
        "presented_token_short" => Ok(vec![b's'; SHORT_TOKEN_BYTES]),
        "presented_token_long" => Ok(vec![b'l'; LONG_TOKEN_BYTES]),
        "wrong_token" => Ok(vec![b'w'; 32]),
        "configured_token_short" | "configured_token_long" | "correct_token"
        | "nonce_short" | "nonce_long" | "nonce_mismatch" | "protocol_mismatch" => {
            configured_token()
        }
        _ => Err(format!("unknown R2 handshake case {case:?}").into()),
    }
}

fn case_nonce(case: &str, endpoint: &str) -> Result<Vec<u8>, Box<dyn Error>> {
    match case {
        "nonce_short" => Ok(vec![b'n'; SHORT_NONCE_BYTES]),
        "nonce_long" => Ok(vec![b'n'; LONG_NONCE_BYTES]),
        _ => fresh_nonce(endpoint, b"dfmcp-live-probe-handshake-nonce-v1\0"),
    }
}

fn run_handshake_case(case: &str) -> Result<(), Box<dyn Error>> {
    let (endpoint, mut client) = open_probe_client()?;
    let nonce = case_nonce(case, &endpoint)?;
    let protocol_major = if case == "protocol_mismatch" {
        BRIDGE_PROTOCOL_MAJOR.saturating_add(1)
    } else {
        BRIDGE_PROTOCOL_MAJOR
    };
    let request = ProbeHandshakeRequest {
        protocol_major,
        protocol_minor: BRIDGE_PROTOCOL_MINOR,
        client_name: "dfmcp-live-probe".to_owned(),
        client_version: env!("CARGO_PKG_VERSION").to_owned(),
        client_nonce: nonce.clone(),
        bearer_token: case_token(case)?,
    };
    let reply = client.handshake(&request)?;
    let expected_nonce = if case == "nonce_mismatch" {
        let mut mismatched = nonce.clone();
        if let Some(first) = mismatched.first_mut() {
            *first ^= 0xff;
        }
        mismatched
    } else {
        nonce.clone()
    };
    let nonce_correlated = reply.nonce_correlated(&expected_nonce);
    let effective_accepted = reply.accepted && nonce_correlated;
    let error_code = if case == "nonce_mismatch" && !nonce_correlated {
        Some("CLIENT_NONCE_MISMATCH".to_owned())
    } else if reply.accepted {
        None
    } else {
        Some(reply.failure_code.clone())
    };
    let result = json!({
        "schema": "dfmcp.live-read-probe/1",
        "kind": "handshake",
        "case": case,
        "endpoint_class": "numeric_loopback",
        "accepted": effective_accepted,
        "server_accepted": reply.accepted,
        "error_code": error_code,
        "failure_message": reply.failure_message,
        "protocol_major": reply.protocol_major,
        "protocol_minor": reply.protocol_minor,
        "bridge_version": reply.bridge_version,
        "dfhack_version": reply.dfhack_version,
        "dwarf_fortress_version": reply.df_version,
        "world_loaded": reply.world_loaded,
        "fortress_mode": reply.fortress_mode,
        "nonce_correlated": nonce_correlated,
        "bridge_generation": reply.bridge_generation,
        "supported_methods": reply.supported_methods,
        "sensitive_manifest_disclosed": reply.sensitive_manifest_disclosed(),
    });
    let _stream = client.close()?;
    println!("{}", serde_json::to_string(&result)?);
    Ok(())
}

fn valid_probe_handshake(
    client: &mut DfHackProbeClient<TcpStream>,
    endpoint: &str,
) -> Result<(Vec<u8>, u64), Box<dyn Error>> {
    let nonce = fresh_nonce(endpoint, b"dfmcp-live-probe-observation-nonce-v1\0")?;
    let request = ProbeHandshakeRequest {
        protocol_major: BRIDGE_PROTOCOL_MAJOR,
        protocol_minor: BRIDGE_PROTOCOL_MINOR,
        client_name: "dfmcp-live-probe".to_owned(),
        client_version: env!("CARGO_PKG_VERSION").to_owned(),
        client_nonce: nonce.clone(),
        bearer_token: configured_token()?,
    };
    let reply = client.handshake(&request)?;
    if !reply.accepted || !reply.nonce_correlated(&nonce) {
        return Err(format!(
            "observation case requires a valid handshake; bridge returned {:?}",
            reply.failure_code
        )
        .into());
    }
    Ok((nonce, reply.bridge_generation))
}

fn raw_observation(
    client: &mut DfHackProbeClient<TcpStream>,
    nonce: &[u8],
    offset: u32,
    maximum: u32,
    include_names: bool,
) -> Result<dfmcp_adapter::ProbeObservationReply, Box<dyn Error>> {
    Ok(client.read_observation(&ProbeObservationRequest {
        protocol_major: BRIDGE_PROTOCOL_MAJOR,
        protocol_minor: BRIDGE_PROTOCOL_MINOR,
        client_nonce: nonce.to_vec(),
        bearer_token: configured_token()?,
        citizen_offset: offset,
        max_citizens: maximum,
        include_names,
    })?)
}

fn run_observation_case(case: &str) -> Result<(), Box<dyn Error>> {
    let (endpoint, mut client) = open_probe_client()?;
    let (nonce, negotiated_generation) = valid_probe_handshake(&mut client, &endpoint)?;
    let (requested_offset, maximum, needs_total) = match case {
        "offset_at_total" | "offset_beyond_total" => (0, MAX_CITIZENS_PER_PAGE, true),
        "oversize_request" => (0, MAX_CITIZENS_PER_PAGE.saturating_add(1), false),
        "running_multipage_rejected" => (0, 1, false),
        "world_unloaded" | "non_fortress_mode" => (0, 1, false),
        _ => return Err(format!("unknown observation case {case:?}").into()),
    };
    let (request_offset, reply) = if needs_total {
        let baseline = raw_observation(
            &mut client,
            &nonce,
            0,
            MAX_CITIZENS_PER_PAGE,
            true,
        )?;
        if !baseline.accepted {
            return Err(format!(
                "offset case baseline was rejected with {:?}",
                baseline.failure_code
            )
            .into());
        }
        let offset = if case == "offset_beyond_total" {
            baseline.citizen_count_total.saturating_add(1)
        } else {
            baseline.citizen_count_total
        };
        (offset, raw_observation(&mut client, &nonce, offset, 1, true)?)
    } else {
        (
            requested_offset,
            raw_observation(&mut client, &nonce, requested_offset, maximum, true)?,
        )
    };
    if reply.bridge_generation != 0 && reply.bridge_generation != negotiated_generation {
        return Err("observation reply changed bridge generation after handshake".into());
    }
    let running_rejected = case == "running_multipage_rejected"
        && reply.accepted
        && !reply.paused
        && !reply.complete;
    if case == "running_multipage_rejected" && !running_rejected {
        return Err(
            "running_multipage_rejected requires a running fortress and a nonterminal first page"
                .into(),
        );
    }
    let effective_error = if running_rejected {
        Some(ErrorCode::PreconditionsFailed.as_str().to_owned())
    } else if reply.accepted {
        None
    } else {
        Some(reply.failure_code.clone())
    };
    let result = json!({
        "schema": "dfmcp.live-read-probe/1",
        "kind": "observation",
        "case": case,
        "accepted": reply.accepted && !running_rejected,
        "server_accepted": reply.accepted,
        "error_code": effective_error,
        "failure_message": reply.failure_message,
        "nonce_correlated": reply.nonce_correlated(&nonce),
        "bridge_generation": reply.bridge_generation,
        "world_loaded": reply.world_loaded,
        "fortress_mode": reply.fortress_mode,
        "paused": reply.paused,
        "current_year": reply.current_year,
        "current_year_tick": reply.current_year_tick,
        "site_id": reply.site_id,
        "citizen_count": reply.citizen_count_total,
        "requested_offset": request_offset,
        "canonical_offset": reply.citizen_offset,
        "requested_maximum": maximum,
        "returned_citizens": reply.citizens.len(),
        "complete": reply.complete,
        "pages_attempted": 1,
        "published": false,
        "world_posture_disclosed": reply.world_posture_disclosed(),
    });
    let _stream = client.close()?;
    println!("{}", serde_json::to_string(&result)?);
    Ok(())
}

fn live_connection_config(address: SocketAddr) -> Result<LiveConnectionConfig, Box<dyn Error>> {
    let (connect_timeout, read_timeout, write_timeout) = timeouts()?;
    Ok(LiveConnectionConfig {
        endpoint: address,
        connect_timeout,
        read_timeout,
        write_timeout,
        client_name: "dfmcp-live-probe-capsule".to_owned(),
        client_version: env!("CARGO_PKG_VERSION").to_owned(),
    })
}

fn run_capsule(page_size: u32, include_names: bool) -> Result<(), Box<dyn Error>> {
    if page_size == 0 || page_size > MAX_CITIZENS_PER_PAGE {
        return Err(format!("page_size must be in 1..={MAX_CITIZENS_PER_PAGE}").into());
    }
    let endpoint = endpoint_text()?;
    let address = parse_loopback_endpoint(&endpoint)?;
    let nonce = fresh_nonce(&endpoint, b"dfmcp-live-probe-capsule-nonce-v1\0")?;
    let credentials = BridgeCredentials::new(configured_token()?, nonce)?;
    let mut source = connect_authenticated_live_source(
        &live_connection_config(address)?,
        credentials,
    )?;
    let hard_limit = u32::try_from(MAX_CAPSULE_CITIZENS)
        .map_err(|_| "MAX_CAPSULE_CITIZENS does not fit u32")?;
    let maximum = bounded_env_u32(
        "DFMCP_BRIDGE_MAX_CITIZENS",
        hard_limit,
        0,
        hard_limit,
    )?;
    let mut assembler = ObservationAssembler::with_names(source.bridge_manifest(), include_names);
    let mut page_count = 0u32;
    loop {
        let offset = assembler.next_offset()?;
        let page = source.read_observation_page(offset, page_size, include_names)?;
        if page.citizen_count_total > maximum {
            return Err(format!(
                "fortress citizen count {} exceeds configured complete-roster ceiling {maximum}",
                page.citizen_count_total
            )
            .into());
        }
        if !page.paused && !page.complete {
            return Err(
                "cannot assemble a coherent multipage observation while Dwarf Fortress is running"
                    .into(),
            );
        }
        let complete = page.complete;
        assembler.push_page(page)?;
        page_count = page_count
            .checked_add(1)
            .ok_or("capsule page counter overflowed")?;
        if complete {
            break;
        }
    }
    let capsule = assembler.finalize()?;
    let fortress_id = derive_live_fortress_id(&capsule)?;
    let projection = project_live_capsule(&capsule, fortress_id, ObservationCursor::ORIGIN)?;
    projection.validate_against(&capsule)?;
    let mut identity_bytes = b"dfmcp.live-probe.citizen-identities/1\0".to_vec();
    for citizen in &capsule.citizens {
        identity_bytes.extend_from_slice(&citizen.unit_id.to_le_bytes());
    }
    let citizen_identity = Digest32::of_bytes(&identity_bytes);
    let anchor = projection.snapshot.anchor();
    let result = json!({
        "schema": "dfmcp.live-read-probe/1",
        "kind": "capsule",
        "paused": capsule.paused,
        "names_included": include_names,
        "page_size": page_size,
        "page_count": page_count,
        "citizen_count": capsule.citizen_coverage.total,
        "complete": capsule.citizen_coverage.proves_complete_roster(),
        "publication_count": 1,
        "bridge_generation": capsule.bridge.bridge_generation,
        "capsule_sha256": capsule.content_digest.to_string(),
        "snapshot_sha256": projection.snapshot.state_hash.to_string(),
        "citizen_identity_sha256": citizen_identity.to_string(),
        "anchor": {
            "fortress_id": anchor.fortress_id.to_string(),
            "epoch": anchor.cursor.epoch,
            "sequence": anchor.cursor.sequence,
            "game_tick": anchor.tick.get(),
            "state_hash": anchor.state_hash.to_string(),
        },
        "source": {
            "dwarf_fortress_version": capsule.bridge.df_version,
            "dfhack_version": capsule.bridge.dfhack_version,
            "bridge_version": capsule.bridge.bridge_version,
            "bridge_protocol": format!("{BRIDGE_PROTOCOL_MAJOR}.{BRIDGE_PROTOCOL_MINOR}"),
        },
    });
    let client = source.into_inner();
    let _stream = client.close()?;
    println!("{}", serde_json::to_string(&result)?);
    Ok(())
}

fn run_agent_turn() -> Result<(), Box<dyn Error>> {
    let encoded = dfmcp_mcp::live_server::fortress_open_session(
        None, None, None, None, None, None, None, None, None,
    );
    let value: JsonValue = serde_json::from_str(&encoded)?;
    if value.get("agent_turn").is_none() {
        return Err("fortress_open_session did not return an Agent Turn Packet".into());
    }
    println!("{}", serde_json::to_string(&value)?);
    Ok(())
}
