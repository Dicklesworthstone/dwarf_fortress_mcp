//! Modern-only (MCP 2026-07-28) stdio server for the fortress narrow waist.
//!
//! This executable contract currently exposes a process-local deterministic
//! laboratory, not a live Dwarf Fortress or durable service. Session-scoped
//! capability negotiation provides per-session state and
//! negotiated grants replace the previous process-local laboratory state.
//! Transport identity grants nothing; every capability comes from the
//! `CapabilityGrant`s negotiated in `fortress_open_session`.
//!
//! Per-session state lives in `SESSIONS`, keyed by the freshly minted
//! `SessionId` returned by `fortress_open_session`. Subsequent tools take
//! `session_id` as an argument and dispatch against that session only.
//! Concurrent stdio clients therefore get independent adapters, anchors,
//! plans, and receipts. No transport type crosses the adapter seam:
//! `dfmcp-core` `CapabilityGrant` is the sole authority.
//!
//! See `docs/FASTMCP_INTEGRATION.md` §6 for the security posture and
//! `design/registries/CAPABILITIES.md` for the negotiated-capability registry.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard, RwLock};

use crate::doctor::DoctorInspector;
use dfmcp_adapter::{
    CancelMode, GameAdapter, InterestSet, ObservationPayload, ObservationRequest, Projection,
    QueryRequest,
};
use dfmcp_core::{
    ActionId, Capability, CapabilityGrant, CapabilityScope, CheckpointId, CommitState, DfmcpError,
    Digest32, EntityId, ErrorCode, FortressId, GameTick, IntentId, MapCuboid, OperationContext,
    RequestId, Result, RiskTier, SessionId, StateAnchor, WorkBudget,
};
use dfmcp_intent::{
    Action, Constraint, EffectWorkState, Intent, ObligationStatus, PreparedPlan,
    RecoveredObligation, RequestedAction, StaticPlanner,
};
use dfmcp_lab::MemoryAdapter;
use dfmcp_world::topology::get_transitive_dependencies;
use dfmcp_world::{
    EdgeKind, Predicate, PredicateEvidence, PredicateTruth, QueryOrder, WorldQuery, WorldSnapshot,
};
use fastmcp_rust::modern::ServerBuilder;
use fastmcp_rust::prelude::*;
use serde_json::json;

#[path = "task_session.rs"]
pub(crate) mod task_session;

#[path = "restore_work.rs"]
mod restore_work;

#[path = "objectives.rs"]
mod objectives;
use objectives::{Objective, objective_history_roots};

#[path = "plan_resources.rs"]
pub(crate) mod plan_resources;

#[path = "lab_goal_explanation.rs"]
pub(crate) mod goal_explanation;

#[path = "lab_plan_cancellation.rs"]
mod plan_cancellation;

#[path = "production_source.rs"]
pub(crate) mod production_source;

#[path = "resource_acquisition.rs"]
mod resource_acquisition;
#[path = "resource_acquisition_continuation.rs"]
mod resource_acquisition_continuation;

#[cfg(test)]
#[path = "production_resource_mcp_tests.rs"]
mod production_resource_mcp_tests;
#[cfg(test)]
#[path = "resource_acquisition_continuation_mcp_tests.rs"]
mod resource_acquisition_continuation_mcp_tests;
#[cfg(test)]
#[path = "resource_acquisition_mcp_tests.rs"]
mod resource_acquisition_mcp_tests;

#[path = "lab_structured_query.rs"]
pub(crate) mod structured_query;

#[path = "lab_query_condition.rs"]
mod query_condition;

#[path = "lab_query_budget.rs"]
pub(crate) mod query_budget;

// The laboratory owns its temporal query state independently of every native
// profile. Reuse the same selection, comparison and publication semantics.
#[allow(dead_code, clippy::duplicate_mod)]
#[path = "semantic_query.rs"]
pub(crate) mod semantic_query;

#[allow(dead_code, clippy::duplicate_mod)]
#[path = "query_response.rs"]
mod query_response;

#[path = "lab_query_comparison.rs"]
mod query_comparison;

#[path = "lab_condition_replay.rs"]
mod condition_replay;

#[cfg(test)]
#[path = "lab_condition_replay_mcp_tests.rs"]
mod condition_replay_mcp_tests;

#[cfg(test)]
#[path = "lab_query_comparison_mcp_tests.rs"]
mod query_comparison_mcp_tests;

#[path = "lab_temporal_query.rs"]
mod temporal_query;

#[path = "lab_temporal_response.rs"]
mod temporal_response;

#[path = "lab_temporal_handler.rs"]
mod temporal_handler;

#[cfg(test)]
#[path = "lab_temporal_mcp_tests.rs"]
mod temporal_mcp_tests;

#[cfg(test)]
#[path = "lab_temporal_watch_mcp_tests.rs"]
mod temporal_watch_mcp_tests;

#[cfg(test)]
#[path = "lab_structured_query_tests.rs"]
mod structured_query_tests;

#[cfg(test)]
#[path = "lab_condition_mcp_tests.rs"]
mod condition_mcp_tests;

#[cfg(test)]
#[path = "lab_query_budget_mcp_tests.rs"]
mod query_budget_mcp_tests;

#[path = "room_provisioning.rs"]
mod room_provisioning;

#[path = "room_continuation.rs"]
mod room_continuation;

#[path = "connected_room.rs"]
mod connected_room;

#[path = "connected_room_continuation.rs"]
mod connected_room_continuation;

#[cfg(test)]
#[path = "connected_room_mcp_tests.rs"]
mod connected_room_mcp_tests;

#[cfg(test)]
#[path = "room_source_server_tests.rs"]
mod room_source_server_tests;

#[cfg(test)]
#[path = "room_continuation_mcp_tests.rs"]
mod room_continuation_mcp_tests;

#[path = "plan_forecast.rs"]
mod plan_forecast;
use plan_forecast::forecast_plan;
#[path = "lab_project_forecast.rs"]
mod project_forecast;
#[path = "lab_plan_comparison.rs"]
mod plan_comparison;
#[path = "lab_plan_comparison_handler.rs"]
mod plan_comparison_handler;
#[cfg(test)]
#[path = "lab_plan_comparison_mcp_tests.rs"]
mod plan_comparison_mcp_tests;
#[path = "lab_plan_comparison_response.rs"]
mod plan_comparison_response;

#[path = "lab_progress.rs"]
mod lab_progress;

#[path = "selected_plan_wait.rs"]
mod selected_plan_wait;

#[path = "selected_watch_wait.rs"]
mod selected_watch_wait;

#[cfg(test)]
#[path = "lab_watch_wait_tests.rs"]
mod watch_wait_mcp_tests;

#[path = "civilian_recovery.rs"]
mod civilian_recovery;

#[path = "server_alert_remedies.rs"]
mod alert_remedies;

#[cfg(test)]
#[path = "civilian_recovery_mcp_tests.rs"]
mod civilian_recovery_mcp_tests;

#[cfg(test)]
#[path = "lab_progress_mcp_tests.rs"]
mod lab_progress_mcp_tests;

#[path = "goal_continuation.rs"]
pub(crate) mod goal_continuation;

#[path = "goal_custody.rs"]
mod goal_custody;

#[cfg(test)]
#[path = "goal_continuation_mcp_tests.rs"]
mod goal_continuation_mcp_tests;

#[cfg(test)]
#[path = "physical_work_mcp_tests.rs"]
mod physical_work_mcp_tests;

#[cfg(test)]
#[path = "production_objective_tests.rs"]
mod production_objective_tests;

#[cfg(test)]
#[path = "objective_lifecycle_tests.rs"]
mod objective_lifecycle_tests;

/// One granted capability record returned to the client.
#[derive(Clone, Debug, PartialEq, Eq)]
struct NegotiatedCapability {
    capability: Capability,
    max_risk: RiskTier,
}

/// Per-session state. Lives behind an `Arc<Mutex<…>>` so multiple tool calls
/// within the same session share state without crossing `static` boundaries.
pub(crate) struct LabSession {
    pub(crate) session_id: SessionId,
    /// Reclaim private foreground query roots with their owning session.
    _query_resources: temporal_query::SessionGuard,
    #[allow(dead_code)]
    pub(crate) fortress_id: FortressId,
    /// The capabilities the caller negotiated in `fortress_open_session`.
    /// Transport identity grants nothing; these are the only authority.
    pub(crate) grants: Vec<CapabilityGrant>,
    /// The budget the caller negotiated.
    pub(crate) budget: WorkBudget,
    /// The seven MCP_SURFACE.md §Versioning negotiation items recorded at
    /// `fortress_open_session`.
    pub(crate) negotiation: SessionNegotiation,
    /// Per-session request counter (used as dfmcp RequestId).
    next_request_id: u128,
    /// The owned lab adapter.
    pub(crate) adapter: MemoryAdapter,
    /// Pending prepared plan awaiting commit.
    pending: Option<PendingPlan>,
    /// Most recent committed action id (for wait/cancel).
    last_action: Option<ActionId>,
    /// Every action of the most recent committed plan, in step order.
    last_plan_actions: Vec<ActionId>,
    /// Every committed action with unfinished proof or physical work when last
    /// inspected. `fortress.wait` also dispatches eligible deferred steps.
    open_actions: Vec<ActionId>,
    /// Bounded plan-digest to payload map for idempotent re-commit (ADR-006).
    commit_receipts: BTreeMap<String, String>,
    /// Authority each committed plan required; a replay must still hold it.
    commit_authority: BTreeMap<String, Vec<(Capability, RiskTier)>>,
    /// The shared fortress this session joined, if any. While a tool call
    /// runs, `with_session` swaps the world's adapter and lease book into
    /// `adapter` and `leases`; otherwise they are this session's own.
    shared: Option<Arc<Mutex<SharedWorld>>>,
    /// Spatial leases fencing committed temporal work.
    leases: LeaseBook,
    /// Members of the shared fortress during the current call (0 if private).
    shared_members: usize,
    /// Scenario name when this session's fortress is crash-durable: every
    /// state change is persisted to the durable laboratory store.
    durable_scenario: Option<String>,
    /// Last durable-persistence failure, reported until a later save succeeds.
    durability_fault: Option<String>,
    /// Plans committed in this process on a durable fortress whose steps are
    /// not all final; their step states are journaled after every call.
    durable_plans: BTreeMap<Digest32, PreparedPlan>,
    /// Retirement staged by restore, published with the restored world.
    durable_restore: BTreeMap<Digest32, Vec<u32>>,
    /// Steps of plans committed before a durable restart, re-proven against
    /// observation after every call until they are final.
    carried: Vec<CarriedStep>,
    /// Every tool call of this session, for deterministic replay bundles.
    pub(crate) replay: crate::replay::ReplayLog,
    /// Immutable world versions this session saw, retained by reachability:
    /// the recent window plus every version a live plan or checkpoint names,
    /// so every turn can say exactly what changed.
    history: dfmcp_world::retention::VersionRetention,
    /// The intent behind every committed plan, re-evaluated against each
    /// observation: dispatch success is not goal success.
    objectives: Vec<Objective>,
    /// Goal abandonment staged by restore, published with that exact world.
    objective_restore: BTreeSet<Digest32>,
    /// Accepted civilian observations belong to this world, never a caller.
    civilian_recovery: civilian_recovery::CivilianRecovery,
}

/// Observe the exact original goal separately from its actions and physical
/// work. The caller resolves any witnessed rebase to the receipt's actual
/// plan digest; a missing original goal never proves completion.
pub(crate) fn original_goal_observation(
    session: &LabSession,
    actual_plan_digest: &str,
) -> Result<(PredicateTruth, serde_json::Value)> {
    objectives::original_goal_observation(session, actual_plan_digest)
}

/// Every tracked objective with whether the current world satisfies it.
pub(crate) fn objectives_json(session: &LabSession) -> serde_json::Value {
    objectives::objectives_json(session)
}

/// Most recent world versions each session retains for change reporting.
const MAX_SESSION_HISTORY: usize = 32;
/// Older versions a session may keep alive because live work names them.
const MAX_PINNED_VERSIONS: usize = 1_100;

fn new_history() -> dfmcp_world::retention::VersionRetention {
    dfmcp_world::retention::VersionRetention::new(MAX_SESSION_HISTORY, MAX_PINNED_VERSIONS)
}

/// Records the current world version, then frees every version that neither
/// the recent window nor a live root reaches. Live roots: the pending plan's
/// sealed anchor, durable in-flight plans' anchors, and every checkpoint.
fn remember_version(session: &mut LabSession) {
    session
        .civilian_recovery
        .observe(session.adapter.snapshot());
    let current = session.adapter.snapshot().clone();
    session.history.record(&current);
    let mut roots: std::collections::BTreeSet<Digest32> =
        session.adapter.checkpoint_state_hashes().collect();
    roots.extend(session.pending.iter().map(|p| p.plan.anchor.state_hash));
    roots.extend(session.durable_plans.values().map(|p| p.anchor.state_hash));
    roots.extend(objective_history_roots(session));
    session.history.collect(&roots);
}

/// Dynamic civilian advice uses the world's retained monitor. Pure snapshot
/// alerts cannot establish a historical clear interval.
fn world_alerts(session: &LabSession) -> Vec<serde_json::Value> {
    let snapshot = session.adapter.snapshot();
    let mut alerts = crate::lab_world::world_alerts(snapshot);
    alerts.retain(|alert| alert["domain"] != "civilian_safety");
    alerts.extend(session.civilian_recovery.alerts());
    alert_remedies::qualify(
        snapshot,
        &session.civilian_recovery,
        &read_context(session),
        &mut alerts,
    );
    alerts
}

/// Why a version is not readable, as an explicit, honest reason.
fn not_retained_reason(session: &LabSession, hex: &str) -> String {
    use dfmcp_world::retention::VersionStatus;
    match session
        .history
        .find_hex(hex)
        .map(|h| session.history.status(&h))
    {
        Some(VersionStatus::Collected { .. }) => format!(
            "world version {hex} was collected: it left the last {MAX_SESSION_HISTORY} versions and no pending plan or checkpoint still names it"
        ),
        _ => format!("world version {hex} was never observed by this session"),
    }
}

/// Resolve an exact immutable version for an already-authorized resource
/// read while `with_session` owns the current fortress. Addresses never fall
/// back to a newer version or another session's history.
pub(crate) fn retained_resource_snapshot<'a>(
    session: &'a LabSession,
    hash: Option<&str>,
) -> Result<&'a WorldSnapshot> {
    let current = session.adapter.snapshot();
    match hash {
        None => Ok(current),
        Some(hash) if current.state_hash.to_hex() == hash => Ok(current),
        Some(hash) => session
            .history
            .find_hex(hash)
            .and_then(|digest| session.history.get(&digest))
            .ok_or_else(|| {
                DfmcpError::new(ErrorCode::StaleAnchor, not_retained_reason(session, hash))
            }),
    }
}

/// Observed world changes from the version with `from_state_hash` to the
/// newest version this session saw. `None` when the session is unknown;
/// a single `history_not_retained` item when the base aged out.
pub(crate) fn world_changes_since(
    session_id: &str,
    from_state_hash: &str,
) -> Option<Vec<serde_json::Value>> {
    let session = lookup_session_str(session_id).ok()?;
    let guard = session.lock().ok()?;
    let target = guard.history.newest()?;
    if target.state_hash.to_hex() == from_state_hash {
        return Some(Vec::new());
    }
    match guard
        .history
        .find_hex(from_state_hash)
        .and_then(|hash| guard.history.get(&hash))
    {
        Some(base) => Some(crate::world_changes::describe(base, target)),
        None => Some(vec![json!({
            "kind": "history_not_retained",
            "subject": {"from_state_hash": from_state_hash},
            "epistemic_state": "unknown",
            "invalidates": [],
            "evidence": [],
            "note": not_retained_reason(&guard, from_state_hash) + "; observe or query to re-establish the picture",
        })]),
    }
}

/// A step committed before a durable restart. No action handle survives the
/// restart, so its sealed proof is evaluated directly against observation.
#[derive(Clone, Debug)]
pub(crate) struct CarriedStep {
    plan_digest: Digest32,
    step: Option<dfmcp_core::StepId>,
    kind: &'static str,
    proof: Predicate,
    failure: Option<Predicate>,
    deadline: Option<dfmcp_core::GameTick>,
    monitor: Option<RecoveredObligation>,
    recorded_state: String,
    proof_anchor: Option<StateAnchor>,
    observation_error: Option<String>,
    failure_reason: Option<String>,
    /// A reproducible effect identity, retained independently of goal proof.
    /// Recovery observes it but never restores authority to dispatch it.
    work_step: Option<dfmcp_intent::PlanStep>,
    work_state: EffectWorkState,
    work_anchor: Option<StateAnchor>,
    /// Durable state, or indeterminate when recovery cannot certify it.
    state: String,
}

impl CarriedStep {
    fn to_json(&self) -> serde_json::Value {
        let mut work = effect_work_json(&self.work_state);
        work["observed_anchor"] = self
            .work_anchor
            .as_ref()
            .map(anchor_json)
            .unwrap_or(serde_json::Value::Null);
        json!({
            "plan_digest": self.plan_digest.to_hex(),
            "step": self.step.map(|step| step.get()),
            "action": self.kind,
            "state": self.state,
            "deadline_tick": self.deadline.map(|tick| tick.0),
            "failure_predicate": self.failure.as_ref().map(crate::lab_world::predicate_json),
            "recorded_state": self.recorded_state,
            "proof_anchor": self.proof_anchor.as_ref().map(anchor_json),
            "observation_error": self.observation_error,
            "failure_reason": self.failure_reason,
            "work_state": work,
            "recovery_class": if self.state == "indeterminate" { "reconciliation_required" } else { "never_unchanged" },
            "blind_retry_allowed": false,
            "stability": self.monitor.as_ref().and_then(|monitor| match monitor.status() {
                Some(ObligationStatus::Active { consecutive_stable_observations, .. }) => Some(json!({
                    "consecutive_observations": consecutive_stable_observations,
                    "recovery_anchor": anchor_json(&monitor.recovery_anchor()),
                    "policy": "unfinished stability resets at recovery; archived frontier is not a positive sample",
                })),
                _ => None,
            }),
        })
    }

    fn is_final(&self) -> bool {
        self.work_state.is_quiescent()
            && matches!(
                self.state.as_str(),
                "verified"
                    | "failed"
                    | "cancelled"
                    | "compensated"
                    | "not_dispatched"
                    | "abandoned"
            )
    }
}

/// Current Observe authority gates every resumed proof. Advance on a shadow
/// so an invalid observation cannot partially certify the recovery frontier.
fn observe_carried(session: &mut LabSession) -> Result<()> {
    if !session.carried.iter().any(|step| !step.is_final()) {
        return Ok(());
    }
    let ctx = context_for(session, session.next_request_id);
    if let Err(error) = authorize_entry(&ctx, Capability::Observe, RiskTier::ReadOnly) {
        for step in &mut session.carried {
            if !step.is_final() {
                if let Some(monitor) = &mut step.monitor {
                    monitor.observation_interrupted()?;
                }
                step.observation_error =
                    Some(format!("{}: {}", error.code.as_str(), error.message));
            }
        }
        return Err(error);
    }
    let observed = (|| -> Result<Vec<CarriedStep>> {
        let snapshot = session.adapter.snapshot();
        let evidence = PredicateEvidence::laboratory(snapshot)?;
        let mut next = session.carried.clone();
        for step in &mut next {
            if step.is_final() {
                continue;
            }
            step.observation_error = None;
            if let Some(work) = &step.work_step {
                // An anchored failure may have preceded dispatch; the legacy
                // journal does not distinguish that case. Never infer absence
                // from a missing work entity: retain Unknown for reconciliation.
                step.work_state = dfmcp_intent::inspect_effect_work(
                    snapshot,
                    &work.action,
                    &work.idempotency_key,
                    true,
                )?;
                step.work_anchor = Some(snapshot.anchor());
                if step.state == "dispatched"
                    && matches!(work.action, Action::HarvestResource { .. })
                {
                    dfmcp_intent::effects::validate_harvest_identity(
                        snapshot,
                        dfmcp_intent::effects::created_entity_id(&work.idempotency_key, 0),
                        &work.action,
                    )?;
                }
            }
            if step.state != "dispatched" {
                continue;
            }
            if let Some(monitor) = &mut step.monitor {
                monitor.observe_with_evidence(&evidence)?;
                match monitor.status() {
                    Some(ObligationStatus::Fulfilled { .. }) => {
                        step.state = "verified".to_owned();
                        step.proof_anchor = monitor.last_observation_anchor();
                    }
                    Some(ObligationStatus::Failed { reason, .. }) => {
                        step.state = "failed".to_owned();
                        step.failure_reason = Some(reason.clone());
                        step.proof_anchor = monitor.last_observation_anchor();
                    }
                    _ => {}
                }
            } else if evidence.establishes(&step.proof)? {
                step.state = "verified".to_owned();
                step.proof_anchor = Some(snapshot.anchor());
            }
            if step.state == "verified"
                && let Some(work) = &step.work_step
            {
                dfmcp_intent::effects::validate_harvest_completion(
                    snapshot,
                    dfmcp_intent::effects::created_entity_id(&work.idempotency_key, 0),
                    &work.action,
                )?;
            }
        }
        Ok(next)
    })();
    match observed {
        Ok(next) => {
            session.carried = next;
            Ok(())
        }
        Err(error) => {
            for retained in &mut session.carried {
                if !retained.is_final() {
                    if let Some(monitor) = &mut retained.monitor {
                        monitor.observation_interrupted()?;
                    }
                    retained.observation_error =
                        Some(format!("{}: {}", error.code.as_str(), error.message));
                }
            }
            Err(error)
        }
    }
}

/// Spatial leases plus the actions that hold them.
#[derive(Clone, Debug, Default)]
pub(crate) struct LeaseBook {
    manager: dfmcp_core::lease::LeaseManager,
    by_action: BTreeMap<ActionId, (SessionId, Vec<dfmcp_core::LeaseId>)>,
    /// Members who consented to unpausing a shared fortress. Any member may
    /// pause at once (the emergency brake), which clears every consent;
    /// unpausing needs all current members.
    unpause_consent: BTreeSet<SessionId>,
}

/// Whether a sealed plan sets the fortress pause flag to `paused`.
fn plan_sets_pause(plan: &PreparedPlan, paused: bool) -> bool {
    plan.steps
        .iter()
        .any(|step| step.action == Action::Pause { paused })
}

/// One fortress shared by several agent sessions: a single canonical world,
/// clock and lease book. Each member keeps its own grants, budget, plans and
/// receipts.
pub(crate) struct SharedWorld {
    adapter: MemoryAdapter,
    leases: LeaseBook,
    members: BTreeSet<SessionId>,
    scenario: String,
    durable: bool,
    durable_plans: BTreeMap<Digest32, PreparedPlan>,
    durable_restore: BTreeMap<Digest32, Vec<u32>>,
    carried: Vec<CarriedStep>,
    durability_fault: Option<String>,
    objectives: Vec<Objective>,
    objective_restore: BTreeSet<Digest32>,
    /// Accepted civilian observations belong to this world, never a caller.
    civilian_recovery: civilian_recovery::CivilianRecovery,
}

/// Admission drains existing calls before changing durable ownership. Calls
/// retain a read guard through their last durable publication.
static LAB_ADMISSION: RwLock<()> = RwLock::new(());

static SHARED_WORLDS: LazyLock<Mutex<BTreeMap<FortressId, Arc<Mutex<SharedWorld>>>>> =
    LazyLock::new(|| Mutex::new(BTreeMap::new()));
const MAX_SHARED_WORLDS: usize = 64;
const MAX_SHARED_MEMBERS: usize = 16;

struct SharedView {
    anchor: StateAnchor,
    paused: bool,
    members: usize,
    joined_existing: bool,
    scenario: String,
}

struct SharedAdmission {
    world: Arc<Mutex<SharedWorld>>,
    view: SharedView,
    adapter: MemoryAdapter,
    recovery: Option<DurableRecovery>,
}

/// Existing joins borrow the running owner without replaying durable recovery.
fn join_shared_world(
    fortress_id: FortressId,
    session_id: SessionId,
    scenario: &mut String,
    scenario_requested: bool,
    seed: &MemoryAdapter,
    durable: bool,
) -> Result<SharedAdmission> {
    let mut registry = SHARED_WORLDS.lock().map_err(|_| {
        DfmcpError::new(
            ErrorCode::InternalInvariantViolation,
            "shared world registry poisoned",
        )
    })?;
    if let Some(world) = registry.get(&fortress_id).cloned() {
        let mut guard = world.lock().map_err(|_| {
            DfmcpError::new(
                ErrorCode::InternalInvariantViolation,
                "shared world poisoned",
            )
        })?;
        if scenario_requested && guard.scenario != *scenario {
            return Err(DfmcpError::new(
                ErrorCode::InvalidRequest,
                format!(
                    "shared fortress {fortress_id} already runs scenario {:?}; omit scenario to join it",
                    guard.scenario
                ),
            ));
        }
        if guard.durable != durable {
            return Err(DfmcpError::new(
                ErrorCode::InvalidRequest,
                format!(
                    "shared fortress {fortress_id} is {}; open it with durable={}",
                    if guard.durable {
                        "crash-durable"
                    } else {
                        "process-local"
                    },
                    guard.durable
                ),
            ));
        }
        if guard.members.len() >= MAX_SHARED_MEMBERS {
            return Err(DfmcpError::new(
                ErrorCode::BudgetExceeded,
                "the shared fortress reached its member bound",
            ));
        }
        guard.members.insert(session_id);
        scenario.clone_from(&guard.scenario);
        let adapter = guard.adapter.clone();
        let view = SharedView {
            anchor: guard.adapter.snapshot().anchor(),
            paused: guard.adapter.snapshot().paused,
            members: guard.members.len(),
            joined_existing: true,
            scenario: guard.scenario.clone(),
        };
        drop(guard);
        return Ok(SharedAdmission {
            world,
            view,
            adapter,
            recovery: None,
        });
    }
    if registry.len() >= MAX_SHARED_WORLDS {
        return Err(DfmcpError::new(
            ErrorCode::BudgetExceeded,
            "the laboratory reached its shared-fortress bound",
        ));
    }
    let (adapter, recovery) = if durable {
        let (adapter, recovery) =
            load_durable_fortress(fortress_id, scenario, scenario_requested, seed)?;
        (adapter, Some(recovery))
    } else {
        (seed.clone(), None)
    };
    let world = SharedWorld {
        adapter: adapter.clone(),
        leases: LeaseBook::default(),
        members: BTreeSet::from([session_id]),
        scenario: scenario.to_owned(),
        durable,
        durable_plans: BTreeMap::new(),
        durable_restore: BTreeMap::new(),
        carried: recovery
            .as_ref()
            .map_or_else(Vec::new, |recovery| recovery.carried.clone()),
        durability_fault: None,
        objectives: recovery
            .as_ref()
            .map_or_else(Vec::new, |recovery| recovery.objectives.clone()),
        objective_restore: BTreeSet::new(),
        civilian_recovery: civilian_recovery::CivilianRecovery::default(),
    };
    let view = SharedView {
        anchor: world.adapter.snapshot().anchor(),
        paused: world.adapter.snapshot().paused,
        members: 1,
        joined_existing: false,
        scenario: scenario.to_owned(),
    };
    let world = Arc::new(Mutex::new(world));
    registry.insert(fortress_id, world.clone());
    Ok(SharedAdmission {
        world,
        view,
        adapter,
        recovery,
    })
}

/// Admission precedes world and session locks and lasts through publication.
/// The late owner check fences a handle resolved just before replacement.
pub(crate) fn with_session<T>(
    session: &Arc<Mutex<LabSession>>,
    poisoned: impl FnOnce() -> T,
    body: impl FnOnce(&mut LabSession) -> T,
) -> T {
    let _admission = match LAB_ADMISSION.read() {
        Ok(guard) => guard,
        Err(_) => return poisoned(),
    };
    if ensure_durable_owner(session).is_err() {
        return poisoned();
    }
    with_session_admitted(session, poisoned, body)
}

/// Synchronized inspection fences ownership exactly as a tool call does, but
/// cannot update objective history, retain a version or publish durable custody.
pub(crate) fn with_session_read<T>(
    session: &Arc<Mutex<LabSession>>,
    unavailable: impl FnOnce(DfmcpError) -> T,
    body: impl FnOnce(&LabSession) -> T,
) -> T {
    let _admission = match LAB_ADMISSION.read() {
        Ok(guard) => guard,
        Err(_) => {
            return unavailable(DfmcpError::new(
                ErrorCode::InternalInvariantViolation,
                "laboratory read admission poisoned",
            ));
        }
    };
    if let Err(error) = ensure_durable_owner(session) {
        return unavailable(error);
    }
    with_session_view(
        session,
        || {
            unavailable(DfmcpError::new(
                ErrorCode::InternalInvariantViolation,
                "laboratory read session poisoned",
            ))
        },
        |guard| body(guard),
        false,
    )
}

/// Opening already holds the write admission guard and uses this helper to
/// avoid recursively acquiring a read guard.
fn with_session_admitted<T>(
    session: &Arc<Mutex<LabSession>>,
    poisoned: impl FnOnce() -> T,
    body: impl FnOnce(&mut LabSession) -> T,
) -> T {
    with_session_view(session, poisoned, body, true)
}

fn with_session_view<T>(
    session: &Arc<Mutex<LabSession>>,
    poisoned: impl FnOnce() -> T,
    body: impl FnOnce(&mut LabSession) -> T,
    publish_progress: bool,
) -> T {
    let shared = match session.lock() {
        Ok(guard) => guard.shared.clone(),
        Err(_) => return poisoned(),
    };
    let mut world = match shared.as_ref().map(|world| world.lock()) {
        None => None,
        Some(Ok(world)) => Some(world),
        Some(Err(_)) => return poisoned(),
    };
    let mut guard = match session.lock() {
        Ok(guard) => guard,
        Err(_) => return poisoned(),
    };
    let prior_shared_members = guard.shared_members;
    if let Some(world) = world.as_mut() {
        std::mem::swap(&mut guard.adapter, &mut world.adapter);
        std::mem::swap(&mut guard.leases, &mut world.leases);
        std::mem::swap(&mut guard.durable_plans, &mut world.durable_plans);
        std::mem::swap(&mut guard.durable_restore, &mut world.durable_restore);
        std::mem::swap(&mut guard.carried, &mut world.carried);
        std::mem::swap(&mut guard.durability_fault, &mut world.durability_fault);
        std::mem::swap(&mut guard.objectives, &mut world.objectives);
        std::mem::swap(&mut guard.objective_restore, &mut world.objective_restore);
        std::mem::swap(&mut guard.civilian_recovery, &mut world.civilian_recovery);
        guard.shared_members = world.members.len();
    }
    if publish_progress {
        let current = &mut *guard;
        current
            .civilian_recovery
            .observe(current.adapter.snapshot());
    }
    let output = body(&mut guard);
    if publish_progress {
        persist_durable_head(&mut guard);
        remember_version(&mut guard);
    }
    if let Some(world) = world.as_mut() {
        std::mem::swap(&mut guard.adapter, &mut world.adapter);
        std::mem::swap(&mut guard.leases, &mut world.leases);
        std::mem::swap(&mut guard.durable_plans, &mut world.durable_plans);
        std::mem::swap(&mut guard.durable_restore, &mut world.durable_restore);
        std::mem::swap(&mut guard.carried, &mut world.carried);
        std::mem::swap(&mut guard.durability_fault, &mut world.durability_fault);
        std::mem::swap(&mut guard.objectives, &mut world.objectives);
        std::mem::swap(&mut guard.objective_restore, &mut world.objective_restore);
        std::mem::swap(&mut guard.civilian_recovery, &mut world.civilian_recovery);
    }
    if !publish_progress {
        guard.shared_members = prior_shared_members;
    }
    output
}

