//! A closed three-way check for ordinary laboratory configuration requests.
//!
//! The current snapshot is never replaced by a merged copy. The caller must
//! replay the retained original source there and compare its complete sealed
//! contract before entering ordinary capability, preparation and commit gates.

use super::*;
use dfmcp_world::{CompareOp, EntityRecord, FactSource};

struct Write {
    value: Value,
    steps: Vec<u32>,
}

#[derive(Default)]
struct Configuration {
    kinds: BTreeMap<EntityId, EntityKind>,
    writes: BTreeMap<(EntityId, String), Write>,
}

impl Configuration {
    fn write(
        &mut self,
        id: EntityId,
        kind: EntityKind,
        field: String,
        value: Value,
        step: u32,
        meter: &mut Meter,
    ) -> Check<()> {
        meter.charge(1, 64)?;
        meter.text(&field)?;
        if id == EntityId::NIL || field.is_empty() || field.len() > 256 {
            return Err("configuration merge requires bounded nonreserved targets");
        }
        configuration_value(&field, &value, meter)?;
        if self.kinds.get(&id).is_some_and(|prior| prior != &kind) {
            return Err("configuration merge target kinds disagree");
        }
        self.kinds.insert(id, kind);
        if self.kinds.len() > MAX_IDENTITIES {
            return Err("configuration merge exceeds its identity bound");
        }
        let key = (id, field);
        if let Some(prior) = self.writes.get_mut(&key) {
            if prior.value != value {
                return Err("configuration merge cannot resolve ordered divergent writes");
            }
            prior.steps.push(step);
        } else {
            self.writes.insert(
                key,
                Write {
                    value,
                    steps: vec![step],
                },
            );
        }
        if self.writes.len() > MAX_FIELDS {
            return Err("configuration merge exceeds its field bound");
        }
        Ok(())
    }

    fn action(
        &mut self,
        action: &Action,
        step: u32,
        fortress: dfmcp_core::FortressId,
        meter: &mut Meter,
    ) -> Check<()> {
        match action {
            Action::ConfigureStockpile {
                stockpile,
                accepts,
                max_bins,
                max_barrels,
                max_wheelbarrows,
            } => {
                // The action's ordered set defines one exact filter value.
                if accepts.len() > 64 {
                    return Err("configuration merge requires a bounded canonical filter set");
                }
                self.write(
                    *stockpile,
                    EntityKind::Stockpile,
                    effects::STOCKPILE_ACCEPTS_FIELD.to_owned(),
                    Value::List(accepts.iter().cloned().map(Value::Text).collect()),
                    step,
                    meter,
                )?;
                for (field, value) in [
                    (effects::STOCKPILE_MAX_BINS_FIELD, max_bins),
                    (effects::STOCKPILE_MAX_BARRELS_FIELD, max_barrels),
                    (effects::STOCKPILE_MAX_WHEELBARROWS_FIELD, max_wheelbarrows),
                ] {
                    if let Some(value) = value {
                        self.write(
                            *stockpile,
                            EntityKind::Stockpile,
                            field.to_owned(),
                            Value::U64(u64::from(*value)),
                            step,
                            meter,
                        )?;
                    }
                }
            }
            Action::SetStandingOrder { key, value } => self.write(
                effects::fortress_settings_entity_id(fortress),
                EntityKind::Other(effects::FORTRESS_SETTINGS_KIND.to_owned()),
                format!("{}{key}", effects::STANDING_ORDER_FIELD_PREFIX),
                Value::Text(value.clone()),
                step,
                meter,
            )?,
            _ => {
                return Err(
                    "configuration merge requires only stockpile and standing-order actions",
                );
            }
        }
        Ok(())
    }

    /// Only positive results established by the reviewed writes may vary from
    /// their old source facts. Guards and arbitrary boolean or aggregate reads
    /// remain on the existing explicit review path.
    fn result(&self, predicate: &Predicate, depth: usize, meter: &mut Meter) -> Check<()> {
        meter.charge(1, 8)?;
        if depth > MAX_PREDICATE_DEPTH {
            return Err("configuration result exceeds its predicate depth bound");
        }
        match predicate {
            Predicate::All(children) if !children.is_empty() && children.len() <= MAX_FIELDS => {
                for child in children {
                    self.result(child, depth + 1, meter)?;
                }
                Ok(())
            }
            Predicate::EntityKind { entity_id, kind }
                if self.kinds.get(entity_id) == Some(kind) =>
            {
                Ok(())
            }
            Predicate::FieldCompare {
                entity_id,
                field,
                op: CompareOp::Eq,
                value,
            } => {
                meter.text(field)?;
                configuration_value(field, value, meter)?;
                if self
                    .writes
                    .get(&(*entity_id, field.clone()))
                    .is_some_and(|write| &write.value == value)
                {
                    Ok(())
                } else {
                    Err("configuration result is not established by the exact reviewed writes")
                }
            }
            _ => Err("configuration merge refuses nonconfiguration result reads"),
        }
    }
}

