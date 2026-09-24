//! A quantity is not a count of stacks. The same bounded measurement feeds
//! stateless inspection and watch predicates; neither infers usable supply.
//! Parent count predicates supply three-valued membership and shared work limits.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(in super::super) enum QuantityUnit {
    StackUnits,
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
    fn include(&mut self, membership: Truth, quantity: Option<u64>) -> Result<()> {
        if membership == Truth::False {
            return Ok(());
        }
        match quantity {
            Some(units) => {
                if membership == Truth::True {
                    self.lower = self
                        .lower
                        .checked_add(units)
                        .ok_or_else(|| bounded("item quantity lower bound exceeds u64"))?;
                }
                if let Some(upper) = self.upper {
                    self.upper = Some(
                        upper
                            .checked_add(units)
                            .ok_or_else(|| bounded("item quantity upper bound exceeds u64"))?,
                    );
                }
            }
            // Never turn a missing, wrongly typed or redacted stack size into
            // zero, a fabricated capacity, or a saturated integer maximum.
            None => self.upper = None,
        }
        Ok(())
    }
    fn compare(self, comparison: Comparison, value: u64) -> Truth {
        if let Some(upper) = self.upper {
            return interval_truth(self.lower, upper, comparison, value);
        }
        match comparison {
            Comparison::Eq if self.lower > value => Truth::False,
            Comparison::Ne if self.lower > value => Truth::True,
            Comparison::Lt if self.lower >= value => Truth::False,
            Comparison::Le if self.lower > value => Truth::False,
            Comparison::Gt if self.lower > value => Truth::True,
            Comparison::Ge if self.lower >= value => Truth::True,
            _ => Truth::Unknown,
        }
    }
}

fn observed_quantity(entity: &EntityRecord, snapshot: &WorldSnapshot) -> Option<u64> {
    let fact = entity.fields.get("stack_size")?;
    if !matches!(&fact.source, FactSource::DfhackField(_))
        || fact.source_digest == Digest32::ZERO
        || fact.observed_at != snapshot.tick
        || match &fact.presence {
            None => false,
            Some(FactPresence::Known(value)) => value != &fact.value,
            _ => true,
        }
    {
        return None;
    }
    // The canonical operations projection uses U64 for item.getStackSize.
    // Do not coerce signed, fixed, textual or boolean fields into stack units.
    match &fact.value {
        WorldValue::U64(units) => Some(*units),
        _ => None,
    }
}

#[derive(Default)]
struct Measurement {
    bounds: Bounds,
    relations: relationships::Bindings,
    population: u64,
    matched: u64,
    unknown_membership: u64,
    unknown_quantity: u64,
    unestablished: u64,
    matches: Vec<Value>,
    unknowns: Vec<Value>,
}
impl Measurement {
    fn json(&self, snapshot: &WorldSnapshot, predicate: &Predicate) -> Result<Value> {
        let mut result = json!({"scope":"observed_projection","quantity_unit":"stack_units","field":"stack_size",
            "quantity_min":self.bounds.lower,"quantity_max":self.bounds.upper,
            "quantity_exact":self.bounds.upper==Some(self.bounds.lower),
            "upper_bound_established":self.bounds.upper.is_some(),
            "item_records":self.population,"matched_records_min":self.matched,
            "matched_records_max":self.matched+self.unknown_membership,
            "unestablished_membership_records":self.unknown_membership,
            "unestablished_quantity_records":self.unknown_quantity,
            "unestablished_records":self.unestablished,
            "matching_examples":self.matches,"unestablished_examples":self.unknowns,
            "examples_complete":self.matched<=2 && self.unestablished<=2,
            "predicate_digest":digest(&json!(predicate))?.to_string(),
            "snapshot_hash":snapshot.state_hash.to_string(),"membership":"dynamic_at_each_sample",
            "usable_supply_proven":false,"complete_world_quantity_proven":false,
            "unknown_quantity_policy":"no_established_upper_bound",
            "interpretation":"Raw selected item stack units, not usable supply, food portions, nutrition, access, reservations or continuous history."});
        self.relations.annotate(&mut result);
        Ok(result)
    }
}

fn measure(
    snapshot: &WorldSnapshot,
    predicate: &Predicate,
    budget: &mut EvaluationBudget,
) -> Result<Measurement> {
    let mut report = Measurement {
        relations: relationships::bind(predicate, snapshot, budget)?,
        ..Measurement::default()
    };
    for entity in snapshot.graph.entities.values() {
        budget.charge()?;
        if entity.kind != EntityKind::Item {
            continue;
        }
        report.population += 1;
        let membership = row_truth_bound(predicate, entity, snapshot, &report.relations, budget)?;
        // A definitely excluded item contributes zero regardless of a missing
        // quantity. An uncertain item with known zero units also contributes zero.
        if membership == Truth::False {
            continue;
        }
        budget.charge()?;
        let quantity = observed_quantity(entity, snapshot);
        report.bounds.include(membership, quantity)?;
        if membership == Truth::True {
            report.matched += 1;
            if report.matches.len() < 2 {
                report.matches.push(example(entity));
            }
        } else {
            report.unknown_membership += 1;
        }
        if quantity.is_none() {
            report.unknown_quantity += 1;
        }
        if membership == Truth::Unknown || quantity.is_none() {
            report.unestablished += 1;
            if report.unknowns.len() < 2 {
                let mut witness = example(entity);
                witness["membership_established"] = json!(membership != Truth::Unknown);
                witness["quantity_established"] = json!(quantity.is_some());
                // No backing field value is serialized when evidence is unknown.
                report.unknowns.push(witness);
            }
        }
    }
    if !report.relations.ready() {
        // Even an empty projection cannot measure a recycled or missing root.
        report.bounds = Bounds {
            lower: 0,
            upper: None,
        };
    }
    budget.check()?;
    Ok(report)
}

