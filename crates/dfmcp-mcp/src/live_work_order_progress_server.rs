#![forbid(unsafe_code)]
//! Explicitly unadmitted, foreground-only work-order progress. No mutation edge.
use dfmcp_adapter::work_order_progress::archive::{
    ArchiveMode, MAX_FRAME_BYTES, PrivateProgressArchiveFile, ProgressArchive,
    ProgressArchiveEntry, ProgressArchiveSummary, open_progress_archive,
};
use dfmcp_adapter::work_order_progress::rpc::{ProgressRpcClient, ProgressTcpStream};
use dfmcp_adapter::work_order_progress::{
    self as progress, ProgressComparison, ProgressObservation, ProgressSession,
};
use dfmcp_core::{
    AgentPhase, Capability, CapabilityGrant, CapabilityScope, ContinuityStatus, DfmcpError,
    Digest32, ErrorCode, FortressId, GameTick, ObservationCursor, OperationContext, RequestId,
    Result, RiskTier, SessionId, StateAnchor, WorkBudget,
};
use fastmcp_rust::modern::ServerBuilder;
use fastmcp_rust::prelude::*;
use serde_json::{Value, json};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, TryLockError};
use std::time::{Duration, Instant};

#[path = "progress_history.rs"]
mod history;
#[path = "progress_watches.rs"]
mod watches;
use progress::watches::{
    BOOK_OPEN_RESERVE, MAX_WATCH_HORIZON, PrivateWatchFile, WatchBook, WatchBookSummary,
    open_watch_book,
};

