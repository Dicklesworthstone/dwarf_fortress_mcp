#![forbid(unsafe_code)]

use std::env;
use std::error::Error;
use std::net::SocketAddr;
use std::process::ExitCode;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use dfmcp_adapter::{
    BridgeCredentials, GameAdapter, HealthStatus, LiveConnectionConfig, MAX_CAPSULE_CITIZENS,
    MAX_CITIZENS_PER_PAGE, connect_authenticated_live_source, derive_live_fortress_id,
    parse_loopback_endpoint, project_live_capsule, read_complete_observation_bounded,
};
use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, Digest32, FortressId, GameTick, IntentId,
    ObservationCursor, OperationContext, RequestId, RiskTier, SessionId, WorkBudget,
};
use dfmcp_intent::{Action, Constraint, Intent, RequestedAction, StaticPlanner};
use dfmcp_lab::MemoryAdapter;
use dfmcp_world::{Predicate, WorldGraph, WorldSnapshot};
use serde_json::json;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("dwarf-fortress-mcp: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let command = env::args().nth(1).map_or_else(|| "help".to_owned(), |value| value);
    match command.as_str() {
        "help" | "--help" | "-h" => print_help(),
        "version" | "--version" | "-V" => print_version(),
        "contract" => print_contract(),
        "doctor" => doctor()?,
        "demo" => demo()?,
        "bridge" => bridge(env::args().nth(2))?,
        "serve" => dfmcp_mcp::run_stdio(),
        "serve-live" => dfmcp_mcp::run_live_stdio(),
        other => {
            return Err(format!("unknown command {other:?}; run with --help").into());
        }
    }
    Ok(())
}

fn print_help() {
    println!(
        "\
dwarf-fortress-mcp {version}
Agent-native semantic control plane for Dwarf Fortress

USAGE:
    dwarf-fortress-mcp <COMMAND>

COMMANDS:
    contract    Print the frozen narrow-waist contract
    doctor      Exercise the deterministic laboratory adapter
    demo        Prepare and commit a verified laboratory pause-state action
    bridge      Authenticate to dfmcp_bridge and publish one canonical live read
    serve       Run the deterministic laboratory MCP server
    serve-live  Run the authenticated read-only live MCP server
    version     Print version information
    help        Print this help

LIVE BRIDGE ENVIRONMENT:
    DFMCP_BRIDGE_ENDPOINT       Numeric loopback IP:port (default 127.0.0.1:5000)
    DFMCP_BRIDGE_TOKEN          Required 32..256-byte shared loopback secret
    DFMCP_BRIDGE_CONNECT_MILLIS Connect deadline, 1..60000 (default 2000)
    DFMCP_BRIDGE_READ_MILLIS    Read deadline, 1..60000 (default 5000)
    DFMCP_BRIDGE_WRITE_MILLIS   Write deadline, 1..60000 (default 5000)
    DFMCP_BRIDGE_PAGE_SIZE      Citizen page size, 1..4096 (default 4096)
    DFMCP_BRIDGE_MAX_CITIZENS   Complete-roster ceiling, 0..100000 (default 100000)
    DFMCP_BRIDGE_INCLUDE_NAMES  true/false for serve-live (default true)

    The maximum page size is the safe default because one DFHack RPC is an
    internally suspended read. Multipage V1 reads are accepted only while the
    fortress remains paused on every page.

STATUS:
    `serve` is the deterministic semantic laboratory. `serve-live` is an
    authenticated read-only server over bridge protocol V1. Both preserve the
    exact same eleven-tool waist; live mutation-stage tools fail closed because
    the bridge registers no mutation methods.
",
        version = env!("CARGO_PKG_VERSION")
    );
}

fn print_version() {
    println!("dwarf-fortress-mcp {}", env!("CARGO_PKG_VERSION"));
}

fn print_contract() {
    println!(
        "\
protocol: dfmcp/0
transport: mcp/2026-07-28 (modern-only) over stdio via the owned fastmcp_rust sibling
server_modes:
  laboratory: serve
  authenticated_live_read_only: serve-live
bridge_read_protocol: dfmcp.bridge.v1 over DFHack native protobuf RPC
bridge_read_methods:
  - Handshake
  - ReadObservation
bridge_mutation_methods: []
tools:
  - fortress.open_session
  - fortress.observe
  - fortress.query
  - fortress.plan
  - fortress.commit
  - fortress.wait
  - fortress.cancel
  - fortress.checkpoint
  - fortress.restore
  - fortress.explain
  - fortress.doctor
mutation_protocol:
  prepare -> revalidate -> commit -> observe -> prove
truth:
  canonical cursor-anchored semantic world state
safety:
  capability scopes, risk tiers, budgets, idempotency, bounded obligations
"
    );
}

