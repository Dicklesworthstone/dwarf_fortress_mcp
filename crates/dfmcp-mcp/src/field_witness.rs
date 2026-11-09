//! Closed field refinement for immediate, model-free laboratory setters.
//!
//! This is a knowledge check for replay, never a merged world or authority.
//! Unsupported reads, unavailable facts and incomplete scans require review.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

use dfmcp_core::{Digest32, EntityId, WorkBudget};
use dfmcp_intent::{Action, PreparedPlan, effects};
use dfmcp_world::{EntityKind, Fact, FactPresence, Predicate, Value, WorldSnapshot};
use serde_json::{Value as Json, json};

const MAX_FIELDS: usize = 256;
const MAX_IDENTITIES: usize = 256;
const MAX_PREDICATE_DEPTH: usize = 32;
const MAX_OPERATIONS: usize = 262_144;
const MAX_BYTES: usize = 1_048_576;
const MAX_RECORDS: usize = 4_096;

// apply_effect validates the optional laboratory models before every action. Their
// absence is a complete-domain read, including misplaced declarations/stamps
// on edges. Reject every production marker conservatively, even a non-food
// recipe, rather than infer which retained task histories may be ignored.
const MODEL_FIELDS: [&str; 27] = [
    effects::LABORATORY_WORKFORCE_MODEL_FIELD,
    effects::LABORATORY_WORKFORCE_TASK_MODEL_FIELD,
    effects::LABORATORY_CONSTRUCTION_MATERIAL_MODEL_FIELD,
    effects::CONSTRUCTION_MATERIAL_MODEL_FIELD,
    effects::CONSTRUCTION_MATERIAL_LEDGER_FIELD,
    effects::CONSTRUCTION_MATERIAL_LEDGER_GENERATION_FIELD,
    effects::CONSTRUCTION_MATERIAL_WOOD_FIELD,
    effects::CONSTRUCTION_MATERIAL_STATE_FIELD,
    effects::CONSTRUCTION_MATERIAL_FORBIDDEN_FIELD,
    effects::CONSTRUCTION_MATERIAL_NEAREST_FIELD,
    effects::CONSTRUCTION_MATERIAL_RESERVE_FIELD,
    effects::LABORATORY_PROVISIONS_MODEL_FIELD,
    effects::PROVISIONS_LEDGER_FIELD,
    effects::PROVISIONS_LEDGER_GENERATION_FIELD,
    effects::PROVISIONS_CONSUMED_FIELD,
    effects::WORK_ORDER_PRODUCTION_MODEL_FIELD,
    effects::LABORATORY_HARVEST_MODEL_FIELD,
    effects::HARVEST_SOURCE_MODEL_FIELD,
    effects::HARVEST_TASK_MODEL_FIELD,
    effects::HARVEST_RESOURCE_FIELD,
    effects::HARVEST_INITIAL_FIELD,
    effects::HARVEST_REMAINING_FIELD,
    effects::HARVEST_YIELDED_FIELD,
    effects::HARVEST_SOURCE_FIELD,
    effects::HARVEST_SOURCE_GENERATION_FIELD,
    effects::HARVEST_LEDGER_FIELD,
    effects::HARVEST_LEDGER_GENERATION_FIELD,
];

type Check<T> = std::result::Result<T, &'static str>;

struct Meter {
    start: Instant,
    operations: usize,
    bytes: usize,
    max_bytes: usize,
    max_millis: u64,
}

impl Meter {
    fn new(budget: WorkBudget) -> Check<Self> {
        budget.validate().map_err(|_| "invalid refinement budget")?;
        Ok(Self {
            start: Instant::now(),
            operations: 0,
            bytes: 0,
            max_bytes: usize::try_from(budget.max_bytes)
                .unwrap_or(MAX_BYTES)
                .min(MAX_BYTES),
            max_millis: budget.max_wall_millis.min(100),
        })
    }

    fn charge(&mut self, operations: usize, bytes: usize) -> Check<()> {
        self.operations = self.operations.saturating_add(operations);
        self.bytes = self.bytes.saturating_add(bytes);
        if self.operations > MAX_OPERATIONS
            || self.bytes > self.max_bytes
            || self.start.elapsed().as_millis() >= u128::from(self.max_millis)
        {
            return Err("field refinement exceeds its work budget");
        }
        Ok(())
    }

