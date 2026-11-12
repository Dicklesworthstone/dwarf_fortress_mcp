//! Complete immutable source and target vector for a finite stock portfolio.
//! This version is disjoint from the single Stock target/1 and /2 encodings.
use super::request::recipe;
use super::workshop_binding;
use crate::semantic_workforce_runtime::{digest, invalid};
use dfmcp_adapter::live_production::{ExistingLaborRequirement, ProductionLayout};
use dfmcp_adapter::semantic_work_orders::{
    ObservedInventoryIncreaseObjective, StockTargetAssessment,
};
use dfmcp_adapter::work_orders::WorkOrderRecipe;
use dfmcp_core::{Digest32, GameTick, IntentId, Result, StateAnchor, StepId};
use dfmcp_intent::{Action, PreparedPlan, derive_step_idempotency_key};
use dfmcp_world::{CompareOp, EntityKind, Predicate, Value};

const PREFIX: &str = "Inventory targets/1 ";
const ASSIGNMENT_PREFIX: &str = "Inventory targets/2 ";
const RESERVED: &str = "Inventory targets";
const ACTION_TAG: &str = "inventory-targets/";
const DOMAIN: &[u8] = b"dfmcp-native-stock-targets-portfolio/1\0";
const ASSIGNMENT_DOMAIN: &[u8] = b"dfmcp-native-assigned-stock-targets-portfolio/2\0";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct TargetRow {
    pub recipe: WorkOrderRecipe,
    pub target: u64,
    pub maximum: u32,
    pub baseline: u64,
    pub deficit: u64,
    pub witness: Digest32,
}