const MAX_BYTES: u64 = 68 * 1024 * 1024;
const WATCH_BYTES: u64 = 192 * 1024 * 1024;
const TRANSIENT_BYTES: u64 = 2 * 1024 * 1024;
const BASE_RESERVE: u64 = 16 * 1024;
const ROW_RESERVE: u64 = 4 * 1024;
const FAMILY: u128 = 12u128 << 57;
static NEXT: AtomicU64 = AtomicU64::new(1);
static SESSION: Mutex<Option<RuntimeSession>> = Mutex::new(None);
type Reader = ProgressSession<ProgressRpcClient<ProgressTcpStream>>;
struct RuntimeSession {
    id: SessionId,
    request: u128,
    anchor: StateAnchor,
    budget: WorkBudget,
    grants: Vec<CapabilityGrant>,
    ids: Vec<u32>,
    reader: Option<Reader>,
    archive: Option<ProgressArchive<PrivateProgressArchiveFile>>,
    record: Option<ProgressArchiveEntry>,
    offline: bool,
    cursors: history::Cursors,
    watch_book: Option<WatchBook<PrivateWatchFile>>,
}
struct Projection {
    value: Value,
    capture: Option<ProgressObservation>,
    comparison: Option<ProgressComparison>,
    historical: bool,
}
impl Projection {
    fn plain(value: Value, historical: bool) -> Self {
        Self {
            value,
            capture: None,
            comparison: None,
            historical,
        }
    }
}
fn error(code: ErrorCode, text: &str) -> DfmcpError {
    DfmcpError::new(code, text)
}
fn runtime_io() -> Result<()> {
    let cx = asupersync::Cx::current().ok_or_else(|| {
        error(
            ErrorCode::CapabilityDenied,
            "progress requires its owned runtime context",
        )
    })?;
    cx.checkpoint().map_err(|_| {
        error(
            ErrorCode::CancellationRequested,
            "progress request cancelled",
        )
    })?;
    if cx.io().is_none() {
        return Err(error(
            ErrorCode::CapabilityDenied,
            "inherited runtime denies progress I/O",
        ));
    }
    Ok(())
}
const ENVIRONMENT: [&str; 6] = [
    "DFMCP_ALLOW_UNADMITTED_WORK_ORDER_PROGRESS_V1_12",
    "DFMCP_WORK_ORDER_PROGRESS_TOKEN",
    "DFMCP_WORK_ORDER_PROGRESS_ENDPOINT",
    "DFMCP_WORK_ORDER_PROGRESS_FORTRESS_ID",
    "DFMCP_WORK_ORDER_PROGRESS_JOURNAL",
    "DFMCP_WORK_ORDER_PROGRESS_WATCHES",
];
fn environment_contract(opt_in: Option<&str>, keys: &[String], admitted: bool) -> Result<()> {
    if opt_in != Some("1")
        || admitted
        || keys
            .iter()
            .any(|k| k.starts_with("DFMCP_") && !ENVIRONMENT.contains(&k.as_str()))
    {
        return Err(error(
            ErrorCode::CapabilityDenied,
            "progress/1.12 requires exact development opt-in and refuses production/admission or other DFMCP environment state",
        ));
    }
    Ok(())
}
fn validate_environment() -> Result<()> {
    let keys = std::env::vars_os()
        .map(|(k, _)| k.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    environment_contract(
        std::env::var(ENVIRONMENT[0]).ok().as_deref(),
        &keys,
        crate::admission::current_admission_provenance().is_some(),
    )
}
fn configured(name: &str, maximum: usize) -> Result<String> {
    let value = std::env::var(name).map_err(|_| {
        error(
            ErrorCode::CapabilityDenied,
            "required progress operator configuration is absent or not UTF-8",
        )
    })?;
    if value.is_empty() || value.len() > maximum || value.contains('\0') {
        return Err(error(
            ErrorCode::InvalidRequest,
            "invalid progress operator configuration bound",
        ));
    }
    Ok(value)
}
fn slot() -> Result<MutexGuard<'static, Option<RuntimeSession>>> {
    match SESSION.try_lock() {
        Ok(g) => Ok(g),
        Err(TryLockError::WouldBlock) => Err(error(
            ErrorCode::BudgetExceeded,
            "progress session is busy; no additional capture started",
        )),
        Err(TryLockError::Poisoned(_)) => Err(error(
            ErrorCode::InternalInvariantViolation,
            "progress session lock poisoned; restart this read-only process",
        )),
    }
}
fn parse_session(raw: &str) -> Result<SessionId> {
    if raw.len() != 32
        || !raw
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(error(
            ErrorCode::InvalidRequest,
            "invalid progress session ID",
        ));
    }
    let value = u128::from_str_radix(raw, 16)
        .map_err(|_| error(ErrorCode::InvalidRequest, "invalid session ID"))?;
    let id = SessionId::new(value);
    if id.get() != value
        || !id.is_process_scoped_live()
        || (value & ((1u128 << 62) - 1)) >> 57 != 12
    {
        return Err(error(
            ErrorCode::InvalidRequest,
            "not a progress/1.12 session ID",
        ));
    }
    Ok(id)
}
fn parse_digest(raw: &str) -> Result<Digest32> {
    if raw.len() != 64
        || !raw
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(error(
            ErrorCode::InvalidRequest,
            "witness must be lowercase SHA-256 hex",
        ));
    }
    let mut out = [0; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&raw[i * 2..i * 2 + 2], 16)
            .map_err(|_| error(ErrorCode::InvalidRequest, "invalid witness"))?;
    }
    Ok(Digest32::from_bytes(out))
}
fn normalized_ids(mut ids: Vec<u32>) -> Result<Vec<u32>> {
    if ids.len() > progress::MAX_TARGETS {
        return Err(error(
            ErrorCode::InvalidRequest,
            "progress selects at most 32 IDs",
        ));
    }
    ids.sort_unstable();
    progress::validate_targets(&ids)?;
    Ok(ids)
}
fn reserve(mut context: OperationContext, rows: usize) -> Result<OperationContext> {
    context.budget.validate()?;
    if rows == 0 || rows > progress::MAX_TARGETS {
        return Err(error(
            ErrorCode::InvalidRequest,
            "invalid progress selection size",
        ));
    }
    let reserve = BASE_RESERVE + ROW_RESERVE * rows as u64;
    if context.budget.max_bytes <= reserve
        || u64::from(context.budget.max_output_tokens) * 4 < reserve
    {
        return Err(error(
            ErrorCode::BudgetExceeded,
            "complete progress output does not fit; no capture was started",
        ));
    }
    context.budget.max_bytes -= reserve;
    Ok(context)
}
fn remaining(
    mut context: OperationContext,
    start: Instant,
    allowance: u64,
) -> Result<OperationContext> {
    let elapsed = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
    context.budget.max_wall_millis = allowance
        .checked_sub(elapsed)
        .filter(|n| *n > 0)
        .ok_or_else(|| {
            error(
                ErrorCode::BudgetExceeded,
                "progress request deadline expired",
            )
        })?;
    Ok(context)
}
impl RuntimeSession {
    fn context(&mut self) -> Result<OperationContext> {
        self.request = self
            .request
            .checked_add(1)
            .ok_or_else(|| error(ErrorCode::BudgetExceeded, "progress request IDs exhausted"))?;
        Ok(OperationContext {
            session_id: self.id,
            request_id: RequestId::new(self.request),
            anchor: self.anchor,
            budget: self.budget,
            grants: self.grants.clone(),
            cancellation_requested: asupersync::Cx::current()
                .is_some_and(|cx| cx.checkpoint().is_err()),
        })
    }
}
fn observation_json(o: &ProgressObservation) -> Value {
    let rows = o.rows().iter().map(|row| {
        let details = row.order.as_ref().map(|s| json!({"job_type":s.job_type,"job_type_key":s.type_key,"reaction":s.reaction,
            "recognized_recipe":s.recipe_name(),"reported_remaining":s.remaining,"reported_total":s.total,
            "validated":s.validated(),"active":s.active(),"raw_status_bits":s.status_bits,
            "frequency":s.frequency,"workshop_id":s.workshop_id,"max_workshops":s.max_workshops,
            "next_check_year":s.next_check_year,"next_check_tick":s.next_check_tick,
            "item_condition_count":s.item_conditions,"order_condition_count":s.order_conditions}));
        json!({"native_order_id":row.native_order_id,"present":row.order.is_some(),"phase":row.phase(),"details":details,
            "production_completion_proven":false,"historical_creation_identity_proven":false})
    }).collect::<Vec<_>>();
    json!({"witness":o.witness().to_string(),"bridge_generation":o.generation(),"capture_sequence":o.sequence(),
        "game_tick":o.tick(),"fortress_id":o.fortress_id().to_string(),"world_folder":o.world_folder(),"site_id":o.site(),
        "paused":o.paused(),"next_order_id":o.next_order_id(),"queue_count":o.queue_count(),"rows":rows,
        "selected_presence_complete":true,"continuous_history_proven":false})
}
fn comparison_json(c: &ProgressComparison) -> Value {
    json!({"status":c.status,"reset_reason":c.reset_reason,"baseline_witness":c.baseline.map(|d|d.to_string()),
        "elapsed_game_ticks":c.elapsed_game_ticks,"client_acknowledged_baseline":false,"continuous_history_proven":false,
        "changes":c.changes.iter().map(|v|json!({"native_order_id":v.native_order_id,"kind":v.kind,
            "before_phase":v.before_phase,"after_phase":v.after_phase,"remaining_counter_decrease":v.remaining_decrease,
            "remaining_counter_increase":v.remaining_increase,"goods_produced_proven":false})).collect::<Vec<_>>()})
}
fn failure(cause: &DfmcpError) -> Value {
    json!({"ok":false,"error":{"code":cause.code.as_str(),"message":cause.message,
        "game_mutation_dispatched":false,"recovery":"Inspect retained history and watch intent or explicitly close/reopen after source failure; no automatic reconnect."}})
}
fn packet(
    operation: &str,
    result: Value,
    context: Option<&OperationContext>,
    capture: Option<&ProgressObservation>,
    comparison: Option<&ProgressComparison>,
) -> String {
    packet_origin(operation, result, context, capture, comparison, false, None)
}
fn packet_origin(
    operation: &str,
    result: Value,
    context: Option<&OperationContext>,
    capture: Option<&ProgressObservation>,
    comparison: Option<&ProgressComparison>,
    historical: bool,
    archive: Option<&ProgressArchiveSummary>,
) -> String {
    packet_value(
        operation, result, context, capture, comparison, historical, archive,
    )
    .to_string()
}
fn packet_value(
    operation: &str,
    result: Value,
    context: Option<&OperationContext>,
    capture: Option<&ProgressObservation>,
    comparison: Option<&ProgressComparison>,
    historical: bool,
    archive: Option<&ProgressArchiveSummary>,
) -> Value {
    let phase = if operation == "fortress.open_session" {
        AgentPhase::Bootstrap
    } else {
        AgentPhase::Inspect
    };
    let continuity = if historical {
        ContinuityStatus::Stale
    } else if capture.is_none() {
        ContinuityStatus::Indeterminate
    } else if comparison.is_some_and(|c| c.status == "reset") {
        ContinuityStatus::Reset
    } else if comparison.is_some_and(|c| c.status == "bootstrap") {
        ContinuityStatus::Bootstrap
    } else {
        ContinuityStatus::Partial
    };
    let mut builder = crate::AgentTurnBuilder::new(operation, phase)
        .continuity(continuity, None, None, comparison.and_then(|c|c.reset_reason).map(str::to_owned))
        .briefing(json!({"runtime":"unadmitted_development","bridge_protocol":"1.12","runtime_admitted":false,"read_only":true,
            "current_freshness_proven":false,"production_completion_proven":false,"historical":historical,
            "progress_archive":archive.map(history::summary_json)}))
        .active_work(json!({"pending_plans":[],"actions":[],"obligations":[],"cancellation_drains":[],
            "indeterminate_effects":[],"publications":[],"confirmations":[],
            "scope":"This read-only session only; other journals, effects and monitors were not examined."}))
        .coverage(json!({"status":"partial","complete_domains":if capture.is_none(){json!([])}else if historical {json!(["historical_presence_for_selected_order_ids"])}else{json!(["presence_for_selected_order_ids"])},
            "partial_domains":if historical{json!(["historical_selected_order_progress_fields"])}else{json!(["selected_order_progress_fields"])} ,"omitted_domains":["continuous_history","full_order_configuration",
            "material_availability","causal_blockers","created_receipt_identity","goods_produced"],"continuation":null}))
        .uncertainty(vec![json!({"code":"counters_are_not_completion","detail":"Absent orders, zero remaining and counter decreases do not independently prove completed goods or resolve a creation receipt."})]);
    if let Some(c) = context {
        builder = builder
            .session_id(c.session_id.to_string())
            .request_id(c.request_id.to_string());
    }
    if let Some(o) = capture {
        builder = builder.anchor(json!({"kind":if historical {"historical_native_order_progress"} else {"selected_native_order_progress"},"fortress_id":o.fortress_id().to_string(),
            "bridge_generation":o.generation(),"capture_sequence":o.sequence(),"game_tick":o.tick(),"witness":o.witness().to_string(),
            "canonical_world_anchor":false}));
    }
    let mut turn = builder.build();
    // The common builder may attach ambient admission provenance. This isolated
    // profile never inherits it, including in an environment-refusal packet.
    if let Some(briefing) = turn.get_mut("briefing").and_then(Value::as_object_mut) {
        briefing.remove("admission");
    }
    json!({"result":result,"agent_turn":turn})
}
fn unbound(operation: &str, cause: &DfmcpError) -> String {
    packet(operation, failure(cause), None, None, None)
}
fn archive_path() -> Result<Option<PathBuf>> {
    if std::env::var_os(ENVIRONMENT[4]).is_none() {
        return Ok(None);
    }
    Ok(Some(PathBuf::from(configured(ENVIRONMENT[4], 4096)?)))
}
fn watch_path(archive: Option<&std::path::Path>) -> Result<Option<PathBuf>> {
    if std::env::var_os(ENVIRONMENT[5]).is_none() {
        return Ok(None);
    }
    let path = PathBuf::from(configured(ENVIRONMENT[5], 4096)?);
    validate_watch_path(archive, &path)?;
    Ok(Some(path))
}
fn validate_watch_path(archive: Option<&std::path::Path>, watches: &std::path::Path) -> Result<()> {
    if archive.is_none_or(|path| path == watches) {
        return Err(error(
            ErrorCode::InvalidRequest,
            "WATCHES requires a distinct operator JOURNAL path",
        ));
    }
    Ok(())
}
fn session_grants(fortress: FortressId, offline: bool) -> Vec<CapabilityGrant> {
    [Capability::Query, Capability::Observe]
        .into_iter()
        .filter(|cap| !offline || *cap == Capability::Query)
        .map(|capability| CapabilityGrant {
            capability,
            scope: CapabilityScope {
                fortress_id: Some(fortress),
                ..CapabilityScope::default()
            },
            max_risk: RiskTier::ReadOnly,
            expires_at_tick: None,
            remaining_uses: None,
        })
        .collect()
}
fn access(s: &mut RuntimeSession, c: &OperationContext) -> Result<Option<ProgressArchiveSummary>> {
    if s.id != c.session_id || s.anchor.fortress_id != c.anchor.fortress_id {
        return Err(error(
            ErrorCode::CapabilityDenied,
            "progress belongs to another session or fortress",
        ));
    }
    let mut current = c.clone();
    current.anchor.tick = GameTick(s.anchor.tick.get().max(c.anchor.tick.get()));
    current.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    let summary = s
        .archive
        .as_mut()
        .map(|a| a.summary(&current))
        .transpose()?;
    if let Some(book) = s.watch_book.as_mut() {
        let archive = s.archive.as_mut().ok_or_else(|| {
            error(
                ErrorCode::CorruptLedger,
                "watch book lost its paired archive",
            )
        })?;
        book.verify_access(archive, &current)?;
    }
    Ok(summary)
}
fn watch_summary(s: &mut RuntimeSession, c: &OperationContext) -> Result<Option<WatchBookSummary>> {
    match (&mut s.watch_book, &mut s.archive) {
        (Some(book), Some(archive)) => book.summary(archive, c).map(Some),
        (None, _) => Ok(None),
        _ => Err(error(
            ErrorCode::CorruptLedger,
            "watch book has no paired archive",
        )),
    }
}
fn render_checked(
    operation: &str,
    projection: Projection,
    s: &mut RuntimeSession,
    c: &OperationContext,
) -> Result<String> {
    let archive = access(s, c)?;
    let summary = watch_summary(s, c)?;
    let mut value = packet_value(
        operation,
        projection.value,
        Some(c),
        projection.capture.as_ref(),
        projection.comparison.as_ref(),
        projection.historical,
        archive.as_ref(),
    );
    watches::attach(&mut value, summary.as_ref(), s.id);
    let out = value.to_string();
    if out.len() as u64
        > c.budget
            .max_bytes
            .min(u64::from(c.budget.max_output_tokens) * 4)
    {
        return Err(error(
            ErrorCode::BudgetExceeded,
            "progress output exceeded its reservation; inspect retained history",
        ));
    }
    Ok(out)
}
fn render(
    operation: &str,
    projection: Projection,
    s: &mut RuntimeSession,
    c: &OperationContext,
) -> String {
    render_checked(operation, projection, s, c).unwrap_or_else(|cause| {
        let mut value = packet_value(
            operation,
            failure(&cause),
            Some(c),
            None,
            None,
            s.offline,
            None,
        );
        watches::unavailable(&mut value, s.watch_book.is_some());
        value.to_string()
    })
}
fn offline_session(
    mut archive: ProgressArchive<PrivateProgressArchiveFile>,
    c: &OperationContext,
    budget: WorkBudget,
) -> Result<RuntimeSession> {
    let summary = archive.summary(c)?;
    if !summary.read_only {
        return Err(error(
            ErrorCode::CapabilityDenied,
            "offline session requires immutable read-only custody",
        ));
    }
    let retained = archive.latest(c)?.ok_or_else(|| {
        error(
            ErrorCode::InvalidRequest,
            "empty progress archive cannot bootstrap observations",
        )
    })?;
    let anchor = StateAnchor {
        tick: GameTick(c.anchor.tick.get().max(summary.authority_tick_floor)),
        state_hash: retained.observation.witness(),
        ..c.anchor
    };
    Ok(RuntimeSession {
        id: c.session_id,
        request: c.request_id.get(),
        anchor,
        budget,
        grants: c
            .grants
            .iter()
            .filter(|g| g.capability == Capability::Query)
            .cloned()
            .collect(),
        ids: retained.observation.ids(),
        reader: None,
        archive: Some(archive),
        record: Some(retained.entry),
        offline: true,
        cursors: history::Cursors::default(),
        watch_book: None,
    })
}
fn publish_session(
    mut session: RuntimeSession,
    projection: Projection,
    c: &OperationContext,
    started: Instant,
    target: &mut Option<RuntimeSession>,
) -> Result<String> {
    if target.is_some() {
        return Err(error(ErrorCode::Conflict, "progress slot already occupied"));
    }
    let out = render_checked("fortress.open_session", projection, &mut session, c)?;
    if started.elapsed() >= Duration::from_millis(c.budget.max_wall_millis) {
        return Err(error(
            ErrorCode::BudgetExceeded,
            "progress bootstrap exceeded its budget; custody was not published",
        ));
    }
    *target = Some(session);
    Ok(out)
}
fn selected_result(
    s: &mut RuntimeSession,
    c: &OperationContext,
    fresh: bool,
) -> Result<Projection> {
    let summary = access(s, c)?;
    let (capture, comparison, entry) = if s.offline {
        let archive = s.archive.as_mut().ok_or_else(|| {
            error(
                ErrorCode::InternalInvariantViolation,
                "offline archive missing",
            )
        })?;
        let record = archive
            .latest(c)?
            .ok_or_else(|| error(ErrorCode::InvalidRequest, "archive contains no capture"))?;
        (record.observation, None, Some(record.entry))
    } else {
        let reader = s
            .reader
            .as_ref()
            .ok_or_else(|| error(ErrorCode::InternalInvariantViolation, "live source missing"))?;
        let capture = reader
            .current(c)?
            .ok_or_else(|| {
                error(
                    ErrorCode::StaleAnchor,
                    "no current capture; history remains available when archive custody is healthy",
                )
            })?
            .clone();
        let comparison = reader.comparison(c)?.cloned();
        if let Some(archive) = s.archive.as_mut() {
            let entry = s.record.as_ref().ok_or_else(|| {
                error(
                    ErrorCode::CorruptLedger,
                    "current capture lacks a durable reference",
                )
            })?;
            let record = archive.record(entry.number, entry.record_digest, c)?;
            if record.observation != capture {
                return Err(error(
                    ErrorCode::CorruptLedger,
                    "current capture and durable record disagree",
                ));
            }
        }
        (capture, comparison, s.record.clone())
    };
    let reference = entry
        .as_ref()
        .zip(summary.as_ref())
        .map(|(e, a)| history::entry_json(e, a.archive_id));
    let value = json!({"ok":true,"historical":s.offline,"source_read_this_call":fresh,"observation":observation_json(&capture),
        "comparison":comparison.as_ref().map(comparison_json),"archive_record":reference,
        "progress_archive":summary.as_ref().map(history::summary_json),
        "next_step":if s.offline {json!({"tool":"fortress.query","session_id":s.id.to_string(),"history":"{\"mode\":\"list\"}"})}
            else {json!({"tool":"fortress.wait","session_id":s.id.to_string(),"expected_witness":capture.witness().to_string()})}});
    Ok(Projection {
        value,
        capture: Some(capture),
        comparison,
        historical: s.offline,
    })
}
fn refresh(s: &mut RuntimeSession, ids: &[u32], c: &OperationContext) -> Result<()> {
    if s.offline {
        return Err(error(
            ErrorCode::CapabilityDenied,
            "offline recovery cannot acquire native observations or append history",
        ));
    }
    let started = Instant::now();
    access(s, c)?;
    if let Some(archive) = s.archive.as_mut() {
        let mut storage = c.clone();
        storage.budget.max_bytes = storage
            .budget
            .max_bytes
            .checked_sub(progress::RPC_BYTE_RESERVE + MAX_FRAME_BYTES)
            .ok_or_else(|| {
                error(
                    ErrorCode::BudgetExceeded,
                    "native capture, durable append and exact readback do not fit",
                )
            })?;
        archive.reserve_capture(&storage)?;
    }
    let work = remaining(c.clone(), started, c.budget.max_wall_millis)?;
    let reader = s
        .reader
        .as_mut()
        .ok_or_else(|| error(ErrorCode::InternalInvariantViolation, "live source missing"))?;
    let archive = &mut s.archive;
    let mut record = None;
    reader.refresh_with_publication(ids, &work, |manifest, capture, context| {
        runtime_io()?;
        if let Some(archive) = archive.as_mut() {
            let mut storage = context.clone();
            storage.budget.max_bytes = storage
                .budget
                .max_bytes
                .checked_sub(progress::RPC_BYTE_RESERVE + MAX_FRAME_BYTES)
                .ok_or_else(|| {
                    error(
                        ErrorCode::BudgetExceeded,
                        "progress archive allowance exhausted",
                    )
                })?;
            record = Some(archive.append(manifest, capture, &storage)?);
        }
        Ok(())
    })?;
    let capture = reader.current(&work)?.ok_or_else(|| {
        error(
            ErrorCode::InternalInvariantViolation,
            "capture publication missing",
        )
    })?;
    s.anchor.tick = GameTick(s.anchor.tick.get().max(capture.tick()));
    s.anchor.state_hash = capture.witness();
    s.record = record;
    s.ids = ids.to_vec();
    Ok(())
}
fn with_session<F>(
    id: String,
    operation: &str,
    rows: Option<usize>,
    max_wall: Option<u64>,
    body: F,
) -> String
where
    F: FnOnce(&mut RuntimeSession, &OperationContext) -> Result<Projection>,
{
    let started = Instant::now();
    let result = (|| {
        runtime_io()?;
        validate_environment()?;
        let id = parse_session(&id)?;
        let mut guard = slot()?;
        let s = guard.as_mut().filter(|s| s.id == id).ok_or_else(|| {
            error(
                ErrorCode::SessionNotFound,
                "progress session absent or closed",
            )
        })?;
        let mut display = s.context()?;
        if let Some(wall) = max_wall {
            display.budget.max_wall_millis = wall.min(display.budget.max_wall_millis);
        }
        let work = reserve(display.clone(), rows.unwrap_or(s.ids.len()))
            .and_then(|c| remaining(c, started, display.budget.max_wall_millis));
        let outcome = work
            .and_then(|work| {
                access(s, &work)?;
                body(s, &work)
            })
            .and_then(|value| {
                if started.elapsed() >= Duration::from_millis(display.budget.max_wall_millis) {
                    Err(error(
                        ErrorCode::BudgetExceeded,
                        "progress acknowledgement deadline expired; inspect retained history",
                    ))
                } else {
                    Ok(value)
                }
            });
        let projection =
            outcome.unwrap_or_else(|cause| Projection::plain(failure(&cause), s.offline));
        Ok::<_, DfmcpError>(render(operation, projection, s, &display))
    })();
    result.unwrap_or_else(|cause| unbound(operation, &cause))
}