    fn text(&mut self, value: &str) -> Check<()> {
        if value.len() > 4_096 {
            return Err("field refinement text exceeds its bound");
        }
        self.charge(1, value.len().saturating_add(8))
    }

    fn scalar(&mut self, value: &Value) -> Check<()> {
        match value {
            Value::Text(text) => self.text(text),
            Value::Bytes(bytes) if bytes.len() <= 4_096 => self.charge(1, bytes.len() + 8),
            Value::List(_) | Value::Object(_) | Value::Bytes(_) => {
                Err("field refinement requires bounded scalar facts and literals")
            }
            _ => self.charge(1, 32),
        }
    }

    fn fact(&mut self, fact: &Fact, tick: dfmcp_core::GameTick) -> Check<()> {
        self.charge(1, 64)?;
        self.scalar(&fact.value)?;
        match &fact.source {
            dfmcp_world::FactSource::Derived(source)
                if source == "dfmcp.lab-scenario/1" || source == "dfmcp.reference-effects/1" => {}
            _ => return Err("field refinement requires qualified laboratory provenance"),
        }
        if fact.source_digest != Digest32::ZERO || fact.observed_at > tick {
            return Err("field refinement requires current qualified laboratory facts");
        }
        match &fact.presence {
            None => {}
            Some(FactPresence::Known(value)) => {
                self.scalar(value)?;
                if value != &fact.value {
                    return Err("field refinement rejects inconsistent known presence");
                }
            }
            Some(FactPresence::Absent) => {}
            _ => return Err("field refinement cannot use unavailable or stale presence"),
        }
        Ok(())
    }
}

#[derive(Default)]
struct Reads {
    identities: BTreeSet<EntityId>,
    kinds: BTreeMap<EntityId, EntityKind>,
    fields: BTreeSet<(EntityId, String)>,
    writes: BTreeSet<(EntityId, String)>,
    pause: bool,
}

impl Reads {
    fn identity(&mut self, id: EntityId, meter: &mut Meter) -> Check<()> {
        meter.charge(1, 8)?;
        if id == EntityId::NIL {
            return Err("field refinement cannot bind the reserved entity identity");
        }
        self.identities.insert(id);
        if self.identities.len() > MAX_IDENTITIES {
            return Err("field refinement identity set exceeds its bound");
        }
        Ok(())
    }

    fn field(&mut self, id: EntityId, field: &str, write: bool, meter: &mut Meter) -> Check<()> {
        self.identity(id, meter)?;
        meter.text(field)?;
        if field.is_empty() || field.len() > 256 {
            return Err("field refinement field name exceeds its bound");
        }
        self.fields.insert((id, field.to_owned()));
        if self.fields.len() > MAX_FIELDS {
            return Err("field refinement read set exceeds its bound");
        }
        if write {
            self.writes.insert((id, field.to_owned()));
        }
        Ok(())
    }

    fn kind(&mut self, id: EntityId, kind: EntityKind, meter: &mut Meter) -> Check<()> {
        self.identity(id, meter)?;
        if self.kinds.get(&id).is_some_and(|prior| prior != &kind) {
            return Err("field refinement has inconsistent target kinds");
        }
        self.kinds.insert(id, kind);
        Ok(())
    }

    fn action(&mut self, action: &Action, meter: &mut Meter) -> Check<()> {
        let (units, field) = match action {
            Action::SetLabor { units, labor, .. } => {
                meter.text(labor)?;
                (units, format!("{}{labor}", effects::LABOR_FIELD_PREFIX))
            }
            Action::SetBurrowMembership { units, burrow, .. } => {
                self.kind(*burrow, EntityKind::Burrow, meter)?;
                (
                    units,
                    format!("{}{}", effects::BURROW_FIELD_PREFIX, burrow.get()),
                )
            }
            _ => return Err("field refinement supports only immediate labor and burrow setters"),
        };
        if units.is_empty() || units.len() > MAX_IDENTITIES {
            return Err("field refinement action scope exceeds its bound");
        }
        for id in units {
            self.kind(*id, EntityKind::Unit, meter)?;
            self.field(*id, &field, true, meter)?;
        }
        Ok(())
    }