/// Operator-selected directory for crash-durable laboratory fortresses.
/// Never client-selected: MCP callers can only ask for `durable=true`.
const LAB_STATE_DIR_ENV: &str = "DFMCP_LAB_STATE_DIR";

/// The durable laboratory store, opened on first use, plus which session
/// currently owns each private durable fortress.
#[derive(Default)]
struct DurableLab {
    store: Option<dfmcp_lab::durable::DurableLabStore>,
    /// The newest private session per durable fortress. Opening a durable
    /// fortress again (for example after an agent lost its session) fences
    /// every older session of it so two writers never interleave.
    owners: BTreeMap<FortressId, SessionId>,
}

static DURABLE_LAB: LazyLock<Mutex<DurableLab>> =
    LazyLock::new(|| Mutex::new(DurableLab::default()));

fn durable_lab() -> MutexGuard<'static, DurableLab> {
    match DURABLE_LAB.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

/// Run `body` against the durable store, opening it from the operator's
/// configuration on first use.
fn with_durable_store<T>(
    body: impl FnOnce(&mut dfmcp_lab::durable::DurableLabStore) -> Result<T>,
) -> Result<T> {
    let mut lab = durable_lab();
    if lab.store.is_none() {
        #[cfg(test)]
        let configured = match TEST_STATE_DIR.lock() {
            Ok(guard) => guard.clone().map(std::ffi::OsString::from),
            Err(_) => None,
        };
        #[cfg(not(test))]
        let configured = std::env::var_os(LAB_STATE_DIR_ENV);
        let root = configured.ok_or_else(|| {
            DfmcpError::new(
                ErrorCode::CapabilityDenied,
                format!(
                    "durable laboratory fortresses are disabled: the operator has not set {LAB_STATE_DIR_ENV} to an absolute directory"
                ),
            )
        })?;
        lab.store = Some(dfmcp_lab::durable::DurableLabStore::open(
            std::path::Path::new(&root),
        )?);
    }
    match lab.store.as_mut() {
        Some(store) => body(store),
        None => Err(DfmcpError::new(
            ErrorCode::InternalInvariantViolation,
            "durable store vanished after opening",
        )),
    }
}

#[cfg(test)]
static TEST_STATE_DIR: Mutex<Option<std::path::PathBuf>> = Mutex::new(None);

#[cfg(test)]
static DURABLE_TEST_SERIAL: Mutex<()> = Mutex::new(());

#[cfg(test)]
pub(crate) fn serialized_durable_tests() -> MutexGuard<'static, ()> {
    match DURABLE_TEST_SERIAL.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

/// Test hook: drop the durable store (releasing its lock) and forget owners,
/// exactly what a process exit does, then use `dir` as the configured root.
#[cfg(test)]
pub(crate) fn simulate_durable_restart(dir: Option<std::path::PathBuf>) {
    let _admission = match LAB_ADMISSION.write() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    if let Ok(mut guard) = TEST_STATE_DIR.lock() {
        *guard = dir;
    }
    let mut lab = durable_lab();
    lab.store = None;
    lab.owners.clear();
    drop(lab);
    // Durable shared worlds die with the process too; process-local ones
    // belong to other (parallel) tests and stay.
    if let Ok(mut registry) = SHARED_WORLDS.lock() {
        registry.retain(|_, world| world.lock().is_ok_and(|world| !world.durable));
    }
}

/// A readable default summary for an action plan: each step's kind and its
/// identifying arguments, e.g. `set_labor BREW=true for 1003; designate_dig
/// mine [1,3,10]..[4,5,10]`. Objectives carry it, so it must say what the
/// plan is for rather than that it is a plan.
fn describe_actions(raw: &str) -> String {
    const MAX_DESCRIBED_BYTES: usize = 240;
    let Ok(serde_json::Value::Array(steps)) = serde_json::from_str::<serde_json::Value>(raw) else {
        return "execute semantic actions".to_owned();
    };
    let text = |v: &serde_json::Value| match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(items) => items
            .iter()
            .map(|i| i.as_str().map_or_else(|| i.to_string(), str::to_owned))
            .collect::<Vec<_>>()
            .join(","),
        other => other.to_string(),
    };
    let parts: Vec<String> = steps
        .iter()
        .map(|step| {
            let action = &step["action"];
            let kind = action["kind"].as_str().unwrap_or("action");
            let mut words = vec![kind.to_owned()];
            for key in [
                "mode",
                "building",
                "job_token",
                "labor",
                "name",
                "key",
                "value",
            ] {
                if let Some(value) = action.get(key).filter(|v| !v.is_null()) {
                    words.push(text(value));
                }
            }
            if let Some(enabled) = action.get("enabled").and_then(serde_json::Value::as_bool) {
                words.push(if enabled { "on" } else { "off" }.to_owned());
            }
            if let Some(assigned) = action.get("assigned").and_then(serde_json::Value::as_bool) {
                words.push(if assigned { "join" } else { "leave" }.to_owned());
            }
            if let Some(amount) = action.get("amount").and_then(serde_json::Value::as_u64) {
                words.push(format!("x{amount}"));
            }
            for (key, label) in [("units", "for"), ("squad", "into"), ("burrow", "in")] {
                if let Some(value) = action.get(key).filter(|v| !v.is_null()) {
                    words.push(format!("{label} {}", text(value)));
                }
            }
            if let (Some(min), Some(max)) = (action.get("min"), action.get("max")) {
                words.push(format!("{min}..{max}"));
            }
            words.join(" ")
        })
        .collect();
    let mut summary = parts.join("; ");
    if summary.is_empty() {
        return "execute semantic actions".to_owned();
    }
    if summary.len() > MAX_DESCRIBED_BYTES {
        let mut cut = MAX_DESCRIBED_BYTES;
        while !summary.is_char_boundary(cut) {
            cut -= 1;
        }
        summary.truncate(cut);
        summary.push_str("...");
    }
    summary
}

/// Test hook: forget a process-local shared fortress and its member
/// sessions, so exploration campaigns stay inside the shared-world bound.
#[cfg(test)]
pub(crate) fn release_shared_world(fortress_id: FortressId) {
    let world = SHARED_WORLDS
        .lock()
        .ok()
        .and_then(|mut registry| registry.remove(&fortress_id));
    if let Some(members) = world.and_then(|w| w.lock().ok().map(|w| w.members.clone())) {
        let mut registry = sessions();
        for member in members {
            registry.remove(&member);
        }
    }
}

/// Test hook: let the open durable store accept `budget` more journal
/// appends before behaving as if the process died.
#[cfg(test)]
pub(crate) fn inject_durable_crash_after(budget: usize) {
    if let Some(store) = durable_lab().store.as_mut() {
        store.set_append_budget(Some(budget));
    }
}

/// Persist the session's world when it is durable and changed. A failure is
/// kept on the session (and surfaced by the Agent Turn and doctor) rather
/// than silently ignored; the response that caused it is already computed.
fn persist_durable_head(session: &mut LabSession) {
    let satisfied = objectives::newly_satisfied_objectives(session);
    let abandoned: Vec<_> = session.objective_restore.iter().copied().collect();
    let Some(scenario) = session.durable_scenario.clone() else {
        let anchor = session.adapter.snapshot().anchor();
        objectives::record_objective_progress(session, anchor, &satisfied, &abandoned);
        return;
    };
    let snapshot = session.adapter.snapshot().clone();
    let fortress = session.fortress_id;
    // Publish step states and their world together. Separate state-before-head
    // writes can leave an immediately verified effect ahead of the recovered
    // world; reversing those writes can lose the dispatch record instead.
    let mut finished: Vec<Digest32> = Vec::new();
    let mut updates: Vec<(Digest32, u32, &'static str)> = Vec::new();
    for (digest, plan) in &session.durable_plans {
        let mut all_final = true;
        for step in &plan.steps {
            let token = match session
                .adapter
                .step_receipt(plan.id, step.id)
                .map(|receipt| receipt.state)
            {
                None | Some(CommitState::Prepared) => None,
                Some(CommitState::Verified) => Some("verified"),
                Some(CommitState::Failed) => Some("failed"),
                Some(CommitState::Cancelled) => Some("cancelled"),
                Some(CommitState::Compensated) => Some("compensated"),
                Some(_) => Some("dispatched"),
            };
            match token {
                Some(token) => {
                    all_final &= token != "dispatched";
                    all_final &=
                        session
                            .adapter
                            .step_receipt(plan.id, step.id)
                            .is_some_and(|receipt| {
                                action_fully_drained(&session.adapter, receipt.action_id)
                            });
                    updates.push((*digest, step.id.get(), token));
                }
                None => {
                    all_final = false;
                    // This live step can still dispatch later. Its current
                    // absence nevertheless belongs in the atomic frontier.
                    updates.push((*digest, step.id.get(), "not_dispatched"));
                }
            }
        }
        if all_final {
            finished.push(*digest);
        }
    }
    // Authorization gates proof independently of persistence: even a denied
    // read must not prevent saving the world's current state.
    let _ = observe_carried(session);
    let carried_updates: Vec<(Digest32, u32, String)> = session
        .carried
        .iter()
        .filter(|c| c.state != "indeterminate")
        .filter_map(|c| Some((c.plan_digest, c.step?.get(), c.state.clone())))
        .collect();
    let carried_done: BTreeSet<Digest32> = session
        .carried
        .iter()
        .map(|c| c.plan_digest)
        .filter(|digest| {
            session
                .carried
                .iter()
                .filter(|c| c.plan_digest == *digest)
                .all(CarriedStep::is_final)
        })
        .collect();
    let result = with_durable_store(|store| {
        let mut frontier = BTreeMap::new();
        for (digest, step, token) in &updates {
            frontier.insert((*digest, *step), (*token).to_owned());
        }
        for (digest, step, token) in &carried_updates {
            // A carried commit stays visible after it is retired; only an
            // unfinished one still takes step records.
            if store.commit(fortress, *digest).is_some() {
                frontier.insert((*digest, *step), token.clone());
            }
        }
        // A later physical-work observation must not re-anchor a historic
        // terminal proof. Publish the new world while retaining the first
        // atomic terminal frontier for an unchanged outcome.
        frontier.retain(|(digest, step), state| {
            let terminal = matches!(
                state.as_str(),
                "verified" | "failed" | "cancelled" | "compensated"
            );
            !terminal
                || !store.commit(fortress, *digest).is_some_and(|commit| {
                    commit.steps.get(step) == Some(state) && commit.step_anchors.contains_key(step)
                })
        });
        for (digest, steps) in &session.durable_restore {
            for step in steps {
                frontier.insert((*digest, *step), "abandoned".to_owned());
            }
        }
        let frontier: Vec<_> = frontier
            .into_iter()
            .map(
                |((plan_digest, step), state)| dfmcp_lab::durable::DurableStepUpdate {
                    plan_digest,
                    step,
                    state,
                },
            )
            .collect();
        let retired: Vec<_> = finished
            .iter()
            .chain(carried_done.iter())
            .chain(session.durable_restore.keys())
            .copied()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        store.persist_progress_with_objectives(
            &scenario, &snapshot, &frontier, &retired, &satisfied, &abandoned,
        )
    });
    if result.is_ok() {
        for digest in &finished {
            session.durable_plans.remove(digest);
        }
        session.durable_restore.clear();
        objectives::record_objective_progress(session, snapshot.anchor(), &satisfied, &abandoned);
    }
    match result {
        Ok(()) => session.durability_fault = None,
        Err(error) => {
            session.durability_fault = Some(format!("{}: {}", error.code.as_str(), error.message));
        }
    }
}

/// Fail closed while a durable fortress has state it could not persist:
/// retry the save, and refuse further effects until it succeeds.
fn durability_gate(session: &mut LabSession) -> Result<()> {
    if session.durability_fault.is_none() {
        return Ok(());
    }
    persist_durable_head(session);
    match &session.durability_fault {
        None => Ok(()),
        Some(fault) => Err(DfmcpError::new(
            ErrorCode::AdapterUnavailable,
            format!(
                "durable fortress could not persist its latest state ({fault}); effects are refused until persistence recovers"
            ),
        )),
    }
}

/// Whether a private durable session has been superseded by a newer one.
fn ensure_durable_owner(session: &Arc<Mutex<LabSession>>) -> Result<()> {
    let (session_id, fortress_id, durable, shared) = match session.lock() {
        Ok(guard) => (
            guard.session_id,
            guard.fortress_id,
            guard.durable_scenario.is_some(),
            guard.shared.clone(),
        ),
        Err(_) => {
            return Err(DfmcpError::new(
                ErrorCode::InternalInvariantViolation,
                "session poisoned",
            ));
        }
    };
    if !durable {
        return Ok(());
    }
    if let Some(shared) = shared {
        let registry = SHARED_WORLDS.lock().map_err(|_| {
            DfmcpError::new(
                ErrorCode::InternalInvariantViolation,
                "shared world registry poisoned",
            )
        })?;
        return if registry
            .get(&fortress_id)
            .is_some_and(|current| Arc::ptr_eq(current, &shared))
        {
            Ok(())
        } else {
            Err(DfmcpError::new(
                ErrorCode::Conflict,
                "this shared durable session belongs to an earlier process epoch; open a new session",
            ))
        };
    }
    match durable_lab().owners.get(&fortress_id) {
        Some(owner) if *owner == session_id => Ok(()),
        Some(owner) => Err(DfmcpError::new(
            ErrorCode::Conflict,
            format!(
                "this session was superseded: durable fortress {fortress_id} was reopened by session {owner}; continue there"
            ),
        ).retryable(false)),
        None => Err(DfmcpError::new(
            ErrorCode::Conflict,
            "this durable session no longer owns the fortress; open a new session",
        )),
    }
}

/// What reopening a durable fortress recovered.
struct DurableRecovery {
    resumed: bool,
    recovered_from: Option<StateAnchor>,
    checkpoints: usize,
    torn_tail_bytes: u64,
    /// Dispatched steps of earlier commits, still to be proven.
    carried: Vec<CarriedStep>,
    /// What happened to every earlier unfinished commit.
    commits: Vec<serde_json::Value>,
    /// Goals are retained independently of unfinished action commits.
    objectives: Vec<Objective>,
}

/// Rebuild the steps of a commit made before a restart. The plan is
/// recompiled from its recorded request against the exact world it was
/// sealed on; determinism must reproduce the sealed digest, or the commit is
/// retained as indeterminate instead of claiming its earlier effects were absent.
fn recover_commit(
    store: &mut dfmcp_lab::durable::DurableLabStore,
    commit: &dfmcp_lab::durable::DurableCommit,
    carried: &mut Vec<CarriedStep>,
    recovered: &WorldSnapshot,
) -> Result<serde_json::Value> {
    let fortress = commit.fortress_id;
    let sealed = store.load_snapshot(commit.sealed_state_hash)?;
    let original_head = store.head(fortress).cloned();
    let head_diverged = original_head
        .as_ref()
        .is_none_or(|head| head.anchor != sealed.anchor());
    let source = PlanSource::from_durable(&commit.source);
    // This internal pure planning grant verifies the sealed request. It
    // confers no authority on a session, action, or recovery monitor.
    let context = OperationContext {
        session_id: SessionId::new(u128::MAX),
        request_id: RequestId::new(commit.intent_id),
        anchor: sealed.anchor(),
        budget: MAX_LAB_BUDGET,
        grants: vec![CapabilityGrant {
            capability: Capability::Plan,
            scope: CapabilityScope {
                fortress_id: Some(fortress),
                ..CapabilityScope::default()
            },
            max_risk: RiskTier::ReadOnly,
            expires_at_tick: None,
            remaining_uses: None,
        }],
        cancellation_requested: false,
    };
    let recompiled = source
        .intent(IntentId::new(commit.intent_id), &sealed)
        .and_then(|intent| StaticPlanner::default().prepare_laboratory(&sealed, &intent, &context));
    let plan = match recompiled {
        Ok(plan) if plan.digest == commit.plan_digest => plan,
        outcome => {
            let reason = match outcome {
                Ok(plan) => format!(
                    "recompilation produced digest {} instead of the sealed {}",
                    plan.digest, commit.plan_digest
                ),
                Err(error) => error.message,
            };
            carried.push(CarriedStep {
                plan_digest: commit.plan_digest,
                step: None,
                kind: "unverifiable_plan",
                proof: Predicate::False,
                failure: None,
                deadline: None,
                monitor: None,
                recorded_state: "unverifiable".to_owned(),
                proof_anchor: None,
                observation_error: Some(reason.clone()),
                failure_reason: None,
                work_step: None,
                work_state: EffectWorkState::Unknown {
                    entity_id: None,
                    reason: "sealed effect identity could not be reproduced".to_owned(),
                },
                work_anchor: None,
                state: "indeterminate".to_owned(),
            });
            return Ok(json!({
                "plan_digest": commit.plan_digest.to_hex(),
                "status": "unverifiable",
                "reason": reason,
                "state": "indeterminate",
                "recorded_steps": commit.steps,
                "blind_retry_allowed": false,
                "note": "the sealed plan could not be reproduced; its durable record remains retained and its effects require reconciliation before re-planning",
            }));
        }
    };
    let mut steps = Vec::new();
    let mut open = 0usize;
    for step in &plan.steps {
        let recorded = commit.steps.get(&step.id.get()).map(String::as_str);
        let kind = crate::lab_world::action_kind(&step.action);
        let mut recorded_anchor = commit.step_anchors.get(&step.id.get()).copied();
        // Legacy peers could publish a head without the originating session's
        // step record. Divergent unanchored absence cannot prove nondispatch.
        let unanchored_absence = recorded_anchor.is_none()
            && (matches!(recorded, Some("not_dispatched" | "abandoned"))
                || (head_diverged && recorded.is_none()));
        let state = match recorded {
            None if !unanchored_absence => {
                if let Some(head) = &original_head {
                    store.persist_progress(
                        &head.scenario,
                        &sealed,
                        &[dfmcp_lab::durable::DurableStepUpdate {
                            plan_digest: commit.plan_digest,
                            step: step.id.get(),
                            state: "not_dispatched".to_owned(),
                        }],
                        &[],
                    )?;
                    recorded_anchor = store
                        .commit(fortress, commit.plan_digest)
                        .and_then(|commit| commit.step_anchors.get(&step.id.get()))
                        .copied();
                    if recorded_anchor != Some(sealed.anchor()) {
                        return Err(DfmcpError::new(
                            ErrorCode::CorruptLedger,
                            "new nondispatch recovery evidence did not retain its exact sealed frontier",
                        ));
                    }
                }
                "not_dispatched".to_owned()
            }
            None => "not_recorded".to_owned(),
            Some(state) => state.to_owned(),
        };
        let unanchored_terminal = recorded_anchor.is_none()
            && matches!(
                state.as_str(),
                "verified" | "failed" | "cancelled" | "compensated"
            );
        let invalid_nondispatch = if state == "not_dispatched" && !unanchored_absence {
            (|| -> Result<()> {
                let anchor = recorded_anchor.ok_or_else(|| {
                    DfmcpError::new(
                        ErrorCode::CorruptLedger,
                        "recovered nondispatch has no atomic frontier",
                    )
                })?;
                let upper = original_head
                    .as_ref()
                    .ok_or_else(|| {
                        DfmcpError::new(
                            ErrorCode::CorruptLedger,
                            "recovered nondispatch has no retained fortress head",
                        )
                    })?
                    .anchor;
                let archived = store.load_snapshot(anchor.state_hash)?;
                goal_custody::verify_nondispatch_frontier(
                    &plan,
                    step.id.get(),
                    anchor,
                    &archived,
                    Some(upper),
                )?;
                if !goal_custody::effect_identity_absent(step, recovered) {
                    return Err(DfmcpError::new(
                        ErrorCode::CorruptLedger,
                        "current effect identity contradicts recovered nondispatch custody",
                    ));
                }
                Ok(())
            })()
            .err()
            .map(|error| format!("{}: {}", error.code.as_str(), error.message))
        } else {
            None
        };
        let work_state = if let Some(reason) = &invalid_nondispatch {
            EffectWorkState::Unknown {
                entity_id: None,
                reason: reason.clone(),
            }
        } else if state == "not_dispatched" && !unanchored_absence {
            EffectWorkState::NeverDispatched
        } else {
            dfmcp_intent::inspect_effect_work(recovered, &step.action, &step.idempotency_key, true)
                .unwrap_or_else(|error| EffectWorkState::Unknown {
                    entity_id: None,
                    reason: format!("{}: {}", error.code.as_str(), error.message),
                })
        };
        let unfinished = state == "dispatched"
            || unanchored_terminal
            || unanchored_absence
            || !work_state.is_quiescent();
        if unfinished || work_state == EffectWorkState::NeverDispatched {
            open += usize::from(unfinished);
            let mut predicates = step.postconditions.clone();
            if let Some(obligation) = &step.obligation {
                predicates.push(obligation.terminal.clone());
            }
            let proof = Predicate::All(predicates).normalized();
            let mut identity = b"dfmcp-recovered-obligation-v1".to_vec();
            identity.extend_from_slice(commit.plan_digest.as_bytes());
            identity.extend_from_slice(&step.id.get().to_be_bytes());
            let proof_id = ActionId::new(Digest32::of_bytes(&identity).first_u128().max(1));
            let monitor_result =
                if state != "dispatched" || unanchored_terminal || unanchored_absence {
                    Ok(None)
                } else if let Some(obligation) = &step.obligation {
                    let mut spec = obligation.clone();
                    spec.terminal = proof.clone();
                    RecoveredObligation::new_laboratory(proof_id, spec, sealed.tick, recovered)
                        .map(Some)
                } else {
                    proof.validate_shape().and_then(|()| {
                        if matches!(proof, Predicate::True | Predicate::False) {
                            Err(DfmcpError::new(
                                ErrorCode::InvalidPlan,
                                "recovered effect has no nontrivial proof predicate",
                            ))
                        } else {
                            Ok(None)
                        }
                    })
                };
            let (monitor, monitor_error) = match monitor_result {
                Ok(monitor) => (monitor, None),
                Err(error) => (
                    None,
                    Some(format!(
                        "recovered proof specification is inadmissible: {}: {}",
                        error.code.as_str(),
                        error.message,
                    )),
                ),
            };
            let unresolved = unanchored_terminal
                || unanchored_absence
                || invalid_nondispatch.is_some()
                || monitor_error.is_some();
            carried.push(CarriedStep {
                plan_digest: commit.plan_digest,
                step: Some(step.id),
                kind,
                proof,
                failure: step.obligation.as_ref().and_then(|o| o.failure.clone()),
                deadline: step.obligation.as_ref().map(|o| o.deadline_tick),
                monitor,
                recorded_state: state.clone(),
                proof_anchor: if unresolved { None } else { recorded_anchor },
                observation_error: if unanchored_terminal {
                    Some("legacy terminal state has no atomic world frontier; its effects require reconciliation".to_owned())
                } else if unanchored_absence {
                    Some("the legacy world advanced without an anchored dispatch record; absence of that record cannot prove the effect was not dispatched".to_owned())
                } else {
                    invalid_nondispatch.clone().or(monitor_error)
                },
                failure_reason: None,
                work_step: Some(step.clone()),
                work_state: work_state.clone(),
                work_anchor: Some(recovered.anchor()),
                state: if unresolved { "indeterminate".to_owned() } else { state.clone() },
            });
        }
        let recovered_state = carried
            .last()
            .filter(|carried| {
                carried.plan_digest == commit.plan_digest && carried.step == Some(step.id)
            })
            .map_or(state.as_str(), |carried| carried.state.as_str());
        steps.push(json!({
            "step": step.id.get(), "action": kind, "state": recovered_state,
            "recorded_state": recorded,
            "recorded_anchor": recorded_anchor.as_ref().map(anchor_json),
            "work_state": effect_work_json(&work_state),
            "blind_retry_allowed": false,
            "proof_class": if invalid_nondispatch.is_some() { "invalid_nondispatch_custody" }
                else if unanchored_terminal || unanchored_absence { "unanchored_legacy_record" }
                else if recorded_anchor.is_some() { "atomic_world_frontier" } else { "not_proven" },
        }));
    }
    if open == 0 {
        store.retire_commit(fortress, commit.plan_digest)?;
    }
    Ok(json!({
        "plan_digest": commit.plan_digest.to_hex(),
        "status": if open == 0 { "final" } else { "carried" },
        "steps": steps,
    }))
}

/// Load (or start) a durable fortress: the latest persisted world in a new
/// observation epoch with every durable checkpoint restorable, or the named
/// scenario when the store has never seen this fortress.
fn load_durable_fortress(
    fortress_id: FortressId,
    scenario: &mut String,
    scenario_requested: bool,
    fresh: &MemoryAdapter,
) -> Result<(MemoryAdapter, DurableRecovery)> {
    with_durable_store(|store| {
        let report = store.report();
        let Some(head) = store.head(fortress_id).cloned() else {
            return Ok((
                fresh.clone(),
                DurableRecovery {
                    resumed: false,
                    recovered_from: None,
                    checkpoints: 0,
                    torn_tail_bytes: report.torn_tail_bytes,
                    carried: Vec::new(),
                    commits: Vec::new(),
                    objectives: Vec::new(),
                },
            ));
        };
        if scenario_requested && head.scenario != *scenario {
            return Err(DfmcpError::new(
                ErrorCode::InvalidRequest,
                format!(
                    "durable fortress {fortress_id} already runs scenario {:?}; omit scenario to resume it",
                    head.scenario
                ),
            ));
        }
        scenario.clone_from(&head.scenario);
        let snapshot = store.load_snapshot(head.anchor.state_hash)?;
        if snapshot.fortress_id != fortress_id {
            return Err(DfmcpError::new(
                ErrorCode::CorruptLedger,
                "durable head belongs to a different fortress",
            ));
        }
        let mut adapter = MemoryAdapter::recovered(snapshot.clone())?;
        // The durable head is still the pre-recovery world. Bind retained goals
        // to this exact reset before any recovered progress can publish a head.
        let goal_recovery = goal_continuation::RecoveredGoalLineage::verify(
            &snapshot,
            head.anchor,
            adapter.snapshot().anchor(),
        )?;
        drop(snapshot);
        let mut checkpoints = 0;
        for checkpoint in store.checkpoints(fortress_id).cloned().collect::<Vec<_>>() {
            adapter.adopt_checkpoint(
                checkpoint.checkpoint_id,
                store.load_snapshot(checkpoint.state_hash)?,
            )?;
            checkpoints += 1;
        }
        let mut carried = Vec::new();
        let mut commits = Vec::new();
        let objectives = store
            .objectives(fortress_id)
            .map(|retained| objectives::recover_objective(store, retained, &goal_recovery))
            .collect();
        for commit in store.commits(fortress_id).cloned().collect::<Vec<_>>() {
            commits.push(recover_commit(
                store,
                &commit,
                &mut carried,
                adapter.snapshot(),
            )?);
        }
        Ok((
            adapter,
            DurableRecovery {
                resumed: true,
                recovered_from: Some(head.anchor),
                checkpoints,
                torn_tail_bytes: report.torn_tail_bytes,
                carried,
                commits,
                objectives,
            },
        ))
    })
}

/// Doctor view of durability for one session.
fn durability_json(session: &LabSession) -> serde_json::Value {
    let Some(scenario) = session.durable_scenario.as_ref() else {
        return json!({
            "durable": false,
            "note": "process-local: this fortress is lost when the server process exits",
        });
    };
    let lab = durable_lab();
    let report = lab
        .store
        .as_ref()
        .map(dfmcp_lab::durable::DurableLabStore::report);
    let head = lab
        .store
        .as_ref()
        .and_then(|store| store.head(session.fortress_id).cloned());
    let goals_observable = authorize_entry(
        &context_for(session, session.next_request_id),
        Capability::Observe,
        RiskTier::ReadOnly,
    )
    .is_ok();
    json!({
        "durable": true,
        "scenario": scenario,
        "fault": session.durability_fault,
        "persisted_anchor": head.as_ref().map(|head| anchor_json(&head.anchor)),
        "carried_obligations": session.carried.iter().map(CarriedStep::to_json).collect::<Vec<_>>(),
        "retained_objectives": goals_observable.then_some(session.objectives.len()),
        "objective_history": "original sources and first verified achievement anchors survive retired action commits; legacy records without objectives have no invented goal history",
        "persisted_is_current": head.as_ref().is_some_and(|head| head.anchor == session.adapter.snapshot().anchor()),
        "store": report.map(|report| json!({
            "records": report.records,
            "fortresses": report.fortresses,
            "checkpoints": report.checkpoints,
            "chain_head": report.chain_head.to_hex(),
            "torn_tail_bytes_discarded_at_open": report.torn_tail_bytes,
            "compactions": report.compactions,
        })),
        "note": "laboratory durability: world state, checkpoints and bounded recovery proofs survive process loss; adapter action handles and dispatch authority do not",
    })
}

/// A plan sealed by `fortress_plan` and awaiting `fortress_commit`.
struct PendingPlan {
    plan: PreparedPlan,
    digest: String,
    /// How the intent was requested, so a stale plan can be replayed.
    source: PlanSource,
}

/// The agent-level request behind a sealed plan.
#[derive(Clone, Debug)]
enum PlanSource {
    Pause {
        summary: String,
        paused_target: bool,
    },
    Actions {
        summary: String,
        raw: String,
    },
    /// An objective: a blueprint template decomposed by the blueprint
    /// planner into dig, furnishing and dependency steps.
    Blueprint {
        summary: String,
        raw: String,
    },
    /// Original normalized quotas, retained across stock-dependent lowering,
    /// stale-plan replay and exact sealed-world recovery.
    Production {
        summary: String,
        raw: String,
    },
    /// A new explicit pursuit of the complete retained production request.
    /// Flat lineage and source bytes participate in the newly reviewed seal.
    ProductionContinuation {
        summary: String,
        raw: String,
    },
    /// A new residual pursuit of the complete original room layout. Its exact
    /// placement proof is independent of every historical plan in the lineage.
    RoomContinuation {
        summary: String,
        raw: String,
    },
    /// Owned stable clearance and the complete exact civilian census.
    CivilianRelease {
        summary: String,
        raw: String,
    },
    /// The complete versioned room plus its immutable entrance and routing domain.
    ConnectedRoom {
        summary: String,
        raw: String,
    },
    /// A reviewed residual pursuit retaining the complete original access goal.
    ConnectedRoomContinuation {
        summary: String,
        raw: String,
    },
    /// A reviewed residual pursuit of the original finite WOOD/PLANT minima.
    ResourceAcquisitionContinuation {
        summary: String,
        raw: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum GoalContinuation {
    Production(goal_continuation::ProductionContinuation),
    Room(room_continuation::RoomContinuation),
    ConnectedRoom(connected_room_continuation::ConnectedRoomContinuation),
    ResourceAcquisition(resource_acquisition_continuation::ResourceAcquisitionContinuation),
}

impl GoalContinuation {
    fn is_resource_aware_room(&self) -> bool {
        match self {
            Self::Room(source) => source.is_resource_aware(),
            Self::ConnectedRoom(source) => source.is_resource_aware(),
            _ => false,
        }
    }

    fn resource_room_goal_truth(&self, snapshot: &WorldSnapshot) -> Result<PredicateTruth> {
        match self {
            Self::Room(source) if source.is_resource_aware() => {
                source.semantic_goal_truth(snapshot)
            }
            Self::ConnectedRoom(source) if source.is_resource_aware() => {
                source.semantic_goal_truth(snapshot)
            }
            _ => Err(DfmcpError::new(
                ErrorCode::InvalidIntent,
                "semantic room completion requires the retained resource-aware room request",
            )),
        }
    }

    fn parent(&self) -> Digest32 {
        match self {
            Self::Production(source) => source.parent,
            Self::Room(source) => source.parent,
            Self::ConnectedRoom(source) => source.parent,
            Self::ResourceAcquisition(source) => source.parent,
        }
    }

    fn root(&self) -> Digest32 {
        match self {
            Self::Production(source) => source.root,
            Self::Room(source) => source.root,
            Self::ConnectedRoom(source) => source.root,
            Self::ResourceAcquisition(source) => source.root,
        }
    }

    fn requires_semantic_sibling_check(&self) -> bool {
        matches!(
            self,
            Self::Room(_) | Self::ConnectedRoom(_) | Self::ResourceAcquisition(_)
        ) || matches!(self, Self::Production(source) if source.is_resource_aware()
            || source.is_current_consumption_reserve())
    }

    fn lineage_json(&self) -> serde_json::Value {
        match self {
            Self::Production(source) => source.lineage_json(),
            Self::Room(source) => source.lineage_json(),
            Self::ConnectedRoom(source) => source.lineage_json(),
            Self::ResourceAcquisition(source) => source.lineage_json(),
        }
    }

    fn same_original_request(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Production(left), Self::Production(right)) => left.same_original_request(right),
            (Self::Room(left), Self::Room(right)) => left.same_original_request(right),
            (Self::ConnectedRoom(left), Self::ConnectedRoom(right)) => {
                left.same_original_request(right)
            }
            (Self::ResourceAcquisition(left), Self::ResourceAcquisition(right)) => {
                left.same_original_request(right)
            }
            _ => false,
        }
    }

    fn into_source(self, summary: String) -> PlanSource {
        match self {
            Self::Production(source) => PlanSource::ProductionContinuation {
                summary,
                raw: source.canonical_json(),
            },
            Self::Room(source) => PlanSource::RoomContinuation {
                summary,
                raw: source.canonical_json(),
            },
            Self::ConnectedRoom(source) => PlanSource::ConnectedRoomContinuation {
                summary,
                raw: source.canonical_json(),
            },
            Self::ResourceAcquisition(source) => PlanSource::ResourceAcquisitionContinuation {
                summary,
                raw: source.canonical_json(),
            },
        }
    }
}

impl PlanSource {
    fn is_resource_aware_room(&self) -> bool {
        match self {
            Self::Blueprint { raw, .. } if room_provisioning::is_provisioned_room(raw) => {
                room_provisioning::ProvisionedRoomRequest::parse(raw)
                    .is_ok_and(|request| request.is_resource_aware())
            }
            Self::RoomContinuation { raw, .. } => room_continuation::RoomContinuation::parse(raw)
                .is_ok_and(|source| source.is_resource_aware()),
            Self::ConnectedRoom { raw, .. } => connected_room::ConnectedRoomRequest::parse(raw)
                .is_ok_and(|request| request.is_resource_aware()),
            Self::ConnectedRoomContinuation { raw, .. } => {
                connected_room_continuation::ConnectedRoomContinuation::parse(raw)
                    .is_ok_and(|source| source.is_resource_aware())
            }
            _ => false,
        }
    }

    fn resource_room_goal_truth(&self, snapshot: &WorldSnapshot) -> Result<PredicateTruth> {
        match self {
            Self::Blueprint { raw, .. } if room_provisioning::is_provisioned_room(raw) => {
                room_provisioning::ProvisionedRoomRequest::parse(raw)?.semantic_goal_truth(snapshot)
            }
            Self::RoomContinuation { raw, .. } => {
                room_continuation::RoomContinuation::parse(raw)?.semantic_goal_truth(snapshot)
            }
            Self::ConnectedRoom { raw, .. } => {
                connected_room::ConnectedRoomRequest::parse(raw)?.semantic_goal_truth(snapshot)
            }
            Self::ConnectedRoomContinuation { raw, .. } => {
                connected_room_continuation::ConnectedRoomContinuation::parse(raw)?
                    .semantic_goal_truth(snapshot)
            }
            _ => Err(DfmcpError::new(
                ErrorCode::InvalidIntent,
                "semantic room completion requires the complete retained room source",
            )),
        }
    }

    fn is_resource_acquisition(&self) -> bool {
        matches!(self, Self::ResourceAcquisitionContinuation { .. })
            || matches!(self, Self::Blueprint { raw, .. } if resource_acquisition::is_request(raw))
    }

    fn is_resource_aware_production(&self) -> bool {
        match self {
            Self::Production { raw, .. } => crate::lab_world::ProductionRequest::parse(raw)
                .is_ok_and(|request| request.is_resource_aware()),
            Self::ProductionContinuation { raw, .. } => {
                goal_continuation::ProductionContinuation::parse(raw)
                    .is_ok_and(|source| source.is_resource_aware())
            }
            _ => false,
        }
    }

    /// Only new /6 pursuits admit a qualified future-reserve deficit while
    /// current stock minima hold. Historical /3 sources keep their semantics.
    fn is_consumption_reserve_continuation(&self) -> bool {
        matches!(self, Self::ProductionContinuation { raw, .. }
            if goal_continuation::ProductionContinuation::parse(raw)
                .is_ok_and(|source| source.is_current_consumption_reserve()))
    }

    /// These compilers select a current stage from the complete supply, stock,
    /// staffing and work census. A narrow action witness cannot replay them.
    fn requires_complete_source_review(&self) -> bool {
        self.is_resource_acquisition()
            || self.is_resource_aware_production()
            || self.is_resource_aware_room()
            || self.is_consumption_reserve_continuation()
    }

    fn resource_replan_step(&self, session_id: SessionId) -> Option<serde_json::Value> {
        if !self.requires_complete_source_review() {
            return None;
        }
        let (summary, blueprint) = match self {
            Self::Blueprint { summary, raw }
            | Self::Production { summary, raw }
            | Self::ConnectedRoom { summary, raw } => (summary, raw.clone()),
            Self::RoomContinuation { summary, raw } => {
                let source = room_continuation::RoomContinuation::parse(raw).ok()?;
                (
                    summary,
                    json!({"template":"continue_goal", "plan_digest":source.parent.to_hex()})
                        .to_string(),
                )
            }
            Self::ConnectedRoomContinuation { summary, raw } => {
                let source =
                    connected_room_continuation::ConnectedRoomContinuation::parse(raw).ok()?;
                (
                    summary,
                    json!({"template":"continue_goal", "plan_digest":source.parent.to_hex()})
                        .to_string(),
                )
            }
            Self::ProductionContinuation { summary, raw } => {
                let source = goal_continuation::ProductionContinuation::parse(raw).ok()?;
                (
                    summary,
                    json!({
                        "template": "continue_goal",
                        "plan_digest": source.parent.to_hex(),
                    })
                    .to_string(),
                )
            }
            Self::ResourceAcquisitionContinuation { summary, raw } => {
                let source =
                    resource_acquisition_continuation::ResourceAcquisitionContinuation::parse(raw)
                        .ok()?;
                (
                    summary,
                    json!({
                        "template": "continue_goal",
                        "plan_digest": source.parent.to_hex(),
                    })
                    .to_string(),
                )
            }
            _ => return None,
        };
        Some(json!({
            "tool": "fortress.plan",
            "arguments": {
                "session_id": session_id.to_string(),
                "summary": summary,
                "blueprint": blueprint,
            },
        }))
    }

    fn durable(&self) -> dfmcp_lab::durable::DurablePlanSource {
        use dfmcp_lab::durable::DurablePlanSource as D;
        match self {
            Self::Pause {
                summary,
                paused_target,
            } => D::Pause {
                summary: summary.clone(),
                paused: *paused_target,
            },
            Self::Actions { summary, raw } => D::Actions {
                summary: summary.clone(),
                raw: raw.clone(),
            },
            Self::Blueprint { summary, raw } => D::Blueprint {
                summary: summary.clone(),
                raw: raw.clone(),
            },
            Self::Production { summary, raw } => D::Production {
                summary: summary.clone(),
                raw: raw.clone(),
            },
            Self::ProductionContinuation { summary, raw } => D::ProductionContinuation {
                summary: summary.clone(),
                raw: raw.clone(),
            },
            Self::RoomContinuation { summary, raw } => D::RoomContinuation {
                summary: summary.clone(),
                raw: raw.clone(),
            },
            Self::CivilianRelease { summary, raw } => D::CivilianRelease {
                summary: summary.clone(),
                raw: raw.clone(),
            },
            Self::ConnectedRoom { summary, raw } => D::ConnectedRoom {
                summary: summary.clone(),
                raw: raw.clone(),
            },
            Self::ConnectedRoomContinuation { summary, raw } => D::ConnectedRoomContinuation {
                summary: summary.clone(),
                raw: raw.clone(),
            },
            Self::ResourceAcquisitionContinuation { summary, raw } => {
                D::ResourceAcquisitionContinuation {
                    summary: summary.clone(),
                    raw: raw.clone(),
                }
            }
        }
    }

    fn from_durable(source: &dfmcp_lab::durable::DurablePlanSource) -> Self {
        use dfmcp_lab::durable::DurablePlanSource as D;
        match source {
            D::Pause { summary, paused } => Self::Pause {
                summary: summary.clone(),
                paused_target: *paused,
            },
            D::Actions { summary, raw } => Self::Actions {
                summary: summary.clone(),
                raw: raw.clone(),
            },
            D::Blueprint { summary, raw } => Self::Blueprint {
                summary: summary.clone(),
                raw: raw.clone(),
            },
            D::Production { summary, raw } => Self::Production {
                summary: summary.clone(),
                raw: raw.clone(),
            },
            D::ProductionContinuation { summary, raw } => Self::ProductionContinuation {
                summary: summary.clone(),
                raw: raw.clone(),
            },
            D::RoomContinuation { summary, raw } => Self::RoomContinuation {
                summary: summary.clone(),
                raw: raw.clone(),
            },
            D::CivilianRelease { summary, raw } => Self::CivilianRelease {
                summary: summary.clone(),
                raw: raw.clone(),
            },
            D::ConnectedRoom { summary, raw } => Self::ConnectedRoom {
                summary: summary.clone(),
                raw: raw.clone(),
            },
            D::ConnectedRoomContinuation { summary, raw } => Self::ConnectedRoomContinuation {
                summary: summary.clone(),
                raw: raw.clone(),
            },
            D::ResourceAcquisitionContinuation { summary, raw } => {
                Self::ResourceAcquisitionContinuation {
                    summary: summary.clone(),
                    raw: raw.clone(),
                }
            }
        }
    }

    fn intent(&self, id: IntentId, snapshot: &WorldSnapshot) -> Result<Intent> {
        self.compile(id, snapshot).map(|(intent, _)| intent)
    }

    fn forecast(
        &self,
        adapter: &MemoryAdapter,
        plan: &PreparedPlan,
        context: &OperationContext,
    ) -> serde_json::Value {
        let reserve_until = match self {
            Self::Production { raw, .. } => crate::lab_world::ProductionRequest::parse(raw)
                .map(|request| request.reserve_until_tick()),
            Self::ProductionContinuation { raw, .. } => {
                goal_continuation::ProductionContinuation::parse(raw)
                    .map(|source| source.reserve_until_tick())
            }
            _ => Ok(None),
        };
        match reserve_until {
            Ok(Some(until)) => {
                plan_forecast::forecast_plan_with_reserve(adapter, plan, context, Some(until))
            }
            Ok(None) => forecast_plan(adapter, plan, context),
            Err(error) => json!({
                "epistemic_state": "predicted",
                "available": false,
                "reason": {"code": error.code.as_str(), "message": error.message},
            }),
        }
    }

    fn continuation(&self) -> Result<Option<GoalContinuation>> {
        match self {
            Self::ProductionContinuation { raw, .. } => {
                goal_continuation::ProductionContinuation::parse(raw)
                    .map(|source| Some(GoalContinuation::Production(source)))
            }
            Self::RoomContinuation { raw, .. } => room_continuation::RoomContinuation::parse(raw)
                .map(|source| Some(GoalContinuation::Room(source))),
            Self::ConnectedRoomContinuation { raw, .. } => {
                connected_room_continuation::ConnectedRoomContinuation::parse(raw)
                    .map(|source| Some(GoalContinuation::ConnectedRoom(source)))
            }
            Self::ResourceAcquisitionContinuation { raw, .. } => {
                resource_acquisition_continuation::ResourceAcquisitionContinuation::parse(raw)
                    .map(|source| Some(GoalContinuation::ResourceAcquisition(source)))
            }
            _ => Ok(None),
        }
    }

    fn can_continue(&self) -> bool {
        match self {
            Self::Production { .. }
            | Self::ProductionContinuation { .. }
            | Self::RoomContinuation { .. }
            | Self::ConnectedRoom { .. }
            | Self::ConnectedRoomContinuation { .. }
            | Self::ResourceAcquisitionContinuation { .. } => true,
            Self::Blueprint { raw, .. } => {
                room_provisioning::is_provisioned_room(raw) || resource_acquisition::is_request(raw)
            }
            _ => false,
        }
    }

    fn continuation_root(&self, own_digest: Digest32) -> Result<Option<Digest32>> {
        Ok(match self.continuation()? {
            Some(source) => Some(source.root()),
            None => self.can_continue().then_some(own_digest),
        })
    }

    fn continuation_json(&self) -> serde_json::Value {
        match self.continuation() {
            Ok(Some(source)) => source.lineage_json(),
            Ok(None) => serde_json::Value::Null,
            Err(error) => json!({"status": "indeterminate", "reason": error.message}),
        }
    }

    /// Compile the original request and its optional production analysis from
    /// the same snapshot. Every replay takes this path; stored actions are not
    /// substituted for a production objective.
    fn compile(
        &self,
        id: IntentId,
        snapshot: &WorldSnapshot,
    ) -> Result<(Intent, Option<serde_json::Value>)> {
        match self {
            Self::Actions { summary, raw } => {
                Ok((semantic_intent(id, snapshot, summary.clone(), raw)?, None))
            }
            Self::Production { summary, raw } => {
                let request = crate::lab_world::ProductionRequest::parse(raw)?;
                production_source::compile(&request, id, snapshot, summary)
                    .map(|(intent, analysis)| (intent, Some(analysis)))
            }
            Self::ProductionContinuation { summary, raw } => {
                goal_continuation::ProductionContinuation::parse(raw)?
                    .compile(id, snapshot, summary)
                    .map(|(intent, analysis)| (intent, Some(analysis)))
            }
            Self::RoomContinuation { summary, raw } => {
                room_continuation::RoomContinuation::parse(raw)?
                    .compile(id, snapshot, summary)
                    .map(|(intent, analysis)| (intent, Some(analysis)))
            }
            Self::CivilianRelease { summary, raw } => civilian_recovery::ReleaseSource::parse(raw)?
                .compile(id, snapshot, summary)
                .map(|(intent, analysis)| (intent, Some(analysis))),
            Self::ConnectedRoom { summary, raw } => {
                connected_room::ConnectedRoomRequest::parse(raw)?
                    .compile(id, snapshot, summary)
                    .map(|(intent, analysis)| (intent, Some(analysis)))
            }
            Self::ConnectedRoomContinuation { summary, raw } => {
                connected_room_continuation::ConnectedRoomContinuation::parse(raw)?
                    .compile(id, snapshot, summary)
                    .map(|(intent, analysis)| (intent, Some(analysis)))
            }
            Self::ResourceAcquisitionContinuation { summary, raw } => {
                resource_acquisition_continuation::ResourceAcquisitionContinuation::parse(raw)?
                    .compile(id, snapshot, summary)
                    .map(|(intent, analysis)| (intent, Some(analysis)))
            }
            Self::Blueprint { summary, raw } => {
                if resource_acquisition::is_request(raw) {
                    return resource_acquisition::ResourceAcquisitionRequest::parse(raw)?
                        .compile(id, snapshot, summary)
                        .map(|(intent, analysis)| (intent, Some(analysis)));
                }
                if room_provisioning::is_provisioned_room(raw) {
                    return room_provisioning::ProvisionedRoomRequest::parse(raw)?
                        .compile(id, snapshot, summary)
                        .map(|(intent, analysis)| (intent, Some(analysis)));
                }
                let (origin, template) = crate::lab_world::parse_blueprint(raw)?;
                let index = crate::lab_world::spatial_index(snapshot)?;
                let mut intent = dfmcp_intent::BlueprintPlanner
                    .compile_furnished_blueprint_intent(
                        id,
                        snapshot.anchor(),
                        origin,
                        template,
                        &index,
                    )?;
                if !summary.is_empty() {
                    intent.summary = summary.clone();
                }
                Ok((intent, None))
            }
            Self::Pause {
                summary,
                paused_target,
            } => Ok((
                Intent {
                    id,
                    anchor: snapshot.anchor(),
                    summary: summary.clone(),
                    terminal_condition: Predicate::Paused(*paused_target),
                    constraints: vec![Constraint::MaxRisk(RiskTier::Reversible)],
                    requested_actions: vec![RequestedAction {
                        action: Action::Pause {
                            paused: *paused_target,
                        },
                        preconditions: vec![Predicate::Paused(!*paused_target)],
                        postconditions: vec![Predicate::Paused(*paused_target)],
                        compensation: None,
                        obligation: None,
                        depends_on: Vec::new(),
                    }],
                },
                None,
            )),
        }
    }
}

/// MCP_SURFACE.md §Versioning: the seven negotiation items every session
/// records. The laboratory fills the bridge and manifest slots with honest
/// absence markers; the authenticated live plane fills them from the admitted
/// compatibility tuple instead.
#[derive(Clone)]
pub(crate) struct SessionNegotiation {
    mcp_protocol_version: &'static str,
    dfmcp_protocol_version: &'static str,
    schema_catalog_digest: String,
    bridge_protocol_version: &'static str,
    canonical_schema_version: &'static str,
    manifests: &'static str,
    compatibility_level: String,
}

impl SessionNegotiation {
    fn laboratory(compatibility_level: String) -> Self {
        Self {
            mcp_protocol_version: "2026-07-28",
            dfmcp_protocol_version: "dfmcp/0",
            schema_catalog_digest: schema_catalog_digest(),
            bridge_protocol_version: "dfmcp.bridge/v1 (proposed; laboratory has no live bridge)",
            canonical_schema_version: "0.1.0",
            manifests: "absent: deterministic laboratory adapter",
            compatibility_level,
        }
    }

    pub(crate) fn to_json(&self) -> serde_json::Value {
        json!({
            "mcp_protocol_version": self.mcp_protocol_version,
            "dfmcp_protocol_version": self.dfmcp_protocol_version,
            "schema_catalog_digest": self.schema_catalog_digest,
            "bridge_protocol_version": self.bridge_protocol_version,
            "canonical_schema_version": self.canonical_schema_version,
            "manifests": self.manifests,
            "compatibility_level": self.compatibility_level,
        })
    }
}

/// Deterministic digest over the frozen 11-tool schema catalog (SCHEMAS
/// registry: every tool input schema at 0.1.0). Identical for every session
/// and stable across processes for an identical tool registry.
fn schema_catalog_digest() -> String {
    const TOOLS: [&str; 11] = [
        "fortress.cancel",
        "fortress.checkpoint",
        "fortress.commit",
        "fortress.doctor",
        "fortress.explain",
        "fortress.open_session",
        "fortress.observe",
        "fortress.plan",
        "fortress.query",
        "fortress.restore",
        "fortress.wait",
    ];
    let mut catalog = String::from("dfmcp.schema-catalog/1\n");
    for tool in TOOLS {
        catalog.push_str(tool);
        catalog.push_str(":0.1.0\n");
    }
    Digest32::of_bytes(catalog.as_bytes()).to_hex()
}

/// Process-wide session registry, keyed by `SessionId`. Replaces the previous
/// `static LAB` so that two concurrent stdio sessions are independent.
static SESSIONS: LazyLock<Mutex<BTreeMap<SessionId, Arc<Mutex<LabSession>>>>> =
    LazyLock::new(|| Mutex::new(BTreeMap::new()));

/// Counter for minting fresh `SessionId`s.
static NEXT_SESSION_COUNTER: LazyLock<Mutex<u128>> = LazyLock::new(|| Mutex::new(1));

const MAX_LAB_SESSIONS: usize = 1_024;
/// Output-token budget of a laboratory session that requests none.
const LAB_DEFAULT_OUTPUT_TOKENS: u32 = 4_096;
const MAX_LAB_COMMIT_RECEIPTS: usize = 4_096;
/// Committed actions a session may have open (not yet terminal) at once.
const MAX_OPEN_ACTIONS: usize = 1_024;
const MAX_CAPABILITY_REQUESTS: usize = 32;
const MAX_CAPABILITY_NAME_BYTES: usize = 64;
const MAX_RISK_NAME_BYTES: usize = 32;
const MAX_FORTRESS_SELECTOR_BYTES: usize = 20;
const MAX_SUMMARY_BYTES: usize = 4_096;
const MAX_LABEL_BYTES: usize = 256;
const MAX_MODE_BYTES: usize = crate::lab_world::MAX_QUERY_JSON_BYTES;
const U128_HEX_ID_BYTES: usize = 32;
const DIGEST_HEX_BYTES: usize = 64;
const MAX_LAB_BUDGET: WorkBudget = WorkBudget {
    max_wall_millis: 60_000,
    max_game_ticks: 1_000_000,
    max_entities: 1_000_000,
    max_bytes: 16 * 1024 * 1024,
    max_output_tokens: 65_536,
    max_actions: 4_096,
};

fn sessions() -> MutexGuard<'static, BTreeMap<SessionId, Arc<Mutex<LabSession>>>> {
    match SESSIONS.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

/// Number of currently registered sessions (resource-plane diagnostics).
pub(crate) fn active_session_count() -> usize {
    sessions().len()
}

fn next_session_counter() -> Result<u128> {
    let mut counter = match NEXT_SESSION_COUNTER.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    let id = *counter;
    if id == 0 {
        return Err(DfmcpError::new(
            ErrorCode::InternalInvariantViolation,
            "session identifier zero is reserved",
        ));
    }
    *counter = counter.checked_add(1).ok_or_else(|| {
        DfmcpError::new(
            ErrorCode::BudgetExceeded,
            "process-local session identifier space is exhausted",
        )
    })?;
    Ok(id)
}

pub(crate) fn parse_session_id_arg(value: &str) -> Result<SessionId> {
    if value.len() != U128_HEX_ID_BYTES || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(DfmcpError::new(
            ErrorCode::InvalidRequest,
            "session_id must be the 32-character hexadecimal identifier returned by fortress_open_session",
        ));
    }
    let parsed = u128::from_str_radix(value, 16).map_err(|_| {
        DfmcpError::new(
            ErrorCode::InvalidRequest,
            "session_id is not a valid hexadecimal u128 identifier",
        )
    })?;
    if parsed == 0 {
        return Err(DfmcpError::new(
            ErrorCode::InvalidRequest,
            "session_id zero is reserved",
        ));
    }
    Ok(SessionId::new(parsed))
}

