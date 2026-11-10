//! Original production goals survive resource stages, cancellation and recovery.

use super::*;
use crate::lab_world::{scenario_snapshot, starter};
use dfmcp_core::{
    CapabilityGrant, CapabilityScope, FortressId, GameTick, RequestId, SessionId, WorkBudget,
};
use dfmcp_intent::{Action, HarvestResource, effects};
use dfmcp_lab::MemoryAdapter;
use dfmcp_world::{FactPresence, Predicate, PredicateEvidence, Value as WorldValue};

fn world() -> Result<WorldSnapshot> {
    scenario_snapshot("starter_fortress_resources", FortressId::new(8_286), false)
}

fn context(snapshot: &WorldSnapshot) -> OperationContext {
    OperationContext {
        session_id: SessionId::new(8_286),
        request_id: RequestId::new(1),
        anchor: snapshot.anchor(),
        budget: WorkBudget::default(),
        cancellation_requested: false,
        grants: [Capability::Observe, Capability::Query, Capability::Plan]
            .into_iter()
            .map(|capability| CapabilityGrant {
                capability,
                scope: CapabilityScope {
                    fortress_id: Some(snapshot.fortress_id),
                    ..CapabilityScope::default()
                },
                max_risk: RiskTier::ReadOnly,
                expires_at_tick: None,
                remaining_uses: None,
            })
            .collect(),
    }
}

fn request(assign_labor: bool) -> Result<ProductionRequest> {
    ProductionRequest::parse_current(
        &json!({
            "template": "production",
            "quotas": [{"item": "BED", "minimum": 24}],
            "acquire_resources": {"assign_labor": assign_labor},
            "prerequisites": {
                "assign_labor": false,
                "workshops": [{"building": "workshop:Carpenters", "location": [0, 0, 10]}],
            },
        })
        .to_string(),
    )
}

fn source(assign_labor: bool) -> Result<ProductionContinuation> {
    let root = Digest32::of_bytes(b"original supplied production objective");
    ProductionContinuation::new(root, root, request(assign_labor)?)
}

fn prepare(snapshot: &WorldSnapshot, intent: &Intent) -> Result<PreparedPlan> {
    StaticPlanner::default().prepare_laboratory(snapshot, intent, &context(snapshot))
}

fn publish(snapshot: &mut WorldSnapshot) -> Result<()> {
    snapshot.cursor = snapshot
        .cursor
        .checked_next()
        .ok_or_else(|| invalid("fixture cursor overflow"))?;
    snapshot.refresh_hash();
    Ok(())
}

fn apply(snapshot: &mut WorldSnapshot, plan: &PreparedPlan) -> Result<()> {
    for step in &plan.steps {
        effects::apply_effect(snapshot, &step.action, &step.idempotency_key)?;
        publish(snapshot)?;
    }
    Ok(())
}

fn advance(snapshot: &mut WorldSnapshot, ticks: u64) -> Result<()> {
    snapshot.tick = snapshot
        .tick
        .checked_add(ticks)
        .ok_or_else(|| invalid("fixture tick overflow"))?;
    effects::advance_effects(snapshot, ticks)?;
    publish(snapshot)
}

fn stock(snapshot: &WorldSnapshot, field: &str) -> Result<u64> {
    match snapshot
        .graph
        .entities
        .get(&starter::STOCK_LEDGER)
        .and_then(|record| record.fields.get(field))
        .and_then(|fact| fact.known_value())
    {
        Some(WorldValue::U64(value)) => Ok(*value),
        _ => Err(invalid("fixture stock is not known")),
    }
}

