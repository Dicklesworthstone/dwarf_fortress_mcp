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
use std::sync::{Arc, LazyLock, Mutex, MutexGuard};

use crate::doctor::DoctorInspector;
use dfmcp_adapter::{
    CancelMode, GameAdapter, InterestSet, ObservationPayload, ObservationRequest, Projection,
    QueryRequest,
};
use dfmcp_core::{
    ActionId, Capability, CapabilityGrant, CapabilityScope, CheckpointId, CommitState, DfmcpError,
    Digest32, EntityId, ErrorCode, FortressId, IntentId, OperationContext, RequestId, Result,
    RiskTier, SessionId, StateAnchor, WorkBudget,
};
use dfmcp_intent::{Action, Constraint, Intent, PreparedPlan, RequestedAction, StaticPlanner};
use dfmcp_lab::MemoryAdapter;
use dfmcp_world::topology::get_transitive_dependencies;
use dfmcp_world::{EdgeKind, Predicate, QueryOrder, WorldQuery, WorldSnapshot};
use fastmcp_rust::modern::ServerBuilder;
use fastmcp_rust::prelude::*;
use serde_json::json;

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
    /// Every committed action, across plans, that was not terminal when last
    /// polled. `fortress.wait` polls all of them, which is also what
    /// dispatches deferred steps once their dependencies verify.
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
    /// Steps of plans committed before a durable restart, re-proven against
    /// observation after every call until they are final.
    carried: Vec<CarriedStep>,
    /// Every tool call of this session, for deterministic replay bundles.
    pub(crate) replay: crate::replay::ReplayLog,
    /// Bounded, immutable history of the world versions this session saw,
    /// newest last, so every turn can say exactly what changed.
    history: std::collections::VecDeque<WorldSnapshot>,
    /// The intent behind every committed plan, re-evaluated against each
    /// observation: dispatch success is not goal success.
    objectives: Vec<Objective>,
}

/// A committed intent whose terminal condition is the goal.
#[derive(Clone, Debug)]
struct Objective {
    plan_digest: String,
    summary: String,
    terminal: Predicate,
    committed_tick: u64,
}

const MAX_OBJECTIVES: usize = 64;

/// Every tracked objective with whether the current world satisfies it.
pub(crate) fn objectives_json(session: &LabSession) -> serde_json::Value {
    let snapshot = session.adapter.snapshot();
    json!(
        session
            .objectives
            .iter()
            .map(|objective| {
                let satisfied = dfmcp_world::evaluate(snapshot, &objective.terminal);
                json!({
                    "plan_digest": objective.plan_digest,
                    "summary": objective.summary,
                    "status": if satisfied { "achieved" } else { "not_yet_observed" },
                    "epistemic_state": "observed",
                    "terminal_condition": crate::lab_world::predicate_json(&objective.terminal),
                    "committed_tick": objective.committed_tick,
                })
            })
            .collect::<Vec<_>>()
    )
}

/// World versions each session retains for change reporting.
const MAX_SESSION_HISTORY: usize = 32;

fn remember_version(session: &mut LabSession) {
    let current = session.adapter.snapshot();
    if session
        .history
        .back()
        .is_some_and(|last| last.state_hash == current.state_hash)
    {
        return;
    }
    if session.history.len() == MAX_SESSION_HISTORY {
        session.history.pop_front();
    }
    session.history.push_back(current.clone());
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
    let target = guard.history.back()?;
    if target.state_hash.to_hex() == from_state_hash {
        return Some(Vec::new());
    }
    match guard
        .history
        .iter()
        .find(|version| version.state_hash.to_hex() == from_state_hash)
    {
        Some(base) => Some(crate::world_changes::describe(base, target)),
        None => Some(vec![json!({
            "kind": "history_not_retained",
            "subject": {"from_state_hash": from_state_hash},
            "epistemic_state": "unknown",
            "invalidates": [],
            "evidence": [],
            "note": "the previous anchor is older than this session's retained history; observe or query to re-establish the picture",
        })]),
    }
}

/// A step committed before a durable restart. No action handle survives the
/// restart, so its sealed proof is evaluated directly against observation.
#[derive(Clone, Debug)]
pub(crate) struct CarriedStep {
    plan_digest: Digest32,
    step: dfmcp_core::StepId,
    kind: &'static str,
    proof: Predicate,
    failure: Option<Predicate>,
    deadline: Option<dfmcp_core::GameTick>,
    /// A durable step state from `dfmcp_lab::durable::STEP_STATES`.
    state: String,
}