pub(crate) fn lookup_session(session_id: SessionId) -> Result<Arc<Mutex<LabSession>>> {
    let guard = sessions();
    guard.get(&session_id).cloned().ok_or_else(|| {
        DfmcpError::new(
            ErrorCode::SessionNotFound,
            "no open session with the supplied session_id; call fortress_open_session first",
        )
    })
}

/// Look up a session by its hexadecimal identifier without durable fencing
/// (diagnostic reads such as replay recording and export).
pub(crate) fn lookup_session_str(session_id: &str) -> Result<Arc<Mutex<LabSession>>> {
    lookup_session(parse_session_id_arg(session_id)?)
}

/// The replay bundle of a session's recorded calls.
#[cfg(test)]
pub(crate) fn replay_bundle_for(session_id: &str) -> Result<serde_json::Value> {
    let session = lookup_session_str(session_id)?;
    let guard = session
        .lock()
        .map_err(|_| DfmcpError::new(ErrorCode::InternalInvariantViolation, "session poisoned"))?;
    Ok(crate::replay::bundle_json(&guard.replay))
}

/// Resource callers already hold the current, owner-fenced read view. Keep
/// authorization and replay inspection within that single admission boundary.
pub(crate) fn resource_replay_bundle(session: &LabSession) -> serde_json::Value {
    crate::replay::bundle_json(&session.replay)
}

pub(crate) fn resolve_session(session_id: Option<String>) -> Result<Arc<Mutex<LabSession>>> {
    if let Some(id_str) = session_id {
        let parsed = parse_session_id_arg(&id_str)?;
        let session = lookup_session(parsed)?;
        ensure_durable_owner(&session)?;
        Ok(session)
    } else {
        Err(DfmcpError::new(
            ErrorCode::InvalidRequest,
            "session_id is required; call fortress_open_session first and pass its returned identifier",
        ))
    }
}

fn next_request_id(session: &mut LabSession) -> Result<u128> {
    let next = session.next_request_id.checked_add(1).ok_or_else(|| {
        DfmcpError::new(
            ErrorCode::BudgetExceeded,
            "session request identifier space is exhausted",
        )
    })?;
    session.next_request_id = next;
    Ok(next)
}

fn validate_lab_budget(budget: WorkBudget) -> Result<()> {
    budget.validate()?;
    if budget.max_wall_millis > MAX_LAB_BUDGET.max_wall_millis
        || budget.max_game_ticks > MAX_LAB_BUDGET.max_game_ticks
        || budget.max_entities > MAX_LAB_BUDGET.max_entities
        || budget.max_bytes > MAX_LAB_BUDGET.max_bytes
        || budget.max_output_tokens > MAX_LAB_BUDGET.max_output_tokens
        || budget.max_actions > MAX_LAB_BUDGET.max_actions
    {
        return Err(DfmcpError::new(
            ErrorCode::BudgetExceeded,
            "requested work budget exceeds the process-local laboratory ceiling",
        ));
    }
    Ok(())
}

#[cfg(test)]
fn seed_snapshot(fortress_id: FortressId, paused: bool) -> WorldSnapshot {
    WorldSnapshot::new(
        fortress_id,
        dfmcp_core::GameTick(1),
        dfmcp_core::ObservationCursor::ORIGIN,
        paused,
        dfmcp_world::WorldGraph::default(),
    )
}

/// Build the per-request `OperationContext` from the **session's** negotiated
/// grants and budget. This is the gate that keeps transport identity from
/// granting authority: nothing here reads from `McpContext` or transport
/// state. Authority is exactly what `fortress_open_session` returned.
fn context_for(session: &LabSession, request_id: u128) -> OperationContext {
    OperationContext {
        session_id: session.session_id,
        request_id: RequestId::new(request_id),
        anchor: session.adapter.snapshot().anchor(),
        budget: session.budget,
        grants: session.grants.clone(),
        cancellation_requested: false,
    }
}

pub(crate) fn next_context(session: &mut LabSession) -> Result<(u128, OperationContext)> {
    let request_id = next_request_id(session)?;
    Ok((request_id, context_for(session, request_id)))
}

/// Resource inspection uses current authority without consuming a semantic call ID.
pub(crate) fn read_context(session: &LabSession) -> OperationContext {
    context_for(session, session.next_request_id)
}

/// Entry-level authorization gate (CAPABILITIES.md enforcement point: MCP
/// intake). Deterministic denial BEFORE any state is read or any effect is
/// prepared. `ctx` carries exactly the session's negotiated grants —
/// transport identity grants nothing — and the adapter re-authorizes at the
/// effect boundary.
pub(crate) fn authorize_entry(
    ctx: &OperationContext,
    capability: Capability,
    risk: RiskTier,
) -> Result<()> {
    ctx.authorize(capability, risk, &[], None)
}

/// A bounded wait that moves the simulation is a clock effect. Its clock
/// authority and ability to observe the result must cover the complete span.
fn authorize_wait_advance(
    ctx: &OperationContext,
    requested_ticks: u64,
    paused: bool,
) -> Result<()> {
    if requested_ticks == 0 {
        return Ok(());
    }
    authorize_entry(ctx, Capability::ControlClock, RiskTier::Reversible)?;
    if !paused {
        let mut final_context = ctx.clone();
        final_context.anchor.tick = GameTick(
            ctx.anchor
                .tick
                .0
                .checked_add(requested_ticks)
                .ok_or_else(|| {
                    DfmcpError::new(
                        ErrorCode::BudgetExceeded,
                        "wait exceeds the game-time horizon",
                    )
                })?,
        );
        authorize_entry(
            &final_context,
            Capability::ControlClock,
            RiskTier::Reversible,
        )?;
        authorize_entry(&final_context, Capability::Observe, RiskTier::ReadOnly)?;
    }
    Ok(())
}

pub(crate) fn anchor_json(anchor: &StateAnchor) -> serde_json::Value {
    json!({
        "fortress_id": format!("{}", anchor.fortress_id),
        "game_tick": anchor.tick.0,
        "epoch": anchor.cursor.epoch,
        "sequence": anchor.cursor.sequence,
        "state_hash": anchor.state_hash.to_string(),
    })
}

fn error_payload(operation: &str, message: &str) -> String {
    coded_error_payload(operation, ErrorCode::InvalidRequest, message)
}

pub(crate) fn coded_error_payload(operation: &str, code: ErrorCode, message: &str) -> String {
    json!({
        "ok": false,
        "error": {
            "operation": operation,
            "code": code.as_str(),
            "message": message,
            "retryable": false,
            "details": [],
        },
    })
    .to_string()
}

pub(crate) fn dfmcp_error_payload(operation: &str, error: &DfmcpError) -> String {
    json!({
        "ok": false,
        "error": {
            "operation": operation,
            "code": error.code.as_str(),
            "message": error.message,
            "retryable": error.retryable,
            "details": error.details,
        },
    })
    .to_string()
}

pub(crate) fn mutex_poisoned_payload(operation: &str) -> String {
    coded_error_payload(
        operation,
        ErrorCode::InternalInvariantViolation,
        "session mutex was poisoned; the session cannot be used safely",
    )
}

pub(crate) fn snapshot_json(snapshot: &WorldSnapshot) -> serde_json::Value {
    json!({
        "ok": true,
        "fortress_id": format!("{}", snapshot.fortress_id),
        "game_tick": snapshot.tick.0,
        "cursor": {
            "epoch": snapshot.cursor.epoch,
            "sequence": snapshot.cursor.sequence,
        },
        "paused": snapshot.paused,
        "state_hash": snapshot.state_hash.to_string(),
    })
}

/// Negotiate a `CapabilityGrant` list from requested capability strings and
/// their ceiling risk tiers. Each requested capability becomes one grant over
/// the session's fortress selector. The returned list is exactly what gets
/// installed on the session — no defaults, no extras.
fn negotiate_grants(
    fortress_id: FortressId,
    requested: &[NegotiatedCapability],
) -> Vec<CapabilityGrant> {
    requested
        .iter()
        .map(|req| CapabilityGrant {
            capability: req.capability,
            scope: CapabilityScope {
                fortress_id: Some(fortress_id),
                ..CapabilityScope::default()
            },
            max_risk: req.max_risk,
            expires_at_tick: None,
            remaining_uses: None,
        })
        .collect()
}

