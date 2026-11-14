//! Meaningful outcomes for an explicit set of original laboratory projects.
//!
//! Every member is inspected at one shared frontier. Complete source and output
//! admission precede any exact-endpoint write; independent durable endpoint
//! records keep their successful prefix if a later publication fails.

use super::*;
use crate::agent_turn::{AgentPhase, AgentTurnBuilder, ContinuityStatus, ObservationProfile};
use selected_plan_wait::SelectedPlanWait;
use serde_json::Value;
use std::time::{Duration, Instant};

const MAX_PLANS: usize = 8;
const MAX_INSPECTIONS: u64 = 4_097;

fn invalid(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::InvalidRequest, message)
}

fn exhausted(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::BudgetExceeded, message)
}

#[derive(Clone, Copy)]
enum PlanPolicy {
    All,
    Any,
}

impl PlanPolicy {
    fn parse(value: Option<&str>) -> Result<Self> {
        match value {
            None | Some("all") => Ok(Self::All),
            Some("any") => Ok(Self::Any),
            Some(_) => Err(invalid("plan_policy must be all or any")),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Any => "any",
        }
    }

    fn complete_reason(self) -> &'static str {
        match self {
            Self::All => "all_complete",
            Self::Any => "any_complete",
        }
    }
}

pub(super) struct SelectedPlanSetWait {
    members: Vec<SelectedPlanWait>,
    policy: PlanPolicy,
    basis: StateAnchor,
    started: Instant,
    inspections: u64,
    inspected_records: u64,
    inspected_bytes: u64,
    charged_predicate_nodes: u64,
    maximum_predicate_node_bound: u64,
    maximum_source_records: u64,
    maximum_source_bytes: u64,
    maximum_source_snapshots: usize,
    stop: Option<String>,
    attention: Vec<Value>,
    triggered: Vec<String>,
    evaluated_anchor: Option<StateAnchor>,
    endpoint_publication_complete: bool,
    failed_endpoint_plan: Option<String>,
    published: Option<String>,
}

