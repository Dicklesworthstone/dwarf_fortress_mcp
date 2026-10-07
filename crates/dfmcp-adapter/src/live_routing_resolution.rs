//! Source-bound conversion of advisory routes into exact native plan values.
//!
//! These values have no dispatch authority. Existing family sessions must still
//! acquire fresh native preparation, authorize, journal, confirm and revalidate.

use dfmcp_core::{DfmcpError, Digest32, EntityId, ErrorCode, Result, StateAnchor, StepId};
use dfmcp_intent::{Action, MaterialSelector, PlanStep, PreparedPlan};
use dfmcp_world::{CompareOp, EntityKind, FactSource, Predicate, PredicateEvidence, Value};

use crate::LiveObservationCapsule;
use crate::build_placement::{BuildCapture, BuildItem, BuildPlan};
use crate::live_operations::item_entity_id;
use crate::live_projection::{LiveWorldProjection, entity_id_to_raw_unit_id};
use crate::live_spatial::{
    SpatialStateView,
    citizens::{LiveSpatialCitizenState, citizen_entity_id},
};
use crate::workforce_control::{
    AssignmentEffect, AssignmentPhase, AssignmentPlan, AssignmentSpec, WorkforceCapture,
};

use super::{LiveRequest, route_step, unit_ids};

fn invalid(message: impl Into<String>) -> DfmcpError {
    DfmcpError::new(ErrorCode::AdapterRejected, message)
}

fn stale(message: impl Into<String>) -> DfmcpError {
    DfmcpError::new(ErrorCode::StaleAnchor, message)
}

/// The exact canonical identity encoding established by a typed projection.
/// No numeric-range heuristic or caller-selected wire protocol is used.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LiveIdentitySchema {
    CitizensV1,
    SpatialV1_8,
}

impl LiveIdentitySchema {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CitizensV1 => "dfmcp.live_world_projection/2",
            Self::SpatialV1_8 => "spatial/1.8",
        }
    }
}

/// One independently source-qualified canonical version and the live projection
/// that produced it. Private fields prevent substitution of a schema, generation,
/// source digest or canonical anchor after construction. This is not admission.
#[derive(Clone, Debug)]
pub struct LiveRoutingEvidence<'a> {
    evidence: PredicateEvidence<'a>,
    schema: LiveIdentitySchema,
    source_digest: Digest32,
    generation: u64,
    folder: String,
    site: u32,
    map_dimensions: Option<[u32; 3]>,
}

impl<'a> LiveRoutingEvidence<'a> {
    /// The observing shell supplies its independently issued fact/domain scope.
    /// An untrusted, asserted, replayed or laboratory scope cannot resolve live
    /// identities merely because its bytes resemble a native projection.
    pub fn citizens_v1(
        evidence: PredicateEvidence<'a>,
        projection: &LiveWorldProjection,
        capsule: &LiveObservationCapsule,
    ) -> Result<Self> {
        projection.validate_against(capsule)?;
        if evidence.snapshot().anchor() != projection.snapshot.anchor() {
            return Err(stale(
                "routing evidence differs from the exact citizen projection",
            ));
        }
        let site =
            u32::try_from(capsule.site_id).map_err(|_| invalid("negative live fortress site"))?;
        Self::bind(
            evidence,
            LiveIdentitySchema::CitizensV1,
            capsule.content_digest,
            capsule.bridge.bridge_generation,
            &capsule.world_folder,
            site,
            None,
        )
    }

