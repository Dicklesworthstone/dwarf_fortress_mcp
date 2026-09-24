//! Archive-only spatial/1.8 sessions. The only retained source is a verified
//! read-only observation journal; there is no credential or connection to revive.
#[path = "spatial_archive_queries.rs"]
mod queries;
pub(super) use queries::query;

use super::*;
use dfmcp_adapter::operations_journal::{
    JournalEntry, JournalLimits, PrivateJournalFile, Spatial18, TailRecovery,
};
use dfmcp_core::{Digest32, FortressId, GameTick, ObservationCursor};
use std::path::Path;

struct ArchiveSource {
    fenced: bool,
}
impl Source for ArchiveSource {
    fn read(&mut self, _: Duration) -> Result<LiveSpatialCitizenObservation> {
        Err(error(
            ErrorCode::CapabilityDenied,
            "archive-only sessions cannot acquire live observations",
        ))
    }
    fn poisoned(&self) -> bool {
        self.fenced
    }
    fn fence(&mut self) {
        self.fenced = true;
    }
    fn pages(&self) -> u32 {
        0
    }
    fn archive_only(&self) -> bool {
        true
    }
}

pub(super) fn requested_capabilities(input: Option<Vec<String>>) -> Result<Vec<Capability>> {
    let names = input.unwrap_or_else(|| vec!["query".into(), "doctor".into()]);
    let caps = capabilities(Some(names))?;
    if !caps.contains(&Capability::Query) || caps.contains(&Capability::Observe) {
        return Err(error(
            ErrorCode::CapabilityDenied,
            "archive-only sessions require Query and optionally Doctor; Observe is not available",
        ));
    }
    Ok(caps)
}

pub(super) fn validate_configuration(
    path: Option<&Path>,
    repair: TailRecovery,
    watch_path: Option<&Path>,
) -> Result<()> {
    if path.is_none() {
        return Err(error(
            ErrorCode::InvalidRequest,
            "archive-only sessions require DFMCP_SPATIAL_CITIZEN_JOURNAL",
        ));
    }
    if repair != TailRecovery::Refuse || watch_path.is_some() {
        return Err(error(
            ErrorCode::CapabilityDenied,
            "archive-only sessions refuse repair and watch recovery; unset journal repair and DFMCP_SPATIAL_CITIZEN_WATCH_JOURNAL",
        ));
    }
    Ok(())
}

fn grants(caps: &[Capability], fortress: Option<FortressId>) -> Vec<CapabilityGrant> {
    caps.iter()
        .map(|capability| CapabilityGrant {
            capability: *capability,
            scope: CapabilityScope {
                fortress_id: fortress,
                ..CapabilityScope::default()
            },
            max_risk: RiskTier::ReadOnly,
            expires_at_tick: None,
            remaining_uses: None,
        })
        .collect()
}

pub(super) fn validate_limits(
    value: &LiveSpatialCitizenObservation,
    limits: CitizenSpatialLimits,
) -> Result<()> {
    let op = value.spatial().operations();
    let counts = limits.spatial.operations;
    if op.jobs.jobs.len() > counts.jobs as usize
        || op.buildings.len() > counts.buildings as usize
        || op.items.len() > counts.items as usize
        || value.citizens().len() > limits.citizens as usize
        || value.spatial().terrain().map.region != limits.spatial.region
        || value.encode_payload()?.len() > counts.payload_bytes
    {
        return Err(error(
            ErrorCode::BudgetExceeded,
            "archived spatial/1.8 capture exceeds the requested acquisition bounds or region",
        ));
    }
    Ok(())
}

pub(super) fn open(
    id: SessionId,
    slot: Slot,
    path: &Path,
    limits: CitizenSpatialLimits,
    budget: WorkBudget,
    caps: &[Capability],
) -> Result<Session> {
    if !caps.contains(&Capability::Query)
        || caps
            .iter()
            .any(|cap| !matches!(cap, Capability::Query | Capability::Doctor))
    {
        return Err(error(
            ErrorCode::CapabilityDenied,
            "archive bootstrap cannot carry live observation or effect authority",
        ));
    }
    budget.validate()?;
    limits.validate()?;
    let mut replay_budget = budget;
    replay_budget.max_bytes = limits.spatial.operations.payload_bytes as u64;
    let bootstrap = OperationContext {
        session_id: id,
        request_id: RequestId::new(1),
        anchor: StateAnchor {
            fortress_id: FortressId::NIL,
            cursor: ObservationCursor::ORIGIN,
            tick: GameTick(0),
            state_hash: Digest32::ZERO,
        },
        budget: replay_budget,
        grants: grants(caps, None),
        cancellation_requested: false,
    };
    // No endpoint/token environment lookup occurs on this branch. The verified
    // file header supplies identity, not authority or a claim of current time.
    let journal =
        PrivateJournalFile::open_recovery::<Spatial18>(path, &bootstrap, JournalLimits::default())?;
    let value = journal.state().observation_full().ok_or_else(|| {
        error(
            ErrorCode::CursorGap,
            "archive contains no complete observations; cannot bootstrap a historical world",
        )
    })?;
    validate_limits(value, limits)?;
    let state = journal.state().clone();
    let fortress = state
        .snapshot()
        .ok_or_else(|| error(ErrorCode::CorruptLedger, "archive projection absent"))?
        .fortress_id;
    Ok(Session {
        id,
        source: Box::new(ArchiveSource { fenced: false }),
        state,
        limits,
        journal: Some(journal),
        budget,
        grants: grants(caps, Some(fortress)),
        request: 0,
        _watch_journal: None,
        _slot: slot,
    })
}

