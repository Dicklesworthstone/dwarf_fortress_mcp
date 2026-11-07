//! Source-qualified current labor for an order that needs no assignment effect.
//!
//! The original terminal predicate retains the exact citizen identities and
//! positive labor requirement. Neither this projection nor a true predicate is
//! an Applied workforce receipt or authority to change a work detail.

use std::collections::BTreeMap;

use dfmcp_core::{EntityId, ObservationCursor, StateAnchor};
use dfmcp_intent::Action;
use dfmcp_world::{
    CompareOp, EntityKind, EntityRecord, EvidenceCoverage, EvidencePolicy, EvidenceSource, Fact,
    FactSource, Predicate, PredicateEvidence, PredicateTruth, Value, WorldGraph, WorldSnapshot,
};

use crate::live_routing::LiveIdentitySchema;
use crate::semantic_work_orders::{ObservedInventoryObjective, RetainedOrderObjective};
use crate::semantic_workforce::projection::{
    ELIGIBILITY_SOURCE, HISTORICAL_ID_SOURCE, LABOR_SOURCE, UNIT_ID_SOURCE,
};
use crate::workforce_control::WorkforceCapture;

use super::*;

const PROJECTION_BYTES: u64 = 16 * MIB;

/// Preserve the established direct semantic-goal read allowance, then pay for
/// this wrapper's own call, selected projection work, and live final check. The
/// whole allowance is reserved by its caller before either native read starts.
pub(super) fn observation_bytes(view_cost: u64) -> Result<u64> {
    workforce_operation_bytes(view_cost)?
        .checked_add(16 * MIB + PROJECTION_BYTES + LOCAL_BYTES + 1)
        .ok_or_else(exhausted)
}

fn refused(message: &str) -> DfmcpError {
    error(ErrorCode::AdapterRejected, message)
}

/// A closed requirement, not source authority. Native historical identities
/// are sealed in the original parent alongside the positive labor predicate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExistingLaborRequirement {
    labor: String,
    units: Vec<(EntityId, u32)>,
}

impl ExistingLaborRequirement {
    pub fn new(labor: String, mut units: Vec<(EntityId, u32)>) -> Result<Self> {
        if labor.is_empty()
            || labor.len() > 64
            || !labor
                .bytes()
                .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
            || units.is_empty()
            || units.len() > 32
            || units
                .iter()
                .any(|(id, historical)| *id == EntityId::NIL || *historical > i32::MAX as u32)
        {
            return Err(refused(
                "existing labor requires one exact labor and 1..32 named citizens",
            ));
        }
        units.sort_unstable();
        if units.windows(2).any(|pair| pair[0].0 == pair[1].0) {
            return Err(refused(
                "existing labor citizen identities must be distinct",
            ));
        }
        Ok(Self { labor, units })
    }

    pub fn units(&self) -> Vec<EntityId> {
        self.units.iter().map(|(id, _)| *id).collect()
    }

    pub fn labor(&self) -> &str {
        &self.labor
    }

    pub fn predicate(&self) -> Predicate {
        Predicate::All(
            self.units
                .iter()
                .flat_map(|(id, historical)| {
                    [
                        self.field(*id, &format!("labor.{}", self.labor), Value::Bool(true)),
                        self.field(
                            *id,
                            "historical_figure_id",
                            Value::U64(u64::from(*historical)),
                        ),
                        self.field(*id, "workforce_eligible", Value::Bool(true)),
                    ]
                })
                .collect(),
        )
        .normalized()
    }

    fn field(&self, id: EntityId, field: &str, value: Value) -> Predicate {
        Predicate::FieldCompare {
            entity_id: id,
            field: field.to_owned(),
            op: CompareOp::Eq,
            value,
        }
    }

