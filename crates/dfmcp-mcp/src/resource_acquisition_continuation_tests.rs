//! Exact retained stock goals, strict lineage bytes, and real residual harvests.

use super::*;
use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, EntityId, FortressId, GameTick,
    ObservationCursor, OperationContext, RequestId, RiskTier, SessionId, WorkBudget,
};
use dfmcp_intent::{Action, HarvestResource, PreparedPlan, StaticPlanner, effects};
use dfmcp_world::{
    EntityKind, EntityRecord, Fact, FactPresence, FactSource, Predicate, PredicateEvidence,
    Value as WorldValue, WorldGraph,
};

const LEDGER: EntityId = EntityId::new(91);
const WORKER: EntityId = EntityId::new(11);
const WOOD: EntityId = EntityId::new(81);

fn fact(value: WorldValue, tick: GameTick) -> Fact {
    Fact::known(
        value,
        tick,
        FactSource::Derived("dfmcp.lab-scenario/1".to_owned()),
        Digest32::ZERO,
    )
}

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
            label: format!("resource continuation fixture {id}"),
            fields: fields
                .into_iter()
                .map(|(name, value)| (name.to_owned(), fact(value, snapshot.tick)))
                .collect(),
        },
    );
    snapshot.refresh_hash();
}

fn world() -> WorldSnapshot {
    let mut snapshot = WorldSnapshot::new(
        FortressId::new(8_273),
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
            (effects::METABOLISM_TICKS_FIELD, WorldValue::U64(0)),
        ],
    );
    insert(
        &mut snapshot,
        WORKER,
        EntityKind::Unit,
        vec![
            ("alive", WorldValue::Bool(true)),
            ("labor.WOODCUTTING", WorldValue::Bool(true)),
            ("labor.PLANT_GATHERING", WorldValue::Bool(true)),
            (effects::SQUAD_FIELD, WorldValue::Null),
        ],
    );
    for (id, resource) in [
        (WOOD, HarvestResource::Wood),
        (EntityId::new(82), HarvestResource::Plant),
    ] {
        insert(
            &mut snapshot,
            id,
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

fn request(assign_labor: bool) -> Result<ResourceAcquisitionRequest> {
    ResourceAcquisitionRequest::parse_current(
        &json!({
            "template": "resource_acquisition",
            "quotas": [{"resource":"PLANT","minimum":4},{"resource":"WOOD","minimum":5}],
            "assign_labor": assign_labor,
        })
        .to_string(),
    )
}

fn source() -> Result<ResourceAcquisitionContinuation> {
    let root = Digest32::of_bytes(b"original resource stock objective");
    ResourceAcquisitionContinuation::new(root, root, request(true)?)
}

fn prepare(snapshot: &WorldSnapshot, intent: &Intent) -> Result<PreparedPlan> {
    StaticPlanner::default().prepare_laboratory(
        snapshot,
        intent,
        &OperationContext {
            session_id: SessionId::new(8_273),
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

fn set(snapshot: &mut WorldSnapshot, id: EntityId, name: &str, value: WorldValue) -> Result<()> {
    snapshot
        .graph
        .entities
        .get_mut(&id)
        .ok_or_else(|| invalid("missing fixture entity"))?
        .fields
        .insert(name.to_owned(), fact(value, snapshot.tick));
    snapshot.refresh_hash();
    Ok(())
}

fn apply(snapshot: &mut WorldSnapshot, action: &Action, key: &str) -> Result<()> {
    let mut candidate = snapshot.clone();
    effects::apply_effect(&mut candidate, action, key)?;
    candidate.refresh_hash();
    *snapshot = candidate;
    Ok(())
}

fn advance(snapshot: &mut WorldSnapshot, ticks: u64) -> Result<()> {
    let mut candidate = snapshot.clone();
    candidate.tick = candidate
        .tick
        .checked_add(ticks)
        .ok_or_else(|| invalid("fixture clock overflow"))?;
    effects::advance_effects(&mut candidate, ticks)?;
    candidate.refresh_hash();
    *snapshot = candidate;
    Ok(())
}

#[test]
fn fixed_original_request_and_flat_lineage_round_trip_without_growth() -> Result<()> {
    let first = source()?;
    let raw = first.canonical_json();
    let value: Value = serde_json::from_str(&raw).map_err(|error| invalid(error.to_string()))?;
    assert_eq!(value["schema"], SOURCE_SCHEMA);
    assert_eq!(value["planner"], PLANNER);
    assert_eq!(value["resource_acquisition"]["planner"], PLANNER);
    assert_eq!(value["resource_acquisition"]["assign_labor"], true);
    assert_eq!(
        value["resource_acquisition"]["quotas"],
        json!([
            {"resource":"WOOD","minimum":5}, {"resource":"PLANT","minimum":4},
        ])
    );
    assert_eq!(ResourceAcquisitionContinuation::parse(&raw)?, first);
    let mut bytes = SOURCE_DOMAIN.to_vec();
    bytes.extend_from_slice(raw.as_bytes());
    assert_eq!(first.source_digest(), Digest32::of_bytes(&bytes));
    let mut next = first.clone();
    for index in 0..32 {
        next = next.next(Digest32::of_bytes(&[index]))?;
        assert_eq!(next.root, first.root);
        assert_eq!(next.request, first.request);
        assert_eq!(next.canonical_json().len(), raw.len());
        assert_eq!(
            ResourceAcquisitionContinuation::parse(&next.canonical_json())?,
            next
        );
    }
    assert!(!first.same_original_request(&next));
    assert!(
        next.same_original_request(&ResourceAcquisitionContinuation::parse(
            &next.canonical_json()
        )?)
    );
    assert_eq!(next.lineage_json()["historical_exact_goal_unchanged"], true);
    assert_eq!(next.lineage_json()["replacement_work_dispatched"], false);
    Ok(())
}

#[test]
fn strict_outer_and_nested_grammar_rejects_unknown_null_missing_and_future_sources() -> Result<()> {
    let first = source()?;
    let baseline: Value = serde_json::from_str(&first.canonical_json())
        .map_err(|error| invalid(error.to_string()))?;
    for (field, replacement) in [
        ("schema", json!("dfmcp.resource-acquisition-continuation/2")),
        ("schema", Value::Null),
        ("planner", json!("resource_acquisition_v2")),
        ("planner", Value::Null),
        ("parent_plan_digest", json!(Digest32::ZERO.to_hex())),
        ("root_plan_digest", json!("not a digest")),
        ("root_plan_digest", Value::Null),
        ("resource_acquisition", Value::Null),
        ("work_quiescent", json!(true)),
        ("authority", json!(["designate"])),
    ] {
        let mut changed = baseline.clone();
        changed[field] = replacement;
        assert!(
            ResourceAcquisitionContinuation::parse(&changed.to_string()).is_err(),
            "{changed}"
        );
    }
    for (field, replacement) in [
        ("template", json!("production")),
        ("planner", json!("resource_acquisition_v2")),
        ("planner", Value::Null),
        ("assign_labor", Value::Null),
        ("assign_labor", json!("true")),
        ("extra", json!(true)),
        ("quotas", json!([])),
        (
            "quotas",
            json!([{"resource":"WOOD","minimum":1},{"resource":"WOOD","minimum":2}]),
        ),
        ("quotas", json!([{"resource":"WOOD","minimum":0}])),
        ("quotas", json!([{"resource":"WOOD","minimum":1.5}])),
        (
            "quotas",
            json!([{"resource":"WOOD","minimum":1,"source":"81"}]),
        ),
    ] {
        let mut changed = baseline.clone();
        changed["resource_acquisition"][field] = replacement;
        assert!(
            ResourceAcquisitionContinuation::parse(&changed.to_string()).is_err(),
            "{changed}"
        );
    }
    for field in [
        "schema",
        "planner",
        "parent_plan_digest",
        "root_plan_digest",
        "resource_acquisition",
    ] {
        let mut changed = baseline.clone();
        changed
            .as_object_mut()
            .ok_or_else(|| invalid("missing outer object"))?
            .remove(field);
        assert!(ResourceAcquisitionContinuation::parse(&changed.to_string()).is_err());
    }
    let mut missing_inner = baseline.clone();
    missing_inner["resource_acquisition"]
        .as_object_mut()
        .ok_or_else(|| invalid("missing inner object"))?
        .remove("planner");
    assert!(ResourceAcquisitionContinuation::parse(&missing_inner.to_string()).is_err());
    assert!(
        ResourceAcquisitionContinuation::new(Digest32::ZERO, first.root, request(true)?).is_err()
    );
    assert!(
        ResourceAcquisitionContinuation::new(first.parent, Digest32::ZERO, request(true)?).is_err()
    );
    assert!(
        ResourceAcquisitionContinuation::parse(&format!("{}\0", first.canonical_json())).is_err()
    );
    assert!(
        ResourceAcquisitionContinuation::parse(&format!(
            "{}{}",
            first.canonical_json(),
            " ".repeat(MAX_REQUEST_BYTES)
        ))
        .is_err()
    );
    Ok(())
}

#[test]
fn nested_duplicate_keys_are_rejected_before_any_value_normalization() -> Result<()> {
    let raw = source()?.canonical_json();
    for (needle, replacement) in [
        (
            format!("\"schema\":\"{SOURCE_SCHEMA}\""),
            format!("\"schema\":\"{SOURCE_SCHEMA}\",\"schema\":\"{SOURCE_SCHEMA}\""),
        ),
        (
            "\"assign_labor\":true".to_owned(),
            "\"assign_labor\":false,\"assign_labor\":true".to_owned(),
        ),
        (
            "\"template\":\"resource_acquisition\"".to_owned(),
            "\"template\":\"resource_acquisition\",\"template\":\"resource_acquisition\""
                .to_owned(),
        ),
        (
            "\"minimum\":5".to_owned(),
            "\"minimum\":1,\"minimum\":5".to_owned(),
        ),
        (
            "\"resource\":\"WOOD\"".to_owned(),
            "\"resource\":\"PLANT\",\"resource\":\"WOOD\"".to_owned(),
        ),
    ] {
        let changed = raw.replace(&needle, &replacement);
        assert_ne!(changed, raw);
        assert!(
            ResourceAcquisitionContinuation::parse(&changed).is_err(),
            "{changed}"
        );
    }
    let doubled_request = raw.replacen(
        "\"resource_acquisition\":",
        &format!(
            "\"resource_acquisition\":{},\"resource_acquisition\":",
            source()?.request.canonical_json()
        ),
        1,
    );
    assert!(ResourceAcquisitionContinuation::parse(&doubled_request).is_err());
    Ok(())
}

#[test]
fn one_seal_binds_parent_root_and_unused_assignment_permission() -> Result<()> {
    let snapshot = world();
    let original_snapshot = snapshot.clone();
    let first = source()?;
    let id = IntentId::new(8_273);
    let summary = "x".repeat(150);
    let (intent, analysis) = first.compile(id, &snapshot, &summary)?;
    assert_eq!(
        intent
            .summary
            .matches("resource acquisition continuation")
            .count(),
        1
    );
    assert!(!intent.summary.contains("resource acquisition source"));
    assert!(intent.summary.len() <= StaticPlanner::default().policy.max_string_bytes);
    assert_eq!(analysis["source_digest"], first.source_digest().to_hex());
    assert_eq!(analysis["continuation"], first.lineage_json());
    let sealed = prepare(&snapshot, &intent)?;
    for changed in [
        first.next(Digest32::of_bytes(b"different parent"))?,
        ResourceAcquisitionContinuation::new(
            first.parent,
            Digest32::of_bytes(b"different root"),
            first.request.clone(),
        )?,
        ResourceAcquisitionContinuation::new(first.parent, first.root, request(false)?)?,
    ] {
        let (other, _) = changed.compile(id, &snapshot, &summary)?;
        assert_eq!(intent.requested_actions, other.requested_actions);
        assert_eq!(intent.terminal_condition, other.terminal_condition);
        assert_ne!(prepare(&snapshot, &other)?.digest, sealed.digest);
    }
    let (root_intent, _) = first.request.compile(id, &snapshot, &summary)?;
    assert_ne!(prepare(&snapshot, &root_intent)?.digest, sealed.digest);
    assert_eq!(
        snapshot, original_snapshot,
        "pure compilation mutated the source"
    );
    Ok(())
}

#[test]
fn saved_continuation_reconstructs_exact_intent_analysis_and_prepared_seal() -> Result<()> {
    let snapshot = world();
    let source = source()?;
    let (intent, analysis) =
        source.compile(IntentId::new(92), &snapshot, "preserve both quotas")?;
    let (replayed, replayed_analysis) = ResourceAcquisitionContinuation::parse(
        &source.canonical_json(),
    )?
    .compile(IntentId::new(92), &snapshot, "preserve both quotas")?;
    assert_eq!(replayed, intent);
    assert_eq!(replayed_analysis, analysis);
    assert_eq!(prepare(&snapshot, &replayed)?, prepare(&snapshot, &intent)?);
    Ok(())
}

#[test]
fn residual_lowering_retains_met_companion_minimum_and_original_assignment_policy() -> Result<()> {
    let mut snapshot = world();
    set(
        &mut snapshot,
        LEDGER,
        effects::STOCK_WOOD_FIELD,
        WorldValue::U64(4),
    )?;
    set(
        &mut snapshot,
        LEDGER,
        effects::STOCK_PLANT_FIELD,
        WorldValue::U64(4),
    )?;
    set(
        &mut snapshot,
        WORKER,
        "labor.WOODCUTTING",
        WorldValue::Bool(false),
    )?;
    let source = source()?;
    assert_eq!(source.stock_goal_truth(&snapshot)?, PredicateTruth::False);
    let (intent, analysis) = source.compile(IntentId::new(93), &snapshot, "")?;
    assert_eq!(intent.requested_actions.len(), 2);
    assert!(matches!(
        intent.requested_actions[0].action,
        Action::SetLabor { enabled: true, .. }
    ));
    assert!(matches!(
        intent.requested_actions[1].action,
        Action::HarvestResource {
            resource: HarvestResource::Wood,
            amount: 1,
            ..
        }
    ));
    assert_eq!(intent.requested_actions[1].depends_on, vec![0]);
    assert_eq!(analysis["requirements"][1]["minimum"], 4);
    assert_eq!(analysis["requirements"][1]["deficit"], 0);
    let plan = prepare(&snapshot, &intent)?;
    for step in &plan.steps {
        apply(&mut snapshot, &step.action, &step.idempotency_key)?;
    }
    advance(&mut snapshot, 50)?;
    assert!(PredicateEvidence::laboratory(&snapshot)?.establishes(&intent.terminal_condition)?);
    set(
        &mut snapshot,
        LEDGER,
        effects::STOCK_PLANT_FIELD,
        WorldValue::U64(3),
    )?;
    assert_eq!(
        PredicateEvidence::laboratory(&snapshot)?.evaluate(&intent.terminal_condition)?,
        PredicateTruth::False
    );
    Ok(())
}

#[test]
fn cancelled_partial_harvest_continues_only_the_observed_deficit_and_preserves_old_proof()
-> Result<()> {
    let mut snapshot = world();
    let original = ResourceAcquisitionRequest::parse_current(
        r#"{"template":"resource_acquisition","quotas":[{"resource":"WOOD","minimum":6}]}"#,
    )?;
    let (intent, _) = original.compile(IntentId::new(94), &snapshot, "original six logs")?;
    let parent = prepare(&snapshot, &intent)?;
    let step = &parent.steps[0];
    apply(&mut snapshot, &step.action, &step.idempotency_key)?;
    advance(&mut snapshot, 50)?;
    effects::cancel_effect(&mut snapshot, &step.action, &step.idempotency_key)?;
    snapshot.refresh_hash();
    let cancelled = snapshot
        .graph
        .entities
        .get(&effects::created_entity_id(&step.idempotency_key, 0))
        .ok_or_else(|| invalid("missing original harvest"))?
        .clone();
    let continuation =
        ResourceAcquisitionContinuation::new(parent.digest, parent.digest, original)?;
    let (child, analysis) =
        continuation.compile(IntentId::new(95), &snapshot, "finish the same stock goal")?;
    assert_eq!(analysis["requirements"][0]["observed"], 4);
    assert_eq!(analysis["requirements"][0]["deficit"], 2);
    assert_eq!(analysis["allocations"][0]["amount"], 2);
    let child = prepare(&snapshot, &child)?;
    for next in &child.steps {
        apply(&mut snapshot, &next.action, &next.idempotency_key)?;
    }
    advance(&mut snapshot, 100)?;
    assert_eq!(
        continuation.stock_goal_truth(&snapshot)?,
        PredicateTruth::True
    );
    assert!(
        PredicateEvidence::laboratory(&snapshot)?
            .establishes(&Predicate::All(child.steps[0].postconditions.clone()))?
    );
    assert_eq!(
        PredicateEvidence::laboratory(&snapshot)?
            .evaluate(&Predicate::All(step.postconditions.clone()))?,
        PredicateTruth::False
    );
    assert_eq!(snapshot.graph.entities.get(&cancelled.id), Some(&cancelled));
    let report =
        effects::harvest_report(&snapshot)?.ok_or_else(|| invalid("missing finite report"))?;
    let source = report
        .sources
        .iter()
        .find(|source| source.source == WOOD)
        .ok_or_else(|| invalid("missing original wood source"))?;
    assert_eq!((source.remaining, source.harvested), (37, 3));
    assert!(
        matches!(continuation.compile(IntentId::new(96), &snapshot, ""), Err(error) if error.code == ErrorCode::InvalidIntent)
    );
    Ok(())
}

#[test]
fn active_original_work_is_not_future_stock_and_unavailable_custody_is_not_truth() -> Result<()> {
    let mut snapshot = world();
    let source = source()?;
    let (original, _) = source.request.compile(IntentId::new(97), &snapshot, "")?;
    let plan = prepare(&snapshot, &original)?;
    apply(
        &mut snapshot,
        &plan.steps[0].action,
        &plan.steps[0].idempotency_key,
    )?;
    assert_eq!(source.stock_goal_truth(&snapshot)?, PredicateTruth::False);
    assert!(
        matches!(source.compile(IntentId::new(98), &snapshot, ""), Err(error) if error.code == ErrorCode::PreconditionsFailed && error.message.contains("harvest remains active"))
    );
    for variant in 0..3 {
        let mut unavailable = snapshot.clone();
        let record = unavailable
            .graph
            .entities
            .get_mut(&LEDGER)
            .ok_or_else(|| invalid("missing ledger fixture"))?;
        match variant {
            0 => record.generation += 1,
            1 => {
                record
                    .fields
                    .get_mut(effects::STOCK_WOOD_FIELD)
                    .ok_or_else(|| invalid("missing stock fixture"))?
                    .presence = Some(FactPresence::Unknown("unobserved".to_owned()))
            }
            _ => {
                record
                    .fields
                    .get_mut(effects::STOCK_WOOD_FIELD)
                    .ok_or_else(|| invalid("missing stock fixture"))?
                    .observed_at = GameTick(unavailable.tick.0 + 1)
            }
        }
        unavailable.refresh_hash();
        assert!(source.stock_goal_truth(&unavailable).is_err());
        assert!(source.compile(IntentId::new(99), &unavailable, "").is_err());
    }
    Ok(())
}

#[test]
fn fulfilled_semantic_stock_needs_no_worker_or_replacement_effect() -> Result<()> {
    let mut snapshot = world();
    set(
        &mut snapshot,
        LEDGER,
        effects::STOCK_WOOD_FIELD,
        WorldValue::U64(5),
    )?;
    set(
        &mut snapshot,
        LEDGER,
        effects::STOCK_PLANT_FIELD,
        WorldValue::U64(4),
    )?;
    snapshot.graph.entities.remove(&WORKER);
    snapshot.refresh_hash();
    let before = snapshot.clone();
    let source = source()?;
    assert_eq!(source.stock_goal_truth(&snapshot)?, PredicateTruth::True);
    assert!(
        matches!(source.compile(IntentId::new(100), &snapshot, ""), Err(error) if error.code == ErrorCode::InvalidIntent && error.message.contains("already satisfied"))
    );
    assert_eq!(snapshot, before);
    Ok(())
}

#[test]
fn continuation_summary_obeys_exact_single_seal_byte_and_control_bounds() -> Result<()> {
    let source = source()?;
    let suffix = format!(
        " [resource acquisition continuation {}]",
        source.source_digest().to_hex()
    );
    let remaining = StaticPlanner::default().policy.max_string_bytes - suffix.len();
    let (intent, _) = source.compile(IntentId::new(101), &world(), &"x".repeat(remaining))?;
    assert_eq!(
        intent.summary.len(),
        StaticPlanner::default().policy.max_string_bytes
    );
    for summary in [
        "x".repeat(remaining + 1),
        "unsafe\nsummary".to_owned(),
        "unsafe\0summary".to_owned(),
    ] {
        assert!(
            source
                .compile(IntentId::new(101), &world(), &summary)
                .is_err()
        );
    }
    Ok(())
}
