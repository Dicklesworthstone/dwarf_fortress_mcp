//! Read an explicitly captured planning source or the exact operations source
//! of a successfully published original goal. Query itself performs no native
//! contact, reprojection, or new proof-authority issuance.
use super::*;
use dfmcp_adapter::semantic_work_orders::WorkOrderGoalProjection;
use dfmcp_core::{DfmcpError, ErrorCode, StepId};
use dfmcp_world::WorldSnapshot;
use serde::Deserialize;

#[allow(clippy::duplicate_mod)]
#[path = "../operations_production.rs"]
mod production;
#[allow(clippy::duplicate_mod)]
#[path = "../semantic_query_core.rs"]
mod structured;
// Shared pure query engines use the same typed filter decoder and exact source
// anchor. Stateful watches and effect-bearing artifacts remain outside this API.
use structured as core_query;
#[allow(clippy::duplicate_mod)]
#[path = "../event_query.rs"]
mod event_query;
#[allow(clippy::duplicate_mod)]
#[path = "../operational_query.rs"]
mod operational;
#[allow(clippy::duplicate_mod)]
#[path = "../ranked_entity_query.rs"]
mod ranked_entity_query;
#[allow(clippy::duplicate_mod)]
#[path = "../related_entity_query.rs"]
mod related_entity_query;
#[allow(clippy::duplicate_mod)]
#[path = "../spatial_entity_query.rs"]
mod spatial_entity_query;

const MAX_REQUEST: usize = 65_536;
const MAX_QUERY_TOKENS: u32 = 4_096;
pub(super) const QUERY_KINDS: &[&str] = &[
    "production_diagnosis",
    "inventory_plan",
    "entities",
    "inspect",
    "traverse",
    "dependencies",
    "aggregate",
    "search",
    "events",
    "spatial_entities",
    "related_entities",
    "ranked_entities",
];
pub(super) const INVENTORY_TOTALS_SEMANTICS: &str = "observed_stock_not_usable_supply";

/// A bounded complete summary of the captured item records. This deliberately
/// keeps physical stack quantities separate from allocatable or usable supply.
pub(super) fn inventory_totals_request(anchor: StateAnchor) -> Value {
    json!({"schema":"dfmcp.query/1","expected_anchor":anchor_json(anchor),
        "query":{"kind":"aggregate","kinds":["item"],
            "group_by":{"kind":"field","field":"type_key"},
            "metrics":[{"name":"stack_units","field":"stack_size","numeric_type":"u64"}],
            "max_groups":32}})
}

pub(crate) struct InspectionRequest(Value);
impl InspectionRequest {
    pub(crate) fn parse(raw: &str) -> Result<Self> {
        if raw.len() > MAX_REQUEST {
            return Err(exhausted(
                "production inspection request exceeds 65536 bytes",
            ));
        }
        // Direct deserialization also refuses duplicate envelope fields. Each
        // query engine independently validates its own closed query fields.
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Envelope {
            schema: String,
            expected_anchor: Option<Value>,
            query: Value,
        }
        let envelope: Envelope = serde_json::from_str(raw)
            .map_err(|_| invalid("invalid production inspection envelope"))?;
        if envelope.schema != "dfmcp.query/1"
            || !envelope.query["kind"]
                .as_str()
                .is_some_and(|kind| QUERY_KINDS.contains(&kind))
        {
            return Err(invalid(
                "unsupported production inspection schema or query kind",
            ));
        }
        let mut value = json!({"schema":envelope.schema,"query":envelope.query});
        if let Some(anchor) = envelope.expected_anchor {
            value["expected_anchor"] = anchor;
        }
        Ok(Self(value))
    }
}

