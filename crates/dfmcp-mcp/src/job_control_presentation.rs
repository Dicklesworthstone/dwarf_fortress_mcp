//! Bounded, authority-free presentation and opaque recovery continuations.

use std::collections::{BTreeMap, VecDeque};
use dfmcp_adapter::job_suspension::JobObservation;
use dfmcp_adapter::job_suspension::coordinator::{DurableJobRecord, DurableJobState, JobJournalSummary};
use dfmcp_core::{DfmcpError, Digest32, ErrorCode, OperationContext, Result, SessionId};
use serde_json::{Value, json};
use crate::{AgentPhase, AgentTurnBuilder, ContinuityStatus, ObservationProfile, empty_active_work};

pub(super) const BASE_RESERVE: u64 = 12 * 1024;
pub(super) const RECORD_RESERVE: u64 = 12 * 1024;
pub(super) const MAX_PAGE: usize = 16;

pub(super) fn error(code: ErrorCode, message: &str) -> DfmcpError {
    DfmcpError::new(code, message)
}

pub(super) fn digest(text: &str) -> Result<Digest32> {
    if text.len() != 64 || !text.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
        return Err(error(ErrorCode::InvalidRequest, "expected a canonical lowercase SHA-256 digest"));
    }
    let mut bytes = [0; 32];
    for (index, pair) in text.as_bytes().chunks_exact(2).enumerate() {
        let pair = std::str::from_utf8(pair).map_err(|_| error(ErrorCode::InvalidRequest, "invalid digest"))?;
        bytes[index] = u8::from_str_radix(pair, 16).map_err(|_| error(ErrorCode::InvalidRequest, "invalid digest"))?;
    }
    Ok(Digest32::from_bytes(bytes))
}

pub(super) fn state_name(state: DurableJobState) -> &'static str {
    match state {
        DurableJobState::Prepared => "prepared",
        DurableJobState::DispatchStarted => "dispatch_started",
        DurableJobState::Indeterminate => "indeterminate",
        DurableJobState::Applied => "applied",
        DurableJobState::NotApplied => "not_applied",
        DurableJobState::Refused => "refused",
        DurableJobState::CancelledBeforeDispatch => "cancelled_before_dispatch",
    }
}

pub(super) fn observation_json(o: &JobObservation) -> Value {
    json!({"domain":"selected_native_job", "canonical_world_snapshot":false,
        "native_job_id":o.job_id(), "next_job_id":o.next_job_id(),
        "bridge_generation":o.generation(), "intervention_sequence":o.sequence(),
        "game_tick":o.tick(), "witness":o.witness().to_string(),
        "world_folder":o.world_folder(), "site_id":o.site_id(),
        "native_job_type":o.job_type(), "type_key":o.type_key(), "reaction":o.reaction(),
        "holder_id":o.holder_id(), "native_holder_type":o.holder_type(),
        "worker_id":o.worker_id(), "position":o.position(),
        "completion_timer":o.completion_timer(), "attachment_count":o.attachment_count(),
        "filter_count":o.filter_count(), "suspended":o.suspended(), "repeating":o.repeating(),
        "paused":o.paused(), "holder_complete":o.holder_complete(),
        "production_holder":o.production_holder(), "supported":o.supported(),
        "eligible":o.eligible(), "eligibility_is_authority":false})
}

pub(super) fn record_json(record: &DurableJobRecord) -> Value {
    let plan = record.plan(); let effect = record.effect();
    json!({"idempotency_key":plan.key(), "plan_digest":plan.digest().to_string(),
        "expected_witness":plan.observation().witness().to_string(),
        "desired_suspended":plan.desired(), "state":state_name(record.state()),
        "observation":observation_json(plan.observation()),
        "source":{"bridge_protocol":"1.9", "df_version":record.manifest().df_version,
            "dfhack_version":record.manifest().dfhack_version},
        "native_state":match effect.state() {
            dfmcp_adapter::job_suspension::SuspensionState::Prepared => "prepared",
            dfmcp_adapter::job_suspension::SuspensionState::Unknown => "unknown",
            dfmcp_adapter::job_suspension::SuspensionState::Applied => "applied",
            dfmcp_adapter::job_suspension::SuspensionState::NotApplied => "not_applied",
            dfmcp_adapter::job_suspension::SuspensionState::Refused => "refused"},
        "observed_suspended":effect.observed_suspended(),
        "after_witness":effect.after_witness().map(|d|d.to_string()),
        "receipt_digest":effect.receipt().map(|d|d.to_string()),
        "reconciliation_required":record.state().reconciliation_required(),
        "safe_to_redispatch":false, "current_live_state_proven":false,
        "production_goal_completion_proven":false})
}

