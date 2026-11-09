//! The real retained-source parser and compiler, independent of transport state.

use super::*;
use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, EntityId, FortressId, GameTick,
    ObservationCursor, OperationContext, RequestId, SessionId, WorkBudget,
};
use dfmcp_intent::{PreparedPlan, effects};
use dfmcp_world::{EntityKind, EntityRecord, Fact, FactSource, Value as WorldValue, WorldGraph};

const LEDGER: EntityId = EntityId::new(91);

fn insert(
    snapshot: &mut WorldSnapshot,
    id: EntityId,
    kind: EntityKind,
    fields: Vec<(&str, WorldValue)>,
) {
    snapshot.graph.entities.insert(
        id,
        EntityRecord {
            id,
            generation: 1,
            revision: 1,
            kind,
            label: format!("acquisition source fixture {id}"),
            fields: fields
                .into_iter()
                .map(|(name, value)| {
                    (
                        name.to_owned(),
                        Fact::known(
                            value,
                            snapshot.tick,
                            FactSource::Derived("dfmcp.lab-scenario/1".to_owned()),
                            Digest32::ZERO,
                        ),
                    )
                })
                .collect(),
        },
    );
    snapshot.refresh_hash();
}

fn world() -> WorldSnapshot {
    let mut snapshot = WorldSnapshot::new(
        FortressId::new(8_272),
        GameTick(1_000),
        ObservationCursor::ORIGIN,
        false,
        WorldGraph::default(),
    );
    let settings = effects::fortress_settings_entity_id(snapshot.fortress_id);
    insert(
        &mut snapshot,
        settings,
        EntityKind::Other(effects::FORTRESS_SETTINGS_KIND.to_owned()),
        vec![
            (
                effects::LABORATORY_WORKFORCE_MODEL_FIELD,
                WorldValue::Text(effects::LABORATORY_WORKFORCE_MODEL.to_owned()),
            ),
            (
                effects::LABORATORY_HARVEST_MODEL_FIELD,
                WorldValue::Text(effects::LABORATORY_HARVEST_MODEL.to_owned()),
            ),
            (
                effects::LABORATORY_PROVISIONS_MODEL_FIELD,
                WorldValue::Text(effects::LABORATORY_PROVISIONS_MODEL.to_owned()),
            ),
        ],
    );
    insert(
        &mut snapshot,
        LEDGER,
        EntityKind::Other(effects::STOCK_LEDGER_KIND.to_owned()),
        vec![
            (effects::STOCK_WOOD_FIELD, WorldValue::U64(3)),
            (effects::STOCK_PLANT_FIELD, WorldValue::U64(1)),
            (effects::STOCK_FOOD_FIELD, WorldValue::U64(10_000)),
            (effects::STOCK_DRINK_FIELD, WorldValue::U64(10_000)),
            (effects::STOCK_BED_FIELD, WorldValue::U64(0)),
            (effects::METABOLISM_TICKS_FIELD, WorldValue::U64(0)),
        ],
    );
    insert(
        &mut snapshot,
        EntityId::new(11),
        EntityKind::Unit,
        vec![
            ("alive", WorldValue::Bool(true)),
            ("labor.WOODCUTTING", WorldValue::Bool(true)),
            ("labor.PLANT_GATHERING", WorldValue::Bool(true)),
            (effects::SQUAD_FIELD, WorldValue::Null),
        ],
    );
    for (id, resource) in [(81, HarvestResource::Wood), (82, HarvestResource::Plant)] {
        insert(
            &mut snapshot,
            EntityId::new(id),
            EntityKind::Plant,
            vec![
                (
                    effects::HARVEST_SOURCE_MODEL_FIELD,
                    WorldValue::Text(effects::LABORATORY_HARVEST_MODEL.to_owned()),
                ),
                (
                    effects::HARVEST_RESOURCE_FIELD,
                    WorldValue::Text(resource.as_str().to_owned()),
                ),
                (effects::HARVEST_INITIAL_FIELD, WorldValue::U64(40)),
                (effects::HARVEST_REMAINING_FIELD, WorldValue::U64(40)),
                (effects::HARVEST_YIELDED_FIELD, WorldValue::U64(0)),
            ],
        );
    }
    snapshot
}