fn parse_capability_request(requested: &[(String, String)]) -> Result<Vec<NegotiatedCapability>> {
    if requested.len() > MAX_CAPABILITY_REQUESTS {
        return Err(DfmcpError::new(
            ErrorCode::BudgetExceeded,
            "requested capability count exceeds the explicit session bound",
        ));
    }
    let mut out = Vec::with_capacity(requested.len());
    let mut seen: BTreeSet<Capability> = BTreeSet::new();
    for (cap_str, risk_str) in requested {
        if cap_str.len() > MAX_CAPABILITY_NAME_BYTES || risk_str.len() > MAX_RISK_NAME_BYTES {
            return Err(DfmcpError::new(
                ErrorCode::BudgetExceeded,
                "capability or risk name exceeds its explicit byte bound",
            ));
        }
        let capability = match cap_str.as_str() {
            "observe" => Capability::Observe,
            "query" => Capability::Query,
            "plan" => Capability::Plan,
            "designate" => Capability::Designate,
            "construct" => Capability::Construct,
            "configure_labor" => Capability::ConfigureLabor,
            "configure_production" => Capability::ConfigureProduction,
            "configure_logistics" => Capability::ConfigureLogistics,
            "configure_military" => Capability::ConfigureMilitary,
            "control_clock" => Capability::ControlClock,
            "checkpoint" => Capability::Checkpoint,
            "restore" => Capability::Restore,
            "extension" => Capability::Extension,
            "diagnostic_raw" => Capability::DiagnosticRaw,
            "doctor" => Capability::Doctor,
            "repair_plan" => Capability::RepairPlan,
            "repair_apply" => Capability::RepairApply,
            "admin" => Capability::Admin,
            other => {
                return Err(DfmcpError::new(
                    ErrorCode::InvalidRequest,
                    format!("unsupported capability {other:?}; check the registry"),
                ));
            }
        };
        if !matches!(
            capability,
            Capability::Observe
                | Capability::Query
                | Capability::Plan
                | Capability::Designate
                | Capability::Construct
                | Capability::ConfigureLabor
                | Capability::ConfigureProduction
                | Capability::ConfigureLogistics
                | Capability::ConfigureMilitary
                | Capability::ControlClock
                | Capability::Checkpoint
                | Capability::Restore
                | Capability::Doctor
        ) {
            return Err(DfmcpError::new(
                ErrorCode::CompatibilityUnknown,
                format!(
                    "capability {cap_str:?} is not implemented by the process-local laboratory"
                ),
            ));
        }
        if !seen.insert(capability) {
            return Err(DfmcpError::new(
                ErrorCode::InvalidRequest,
                format!("capability {cap_str} requested more than once"),
            ));
        }
        let max_risk = match risk_str.as_str() {
            "read_only" => RiskTier::ReadOnly,
            "reversible" => RiskTier::Reversible,
            "guarded" => RiskTier::Guarded,
            "irreversible" => RiskTier::Irreversible,
            other => {
                return Err(DfmcpError::new(
                    ErrorCode::InvalidRequest,
                    format!("unsupported risk tier {other:?} for {cap_str}"),
                ));
            }
        };
        out.push(NegotiatedCapability {
            capability,
            max_risk,
        });
    }
    Ok(out)
}

// ============================================================================
// fortress.open_session
// ============================================================================

/// Open a laboratory fortress session. Negotiation inputs are the fortress
/// selector, the requested capability set with risk ceilings, and the work
/// budget. The session is independent from every other session in this
/// process; transport identity grants nothing.
#[tool(
    description = "Open a fortress session against the deterministic laboratory adapter. Negotiates a per-session capability set and budget, then returns a session_id for all subsequent tool calls. Transport identity grants nothing; every authority comes from the negotiated grants."
)]
#[allow(clippy::too_many_arguments)]
pub fn fortress_open_session(
    paused: Option<bool>,
    fortress_selector: Option<String>,
    requested_capabilities: Option<Vec<(String, String)>>,
    max_wall_millis: Option<u64>,
    max_game_ticks: Option<u64>,
    max_entities: Option<u32>,
    max_bytes: Option<u64>,
    max_output_tokens: Option<u32>,
    max_actions: Option<u32>,
) -> String {
    open_session_in_scenario(
        paused,
        fortress_selector,
        requested_capabilities,
        max_wall_millis,
        max_game_ticks,
        max_entities,
        max_bytes,
        max_output_tokens,
        max_actions,
        None,
        None,
        None,
    )
}

/// Open a laboratory session seeded from a named scenario (`empty` by default;
/// `starter_fortress` provides rock, a hall, dwarves, a stockpile, a burrow
/// and a squad for exercising every action family).
#[allow(clippy::too_many_arguments)]
pub(crate) fn open_session_in_scenario(
    paused: Option<bool>,
    fortress_selector: Option<String>,
    requested_capabilities: Option<Vec<(String, String)>>,
    max_wall_millis: Option<u64>,
    max_game_ticks: Option<u64>,
    max_entities: Option<u32>,
    max_bytes: Option<u64>,
    max_output_tokens: Option<u32>,
    max_actions: Option<u32>,
    scenario: Option<String>,
    shared: Option<bool>,
    durable: Option<bool>,
) -> String {
    let shared = shared.unwrap_or(false);
    let durable = durable.unwrap_or(false);
    let scenario_requested = scenario.is_some();
    let mut scenario = scenario.unwrap_or_else(|| "empty".to_owned());
    if scenario.len() > 64 {
        return coded_error_payload(
            "fortress.open_session",
            ErrorCode::BudgetExceeded,
            "scenario name exceeds its explicit byte bound",
        );
    }
    let paused = paused.is_none_or(|value| value);
    let selector_str = fortress_selector.map_or_else(|| "1".to_owned(), |value| value);
    if selector_str.len() > MAX_FORTRESS_SELECTOR_BYTES {
        return coded_error_payload(
            "fortress.open_session",
            ErrorCode::BudgetExceeded,
            "fortress_selector exceeds the maximum decimal u64 length",
        );
    }
    let parsed_fortress: Result<FortressId> = match selector_str.parse::<u64>() {
        Ok(value) => Ok(FortressId::new(value)),
        Err(_) => Err(DfmcpError::new(
            ErrorCode::InvalidRequest,
            "fortress_selector must be a u64 decimal string",
        )),
    };
    let fortress_id = match parsed_fortress {
        Ok(value) if value != FortressId::NIL => value,
        Ok(_) => {
            return error_payload(
                "fortress.open_session",
                "fortress_selector zero is reserved",
            );
        }
        Err(error) => return dfmcp_error_payload("fortress.open_session", &error),
    };

    let default_caps = vec![
        ("observe".to_owned(), "read_only".to_owned()),
        ("query".to_owned(), "read_only".to_owned()),
        ("plan".to_owned(), "reversible".to_owned()),
        ("control_clock".to_owned(), "reversible".to_owned()),
        ("checkpoint".to_owned(), "guarded".to_owned()),
        ("restore".to_owned(), "guarded".to_owned()),
        ("doctor".to_owned(), "read_only".to_owned()),
    ];
    let requested_caps_raw = requested_capabilities.unwrap_or(default_caps);
    let requested_caps = match parse_capability_request(&requested_caps_raw) {
        Ok(value) => value,
        Err(error) => return dfmcp_error_payload("fortress.open_session", &error),
    };

    let budget = WorkBudget {
        max_wall_millis: max_wall_millis
            .map_or(WorkBudget::CONSERVATIVE_DEFAULT.max_wall_millis, |value| {
                value
            }),
        max_game_ticks: max_game_ticks
            .map_or(WorkBudget::CONSERVATIVE_DEFAULT.max_game_ticks, |value| {
                value
            }),
        max_entities: max_entities
            .map_or(WorkBudget::CONSERVATIVE_DEFAULT.max_entities, |value| value),
        max_bytes: max_bytes.map_or(WorkBudget::CONSERVATIVE_DEFAULT.max_bytes, |value| value),
        // The laboratory's Agent Turn is rich; 1,500 tokens (the conservative
        // live default) cannot carry it with plan detail, so lab sessions
        // default to 4,096 unless the caller negotiates otherwise.
        max_output_tokens: max_output_tokens.map_or(LAB_DEFAULT_OUTPUT_TOKENS, |value| value),
        max_actions: max_actions
            .map_or(WorkBudget::CONSERVATIVE_DEFAULT.max_actions, |value| value),
    };
    if let Err(error) = validate_lab_budget(budget) {
        return dfmcp_error_payload("fortress.open_session", &error);
    }

    let grants = negotiate_grants(fortress_id, &requested_caps);
    let seed = match crate::lab_world::scenario_snapshot(&scenario, fortress_id, paused) {
        Ok(snapshot) => snapshot,
        Err(error) => return dfmcp_error_payload("fortress.open_session", &error),
    };
    let fresh = MemoryAdapter::new(seed);
    let _admission = match LAB_ADMISSION.write() {
        Ok(guard) => guard,
        Err(_) => return mutex_poisoned_payload("fortress.open_session"),
    };
    // Exclude in-flight publication and concurrent opens while changing
    // ownership. Private and shared writers cannot target the same store.
    if durable {
        if shared {
            if durable_lab().owners.contains_key(&fortress_id) {
                return coded_error_payload(
                    "fortress.open_session",
                    ErrorCode::Conflict,
                    "this durable fortress has a private owner; restart the server before changing it to shared mode",
                );
            }
        } else {
            let shared_exists = match SHARED_WORLDS.lock() {
                Ok(registry) => registry.contains_key(&fortress_id),
                Err(_) => return mutex_poisoned_payload("fortress.open_session"),
            };
            if shared_exists {
                return coded_error_payload(
                    "fortress.open_session",
                    ErrorCode::Conflict,
                    "this fortress has a shared owner; restart the server before changing it to private durable mode",
                );
            }
            let previous_owner = durable_lab().owners.get(&fortress_id).copied();
            if let Some(previous) = previous_owner.and_then(|id| lookup_session(id).ok()) {
                let fault = with_session_admitted(
                    &previous,
                    || Some("previous session unavailable".to_owned()),
                    |guard| {
                        persist_durable_head(guard);
                        guard.durability_fault.clone()
                    },
                );
                if let Some(fault) = fault {
                    return coded_error_payload(
                        "fortress.open_session",
                        ErrorCode::AdapterUnavailable,
                        &format!(
                            "the current owner has unpublished durable state: {fault}; recover persistence before replacing it"
                        ),
                    );
                }
            }
        }
    } else if shared && durable_lab().owners.contains_key(&fortress_id) {
        return coded_error_payload(
            "fortress.open_session",
            ErrorCode::Conflict,
            "this fortress has a private durable owner; restart the server before opening a shared world with that selector",
        );
    }
    if sessions().len() >= MAX_LAB_SESSIONS {
        return coded_error_payload(
            "fortress.open_session",
            ErrorCode::BudgetExceeded,
            "process-local laboratory reached its explicit session bound",
        );
    }
    let session_counter = match next_session_counter() {
        Ok(value) => value,
        Err(error) => return dfmcp_error_payload("fortress.open_session", &error),
    };
    let session_id = SessionId::new(session_counter);
    let (adapter, recovery, shared_world, shared_view) = if shared {
        match join_shared_world(
            fortress_id,
            session_id,
            &mut scenario,
            scenario_requested,
            &fresh,
            durable,
        ) {
            Ok(admission) => (
                admission.adapter,
                admission.recovery,
                Some(admission.world),
                Some(admission.view),
            ),
            Err(error) => return dfmcp_error_payload("fortress.open_session", &error),
        }
    } else if durable {
        match load_durable_fortress(fortress_id, &mut scenario, scenario_requested, &fresh) {
            Ok((adapter, recovery)) => (adapter, Some(recovery), None, None),
            Err(error) => return dfmcp_error_payload("fortress.open_session", &error),
        }
    } else {
        (fresh, None, None, None)
    };
    let identity = adapter.identity();
    let negotiation = SessionNegotiation::laboratory(format!("{:?}", identity.compatibility));
    let (snapshot_anchor, paused_after) = shared_view.as_ref().map_or_else(
        || (adapter.snapshot().anchor(), adapter.snapshot().paused),
        |view| (view.anchor, view.paused),
    );
    let session = Arc::new(Mutex::new(LabSession {
        session_id,
        _query_resources: temporal_query::SessionGuard::new(session_id),
        fortress_id,
        grants,
        budget,
        negotiation: negotiation.clone(),
        next_request_id: 0,
        adapter,
        pending: None,
        last_action: None,
        last_plan_actions: Vec::new(),
        open_actions: Vec::new(),
        commit_receipts: BTreeMap::new(),
        commit_authority: BTreeMap::new(),
        shared: shared_world,
        leases: LeaseBook::default(),
        shared_members: 0,
        durable_scenario: durable.then(|| scenario.clone()),
        durability_fault: None,
        durable_plans: BTreeMap::new(),
        durable_restore: BTreeMap::new(),
        carried: if shared {
            Vec::new()
        } else {
            recovery
                .as_ref()
                .map_or_else(Vec::new, |recovery| recovery.carried.clone())
        },
        replay: {
            let mut log = crate::replay::ReplayLog::default();
            if shared {
                log.mark_not_replayable("the session shares its fortress with other agents");
            }
            if durable {
                log.mark_not_replayable(
                    "the session's fortress is durable and may resume external state",
                );
            }
            log
        },
        history: new_history(),
        objectives: if shared {
            Vec::new()
        } else {
            recovery
                .as_ref()
                .map_or_else(Vec::new, |recovery| recovery.objectives.clone())
        },
        objective_restore: BTreeSet::new(),
        civilian_recovery: civilian_recovery::CivilianRecovery::default(),
    }));
    {
        let mut registry = sessions();
        if registry.len() >= MAX_LAB_SESSIONS {
            return coded_error_payload(
                "fortress.open_session",
                ErrorCode::BudgetExceeded,
                "process-local laboratory reached its explicit session bound",
            );
        }
        if registry.contains_key(&session_id) {
            return coded_error_payload(
                "fortress.open_session",
                ErrorCode::InternalInvariantViolation,
                "fresh session identifier unexpectedly collided with an existing session",
            );
        }
        registry.insert(session_id, session.clone());
    }
    // Retain the opening world version so the first turn that changes it can
    // say exactly what changed.
    with_session_admitted(&session, || (), |_| ());
    if durable {
        if !shared {
            durable_lab().owners.insert(fortress_id, session_id);
        }
        // Persist the opening state (a resumed world's new epoch included) now,
        // so a crash before the first state change still resumes it.
        let fault = with_session_admitted(
            &session,
            || Some("session poisoned".to_owned()),
            |guard| {
                persist_durable_head(guard);
                guard.durability_fault.clone()
            },
        );
        if let Some(fault) = fault {
            if shared {
                // Preserve the world and its pending frontier, but do not
                // leave an unreachable member blocking unanimous unpause.
                sessions().remove(&session_id);
                let world = session.lock().ok().and_then(|guard| guard.shared.clone());
                if let Some(world) = world
                    && let Ok(mut guard) = world.lock()
                {
                    guard.members.remove(&session_id);
                    guard.leases.unpause_consent.remove(&session_id);
                }
            }
            return coded_error_payload(
                "fortress.open_session",
                ErrorCode::AdapterUnavailable,
                &format!("durable fortress could not be persisted: {fault}"),
            );
        }
    }
    let granted_strings: Vec<&str> = requested_caps
        .iter()
        .map(|c| c.capability.as_str())
        .collect();
    let (untracked, goal_status, workforce, construction_materials, provisions, harvesting) =
        with_session_admitted(
            &session,
            || {
                (
                    json!({"state": "unknown", "quiescent": false, "items": null,
            "reason": "session observation unavailable"}),
                    serde_json::Value::Null,
                    None,
                    None,
                    None,
                    None,
                )
            },
            |guard| {
                (
                    untracked_work_json(guard),
                    objectives_json(guard),
                    session_workforce_json(guard),
                    session_construction_material_json(guard),
                    session_provisions_json(guard),
                    session_harvest_json(guard),
                )
            },
        );
    let mut payload = json!({
        "ok": true,
        "session_id": format!("{session_id}"),
        "adapter": identity.name,
        "compatibility": format!("{:?}", identity.compatibility),
        "fortress_loaded": true,
        "fortress_id": format!("{fortress_id}"),
        "granted_capabilities": granted_strings,
        "untracked_work": untracked,
        "objectives": goal_status,
        "negotiation": negotiation.to_json(),
        "budget": {
            "max_wall_millis": budget.max_wall_millis,
            "max_game_ticks": budget.max_game_ticks,
            "max_entities": budget.max_entities,
            "max_bytes": budget.max_bytes,
            "max_output_tokens": budget.max_output_tokens,
            "max_actions": budget.max_actions,
        },
        "anchor": anchor_json(&snapshot_anchor),
        "paused": paused_after,
        "scenario": shared_view.as_ref().map_or(scenario.as_str(), |view| view.scenario.as_str()),
        "durable": recovery.as_ref().map(|recovery| json!({
            "resumed": recovery.resumed,
            "recovered_from_anchor": recovery.recovered_from.as_ref().map(anchor_json),
            "restorable_checkpoints": recovery.checkpoints,
            "recovered_commits": recovery.commits,
            "carried_obligations": recovery.carried.iter().map(CarriedStep::to_json).collect::<Vec<_>>(),
            "retained_objectives": if goal_status[0]["status"] == "unavailable" {
                None
            } else { Some(recovery.objectives.len()) },
            "torn_tail_bytes_discarded_at_store_open": recovery.torn_tail_bytes,
            "note": if recovery.resumed {
                "resumed the last persisted world in a new observation epoch: existing world work continues on wait; prior action handles and dispatch authority are gone. Recovered proof monitors keep sealed deadlines and require current authorized observations. Older sessions are fenced."
            } else {
                "new crash-durable fortress: every state change and checkpoint is persisted and survives server restarts"
            },
        })).or_else(|| durable.then(|| json!({
            "resumed": false,
            "joined_existing": true,
            "note": "joined the running durable fortress without replaying recovery or changing its observation epoch; durable progress and persistence faults are shared",
        }))),
        "shared_world": shared_view.as_ref().map(|view| json!({
            "fortress_id": format!("{fortress_id}"),
            "members": view.members,
            "joined_existing": view.joined_existing,
            "note": "one canonical world, clock and lease book; your grants, budget, plans and receipts stay your own",
        })),
        "note": "session_id is required for all subsequent tool calls; transport identity grants nothing",
    });
    if let Some(workforce) = workforce {
        payload["workforce"] = workforce;
    }
    if let Some(materials) = construction_materials {
        payload["construction_materials"] = materials;
    }
    if let Some(provisions) = provisions {
        payload["provisions"] = provisions;
    }
    if let Some(harvesting) = harvesting {
        payload["harvesting"] = harvesting;
    }
    payload.to_string()
}

// ============================================================================
// fortress.observe
// ============================================================================

/// Return the current bounded snapshot projection at the laboratory anchor.
#[tool(
    description = "Observe the current fortress state for an open session. Requires the session_id returned by fortress_open_session."
)]
pub fn fortress_observe(session_id: Option<String>) -> String {
    let session = match resolve_session(session_id) {
        Ok(value) => value,
        Err(error) => return dfmcp_error_payload("fortress.observe", &error),
    };
    with_session(
        &session,
        || mutex_poisoned_payload("fortress.observe"),
        |guard| {
            let (_, ctx) = match next_context(guard) {
                Ok(value) => value,
                Err(error) => return dfmcp_error_payload("fortress.observe", &error),
            };
            if let Err(error) = authorize_entry(&ctx, Capability::Observe, RiskTier::ReadOnly) {
                return dfmcp_error_payload("fortress.observe", &error);
            }
            let request = ObservationRequest {
                since: None,
                projection: Projection::Summary,
                interest: InterestSet::default(),
                max_entities: guard.budget.max_entities,
                max_bytes: guard.budget.max_bytes,
                max_output_tokens: guard.budget.max_output_tokens,
                continuation: None,
            };
            match guard.adapter.observe(&request, &ctx) {
                Ok(frame) => match frame.payload {
                    ObservationPayload::Snapshot(snapshot) => {
                        let mut payload = snapshot_json(&snapshot);
                        payload["projection"] = json!("summary");
                        payload["session_id"] = json!(format!("{}", guard.session_id));
                        payload["evidence_count"] = json!(frame.evidence.len());
                        payload["world"] = crate::lab_world::briefing(&snapshot);
                        payload["untracked_work"] = untracked_work_json(guard);
                        payload["world_alerts"] = json!(world_alerts(guard));
                        payload["civilian_recovery"] = guard.civilian_recovery.projection();
                        payload["objectives"] = objectives_json(guard);
                        payload["query_work"] = query_work_json(&ctx);
                        payload.to_string()
                    }
                    ObservationPayload::Delta(_) | ObservationPayload::Heartbeat(_) => {
                        coded_error_payload(
                            "fortress.observe",
                            ErrorCode::InternalInvariantViolation,
                            "full laboratory observation unexpectedly returned a non-snapshot payload",
                        )
                    }
                },
                Err(error) => dfmcp_error_payload("fortress.observe", &error),
            }
        },
    )
}

// ============================================================================
// fortress.query
// ============================================================================

/// Run a bounded laboratory query: `summary`, `entities`, or a JSON object
/// `{"mode":"entities","kind":"unit","limit":25,"offset":0}` /
/// `{"mode":"terrain","min":[x,y,z],"max":[x,y,z]}`.
#[tool(
    description = "Run a bounded laboratory query. The closed structured wrapper {\"mode\":\"structured\",\"request\":{\"schema\":\"dfmcp.query/1\",\"query\":{...}},\"at\"?:state_hash} exposes stateless typed queries under current Query authority. The closed temporal wrapper {mode:temporal,request:{schema:dfmcp.query/1,query:{kind:capture|changes|baselines|release_baseline|watch|poll_watch|poll_watches|watches|cancel_watch|release_watch,...}}} owns up to eight foreground selected baselines per session; capture uses key, select and max_game_ticks, changes uses baseline with optional limit/continuation, release_baseline uses baseline. Foreground watches use key, condition, optional failure_condition, deadline_tick, optional poll_interval_ticks and stable_observations; poll_watches evaluates one to eight selected handles (or all when omitted) at the same current snapshot with atomic publication. Field/generation, observed-projection entity counts, pause/tick and Boolean conditions use registered laboratory evidence. Polling never advances game time or acquires native observations. Structured condition_evaluation preflights the same predicates without retaining a watch, including exact historical at sources. Complete results and current active work are published atomically before private baseline/watch changes; restart loses handles, and explicit polling invalidates nonterminal watches after restore. The closed query_changes wrapper takes before (exact retained hash), after (optional, defaults to current), select (complete entities selection), limit and continuation; it reports selected endpoint differences without proving intermediate history or mutation evidence. Other mode: \"summary\" (default), \"entities\", or a JSON object {\"mode\":\"entities\",\"kind\":\"unit\",\"limit\":25,\"offset\":0} or {\"mode\":\"terrain\",\"min\":[x,y,z],\"max\":[x,y,z]}. Full DfQL is not implemented."
)]
pub fn fortress_query(session_id: Option<String>, mode: Option<String>) -> String {
    match query_publication(session_id, mode, std::time::Instant::now(), false) {
        QueryPublication::Pending(response) | QueryPublication::Published(response) => response,
        QueryPublication::Projected { .. } => coded_error_payload(
            "fortress.query",
            ErrorCode::InternalInvariantViolation,
            "direct laboratory query unexpectedly deferred page publication",
        ),
    }
}

/// Retained query mutations publish their complete Agent Turn before commit.
/// The outer facade must not fit, replace or refuse that accepted packet later.
pub(crate) enum QueryPublication {
    Pending(String),
    Published(String),
    /// A projected query captures presentation dependencies outside the
    /// session/world locks, then executes once against its exact admitted source.
    /// This provisional orientation is internal and never returned by a tool.
    Projected {
        orientation: String,
        query: Box<PreparedProjectedQuery>,
    },
}

pub(crate) struct PreparedProjectedQuery {
    session: Arc<Mutex<LabSession>>,
    current: OperationContext,
    source: StateAnchor,
    request: structured_query::Request,
}

impl PreparedProjectedQuery {
    pub(crate) fn execute(
        self,
        started: std::time::Instant,
        fits: &dyn Fn(&serde_json::Value) -> Result<bool>,
    ) -> serde_json::Value {
        with_session_read(
            &self.session,
            |error| query_budget::response_payload(Err(error), &self.current, true),
            |guard| {
                let current = context_for(guard, self.current.request_id.get());
                let mut query_work = None;
                let result = authorize_entry(&current, Capability::Query, RiskTier::ReadOnly)
                    .and_then(|()| {
                        if current.anchor != self.current.anchor {
                            return Err(DfmcpError::new(
                                ErrorCode::StaleAnchor,
                                "current world changed while preparing the query packet; retry at the new anchor",
                            ));
                        }
                        if current.budget != self.current.budget {
                            return Err(DfmcpError::new(
                                ErrorCode::BudgetExceeded,
                                "query budget changed while preparing the complete packet",
                            ));
                        }
                        if current.grants != self.current.grants {
                            return Err(DfmcpError::new(
                                ErrorCode::CapabilityDenied,
                                "session authority changed while preparing the query packet",
                            ));
                        }
                        let hash = self.source.state_hash.to_string();
                        let source = retained_resource_snapshot(guard, Some(&hash))?;
                        if source.anchor() != self.source {
                            return Err(DfmcpError::new(
                                ErrorCode::StaleAnchor,
                                "retained query source changed before page execution",
                            ));
                        }
                        // Baselines and watches can change without moving the
                        // world anchor. Capture their current catalog only after
                        // reacquiring this owner, then reuse it for every pure
                        // fit and the final payload. Renderers must not reenter
                        // either retained-state store while this lock is held.
                        let work = query_work_json(&current);
                        let result = structured_query::execute_projected(
                            source,
                            &current,
                            &self.request,
                            started,
                            &|page| {
                                let mut complete = page.clone();
                                complete["query_work"] = work.clone();
                                fits(&complete)
                            },
                        );
                        query_work = Some(work);
                        result
                    });
                let mut payload = query_budget::response_payload(result, &current, true);
                if let Some(work) = query_work {
                    payload["query_work"] = work;
                }
                payload
            },
        )
    }
}

/// Private query state, historical analysis and syntax refusals do not publish
/// durable-world progress. Keep the same admission/owner/world/session fences
/// without persistence or retaining a new world version as a read side effect.
fn with_query_session<T>(
    session: &Arc<Mutex<LabSession>>,
    read_only: bool,
    poisoned: impl FnOnce() -> T,
    body: impl FnOnce(&mut LabSession) -> T,
) -> T {
    if !read_only {
        return with_session(session, poisoned, body);
    }
    let _admission = match LAB_ADMISSION.read() {
        Ok(guard) => guard,
        Err(_) => return poisoned(),
    };
    if ensure_durable_owner(session).is_err() {
        return poisoned();
    }
    with_session_view(session, poisoned, body, false)
}

pub(crate) fn fortress_query_publication(
    session_id: Option<String>,
    mode: Option<String>,
    started: std::time::Instant,
) -> QueryPublication {
    query_publication(session_id, mode, started, true)
}

fn query_publication(
    session_id: Option<String>,
    mode: Option<String>,
    started: std::time::Instant,
    defer_pages: bool,
) -> QueryPublication {
    let mode = mode.map_or_else(|| "summary".to_owned(), |value| value);
    let structured = structured_query::parse(&mode);
    let comparison = query_comparison::parse(&mode);
    let replay = condition_replay::parse(&mode);
    let temporal = temporal_query::parse(&mode);
    let is_structured = matches!(&structured, Ok(Some(_)));
    let is_temporal = matches!(&temporal, Ok(Some(_)));
    let is_replay = matches!(&replay, Ok(Some(_)));
    let invalid_query = structured.is_err() || comparison.is_err() || replay.is_err() || temporal.is_err();
    let session = match resolve_session(session_id) {
        Ok(value) => value,
        Err(error) => {
            return QueryPublication::Pending(dfmcp_error_payload("fortress.query", &error));
        }
    };
    with_query_session(
        &session,
        is_temporal || is_replay || invalid_query,
        || QueryPublication::Pending(mutex_poisoned_payload("fortress.query")),
        |guard| {
            let (_, ctx) = match next_context(guard) {
                Ok(value) => value,
                Err(error) => {
                    return QueryPublication::Pending(dfmcp_error_payload(
                        "fortress.query",
                        &error,
                    ));
                }
            };
            let result =
                authorize_entry(&ctx, Capability::Query, RiskTier::ReadOnly).and_then(|()| {
                    let structured = structured?;
                    let comparison = comparison?;
                    let replay = replay?;
                    let temporal = temporal?;
                    if structured.is_none()
                        && comparison.is_none()
                        && replay.is_none()
                        && temporal.is_none()
                        && mode.len() > MAX_MODE_BYTES
                    {
                        return Err(DfmcpError::new(
                            ErrorCode::BudgetExceeded,
                            "query mode exceeds its explicit byte bound",
                        ));
                    }
                    if let Some(request) = &temporal {
                        let spent_millis = started.elapsed().as_nanos().div_ceil(1_000_000);
                        if spent_millis >= u128::from(ctx.budget.max_wall_millis) {
                            return Err(DfmcpError::new(ErrorCode::BudgetExceeded,
                                "temporal query exhausted its wall-time allowance before owning the session"));
                        }
                        let mut remaining = ctx.clone();
                        remaining.budget.max_wall_millis -= spent_millis as u64;
                        let response = temporal_handler::execute(guard, &remaining, request)?;
                        if matches!(request.kind(), "capture" | "watch") {
                            guard.replay.mark_not_replayable(
                                "query baselines and watches use process-local session handles that cannot be replayed into a new session",
                            );
                        }
                        return Ok(QueryPublication::Published(response));
                    }
                    if let Some(request) = &replay {
                        let spent_millis = started.elapsed().as_nanos().div_ceil(1_000_000);
                        if spent_millis >= u128::from(ctx.budget.max_wall_millis) {
                            return Err(DfmcpError::new(ErrorCode::BudgetExceeded,
                                "condition replay exhausted its wall-time allowance before owning the session"));
                        }
                        let mut remaining = ctx.clone();
                        remaining.budget.max_wall_millis -= spent_millis as u64;
                        let payload = condition_replay::execute(
                            &guard.history, guard.adapter.snapshot(), &remaining, request,
                        )?;
                        return Ok(QueryPublication::Pending(query_budget::response(
                            Ok(payload), &ctx, false,
                        )));
                    }
                    // Select historical state only after authorizing at the current
                    // game tick. Neither an old source nor its cursor renews a grant.
                    let payload = if let Some(request) = &comparison {
                        let before = retained_resource_snapshot(guard, Some(&request.before))?;
                        let after = retained_resource_snapshot(guard, request.after.as_deref())?;
                        query_comparison::execute(before, after, &ctx, request)?
                    } else if let Some(request) = structured {
                        let source = retained_resource_snapshot(guard, request.at.as_deref())?;
                        if defer_pages && request.is_projected_page() {
                            // The session's normal post-call retention and the
                            // facade's change projection also inspect this
                            // source. Bound it before releasing this first lock.
                            query_budget::admit_sources(&[source], &ctx)?;
                            let orientation =
                                structured_query::orientation(source.anchor(), &ctx, &request);
                            return Ok(QueryPublication::Projected {
                                orientation: query_budget::response(Ok(orientation), &ctx, true),
                                query: Box::new(PreparedProjectedQuery {
                                    session: Arc::clone(&session),
                                    current: ctx.clone(),
                                    source: source.anchor(),
                                    request,
                                }),
                            });
                        }
                        structured_query::execute(source, &ctx, &request)?
                    } else {
                        legacy_query(guard, &ctx, &mode)?
                    };
                    Ok(QueryPublication::Pending(query_budget::response(
                        Ok(payload),
                        &ctx,
                        is_structured,
                    )))
                });
            match result {
                Ok(published) => published,
                Err(error) => QueryPublication::Pending(query_budget::response(
                    Err(error),
                    &ctx,
                    is_structured,
                )),
            }
        },
    )
}

