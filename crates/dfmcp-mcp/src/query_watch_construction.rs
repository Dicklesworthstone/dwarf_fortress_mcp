//! Bounded whole-plan furniture goals. Expand one target at a time into the
//! EXISTING field/count/relationship evaluator; never build a second truth or
//! watch state machine. Every expansion shares the caller's evaluation budget.
use super::*;
use dfmcp_adapter::live_operations::{building_entity_id, item_entity_id};
use dfmcp_world::EntityKind;
use std::collections::BTreeSet;

pub(super) const POLICY: &str = "dfmcp.furniture-set-condition/1";
pub(super) const MAX_TARGETS: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Test {
    AllComplete,
    AnyRemoval,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Kind {
    Bed,
    Chair,
    Table,
}
impl Kind {
    fn names(self) -> (&'static str, &'static str) {
        match self {
            Self::Bed => ("Bed", "BED"),
            Self::Chair => ("Chair", "CHAIR"),
            Self::Table => ("Table", "TABLE"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Target {
    pub(super) building_native_id: u32,
    pub(super) building_generation: u32,
    pub(super) kind: Kind,
    pub(super) max_stage: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) item_native_id: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) item_generation: Option<u32>,
}

pub(super) fn validate(targets: &[Target]) -> Result<()> {
    if targets.is_empty() || targets.len() > MAX_TARGETS {
        return Err(bounded("furniture set requires 1..32 complete targets"));
    }
    let mut prior = None;
    let mut items = BTreeSet::new();
    for target in targets {
        if target.building_native_id >= i32::MAX as u32
            || target.building_generation == 0
            || !(1..=32).contains(&target.max_stage)
            || prior.is_some_and(|id| id >= target.building_native_id)
            || target.item_native_id.is_some() != target.item_generation.is_some()
            || target.item_generation == Some(0)
        {
            return Err(invalid("invalid, unordered or duplicate furniture target"));
        }
        if let Some(item) = target.item_native_id {
            if item >= i32::MAX as u32 || !items.insert(item) {
                return Err(invalid("invalid or multiply assigned furniture item"));
            }
        }
        prior = Some(target.building_native_id);
    }
    Ok(())
}

fn literal(kind: &str, value: Value) -> Value {
    json!({"type":kind,"value":value})
}
fn field(id: EntityId, generation: u32, name: &str, value: Value) -> Value {
    json!({"op":"field","entity_id":id.to_string(),"generation":generation,
        "field":name,"comparison":"eq","value":value})
}
fn row_field(name: &str, value: Value) -> Value {
    json!({"op":"field","field":name,"comparison":"eq","value":value})
}
fn related(id: EntityId, generation: u32, relation: &str, direction: &str) -> Value {
    json!({"op":"related","entity_id":id.to_string(),"generation":generation,
        "relation":relation,"direction":direction})
}
fn count(kind: &str, predicate: Value, value: u64) -> Value {
    json!({"op":"entity_count","scope":"observed_projection","kind":kind,
        "predicate":predicate,"comparison":"eq","value":value})
}
fn all(args: Vec<Value>) -> Value {
    json!({"op":"all","args":args})
}

/// Exact recipe used by the original per-building construction proposal.
/// The only runtime variable is a validated, closed target. Expansion is not an
/// arbitrary nested condition supplied by a caller; it cannot recurse into sets.
fn recipe(target: &Target, test: Test) -> Value {
    let building = building_entity_id(target.building_native_id);
    let generation = target.building_generation;
    let (building_kind, item_kind) = target.kind.names();
    let removal = row_field("type_key", literal("text", json!("DestroyBuilding")));
    let selected_jobs = all(vec![
        related(building, generation, "contained_in", "incoming"),
        if test == Test::AnyRemoval { removal } else {
            json!({"op":"any","args":[removal,
                row_field("type_key",literal("text",json!("ConstructBuilding")))]})
        },
    ]);
    if test == Test::AnyRemoval {
        let mut condition = count("job", selected_jobs, 0);
        condition["comparison"] = json!("gt");
        return condition;
    }
    let mut conditions = vec![
        field(building, generation, "type_key", literal("text", json!(building_kind))),
        field(building, generation, "build_stage", literal("i64", json!(target.max_stage))),
        field(building, generation, "max_build_stage", literal("i64", json!(target.max_stage))),
        count("job", selected_jobs, 0),
    ];
    if let (Some(native), Some(generation)) = (target.item_native_id, target.item_generation) {
        let item = item_entity_id(native);
        conditions.push(field(item, generation, "type_key", literal("text", json!(item_kind))));
        for (name, value) in [("in_building", true), ("in_job", false), ("removed", false),
            ("on_ground", false), ("in_inventory", false)] {
            conditions.push(field(item, generation, name, literal("bool", json!(value))));
        }
        conditions.push(count("item", all(vec![
            row_field("native_item_id", literal("u64", json!(native))),
            related(building, target.building_generation, "contained_in", "incoming"),
        ]), 1));
        conditions.push(count("item", related(item, generation, "contained_in", "outgoing"), 0));
        conditions.push(count("job", related(item, generation, "uses", "incoming"), 0));
    }
    all(conditions)
}

fn bind_root(
    snapshot: &WorldSnapshot, id: EntityId, generation: u32, kind: EntityKind, probe: &mut Probe,
) -> bool {
    let Some(entity) = snapshot.graph.entities.get(&id) else { return false; };
    probe.invalid_generation |= entity.generation != generation;
    entity.generation == generation && entity.revision != 0 && entity.kind == kind
}

pub(super) fn evaluate(
    probe: &mut Probe, snapshot: &WorldSnapshot, targets: &[Target], test: Test,
    budget: &mut counts::EvaluationBudget,
) -> Result<Truth> {
    validate(targets)?;
    budget.check()?;
    let mut rows = Vec::with_capacity(targets.len());
    let mut decisive = false;
    let mut unknown = false;
    for target in targets {
        budget.charge()?;
        let mut local = Probe::default();
        // Bind EVERY explicit reference, even for removal-only tests and after
        // an earlier decisive target. Recycled IDs must not hide in a group.
        let mut bound = bind_root(snapshot, building_entity_id(target.building_native_id),
            target.building_generation, EntityKind::Building, &mut local);
        if let (Some(id), Some(generation)) = (target.item_native_id, target.item_generation) {
            bound &= bind_root(snapshot, item_entity_id(id), generation, EntityKind::Item, &mut local);
        }
        let condition: Condition = serde_json::from_value(recipe(target, test))
            .map_err(|_| invalid("internal furniture recipe is invalid"))?;
        let evaluated = local.evaluate_bounded(&condition, snapshot, budget)?;
        let truth = if bound { evaluated } else { Truth::Unknown };
        probe.invalid_generation |= local.invalid_generation;
        decisive |= match test {
            Test::AllComplete => truth == Truth::False,
            Test::AnyRemoval => truth == Truth::True,
        };
        unknown |= truth == Truth::Unknown;
        // Retain whole-selection diagnostics rather than a huge repeated
        // predicate trace. Its digest covers the exact anchor, target and full
        // shared-engine evidence. A singleton condition_evaluation drills down.
        let evidence = digest(&json!({"domain":POLICY,"anchor":anchor(snapshot.anchor()),
            "target":target,"test":test,"truth":truth.text(),"roots_bound":bound,
            "generation_mismatch":local.invalid_generation,"facts":local.facts}))?;
        rows.push(json!({"building_native_id":target.building_native_id,
            "item_native_id":target.item_native_id,"truth":truth.text(),
            "roots_bound":bound,"generation_mismatch":local.invalid_generation,
            "evidence_digest":evidence.to_string()}));
        budget.check()?;
    }
    let truth = if decisive {
        Truth::from_bool(test == Test::AnyRemoval)
    } else if unknown { Truth::Unknown } else { Truth::from_bool(test == Test::AllComplete) };
    probe.facts.push(json!({"op":"furniture_set","policy":POLICY,"test":test,
        "truth":truth.text(),"selected":targets.len(),"records":rows,
        "records_complete":true,"scope":"observed_projection",
        "native_effect_completion_proven":false}));
    budget.check()?;
    Ok(truth)
}

pub(super) fn extend_schema(mut schema: Value) -> Result<Value> {
    let extension: Value = serde_json::from_str(include_str!(
        "../../../schemas/mcp_watch_furniture_set_v1.json"))
        .map_err(|_| invalid("embedded furniture-set schema is invalid"))?;
    schema["$defs"]["watch_condition"]["oneOf"].as_array_mut()
        .ok_or_else(|| invalid("watch condition variants absent"))?.push(extension);
    Ok(schema)
}

#[cfg(test)]
#[path = "query_watch_construction_tests.rs"]
mod tests;