fn doctor() -> Result<(), Box<dyn Error>> {
    let snapshot = sample_snapshot();
    let mut adapter = MemoryAdapter::new(snapshot);
    let context = context(adapter.snapshot(), 1);
    let health = adapter.health(&context)?;
    let status = match health.status {
        HealthStatus::Healthy => "healthy",
        HealthStatus::Degraded => "degraded",
        HealthStatus::ReadOnly => "read_only",
        HealthStatus::Unavailable => "unavailable",
    };
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "status": status,
            "adapter": health.identity.name,
            "compatibility": format!("{:?}", health.identity.compatibility),
            "fortress_loaded": health.fortress_loaded,
            "anchor": health.current_anchor.map(|anchor| anchor.state_hash.to_string()),
        }))?
    );
    Ok(())
}

fn bridge(endpoint: Option<String>) -> Result<(), Box<dyn Error>> {
    let target = match endpoint {
        Some(value) => value,
        None => match env::var("DFMCP_BRIDGE_ENDPOINT") {
            Ok(value) => value,
            Err(env::VarError::NotPresent) => "127.0.0.1:5000".to_owned(),
            Err(env::VarError::NotUnicode(_)) => {
                return Err("DFMCP_BRIDGE_ENDPOINT must be valid UTF-8".into());
            }
        },
    };
    let address = parse_loopback_endpoint(&target)?;
    let token = env::var("DFMCP_BRIDGE_TOKEN").map_err(|_| {
        "DFMCP_BRIDGE_TOKEN is required and must match the secret inherited by Dwarf Fortress/DFHack"
    })?;
    let page_size = bounded_env_u32(
        "DFMCP_BRIDGE_PAGE_SIZE",
        MAX_CITIZENS_PER_PAGE,
        1,
        MAX_CITIZENS_PER_PAGE,
    )?;
    let hard_citizen_limit = u32::try_from(MAX_CAPSULE_CITIZENS)
        .map_err(|_| "MAX_CAPSULE_CITIZENS does not fit u32")?;
    let max_citizens = bounded_env_u32(
        "DFMCP_BRIDGE_MAX_CITIZENS",
        hard_citizen_limit,
        0,
        hard_citizen_limit,
    )?;
    let connect_millis = bounded_env_u64(
        "DFMCP_BRIDGE_CONNECT_MILLIS",
        2_000,
        1,
        60_000,
    )?;
    let read_millis = bounded_env_u64("DFMCP_BRIDGE_READ_MILLIS", 5_000, 1, 60_000)?;
    let write_millis = bounded_env_u64("DFMCP_BRIDGE_WRITE_MILLIS", 5_000, 1, 60_000)?;
    let nonce = bridge_nonce(address)?;
    let credentials = BridgeCredentials::new(token.into_bytes(), nonce)?;
    let mut source = connect_authenticated_live_source(
        &LiveConnectionConfig {
            endpoint: address,
            connect_timeout: Duration::from_millis(connect_millis),
            read_timeout: Duration::from_millis(read_millis),
            write_timeout: Duration::from_millis(write_millis),
            client_name: "dwarf-fortress-mcp-cli".to_owned(),
            client_version: env!("CARGO_PKG_VERSION").to_owned(),
        },
        credentials,
    )?;
    let capsule = read_complete_observation_bounded(
        &mut source,
        page_size,
        true,
        max_citizens,
    )?;
    let fortress_id = derive_live_fortress_id(&capsule)?;
    let projection = project_live_capsule(
        &capsule,
        fortress_id,
        ObservationCursor::ORIGIN,
    )?;
    projection.validate_against(&capsule)?;
    let complete_domains = projection
        .receipt
        .coverage()
        .domains
        .values()
        .filter(|domain| domain.status.as_str() == "complete")
        .map(|domain| domain.domain.clone())
        .collect::<Vec<_>>();
    let omitted_domains = projection
        .receipt
        .coverage()
        .domains
        .values()
        .filter(|domain| domain.status.as_str() == "omitted")
        .map(|domain| {
            json!({
                "domain": domain.domain,
                "reason": domain.reason,
            })
        })
        .collect::<Vec<_>>();
    let result = json!({
        "status": "authenticated_read_only_live_observation",
        "endpoint": target,
        "bridge": {
            "protocol": format!(
                "{}.{}",
                dfmcp_adapter::BRIDGE_PROTOCOL_MAJOR,
                dfmcp_adapter::BRIDGE_PROTOCOL_MINOR
            ),
            "version": capsule.bridge.bridge_version,
            "generation": capsule.bridge.bridge_generation,
            "dfhack_version": capsule.bridge.dfhack_version,
            "dwarf_fortress_version": capsule.bridge.df_version,
            "supported_methods": capsule.bridge.supported_methods,
            "mutation_methods": [],
        },
        "fortress": {
            "fortress_id": fortress_id.to_string(),
            "site_id": capsule.site_id,
            "world_name": capsule.world_name,
            "world_folder": capsule.world_folder,
            "paused": capsule.paused,
            "calendar_year": capsule.current_year,
            "year_tick": capsule.current_year_tick,
            "absolute_game_tick": projection.snapshot.tick.get(),
            "citizens": capsule.citizen_coverage.total,
        },
        "capsule": {
            "schema": "dfmcp.live-observation-capsule.v2",
            "digest": capsule.content_digest.to_string(),
            "canonical_bytes": capsule.canonical_bytes.len(),
            "complete": capsule.citizen_coverage.proves_complete_roster(),
            "names_included": capsule.names_included,
        },
        "snapshot": {
            "projection_schema": projection.receipt.schema(),
            "anchor": {
                "epoch": projection.snapshot.cursor.epoch,
                "sequence": projection.snapshot.cursor.sequence,
                "game_tick": projection.snapshot.tick.get(),
                "state_hash": projection.snapshot.state_hash.to_string(),
            },
            "entities": projection.snapshot.graph.entities.len(),
            "edges": projection.snapshot.graph.edges.len(),
        },
        "coverage": {
            "complete_domains": complete_domains,
            "omitted_domains": omitted_domains,
        },
    });
    let client = source.into_inner();
    let _stream = client.close()?;
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}