    /// Recognize the complete sealed shape, never a caller-supplied flag. A
    /// partial or malformed labor-bearing single-order plan refuses instead of
    /// silently becoming an ordinary unguarded order.
    pub fn from_plan(plan: &PreparedPlan) -> Result<Option<Self>> {
        // Public callers need the same bounded visitor before normalization,
        // digest validation or recursive traversal as the enclosing executor.
        // Archive encoding does not call the production policy, avoiding a
        // cycle when that policy asks us to recognize this closed shape.
        dfmcp_intent::plan_archive::encode(plan, archive_limits())?;
        if plan.steps.len() != 1 || !matches!(plan.steps[0].action, Action::CreateWorkOrder { .. })
        {
            return Ok(None);
        }
        let mut parent = Vec::new();
        atoms(&plan.terminal_condition, &mut parent);
        if !has_labor_requirement(&plan.terminal_condition) {
            return Ok(None);
        }
        let step = &plan.steps[0];
        if step.id != StepId::ZERO
            || !step.depends_on.is_empty()
            || (ObservedInventoryObjective::from_selected_plan(plan, step.id).is_none()
                && RetainedOrderObjective::from_selected_plan(plan, step.id).is_none())
        {
            return Err(refused(
                "existing labor requires one original closed finite-order objective",
            ));
        }
        let terminal = &step.obligation.as_ref().ok_or_else(custody)?.terminal;
        let mut order = Vec::new();
        atoms(terminal, &mut order);
        for predicate in order {
            let at = parent
                .iter()
                .position(|value| *value == predicate)
                .ok_or_else(|| refused("parent omits an original order requirement"))?;
            parent.remove(at);
        }
        let mut labor = None;
        let mut selected = BTreeMap::<EntityId, (bool, bool, Option<u32>)>::new();
        for predicate in parent {
            let Predicate::FieldCompare {
                entity_id,
                field,
                op: CompareOp::Eq,
                value,
            } = predicate
            else {
                return Err(refused(
                    "existing labor parent contains an unsupported requirement",
                ));
            };
            let row = selected.entry(*entity_id).or_default();
            match (field.as_str(), value) {
                (field, Value::Bool(true)) if field.starts_with("labor.") && !row.0 => {
                    let key = field.strip_prefix("labor.").ok_or_else(custody)?;
                    if labor.as_ref().is_some_and(|old| old != key) {
                        return Err(refused("existing labor must name one exact labor"));
                    }
                    labor = Some(key.to_owned());
                    row.0 = true;
                }
                ("workforce_eligible", Value::Bool(true)) if !row.1 => row.1 = true,
                ("historical_figure_id", Value::U64(value)) if row.2.is_none() => {
                    row.2 = Some(
                        u32::try_from(*value)
                            .map_err(|_| refused("historical identity exceeds bounds"))?,
                    );
                }
                _ => {
                    return Err(refused(
                        "existing labor requires exact positive labor and eligibility plus historical identity",
                    ));
                }
            }
        }
        let units = selected
            .into_iter()
            .map(|(id, (enabled, eligible, historical))| {
                if !enabled || !eligible {
                    return Err(refused("existing labor citizen requirement is incomplete"));
                }
                Ok((
                    id,
                    historical
                        .ok_or_else(|| refused("existing labor historical identity is absent"))?,
                ))
            })
            .collect::<Result<Vec<_>>>()?;
        let requirement = Self::new(
            labor.ok_or_else(|| refused("existing labor key is absent"))?,
            units,
        )?;
        if plan.terminal_condition
            != Predicate::All(vec![requirement.predicate(), terminal.clone()]).normalized()
        {
            return Err(refused(
                "existing labor parent differs from its complete declared requirement",
            ));
        }
        Ok(Some(requirement))
    }
}

fn has_labor_requirement(predicate: &Predicate) -> bool {
    let labor_field = |field: &str| {
        field.starts_with("labor.")
            || matches!(field, "historical_figure_id" | "workforce_eligible")
    };
    match predicate {
        Predicate::FieldCompare { field, .. } => labor_field(field),
        Predicate::EntityQuantity {
            filter_field,
            quantity_field,
            ..
        } => labor_field(filter_field) || quantity_field.as_deref().is_some_and(labor_field),
        Predicate::All(children) | Predicate::Any(children) => {
            children.iter().any(has_labor_requirement)
        }
        Predicate::Not(inner) => has_labor_requirement(inner),
        _ => false,
    }
}