/// Legacy query grammar shares source admission and complete-packet publication
/// with structured queries. Limits and profile omissions cannot hide scan work.
fn legacy_query(
    guard: &mut LabSession,
    context: &OperationContext,
    mode: &str,
) -> Result<serde_json::Value> {
    if civilian_recovery::is_recovery_query(mode)? {
        let source_work = query_budget::admit_sources(&[guard.adapter.snapshot()], context)?;
        let mut payload = guard.civilian_recovery.projection();
        payload["ok"] = json!(true);
        payload["session_id"] = json!(guard.session_id.to_string());
        payload["mode"] = json!("civilian_recovery");
        payload["source_work"] = source_work;
        let bytes = payload.to_string().len() as u64;
        if bytes > context.budget.max_bytes
            || bytes > u64::from(context.budget.max_output_tokens).saturating_mul(4)
        {
            return Err(DfmcpError::new(
                ErrorCode::BudgetExceeded,
                "complete civilian recovery census exceeds the negotiated query allowance",
            ));
        }
        return Ok(payload);
    }
    if mode.trim_start().starts_with('{')
        && let Some(payload) = historical_query(guard, context, mode)?
    {
        return Ok(payload);
    }
    let snapshot = guard.adapter.snapshot();
    let source_work = query_budget::admit_sources(&[snapshot], context)?;
    if mode != "summary" {
        let mut payload = crate::lab_world::query(snapshot, mode)?;
        crate::resources::link_entity_resources(&mut payload, guard.session_id, snapshot.anchor())?;
        payload["ok"] = json!(true);
        payload["session_id"] = json!(guard.session_id.to_string());
        payload["anchor"] = anchor_json(&snapshot.anchor());
        payload["game_tick"] = json!(snapshot.tick.0);
        payload["source_work"] = source_work;
        return Ok(payload);
    }
    let request = QueryRequest {
        anchor: context.anchor,
        query: WorldQuery {
            kinds: Vec::new(),
            predicate: None,
            order: QueryOrder::EntityIdAscending,
            limit: context.budget.max_entities,
            continuation: None,
        },
        max_output_tokens: context.budget.max_output_tokens,
        continuation: None,
    };
    let response = guard.adapter.query(&request, context)?;
    let mut payload = snapshot_json(guard.adapter.snapshot());
    payload["matched"] = json!(response.matched);
    payload["returned"] = json!(response.rows.len());
    payload["truncated"] = json!(response.truncated);
    payload["continuation"] = json!(response.continuation);
    payload["session_id"] = json!(guard.session_id.to_string());
    payload["source_work"] = source_work;
    Ok(payload)
}

/// Queries over retained world versions: `{"mode":"changes","since":H}`
/// reports observed changes from version `H` to now, and any entities or
/// terrain query with `"at":H` reads version `H` exactly. `Ok(None)` means the
/// request is an ordinary current-state query.
fn historical_query(
    guard: &LabSession,
    context: &OperationContext,
    raw: &str,
) -> Result<Option<serde_json::Value>> {
    let mut spec: serde_json::Value = serde_json::from_str(raw).map_err(|error| {
        DfmcpError::new(
            ErrorCode::InvalidRequest,
            format!("query is not JSON: {error}"),
        )
    })?;
    let retained = |hash: &str| {
        guard
            .history
            .find_hex(hash)
            .and_then(|digest| guard.history.get(&digest))
            .ok_or_else(|| {
                DfmcpError::new(ErrorCode::StaleAnchor, not_retained_reason(guard, hash))
            })
    };
    let current = guard.adapter.snapshot();
    if spec["mode"] == "changes" {
        let since = spec["since"].as_str().ok_or_else(|| {
            DfmcpError::new(
                ErrorCode::InvalidRequest,
                "changes query needs \"since\": a state_hash from an earlier anchor",
            )
        })?;
        let base = retained(since)?;
        let source_work = if base.anchor() == current.anchor() {
            query_budget::admit_sources(&[current], context)?
        } else {
            query_budget::admit_sources(&[base, current], context)?
        };
        let changes = crate::world_changes::describe(base, current);
        return Ok(Some(json!({
            "ok": true,
            "session_id": format!("{}", guard.session_id),
            "mode": "changes",
            "since_anchor": anchor_json(&base.anchor()),
            "anchor": anchor_json(&current.anchor()),
            "changes": changes,
            "source_work": source_work,
            "epistemic_state": "observed",
        })));
    }
    let Some(at) = spec
        .get("at")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
    else {
        return Ok(None);
    };
    if let Some(object) = spec.as_object_mut() {
        object.remove("at");
    }
    let version = if current.state_hash.to_hex() == at {
        current
    } else {
        retained(&at)?
    };
    let source_work = query_budget::admit_sources(&[version], context)?;
    let mut payload = crate::lab_world::query(version, &spec.to_string())?;
    payload["ok"] = json!(true);
    payload["session_id"] = json!(format!("{}", guard.session_id));
    payload["anchor"] = anchor_json(&version.anchor());
    payload["game_tick"] = json!(version.tick.0);
    payload["historical"] = json!(version.state_hash != current.state_hash);
    payload["current_anchor"] = anchor_json(&current.anchor());
    payload["source_work"] = source_work;
    payload["epistemic_state"] = json!("observed");
    Ok(Some(payload))
}

// ============================================================================
// fortress.plan
// ============================================================================

/// Compile a pause/resume intent into an immutable, inspectable plan without effects.
#[tool(
    description = "Compile a laboratory pause/resume intent into an immutable, inspectable plan without effects."
)]
pub fn fortress_plan(
    session_id: Option<String>,
    summary: Option<String>,
    paused_target: Option<bool>,
) -> String {
    plan_with_actions(session_id, summary, paused_target, None)
}

/// Compile either the legacy pause intent or a JSON array of semantic action
/// steps (see `lab_world::parse_steps`). Missing postconditions, obligations
/// and compensations are sealed from the reference action model.
pub(crate) fn plan_with_actions(
    session_id: Option<String>,
    summary: Option<String>,
    paused_target: Option<bool>,
    actions: Option<String>,
) -> String {
    plan_request(session_id, summary, paused_target, actions, None, None)
}

/// `fortress.plan` over every request form: pause/resume, explicit semantic
/// actions, or a blueprint objective the planner decomposes into steps.
/// Candidate selection returns its complete packet before changing pending state.
/// The facade must return a published packet unchanged.
pub(crate) enum PlanPublication {
    Pending(String),
    Published(String),
}

pub(crate) fn plan_request(
    session_id: Option<String>,
    summary: Option<String>,
    paused_target: Option<bool>,
    actions: Option<String>,
    blueprint: Option<String>,
    production: Option<String>,
) -> String {
    match plan_request_publication(
        session_id,
        summary,
        paused_target,
        actions,
        blueprint,
        production,
        std::time::Instant::now(),
    ) {
        PlanPublication::Pending(output) | PlanPublication::Published(output) => output,
    }
}

pub(crate) fn plan_request_publication(
    session_id: Option<String>,
    summary: Option<String>,
    paused_target: Option<bool>,
    actions: Option<String>,
    blueprint: Option<String>,
    production: Option<String>,
    started: std::time::Instant,
) -> PlanPublication {
    if let Some(raw) = blueprint
        .as_deref()
        .filter(|raw| is_resource_aware_room_request(raw))
    {
        if actions.is_some() || production.is_some() || paused_target.is_some() {
            return PlanPublication::Pending(coded_error_payload(
                "fortress.plan",
                ErrorCode::InvalidRequest,
                "resource-aware rooms contain the complete original project; supply the blueprint without actions, production or a pause target",
            ));
        }
        return match plan_comparison_handler::execute_resource_room(
            session_id, summary, raw, started,
        ) {
            Ok(output) => PlanPublication::Published(output),
            Err(error) => PlanPublication::Pending(dfmcp_error_payload("fortress.plan", &error)),
        };
    }
    let staged_production = blueprint
        .as_deref()
        .filter(|raw| is_resource_aware_production_request(raw))
        .or_else(|| {
            production
                .as_deref()
                .filter(|raw| is_resource_aware_production_request(raw))
        });
    if let Some(raw) = staged_production {
        if actions.is_some()
            || paused_target.is_some()
            || (blueprint.is_some() && production.is_some())
        {
            return PlanPublication::Pending(coded_error_payload(
                "fortress.plan",
                ErrorCode::InvalidRequest,
                "resource-aware production contains its complete objective; supply it once without actions or a pause target",
            ));
        }
        return match plan_comparison_handler::execute_resource_production(
            session_id, summary, raw, started,
        ) {
            Ok(output) => PlanPublication::Published(output),
            Err(error) => PlanPublication::Pending(dfmcp_error_payload("fortress.plan", &error)),
        };
    }
    if blueprint
        .as_deref()
        .is_some_and(resource_acquisition::is_request)
    {
        if actions.is_some() || production.is_some() || paused_target.is_some() {
            return PlanPublication::Pending(coded_error_payload(
                "fortress.plan",
                ErrorCode::InvalidRequest,
                "resource acquisition contains its complete objective; do not combine it with actions, production or a pause target",
            ));
        }
        let Some(raw) = blueprint else {
            return PlanPublication::Pending(coded_error_payload(
                "fortress.plan",
                ErrorCode::InternalInvariantViolation,
                "resource acquisition source disappeared before planning",
            ));
        };
        return match plan_comparison_handler::execute_resource_acquisition(
            session_id, summary, &raw, started,
        ) {
            Ok(output) => PlanPublication::Published(output),
            Err(error) => PlanPublication::Pending(dfmcp_error_payload("fortress.plan", &error)),
        };
    }
    if let Some(raw) = blueprint
        .as_deref()
        .filter(|raw| goal_continuation::is_request(raw))
    {
        if actions.is_some() || production.is_some() || paused_target.is_some() {
            return PlanPublication::Pending(coded_error_payload(
                "fortress.plan",
                ErrorCode::InvalidRequest,
                "continue_goal contains its retained objective; do not combine it with actions, production or a pause target",
            ));
        }
        let parent = match goal_continuation::parse_request(raw) {
            Ok(parent) => parent,
            Err(error) => {
                return PlanPublication::Pending(dfmcp_error_payload("fortress.plan", &error));
            }
        };
        match plan_comparison_handler::execute_resource_continuation(
            session_id.clone(),
            summary.clone(),
            parent,
            started,
        ) {
            Ok(Some(output)) => return PlanPublication::Published(output),
            Ok(None) => {}
            Err(error) => {
                return PlanPublication::Pending(dfmcp_error_payload("fortress.plan", &error));
            }
        }
    }
    let parsed = blueprint.as_deref().map(plan_comparison::parse).transpose();
    let request = match parsed {
        Ok(request) => request.flatten(),
        Err(error) => {
            return PlanPublication::Pending(dfmcp_error_payload("fortress.plan", &error));
        }
    };
    if let Some(request) = request {
        if summary.is_some() || paused_target.is_some() || actions.is_some() || production.is_some()
        {
            return PlanPublication::Pending(coded_error_payload(
                "fortress.plan",
                ErrorCode::InvalidRequest,
                "candidate wrappers contain their complete sources; do not combine them with outer plan fields",
            ));
        }
        return match plan_comparison_handler::execute(session_id, &request, started) {
            Ok(output) => PlanPublication::Published(output),
            Err(error) => PlanPublication::Pending(dfmcp_error_payload("fortress.plan", &error)),
        };
    }
    PlanPublication::Pending(plan_request_legacy(
        session_id,
        summary,
        paused_target,
        actions,
        blueprint,
        production,
    ))
}

/// Detection routes original bytes to the strict versioned parser. It never
/// normalizes away duplicate keys or treats a malformed policy as an opt-out.
fn is_resource_aware_production_request(raw: &str) -> bool {
    raw.len() <= crate::lab_world::MAX_ACTIONS_JSON_BYTES
        && serde_json::from_str::<serde_json::Value>(raw).is_ok_and(|request| {
            request["template"] == "production"
                && (request.get("acquire_resources").is_some()
                    || request["planner"] == "resource_aware_v1"
                    || request["planner"] == "resource_reserve_v1")
        })
}

/// Detection preserves the original bytes for strict parsing. A malformed or
/// duplicate acquisition policy must never fall back to ordinary room intake.
fn is_resource_aware_room_request(raw: &str) -> bool {
    raw.len() <= 64 * 1_024
        && serde_json::from_str::<serde_json::Value>(raw).is_ok_and(|request| {
            let room = if request["template"] == "provisioned_room" {
                &request
            } else if request["template"] == "connected_room" {
                &request["room"]
            } else {
                return false;
            };
            room.get("acquire_resources").is_some()
                || room["planner"] == "provisioned_room_resources_v1"
                || request["planner"] == "connected_room_resources_v1"
                || request["planner"] == "connected_room_stairs_resources_v1"
        })
}

fn plan_request_legacy(
    session_id: Option<String>,
    summary: Option<String>,
    paused_target: Option<bool>,
    actions: Option<String>,
    blueprint: Option<String>,
    production: Option<String>,
) -> String {
    // A production objective arrives as a blueprint template.
    let continuation = match blueprint
        .as_deref()
        .filter(|raw| goal_continuation::is_request(raw))
        .map(goal_continuation::parse_request)
        .transpose()
    {
        Ok(request) => request,
        Err(error) => return dfmcp_error_payload("fortress.plan", &error),
    };
    if continuation.is_some() && paused_target.is_some() {
        return coded_error_payload(
            "fortress.plan",
            ErrorCode::InvalidRequest,
            "continue_goal cannot be combined with a pause target",
        );
    }
    if production.is_some()
        && blueprint
            .as_deref()
            .is_some_and(crate::lab_world::is_production_objective)
    {
        return coded_error_payload(
            "fortress.plan",
            ErrorCode::InvalidRequest,
            "production and its blueprint alias cannot both be supplied",
        );
    }
    let (blueprint, production) = match blueprint {
        Some(raw) if crate::lab_world::is_production_objective(&raw) => (None, Some(raw)),
        other => (other, production),
    };
    if [actions.is_some(), blueprint.is_some(), production.is_some()]
        .iter()
        .filter(|named| **named)
        .count()
        > 1
    {
        return coded_error_payload(
            "fortress.plan",
            ErrorCode::InvalidRequest,
            "a plan request names at most one of actions, blueprint or production",
        );
    }
    let default_summary = if production.is_some() {
        "meet production quotas".to_owned()
    } else if blueprint.is_some() {
        String::new()
    } else if let Some(raw) = actions.as_deref() {
        describe_actions(raw)
    } else {
        "unpause the simulation".to_owned()
    };
    let summary_supplied = summary.is_some();
    let summary = summary.map_or(default_summary, |value| value);
    if summary.len() > MAX_SUMMARY_BYTES {
        return coded_error_payload(
            "fortress.plan",
            ErrorCode::BudgetExceeded,
            "plan summary exceeds its explicit byte bound",
        );
    }
    let session = match resolve_session(session_id) {
        Ok(value) => value,
        Err(error) => return dfmcp_error_payload("fortress.plan", &error),
    };
    with_session(
        &session,
        || mutex_poisoned_payload("fortress.plan"),
        |guard| {
            let (rid, ctx) = match next_context(guard) {
                Ok(value) => value,
                Err(error) => return dfmcp_error_payload("fortress.plan", &error),
            };
            if let Err(error) = authorize_entry(&ctx, Capability::Plan, RiskTier::ReadOnly) {
                return dfmcp_error_payload("fortress.plan", &error);
            }
            let source = if let Some(parent) = continuation {
                match objectives::continuation_source(
                    guard,
                    parent,
                    summary_supplied.then_some(summary),
                    None,
                ) {
                    Ok(source) => source,
                    Err(error) => return dfmcp_error_payload("fortress.plan", &error),
                }
            } else {
                match (production, actions, blueprint) {
                    (Some(raw), _, _) => {
                        let request = match crate::lab_world::ProductionRequest::parse_current(&raw)
                        {
                            Ok(request) => request,
                            Err(error) => return dfmcp_error_payload("fortress.plan", &error),
                        };
                        PlanSource::Production {
                            summary,
                            raw: request.canonical_json(),
                        }
                    }
                    (_, _, Some(raw)) if civilian_recovery::is_release_request(&raw) => {
                        if paused_target.is_some() {
                            return coded_error_payload(
                                "fortress.plan",
                                ErrorCode::InvalidRequest,
                                "civilian release cannot be combined with a pause target",
                            );
                        }
                        if guard.adapter.snapshot().graph.entities.len()
                            > ctx.budget.max_entities as usize
                        {
                            return coded_error_payload(
                                "fortress.plan",
                                ErrorCode::BudgetExceeded,
                                "civilian release requires allowance for the complete current census",
                            );
                        }
                        let source =
                            civilian_recovery::parse_release_request(&raw).and_then(|burrow| {
                                guard
                                    .civilian_recovery
                                    .release_source(guard.adapter.snapshot(), burrow)
                            });
                        let source = match source {
                            Ok(source) => source,
                            Err(error) => return dfmcp_error_payload("fortress.plan", &error),
                        };
                        if let Err(error) = source.validate_budget(guard.adapter.snapshot(), &ctx) {
                            return dfmcp_error_payload("fortress.plan", &error);
                        }
                        PlanSource::CivilianRelease {
                            summary,
                            raw: source.canonical_json(),
                        }
                    }
                    (_, _, Some(raw)) if connected_room::is_connected_room(&raw) => {
                        let request =
                            match connected_room::ConnectedRoomRequest::parse_current(&raw) {
                                Ok(request) => request,
                                Err(error) => return dfmcp_error_payload("fortress.plan", &error),
                            };
                        PlanSource::ConnectedRoom {
                            summary,
                            raw: request.canonical_json(),
                        }
                    }
                    (_, _, Some(raw)) => {
                        let raw = if resource_acquisition::is_request(&raw) {
                            match resource_acquisition::ResourceAcquisitionRequest::parse_current(
                                &raw,
                            ) {
                                Ok(request) => request.canonical_json(),
                                Err(error) => return dfmcp_error_payload("fortress.plan", &error),
                            }
                        } else if room_provisioning::is_provisioned_room(&raw) {
                            match room_provisioning::ProvisionedRoomRequest::parse_current(&raw) {
                                Ok(request) => request.canonical_json(),
                                Err(error) => return dfmcp_error_payload("fortress.plan", &error),
                            }
                        } else {
                            raw
                        };
                        PlanSource::Blueprint { summary, raw }
                    }
                    (_, Some(raw), None) => PlanSource::Actions { summary, raw },
                    (None, None, None) => PlanSource::Pause {
                        summary,
                        paused_target: paused_target.is_some_and(|value| value),
                    },
                }
            };
            let snapshot = guard.adapter.snapshot();
            if source.requires_complete_source_review() {
                let admitted = authorize_entry(&ctx, Capability::Observe, RiskTier::ReadOnly)
                    .and_then(|()| query_budget::admit_sources(&[snapshot], &ctx));
                if let Err(error) = admitted {
                    return dfmcp_error_payload("fortress.plan", &error);
                }
            }
            let (intent, production_analysis) = match source.compile(IntentId::new(rid), snapshot) {
                Ok(compiled) => compiled,
                Err(error) => return dfmcp_error_payload("fortress.plan", &error),
            };

            match StaticPlanner::default().prepare_laboratory(snapshot, &intent, &ctx) {
                Ok(plan) => {
                    let digest = plan.digest.to_string();
                    let pending_digest = digest.clone();
                    let payload = json!({
                        "ok": true,
                        "session_id": format!("{}", guard.session_id),
                        "plan_id": format!("{}", plan.id),
                        "plan_digest": digest,
                        "terminal_condition": format!("{:?}", intent.terminal_condition),
                        "max_risk": plan.max_risk.as_str(),
                        "required_capabilities": plan.required_capabilities.iter().map(|c| c.as_str()).collect::<Vec<_>>(),
                        "requires_checkpoint": plan.requires_checkpoint,
                        "expires_at_tick": plan.expires_at_tick.0,
                        "steps": crate::lab_world::plan_steps_json(&plan),
                        "forecast": source.forecast(&guard.adapter, &plan, &ctx),
                        "live_routing": live_routing_json(&plan),
                        "production": if matches!(&source, PlanSource::CivilianRelease { .. }) || source.is_resource_acquisition() { None } else { production_analysis.as_ref() },
                        "civilian_release": if matches!(&source, PlanSource::CivilianRelease { .. }) { production_analysis.as_ref() } else { None },
                        "resource_acquisition": if source.is_resource_acquisition() { production_analysis.as_ref() } else { None },
                        "continuation": source.continuation_json(),
                        "note": "sealed plan; commit it with fortress_commit before expiry",
                    });
                    guard.pending = Some(PendingPlan {
                        plan,
                        digest: pending_digest,
                        source,
                    });
                    payload.to_string()
                }
                Err(error) => dfmcp_error_payload("fortress.plan", &error),
            }
        },
    )
}

/// Build an intent from semantic action steps. Its terminal condition is the
/// conjunction of every step's reference postcondition, so an intent whose
/// effects already hold is refused instead of being re-executed.
fn semantic_intent(
    id: IntentId,
    snapshot: &WorldSnapshot,
    summary: String,
    raw: &str,
) -> Result<Intent> {
    let requested_actions = crate::lab_world::parse_steps(raw)?;
    let mut terminal = Vec::new();
    let mut max_risk = RiskTier::ReadOnly;
    for (index, requested) in requested_actions.iter().enumerate() {
        let step =
            dfmcp_core::StepId::new(u32::try_from(index).map_err(|_| {
                DfmcpError::new(ErrorCode::BudgetExceeded, "too many semantic steps")
            })?);
        let action = requested.action.normalized();
        let key = dfmcp_intent::derive_step_idempotency_key(id, snapshot.anchor(), step, &action);
        terminal.extend(dfmcp_intent::effects::default_postconditions(
            &action,
            &key,
            snapshot.fortress_id,
        ));
        max_risk = max_risk.max(action.risk());
    }
    Ok(Intent {
        id,
        anchor: snapshot.anchor(),
        summary,
        terminal_condition: Predicate::All(terminal).normalized(),
        constraints: vec![Constraint::MaxRisk(max_risk)],
        requested_actions,
    })
}

/// A resumable handoff packet: everything a fresh agent needs to continue this
/// session safely without the transcript. Read-only; it grants nothing.
pub(crate) fn handoff_json(session: &LabSession) -> serde_json::Value {
    let snapshot = session.adapter.snapshot();
    let action_view = |action_id: ActionId| {
        let receipt = session.adapter.action_receipt(action_id);
        let step = session.adapter.action_step(action_id);
        json!({
            "action_id": format!("{action_id}"),
            "step": receipt.map(|r| r.step_id.get()),
            "state": receipt.map(|r| format!("{:?}", r.state)),
            "work_state": action_work_json(session, action_id),
            "risk": step.map(|s| s.risk.as_str()),
            "capability": step.map(|s| s.required_capability.as_str()),
            "obligation": step.and_then(|s| s.obligation.as_ref()).map(|o| json!({
                "terminal": crate::lab_world::predicate_json(&o.terminal),
                "deadline_tick": o.deadline_tick.0,
            })),
        })
    };
    let pending = session.pending.as_ref().map(|pending| {
        json!({
            "plan_digest": pending.digest,
            "continuation": pending.source.continuation_json(),
            "expires_at_tick": pending.plan.expires_at_tick.0,
            "anchor_sequence": pending.plan.anchor.cursor.sequence,
            "required_capabilities": plan_authority(&pending.plan)
                .iter()
                .map(|(capability, risk)| json!({"capability": capability.as_str(), "max_risk": risk.as_str()}))
                .collect::<Vec<_>>(),
            "steps": crate::lab_world::plan_steps_json(&pending.plan),
        })
    });
    let mut resume = vec![json!({
        "tool": "fortress.observe",
        "arguments": {"session_id": format!("{}", session.session_id)},
        "why": "re-establish the current anchor before acting on anything below",
    })];
    if let Some(pending) = session.pending.as_ref() {
        resume.push(json!({
            "tool": "fortress.commit",
            "arguments": {"session_id": format!("{}", session.session_id), "plan_digest": pending.digest},
            "why": "a sealed plan awaits commit; it is refused if the anchor moved, in which case re-plan the same intent",
        }));
    }
    if !session.open_actions.is_empty() {
        let terminal_work = session.open_actions.iter().any(|id| {
            session
                .adapter
                .action_receipt(*id)
                .is_some_and(|receipt| receipt.state.is_terminal())
                && !action_fully_drained(&session.adapter, *id)
        });
        resume.push(if terminal_work {
            json!({
                "tool": "fortress.cancel",
                "arguments": {"session_id": session.session_id.to_string(), "scope": "session", "mode": "stop_future_steps"},
                "why": "a terminal goal still owns active or unresolved physical work; cleanup requires current original effect authority",
            })
        } else if snapshot.paused {
            json!({
                "tool": "fortress.plan",
                "arguments": {"session_id": format!("{}", session.session_id), "paused_target": false},
                "why": "committed work is open but the fortress is paused, so it cannot progress",
            })
        } else {
            json!({
                "tool": "fortress.wait",
                "arguments": {"session_id": format!("{}", session.session_id), "max_game_ticks": 100},
                "why": "committed work is open; let bounded game time pass and prove obligations",
            })
        });
    }
    let alerts = world_alerts(session);
    if session.carried.iter().any(|carried| !carried.is_final()) && !snapshot.paused {
        resume.push(json!({
            "tool": "fortress.wait",
            "arguments": {"session_id": format!("{}", session.session_id), "max_game_ticks": 100},
            "why": "recovered goals and physical work require current observation; terminal proof receipts remain unchanged while work progresses",
        }));
    }
    for alert in &alerts {
        if alert["severity"] == "critical" && alert["remedy"]["available"] == true {
            let mut arguments = alert["remedy"]["arguments"].clone();
            arguments["session_id"] = json!(format!("{}", session.session_id));
            resume.push(json!({
                "tool": "fortress.plan",
                "arguments": arguments,
                "why": alert["finding"],
            }));
        }
    }
    let oldest_retained = session
        .history
        .oldest()
        .map(|version| version.state_hash.to_hex());
    json!({
        "ok": true,
        "schema": "dfmcp.lab-handoff/1",
        "durability": durability_json(session),
        "world_alerts": alerts,
        "civilian_recovery": session.civilian_recovery.projection(),
        "objectives": objectives_json(session),
        "orientation": {
            "replay_bundle": format!("df://session/{}/replay", session.session_id),
            "changes_since_oldest_retained": oldest_retained.map(|hash| json!({
                "tool": "fortress.query",
                "arguments": {"mode": json!({"mode": "changes", "since": hash}).to_string()},
            })),
            "retained_versions": session.history.len(),
        },
        "session_id": format!("{}", session.session_id),
        "fortress_id": format!("{}", session.fortress_id),
        "anchor": anchor_json(&snapshot.anchor()),
        "game_tick": snapshot.tick.0,
        "paused": snapshot.paused,
        "granted_capabilities": session
            .grants
            .iter()
            .map(|grant| json!({"capability": grant.capability.as_str(), "max_risk": grant.max_risk.as_str()}))
            .collect::<Vec<_>>(),
        "budget": {
            "max_game_ticks": session.budget.max_game_ticks,
            "max_entities": session.budget.max_entities,
            "max_bytes": session.budget.max_bytes,
            "max_output_tokens": session.budget.max_output_tokens,
            "max_actions": session.budget.max_actions,
        },
        "pending_plan": pending,
        "open_actions": session.open_actions.iter().copied().map(action_view).collect::<Vec<_>>(),
        "untracked_work": untracked_work_json(session),
        "last_plan_actions": session.last_plan_actions.iter().copied().map(action_view).collect::<Vec<_>>(),
        "query_work": query_work_json(&read_context(session)),
        "mcp_tasks": crate::task_service::session_handles(&session.session_id.to_string()),
        "mcp_task_coverage": crate::task_service::session_handle_coverage(&session.session_id.to_string()),
        "committed_plan_digests": session.commit_receipts.keys().collect::<Vec<_>>(),
        "resume_protocol": resume,
        "authority": "reading this packet grants nothing; every commit and cancel re-checks the session's negotiated grants",
        "epistemic_note": "action states are the last recorded receipts; fortress.wait re-evaluates them against a fresh observation",
    })
}

/// Query resources remain discoverable without sampling a watch or acquiring
/// world facts. Query authority is independent of the observation/handoff grant;
/// an unavailable catalog must never look like a proven empty work registry.
fn query_work_json(context: &OperationContext) -> serde_json::Value {
    let started = std::time::Instant::now();
    let collect = || -> Result<serde_json::Value> {
        let baselines = temporal_query::retained_baseline_work(context)?;
        let watches = temporal_query::retained_watch_work(context)?;
        let value = json!({
            "status": "available",
            "anchor": anchor_json(&context.anchor),
            "storage": "bounded_process_local",
            "sampling": "explicit_foreground_poll_only",
            "query_baselines": baselines,
            "query_watches": watches["query_watches"],
            "condition_watches": watches["condition_watches"],
        });
        let maximum = context
            .budget
            .max_bytes
            .min(u64::from(context.budget.max_output_tokens).saturating_mul(4));
        if value.to_string().len() as u64 > maximum
            || started.elapsed() >= std::time::Duration::from_millis(context.budget.max_wall_millis)
        {
            return Err(DfmcpError::new(
                ErrorCode::BudgetExceeded,
                "complete query work catalog exceeds its publication allowance",
            ));
        }
        authorize_entry(context, Capability::Query, RiskTier::ReadOnly)?;
        Ok(value)
    };
    match collect() {
        Ok(value) => value,
        Err(error) => json!({
            "status": "unavailable",
            "anchor": anchor_json(&context.anchor),
            "code": error.code.as_str(),
            "reason": error.message,
            "absence_proven": false,
        }),
    }
}

/// Exclusive spatial leases for every step that excavates or builds. Retained
/// action ownership fences unresolved work even after a lease's TTL expires.
/// A session never conflicts with itself.
/// The caller restores the prior lease book if the commit then fails.
fn acquire_plan_leases(
    session: &mut LabSession,
    plan: &PreparedPlan,
) -> Result<Vec<(dfmcp_core::StepId, Vec<dfmcp_core::LeaseId>)>> {
    let now = session.adapter.snapshot().tick;
    // A failed later reservation must not publish earlier reservations or
    // consume their identifiers. Publish the complete lease set together.
    let mut manager = session.leases.manager.clone();
    manager.cleanup_expired_leases(now);
    let mut acquired = Vec::new();
    let untracked = if plan.steps.iter().any(|step| {
        matches!(
            step.action,
            Action::DesignateDig { .. } | Action::Build { .. } | Action::PlaceFurniture { .. }
        )
    }) {
        untracked_work(session)?
    } else {
        Vec::new()
    };
    for step in &plan.steps {
        let area = match &step.action {
            Action::DesignateDig { area, .. } => *area,
            Action::Build { footprint, .. } => *footprint,
            Action::PlaceFurniture { location, .. } => MapCuboid {
                min: *location,
                max: *location,
            },
            _ => continue,
        };
        if untracked.iter().any(|work| work.conflicts_with(&area)) {
            return Err(DfmcpError::new(
                ErrorCode::Conflict,
                "untracked reference work has an active or unresolved footprint in this region; reconcile or observe quiescence first",
            ));
        }
        for carried in &session.carried {
            if carried.work_state.is_quiescent() {
                continue;
            }
            let work = carried.work_step.as_ref().ok_or_else(|| DfmcpError::new(
                ErrorCode::Conflict,
                "a recovered effect has unresolved identity; reconcile or restore before reserving a region",
            ))?;
            let owned_area = match &work.action {
                Action::DesignateDig { area, .. } => *area,
                Action::Build { footprint, .. } => *footprint,
                Action::PlaceFurniture { location, .. } => MapCuboid {
                    min: *location,
                    max: *location,
                },
                _ => continue,
            };
            if dfmcp_core::lease::cuboids_intersect(&area, &owned_area) {
                return Err(DfmcpError::new(
                    ErrorCode::Conflict,
                    "the requested region overlaps unresolved physical work carried across recovery",
                ));
            }
        }
        for (action_id, (holder, _)) in &session.leases.by_action {
            if *holder == session.session_id || action_fully_drained(&session.adapter, *action_id) {
                continue;
            }
            let owned = session.adapter.action_step(*action_id).ok_or_else(|| {
                DfmcpError::new(
                    ErrorCode::Conflict,
                    "another member's retained work cannot be resolved; its region remains fenced",
                )
            })?;
            let owned_area = match &owned.action {
                Action::DesignateDig { area, .. } => *area,
                Action::Build { footprint, .. } => *footprint,
                Action::PlaceFurniture { location, .. } => MapCuboid {
                    min: *location,
                    max: *location,
                },
                _ => continue,
            };
            if dfmcp_core::lease::cuboids_intersect(&area, &owned_area) {
                return Err(DfmcpError::new(
                    ErrorCode::Conflict,
                    format!(
                        "step {} overlaps unresolved work owned by session {holder}; proof expiry does not release physical work",
                        step.id.get(),
                    ),
                ));
            }
        }
        let ttl = step.obligation.as_ref().map_or(1, |obligation| {
            obligation.deadline_tick.0.saturating_sub(now.0).max(1)
        });
        let lease = manager
            .acquire_spatial_lease(session.session_id, area, true, now, ttl)
            .map_err(|error| {
                DfmcpError::new(
                    error.code,
                    format!(
                        "step {} cannot lease its region: {}",
                        step.id.get(),
                        error.message
                    ),
                )
            })?;
        acquired.push((step.id, vec![lease]));
    }
    session.leases.manager = manager;
    Ok(acquired)
}

