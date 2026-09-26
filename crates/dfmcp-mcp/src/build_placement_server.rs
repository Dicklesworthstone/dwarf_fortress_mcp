#![forbid(unsafe_code)]
//! Isolated furniture/1.19 development control through the frozen eleven tools.
//! One session owns private journal custody and its original preparation source.
use dfmcp_adapter::build_placement::journal::private_file::{PrivateBuildFile, open_private_build};
use dfmcp_adapter::build_placement::journal::{BuildGuard, BuildInventory, BuildMode, BuildSource};
use dfmcp_adapter::build_placement::rpc::BuildRpc;
use dfmcp_adapter::build_placement::session::BuildSession;
use dfmcp_adapter::build_placement::{BuildBinding, BuildKind, BuildPlan, BuildSelection};
use dfmcp_adapter::control_effect_journal::EffectJournalStorage;
use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, Digest32, ErrorCode, GameTick, LeaseManager,
    ObservationCursor, OperationContext, RequestId, Result, RiskTier, SessionId, StateAnchor,
    WorkBudget,
};
use fastmcp_rust::modern::ServerBuilder;
use fastmcp_rust::prelude::*;
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::{
    Mutex, MutexGuard, TryLockError,
    atomic::{AtomicU64, Ordering},
};
use std::time::{Duration, Instant};
mod policy;
mod presentation;
mod runtime;
use presentation::{OUTPUT_BYTES, digest, failure, inventory, packet};
use runtime::Config;

