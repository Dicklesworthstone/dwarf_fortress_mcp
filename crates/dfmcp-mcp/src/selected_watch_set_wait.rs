//! One exact laboratory watch set inside the shared foreground wait owner.
//! Every selected watch keeps its original definition and temporal state machine.
//! One detailed candidate publication commits the whole set or none of it.

use super::*;
use crate::agent_turn::{AgentPhase, AgentTurnBuilder, ContinuityStatus, ObservationProfile};
use serde_json::Value;
use std::time::{Duration, Instant};

const MAX_WATCHES: usize = 8;
const MAX_SAMPLE_ROUNDS: u64 = 4_097;

fn invalid(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::InvalidRequest, message)
}

fn exhausted(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::BudgetExceeded, message)
}

#[derive(Clone, Copy)]
enum WatchPolicy {
    AllSatisfied,
    AnySatisfied,
}

impl WatchPolicy {
    fn parse(value: Option<&str>) -> Result<Self> {
        match value {
            None | Some("all_satisfied") => Ok(Self::AllSatisfied),
            Some("any_satisfied") => Ok(Self::AnySatisfied),
            Some(_) => Err(invalid("watch_policy must be all_satisfied or any_satisfied")),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::AllSatisfied => "all_satisfied",
            Self::AnySatisfied => "any_satisfied",
        }
    }
}

struct OriginalWatch {
    definition: Value,
    created_at: Value,
    initial_samples: u64,
}

struct SampleWork {
    rounds: u64,
    samples_added: u64,
    inspected_records: u64,
    inspected_bytes: u64,
}

pub(super) struct SelectedWatchSetWait {
    handles: Vec<String>,
    policy: WatchPolicy,
    basis: StateAnchor,
    started: Instant,
    originals: Vec<OriginalWatch>,
    records: Vec<Value>,
    rounds: u64,
    samples_added: u64,
    inspected_records: u64,
    inspected_bytes: u64,
    stop: Option<&'static str>,
    triggered: Vec<String>,
    published: Option<String>,
}

