//! A canceled future reserve remains the same objective while today's stock
//! still satisfies its minima. /6 is a new pursuit, not an edit to /3 history.

use super::*;
use crate::lab_world::{scenario_snapshot, starter};
use dfmcp_core::{CapabilityGrant, CapabilityScope, FortressId, RequestId, SessionId, WorkBudget};
use dfmcp_intent::{Action, effects};
use dfmcp_lab::MemoryAdapter;
use dfmcp_world::{EntityKind, FactPresence, PredicateEvidence, Value as WorldValue};

fn world() -> Result<WorldSnapshot> {
    scenario_snapshot(
        "starter_fortress_provisions",
        FortressId::new(78_611),
        false,
    )
}

fn context(snapshot: &WorldSnapshot) -> OperationContext {
    OperationContext {
        session_id: SessionId::new(78_611),
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

fn request(until: u64) -> Result<ProductionRequest> {
    ProductionRequest::parse_current(
        &json!({
            "template":"production", "reserve_until_tick":until,
            "quotas":[{"item":"DRINK","minimum":40},{"item":"FOOD","minimum":60}],
        })
        .to_string(),
    )
}

fn source(until: u64) -> Result<ProductionContinuation> {
    let root = Digest32::of_bytes(b"ordinary reserve with currently satisfied stock");
    ProductionContinuation::new(root, root, request(until)?)
}

fn seal(
    snapshot: &WorldSnapshot,
    source: &ProductionContinuation,
    id: u128,
) -> Result<PreparedPlan> {
    let (intent, _) = source.compile(
        IntentId::new(id),
        snapshot,
        "preserve original future reserve",
    )?;
    StaticPlanner::default().prepare_laboratory(snapshot, &intent, &context(snapshot))
}

fn publish(snapshot: &mut WorldSnapshot) -> Result<()> {
    snapshot.cursor = snapshot
        .cursor
        .checked_next()
        .ok_or_else(|| invalid("reserve fixture cursor exhausted"))?;
    snapshot.refresh_hash();
    Ok(())
}

fn apply(snapshot: &mut WorldSnapshot, plan: &PreparedPlan) -> Result<()> {
    for step in &plan.steps {
        assert!(matches!(step.action, Action::CreateWorkOrder { .. }));
        effects::apply_effect(snapshot, &step.action, &step.idempotency_key)?;
        publish(snapshot)?;
    }
    Ok(())
}

fn advance(snapshot: &mut WorldSnapshot, ticks: u64) -> Result<()> {
    snapshot.tick = snapshot
        .tick
        .checked_add(ticks)
        .ok_or_else(|| invalid("tick overflow"))?;
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
        _ => Err(invalid("reserve stock fixture missing")),
    }
}

#[test]
fn current_reserve_source_roundtrips_and_explicitly_upgrades_only_new_pursuits() -> Result<()> {
    let snapshot = world()?;
    let current = source(2401)?;
    assert_eq!(current.version, SourceVersion::ConsumptionReserveCurrentV1);
    assert!(!current.is_resource_aware());
    assert!(current.is_current_consumption_reserve());
    assert!(current.admits_remaining_reserve());
    let bytes = current.canonical_json();
    let mut digest_bytes = b"dfmcp-production-continuation-source/6\0".to_vec();
    digest_bytes.extend_from_slice(bytes.as_bytes());
    assert_eq!(current.source_digest(), Digest32::of_bytes(&digest_bytes));
    let reopened = ProductionContinuation::parse(&bytes)?;
    assert_eq!(current, reopened);
    assert_eq!(
        seal(&snapshot, &current, 1)?,
        seal(&snapshot, &reopened, 1)?
    );

    let legacy = ProductionContinuation::from_parts(
        current.parent,
        current.root,
        current.request.clone(),
        SourceVersion::ConsumptionReserveV1,
    )?;
    let historical_bytes = legacy.canonical_json();
    let historical_seal = seal(&snapshot, &legacy, 1)?;
    let parsed = ProductionContinuation::parse(&historical_bytes)?;
    assert_eq!(parsed.version, SourceVersion::ConsumptionReserveV1);
    assert!(!parsed.admits_remaining_reserve());
    assert_eq!(parsed.canonical_json(), historical_bytes);
    assert_eq!(seal(&snapshot, &parsed, 1)?, historical_seal);
    assert_ne!(historical_seal.digest, seal(&snapshot, &current, 1)?.digest);
    assert_eq!(historical_seal.steps, seal(&snapshot, &current, 1)?.steps);

    let next = parsed.next(Digest32::of_bytes(b"new reserve review"))?;
    assert!(next.is_current_consumption_reserve());
    assert_eq!(next.request, parsed.request);
    assert_eq!(next.root, parsed.root);
    assert_eq!(next.reserve_until_tick(), Some(2401));
    assert_eq!(parsed.canonical_json(), historical_bytes);
    assert_eq!(
        next.lineage_json()["source_schema"],
        CURRENT_RESERVE_SOURCE_SCHEMA
    );
    assert_eq!(
        next.lineage_json()["current_minima_do_not_establish_future_reserve"],
        true
    );
    assert_eq!(next.lineage_json()["requires_original_stock_ledger"], true);
    Ok(())
}

#[test]
fn canceled_current_true_reserve_uses_observed_residuals_and_exact_absolute_horizon() -> Result<()>
{
    let initial = source(2401)?;
    let mut snapshot = world()?;
    let original = seal(&snapshot, &initial, 2)?;
    let ledger = ProductionGoalLedger::capture(&snapshot)?;
    assert_eq!(
        PredicateEvidence::laboratory(&snapshot)?.evaluate(&original.terminal_condition)?,
        PredicateTruth::True
    );
    apply(&mut snapshot, &original)?;
    // Two batches per order have actually finished; cancellation preserves
    // that stock and its finite PLANT debit without claiming future output.
    advance(&mut snapshot, 100)?;
    let drink = stock(&snapshot, effects::STOCK_DRINK_FIELD)?;
    let food = stock(&snapshot, effects::STOCK_FOOD_FIELD)?;
    for step in &original.steps {
        effects::cancel_effect(&mut snapshot, &step.action, &step.idempotency_key)?;
        publish(&mut snapshot)?;
    }
    let immutable = original.clone();
    let canceled_history: Vec<_> = original
        .steps
        .iter()
        .map(|step| {
            snapshot
                .graph
                .entities
                .get(&effects::created_entity_id(&step.idempotency_key, 0))
                .cloned()
        })
        .collect();
    assert_eq!(
        PredicateEvidence::laboratory(&snapshot)?.evaluate(&original.terminal_condition)?,
        PredicateTruth::True
    );
    let continuation =
        ProductionContinuation::new(original.digest, original.digest, initial.request)?;
    assert!(continuation.reserve_work_required(&snapshot)?);
    continuation.validate_resource_goal_evidence(
        &original,
        ledger,
        original.anchor,
        &snapshot,
        &context(&snapshot),
        true,
    )?;
    let (intent, analysis) =
        continuation.compile(IntentId::new(3), &snapshot, "finish original reserve")?;
    assert_eq!(analysis["consumption"]["reserve_until_tick"], 2401);
    assert_eq!(analysis["consumption"]["reserve_horizon_ticks"], 2300);
    let next =
        StaticPlanner::default().prepare_laboratory(&snapshot, &intent, &context(&snapshot))?;
    assert_eq!(next.terminal_condition, original.terminal_condition);
    assert!(
        next.steps
            .iter()
            .filter_map(|step| step.obligation.as_ref())
            .all(|obligation| obligation.deadline_tick.0 < 2401)
    );
    assert!(next.steps.iter().all(|step| {
        !original
            .steps
            .iter()
            .any(|old| old.idempotency_key == step.idempotency_key)
    }));
    assert_eq!(stock(&snapshot, effects::STOCK_DRINK_FIELD)?, drink);
    assert_eq!(stock(&snapshot, effects::STOCK_FOOD_FIELD)?, food);
    apply(&mut snapshot, &next)?;
    advance(&mut snapshot, 1000)?;
    assert!(!continuation.reserve_work_required(&snapshot)?);
    assert_eq!(original, immutable);
    for (step, history) in original.steps.iter().zip(canceled_history) {
        assert_eq!(
            snapshot
                .graph
                .entities
                .get(&effects::created_entity_id(&step.idempotency_key, 0)),
            history.as_ref()
        );
    }
    let remaining = 2401 - snapshot.tick.0;
    advance(&mut snapshot, remaining)?;
    assert!(stock(&snapshot, effects::STOCK_DRINK_FIELD)? >= 40);
    assert!(stock(&snapshot, effects::STOCK_FOOD_FIELD)? >= 60);
    Ok(())
}

#[test]
fn current_reserve_refuses_funded_expired_unknown_and_unquiet_work() -> Result<()> {
    let initial = source(2401)?;
    let snapshot = world()?;
    let original = seal(&snapshot, &initial, 4)?;
    let ledger = ProductionGoalLedger::capture(&snapshot)?;
    let refuse = |candidate: &WorldSnapshot, quiet| {
        initial.validate_resource_goal_evidence(
            &original,
            ledger,
            original.anchor,
            candidate,
            &context(candidate),
            quiet,
        )
    };
    assert_eq!(
        refuse(&snapshot, false).err().map(|error| error.code),
        Some(ErrorCode::PreconditionsFailed)
    );

    let mut funded = snapshot.clone();
    for field in [effects::STOCK_DRINK_FIELD, effects::STOCK_FOOD_FIELD] {
        let fact = funded
            .graph
            .entities
            .get_mut(&starter::STOCK_LEDGER)
            .and_then(|record| record.fields.get_mut(field))
            .ok_or_else(|| invalid("fixture field missing"))?;
        fact.value = WorldValue::U64(1000);
        fact.presence = None;
    }
    publish(&mut funded)?;
    assert!(!initial.reserve_work_required(&funded)?);
    assert_eq!(
        refuse(&funded, true).err().map(|error| error.code),
        Some(ErrorCode::InvalidIntent)
    );

    let mut expired = snapshot.clone();
    advance(&mut expired, 2400)?;
    assert!(refuse(&expired, true).is_err());
    assert!(
        initial
            .compile(IntentId::new(5), &expired, "do not renew")
            .is_err()
    );

    for field in [effects::STOCK_DRINK_FIELD, effects::METABOLISM_TICKS_FIELD] {
        let mut unknown = snapshot.clone();
        unknown
            .graph
            .entities
            .get_mut(&starter::STOCK_LEDGER)
            .and_then(|record| record.fields.get_mut(field))
            .ok_or_else(|| invalid("fixture field missing"))?
            .presence = Some(FactPresence::Unknown("not observed".to_owned()));
        publish(&mut unknown)?;
        assert_eq!(
            refuse(&unknown, true).err().map(|error| error.code),
            Some(ErrorCode::PreconditionsFailed)
        );
    }
    Ok(())
}

#[test]
fn current_reserve_requires_current_read_authority_original_ledger_and_canonical_population()
-> Result<()> {
    let initial = source(2401)?;
    let snapshot = world()?;
    let original = seal(&snapshot, &initial, 6)?;
    let ledger = ProductionGoalLedger::capture(&snapshot)?;
    for capability in [Capability::Observe, Capability::Query, Capability::Plan] {
        let mut revoked = context(&snapshot);
        revoked
            .grants
            .retain(|grant| grant.capability != capability);
        assert_eq!(
            initial
                .validate_resource_goal_evidence(
                    &original,
                    ledger,
                    original.anchor,
                    &snapshot,
                    &revoked,
                    true
                )
                .err()
                .map(|error| error.code),
            Some(ErrorCode::CapabilityDenied)
        );
    }
    let mut replaced = snapshot.clone();
    replaced
        .graph
        .entities
        .get_mut(&starter::STOCK_LEDGER)
        .ok_or_else(|| invalid("ledger missing"))?
        .generation += 1;
    publish(&mut replaced)?;
    assert_eq!(
        initial
            .validate_resource_goal_evidence(
                &original,
                ledger,
                original.anchor,
                &replaced,
                &context(&replaced),
                true
            )
            .err()
            .map(|error| error.code),
        Some(ErrorCode::PreconditionsFailed)
    );

    let mut aliases = snapshot.clone();
    for record in aliases
        .graph
        .entities
        .values_mut()
        .filter(|record| record.kind == EntityKind::Unit)
    {
        record.kind = EntityKind::Other("unit".to_owned());
    }
    aliases.refresh_hash();
    assert_eq!(
        aliases.anchor(),
        snapshot.anchor(),
        "the legacy canonical alias shares bytes"
    );
    assert_eq!(
        initial
            .reserve_work_required(&aliases)
            .err()
            .map(|error| error.code),
        Some(ErrorCode::PreconditionsFailed)
    );
    Ok(())
}

#[test]
fn exact_recovery_preserves_original_reserve_ledger_and_rejects_invented_epochs() -> Result<()> {
    let initial = source(2401)?;
    let mut snapshot = world()?;
    let original = seal(&snapshot, &initial, 7)?;
    let ledger = ProductionGoalLedger::capture(&snapshot)?;
    apply(&mut snapshot, &original)?;
    for step in &original.steps {
        effects::cancel_effect(&mut snapshot, &step.action, &step.idempotency_key)?;
        publish(&mut snapshot)?;
    }
    let recovered = MemoryAdapter::recovered(snapshot.clone())?;
    let binding =
        RecoveredGoalLineage::verify(&snapshot, snapshot.anchor(), recovered.snapshot().anchor())?;
    initial.validate_resource_goal_evidence(
        &original,
        ledger,
        binding.active_anchor(),
        recovered.snapshot(),
        &context(recovered.snapshot()),
        true,
    )?;
    let twice = MemoryAdapter::recovered(recovered.snapshot().clone())?;
    let again = RecoveredGoalLineage::verify(
        recovered.snapshot(),
        recovered.snapshot().anchor(),
        twice.snapshot().anchor(),
    )?;
    initial.validate_resource_goal_evidence(
        &original,
        ledger,
        again.active_anchor(),
        twice.snapshot(),
        &context(twice.snapshot()),
        true,
    )?;
    assert_eq!(
        initial
            .validate_resource_goal_evidence(
                &original,
                ledger,
                original.anchor,
                twice.snapshot(),
                &context(twice.snapshot()),
                true
            )
            .err()
            .map(|error| error.code),
        Some(ErrorCode::StaleAnchor)
    );
    let (_, analysis) =
        initial.compile(IntentId::new(8), twice.snapshot(), "same original reserve")?;
    assert_eq!(analysis["consumption"]["reserve_until_tick"], 2401);
    assert_eq!(
        initial.request.canonical_json(),
        request(2401)?.canonical_json()
    );
    Ok(())
}

#[test]
fn current_reserve_source_rejects_changed_generation_and_duplicate_original_fields() -> Result<()> {
    let source = source(2401)?;
    let raw = source.canonical_json();
    let base: Value = serde_json::from_str(&raw).map_err(|error| invalid(error.to_string()))?;
    for schema in [
        LEGACY_SOURCE_SCHEMA,
        SOURCE_SCHEMA,
        RESOURCE_SOURCE_SCHEMA,
        RESOURCE_RESERVE_SOURCE_SCHEMA,
    ] {
        let mut changed = base.clone();
        changed["schema"] = json!(schema);
        assert!(ProductionContinuation::parse(&changed.to_string()).is_err());
    }
    for (field, value) in [
        ("reserve_until_tick", Value::Null),
        ("planner", json!("consumption_aware_v1")),
        ("acquire_resources", json!({"assign_labor":false})),
    ] {
        let mut changed = base.clone();
        changed["production"][field] = value;
        assert!(ProductionContinuation::parse(&changed.to_string()).is_err());
    }
    for (needle, replacement) in [
        (
            "\"reserve_until_tick\":2401",
            "\"reserve_until_tick\":2402,\"reserve_until_tick\":2401",
        ),
        ("\"minimum\":40", "\"minimum\":1,\"minimum\":40"),
    ] {
        let changed = raw.replacen(needle, replacement, 1);
        assert_ne!(changed, raw);
        assert!(ProductionContinuation::parse(&changed).is_err());
    }
    let changed = source.next(Digest32::of_bytes(b"different parent"))?;
    assert!(!source.same_original_request(&changed));
    let another_tick = source.request.clone();
    let mut changed: Value = serde_json::from_str(&another_tick.canonical_json())
        .map_err(|error| invalid(error.to_string()))?;
    changed["reserve_until_tick"] = json!(2402);
    let changed = ProductionRequest::parse(&changed.to_string())?;
    let changed = ProductionContinuation::new(source.parent, source.root, changed)?;
    assert_ne!(source.source_digest(), changed.source_digest());
    assert!(!source.same_original_request(&changed));
    Ok(())
}
