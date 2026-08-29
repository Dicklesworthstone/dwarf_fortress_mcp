#![forbid(unsafe_code)]

use std::env;
use std::error::Error;
use std::process::ExitCode;

use dfmcp_adapter::{GameAdapter, HealthStatus};
use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, FortressId, GameTick, IntentId,
    ObservationCursor, OperationContext, RequestId, RiskTier, SessionId, WorkBudget,
};
use dfmcp_intent::{Action, Constraint, Intent, RequestedAction, StaticPlanner};
use dfmcp_lab::MemoryAdapter;
use dfmcp_world::{Predicate, WorldGraph, WorldSnapshot};

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
    let command = env::args().nth(1).unwrap_or_else(|| "help".to_owned());
    match command.as_str() {
        "help" | "--help" | "-h" => print_help(),
        "version" | "--version" | "-V" => print_version(),
        "contract" => print_contract(),
        "doctor" => doctor()?,
        "demo" => demo()?,
        "serve" => dfmcp_mcp::run_stdio(),
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
Design-first semantic control plane for Dwarf Fortress agents

USAGE:
    dwarf-fortress-mcp <COMMAND>

COMMANDS:
    contract    Print the frozen phase-zero narrow-waist contract
    doctor      Exercise the deterministic laboratory adapter
    demo        Prepare and commit a verified semantic pause-state action
    serve       Run the MCP 2026-07-28 modern-only stdio server (fastmcp_rust)
    version     Print version information
    help        Print this help

STATUS:
    Executable contract scaffold plus a laboratory MCP transport. The stdio
    server runs the owned fastmcp_rust sibling pinned to MCP 2026-07-28,
    modern-only. Live DFHack integration is not claimed.
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
        "{{\n  \"status\": \"{status}\",\n  \"adapter\": \"{}\",\n  \"compatibility\": \"{:?}\",\n  \"fortress_loaded\": {},\n  \"anchor\": \"{}\"\n}}",
        health.identity.name,
        health.identity.compatibility,
        health.fortress_loaded,
        health
            .current_anchor
            .map(|anchor| anchor.state_hash.to_string())
            .unwrap_or_else(|| "none".to_owned())
    );
    Ok(())
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
        "{{\n  \"plan_id\": \"{}\",\n  \"plan_digest\": \"{}\",\n  \"action_id\": \"{}\",\n  \"state\": \"{:?}\",\n  \"paused\": {},\n  \"cursor\": {{\"epoch\": {}, \"sequence\": {}}},\n  \"state_hash\": \"{}\"\n}}",
        plan.id,
        plan.digest,
        action.action_id,
        action.state,
        adapter.snapshot().paused,
        adapter.snapshot().cursor.epoch,
        adapter.snapshot().cursor.sequence,
        adapter.snapshot().state_hash
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