impl SelectedWatchSetWait {
    pub(super) fn new(
        session: &LabSession,
        mut handles: Vec<String>,
        policy: Option<String>,
        requested_ticks: u64,
        started: Instant,
    ) -> Result<Self> {
        let policy = WatchPolicy::parse(policy.as_deref())?;
        if handles.is_empty() || handles.len() > MAX_WATCHES {
            return Err(invalid("watch_ids must contain one to eight explicit handles"));
        }
        handles.sort();
        if handles.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(invalid("watch_ids must be unique"));
        }
        let context = read_context(session);
        context.budget.validate()?;
        authorize_entry(&context, Capability::Observe, RiskTier::ReadOnly)?;
        authorize_entry(&context, Capability::Query, RiskTier::ReadOnly)?;
        let records = semantic_query::laboratory_watch_records(&context, &handles)?;
        if records.len() != handles.len() {
            return Err(invalid("selected watch metadata does not cover the exact set"));
        }
        let mut originals = Vec::with_capacity(handles.len());
        for (handle, record) in handles.iter().zip(&records) {
            if record["watch"] != *handle
                || !record["definition"].is_object()
                || !record["created_at"].is_object()
            {
                return Err(invalid("selected watch has no complete original identity"));
            }
            status(record)?;
            originals.push(OriginalWatch {
                definition: record["definition"].clone(),
                created_at: record["created_at"].clone(),
                initial_samples: record["sample_count"].as_u64()
                    .ok_or_else(|| invalid("selected watch has no original sample count"))?,
            });
        }
        if requested_ticks > 0 && !session.adapter.snapshot().paused {
            let mut final_context = context.clone();
            final_context.anchor.tick = GameTick(context.anchor.tick.0.checked_add(requested_ticks)
                .ok_or_else(|| exhausted("selected watch set exceeds the game-time horizon"))?);
            authorize_entry(&final_context, Capability::Query, RiskTier::ReadOnly)?;
        }
        let selected = Self {
            handles,
            policy,
            basis: context.anchor,
            started,
            originals,
            records,
            rounds: 0,
            samples_added: 0,
            inspected_records: 0,
            inspected_bytes: 0,
            stop: None,
            triggered: Vec::new(),
            published: None,
        };
        selected.check(session)?;
        Ok(selected)
    }

    pub(super) fn check(&self, session: &LabSession) -> Result<()> {
        let context = read_context(session);
        authorize_entry(&context, Capability::Observe, RiskTier::ReadOnly)?;
        authorize_entry(&context, Capability::Query, RiskTier::ReadOnly)?;
        if self.started.elapsed() >= Duration::from_millis(context.budget.max_wall_millis) {
            return Err(exhausted("selected watch set exhausted its shared wall-time budget"));
        }
        Ok(())
    }

    fn remaining_context(&self, session: &LabSession) -> Result<OperationContext> {
        self.check(session)?;
        let mut context = read_context(session);
        let spent = self.started.elapsed().as_nanos().div_ceil(1_000_000);
        if spent >= u128::from(context.budget.max_wall_millis) {
            return Err(exhausted("selected watch set has no remaining inspection budget"));
        }
        context.budget.max_wall_millis -= spent as u64;
        Ok(context)
    }

    pub(super) fn stopped(&self) -> bool {
        self.stop.is_some()
    }

    pub(super) fn ceiling(&self, now: u64, other: Option<u64>) -> Option<u64> {
        let mut ceiling = other;
        for record in self.records.iter().filter(|record| record["terminal"] == false) {
            for field in ["next_sample_tick", "deadline_tick"] {
                if let Some(tick) = record[field].as_u64().filter(|tick| *tick > now) {
                    let distance = tick - now;
                    ceiling = Some(ceiling.map_or(distance, |existing| existing.min(distance)));
                }
            }
        }
        ceiling
    }

    pub(super) fn projection(&self, session: &LabSession, evaluation_complete: bool) -> Value {
        let mut records = self.records.clone();
        if !evaluation_complete {
            let current = anchor_json(&session.adapter.snapshot().anchor());
            for record in &mut records {
                record["evaluation_current"] = json!(false);
                record["needs_poll"] = json!(
                    record["terminal"] == false && record["last_evaluated_anchor"] != current
                );
            }
        }
        let context = read_context(session);
        self.outcome(
            &records, self.stop, &self.triggered, evaluation_complete,
            SampleWork {
                rounds: self.rounds,
                samples_added: self.samples_added,
                inspected_records: self.inspected_records,
                inspected_bytes: self.inspected_bytes,
            },
            &context,
        )
    }

    fn outcome(
        &self,
        records: &[Value],
        stop: Option<&'static str>,
        triggered: &[String],
        evaluation_complete: bool,
        work: SampleWork,
        context: &OperationContext,
    ) -> Value {
        let satisfied = records.iter().filter(|record| record["status"] == "satisfied").count();
        let terminal = records.iter().filter(|record| record["terminal"] == true).count();
        json!({
            "schema":"dfmcp.selected-watch-set-wait/1",
            "watch_ids":self.handles,
            "watch_policy":self.policy.name(),
            "records":records,
            "stop_reason":stop,
            "triggered_watch_ids":triggered,
            "group_satisfied":evaluation_complete
                && matches!(stop, Some("all_satisfied" | "any_satisfied")),
            "evaluation_complete":evaluation_complete,
            "original_definitions_unchanged":true,
            "selection_scope":"exact_owned_watch_set",
            "selected":records.len(),
            "satisfied":satisfied,
            "all_satisfied":!records.is_empty() && satisfied == records.len(),
            "any_satisfied":satisfied > 0,
            "all_terminal":!records.is_empty() && terminal == records.len(),
            "sampling_rounds":work.rounds,
            "samples_added":work.samples_added,
            "source_work":{
                "inspected_records":work.inspected_records,
                "canonical_bytes":work.inspected_bytes,
                "maximum_source_records":context.budget.max_entities,
                "maximum_source_bytes":context.budget.max_bytes,
                "size_limit_scope":"each_complete_observed_source",
                "maximum_inspections":MAX_SAMPLE_ROUNDS,
                "maximum_selected_watches":MAX_WATCHES,
                "total_work_bound":"shared_wall_deadline_and_bounded_foreground_slices",
            },
            "atomic_watch_publication":true,
            "completion_scope":"retained_watch_outcomes",
            "terminal_evidence_is_historical":true,
            "simultaneous_current_truth_proven":false,
            "continuous_between_observations":false,
            "evidence_policy":"laboratory_reference/1",
            "durable":false,
            "game_effect_success_proven":false,
            "mutation_authority":false,
        })
    }

    pub(super) fn sample(&mut self, session: &LabSession, base: Value) -> Result<()> {
        self.sample_with_finalizer(session, base, |packet| Ok(packet))
    }

    #[cfg(test)]
    pub(super) fn refuse_sample_publication(
        &mut self,
        session: &LabSession,
        base: Value,
    ) -> Result<()> {
        self.sample_with_finalizer(session, base, |_| {
            Err(exhausted("injected refusal after complete watch-set publication rendering"))
        })
    }

    fn sample_with_finalizer<F>(
        &mut self,
        session: &LabSession,
        base: Value,
        finalize: F,
    ) -> Result<()>
    where
        F: FnOnce(String) -> Result<String>,
    {
        let current = self.remaining_context(session)?;
        let rounds = self.rounds.checked_add(1).filter(|rounds| *rounds <= MAX_SAMPLE_ROUNDS)
            .ok_or_else(|| exhausted("selected watch set reached its inspection limit"))?;
        let source = query_budget::admit_sources(&[session.adapter.snapshot()], &current)?;
        let inspected_records = self.inspected_records.checked_add(source["records"].as_u64()
            .ok_or_else(|| invalid("selected watch set source has no record count"))?)
            .ok_or_else(|| exhausted("selected watch set record accounting overflow"))?;
        let inspected_bytes = self.inspected_bytes.checked_add(source["canonical_bytes"].as_u64()
            .ok_or_else(|| invalid("selected watch set source has no byte count"))?)
            .ok_or_else(|| exhausted("selected watch set byte accounting overflow"))?;
        // Acquire the other retained store and Task metadata before the watch
        // engine owns its candidate. Its publication callback is lock-free.
        let baselines = temporal_query::retained_baseline_work(&self.remaining_context(session)?)?;
        let view = temporal_handler::projection(session, &current)?;
        let mut active_work = view.active_work;
        active_work["query_baselines"] = baselines;
        active_work["mcp_tasks"] = crate::task_service::session_handles(&view.session_id);
        active_work["mcp_task_coverage"] = crate::task_service::session_handle_coverage(&view.session_id);
        let context = self.remaining_context(session)?;
        let mut accepted = None;
        let encoded = semantic_query::execute_laboratory_watch_set_with_publisher(
            session.adapter.snapshot(), &context, &self.handles,
            |candidate| {
                if candidate["record_detail"] != "complete_definition_and_evidence"
                    || candidate["atomic_watch_publication"] != true
                    || candidate["anchor"] != anchor_json(&context.anchor)
                {
                    return Err(invalid("selected watch set has no complete atomic candidate"));
                }
                let records = candidate["records"].as_array()
                    .filter(|records| records.len() == self.handles.len())
                    .cloned()
                    .ok_or_else(|| invalid("candidate differs from the exact selected watch set"))?;
                let mut samples_added = 0u64;
                for ((handle, original), record) in self.handles.iter().zip(&self.originals).zip(&records) {
                    if record["watch"] != *handle
                        || record["definition"] != original.definition
                        || record["created_at"] != original.created_at
                    {
                        return Err(DfmcpError::new(
                            ErrorCode::Conflict,
                            "selected watch definition or membership changed during foreground wait",
                        ));
                    }
                    let added = record["sample_count"].as_u64()
                        .and_then(|count| count.checked_sub(original.initial_samples))
                        .ok_or_else(|| invalid("selected watch sample count moved backwards"))?;
                    samples_added = samples_added.checked_add(added)
                        .ok_or_else(|| exhausted("selected watch set sample accounting overflow"))?;
                }
                let tick_limit = base["wait_progress"]["remaining_game_ticks"].as_u64()
                    .ok_or_else(|| invalid("selected watch set lacks its remaining tick allowance"))? == 0;
                let (stop, triggered) = stop_reason(
                    &records, self.policy, session.adapter.snapshot().paused, tick_limit,
                )?;
                let catalogs = temporal_query::candidate_watch_work(&candidate, &context)?;
                active_work["query_watches"] = catalogs["query_watches"].clone();
                active_work["condition_watches"] = catalogs["condition_watches"].clone();
                let mut payload = base;
                payload["selected_watch_set_wait"] = self.outcome(
                    &records, stop, &triggered, true,
                    SampleWork { rounds, samples_added, inspected_records, inspected_bytes },
                    &context,
                );
                let rendered = render(
                    &context, self.basis, &view.session_id, view.objectives, active_work, payload,
                )?;
                let rendered = finalize(rendered)?;
                self.check(session)?;
                accepted = Some((records, stop, triggered, samples_added));
                Ok(rendered)
            },
        )?;
        // No fallible work follows the atomic selected-set root swap.
        if let Some((records, stop, triggered, samples_added)) = accepted {
            self.records = records;
            self.stop = stop;
            self.triggered = triggered;
            self.samples_added = samples_added;
        }
        self.rounds = rounds;
        self.inspected_records = inspected_records;
        self.inspected_bytes = inspected_bytes;
        self.published = Some(encoded);
        Ok(())
    }

    pub(super) fn take_published(&mut self) -> Option<String> {
        self.published.take()
    }
}