fn configuration_value(field: &str, value: &Value, meter: &mut Meter) -> Check<()> {
    if field == effects::STOCKPILE_ACCEPTS_FIELD {
        let Value::List(items) = value else {
            return Err("stockpile filters must be a canonical text set");
        };
        if items.len() > 64 {
            return Err("stockpile filters exceed their configuration merge bound");
        }
        let mut prior: Option<&str> = None;
        for item in items {
            let Value::Text(token) = item else {
                return Err("stockpile filters must contain only text tokens");
            };
            meter.text(token)?;
            if token.is_empty() || prior.is_some_and(|prior| prior >= token.as_str()) {
                return Err("stockpile filters must be sorted distinct nonempty text tokens");
            }
            prior = Some(token);
        }
        meter.charge(1, 8)
    } else if [
        effects::STOCKPILE_MAX_BINS_FIELD,
        effects::STOCKPILE_MAX_BARRELS_FIELD,
        effects::STOCKPILE_MAX_WHEELBARROWS_FIELD,
    ]
    .contains(&field)
    {
        match value {
            Value::U64(value) if u32::try_from(*value).is_ok() => meter.charge(1, 32),
            _ => Err("stockpile capacity is not a bounded unsigned setting"),
        }
    } else if field
        .strip_prefix(effects::STANDING_ORDER_FIELD_PREFIX)
        .is_some_and(|key| !key.is_empty())
    {
        match value {
            Value::Text(text) if !text.is_empty() => meter.text(text),
            _ => Err("standing-order setting is not nonempty text"),
        }
    } else {
        Err("configuration merge encountered an unsupported field")
    }
}

fn qualified_fact(
    field: &str,
    fact: &Fact,
    tick: dfmcp_core::GameTick,
    meter: &mut Meter,
) -> Check<()> {
    meter.charge(1, 64)?;
    configuration_value(field, &fact.value, meter)?;
    match &fact.source {
        FactSource::Derived(source)
            if source == "dfmcp.lab-scenario/1" || source == "dfmcp.reference-effects/1" => {}
        _ => return Err("configuration merge requires qualified laboratory provenance"),
    }
    if fact.source_digest != Digest32::ZERO || fact.observed_at > tick {
        return Err("configuration merge requires current qualified laboratory facts");
    }
    match &fact.presence {
        None | Some(FactPresence::Absent) => Ok(()),
        Some(FactPresence::Known(value)) if value == &fact.value => {
            configuration_value(field, value, meter)
        }
        _ => Err("configuration merge cannot use unavailable or inconsistent presence"),
    }
}

fn record_identity(record: &EntityRecord, id: EntityId, kind: &EntityKind) -> Check<()> {
    if record.id != id
        || record.generation == 0
        || record.revision == 0
        || record.revision == u64::MAX
        || &record.kind != kind
    {
        return Err("configuration entity identity, generation, kind or revision is invalid");
    }
    Ok(())
}

fn creation(record: &EntityRecord, now: &WorldSnapshot, meter: &mut Meter) -> Check<()> {
    if record.generation != 1 || record.fields.is_empty() || record.fields.len() > MAX_FIELDS {
        return Err(
            "concurrent settings creation lacks its canonical generation or bounded fields",
        );
    }
    // The reference action creates exactly this shared identity and leaves
    // every other standing-order key untouched. No generic entity insertion
    // or replacement rule is inferred from this one registered action family.
    for (field, fact) in &record.fields {
        meter.text(field)?;
        if field.len() > 256
            || !field
                .strip_prefix(effects::STANDING_ORDER_FIELD_PREFIX)
                .is_some_and(|key| !key.is_empty())
        {
            return Err("concurrent settings creation contains unsupported fields");
        }
        qualified_fact(field, fact, now.tick, meter)?;
        if fact.known_value().is_none() {
            return Err("concurrent settings creation contains an unavailable setting");
        }
    }
    Ok(())
}

fn fact_description(fact: Option<&Fact>) -> Json {
    json!({
        "presence":match fact.map(|fact| fact.presence.as_ref()) {
            None => "missing",
            Some(Some(FactPresence::Absent)) => "absent",
            _ => "known",
        },
        "fact_digest":fact.map(|fact| Digest32::of_bytes(&fact.canonical_bytes()).to_hex()),
    })
}

fn setting_json(value: &Value) -> Check<Json> {
    Ok(match value {
        Value::U64(value) => json!({"type":"u64", "value":value}),
        Value::Text(value) => json!({"type":"text", "value":value}),
        Value::List(values) => {
            let values = values
                .iter()
                .map(|value| match value {
                    Value::Text(value) => Ok(value),
                    _ => Err("configuration certificate requires a canonical text set"),
                })
                .collect::<Check<Vec<_>>>()?;
            json!({"type":"text_set", "value":values})
        }
        _ => return Err("configuration certificate encountered an unsupported setting"),
    })
}