#[tool(
    description = "Open unadmitted progress/1.12. Live mode requires 1..32 native_order_ids and acquires a complete capture. With an operator JOURNAL, sync each capture before publication. recovery_only=true requires an existing nonempty archive, omits IDs, grants only Query and never reads endpoint/token or contacts DFHack. Optional operator WATCHES opens a distinct paired intent book before connecting and exposes restart-safe sampled predicates. No game mutation or creation reconciliation exists."
)]
pub fn fortress_open_session(
    native_order_ids: Option<Vec<u32>>,
    max_wall_millis: Option<u64>,
    max_bytes: Option<u64>,
    max_output_tokens: Option<u32>,
    recovery_only: Option<bool>,
) -> String {
    let started = Instant::now();
    let result = (|| {
        runtime_io()?;
        validate_environment()?;
        let offline = recovery_only.unwrap_or(false);
        let path = archive_path()?;
        let watch_path = watch_path(path.as_deref())?;
        let byte_ceiling = if watch_path.is_some() {
            WATCH_BYTES
        } else {
            MAX_BYTES
        };
        let ids = match (offline, native_order_ids) {
            (true, None) if path.is_some() => Vec::new(),
            (false, Some(ids)) => normalized_ids(ids)?,
            _ => {
                return Err(error(
                    ErrorCode::InvalidRequest,
                    "live mode requires IDs; offline requires JOURNAL and no IDs",
                ));
            }
        };
        let mut guard = slot()?;
        if guard.is_some() {
            return Err(error(
                ErrorCode::Conflict,
                "close the existing progress session first",
            ));
        }
        let raw_fortress = configured(ENVIRONMENT[3], 20)?;
        let value = raw_fortress.parse::<u64>().map_err(|_| {
            error(
                ErrorCode::InvalidRequest,
                "fortress ID must be canonical nonzero decimal",
            )
        })?;
        if value == 0 || value.to_string() != raw_fortress {
            return Err(error(
                ErrorCode::InvalidRequest,
                "fortress ID must be canonical nonzero decimal",
            ));
        }
        let fortress = FortressId::new(value);
        let budget = WorkBudget {
            max_wall_millis: max_wall_millis.unwrap_or(5000),
            max_game_ticks: if watch_path.is_some() {
                MAX_WATCH_HORIZON
            } else {
                0
            },
            max_entities: 4096,
            max_bytes: max_bytes.unwrap_or(if path.is_some() {
                byte_ceiling
            } else {
                TRANSIENT_BYTES
            }),
            max_output_tokens: max_output_tokens.unwrap_or(65_536),
            max_actions: 1,
        };
        budget.validate()?;
        if budget.max_wall_millis > 60_000
            || budget.max_bytes > byte_ceiling
            || budget.max_output_tokens > 131_072
        {
            return Err(error(
                ErrorCode::BudgetExceeded,
                "progress limits exceed 60000ms, the configured archive/watch byte ceiling or 131072 output proxy units",
            ));
        }
        let seq = NEXT
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < (1u64 << 57)).then_some(n + 1)
            })
            .map_err(|_| {
                error(
                    ErrorCode::BudgetExceeded,
                    "progress session identities exhausted",
                )
            })?;
        let id = SessionId::new((1u128 << 127) | FAMILY | u128::from(seq));
        let grants = session_grants(fortress, offline);
        let anchor = StateAnchor {
            fortress_id: fortress,
            cursor: ObservationCursor::ORIGIN,
            tick: GameTick(0),
            state_hash: Digest32::ZERO,
        };
        let context = OperationContext {
            session_id: id,
            request_id: RequestId::new(1),
            anchor,
            budget,
            grants: grants.clone(),
            cancellation_requested: false,
        };
        let mut work = reserve(
            context.clone(),
            if offline {
                progress::MAX_TARGETS
            } else {
                ids.len()
            },
        )?;
        work = remaining(work, started, budget.max_wall_millis)?;
        let mut archive = path
            .as_deref()
            .map(|path| {
                open_progress_archive(
                    path,
                    if offline {
                        ArchiveMode::Offline
                    } else {
                        ArchiveMode::Live
                    },
                    &work,
                )
            })
            .transpose()?;
        if let Some(a) = archive.as_mut() {
            let summary = a.summary(&work)?;
            work.budget.max_bytes = work
                .budget
                .max_bytes
                .checked_sub(summary.retained_bytes + 160)
                .filter(|n| *n >= 2 * MAX_FRAME_BYTES)
                .ok_or_else(|| {
                    error(
                        ErrorCode::BudgetExceeded,
                        "archive replay and bootstrap response do not fit",
                    )
                })?;
            work.anchor.tick = GameTick(summary.authority_tick_floor);
        }
        work = remaining(work, started, budget.max_wall_millis)?;
        runtime_io()?;
        // Verify all durable intent before endpoint/credential reads or acquisition.
        let mut watch_book = if let Some(path) = watch_path.as_deref() {
            let remaining_bytes = work
                .budget
                .max_bytes
                .checked_sub(BOOK_OPEN_RESERVE)
                .filter(|left| {
                    *left
                        >= progress::BOOTSTRAP_BYTE_RESERVE
                            + progress::RPC_BYTE_RESERVE
                            + 2 * MAX_FRAME_BYTES
                })
                .ok_or_else(|| {
                    error(
                        ErrorCode::BudgetExceeded,
                        "watch replay and bootstrap work do not fit",
                    )
                })?;
            let a = archive
                .as_mut()
                .ok_or_else(|| error(ErrorCode::InvalidRequest, "WATCHES requires JOURNAL"))?;
            let book = open_watch_book(
                path,
                if offline {
                    ArchiveMode::Offline
                } else {
                    ArchiveMode::Live
                },
                a,
                &work,
            )?;
            work.budget.max_bytes = remaining_bytes;
            work = remaining(work, started, budget.max_wall_millis)?;
            Some(book)
        } else {
            None
        };
        // This branch intentionally precedes all endpoint/credential reads.
        let mut session = if offline {
            let a = archive.take().ok_or_else(|| {
                error(
                    ErrorCode::InvalidRequest,
                    "offline mode needs a progress archive",
                )
            })?;
            let mut session = offline_session(a, &work, budget)?;
            session.watch_book = watch_book.take();
            session
        } else {
            work.budget.max_bytes = work
                .budget
                .max_bytes
                .checked_sub(progress::BOOTSTRAP_BYTE_RESERVE)
                .filter(|left| *left >= progress::RPC_BYTE_RESERVE + 2 * MAX_FRAME_BYTES)
                .ok_or_else(|| {
                    error(
                        ErrorCode::BudgetExceeded,
                        "native bootstrap and durable read do not fit",
                    )
                })?;
            let endpoint = match std::env::var(ENVIRONMENT[2]) {
                Ok(v) if v.len() <= 128 => v,
                Err(std::env::VarError::NotPresent) => "127.0.0.1:5000".to_owned(),
                _ => {
                    return Err(error(
                        ErrorCode::InvalidRequest,
                        "progress endpoint must be bounded UTF-8 numeric loopback",
                    ));
                }
            }
            .parse::<SocketAddr>()
            .map_err(|_| {
                error(
                    ErrorCode::InvalidRequest,
                    "progress endpoint must be numeric loopback",
                )
            })?;
            let token = configured(ENVIRONMENT[1], 256)?.into_bytes();
            let source = ProgressRpcClient::connect(
                endpoint,
                token,
                id.get().to_be_bytes().to_vec(),
                Duration::from_millis(work.budget.max_wall_millis),
            )?;
            work = remaining(work, started, budget.max_wall_millis)?;
            runtime_io()?;
            let reader = ProgressSession::new(source, &work)?;
            let mut session = RuntimeSession {
                id,
                request: 1,
                anchor: StateAnchor {
                    tick: work.anchor.tick,
                    ..anchor
                },
                budget,
                grants,
                ids: ids.clone(),
                reader: Some(reader),
                archive,
                record: None,
                offline: false,
                cursors: history::Cursors::default(),
                watch_book: watch_book.take(),
            };
            refresh(&mut session, &ids, &work)?;
            session
        };
        work = remaining(work, started, budget.max_wall_millis)?;
        let mut projection = selected_result(&mut session, &work, !offline)?;
        projection.value = json!({"ok":true,"session_id":id.to_string(),"recovery_only":offline,"read_only":true,
            "capabilities":if offline{json!(["query"])}else{json!(["query","observe"])},"capture":projection.value,
            "close":{"tool":"fortress.cancel","session_id":id.to_string()}});
        publish_session(session, projection, &context, started, &mut guard)
    })();
    result.unwrap_or_else(|cause| unbound("fortress.open_session", &cause))
}

