//! Unsigned field totals over an explicitly named laboratory projection.
//!
//! Membership and values use the same owner-issued reference evidence policy.
//! Unknown rows contribute sound intervals, never invented zero quantities.
//! Native stack measurements keep their independent protocol and source rules.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(in super::super) enum Kind {
    Unit,
    Job,
    Building,
    Item,
    TileFeature,
    Announcement,
    StockLedger,
}

impl Kind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Unit => "unit",
            Self::Job => "job",
            Self::Building => "building",
            Self::Item => "item",
            Self::TileFeature => "tile_feature",
            Self::Announcement => "announcement",
            Self::StockLedger => dfmcp_intent::effects::STOCK_LEDGER_KIND,
        }
    }
}

pub(in super::super) struct Selection<'a> {
    pub(in super::super) kind: Kind,
    pub(in super::super) field: &'a str,
    pub(in super::super) predicate: &'a Predicate,
    pub(in super::super) comparison: Comparison,
    pub(in super::super) threshold: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Bounds {
    lower: u64,
    upper: Option<u64>,
}

impl Default for Bounds {
    fn default() -> Self {
        Self {
            lower: 0,
            upper: Some(0),
        }
    }
}

impl Bounds {
    fn include(&mut self, membership: Truth, amount: Option<u64>) -> Result<()> {
        if membership == Truth::False {
            return Ok(());
        }
        if let Some(amount) = amount {
            if membership == Truth::True {
                self.lower = self
                    .lower
                    .checked_add(amount)
                    .ok_or_else(|| bounded("field_sum lower bound exceeds u64"))?;
            }
            if let Some(upper) = self.upper {
                self.upper = Some(
                    upper
                        .checked_add(amount)
                        .ok_or_else(|| bounded("field_sum upper bound exceeds u64"))?,
                );
            }
        } else {
            self.upper = None;
        }
        Ok(())
    }

    fn compare(self, comparison: Comparison, threshold: u64) -> Truth {
        if let Some(upper) = self.upper {
            return interval_truth(self.lower, upper, comparison, threshold);
        }
        // An unavailable quantity has no established finite upper bound.
        // Only conclusions true for the entire remaining interval are eligible.
        match comparison {
            Comparison::Eq if self.lower > threshold => Truth::False,
            Comparison::Ne if self.lower > threshold => Truth::True,
            Comparison::Lt if self.lower >= threshold => Truth::False,
            Comparison::Le if self.lower > threshold => Truth::False,
            Comparison::Gt if self.lower > threshold => Truth::True,
            Comparison::Ge if self.lower >= threshold => Truth::True,
            _ => Truth::Unknown,
        }
    }
}

fn amount(entity: &EntityRecord, snapshot: &WorldSnapshot, field: &str) -> Option<u64> {
    let fact = entity.fields.get(field)?;
    match SourcePolicy::LaboratoryReference
        .fact_value(fact, snapshot.tick)
        .ok()?
    {
        WorldValue::U64(value) => Some(*value),
        _ => None,
    }
}

pub(in super::super) fn evaluate(
    probe: &mut Probe,
    snapshot: &WorldSnapshot,
    selection: Selection<'_>,
    budget: &mut EvaluationBudget,
) -> Result<Truth> {
    if probe.source_policy != SourcePolicy::LaboratoryReference {
        return Err(invalid(
            "field_sum requires the owner-issued laboratory reference evidence policy",
        ));
    }
    // Keep the helper closed even for internal callers that bypass definition
    // admission: field/depth/node bounds and lab-only membership still apply.
    name(selection.field, 128)?;
    validate(selection.predicate, 2)?;
    laboratory::validate_population(selection.predicate)?;
    let relations = relationships::Bindings::default();
    let mut bounds = Bounds::default();
    let mut population = 0u64;
    let mut matched = 0u64;
    let mut excluded = 0u64;
    let mut unknown_membership = 0u64;
    let mut unknown_value = 0u64;
    let mut unknown_identity = 0u64;
    let mut unestablished = 0u64;
    let mut matching_examples = Vec::new();
    let mut unknown_examples = Vec::new();
    for entity in snapshot.graph.entities.values() {
        budget.charge()?;
        // Canonical entity-kind identity is its string, including Other aliases.
        if entity.kind.as_str() != selection.kind.as_str() {
            continue;
        }
        population += 1;
        let identity_known =
            entity.id != EntityId::NIL && entity.generation != 0 && entity.revision != 0;
        let membership = if identity_known {
            row_truth_with_policy(
                selection.predicate,
                entity,
                snapshot,
                &relations,
                budget,
                SourcePolicy::LaboratoryReference,
            )?
        } else {
            Truth::Unknown
        };
        if membership == Truth::False {
            excluded += 1;
            continue;
        }
        budget.charge()?;
        let quantity = identity_known
            .then(|| amount(entity, snapshot, selection.field))
            .flatten();
        bounds.include(membership, quantity)?;
        if membership == Truth::True {
            matched += 1;
            if matching_examples.len() < 2 {
                matching_examples.push(example(entity));
            }
        } else {
            unknown_membership += 1;
        }
        if quantity.is_none() {
            unknown_value += 1;
        }
        if !identity_known {
            unknown_identity += 1;
        }
        if membership == Truth::Unknown || quantity.is_none() {
            unestablished += 1;
            if unknown_examples.len() < 2 {
                let mut witness = example(entity);
                witness["membership_established"] = json!(membership != Truth::Unknown);
                witness["value_established"] = json!(quantity.is_some());
                witness["identity_established"] = json!(identity_known);
                unknown_examples.push(witness);
            }
        }
    }
    budget.check()?;
    let truth = bounds.compare(selection.comparison, selection.threshold);
    probe.facts.push(json!({
        "op":"field_sum", "scope":"observed_projection", "kind":selection.kind,
        "field":selection.field, "value_type":"u64",
        "sum_min":bounds.lower, "sum_max":bounds.upper,
        "sum_exact":bounds.upper == Some(bounds.lower),
        "upper_bound_established":bounds.upper.is_some(),
        "population":population, "matched_records_min":matched,
        "matched_records_max":matched + unknown_membership, "excluded_records":excluded,
        "unestablished_membership_records":unknown_membership,
        "unestablished_value_records":unknown_value,
        "unestablished_identity_records":unknown_identity,
        "unestablished_records":unestablished,
        "comparison":selection.comparison, "threshold":selection.threshold, "truth":truth.text(),
        "predicate_digest":digest(&json!(selection.predicate))?.to_string(),
        "snapshot_hash":snapshot.state_hash.to_string(),
        "matching_examples":matching_examples, "unestablished_examples":unknown_examples,
        "examples_complete":matched <= 2 && unestablished <= 2,
        "membership":"dynamic_at_each_sample",
        "source_policy":"laboratory_reference/1",
        "unknown_value_policy":"no_established_upper_bound",
        "usable_supply_proven":false, "complete_world_quantity_proven":false,
        "interpretation":"Unsigned selected field units in the laboratory projection; not native stack units, usable supply, reservations, causal attribution or continuous history."
    }));
    budget.check()?;
    Ok(truth)
}

#[cfg(test)]
#[path = "query_watch_field_sum_tests.rs"]
mod tests;