/// This host publishes `tick`, while the reusable structured-query engine has
/// an established `game_tick` wire shape. Accept only the complete host anchor
/// before translating it; otherwise normalization could disguise a stale or
/// malformed caller precondition. Other structured-query hosts remain unchanged.
fn execute_structured(
    snapshot: &WorldSnapshot,
    context: &OperationContext,
    input: &Value,
) -> Result<Value> {
    context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    let public_anchor = anchor_json(snapshot.anchor());
    let engine_anchor = structured::anchor_json(snapshot.anchor());
    let mut translated = input.clone();
    if let Some(expected) = input.get("expected_anchor") {
        if *expected != public_anchor {
            return Err(DfmcpError::new(
                ErrorCode::StaleAnchor,
                "expected_anchor does not match the complete production inspection anchor",
            ));
        }
        translated["expected_anchor"] = engine_anchor.clone();
    }
    let mut payload = match translated["query"]["kind"].as_str() {
        Some("aggregate" | "search") => operational::execute(snapshot, context, &translated)?,
        Some("events") => event_query::execute(snapshot, context, &translated)?,
        Some("spatial_entities") => spatial_entity_query::execute(snapshot, context, &translated)?,
        Some("related_entities") => related_entity_query::execute(snapshot, context, &translated)?,
        Some("ranked_entities") => ranked_entity_query::execute(snapshot, context, &translated)?,
        _ => structured::execute(snapshot, context, &translated)?,
    };
    payload["anchor"] = public_anchor.clone();
    if let Some(witness) = payload.get_mut("witness") {
        if witness["anchor"] != engine_anchor {
            return Err(DfmcpError::new(
                ErrorCode::InternalInvariantViolation,
                "structured query witness changed its source anchor",
            ));
        }
        witness["anchor"] = public_anchor;
    }
    // A stale fact's reason carries its historical anchor, not the current
    // source anchor. Rename only the known wire key without rebasing that fact
    // or touching user-visible object values. Every replacement shrinks output.
    if let Some(rows) = payload.get_mut("rows").and_then(Value::as_array_mut) {
        for row in rows {
            normalize_stale_fact_anchors(row);
        }
    }
    if let Some(row) = payload.get_mut("row") {
        normalize_stale_fact_anchors(row);
    }
    if payload["kind"] == "aggregate" {
        payload["stock_semantics"] = json!(INVENTORY_TOTALS_SEMANTICS);
        payload["eligibility"] = json!("not_assessed");
        payload["reservation_created"] = json!(false);
        payload["original_goal_evidence"] = json!(false);
        // Account for the additional stock-interpretation fields as part of
        // the result. A complete summary cannot be clipped to fit a budget.
        let maximum = context
            .budget
            .max_bytes
            .min(u64::from(context.budget.max_output_tokens).saturating_mul(4));
        let bytes = serde_json::to_vec(&payload)
            .map_err(|_| invalid("cannot encode production aggregate result"))?;
        if bytes.len() as u64 > maximum {
            return Err(exhausted(
                "complete production aggregate exceeds the response budget; narrow kinds, metrics, or groups",
            ));
        }
    }
    // Continuations are opaque engine tokens. Keeping them unchanged preserves
    // the exact session, selected-field and complete-snapshot identity checks.
    Ok(payload)
}

fn normalize_stale_fact_anchors(row: &mut Value) {
    if let Some(fields) = row.get_mut("fields").and_then(Value::as_object_mut) {
        for fact in fields.values_mut() {
            if fact["presence"] == "stale"
                && let Some(reason) = fact.get_mut("reason").and_then(Value::as_object_mut)
                && let Some(tick) = reason.remove("game_tick")
            {
                reason.insert("tick".into(), tick);
            }
        }
    }
}

pub(super) struct InspectionSource {
    step: StepId,
    parent_anchor: StateAnchor,
    projection: WorkOrderGoalProjection,
}
impl InspectionSource {
    pub(super) fn from_sample(sample: &live_production::SelectedParentGoalResult) -> Self {
        Self {
            step: sample.step_id(),
            parent_anchor: sample.parent_progress().anchor,
            projection: sample.operations_projection().clone(),
        }
    }
}

impl Runtime {
    fn current_inspection_source(&self) -> Option<&InspectionSource> {
        if self.mode == Mode::Offline
            || self.fenced
            || self.goal_current_withdrawn
            || self.clock_withholds_goal()
            || self.parent_goal_observation_fenced()
        {
            return None;
        }
        let source = self.inspection_source.as_ref()?;
        let selected = self.goal_monitor.as_ref()?.latest()?;
        let parent = self.parent_goal.as_ref()?;
        let anchor = source.projection.anchor();
        if self.selected_order_step() != Some(source.step)
            || selected.anchor != anchor
            || parent.anchor != source.parent_anchor
            || parent.selected_anchor != anchor
            || anchor.fortress_id != self.anchor.fortress_id
            || anchor.cursor.epoch != self.anchor.cursor.epoch
            || anchor.tick != self.anchor.tick
            || anchor.tick < self.observation_tick_floor
        {
            return None;
        }
        // A completed portfolio has a separate joined anchor. Only its exact
        // sealed selected-anchor tuple may authorize this retained leaf.
        let joined = source.parent_anchor == self.anchor
            || self
                .portfolio
                .as_ref()
                .and_then(|p| p.current.as_ref())
                .is_some_and(|p| {
                    p.anchor == self.anchor && p.selected_anchors.contains(&(source.step, anchor))
                });
        joined.then_some(source)
    }