const FAMILY: u128 = 19u128 << 57;
const MAX_WORK_BYTES: u64 = 1024 * 1024 * 1024;
// Conservative admission reservations cover complete bounded journal replays,
// fsync/readback, one source lifecycle, and an intact final packet.
const VIEW_BYTES: u64 = 20 * 1024 * 1024;
const LOCAL_BYTES: u64 = 32 * 1024 * 1024;
const EFFECT_BYTES: u64 = 512 * 1024 * 1024;
const SOURCE_BYTES: u64 = 4 * 1024 * 1024;
static NEXT: AtomicU64 = AtomicU64::new(1);
static SESSION: Mutex<Option<Entry>> = Mutex::new(None);
struct Entry {
    state: State<PrivateBuildFile, BuildRpc>,
    config: Config,
}
fn error(code: ErrorCode, message: &str) -> dfmcp_core::DfmcpError {
    dfmcp_core::DfmcpError::new(code, message)
}
fn exhausted() -> dfmcp_core::DfmcpError {
    error(
        ErrorCode::BudgetExceeded,
        "complete furniture work and response exceed the allowance",
    )
}
fn unbound(op: &str, e: &dfmcp_core::DfmcpError) -> String {
    let mut result = failure(e);
    result["effect_may_have_occurred"] = json!(matches!(op, "fortress.commit" | "fortress.cancel"));
    packet(op, result, None, None, None, None, None, None, None)
}
fn key(raw: &str) -> Result<()> {
    if raw.is_empty()
        || raw.len() > 128
        || !raw
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
    {
        return Err(error(
            ErrorCode::InvalidRequest,
            "invalid furniture operation key",
        ));
    }
    Ok(())
}
fn parse_selection(raw: &str) -> Result<BuildSelection> {
    if raw.len() > 128 {
        return Err(error(
            ErrorCode::InvalidRequest,
            "furniture selection exceeds its bound",
        ));
    }
    let (kind, item, x, y, z): (String, u32, u32, u32, u32) =
        serde_json::from_str(raw).map_err(|_| {
            error(
                ErrorCode::InvalidRequest,
                "selection must be [kind,item_id,x,y,z]",
            )
        })?;
    let kind = match kind.as_str() {
        "bed" => BuildKind::Bed,
        "chair" => BuildKind::Chair,
        "table" => BuildKind::Table,
        _ => {
            return Err(error(
                ErrorCode::InvalidRequest,
                "only ordinary bed, chair, or table is supported",
            ));
        }
    };
    BuildSelection::new(kind, item, [x, y, z])
}
struct Review {
    key: String,
    plan: Digest32,
    witness: Digest32,
    seal: Digest32,
}
struct State<S, N> {
    id: SessionId,
    request: u128,
    budget: WorkBudget,
    grants: Vec<CapabilityGrant>,
    control: BuildSession<S, N>,
    binding: BuildBinding,
    policy: policy::Policy,
    leases: LeaseManager,
    review: Option<Review>,
    historical: Option<BuildInventory>,
    pending_hint: Option<Value>,
}
impl<S: EffectJournalStorage, N: BuildSource> State<S, N> {
    fn new(mut control: BuildSession<S, N>, c: &OperationContext, config: &Config) -> Result<Self> {
        let view = control.inventory(c)?;
        let binding = control.binding().clone();
        config.matches(&binding)?;
        let mut leases = LeaseManager::new();
        let mut current = c.clone();
        current.anchor.tick = GameTick(control.high_tick());
        let policy = policy::Policy::new(
            config.clone(),
            binding.clone(),
            view.journal_id,
            &current,
            &mut leases,
        )?;
        Ok(Self {
            id: c.session_id,
            request: c.request_id.get(),
            budget: c.budget,
            grants: c.grants.clone(),
            control,
            binding,
            policy,
            leases,
            review: None,
            historical: Some(view),
            pending_hint: None,
        })
    }
    fn context(&mut self, write: bool, wall: Option<u64>) -> Result<OperationContext> {
        self.request = self.request.checked_add(1).ok_or_else(exhausted)?;
        let mut budget = self.budget;
        if let Some(w) = wall {
            budget.max_wall_millis = budget.max_wall_millis.min(w);
        }
        let mut c = context(
            self.id,
            RequestId::new(self.request),
            self.binding.fortress().fortress_id(),
            self.control.high_tick(),
            budget,
            self.control.mode(),
            false,
        );
        c.grants = self
            .grants
            .iter()
            .filter(|g| write || !matches!(g.capability, Capability::Plan | Capability::Construct))
            .cloned()
            .collect();
        Ok(c)
    }
    fn abandon(&mut self) {
        self.review = None;
        self.control.abandon_preparation();
    }
    fn seal(&self) -> Option<Digest32> {
        if self.control.has_preparation_connection() {
            self.review.as_ref().map(|v| v.seal)
        } else {
            None
        }
    }
    fn disclosure_context(&self, c: &OperationContext) -> Result<OperationContext> {
        let mut current = c.clone();
        current.anchor.tick = GameTick(current.anchor.tick.get().max(self.control.high_tick()));
        if current.session_id != self.id
            || current.anchor.fortress_id != self.binding.fortress().fortress_id()
        {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "furniture evidence belongs to another session or fortress",
            ));
        }
        // Cached evidence has the same disclosure boundary as a fresh journal
        // read. A failed custody or budget check cannot revive expired Query.
        current.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
        Ok(current)
    }
    fn failed(&self, op: &str, c: Option<&OperationContext>, e: &dfmcp_core::DfmcpError) -> String {
        let current = match c.map(|c| self.disclosure_context(c)) {
            Some(Ok(c)) => c,
            Some(Err(denied)) => return unbound(op, &denied),
            None => return unbound(op, e),
        };
        let mut result = failure(e);
        result["effect_may_have_occurred"] =
            json!(matches!(op, "fortress.commit" | "fortress.cancel"));
        result["pending_identity_hint"] = self.pending_hint.clone().unwrap_or(Value::Null);
        result["source_summary"] = self
            .control
            .native_summary()
            .map(presentation::source_summary)
            .unwrap_or(Value::Null);
        packet(
            op,
            result,
            Some(&current),
            Some(&self.binding),
            None,
            self.historical.as_ref(),
            Some(&self.policy),
            None,
            None,
        )
    }
    fn close_packet(
        &self,
        c: &OperationContext,
        view: Option<&BuildInventory>,
        release: bool,
    ) -> String {
        let result = json!({"ok":true,"scope":"session","closed":true,"release_for_recovery":release,
            "effects_cancelled":false,"history_erased":false,"native_quiescence_proven":false});
        match self.disclosure_context(c) {
            Ok(current) => packet(
                "fortress.cancel",
                result,
                Some(&current),
                Some(&self.binding),
                view,
                self.historical.as_ref(),
                None,
                None,
                None,
            ),
            // Explicit recovery release may relinquish owned custody even
            // after Query revocation. It publishes no retained fortress facts.
            Err(_) => packet(
                "fortress.cancel",
                result,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
            ),
        }
    }
}
fn grants(fortress: dfmcp_core::FortressId, mode: BuildMode, write: bool) -> Vec<CapabilityGrant> {
    [
        Capability::Query,
        Capability::Observe,
        Capability::Plan,
        Capability::Construct,
    ]
    .into_iter()
    .filter(|v| {
        *v == Capability::Query
            || (mode == BuildMode::Control && (*v == Capability::Observe || write))
    })
    .map(|capability| CapabilityGrant {
        capability,
        scope: CapabilityScope {
            fortress_id: Some(fortress),
            ..CapabilityScope::default()
        },
        max_risk: if matches!(capability, Capability::Plan | Capability::Construct) {
            RiskTier::Guarded
        } else {
            RiskTier::ReadOnly
        },
        expires_at_tick: None,
        remaining_uses: None,
    })
    .collect()
}
fn context(
    id: SessionId,
    request_id: RequestId,
    fortress: dfmcp_core::FortressId,
    tick: u64,
    budget: WorkBudget,
    mode: BuildMode,
    write: bool,
) -> OperationContext {
    OperationContext {
        session_id: id,
        request_id,
        budget,
        cancellation_requested: false,
        anchor: StateAnchor {
            fortress_id: fortress,
            cursor: ObservationCursor::ORIGIN,
            tick: GameTick(tick),
            state_hash: Digest32::ZERO,
        },
        grants: grants(fortress, mode, write),
    }
}
struct Work {
    deadline: Instant,
    bytes: u64,
}
impl Work {
    fn new(c: &OperationContext, started: Instant) -> Result<Self> {
        c.budget.validate()?;
        if c.budget.max_wall_millis > 60000
            || c.budget.max_bytes > MAX_WORK_BYTES
            || c.budget.max_output_tokens > 65536
            || u64::from(c.budget.max_output_tokens) * 4 < OUTPUT_BYTES
        {
            return Err(exhausted());
        }
        let out = Self {
            deadline: started
                .checked_add(Duration::from_millis(c.budget.max_wall_millis))
                .ok_or_else(exhausted)?,
            bytes: c
                .budget
                .max_bytes
                .checked_sub(OUTPUT_BYTES + 2 * VIEW_BYTES)
                .ok_or_else(exhausted)?,
        };
        out.current(c)?;
        Ok(out)
    }
    fn current(&self, c: &OperationContext) -> Result<OperationContext> {
        let remaining = self
            .deadline
            .checked_duration_since(Instant::now())
            .ok_or_else(exhausted)?
            .as_millis();
        if remaining == 0 {
            return Err(exhausted());
        }
        let mut out = c.clone();
        out.budget.max_wall_millis = u64::try_from(remaining).map_err(|_| exhausted())?;
        out.budget.max_bytes = self.bytes;
        Ok(out)
    }
    fn take(&mut self, c: &OperationContext, bytes: u64) -> Result<OperationContext> {
        let mut out = self.current(c)?;
        self.bytes = self.bytes.checked_sub(bytes).ok_or_else(exhausted)?;
        out.budget.max_bytes = bytes;
        Ok(out)
    }
    fn view(&self, c: &OperationContext) -> Result<OperationContext> {
        let mut out = self.current(c)?;
        out.budget.max_bytes = VIEW_BYTES;
        Ok(out)
    }
}
#[derive(Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
enum Query {
    Records {
        limit: Option<usize>,
        offset: Option<usize>,
        head: Option<String>,
    },
    Get {
        idempotency_key: String,
        plan_digest: String,
    },
    Selection {
        witness: String,
    },
    Schema {},
}
impl Query {
    fn parse(raw: &str) -> Result<Self> {
        if raw.len() > 2048 {
            return Err(error(
                ErrorCode::InvalidRequest,
                "furniture query exceeds 2 KiB",
            ));
        }
        let shape: Value = serde_json::from_str(raw)
            .map_err(|_| error(ErrorCode::InvalidRequest, "invalid furniture query"))?;
        if shape
            .as_object()
            .is_some_and(|m| m.values().any(Value::is_null))
        {
            return Err(error(
                ErrorCode::InvalidRequest,
                "omit absent query fields instead of null",
            ));
        }
        // Parse original bytes, so duplicate fields cannot hide behind a Value map.
        let value: Self = serde_json::from_str(raw)
            .map_err(|_| error(ErrorCode::InvalidRequest, "invalid closed furniture query"))?;
        match &value {
            Self::Records {
                limit,
                offset,
                head,
            } => {
                if !(1..=8).contains(&limit.unwrap_or(8))
                    || offset.unwrap_or(0) > 256
                    || (offset.unwrap_or(0) > 0 && head.is_none())
                {
                    return Err(error(
                        ErrorCode::InvalidRequest,
                        "invalid furniture record page",
                    ));
                }
                if let Some(h) = head {
                    digest(h)?;
                }
            }
            Self::Get {
                idempotency_key,
                plan_digest,
            } => {
                key(idempotency_key)?;
                digest(plan_digest)?;
            }
            Self::Selection { witness } => {
                digest(witness)?;
            }
            Self::Schema {} => {}
        }
        Ok(value)
    }
}
enum Action {
    Observe(BuildSelection),
    Plan {
        key: String,
        witness: Digest32,
    },
    Commit {
        key: String,
        plan: Digest32,
        seal: Digest32,
    },
    Recover(String, Digest32, bool),
    Explain(String, Digest32),
    Query(Query),
    Inventory,
    Denied,
}
fn schema() -> Value {
    json!({"schema":"dfmcp.build-placement-mcp-query/1","max_bytes":2048,"closed":true,
    "modes":{"records":{"limit":"optional integer 1..8","offset":"optional integer 0..256; head required when nonzero","head":"optional lowercase SHA-256 exact journal head"},
        "get":{"idempotency_key":"1..128 ASCII letters/digits/dot/underscore/hyphen","plan_digest":"lowercase SHA-256"},"selection":{"witness":"lowercase SHA-256"},"schema":{}},
    "native_calls":0,"null_fields_allowed":false,"duplicate_fields_allowed":false})
}
#[allow(clippy::too_many_arguments)]
fn perform<S, N, G, F>(
    state: &mut State<S, N>,
    c: &OperationContext,
    work: &mut Work,
    before: &BuildInventory,
    action: Action,
    runtime: &mut G,
    connect: F,
) -> Result<Value>
where
    S: EffectJournalStorage,
    N: BuildSource,
    G: BuildGuard,
    F: FnOnce(BuildSelection, bool, &BuildBinding, &OperationContext, Duration) -> Result<N>,
{
    match action {
        Action::Observe(selection) => {
            state.abandon();
            let mut guard = policy::Guard {
                policy: &state.policy,
                leases: &state.leases,
                runtime,
                confirmation: None,
            };
            let capture = state.control.observe(
                selection,
                &work.take(c, EFFECT_BYTES)?,
                |b, c, d| connect(selection, false, b, c, d),
                &mut guard,
            )?;
            Ok(
                json!({"ok":true,"observation":presentation::observation(&capture),"game_mutation_dispatched":false,
                    "source_summary":state.control.native_summary().map(presentation::source_summary)}),
            )
        }
        Action::Plan { key, witness } => {
            if let Some(review) = &state.review {
                if review.key != key || review.witness != witness {
                    return Err(error(
                        ErrorCode::Conflict,
                        "another exact furniture review is pending",
                    ));
                }
                let old = state
                    .control
                    .get(&key, review.plan, &work.take(c, LOCAL_BYTES)?)?;
                state.policy.evaluate(old.plan(), c, &state.leases)?;
                return Ok(
                    json!({"ok":true,"plan":presentation::record(&old),"plan_digest":review.plan.to_string(),"review_seal":review.seal.to_string(),"replayed_locally":true}),
                );
            }
            let capture = state
                .control
                .selected()
                .filter(|v| v.witness() == witness)
                .ok_or_else(|| {
                    error(
                        ErrorCode::StaleAnchor,
                        "planning requires this session's exact selected observation",
                    )
                })?;
            let plan = BuildPlan::new(&key, capture.clone())?;
            state.policy.evaluate(&plan, c, &state.leases)?;
            state.pending_hint = Some(
                json!({"idempotency_key":key,"plan_digest":plan.digest().to_string(),"outcome":"unverified","retry_commit_permitted":false}),
            );
            let mut guard = policy::Guard {
                policy: &state.policy,
                leases: &state.leases,
                runtime,
                confirmation: None,
            };
            let entry =
                state
                    .control
                    .prepare(&key, witness, &work.take(c, EFFECT_BYTES)?, &mut guard)?;
            if state.control.has_preparation_connection() {
                state.review = Some(Review {
                    key,
                    plan: entry.plan().digest(),
                    witness,
                    seal: state.policy.seal(entry.plan()),
                });
            }
            Ok(
                json!({"ok":true,"plan":presentation::record(&entry),"plan_digest":entry.plan().digest().to_string(),"review_seal":state.seal().map(|s|s.to_string()),"game_mutation_dispatched":false}),
            )
        }
        Action::Commit { key, plan, seal } => {
            let old = state.control.get(&key, plan, &work.take(c, LOCAL_BYTES)?)?;
            if !old.unresolved() {
                return Ok(
                    json!({"ok":true,"effect":presentation::record(&old),"native_calls":0,"historical_replay":true}),
                );
            }
            if !state
                .review
                .as_ref()
                .is_some_and(|r| r.key == key && r.plan == plan && r.seal == seal)
            {
                return Err(error(
                    ErrorCode::CapabilityDenied,
                    "exact local furniture review must be confirmed",
                ));
            }
            state
                .policy
                .evaluate(old.plan(), &work.current(c)?, &state.leases)?;
            let review = state.review.take().ok_or_else(|| {
                error(ErrorCode::CapabilityDenied, "furniture review unavailable")
            })?;
            let mut guard = policy::Guard {
                policy: &state.policy,
                leases: &state.leases,
                runtime,
                confirmation: Some(review.seal),
            };
            let result = state
                .control
                .commit(&key, plan, &work.take(c, EFFECT_BYTES)?, &mut guard);
            state.abandon();
            Ok(
                json!({"ok":true,"effect":presentation::record(&result?),"building_completion_proven":false,"retry_commit_permitted":false}),
            )
        }
        Action::Recover(key, plan, cancel) => {
            state.abandon();
            let old = state.control.get(&key, plan, &work.take(c, LOCAL_BYTES)?)?;
            let selection = old.plan().before().selection();
            let mut guard = policy::Guard {
                policy: &state.policy,
                leases: &state.leases,
                runtime,
                confirmation: None,
            };
            let entry = state.control.recover(
                &key,
                plan,
                cancel,
                &work.take(c, EFFECT_BYTES)?,
                |b, c, d| connect(selection, true, b, c, d),
                &mut guard,
            )?;
            Ok(
                json!({"ok":true,"effect":presentation::record(&entry),"commit_retried":false,"building_undone":false}),
            )
        }
        Action::Explain(key, plan) => {
            let entry = state.control.get(&key, plan, &work.take(c, LOCAL_BYTES)?)?;
            Ok(json!({"ok":true,"record":presentation::record(&entry),"native_calls":0}))
        }
        Action::Query(Query::Get {
            idempotency_key,
            plan_digest,
        }) => {
            let entry = state.control.get(
                &idempotency_key,
                digest(&plan_digest)?,
                &work.take(c, LOCAL_BYTES)?,
            )?;
            Ok(json!({"ok":true,"record":presentation::record(&entry),"native_calls":0}))
        }
        Action::Query(Query::Records {
            limit,
            offset,
            head,
        }) => {
            if let Some(head) = head {
                if digest(&head)? != before.head {
                    return Err(error(
                        ErrorCode::StaleAnchor,
                        "furniture journal continuation head changed",
                    ));
                }
            }
            let limit = limit.unwrap_or(8);
            let offset = offset.unwrap_or(0);
            let mut ordered = before.entries().iter().collect::<Vec<_>>();
            ordered.sort_by_key(|e| (!e.unresolved(), e.plan().key()));
            if offset > ordered.len() {
                return Err(error(
                    ErrorCode::InvalidRequest,
                    "record offset exceeds exact journal inventory",
                ));
            }
            let end = (offset + limit).min(ordered.len());
            let rows = ordered[offset..end]
                .iter()
                .map(|e| presentation::summary(e))
                .collect::<Vec<_>>();
            Ok(
                json!({"ok":true,"records":rows,"journal":inventory(before),"offset":offset,"next_query":if end<ordered.len(){json!({"mode":"records","limit":limit,"offset":end,"head":before.head.to_string()})}else{Value::Null},"native_calls":0}),
            )
        }
        Action::Query(Query::Selection { witness }) => {
            let witness = digest(&witness)?;
            let capture = state
                .control
                .selected()
                .filter(|v| v.witness() == witness)
                .ok_or_else(|| {
                    error(
                        ErrorCode::StaleAnchor,
                        "selected furniture observation is no longer retained",
                    )
                })?;
            Ok(json!({"ok":true,"observation":presentation::observation(capture),"native_calls":0}))
        }
        Action::Query(Query::Schema {}) => {
            Ok(json!({"ok":true,"query_schema":schema(),"native_calls":0}))
        }
        Action::Inventory => Ok(
            json!({"ok":true,"journal":inventory(before),"native_calls":0,"live_game_health_checked":false}),
        ),
        Action::Denied => Err(error(
            ErrorCode::CapabilityDenied,
            "furniture journal is not a game checkpoint or restore",
        )),
    }
}
#[allow(clippy::too_many_arguments)]
fn run_action<S, N, G, F>(
    state: &mut State<S, N>,
    c: OperationContext,
    op: &str,
    action: Result<Action>,
    started: Instant,
    runtime: &mut G,
    connect: F,
) -> String
where
    S: EffectJournalStorage,
    N: BuildSource,
    G: BuildGuard,
    F: FnOnce(BuildSelection, bool, &BuildBinding, &OperationContext, Duration) -> Result<N>,
{
    if op == "fortress.observe" {
        state.abandon();
    }
    let mut work = match Work::new(&c, started) {
        Ok(v) => v,
        Err(e) => return state.failed(op, Some(&c), &e),
    };
    let before = match work.view(&c).and_then(|c| state.control.inventory(&c)) {
        Ok(v) => v,
        Err(e) => {
            state.abandon();
            return state.failed(op, Some(&c), &e);
        }
    };
    state.historical = Some(before.clone());
    let outcome = (|| {
        perform(
            state,
            &work.current(&c)?,
            &mut work,
            &before,
            action?,
            runtime,
            connect,
        )
    })();
    if outcome.is_err()
        && matches!(
            op,
            "fortress.observe"
                | "fortress.plan"
                | "fortress.commit"
                | "fortress.wait"
                | "fortress.cancel"
        )
    {
        state.abandon();
    }
    let display = match state.disclosure_context(&c) {
        Ok(current) => current,
        Err(e) => {
            state.abandon();
            return unbound(op, &e);
        }
    };
    let after = work
        .view(&display)
        .and_then(|c| state.control.inventory(&c));
    let (mut result, verified) = match after {
        Ok(v) => {
            state.historical = Some(v.clone());
            state.pending_hint = None;
            (outcome.unwrap_or_else(|e| failure(&e)), Some(v))
        }
        Err(e) => {
            state.abandon();
            let mut f = failure(&e);
            f["post_operation_inventory_unverified"] = json!(true);
            f["pending_identity_hint"] = state.pending_hint.clone().unwrap_or(Value::Null);
            (f, None)
        }
    };
    if result["ok"] != true {
        result["effect_may_have_occurred"] =
            json!(matches!(op, "fortress.commit" | "fortress.cancel"));
    }
    result["source_summary"] = state
        .control
        .native_summary()
        .map(presentation::source_summary)
        .unwrap_or(Value::Null);
    if op == "fortress.plan"
        && result["error"]["code"] == ErrorCode::EffectIndeterminate.as_str()
        && verified
            .as_ref()
            .is_some_and(|v| v.head == before.head && v.pending().is_none())
        && state
            .control
            .native_summary()
            .is_some_and(|s| s.unresolved())
    {
        result["error"]["message"] = json!(
            "Retained native history blocks new preparation. No new local obligation was created; preserve and reconcile the journal that owns the unresolved native work."
        );
        result["new_local_obligation_created"] = json!(false);
        result["recovery_target"] = json!("journal_owning_unresolved_native_history");
    }
    let rendered = packet(
        op,
        result,
        Some(&display),
        Some(&state.binding),
        verified.as_ref(),
        state.historical.as_ref(),
        Some(&state.policy),
        state.seal(),
        state.control.selected(),
    );
    if rendered.len() as u64 > OUTPUT_BYTES || work.current(&display).is_err() {
        state.abandon();
        return state.failed(
            op,
            Some(&display),
            &error(
                ErrorCode::EffectIndeterminate,
                "complete response unavailable; inspect original journal",
            ),
        );
    }
    rendered
}
fn lock() -> Result<MutexGuard<'static, Option<Entry>>> {
    match SESSION.try_lock() {
        Ok(v) => Ok(v),
        Err(TryLockError::WouldBlock) => Err(error(
            ErrorCode::Conflict,
            "furniture session already serving a request",
        )),
        Err(TryLockError::Poisoned(_)) => Err(error(
            ErrorCode::InternalInvariantViolation,
            "furniture session poisoned; restart for recovery",
        )),
    }
}
fn session_id(raw: &str) -> Result<SessionId> {
    if raw.len() != 32
        || !raw
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(error(
            ErrorCode::InvalidRequest,
            "invalid furniture session ID",
        ));
    }
    let n = u128::from_str_radix(raw, 16)
        .map_err(|_| error(ErrorCode::InvalidRequest, "invalid furniture session ID"))?;
    let id = SessionId::new(n);
    if id.get() != n || !id.is_process_scoped_live() || (n & ((1u128 << 62) - 1)) >> 57 != 19 {
        return Err(error(
            ErrorCode::InvalidRequest,
            "wrong furniture session family",
        ));
    }
    Ok(id)
}
async fn with_session(
    raw: String,
    op: &'static str,
    wall: Option<u64>,
    action: Result<Action>,
) -> String {
    runtime::owned(op, move |control| {
        let result = (|| {
            let id = session_id(&raw)?;
            let mut locked = lock()?;
            let entry = locked
                .as_mut()
                .filter(|e| e.state.id == id)
                .ok_or_else(|| error(ErrorCode::SessionNotFound, "furniture session absent"))?;
            let c = match entry
                .state
                .context(runtime::enabled().unwrap_or(false), wall)
            {
                Ok(c) => c,
                Err(e) => return Ok(entry.state.failed(op, None, &e)),
            };
            if let Err(e) = runtime::boundary(&control, &entry.config, false) {
                entry.state.abandon();
                return Ok(entry.state.failed(op, Some(&c), &e));
            }
            let config = entry.config.clone();
            let mut guard = runtime::Guard {
                control: &control,
                config: &config,
            };
            let rendered = run_action(
                &mut entry.state,
                c.clone(),
                op,
                action,
                control.started,
                &mut guard,
                |selection, recovery, b, c, d| {
                    config.matches(b)?;
                    runtime::connect(
                        &config,
                        selection,
                        if recovery { Some(b) } else { None },
                        c,
                        d,
                    )
                },
            );
            if let Err(e) = runtime::boundary(&control, &config, false) {
                entry.state.abandon();
                return Ok(entry.state.failed(op, Some(&c), &e));
            }
            Ok(rendered)
        })();
        match result {
            Ok(v) => v,
            Err(e) => unbound(op, &e),
        }
    })
    .await
}