pub(super) fn prove(
    plan: &PreparedPlan,
    base: &WorldSnapshot,
    now: &WorldSnapshot,
    budget: WorkBudget,
) -> Check<FieldWitness> {
    if base.anchor() != plan.anchor
        || base.fortress_id != now.fortress_id
        || base.cursor.epoch != now.cursor.epoch
        || now.cursor.sequence <= base.cursor.sequence
        || now.tick != base.tick
    {
        return Err("configuration merge requires the sealed same-tick forward source lineage");
    }
    if plan.steps.is_empty() || plan.steps.len() > 64.min(budget.max_actions as usize) {
        return Err("configuration merge plan exceeds its action bound");
    }
    let mut meter = Meter::new(budget)?;
    let mut configuration = Configuration::default();
    for step in &plan.steps {
        if !step.preconditions.is_empty()
            || step.compensation.is_some()
            || step.obligation.is_some()
        {
            return Err(
                "configuration guards, compensation and temporal obligations require review",
            );
        }
        configuration.action(&step.action, step.id.get(), base.fortress_id, &mut meter)?;
    }
    configuration.result(&plan.terminal_condition, 0, &mut meter)?;
    for step in &plan.steps {
        if step.postconditions.is_empty() || step.postconditions.len() > MAX_FIELDS {
            return Err("configuration postconditions exceed their closed result bound");
        }
        for predicate in &step.postconditions {
            configuration.result(predicate, 0, &mut meter)?;
        }
    }
    if configuration.kinds.len() > budget.max_entities as usize {
        return Err("configuration merge scope exceeds the entity budget");
    }
    model_free(base, budget, &mut meter)?;
    model_free(now, budget, &mut meter)?;
    let settings = effects::fortress_settings_entity_id(base.fortress_id);
    let mut identities = Vec::new();
    for (id, kind) in &configuration.kinds {
        meter.charge(1, 256)?;
        let before = base.graph.entities.get(id);
        let after = now.graph.entities.get(id);
        for record in before.into_iter().chain(after) {
            record_identity(record, *id, kind)?;
        }
        let disposition = match (before, after) {
            (Some(before), Some(after))
                if before.generation == after.generation && after.revision >= before.revision =>
            {
                "retained_identity"
            }
            (None, None) if *id == settings => "create_canonical_settings",
            (None, Some(after)) if *id == settings => {
                creation(after, now, &mut meter)?;
                "reuse_concurrently_created_canonical_settings"
            }
            _ => return Err("configuration entity was removed, replaced or regressed"),
        };
        identities.push(json!({
            "entity_id":id.to_string(), "kind":kind.as_str(), "disposition":disposition,
            "base_generation":before.map(|record| record.generation),
            "current_generation":after.map(|record| record.generation),
            "base_revision":before.map(|record| record.revision),
            "current_revision":after.map(|record| record.revision),
        }));
    }
    let mut writes = Vec::new();
    for ((id, field), write) in configuration.writes {
        meter.charge(1, 256)?;
        let before = base
            .graph
            .entities
            .get(&id)
            .and_then(|record| record.fields.get(&field));
        let after = now
            .graph
            .entities
            .get(&id)
            .and_then(|record| record.fields.get(&field));
        for fact in before.into_iter().chain(after) {
            qualified_fact(&field, fact, base.tick, &mut meter)?;
        }
        if before != after
            && base.graph.entities.get(&id).is_some_and(|record| {
                now.graph
                    .entities
                    .get(&id)
                    .is_some_and(|current| current.revision <= record.revision)
            })
        {
            return Err("a configuration write fact changed without an advancing revision");
        }
        let disposition = if before == after {
            "unchanged_base_fact"
        } else if after.and_then(Fact::known_value) == Some(&write.value) {
            "converged_to_requested_value"
        } else {
            return Err(
                "a configuration write diverged from both its base fact and requested value",
            );
        };
        writes.push(json!({
            "entity_id":id.to_string(), "field":field, "steps":write.steps,
            "requested_value":setting_json(&write.value)?, "base":fact_description(before),
            "current":fact_description(after), "disposition":disposition,
        }));
    }
    meter.charge(1, 0)?;
    Ok(FieldWitness {
        configuration: true,
        description: json!({
            "domain":"laboratory_model_free_configuration/1",
            "identities":identities, "writes":writes,
            "tie_break":"preserve_current_unwritten_fields; accept_unchanged_base_or_identical_requested_value; refuse_divergence",
            "model_absence":{"coverage":"complete_entities_and_edges", "fields":MODEL_FIELDS,
                "entity_kinds":[effects::HARVEST_TASK_KIND]},
            "bounds":{"identities":MAX_IDENTITIES, "fields":MAX_FIELDS,
                "records_per_version":MAX_RECORDS.min(budget.max_entities as usize),
                "operations":MAX_OPERATIONS, "bytes":meter.max_bytes},
        }),
    })
}

#[cfg(test)]
#[path = "configuration_witness_tests.rs"]
mod tests;