/// A terminal goal receipt cannot release work that still exists in the world.
fn release_action_leases(session: &mut LabSession, action_id: ActionId) {
    if !action_fully_drained(&session.adapter, action_id) {
        return;
    }
    if let Some((holder, leases)) = session.leases.by_action.remove(&action_id) {
        for lease in leases {
            // An expired lease may already have been cleaned up.
            let _ = session.leases.manager.release_lease(lease, holder);
        }
    }
}

fn effect_work_json(state: &EffectWorkState) -> serde_json::Value {
    let (kind, entity_id, reason) = match state {
        EffectWorkState::NeverDispatched => ("never_dispatched", None, None),
        EffectWorkState::Active { entity_id } => ("active", Some(*entity_id), None),
        EffectWorkState::Quiescent { entity_id } => ("quiescent", *entity_id, None),
        EffectWorkState::Unknown { entity_id, reason } => {
            ("unknown", *entity_id, Some(reason.as_str()))
        }
    };
    json!({
        "state": kind,
        "quiescent": state.is_quiescent(),
        "entity_id": entity_id.map(|id| id.to_string()),
        "reason": reason,
    })
}

fn action_work_json(session: &LabSession, action_id: ActionId) -> serde_json::Value {
    let adapter = &session.adapter;
    let (mut view, entity_id) = match adapter.action_work_state(action_id) {
        Ok(state) => {
            let entity_id = match &state {
                EffectWorkState::NeverDispatched => None,
                EffectWorkState::Active { entity_id } => Some(*entity_id),
                EffectWorkState::Quiescent { entity_id }
                | EffectWorkState::Unknown { entity_id, .. } => *entity_id,
            };
            (effect_work_json(&state), entity_id)
        }
        Err(error) => (
            effect_work_json(&EffectWorkState::Unknown {
                entity_id: None,
                reason: format!("{}: {}", error.code.as_str(), error.message),
            }),
            None,
        ),
    };
    view["observed_anchor"] = anchor_json(&adapter.snapshot().anchor());
    let ctx = context_for(session, session.next_request_id);
    if let Some(entity_id) = entity_id
        && authorize_entry(&ctx, Capability::Observe, RiskTier::ReadOnly).is_ok()
    {
        match crate::lab_world::workforce_action_json(adapter.snapshot(), entity_id) {
            Ok(Some(workforce)) => view["workforce"] = workforce,
            Ok(None) => {}
            Err(error) => {
                view["workforce"] = json!({
                    "available": false,
                    "epistemic_state": "unknown",
                    "reason": error.message,
                    "code": error.code.as_str(),
                    "source_anchor": anchor_json(&adapter.snapshot().anchor()),
                });
            }
        }
        match crate::lab_world::construction_material_action_json(adapter.snapshot(), entity_id) {
            Ok(Some(materials)) => view["construction_materials"] = materials,
            Ok(None) => {}
            Err(error) => {
                view["construction_materials"] = json!({
                    "available": false,
                    "epistemic_state": "unknown",
                    "reason": error.message,
                    "code": error.code.as_str(),
                    "source_anchor": anchor_json(&adapter.snapshot().anchor()),
                });
            }
        }
        match crate::lab_world::provisions_action_json(adapter.snapshot(), entity_id) {
            Ok(Some(provisions)) => view["provisions"] = provisions,
            Ok(None) => {}
            Err(error) => {
                view["provisions"] = json!({
                    "available": false,
                    "epistemic_state": "unknown",
                    "reason": error.message,
                    "code": error.code.as_str(),
                    "source_anchor": anchor_json(&adapter.snapshot().anchor()),
                });
            }
        }
        match crate::lab_world::harvest_action_json(adapter.snapshot(), entity_id) {
            Ok(Some(harvesting)) => view["harvesting"] = harvesting,
            Ok(None) => {}
            Err(error) => {
                view["harvesting"] = json!({
                    "available": false,
                    "epistemic_state": "unknown",
                    "reason": error.message,
                    "code": error.code.as_str(),
                    "source_anchor": anchor_json(&adapter.snapshot().anchor()),
                });
            }
        }
    }
    view
}

fn action_fully_drained(adapter: &MemoryAdapter, action_id: ActionId) -> bool {
    adapter
        .action_receipt(action_id)
        .is_some_and(|receipt| receipt.state.is_terminal())
        && adapter
            .action_work_state(action_id)
            .is_ok_and(|state| state.is_quiescent())
}

fn effect_drain_json(receipt: &dfmcp_lab::EffectDrainReceipt) -> serde_json::Value {
    json!({
        "action_id": receipt.action_id.to_string(),
        "before": effect_work_json(&receipt.before),
        "after": effect_work_json(&receipt.after),
        "observed_anchor": anchor_json(&receipt.observed_anchor),
        "stopped_work": receipt.stopped_work,
        "proof_receipt_preserved": true,
        "evidence": receipt.evidence.iter().map(|evidence| json!({
            "evidence_id": evidence.id.to_string(),
            "digest": evidence.digest.to_hex(),
            "kind": format!("{:?}", evidence.kind),
            "summary": evidence.summary,
            "anchor": anchor_json(&evidence.anchor),
        })).collect::<Vec<_>>(),
    })
}

fn retain_open_actions(session: &mut LabSession) {
    session
        .open_actions
        .retain(|id| !action_fully_drained(&session.adapter, *id));
}

/// An authorized emergency pause starts a new shared-clock decision. Call
/// immediately after the successful pause phase, even if later cleanup fails.
fn reset_emergency_unpause_consent(session: &mut LabSession, mode: CancelMode) {
    if mode == CancelMode::EmergencyPauseAndDrain && session.adapter.snapshot().paused {
        session.leases.unpause_consent.clear();
    }
}

/// Cancellation can publish a request or emergency pause before finalization
/// refuses. Expose the current world and work on those errors as well.
fn cancel_action_error_payload(
    session: &LabSession,
    action_id: ActionId,
    error: &DfmcpError,
) -> String {
    let mut payload: serde_json::Value =
        serde_json::from_str(&dfmcp_error_payload("fortress.cancel", error))
            .unwrap_or_else(|_| json!({"ok": false}));
    let receipt = session.adapter.action_receipt(action_id);
    let work = action_work_json(session, action_id);
    let remaining_nonterminal = usize::from(!receipt.is_some_and(|row| row.state.is_terminal()));
    let remaining_work = usize::from(work["quiescent"] != true);
    payload["scope"] = json!("last_action");
    payload["action_id"] = json!(action_id.to_string());
    payload["final_state"] = json!(receipt.map(|row| format!("{:?}", row.state)));
    payload["work_state"] = work;
    payload["paused"] = json!(session.adapter.snapshot().paused);
    payload["untracked_work"] = untracked_work_json(session);
    payload["observed_anchor"] = anchor_json(&session.adapter.snapshot().anchor());
    payload["drain_progress"] = json!({
        "actions_total": 1, "drained": 0,
        "remaining_nonterminal": remaining_nonterminal,
        "remaining_work": remaining_work, "quiescent": false,
    });
    payload["finalize_certificate"] = serde_json::Value::Null;
    payload.to_string()
}

fn untracked_work(session: &LabSession) -> Result<Vec<restore_work::UntrackedWork>> {
    let mut known: BTreeSet<EntityId> = session.adapter.known_work_entity_ids().collect();
    for carried in &session.carried {
        if let Some(step) = &carried.work_step
            && matches!(
                step.action,
                Action::DesignateDig { .. }
                    | Action::Build { .. }
                    | Action::PlaceFurniture { .. }
                    | Action::CreateWorkOrder { .. }
                    | Action::HarvestResource { .. }
            )
        {
            known.insert(dfmcp_intent::effects::created_entity_id(
                &step.idempotency_key,
                0,
            ));
        }
    }
    restore_work::inspect_untracked_work(
        session.adapter.snapshot(),
        &known,
        session.budget.max_entities,
    )
}

fn untracked_work_json(session: &LabSession) -> serde_json::Value {
    let anchor = anchor_json(&session.adapter.snapshot().anchor());
    let ctx = context_for(session, session.next_request_id);
    let observed = authorize_entry(&ctx, Capability::Observe, RiskTier::ReadOnly)
        .and_then(|()| untracked_work(session));
    match observed {
        Ok(work) => json!({
            "state": "observed", "quiescent": work.is_empty(),
            "items": work.iter().map(restore_work::UntrackedWork::to_json).collect::<Vec<_>>(),
            "observed_anchor": anchor,
            "note": "reference work without a retained action handle remains visible; reading it grants no cancellation authority",
        }),
        Err(error) => json!({
            "state": "unknown", "quiescent": false, "items": null,
            "observed_anchor": anchor,
            "reason": format!("{}: {}", error.code.as_str(), error.message),
        }),
    }
}

/// Current scheduling information is observation, not an effect receipt.
/// Check authority before even inspecting whether this world selected a model.
fn session_workforce_json(session: &LabSession) -> Option<serde_json::Value> {
    let ctx = context_for(session, session.next_request_id);
    if authorize_entry(&ctx, Capability::Observe, RiskTier::ReadOnly).is_err() {
        return None;
    }
    match crate::lab_world::workforce_json(session.adapter.snapshot()) {
        Ok(workforce) => workforce,
        Err(error) => Some(json!({
            "available": false,
            "epistemic_state": "unknown",
            "reason": error.message,
            "code": error.code.as_str(),
            "source_anchor": anchor_json(&session.adapter.snapshot().anchor()),
        })),
    }
}

fn session_construction_material_json(session: &LabSession) -> Option<serde_json::Value> {
    let ctx = context_for(session, session.next_request_id);
    if authorize_entry(&ctx, Capability::Observe, RiskTier::ReadOnly).is_err() {
        return None;
    }
    match crate::lab_world::construction_material_json(session.adapter.snapshot()) {
        Ok(materials) => materials,
        Err(error) => Some(json!({
            "available": false,
            "epistemic_state": "unknown",
            "reason": error.message,
            "code": error.code.as_str(),
            "source_anchor": anchor_json(&session.adapter.snapshot().anchor()),
        })),
    }
}

fn session_harvest_json(session: &LabSession) -> Option<serde_json::Value> {
    let ctx = context_for(session, session.next_request_id);
    if authorize_entry(&ctx, Capability::Observe, RiskTier::ReadOnly).is_err() {
        return None;
    }
    match crate::lab_world::harvest_json(session.adapter.snapshot()) {
        Ok(harvesting) => harvesting,
        Err(error) => Some(json!({
            "available": false,
            "epistemic_state": "unknown",
            "reason": error.message,
            "code": error.code.as_str(),
            "source_anchor": anchor_json(&session.adapter.snapshot().anchor()),
        })),
    }
}

fn session_provisions_json(session: &LabSession) -> Option<serde_json::Value> {
    let ctx = context_for(session, session.next_request_id);
    if authorize_entry(&ctx, Capability::Observe, RiskTier::ReadOnly).is_err() {
        return None;
    }
    match crate::lab_world::provisions_json(session.adapter.snapshot()) {
        Ok(provisions) => provisions,
        Err(error) => Some(json!({
            "available": false,
            "epistemic_state": "unknown",
            "reason": error.message,
            "code": error.code.as_str(),
            "source_anchor": anchor_json(&session.adapter.snapshot().anchor()),
        })),
    }
}

/// Bound the aggregate physical effects before starting a multi-action drain.
/// Fresh contexts for individual steps must not reset the call's action budget.
fn authorize_drain_budget(
    session: &LabSession,
    actions: &[ActionId],
    mode: CancelMode,
) -> Result<()> {
    if actions.len() > session.budget.max_actions as usize {
        return Err(DfmcpError::new(
            ErrorCode::BudgetExceeded,
            "the selected drain exceeds the session action budget; use scope=oldest_open_plan to drain retained plans individually",
        ));
    }
    let mut effects = 0u64;
    let mut needs_drain = false;
    for id in actions {
        let receipt = session.adapter.action_receipt(*id).ok_or_else(|| {
            DfmcpError::new(
                ErrorCode::Conflict,
                "retained action missing; drain budget and quiescence are unresolved",
            )
        })?;
        let work = session.adapter.action_work_state(*id)?;
        let terminal = receipt.state.is_terminal();
        needs_drain |= !terminal || !work.is_quiescent();
        effects += u64::from(matches!(work, EffectWorkState::Active { .. }));
        if !terminal
            && mode == CancelMode::CompensateReversible
            && work != EffectWorkState::NeverDispatched
            && session
                .adapter
                .action_step(*id)
                .is_some_and(|step| step.compensation.is_some())
        {
            effects += 1;
        }
    }
    effects += u64::from(
        needs_drain
            && mode == CancelMode::EmergencyPauseAndDrain
            && !session.adapter.snapshot().paused,
    );
    if effects > u64::from(session.budget.max_actions) {
        return Err(DfmcpError::new(
            ErrorCode::BudgetExceeded,
            "physical stops, emergency pause and compensation exceed the aggregate action budget",
        ));
    }
    Ok(())
}

/// A sealed plan whose anchor moved (another member acted, or game time
/// passed) is never committed blind. Replay the original request at the
/// current anchor: the planner re-checks every precondition and re-seals,
/// and the new plan becomes pending for an explicit commit of its digest.
/// Revalidate a stale plan by its read witness: when nothing the plan read
/// changed between the version it was sealed on and now, replay its intent
/// at the current anchor and accept the replay only if it preserves the entire
/// reviewed execution contract. Returns the re-sealed plan and its certificate,
/// or why explicit review of the new plan is required.
fn rebase_by_witness(
    session: &mut LabSession,
    stale: &PendingPlan,
) -> std::result::Result<(PreparedPlan, serde_json::Value), serde_json::Value> {
    if stale.source.requires_complete_source_review() {
        return Err(json!({
            "accepted": false,
            "reason": "resource planning reads complete finite supply, active demand, stock and workforce domains; review a fresh source allocation and stage",
        }));
    }
    if matches!(stale.source, PlanSource::CivilianRelease { .. }) {
        return Err(
            json!({"accepted":false, "reason":"civilian release requires a newly reviewed current owned clearance"}),
        );
    }
    if dfmcp_intent::workforce_planning::plan_uses_shared_workforce(&stale.plan) {
        return Err(json!({
            "accepted": false,
            "reason": "shared workforce deadlines read the complete current workload; review a newly sealed intent replay",
        }));
    }
    // Quota lowering reads the complete production domain, including absence
    // of competing orders, current service and setup candidates. The current
    // action/predicate witness does not cover those range and negative reads.
    // A changed anchor therefore requires a newly reviewed source replay;
    // equal action bytes alone cannot certify unchanged sealed deadlines.
    if matches!(
        &stale.source,
        PlanSource::Production { .. }
            | PlanSource::ProductionContinuation { .. }
            | PlanSource::RoomContinuation { .. }
            | PlanSource::ConnectedRoom { .. }
            | PlanSource::ConnectedRoomContinuation { .. }
    ) || stale
        .plan
        .steps
        .iter()
        .any(|step| matches!(step.action, Action::PlaceFurniture { .. }))
    {
        return Err(json!({
            "accepted": false,
            "reason": "production and item-backed placement require current workload, inventory and occupancy evidence; review a newly sealed intent replay",
        }));
    }
    let base = session
        .history
        .get(&stale.plan.anchor.state_hash)
        .filter(|version| version.anchor() == stale.plan.anchor)
        .cloned()
        .ok_or_else(
            || json!({"accepted": false, "reason": "the sealed version is no longer retained"}),
        )?;
    let witness = crate::witness::ReadWitness::of(&stale.plan);
    let now = session.adapter.snapshot().clone();
    let refinement = if let Some(change) = witness.first_change(&base, &now) {
        // Only raw semantic actions have a closed source interpretation here.
        // Blueprint/objective lowering can read more than its sealed steps.
        if !matches!(stale.source, PlanSource::Actions { .. }) {
            return Err(
                json!({"accepted":false, "reason":"a read of the plan changed", "change":change}),
            );
        }
        match crate::witness::FieldWitness::prove(&stale.plan, &base, &now, session.budget) {
            Ok(proof) => Some((proof, change)),
            Err(reason) => {
                return Err(
                    json!({"accepted":false, "reason":"a read of the plan changed",
                    "change":change, "refinement_refusal":reason}),
                );
            }
        }
    } else {
        None
    };
    let refused = |error: DfmcpError| json!({"accepted": false, "reason": error.message});
    let rid = next_request_id(session).map_err(refused)?;
    let (_, ctx) = next_context(session).map_err(refused)?;
    let plan = stale
        .source
        .intent(IntentId::new(rid), &now)
        .and_then(|intent| StaticPlanner::default().prepare_laboratory(&now, &intent, &ctx))
        .map_err(refused)?;
    if !crate::witness::same_actions(&stale.plan, &plan) {
        return Err(
            json!({"accepted": false, "reason": "the replayed intent changed the reviewed execution contract"}),
        );
    }
    let certificate = refinement.map_or_else(
        || crate::witness::certificate(&stale.plan, &plan, &base, &now, &witness),
        |(proof, change)| proof.certificate(&stale.plan, &plan, &change),
    );
    Ok((plan, certificate))
}

fn replay_stale_plan(session: &mut LabSession, stale: PendingPlan) -> String {
    if let Some(next_step) = stale.source.resource_replan_step(session.session_id) {
        let from_digest = stale.digest.clone();
        let production = stale.source.is_resource_aware_production();
        let room = stale.source.is_resource_aware_room();
        let reserve = stale.source.is_consumption_reserve_continuation();
        session.pending = Some(stale);
        let error = DfmcpError::new(
            ErrorCode::StaleAnchor,
            if room {
                "room supply or geometry observation changed; request and review the current stage of the complete original room project before committing"
            } else if production {
                "production supply observation changed; request and review the current stage of the complete original production objective before committing"
            } else if reserve {
                "reserve demand observation changed; request and review a new pursuit of the original production goal and absolute reserve tick before committing"
            } else {
                "resource acquisition observation changed; request and review a fresh allocation from the complete original stock objective before committing"
            },
        );
        let mut payload = serde_json::from_str::<serde_json::Value>(&dfmcp_error_payload(
            "fortress.commit",
            &error,
        ))
        .unwrap_or_else(|_| json!({"ok": false}));
        payload["session_id"] = json!(session.session_id.to_string());
        payload["rebase"] = json!({
            "method": if room { "fresh_resource_room_required" } else if production { "fresh_resource_production_required" } else if reserve { "fresh_production_reserve_required" } else { "fresh_resource_acquisition_required" },
            "from_digest": from_digest,
            "pending_plan_preserved": true,
            "game_effect": "none",
        });
        payload["next_step"] = next_step;
        return payload.to_string();
    }
    if matches!(stale.source, PlanSource::CivilianRelease { .. }) {
        session.pending = Some(stale);
        return coded_error_payload(
            "fortress.commit",
            ErrorCode::StaleAnchor,
            "civilian release observation changed; request and review a fresh release plan",
        );
    }
    if let Err(error) = objectives::validate_continuation(session, &stale.source, None) {
        session.pending = Some(stale);
        return dfmcp_error_payload("fortress.commit", &error);
    }
    let rid = match next_request_id(session) {
        Ok(value) => value,
        Err(error) => return dfmcp_error_payload("fortress.commit", &error),
    };
    let (_, ctx) = match next_context(session) {
        Ok(value) => value,
        Err(error) => return dfmcp_error_payload("fortress.commit", &error),
    };
    let snapshot = session.adapter.snapshot();
    let stale_error = DfmcpError::new(
        ErrorCode::StaleAnchor,
        "the sealed plan's anchor is no longer current; nothing was committed",
    );
    let mut payload: serde_json::Value =
        serde_json::from_str(&dfmcp_error_payload("fortress.commit", &stale_error))
            .unwrap_or_else(|_| json!({"ok": false}));
    let replayed =
        stale
            .source
            .compile(IntentId::new(rid), snapshot)
            .and_then(|(intent, analysis)| {
                StaticPlanner::default()
                    .prepare_laboratory(snapshot, &intent, &ctx)
                    .map(|plan| (plan, analysis))
            });
    match replayed {
        Ok((plan, production_analysis)) => {
            let digest = plan.digest.to_string();
            payload["rebased_plan"] = json!({
                "forecast": stale.source.forecast(&session.adapter, &plan, &ctx),
                "live_routing": live_routing_json(&plan),
                "plan_digest": digest,
                "expires_at_tick": plan.expires_at_tick.0,
                "required_capabilities": plan.required_capabilities.iter().map(|c| c.as_str()).collect::<Vec<_>>(),
                "steps": crate::lab_world::plan_steps_json(&plan),
                "production": production_analysis,
                "continuation": stale.source.continuation_json(),
            });
            payload["rebase"] = json!({
                "method": "intent_replay",
                "from_digest": stale.digest,
                "to_digest": digest,
                "from_anchor": anchor_json(&stale.plan.anchor),
                "to_anchor": anchor_json(&snapshot.anchor()),
                "note": "review the rebased steps, then commit the new digest; the stale plan was discarded",
            });
            session.pending = Some(PendingPlan {
                plan,
                digest,
                source: stale.source,
            });
        }
        Err(error) => {
            payload["rebase"] = json!({
                "method": "intent_replay",
                "from_digest": stale.digest,
                "refused": {"code": error.code.as_str(), "message": error.message},
                "note": "the original request no longer plans at the current anchor; re-observe and re-plan",
            });
        }
    }
    payload.to_string()
}

/// How the sealed plan maps onto the live DFHack development families: the
/// typed request per step or the reason none exists. Routing grants nothing.
fn live_routing_json(plan: &PreparedPlan) -> serde_json::Value {
    use dfmcp_adapter::live_routing::{LiveRequest, LiveResolution, route_plan};
    let route = match route_plan(plan) {
        Ok(route) => route,
        Err(error) => return json!({"error": error.message}),
    };
    let steps: Vec<serde_json::Value> = route
        .steps
        .iter()
        .map(|step| match &step.outcome {
            Ok(routed) => {
                let request = match &routed.request {
                    LiveRequest::Pause { paused } => json!({"pause": paused}),
                    LiveRequest::Dig { regions } => json!({
                        "dig_regions": regions.iter().map(|r| r.coordinates()).collect::<Vec<_>>(),
                    }),
                    LiveRequest::Furniture { kind, target, material } => {
                        json!({"furniture": kind.as_str(), "target": target,
                            "material": {
                                "required_tokens": material.required_tokens,
                                "forbidden_tokens": material.forbidden_tokens,
                                "prefer_nearest": material.prefer_nearest,
                                "reserve_count": material.reserve_count,
                            },
                        })
                    }
                    LiveRequest::WorkOrder { spec } => {
                        json!({"recipe": spec.recipe().as_str(), "amount": spec.amount()})
                    }
                    LiveRequest::WorkDetail { units, labor, assigned } => {
                        json!({"canonical_units": units.iter().map(ToString::to_string).collect::<Vec<_>>(),
                            "labor": labor, "assigned": assigned,
                            "native_units": null,
                        })
                    }
                };
                let requires: Vec<String> = routed
                    .requires
                    .iter()
                    .map(|r| match r {
                        LiveResolution::FurnitureItem { kind, .. } => {
                            format!(
                                "an exact unclaimed {} item from a live inventory read satisfying every retained material selector",
                                kind.as_str()
                            )
                        }
                        LiveResolution::WorkDetailForLabor { labor } => {
                            format!("an evidence-bound native unit mapping and selected-only work detail containing exactly labor {labor}; verify the native readback changed no other labor")
                        }
                    })
                    .collect();
                json!({
                    "step": step.step.get(),
                    "routable": true,
                    "execution_ready": false,
                    "protocol": routed.family.protocol(),
                    "dev_server": routed.family.dev_server(),
                    "dev_server_observations": match &routed.request {
                        LiveRequest::Dig { regions } => json!(regions
                            .iter()
                            .map(|r| json!({"tool": "fortress.observe", "arguments": {"region": r.coordinates()}}))
                            .collect::<Vec<_>>()),
                        _ => json!([]),
                    },
                    "request": request,
                    "requires_live_resolution": requires,
                    "live_preconditions": routed.live_preconditions,
                })
            }
            Err(refusal) => json!({
                "step": step.step.get(),
                "routable": false,
                "reason": refusal.reason,
            }),
        })
        .collect();
    json!({
        "fully_routable": route.fully_routable(),
        "execution_ready": false,
        "protocols": route.families().iter().map(|f| f.protocol()).collect::<Vec<_>>(),
        "steps": steps,
        "admission": "unadmitted development families only; routing is not authority and no live effect is admitted",
    })
}

/// The (capability, risk ceiling) pairs a sealed plan needs to be committed.
fn plan_authority(plan: &PreparedPlan) -> Vec<(Capability, RiskTier)> {
    let mut authority: BTreeMap<Capability, RiskTier> = BTreeMap::new();
    for step in &plan.steps {
        let entry = authority
            .entry(step.required_capability)
            .or_insert(step.risk);
        *entry = (*entry).max(step.risk);
    }
    if plan.requires_checkpoint {
        authority
            .entry(Capability::Checkpoint)
            .or_insert(RiskTier::Guarded);
    }
    authority.into_iter().collect()
}

// ============================================================================
// fortress.commit
// ============================================================================