#[test]
fn resource_continuation_keeps_the_complete_fixed_request_and_flat_lineage() -> Result<()> {
    let original = source(true)?;
    let raw = original.canonical_json();
    let value: Value = serde_json::from_str(&raw).map_err(|error| invalid(error.to_string()))?;
    assert_eq!(value["schema"], RESOURCE_SOURCE_SCHEMA);
    assert_eq!(value["production"]["planner"], "resource_aware_v1");
    assert_eq!(
        value["production"]["acquire_resources"]["assign_labor"],
        true
    );
    assert_eq!(value["production"]["prerequisites"]["assign_labor"], false);
    assert_eq!(value["production"]["quotas"][0]["minimum"], 24);
    assert_eq!(ProductionContinuation::parse(&raw)?, original);
    assert_eq!(original.version, SourceVersion::ResourceAwareV1);
    assert!(original.is_resource_aware());
    assert_eq!(original.reserve_until_tick(), None);
    let mut digest_bytes = b"dfmcp-production-continuation-source/4\0".to_vec();
    digest_bytes.extend_from_slice(raw.as_bytes());
    assert_eq!(original.source_digest(), Digest32::of_bytes(&digest_bytes));
    let mut next = original.clone();
    for ordinal in 0u64..16 {
        next = next.next(Digest32::of_bytes(&ordinal.to_be_bytes()))?;
        next = ProductionContinuation::parse(&next.canonical_json())?;
        assert_eq!(next.request, original.request);
        assert_eq!(next.root, original.root);
        assert_eq!(next.canonical_json().len(), raw.len());
        assert_eq!(next.version, SourceVersion::ResourceAwareV1);
    }
    assert_eq!(next.lineage_json()["source_schema"], RESOURCE_SOURCE_SCHEMA);
    assert_eq!(
        next.lineage_json()["original_resource_policy_preserved"],
        true
    );
    assert_eq!(next.lineage_json()["next_stage_dispatched"], false);
    Ok(())
}

