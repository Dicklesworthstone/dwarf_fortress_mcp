//! Read the exact operations source of a successfully published original goal.
//! No capture, reprojection, native contact, or new proof authority lives here.
use super::*;
use dfmcp_adapter::semantic_work_orders::WorkOrderGoalProjection;
use dfmcp_core::StepId;
use serde::Deserialize;

#[allow(clippy::duplicate_mod)]
#[path = "../operations_production.rs"]
mod production;
#[allow(clippy::duplicate_mod)]
#[path = "../semantic_query_core.rs"]
mod structured;

const MAX_REQUEST: usize = 65_536;
const MAX_QUERY_TOKENS: u32 = 4_096;

pub(crate) struct InspectionRequest(Value);
impl InspectionRequest {
    pub(crate) fn parse(raw: &str) -> Result<Self> {
        if raw.len() > MAX_REQUEST {
            return Err(exhausted("production inspection request exceeds 65536 bytes"));
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
            || !matches!(
                envelope.query["kind"].as_str(),
                Some(
                    "production_diagnosis"
                        | "inventory_plan"
                        | "entities"
                        | "inspect"
                        | "traverse"
                        | "dependencies"
                )
            )
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
                    p.anchor == self.anchor
                        && p.selected_anchors.contains(&(source.step, anchor))
                });
        joined.then_some(source)
    }

    pub(super) fn inspection_source_json(&self) -> Value {
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
                    "query_kinds":["production_diagnosis","inventory_plan","entities","inspect","traverse","dependencies"],
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
            structured::execute(snapshot, &query_context, &request.0)?
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inspection_envelope_is_closed_and_cannot_select_effects() {
        for kind in [
            "production_diagnosis",
            "inventory_plan",
            "entities",
            "inspect",
            "traverse",
            "dependencies",
        ] {
            let request = json!({"schema":"dfmcp.query/1","query":{"kind":kind}});
            assert!(InspectionRequest::parse(&request.to_string()).is_ok());
        }
        for raw in [
            r#"{"schema":"dfmcp.query/1","query":{"kind":"furniture_allocation"}}"#,
            r#"{"schema":"dfmcp.query/1","query":{"kind":"production_diagnosis"},"confirmed":true}"#,
            r#"{"schema":"dfmcp.query/1","schema":"dfmcp.query/1","query":{"kind":"production_diagnosis"}}"#,
            r#"{"schema":"dfmcp.query/2","query":{"kind":"production_diagnosis"}}"#,
        ] {
            assert!(InspectionRequest::parse(raw).is_err(), "{raw}");
        }
        assert!(InspectionRequest::parse(&" ".repeat(MAX_REQUEST + 1)).is_err());
    }
}