fn bridge_nonce(address: SocketAddr) -> Result<Vec<u8>, Box<dyn Error>> {
    let elapsed = SystemTime::now().duration_since(UNIX_EPOCH)?;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"dfmcp-bridge-cli-nonce-v1\0");
    bytes.extend_from_slice(&elapsed.as_nanos().to_be_bytes());
    bytes.extend_from_slice(&std::process::id().to_be_bytes());
    bytes.extend_from_slice(address.to_string().as_bytes());
    bytes.extend_from_slice(env!("CARGO_PKG_VERSION").as_bytes());
    Ok(Digest32::of_bytes(&bytes).as_bytes().to_vec())
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
    Ok(u32::try_from(value).map_err(|_| format!("{name} does not fit u32"))?)
}

fn demo() -> Result<(), Box<dyn Error>> {
    let snapshot = sample_snapshot();
    let intent = Intent {
        id: IntentId::new(1),
        anchor: snapshot.anchor(),
        summary: "unpause the deterministic laboratory fortress".to_owned(),
        terminal_condition: Predicate::Paused(false),
        constraints: vec![Constraint::MaxRisk(RiskTier::Reversible)],
        requested_actions: vec![RequestedAction {
            action: Action::Pause { paused: false },
            preconditions: vec![Predicate::Paused(true)],
            postconditions: Vec::new(),
            compensation: None,
            obligation: None,
            depends_on: Vec::new(),
        }],
    };
    let plan_context = context(&snapshot, 1);
    let plan = StaticPlanner::default().prepare(&snapshot, &intent, &plan_context)?;
    let mut adapter = MemoryAdapter::new(snapshot);
    let prepare_context = context(adapter.snapshot(), 2);
    let prepared = adapter.prepare(&plan, &prepare_context)?;
    let commit_context = context(adapter.snapshot(), 3);
    let committed = adapter.commit(&plan, &prepared, &commit_context)?;
    let action = committed
        .actions
        .first()
        .ok_or_else(|| "commit returned no action receipts".to_owned())?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "plan_id": plan.id.to_string(),
            "plan_digest": plan.digest.to_string(),
            "action_id": action.action_id.to_string(),
            "state": format!("{:?}", action.state),
            "paused": adapter.snapshot().paused,
            "cursor": {
                "epoch": adapter.snapshot().cursor.epoch,
                "sequence": adapter.snapshot().cursor.sequence,
            },
            "state_hash": adapter.snapshot().state_hash.to_string(),
        }))?
    );
    Ok(())
}