pub(super) fn summary_json(s: &JobJournalSummary) -> Value {
    json!({"fortress_id":s.fortress_id.to_string(), "journal_id":s.journal_id.to_string(),
        "head":s.head.to_string(), "transitions":s.transitions, "retained_bytes":s.retained_bytes,
        "records":s.records, "prepared":s.prepared, "reconciliation_required":s.unresolved,
        "terminal":s.terminal, "read_only":s.read_only, "custody_checked":true,
        "evidence_domain":"durable_job_coordination", "current_live_state_proven":false})
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Filter { All, Pending, Reconciliation }
impl Filter {
    pub(super) fn parse(value: &str) -> Result<Self> {
        match value {
            "all" => Ok(Self::All), "pending" => Ok(Self::Pending),
            "reconciliation_required" => Ok(Self::Reconciliation),
            _ => Err(error(ErrorCode::InvalidRequest,
                "job state filter must be all, pending, or reconciliation_required")),
        }
    }
    pub(super) fn name(self) -> &'static str {
        match self { Self::All => "all", Self::Pending => "pending", Self::Reconciliation => "reconciliation_required" }
    }
    pub(super) fn matches(self, state: DurableJobState) -> bool {
        match self { Self::All => true, Self::Pending => !state.terminal(), Self::Reconciliation => state.reconciliation_required() }
    }
    pub(super) fn total(self, summary: &JobJournalSummary) -> usize {
        match self { Self::All => summary.records, Self::Pending => summary.prepared + summary.unresolved,
            Self::Reconciliation => summary.unresolved }
    }
}

#[derive(Clone)]
struct Cursor {
    session: SessionId, journal: Digest32, head: Digest32,
    after: String, filter: Filter, limit: usize,
}
/// A checksum is not authority. Accept only a token actually issued in this
/// session, retaining up to 64 cursors for replay after a lost page response.
#[derive(Default)]
pub(super) struct Continuations { issued: BTreeMap<String, Cursor>, order: VecDeque<String> }
impl Continuations {
    pub(super) fn issue(&mut self, session: SessionId, summary: &JobJournalSummary,
        after: String, filter: Filter, limit: usize) -> String
    {
        let mut bytes = b"dfmcp-job-mcp-page/1\0".to_vec();
        bytes.extend_from_slice(&session.get().to_be_bytes());
        bytes.extend_from_slice(summary.journal_id.as_bytes()); bytes.extend_from_slice(summary.head.as_bytes());
        bytes.extend_from_slice(&(limit as u32).to_be_bytes());
        bytes.extend_from_slice(filter.name().as_bytes()); bytes.push(0); bytes.extend_from_slice(after.as_bytes());
        let token = Digest32::of_bytes(&bytes).to_string();
        if !self.issued.contains_key(&token) {
            if self.order.len() == 64 {
                if let Some(old) = self.order.pop_front() { self.issued.remove(&old); }
            }
            self.order.push_back(token.clone());
            self.issued.insert(token.clone(), Cursor { session, journal: summary.journal_id,
                head: summary.head, after, filter, limit });
        }
        token
    }
    pub(super) fn resolve(&self, token: &str, session: SessionId, summary: &JobJournalSummary,
        filter: Filter, limit: usize) -> Result<String>
    {
        digest(token)?;
        let cursor = self.issued.get(token).ok_or_else(|| error(ErrorCode::InvalidRequest,
            "job continuation was not issued here or has expired; restart discovery"))?;
        if cursor.session != session || cursor.journal != summary.journal_id
            || cursor.filter != filter || cursor.limit != limit
        {
            return Err(error(ErrorCode::Conflict, "job continuation belongs to another session, journal, filter, or limit"));
        }
        if cursor.head != summary.head {
            return Err(error(ErrorCode::StaleAnchor, "job journal changed; restart discovery without a continuation"));
        }
        Ok(cursor.after.clone())
    }
}

pub(super) fn failure(cause: &DfmcpError, operation: &str) -> Value {
    let uncertain = cause.code == ErrorCode::EffectIndeterminate || operation == "fortress.commit";
    json!({"ok":false, "error":{"code":cause.code.as_str(), "message":cause.message,
        "recovery_class":if uncertain {"reconciliation_required"} else {"refresh_and_retry"},
        "effect_may_have_occurred":uncertain, "safe_to_redispatch":false,
        "next_step":if uncertain {"Inspect the durable key with fortress.explain; close and reopen in reconcile mode if the source or journal is fenced."}
            else {"Inspect fortress.doctor and the reported journal; refresh the selection or restart discovery as required."}}})
}