fn raw(assign_labor: bool) -> String {
    json!({
        "template": "resource_acquisition",
        "quotas": [{"resource": "PLANT", "minimum": 4}, {"resource": "WOOD", "minimum": 5}],
        "assign_labor": assign_labor,
    })
    .to_string()
}

fn prepare(snapshot: &WorldSnapshot, intent: &Intent) -> Result<PreparedPlan> {
    StaticPlanner::default().prepare_laboratory(
        snapshot,
        intent,
        &OperationContext {
            session_id: SessionId::new(8_272),
            request_id: RequestId::new(1),
            anchor: snapshot.anchor(),
            budget: WorkBudget::CONSERVATIVE_DEFAULT,
            cancellation_requested: false,
            grants: vec![CapabilityGrant {
                capability: Capability::Plan,
                max_risk: RiskTier::ReadOnly,
                scope: CapabilityScope::default(),
                expires_at_tick: None,
                remaining_uses: None,
            }],
        },
    )
}

#[test]
fn current_intake_pins_archived_algorithm_and_canonical_quota_order() -> Result<()> {
    let request = ResourceAcquisitionRequest::parse_current(&raw(false))?;
    let canonical = request.canonical_json();
    let saved: Value =
        serde_json::from_str(&canonical).map_err(|error| invalid(error.to_string()))?;
    assert_eq!(saved["planner"], PLANNER);
    assert_eq!(saved["quotas"][0]["resource"], "WOOD");
    assert_eq!(saved["quotas"][1]["resource"], "PLANT");
    assert_eq!(saved["assign_labor"], false);
    assert_eq!(ResourceAcquisitionRequest::parse(&canonical)?, request);
    assert!(ResourceAcquisitionRequest::parse(&raw(false)).is_err());
    let mut omitted = saved.clone();
    if let Some(object) = omitted.as_object_mut() {
        object.remove("assign_labor");
    }
    assert_eq!(
        ResourceAcquisitionRequest::parse_current(&omitted.to_string())?.canonical_json(),
        canonical
    );
    Ok(())
}

#[test]
fn malformed_or_ambiguous_original_sources_are_refused() -> Result<()> {
    let baseline: Value =
        serde_json::from_str(&raw(false)).map_err(|error| invalid(error.to_string()))?;
    for (field, value) in [
        ("template", json!("production")),
        ("planner", json!("resource_acquisition_v2")),
        ("planner", Value::Null),
        ("assign_labor", Value::Null),
        ("assign_labor", json!("true")),
        ("unknown", json!(true)),
        ("quotas", Value::Null),
        ("quotas", json!([])),
        (
            "quotas",
            json!([{"resource":"WOOD","minimum":1},{"resource":"WOOD","minimum":2}]),
        ),
        (
            "quotas",
            json!([{"resource":"WOOD","minimum":1},{"resource":"PLANT","minimum":2},{"resource":"WOOD","minimum":3}]),
        ),
    ] {
        let mut request = baseline.clone();
        request[field] = value;
        assert!(
            ResourceAcquisitionRequest::parse_current(&request.to_string()).is_err(),
            "{request}"
        );
    }
    for (field, value) in [
        ("resource", json!("wood")),
        ("resource", json!("FOOD")),
        ("resource", Value::Null),
        ("minimum", json!(0)),
        ("minimum", json!(-1)),
        ("minimum", json!(1.5)),
        ("minimum", Value::Null),
        ("minimum", json!("2")),
        ("source", json!("81")),
    ] {
        let mut request = baseline.clone();
        request["quotas"][0][field] = value;
        assert!(
            ResourceAcquisitionRequest::parse_current(&request.to_string()).is_err(),
            "{request}"
        );
    }
    for request in [
        r#"{"template":"resource_acquisition","template":"resource_acquisition","quotas":[{"resource":"WOOD","minimum":2}]}"#,
        r#"{"template":"resource_acquisition","assign_labor":false,"assign_labor":true,"quotas":[{"resource":"WOOD","minimum":2}]}"#,
        r#"{"template":"resource_acquisition","quotas":[{"resource":"WOOD","resource":"PLANT","minimum":2}]}"#,
        r#"{"template":"resource_acquisition","quotas":[{"resource":"WOOD","minimum":2,"minimum":3}]}"#,
        r#"{"template":"resource_acquisition","quotas":[{"resource":"WOOD","minimum":18446744073709551616}]}"#,
    ] {
        assert!(
            ResourceAcquisitionRequest::parse_current(request).is_err(),
            "{request}"
        );
    }
    assert!(
        ResourceAcquisitionRequest::parse_current(&format!(
            "{}{}",
            raw(false),
            " ".repeat(MAX_REQUEST_BYTES)
        ))
        .is_err()
    );
    assert!(ResourceAcquisitionRequest::parse_current(&format!("{}\0", raw(false))).is_err());
    Ok(())
}