fn atoms<'a>(predicate: &'a Predicate, out: &mut Vec<&'a Predicate>) {
    if let Predicate::All(children) = predicate {
        for child in children {
            atoms(child, out);
        }
    } else {
        out.push(predicate);
    }
}

/// Fresh selected workforce facts. This value has no native assignment plan,
/// semantic assignment review, effect receipt or mutation permit.
#[derive(Clone, Debug)]
pub struct ExistingLaborProjection {
    snapshot: WorldSnapshot,
    capture: WorkforceCapture,
    original_anchor: StateAnchor,
    selection_seal: Digest32,
    requirement: ExistingLaborRequirement,
}

impl ExistingLaborProjection {
    pub fn new(
        original: &PreparedPlan,
        identity: &LiveRoutingEvidence<'_>,
        capture: WorkforceCapture,
        cursor: ObservationCursor,
    ) -> Result<Self> {
        let requirement = ExistingLaborRequirement::from_plan(original)?
            .ok_or_else(|| refused("original plan has no closed existing-labor requirement"))?;
        let units = identity.resolve_units(&requirement.units())?;
        if identity.anchor() != original.anchor
            || capture.fortress_id() != original.anchor.fortress_id
            || capture.tick() < original.anchor.tick.get()
            || !capture.paused()
            || cursor.epoch != original.anchor.cursor.epoch
            || cursor.sequence < original.anchor.cursor.sequence
            || units
                .iter()
                .any(|unit| capture.ids().binary_search(&unit.native_id).is_err())
            || !capture.labor_keys().contains(&requirement.labor)
        {
            return Err(error(
                ErrorCode::StaleAnchor,
                "existing labor differs from its original selected identity or paused source",
            ));
        }
        let tick = GameTick(capture.tick());
        let digest = capture.witness();
        let mut entities = BTreeMap::new();
        for unit in &units {
            let citizen = capture
                .citizens()
                .iter()
                .find(|citizen| citizen.id() == unit.native_id)
                .ok_or_else(custody)?;
            if requirement
                .units
                .iter()
                .find(|(id, _)| *id == unit.canonical_id)
                .map(|(_, historical)| *historical)
                != Some(citizen.historical_id())
            {
                return Err(error(
                    ErrorCode::StaleAnchor,
                    "existing labor replaced an original citizen historical identity",
                ));
            }
            let (field, value) = match identity.schema() {
                LiveIdentitySchema::CitizensV1 => {
                    ("raw_unit_id", Value::I64(i64::from(citizen.id())))
                }
                LiveIdentitySchema::SpatialV1_8 => {
                    ("native_unit_id", Value::U64(u64::from(citizen.id())))
                }
                LiveIdentitySchema::DigTerrainV1_8 => {
                    return Err(refused("terrain cannot supply existing worker identities"));
                }
            };
            let observed = |value, source: &str| {
                Fact::known(
                    value,
                    tick,
                    FactSource::DfhackField(source.to_owned()),
                    digest,
                )
            };
            let mut fields = BTreeMap::from([
                (field.to_owned(), observed(value, UNIT_ID_SOURCE)),
                (
                    "historical_figure_id".to_owned(),
                    observed(
                        Value::U64(u64::from(citizen.historical_id())),
                        HISTORICAL_ID_SOURCE,
                    ),
                ),
                (
                    "workforce_eligible".to_owned(),
                    observed(Value::Bool(citizen.eligible()), ELIGIBILITY_SOURCE),
                ),
            ]);
            for (key, bit) in capture.labor_keys().iter().zip(citizen.labors()) {
                fields.insert(
                    format!("labor.{key}"),
                    observed(Value::Bool(*bit == 1), LABOR_SOURCE),
                );
            }
            entities.insert(
                unit.canonical_id,
                EntityRecord {
                    id: unit.canonical_id,
                    generation: unit.canonical_generation,
                    revision: cursor.sequence,
                    kind: EntityKind::Unit,
                    label: format!("citizen {}", citizen.id()),
                    fields,
                },
            );
        }
        let snapshot = WorldSnapshot::new(
            original.anchor.fortress_id,
            tick,
            cursor,
            true,
            WorldGraph {
                entities,
                ..WorldGraph::default()
            },
        );
        let mut identity_bytes = b"dfmcp-existing-labor-selection/1\0".to_vec();
        identity_bytes.extend_from_slice(original.digest.as_bytes());
        identity_bytes.extend_from_slice(identity.source_digest().as_bytes());
        identity_bytes.extend_from_slice(&requirement.predicate().canonical_bytes());
        Ok(Self {
            snapshot,
            capture,
            original_anchor: original.anchor,
            selection_seal: Digest32::of_bytes(&identity_bytes),
            requirement,
        })
    }

