#![forbid(unsafe_code)]
//! Isolated furniture/1.19 development control through the frozen eleven tools.
//! One session owns private journal custody and its original preparation source.
use dfmcp_adapter::build_placement::journal::private_file::{PrivateBuildFile, open_private_build};
use dfmcp_adapter::build_placement::journal::{BuildGuard, BuildInventory, BuildMode, BuildSource};
use dfmcp_adapter::build_placement::rpc::BuildRpc;
use dfmcp_adapter::build_placement::session::BuildSession;
use dfmcp_adapter::build_placement::{BuildBinding, BuildKind, BuildPlan, BuildSelection};
use dfmcp_adapter::control_effect_journal::EffectJournalStorage;
use dfmcp_adapter::furniture_batch::store::open_private_batch;
use dfmcp_adapter::furniture_batch::{BatchDefinition, FurniturePlan};
use dfmcp_adapter::furniture_handoff::{FurnitureRequest, Handoff};
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
mod allocation;
mod batch;
mod completion;
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
// One process-wide session entry; variant size is irrelevant.
#[allow(clippy::large_enum_variant)]
enum Entry {
    Placement {
        state: State<PrivateBuildFile, BuildRpc>,
        config: Config,
    },
    Completion {
        state: completion::Recovery,
        config: Config,
    },
}
impl Entry {
    fn id(&self) -> SessionId {
        match self {
            Self::Placement { state, .. } => state.id,
            Self::Completion { state, .. } => state.id,
        }
    }
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
    head: Digest32,
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
    batch: Option<batch::Parent>,
    completion: Option<completion::Monitor>,
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
            batch: None,
            completion: None,
        })
    }
    fn context(&mut self, write: bool, wall: Option<u64>) -> Result<OperationContext> {
        if let Some(monitor) = &mut self.completion {
            monitor.verified = false;
            monitor.store_verified = false;
        }
        self.request = self.request.checked_add(1).ok_or_else(exhausted)?;
        let mut budget = self.budget;
        if let Some(w) = wall {
            budget.max_wall_millis = budget.max_wall_millis.min(w);
        }
        let mut c = context(
            self.id,
            RequestId::new(self.request),
            self.binding.fortress().fortress_id(),
            self.control.high_tick().max(
                self.completion
                    .as_ref()
                    .and_then(|m| m.store.progress().last_tick)
                    .unwrap_or(0),
            ),
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
    fn attach_batch(&self, result: &mut Value, view: Option<&BuildInventory>, verified: bool) {
        if let Some(parent) = &self.batch {
            result["batch"] = batch::display(parent, view, verified);
        }
        if let Some(monitor) = &self.completion {
            result["completion"] = completion::display(monitor, verified && monitor.verified);
        }
    }
    fn verify_batch(&mut self, c: &OperationContext, view: &BuildInventory) -> Result<()> {
        if let Some(parent) = &mut self.batch {
            batch::verify(parent, view, c)?;
        }
        Ok(())
    }
    fn disclosure_context(&self, c: &OperationContext) -> Result<OperationContext> {
        let mut current = c.clone();
        current.anchor.tick = GameTick(
            current.anchor.tick.get().max(self.control.high_tick()).max(
                self.completion
                    .as_ref()
                    .and_then(|m| m.store.progress().last_tick)
                    .unwrap_or(0),
            ),
        );
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
        self.attach_batch(&mut result, self.historical.as_ref(), false);
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
        let mut result = json!({"ok":true,"scope":"session","closed":true,"release_for_recovery":release,
            "effects_cancelled":false,"history_erased":false,"native_quiescence_proven":false});
        match self.disclosure_context(c) {
            Ok(current) => {
                self.attach_batch(&mut result, view.or(self.historical.as_ref()), false);
                packet(
                    "fortress.cancel",
                    result,
                    Some(&current),
                    Some(&self.binding),
                    view,
                    self.historical.as_ref(),
                    None,
                    None,
                    None,
                )
            }
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
    Batch {},
    Allocation {
        #[serde(default)]
        view: allocation::View,
    },
    Completion {},
    CompletionStart {
        deadline: u64,
        interval: Option<u32>,
        stable_samples: Option<u32>,
        stable_span: Option<u64>,
        max_gap: Option<u32>,
        max_observations: Option<u32>,
    },
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
            Self::Schema {}
            | Self::Batch {}
            | Self::Allocation { .. }
            | Self::Completion {}
            | Self::CompletionStart { .. } => {}
        }
        Ok(value)
    }
}
enum Action {
    Observe(BuildSelection),
    ObserveNext,
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
    StopBatch,
    Completion(completion::Action),
    Denied,
}
fn schema() -> Value {
    json!({"schema":"dfmcp.build-placement-mcp-query/1","max_bytes":2048,"closed":true,
    "modes":{"records":{"limit":"optional integer 1..8","offset":"optional integer 0..256; head required when nonzero","head":"optional lowercase SHA-256 exact journal head"},
        "get":{"idempotency_key":"1..128 ASCII letters/digits/dot/underscore/hyphen","plan_digest":"lowercase SHA-256"},"selection":{"witness":"lowercase SHA-256"},"batch":{},"completion":{},
        "allocation":{"view":"optional request (default) or items; retained original evidence only"},
        "completion_start":{"deadline":"required absolute game tick","interval":"optional 1..403200; default 1","stable_samples":"optional 2..64; default 2","stable_span":"optional 1..4032000; default 1","max_gap":"optional interval..4032000; default 1200","max_observations":"optional stable_samples..512; default 512"},"schema":{}},
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
        Action::Observe(_) | Action::ObserveNext => {
            state.abandon();
            if state.batch.is_some() {
                state.verify_batch(&work.take(c, batch::GUARD_BYTES)?, before)?;
            }
            let next = state
                .batch
                .as_ref()
                .map(|p| batch::next(p, before))
                .transpose()?;
            let selection = match action {
                Action::Observe(selection) => {
                    if next
                        .as_ref()
                        .is_some_and(|(_, expected)| *expected != selection)
                    {
                        return Err(error(
                            ErrorCode::Conflict,
                            "selection differs from the original next batch step",
                        ));
                    }
                    selection
                }
                _ => next
                    .as_ref()
                    .map(|(_, selection)| *selection)
                    .ok_or_else(|| {
                        error(
                            ErrorCode::InvalidRequest,
                            "next requires an original furniture batch",
                        )
                    })?,
            };
            let guard_bytes = if state.batch.is_some() {
                work.take(c, batch::GUARD_BYTES)?.budget.max_bytes
            } else {
                0
            };
            let mut custody = batch::Guard {
                parent: &mut state.batch,
                view: before,
                key: next.as_ref().map(|(key, _)| key.as_str()),
                runtime,
                bytes: guard_bytes,
            };
            let mut guard = policy::Guard {
                policy: &state.policy,
                leases: &state.leases,
                runtime: &mut custody,
                confirmation: None,
            };
            let capture = state.control.observe(
                selection,
                &work.take(c, EFFECT_BYTES)?,
                |b, c, d| connect(selection, false, b, c, d),
                &mut guard,
            )?;
            if let (Some(parent), Some((key, _))) = (&state.batch, next) {
                parent.definition().validate_next(
                    before,
                    parent.stopped(),
                    &key,
                    selection,
                    Some(&capture),
                )?;
            }
            Ok(
                json!({"ok":true,"observation":presentation::observation(&capture),"game_mutation_dispatched":false,
                    "source_summary":state.control.native_summary().map(presentation::source_summary)}),
            )
        }
        Action::Plan { key, witness } => {
            if state.batch.is_some() {
                state.verify_batch(&work.take(c, batch::GUARD_BYTES)?, before)?;
            }
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
                if let Some(parent) = &state.batch {
                    parent.definition().validate_next(
                        before,
                        parent.stopped(),
                        &key,
                        old.plan().before().selection(),
                        Some(old.plan().before()),
                    )?;
                    if review.head != before.head {
                        return Err(error(
                            ErrorCode::StaleAnchor,
                            "reviewed batch journal head changed",
                        ));
                    }
                }
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
            if let Some(parent) = &state.batch {
                parent.definition().validate_next(
                    before,
                    parent.stopped(),
                    &key,
                    plan.before().selection(),
                    Some(plan.before()),
                )?;
            }
            state.pending_hint = Some(
                json!({"idempotency_key":key,"plan_digest":plan.digest().to_string(),"outcome":"unverified","retry_commit_permitted":false}),
            );
            let guard_bytes = if state.batch.is_some() {
                work.take(c, batch::GUARD_BYTES)?.budget.max_bytes
            } else {
                0
            };
            let mut custody = batch::Guard {
                parent: &mut state.batch,
                view: before,
                key: Some(&key),
                runtime,
                bytes: guard_bytes,
            };
            let mut guard = policy::Guard {
                policy: &state.policy,
                leases: &state.leases,
                runtime: &mut custody,
                confirmation: None,
            };
            let entry =
                state
                    .control
                    .prepare(&key, witness, &work.take(c, EFFECT_BYTES)?, &mut guard)?;
            if state.control.has_preparation_connection() {
                let head = if state.batch.is_some() {
                    let view = state.control.inventory(&work.take(c, LOCAL_BYTES)?)?;
                    state.verify_batch(&work.take(c, batch::GUARD_BYTES)?, &view)?;
                    view.head
                } else {
                    before.head
                };
                let policy_seal = state.policy.seal(entry.plan());
                let seal = state.batch.as_ref().map_or(policy_seal, |p| {
                    batch::seal(p, entry.plan(), head, policy_seal)
                });
                state.review = Some(Review {
                    key,
                    plan: entry.plan().digest(),
                    witness,
                    seal,
                    head,
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
            if state.batch.is_some() {
                state.verify_batch(&work.take(c, batch::GUARD_BYTES)?, before)?;
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
            let policy_seal = state.policy.seal(old.plan());
            if let Some(parent) = &state.batch {
                if review.head != before.head
                    || review.seal != batch::seal(parent, old.plan(), before.head, policy_seal)
                {
                    return Err(error(
                        ErrorCode::StaleAnchor,
                        "complete batch or prepared journal review changed",
                    ));
                }
                parent.definition().validate_next(
                    before,
                    parent.stopped(),
                    &key,
                    old.plan().before().selection(),
                    Some(old.plan().before()),
                )?;
            }
            let guard_bytes = if state.batch.is_some() {
                work.take(c, batch::GUARD_BYTES)?.budget.max_bytes
            } else {
                0
            };
            let mut custody = batch::Guard {
                parent: &mut state.batch,
                view: before,
                key: Some(&key),
                runtime,
                bytes: guard_bytes,
            };
            let mut guard = policy::Guard {
                policy: &state.policy,
                leases: &state.leases,
                runtime: &mut custody,
                confirmation: Some(policy_seal),
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
            if let Some(head) = head
                && digest(&head)? != before.head
            {
                return Err(error(
                    ErrorCode::StaleAnchor,
                    "furniture journal continuation head changed",
                ));
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
        Action::Query(Query::Batch {}) => {
            if state.batch.is_none() {
                return Err(error(
                    ErrorCode::InvalidRequest,
                    "no furniture batch is configured",
                ));
            }
            state.verify_batch(&work.take(c, batch::GUARD_BYTES)?, before)?;
            Ok(json!({"ok":true,"native_calls":0}))
        }
        Action::Query(Query::Allocation { view }) => {
            state.verify_batch(&work.take(c, batch::GUARD_BYTES)?, before)?;
            let handoff = state
                .batch
                .as_ref()
                .and_then(|p| p.definition().handoff())
                .ok_or_else(|| {
                    error(
                        ErrorCode::InvalidRequest,
                        "this batch has no allocation origin",
                    )
                })?;
            Ok(json!({
                "ok":true,
                "allocation":allocation::display(handoff, view)?,
                "native_calls":0
            }))
        }
        Action::StopBatch => {
            state.abandon();
            let parent = state.batch.as_mut().ok_or_else(|| {
                error(
                    ErrorCode::InvalidRequest,
                    "no furniture batch is configured",
                )
            })?;
            batch::verify(parent, before, &work.take(c, batch::GUARD_BYTES)?)?;
            parent.stop(&work.take(c, batch::GUARD_BYTES)?)?;
            Ok(
                json!({"ok":true,"scope":"batch","stopped":true,"native_calls":0,
                "effects_cancelled":false,"building_undone":false,"original_key_recovery_preserved":true}),
            )
        }
        Action::Inventory => Ok(
            json!({"ok":true,"journal":inventory(before),"native_calls":0,"live_game_health_checked":false}),
        ),
        Action::Completion(_)
        | Action::Query(Query::Completion {} | Query::CompletionStart { .. }) => Err(error(
            ErrorCode::InvalidRequest,
            "completion requires the original private batch session",
        )),
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
    let batch_verified = if state.batch.is_some() {
        match verified
            .as_ref()
            .ok_or_else(|| error(ErrorCode::CorruptLedger, "child inventory unavailable"))
            .and_then(|v| state.verify_batch(&work.view(&display)?, v))
        {
            Ok(()) => true,
            Err(e) => {
                state.abandon();
                result["batch_inventory_unverified"] = json!(true);
                if matches!(
                    op,
                    "fortress.observe" | "fortress.plan" | "fortress.commit" | "fortress.doctor"
                ) {
                    result["ok"] = json!(false);
                    result["error"] = failure(&e)["error"].clone();
                    result["review_seal"] = Value::Null;
                    result["retry_commit_permitted"] = json!(false);
                    result["effect_may_have_occurred"] = json!(op == "fortress.commit");
                }
                false
            }
        }
    } else {
        false
    };
    if let Some(allocation) = result.get_mut("allocation") {
        allocation["inventory_verified"] = json!(batch_verified);
    }
    if state.completion.is_some() {
        let checked = verified
            .as_ref()
            .ok_or_else(|| error(ErrorCode::CorruptLedger, "original inventory unavailable"))
            .and_then(|view| completion::verify(state, &display, &mut work, view, batch_verified));
        if let Err(e) = checked {
            result["completion_inventory_unverified"] = json!(true);
            if op == "fortress.doctor" {
                result["ok"] = json!(false);
                result["error"] = failure(&e)["error"].clone();
            }
        }
    }
    state.attach_batch(
        &mut result,
        verified.as_ref().or(state.historical.as_ref()),
        batch_verified,
    );
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
    let monitor_action = matches!(&action, Ok(Action::Completion(_)));
    let read_only = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(monitor_action));
    let worker_read_only = read_only.clone();
    let output = runtime::owned(op, move |control| {
        let result = (|| {
            let id = session_id(&raw)?;
            let mut locked = lock()?;
            let entry = locked
                .as_mut()
                .filter(|e| e.id() == id)
                .ok_or_else(|| error(ErrorCode::SessionNotFound, "furniture session absent"))?;
            let (state, config) = match entry {
                Entry::Completion { state, config } => {
                    worker_read_only.store(true, Ordering::Release);
                    return Ok(completion::run_recovery(
                        state, config, &control, op, action, wall,
                    ));
                }
                Entry::Placement { state, config } => (state, config),
            };
            let c = match state.context(runtime::enabled().unwrap_or(false), wall) {
                Ok(c) => c,
                Err(e) => return Ok(state.failed(op, None, &e)),
            };
            if let Err(e) = runtime::boundary(&control, config, false) {
                state.abandon();
                return Ok(state.failed(op, Some(&c), &e));
            }
            let config = config.clone();
            let action = match action {
                Ok(Action::Completion(action)) => {
                    return Ok(completion::run(state, &config, &control, c, op, action));
                }
                other => other,
            };
            let mut guard = runtime::Guard {
                control: &control,
                config: &config,
            };
            let rendered = run_action(
                state,
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
                state.abandon();
                return Ok(state.failed(op, Some(&c), &e));
            }
            Ok(rendered)
        })();
        match result {
            Ok(v) => v,
            Err(e) => unbound(op, &e),
        }
    })
    .await;
    if read_only.load(Ordering::Acquire) {
        completion::read_only_packet(output)
    } else {
        output
    }
}

#[tool(
    name = "fortress.open_session",
    description = "Open isolated furniture/1.19 custody. Use exactly one of selection JSON [kind,item_id,x,y,z], furniture_plan as complete dfmcp.furniture-plan/1 JSON, or furniture_request as dfmcp.furniture-request/1 JSON. A request allocates all slots from one fresh operations/1.4 capture and seals its constraints and chosen items into operator-configured batch custody; shortage creates no partial batch. Reopen retained custody without any intent input or native bootstrap. No client paths or policies. Opening never prepares or places furniture."
)]
pub async fn fortress_open_session(
    selection: Option<String>,
    max_wall_millis: Option<u64>,
    max_bytes: Option<u64>,
    max_output_tokens: Option<u32>,
    furniture_plan: Option<String>,
    furniture_request: Option<String>,
) -> String {
    runtime::owned("fortress.open_session", move |control| {
        let result = (|| {
            let config = runtime::configuration()?;
            runtime::boundary(&control, &config, false)?;
            // Parse and bound complete intent before native contact or storage.
            if usize::from(selection.is_some()) + usize::from(furniture_plan.is_some()) + usize::from(furniture_request.is_some()) > 1 {
                return Err(error(ErrorCode::InvalidRequest, "furniture intent inputs are mutually exclusive"));
            }
            let mut plan = furniture_plan.as_deref().map(|s|FurniturePlan::decode(s.as_bytes())).transpose()?;
            let requested = furniture_request.as_deref().map(|s|FurnitureRequest::decode(s.as_bytes())).transpose()?;
            let importing = plan.is_some() || requested.is_some();
            if let Some(requested) = &requested {
                if config.batch_path.is_none() || config.mode != BuildMode::Control || config.completion_only {
                    return Err(error(ErrorCode::InvalidRequest, "furniture request requires new Control batch custody"));
                }
                allocation::validate_request(requested, &config)?;
            }
            let mut selected = if let Some(plan) = &plan {
                if config.batch_path.is_none() || config.mode != BuildMode::Control || selection.is_some() {
                    return Err(error(ErrorCode::InvalidRequest, "complete plan requires configured Control batch custody and no selection"));
                }
                batch::reserve_output(plan)?;
                for step in plan.steps() {
                    let halo = config.selection(step.selection)?;
                    if config.protected.iter().any(|region|dfmcp_core::cuboids_intersect(region,&halo)) {
                        return Err(error(ErrorCode::CapabilityDenied, "batch target intersects protected region"));
                    }
                }
                plan.ordered_steps().next().map(|step|step.selection)
            } else if config.batch_path.is_some() {
                if selection.is_some() {
                    return Err(error(ErrorCode::InvalidRequest, "reopening a batch derives selections from its retained plan"));
                }
                None
            } else {
                let selected = selection.as_deref().map(parse_selection).transpose()?;
                if (config.mode == BuildMode::Control) != selected.is_some() {
                    return Err(error(ErrorCode::InvalidRequest, "single control requires selection; recovery/offline must omit it"));
                }
                selected
            };
            let budget = WorkBudget { max_wall_millis:max_wall_millis.unwrap_or(10000),
                max_bytes:max_bytes.unwrap_or(MAX_WORK_BYTES),max_output_tokens:max_output_tokens.unwrap_or(16384),
                max_entities:73729,max_actions:1,max_game_ticks:0 };
            let mut locked = lock()?;
            if locked.is_some() { return Err(error(ErrorCode::Conflict, "release the current furniture session first")); }
            let seq = NEXT.try_update(Ordering::AcqRel,Ordering::Acquire,|v|(v<(1u64<<57)).then_some(v+1)).map_err(|_|exhausted())?;
            let id = SessionId::new((1u128<<127)|FAMILY|u128::from(seq));
            let mut c = context(id,RequestId::new(1),config.fortress.fortress_id(),0,budget,config.mode,runtime::enabled()?);
            let mut work = Work::new(&c,control.started)?;
            if config.completion_only {
                if selection.is_some() || plan.is_some() || requested.is_some() {
                    return Err(error(ErrorCode::InvalidRequest, "completion recovery reopens only its retained original monitor"));
                }
                let (state, output) = completion::open_recovery(&config, &control, &c, &mut work)?;
                *locked = Some(Entry::Completion { state, config });
                return Ok(output);
            }
            if importing {
                c.authorize(Capability::Plan, RiskTier::Guarded, &[], None)?;
                c.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
                // Import only creates a missing parent. Reopening derives all
                // intent from retained custody, so a substituted client plan
                // must never select even a bootstrap native observation.
                if let Some(path) = &config.batch_path {
                    match std::fs::symlink_metadata(path) {
                        Ok(_) => return Err(error(ErrorCode::Conflict,
                            "existing batch custody must be reopened without a supplied plan")),
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {},
                        Err(_) => return Err(error(ErrorCode::CorruptLedger,
                            "batch parent existence cannot be established")),
                    }
                    work.current(&c)?;
                }
            }
            let mut handoff = None;
            if let Some(requested) = &requested {
                let observed = runtime::furniture_supply(&config, &work.take(&c, 20 * 1024 * 1024)?)?;
                runtime::boundary(&control, &config, false)?;
                let anchor = observed.snapshot().ok_or_else(|| error(ErrorCode::AdapterRejected,
                    "furniture supply lacks a complete operations snapshot"))?.anchor();
                c.anchor.tick = GameTick(c.anchor.tick.get().max(anchor.tick.get()));
                c.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
                c.authorize(Capability::Plan, RiskTier::Guarded, &[], None)?;
                let mut analysis_context = work.take(&c, 256 * 1024 * 1024)?;
                analysis_context.anchor = anchor;
                // Native I/O has finished, but CPU-bound allocation still belongs
                // to this joined request. Keep cancellation, inherited runtime
                // restrictions and the operator's Plan opt-in live throughout it.
                let outcome = Handoff::allocate_with_check(&observed, &analysis_context, config.endpoint,
                    requested, dfmcp_adapter::furniture_supply::MAX_WORK,
                    &mut || runtime::boundary(&control, &config, true))?;
                work.current(&c)?;
                runtime::boundary(&control, &config, false)?;
                if outcome.handoff.is_none() {
                    let result = json!({"ok":true,"status":"furniture_shortage","session_opened":false,
                        "session_id":null,"allocation":allocation::shortage(&outcome, requested)?,
                        "native_preparation_dispatched":false,"game_mutation_dispatched":false,
                        "batch_created":false,"retry_commit_permitted":false});
                    let output = packet("fortress.open_session",result,Some(&c),None,None,None,None,None,None);
                    if output.len() as u64 > OUTPUT_BYTES { return Err(exhausted()); }
                    work.current(&c)?.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
                    runtime::boundary(&control, &config, false)?;
                    return Ok(output);
                }
                let allocated = outcome.handoff.ok_or_else(|| error(ErrorCode::InternalInvariantViolation,
                    "complete furniture allocation lost its handoff"))?;
                batch::reserve_output(allocated.plan())?;
                selected = allocated.plan().ordered_steps().next().map(|step|step.selection);
                plan = Some(allocated.plan().clone());
                handoff = Some(allocated);
            }
            let binding = if let Some(selection) = selected {
                config.selection(selection)?;
                let child = work.take(&c,SOURCE_BYTES)?;
                let source = runtime::connect(&config,selection,None,&child,Duration::from_millis(child.budget.max_wall_millis))?;
                let capture = source.initial_capture().ok_or_else(||error(ErrorCode::AdapterRejected,"furniture bootstrap capture missing"))?;
                if capture.fortress()!=&config.fortress || capture.selection()!=selection {
                    return Err(error(ErrorCode::StaleAnchor,"furniture bootstrap source differs from configuration"));
                }
                if let Some(handoff) = &handoff {
                    handoff.validate_capture(source.binding(), capture)?;
                }
                c.anchor.tick = GameTick(capture.tick());
                Some(source.binding().clone())
            } else { None };
            runtime::boundary(&control,&config,importing)?;
            let source_reserve = if config.completion_path.is_some() || handoff.is_some() {
                completion::open_reserve(&config.path, 16 * 1024 * 1024)?
            } else { EFFECT_BYTES };
            let mut journal = open_private_build(&config.path,&work.take(&c,source_reserve)?,config.mode,binding)?;
            config.matches(journal.binding())?;
            c.anchor.tick = GameTick(c.anchor.tick.get().max(journal.high_tick()));
            let mut retained_parent = None;
            if let Some(path) = &config.batch_path {
                let view = journal.inventory(&work.take(&c,LOCAL_BYTES)?)?;
                let expected = if let Some(plan) = plan {
                    if !view.entries().is_empty() {
                        return Err(error(ErrorCode::Conflict,"new batch requires an empty original journal; reopen retained work without a plan"));
                    }
                    Some(if let Some(handoff) = handoff {
                        BatchDefinition::from_handoff(handoff,journal.binding().clone(),view.journal_id)?
                    } else {
                        BatchDefinition::new(plan,journal.binding().clone(),view.journal_id)?
                    })
                } else { None };
                runtime::boundary(&control,&config,importing)?;
                let mut parent = open_private_batch(path,&work.take(&c,LOCAL_BYTES)?,config.mode,expected)?;
                batch::matches(parent.definition(),journal.binding())?;
                batch::reserve_output(parent.definition().plan())?;
                batch::verify(&mut parent,&view,&work.take(&c,batch::GUARD_BYTES)?)?;
                // An allocated batch may restart before its first child exists.
                // Its retained source tick must initialize the new session and
                // host lease, rather than creating an already-expired tick-zero
                // lease or requiring another inventory capture/reallocation.
                if let Some(handoff) = parent.definition().handoff() {
                    c.anchor.tick = GameTick(c.anchor.tick.get().max(handoff.source().anchor.tick.get()));
                    c.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
                }
                retained_parent = Some(parent);
            }
            let session = BuildSession::<_,BuildRpc>::new(journal,&work.view(&c)?)?;
            let mut state = State::new(session,&work.take(&c,LOCAL_BYTES)?,&config)?;
            state.budget = budget;
            state.grants = c.grants.clone();
            state.batch = retained_parent;
            c = state.disclosure_context(&c)?;
            let view = state.control.inventory(&work.view(&c)?)?;
            state.verify_batch(&work.take(&c,batch::GUARD_BYTES)?,&view)?;
            completion::reopen(&mut state, &config, &c, &mut work, &view)?;
            let mut result = json!({"ok":true,"session_opened":true,"session_id":id.to_string(),"mode":mode_name(config.mode),"journal":inventory(&view),
                "capabilities":c.grants.iter().map(|g|g.capability.as_str()).collect::<Vec<_>>(),"planning_observation_retained":false,
                "native_preparation_dispatched":false,"game_mutation_dispatched":false});
            state.attach_batch(&mut result,Some(&view),true);
            let output = packet("fortress.open_session",result,Some(&c),Some(&state.binding),Some(&view),None,Some(&state.policy),None,None);
            if output.len() as u64>OUTPUT_BYTES { return Err(exhausted()); }
            work.current(&c)?;
            runtime::boundary(&control,&config,importing)?;
            *locked = Some(Entry::Placement{state,config});
            Ok(output)
        })();
        match result { Ok(v)=>v,Err(e)=>unbound("fortress.open_session",&e) }
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
    description = "Capture an exact furniture selection, next for the original batch step, or completion for one query-only whole-plan construction sample. Completion brackets all original receipts around one complete operations capture and durably records progress. Observation abandons local placement permission, never pending work."
)]
pub async fn fortress_observe(session_id: String, selection: String) -> String {
    with_session(
        session_id,
        "fortress.observe",
        None,
        if selection == "completion" {
            Ok(Action::Completion(completion::Action::Sample))
        } else if selection == "next" {
            Ok(Action::ObserveNext)
        } else {
            parse_selection(&selection).map(Action::Observe)
        },
    )
    .await
}
#[tool(
    name = "fortress.plan",
    description = "Prepare one furniture placement from this session's exact observation witness under current scope, lease, protected regions and checkpoint policy. A batch requires its returned original step key; review also binds the complete parent plan and prepared journal head. Returns native plan digest and review seal. Preparation inserts no building."
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
    description = "Inspect local evidence with closed JSON modes batch/records/get/selection/schema/completion/allocation. allocation has view request (default) or items and returns original sealed constraints or selected-item evidence without reallocation. completion_start creates a fixed-deadline monitor from every original Placed receipt in a completed batch under Query authority. completion inspects retained progress and original custody. No native calls or renewed placement permission."
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
            Query::Completion {} => Ok(Action::Completion(completion::Action::Inspect)),
            Query::CompletionStart {
                deadline,
                interval,
                stable_samples,
                stable_span,
                max_gap,
                max_observations,
            } => Ok(Action::Completion(completion::Action::Start(
                dfmcp_adapter::construction_plan::Timing {
                    deadline,
                    interval: interval.unwrap_or(1),
                    stable_samples: stable_samples.unwrap_or(2),
                    stable_span: stable_span.unwrap_or(1),
                    max_gap: max_gap.unwrap_or(1200),
                    max_observations: max_observations.unwrap_or(512),
                },
            ))),
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
    let output = runtime::owned("fortress.cancel", move |control| {
        let result = (|| {
            control.checkpoint()?;
            let id = session_id(&raw)?;
            let mut locked = lock()?;
            let entry = locked
                .as_mut()
                .filter(|e| e.id() == id)
                .ok_or_else(|| error(ErrorCode::SessionNotFound, "furniture session absent"))?;
            let (state, config) = match entry {
                Entry::Completion { state, config } => {
                    let output = completion::close_recovery(state, config, &control, release)?;
                    drop(locked.take());
                    return Ok(output);
                }
                Entry::Placement { state, config } => (state, config),
            };
            let c = state.context(false, None)?;
            let view = if release {
                None
            } else {
                if let Err(e) = runtime::boundary(&control, config, false) {
                    return Ok(state.failed("fortress.cancel", Some(&c), &e));
                }
                let work = Work::new(&c, control.started)?;
                Some(state.control.inventory(&work.view(&c)?)?)
            };
            if !release && view.as_ref().is_some_and(|v| v.pending().is_some()) {
                return Ok(state.failed(
                    "fortress.cancel",
                    Some(&c),
                    &error(
                        ErrorCode::EffectIndeterminate,
                        "unresolved work requires explicit recovery release",
                    ),
                ));
            }
            let output = state.close_packet(&c, view.as_ref(), release);
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
    .await;
    completion::read_only_packet(output)
}
#[tool(
    name = "fortress.cancel",
    description = "scope=completion cancels only the local completion monitor, preserving placement effects and evidence. scope=batch stops new batch steps under Query. scope=effect retires an exact prepared native record; cannot undo construction. scope=session releases custody; release_for_recovery preserves unresolved identities."
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
        ("batch", None, None) if !release => {
            with_session(session_id, "fortress.cancel", None, Ok(Action::StopBatch)).await
        }
        ("completion", None, None) if !release => {
            with_session(
                session_id,
                "fortress.cancel",
                None,
                Ok(Action::Completion(completion::Action::Cancel)),
            )
            .await
        }
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
        .instructions("Unadmitted furniture/1.19 development control. With operator batch custody, use open_session furniture_request (dfmcp.furniture-request/1 JSON) to allocate all requested slots from one fresh operations/1.4 capture, or import an exact furniture_plan (dfmcp.furniture-plan/1 JSON). A shortage creates no session or partial batch. Inspect query mode batch and observe selection next. Query mode allocation with view request or items inspects the immutable original constraints and selection. Review and commit exactly one returned original key at a time; only retained terminal Placed prefixes unlock more steps. Reopening a batch omits all intent inputs, performs no reallocation, and restores no permit. scope=batch cancellation permanently stops advancement while preserving original-key recovery. Single-selection control remains available without batch configuration. Default policy requires an unavailable game checkpoint; only explicit operator disposable-fortress policy permits placement. Query the original key after uncertainty; never retry commit. Placed proves historical stage-zero registration. Once every original step is Placed, completion_start fixes a whole-plan goal, observe selection completion acquires one sample, and query completion or local cancel inspects or stops the monitor. No canonical world anchor or global controller fence. No arbitrary command, clock advancement, checkpoint or restore.").build();
    crate::run_modern_stdio(server);
}
#[cfg(test)]
mod tests;

#[cfg(test)]
mod allocation_owner_tests;