    pub(super) fn inspection_source_json(&self) -> Value {
        if self.original.is_none() {
            return self.planning_inspection_json();
        }
        self.current_inspection_source()
            .map_or(Value::Null, |source| {
                let anchor = anchor_json(source.projection.anchor());
                json!({"scope":"captured_operations","attribution":"not_order_attributed",
                    "order_step_id":source.step.get(),"anchor":anchor,
                    "parent_anchor":anchor_json(self.anchor),
                    "source_digest":source.projection.source_digest().to_hex(),
                    "restored_from_history":false,"mutation_authority":false,
                    "diagnosis_request":{"schema":"dfmcp.query/1","expected_anchor":anchor,
                        "query":{"kind":"production_diagnosis","limit":4}},
                    "inventory_totals_request":inventory_totals_request(source.projection.anchor()),
                    "inventory_totals_semantics":INVENTORY_TOTALS_SEMANTICS,
                    "query_kinds":QUERY_KINDS,
                    "unknown":["job_to_original_order_attribution","full_native_job_requirements",
                        "path_access","labor_eligibility","successful_job_completion"]})
            })
    }

    pub(crate) fn inspect_production(
        &mut self,
        request: InspectionRequest,
        step: Option<u32>,
        context: &OperationContext,
    ) -> Result<View> {
        let result = self.inspect_production_inner(request, step, context);
        if result.is_err() {
            self.abandon_preparation();
        }
        result
    }

    fn inspect_production_inner(
        &mut self,
        request: InspectionRequest,
        step: Option<u32>,
        context: &OperationContext,
    ) -> Result<View> {
        self.check(context, false)?;
        if self.original.is_none() {
            if step.is_some() {
                return Err(invalid(
                    "pre-plan production inspection has no original order selector",
                ));
            }
            return self.inspect_planning_production(request, context);
        }
        if step.is_some() {
            self.select_order(step, context)?;
        }
        let mut work = Work::new(context)?;
        let view_budget = work.take(256 * MIB)?;
        let final_custody = work.take(256 * MIB)?;
        self.restore_clock_target(&mut work)?;
        self.restore_inventory_clock_target(&mut work)?;
        let mut view = self.view(&mut Work::new(&work.narrow(&view_budget)?)?)?;

        // Check current authority before rebinding to a retained selected anchor.
        // A historical observation cannot revive an expired or withdrawn grant.
        let mut authority = work.current()?;
        authority.anchor.tick = authority.anchor.tick.max(self.observation_tick_floor);
        authority.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
        authority.authorize(Capability::Observe, RiskTier::ReadOnly, &[], None)?;
        let source = self.current_inspection_source().ok_or_else(|| {
            denied("no current production inspection source; Wait must publish a fresh selected goal sample")
        })?;
        let original = self
            .original
            .as_ref()
            .ok_or_else(|| denied("original plan absent"))?;
        // Revalidate the operator's complete source/version/generation pins.
        // The returned goal policy is not proof of diagnostic causation.
        self.config.orders.acceptance().goal_for_selected(
            original,
            source.step,
            &source.projection,
            &authority,
        )?;
        let snapshot = source.projection.snapshot();
        if snapshot.graph.entities.len() > authority.budget.max_entities as usize {
            return Err(exhausted(
                "production inspection exceeds the complete source entity budget",
            ));
        }
        let bytes = u64::try_from(snapshot.canonical_bytes().len())
            .map_err(|_| exhausted("production inspection source size overflow"))?;
        work.take(bytes)?;
        let mut query_context = work.rest(512 * MIB)?;
        query_context.anchor = source.projection.anchor();
        query_context.budget.max_output_tokens =
            query_context.budget.max_output_tokens.min(MAX_QUERY_TOKENS);
        let mut payload = if production::handles(&request.0) {
            production::execute(&source.projection, &query_context, &request.0)?
        } else {
            execute_structured(snapshot, &query_context, &request.0)?
        };
        payload["inspection_scope"] = json!("captured_operations");
        payload["attribution"] = json!("not_order_attributed");
        payload["source_digest"] = json!(source.projection.source_digest().to_hex());
        payload["order_step_id"] = json!(source.step.get());
        payload["mutation_authority"] = json!(false);
        payload["commit_compatible"] = json!(false);

        // Pay for the final immutable custody check separately, under the
        // original shrinking operation deadline.
        self.config.current()?;
        let mut final_work = Work::new(&work.narrow(&final_custody)?)?;
        self.arm(&mut final_work)?;
        let verified = self
            .custody
            .as_ref()
            .ok_or_else(|| denied("production inspection custody absent"))?
            .verify(&final_work.current()?);
        let disarmed = self
            .custody
            .as_ref()
            .ok_or_else(|| denied("production inspection custody absent"))?
            .disarm();
        verified?;
        disarmed?;
        work.current()?;
        self.check(&work.narrow(context)?, false)?;
        view.value["production_inspection"] = payload;
        Ok(view)
    }