#[tool(
    name = "fortress.open_session",
    description = "Open isolated furniture/1.19 development custody. Control requires selection as closed JSON [kind,item_id,x,y,z], kind bed/chair/table. Recover and offline open the existing operator-selected private journal without a native connection. No path, endpoint, token, scope or checkpoint policy is client-selectable. Opening dispatches no preparation or game mutation."
)]
pub async fn fortress_open_session(
    selection: Option<String>,
    max_wall_millis: Option<u64>,
    max_bytes: Option<u64>,
    max_output_tokens: Option<u32>,
) -> String {
    runtime::owned("fortress.open_session",move|control|{
        let result=(||{
            let config=runtime::configuration()?;runtime::boundary(&control,&config,false)?;
            let selected=selection.as_deref().map(parse_selection).transpose()?;
            if (config.mode==BuildMode::Control)!=selected.is_some(){return Err(error(ErrorCode::InvalidRequest,"control requires selection; recovery/offline must omit it"));}
            let budget=WorkBudget{max_wall_millis:max_wall_millis.unwrap_or(10000),max_bytes:max_bytes.unwrap_or(MAX_WORK_BYTES),max_output_tokens:max_output_tokens.unwrap_or(16384),max_entities:65536,max_actions:1,max_game_ticks:0};
            let mut locked=lock()?;if locked.is_some(){return Err(error(ErrorCode::Conflict,"release the current furniture session first"));}
            let seq=NEXT.try_update(Ordering::AcqRel,Ordering::Acquire,|v|(v<(1u64<<57)).then_some(v+1)).map_err(|_|exhausted())?;
            let id=SessionId::new((1u128<<127)|FAMILY|u128::from(seq));
            let mut c=context(id,RequestId::new(1),config.fortress.fortress_id(),0,budget,config.mode,runtime::enabled()?);
            let mut work=Work::new(&c,control.started)?;
            let binding=if let Some(selection)=selected{
                config.selection(selection)?;
                let child=work.take(&c,SOURCE_BYTES)?;
                let source=runtime::connect(&config,selection,None,&child,Duration::from_millis(child.budget.max_wall_millis))?;
                let capture=source.initial_capture().ok_or_else(||error(ErrorCode::AdapterRejected,"furniture bootstrap capture missing"))?;
                if capture.fortress()!=&config.fortress||capture.selection()!=selection{return Err(error(ErrorCode::StaleAnchor,"furniture bootstrap source differs from configuration"));}
                c.anchor.tick=GameTick(capture.tick());let binding=source.binding().clone();drop(source);Some(binding)
            }else{None};
            runtime::boundary(&control,&config,false)?;
            let journal=open_private_build(&config.path,&work.take(&c,EFFECT_BYTES)?,config.mode,binding)?;
            config.matches(journal.binding())?;
            let session=BuildSession::<_,BuildRpc>::new(journal,&work.view(&c)?)?;
            let mut state=State::new(session,&work.take(&c,LOCAL_BYTES)?,&config)?;state.budget=budget;state.grants=c.grants.clone();
            c=state.disclosure_context(&c)?;let view=state.control.inventory(&work.view(&c)?)?;
            let output=packet("fortress.open_session",json!({"ok":true,"session_id":id.to_string(),"mode":mode_name(config.mode),"journal":inventory(&view),
                "capabilities":c.grants.iter().map(|g|g.capability.as_str()).collect::<Vec<_>>(),"planning_observation_retained":false,
                "native_preparation_dispatched":false,"game_mutation_dispatched":false}),Some(&c),Some(&state.binding),Some(&view),None,Some(&state.policy),None,None);
            if output.len() as u64>OUTPUT_BYTES{return Err(exhausted());}work.current(&c)?;runtime::boundary(&control,&config,false)?;
            *locked=Some(Entry{state,config});Ok(output)
        })();match result{Ok(v)=>v,Err(e)=>unbound("fortress.open_session",&e)}
    }).await
}
fn mode_name(mode: BuildMode) -> &'static str {
    match mode {
        BuildMode::Control => "control",
        BuildMode::Recover => "recover",
        BuildMode::Offline => "offline",
    }
}
#[tool(
    name = "fortress.observe",
    description = "Capture one exact ordinary bed/chair/table item and 3x3 target context. selection is closed JSON [kind,item_id,x,y,z]. Retain its original connection for reviewed preparation and one commit. A new observation abandons local permission, never durable pending work."
)]
pub async fn fortress_observe(session_id: String, selection: String) -> String {
    with_session(
        session_id,
        "fortress.observe",
        None,
        parse_selection(&selection).map(Action::Observe),
    )
    .await
}
#[tool(
    name = "fortress.plan",
    description = "Prepare one furniture placement from this session's exact observation witness under current operator scope, item and target lease, protected regions and checkpoint policy. Returns native plan digest and policy-bound review seal. No building insertion occurs at prepare."
)]
pub async fn fortress_plan(
    session_id: String,
    idempotency_key: String,
    observation_witness: String,
) -> String {
    let action = key(&idempotency_key)
        .and_then(|_| digest(&observation_witness))
        .map(|witness| Action::Plan {
            key: idempotency_key,
            witness,
        });
    with_session(session_id, "fortress.plan", None, action).await
}
#[tool(
    name = "fortress.commit",
    description = "Confirm exact plan digest and review seal for one native placement attempt on the original preparation connection. Current policy and complete capture are revalidated after dispatch state is durable. Uncertainty requires original-key recovery. Placed proves historical stage-zero construction-job registration, not building completion or usability."
)]
pub async fn fortress_commit(
    session_id: String,
    idempotency_key: String,
    plan_digest: String,
    review_seal: String,
) -> String {
    let action = key(&idempotency_key)
        .and_then(|_| Ok((digest(&plan_digest)?, digest(&review_seal)?)))
        .map(|(plan, seal)| Action::Commit {
            key: idempotency_key,
            plan,
            seal,
        });
    with_session(session_id, "fortress.commit", None, action).await
}
#[tool(
    name = "fortress.query",
    description = "Bounded local furniture journal inventory, exact record, retained selection or query schema. query is closed JSON with mode records/get/selection/schema. Records page 1..8, with head-bound next_query. No native calls and no commitment from recovered records."
)]
pub async fn fortress_query(session_id: String, query: String) -> String {
    with_session(
        session_id,
        "fortress.query",
        None,
        Query::parse(&query).and_then(|q| match q {
            Query::Get {
                idempotency_key,
                plan_digest,
            } => Ok(Action::Explain(idempotency_key, digest(&plan_digest)?)),
            q => Ok(Action::Query(q)),
        }),
    )
    .await
}
fn effect_action(
    key_value: String,
    raw: String,
    make: impl FnOnce(String, Digest32) -> Action,
) -> Result<Action> {
    key(&key_value)?;
    Ok(make(key_value, digest(&raw)?))
}
#[tool(
    name = "fortress.wait",
    description = "Query the original furniture outcome at most once and synchronize validated evidence. Terminal retained records and absorbing indeterminate receipts return locally. No commit retry, timer, clock advancement or building-completion claim. Querying clears local commit permission."
)]
pub async fn fortress_wait(
    session_id: String,
    idempotency_key: String,
    plan_digest: String,
    max_wall_millis: Option<u64>,
) -> String {
    with_session(
        session_id,
        "fortress.wait",
        max_wall_millis,
        effect_action(idempotency_key, plan_digest, |k, p| {
            Action::Recover(k, p, false)
        }),
    )
    .await
}
#[tool(
    name = "fortress.explain",
    description = "Inspect the exact retained furniture plan, before-capture and native receipt as historical evidence. Offline operation; no new observation and no mutation."
)]
pub async fn fortress_explain(
    session_id: String,
    idempotency_key: String,
    plan_digest: String,
) -> String {
    with_session(
        session_id,
        "fortress.explain",
        None,
        effect_action(idempotency_key, plan_digest, Action::Explain),
    )
    .await
}
async fn close(raw: String, release: bool) -> String {
    runtime::owned("fortress.cancel", move |control| {
        let result = (|| {
            control.checkpoint()?;
            let id = session_id(&raw)?;
            let mut locked = lock()?;
            let entry = locked
                .as_mut()
                .filter(|e| e.state.id == id)
                .ok_or_else(|| error(ErrorCode::SessionNotFound, "furniture session absent"))?;
            let c = entry.state.context(false, None)?;
            let view = if release {
                None
            } else {
                if let Err(e) = runtime::boundary(&control, &entry.config, false) {
                    return Ok(entry.state.failed("fortress.cancel", Some(&c), &e));
                }
                let work = Work::new(&c, control.started)?;
                Some(entry.state.control.inventory(&work.view(&c)?)?)
            };
            if !release && view.as_ref().is_some_and(|v| v.pending().is_some()) {
                return Ok(entry.state.failed(
                    "fortress.cancel",
                    Some(&c),
                    &error(
                        ErrorCode::EffectIndeterminate,
                        "unresolved work requires explicit recovery release",
                    ),
                ));
            }
            let output = entry.state.close_packet(&c, view.as_ref(), release);
            if output.len() as u64 > OUTPUT_BYTES {
                return Err(exhausted());
            }
            drop(locked.take());
            Ok(output)
        })();
        match result {
            Ok(v) => v,
            Err(e) => unbound("fortress.cancel", &e),
        }
    })
    .await
}
#[tool(
    name = "fortress.cancel",
    description = "scope=effect retires only the exact prepared native furniture record under Query recovery authority; cannot undo construction. scope=session releases settled custody, or release_for_recovery=true releases unresolved ownership while preserving all durable evidence. No blind retry after reopening."
)]
pub async fn fortress_cancel(
    session_id: String,
    scope: String,
    idempotency_key: Option<String>,
    plan_digest: Option<String>,
    release_for_recovery: Option<bool>,
) -> String {
    let release = release_for_recovery.unwrap_or(false);
    match (scope.as_str(), idempotency_key, plan_digest) {
        ("session", None, None) => close(session_id, release).await,
        ("effect", Some(k), Some(p)) if !release => {
            with_session(
                session_id,
                "fortress.cancel",
                None,
                effect_action(k, p, |k, p| Action::Recover(k, p, true)),
            )
            .await
        }
        _ => {
            with_session(
                session_id,
                "fortress.cancel",
                None,
                Err(error(
                    ErrorCode::InvalidRequest,
                    "cancel scope and identity disagree",
                )),
            )
            .await
        }
    }
}
#[tool(
    name = "fortress.checkpoint",
    description = "Unavailable: the furniture coordination journal is not a verified Dwarf Fortress game save. Default checkpoint policy refuses placement."
)]
pub async fn fortress_checkpoint(session_id: String) -> String {
    with_session(session_id, "fortress.checkpoint", None, Ok(Action::Denied)).await
}
#[tool(
    name = "fortress.restore",
    description = "Unavailable: replaying furniture journal evidence cannot restore game state or remove a constructed building."
)]
pub async fn fortress_restore(session_id: String) -> String {
    with_session(session_id, "fortress.restore", None, Ok(Action::Denied)).await
}
#[tool(
    name = "fortress.doctor",
    description = "Verify local furniture coordination custody and inventory. Does not inspect live fortress health, building completion, other controllers or production admission."
)]
pub async fn fortress_doctor(session_id: String) -> String {
    with_session(session_id, "fortress.doctor", None, Ok(Action::Inventory)).await
}
pub fn run_stdio() {
    if let Err(e) = runtime::configuration() {
        eprintln!("{e}");
        std::process::exit(1);
    }
    let server=ServerBuilder::new("dfmcp-build-placement-dev",env!("CARGO_PKG_VERSION"))
        .tool(FortressOpenSession).tool(FortressObserve).tool(FortressQuery).tool(FortressPlan).tool(FortressCommit)
        .tool(FortressWait).tool(FortressCancel).tool(FortressCheckpoint).tool(FortressRestore).tool(FortressExplain).tool(FortressDoctor)
        .instructions("Unadmitted furniture/1.19 development control. Observe one exact ordinary furniture item and target, review the prepared plan and policy seal, then commit once on the original connection. Default policy requires an unavailable game checkpoint; only explicit operator disposable-fortress policy permits placement. Query the original key after uncertainty. Reopened preparation cannot commit. Placed proves historical stage-zero construction registration, not completed usable furniture. Canonical world anchor is unavailable. Item and target require host scope and avoid protected regions; no global controller fence. No arbitrary command, clock advancement, game checkpoint or restore.").build();
    crate::run_modern_stdio(server);
}
#[cfg(test)]
mod tests;
