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
    Digest32, EntityId, ErrorCode, FortressId, GameTick, IntentId, OperationContext, RequestId,
    Result, RiskTier, SessionId, StateAnchor, WorkBudget,
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

#[path = "production_source.rs"]
mod production_source;

#[path = "plan_forecast.rs"]
mod plan_forecast;
use plan_forecast::forecast_plan;

#[path = "goal_continuation.rs"]
mod goal_continuation;

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
    let current = session.adapter.snapshot().clone();
    session.history.record(&current);
    let mut roots: std::collections::BTreeSet<Digest32> =
        session.adapter.checkpoint_state_hashes().collect();
    roots.extend(session.pending.iter().map(|p| p.plan.anchor.state_hash));
    roots.extend(session.durable_plans.values().map(|p| p.anchor.state_hash));
    roots.extend(objective_history_roots(session));
    session.history.collect(&roots);
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

/// Opening already holds the write admission guard and uses this helper to
/// avoid recursively acquiring a read guard.
fn with_session_admitted<T>(
    session: &Arc<Mutex<LabSession>>,
    poisoned: impl FnOnce() -> T,
    body: impl FnOnce(&mut LabSession) -> T,
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
    if let Some(world) = world.as_mut() {
        std::mem::swap(&mut guard.adapter, &mut world.adapter);
        std::mem::swap(&mut guard.leases, &mut world.leases);
        std::mem::swap(&mut guard.durable_plans, &mut world.durable_plans);
        std::mem::swap(&mut guard.durable_restore, &mut world.durable_restore);
        std::mem::swap(&mut guard.carried, &mut world.carried);
        std::mem::swap(&mut guard.durability_fault, &mut world.durability_fault);
        std::mem::swap(&mut guard.objectives, &mut world.objectives);
        std::mem::swap(&mut guard.objective_restore, &mut world.objective_restore);
        guard.shared_members = world.members.len();
    }
    let output = body(&mut guard);
    persist_durable_head(&mut guard);
    remember_version(&mut guard);
    if let Some(world) = world.as_mut() {
        std::mem::swap(&mut guard.adapter, &mut world.adapter);
        std::mem::swap(&mut guard.leases, &mut world.leases);
        std::mem::swap(&mut guard.durable_plans, &mut world.durable_plans);
        std::mem::swap(&mut guard.durable_restore, &mut world.durable_restore);
        std::mem::swap(&mut guard.carried, &mut world.carried);
        std::mem::swap(&mut guard.durability_fault, &mut world.durability_fault);
        std::mem::swap(&mut guard.objectives, &mut world.objectives);
        std::mem::swap(&mut guard.objective_restore, &mut world.objective_restore);
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
        let recorded_anchor = commit.step_anchors.get(&step.id.get()).copied();
        // Legacy peers could publish a head without the originating session's
        // step record. Divergent unanchored absence cannot prove nondispatch.
        let unanchored_absence = recorded_anchor.is_none()
            && head_diverged
            && matches!(recorded, None | Some("not_dispatched" | "abandoned"));
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
        let work_state = if state == "not_dispatched" && !unanchored_absence {
            EffectWorkState::NeverDispatched
        } else {
            dfmcp_intent::inspect_effect_work(recovered, &step.action, &step.idempotency_key, true)
                .unwrap_or_else(|error| EffectWorkState::Unknown {
                    entity_id: None,
                    reason: format!("{}: {}", error.code.as_str(), error.message),
                })
        };
        if state == "dispatched"
            || unanchored_terminal
            || unanchored_absence
            || !work_state.is_quiescent()
        {
            open += 1;
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
            let unresolved = unanchored_terminal || unanchored_absence || monitor_error.is_some();
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
                    monitor_error
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
            "proof_class": if unanchored_terminal || unanchored_absence { "unanchored_legacy_record" }
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
        let mut adapter = MemoryAdapter::recovered(snapshot)?;
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
            .map(|retained| objectives::recover_objective(store, retained))
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
}

impl PlanSource {
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
        }
    }

    fn intent(&self, id: IntentId, snapshot: &WorldSnapshot) -> Result<Intent> {
        self.compile(id, snapshot).map(|(intent, _)| intent)
    }

    fn continuation(&self) -> Result<Option<goal_continuation::ProductionContinuation>> {
        match self {
            Self::ProductionContinuation { raw, .. } => {
                goal_continuation::ProductionContinuation::parse(raw).map(Some)
            }
            _ => Ok(None),
        }
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
            Self::Blueprint { summary, raw } => {
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
pub(crate) fn replay_bundle_for(session_id: &str) -> Result<serde_json::Value> {
    let session = lookup_session_str(session_id)?;
    let guard = session
        .lock()
        .map_err(|_| DfmcpError::new(ErrorCode::InternalInvariantViolation, "session poisoned"))?;
    Ok(crate::replay::bundle_json(&guard.replay))
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
    let requested_caps_raw =
        requested_capabilities.map_or_else(|| default_caps, |capabilities| capabilities);
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
    let (untracked, goal_status) = with_session_admitted(
        &session,
        || {
            (
                json!({"state": "unknown", "quiescent": false, "items": null,
            "reason": "session observation unavailable"}),
                serde_json::Value::Null,
            )
        },
        |guard| (untracked_work_json(guard), objectives_json(guard)),
    );
    json!({
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
    })
    .to_string()
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
                        payload["world_alerts"] = json!(crate::lab_world::world_alerts(&snapshot));
                        payload["objectives"] = objectives_json(guard);
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
    description = "Run a bounded laboratory query. mode: \"summary\" (default), \"entities\", or a JSON object {\"mode\":\"entities\",\"kind\":\"unit\",\"limit\":25,\"offset\":0} or {\"mode\":\"terrain\",\"min\":[x,y,z],\"max\":[x,y,z]}. Full DfQL is not implemented."
)]
pub fn fortress_query(session_id: Option<String>, mode: Option<String>) -> String {
    let mode = mode.map_or_else(|| "summary".to_owned(), |value| value);
    if mode.len() > MAX_MODE_BYTES {
        return coded_error_payload(
            "fortress.query",
            ErrorCode::BudgetExceeded,
            "query mode exceeds its explicit byte bound",
        );
    }
    let session = match resolve_session(session_id) {
        Ok(value) => value,
        Err(error) => return dfmcp_error_payload("fortress.query", &error),
    };
    with_session(
        &session,
        || mutex_poisoned_payload("fortress.query"),
        |guard| {
            let (_, ctx) = match next_context(guard) {
                Ok(value) => value,
                Err(error) => return dfmcp_error_payload("fortress.query", &error),
            };
            if let Err(error) = authorize_entry(&ctx, Capability::Query, RiskTier::ReadOnly) {
                return dfmcp_error_payload("fortress.query", &error);
            }
            if mode.trim_start().starts_with('{') {
                match historical_query(guard, &mode) {
                    Ok(Some(payload)) => return payload,
                    Ok(None) => {}
                    Err(error) => return dfmcp_error_payload("fortress.query", &error),
                }
            }
            if mode != "summary" {
                let snapshot = guard.adapter.snapshot();
                return match crate::lab_world::query(snapshot, &mode) {
                    Ok(mut payload) => {
                        payload["ok"] = json!(true);
                        payload["session_id"] = json!(format!("{}", guard.session_id));
                        payload["anchor"] = anchor_json(&snapshot.anchor());
                        payload["game_tick"] = json!(snapshot.tick.0);
                        payload.to_string()
                    }
                    Err(error) => dfmcp_error_payload("fortress.query", &error),
                };
            }
            let request = QueryRequest {
                anchor: ctx.anchor,
                query: WorldQuery {
                    kinds: Vec::new(),
                    predicate: None,
                    order: QueryOrder::EntityIdAscending,
                    limit: guard.budget.max_entities,
                    continuation: None,
                },
                max_output_tokens: guard.budget.max_output_tokens,
                continuation: None,
            };
            match guard.adapter.query(&request, &ctx) {
                Ok(response) => {
                    let snapshot = guard.adapter.snapshot();
                    let mut payload = snapshot_json(snapshot);
                    payload["matched"] = json!(response.matched);
                    payload["returned"] = json!(response.rows.len());
                    payload["truncated"] = json!(response.truncated);
                    payload["continuation"] = json!(response.continuation);
                    payload["session_id"] = json!(format!("{}", guard.session_id));
                    payload.to_string()
                }
                Err(error) => dfmcp_error_payload("fortress.query", &error),
            }
        },
    )
}

/// Queries over retained world versions: `{"mode":"changes","since":H}`
/// reports observed changes from version `H` to now, and any entities or
/// terrain query with `"at":H` reads version `H` exactly. `Ok(None)` means the
/// request is an ordinary current-state query.
fn historical_query(guard: &LabSession, raw: &str) -> Result<Option<String>> {
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
        let changes = crate::world_changes::describe(base, current);
        return Ok(Some(
            json!({
                "ok": true,
                "session_id": format!("{}", guard.session_id),
                "mode": "changes",
                "since_anchor": anchor_json(&base.anchor()),
                "anchor": anchor_json(&current.anchor()),
                "changes": changes,
                "epistemic_state": "observed",
            })
            .to_string(),
        ));
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
    let mut payload = crate::lab_world::query(version, &spec.to_string())?;
    payload["ok"] = json!(true);
    payload["session_id"] = json!(format!("{}", guard.session_id));
    payload["anchor"] = anchor_json(&version.anchor());
    payload["game_tick"] = json!(version.tick.0);
    payload["historical"] = json!(version.state_hash != current.state_hash);
    payload["current_anchor"] = anchor_json(&current.anchor());
    payload["epistemic_state"] = json!("observed");
    Ok(Some(payload.to_string()))
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
pub(crate) fn plan_request(
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
                    (_, _, Some(raw)) => PlanSource::Blueprint { summary, raw },
                    (_, Some(raw), None) => PlanSource::Actions { summary, raw },
                    (None, None, None) => PlanSource::Pause {
                        summary,
                        paused_target: paused_target.is_some_and(|value| value),
                    },
                }
            };
            let snapshot = guard.adapter.snapshot();
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
                        "forecast": forecast_plan(&guard.adapter, &plan, &ctx),
                        "live_routing": live_routing_json(&plan),
                        "production": production_analysis,
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
            "work_state": action_work_json(&session.adapter, action_id),
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
    let alerts = crate::lab_world::world_alerts(snapshot);
    if session.carried.iter().any(|carried| !carried.is_final()) && !snapshot.paused {
        resume.push(json!({
            "tool": "fortress.wait",
            "arguments": {"session_id": format!("{}", session.session_id), "max_game_ticks": 100},
            "why": "recovered goals and physical work require current observation; terminal proof receipts remain unchanged while work progresses",
        }));
    }
    for alert in &alerts {
        if alert["severity"] == "critical" && alert["remedy"].is_object() {
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
        "mcp_tasks": crate::task_service::session_handles(&session.session_id.to_string()),
        "mcp_task_coverage": crate::task_service::session_handle_coverage(&session.session_id.to_string()),
        "committed_plan_digests": session.commit_receipts.keys().collect::<Vec<_>>(),
        "resume_protocol": resume,
        "authority": "reading this packet grants nothing; every commit and cancel re-checks the session's negotiated grants",
        "epistemic_note": "action states are the last recorded receipts; fortress.wait re-evaluates them against a fresh observation",
    })
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
            Action::DesignateDig { .. } | Action::Build { .. }
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
                Action::DesignateDig { area, .. } => area,
                Action::Build { footprint, .. } => footprint,
                _ => continue,
            };
            if dfmcp_core::lease::cuboids_intersect(&area, owned_area) {
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
                Action::DesignateDig { area, .. } => area,
                Action::Build { footprint, .. } => footprint,
                _ => continue,
            };
            if dfmcp_core::lease::cuboids_intersect(&area, owned_area) {
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

fn action_work_json(adapter: &MemoryAdapter, action_id: ActionId) -> serde_json::Value {
    let mut view = match adapter.action_work_state(action_id) {
        Ok(state) => effect_work_json(&state),
        Err(error) => effect_work_json(&EffectWorkState::Unknown {
            entity_id: None,
            reason: format!("{}: {}", error.code.as_str(), error.message),
        }),
    };
    view["observed_anchor"] = anchor_json(&adapter.snapshot().anchor());
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
    let work = action_work_json(&session.adapter, action_id);
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
                Action::DesignateDig { .. } | Action::Build { .. } | Action::CreateWorkOrder { .. }
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
/// at the current anchor and accept the replay if it performs the very same
/// actions. Returns the re-sealed plan and its certificate, or why not.
fn rebase_by_witness(
    session: &mut LabSession,
    stale: &PendingPlan,
) -> std::result::Result<(PreparedPlan, serde_json::Value), serde_json::Value> {
    // Quota lowering reads the complete production domain, including absence
    // of competing orders, current service and setup candidates. The current
    // action/predicate witness does not cover those range and negative reads.
    // A changed anchor therefore requires a newly reviewed source replay;
    // equal action bytes alone cannot certify unchanged sealed deadlines.
    if matches!(
        &stale.source,
        PlanSource::Production { .. } | PlanSource::ProductionContinuation { .. }
    ) {
        return Err(json!({
            "accepted": false,
            "reason": "production planning requires current workload and prerequisite evidence; review a newly sealed intent replay",
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
    if let Some(change) = witness.first_change(&base, &now) {
        return Err(
            json!({"accepted": false, "reason": "a read of the plan changed", "change": change}),
        );
    }
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
            json!({"accepted": false, "reason": "the replayed intent no longer yields the same actions"}),
        );
    }
    let certificate = crate::witness::certificate(&stale.plan, &plan, &base, &now, &witness);
    Ok((plan, certificate))
}

fn replay_stale_plan(session: &mut LabSession, stale: PendingPlan) -> String {
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
                "forecast": forecast_plan(&session.adapter, &plan, &ctx),
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
                                );
                            }
                            guard.last_action =
                                receipt.actions.first().map(|action| action.action_id);
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
                                    "work_state": action_work_json(&guard.adapter, action.action_id),
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

/// Poll the most recent committed action and return its current state.
#[tool(
    description = "Poll the most recent committed action in this session. Returns the action receipt state from the laboratory adapter's bounded obligation machinery."
)]
pub fn fortress_wait(session_id: Option<String>) -> String {
    wait_with_ticks(session_id, None)
}

/// Optionally let laboratory game time pass (only while the fortress is
/// unpaused, bounded by the session's game-tick budget), then poll every
/// action of the most recent committed plan against the new observation.
pub(crate) fn wait_with_ticks(session_id: Option<String>, max_game_ticks: Option<u64>) -> String {
    let session = match resolve_session(session_id) {
        Ok(value) => value,
        Err(error) => return dfmcp_error_payload("fortress.wait", &error),
    };
    with_session(
        &session,
        || mutex_poisoned_payload("fortress.wait"),
        |guard| {
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
            if action_id.is_none() && max_game_ticks.is_none_or(|ticks| ticks == 0) {
                return coded_error_payload(
                    "fortress.wait",
                    ErrorCode::Conflict,
                    "no committed action yet; call fortress_commit first, or pass max_game_ticks to let time pass",
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
            let advanced = if requested_ticks > 0 && !paused {
                if let Err(error) = guard.adapter.advance_ticks(requested_ticks) {
                    return dfmcp_error_payload("fortress.wait", &error);
                }
                requested_ticks
            } else {
                0
            };
            let (_, ctx) = match next_context(guard) {
                Ok(value) => value,
                Err(error) => return dfmcp_error_payload("fortress.wait", &error),
            };
            let task = match action_id
                .map(|action_id| {
                    crate::tasks::project_action_task(&mut guard.adapter, action_id, &ctx)
                })
                .transpose()
            {
                Ok(task) => task,
                Err(error) => return dfmcp_error_payload("fortress.wait", &error),
            };
            // Poll every open action in commit order (prerequisites before their
            // dependents). Retire only terminal proofs with quiescent work.
            let mut polled_actions = Vec::new();
            let mut still_open = Vec::new();
            for open in guard.open_actions.clone() {
                let (_, poll_ctx) = match next_context(guard) {
                    Ok(value) => value,
                    Err(error) => return dfmcp_error_payload("fortress.wait", &error),
                };
                match guard.adapter.poll_action(open, &poll_ctx) {
                    Ok(receipt) => {
                        if action_fully_drained(&guard.adapter, open) {
                            release_action_leases(guard, open);
                        } else {
                            still_open.push(open);
                        }
                        polled_actions.push(json!({
                            "action_id": format!("{open}"),
                            "step": receipt.step_id.get(),
                            "state": format!("{:?}", receipt.state),
                            "message": receipt.message,
                            "work_state": action_work_json(&guard.adapter, open),
                        }));
                    }
                    Err(error) => return dfmcp_error_payload("fortress.wait", &error),
                }
            }
            guard.open_actions = still_open;
            if let Err(error) = observe_carried(guard) {
                return dfmcp_error_payload("fortress.wait", &error);
            }
            let snapshot = guard.adapter.snapshot();
            let mut payload = match (action_id, task) {
                (Some(action_id), Some(task)) => json!({
                    "ok": true,
                    "session_id": format!("{}", guard.session_id),
                    "action_id": format!("{}", action_id),
                    "task_id": task.task_id,
                    "handle_kind": "action_projection_only",
                    "modern_task_discovery": format!("df://session/{}/tasks", guard.session_id),
                    "status": task.status.as_str(),
                    "commit_state": format!("{:?}", task.commit_state),
                    "summary": task.summary,
                    "work_state": action_work_json(&guard.adapter, action_id),
                    "observed_anchor": anchor_json(&snapshot.anchor()),
                }),
                _ => json!({
                    "ok": true,
                    "session_id": format!("{}", guard.session_id),
                    "action_id": null,
                    "status": "time_passed",
                    "summary": "no action of this session is open; game time passed for work already in the world",
                    "observed_anchor": anchor_json(&snapshot.anchor()),
                }),
            };
            payload["paused"] = json!(guard.adapter.snapshot().paused);
            payload["untracked_work"] = untracked_work_json(guard);
            payload["polled_actions"] = json!(polled_actions);
            payload["open_actions_remaining"] = json!(guard.open_actions.len());
            if max_game_ticks.is_some() {
                payload["advanced_game_ticks"] = json!(advanced);
                payload["game_tick"] = json!(snapshot.tick.0);
                payload["world_alerts"] =
                    json!(crate::lab_world::world_alerts(guard.adapter.snapshot()));
                payload["objectives"] = objectives_json(guard);
                if !guard.carried.is_empty() {
                    // The bounded evaluation above owns proof status; a raw
                    // predicate match cannot substitute for cadence/stability.
                    payload["carried_obligations"] = json!(
                        guard
                            .carried
                            .iter()
                            .map(CarriedStep::to_json)
                            .collect::<Vec<_>>()
                    );
                }
                if requested_ticks > 0 && paused {
                    payload["blocked"] = json!(
                        "the fortress is paused, so no work progresses; commit an unpause plan to let time pass"
                    );
                }
            }
            payload.to_string()
        },
    )
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
    let drain_scope = match scope.as_deref() {
        None | Some("last_action") => None,
        Some("plan") => Some("plan"),
        Some("session") => Some("session"),
        Some("oldest_open_plan") => Some("oldest_open_plan"),
        Some(other) => {
            return coded_error_payload(
                "fortress.cancel",
                ErrorCode::InvalidRequest,
                &format!(
                    "unsupported cancellation scope {other:?}; use last_action, plan, oldest_open_plan or session"
                ),
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
                            "work_state": action_work_json(&guard.adapter, action_id),
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
                    "work_state": action_work_json(&guard.adapter, action_id),
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
        let work_before = action_work_json(&guard.adapter, action_id);
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
            "work_after": action_work_json(&guard.adapter, action_id),
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
    description = "Explain what happened in this session: return recent transcript events, or graph dependencies and causal topology for a specified entity."
)]
pub fn fortress_explain(session_id: Option<String>, entity_id: Option<String>) -> String {
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