    /// Bind the currently published spatial/1.8 version, including its real
    /// generation history. A detached snapshot plus a claimed profile is refused.
    pub fn spatial_v1_8(
        evidence: PredicateEvidence<'a>,
        state: &LiveSpatialCitizenState,
    ) -> Result<Self> {
        let snapshot = state
            .snapshot()
            .ok_or_else(|| invalid("spatial/1.8 has no published snapshot"))?;
        let observation = state
            .observation_full()
            .ok_or_else(|| invalid("spatial/1.8 has no complete source observation"))?;
        observation.validate()?;
        if evidence.snapshot().anchor() != snapshot.anchor() {
            return Err(stale(
                "routing evidence differs from the exact spatial/1.8 projection",
            ));
        }
        let source = observation.spatial().terrain();
        Self::bind(
            evidence,
            LiveIdentitySchema::SpatialV1_8,
            observation.source_digest()?,
            source.bridge_generation,
            &source.world_folder,
            source.site_id,
            Some(source.map_dimensions),
        )
    }

    fn bind(
        evidence: PredicateEvidence<'a>,
        schema: LiveIdentitySchema,
        source_digest: Digest32,
        generation: u64,
        folder: &str,
        site: u32,
        map_dimensions: Option<[u32; 3]>,
    ) -> Result<Self> {
        if source_digest == Digest32::ZERO
            || generation == 0
            || folder.is_empty()
            || folder.len() > 512
            || folder.contains('\0')
            || evidence.snapshot().fortress_id
                != crate::workforce_control::fortress_id(folder, site)
        {
            return Err(invalid(
                "routing source lacks an exact native fortress identity",
            ));
        }
        Ok(Self {
            evidence,
            schema,
            source_digest,
            generation,
            folder: folder.to_owned(),
            site,
            map_dimensions,
        })
    }

    #[must_use]
    pub fn anchor(&self) -> StateAnchor {
        self.evidence.snapshot().anchor()
    }

    #[must_use]
    pub const fn schema(&self) -> LiveIdentitySchema {
        self.schema
    }

    #[must_use]
    pub const fn source_digest(&self) -> Digest32 {
        self.source_digest
    }

    /// Resolve only observed canonical unit entities from this exact schema.
    /// Identity includes the retained canonical generation, not only native ID.
    pub fn resolve_units(&self, units: &[EntityId]) -> Result<Vec<NativeUnitIdentity>> {
        let units = unit_ids(units).map_err(|refusal| invalid(refusal.reason))?;
        let mut out = Vec::with_capacity(units.len());
        for canonical_id in units {
            let entity = self
                .evidence
                .snapshot()
                .graph
                .entities
                .get(&canonical_id)
                .ok_or_else(|| invalid("canonical unit is not present in the source snapshot"))?;
            if entity.generation == 0
                || !self.evidence.establishes(&Predicate::EntityKind {
                    entity_id: canonical_id,
                    kind: EntityKind::Unit,
                })?
            {
                return Err(invalid(
                    "canonical unit kind or generation is not established",
                ));
            }
            let (native_id, field, source_name, value) = match self.schema {
                LiveIdentitySchema::CitizensV1 => {
                    let raw = entity_id_to_raw_unit_id(canonical_id)
                        .ok_or_else(|| invalid("canonical ID is outside the V1 unit schema"))?;
                    (
                        raw as u32,
                        "raw_unit_id",
                        "dfmcp_bridge.ReadObservation.citizen.unit_id",
                        Value::I64(i64::from(raw)),
                    )
                }
                LiveIdentitySchema::SpatialV1_8 => {
                    let fact = entity
                        .fields
                        .get("native_unit_id")
                        .ok_or_else(|| invalid("spatial citizen has no native identity fact"))?;
                    let Some(Value::U64(raw)) = fact.known_value() else {
                        return Err(invalid("spatial citizen native identity is unavailable"));
                    };
                    let native = u32::try_from(*raw)
                        .ok()
                        .filter(|id| *id <= i32::MAX as u32)
                        .ok_or_else(|| {
                            invalid("spatial citizen native identity is out of range")
                        })?;
                    if citizen_entity_id(native) != canonical_id {
                        return Err(invalid(
                            "spatial citizen ID disagrees with its native identity",
                        ));
                    }
                    (
                        native,
                        "native_unit_id",
                        "spatial/1.8.citizen.unit_id",
                        Value::U64(*raw),
                    )
                }
            };
            self.require_fact(canonical_id, field, Some(source_name), value)?;
            out.push(NativeUnitIdentity {
                canonical_id,
                canonical_generation: entity.generation,
                native_id,
            });
        }
        out.sort_by_key(|unit| unit.native_id);
        if out
            .windows(2)
            .any(|pair| pair[0].native_id == pair[1].native_id)
        {
            return Err(invalid(
                "multiple canonical units resolve to one native identity",
            ));
        }
        Ok(out)
    }