pub(in super::super) fn evaluate(
    probe: &mut Probe,
    snapshot: &WorldSnapshot,
    predicate: &Predicate,
    comparison: Comparison,
    value: u64,
    budget: &mut EvaluationBudget,
) -> Result<Truth> {
    let measured = measure(snapshot, predicate, budget)?;
    let truth = measured
        .relations
        .guard(measured.bounds.compare(comparison, value));
    let mut fact = measured.json(snapshot, predicate)?;
    fact["op"] = json!("item_quantity");
    fact["comparison"] = json!(comparison);
    fact["threshold"] = json!(value);
    fact["truth"] = json!(truth.text());
    budget.check()?;
    probe.invalid_generation |= measured.relations.invalid_generation();
    probe.facts.push(fact);
    Ok(truth)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct QuantityEnvelope {
    schema: String,
    expected_anchor: Option<Value>,
    query: QuantityRequest,
}
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum QuantityRequest {
    ItemQuantity {
        scope: Scope,
        quantity_unit: QuantityUnit,
        predicate: Predicate,
    },
}

/// Pure inspection. The enclosing runtime attaches current work and reserves its
/// full Agent Turn; historical callers retain their own exact-record boundary.
pub(in super::super) fn query(
    snapshot: &WorldSnapshot,
    context: &OperationContext,
    input: &Value,
) -> Result<Value> {
    let mut budget = EvaluationBudget::new(context.budget.max_wall_millis);
    authorize(snapshot, context)?;
    validate_input(input)?;
    let request: QuantityEnvelope = serde_json::from_value(input.clone())
        .map_err(|_| invalid("invalid item_quantity request"))?;
    if request.schema != "dfmcp.query/1" {
        return Err(invalid("item_quantity requires dfmcp.query/1"));
    }
    if request
        .expected_anchor
        .as_ref()
        .is_some_and(|value| value != &anchor(context.anchor))
    {
        return Err(failure(
            ErrorCode::StaleAnchor,
            "item_quantity expected_anchor differs from the selected observation",
        ));
    }
    let QuantityRequest::ItemQuantity {
        scope: Scope::ObservedProjection,
        quantity_unit: QuantityUnit::StackUnits,
        predicate,
    } = request.query;
    if validate(&predicate, 2)? + 1 > MAX_CONDITIONS {
        return Err(bounded(
            "item quantity predicate exceeds the shared node bound",
        ));
    }
    let quantity = measure(snapshot, &predicate, &mut budget)?.json(snapshot, &predicate)?;
    let evidence = digest(
        &json!({"domain":"dfmcp-item-quantity/1","anchor":anchor(context.anchor),"quantity":quantity}),
    )?;
    let result = json!({"schema":"dfmcp.query.result/1","kind":"item_quantity","anchor":anchor(context.anchor),
        "quantity":quantity,"evidence_digest":evidence.to_string(),"native_captures":0,
        "watch_registered":false,"watch_evaluated":false,"mutation_authority":false,
        "coverage":{"domain":"selected_observed_item_projection","absence_proven":false,
            "usable_supply_proven":false,"continuous_between_observations":false},
        "truncated":false,"continuation":null});
    let maximum = context
        .budget
        .max_bytes
        .min(u64::from(context.budget.max_output_tokens) * 4);
    if serde_json::to_vec(&result)
        .map_err(|_| invalid("item quantity response cannot be encoded"))?
        .len() as u64
        > maximum
    {
        return Err(bounded(
            "complete item quantity summary exceeds the result budget",
        ));
    }
    context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    budget.check()?;
    Ok(result)
}

pub(super) fn extend_schema(mut schema: Value) -> Result<Value> {
    let extension: Value =
        serde_json::from_str(include_str!("../../../schemas/mcp_item_quantity_v1.json"))
            .map_err(|_| invalid("embedded item quantity schema is invalid"))?;
    schema["$defs"]["watch_condition"]["oneOf"]
        .as_array_mut()
        .ok_or_else(|| invalid("watch condition variants absent"))?
        .push(extension["condition"].clone());
    schema["$defs"]["query"]["oneOf"]
        .as_array_mut()
        .ok_or_else(|| invalid("query variants absent"))?
        .push(extension["query"].clone());
    Ok(schema)
}

#[cfg(test)]
#[path = "query_watch_quantity_tests.rs"]
mod tests;
