#![forbid(unsafe_code)]
//! Isolated run/1.13 development MCP. Durable coordination, never production admission.
use dfmcp_adapter::bounded_run::{
    RunObservation, RunPlan, RunRecord, RunSpec,
    journal::{
        DurableRun, MAX_JOURNAL_BYTES, MAX_RUNS, RPC_RESERVE_BYTES, RunBinding, RunJournal,
        RunMode, RunState,
    },
    private_file::{PrivateRunFile, open_private_run_journal},
    rpc::{RunManifest, RunRpcClient, RunSource, RunTcpStream},
    validate_key,
};
use dfmcp_adapter::control_effect_journal::EffectJournalStorage;
use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, Digest32, ErrorCode, FortressId, GameTick,
    ObservationCursor, OperationContext, RequestId, Result, RiskTier, SessionId, StateAnchor,
    WorkBudget,
};
use fastmcp_rust::modern::ServerBuilder;
use fastmcp_rust::prelude::*;
use serde_json::{Value, json};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, TryLockError};
use std::time::Instant;
mod presentation;
use presentation::{
    BASE_RESERVE, Cursors, Filter, MAX_PAGE, PageKey, ROW_RESERVE, View, digest, error, failure,
    packet,
};

const MAX_BYTES: u64 = 16 * 1024 * 1024;
const LIST_RESERVE: u64 = MAX_JOURNAL_BYTES as u64 + MAX_RUNS as u64 * 512 + 1024;
const CONNECT_RESERVE: u64 = 7 * RPC_RESERVE_BYTES + 24;
const FAMILY: u128 = 13u128 << 57;
static NEXT: AtomicU64 = AtomicU64::new(1);
type LiveSession = RunSession<PrivateRunFile, NativeFactory>;
static SESSION: Mutex<Option<LiveSession>> = Mutex::new(None);