    fn require_fact(
        &self,
        entity_id: EntityId,
        field: &str,
        source_name: Option<&str>,
        value: Value,
    ) -> Result<()> {
        let snapshot = self.evidence.snapshot();
        let fact = snapshot
            .graph
            .entities
            .get(&entity_id)
            .and_then(|entity| entity.fields.get(field))
            .ok_or_else(|| invalid("required live identity or item fact is absent"))?;
        if fact.source_digest != self.source_digest
            || fact.observed_at != snapshot.tick
            || !matches!(&fact.source, FactSource::DfhackField(name)
                if source_name.is_none_or(|expected| name == expected))
            || !self.evidence.establishes(&Predicate::FieldCompare {
                entity_id,
                field: field.to_owned(),
                op: CompareOp::Eq,
                value,
            })?
        {
            return Err(invalid(
                "required live fact lacks eligible exact-source evidence",
            ));
        }
        Ok(())
    }

    fn native_capture(
        &self,
        folder: &str,
        site: u32,
        generation: u64,
        tick: u64,
        paused: bool,
    ) -> Result<()> {
        if folder != self.folder
            || site != self.site
            || generation != self.generation
            || tick != self.anchor().tick.get()
            || !paused
            || !self.evidence.establishes(&Predicate::Paused(true))?
        {
            return Err(stale(
                "native resolution requires the same fortress, generation and observed paused tick",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeUnitIdentity {
    pub canonical_id: EntityId,
    pub canonical_generation: u32,
    pub native_id: u32,
}

/// Exact native membership candidate with its original semantic/source binding.
/// It must not be treated as a committed effect or a single-labor success proof.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedWorkforceRoute {
    semantic_plan: Digest32,
    step: StepId,
    anchor: StateAnchor,
    source_digest: Digest32,
    units: Vec<NativeUnitIdentity>,
    labor_column: usize,
    native_plan: AssignmentPlan,
}

impl ResolvedWorkforceRoute {
    #[must_use]
    pub const fn semantic_plan_digest(&self) -> Digest32 {
        self.semantic_plan
    }
    #[must_use]
    pub const fn step(&self) -> StepId {
        self.step
    }
    #[must_use]
    pub const fn anchor(&self) -> StateAnchor {
        self.anchor
    }
    #[must_use]
    pub const fn source_digest(&self) -> Digest32 {
        self.source_digest
    }
    #[must_use]
    pub fn units(&self) -> &[NativeUnitIdentity] {
        &self.units
    }
    #[must_use]
    pub fn native_plan(&self) -> &AssignmentPlan {
        &self.native_plan
    }

    /// Native membership Applied permits other labor recomputation and removal
    /// may leave the requested labor enabled. Require the original single-labor
    /// semantics independently; unknown or broader readback cannot prove them.
    pub fn verify_labor_effect(&self, effect: &AssignmentEffect) -> Result<()> {
        let effect = AssignmentEffect::decode(effect.canonical_bytes(), &self.native_plan)?;
        if effect.phase() != AssignmentPhase::Applied {
            return Err(invalid(
                "native membership outcome does not prove a labor change",
            ));
        }
        for (before, after) in self
            .native_plan
            .before()
            .citizens()
            .iter()
            .zip(effect.post_citizens())
        {
            if before.id() != after.id()
                || before.historical_id() != after.historical_id()
                || before.labors().iter().zip(after.labors()).enumerate().any(
                    |(column, (old, new))| {
                        if column == self.labor_column {
                            *new != u8::from(self.native_plan.spec().assigned())
                        } else {
                            old != new
                        }
                    },
                )
            {
                return Err(invalid(
                    "work-detail readback does not preserve the original single-labor action",
                ));
            }
        }
        Ok(())
    }
}

fn selected_step<'a>(
    plan: &'a PreparedPlan,
    step_id: StepId,
    evidence: &LiveRoutingEvidence<'_>,
) -> Result<&'a PlanStep> {
    plan.validate_structure()?;
    if plan.anchor != evidence.anchor() {
        return Err(stale(
            "semantic plan and routing evidence name different canonical anchors",
        ));
    }
    let step = plan
        .steps
        .iter()
        .find(|step| step.id == step_id)
        .ok_or_else(|| invalid("semantic plan does not contain the selected step"))?;
    super::route_shape(&step.action).map_err(|refusal| invalid(refusal.reason))?;
    for predicate in &step.preconditions {
        if !evidence.evidence.establishes(predicate)? {
            return Err(invalid(
                "semantic step precondition is false or unknown at resolution",
            ));
        }
    }
    Ok(step)
}

/// Resolve a single-labor request without broadening it to a multi-labor detail.
/// The complete selected citizen set must exactly match the native capture.
pub fn resolve_workforce_step(
    plan: &PreparedPlan,
    step_id: StepId,
    evidence: &LiveRoutingEvidence<'_>,
    capture: &WorkforceCapture,
) -> Result<ResolvedWorkforceRoute> {
    let step = selected_step(plan, step_id, evidence)?;
    let Action::SetLabor {
        units,
        labor,
        enabled,
    } = &step.action
    else {
        return Err(invalid("selected step is not a single-labor action"));
    };
    evidence.native_capture(
        capture.folder(),
        capture.site(),
        capture.generation(),
        capture.tick(),
        capture.paused(),
    )?;
    let units = evidence.resolve_units(units)?;
    if units.iter().map(|unit| unit.native_id).collect::<Vec<_>>() != capture.ids()
        || !capture.automatic()
        || capture.citizens().iter().any(|unit| !unit.eligible())
    {
        return Err(invalid(
            "native workforce selection or eligibility differs from the semantic units",
        ));
    }
    let column = capture
        .labor_keys()
        .iter()
        .position(|key| key == labor)
        .ok_or_else(|| invalid("requested labor is absent from the native labor-key schema"))?;
    let matches = capture
        .details()
        .iter()
        .enumerate()
        .filter(|(_, detail)| {
            detail.selected_only()
                && detail
                    .labors()
                    .iter()
                    .enumerate()
                    .all(|(index, enabled)| *enabled == u8::from(index == column))
        })
        .collect::<Vec<_>>();
    // Detail indices are capture-local; even two superficially equal candidates
    // are ambiguous and may have different ownership/membership intent.
    if matches.len() != 1 {
        return Err(invalid(
            "single-labor action needs one unambiguous selected-only detail containing only that labor",
        ));
    }
    let (detail_index, _) = matches[0];
    if !enabled
        && capture.details().iter().enumerate().any(|(index, detail)| {
            index != detail_index
                && detail.labors()[column] != 0
                && (!detail.selected_only()
                    || units
                        .iter()
                        .any(|unit| detail.members().binary_search(&unit.native_id).is_ok()))
        })
    {
        return Err(invalid(
            "another work detail may keep the requested labor enabled",
        ));
    }
    let detail = u32::try_from(detail_index)
        .map_err(|_| invalid("native work-detail index exceeds bounds"))?;
    let native_plan = AssignmentPlan::new(
        &step.idempotency_key,
        AssignmentSpec::new(detail, *enabled)?,
        capture.clone(),
    )?;
    Ok(ResolvedWorkforceRoute {
        semantic_plan: plan.digest,
        step: step_id,
        anchor: evidence.anchor(),
        source_digest: evidence.source_digest,
        units,
        labor_column: column,
        native_plan,
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedFurnitureRoute {
    semantic_plan: Digest32,
    step: StepId,
    anchor: StateAnchor,
    source_digest: Digest32,
    native_plan: BuildPlan,
}

impl ResolvedFurnitureRoute {
    #[must_use]
    pub const fn semantic_plan_digest(&self) -> Digest32 {
        self.semantic_plan
    }
    #[must_use]
    pub const fn step(&self) -> StepId {
        self.step
    }
    #[must_use]
    pub const fn anchor(&self) -> StateAnchor {
        self.anchor
    }
    #[must_use]
    pub const fn source_digest(&self) -> Digest32 {
        self.source_digest
    }
    #[must_use]
    pub fn native_plan(&self) -> &BuildPlan {
        &self.native_plan
    }
}

/// Bind one exact eligible furniture item to the original semantic step. The
/// current capture has numeric material IDs but no admitted token dictionary,
/// complete nearest-item comparison, or resource reservation proof. Such semantic
/// constraints are explicitly refused, never erased or silently approximated.
pub fn resolve_furniture_step(
    plan: &PreparedPlan,
    step_id: StepId,
    evidence: &LiveRoutingEvidence<'_>,
    capture: &BuildCapture,
) -> Result<ResolvedFurnitureRoute> {
    let step = selected_step(plan, step_id, evidence)?;
    let routed = route_step(step)
        .outcome
        .map_err(|refusal| invalid(refusal.reason))?;
    let LiveRequest::Furniture {
        kind,
        target,
        material,
    } = routed.request
    else {
        return Err(invalid("selected step is not a supported furniture action"));
    };
    if material != MaterialSelector::default() {
        return Err(invalid(
            "build/1.19 cannot establish material tokens, nearest-item policy or reservations; the original selector is retained and unresolved",
        ));
    }
    if evidence.schema != LiveIdentitySchema::SpatialV1_8 {
        return Err(invalid(
            "furniture resolution requires an exact spatial/1.8 item projection",
        ));
    }
    evidence.native_capture(
        capture.folder(),
        capture.site(),
        capture.generation(),
        capture.tick(),
        capture.paused(),
    )?;
    let selection = capture.selection();
    if selection.kind() != kind
        || selection.target() != target
        || evidence.map_dimensions != Some(capture.dimensions())
        || !capture.eligible()
    {
        return Err(invalid(
            "native furniture capture does not match the eligible original selection",
        ));
    }
    let BuildItem::Visible(item) = capture.item() else {
        return Err(invalid("selected furniture item is not observed"));
    };
    let canonical_id = item_entity_id(selection.item_id());
    if !evidence.evidence.establishes(&Predicate::EntityKind {
        entity_id: canonical_id,
        kind: EntityKind::Item,
    })? {
        return Err(invalid("selected canonical furniture item is not observed"));
    }
    for (field, value) in [
        ("native_item_id", Value::U64(u64::from(selection.item_id()))),
        ("item_type", Value::I64(i64::from(item.native_type()))),
        ("subtype", Value::I64(i64::from(item.subtype()))),
        ("material_type", Value::I64(i64::from(item.material()))),
        (
            "material_index",
            Value::I64(i64::from(item.material_index())),
        ),
        (
            "raw_position",
            Value::Coord(dfmcp_core::MapCoord::new(
                item.position()[0] as i32,
                item.position()[1] as i32,
                item.position()[2] as i32,
            )),
        ),
        ("on_ground", Value::Bool(true)),
        ("in_job", Value::Bool(false)),
    ] {
        evidence.require_fact(canonical_id, field, None, value)?;
    }
    Ok(ResolvedFurnitureRoute {
        semantic_plan: plan.digest,
        step: step_id,
        anchor: evidence.anchor(),
        source_digest: evidence.source_digest,
        native_plan: BuildPlan::new(&step.idempotency_key, capture.clone())?,
    })
}

#[cfg(test)]
#[path = "live_routing_resolution_tests.rs"]
mod tests;
