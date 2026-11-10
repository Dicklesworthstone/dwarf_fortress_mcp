//! Exact reserve horizons survive resource stages and retained-source replay.

use super::*;

fn reserve_source(until: u64, assign_labor: bool) -> Result<ProductionContinuation> {
    let root = Digest32::of_bytes(b"original resource reserve objective");
    let request = ProductionRequest::parse_current(
        &json!({
            "template": "production",
            "quotas": [{"item": "BED", "minimum": 24}],
            "acquire_resources": {"assign_labor": assign_labor},
            "reserve_until_tick": until,
            "prerequisites": {
                "assign_labor": false,
                "workshops": [{"building": "workshop:Carpenters", "location": [0, 0, 10]}],
            },
        })
        .to_string(),
    )?;
    ProductionContinuation::new(root, root, request)
}

#[test]
fn resource_reserve_archive_keeps_exact_generation_policy_horizon_and_flat_lineage() -> Result<()> {
    let original = reserve_source(6_001, true)?;
    let raw = original.canonical_json();
    let value: Value = serde_json::from_str(&raw).map_err(|error| invalid(error.to_string()))?;
    assert_eq!(value["schema"], RESOURCE_RESERVE_SOURCE_SCHEMA);
    assert_eq!(value["production"]["planner"], "resource_reserve_v1");
    assert_eq!(value["production"]["reserve_until_tick"], 6_001);
    assert_eq!(
        value["production"]["acquire_resources"]["assign_labor"],
        true
    );
    assert_eq!(value["production"]["prerequisites"]["assign_labor"], false);
    assert_eq!(value["production"]["quotas"][0]["minimum"], 24);
    assert_eq!(ProductionContinuation::parse(&raw)?, original);
    assert_eq!(original.version, SourceVersion::ResourceReserveV1);
    assert!(original.is_resource_aware());
    assert!(original.is_resource_reserve());
    assert_eq!(original.reserve_until_tick(), Some(6_001));
    let mut digest_bytes = b"dfmcp-production-continuation-source/5\0".to_vec();
    digest_bytes.extend_from_slice(raw.as_bytes());
    assert_eq!(original.source_digest(), Digest32::of_bytes(&digest_bytes));
    let mut next = original.clone();
    for ordinal in 0u64..16 {
        next = next.next(Digest32::of_bytes(&ordinal.to_be_bytes()))?;
        next = ProductionContinuation::parse(&next.canonical_json())?;
        assert_eq!(next.request, original.request);
        assert_eq!(next.root, original.root);
        assert_eq!(next.reserve_until_tick(), Some(6_001));
        assert_eq!(next.canonical_json().len(), raw.len());
        assert_eq!(next.version, SourceVersion::ResourceReserveV1);
    }
    assert_eq!(
        next.lineage_json()["source_schema"],
        RESOURCE_RESERVE_SOURCE_SCHEMA
    );
    assert_eq!(
        next.lineage_json()["original_absolute_horizon_preserved"],
        true
    );
    assert_eq!(
        next.lineage_json()["original_resource_policy_preserved"],
        true
    );
    assert_eq!(next.lineage_json()["next_stage_dispatched"], false);
    Ok(())
}