impl SelectedPlanSetWait {
    pub(super) fn new(
        session: &LabSession,
        mut digests: Vec<String>,
        policy: Option<String>,
        requested_ticks: u64,
        started: Instant,
    ) -> Result<Self> {
        let policy = PlanPolicy::parse(policy.as_deref())?;
        if digests.is_empty() || digests.len() > MAX_PLANS {
            return Err(invalid(
                "plan_digests must contain one to eight explicit originals",
            ));
        }
        digests.sort();
        if digests.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(invalid("plan_digests must be unique"));
        }
        let parsed = digests
            .iter()
            .map(|digest| selected_plan_wait::parse_digest(digest))
            .collect::<Result<Vec<_>>>()?;
        let context = read_context(session);
        context.budget.validate()?;
        authorize_entry(&context, Capability::Observe, RiskTier::ReadOnly)?;
        authorize_entry(&context, Capability::Query, RiskTier::ReadOnly)?;
        if requested_ticks > 0 && !session.adapter.snapshot().paused {
            let mut end = context.clone();
            end.anchor.tick = GameTick(
                context
                    .anchor
                    .tick
                    .0
                    .checked_add(requested_ticks)
                    .ok_or_else(|| exhausted("selected plan set exceeds the game-time horizon"))?,
            );
            authorize_entry(&end, Capability::Query, RiskTier::ReadOnly)?;
        }
        let members = SelectedPlanWait::set_members(session, &parsed, started)?;
        // Original proof complexity is immutable. Admit the whole selected
        // domain before even constructing the initial objective catalogue.
        SelectedPlanWait::set_predicate_node_bound(&members, session)?;
        let selected = Self {
            members,
            policy,
            basis: context.anchor,
            started,
            inspections: 0,
            inspected_records: 0,
            inspected_bytes: 0,
            charged_predicate_nodes: 0,
            maximum_predicate_node_bound: 0,
            maximum_source_records: 0,
            maximum_source_bytes: 0,
            maximum_source_snapshots: 0,
            stop: None,
            attention: Vec::new(),
            triggered: Vec::new(),
            evaluated_anchor: None,
            endpoint_publication_complete: false,
            failed_endpoint_plan: None,
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
            return Err(exhausted(
                "selected plan set exhausted its shared foreground deadline",
            ));
        }
        Ok(())
    }

    fn remaining_context(&self, session: &LabSession) -> Result<OperationContext> {
        self.check(session)?;
        let mut context = read_context(session);
        let spent = self.started.elapsed().as_nanos().div_ceil(1_000_000);
        if spent >= u128::from(context.budget.max_wall_millis) {
            return Err(exhausted(
                "selected plan set has no remaining inspection time",
            ));
        }
        context.budget.max_wall_millis -= spent as u64;
        Ok(context)
    }

    pub(super) fn stopped(&self) -> bool {
        self.stop.is_some()
    }

    /// Inspect one explicit set using only the currently retained version book.
    /// A Task owns no foreground progression or endpoint publication. Even the
    /// first exact horizon is pending until a caller explicitly captures it.
    pub(super) fn observe_retained(
        session: &LabSession,
        digests: &[String],
        policy: &str,
        started: Instant,
    ) -> Result<Value> {
        if digests.is_empty() || digests.len() > MAX_PLANS {
            return Err(invalid(
                "plan_digests must contain one to eight explicit originals",
            ));
        }
        let mut selected = Self::new(
            session,
            digests.to_vec(),
            Some(policy.to_owned()),
            0,
            started,
        )?;
        let source = SelectedPlanWait::admit_retained_set_source(&selected.members, session)?;
        selected.record_source(&source)?;
        let mut plans = Vec::with_capacity(selected.members.len());
        for member in &mut selected.members {
            let observed = member.observe_set_member(session, &source)?;
            let mut plan = observed.projection;
            plan["original_goal_pending"] = json!(observed.pending);
            plan["complete_original_goal_truth"] = json!(match observed.truth {
                PredicateTruth::True => "true",
                PredicateTruth::False => "false",
                PredicateTruth::Unknown => "unknown",
            });
            plan["source_work"]["accounting_shared_with_selected_set"] = json!(true);
            plan["other_session_work_progresses"] = json!(false);
            plans.push(plan);
        }
        selected.evaluated_anchor = Some(session.adapter.snapshot().anchor());
        selected.classify(&plans);
        let capture_required = plans
            .iter()
            .any(|plan| plan["reserve_capture_required"] == true);
        let mut outcome = selected.outcome(session, plans, true);
        outcome["caller_plan_digests"] = json!(digests);
        outcome["observation_only"] = json!(true);
        outcome["other_committed_work_progresses"] = json!(false);
        outcome["reserve_capture_required"] = json!(capture_required);
        outcome["archive_hydration_performed"] = json!(false);
        outcome["endpoint_publication"] = json!({
            "mode":"retained_evidence_only", "operation":"passive_observation",
            "endpoints_captured":false,"publication_performed":false,
            "atomic_set_publication":false,
        });
        outcome["source_work"]["repetition_bounds"] =
            json!("one_current_frontier_under_one_observation_deadline");
        let maximum = session
            .budget
            .max_bytes
            .min(u64::from(session.budget.max_output_tokens) * 4);
        if outcome.to_string().len() as u64 > maximum {
            return Err(exhausted(
                "complete passive original set exceeds its response budget",
            ));
        }
        selected.check(session)?;
        Ok(outcome)
    }

    pub(super) fn ceiling(&self, now: u64, other: Option<u64>) -> Option<u64> {
        self.members
            .iter()
            .fold(other, |ceiling, member| member.ceiling(now, ceiling))
    }

    fn plans(&self, session: &LabSession, inspection_complete: bool) -> Vec<Value> {
        self.members
            .iter()
            .map(|member| {
                let mut plan = member.projection(session, inspection_complete);
                plan["source_work"]["accounting_shared_with_selected_set"] = json!(true);
                plan
            })
            .collect()
    }

    pub(super) fn projection(&self, session: &LabSession, inspection_complete: bool) -> Value {
        let current = session.adapter.snapshot().anchor();
        let inspection_complete = inspection_complete
            && self.evaluated_anchor == Some(current)
            && self.endpoint_publication_complete;
        self.outcome(
            session,
            self.plans(session, inspection_complete),
            inspection_complete,
        )
    }

    fn outcome(&self, session: &LabSession, plans: Vec<Value>, inspection_complete: bool) -> Value {
        let complete = plans.iter().filter(|plan| plan["complete"] == true).count();
        let all_complete = !plans.is_empty() && complete == plans.len();
        let any_complete = complete != 0;
        let group_complete = inspection_complete
            && self.attention.is_empty()
            && match self.policy {
                PlanPolicy::All => all_complete,
                PlanPolicy::Any => any_complete,
            };
        let interrupted = self
            .attention
            .iter()
            .map(|cause| cause["plan_digest"].clone())
            .collect::<Vec<_>>();
        let unfinished = plans
            .iter()
            .filter(|plan| plan["complete"] != true)
            .map(|plan| plan["plan_digest"].clone())
            .collect::<Vec<_>>();
        json!({
            "schema":"dfmcp.selected-plan-set-wait/1",
            "requested_plan_digests":self.members.iter().map(|member| member.requested_digest().to_hex()).collect::<Vec<_>>(),
            "plan_digests":self.members.iter().map(|member| member.actual_digest().to_hex()).collect::<Vec<_>>(),
            "plan_policy":self.policy.name(),
            "selection_scope":"exact_original_plan_set",
            "membership_order":"actual_original_plan_digest",
            "plans":plans,
            "selected":self.members.len(),
            "complete_plans":complete,
            "all_complete":inspection_complete && all_complete,
            "any_complete":inspection_complete && any_complete,
            "group_complete":group_complete,
            "inspection_complete":inspection_complete,
            "stop_reason":self.stop,
            "attention":self.attention,
            "interrupted_plan_digests":interrupted,
            "unfinished_plan_digests":unfinished,
            "triggered_plan_digests":self.triggered,
            "evaluated_anchor":self.evaluated_anchor.as_ref().map(anchor_json),
            "observed_anchor":anchor_json(&session.adapter.snapshot().anchor()),
            "completion_scope":"current_original_outcomes_at_one_frontier",
            "earlier_completion_is_latched":false,
            "simultaneous_current_goal_truth_proven":inspection_complete && all_complete,
            "continuous_between_observations":false,
            "other_committed_work_progresses":true,
            "descendant_plans_included":false,
            "dispatch_handles_restored":false,
            "automatic_continuation":false,
            "automatic_cancellation":false,
            "endpoint_publication":{
                "mode":"independent_immutable_original_records",
                "atomic_set_publication":false,
                "complete":inspection_complete,
                "failed_plan_digest":self.failed_endpoint_plan,
                "successful_prefix_retained":true,
            },
            "source_work":{
                "inspections":self.inspections,
                "inspected_records":self.inspected_records,
                "inspected_canonical_and_retained_bytes":self.inspected_bytes,
                "maximum_source_records":self.maximum_source_records,
                "maximum_source_bytes":self.maximum_source_bytes,
                "maximum_distinct_snapshots":self.maximum_source_snapshots,
                "size_limit_scope":"each_complete_selected_source_union",
                "historical_snapshot_deduplication":"exact_anchor",
                "maximum_selected_plans":MAX_PLANS,
                "maximum_inspections":MAX_INSPECTIONS,
                "repetition_bounds":"one_foreground_deadline_and_shared_slice_poll_dispatch_limits",
            },
            "proof_work":{
                "accounting":"conservative_predicate_node_upper_bound",
                "charged_node_visits":self.charged_predicate_nodes,
                "maximum_frontier_node_bound":self.maximum_predicate_node_bound,
                "maximum_nodes_per_frontier":selected_plan_wait::MAX_SET_PREDICATE_NODES,
                "scope":"selected_original_goal_stage_and_reserve_evaluations",
                "semantic_goal_node_reserve":selected_plan_wait::SEMANTIC_GOAL_NODE_RESERVE,
                "actual_node_visits_measured":false,
            },
        })
    }

    pub(super) fn sample(&mut self, session: &mut LabSession, base: Value) -> Result<()> {
        let result = self.sample_frontier(session, base);
        if result.is_err() {
            self.endpoint_publication_complete = false;
            for member in &mut self.members {
                member.discard_unpublished_endpoint(session);
            }
        }
        result
    }

    fn sample_frontier(&mut self, session: &mut LabSession, mut base: Value) -> Result<()> {
        self.check(session)?;
        if self.inspections >= MAX_INSPECTIONS {
            return Err(exhausted(
                "selected plan set reached its shared inspection limit",
            ));
        }
        let source = SelectedPlanWait::admit_set_source(&self.members, session)?;
        self.record_source(&source)?;
        self.endpoint_publication_complete = false;
        self.failed_endpoint_plan = None;
        // Reinspect every member, including a previously complete consumable
        // goal. Earlier truth never latches a later all/any outcome.
        for member in &mut self.members {
            member.stage_set_member(session, &source)?;
        }
        self.evaluated_anchor = Some(session.adapter.snapshot().anchor());
        let plans = self.plans(session, true);
        self.classify(&plans);
        let remaining = base["wait_progress"]["remaining_game_ticks"]
            .as_u64()
            .ok_or_else(|| {
                invalid("selected plan set lacks the shared remaining tick allowance")
            })?;
        if self.stop.is_none() {
            self.stop = if session.adapter.snapshot().paused {
                Some("paused".to_owned())
            } else if remaining == 0 {
                Some("tick_limit".to_owned())
            } else {
                None
            };
        }
        let current = self.remaining_context(session)?;
        let mut view = temporal_handler::projection(session, &current)?;
        let mut active_work = view.active_work;
        active_work["query_baselines"] =
            temporal_query::retained_baseline_work(&self.remaining_context(session)?)?;
        let watches = temporal_query::retained_watch_work(&self.remaining_context(session)?)?;
        active_work["query_watches"] = watches["query_watches"].clone();
        active_work["condition_watches"] = watches["condition_watches"].clone();
        active_work["mcp_tasks"] = crate::task_service::session_handles(&view.session_id);
        active_work["mcp_task_coverage"] =
            crate::task_service::session_handle_coverage(&view.session_id);
        if let Some(carried) = base.get("carried_obligations") {
            active_work["recovered_obligations"] = carried.clone();
        }
        // Both catalogues were projected before the independent E appends.
        // Overlay only the prospective endpoint fields in the private packet,
        // so the admitted response describes the exact accepted publication.
        overlay_endpoints(&plans, &mut base["objectives"])?;
        overlay_endpoints(&plans, &mut view.objectives)?;
        base["selected_plan_set_wait"] = self.outcome(session, plans, true);
        let rendered = render(
            &self.remaining_context(session)?,
            self.basis,
            &view.session_id,
            view.objectives,
            active_work,
            base,
        )?;
        self.check(session)?;
        // The complete prospective result fits before the first E append. Each
        // immutable original publishes independently. A later failure leaves
        // the already accepted prefix in custody and prevents another slice.
        for member in &mut self.members {
            if let Err(error) = member.retain_set_endpoint(session) {
                self.failed_endpoint_plan = Some(member.actual_digest().to_hex());
                return Err(error);
            }
        }
        // No fallible projection or fitting follows the final accepted append.
        self.endpoint_publication_complete = true;
        self.published = Some(rendered);
        Ok(())
    }

    fn record_source(&mut self, source: &selected_plan_wait::PlanSetSource) -> Result<()> {
        let inspections = self
            .inspections
            .checked_add(1)
            .filter(|count| *count <= MAX_INSPECTIONS)
            .ok_or_else(|| exhausted("selected plan set reached its shared inspection limit"))?;
        self.inspections = inspections;
        self.inspected_records = self
            .inspected_records
            .checked_add(source.records)
            .ok_or_else(|| exhausted("selected plan set source record accounting overflowed"))?;
        self.inspected_bytes = self
            .inspected_bytes
            .checked_add(source.bytes)
            .ok_or_else(|| exhausted("selected plan set source byte accounting overflowed"))?;
        self.charged_predicate_nodes = self
            .charged_predicate_nodes
            .checked_add(source.predicate_node_bound)
            .ok_or_else(|| exhausted("selected plan set predicate accounting overflowed"))?;
        self.maximum_predicate_node_bound = self
            .maximum_predicate_node_bound
            .max(source.predicate_node_bound);
        self.maximum_source_records = self.maximum_source_records.max(source.records);
        self.maximum_source_bytes = self.maximum_source_bytes.max(source.bytes);
        self.maximum_source_snapshots = self.maximum_source_snapshots.max(source.snapshots);
        Ok(())
    }

    fn classify(&mut self, plans: &[Value]) {
        self.attention = plans
            .iter()
            .filter_map(|plan| {
                let reason = plan["stop_reason"].as_str()?;
                if matches!(reason, "complete" | "paused" | "tick_limit") {
                    return None;
                }
                Some(json!({"plan_digest":plan["plan_digest"],
                "requested_plan_digest":plan["requested_plan_digest"],"stop_reason":reason}))
            })
            .collect();
        let completed = plans
            .iter()
            .filter(|plan| plan["complete"] == true)
            .filter_map(|plan| plan["plan_digest"].as_str().map(str::to_owned))
            .collect::<Vec<_>>();
        let group_complete = match self.policy {
            PlanPolicy::All => !plans.is_empty() && completed.len() == plans.len(),
            PlanPolicy::Any => !completed.is_empty(),
        };
        if let Some(first) = self.attention.first() {
            self.stop = first["stop_reason"].as_str().map(str::to_owned);
            self.triggered = self
                .attention
                .iter()
                .filter_map(|cause| cause["plan_digest"].as_str().map(str::to_owned))
                .collect();
        } else if group_complete {
            self.stop = Some(self.policy.complete_reason().to_owned());
            self.triggered = completed;
        } else {
            self.stop = None;
            self.triggered.clear();
        }
    }

    pub(super) fn take_published(&mut self) -> Option<String> {
        self.published.take()
    }
}