fn status(record: &Value) -> Result<&str> {
    match record["status"].as_str() {
        Some(value @ ("waiting" | "candidate" | "blocked_unknown" | "satisfied"
            | "failed" | "expired" | "invalidated" | "cancelled")) => Ok(value),
        _ => Err(invalid("selected watch set contains an unknown status")),
    }
}

fn stop_reason(
    records: &[Value],
    policy: WatchPolicy,
    paused: bool,
    tick_limit: bool,
) -> Result<(Option<&'static str>, Vec<String>)> {
    for record in records {
        status(record)?;
    }
    // Attention dominates a simultaneous or historical group success. Canonical
    // priority and handle ordering cannot depend on caller selection order.
    for attention in ["failed", "expired", "invalidated", "cancelled", "blocked_unknown"] {
        let triggered = records.iter().filter(|record| record["status"] == attention)
            .filter_map(|record| record["watch"].as_str().map(str::to_owned))
            .collect::<Vec<_>>();
        if !triggered.is_empty() {
            return Ok((Some(attention), triggered));
        }
    }
    let satisfied = records.iter().filter(|record| record["status"] == "satisfied")
        .filter_map(|record| record["watch"].as_str().map(str::to_owned))
        .collect::<Vec<_>>();
    let complete = match policy {
        WatchPolicy::AllSatisfied => !records.is_empty() && satisfied.len() == records.len(),
        WatchPolicy::AnySatisfied => !satisfied.is_empty(),
    };
    if complete {
        return Ok((Some(policy.name()), satisfied));
    }
    Ok((if paused { Some("paused") } else if tick_limit { Some("tick_limit") } else { None }, Vec::new()))
}