#[test]
fn resource_reserve_archive_cannot_cross_generations_or_erase_permissions() -> Result<()> {
    let original = reserve_source(6_001, false)?;
    let baseline: Value = serde_json::from_str(&original.canonical_json())
        .map_err(|error| invalid(error.to_string()))?;
    for schema in [
        LEGACY_SOURCE_SCHEMA,
        SOURCE_SCHEMA,
        RESERVE_SOURCE_SCHEMA,
        RESOURCE_SOURCE_SCHEMA,
    ] {
        let mut changed = baseline.clone();
        changed["schema"] = json!(schema);
        assert!(
            ProductionContinuation::parse(&changed.to_string()).is_err(),
            "{changed}"
        );
    }
    for (field, value) in [
        ("planner", json!("resource_aware_v1")),
        ("planner", json!("consumption_reserve_v1")),
        ("reserve_until_tick", Value::Null),
        ("acquire_resources", Value::Null),
        ("acquire_resources", json!({"assign_labor":null})),
        (
            "acquire_resources",
            json!({"assign_labor":false,"automatic_commit":true}),
        ),
    ] {
        let mut changed = baseline.clone();
        changed["production"][field] = value;
        assert!(
            ProductionContinuation::parse(&changed.to_string()).is_err(),
            "{changed}"
        );
    }
    for field in ["planner", "reserve_until_tick", "acquire_resources"] {
        let mut changed = baseline.clone();
        changed["production"]
            .as_object_mut()
            .ok_or_else(|| invalid("fixture production missing"))?
            .remove(field);
        assert!(ProductionContinuation::parse(&changed.to_string()).is_err());
    }
    let raw = original.canonical_json();
    for (needle, replacement) in [
        (
            "\"reserve_until_tick\":6001",
            "\"reserve_until_tick\":6002,\"reserve_until_tick\":6001",
        ),
        (
            "\"planner\":\"resource_reserve_v1\"",
            "\"planner\":\"resource_aware_v1\",\"planner\":\"resource_reserve_v1\"",
        ),
        (
            "\"assign_labor\":false",
            "\"assign_labor\":true,\"assign_labor\":false",
        ),
        ("\"minimum\":24", "\"minimum\":1,\"minimum\":24"),
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
fn resource_reserve_archive_replays_exact_plan_and_binds_unused_horizon() -> Result<()> {
    let snapshot = world()?;
    let before = snapshot.clone();
    let original = reserve_source(6_001, false)?;
    let (intent, analysis) =
        original.compile(IntentId::new(195), &snapshot, "supply reserve beds")?;
    let replay = ProductionContinuation::parse(&original.canonical_json())?.compile(
        IntentId::new(195),
        &snapshot,
        "supply reserve beds",
    )?;
    assert_eq!((intent.clone(), analysis), replay);
    assert_eq!(prepare(&snapshot, &intent)?, prepare(&snapshot, &replay.0)?);
    for changed in [
        reserve_source(6_002, false)?,
        reserve_source(6_001, true)?,
        original.next(Digest32::of_bytes(b"another reserve parent"))?,
    ] {
        let (other, _) = changed.compile(IntentId::new(195), &snapshot, "supply reserve beds")?;
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
        "planning cannot manufacture reserved ingredients"
    );
    Ok(())
}

#[test]
fn canceled_resource_reserve_recovers_residual_work_without_moving_its_horizon() -> Result<()> {
    let mut snapshot = world()?;
    let initial = reserve_source(6_001, false)?;
    let (intent, _) = initial.compile(IntentId::new(196), &snapshot, "reserve beds")?;
    let original = prepare(&snapshot, &intent)?;
    let ledger = ProductionGoalLedger::capture(&snapshot)?;
    let immutable = original.clone();
    apply(&mut snapshot, &original)?;
    advance(&mut snapshot, effects::HARVEST_TICKS_PER_UNIT)?;
    let step = original
        .steps
        .first()
        .ok_or_else(|| invalid("harvest missing"))?;
    effects::cancel_effect(&mut snapshot, &step.action, &step.idempotency_key)?;
    publish(&mut snapshot)?;
    assert_eq!(stock(&snapshot, effects::STOCK_WOOD_FIELD)?, 21);
    let task = effects::created_entity_id(&step.idempotency_key, 0);
    let canceled = snapshot.graph.entities.get(&task).cloned();
    let recovered = MemoryAdapter::recovered(snapshot.clone())?;
    let recovery =
        RecoveredGoalLineage::verify(&snapshot, snapshot.anchor(), recovered.snapshot().anchor())?;
    let continuation =
        ProductionContinuation::new(original.digest, original.digest, initial.request.clone())?;
    continuation.validate_resource_goal_evidence(
        &original,
        ledger,
        recovery.active_anchor(),
        recovered.snapshot(),
        &context(recovered.snapshot()),
        true,
    )?;
    let (next, analysis) = continuation.compile(
        IntentId::new(197),
        recovered.snapshot(),
        "finish original reserve acquisition",
    )?;
    let next = prepare(recovered.snapshot(), &next)?;
    assert_eq!(next.terminal_condition, original.terminal_condition);
    assert!(matches!(
        next.steps[0].action,
        Action::HarvestResource {
            resource: HarvestResource::Wood,
            amount: 3,
            ..
        }
    ));
    assert_eq!(analysis["consumption"]["reserve_until_tick"], 6_001);
    assert_ne!(next.steps[0].idempotency_key, step.idempotency_key);
    assert_eq!(original, immutable);
    assert_eq!(
        recovered.snapshot().graph.entities.get(&task),
        canceled.as_ref()
    );

    let mut expired = recovered.snapshot().clone();
    let remaining = 6_001 - expired.tick.0;
    advance(&mut expired, remaining)?;
    let chained = continuation.next(next.digest)?;
    assert_eq!(chained.reserve_until_tick(), Some(6_001));
    assert!(
        chained
            .compile(IntentId::new(198), &expired, "do not renew reserve")
            .is_err()
    );
    assert!(
        chained
            .validate_resource_goal_evidence(
                &original,
                ledger,
                recovery.active_anchor(),
                &expired,
                &context(&expired),
                true,
            )
            .is_err()
    );
    Ok(())
}

#[test]
fn current_true_resource_reserve_needs_qualified_demand_current_grants_and_quiet_work() -> Result<()>
{
    let mut snapshot = world()?;
    let request = ProductionRequest::parse_current(
        r#"{"template":"production","quotas":[{"item":"DRINK","minimum":40},{"item":"FOOD","minimum":60}],"acquire_resources":{"assign_labor":false},"reserve_until_tick":12001}"#,
    )?;
    let root = Digest32::of_bytes(b"future reserve while present minima hold");
    let source = ProductionContinuation::new(root, root, request)?;
    let (intent, _) = source.compile(IntentId::new(199), &snapshot, "future reserve")?;
    let original = prepare(&snapshot, &intent)?;
    let ledger = ProductionGoalLedger::capture(&snapshot)?;
    apply(&mut snapshot, &original)?;
    advance(&mut snapshot, effects::HARVEST_TICKS_PER_UNIT)?;
    assert_eq!(
        source.semantic_goal_truth_in_ledger(&snapshot, ledger)?,
        PredicateTruth::True
    );
    assert!(source.resource_reserve_work_required(&snapshot)?);
    let validate = |at: &WorldSnapshot, ctx: &OperationContext, quiet| {
        source.validate_resource_goal_evidence(&original, ledger, original.anchor, at, ctx, quiet)
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
        .and_then(|ledger| ledger.fields.get_mut(effects::STOCK_DRINK_FIELD))
        .ok_or_else(|| invalid("drink stock missing"))?
        .presence = Some(FactPresence::Unknown(
        "reserve input unavailable".to_owned(),
    ));
    publish(&mut unknown)?;
    assert!(validate(&unknown, &context(&unknown), true).is_err());
    let mut funded = snapshot.clone();
    for field in [effects::STOCK_DRINK_FIELD, effects::STOCK_FOOD_FIELD] {
        funded
            .graph
            .entities
            .get_mut(&starter::STOCK_LEDGER)
            .and_then(|ledger| ledger.fields.get_mut(field))
            .ok_or_else(|| invalid("reserve output stock missing"))?
            .value = WorldValue::U64(1_000);
    }
    publish(&mut funded)?;
    assert!(!source.resource_reserve_work_required(&funded)?);
    assert_eq!(
        validate(&funded, &context(&funded), true)
            .err()
            .map(|error| error.code),
        Some(ErrorCode::InvalidIntent)
    );
    Ok(())
}