#[test]
fn archived_generations_cannot_import_or_erase_resource_permission() -> Result<()> {
    let original = source(false)?;
    let baseline: Value = serde_json::from_str(&original.canonical_json())
        .map_err(|error| invalid(error.to_string()))?;
    for schema in [LEGACY_SOURCE_SCHEMA, SOURCE_SCHEMA, RESERVE_SOURCE_SCHEMA] {
        let mut changed = baseline.clone();
        changed["schema"] = json!(schema);
        assert!(ProductionContinuation::parse(&changed.to_string()).is_err());
    }
    for (field, value) in [
        ("planner", json!("consumption_aware_v1")),
        ("planner", json!("resource_aware_v2")),
        ("acquire_resources", Value::Null),
        ("acquire_resources", json!({"assign_labor":null})),
        (
            "acquire_resources",
            json!({"assign_labor":false,"assume_supply":true}),
        ),
        ("reserve_until_tick", json!(4801)),
    ] {
        let mut changed = baseline.clone();
        changed["production"][field] = value;
        assert!(
            ProductionContinuation::parse(&changed.to_string()).is_err(),
            "{changed}"
        );
    }
    for field in ["planner", "acquire_resources"] {
        let mut changed = baseline.clone();
        changed["production"]
            .as_object_mut()
            .ok_or_else(|| invalid("fixture production missing"))?
            .remove(field);
        assert!(ProductionContinuation::parse(&changed.to_string()).is_err());
    }
    let ordinary = ProductionRequest::parse_current(
        r#"{"template":"production","quotas":[{"item":"BED","minimum":24}]}"#,
    )?;
    assert!(
        ProductionContinuation::from_parts(
            original.parent,
            original.root,
            ordinary,
            SourceVersion::ResourceAwareV1,
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn resource_archive_rejects_duplicate_nested_permissions_and_quota_keys() -> Result<()> {
    let raw = source(false)?.canonical_json();
    for (needle, replacement) in [
        (
            "\"assign_labor\":false",
            "\"assign_labor\":true,\"assign_labor\":false",
        ),
        ("\"minimum\":24", "\"minimum\":1,\"minimum\":24"),
        ("\"item\":\"BED\"", "\"item\":\"DOOR\",\"item\":\"BED\""),
        (
            "\"planner\":\"resource_aware_v1\"",
            "\"planner\":\"consumption_aware_v1\",\"planner\":\"resource_aware_v1\"",
        ),
        (
            "\"acquire_resources\":{",
            "\"acquire_resources\":{\"assign_labor\":true},\"acquire_resources\":{",
        ),
    ] {
        let changed = raw.replacen(needle, replacement, 1);
        assert_ne!(changed, raw);
        assert!(
            ProductionContinuation::parse(&changed).is_err(),
            "{changed}"
        );
    }
    Ok(())
}

#[test]
fn resource_archive_replays_exact_plan_and_seals_unused_policy_and_lineage() -> Result<()> {
    let snapshot = world()?;
    let before = snapshot.clone();
    let original = source(false)?;
    let (intent, analysis) = original.compile(IntentId::new(85), &snapshot, "supply beds")?;
    let replay = ProductionContinuation::parse(&original.canonical_json())?.compile(
        IntentId::new(85),
        &snapshot,
        "supply beds",
    )?;
    assert_eq!((intent.clone(), analysis.clone()), replay);
    assert_eq!(prepare(&snapshot, &intent)?, prepare(&snapshot, &replay.0)?);
    assert_eq!(analysis["continuation"], original.lineage_json());
    for changed in [
        source(true)?,
        original.next(Digest32::of_bytes(b"another parent"))?,
    ] {
        let (other, _) = changed.compile(IntentId::new(85), &snapshot, "supply beds")?;
        assert_eq!(intent.requested_actions, other.requested_actions);
        assert_eq!(intent.terminal_condition, other.terminal_condition);
        assert_ne!(
            prepare(&snapshot, &intent)?.digest,
            prepare(&snapshot, &other)?.digest
        );
        assert!(!original.same_original_request(&changed));
    }
    assert_eq!(
        snapshot, before,
        "compiling a continuation cannot acquire inputs"
    );
    Ok(())
}

#[test]
fn canceled_acquisition_resumes_original_outputs_without_rewriting_old_harvest_proof() -> Result<()>
{
    let mut snapshot = world()?;
    let initial = source(false)?;
    let (intent, _) = initial.compile(IntentId::new(86), &snapshot, "supply beds")?;
    let original = prepare(&snapshot, &intent)?;
    let original_ledger = ProductionGoalLedger::capture(&snapshot)?;
    let terminal = original.terminal_condition.clone();
    assert!(original.steps.iter().all(|step| matches!(
        step.action,
        Action::HarvestResource {
            resource: HarvestResource::Wood,
            ..
        }
    )));
    apply(&mut snapshot, &original)?;
    advance(&mut snapshot, effects::HARVEST_TICKS_PER_UNIT)?;
    let step = original
        .steps
        .first()
        .ok_or_else(|| invalid("missing harvest step"))?;
    effects::cancel_effect(&mut snapshot, &step.action, &step.idempotency_key)?;
    publish(&mut snapshot)?;
    let original_task = effects::created_entity_id(&step.idempotency_key, 0);
    let canceled = snapshot
        .graph
        .entities
        .get(&original_task)
        .ok_or_else(|| invalid("missing original harvest task"))?
        .clone();
    assert_eq!(stock(&snapshot, effects::STOCK_WOOD_FIELD)?, 21);
    assert!(
        dfmcp_intent::inspect_effect_work(&snapshot, &step.action, &step.idempotency_key, true,)?
            .is_quiescent()
    );
    let continuation =
        ProductionContinuation::new(original.digest, original.digest, initial.request.clone())?;
    continuation.validate_resource_goal_evidence(
        &original,
        original_ledger,
        original.anchor,
        &snapshot,
        &context(&snapshot),
        true,
    )?;
    let (next, _) = continuation.compile(IntentId::new(87), &snapshot, "finish acquisition")?;
    let next = prepare(&snapshot, &next)?;
    assert_eq!(next.terminal_condition, terminal);
    assert!(matches!(
        next.steps[0].action,
        Action::HarvestResource {
            resource: HarvestResource::Wood,
            amount: 3,
            ..
        }
    ));
    apply(&mut snapshot, &next)?;
    advance(&mut snapshot, effects::HARVEST_TICKS_PER_UNIT * 3)?;
    assert_eq!(stock(&snapshot, effects::STOCK_WOOD_FIELD)?, 24);
    assert_eq!(
        continuation.semantic_goal_truth_in_ledger(&snapshot, original_ledger)?,
        PredicateTruth::False
    );
    let production = continuation.next(next.digest)?;
    let (produce, _) = production.compile(IntentId::new(88), &snapshot, "produce original beds")?;
    let produce = prepare(&snapshot, &produce)?;
    assert_eq!(produce.terminal_condition, terminal);
    assert!(
        produce
            .steps
            .iter()
            .all(|step| matches!(step.action, Action::CreateWorkOrder { .. }))
    );
    apply(&mut snapshot, &produce)?;
    advance(&mut snapshot, effects::WORK_ORDER_TICKS_PER_UNIT * 24)?;
    assert_eq!(stock(&snapshot, effects::STOCK_BED_FIELD)?, 24);
    assert_eq!(stock(&snapshot, effects::STOCK_WOOD_FIELD)?, 0);
    assert_eq!(
        production.semantic_goal_truth_in_ledger(&snapshot, original_ledger)?,
        PredicateTruth::True
    );
    assert_eq!(
        crate::lab_world::plan_goal_truth(&snapshot, &produce)?,
        PredicateTruth::True
    );
    assert_eq!(
        crate::lab_world::plan_goal_truth(&snapshot, &original)?,
        PredicateTruth::False
    );
    assert_eq!(
        PredicateEvidence::laboratory(&snapshot)?
            .evaluate(&Predicate::All(step.postconditions.clone()))?,
        PredicateTruth::False
    );
    assert_eq!(snapshot.graph.entities.get(&original_task), Some(&canceled));
    assert_eq!(
        production
            .validate_resource_goal_evidence(
                &original,
                original_ledger,
                original.anchor,
                &snapshot,
                &context(&snapshot),
                true,
            )
            .err()
            .map(|error| error.code),
        Some(ErrorCode::InvalidIntent)
    );
    assert!(
        production
            .compile(IntentId::new(89), &snapshot, "do not duplicate")
            .is_err()
    );
    Ok(())
}

#[test]
fn resource_continuation_requires_current_authority_lineage_truth_and_quiet_work() -> Result<()> {
    let snapshot = world()?;
    let source = source(false)?;
    let (intent, _) = source.compile(IntentId::new(90), &snapshot, "supply beds")?;
    let original = prepare(&snapshot, &intent)?;
    let original_ledger = ProductionGoalLedger::capture(&snapshot)?;
    let validate = |at: &WorldSnapshot, ctx: &OperationContext, quiet| {
        source.validate_resource_goal_evidence(
            &original,
            original_ledger,
            original.anchor,
            at,
            ctx,
            quiet,
        )
    };
    validate(&snapshot, &context(&snapshot), true)?;
    assert_eq!(
        validate(&snapshot, &context(&snapshot), false)
            .err()
            .map(|error| error.code),
        Some(ErrorCode::PreconditionsFailed)
    );
    for capability in [Capability::Observe, Capability::Query, Capability::Plan] {
        let mut denied = context(&snapshot);
        for grant in &mut denied.grants {
            if grant.capability == capability {
                grant.expires_at_tick = Some(GameTick(0));
            }
        }
        assert_eq!(
            validate(&snapshot, &denied, true)
                .err()
                .map(|error| error.code),
            Some(ErrorCode::CapabilityDenied)
        );
    }
    let mut unknown = snapshot.clone();
    unknown
        .graph
        .entities
        .get_mut(&starter::STOCK_LEDGER)
        .and_then(|ledger| ledger.fields.get_mut(effects::STOCK_BED_FIELD))
        .ok_or_else(|| invalid("fixture bed stock missing"))?
        .presence = Some(FactPresence::Unknown("unobserved output".to_owned()));
    publish(&mut unknown)?;
    assert!(validate(&unknown, &context(&unknown), true).is_err());
    let mut restored = snapshot.clone();
    restored.cursor = restored
        .cursor
        .checked_reset_epoch()
        .ok_or_else(|| invalid("fixture epoch overflow"))?;
    restored.refresh_hash();
    assert_eq!(
        validate(&restored, &context(&restored), true)
            .err()
            .map(|error| error.code),
        Some(ErrorCode::StaleAnchor)
    );
    let mut forked = snapshot.clone();
    forked.paused = !forked.paused;
    forked.refresh_hash();
    assert_eq!(
        validate(&forked, &context(&forked), true)
            .err()
            .map(|error| error.code),
        Some(ErrorCode::StaleAnchor)
    );
    Ok(())
}

#[test]
fn verified_recovery_retains_resource_policy_and_never_reuses_the_old_action_identity() -> Result<()>
{
    let mut snapshot = world()?;
    let source = source(false)?;
    let (intent, _) = source.compile(IntentId::new(91), &snapshot, "supply beds")?;
    let original = prepare(&snapshot, &intent)?;
    let original_ledger = ProductionGoalLedger::capture(&snapshot)?;
    let immutable = original.clone();
    apply(&mut snapshot, &original)?;
    let step = original
        .steps
        .first()
        .ok_or_else(|| invalid("fixture harvest missing"))?;
    effects::cancel_effect(&mut snapshot, &step.action, &step.idempotency_key)?;
    publish(&mut snapshot)?;
    let recovered = MemoryAdapter::recovered(snapshot.clone())?;
    let recovery =
        RecoveredGoalLineage::verify(&snapshot, snapshot.anchor(), recovered.snapshot().anchor())?;
    let continuation =
        ProductionContinuation::new(original.digest, original.digest, source.request)?;
    continuation.validate_resource_goal_evidence(
        &original,
        original_ledger,
        recovery.active_anchor(),
        recovered.snapshot(),
        &context(recovered.snapshot()),
        true,
    )?;
    let (new_intent, _) = ProductionContinuation::parse(&continuation.canonical_json())?.compile(
        IntentId::new(92),
        recovered.snapshot(),
        "continue after recovery",
    )?;
    let new_plan = prepare(recovered.snapshot(), &new_intent)?;
    assert_eq!(new_plan.terminal_condition, original.terminal_condition);
    assert_ne!(
        new_plan.steps[0].idempotency_key,
        original.steps[0].idempotency_key
    );
    assert_eq!(original, immutable);
    assert_eq!(continuation.version, SourceVersion::ResourceAwareV1);
    assert_eq!(continuation.lineage_json()["next_stage_dispatched"], false);
    Ok(())
}

#[test]
fn funded_parent_continuation_cannot_adopt_a_replacement_stock_ledger() -> Result<()> {
    let snapshot = world()?;
    let request = ProductionRequest::parse_current(
        r#"{"template":"production","quotas":[{"item":"BED","minimum":2}],"acquire_resources":{}}"#,
    )?;
    let root = Digest32::of_bytes(b"funded original ledger");
    let source = ProductionContinuation::new(root, root, request)?;
    let (intent, _) = source.compile(IntentId::new(93), &snapshot, "already funded beds")?;
    let original = prepare(&snapshot, &intent)?;
    assert!(
        original
            .steps
            .iter()
            .all(|step| !matches!(step.action, Action::HarvestResource { .. }))
    );
    let ledger = ProductionGoalLedger::capture(&snapshot)?;
    for new_id in [starter::STOCK_LEDGER, dfmcp_core::EntityId::new(990_002)] {
        for current_beds in [0, 2] {
            let mut changed = snapshot.clone();
            let mut record = changed
                .graph
                .entities
                .remove(&starter::STOCK_LEDGER)
                .ok_or_else(|| invalid("original fixture ledger missing"))?;
            record.id = new_id;
            record.generation += 1;
            record
                .fields
                .get_mut(effects::STOCK_BED_FIELD)
                .ok_or_else(|| invalid("fixture bed stock missing"))?
                .value = WorldValue::U64(current_beds);
            changed.graph.entities.insert(new_id, record);
            publish(&mut changed)?;
            let error = source
                .validate_resource_goal_evidence(
                    &original,
                    ledger,
                    original.anchor,
                    &changed,
                    &context(&changed),
                    true,
                )
                .err()
                .ok_or_else(|| invalid("replaced ledger admitted a continuation"))?;
            assert_eq!(error.code, ErrorCode::PreconditionsFailed);
        }
    }
    Ok(())
}

#[path = "goal_resource_reserve_continuation_tests.rs"]
mod resource_reserve;
