//! One exact retained laboratory condition in the shared foreground wait loop.
//! The existing watch state machine owns predicate evidence. This owner clips
//! simulation frontiers and accepts a complete Wait packet before that machine
//! publishes a new root. It never changes a condition or synthesizes a sample.

use super::*;
use crate::agent_turn::{AgentPhase, AgentTurnBuilder, ContinuityStatus, ObservationProfile};
use serde_json::Value;
use std::time::{Duration, Instant};

fn invalid(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::InvalidRequest, message)
}

fn exhausted(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::BudgetExceeded, message)
}

// One admission sample plus every possible bounded foreground slice.
const MAX_SAMPLE_ROUNDS: u64 = 4_097;

pub(super) struct SelectedWatchWait {
    handle: String,
    basis: StateAnchor,
    started: Instant,
    definition: Option<(Value, Value)>,
    record: Option<Value>,
    initial_samples: Option<u64>,
    rounds: u64,
    inspected_records: u64,
    inspected_bytes: u64,
    stop: Option<&'static str>,
    published: Option<String>,
}

impl SelectedWatchWait {
    pub(super) fn new(
        session: &LabSession,
        handle: String,
        requested_ticks: u64,
        started: Instant,
    ) -> Result<Self> {
        let hash = handle.strip_prefix("watch:").ok_or_else(|| invalid("invalid watch_id"))?;
        if hash.len() != 64
            || !hash.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(invalid("watch_id must be one complete retained watch handle"));
        }
        let context = read_context(session);
        context.budget.validate()?;
        authorize_entry(&context, Capability::Observe, RiskTier::ReadOnly)?;
        authorize_entry(&context, Capability::Query, RiskTier::ReadOnly)?;
        let record = semantic_query::laboratory_watch_record(&context, &handle)?;
        let initial_samples = record["sample_count"].as_u64()
            .ok_or_else(|| invalid("selected watch has no original sample count"))?;
        let definition = (record["definition"].clone(),record["created_at"].clone());
        if requested_ticks > 0 && !session.adapter.snapshot().paused {
            let mut final_context = context.clone();
            final_context.anchor.tick = GameTick(context.anchor.tick.0.checked_add(requested_ticks)
                .ok_or_else(|| exhausted("selected watch wait exceeds the game-time horizon"))?);
            authorize_entry(&final_context, Capability::Query, RiskTier::ReadOnly)?;
        }
        let selected = Self {
            handle, basis: context.anchor, started, definition: Some(definition), record: Some(record),
            initial_samples: Some(initial_samples), rounds: 0, inspected_records: 0, inspected_bytes: 0,
            stop: None, published: None,
        };
        selected.check(session)?;
        Ok(selected)
    }

    pub(super) fn check(&self, session: &LabSession) -> Result<()> {
        let context = read_context(session);
        authorize_entry(&context, Capability::Observe, RiskTier::ReadOnly)?;
        authorize_entry(&context, Capability::Query, RiskTier::ReadOnly)?;
        if self.started.elapsed() >= Duration::from_millis(context.budget.max_wall_millis) {
            return Err(exhausted("selected watch wait exhausted its shared wall-time budget"));
        }
        Ok(())
    }

    fn remaining_context(&self, session: &LabSession) -> Result<OperationContext> {
        self.check(session)?;
        let mut context = read_context(session);
        let spent = self.started.elapsed().as_nanos().div_ceil(1_000_000);
        if spent >= u128::from(context.budget.max_wall_millis) {
            return Err(exhausted("selected watch wait has no remaining inspection budget"));
        }
        context.budget.max_wall_millis -= spent as u64;
        Ok(context)
    }

    pub(super) fn stopped(&self) -> bool {
        self.stop.is_some()
    }

    pub(super) fn ceiling(&self, now: u64, other: Option<u64>) -> Option<u64> {
        let mut ceiling = other;
        if let Some(record) = &self.record {
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
        let samples = self.record.as_ref().and_then(|record| record["sample_count"].as_u64());
        let mut record = self.record.clone();
        if !evaluation_complete && let Some(record) = &mut record {
            // A later published world may differ from the last accepted watch
            // sample. Never attach its cached "current" bit to that new anchor.
            record["evaluation_current"] = json!(false);
            record["needs_poll"] = json!(
                record["terminal"] == false
                    && record["last_evaluated_anchor"] != anchor_json(&session.adapter.snapshot().anchor())
            );
        }
        json!({
            "schema":"dfmcp.selected-watch-wait/1", "watch_id":self.handle,
            "record":record, "stop_reason":self.stop,
            "watch_satisfied":evaluation_complete && self.stop == Some("satisfied"),
            "evaluation_complete":evaluation_complete,
            "original_definition_unchanged":self.definition.is_some(),
            "sampling_rounds":self.rounds,
            "samples_added":samples.zip(self.initial_samples).map(|(now,before)|now.saturating_sub(before)),
            "source_work":{"inspected_records":self.inspected_records,
                "canonical_bytes":self.inspected_bytes,
                "size_limit_scope":"each_complete_observed_source",
                "maximum_inspections":MAX_SAMPLE_ROUNDS,
                "total_work_bound":"shared_wall_deadline_and_bounded_foreground_slices"},
            "evidence_policy":"laboratory_reference/1", "durable":false,
            "terminal_evidence_is_historical":true, "continuous_between_observations":false,
            "game_effect_success_proven":false, "mutation_authority":false,
        })
    }

    /// Sample only the exact current published world. The complete candidate
    /// packet is accepted under the watch lock before its retained root changes.
    pub(super) fn sample(&mut self, session: &LabSession, base: Value) -> Result<()> {
        self.sample_with_finalizer(session, base, |packet| Ok(packet))
    }

    #[cfg(test)]
    pub(super) fn refuse_sample_publication(&mut self, session: &LabSession, base: Value) -> Result<()> {
        self.sample_with_finalizer(session, base, |_| {
            Err(exhausted("injected refusal after complete watch publication rendering"))
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
            .ok_or_else(|| exhausted("selected watch wait reached its inspection limit"))?;
        let source = query_budget::admit_sources(&[session.adapter.snapshot()], &current)?;
        let inspected_records = self.inspected_records.checked_add(source["records"].as_u64()
            .ok_or_else(|| invalid("selected watch source has no record count"))?)
            .ok_or_else(|| exhausted("selected watch source record accounting overflow"))?;
        let inspected_bytes = self.inspected_bytes.checked_add(source["canonical_bytes"].as_u64()
            .ok_or_else(|| invalid("selected watch source has no byte count"))?)
            .ok_or_else(|| exhausted("selected watch source byte accounting overflow"))?;
        // Take metadata before entering the watch publisher: no recursive lock
        // or session acquisition is permitted while its candidate is held.
        let baselines = temporal_query::retained_baseline_work(&self.remaining_context(session)?)?;
        let view = temporal_handler::projection(session, &current)?;
        let mut active_work = view.active_work;
        active_work["query_baselines"] = baselines;
        active_work["mcp_tasks"] = crate::task_service::session_handles(&view.session_id);
        active_work["mcp_task_coverage"] = crate::task_service::session_handle_coverage(&view.session_id);
        let context = self.remaining_context(session)?;
        let mut accepted_record = None;
        let mut accepted_stop = None;
        let mut initial_samples = self.initial_samples;
        let encoded = semantic_query::execute_laboratory_watch_with_publisher(
            session.adapter.snapshot(), &context,
            &json!({"schema":"dfmcp.query/1","query":{"kind":"poll_watch","watch":self.handle}}),
            |candidate| {
                let record = candidate["record"].clone();
                if record["watch"] != self.handle || !record["definition"].is_object()
                    || !record["created_at"].is_object()
                {
                    return Err(invalid("selected watch lost its immutable original identity"));
                }
                if self.definition.as_ref().is_some_and(|(definition, created)| {
                    &record["definition"] != definition || &record["created_at"] != created
                }) {
                    return Err(DfmcpError::new(ErrorCode::Conflict,
                        "selected watch definition changed during its foreground wait"));
                }
                let samples = record["sample_count"].as_u64()
                    .ok_or_else(|| invalid("selected watch has no bounded sample count"))?;
                let first = *initial_samples.get_or_insert(samples);
                let stop = stop_reason(&record, session.adapter.snapshot().paused,
                    base["wait_progress"]["remaining_game_ticks"] == 0)?;
                let catalogs = temporal_query::candidate_watch_work(&candidate, &context)?;
                active_work["query_watches"] = catalogs["query_watches"].clone();
                active_work["condition_watches"] = catalogs["condition_watches"].clone();
                let mut payload = base;
                payload["selected_watch_wait"] = json!({
                    "schema":"dfmcp.selected-watch-wait/1", "watch_id":self.handle,
                    "record":record, "stop_reason":stop,
                    "watch_satisfied":stop == Some("satisfied"), "evaluation_complete":true,
                    "original_definition_unchanged":true, "sampling_rounds":rounds,
                    "samples_added":samples.saturating_sub(first),
                    "source_work":{"inspected_records":inspected_records,"canonical_bytes":inspected_bytes,
                        "maximum_source_records":context.budget.max_entities,
                        "maximum_source_bytes":context.budget.max_bytes,
                        "size_limit_scope":"each_complete_observed_source",
                        "maximum_inspections":MAX_SAMPLE_ROUNDS,
                        "total_work_bound":"shared_wall_deadline_and_bounded_foreground_slices"},
                    "evidence_policy":"laboratory_reference/1", "durable":false,
                    "terminal_evidence_is_historical":true, "continuous_between_observations":false,
                    "game_effect_success_proven":false, "mutation_authority":false,
                });
                let rendered = render(&context, self.basis, &view.session_id, view.objectives,
                    active_work, payload)?;
                let rendered = finalize(rendered)?;
                self.check(session)?;
                accepted_record = Some(record);
                accepted_stop = stop;
                Ok(rendered)
            },
        )?;
        // All fallible publication checks preceded the watch root swap.
        if let Some(record) = accepted_record {
            if self.definition.is_none() {
                self.definition = Some((record["definition"].clone(),record["created_at"].clone()));
            }
            self.record = Some(record);
        }
        self.initial_samples = initial_samples;
        self.stop = accepted_stop;
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

fn stop_reason(record: &Value, paused: bool, tick_limit: bool) -> Result<Option<&'static str>> {
    let stop = match record["status"].as_str() {
        Some("satisfied") => Some("satisfied"),
        Some("failed") => Some("failed"),
        Some("expired") => Some("expired"),
        Some("invalidated") => Some("invalidated"),
        Some("cancelled") => Some("cancelled"),
        Some("blocked_unknown") => Some("blocked_unknown"),
        Some("waiting" | "candidate") if paused => Some("paused"),
        Some("waiting" | "candidate") if tick_limit => Some("tick_limit"),
        Some("waiting" | "candidate") => None,
        _ => return Err(invalid("selected watch has an unknown status")),
    };
    Ok(stop)
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
    let selected = payload["selected_watch_wait"].clone();
    let packet = AgentTurnBuilder::new("fortress.wait", AgentPhase::Verify)
        .session_id(session_id.to_owned()).request_id(context.request_id.to_string())
        .turn_id(format!("watch-wait-{}",context.request_id))
        .anchor(anchor_json(&context.anchor))
        .continuity(ContinuityStatus::Partial, Some(anchor_json(&basis)), None, None)
        .profile(ObservationProfile::Tactical)
        .briefing(json!({"runtime":"laboratory_reference_world","adapter":"memory",
            "paused":payload["paused"],"objectives":objectives,
            "selected_watch":selected["watch_id"],"stop_reason":selected["stop_reason"],
            "watch_storage":"bounded_process_local","live_game_qualification":false}))
        .active_work(active_work)
        .coverage(json!({"status":"partial","complete_domains":["laboratory.source_anchor",
            "laboratory.session_query_baseline_catalog","laboratory.session_query_watch_catalog"],
            "partial_domains":[{"domain":"laboratory.selected_watch_samples",
                "reason":"samples cover published frontiers only; internal cursor gaps reset positive stability"}],
            "omitted_domains":["live_dwarf_fortress_state"],"absence_proof_scope":[],
            "continuous_between_observations":false,"eligible_as_mutation_evidence":false}))
        .budget(json!({"requested":null,"admitted":{
            "maximum_response_bytes":maximum,"max_output_tokens":context.budget.max_output_tokens,
            "max_game_ticks":context.budget.max_game_ticks,"max_wall_millis":context.budget.max_wall_millis},
            "consumed":{"canonical_source":selected["source_work"],
                "game_ticks":payload["wait_progress"]["advanced_game_ticks"],"native_captures":0},
            "remaining":null,"soft_stop_reason":selected["stop_reason"],"hard_stop_reason":null}))
        .references(vec![json!({"kind":"condition_watch","watch":selected["watch_id"],
            "evidence_digest":selected["record"]["evidence_digest"],
            "anchor":selected["record"]["last_evaluated_anchor"]})])
        .attach(payload);
    if packet.len() as u64 > maximum {
        return Err(exhausted("complete selected watch wait, game work and Agent Turn exceed the response budget"));
    }
    Ok(packet)
}
