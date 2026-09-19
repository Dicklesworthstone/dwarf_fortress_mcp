//! Authority-free, bounded projection for isolated work-orders/1.10.
use std::collections::BTreeMap;
use dfmcp_adapter::work_order_control::{CreationRecord, CreationState, CreationSummary, JournalMode};
use dfmcp_adapter::work_orders::{WorkOrderObservation, WorkOrderRecipe, WorkOrderState};
use dfmcp_core::{Capability, DfmcpError, Digest32, ErrorCode, OperationContext, Result, RiskTier, SessionId};
use serde_json::{Value, json};
use crate::agent_turn::{AgentPhase, AgentTurnBuilder, ContinuityStatus};

pub(super) const BASE_RESERVE: u64 = 16 * 1024;
pub(super) const RECORD_RESERVE: u64 = 64 * 1024;
pub(super) const MAX_PAGE: usize = 8;
pub(super) fn error(code: ErrorCode, text: &str) -> DfmcpError { DfmcpError::new(code, text) }
pub(super) fn digest(raw: &str) -> Result<Digest32> {
    if raw.len() != 64 || !raw.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
        return Err(error(ErrorCode::InvalidRequest, "expected canonical lowercase SHA-256 hex"));
    }
    let mut bytes = [0; 32];
    for (i, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&raw[2*i..2*i+2], 16)
            .map_err(|_| error(ErrorCode::InvalidRequest, "invalid SHA-256 hex"))?;
    }
    Ok(Digest32::from_bytes(bytes))
}
pub(super) fn recipe(raw: &str) -> Result<WorkOrderRecipe> {
    match raw {
        "wooden_bed" => Ok(WorkOrderRecipe::WoodenBed), "wooden_door" => Ok(WorkOrderRecipe::WoodenDoor),
        "wooden_table" => Ok(WorkOrderRecipe::WoodenTable), "wooden_chair" => Ok(WorkOrderRecipe::WoodenChair),
        _ => Err(error(ErrorCode::InvalidRequest, "unknown finite furniture recipe")),
    }
}
pub(super) fn mode_name(mode: JournalMode) -> &'static str {
    match mode { JournalMode::Control => "control", JournalMode::Reconcile => "reconcile", JournalMode::Offline => "offline" }
}
pub(super) fn observation_json(o: &WorkOrderObservation) -> Value {
    json!({"kind":"native_order_queue_membership", "fortress_id":o.fortress_id().to_string(),
        "bridge_generation":o.generation(), "intervention_sequence":o.sequence(), "game_tick":o.tick(),
        "world_folder":o.world_folder(), "site_id":o.site_id(), "paused":o.paused(),
        "next_order_id":o.next_order_id(), "order_ids":o.order_ids(), "witness":o.witness().to_string(),
        "eligible_at_observation":o.eligible(), "complete_queue_membership":true,
        "existing_order_configurations_observed":false, "production_feasibility_proven":false})
}
pub(super) fn record_json(record: &CreationRecord) -> Value {
    let plan = record.plan(); let effect = record.effect();
    let native_state = match effect.state() {
        WorkOrderState::Prepared => "prepared", WorkOrderState::Unknown => "unknown",
        WorkOrderState::Created => "created", WorkOrderState::Refused => "refused",
    };
    json!({"idempotency_key":plan.key(), "plan_digest":plan.digest().to_string(),
        "recipe":plan.spec().recipe().as_str(), "amount":plan.spec().amount(),
        "state":record.state().as_str(), "original_observation":observation_json(plan.observation()),
        "native_state":native_state, "created_order_id":effect.created_order_id(),
        "observed_tick":effect.observed_tick(), "after_witness":effect.after_witness().map(|d|d.to_string()),
        "configuration_witness":effect.configuration_witness().map(|d|d.to_string()),
        "receipt":effect.receipt().map(|d|d.to_string()),
        "native_effect_hex":effect.canonical_bytes().iter().map(|b|format!("{b:02x}")).collect::<String>(),
        "source":{"df_version":record.manifest().df_version,"dfhack_version":record.manifest().dfhack_version,
            "generation":record.manifest().generation,"protocol":"1.10"},
        "reconciliation_required":record.state().unresolved(), "safe_to_retry_insertion":false,
        "production_goal_completion_proven":false, "current_freshness_proven":false})
}
pub(super) fn summary_json(s: &CreationSummary) -> Value {
    json!({"journal_id":s.journal_id.to_string(),"head":s.head.to_string(),"fortress_id":s.fortress_id.to_string(),
        "records":s.records,"prepared":s.prepared,"unresolved":s.unresolved,"terminal":s.terminal,
        "transitions":s.transitions,"retained_bytes":s.retained_bytes,"mode":mode_name(s.mode),
        "restart_recovery":true,"scope":"creation_coordination_not_game_history"})
}
pub(super) fn failure(cause: &DfmcpError, operation: &str) -> Value {
    let uncertain = cause.code == ErrorCode::EffectIndeterminate;
    json!({"ok":false,"error":{"code":cause.code.as_str(),"message":cause.message,
        "retryable":false,"effect_may_have_occurred":if cause.code == ErrorCode::CorruptLedger { Value::Null } else { json!(uncertain) },
        "recovery_class":if uncertain {"reconciliation_required"} else if cause.code == ErrorCode::CorruptLedger {
            "operator_action_required"} else {"never_unchanged"}},
        "operation":operation,"safe_to_retry_insertion":false,"production_goal_completion_proven":false})
}
pub(super) struct TurnView<'a> {
    pub context: Option<&'a OperationContext>, pub mode: Option<JournalMode>,
    pub summary: Option<&'a CreationSummary>, pub selected: Option<&'a WorkOrderObservation>,
}
pub(super) fn packet(operation: &str, result: Value, view: TurnView<'_>) -> String {
    let phase = match operation {
        "fortress.open_session" => AgentPhase::Bootstrap, "fortress.observe" => AgentPhase::Orient,
        "fortress.plan" => AgentPhase::Propose, "fortress.commit" => AgentPhase::Commit,
        "fortress.wait" => AgentPhase::Reconcile, _ => AgentPhase::Inspect,
    };
    let production = view.mode == Some(JournalMode::Control) && view.context.is_some_and(|c|
        c.authorize(Capability::ConfigureProduction, RiskTier::Reversible, &[], None).is_ok());
    let mut work = crate::agent_turn::empty_active_work();
    work["scope"] = json!("this_creation_journal_only");
    work["state_known"] = json!(view.summary.is_some());
    let mut recommendations = Vec::new();
    if let Some(s) = view.summary {
        work["prepared_count"] = json!(s.prepared); work["unresolved_count"] = json!(s.unresolved);
        work["terminal_count"] = json!(s.terminal); work["details_omitted"] = json!(s.prepared + s.unresolved);
        let arguments = json!({"state":"pending","limit":2,
            "session_id":view.context.map(|c|c.session_id.to_string())});
        let discovery = json!({"tool":"fortress.query","arguments":arguments});
        work["discovery"] = discovery.clone();
        if s.prepared != 0 { work["pending_plans"] = json!([{"retained_count":s.prepared,"discover":discovery}]); }
        if s.unresolved != 0 { work["indeterminate_effects"] = json!([{"retained_count":s.unresolved,"discover":discovery}]); }
        if s.prepared + s.unresolved != 0 {
            recommendations.push(json!({"tool":"fortress.query","arguments":arguments,
                "reason":"discover retained work before new creation; unresolved attempts cannot be retried"}));
        }
    }
    let mut turn = AgentTurnBuilder::new(operation, phase)
        .continuity(if view.summary.is_some_and(|s|s.unresolved != 0) { ContinuityStatus::Indeterminate }
            else { ContinuityStatus::Partial }, None, None, None)
        .briefing(json!({"runtime":"unadmitted_development","bridge_protocol":"1.10",
            "mode":view.mode.map(mode_name),"runtime_admitted":false,"mutation_admissible":false,
            "development_production_granted":production,"current_freshness_proven":false,
            "native_created_is_not_production_completion":true}))
        .active_work(work).recommendations(recommendations)
        .affordances(vec![json!({"action":"work_order.create","enabled":production
            && view.selected.is_some_and(|o|o.eligible()) && view.summary.is_some_and(|s|s.unresolved == 0),
            "recipes":["wooden_bed","wooden_door","wooden_table","wooden_chair"],"amount_min":1,"amount_max":100,
            "requires":"current exact queue observation, sealed plan and independent commit authorization"})])
        .coverage(json!({"status":"partial","complete_domains":[],
            "partial_domains":["retained_creation_coordination","selected_order_queue_membership"],
            "omitted_domains":["production_progress","order_approval","material_availability","world_state","other_controllers"]}))
        .uncertainty(vec![json!({"code":"development_evidence_only",
            "detail":"No admitted runtime, current world snapshot, completed production or other-controller safety is established."}),
            json!({"code":"retained_work_scope","detail":"Absent summary means custody or authority is unverified, not that no work exists."})]);
    if let Some(c) = view.context {
        turn = turn.session_id(c.session_id.to_string()).request_id(c.request_id.to_string())
            .budget(json!({"admitted":{"max_bytes":c.budget.max_bytes,
                "max_output_tokens":c.budget.max_output_tokens,"max_wall_millis":c.budget.max_wall_millis},
                "token_accounting":"four_byte_proxy_not_tokenizer_count"}));
    }
    if let Some(o) = view.selected {
        turn = turn.anchor(json!({"kind":"selected_native_queue_not_canonical_world",
            "fortress_id":o.fortress_id().to_string(),"bridge_generation":o.generation(),
            "intervention_sequence":o.sequence(),"game_tick":o.tick(),"witness":o.witness().to_string()}));
    }
    let mut value = turn.build();
    // The shared builder attaches ambient production provenance. Even a refusal
    // from this isolated profile must not inherit that provenance or claim admission.
    if let Some(briefing) = value.get_mut("briefing").and_then(Value::as_object_mut) {
        briefing.remove("admission");
    }
    json!({"agent_turn":value,"result":result}).to_string()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Filter { All, Pending, Unresolved }
impl Filter {
    pub fn parse(raw: &str) -> Result<Self> {
        match raw { "all" => Ok(Self::All), "pending" => Ok(Self::Pending),
            "reconciliation_required" => Ok(Self::Unresolved),
            _ => Err(error(ErrorCode::InvalidRequest,"creation state must be all, pending or reconciliation_required")) }
    }
    pub fn name(self) -> &'static str {
        match self { Self::All => "all", Self::Pending => "pending", Self::Unresolved => "reconciliation_required" }
    }
    pub fn matches(self, state: CreationState) -> bool {
        match self { Self::All => true, Self::Pending => !state.terminal(), Self::Unresolved => state.unresolved() }
    }
    pub fn total(self, s: &CreationSummary) -> usize {
        match self { Self::All => s.records, Self::Pending => s.prepared+s.unresolved, Self::Unresolved => s.unresolved }
    }
}
struct Cursor {
    token: String, session: SessionId, journal: Digest32, head: Digest32,
    after: String, filter: Filter, limit: usize,
}
#[derive(Default)]
pub(super) struct Continuations { serial: u64, issued: BTreeMap<u64, Cursor> }
impl Continuations {
    pub fn resolve(&self, token: &str, session: SessionId, summary: &CreationSummary,
        filter: Filter, limit: usize) -> Result<String>
    {
        digest(token)?;
        let cursor = self.issued.values().find(|c|c.token == token)
            .ok_or_else(||error(ErrorCode::StaleAnchor,"creation continuation expired or was not issued; restart discovery"))?;
        if cursor.session != session || cursor.journal != summary.journal_id || cursor.head != summary.head
            || cursor.filter != filter || cursor.limit != limit
        { return Err(error(ErrorCode::StaleAnchor,"creation continuation scope or journal head changed")); }
        Ok(cursor.after.clone())
    }
    pub fn issue(&mut self, session: SessionId, summary: &CreationSummary,
        after: String, filter: Filter, limit: usize) -> Result<String>
    {
        self.serial = self.serial.checked_add(1)
            .ok_or_else(||error(ErrorCode::BudgetExceeded,"creation continuation identities exhausted"))?;
        let mut bytes = b"dfmcp-creation-continuation/1\0".to_vec();
        bytes.extend_from_slice(&session.get().to_be_bytes()); bytes.extend_from_slice(&self.serial.to_be_bytes());
        bytes.extend_from_slice(summary.journal_id.as_bytes()); bytes.extend_from_slice(summary.head.as_bytes());
        bytes.extend_from_slice(filter.name().as_bytes()); bytes.extend_from_slice(&(limit as u64).to_be_bytes());
        bytes.extend_from_slice(after.as_bytes());
        let token = Digest32::of_bytes(&bytes).to_string();
        if self.issued.len() == 64 { self.issued.pop_first(); }
        self.issued.insert(self.serial, Cursor { token:token.clone(),session,journal:summary.journal_id,
            head:summary.head,after,filter,limit });
        Ok(token)
    }
}

#[cfg(test)]
#[path = "work_order_presentation_tests.rs"]
mod tests;