/// Revalidate and idempotently commit the pending prepared plan. Requires the
/// exact plan digest; returns per-action receipts and the post-commit anchor.
#[tool(
    description = "Commit the pending prepared plan for the open session: prepare/revalidate, dispatch, observe, and verify. Requires the exact plan digest returned by fortress_plan."
)]
pub fn fortress_commit(session_id: Option<String>, plan_digest: String) -> String {
    if plan_digest.len() != DIGEST_HEX_BYTES
        || !plan_digest.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return coded_error_payload(
            "fortress.commit",
            ErrorCode::InvalidRequest,
            "plan_digest must be the 64-character hexadecimal digest returned by fortress_plan",
        );
    }
    let session = match resolve_session(session_id) {
        Ok(value) => value,
        Err(error) => return dfmcp_error_payload("fortress.commit", &error),
    };
    with_session(
        &session,
        || mutex_poisoned_payload("fortress.commit"),
        |guard| {
            if let Err(error) = durability_gate(guard) {
                return dfmcp_error_payload("fortress.commit", &error);
            }
            {
                let (_, entry_ctx) = match next_context(guard) {
                    Ok(value) => value,
                    Err(error) => return dfmcp_error_payload("fortress.commit", &error),
                };
                // Commit authority is the sealed plan's own capability set (checked
                // here and again per step by the adapter), not a fixed clock grant.
                let required = guard
                    .commit_authority
                    .get(&plan_digest)
                    .cloned()
                    .or_else(|| {
                        guard
                            .pending
                            .as_ref()
                            .filter(|pending| pending.digest == plan_digest)
                            .map(|pending| plan_authority(&pending.plan))
                    })
                    .unwrap_or_else(|| vec![(Capability::ControlClock, RiskTier::Reversible)]);
                for (capability, risk) in required {
                    if let Err(error) = authorize_entry(&entry_ctx, capability, risk) {
                        return dfmcp_error_payload("fortress.commit", &error);
                    }
                }
            }

            // Receipt replay is independent of whichever later plan is currently
            // pending. Reauthorize first, then return the exact stable payload without
            // consuming or replacing that unrelated plan.
            if let Some(payload) = guard.commit_receipts.get(&plan_digest).cloned() {
                let (_, replay_context) = match next_context(guard) {
                    Ok(value) => value,
                    Err(error) => return dfmcp_error_payload("fortress.commit", &error),
                };
                let required = guard
                    .commit_authority
                    .get(&plan_digest)
                    .cloned()
                    .unwrap_or_else(|| vec![(Capability::ControlClock, RiskTier::Reversible)]);
                for (capability, risk) in required {
                    if let Err(error) = replay_context.authorize(capability, risk, &[], None) {
                        return dfmcp_error_payload("fortress.commit", &error);
                    }
                }
                return payload;
            }

            let pending = match guard.pending.take() {
                Some(pending) => pending,
                None => {
                    return coded_error_payload(
                        "fortress.commit",
                        ErrorCode::InvalidPlan,
                        "no pending plan; call fortress_plan first",
                    );
                }
            };
            if pending.digest != plan_digest {
                guard.pending = Some(pending);
                return coded_error_payload(
                    "fortress.commit",
                    ErrorCode::Conflict,
                    "plan digest does not match the pending prepared plan; plans are sealed over their digest",
                );
            }
            if pending.source.is_resource_aware_production()
                || pending.source.is_resource_aware_room()
                || pending.source.is_consumption_reserve_continuation()
            {
                // Current read authority and complete-source budgets apply at
                // commit as well as proposal, before leases or effect custody.
                let current = context_for(guard, guard.next_request_id);
                let admitted = [Capability::Observe, Capability::Query, Capability::Plan]
                    .into_iter()
                    .try_for_each(|capability| {
                        authorize_entry(&current, capability, RiskTier::ReadOnly)
                    })
                    .and_then(|()| {
                        query_budget::admit_sources(&[guard.adapter.snapshot()], &current)
                            .map(|_| ())
                    });
                if let Err(error) = admitted {
                    guard.pending = Some(pending);
                    return dfmcp_error_payload("fortress.commit", &error);
                }
            }
            if let PlanSource::CivilianRelease { raw, .. } = &pending.source {
                let validated = civilian_recovery::ReleaseSource::parse(raw)
                    .and_then(|source| {
                        source.validate_budget(guard.adapter.snapshot(), &read_context(guard))
                    })
                    .and_then(|()| {
                        guard
                            .civilian_recovery
                            .validate_release(guard.adapter.snapshot(), raw)
                    });
                if let Err(error) = validated {
                    guard.pending = Some(pending);
                    return dfmcp_error_payload("fortress.commit", &error);
                }
            }
            // New pursuit is reviewed separately from old goal history. Repeat
            // current truth/authority and complete-lineage quiescence checks
            // before any lease, durable admission or adapter reservation.
            let retry_candidate = (pending.plan.anchor == guard.adapter.snapshot().anchor())
                .then_some(pending.plan.digest);
            if let Err(error) =
                objectives::validate_continuation(guard, &pending.source, retry_candidate)
            {
                guard.pending = Some(pending);
                return dfmcp_error_payload("fortress.commit", &error);
            }
            if !guard.commit_receipts.contains_key(&plan_digest)
                && guard.commit_receipts.len() >= MAX_LAB_COMMIT_RECEIPTS
            {
                guard.pending = Some(pending);
                return coded_error_payload(
                    "fortress.commit",
                    ErrorCode::BudgetExceeded,
                    "session commit-receipt store reached its explicit bound",
                );
            }
            if guard.open_actions.len() + pending.plan.steps.len() > MAX_OPEN_ACTIONS {
                guard.pending = Some(pending);
                return coded_error_payload(
                    "fortress.commit",
                    ErrorCode::BudgetExceeded,
                    "the session already tracks its maximum number of open actions; wait for or cancel existing work first",
                );
            }
            let mut pending = pending;
            let requested_digest = plan_digest.clone();
            let mut plan_digest = plan_digest.clone();
            let mut witness_rebase = None;
            if pending.plan.anchor != guard.adapter.snapshot().anchor() {
                match rebase_by_witness(guard, &pending) {
                    Ok((plan, certificate)) => {
                        plan_digest = plan.digest.to_string();
                        pending = PendingPlan {
                            plan,
                            digest: plan_digest.clone(),
                            source: pending.source,
                        };
                        witness_rebase = Some(certificate);
                    }
                    Err(reason) => {
                        let mut payload: serde_json::Value =
                            serde_json::from_str(&replay_stale_plan(guard, pending))
                                .unwrap_or_else(|_| json!({"ok": false}));
                        payload["witness_check"] = reason;
                        return payload.to_string();
                    }
                }
            }
            let resource_output_ledger = match objectives::prepare_resource_output_ledger(
                guard,
                &pending.plan,
                &pending.source,
            ) {
                Ok(ledger) => ledger,
                Err(error) => {
                    guard.pending = Some(pending);
                    return dfmcp_error_payload("fortress.commit", &error);
                }
            };
            let evicted_objectives =
                match objectives::objective_evictions(guard, pending.plan.digest) {
                    Ok(evicted) => evicted,
                    Err(error) => {
                        guard.pending = Some(pending);
                        return dfmcp_error_payload("fortress.commit", &error);
                    }
                };
            if guard.shared_members > 1 && plan_sets_pause(&pending.plan, false) {
                let me = guard.session_id;
                guard.leases.unpause_consent.insert(me);
                let votes = guard.leases.unpause_consent.len();
                if votes < guard.shared_members {
                    let members = guard.shared_members;
                    guard.pending = Some(pending);
                    return json!({
                        "ok": false,
                        "error": {
                            "operation": "fortress.commit",
                            "code": "conflict",
                            "message": format!(
                                "unpause consent recorded ({votes} of {members}); the shared fortress stays paused until every member consents"
                            ),
                            "retryable": true,
                            "details": [],
                        },
                        "clock_consent": {"votes": votes, "members": members, "policy": "unanimous_unpause"},
                        "mutation_dispatched": false,
                    })
                    .to_string();
                }
            }
            let leases_before = guard.leases.clone();
            let plan_leases = match acquire_plan_leases(guard, &pending.plan) {
                Ok(leases) => leases,
                Err(error) => {
                    guard.pending = Some(pending);
                    return dfmcp_error_payload("fortress.commit", &error);
                }
            };
            let (_, prepare_ctx) = match next_context(guard) {
                Ok(value) => value,
                Err(error) => {
                    guard.leases = leases_before;
                    guard.pending = Some(pending);
                    return dfmcp_error_payload("fortress.commit", &error);
                }
            };
            match guard.adapter.prepare(&pending.plan, &prepare_ctx) {
                Ok(prepared) => {
                    let (_, commit_ctx) = match next_context(guard) {
                        Ok(value) => value,
                        Err(error) => {
                            guard.leases = leases_before;
                            guard.pending = Some(pending);
                            return dfmcp_error_payload("fortress.commit", &error);
                        }
                    };
                    let sealed = guard
                        .durable_scenario
                        .is_some()
                        .then(|| guard.adapter.snapshot().clone());
                    if let Some(sealed) = sealed.as_ref() {
                        // Original goal and commit are admitted together before
                        // the effect. Neither can disappear after action Done.
                        let source = pending.source.durable();
                        let digest = pending.plan.digest;
                        let intent = pending.plan.intent_id.get();
                        if let Err(error) = with_durable_store(|store| {
                            store.persist_objective_commit(
                                sealed,
                                digest,
                                intent,
                                source,
                                guard.session_id,
                                &evicted_objectives,
                            )
                        }) {
                            guard.leases = leases_before;
                            guard.pending = Some(pending);
                            return dfmcp_error_payload("fortress.commit", &error);
                        }
                        // A subsequent adapter refusal cannot erase the durable
                        // admission. Keep it visible for reconciliation and retry.
                        objectives::install_objective(
                            guard,
                            &pending.plan,
                            pending.source.clone(),
                            &evicted_objectives,
                            resource_output_ledger,
                        );
                    }
                    match guard.adapter.commit(&pending.plan, &prepared, &commit_ctx) {
                        Ok(receipt) => {
                            if sealed.is_some() {
                                guard
                                    .durable_plans
                                    .insert(pending.plan.digest, pending.plan.clone());
                            }
                            if sealed.is_none() {
                                objectives::install_objective(
                                    guard,
                                    &pending.plan,
                                    pending.source.clone(),
                                    &evicted_objectives,
                                    resource_output_ledger,
                                );
                            }
                            guard.last_action =
                                receipt.actions.last().map(|action| action.action_id);
                            guard.last_plan_actions = receipt
                                .actions
                                .iter()
                                .map(|action| action.action_id)
                                .collect();
                            for action in &receipt.actions {
                                if !action_fully_drained(&guard.adapter, action.action_id)
                                    && !guard.open_actions.contains(&action.action_id)
                                {
                                    guard.open_actions.push(action.action_id);
                                }
                                if let Some((_, ids)) =
                                    plan_leases.iter().find(|(step, _)| *step == action.step_id)
                                {
                                    let holder = guard.session_id;
                                    guard
                                        .leases
                                        .by_action
                                        .insert(action.action_id, (holder, ids.clone()));
                                    release_action_leases(guard, action.action_id);
                                }
                            }
                            if plan_sets_pause(&pending.plan, true)
                                || plan_sets_pause(&pending.plan, false)
                            {
                                // A pause resets consensus; an unpause consumed it.
                                guard.leases.unpause_consent.clear();
                            }
                            let authority = plan_authority(&pending.plan);
                            guard
                                .commit_authority
                                .insert(plan_digest.clone(), authority);
                            let snapshot = guard.adapter.snapshot();
                            let paused = snapshot.paused;
                            let payload = json!({
                                "ok": true,
                                "session_id": format!("{}", guard.session_id),
                                "plan_id": format!("{}", receipt.plan_id),
                                "plan_digest": receipt.plan_digest.to_string(),
                                "actions": receipt.actions.iter().map(|action| json!({
                                    "action_id": format!("{}", action.action_id),
                                    "step": action.step_id.get(),
                                    "state": format!("{:?}", action.state),
                                    "work_state": action_work_json(guard, action.action_id),
                                    "message": action.message,
                                })).collect::<Vec<_>>(),
                                "observed_anchor": anchor_json(&receipt.observed_anchor),
                                "paused": paused,
                                "untracked_work": untracked_work_json(guard),
                                "witness_rebase": witness_rebase,
                            });
                            let payload_text = payload.to_string();
                            guard
                                .commit_receipts
                                .insert(plan_digest.clone(), payload_text.clone());
                            if requested_digest != plan_digest {
                                // A retry with the digest the agent sealed must
                                // return this same receipt.
                                guard
                                    .commit_receipts
                                    .insert(requested_digest.clone(), payload_text.clone());
                                if let Some(authority) =
                                    guard.commit_authority.get(&plan_digest).cloned()
                                {
                                    guard
                                        .commit_authority
                                        .insert(requested_digest.clone(), authority);
                                }
                            }
                            payload_text
                        }
                        Err(error) => {
                            guard.leases = leases_before;
                            guard.pending = Some(pending);
                            dfmcp_error_payload("fortress.commit", &error)
                        }
                    }
                }
                Err(error) => {
                    guard.leases = leases_before;
                    guard.pending = Some(pending);
                    dfmcp_error_payload("fortress.commit", &error)
                }
            }
        },
    )
}

// ============================================================================
// fortress.wait
// ============================================================================

/// Observe retained work and return the most recent action's current state.
#[tool(
    description = "Observe retained laboratory work and ready dependent steps in bounded foreground rounds. Returns the most recent action's receipt from the final observed frontier."
)]
pub fn fortress_wait(session_id: Option<String>) -> String {
    wait_with_ticks(session_id, None)
}

fn wait_progress_json(requested: u64, advanced: u64, rounds: u64) -> serde_json::Value {
    json!({
        "requested_game_ticks": requested,
        "advanced_game_ticks": advanced,
        "remaining_game_ticks": requested.saturating_sub(advanced),
        "observation_rounds": rounds,
        "method": "bounded_foreground_dependency_progression",
    })
}

fn wait_receipt_json(
    session: &LabSession,
    receipt: &dfmcp_adapter::ActionReceipt,
) -> serde_json::Value {
    json!({
        "action_id": receipt.action_id.to_string(),
        "step": receipt.step_id.get(),
        "state": format!("{:?}", receipt.state),
        "message": receipt.message,
        "observed_anchor": anchor_json(&receipt.observed_anchor),
        "work_state": action_work_json(session, receipt.action_id),
    })
}

/// Pure projection of the already published progression frontier. It cannot
/// poll an action, dispatch an effect, or sample a retained condition.
fn wait_payload(
    session: &LabSession,
    action_id: Option<ActionId>,
    tracked_order: &[ActionId],
    receipts: &BTreeMap<ActionId, dfmcp_adapter::ActionReceipt>,
    progress: &lab_progress::ForegroundProgress,
    include_world: bool,
) -> Result<serde_json::Value> {
    let advanced = progress.advanced_game_ticks();
    let requested = advanced + progress.remaining_game_ticks();
    let polled_actions = tracked_order
        .iter()
        .filter_map(|id| receipts.get(id))
        .map(|receipt| wait_receipt_json(session, receipt))
        .collect::<Vec<_>>();
    let task = action_id
        .map(|id| {
            session.adapter.action_receipt(id)
                .map(crate::tasks::project_action_receipt)
                .ok_or_else(|| DfmcpError::new(
                    ErrorCode::InvalidRequest, format!("unknown action {id}")))
        })
        .transpose()?;
    let snapshot = session.adapter.snapshot();
    let mut payload = match (action_id, task) {
        (Some(action_id), Some(task)) => json!({
            "ok": true,
            "session_id": session.session_id.to_string(),
            "action_id": action_id.to_string(),
            "task_id": task.task_id,
            "handle_kind": "action_projection_only",
            "modern_task_discovery": format!("df://session/{}/tasks", session.session_id),
            "status": task.status.as_str(),
            "commit_state": format!("{:?}", task.commit_state),
            "summary": task.summary,
            "work_state": action_work_json(session, action_id),
            "observed_anchor": anchor_json(&snapshot.anchor()),
        }),
        _ => json!({
            "ok": true,
            "session_id": session.session_id.to_string(),
            "action_id": null,
            "status": if advanced > 0 { "time_passed" } else { "observed" },
            "summary": if advanced > 0 {
                "game time passed while observing work already in the world"
            } else {
                "retained work was observed at the current frontier without advancing game time"
            },
            "observed_anchor": anchor_json(&snapshot.anchor()),
        }),
    };
    payload["paused"] = json!(snapshot.paused);
    payload["untracked_work"] = untracked_work_json(session);
    payload["polled_actions"] = json!(polled_actions);
    payload["open_actions_remaining"] = json!(session.open_actions.len());
    payload["wait_progress"] = wait_progress_json(requested, advanced, progress.slices());
    payload["wait_progress"]["action_polls"] = json!(progress.polls());
    payload["wait_progress"]["deferred_actions_dispatched"] = json!(progress.dispatched_actions());
    if include_world {
        payload["advanced_game_ticks"] = json!(advanced);
        payload["game_tick"] = json!(snapshot.tick.0);
        payload["world_alerts"] = json!(world_alerts(session));
        payload["civilian_recovery"] = session.civilian_recovery.projection();
        payload["objectives"] = objectives_json(session);
        if !session.carried.is_empty() {
            payload["carried_obligations"] = json!(
                session.carried.iter().map(CarriedStep::to_json).collect::<Vec<_>>()
            );
        }
        if requested > advanced && snapshot.paused {
            payload["blocked"] = json!(
                "the fortress is paused, so no work progresses; commit an unpause plan to let time pass"
            );
        }
    }
    Ok(payload)
}

fn wait_progress_error(
    session: &LabSession,
    error: &DfmcpError,
    progress: &lab_progress::ForegroundProgress,
    receipts: &BTreeMap<ActionId, dfmcp_adapter::ActionReceipt>,
    selected: Option<&selected_plan_wait::SelectedPlanWait>,
    watch: Option<&selected_watch_wait::SelectedWatchWait>,
) -> String {
    let advanced = progress.advanced_game_ticks();
    let requested = advanced + progress.remaining_game_ticks();
    let mut payload = json!({
        "ok": false,
        "error": {
            "operation": "fortress.wait",
            "code": error.code.as_str(),
            "message": error.message,
            "retryable": error.retryable,
            "details": error.details,
        },
        "wait_progress": wait_progress_json(requested, advanced, progress.slices()),
        "advanced_game_ticks": advanced,
        "partial_progress": progress.slices() > 0,
    });
    payload["wait_progress"]["action_polls"] = json!(progress.polls());
    payload["wait_progress"]["deferred_actions_dispatched"] = json!(progress.dispatched_actions());
    // Earlier complete slices are real progress even when this wait refuses.
    // Disclose their current frontier only under current observation authority.
    let ctx = context_for(session, session.next_request_id);
    if authorize_entry(&ctx, Capability::Observe, RiskTier::ReadOnly).is_ok() {
        let snapshot = session.adapter.snapshot();
        payload["session_id"] = json!(session.session_id.to_string());
        payload["observed_anchor"] = anchor_json(&snapshot.anchor());
        payload["paused"] = json!(snapshot.paused);
        payload["game_tick"] = json!(snapshot.tick.0);
        payload["untracked_work"] = untracked_work_json(session);
        payload["polled_actions"] = json!(
            receipts
                .values()
                .map(|receipt| wait_receipt_json(session, receipt))
                .collect::<Vec<_>>()
        );
        payload["open_actions_remaining"] = json!(session.open_actions.len());
        payload["durability_pending"] = json!(session.durability_fault.is_some());
        if !session.carried.is_empty() {
            payload["carried_obligations"] = json!(
                session
                    .carried
                    .iter()
                    .map(CarriedStep::to_json)
                    .collect::<Vec<_>>()
            );
        }
        if let Some(action_id) = session.last_action {
            payload["action_id"] = json!(action_id.to_string());
            payload["work_state"] = action_work_json(session, action_id);
            if let Some(receipt) = session.adapter.action_receipt(action_id) {
                payload["commit_state"] = json!(format!("{:?}", receipt.state));
            }
        }
        if authorize_entry(&ctx, Capability::Query, RiskTier::ReadOnly).is_ok() {
            if let Some(selected) = selected {
                payload["selected_plan_wait"] = selected.projection(session, false);
            }
            if let Some(watch) = watch {
                payload["selected_watch_wait"] = watch.projection(session, false);
            }
            // Partial errors never certify a complete selected outcome. Each
            // last accepted sample retains its own anchor, distinct from the
            // current world; earlier published game time is not rolled back.
        }
    }
    payload.to_string()
}

/// Recovered work has no live adapter action handle, but its original cadence
/// and deadline must constrain the same foreground observation loop.
fn carried_wait_ceiling(session: &LabSession) -> Option<u64> {
    let now = session.adapter.snapshot().tick;
    session
        .carried
        .iter()
        .filter(|step| !step.is_final())
        .filter_map(|step| step.work_step.as_ref()?.obligation.as_ref())
        .map(|spec| {
            let cadence = spec.poll_interval_ticks.max(1);
            let until_deadline = spec.deadline_tick.0.saturating_sub(now.0);
            if until_deadline == 0 {
                cadence
            } else {
                cadence.min(until_deadline)
            }
        })
        .min()
}

/// The watch owner has already admitted its complete Agent Turn. Its caller
/// must return Published unchanged, with no subsequent output fitting.
pub(crate) enum WaitPublication {
    Pending(String),
    Published(String),
}

impl WaitPublication {
    fn into_response(self) -> String {
        match self {
            Self::Pending(response) | Self::Published(response) => response,
        }
    }
}

/// The watch path publishes each world frontier before its candidate query
/// state. Avoid a second implicit durable write after that publication.
fn with_wait_session<T>(
    session: &Arc<Mutex<LabSession>>,
    watch_selected: bool,
    poisoned: impl FnOnce() -> T,
    body: impl FnOnce(&mut LabSession) -> T,
) -> T {
    if !watch_selected {
        return with_session(session, poisoned, body);
    }
    let _admission = match LAB_ADMISSION.read() {
        Ok(guard) => guard,
        Err(_) => return poisoned(),
    };
    if ensure_durable_owner(session).is_err() {
        return poisoned();
    }
    with_session_view(session, poisoned, body, false)
}

/// Let authorized laboratory time pass in bounded slices, observing all owned
/// work in dependency order and publishing each frontier before continuing.
pub(crate) fn wait_with_ticks(session_id: Option<String>, max_game_ticks: Option<u64>) -> String {
    wait_with_selection(session_id, max_game_ticks, None, None).into_response()
}

/// Progress the same owned work, stopping at one exact original plan's goal,
/// proof and physical-work outcome instead of spending the whole tick cap.
pub(crate) fn wait_for_plan(
    session_id: Option<String>,
    max_game_ticks: Option<u64>,
    plan_digest: String,
) -> String {
    let digest = match selected_plan_wait::parse_digest(&plan_digest) {
        Ok(digest) => digest,
        Err(error) => return dfmcp_error_payload("fortress.wait", &error),
    };
    wait_with_selection(session_id, max_game_ticks, Some(digest), None).into_response()
}

/// Select one exact session-owned laboratory condition, preserving its original
/// predicates, deadline, cadence, stability policy and evidence source.
pub(crate) fn wait_for_watch(
    session_id: Option<String>,
    max_game_ticks: Option<u64>,
    watch_id: String,
) -> WaitPublication {
    wait_with_selection(session_id, max_game_ticks, None, Some(watch_id))
}

fn wait_with_selection(
    session_id: Option<String>,
    max_game_ticks: Option<u64>,
    selected_digest: Option<Digest32>,
    watch_id: Option<String>,
) -> WaitPublication {
    let started = std::time::Instant::now();
    let session = match resolve_session(session_id) {
        Ok(value) => value,
        Err(error) => return WaitPublication::Pending(dfmcp_error_payload("fortress.wait", &error)),
    };
    let watch_selected = watch_id.is_some();
    let mut published = false;
    let mut response_budget = None;
    let response = with_wait_session(
        &session,
        watch_selected,
        || mutex_poisoned_payload("fortress.wait"),
        |guard| {
            if watch_selected {
                response_budget = Some(guard.budget);
            }
            if let Err(error) = durability_gate(guard) {
                return dfmcp_error_payload("fortress.wait", &error);
            }
            let (_, entry_ctx) = match next_context(guard) {
                Ok(value) => value,
                Err(error) => return dfmcp_error_payload("fortress.wait", &error),
            };
            if let Err(error) = authorize_entry(&entry_ctx, Capability::Observe, RiskTier::ReadOnly)
            {
                return dfmcp_error_payload("fortress.wait", &error);
            }
            let action_id = guard.last_action;
            // Without any committed action, a bounded wait still lets game
            // time pass for work that already lives in the world (for
            // example designations carried across a durable restart).
            if action_id.is_none()
                && guard.carried.iter().all(CarriedStep::is_final)
                && max_game_ticks.is_none_or(|ticks| ticks == 0)
                && !watch_selected
            {
                return coded_error_payload(
                    "fortress.wait",
                    ErrorCode::Conflict,
                    "no committed or recovering action yet; call fortress_commit first, or pass max_game_ticks to let time pass",
                );
            }
            let requested_ticks = max_game_ticks.unwrap_or(0);
            if requested_ticks > guard.budget.max_game_ticks {
                return coded_error_payload(
                    "fortress.wait",
                    ErrorCode::BudgetExceeded,
                    "max_game_ticks exceeds the session's negotiated game-tick budget",
                );
            }
            let paused = guard.adapter.snapshot().paused;
            if let Err(error) = authorize_wait_advance(&entry_ctx, requested_ticks, paused) {
                return dfmcp_error_payload("fortress.wait", &error);
            }
            let mut selected = match selected_digest
                .map(|digest| selected_plan_wait::SelectedPlanWait::new(guard, digest, started))
                .transpose()
            {
                Ok(selected) => selected,
                Err(error) => return dfmcp_error_payload("fortress.wait", &error),
            };
            let mut watcher = match watch_id
                .map(|handle| selected_watch_wait::SelectedWatchWait::new(
                    guard, handle, requested_ticks, started,
                ))
                .transpose()
            {
                Ok(watcher) => watcher,
                Err(error) => return dfmcp_error_payload("fortress.wait", &error),
            };
            let admitted_progress = if selected.is_some() || watcher.is_some() {
                lab_progress::ForegroundProgress::new_with_start(
                    &guard.adapter,
                    requested_ticks,
                    &entry_ctx,
                    started,
                )
            } else {
                lab_progress::ForegroundProgress::new(&guard.adapter, requested_ticks, &entry_ctx)
            };
            let mut progress = match admitted_progress {
                Ok(progress) => progress,
                Err(error) => return dfmcp_error_payload("fortress.wait", &error),
            };
            // A wait is one bounded foreground operation. Each complete slice
            // observes prerequisites before dispatching their successors, then
            // publishes world, action proofs and original-goal history before
            // another slice may consume time or cause an effect.
            let tracked_order = guard.open_actions.clone();
            let mut latest_receipts = BTreeMap::new();
            // Admit the complete first watch result before any game effect.
            if let Some(watch) = &mut watcher {
                let payload = match wait_payload(
                    guard, action_id, &tracked_order, &latest_receipts, &progress, true,
                ) {
                    Ok(payload) => payload,
                    Err(error) => return wait_progress_error(
                        guard, &error, &progress, &latest_receipts, selected.as_ref(), Some(watch),
                    ),
                };
                if let Err(error) = watch.sample(guard, payload) {
                    return wait_progress_error(
                        guard, &error, &progress, &latest_receipts, selected.as_ref(), Some(watch),
                    );
                }
            }
            loop {
                if let Some(watch) = &watcher {
                    // An accepted stop packet is final. Do not introduce a
                    // fallible check after publishing its retained watch root.
                    if watch.stopped() {
                        break;
                    }
                    if let Err(error) = watch.check(guard) {
                        return wait_progress_error(
                            guard, &error, &progress, &latest_receipts, selected.as_ref(), Some(watch),
                        );
                    }
                }
                if let Some(selected) = &selected {
                    if let Err(error) = selected.check(guard) {
                        return wait_progress_error(
                            guard,
                            &error,
                            &progress,
                            &latest_receipts,
                            Some(selected),
                            watcher.as_ref(),
                        );
                    }
                    if selected.stopped() {
                        break;
                    }
                }
                let (_, ctx) = match next_context(guard) {
                    Ok(value) => value,
                    Err(error) => {
                        return wait_progress_error(guard, &error, &progress, &latest_receipts, selected.as_ref(), watcher.as_ref());
                    }
                };
                let carried_ceiling = carried_wait_ceiling(guard);
                let ceiling = selected.as_ref().map_or(carried_ceiling, |selected| {
                    selected.ceiling(guard.adapter.snapshot().tick.0, carried_ceiling)
                });
                let ceiling = watcher.as_ref().map_or(ceiling, |watch| {
                    watch.ceiling(guard.adapter.snapshot().tick.0, ceiling)
                });
                let tracked = guard.open_actions.clone();
                let slice = match progress.step(&mut guard.adapter, &tracked, &ctx, ceiling)
                {
                    Ok(Some(slice)) => slice,
                    Ok(None) => break,
                    Err(error) => {
                        return wait_progress_error(guard, &error, &progress, &latest_receipts, selected.as_ref(), watcher.as_ref());
                    }
                };
                let finished = slice.finished;
                for receipt in slice.receipts {
                    latest_receipts.insert(receipt.action_id, receipt);
                }
                let mut still_open = Vec::new();
                for open in tracked {
                    if action_fully_drained(&guard.adapter, open) {
                        release_action_leases(guard, open);
                    } else {
                        still_open.push(open);
                    }
                }
                guard.open_actions = still_open;
                if let Err(error) = observe_carried(guard) {
                    // The accepted adapter slice is already real even when
                    // recovered proof inspection refuses. Watch ownership has
                    // no trailing implicit save, so publish or fence this
                    // frontier before reporting its partial progress.
                    if watcher.is_some() {
                        persist_durable_head(guard);
                        if guard.durability_fault.is_none() {
                            remember_version(guard);
                        }
                    }
                    return wait_progress_error(
                        guard, &error, &progress, &latest_receipts,
                        selected.as_ref(), watcher.as_ref(),
                    );
                }
                persist_durable_head(guard);
                if let Some(fault) = &guard.durability_fault {
                    let error = DfmcpError::new(
                        ErrorCode::AdapterUnavailable,
                        format!(
                            "wait stopped at an unpublished durable frontier ({fault}); further progress requires successful persistence"
                        ),
                    );
                    return wait_progress_error(guard, &error, &progress, &latest_receipts, selected.as_ref(), watcher.as_ref());
                }
                remember_version(guard);
                if let Some(selected) = &mut selected
                    && let Err(error) = selected.inspect(guard)
                {
                    return wait_progress_error(
                        guard,
                        &error,
                        &progress,
                        &latest_receipts,
                        Some(selected),
                        watcher.as_ref(),
                    );
                }
                if let Some(watch) = &mut watcher {
                    let payload = match wait_payload(
                        guard, action_id, &tracked_order, &latest_receipts, &progress, true,
                    ) {
                        Ok(payload) => payload,
                        Err(error) => return wait_progress_error(
                            guard, &error, &progress, &latest_receipts, selected.as_ref(), Some(watch),
                        ),
                    };
                    if let Err(error) = watch.sample(guard, payload) {
                        return wait_progress_error(
                            guard, &error, &progress, &latest_receipts, selected.as_ref(), Some(watch),
                        );
                    }
                }
                if finished {
                    break;
                }
            }
            if let Some(watch) = &mut watcher {
                if let Some(response) = watch.take_published() {
                    published = true;
                    return response;
                }
                return wait_progress_error(
                    guard,
                    &DfmcpError::new(ErrorCode::InternalInvariantViolation,
                        "selected watch wait has no admitted publication"),
                    &progress, &latest_receipts, selected.as_ref(), Some(watch),
                );
            }
            let mut payload = match wait_payload(
                guard, action_id, &tracked_order, &latest_receipts, &progress,
                max_game_ticks.is_some(),
            ) {
                Ok(payload) => payload,
                Err(error) => return wait_progress_error(
                    guard, &error, &progress, &latest_receipts, selected.as_ref(), None,
                ),
            };
            if let Some(selected) = &mut selected {
                selected.finish(guard.adapter.snapshot().paused);
                payload["selected_plan_wait"] = selected.projection(guard, true);
            }
            if let Some(selected) = &selected
                && let Err(error) = selected.check(guard)
            {
                return wait_progress_error(
                    guard,
                    &error,
                    &progress,
                    &latest_receipts,
                    Some(selected),
                    None,
                );
            }
            payload.to_string()
        },
    );
    if published {
        WaitPublication::Published(response)
    } else {
        // Error projection must honor the current owning budget, even when
        // the facade's orientation cache still contains the opening budget.
        let response = if let Some(budget) = response_budget
            && let Ok(mut payload) = serde_json::from_str::<serde_json::Value>(&response)
        {
            payload["wait_response_budget"] = json!({
                "max_bytes": budget.max_bytes,
                "max_output_tokens": budget.max_output_tokens,
                "maximum_response_bytes": budget.max_bytes.min(u64::from(budget.max_output_tokens) * 4),
            });
            payload.to_string()
        } else {
            response
        };
        WaitPublication::Pending(response)
    }
}

// ============================================================================
// fortress.cancel
// ============================================================================

/// Request, drain, and finalize cancellation of the most recent action with
/// authorized compensation. Cancellation never deletes records.
#[tool(
    description = "Cancel the most recent committed action in this session: request, drain, compensate when authorized, and finalize."
)]
pub fn fortress_cancel(session_id: Option<String>, mode: Option<String>) -> String {
    cancel_in_scope(session_id, mode, None)
}