    fn predicate(&mut self, predicate: &Predicate, depth: usize, meter: &mut Meter) -> Check<()> {
        meter.charge(1, 8)?;
        if depth > MAX_PREDICATE_DEPTH {
            return Err("field refinement predicate depth exceeds its bound");
        }
        match predicate {
            Predicate::True | Predicate::False => Ok(()),
            Predicate::EntityExists(id) => self.identity(*id, meter),
            Predicate::EntityKind { entity_id, kind } => {
                meter.text(kind.as_str())?;
                self.identity(*entity_id, meter)
            }
            Predicate::FieldCompare {
                entity_id,
                field,
                value,
                ..
            } => {
                meter.scalar(value)?;
                self.field(*entity_id, field, false, meter)
            }
            Predicate::Paused(_) => {
                self.pause = true;
                Ok(())
            }
            Predicate::All(children) | Predicate::Any(children) => {
                if children.len() > MAX_FIELDS {
                    return Err("field refinement predicate width exceeds its bound");
                }
                // Include every branch, including false branches and negation.
                for child in children {
                    self.predicate(child, depth + 1, meter)?;
                }
                Ok(())
            }
            Predicate::Not(child) => self.predicate(child, depth + 1, meter),
            _ => Err(
                "range, relation, aggregate and owned-history reads require conservative replay",
            ),
        }
    }
}

fn model_free(snapshot: &WorldSnapshot, budget: WorkBudget, meter: &mut Meter) -> Check<()> {
    let records = snapshot
        .graph
        .entities
        .len()
        .saturating_add(snapshot.graph.edges.len());
    if records > MAX_RECORDS.min(budget.max_entities as usize) {
        return Err("complete model-absence census exceeds the entity budget");
    }
    // Harvest task identity alone is retained model history, even if every
    // field was lost. It cannot establish a model-free input to apply_effect.
    for record in snapshot.graph.entities.values() {
        meter.charge(1, 16)?;
        if matches!(&record.kind, EntityKind::Other(kind) if kind == effects::HARVEST_TASK_KIND) {
            return Err("field refinement requires absence of retained harvest task identities");
        }
    }
    for fields in snapshot
        .graph
        .entities
        .values()
        .map(|record| &record.fields)
        .chain(snapshot.graph.edges.values().map(|record| &record.fields))
    {
        meter.charge(MODEL_FIELDS.len(), 16)?;
        if MODEL_FIELDS.iter().any(|field| fields.contains_key(*field)) {
            return Err("field refinement requires a complete model-free laboratory domain");
        }
    }
    Ok(())
}

/// A bounded proof that the registered reads and prior write values agree.
/// The caller must still replay the original Actions source, compare the whole
/// reviewed contract, and use normal current authorization/lease/commit gates.
pub(crate) struct FieldWitness {
    description: Json,
}