pub(super) struct TurnView<'a> {
    pub context: Option<&'a OperationContext>,
    pub mode: &'a str,
    pub summary: Option<&'a JobJournalSummary>,
    pub selected: Option<&'a JobObservation>,
}

pub(super) fn packet(operation: &str, result: Value, view: TurnView<'_>) -> String {
    let phase = match operation {
        "fortress.open_session" => AgentPhase::Bootstrap,
        "fortress.plan" => AgentPhase::Propose,
        "fortress.commit" | "fortress.cancel" => AgentPhase::Commit,
        "fortress.wait" => AgentPhase::Reconcile,
        _ => AgentPhase::Inspect,
    };
    let mut active = empty_active_work();
    // Counts are complete for the journal, not a falsely empty list of work.
    active["counts"] = view.summary.map_or(Value::Null, |s| json!({
        "prepared":s.prepared,"reconciliation_required":s.unresolved,"terminal":s.terminal}));
    active["details_omitted"] = json!(view.summary.is_none_or(|s|s.prepared + s.unresolved != 0));
    active["discovery"] = json!({"tool":"fortress.query", "state":"pending",
        "session_id":view.context.map(|c|c.session_id.to_string()), "limit":8});
    if result.get("closed").and_then(Value::as_bool) == Some(true) {
        active["discovery"] = json!({"tool":"fortress.open_session","mode":"offline"});
    }
    let mut builder = AgentTurnBuilder::new(operation, phase)
        .profile(ObservationProfile::Forensic)
        .continuity(if view.summary.is_some_and(|s|s.unresolved != 0) {
            ContinuityStatus::Indeterminate } else { ContinuityStatus::Partial }, None, None, None)
        .briefing(json!({"runtime":"unadmitted_development", "bridge_protocol":"1.9",
            "runtime_admitted":false, "mutation_admissible":false,
            "development_mutation_enabled":view.mode == "control" && view.context.is_some_and(|c|
                c.authorize(dfmcp_core::Capability::ConfigureProduction, dfmcp_core::RiskTier::Reversible, &[], None).is_ok()),
            "mode":view.mode,
            "durable_job_journal":view.summary.map(summary_json),
            "selected_job_evidence_retained":view.selected.is_some(),
            "canonical_world_anchor_available":false}))
        .active_work(active)
        .coverage(json!({"status":"partial", "complete_domains":if view.summary.is_some() {
            json!(["retained_job_journal_counts"]) } else { json!([]) },
            "partial_domains":["selected_native_job","retained_job_effect_records"],
            "omitted_domains":["canonical_world_state","other_jobs","production_goal_completion",
                "work_order_creation","labor_assignment","checkpoint","restore"]}))
        .uncertainty(vec![json!({"code":"unadmitted_selected_job_scope",
            "detail":"Job observations and journal receipts are not a canonical world snapshot, current live-state proof, or completed production goal."})]);
    if let Some(c) = view.context {
        builder = builder.session_id(c.session_id.to_string()).request_id(c.request_id.to_string())
            .budget(json!({"admitted":{"max_wall_millis":c.budget.max_wall_millis,
                "max_bytes":c.budget.max_bytes,"max_output_tokens":c.budget.max_output_tokens},
                "output_token_accounting":"four_byte_proxy","consumed":{},"remaining":null}));
    }
    if let Some(o) = view.selected {
        builder = builder.anchor(json!({"domain":"selected_native_job",
            "fortress_id":view.context.map(|c|c.anchor.fortress_id.to_string()),
            "native_job_id":o.job_id(), "bridge_generation":o.generation(),
            "intervention_sequence":o.sequence(), "game_tick":o.tick(),
            "witness":o.witness().to_string(), "canonical_world_anchor":false}));
    }
    let mut turn = builder.build();
    // The common builder decorates admitted callers. Even a direct invocation
    // rejected inside such a process must never export that provenance here.
    if let Some(briefing) = turn.get_mut("briefing").and_then(Value::as_object_mut) {
        briefing.remove("admission");
    }
    json!({"result":result,"agent_turn":turn}).to_string()
}

#[cfg(test)]
#[path = "job_control_presentation_tests.rs"]
mod tests;