fn runtime_io() -> Result<()> {
    let cx = asupersync::Cx::current().ok_or_else(|| {
        error(
            ErrorCode::CapabilityDenied,
            "run MCP I/O requires its owned runtime context",
        )
    })?;
    cx.checkpoint().map_err(|_| {
        error(
            ErrorCode::CancellationRequested,
            "run foreground request cancelled",
        )
    })?;
    if cx.io().is_none() {
        return Err(error(
            ErrorCode::CapabilityDenied,
            "inherited context denies run MCP I/O",
        ));
    }
    Ok(())
}
fn clock_enabled() -> bool {
    std::env::var("DFMCP_RUN_ALLOW_CLOCK").ok().as_deref() == Some("1")
}
fn native_guard(control: bool) -> Result<()> {
    runtime_io()?;
    validate_environment()?;
    if control && !clock_enabled() {
        return Err(error(
            ErrorCode::CapabilityDenied,
            "operator clock authority is disabled",
        ));
    }
    Ok(())
}
const ALLOWED: [&str; 5] = [
    "DFMCP_ALLOW_UNADMITTED_RUN_V1_13",
    "DFMCP_RUN_ALLOW_CLOCK",
    "DFMCP_RUN_ENDPOINT",
    "DFMCP_RUN_TOKEN",
    "DFMCP_RUN_JOURNAL",
];
fn environment_contract(
    opt_in: Option<&str>,
    clock: Option<&str>,
    keys: &[String],
    admitted: bool,
) -> Result<()> {
    if opt_in != Some("1")
        || admitted
        || clock.is_some_and(|v| v != "1")
        || keys
            .iter()
            .any(|k| k.starts_with("DFMCP_") && !ALLOWED.contains(&k.as_str()))
    {
        return Err(error(
            ErrorCode::CapabilityDenied,
            "run/1.13 requires exact development opt-in and refuses production or other DFMCP environment state",
        ));
    }
    Ok(())
}
fn validate_environment() -> Result<()> {
    let clock = std::env::var("DFMCP_RUN_ALLOW_CLOCK");
    if matches!(clock, Err(std::env::VarError::NotUnicode(_))) {
        return Err(error(
            ErrorCode::CapabilityDenied,
            "clock opt-in is not UTF-8",
        ));
    }
    environment_contract(
        std::env::var("DFMCP_ALLOW_UNADMITTED_RUN_V1_13")
            .ok()
            .as_deref(),
        clock.ok().as_deref(),
        &std::env::vars_os()
            .map(|(k, _)| k.to_string_lossy().into_owned())
            .collect::<Vec<_>>(),
        crate::admission::current_admission_provenance().is_some(),
    )
}
fn configured(name: &str, maximum: usize) -> Result<String> {
    let text = std::env::var(name).map_err(|_| {
        error(
            ErrorCode::CapabilityDenied,
            "required operator run configuration missing or non-UTF-8",
        )
    })?;
    if text.is_empty() || text.len() > maximum || text.contains('\0') {
        return Err(error(
            ErrorCode::InvalidRequest,
            "operator run configuration exceeds its bound",
        ));
    }
    Ok(text)
}
fn parse_mode(raw: &str) -> Result<RunMode> {
    match raw {
        "offline" => Ok(RunMode::Offline),
        "recover" => Ok(RunMode::Recover),
        "control" => Ok(RunMode::Control),
        _ => Err(error(
            ErrorCode::InvalidRequest,
            "mode must be offline, recover or control",
        )),
    }
}
fn grants(mode: RunMode, enabled: bool) -> Result<Vec<CapabilityGrant>> {
    if mode == RunMode::Control && !enabled {
        return Err(error(
            ErrorCode::CapabilityDenied,
            "control requires operator DFMCP_RUN_ALLOW_CLOCK=1",
        ));
    }
    let capabilities = if mode == RunMode::Control {
        vec![
            Capability::Query,
            Capability::Plan,
            Capability::ControlClock,
        ]
    } else {
        vec![Capability::Query]
    };
    Ok(capabilities
        .into_iter()
        .map(|capability| CapabilityGrant {
            capability,
            scope: CapabilityScope {
                fortress_id: Some(FortressId::NIL),
                ..CapabilityScope::default()
            },
            max_risk: if capability == Capability::Query {
                RiskTier::ReadOnly
            } else {
                RiskTier::Guarded
            },
            expires_at_tick: None,
            remaining_uses: None,
        })
        .collect())
}
fn remaining(
    mut context: OperationContext,
    started: Instant,
    total: u64,
) -> Result<OperationContext> {
    let elapsed = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    context.budget.max_wall_millis = total
        .checked_sub(elapsed)
        .filter(|n| *n > 0)
        .map(|n| n.min(context.budget.max_wall_millis))
        .ok_or_else(|| {
            error(
                ErrorCode::BudgetExceeded,
                "run foreground deadline exhausted",
            )
        })?;
    Ok(context)
}
trait SourceFactory {
    type Source: RunSource;
    fn connect(&mut self, context: &OperationContext) -> Result<Self::Source>;
}
struct NativeFactory {
    endpoint: SocketAddr,
    token: Vec<u8>,
}
struct GuardedSource(RunRpcClient<RunTcpStream>);
impl SourceFactory for NativeFactory {
    type Source = GuardedSource;
    fn connect(&mut self, c: &OperationContext) -> Result<GuardedSource> {
        native_guard(false)?;
        let mut nonce = c.session_id.get().to_be_bytes().to_vec();
        nonce.extend_from_slice(&c.request_id.get().to_be_bytes());
        Ok(GuardedSource(RunRpcClient::connect(
            self.endpoint,
            self.token.clone(),
            nonce,
            c,
        )?))
    }
}
impl RunSource for GuardedSource {
    fn manifest(&self) -> &RunManifest {
        self.0.manifest()
    }
    fn endpoint(&self) -> Option<SocketAddr> {
        self.0.endpoint()
    }
    fn observe(&mut self, c: &OperationContext) -> Result<RunObservation> {
        native_guard(false)?;
        self.0.restrict_deadline(c)?;
        self.0.observe(c)
    }
    fn prepare(&mut self, p: &RunPlan, c: &OperationContext) -> Result<RunRecord> {
        native_guard(true)?;
        self.0.restrict_deadline(c)?;
        self.0.prepare(p, c)
    }
    fn commit(&mut self, p: &RunPlan, c: &OperationContext) -> Result<RunRecord> {
        native_guard(true)?;
        self.0.restrict_deadline(c)?;
        self.0.commit(p, c)
    }
    fn query(&mut self, p: &RunPlan, c: &OperationContext) -> Result<Option<RunRecord>> {
        native_guard(false)?;
        self.0.restrict_deadline(c)?;
        self.0.query(p, c)
    }
    fn cancel(&mut self, p: &RunPlan, c: &OperationContext) -> Result<RunRecord> {
        native_guard(true)?;
        self.0.restrict_deadline(c)?;
        self.0.cancel(p, c)
    }
}
fn native_factory() -> Result<NativeFactory> {
    let text = match std::env::var("DFMCP_RUN_ENDPOINT") {
        Err(std::env::VarError::NotPresent) => "127.0.0.1:5000".into(),
        Ok(v) if v.len() <= 128 => v,
        _ => {
            return Err(error(
                ErrorCode::InvalidRequest,
                "run endpoint must be bounded numeric loopback",
            ));
        }
    };
    let endpoint = dfmcp_adapter::parse_loopback_endpoint(&text)?;
    let token = configured("DFMCP_RUN_TOKEN", 256)?.into_bytes();
    if token.len() < 32 {
        return Err(error(
            ErrorCode::CapabilityDenied,
            "run credential is too short",
        ));
    }
    Ok(NativeFactory { endpoint, token })
}
fn source_anchor(observation: &RunObservation) -> StateAnchor {
    StateAnchor {
        fortress_id: FortressId::NIL,
        cursor: ObservationCursor {
            epoch: observation.generation(),
            sequence: observation.sequence(),
        },
        tick: GameTick(observation.tick().unwrap_or(0)),
        state_hash: observation.witness(),
    }
}
struct RunSession<S, F> {
    id: SessionId,
    request: u128,
    anchor: StateAnchor,
    budget: WorkBudget,
    grants: Vec<CapabilityGrant>,
    journal: RunJournal<S>,
    factory: Option<F>,
    selected: Option<RunObservation>,
    cursors: Cursors,
}
#[derive(Default)]
struct Limits {
    wall: Option<u64>,
    bytes: Option<u64>,
    tokens: Option<u32>,
}
enum Action {
    Observe,
    Plan {
        key: String,
        spec: RunSpec,
        witness: Digest32,
    },
    Commit {
        key: String,
        digest: Digest32,
        confirm: bool,
    },
    Wait {
        key: String,
        digest: Digest32,
    },
    Cancel {
        key: String,
        digest: Digest32,
    },
    Explain {
        key: String,
        digest: Digest32,
    },
    Query {
        filter: Filter,
        limit: usize,
        continuation: Option<String>,
    },
    Doctor,
    Unavailable,
}
impl<S: EffectJournalStorage, F: SourceFactory> RunSession<S, F> {
    fn context(&mut self, enabled: bool, cancelled: bool) -> Result<OperationContext> {
        self.request = self
            .request
            .checked_add(1)
            .ok_or_else(|| error(ErrorCode::BudgetExceeded, "run request IDs exhausted"))?;
        Ok(OperationContext {
            session_id: self.id,
            request_id: RequestId::new(self.request),
            anchor: self.anchor,
            budget: self.budget,
            grants: self
                .grants
                .iter()
                .filter(|g| {
                    enabled || !matches!(g.capability, Capability::Plan | Capability::ControlClock)
                })
                .cloned()
                .collect(),
            cancellation_requested: cancelled,
        })
    }
    fn connect(
        &mut self,
        c: &mut OperationContext,
        started: Instant,
        total: u64,
    ) -> Result<F::Source> {
        if self.journal.mode() == RunMode::Offline {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "offline run session has no native connection",
            ));
        }
        let mut native = remaining(c.clone(), started, total)?;
        c.budget.max_bytes = c
            .budget
            .max_bytes
            .checked_sub(CONNECT_RESERVE)
            .filter(|v| *v > 0)
            .ok_or_else(|| {
                error(
                    ErrorCode::BudgetExceeded,
                    "run handshake and effect work do not fit",
                )
            })?;
        native.budget.max_bytes = native
            .budget
            .max_bytes
            .min(CONNECT_RESERVE + 3 * RPC_RESERVE_BYTES);
        self.factory
            .as_mut()
            .ok_or_else(|| {
                error(
                    ErrorCode::CapabilityDenied,
                    "no native run source configured",
                )
            })?
            .connect(&native)
    }
    fn control(&self, c: &OperationContext) -> Result<()> {
        if self.journal.mode() != RunMode::Control {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "recovery cannot be promoted to run control",
            ));
        }
        c.authorize(Capability::ControlClock, RiskTier::Guarded, &[], None)
    }
    fn selected_record(records: &[DurableRun], key: &str, digest: Digest32) -> Result<DurableRun> {
        validate_key(key)?;
        records
            .iter()
            .find(|r| r.plan().key() == key && r.plan().digest() == digest)
            .cloned()
            .ok_or_else(|| {
                error(
                    ErrorCode::InvalidRequest,
                    "run key/digest not retained in this journal",
                )
            })
    }
    fn perform(
        &mut self,
        action: Action,
        mut c: OperationContext,
        started: Instant,
        total: u64,
    ) -> Result<Value> {
        // Reserve and verify the complete retained domain once, including for
        // local/terminal lookups. This cost is not reused as effect I/O budget.
        let mut read = remaining(c.clone(), started, total)?;
        c.budget.max_bytes = c
            .budget
            .max_bytes
            .checked_sub(LIST_RESERVE)
            .filter(|v| *v > 0)
            .ok_or_else(|| {
                error(
                    ErrorCode::BudgetExceeded,
                    "complete run journal verification exceeds work allowance",
                )
            })?;
        read.budget.max_bytes = LIST_RESERVE;
        let records = self.journal.records(&read)?;
        c = remaining(c, started, total)?;
        match action {
            Action::Query {
                filter,
                limit,
                continuation,
            } => {
                let key = PageKey {
                    session: self.id,
                    journal: self.journal.id(),
                    head: self.journal.head(),
                    filter,
                    limit,
                };
                let selected = records
                    .iter()
                    .filter(|r| filter.matches(r))
                    .collect::<Vec<_>>();
                let offset = continuation
                    .as_deref()
                    .map(|s| self.cursors.resolve(s, &key))
                    .transpose()?
                    .unwrap_or(0);
                if offset > selected.len() {
                    return Err(error(
                        ErrorCode::StaleAnchor,
                        "run continuation is outside retained selection",
                    ));
                }
                let end = (offset + limit).min(selected.len());
                let next = if end < selected.len() {
                    Some(self.cursors.issue(key, end)?)
                } else {
                    None
                };
                Ok(
                    json!({"ok":true,"records":selected[offset..end].iter().map(|r|presentation::record(r)).collect::<Vec<_>>(),
                    "matching_records":selected.len(),"state":filter.name(),"continuation":next,
                    "complete_matching_set_in_this_response":offset==0&&end==selected.len(),"native_calls":0}),
                )
            }
            Action::Explain { key, digest } => Ok(
                json!({"ok":true,"effect":presentation::record(&Self::selected_record(&records,&key,digest)?),"native_calls":0}),
            ),
            Action::Doctor => Ok(
                json!({"ok":true,"journal_id":self.journal.id().to_string(),"head":self.journal.head().to_string(),
                "retained_runs":records.len(),"transitions":self.journal.transitions(),"native_calls":0,"runtime_admitted":false}),
            ),
            Action::Unavailable => Err(error(
                ErrorCode::CapabilityDenied,
                "run/1.13 does not implement checkpoints, restore or other game action families",
            )),
            Action::Observe => {
                self.selected = None;
                let mut source = self.connect(&mut c, started, total)?;
                let observed = self
                    .journal
                    .observe(&mut source, &remaining(c, started, total)?)?;
                self.anchor = source_anchor(&observed);
                let result = Ok(
                    json!({"ok":true,"observation":presentation::observation(&observed),"game_mutation_dispatched":false}),
                );
                self.selected = Some(observed);
                result
            }
            Action::Plan { key, spec, witness } => {
                self.control(&c)?;
                c.authorize(Capability::Plan, RiskTier::Guarded, &[], None)?;
                validate_key(&key)?;
                if u64::from(spec.game_ticks()) > c.budget.max_game_ticks {
                    return Err(error(
                        ErrorCode::BudgetExceeded,
                        "requested run exceeds session tick allowance",
                    ));
                }
                let observed = self
                    .selected
                    .as_ref()
                    .filter(|o| o.witness() == witness)
                    .ok_or_else(|| {
                        error(
                            ErrorCode::StaleAnchor,
                            "observe first; plan requires the exact retained paused witness",
                        )
                    })?
                    .clone();
                let plan = RunPlan::new(&key, spec, observed)?;
                let mut source = self.connect(&mut c, started, total)?;
                let result =
                    self.journal
                        .prepare(&mut source, plan, &remaining(c, started, total)?);
                if result.is_err() {
                    self.selected = None;
                }
                Ok(
                    json!({"ok":true,"effect":presentation::record(&result?),"unpause_dispatched":false}),
                )
            }
            Action::Commit {
                key,
                digest,
                confirm,
            } => {
                self.control(&c)?;
                if !confirm {
                    return Err(error(
                        ErrorCode::CapabilityDenied,
                        "commit requires explicit confirmation of the sealed bounded run",
                    ));
                }
                let record = Self::selected_record(&records, &key, digest)?;
                if record.terminal() {
                    return Ok(
                        json!({"ok":true,"effect":presentation::record(&record),"native_calls":0,"replayed_receipt":true}),
                    );
                }
                if record.state() != RunState::Prepared
                    || records.iter().any(DurableRun::unresolved)
                {
                    return Err(error(
                        ErrorCode::EffectIndeterminate,
                        "a run is unresolved; query/cancel rather than replaying commit",
                    ));
                }
                self.selected = None;
                let mut source = self.connect(&mut c, started, total)?;
                let outcome = self.journal.commit(
                    &mut source,
                    &key,
                    digest,
                    &remaining(c, started, total)?,
                )?;
                Ok(
                    json!({"ok":true,"effect":presentation::record(&outcome),"operation_acknowledged":true,"goal_completion_proven":false}),
                )
            }
            Action::Wait { key, digest } => {
                let record = Self::selected_record(&records, &key, digest)?;
                if record.terminal() {
                    return Ok(
                        json!({"ok":true,"effect":presentation::record(&record),"native_calls":0,"historical":true}),
                    );
                }
                let mut source = self.connect(&mut c, started, total)?;
                let result = self.journal.reconcile(
                    &mut source,
                    &key,
                    digest,
                    &remaining(c, started, total)?,
                );
                if result.is_err() {
                    self.selected = None;
                }
                Ok(
                    json!({"ok":true,"effect":presentation::record(&result?),"query_samples":1,"unpause_dispatched":false}),
                )
            }
            Action::Cancel { key, digest } => {
                self.control(&c)?;
                let record = Self::selected_record(&records, &key, digest)?;
                let outcome = if record.terminal()
                    || matches!(record.state(), RunState::Intent | RunState::Prepared)
                {
                    self.journal
                        .cancel(None, &key, digest, &remaining(c, started, total)?)?
                } else {
                    self.selected = None;
                    let mut source = self.connect(&mut c, started, total)?;
                    self.journal.cancel(
                        Some(&mut source),
                        &key,
                        digest,
                        &remaining(c, started, total)?,
                    )?
                };
                Ok(
                    json!({"ok":true,"effect":presentation::record(&outcome),"unpause_dispatched":false,"rollback_performed":false}),
                )
            }
        }
    }
    fn render(
        &self,
        c: &OperationContext,
        operation: &str,
        result: Value,
        verified: bool,
    ) -> String {
        let mut c = c.clone();
        c.anchor = self.anchor;
        if let Err(cause) = c.authorize(Capability::Query, RiskTier::ReadOnly, &[], None) {
            return unbound(operation, &cause);
        }
        // Response anchors identify the published coordination root. The exact
        // native precondition observation is projected separately, never passed
        // off as current game state after a dispatch or historical lookup.
        c.anchor = StateAnchor {
            fortress_id: FortressId::NIL,
            cursor: ObservationCursor {
                epoch: self.journal.binding().manifest().generation,
                sequence: self.journal.transitions(),
            },
            tick: GameTick(0),
            state_hash: self.journal.head(),
        };
        let records = self.journal.cached_records().cloned().collect::<Vec<_>>();
        packet(
            operation,
            result,
            View {
                context: Some(&c),
                mode: Some(self.journal.mode()),
                journal: Some(
                    json!({"id":self.journal.id().to_string(),"head":self.journal.head().to_string(),
                "transitions":self.journal.transitions(),"source_generation":self.journal.binding().manifest().generation,
                "fenced":self.journal.fenced()}),
                ),
                records: &records,
                selected: self.selected.as_ref(),
                custody_verified: verified && !self.journal.fenced(),
            },
        )
    }
}
fn unbound(operation: &str, cause: &dfmcp_core::DfmcpError) -> String {
    packet(
        operation,
        failure(cause, operation),
        View {
            context: None,
            mode: None,
            journal: None,
            records: &[],
            selected: None,
            custody_verified: false,
        },
    )
}
fn narrowed(
    mut c: OperationContext,
    limits: Limits,
    rows: usize,
) -> Result<(OperationContext, OperationContext)> {
    if rows > MAX_PAGE {
        return Err(error(
            ErrorCode::InvalidRequest,
            "run output page exceeds eight records",
        ));
    }
    if let Some(v) = limits.wall {
        c.budget.max_wall_millis = c.budget.max_wall_millis.min(v);
    }
    if let Some(v) = limits.bytes {
        c.budget.max_bytes = c.budget.max_bytes.min(v);
    }
    if let Some(v) = limits.tokens {
        c.budget.max_output_tokens = c.budget.max_output_tokens.min(v);
    }
    c.budget.validate()?;
    let reserve = BASE_RESERVE + ROW_RESERVE * rows as u64;
    if c.budget.max_bytes <= reserve || u64::from(c.budget.max_output_tokens) * 4 < reserve {
        return Err(error(
            ErrorCode::BudgetExceeded,
            "complete run response does not fit; no native work or journal transition started",
        ));
    }
    let mut work = c.clone();
    work.budget.max_bytes -= reserve;
    Ok((c, work))
}
fn run_action<S: EffectJournalStorage, F: SourceFactory>(
    state: &mut RunSession<S, F>,
    c: OperationContext,
    operation: &str,
    limits: Limits,
    rows: usize,
    action: Result<Action>,
) -> String {
    let started = Instant::now();
    let (display, work) = match narrowed(c.clone(), limits, rows) {
        Ok(pair) => pair,
        Err(cause) => return unbound(operation, &cause),
    };
    let outcome =
        action.and_then(|a| state.perform(a, work, started, display.budget.max_wall_millis));
    let outcome = outcome.and_then(|value| {
        remaining(display.clone(), started, display.budget.max_wall_millis).map(|_| value)
    });
    let verified = outcome.is_ok();
    let out = state.render(
        &display,
        operation,
        outcome.unwrap_or_else(|e| failure(&e, operation)),
        verified,
    );
    if out.len() as u64
        <= display
            .budget
            .max_bytes
            .min(u64::from(display.budget.max_output_tokens) * 4)
    {
        return out;
    }
    // Never turn post-effect output refusal into proof of no dispatch.
    unbound(
        operation,
        &error(
            if matches!(operation, "fortress.commit" | "fortress.cancel") {
                ErrorCode::EffectIndeterminate
            } else {
                ErrorCode::BudgetExceeded
            },
            "run result exceeded reserved output; inspect retained journal evidence before further control",
        ),
    )
}
fn session_lock() -> Result<MutexGuard<'static, Option<LiveSession>>> {
    match SESSION.try_lock() {
        Ok(s) => Ok(s),
        Err(TryLockError::WouldBlock) => Err(error(
            ErrorCode::BudgetExceeded,
            "run session is serving another bounded request",
        )),
        Err(TryLockError::Poisoned(_)) => Err(error(
            ErrorCode::InternalInvariantViolation,
            "run session poisoned; restart and recover journal",
        )),
    }
}
fn session_id(raw: &str) -> Result<SessionId> {
    if raw.len() != 32
        || !raw
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(error(ErrorCode::InvalidRequest, "invalid run session ID"));
    }
    let value = u128::from_str_radix(raw, 16)
        .map_err(|_| error(ErrorCode::InvalidRequest, "invalid run session ID"))?;
    let id = SessionId::new(value);
    if id.get() != value
        || !id.is_process_scoped_live()
        || (value & ((1u128 << 62) - 1)) >> 57 != 13
    {
        return Err(error(ErrorCode::InvalidRequest, "not a run/1.13 session"));
    }
    Ok(id)
}
fn with_session(
    raw: String,
    operation: &str,
    limits: Limits,
    rows: usize,
    action: Result<Action>,
) -> String {
    let result = (|| {
        runtime_io()?;
        validate_environment()?;
        let id = session_id(&raw)?;
        let mut guard = session_lock()?;
        let state = guard
            .as_mut()
            .filter(|s| s.id == id)
            .ok_or_else(|| error(ErrorCode::SessionNotFound, "run session absent or closed"))?;
        let c = state.context(clock_enabled(), false)?;
        let out = run_action(state, c, operation, limits, rows, action);
        runtime_io()?;
        Ok(out)
    })();
    result.unwrap_or_else(|e| unbound(operation, &e))
}