#[tool(
    description = "Acquire one complete selected progress capture; optional IDs change selection. When journaling, capacity and output are reserved before acquisition and the complete capture is synced before publication. Offline sessions refuse. No manager-order or clock mutation occurs."
)]
pub fn fortress_observe(session_id: String, native_order_ids: Option<Vec<u32>>) -> String {
    let ids = match native_order_ids.map(normalized_ids).transpose() {
        Ok(v) => v,
        Err(c) => return unbound("fortress.observe", &c),
    };
    with_session(
        session_id,
        "fortress.observe",
        ids.as_ref().map(Vec::len),
        None,
        |s, c| {
            let ids = ids.unwrap_or_else(|| s.ids.clone());
            refresh(s, &ids, c)?;
            selected_result(s, c, true)
        },
    )
}
#[tool(
    description = "Inspect retained progress without native calls. Optional expected_witness binds the current capture. Alternatively history is a <=2048-byte JSON request: mode=list with limit/continuation; mode=record with archive_id,number,record_digest; mode=changes with archive_id,before_number,before_digest,after_number,after_digest. History and expected_witness are mutually exclusive. With operator WATCHES, history also accepts watch_register, watch_list, watch_status and watch_cancel. watch_register requires archive_id,key,native_order_id,goal,deadline_game_tick,cadence_game_ticks,stable_samples,origin_number,origin_digest; remaining_at_most additionally requires threshold. watch_status/cancel require archive_id,key,definition_digest; cancel additionally requires expected_archive_head. Cancellation retires local intent only. Read modes work offline. Exact history remains available after native failure."
)]
pub fn fortress_query(
    session_id: String,
    expected_witness: Option<String>,
    history: Option<String>,
) -> String {
    enum Query {
        History(history::Request),
        Watch(watches::Request),
    }
    let request = match history
        .as_deref()
        .map(|raw| {
            watches::Request::parse(raw)
                .map(Query::Watch)
                .or_else(|_| history::Request::parse(raw).map(Query::History))
        })
        .transpose()
    {
        Ok(request) if request.is_none() || expected_witness.is_none() => request,
        Ok(_) => {
            return unbound(
                "fortress.query",
                &error(
                    ErrorCode::InvalidRequest,
                    "history and current witness cannot be mixed",
                ),
            );
        }
        Err(cause) => return unbound("fortress.query", &cause),
    };
    with_session(
        session_id,
        "fortress.query",
        request.as_ref().map(|_| progress::MAX_TARGETS),
        None,
        |s, c| {
            if let Some(request) = request {
                let archive = s.archive.as_mut().ok_or_else(|| {
                    error(
                        ErrorCode::CapabilityDenied,
                        "this session has no operator-configured progress archive",
                    )
                })?;
                match request {
                    Query::History(request) => {
                        let answer = history::query(archive, &mut s.cursors, request, c)?;
                        return Ok(Projection {
                            value: answer.value,
                            capture: answer.capture,
                            comparison: answer.comparison,
                            historical: true,
                        });
                    }
                    Query::Watch(request) => {
                        let book = s.watch_book.as_mut().ok_or_else(|| {
                            error(
                                ErrorCode::CapabilityDenied,
                                "no operator WATCHES book is configured",
                            )
                        })?;
                        return watches::query(book, archive, request, c)
                            .map(|value| Projection::plain(value, true));
                    }
                }
            }
            let projection = selected_result(s, c, false)?;
            if let Some(w) = expected_witness {
                let expected = parse_digest(&w)?;
                if projection
                    .capture
                    .as_ref()
                    .is_none_or(|o| o.witness() != expected)
                {
                    return Err(error(
                        ErrorCode::StaleAnchor,
                        "progress baseline changed; query current capture first",
                    ));
                }
            }
            Ok(projection)
        },
    )
}
fn check_witness(s: &mut RuntimeSession, c: &OperationContext, raw: &str) -> Result<()> {
    let expected = parse_digest(raw)?;
    let projection = selected_result(s, c, false)?;
    if projection
        .capture
        .as_ref()
        .is_none_or(|o| o.witness() != expected)
    {
        return Err(error(
            ErrorCode::StaleAnchor,
            "progress baseline changed; query current capture first",
        ));
    }
    Ok(())
}
#[tool(
    description = "Perform one foreground refresh from the exact retained witness. Archived sessions sync before publication; offline sessions refuse native work. Reopening creates a new history segment. No polling loop, clock advance, mutation or creation reconciliation exists."
)]
pub fn fortress_wait(
    session_id: String,
    expected_witness: String,
    max_wall_millis: Option<u64>,
) -> String {
    with_session(
        session_id,
        "fortress.wait",
        None,
        max_wall_millis,
        |s, c| {
            if s.offline {
                return Err(error(
                    ErrorCode::CapabilityDenied,
                    "offline progress recovery cannot wait on DFHack",
                ));
            }
            let started = Instant::now();
            check_witness(s, c, &expected_witness)?;
            let mut work = remaining(c.clone(), started, c.budget.max_wall_millis)?;
            if s.archive.is_some() {
                work.budget.max_bytes = work
                    .budget
                    .max_bytes
                    .checked_sub(MAX_FRAME_BYTES)
                    .ok_or_else(|| {
                        error(
                            ErrorCode::BudgetExceeded,
                            "witness recheck and new capture do not fit",
                        )
                    })?;
            }
            let ids = s.ids.clone();
            refresh(s, &ids, &work)?;
            let work = remaining(work, started, c.budget.max_wall_millis)?;
            selected_result(s, &work, true)
        },
    )
}