    pub fn snapshot(&self) -> &WorldSnapshot {
        &self.snapshot
    }
    pub fn capture(&self) -> &WorkforceCapture {
        &self.capture
    }
    pub fn anchor(&self) -> StateAnchor {
        self.snapshot.anchor()
    }
    pub fn source_digest(&self) -> Digest32 {
        self.capture.witness()
    }
    pub fn original_anchor(&self) -> StateAnchor {
        self.original_anchor
    }
    pub fn selection_seal(&self) -> Digest32 {
        self.selection_seal
    }
    pub fn requirement(&self) -> &ExistingLaborRequirement {
        &self.requirement
    }

    pub fn evidence(&self, policy: EvidencePolicy) -> Result<PredicateEvidence<'_>> {
        if policy.all_entities == EvidenceCoverage::Complete
            || policy.entity_kinds.iter().any(|(kind, coverage)| *kind != EntityKind::Unit || *coverage == EvidenceCoverage::Complete)
            || policy.all_edges != EvidenceCoverage::Unknown
            || !policy.edge_kinds.is_empty()
            || !policy.terrain_regions.is_empty()
            || policy.sources.iter().any(|source| !matches!(source, EvidenceSource::Observed { field, source_digest }
                if *source_digest == self.source_digest() && [UNIT_ID_SOURCE, HISTORICAL_ID_SOURCE, ELIGIBILITY_SOURCE, LABOR_SOURCE].contains(&field.as_str())))
        {
            return Err(error(ErrorCode::CapabilityDenied, "existing labor policy exceeds its exact observed selected-citizen domain"));
        }
        PredicateEvidence::scoped(&self.snapshot, policy)
    }
}

pub(super) fn retain_floor(
    previous: &mut Option<WorkforceCapture>,
    fenced: &mut bool,
    high_tick: &mut u64,
    capture: &WorkforceCapture,
) -> Result<()> {
    *high_tick = (*high_tick).max(capture.tick());
    if previous.as_ref().is_some_and(|old| {
        capture.generation() != old.generation()
            || capture.sequence() < old.sequence()
            || capture.tick() < old.tick()
            || capture.folder() != old.folder()
            || capture.site() != old.site()
            || capture.labor_keys() != old.labor_keys()
            || capture.ids() != old.ids()
            || capture
                .citizens()
                .iter()
                .zip(old.citizens())
                .any(|(a, b)| a.historical_id() != b.historical_id())
    }) {
        *fenced = true;
        return Err(error(
            ErrorCode::StaleAnchor,
            "existing labor source regressed or changed its selected identity",
        ));
    }
    *previous = Some(capture.clone());
    Ok(())
}

pub(super) fn require_true(
    projection: &ExistingLaborProjection,
    policy: EvidencePolicy,
) -> Result<()> {
    if projection
        .evidence(policy)?
        .evaluate(&projection.requirement().predicate())?
        != PredicateTruth::True
    {
        return Err(error(
            ErrorCode::CapabilityDenied,
            "original existing labor prerequisite is not currently proved true",
        ));
    }
    Ok(())
}