    fn inspect_planning_production(
        &mut self,
        request: InspectionRequest,
        context: &OperationContext,
    ) -> Result<View> {
        let mut work = Work::new(context)?;
        // Verify the existing empty native custodians without creating an
        // original archive, frontier, goal monitor, or native order receipt.
        let final_view = work.take(256 * MIB)?;
        let mut view = self.view(&mut Work::new(&work.take(256 * MIB)?)?)?;
        let authority = work.current()?;
        inventory_baseline::authorize_observe(&authority, self.observation_tick_floor)?;
        let source = self.current_planning_inspection().ok_or_else(|| {
            denied("no current planning operations; Observe with include_production=true before planning")
        })?;
        let observation = source
            .observation()
            .ok_or_else(|| denied("planning source absent"))?;
        self.config
            .orders
            .acceptance()
            .planning_operations(observation, &authority)?;
        let snapshot = source
            .snapshot()
            .ok_or_else(|| denied("planning projection absent"))?;
        if snapshot.graph.entities.len() > authority.budget.max_entities as usize {
            return Err(exhausted(
                "production inspection exceeds the complete source entity budget",
            ));
        }
        // Reserve the full bounded projection before hashing/serializing it;
        // a tiny page or selector does not bypass the cost of its whole source.
        let source_budget = work.take(planning_inspection::PROJECTION_BYTES)?;
        if snapshot.canonical_bytes().len() as u64 > source_budget.budget.max_bytes {
            return Err(exhausted(
                "planning inspection exceeds its complete source byte budget",
            ));
        }
        let source_digest = source.source_digest()?;
        let mut query_context = work.rest(512 * MIB)?;
        query_context.anchor = snapshot.anchor();
        query_context.budget.max_output_tokens =
            query_context.budget.max_output_tokens.min(MAX_QUERY_TOKENS);
        let mut payload = if production::handles(&request.0) {
            production::execute(source, &query_context, &request.0)?
        } else {
            execute_structured(snapshot, &query_context, &request.0)?
        };
        payload["inspection_scope"] = json!("preplan_captured_operations");
        payload["observation_phase"] = json!("before_plan");
        payload["attribution"] = json!("not_order_attributed");
        payload["source_digest"] = json!(source_digest.to_hex());
        payload["canonical_anchor"] = anchor_json(self.anchor);
        payload["order_step_id"] = Value::Null;
        payload["mutation_authority"] = json!(false);
        payload["commit_compatible"] = json!(false);
        payload["original_goal_evidence"] = json!(false);

        self.config.current()?;
        inventory_baseline::authorize_observe(&work.current()?, self.observation_tick_floor)?;
        // Source-pure Query still verifies immutable current local custody both
        // before and after analysis. No game read or write is hidden here.
        self.view(&mut Work::new(&work.narrow(&final_view)?)?)?;
        self.check(&work.narrow(context)?, false)?;
        view.value["production_inspection"] = payload;
        Ok(view)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inspection_envelope_is_closed_and_cannot_select_effects() {
        for kind in QUERY_KINDS {
            let request = json!({"schema":"dfmcp.query/1","query":{"kind":kind}});
            assert!(InspectionRequest::parse(&request.to_string()).is_ok());
        }
        for raw in [
            r#"{"schema":"dfmcp.query/1","query":{"kind":"furniture_allocation"}}"#,
            r#"{"schema":"dfmcp.query/1","query":{"kind":"watch"}}"#,
            r#"{"schema":"dfmcp.query/1","query":{"kind":"capture"}}"#,
            r#"{"schema":"dfmcp.query/1","query":{"kind":"production_diagnosis"},"confirmed":true}"#,
            r#"{"schema":"dfmcp.query/1","schema":"dfmcp.query/1","query":{"kind":"production_diagnosis"}}"#,
            r#"{"schema":"dfmcp.query/2","query":{"kind":"production_diagnosis"}}"#,
        ] {
            assert!(InspectionRequest::parse(raw).is_err(), "{raw}");
        }
        assert!(InspectionRequest::parse(&" ".repeat(MAX_REQUEST + 1)).is_err());
    }
}

#[cfg(test)]
#[path = "inspection_anchor_tests.rs"]
mod anchor_tests;

#[cfg(test)]
#[path = "inspection_aggregate_tests.rs"]
mod aggregate_tests;