fn sample_snapshot() -> WorldSnapshot {
    WorldSnapshot::new(
        FortressId::new(1),
        GameTick(1),
        ObservationCursor::ORIGIN,
        true,
        WorldGraph::default(),
    )
}

fn context(snapshot: &WorldSnapshot, request_id: u128) -> OperationContext {
    let capabilities = [
        (Capability::Observe, RiskTier::ReadOnly),
        (Capability::Query, RiskTier::ReadOnly),
        (Capability::Plan, RiskTier::ReadOnly),
        (Capability::ControlClock, RiskTier::Reversible),
        (Capability::Checkpoint, RiskTier::Guarded),
        (Capability::Restore, RiskTier::Guarded),
        (Capability::Doctor, RiskTier::ReadOnly),
    ];
    OperationContext {
        session_id: SessionId::new(1),
        request_id: RequestId::new(request_id),
        anchor: snapshot.anchor(),
        budget: WorkBudget::default(),
        grants: capabilities
            .into_iter()
            .map(|(capability, max_risk)| CapabilityGrant {
                capability,
                scope: CapabilityScope {
                    fortress_id: Some(snapshot.fortress_id),
                    ..CapabilityScope::default()
                },
                max_risk,
                expires_at_tick: None,
                remaining_uses: None,
            })
            .collect(),
        cancellation_requested: false,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::error::Error;

    use super::{bounded_env_u32, doctor};
    use dfmcp_adapter::{
        BridgeManifest, ObservationAssembler, ObservationPage, derive_live_fortress_id,
        parse_loopback_endpoint,
    };
    use dfmcp_core::FortressId;

    #[test]
    fn doctor_command_has_the_authority_it_exercises() {
        assert!(doctor().is_ok());
    }

    #[test]
    fn bridge_accepts_only_numeric_loopback_targets() {
        assert!(parse_loopback_endpoint("127.0.0.1:5000").is_ok());
        assert!(parse_loopback_endpoint("[::1]:5000").is_ok());
        assert!(parse_loopback_endpoint("localhost:5000").is_err());
        assert!(parse_loopback_endpoint("192.0.2.1:5000").is_err());
        assert!(parse_loopback_endpoint("").is_err());
    }

    #[test]
    fn cli_uses_the_canonical_nonzero_fortress_identity() -> Result<(), Box<dyn Error>> {
        let manifest = BridgeManifest {
            bridge_version: "0.1.0".to_owned(),
            dfhack_version: "0.51.11-r1".to_owned(),
            df_version: "0.51.11".to_owned(),
            world_loaded: true,
            fortress_mode: true,
            bridge_generation: 1,
            supported_methods: BTreeSet::from([
                "Handshake".to_owned(),
                "ReadObservation".to_owned(),
            ]),
        };
        let page = ObservationPage {
            bridge_generation: 1,
            world_loaded: true,
            fortress_mode: true,
            paused: true,
            current_year: 105,
            current_year_tick: 1,
            world_name: "Realm".to_owned(),
            world_folder: "region1".to_owned(),
            site_id: 7,
            citizen_count_total: 0,
            citizen_offset: 0,
            complete: true,
            citizens: Vec::new(),
        };
        let mut assembler = ObservationAssembler::new(manifest);
        assembler.push_page(page)?;
        let capsule = assembler.finalize()?;
        let first = derive_live_fortress_id(&capsule)?;
        let second = derive_live_fortress_id(&capsule)?;
        assert_eq!(first, second);
        assert_ne!(first, FortressId::NIL);
        Ok(())
    }

    #[test]
    fn bounded_environment_defaults_when_absent() {
        let name = "DFMCP_TEST_ABSENT_U32_918273645";
        assert_eq!(bounded_env_u32(name, 7, 1, 10).ok(), Some(7));
    }
}