impl FieldWitness {
    pub(crate) fn prove(
        plan: &PreparedPlan,
        base: &WorldSnapshot,
        now: &WorldSnapshot,
        budget: WorkBudget,
    ) -> Check<Self> {
        if base.anchor() != plan.anchor
            || base.fortress_id != now.fortress_id
            || base.cursor.epoch != now.cursor.epoch
            || now.cursor.sequence <= base.cursor.sequence
            || now.tick != base.tick
        {
            return Err("field refinement requires the sealed same-tick forward source lineage");
        }
        if plan.steps.is_empty() || plan.steps.len() > 64.min(budget.max_actions as usize) {
            return Err("field refinement plan exceeds its action bound");
        }
        let mut meter = Meter::new(budget)?;
        let mut reads = Reads::default();
        reads.predicate(&plan.terminal_condition, 0, &mut meter)?;
        for step in &plan.steps {
            if step.obligation.is_some() {
                return Err("field refinement cannot weaken a temporal obligation");
            }
            reads.action(&step.action, &mut meter)?;
            if let Some(compensation) = &step.compensation {
                reads.action(compensation, &mut meter)?;
            }
            if step
                .preconditions
                .len()
                .saturating_add(step.postconditions.len())
                > MAX_FIELDS
            {
                return Err("field refinement step predicates exceed their bound");
            }
            for predicate in step.preconditions.iter().chain(&step.postconditions) {
                reads.predicate(predicate, 0, &mut meter)?;
            }
        }
        if reads.identities.len() > budget.max_entities as usize {
            return Err("field refinement scope exceeds the entity budget");
        }
        model_free(base, budget, &mut meter)?;
        model_free(now, budget, &mut meter)?;
        if reads.pause && base.paused != now.paused {
            return Err("the explicit pause read changed");
        }
        let mut identities = Vec::new();
        for id in &reads.identities {
            meter.charge(1, 64)?;
            match (base.graph.entities.get(id), now.graph.entities.get(id)) {
                (Some(before), Some(after)) => {
                    meter.text(before.kind.as_str())?;
                    meter.text(after.kind.as_str())?;
                    if before.id != *id
                        || after.id != *id
                        || before.generation == 0
                        || before.generation != after.generation
                        || before.kind != after.kind
                        || after.revision < before.revision
                        || after.revision == u64::MAX
                        || reads.kinds.get(id).is_some_and(|kind| kind != &after.kind)
                    {
                        return Err(
                            "an entity identity, generation, kind or revision invariant changed",
                        );
                    }
                    identities.push(
                        json!({"entity_id":id.to_string(), "generation":before.generation,
                        "kind":before.kind.as_str(), "base_revision":before.revision,
                        "current_revision":after.revision}),
                    );
                }
                (None, None) if !reads.kinds.contains_key(id) => {
                    identities.push(json!({"entity_id":id.to_string(), "presence":"missing"}));
                }
                _ => {
                    return Err(
                        "an explicitly read entity was inserted, removed or is unavailable",
                    );
                }
            }
        }
        let mut fields = Vec::new();
        for (id, field) in &reads.fields {
            meter.charge(1, 16)?;
            let before = base
                .graph
                .entities
                .get(id)
                .ok_or("a field's entity is unavailable")?
                .fields
                .get(field);
            let after = now
                .graph
                .entities
                .get(id)
                .ok_or("a field's entity is unavailable")?
                .fields
                .get(field);
            for fact in before.into_iter().chain(after) {
                meter.fact(fact, base.tick)?;
            }
            // Missing map entry is structural absence, never a false value.
            // Explicit absence remains distinct; unavailable presence refuses.
            if before != after {
                return Err("an explicit field read or prior write fact changed");
            }
            let presence = match before.map(|fact| fact.presence.as_ref()) {
                None => "missing",
                Some(Some(FactPresence::Absent)) => "absent",
                _ => "known",
            };
            fields.push(json!({"entity_id":id.to_string(), "field":field,
                "presence":presence,
                "fact_digest":before.map(|fact| Digest32::of_bytes(&fact.canonical_bytes()).to_hex()),
                "written":reads.writes.contains(&(*id, field.clone()))}));
        }
        meter.charge(1, 0)?;
        Ok(Self {
            description: json!({
                "domain":"laboratory_model_free_setters/1", "identities":identities,
                "fields":fields, "pause":reads.pause,
                "model_absence":{"coverage":"complete_entities_and_edges", "fields":MODEL_FIELDS,
                    "entity_kinds":[effects::HARVEST_TASK_KIND]},
                "bounds":{"identities":MAX_IDENTITIES, "fields":MAX_FIELDS,
                    "records_per_version":MAX_RECORDS.min(budget.max_entities as usize),
                    "operations":MAX_OPERATIONS, "bytes":meter.max_bytes},
            }),
        })
    }

    pub(crate) fn certificate(
        &self,
        from: &PreparedPlan,
        to: &PreparedPlan,
        coarse_conflict: &Json,
    ) -> Json {
        let body = json!({
            "schema":"dfmcp.field-witness-rebase/1", "from_digest":from.digest.to_hex(),
            "to_digest":to.digest.to_hex(), "base_state_hash":from.anchor.state_hash.to_hex(),
            "current_state_hash":to.anchor.state_hash.to_hex(), "witness":self.description,
            "coarse_conflict":coarse_conflict,
            "policy":"original_intent_replay_with_identical_reviewed_contract_after_unchanged_field_reads_and_prior_writes",
        });
        let digest = Digest32::of_bytes(body.to_string().as_bytes()).to_hex();
        json!({"certificate":body, "certificate_digest":digest})
    }
}

#[cfg(test)]
#[path = "field_witness_tests.rs"]
mod tests;