/// Cancel either the most recent action (`scope` omitted or `last_action`,
/// the historical behaviour), the most recent plan (`scope="plan"`), or all
/// retained open work owned by this session (`scope="session"`). Drain
/// the oldest retained plan with `scope="oldest_open_plan"` when the whole
/// session exceeds a call budget. Dependents drain before prerequisites.
pub(crate) fn cancel_in_scope(
    session_id: Option<String>,
    mode: Option<String>,
    scope: Option<String>,
) -> String {
    let selected_plan = match scope.as_deref().filter(|value| value.starts_with("plan:")) {
        Some(raw) => match plan_cancellation::parse_scope(raw) {
            Ok(digest) => Some(digest),
            Err(error) => return dfmcp_error_payload("fortress.cancel", &error),
        },
        None => None,
    };
    let drain_scope = match scope.as_deref() {
        None | Some("last_action") => None,
        Some("plan") => Some("plan"),
        Some("session") => Some("session"),
        Some("oldest_open_plan") => Some("oldest_open_plan"),
        Some(_) if selected_plan.is_some() => None,
        Some(_) => {
            return coded_error_payload(
                "fortress.cancel",
                ErrorCode::InvalidRequest,
                "unsupported cancellation scope; use last_action, plan, plan:<original-plan-digest>, oldest_open_plan or session",
            );
        }
    };
    if mode
        .as_ref()
        .is_some_and(|value| value.len() > MAX_MODE_BYTES)
    {
        return coded_error_payload(
            "fortress.cancel",
            ErrorCode::BudgetExceeded,
            "cancellation mode exceeds its explicit byte bound",
        );
    }
    let session = match resolve_session(session_id) {
        Ok(value) => value,
        Err(error) => return dfmcp_error_payload("fortress.cancel", &error),
    };
    with_session(
        &session,
        || mutex_poisoned_payload("fortress.cancel"),
        |guard| {
            if let Err(error) = durability_gate(guard) {
                return dfmcp_error_payload("fortress.cancel", &error);
            }
            {
                let (_, entry_ctx) = match next_context(guard) {
                    Ok(value) => value,
                    Err(error) => return dfmcp_error_payload("fortress.cancel", &error),
                };
                if let Err(error) =
                    authorize_entry(&entry_ctx, Capability::Observe, RiskTier::ReadOnly)
                {
                    return dfmcp_error_payload("fortress.cancel", &error);
                }
                // Cancellation changes the world; a read-only session is
                // refused before any session state is disclosed. Each drained
                // action is still authorized against its own capability.
                if guard
                    .grants
                    .iter()
                    .all(|grant| grant.max_risk == RiskTier::ReadOnly)
                {
                    return coded_error_payload(
                        "fortress.cancel",
                        ErrorCode::CapabilityDenied,
                        "cancellation requires a capability above read_only",
                    );
                }
            }
            let cancel_mode = match mode.as_deref() {
                Some("emergency_pause_and_drain") => CancelMode::EmergencyPauseAndDrain,
                Some("stop_future_steps") => CancelMode::StopFutureSteps,
                Some("compensate_reversible") | None => CancelMode::CompensateReversible,
                Some(other) => {
                    return error_payload(
                        "fortress.cancel",
                        &format!("unsupported cancellation mode {other:?}"),
                    );
                }
            };
            if let Some(digest) = selected_plan {
                return plan_cancellation::cancel(guard, cancel_mode, digest);
            }
            if let Some(scope) = drain_scope {
                if scope == "oldest_open_plan" {
                    let Some(oldest) = guard
                        .open_actions
                        .iter()
                        .copied()
                        .find(|id| !action_fully_drained(&guard.adapter, *id))
                    else {
                        return coded_error_payload(
                            "fortress.cancel",
                            ErrorCode::Conflict,
                            "no retained open plan remains in this session",
                        );
                    };
                    let Some(original) = guard.adapter.action_plan_receipt(oldest) else {
                        return cancel_action_error_payload(
                            guard,
                            oldest,
                            &DfmcpError::new(
                                ErrorCode::Conflict,
                                "the oldest open action has no retained original commit; reconciliation is required",
                            ),
                        );
                    };
                    let digest = original.plan_digest.to_string();
                    let actions = original.actions.iter().map(|row| row.action_id).collect();
                    let mut payload: serde_json::Value =
                        serde_json::from_str(&drain_actions(guard, cancel_mode, scope, actions))
                            .unwrap_or_else(|_| json!({"ok": false}));
                    payload["plan_digest"] = json!(digest);
                    return payload.to_string();
                }
                let actions = if scope == "plan" {
                    guard.last_plan_actions.clone()
                } else {
                    guard.open_actions.clone()
                };
                return drain_actions(guard, cancel_mode, scope, actions);
            }
            let Some(action_id) = guard.last_action else {
                return coded_error_payload(
                    "fortress.cancel",
                    ErrorCode::Conflict,
                    "no committed action to cancel; call fortress_commit first",
                );
            };
            if let Err(error) = authorize_drain_budget(guard, &[action_id], cancel_mode) {
                return cancel_action_error_payload(guard, action_id, &error);
            }
            let (_, ctx) = match next_context(guard) {
                Ok(value) => value,
                Err(error) => return cancel_action_error_payload(guard, action_id, &error),
            };
            if let Some(receipt) = guard.adapter.action_receipt(action_id).cloned()
                && receipt.state.is_terminal()
                && !action_fully_drained(&guard.adapter, action_id)
            {
                return match guard
                    .adapter
                    .drain_action_work_in_mode(action_id, cancel_mode, &ctx)
                {
                    Ok(drain) => {
                        reset_emergency_unpause_consent(guard, cancel_mode);
                        release_action_leases(guard, action_id);
                        retain_open_actions(guard);
                        json!({
                            "ok": true,
                            "session_id": guard.session_id.to_string(),
                            "action_id": action_id.to_string(),
                            "requested_state": format!("{:?}", receipt.state),
                            "final_state": format!("{:?}", receipt.state),
                            "physical_drain": effect_drain_json(&drain),
                            "work_state": action_work_json(guard, action_id),
                            "paused": guard.adapter.snapshot().paused,
                            "observed_anchor": anchor_json(&guard.adapter.snapshot().anchor()),
                        })
                        .to_string()
                    }
                    Err(error) => cancel_action_error_payload(guard, action_id, &error),
                };
            }
            match guard.adapter.request_cancel(action_id, cancel_mode, &ctx) {
                Ok(request) => {
                    reset_emergency_unpause_consent(guard, cancel_mode);
                    let (_, finalize_ctx) = match next_context(guard) {
                        Ok(value) => value,
                        Err(error) => return cancel_action_error_payload(guard, action_id, &error),
                    };
                    match guard.adapter.finalize_cancel(action_id, &finalize_ctx) {
                        Ok(finalized) => {
                            release_action_leases(guard, action_id);
                            retain_open_actions(guard);
                            json!({
                    "ok": true,
                    "session_id": format!("{}", guard.session_id),
                    "action_id": format!("{}", finalized.action_id),
                    "requested_state": format!("{:?}", request.state),
                    "final_state": format!("{:?}", finalized.state),
                    "work_state": action_work_json(guard, action_id),
                    "paused": guard.adapter.snapshot().paused,
                    "note": "cancellation is request/drain/compensate/finalize; records are never deleted",
                })
                .to_string()
                        }
                        Err(error) => cancel_action_error_payload(guard, action_id, &error),
                    }
                }
                Err(error) => cancel_action_error_payload(guard, action_id, &error),
            }
        },
    )
}

fn drain_actions(
    guard: &mut LabSession,
    cancel_mode: CancelMode,
    scope: &str,
    actions: Vec<ActionId>,
) -> String {
    if let Err(error) = authorize_drain_budget(guard, &actions, cancel_mode) {
        let mut payload: serde_json::Value =
            serde_json::from_str(&dfmcp_error_payload("fortress.cancel", &error))
                .unwrap_or_else(|_| json!({"ok": false}));
        let unknown = actions
            .iter()
            .filter(|id| {
                !guard
                    .adapter
                    .action_work_state(**id)
                    .is_ok_and(|state| state.is_quiescent())
            })
            .count();
        let pending = actions
            .iter()
            .filter(|id| {
                !guard
                    .adapter
                    .action_receipt(**id)
                    .is_some_and(|receipt| receipt.state.is_terminal())
            })
            .count();
        payload["scope"] = json!(scope);
        payload["drain_progress"] = json!({
            "actions_total": actions.len(), "drained": 0,
            "remaining_nonterminal": pending, "remaining_work": unknown,
            "quiescent": false, "stage": "admission_refused",
        });
        payload["untracked_work"] = untracked_work_json(guard);
        payload["finalize_certificate"] = serde_json::Value::Null;
        payload["observed_anchor"] = anchor_json(&guard.adapter.snapshot().anchor());
        return payload.to_string();
    }
    let mut steps = Vec::with_capacity(actions.len());
    let mut already_terminal = 0usize;
    let mut compensated = 0usize;
    let mut cancelled = 0usize;
    let mut terminal_work_stopped = 0usize;
    let mut failure = None;
    // Never poll eligibility during cancellation: prepared successors must not
    // dispatch. Stop dependents before withdrawing their prerequisites.
    for action_id in actions.iter().rev().copied() {
        let before = match guard.adapter.action_receipt(action_id).cloned() {
            Some(receipt) => receipt,
            None => {
                failure = Some(DfmcpError::new(
                    ErrorCode::Conflict,
                    "the original plan action is no longer retained; quiescence is unresolved",
                ));
                break;
            }
        };
        let work_before = action_work_json(guard, action_id);
        let mut cleanup = None;
        let mut drained = false;
        let outcome = if before.state.is_terminal() {
            already_terminal += 1;
            if action_fully_drained(&guard.adapter, action_id) {
                Ok(())
            } else {
                next_context(guard)
                    .and_then(|(_, ctx)| {
                        guard
                            .adapter
                            .drain_action_work_in_mode(action_id, cancel_mode, &ctx)
                    })
                    .map(|receipt| {
                        reset_emergency_unpause_consent(guard, cancel_mode);
                        drained = receipt.stopped_work;
                        terminal_work_stopped += usize::from(receipt.stopped_work);
                        cleanup = Some(effect_drain_json(&receipt));
                    })
            }
        } else {
            next_context(guard)
                .and_then(|(_, ctx)| guard.adapter.request_cancel(action_id, cancel_mode, &ctx))
                .inspect(|_| {
                    reset_emergency_unpause_consent(guard, cancel_mode);
                })
                .and_then(|_| next_context(guard))
                .and_then(|(_, ctx)| guard.adapter.finalize_cancel(action_id, &ctx))
                .map(|receipt| {
                    drained = true;
                    if receipt.state == CommitState::Compensated {
                        compensated += 1;
                    } else {
                        cancelled += 1;
                    }
                })
        };
        let after = guard.adapter.action_receipt(action_id);
        steps.push(json!({
            "action_id": action_id.to_string(),
            "step": before.step_id.get(),
            "before": format!("{:?}", before.state),
            "after": after.map(|receipt| format!("{:?}", receipt.state)),
            "proof_receipt_digest": after.map(|receipt| receipt.adapter_receipt_digest.to_hex()),
            "proof_anchor": after.map(|receipt| anchor_json(&receipt.observed_anchor)),
            "work_before": work_before,
            "work_after": action_work_json(guard, action_id),
            "physical_drain": cleanup,
            "drained": drained,
        }));
        if let Err(error) = outcome {
            failure = Some(error);
            break;
        }
    }
    let mut remaining_nonterminal = 0usize;
    let mut remaining_work = 0usize;
    for action_id in &actions {
        remaining_nonterminal += usize::from(
            !guard
                .adapter
                .action_receipt(*action_id)
                .is_some_and(|receipt| receipt.state.is_terminal()),
        );
        remaining_work += usize::from(
            !guard
                .adapter
                .action_work_state(*action_id)
                .is_ok_and(|state| state.is_quiescent()),
        );
        release_action_leases(guard, *action_id);
        if !action_fully_drained(&guard.adapter, *action_id)
            && !guard.open_actions.contains(action_id)
        {
            guard.open_actions.push(*action_id);
        }
    }
    retain_open_actions(guard);
    steps.reverse();
    let untracked = untracked_work_json(guard);
    let untracked_quiet = untracked["quiescent"] == true;
    let remaining_carried = guard.carried.iter().filter(|step| !step.is_final()).count();
    let session_quiet = scope != "session" || (untracked_quiet && remaining_carried == 0);
    let quiescent =
        failure.is_none() && remaining_nonterminal == 0 && remaining_work == 0 && session_quiet;
    let anchor = anchor_json(&guard.adapter.snapshot().anchor());
    let finalize_certificate = quiescent.then(|| {
        let canonical = json!({
            "schema": "dfmcp-lab-plan-drain-certificate-v2",
            "scope": scope,
            "actions": actions.iter().map(ToString::to_string).collect::<Vec<_>>(),
            "steps": steps,
            "anchor": anchor,
        });
        json!({
            "digest": Digest32::of_bytes(canonical.to_string().as_bytes()).to_hex(),
            "anchor": anchor,
            "statement": "every original action is terminal and its physical work is proven quiescent",
        })
    });
    let progress = json!({
        "actions_total": actions.len(),
        "already_terminal": already_terminal,
        "drained": compensated + cancelled + terminal_work_stopped,
        "compensated": compensated,
        "cancelled": cancelled,
        "terminal_work_stopped": terminal_work_stopped,
        "remaining_nonterminal": remaining_nonterminal,
        "remaining_work": remaining_work,
        "untracked_work_quiescent": untracked_quiet,
        "remaining_carried": remaining_carried,
        "quiescent": quiescent,
    });
    match failure {
        None => json!({
            "ok": true,
            "session_id": guard.session_id.to_string(),
            "scope": scope,
            "drain_progress": progress,
            "paused": guard.adapter.snapshot().paused,
            "untracked_work": untracked,
            "steps": steps,
            "finalize_certificate": finalize_certificate,
            "observed_anchor": anchor,
            "note": "terminal goal receipts remain history; remaining physical work is stopped under fresh authority with separate evidence",
        }).to_string(),
        Some(error) => {
            let mut payload: serde_json::Value =
                serde_json::from_str(&dfmcp_error_payload("fortress.cancel", &error))
                    .unwrap_or_else(|_| json!({"ok": false}));
            payload["scope"] = json!(scope);
            payload["paused"] = json!(guard.adapter.snapshot().paused);
            payload["untracked_work"] = untracked;
            payload["drain_progress"] = progress;
            payload["steps"] = json!(steps);
            payload["finalize_certificate"] = serde_json::Value::Null;
            payload["observed_anchor"] = anchor;
            payload.to_string()
        }
    }
}

// ============================================================================
// fortress.checkpoint
// ============================================================================

/// Create a content-addressed, labeled recovery point.
#[tool(
    description = "Create a labeled checkpoint for the open session: a content-addressed recovery point with an evidence record."
)]
pub fn fortress_checkpoint(session_id: Option<String>, label: Option<String>) -> String {
    let session = match resolve_session(session_id) {
        Ok(value) => value,
        Err(error) => return dfmcp_error_payload("fortress.checkpoint", &error),
    };
    let label = label.map_or_else(|| "manual".to_owned(), |value| value);
    if label.len() > MAX_LABEL_BYTES {
        return coded_error_payload(
            "fortress.checkpoint",
            ErrorCode::BudgetExceeded,
            "checkpoint label exceeds its explicit byte bound",
        );
    }
    with_session(
        &session,
        || mutex_poisoned_payload("fortress.checkpoint"),
        |guard| {
            if let Err(error) = durability_gate(guard) {
                return dfmcp_error_payload("fortress.checkpoint", &error);
            }
            let (_, ctx) = match next_context(guard) {
                Ok(value) => value,
                Err(error) => return dfmcp_error_payload("fortress.checkpoint", &error),
            };
            if let Err(error) = authorize_entry(&ctx, Capability::Checkpoint, RiskTier::Reversible)
            {
                return dfmcp_error_payload("fortress.checkpoint", &error);
            }
            match guard.adapter.checkpoint(&label, &ctx) {
                Ok(receipt) => {
                    let (durable, durability_error) = if guard.durable_scenario.is_some() {
                        let persisted = guard
                            .adapter
                            .checkpoint_snapshot(receipt.checkpoint_id)
                            .cloned()
                            .ok_or_else(|| {
                                DfmcpError::new(
                                    ErrorCode::InternalInvariantViolation,
                                    "checkpoint vanished before it could be persisted",
                                )
                            })
                            .and_then(|snapshot| {
                                with_durable_store(|store| {
                                    store.persist_checkpoint(
                                        receipt.checkpoint_id,
                                        &receipt.label,
                                        &snapshot,
                                    )
                                })
                            });
                        match persisted {
                            Ok(()) => (true, None),
                            Err(error) => (false, Some(error.message)),
                        }
                    } else {
                        (receipt.durable, None)
                    };
                    json!({
                        "ok": true,
                        "session_id": format!("{}", guard.session_id),
                        "checkpoint_id": format!("{}", receipt.checkpoint_id),
                        "label": receipt.label,
                        "content_digest": receipt.content_digest.to_string(),
                        "durable": durable,
                        "durability_error": durability_error,
                        "anchor": anchor_json(&receipt.anchor),
                    })
                    .to_string()
                }
                Err(error) => dfmcp_error_payload("fortress.checkpoint", &error),
            }
        },
    )
}

// ============================================================================
// fortress.restore
// ============================================================================

/// Restore a checkpoint into a new observation epoch, invalidating stale
/// plans and action handles.
#[tool(
    description = "Restore a checkpoint by id into the open session. Creates a new observation epoch; stale plans and action handles are invalidated."
)]
pub fn fortress_restore(session_id: Option<String>, checkpoint_id: String) -> String {
    if checkpoint_id.len() != U128_HEX_ID_BYTES
        || !checkpoint_id.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return error_payload(
            "fortress.restore",
            "checkpoint_id must be the 32-character hexadecimal identifier returned by fortress_checkpoint",
        );
    }
    let parsed_checkpoint = match u128::from_str_radix(&checkpoint_id, 16) {
        Ok(value) => value,
        Err(_) => {
            return error_payload(
                "fortress.restore",
                "checkpoint_id is not a valid hexadecimal u128 identifier",
            );
        }
    };
    if parsed_checkpoint == 0 {
        return error_payload("fortress.restore", "checkpoint_id zero is reserved");
    }
    let session = match resolve_session(session_id) {
        Ok(value) => value,
        Err(error) => return dfmcp_error_payload("fortress.restore", &error),
    };
    with_session(
        &session,
        || mutex_poisoned_payload("fortress.restore"),
        |guard| {
            if let Err(error) = durability_gate(guard) {
                return dfmcp_error_payload("fortress.restore", &error);
            }
            let (_, ctx) = match next_context(guard) {
                Ok(value) => value,
                Err(error) => return dfmcp_error_payload("fortress.restore", &error),
            };
            if let Err(error) = authorize_entry(&ctx, Capability::Restore, RiskTier::Guarded) {
                return dfmcp_error_payload("fortress.restore", &error);
            }
            if guard.shared_members > 1 {
                return coded_error_payload(
                    "fortress.restore",
                    ErrorCode::Conflict,
                    "restore would rewrite a fortress other agents share; only a sole member may restore",
                );
            }
            // Capture retirement before changing the world. Only unfinished
            // dispatches are abandoned; earlier terminal proof stays intact.
            let restored_commits = if guard.durable_scenario.is_some() {
                match with_durable_store(|store| {
                    Ok(store
                        .commits(guard.fortress_id)
                        .map(|commit| {
                            (
                                commit.plan_digest,
                                commit
                                    .steps
                                    .iter()
                                    .filter(|(_, state)| state.as_str() == "dispatched")
                                    .map(|(step, _)| *step)
                                    .collect(),
                            )
                        })
                        .collect::<BTreeMap<Digest32, Vec<u32>>>())
                }) {
                    Ok(commits) => commits,
                    Err(error) => return dfmcp_error_payload("fortress.restore", &error),
                }
            } else {
                BTreeMap::new()
            };
            match guard
                .adapter
                .restore(CheckpointId::new(parsed_checkpoint), &ctx)
            {
                Ok(receipt) => {
                    guard.pending = None;
                    guard.last_action = None;
                    guard.leases = LeaseBook::default();
                    // The adapter forgot every pre-restore action; so must the session,
                    // or every later wait would poll handles that no longer exist.
                    guard.last_plan_actions.clear();
                    guard.open_actions.clear();
                    guard.commit_receipts.clear();
                    objectives::stage_objective_abandonment(guard);
                    if guard.durable_scenario.is_some() {
                        // Retain the retirement set after an unsuccessful save
                        // so another call can retry the same atomic frontier.
                        guard.durable_restore = restored_commits;
                        guard.durable_plans.clear();
                        guard.carried.clear();
                    }
                    json!({
                    "ok": true,
                    "session_id": format!("{}", guard.session_id),
                    "checkpoint_id": format!("{}", receipt.checkpoint_id),
                    "prior_anchor": anchor_json(&receipt.prior_anchor),
                    "restored_anchor": anchor_json(&receipt.restored_anchor),
                    "content_digest": receipt.content_digest.to_string(),
                    "note": "new observation epoch; pending plans and action handles were invalidated",
                    "objectives": objectives_json(guard),
                    "untracked_work": untracked_work_json(guard),
                })
                .to_string()
                }
                Err(error) => dfmcp_error_payload("fortress.restore", &error),
            }
        },
    )
}

// ============================================================================
// fortress.explain
// ============================================================================

/// Explain recent state transitions or graph dependencies for a specific entity.
#[tool(
    description = "Explain recent transcript events or numeric entity graph dependencies. Typed entity_id selectors plan:<64-lowercase-plan-digest> and goal:<digest> inspect an exact pending/retained plan or committed original goal without advancing proof. Reports separate current terminal requirements, original project/reserve truth, historical achievement, receipts and physical work. Follow the returned original_steps, original_requirements and next requests for bounded pages pinned to the exact typed plan and current source. Current whole-fortress Observe and Query grants are required; no plan, effect, clock or historical proof is changed."
)]
pub fn fortress_explain(session_id: Option<String>, entity_id: Option<String>) -> String {
    if let Some(subject) = entity_id
        .as_deref()
        .filter(|raw| goal_explanation::is_subject(raw))
    {
        return goal_explanation::execute(session_id, subject);
    }
    let session = match resolve_session(session_id) {
        Ok(value) => value,
        Err(error) => return dfmcp_error_payload("fortress.explain", &error),
    };
    with_session(
        &session,
        || mutex_poisoned_payload("fortress.explain"),
        |guard| {
            let (_, ctx) = match next_context(guard) {
                Ok(value) => value,
                Err(error) => return dfmcp_error_payload("fortress.explain", &error),
            };
            if let Err(error) = authorize_entry(&ctx, Capability::Query, RiskTier::ReadOnly) {
                return dfmcp_error_payload("fortress.explain", &error);
            }
            let snapshot = guard.adapter.snapshot();

            if let Some(ent_str) = entity_id {
                if ent_str.len() > MAX_FORTRESS_SELECTOR_BYTES {
                    return coded_error_payload(
                        "fortress.explain",
                        ErrorCode::BudgetExceeded,
                        "entity_id exceeds the maximum decimal u64 length",
                    );
                }
                let parsed_id: Result<EntityId> =
                    ent_str.parse::<u64>().map(EntityId::new).map_err(|_| {
                        DfmcpError::new(
                            ErrorCode::InvalidRequest,
                            "entity_id must be a decimal u64",
                        )
                    });
                let target_id = match parsed_id {
                    Ok(id) if id != EntityId::NIL => id,
                    Ok(_) => {
                        return error_payload("fortress.explain", "entity_id zero is reserved");
                    }
                    Err(error) => return dfmcp_error_payload("fortress.explain", &error),
                };

                let deps =
                    get_transitive_dependencies(&snapshot.graph, target_id, EdgeKind::Requires);
                let deps_str: Vec<String> = deps.iter().map(|id| format!("{}", id.get())).collect();
                let entity_record = snapshot.graph.entities.get(&target_id);
                // A checkable claim: the entity's canonical record is included
                // under this anchor's Merkle root (hand-offs can cite it).
                let tree = dfmcp_world::MerkleStateTree::from_snapshot(snapshot);
                let proof = tree.generate_entity_proof(target_id).map(|proof| {
                    json!({
                        "entities_root": tree.entities_root.to_hex(),
                        "overall_root": tree.overall_root.to_hex(),
                        "leaf_digest": proof.leaf_digest.to_hex(),
                        "sibling_hashes": proof.sibling_hashes.iter().map(|h| h.to_hex()).collect::<Vec<_>>(),
                        "sibling_is_left": proof.sibling_is_left,
                        "verifies": proof.verify_root(&tree.overall_root),
                    })
                });

                json!({
                    "ok": true,
                    "session_id": format!("{}", guard.session_id),
                    "target_entity": format!("{}", target_id.get()),
                    "entity_found": entity_record.is_some(),
                    "entity": entity_record.map(|entity| json!({
                        "kind": entity.kind.as_str(),
                        "label": entity.label,
                        "generation": entity.generation,
                        "revision": entity.revision,
                    })),
                    "inclusion_proof": proof,
                    "anchor": anchor_json(&snapshot.anchor()),
                    "transitive_dependencies": deps_str,
                    "note": "causal explanation derived from directed fortress multigraph topology",
                })
                .to_string()
            } else {
                let events = guard.adapter.transcript();
                let start = events.len().saturating_sub(16);
                let recent: Vec<String> = events
                    .iter()
                    .skip(start)
                    .map(|event| format!("{event:?}"))
                    .collect();
                json!({
                "ok": true,
                "session_id": format!("{}", guard.session_id),
                "transcript_len": events.len(),
                "transcript_truncated": guard.adapter.transcript_truncated(),
                "recent_events": recent,
                "note": "process-local laboratory transcript only; no durable evidence bundle is implemented",
            })
            .to_string()
            }
        },
    )
}

// ============================================================================
// fortress.doctor
// ============================================================================

/// Diagnose adapter health, compatibility identity, telemetry inspector, and the live anchor.
#[tool(
    description = "Diagnose the control plane for the open session: adapter health, telemetry inspector, sessions count, and current anchor."
)]
pub fn fortress_doctor(session_id: Option<String>) -> String {
    let session = match resolve_session(session_id) {
        Ok(value) => value,
        Err(error) => return dfmcp_error_payload("fortress.doctor", &error),
    };
    with_session(
        &session,
        || mutex_poisoned_payload("fortress.doctor"),
        |guard| {
            let (_, ctx) = match next_context(guard) {
                Ok(value) => value,
                Err(error) => return dfmcp_error_payload("fortress.doctor", &error),
            };
            if let Err(error) = authorize_entry(&ctx, Capability::Doctor, RiskTier::ReadOnly) {
                return dfmcp_error_payload("fortress.doctor", &error);
            }
            let health_res = guard.adapter.health(&ctx);

            let active_sessions_count = sessions().len();
            let health_opt = health_res.as_ref().ok();
            let report = DoctorInspector.generate_report(
                active_sessions_count,
                health_opt,
                None,
                guard.leases.manager.active_lease_count(),
                guard.open_actions.len() + guard.carried.iter().filter(|c| !c.is_final()).count(),
            );

            match health_res {
                Ok(health) => json!({
                    "ok": true,
                    "session_id": format!("{}", guard.session_id),
                    "status": if report.is_healthy { "healthy" } else { "degraded" },
                    "active_sessions_count": report.active_sessions_count,
                    "active_leases_count": report.active_leases_count,
                    "retained_action_fences": guard.leases.by_action.keys()
                        .filter(|id| !action_fully_drained(&guard.adapter, **id)).count(),
                    "active_obligations_count": report.active_obligations_count,
                    "adapter": health.identity.name,
                    "compatibility": format!("{:?}", health.identity.compatibility),
                    "fortress_loaded": health.fortress_loaded,
                    "findings": report.findings,
                    "warnings": health.warnings,
                    "current_anchor": health.current_anchor.as_ref().map(anchor_json),
                    "durability": durability_json(guard),
                    "untracked_work": untracked_work_json(guard),
                })
                .to_string(),
                Err(error) => dfmcp_error_payload("fortress.doctor", &error),
            }
        },
    )
}

// ============================================================================
// Server assembly & Transport Admission (WP-14)
// ============================================================================

/// Validates that an HTTP bind address is strictly localhost.
/// Non-localhost binds are rejected by design until the transport-boundary
/// admission design lands (WP-14 / FASTMCP_INTEGRATION.md §6).
pub fn validate_localhost_bind(bind_addr: &str) -> Result<()> {
    let host = if let Some(idx) = bind_addr.rfind(':') {
        &bind_addr[..idx]
    } else {
        bind_addr
    };
    let trimmed = host.trim_matches('[').trim_matches(']');
    if trimmed == "127.0.0.1" || trimmed == "::1" || trimmed == "localhost" {
        Ok(())
    } else {
        Err(DfmcpError::new(
            ErrorCode::CapabilityDenied,
            format!(
                "non-localhost bind address '{bind_addr}' rejected; transport-boundary admission requires localhost-only binding"
            ),
        ))
    }
}

/// Run the modern-only MCP 2026-07-28 server on stdio.
pub fn run_stdio() {
    let server = ServerBuilder::new("dwarf-fortress-mcp", env!("CARGO_PKG_VERSION"))
        .tool(FortressOpenSession)
        .tool(FortressObserve)
        .tool(FortressQuery)
        .tool(FortressPlan)
        .tool(FortressCommit)
        .tool(FortressWait)
        .tool(FortressCancel)
        .tool(FortressCheckpoint)
        .tool(FortressRestore)
        .tool(FortressExplain)
        .tool(FortressDoctor)
        .resource(crate::resources::SessionViewResource)
        .resource(crate::resources::DoctorBundleResource)
        .request_timeout(30)
        .instructions(
            "Dwarf Fortress semantic control plane (laboratory slice). Call fortress_open_session \
             first to negotiate capabilities, then supply the returned session_id to every other \
             tool. Each session has its own adapter, anchor, plans, and receipts; concurrent \
             sessions are independent. Transport identity grants nothing; every authority comes \
             from the negotiated grants. Dispatch success is never goal success: only \
             evidence-backed postcondition verification counts.",
        )
        .build();
    crate::run_modern_stdio(server);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_snapshot_carries_requested_pause_state() {
        assert!(seed_snapshot(FortressId::new(1), true).paused);
        assert!(!seed_snapshot(FortressId::new(1), false).paused);
    }

    #[test]
    fn seed_snapshot_is_origin_anchored() {
        let snapshot = seed_snapshot(FortressId::new(1), true);
        assert_eq!(snapshot.cursor.epoch, 0);
        assert!(snapshot.hash_is_valid());
    }

    #[test]
    fn parse_capability_request_rejects_unknown_capability() {
        let result =
            parse_capability_request(&[("not_a_capability".to_owned(), "read_only".to_owned())]);
        assert!(result.is_err());
    }

    #[test]
    fn parse_capability_request_rejects_unknown_risk() {
        let result = parse_capability_request(&[("observe".to_owned(), "yolo".to_owned())]);
        assert!(result.is_err());
    }

    #[test]
    fn parse_capability_request_rejects_duplicate_capability() {
        let result = parse_capability_request(&[
            ("observe".to_owned(), "read_only".to_owned()),
            ("observe".to_owned(), "guarded".to_owned()),
        ]);
        assert!(result.is_err());
    }

    #[test]
    fn parse_capability_request_accepts_documented_capabilities()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let result = parse_capability_request(&[
            ("observe".to_owned(), "read_only".to_owned()),
            ("plan".to_owned(), "reversible".to_owned()),
            ("control_clock".to_owned(), "reversible".to_owned()),
            ("checkpoint".to_owned(), "guarded".to_owned()),
            ("restore".to_owned(), "guarded".to_owned()),
            ("doctor".to_owned(), "read_only".to_owned()),
        ]);
        let parsed = result?;
        assert_eq!(parsed.len(), 6);
        assert_eq!(parsed[0].capability, Capability::Observe);
        assert_eq!(parsed[0].max_risk, RiskTier::ReadOnly);
        assert_eq!(parsed[5].capability, Capability::Doctor);
        assert_eq!(parsed[5].max_risk, RiskTier::ReadOnly);
        Ok(())
    }

    #[test]
    fn negotiate_grants_scopes_every_grant_to_the_session_fortress() {
        let grants = negotiate_grants(
            FortressId::new(7),
            &[NegotiatedCapability {
                capability: Capability::Observe,
                max_risk: RiskTier::ReadOnly,
            }],
        );
        assert_eq!(grants.len(), 1);
        assert_eq!(grants[0].capability, Capability::Observe);
        assert_eq!(grants[0].scope.fortress_id, Some(FortressId::new(7)));
        assert_eq!(grants[0].max_risk, RiskTier::ReadOnly);
    }

    #[test]
    fn session_counter_mints_unique_increasing_session_ids()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let first = next_session_counter()?;
        let second = next_session_counter()?;
        assert!(
            second > first,
            "session_id counter must be strictly monotonic"
        );
        Ok(())
    }

    #[test]
    fn parse_session_id_arg_round_trips_display_form()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let original = SessionId::new(0xabcde);
        let parsed = parse_session_id_arg(&original.to_string())?;
        assert_eq!(parsed, original);
        Ok(())
    }

    #[test]
    fn parse_session_id_arg_rejects_non_hex_or_noncanonical_input() {
        let result = parse_session_id_arg("not-a-number");
        assert!(result.is_err());
        assert!(parse_session_id_arg("12345").is_err());
        assert!(parse_session_id_arg("00000000000000000000000000000000").is_err());
    }

    #[test]
    fn missing_session_id_is_never_inferred_from_registry_state() {
        let result = resolve_session(None);
        let is_ok = result.is_ok();
        match result {
            Ok(_) => assert!(!is_ok, "missing authority handle must be rejected"),
            Err(error) => assert_eq!(error.code, ErrorCode::InvalidRequest),
        }
    }

    #[test]
    fn structured_errors_preserve_code_retryability_and_details()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let error = DfmcpError::new(ErrorCode::CursorGap, "refresh required")
            .retryable(true)
            .with_detail("epoch", "7");
        let payload: serde_json::Value =
            serde_json::from_str(&dfmcp_error_payload("fortress.observe", &error))?;
        assert_eq!(payload["error"]["code"], "cursor_gap");
        assert_eq!(payload["error"]["retryable"], true);
        assert_eq!(payload["error"]["details"][0][0], "epoch");
        Ok(())
    }

    #[test]
    fn lookup_session_returns_session_not_found_for_unknown_id() {
        let result = lookup_session(SessionId::new(u128::MAX));
        assert!(result.is_err());
    }

    #[test]
    fn budget_validation_rejects_zero_dimension() {
        let bad = WorkBudget {
            max_wall_millis: 0,
            ..WorkBudget::CONSERVATIVE_DEFAULT
        };
        assert!(bad.validate().is_err());
    }
}