impl CarriedStep {
    fn to_json(&self) -> serde_json::Value {
        json!({
            "plan_digest": self.plan_digest.to_hex(),
            "step": self.step.get(),
            "action": self.kind,
            "state": self.state,
            "deadline_tick": self.deadline.map(|tick| tick.0),
        })
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
}

/// Process-local registry of shared fortresses, keyed by fortress selector.
static SHARED_WORLDS: LazyLock<Mutex<BTreeMap<FortressId, Arc<Mutex<SharedWorld>>>>> =
    LazyLock::new(|| Mutex::new(BTreeMap::new()));
const MAX_SHARED_WORLDS: usize = 64;

/// What a joining member learns about the shared fortress.
struct SharedView {
    anchor: StateAnchor,
    paused: bool,
    members: usize,
    joined_existing: bool,
    scenario: String,
}

/// Join (or create) the shared fortress for `fortress_id`. A joiner gets the
/// existing world, never a fresh scenario; naming a different scenario is
/// refused rather than silently ignored.
fn join_shared_world(
    fortress_id: FortressId,
    session_id: SessionId,
    scenario: &str,
    scenario_requested: bool,
    seed: &MemoryAdapter,
    durable: bool,
) -> Result<(Arc<Mutex<SharedWorld>>, SharedView)> {
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
        if scenario_requested && guard.scenario != scenario {
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
        let view = SharedView {
            anchor: guard.adapter.snapshot().anchor(),
            paused: guard.adapter.snapshot().paused,
            members: guard.members.len(),
            joined_existing: true,
            scenario: guard.scenario.clone(),
        };
        drop(guard);
        return Ok((world, view));
    }
    if registry.len() >= MAX_SHARED_WORLDS {
        return Err(DfmcpError::new(
            ErrorCode::BudgetExceeded,
            "the laboratory reached its shared-fortress bound",
        ));
    }
    let world = SharedWorld {
        adapter: seed.clone(),
        leases: LeaseBook::default(),
        members: BTreeSet::from([session_id]),
        scenario: scenario.to_owned(),
        durable,
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
    Ok((world, view))
}

const MAX_SHARED_MEMBERS: usize = 16;

/// Run `body` against a session. For a member of a shared fortress the world
/// lock is held for the whole call and the world's adapter and lease book are
/// swapped into the session, so every member observes and mutates the same
/// canonical state one call at a time. Lock order is always world, then
/// session; nothing locks a world while holding a session.
pub(crate) fn with_session<T>(
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
        guard.shared_members = world.members.len();
    }
    let output = body(&mut guard);
    persist_durable_head(&mut guard);
    remember_version(&mut guard);
    if let Some(world) = world.as_mut() {
        std::mem::swap(&mut guard.adapter, &mut world.adapter);
        std::mem::swap(&mut guard.leases, &mut world.leases);
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

/// Test hook: drop the durable store (releasing its lock) and forget owners,
/// exactly what a process exit does, then use `dir` as the configured root.
#[cfg(test)]
pub(crate) fn simulate_durable_restart(dir: Option<std::path::PathBuf>) {
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
    let Some(scenario) = session.durable_scenario.clone() else {
        return;
    };
    let snapshot = session.adapter.snapshot().clone();
    let fortress = session.fortress_id;
    // Step states first, then the head: a step is never recorded final or
    // dispatched by a world that lacks it, and a crash between the two can
    // only make a dispatched step fail its deadline, never verify falsely.
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
                    updates.push((*digest, step.id.get(), token));
                }
                None => all_final = false,
            }
        }
        if all_final {
            finished.push(*digest);
        }
    }
    for carried in &mut session.carried {
        if carried.state != "dispatched" {
            continue;
        }
        if dfmcp_world::evaluate(&snapshot, &carried.proof) {
            carried.state = "verified".to_owned();
        } else if carried
            .failure
            .as_ref()
            .is_some_and(|failure| dfmcp_world::evaluate(&snapshot, failure))
            || carried
                .deadline
                .is_some_and(|deadline| snapshot.tick > deadline)
        {
            carried.state = "failed".to_owned();
        }
    }
    let carried_updates: Vec<(Digest32, u32, String)> = session
        .carried
        .iter()
        .map(|c| (c.plan_digest, c.step.get(), c.state.clone()))
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
                .all(|c| c.state != "dispatched")
        })
        .collect();
    let result = with_durable_store(|store| {
        for (digest, step, token) in &updates {
            store.persist_step(fortress, *digest, *step, token)?;
        }
        for (digest, step, token) in &carried_updates {
            // A carried commit stays visible after it is retired; only an
            // unfinished one still takes step records.
            if store.commit(fortress, *digest).is_some() {
                store.persist_step(fortress, *digest, *step, token)?;
            }
        }
        store.persist_head(&scenario, &snapshot)?;
        for digest in finished.iter().chain(carried_done.iter()) {
            store.retire_commit(fortress, *digest)?;
        }
        Ok(())
    });
    if result.is_ok() {
        for digest in &finished {
            session.durable_plans.remove(digest);
        }
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
    let (session_id, fortress_id, private_durable) = match session.lock() {
        Ok(guard) => (
            guard.session_id,
            guard.fortress_id,
            guard.durable_scenario.is_some() && guard.shared.is_none(),
        ),
        Err(_) => return Ok(()),
    };
    if !private_durable {
        return Ok(());
    }
    match durable_lab().owners.get(&fortress_id) {
        Some(owner) if *owner != session_id => Err(DfmcpError::new(
            ErrorCode::Conflict,
            format!(
                "this session was superseded: durable fortress {fortress_id} was reopened by session {owner}; continue there"
            ),
        )
        .retryable(false)),
        _ => Ok(()),
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
}

/// Rebuild the steps of a commit made before a restart. The plan is
/// recompiled from its recorded request against the exact world it was
/// sealed on; determinism must reproduce the sealed digest, or the commit is
/// reported unverifiable and abandoned rather than trusted.
fn recover_commit(
    store: &mut dfmcp_lab::durable::DurableLabStore,
    commit: &dfmcp_lab::durable::DurableCommit,
    carried: &mut Vec<CarriedStep>,
) -> Result<serde_json::Value> {
    let fortress = commit.fortress_id;
    let sealed = store.load_snapshot(commit.sealed_state_hash)?;
    let source = PlanSource::from_durable(&commit.source);
    // Recompiling is a pure, internal verification read: it uses a planning
    // grant scoped to this fortress and confers nothing on any session.
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
        .and_then(|intent| StaticPlanner::default().prepare(&sealed, &intent, &context));
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
            for step in commit.steps.keys() {
                store.persist_step(fortress, commit.plan_digest, *step, "abandoned")?;
            }
            store.retire_commit(fortress, commit.plan_digest)?;
            return Ok(json!({
                "plan_digest": commit.plan_digest.to_hex(),
                "status": "unverifiable",
                "reason": reason,
                "note": "the sealed plan could not be reproduced; its effects are indeterminate, so observe before re-planning",
            }));
        }
    };
    let mut steps = Vec::new();
    let mut open = 0usize;
    for step in &plan.steps {
        let recorded = commit.steps.get(&step.id.get()).map(String::as_str);
        let kind = crate::lab_world::action_kind(&step.action);
        let state = match recorded {
            None => {
                store.persist_step(
                    fortress,
                    commit.plan_digest,
                    step.id.get(),
                    "not_dispatched",
                )?;
                "not_dispatched".to_owned()
            }
            Some(state) => state.to_owned(),
        };
        if state == "dispatched" {
            open += 1;
            let proof = step.obligation.as_ref().map_or_else(
                || Predicate::All(step.postconditions.clone()),
                |obligation| obligation.terminal.clone(),
            );
            carried.push(CarriedStep {
                plan_digest: commit.plan_digest,
                step: step.id,
                kind,
                proof,
                failure: step.obligation.as_ref().and_then(|o| o.failure.clone()),
                deadline: step.obligation.as_ref().map(|o| o.deadline_tick),
                state: state.clone(),
            });
        }
        steps.push(json!({"step": step.id.get(), "action": kind, "state": state}));
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
        for commit in store.commits(fortress_id).cloned().collect::<Vec<_>>() {
            commits.push(recover_commit(store, &commit, &mut carried)?);
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
    json!({
        "durable": true,
        "scenario": scenario,
        "fault": session.durability_fault,
        "persisted_anchor": head.as_ref().map(|head| anchor_json(&head.anchor)),
        "carried_obligations": session.carried.iter().map(CarriedStep::to_json).collect::<Vec<_>>(),
        "persisted_is_current": head.as_ref().is_some_and(|head| head.anchor == session.adapter.snapshot().anchor()),
        "store": report.map(|report| json!({
            "records": report.records,
            "fortresses": report.fortresses,
            "checkpoints": report.checkpoints,
            "chain_head": report.chain_head.to_hex(),
            "torn_tail_bytes_discarded_at_open": report.torn_tail_bytes,
            "compactions": report.compactions,
        })),
        "note": "laboratory durability: world state and checkpoints survive process loss; action handles and obligations do not",
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
        }
    }

    fn intent(&self, id: IntentId, snapshot: &WorldSnapshot) -> Result<Intent> {
        match self {
            Self::Actions { summary, raw } => semantic_intent(id, snapshot, summary.clone(), raw),
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
                Ok(intent)
            }
            Self::Pause {
                summary,
                paused_target,
            } => Ok(Intent {
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
            }),
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

pub(crate) fn anchor_json(anchor: &StateAnchor) -> serde_json::Value {
    json!({
        "fortress_id": format!("{}", anchor.fortress_id),
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
    let (seed_adapter, recovery) = if durable {
        match load_durable_fortress(fortress_id, &mut scenario, scenario_requested, &fresh) {
            Ok((adapter, recovery)) => (adapter, Some(recovery)),
            Err(error) => return dfmcp_error_payload("fortress.open_session", &error),
        }
    } else {
        (fresh, None)
    };
    let probe_session = LabSession {
        session_id: SessionId::new(0), // placeholder; replaced below
        fortress_id,
        grants: grants.clone(),
        budget,
        negotiation: SessionNegotiation::laboratory(String::from("pending")),
        next_request_id: 0,
        adapter: seed_adapter,
        pending: None,
        last_action: None,
        last_plan_actions: Vec::new(),
        open_actions: Vec::new(),
        commit_receipts: BTreeMap::new(),
        commit_authority: BTreeMap::new(),
        shared: None,
        leases: LeaseBook::default(),
        shared_members: 0,
        durable_scenario: None,
        durability_fault: None,
        durable_plans: BTreeMap::new(),
        carried: Vec::new(),
        replay: crate::replay::ReplayLog::default(),
        history: std::collections::VecDeque::new(),
        objectives: Vec::new(),
    };
    let identity = probe_session.adapter.identity();
    let negotiation = SessionNegotiation::laboratory(format!("{:?}", identity.compatibility));
    let session_counter = match next_session_counter() {
        Ok(value) => value,
        Err(error) => return dfmcp_error_payload("fortress.open_session", &error),
    };
    let session_id = SessionId::new(session_counter);
    let (shared_world, shared_view) = if shared {
        match join_shared_world(
            fortress_id,
            session_id,
            &scenario,
            scenario_requested,
            &probe_session.adapter,
            durable,
        ) {
            Ok(value) => (Some(value.0), Some(value.1)),
            Err(error) => return dfmcp_error_payload("fortress.open_session", &error),
        }
    } else {
        (None, None)
    };
    let (snapshot_anchor, paused_after) = match shared_view.as_ref() {
        Some(view) => (view.anchor, view.paused),
        None => (
            probe_session.adapter.snapshot().anchor(),
            probe_session.adapter.snapshot().paused,
        ),
    };
    // Move the probe adapter into the registered session.
    let LabSession {
        session_id: _,
        fortress_id: _,
        grants: _,
        budget: _,
        negotiation: _,
        next_request_id: _,
        adapter,
        pending: _,
        last_action: _,
        last_plan_actions: _,
        open_actions: _,
        commit_receipts: _,
        commit_authority: _,
        shared: _,
        leases: _,
        shared_members: _,
        durable_scenario: _,
        durability_fault: _,
        durable_plans: _,
        carried: _,
        replay: _,
        history: _,
        objectives: _,
    } = probe_session;
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
        carried: recovery
            .as_ref()
            .map_or_else(Vec::new, |recovery| recovery.carried.clone()),
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
        history: std::collections::VecDeque::new(),
        objectives: Vec::new(),
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
    with_session(&session, || (), |_| ());
    if durable {
        if !shared {
            durable_lab().owners.insert(fortress_id, session_id);
        }
        // Persist the opening state (a resumed world's new epoch included) now,
        // so a crash before the first state change still resumes it.
        let fault = with_session(
            &session,
            || Some("session poisoned".to_owned()),
            |guard| {
                persist_durable_head(guard);
                guard.durability_fault.clone()
            },
        );
        if let Some(fault) = fault {
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
    json!({
        "ok": true,
        "session_id": format!("{session_id}"),
        "adapter": identity.name,
        "compatibility": format!("{:?}", identity.compatibility),
        "fortress_loaded": true,
        "fortress_id": format!("{fortress_id}"),
        "granted_capabilities": granted_strings,
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
            "torn_tail_bytes_discarded_at_store_open": recovery.torn_tail_bytes,
            "note": if recovery.resumed {
                "resumed the last persisted world in a new observation epoch: designations, construction and work orders continue on wait; action handles, plans and obligations from before are not carried, so re-establish them from observation. Older sessions of this fortress are fenced."
            } else {
                "new crash-durable fortress: every state change and checkpoint is persisted and survives server restarts"
            },
        })),
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
            .iter()
            .rev()
            .find(|version| version.state_hash.to_hex() == hash)
            .ok_or_else(|| {
                DfmcpError::new(
                    ErrorCode::StaleAnchor,
                    format!(
                        "world version {hash} is not retained; this session keeps its last {MAX_SESSION_HISTORY} versions"
                    ),
                )
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
    plan_request(session_id, summary, paused_target, actions, None)
}

/// `fortress.plan` over every request form: pause/resume, explicit semantic
/// actions, or a blueprint objective the planner decomposes into steps.
pub(crate) fn plan_request(
    session_id: Option<String>,
    summary: Option<String>,
    paused_target: Option<bool>,
    actions: Option<String>,
    blueprint: Option<String>,
) -> String {
    if actions.is_some() && blueprint.is_some() {
        return coded_error_payload(
            "fortress.plan",
            ErrorCode::InvalidRequest,
            "a plan request names either actions or a blueprint, not both",
        );
    }
    let default_summary = if blueprint.is_some() {
        ""
    } else if actions.is_some() {
        "execute semantic actions"
    } else {
        "unpause the simulation"
    };
    let summary = summary.map_or_else(|| default_summary.to_owned(), |value| value);
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
            let snapshot = guard.adapter.snapshot();

            let source = match (actions, blueprint) {
                (_, Some(raw)) => PlanSource::Blueprint { summary, raw },
                (Some(raw), None) => PlanSource::Actions { summary, raw },
                (None, None) => PlanSource::Pause {
                    summary,
                    paused_target: paused_target.is_some_and(|value| value),
                },
            };
            let intent = match source.intent(IntentId::new(rid), snapshot) {
                Ok(intent) => intent,
                Err(error) => return dfmcp_error_payload("fortress.plan", &error),
            };

            match StaticPlanner::default().prepare(snapshot, &intent, &ctx) {
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
        resume.push(if snapshot.paused {
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
    if session
        .carried
        .iter()
        .any(|carried| carried.state == "dispatched")
        && !snapshot.paused
    {
        resume.push(json!({
            "tool": "fortress.wait",
            "arguments": {"session_id": format!("{}", session.session_id), "max_game_ticks": 100},
            "why": "obligations carried across a durable restart are re-proven by later observations",
        }));
    }
    for alert in &alerts {
        if alert["severity"] == "critical" {
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
        .front()
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
        "last_plan_actions": session.last_plan_actions.iter().copied().map(action_view).collect::<Vec<_>>(),
        "committed_plan_digests": session.commit_receipts.keys().collect::<Vec<_>>(),
        "resume_protocol": resume,
        "authority": "reading this packet grants nothing; every commit and cancel re-checks the session's negotiated grants",
        "epistemic_note": "action states are the last recorded receipts; fortress.wait re-evaluates them against a fresh observation",
    })
}

/// Exclusive spatial leases for every step that excavates or builds, held
/// until the step's obligation deadline. A region another member is working
/// on is refused before any effect; a session never conflicts with itself.
/// The caller restores the prior lease book if the commit then fails.
fn acquire_plan_leases(
    session: &mut LabSession,
    plan: &PreparedPlan,
) -> Result<Vec<(dfmcp_core::StepId, Vec<dfmcp_core::LeaseId>)>> {
    let now = session.adapter.snapshot().tick;
    session.leases.manager.cleanup_expired_leases(now);
    let mut acquired = Vec::new();
    for step in &plan.steps {
        let area = match &step.action {
            Action::DesignateDig { area, .. } => *area,
            Action::Build { footprint, .. } => *footprint,
            _ => continue,
        };
        let ttl = step.obligation.as_ref().map_or(1, |obligation| {
            obligation.deadline_tick.0.saturating_sub(now.0).max(1)
        });
        let lease = session
            .leases
            .manager
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
    Ok(acquired)
}

/// Release the spatial leases an action held once it is terminal.
fn release_action_leases(session: &mut LabSession, action_id: ActionId) {
    if let Some((holder, leases)) = session.leases.by_action.remove(&action_id) {
        for lease in leases {
            // An expired lease may already have been cleaned up.
            let _ = session.leases.manager.release_lease(lease, holder);
        }
    }
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
    let base = session
        .history
        .iter()
        .rev()
        .find(|version| version.anchor() == stale.plan.anchor)
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
        .and_then(|intent| StaticPlanner::default().prepare(&now, &intent, &ctx))
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
    let replayed = stale
        .source
        .intent(IntentId::new(rid), snapshot)
        .and_then(|intent| StaticPlanner::default().prepare(snapshot, &intent, &ctx));
    match replayed {
        Ok(plan) => {
            let digest = plan.digest.to_string();
            payload["rebased_plan"] = json!({
                "forecast": forecast_plan(&session.adapter, &plan, &ctx),
                "live_routing": live_routing_json(&plan),
                "plan_digest": digest,
                "expires_at_tick": plan.expires_at_tick.0,
                "required_capabilities": plan.required_capabilities.iter().map(|c| c.as_str()).collect::<Vec<_>>(),
                "steps": crate::lab_world::plan_steps_json(&plan),
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

/// Most simulated time slices a forecast may take.
const MAX_FORECAST_SLICES: u64 = 400;

/// Counterfactual: commit the sealed plan on a fork of the current world and
/// run deterministic laboratory time forward to every step's obligation
/// deadline, reporting when each step would verify or fail. The fork is
/// discarded; nothing here changes canonical state or grants authority. The
/// forecast assumes the fortress stays as it is now (paused stays paused) and
/// that no other agent acts.
fn forecast_plan(
    adapter: &MemoryAdapter,
    plan: &PreparedPlan,
    template: &OperationContext,
) -> serde_json::Value {
    let mut fork = adapter.clone();
    let context = |fork: &MemoryAdapter| OperationContext {
        anchor: fork.snapshot().anchor(),
        ..template.clone()
    };
    let start = fork.snapshot().tick;
    let unavailable = |reason: &DfmcpError| {
        json!({
            "epistemic_state": "predicted",
            "available": false,
            "reason": {"code": reason.code.as_str(), "message": reason.message},
        })
    };
    let prepared = match fork.prepare(plan, &context(&fork)) {
        Ok(prepared) => prepared,
        Err(error) => return unavailable(&error),
    };
    let receipt = match fork.commit(plan, &prepared, &context(&fork)) {
        Ok(receipt) => receipt,
        Err(error) => return unavailable(&error),
    };
    let mut outcomes: Vec<(dfmcp_core::StepId, ActionId, CommitState, Option<u64>)> = receipt
        .actions
        .iter()
        .map(|action| {
            let at = action.state.is_terminal().then_some(start.0);
            (action.step_id, action.action_id, action.state, at)
        })
        .collect();
    let horizon = plan
        .steps
        .iter()
        .filter_map(|step| step.obligation.as_ref().map(|o| o.deadline_tick.0))
        .max()
        .unwrap_or(start.0);
    let blocked_by_pause =
        fork.snapshot().paused && outcomes.iter().any(|(_, _, state, _)| !state.is_terminal());
    let span = horizon.saturating_sub(start.0);
    let slice = span
        .div_ceil(MAX_FORECAST_SLICES)
        .max(dfmcp_intent::effects::DEFAULT_POLL_INTERVAL_TICKS);
    if !blocked_by_pause && span > 0 {
        let mut elapsed = 0u64;
        while elapsed <= span && outcomes.iter().any(|(_, _, state, _)| !state.is_terminal()) {
            if fork.advance_ticks(slice).is_err() {
                break;
            }
            elapsed += slice;
            for outcome in &mut outcomes {
                if outcome.2.is_terminal() {
                    continue;
                }
                if let Ok(polled) = fork.poll_action(outcome.1, &context(&fork)) {
                    outcome.2 = polled.state;
                    if polled.state.is_terminal() {
                        outcome.3 = Some(fork.snapshot().tick.0);
                    }
                }
            }
        }
    }
    let steps: Vec<serde_json::Value> = outcomes
        .iter()
        .map(|(step, _, state, at)| {
            json!({
                "step": step.get(),
                "predicted_state": format!("{state:?}"),
                "predicted_terminal_tick": at,
            })
        })
        .collect();
    let completes = outcomes
        .iter()
        .all(|(_, _, state, _)| *state == CommitState::Verified);
    json!({
        "epistemic_state": "predicted",
        "available": true,
        "method": "deterministic_laboratory_simulation_on_a_discarded_fork",
        "from_tick": start.0,
        "horizon_tick": horizon,
        "predicted_complete": completes,
        "predicted_completion_tick": completes.then(|| outcomes.iter().filter_map(|o| o.3).max()).flatten(),
        "resolution_ticks": slice,
        "cadence_note": "deferred steps dispatch when a wait observes their prerequisites, so real completion also depends on how often the agent waits",
        "blocked_by_pause": blocked_by_pause,
        "steps": steps,
        "assumes": "the fortress stays as it is now and no other agent acts; a prediction is not evidence",
    })
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
                    LiveRequest::Furniture { kind, target } => {
                        json!({"furniture": kind.as_str(), "target": target})
                    }
                    LiveRequest::WorkOrder { spec } => {
                        json!({"recipe": spec.recipe().as_str(), "amount": spec.amount()})
                    }
                    LiveRequest::WorkDetail { units, assigned } => {
                        json!({"units": units, "assigned": assigned})
                    }
                };
                let requires: Vec<String> = routed
                    .requires
                    .iter()
                    .map(|r| match r {
                        LiveResolution::FurnitureItem { kind } => {
                            format!(
                                "an exact unclaimed {} item from a live inventory read",
                                kind.as_str()
                            )
                        }
                        LiveResolution::WorkDetailForLabor { labor } => {
                            format!("the live work detail that carries labor {labor}")
                        }
                    })
                    .collect();
                json!({
                    "step": step.step.get(),
                    "routable": true,
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
                        // The commit record precedes the effect: after a crash a
                        // recorded plan with no step state was never dispatched.
                        let source = pending.source.durable();
                        let digest = pending.plan.digest;
                        let intent = pending.plan.intent_id.get();
                        if let Err(error) = with_durable_store(|store| {
                            store.persist_commit(sealed, digest, intent, source)
                        }) {
                            guard.leases = leases_before;
                            guard.pending = Some(pending);
                            return dfmcp_error_payload("fortress.commit", &error);
                        }
                    }
                    match guard.adapter.commit(&pending.plan, &prepared, &commit_ctx) {
                        Ok(receipt) => {
                            if sealed.is_some() {
                                guard
                                    .durable_plans
                                    .insert(pending.plan.digest, pending.plan.clone());
                            }
                            if guard.objectives.len() == MAX_OBJECTIVES {
                                guard.objectives.remove(0);
                            }
                            let committed_tick = guard.adapter.snapshot().tick.0;
                            guard.objectives.push(Objective {
                                plan_digest: plan_digest.clone(),
                                summary: pending.plan.summary.clone(),
                                terminal: pending.plan.terminal_condition.clone(),
                                committed_tick,
                            });
                            guard.last_action =
                                receipt.actions.first().map(|action| action.action_id);
                            guard.last_plan_actions = receipt
                                .actions
                                .iter()
                                .map(|action| action.action_id)
                                .collect();
                            for action in &receipt.actions {
                                if !action.state.is_terminal()
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
                                    if action.state.is_terminal() {
                                        release_action_leases(guard, action.action_id);
                                    }
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
                                    "message": action.message,
                                })).collect::<Vec<_>>(),
                                "observed_anchor": anchor_json(&receipt.observed_anchor),
                                "paused": paused,
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
            {
                let (_, entry_ctx) = match next_context(guard) {
                    Ok(value) => value,
                    Err(error) => return dfmcp_error_payload("fortress.wait", &error),
                };
                if let Err(error) =
                    authorize_entry(&entry_ctx, Capability::Observe, RiskTier::ReadOnly)
                {
                    return dfmcp_error_payload("fortress.wait", &error);
                }
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
            // dependents) and retire the ones that reached a terminal state.
            let mut polled_actions = Vec::new();
            let mut still_open = Vec::new();
            for open in guard.open_actions.clone() {
                let (_, poll_ctx) = match next_context(guard) {
                    Ok(value) => value,
                    Err(error) => return dfmcp_error_payload("fortress.wait", &error),
                };
                match guard.adapter.poll_action(open, &poll_ctx) {
                    Ok(receipt) => {
                        if receipt.state.is_terminal() {
                            release_action_leases(guard, open);
                        } else {
                            still_open.push(open);
                        }
                        polled_actions.push(json!({
                            "action_id": format!("{open}"),
                            "step": receipt.step_id.get(),
                            "state": format!("{:?}", receipt.state),
                            "message": receipt.message,
                        }));
                    }
                    Err(error) => return dfmcp_error_payload("fortress.wait", &error),
                }
            }
            guard.open_actions = still_open;
            let snapshot = guard.adapter.snapshot();
            let mut payload = match (action_id, task) {
                (Some(action_id), Some(task)) => json!({
                    "ok": true,
                    "session_id": format!("{}", guard.session_id),
                    "action_id": format!("{}", action_id),
                    "task_id": task.task_id,
                    "status": task.status.as_str(),
                    "commit_state": format!("{:?}", task.commit_state),
                    "summary": task.summary,
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
            if max_game_ticks.is_some() {
                payload["advanced_game_ticks"] = json!(advanced);
                payload["game_tick"] = json!(snapshot.tick.0);
                payload["polled_actions"] = json!(polled_actions);
                payload["open_actions_remaining"] = json!(guard.open_actions.len());
                payload["world_alerts"] =
                    json!(crate::lab_world::world_alerts(guard.adapter.snapshot()));
                payload["objectives"] = objectives_json(guard);
                if !guard.carried.is_empty() {
                    // Proven against this observation by the durable hook that
                    // runs after this call; report the prior evaluation plus
                    // an immediate re-evaluation so the agent sees progress now.
                    let observed = guard.adapter.snapshot().clone();
                    payload["carried_obligations"] = json!(
                        guard
                            .carried
                            .iter()
                            .map(|carried| {
                                let mut view = carried.to_json();
                                if carried.state == "dispatched" {
                                    view["proven_now"] =
                                        json!(dfmcp_world::evaluate(&observed, &carried.proof));
                                }
                                view
                            })
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
/// the historical behaviour) or every nonterminal action of the most recent
/// committed plan (`scope="plan"`), draining dependents before their
/// prerequisites and reporting measurable drain progress. A finalize
/// certificate is issued only once the plan is quiescent.
pub(crate) fn cancel_in_scope(
    session_id: Option<String>,
    mode: Option<String>,
    scope: Option<String>,
) -> String {
    let plan_scope = match scope.as_deref() {
        None | Some("last_action") => false,
        Some("plan") => true,
        Some(other) => {
            return coded_error_payload(
                "fortress.cancel",
                ErrorCode::InvalidRequest,
                &format!("unsupported cancellation scope {other:?}; use last_action or plan"),
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
                    authorize_entry(&entry_ctx, Capability::ControlClock, RiskTier::Reversible)
                {
                    return dfmcp_error_payload("fortress.cancel", &error);
                }
            }
            let Some(action_id) = guard.last_action else {
                return coded_error_payload(
                    "fortress.cancel",
                    ErrorCode::Conflict,
                    "no committed action to cancel; call fortress_commit first",
                );
            };
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
            if plan_scope {
                return drain_plan(guard, cancel_mode);
            }
            let (_, ctx) = match next_context(guard) {
                Ok(value) => value,
                Err(error) => return dfmcp_error_payload("fortress.cancel", &error),
            };
            match guard.adapter.request_cancel(action_id, cancel_mode, &ctx) {
                Ok(request) => {
                    let (_, finalize_ctx) = match next_context(guard) {
                        Ok(value) => value,
                        Err(error) => return dfmcp_error_payload("fortress.cancel", &error),
                    };
                    match guard.adapter.finalize_cancel(action_id, &finalize_ctx) {
                Ok(finalized) => json!({
                    "ok": true,
                    "session_id": format!("{}", guard.session_id),
                    "action_id": format!("{}", finalized.action_id),
                    "requested_state": format!("{:?}", request.state),
                    "final_state": format!("{:?}", finalized.state),
                    "note": "cancellation is request/drain/compensate/finalize; records are never deleted",
                })
                .to_string(),
                Err(error) => dfmcp_error_payload("fortress.cancel", &error),
                }
                }
                Err(error) => dfmcp_error_payload("fortress.cancel", &error),
            }
        },
    )
}

fn drain_plan(guard: &mut LabSession, cancel_mode: CancelMode) -> String {
    let actions = guard.last_plan_actions.clone();
    let mut steps = Vec::with_capacity(actions.len());
    let mut already_terminal = 0usize;
    let mut compensated = 0usize;
    let mut cancelled = 0usize;
    let mut failure = None;
    // Dependents are later steps; drain them first so no prerequisite is
    // withdrawn underneath work that still depends on it.
    for action_id in actions.iter().rev().copied() {
        let before = match next_context(guard)
            .and_then(|(_, ctx)| guard.adapter.poll_action(action_id, &ctx))
        {
            Ok(receipt) => receipt,
            Err(error) => {
                failure = Some(error);
                break;
            }
        };
        if before.state.is_terminal() {
            already_terminal += 1;
            steps.push(json!({
                "action_id": format!("{action_id}"),
                "step": before.step_id.get(),
                "before": format!("{:?}", before.state),
                "after": format!("{:?}", before.state),
                "drained": false,
            }));
            continue;
        }
        let outcome = next_context(guard)
            .and_then(|(_, ctx)| guard.adapter.request_cancel(action_id, cancel_mode, &ctx))
            .and_then(|_| next_context(guard))
            .and_then(|(_, ctx)| guard.adapter.finalize_cancel(action_id, &ctx));
        match outcome {
            Ok(finalized) => {
                match finalized.state {
                    CommitState::Compensated => compensated += 1,
                    _ => cancelled += 1,
                }
                steps.push(json!({
                    "action_id": format!("{action_id}"),
                    "step": before.step_id.get(),
                    "before": format!("{:?}", before.state),
                    "after": format!("{:?}", finalized.state),
                    "drained": true,
                }));
            }
            Err(error) => {
                failure = Some(error);
                break;
            }
        }
    }
    for action_id in &actions {
        let terminal = guard
            .adapter
            .action_receipt(*action_id)
            .is_some_and(|receipt| receipt.state.is_terminal());
        if terminal {
            release_action_leases(guard, *action_id);
        }
    }
    steps.reverse();
    let total = actions.len();
    let drained = compensated + cancelled;
    let remaining = total.saturating_sub(already_terminal + drained);
    let quiescent = failure.is_none() && remaining == 0;
    let anchor = guard.adapter.snapshot().anchor();
    let finalize_certificate = quiescent.then(|| {
        let mut bytes = b"dfmcp-lab-plan-drain-certificate-v1".to_vec();
        for step in &steps {
            bytes.extend_from_slice(step.to_string().as_bytes());
        }
        bytes.extend_from_slice(anchor.state_hash.as_bytes());
        json!({
            "digest": Digest32::of_bytes(&bytes).to_string(),
            "anchor": anchor_json(&anchor),
            "statement": "every action of the plan is terminal; no dispatched work remains",
        })
    });
    let progress = json!({
        "actions_total": total,
        "already_terminal": already_terminal,
        "drained": drained,
        "compensated": compensated,
        "cancelled": cancelled,
        "remaining_nonterminal": remaining,
        "quiescent": quiescent,
    });
    match failure {
        None => json!({
            "ok": true,
            "session_id": format!("{}", guard.session_id),
            "scope": "plan",
            "drain_progress": progress,
            "steps": steps,
            "finalize_certificate": finalize_certificate,
            "observed_anchor": anchor_json(&anchor),
            "note": "verified actions are history and are not rewritten; plan an inverse to undo them",
        })
        .to_string(),
        Some(error) => {
            let mut payload: serde_json::Value =
                serde_json::from_str(&dfmcp_error_payload("fortress.cancel", &error))
                    .unwrap_or_else(|_| json!({"ok": false}));
            payload["scope"] = json!("plan");
            payload["drain_progress"] = progress;
            payload["steps"] = json!(steps);
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
                    guard.objectives.clear();
                    if guard.durable_scenario.is_some() {
                        let fortress = guard.fortress_id;
                        let digests: BTreeSet<Digest32> = guard
                            .durable_plans
                            .keys()
                            .copied()
                            .chain(guard.carried.iter().map(|c| c.plan_digest))
                            .collect();
                        guard.durable_plans.clear();
                        guard.carried.clear();
                        if let Err(error) = with_durable_store(|store| {
                            for digest in &digests {
                                store.retire_commit(fortress, *digest)?;
                            }
                            Ok(())
                        }) {
                            guard.durability_fault = Some(error.message);
                        }
                    }
                    json!({
                    "ok": true,
                    "session_id": format!("{}", guard.session_id),
                    "checkpoint_id": format!("{}", receipt.checkpoint_id),
                    "prior_anchor": anchor_json(&receipt.prior_anchor),
                    "restored_anchor": anchor_json(&receipt.restored_anchor),
                    "content_digest": receipt.content_digest.to_string(),
                    "note": "new observation epoch; pending plans and action handles were invalidated",
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

                json!({
                    "ok": true,
                    "session_id": format!("{}", guard.session_id),
                    "target_entity": format!("{}", target_id.get()),
                    "entity_found": entity_record.is_some(),
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
            let report =
                DoctorInspector.generate_report(active_sessions_count, health_opt, None, 0, 0);

            match health_res {
                Ok(health) => json!({
                    "ok": true,
                    "session_id": format!("{}", guard.session_id),
                    "status": if report.is_healthy { "healthy" } else { "degraded" },
                    "active_sessions_count": report.active_sessions_count,
                    "adapter": health.identity.name,
                    "compatibility": format!("{:?}", health.identity.compatibility),
                    "fortress_loaded": health.fortress_loaded,
                    "findings": report.findings,
                    "warnings": health.warnings,
                    "current_anchor": health.current_anchor.as_ref().map(anchor_json),
                    "durability": durability_json(guard),
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