#[test]
fn original_options_are_sealed_even_when_the_lowered_actions_are_equal() -> Result<()> {
    let snapshot = world();
    let id = IntentId::new(8_272);
    let (disabled, disabled_analysis) =
        ResourceAcquisitionRequest::parse_current(&raw(false))?.compile(id, &snapshot, "")?;
    let (enabled, enabled_analysis) =
        ResourceAcquisitionRequest::parse_current(&raw(true))?.compile(id, &snapshot, "")?;
    assert_eq!(disabled.requested_actions, enabled.requested_actions);
    assert_eq!(disabled.terminal_condition, enabled.terminal_condition);
    assert_ne!(
        disabled_analysis["source_digest"],
        enabled_analysis["source_digest"]
    );
    assert_ne!(
        prepare(&snapshot, &disabled)?.digest,
        prepare(&snapshot, &enabled)?.digest
    );
    assert_eq!(disabled_analysis["requirements"][0]["observed"], 3);
    assert_eq!(disabled_analysis["requirements"][0]["minimum"], 5);
    assert_eq!(disabled_analysis["requirements"][0]["deficit"], 2);
    assert_eq!(disabled_analysis["requirements"][1]["deficit"], 3);
    assert_eq!(
        disabled_analysis["pending_harvest_credited_as_inventory"],
        false
    );
    assert_eq!(disabled_analysis["ledger_entity_id"], LEDGER.to_string());
    assert_eq!(disabled_analysis["ledger_generation"], 1);
    assert_eq!(disabled_analysis["allocations"][0]["source_generation"], 1);
    assert_eq!(disabled_analysis["native_execution_supported"], false);
    Ok(())
}

#[test]
fn saved_original_source_replays_the_exact_plan_and_never_rewrites_quotas() -> Result<()> {
    let snapshot = world();
    let current = ResourceAcquisitionRequest::parse_current(&raw(false))?;
    let restored = ResourceAcquisitionRequest::parse(&current.canonical_json())?;
    let (intent, analysis) =
        current.compile(IntentId::new(8_272), &snapshot, "original objective")?;
    let (replayed, replayed_analysis) =
        restored.compile(IntentId::new(8_272), &snapshot, "original objective")?;
    assert_eq!(analysis, replayed_analysis);
    assert_eq!(prepare(&snapshot, &intent)?, prepare(&snapshot, &replayed)?);
    assert_eq!(
        analysis["original_request"]["quotas"],
        json!([
            {"resource":"WOOD","minimum":5}, {"resource":"PLANT","minimum":4}
        ])
    );
    Ok(())
}

#[test]
fn satisfied_original_stock_is_an_explicit_effect_free_no_work_result() -> Result<()> {
    let snapshot = world();
    let request = ResourceAcquisitionRequest::parse_current(
        r#"{"template":"resource_acquisition","quotas":[{"resource":"WOOD","minimum":3},{"resource":"PLANT","minimum":1}]}"#,
    )?;
    let error = request
        .compile(IntentId::new(8_272), &snapshot, "")
        .err()
        .ok_or_else(|| invalid("satisfied stock unexpectedly produced effects"))?;
    assert_eq!(error.code, ErrorCode::InvalidIntent);
    assert!(error.message.contains("already satisfied"));
    Ok(())
}

#[test]
fn original_source_seal_respects_the_summary_bound() -> Result<()> {
    let request = ResourceAcquisitionRequest::parse_current(&raw(false))?;
    for summary in ["x".repeat(256), "unsafe\nsummary".to_owned()] {
        assert!(
            request
                .compile(IntentId::new(8_272), &world(), &summary)
                .is_err()
        );
    }
    Ok(())
}