pub(super) fn validate(session: &mut Session, context: &OperationContext) -> Result<()> {
    if !session.source.archive_only() {
        return Ok(());
    }
    let journal = session
        .journal
        .as_mut()
        .ok_or_else(|| error(ErrorCode::CorruptLedger, "archive session lost its journal"))?;
    journal.validate_custody(context)?;
    if !journal.recovery_only()
        || journal.state().snapshot().map(|s| s.anchor()) != Some(context.anchor)
    {
        session.source.fence();
        return Err(error(
            ErrorCode::CorruptLedger,
            "archive session does not match its read-only journal head",
        ));
    }
    Ok(())
}

pub(super) fn entry_json(entry: &JournalEntry) -> Value {
    json!({"record":entry.number,"anchor":anchor_json(entry.anchor),"source_digest":entry.source_digest.to_string(),
        "record_digest":entry.record_digest.to_string(),"previous_digest":entry.previous_digest.to_string(),"encoded_bytes":entry.encoded_bytes})
}

/// All archive results, including zero matches and diagnostics, carry a
/// historical boundary. Empty active work is scoped to this archive-only session;
/// no claim is made that persisted monitoring or game actions do not exist.
pub(super) fn packet(
    session: &Session,
    context: &OperationContext,
    operation: &str,
    selected: Option<&JournalEntry>,
    mut value: Value,
) -> Result<String> {
    let journal = session
        .journal
        .as_ref()
        .ok_or_else(|| error(ErrorCode::CorruptLedger, "archive journal absent"))?;
    let latest = journal
        .entries()
        .last()
        .ok_or_else(|| error(ErrorCode::CursorGap, "archive has no retained observations"))?;
    let entry = selected.unwrap_or(latest);
    if !value.is_object() {
        return Err(error(
            ErrorCode::InternalInvariantViolation,
            "archive response must be an object",
        ));
    }
    if value.get("ok").is_none() {
        value["ok"] = json!(true);
    }
    value["session_id"] = json!(session.id.to_string());
    value["anchor"] = anchor_json(entry.anchor);
    value["archive_only"] = json!(true);
    value["historical"] = json!(true);
    value["live"] = json!(false);
    value["current_freshness_proven"] = json!(false);
    value["native_captures"] = json!(0);
    value["bridge_connection_present"] = json!(false);
    value["mutation_dispatched"] = json!(false);
    value["journal_record"] = entry_json(entry);
    let continuation = value.get("continuation").cloned().unwrap_or(Value::Null);
    let out=AgentTurnBuilder::new(operation,AgentPhase::Inspect)
        .session_id(session.id.to_string()).request_id(context.request_id.to_string()).anchor(anchor_json(entry.anchor))
        .continuity(ContinuityStatus::Partial,None,Some(json!({"reason":"archive_evidence_is_not_current_game_state"})),None)
        .briefing(json!({"runtime":"unadmitted_development","bridge_protocol":"1.8","archive_only":true,
            "runtime_admitted":false,"mutation_admissible":false,"read_only":true,"live":false,
            "bridge_connection_present":false,"current_freshness_proven":false,
            "observation_archive":{"journal_id":journal.id().to_string(),"head":journal.head().to_string(),
                "records":journal.entries().len(),"latest_record":latest.number,"retained_bytes":journal.retained_bytes(),"fenced":journal.fenced()},
            "watch_evidence_loaded":false,"active_work_scope":"this_archive_session_only"}))
        .coverage(json!({"status":"partial","complete_domains":["selected_archived_capture"],
            "omitted_domains":["current_game_state","continuous_game_history","persisted_watch_evidence","game_effects"],
            "temporal_coverage":"retained_observation_endpoints_only","current_freshness_proven":false,"continuation":continuation}))
        .uncertainty(vec![json!({"epistemic_state":"unknown","statement":"Archived observations do not establish the current fortress state or events between captures."})])
        .references(vec![json!({"kind":"verified_archived_spatial_capture","journal_id":journal.id().to_string(),
            "record_digest":entry.record_digest.to_string(),"source_digest":entry.source_digest.to_string()})])
        .attach(value);
    let maximum = context
        .budget
        .max_bytes
        .min(u64::from(context.budget.max_output_tokens) * 4);
    if out.len() as u64 > maximum {
        return Err(error(
            ErrorCode::BudgetExceeded,
            "complete historical Agent Turn exceeds response budget",
        ));
    }
    Ok(out)
}

pub(super) fn result_context(
    session: &Session,
    context: &OperationContext,
    entry: &JournalEntry,
) -> Result<OperationContext> {
    let overhead =
        packet(session, context, "fortress.query", Some(entry), json!({}))?.len() as u64 + 128;
    let mut result = context.clone();
    result.budget.max_bytes = context
        .budget
        .max_bytes
        .min(u64::from(context.budget.max_output_tokens) * 4)
        .checked_sub(overhead)
        .filter(|n| *n > 0)
        .ok_or_else(|| {
            error(
                ErrorCode::BudgetExceeded,
                "archive metadata leaves no query budget",
            )
        })?;
    result.anchor = entry.anchor;
    Ok(result)
}