#[tool(
    description = "Explain retained validation/activity and remaining-work counters without native calls. Inactive does not establish why work is blocked; absence and zero remaining do not certify goods produced."
)]
pub fn fortress_explain(session_id: String) -> String {
    with_session(session_id, "fortress.explain", None, None, |s, c| {
        selected_result(s, c, false)
    })
}
#[tool(
    description = "Release this read-only progress session and its native connection. This also works after source failure or environment revocation; it never cancels or deletes manager orders."
)]
pub fn fortress_cancel(session_id: String) -> String {
    let result = (|| {
        let id = parse_session(&session_id)?;
        let mut guard = slot()?;
        if guard.as_ref().is_none_or(|s| s.id != id) {
            return Err(error(
                ErrorCode::SessionNotFound,
                "progress session absent or already closed",
            ));
        }
        drop(guard.take());
        Ok::<_, DfmcpError>(packet(
            "fortress.cancel",
            json!({"ok":true,"closed":true,"orders_cancelled":false,"watches_cancelled":false,"game_mutation_dispatched":false}),
            None,
            None,
            None,
        ))
    })();
    result.unwrap_or_else(|cause| unbound("fortress.cancel", &cause))
}
fn denied(session_id: String, operation: &str) -> String {
    with_session(session_id, operation, None, None, |_, _| {
        Err(error(
            ErrorCode::CapabilityDenied,
            "progress/1.12 is strictly read-only; use the separately authorized creation runtime for work-order intent",
        ))
    })
}
#[tool(description = "Unavailable: progress/1.12 never prepares effects.")]
pub fn fortress_plan(session_id: String) -> String {
    denied(session_id, "fortress.plan")
}
#[tool(description = "Unavailable: progress/1.12 has no mutation RPC.")]
pub fn fortress_commit(session_id: String) -> String {
    denied(session_id, "fortress.commit")
}
#[tool(description = "Unavailable: progress observations are not game checkpoints.")]
pub fn fortress_checkpoint(session_id: String) -> String {
    denied(session_id, "fortress.checkpoint")
}
#[tool(description = "Unavailable: progress/1.12 cannot restore game state.")]
pub fn fortress_restore(session_id: String) -> String {
    denied(session_id, "fortress.restore")
}
#[tool(
    description = "Inspect local progress-session state without probing the native bridge. This is not compatibility admission, connection-health proof or a game-state refresh."
)]
pub fn fortress_doctor(session_id: String) -> String {
    with_session(session_id, "fortress.doctor", None, None, |s, c| {
        let summary = access(s, c)?;
        let available = if s.offline {
            s.archive
                .as_mut()
                .map(|a| a.latest(c))
                .transpose()?
                .flatten()
                .is_some()
        } else {
            s.reader
                .as_ref()
                .map(|r| r.current(c).map(|o| o.is_some()))
                .transpose()?
                .unwrap_or(false)
        };
        Ok(Projection::plain(
            json!({"ok":true,"runtime_admitted":false,"native_calls":0,
            "selected_order_ids":s.ids,"capture_available":available,"read_only":true,"recovery_only":s.offline,
            "bridge_connection_present":s.reader.is_some(),"progress_archive":summary.as_ref().map(history::summary_json)}),
            s.offline,
        ))
    })
}
pub fn run_stdio() {
    if let Err(cause) = validate_environment() {
        eprintln!("{cause}");
        std::process::exit(1);
    }
    let server=ServerBuilder::new("dfmcp-live-work-order-progress-dev",env!("CARGO_PKG_VERSION"))
        .tool(FortressOpenSession).tool(FortressObserve).tool(FortressQuery).tool(FortressPlan).tool(FortressCommit)
        .tool(FortressWait).tool(FortressCancel).tool(FortressCheckpoint).tool(FortressRestore).tool(FortressExplain).tool(FortressDoctor)
        .instructions("Unadmitted read-only progress/1.12. Select native manager-order IDs, inspect approval/activity/counters, and wait for one new bounded capture using its exact witness. No background polling or clock control. Disappearance, zero remaining or decreasing counters do not prove completed goods or resolve historical creation effects. Comparisons are between sampled endpoints, not continuous history. Unknown configuration is explicit. Optional operator JOURNAL retains complete captures before publication. Open recovery_only=true with no IDs to inspect existing history without DFHack credentials or connection. Query history list/record/changes using exact archive references; do not compare across restart segments. Source failure preserves healthy archive access. Optional operator WATCHES durably retains bounded approval/activity/counter predicates and cancellation. Use query history watch_register/list/status/cancel. Outcomes are derived from every archived sample; new segments retire unfinished watches as continuity_lost. A satisfied observation is not goods production. Session close never cancels watches. Creation and its durable journal remain a separate authorized profile.")
        .build();
    crate::run_modern_stdio(server);
}

#[cfg(test)]
#[path = "live_work_order_progress_server_tests.rs"]
mod tests;