pub(super) fn observe<WJ, WA, F>(
    original: &PreparedPlan,
    identity: &LiveRoutingEvidence<'_>,
    workforce: &mut SemanticWorkforceSession<WJ, WA>,
    factory: &mut F,
    cursor: ObservationCursor,
    context: &OperationContext,
    floor: &mut Option<WorkforceCapture>,
    fenced: &mut bool,
    high_tick: &mut u64,
) -> Result<ExistingLaborProjection>
where
    WJ: EffectJournalStorage,
    WA: EffectJournalStorage,
    F: WorkforceFactory,
{
    let requirement = ExistingLaborRequirement::from_plan(original)?.ok_or_else(custody)?;
    let units = requirement.units();
    let ids = identity
        .resolve_units(&units)?
        .iter()
        .map(|unit| unit.native_id)
        .collect::<Vec<_>>();
    let capture = capture(
        &ids,
        &units,
        workforce,
        factory,
        context,
        (floor, fenced, high_tick),
    )?;
    ExistingLaborProjection::new(original, identity, capture, cursor)
}

/// Close the prerequisite bracket without borrowing another canonical routing
/// view. The complete matching witness is the same source-qualified projection;
/// it supplies no new fact, policy grant or native assignment authority.
pub(super) fn observe_matching<WJ, WA, F>(
    expected: &ExistingLaborProjection,
    workforce: &mut SemanticWorkforceSession<WJ, WA>,
    factory: &mut F,
    context: &OperationContext,
    floor: &mut Option<WorkforceCapture>,
    fenced: &mut bool,
    high_tick: &mut u64,
) -> Result<()>
where
    WJ: EffectJournalStorage,
    WA: EffectJournalStorage,
    F: WorkforceFactory,
{
    let capture = capture(
        &expected.capture().ids(),
        &expected.requirement().units(),
        workforce,
        factory,
        context,
        (floor, fenced, high_tick),
    )?;
    if &capture != expected.capture() {
        return Err(error(
            ErrorCode::StaleAnchor,
            "existing labor changed across the final original identity refresh",
        ));
    }
    Ok(())
}

fn capture<WJ, WA, F>(
    ids: &[u32],
    units: &[EntityId],
    workforce: &mut SemanticWorkforceSession<WJ, WA>,
    factory: &mut F,
    context: &OperationContext,
    retained: (&mut Option<WorkforceCapture>, &mut bool, &mut u64),
) -> Result<WorkforceCapture>
where
    WJ: EffectJournalStorage,
    WA: EffectJournalStorage,
    F: WorkforceFactory,
{
    let (floor, fenced, high_tick) = retained;
    if *fenced {
        return Err(error(
            ErrorCode::StaleAnchor,
            "existing labor source lineage is fenced",
        ));
    }
    if units.len() > context.budget.max_entities as usize {
        return Err(exhausted());
    }
    context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    context.authorize(Capability::Observe, RiskTier::ReadOnly, units, None)?;
    let mut call = Call::new(context, *high_tick)?;
    call.take(PROJECTION_BYTES)?;
    let reading = call.take(call.bytes.checked_sub(1).ok_or_else(exhausted)?)?;
    let acquired = workforce.observe_existing_labor_capture(
        ids,
        &reading,
        (
            |binding, current| factory.connect(binding, current),
            |capture| retain_floor(floor, fenced, high_tick, capture),
        ),
    );
    *high_tick = (*high_tick).max(workforce.high_tick());
    call.floor(*high_tick);
    let capture = acquired?;
    if capture.ids() != ids || capture.tick() < call.current()?.anchor.tick.get() {
        return Err(error(
            ErrorCode::StaleAnchor,
            "existing labor capture differs from its requested current selection",
        ));
    }
    call.current()?
        .authorize(Capability::Observe, RiskTier::ReadOnly, units, None)?;
    Ok(capture)
}