impl TargetRow {
    pub(super) fn item_type(&self) -> &'static str {
        match self.recipe {
            WorkOrderRecipe::WoodenBed => "BED",
            WorkOrderRecipe::WoodenDoor => "DOOR",
            WorkOrderRecipe::WoodenTable => "TABLE",
            WorkOrderRecipe::WoodenChair => "CHAIR",
        }
    }
    pub(super) fn order_name(&self, assign_labor: bool) -> String {
        let version = if assign_labor { 2 } else { 1 };
        format!(
            "native inventory-targets/{version} {} goal",
            self.recipe.as_str()
        )
    }
    pub(super) fn minimum(&self) -> Predicate {
        Predicate::EntityQuantity {
            kind: EntityKind::Item,
            filter_field: "type_key".into(),
            filter_value: Value::Text(self.item_type().into()),
            quantity_field: Some("stack_size".into()),
            op: CompareOp::Ge,
            value: self.target,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct StockTargetsBinding {
    pub rows: Vec<TargetRow>,
    pub original_anchor: StateAnchor,
    pub source_digest: Digest32,
    pub source_generation: u64,
    pub source_counters: [u64; 3],
    pub item_rows: u32,
    assign_labor: bool,
}

fn append_anchor(bytes: &mut Vec<u8>, anchor: StateAnchor) {
    bytes.extend_from_slice(&anchor.fortress_id.get().to_be_bytes());
    bytes.extend_from_slice(&anchor.cursor.epoch.to_be_bytes());
    bytes.extend_from_slice(&anchor.cursor.sequence.to_be_bytes());
    bytes.extend_from_slice(&anchor.tick.get().to_be_bytes());
    bytes.extend_from_slice(anchor.state_hash.as_bytes());
}

impl StockTargetsBinding {
    pub(super) fn from_assessments(values: &[StockTargetAssessment]) -> Result<Self> {
        let first = values
            .first()
            .ok_or_else(|| invalid("stock-target portfolio has no qualified census"))?;
        if values.len() > 4
            || values.iter().any(|value| {
                value.original_anchor() != first.original_anchor()
                    || value.source_digest() != first.source_digest()
                    || value.source_generation() != first.source_generation()
                    || value.source_counters() != first.source_counters()
                    || value.item_rows() != first.item_rows()
            })
        {
            return Err(invalid(
                "stock-target portfolio must use one complete original census",
            ));
        }
        let value = Self {
            rows: values
                .iter()
                .map(|value| TargetRow {
                    recipe: value.recipe(),
                    target: value.target_stack_units(),
                    maximum: value.max_order_amount(),
                    baseline: value.baseline_stack_units(),
                    deficit: value.deficit_stack_units(),
                    witness: value.witness(),
                })
                .collect(),
            original_anchor: first.original_anchor(),
            source_digest: first.source_digest(),
            source_generation: first.source_generation(),
            source_counters: first.source_counters(),
            item_rows: first.item_rows(),
            assign_labor: false,
        };
        value.validate()?;
        Ok(value)
    }

    pub(super) fn from_assessments_with_assignment(
        values: &[StockTargetAssessment],
    ) -> Result<Self> {
        let mut value = Self::from_assessments(values)?;
        value.assign_labor = true;
        Ok(value)
    }

    pub(super) const fn assign_labor(&self) -> bool {
        self.assign_labor
    }

    fn domain(&self) -> &'static [u8] {
        if self.assign_labor {
            ASSIGNMENT_DOMAIN
        } else {
            DOMAIN
        }
    }

    /// Reconstruct the existing adapter's assessment checksum from the complete
    /// retained fields, including met targets that have no positive predicate.
    /// Checksums are identity only; native dispatch always requalifies the source.
    fn row_witness(&self, row: &TargetRow) -> Digest32 {
        let mut bytes = b"dfmcp-native-stock-target-assessment-v1\0".to_vec();
        bytes.push(row.recipe as u8);
        bytes.extend_from_slice(&row.target.to_be_bytes());
        bytes.extend_from_slice(&row.maximum.to_be_bytes());
        bytes.extend_from_slice(&row.baseline.to_be_bytes());
        bytes.extend_from_slice(&row.deficit.to_be_bytes());
        append_anchor(&mut bytes, self.original_anchor);
        bytes.extend_from_slice(self.source_digest.as_bytes());
        bytes.extend_from_slice(&self.source_generation.to_be_bytes());
        for counter in self.source_counters {
            bytes.extend_from_slice(&counter.to_be_bytes());
        }
        bytes.extend_from_slice(&self.item_rows.to_be_bytes());
        Digest32::of_bytes(&bytes)
    }

    fn validate(&self) -> Result<()> {
        if self.rows.is_empty()
            || self.rows.len() > 4
            || self
                .rows
                .windows(2)
                .any(|pair| pair[0].recipe.as_str() >= pair[1].recipe.as_str())
            || self.source_digest == Digest32::ZERO
            || self.source_generation == 0
            || self.source_generation == u64::MAX
            || self
                .source_counters
                .iter()
                .any(|counter| *counter > i32::MAX as u64)
            || self.rows.iter().any(|row| {
                !(1..=1_000_000).contains(&row.target)
                    || !(1..=100).contains(&row.maximum)
                    || row.deficit != row.target.saturating_sub(row.baseline)
                    || row.deficit > u64::from(row.maximum)
                    || row.witness == Digest32::ZERO
                    || row.witness != self.row_witness(row)
            })
        {
            return Err(invalid(
                "stock-target portfolio has invalid source, sizing or identity",
            ));
        }
        for minimum in self.minima() {
            minimum.validate_shape()?;
        }
        Ok(())
    }

    pub(super) fn minima(&self) -> Vec<Predicate> {
        self.rows.iter().map(TargetRow::minimum).collect()
    }

    pub(super) fn summary(&self) -> Result<String> {
        self.validate()?;
        let prefix = if self.assign_labor {
            ASSIGNMENT_PREFIX
        } else {
            PREFIX
        };
        let rows: Vec<_> = self
            .rows
            .iter()
            .map(|row| {
                format!(
                    "{}:{}:{}:{}:{}:{}",
                    row.recipe.as_str(),
                    row.target,
                    row.maximum,
                    row.baseline,
                    row.deficit,
                    row.witness.to_hex(),
                )
            })
            .collect();
        Ok(format!(
            "{prefix}{} {} {},{},{} {} {}",
            self.source_digest.to_hex(),
            self.source_generation,
            self.source_counters[0],
            self.source_counters[1],
            self.source_counters[2],
            self.item_rows,
            rows.join("|"),
        ))
    }

    pub(super) fn witness(&self) -> Result<Digest32> {
        let mut bytes = self.domain().to_vec();
        append_anchor(&mut bytes, self.original_anchor);
        bytes.extend_from_slice(self.summary()?.as_bytes());
        Ok(Digest32::of_bytes(&bytes))
    }

    pub(super) fn identity(
        &self,
        workers: &ExistingLaborRequirement,
        deadline: GameTick,
        cadence: u64,
        stability: u32,
    ) -> Result<Vec<u8>> {
        self.validate()?;
        if self.assign_labor {
            return Err(invalid(
                "assigned inventory targets cannot use existing-worker identity",
            ));
        }
        let mut bytes = DOMAIN.to_vec();
        append_anchor(&mut bytes, self.original_anchor);
        let prerequisite = workers.predicate().canonical_bytes();
        bytes.extend_from_slice(&(prerequisite.len() as u32).to_be_bytes());
        bytes.extend_from_slice(&prerequisite);
        let summary = self.summary()?;
        bytes.extend_from_slice(&(summary.len() as u32).to_be_bytes());
        bytes.extend_from_slice(summary.as_bytes());
        bytes.extend_from_slice(&deadline.get().to_be_bytes());
        bytes.extend_from_slice(&cadence.to_be_bytes());
        bytes.extend_from_slice(&stability.to_be_bytes());
        Ok(bytes)
    }

    /// Seal the complete enabled assignment and canonical citizen identity
    /// guard before any original step key is derived. Its domain is disjoint
    /// from the unchanged existing-worker target vector.
    pub(super) fn assignment_identity(
        &self,
        assignment: &Action,
        preconditions: &Predicate,
        deadline: GameTick,
        cadence: u64,
        stability: u32,
    ) -> Result<Vec<u8>> {
        self.validate()?;
        if !self.assign_labor {
            return Err(invalid(
                "existing-worker inventory targets cannot acquire assignment authority",
            ));
        }
        super::stock_target_binding::assignment_goal(assignment, preconditions)?;
        let action = assignment.canonical_bytes();
        let prerequisite = preconditions.normalized().canonical_bytes();
        let summary = self.summary()?;
        let mut bytes = ASSIGNMENT_DOMAIN.to_vec();
        append_anchor(&mut bytes, self.original_anchor);
        bytes.extend_from_slice(&(action.len() as u32).to_be_bytes());
        bytes.extend_from_slice(&action);
        bytes.extend_from_slice(&(prerequisite.len() as u32).to_be_bytes());
        bytes.extend_from_slice(&prerequisite);
        bytes.extend_from_slice(&(summary.len() as u32).to_be_bytes());
        bytes.extend_from_slice(summary.as_bytes());
        bytes.extend_from_slice(&deadline.get().to_be_bytes());
        bytes.extend_from_slice(&cadence.to_be_bytes());
        bytes.extend_from_slice(&stability.to_be_bytes());
        Ok(bytes)
    }

    fn parse(summary: &str, original_anchor: StateAnchor) -> Result<Option<Self>> {
        if !summary.contains(RESERVED) && !summary.contains("dfmcp-native-stock-targets") {
            return Ok(None);
        }
        let (fields, assign_labor) = if let Some(fields) = summary.strip_prefix(PREFIX) {
            (fields, false)
        } else if let Some(fields) = summary.strip_prefix(ASSIGNMENT_PREFIX) {
            (fields, true)
        } else {
            return Err(invalid("unsupported or misplaced inventory targets marker"));
        };
        let fields: Vec<_> = fields.split(' ').collect();
        if fields.len() != 5 || summary.len() > 4096 {
            return Err(invalid(
                "inventory targets marker must retain all original fields",
            ));
        }
        let number = |value: &str| -> Result<u64> {
            value
                .parse::<u64>()
                .ok()
                .filter(|n| n.to_string() == value)
                .ok_or_else(|| invalid("noncanonical inventory targets number"))
        };
        let counters: Vec<_> = fields[2].split(',').map(number).collect::<Result<_>>()?;
        let source_counters: [u64; 3] = counters
            .try_into()
            .map_err(|_| invalid("inventory targets marker lacks allocation counters"))?;
        let raw_rows: Vec<_> = fields[4].split('|').collect();
        if raw_rows.is_empty() || raw_rows.len() > 4 {
            return Err(invalid("inventory targets marker exceeds its target bound"));
        }
        let mut rows = Vec::with_capacity(raw_rows.len());
        for raw in raw_rows {
            let row: Vec<_> = raw.split(':').collect();
            if row.len() != 6 {
                return Err(invalid("inventory targets marker lacks complete sizing"));
            }
            rows.push(TargetRow {
                recipe: recipe(row[0])?.0,
                target: number(row[1])?,
                maximum: u32::try_from(number(row[2])?)
                    .map_err(|_| invalid("inventory target maximum overflow"))?,
                baseline: number(row[3])?,
                deficit: number(row[4])?,
                witness: digest(row[5])?,
            });
        }
        let value = Self {
            rows,
            original_anchor,
            source_digest: digest(fields[0])?,
            source_generation: number(fields[1])?,
            source_counters,
            item_rows: u32::try_from(number(fields[3])?)
                .map_err(|_| invalid("inventory target item-row count overflow"))?,
            assign_labor,
        };
        value.validate()?;
        if value.summary()? != summary {
            return Err(invalid("inventory targets marker is not canonical"));
        }
        Ok(Some(value))
    }
}

/// Recovery cannot strip the target policy, change a met minimum or turn a
/// deficit into a manual amount. Every original leaf and the complete parent
/// must reproduce the separately versioned identity.
pub(super) fn requirement(plan: &PreparedPlan) -> Result<Option<StockTargetsBinding>> {
    plan.validate_structure()?;
    if !plan.digest_is_valid() || !plan.id_is_valid() {
        return Err(invalid(
            "inventory targets require the sealed original plan",
        ));
    }
    let workshop = workshop_binding::requirement(plan)?;
    let body = if workshop.is_some() {
        plan.summary
            .split_once('\n')
            .map(|(_, body)| body)
            .ok_or_else(|| invalid("inventory targets body is absent after workshop marker"))?
    } else {
        plan.summary.as_str()
    };
    let Some(binding) = StockTargetsBinding::parse(body, plan.anchor)? else {
        if plan.steps.iter().any(|step| {
            matches!(&step.action,
            Action::CreateWorkOrder { name, .. } if name.contains(ACTION_TAG))
        }) {
            return Err(invalid(
                "inventory targets original lost its required policy marker",
            ));
        }
        return Ok(None);
    };
    let layout = ProductionLayout::from_plan(plan)?;
    let positive: Vec<_> = binding.rows.iter().filter(|row| row.deficit > 0).collect();
    let first_order = u32::from(binding.assign_labor);
    if positive.is_empty()
        || layout.assignment_step() != binding.assign_labor.then_some(StepId::ZERO)
        || layout.existing_labor().is_some() == binding.assign_labor
        || plan.steps.len() != positive.len() + first_order as usize
        || Predicate::All(layout.inventory_minima().to_vec()).normalized()
            != Predicate::All(binding.minima()).normalized()
    {
        return Err(invalid(
            "inventory targets original lost its complete target requirements",
        ));
    }
    let mut timing = None;
    for (index, row) in positive.iter().enumerate() {
        let step_id = StepId::new(index as u32 + first_order);
        let step = &plan.steps[step_id.get() as usize];
        let objective = ObservedInventoryIncreaseObjective::from_selected_plan(plan, step_id)
            .ok_or_else(|| {
                invalid("inventory targets leaf lacks its original qualified increase")
            })?;
        let Action::CreateWorkOrder {
            name,
            job_token,
            amount,
            conditions,
        } = &step.action
        else {
            return Err(invalid(
                "inventory targets leaf is not a finite furniture order",
            ));
        };
        let dependencies = if binding.assign_labor {
            vec![StepId::ZERO]
        } else {
            Vec::new()
        };
        if step.id != step_id
            || step.depends_on != dependencies
            || name != &row.order_name(binding.assign_labor)
            || job_token != recipe(row.recipe.as_str())?.1
            || u64::from(*amount) != row.deficit
            || !conditions.is_empty()
            || Predicate::All(step.preconditions.clone()).normalized() != Predicate::Paused(true)
            || objective.item_type() != row.item_type()
            || objective.baseline_stack_units() != row.baseline
            || objective.increase_stack_units() != row.deficit
            || objective.target_stack_units() != row.target
            || objective.baseline_anchor() != binding.original_anchor
            || objective.baseline_source_digest() != binding.source_digest
            || objective.source_generation() != binding.source_generation
            || objective.baseline_source_counters() != binding.source_counters
        {
            return Err(invalid(
                "inventory targets leaf changed its original sizing, source or action",
            ));
        }
        let obligation = step
            .obligation
            .as_ref()
            .ok_or_else(|| invalid("inventory targets leaf lacks an original obligation"))?;
        let current = (
            obligation.deadline_tick,
            obligation.poll_interval_ticks,
            obligation.stable_for_observations,
        );
        if timing.is_some_and(|expected| expected != current) {
            return Err(invalid(
                "inventory targets leaves have different original timing",
            ));
        }
        timing = Some(current);
    }
    let (deadline, cadence, stability) =
        timing.ok_or_else(|| invalid("inventory targets original has no temporal obligation"))?;
    let mut identity = if binding.assign_labor {
        let assignment = &plan.steps[0];
        let preconditions = Predicate::All(assignment.preconditions.clone()).normalized();
        let goal =
            super::stock_target_binding::assignment_goal(&assignment.action, &preconditions)?;
        if Predicate::All(assignment.postconditions.clone()).normalized() != goal {
            return Err(invalid(
                "inventory targets assignment changed its complete labor goal",
            ));
        }
        binding.assignment_identity(
            &assignment.action,
            &preconditions,
            deadline,
            cadence,
            stability,
        )?
    } else {
        let workers = layout
            .existing_labor()
            .ok_or_else(|| invalid("inventory targets require original existing workers"))?;
        binding.identity(workers, deadline, cadence, stability)?
    };
    workshop_binding::append_identity(&mut identity, workshop)?;
    let hash = Digest32::of_bytes(&identity);
    let intent = IntentId::new(u128::from_be_bytes(
        hash.as_bytes()[..16]
            .try_into()
            .map_err(|_| invalid("inventory targets identity width"))?,
    ));
    if intent == IntentId::NIL
        || intent != plan.intent_id
        || plan.steps.iter().any(|step| {
            step.idempotency_key
                != derive_step_idempotency_key(intent, plan.anchor, step.id, &step.action)
        })
    {
        return Err(invalid(
            "inventory targets marker differs from its original identity and keys",
        ));
    }
    Ok(Some(binding))
}