fn render(
    context: &OperationContext,
    basis: StateAnchor,
    session_id: &str,
    objectives: Value,
    active_work: Value,
    mut payload: Value,
) -> Result<String> {
    let maximum = context.budget.max_bytes.min(u64::from(context.budget.max_output_tokens) * 4);
    payload["request_id"] = json!(context.request_id.to_string());
    let selected = payload["selected_watch_set_wait"].clone();
    let references = selected["records"].as_array().map(|records| records.iter().map(|record| {
        json!({"kind":"condition_watch","watch":record["watch"],
            "evidence_digest":record["evidence_digest"],"anchor":record["last_evaluated_anchor"]})
    }).collect::<Vec<_>>()).unwrap_or_default();
    let packet = AgentTurnBuilder::new("fortress.wait", AgentPhase::Verify)
        .session_id(session_id.to_owned()).request_id(context.request_id.to_string())
        .turn_id(format!("watch-set-wait-{}", context.request_id))
        .anchor(anchor_json(&context.anchor))
        .continuity(ContinuityStatus::Partial, Some(anchor_json(&basis)), None, None)
        .profile(ObservationProfile::Tactical)
        .briefing(json!({
            "runtime":"laboratory_reference_world","adapter":"memory",
            "paused":payload["paused"],"objectives":objectives,
            "selected_watches":selected["watch_ids"],"watch_policy":selected["watch_policy"],
            "stop_reason":selected["stop_reason"],"watch_storage":"bounded_process_local",
            "completion_scope":"retained_watch_outcomes","live_game_qualification":false,
        }))
        .active_work(active_work)
        .coverage(json!({
            "status":"partial",
            "complete_domains":["laboratory.source_anchor",
                "laboratory.session_query_baseline_catalog","laboratory.session_query_watch_catalog"],
            "partial_domains":[{"domain":"laboratory.selected_watch_set_samples",
                "reason":"one atomic set per published frontier; internal cursor gaps reset stability and terminal outcomes stay historical"}],
            "omitted_domains":["live_dwarf_fortress_state"],"absence_proof_scope":[],
            "simultaneous_current_truth_proven":false,
            "continuous_between_observations":false,"eligible_as_mutation_evidence":false,
        }))
        .budget(json!({
            "requested":null,"admitted":{
                "maximum_response_bytes":maximum,"max_output_tokens":context.budget.max_output_tokens,
                "max_game_ticks":context.budget.max_game_ticks,"max_wall_millis":context.budget.max_wall_millis,
            },
            "consumed":{"canonical_source":selected["source_work"],
                "game_ticks":payload["wait_progress"]["advanced_game_ticks"],"native_captures":0},
            "remaining":null,"soft_stop_reason":selected["stop_reason"],"hard_stop_reason":null,
        }))
        .references(references)
        .attach(payload);
    if packet.len() as u64 > maximum {
        return Err(exhausted("complete selected watch set, game work and Agent Turn exceed the response budget"));
    }
    Ok(packet)
}
