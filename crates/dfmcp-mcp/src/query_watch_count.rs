//! Quantified conditions over the named observed projection, never a complete
//! world-count or absence claim. Dynamic membership is reevaluated each sample.
use super::*;
use std::time::Instant;
use dfmcp_world::{EntityKind, EntityRecord};

pub(super) const MAX_EVALUATION_WORK: u64 = 1_000_000;

/// Shared by every condition/failure predicate and every watch in a batch.
/// Each entity visit and predicate node consumes a unit, including false rows.
pub(super) struct EvaluationBudget {
    used: u64,
    started: Instant,
    wall_millis: u64,
}
impl EvaluationBudget {
    pub(super) fn new(wall_millis: u64) -> Self {
        Self { used: 0, started: Instant::now(), wall_millis }
    }
    pub(super) fn check(&self) -> Result<()> {
        if self.started.elapsed().as_millis() >= u128::from(self.wall_millis) {
            return Err(bounded("watch evaluation exceeded its foreground deadline"));
        }
        Ok(())
    }
    pub(super) fn charge(&mut self) -> Result<()> {
        if self.used >= MAX_EVALUATION_WORK {
            return Err(bounded("watch evaluation exhausted its shared entity/predicate work allowance"));
        }
        self.used += 1;
        if self.used == 1 || self.used % 128 == 0 { self.check()?; }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Scope { ObservedProjection }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Kind { Unit, Job, Building, Item, TileFeature, Announcement }
impl Kind {
    fn entity_kind(self) -> EntityKind {
        match self {
            Self::Unit => EntityKind::Unit, Self::Job => EntityKind::Job,
            Self::Building => EntityKind::Building, Self::Item => EntityKind::Item,
            Self::TileFeature => EntityKind::TileFeature, Self::Announcement => EntityKind::Announcement,
        }
    }
}

/// No entity ID is captured in a population predicate. Explicit Field watch
/// conditions retain their existing generation fences; these count memberships
/// intentionally follow the current projection instead of a frozen identity set.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum Predicate {
    Always {},
    Field { field: String, comparison: Comparison, value: Literal },
    All { args: Vec<Predicate> },
    Any { args: Vec<Predicate> },
    Not { arg: Box<Predicate> },
}

/// Counts against the enclosing success/failure tree's shared 64-node bound.
pub(super) fn validate(predicate: &Predicate, depth: usize) -> Result<usize> {
    let mut pending = vec![(predicate, depth)];
    let mut nodes = 0usize;
    while let Some((predicate, depth)) = pending.pop() {
        nodes += 1;
        if depth > MAX_CONDITION_DEPTH || nodes > MAX_CONDITIONS {
            return Err(bounded("count predicate exceeds the watch's shared depth/node bounds"));
        }
        match predicate {
            Predicate::Always {} => {}
            Predicate::Field { field, value, .. } => {
                name(field, 128)?;
                if let Literal::Text(text) = value
                    && (text.len() > 1024 || text.contains('\0')) {
                    return Err(invalid("count predicate text exceeds its byte bound or contains NUL"));
                }
            }
            Predicate::Not { arg } => pending.push((arg, depth + 1)),
            Predicate::All { args } | Predicate::Any { args } => {
                if args.is_empty() || nodes.saturating_add(pending.len()).saturating_add(args.len()) > MAX_CONDITIONS {
                    return Err(bounded("count predicate groups must be nonempty and bounded"));
                }
                pending.extend(args.iter().map(|arg| (arg, depth + 1)));
            }
        }
    }
    Ok(nodes)
}

fn row_truth(predicate: &Predicate, entity: &EntityRecord, snapshot: &WorldSnapshot,
    budget: &mut EvaluationBudget) -> Result<Truth> {
    budget.charge()?;
    Ok(match predicate {
        Predicate::Always {} => Truth::True,
        Predicate::Field { field, comparison, value } => {
            match entity.fields.get(field) {
                Some(fact) if matches!(&fact.source, FactSource::DfhackField(_))
                    && fact.source_digest != Digest32::ZERO && fact.observed_at == snapshot.tick
                    && match &fact.presence {
                        None => true,
                        Some(FactPresence::Known(known)) => known == &fact.value,
                        _ => false,
                    } => compare(&fact.value, *comparison, value),
                _ => Truth::Unknown,
            }
        }
        Predicate::Not { arg } => row_truth(arg, entity, snapshot, budget)?.not(),
        Predicate::All { args } | Predicate::Any { args } => {
            let all = matches!(predicate, Predicate::All { .. });
            let mut decisive = false;
            let mut unknown = false;
            for arg in args {
                match row_truth(arg, entity, snapshot, budget)? {
                    Truth::False if all => decisive = true,
                    Truth::True if !all => decisive = true,
                    Truth::Unknown => unknown = true,
                    _ => {}
                }
            }
            if decisive { Truth::from_bool(!all) }
            else if unknown { Truth::Unknown } else { Truth::from_bool(all) }
        }
    })
}

/// True/false only when every integer in the sound interval agrees. In
/// particular, comparing an uncertain interval with != does not manufacture
/// truth merely because one possible count differs from the requested value.
fn interval_truth(lower: u64, upper: u64, comparison: Comparison, value: u64) -> Truth {
    let (yes, no) = match comparison {
        Comparison::Eq => (lower == upper && lower == value, value < lower || value > upper),
        Comparison::Ne => (value < lower || value > upper, lower == upper && lower == value),
        Comparison::Lt => (upper < value, lower >= value),
        Comparison::Le => (upper <= value, lower > value),
        Comparison::Gt => (lower > value, upper <= value),
        Comparison::Ge => (lower >= value, upper < value),
    };
    if yes { Truth::True } else if no { Truth::False } else { Truth::Unknown }
}
fn example(entity: &EntityRecord) -> Value {
    json!({"entity_id":entity.id.to_string(),"generation":entity.generation,"revision":entity.revision})
}

pub(super) fn evaluate(probe: &mut Probe, snapshot: &WorldSnapshot, kind: Kind,
    predicate: &Predicate, comparison: Comparison, value: u64, budget: &mut EvaluationBudget) -> Result<Truth> {
    let mut population = 0u64;
    let mut matched = 0u64;
    let mut unknown = 0u64;
    let mut matching_examples = Vec::new();
    let mut unknown_examples = Vec::new();
    let expected_kind = kind.entity_kind();
    for entity in snapshot.graph.entities.values() {
        budget.charge()?;
        if entity.kind != expected_kind { continue; }
        population += 1;
        match row_truth(predicate, entity, snapshot, budget)? {
            Truth::True => {
                matched += 1;
                if matching_examples.len() < 2 { matching_examples.push(example(entity)); }
            }
            Truth::Unknown => {
                unknown += 1;
                if unknown_examples.len() < 2 { unknown_examples.push(example(entity)); }
            }
            Truth::False => {}
        }
    }
    budget.check()?;
    let upper = matched + unknown;
    let truth = interval_truth(matched, upper, comparison, value);
    probe.facts.push(json!({"op":"entity_count","scope":"observed_projection","kind":kind,
        "predicate_digest":digest(&json!(predicate))?.to_string(),"snapshot_hash":snapshot.state_hash.to_string(),
        "population":population,"matched_min":matched,"matched_max":upper,"unestablished":unknown,
        "comparison":comparison,"threshold":value,"truth":truth.text(),
        "matching_examples":matching_examples,"unestablished_examples":unknown_examples,
        "examples_complete":matched<=2 && unknown<=2,"membership":"dynamic_at_each_sample",
        "complete_world_count_proven":false}));
    Ok(truth)
}

/// Additive discovery for the spatial runtime; existing condition variants and
/// definitions are retained verbatim. Older binaries reject the new op rather
/// than interpreting it as a different condition.
pub(super) fn extend_schema(mut schema: Value) -> Result<Value> {
    let extension: Value = serde_json::from_str(include_str!("../../../schemas/mcp_watch_count_v1.json"))
        .map_err(|_| invalid("embedded count-condition schema is invalid"))?;
    let definitions = extension["$defs"].as_object().ok_or_else(|| invalid("count schema definitions absent"))?;
    let target = schema["$defs"].as_object_mut().ok_or_else(|| invalid("query schema definitions absent"))?;
    for (key, definition) in definitions {
        if target.contains_key(key) { return Err(invalid("count schema definition already registered")); }
        target.insert(key.clone(), definition.clone());
    }
    schema["$defs"]["watch_condition"]["oneOf"].as_array_mut()
        .ok_or_else(|| invalid("watch condition schema variants absent"))?
        .push(extension["condition"].clone());
    Ok(schema)
}

#[cfg(test)]
#[path = "query_watch_count_tests.rs"]
mod tests;