#[tool(
    description = "Open an isolated unadmitted bounded-run session. Default offline reads an existing private journal without bridge credentials. Recover can only query and retain evidence; control requires separate operator clock enablement. Paths, endpoints, protocol and credentials are never tool arguments."
)]
pub fn fortress_open_session(
    mode: Option<String>,
    max_wall_millis: Option<u64>,
    max_bytes: Option<u64>,
    max_output_tokens: Option<u32>,
    max_game_ticks: Option<u64>,
) -> String {
    let result = (|| -> Result<String> {
        runtime_io()?;
        validate_environment()?;
        let started = Instant::now();
        let mode = parse_mode(mode.as_deref().unwrap_or("offline"))?;
        let mut guard = session_lock()?;
        if guard.is_some() {
            return Err(error(
                ErrorCode::Conflict,
                "release the retained run session first",
            ));
        }
        let grants = grants(mode, clock_enabled())?;
        let path = PathBuf::from(configured("DFMCP_RUN_JOURNAL", 4096)?);
        let budget = WorkBudget {
            max_wall_millis: max_wall_millis.unwrap_or(5000),
            max_bytes: max_bytes.unwrap_or(MAX_BYTES),
            max_output_tokens: max_output_tokens.unwrap_or(16_384),
            max_game_ticks: max_game_ticks.unwrap_or(1200),
            max_entities: 256,
            max_actions: 1,
        };
        if budget.max_wall_millis > 60_000
            || budget.max_bytes > MAX_BYTES
            || budget.max_output_tokens > 65_536
            || budget.max_game_ticks > 1200
        {
            return Err(error(
                ErrorCode::BudgetExceeded,
                "run opening exceeds 60000ms, 16MiB, 65536 token-proxy units or 1200 game ticks",
            ));
        }
        let sequence = NEXT
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < (1u64 << 57)).then_some(n + 1)
            })
            .map_err(|_| error(ErrorCode::BudgetExceeded, "run session IDs exhausted"))?;
        let id = SessionId::new((1u128 << 127) | FAMILY | u128::from(sequence));
        let anchor = StateAnchor {
            fortress_id: FortressId::NIL,
            cursor: ObservationCursor::ORIGIN,
            tick: GameTick(0),
            state_hash: Digest32::ZERO,
        };
        let c = OperationContext {
            session_id: id,
            request_id: RequestId::new(1),
            anchor,
            budget,
            grants: grants.clone(),
            cancellation_requested: false,
        };
        let (display, mut work) = narrowed(c, Limits::default(), 0)?;
        // Offline branches before credential/endpoint reads or source construction.
        let mut factory = if mode == RunMode::Offline {
            None
        } else {
            Some(native_factory()?)
        };
        let mut selected = None;
        let mut binding = None;
        if mode == RunMode::Control {
            work.budget.max_bytes = work
                .budget
                .max_bytes
                .checked_sub(CONNECT_RESERVE + RPC_RESERVE_BYTES)
                .filter(|n| *n > LIST_RESERVE)
                .ok_or_else(|| {
                    error(
                        ErrorCode::BudgetExceeded,
                        "run bootstrap source and journal do not fit",
                    )
                })?;
            let mut native = remaining(work.clone(), started, budget.max_wall_millis)?;
            native.budget.max_bytes = CONNECT_RESERVE + RPC_RESERVE_BYTES;
            let source = factory.as_mut().ok_or_else(|| {
                error(
                    ErrorCode::InternalInvariantViolation,
                    "control source absent",
                )
            })?;
            let mut source = source.connect(&native)?;
            let mut read = remaining(work.clone(), started, budget.max_wall_millis)?;
            read.budget.max_bytes = RPC_RESERVE_BYTES;
            let observed = source.observe(&read)?;
            binding = Some(RunBinding::new(
                source.endpoint().ok_or_else(|| {
                    error(ErrorCode::AdapterRejected, "native source has no endpoint")
                })?,
                source.manifest().clone(),
            )?);
            work.anchor = source_anchor(&observed);
            selected = Some(observed);
        }
        runtime_io()?;
        if mode == RunMode::Control {
            native_guard(true)?;
        }
        work = remaining(work, started, budget.max_wall_millis)?;
        let journal = open_private_run_journal(&path, &work, mode, binding)?;
        if factory
            .as_ref()
            .is_some_and(|f| f.endpoint != journal.binding().endpoint())
        {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "operator endpoint differs from retained run journal",
            ));
        }
        let anchor = selected.as_ref().map(source_anchor).unwrap_or(StateAnchor {
            fortress_id: FortressId::NIL,
            cursor: ObservationCursor {
                epoch: journal.binding().manifest().generation,
                sequence: 0,
            },
            tick: GameTick(0),
            state_hash: journal.head(),
        });
        let state = RunSession {
            id,
            request: 1,
            anchor,
            budget,
            grants,
            journal,
            factory,
            selected,
            cursors: Cursors::default(),
        };
        let out=state.render(&display,"fortress.open_session",json!({"ok":true,"session_id":id.to_string(),"mode":mode.as_str(),
            "source_connections_retained":0,"journal_opened":true,"capabilities":state.grants.iter().map(|g|g.capability.as_str()).collect::<Vec<_>>(),
            "discovery":{"tool":"fortress.query","arguments":{"session_id":id.to_string(),"state":"unresolved","limit":2}}}),true);
        if out.len() as u64
            > budget
                .max_bytes
                .min(u64::from(budget.max_output_tokens) * 4)
        {
            return Err(error(
                ErrorCode::BudgetExceeded,
                "run opening result does not fit; session not published",
            ));
        }
        remaining(work, started, budget.max_wall_millis)?;
        runtime_io()?;
        *guard = Some(state);
        Ok(out)
    })();
    result.unwrap_or_else(|e| unbound("fortress.open_session", &e))
}
#[tool(
    description = "Capture the native run clock and its exact witness without advancing time. Failed refresh discards old selection. The source has no verified named-fortress identity."
)]
pub fn fortress_observe(session_id: String) -> String {
    with_session(
        session_id,
        "fortress.observe",
        Limits::default(),
        1,
        Ok(Action::Observe),
    )
}
#[tool(
    description = "Discover durable run intent and outcomes without native calls, including offline. State all/pending/unresolved/terminal; limit 1..8. Whole-record continuations bind session, journal, head, filter and limit."
)]
pub fn fortress_query(
    session_id: String,
    state: Option<String>,
    limit: Option<u32>,
    continuation: Option<String>,
    max_bytes: Option<u64>,
    max_output_tokens: Option<u32>,
) -> String {
    let limit = limit.unwrap_or(2) as usize;
    let action = (|| {
        if !(1..=MAX_PAGE).contains(&limit) {
            return Err(error(
                ErrorCode::InvalidRequest,
                "run page limit must be 1..8",
            ));
        }
        Ok(Action::Query {
            filter: Filter::parse(state.as_deref().unwrap_or("all"))?,
            limit,
            continuation,
        })
    })();
    with_session(
        session_id,
        "fortress.query",
        Limits {
            bytes: max_bytes,
            tokens: max_output_tokens,
            wall: None,
        },
        limit,
        action,
    )
}
#[tool(
    description = "Seal and durably prepare one bounded run from the exact observed paused witness. game_ticks 1..1200; run_wall_millis 1..60000. This does not unpause. Both limits are callback stop triggers, not exact-tick or hard real-time promises."
)]
pub fn fortress_plan(
    session_id: String,
    idempotency_key: String,
    game_ticks: u32,
    run_wall_millis: u32,
    expected_witness: String,
) -> String {
    let action = (|| {
        Ok(Action::Plan {
            key: idempotency_key,
            spec: RunSpec::new(game_ticks, run_wall_millis)?,
            witness: digest(&expected_witness)?,
        })
    })();
    with_session(session_id, "fortress.plan", Limits::default(), 1, action)
}
#[tool(
    description = "Commit one exact durable bounded-run plan with confirm=true. Sync dispatch before unpause; ambiguous outcomes are never retried. Native callback ownership continues after disconnect. Acknowledgement is not goal completion."
)]
pub fn fortress_commit(
    session_id: String,
    idempotency_key: String,
    plan_digest: String,
    confirm: bool,
) -> String {
    with_session(
        session_id,
        "fortress.commit",
        Limits::default(),
        1,
        digest(&plan_digest).map(|digest| Action::Commit {
            key: idempotency_key,
            digest,
            confirm,
        }),
    )
}
#[tool(
    description = "Perform one foreground QueryRun reconciliation and durably retain its evidence. No loop, unpause, automatic retry or budget extension. Missing native records remain unknown; terminal receipts are historical."
)]
pub fn fortress_wait(
    session_id: String,
    idempotency_key: String,
    plan_digest: String,
    max_wall_millis: Option<u64>,
) -> String {
    with_session(
        session_id,
        "fortress.wait",
        Limits {
            wall: max_wall_millis,
            ..Limits::default()
        },
        1,
        digest(&plan_digest).map(|digest| Action::Wait {
            key: idempotency_key,
            digest,
        }),
    )
}
#[tool(
    description = "Inspect a retained run by exact key/digest without native calls, including offline. Historical stop receipts do not prove current pause or goal completion."
)]
pub fn fortress_explain(
    session_id: String,
    idempotency_key: String,
    plan_digest: String,
) -> String {
    with_session(
        session_id,
        "fortress.explain",
        Limits::default(),
        1,
        digest(&plan_digest).map(|digest| Action::Explain {
            key: idempotency_key,
            digest,
        }),
    )
}
fn close(raw: String, release: bool) -> String {
    let result = (|| -> Result<String> {
        let id = session_id(&raw)?;
        let mut guard = session_lock()?;
        let state = guard
            .as_mut()
            .filter(|s| s.id == id)
            .ok_or_else(|| error(ErrorCode::SessionNotFound, "run session absent or closed"))?;
        if !release {
            runtime_io()?;
            let c = state.context(false, false)?;
            if state
                .journal
                .records(&c)?
                .iter()
                .any(|r| !r.terminal() || r.unresolved())
            {
                return Err(error(
                    ErrorCode::Conflict,
                    "run work remains; cancel/reconcile first or explicitly release custody for recovery",
                ));
            }
        }
        let out = packet(
            "fortress.cancel",
            json!({"ok":true,"scope":"session","closed":true,"custody_released":true,
            "effects_cancelled":false,"native_pause_proven":false,"native_calls":0,"journal_erased":false,
            "release_for_recovery":release,"native_stop_owner":"unchanged; not transferred or discharged by closing MCP"}),
            View {
                context: None,
                mode: Some(state.journal.mode()),
                journal: None,
                records: &[],
                selected: None,
                custody_verified: false,
            },
        );
        drop(guard.take());
        Ok(out)
    })();
    result.unwrap_or_else(|e| unbound("fortress.cancel", &e))
}
#[tool(
    description = "scope=effect with key/digest retires undispatched intent or durably requests native safety pause. scope=session without key/digest releases custody only when no work remains, unless release_for_recovery=true explicitly hands off unresolved custody. Closing never erases evidence or claims pause."
)]
pub fn fortress_cancel(
    session_id: String,
    scope: String,
    idempotency_key: Option<String>,
    plan_digest: Option<String>,
    release_for_recovery: Option<bool>,
) -> String {
    match (
        scope.as_str(),
        idempotency_key,
        plan_digest,
        release_for_recovery.unwrap_or(false),
    ) {
        ("session", None, None, release) => close(session_id, release),
        ("effect", Some(key), Some(plan), false) => with_session(
            session_id,
            "fortress.cancel",
            Limits::default(),
            1,
            digest(&plan).map(|digest| Action::Cancel { key, digest }),
        ),
        _ => unbound(
            "fortress.cancel",
            &error(
                ErrorCode::InvalidRequest,
                "cancel requires effect key/digest, or session without effect identity",
            ),
        ),
    }
}
#[tool(description = "Unavailable in run/1.13. A coordination journal is not a game checkpoint.")]
pub fn fortress_checkpoint(session_id: String) -> String {
    with_session(
        session_id,
        "fortress.checkpoint",
        Limits::default(),
        0,
        Ok(Action::Unavailable),
    )
}
#[tool(description = "Unavailable in run/1.13. Reopening a journal never restores game state.")]
pub fn fortress_restore(session_id: String) -> String {
    with_session(
        session_id,
        "fortress.restore",
        Limits::default(),
        0,
        Ok(Action::Unavailable),
    )
}
#[tool(
    description = "Reverify run journal custody and expose bounded counts without native calls. Not a compatibility, current-pause, native-health or production-admission claim."
)]
pub fn fortress_doctor(session_id: String) -> String {
    with_session(
        session_id,
        "fortress.doctor",
        Limits::default(),
        0,
        Ok(Action::Doctor),
    )
}

pub fn run_stdio() {
    if let Err(cause) = validate_environment() {
        eprintln!("{cause}");
        std::process::exit(1);
    }
    let server=ServerBuilder::new("dfmcp-live-run-dev",env!("CARGO_PKG_VERSION"))
        .tool(FortressOpenSession).tool(FortressObserve).tool(FortressQuery).tool(FortressPlan).tool(FortressCommit)
        .tool(FortressWait).tool(FortressCancel).tool(FortressCheckpoint).tool(FortressRestore).tool(FortressExplain).tool(FortressDoctor)
        .instructions("Unadmitted run/1.13. Default offline discovers durable run evidence with no native connection. Control requires operator clock enablement. Observe exact paused state, seal finite tick/wall limits, then explicitly confirm commit once. Native callback owns stopping after disconnect. Query/wait never advance time. Ambiguous dispatch cannot be retried or replaced with another key. Use query/cancel reconciliation. Historical pause is not current pause or goal completion. Source identity is not a named fortress. No checkpoints, restore, shell, Lua or arbitrary commands. Session release is not cancellation of native work.")
        .build();
    crate::run_modern_stdio(server);
}
#[cfg(test)]
mod tests;