fn overlay_endpoints(plans: &[Value], objectives: &mut Value) -> Result<()> {
    let objectives = objectives.as_array_mut().ok_or_else(|| {
        invalid("selected plan set requires the complete current objective catalogue")
    })?;
    for plan in plans {
        let reserve = &plan["reserve_frontier"];
        if reserve["available"] != true {
            continue;
        }
        let mut matches = objectives
            .iter_mut()
            .filter(|objective| objective["plan_digest"] == plan["plan_digest"]);
        let original = matches.next().ok_or_else(|| {
            invalid("a selected endpoint is absent from the current objective catalogue")
        })?;
        if matches.next().is_some() {
            return Err(invalid(
                "a selected endpoint has ambiguous objective catalogue custody",
            ));
        }
        original["retained_reserve_endpoint"] = json!({
            "anchor":reserve["observed_anchor"],"truth":reserve["truth"],
            "durable":reserve["durable"],"historical_only":original["archived"] == true,
            "continuous_maintenance_proven":false,
        });
    }
    Ok(())
}

fn render(
    context: &OperationContext,
    basis: StateAnchor,
    session_id: &str,
    objectives: Value,
    active_work: Value,
    mut payload: Value,
) -> Result<String> {
    let maximum = context
        .budget
        .max_bytes
        .min(u64::from(context.budget.max_output_tokens) * 4);
    payload["request_id"] = json!(context.request_id.to_string());
    let selected = payload["selected_plan_set_wait"].clone();
    let recommendations = crate::agent_facade::selected_plan_set_guidance(
        &selected,
        payload["paused"] == true,
        context.budget.max_game_ticks,
        authorize_entry(context, Capability::Plan, RiskTier::ReadOnly).is_ok(),
        authorize_entry(context, Capability::ControlClock, RiskTier::Reversible).is_ok(),
    );
    let references = selected["plans"]
        .as_array()
        .map(|plans| {
            plans
                .iter()
                .map(|plan| {
                    json!({"kind":"original_plan_outcome","plan_digest":plan["plan_digest"],
            "plan_object_digest":plan["plan_object_digest"],"anchor":plan["observed_anchor"],
            "reserve_anchor":plan["reserve_frontier"]["observed_anchor"]})
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let packet = AgentTurnBuilder::new("fortress.wait", AgentPhase::Verify)
        .session_id(session_id.to_owned()).request_id(context.request_id.to_string())
        .turn_id(format!("plan-set-wait-{}", context.request_id))
        .anchor(anchor_json(&context.anchor))
        .continuity(ContinuityStatus::Partial, Some(anchor_json(&basis)), None, None)
        .profile(ObservationProfile::Tactical)
        .briefing(json!({
            "runtime":"laboratory_reference_world","adapter":"memory",
            "paused":payload["paused"],"objectives":objectives,
            "selected_plans":selected["plan_digests"],"plan_policy":selected["plan_policy"],
            "stop_reason":selected["stop_reason"],"group_complete":selected["group_complete"],
            "completion_scope":"current_original_outcomes_at_one_frontier",
            "live_game_qualification":false,
        }))
        .active_work(active_work)
        .coverage(json!({
            "status":"partial",
            "complete_domains":["laboratory.source_anchor","laboratory.selected_original_outcomes",
                "laboratory.session_query_baseline_catalog","laboratory.session_query_watch_catalog"],
            "partial_domains":[{"domain":"laboratory.selected_original_reserves",
                "reason":"each first exact endpoint is independent; no continuous maintenance or atomic group journal claim"}],
            "omitted_domains":["live_dwarf_fortress_state"],"absence_proof_scope":[],
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
        .recommendations(recommendations)
        .references(references)
        .attach(payload);
    if packet.len() as u64 > maximum {
        return Err(exhausted(
            "complete selected plan set, retained game work and Agent Turn exceed the response budget",
        ));
    }
    Ok(packet)
}
