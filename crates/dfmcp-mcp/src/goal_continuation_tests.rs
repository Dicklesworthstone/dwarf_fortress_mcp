//! Original-quota continuation using actual production, authority and effects.

use super::*;
use crate::lab_world::{scenario_snapshot, starter};
use dfmcp_adapter::GameAdapter;
use dfmcp_core::{
    CapabilityGrant, CapabilityScope, CommitState, FortressId, GameTick, RequestId, SessionId,
    WorkBudget,
};
use dfmcp_intent::{Action, effects};
use dfmcp_lab::MemoryAdapter;
use dfmcp_world::{FactPresence, Value as WorldValue};

fn world() -> Result<WorldSnapshot> {
    scenario_snapshot("starter_fortress", FortressId::new(741), false)
}

fn context(snapshot: &WorldSnapshot) -> OperationContext {
    OperationContext {
        session_id: SessionId::new(1),
        request_id: RequestId::new(1),
        anchor: snapshot.anchor(),
        budget: WorkBudget::default(),
        cancellation_requested: false,
        grants: [
            (Capability::Observe, RiskTier::ReadOnly),
            (Capability::Plan, RiskTier::ReadOnly),
            (Capability::ConfigureProduction, RiskTier::Reversible),
            (Capability::ControlClock, RiskTier::Reversible),
        ]
        .into_iter()
        .map(|(capability, max_risk)| CapabilityGrant {
            capability,
            max_risk,
            scope: CapabilityScope {
                fortress_id: Some(snapshot.fortress_id),
                ..CapabilityScope::default()
            },
            expires_at_tick: None,
            remaining_uses: None,
        })
        .collect(),
    }
}

fn request() -> Result<ProductionRequest> {
    ProductionRequest::parse(
        r#"{"template":"production","quotas":[{"item":"DRINK","minimum":60}]}"#,
    )
}

fn source() -> Result<ProductionContinuation> {
    let root = Digest32::of_bytes(b"original goal");
    ProductionContinuation::new(root, root, request()?)
}

fn original(snapshot: &WorldSnapshot, request: &ProductionRequest) -> Result<PreparedPlan> {
    let compiled = request.compile(snapshot)?;
    let mut intent = Intent {
        id: IntentId::new(1),
        anchor: snapshot.anchor(),
        summary: "original production goal".to_owned(),
        terminal_condition: compiled.terminal.clone(),
        constraints: vec![Constraint::MaxRisk(RiskTier::Reversible)],
        requested_actions: parse_steps(&compiled.actions)?,
    };
    compiled.apply_capacity_horizon(&mut intent)?;
    StaticPlanner::default().prepare_laboratory(snapshot, &intent, &context(snapshot))
}

#[test]
fn continuation_restores_original_stock_after_successful_work_and_consumption() -> Result<()> {
    let mut adapter = MemoryAdapter::new(world()?);
    adapter.advance_ticks(1099)?;
    let original_request = request()?;
    let original = original(adapter.snapshot(), &original_request)?;
    let immutable = original.clone();
    let prepared = adapter.prepare(&original, &context(adapter.snapshot()))?;
    let receipt = adapter.commit(&original, &prepared, &context(adapter.snapshot()))?;
    adapter.advance_ticks(200)?;
    let mut quiet = true;
    for action in &receipt.actions {
        let polled = adapter.poll_action(action.action_id, &context(adapter.snapshot()))?;
        assert_eq!(polled.state, CommitState::Verified);
        quiet &= adapter.action_work_state(action.action_id)?.is_quiescent();
    }
    assert!(quiet);
    assert_eq!(
        PredicateEvidence::laboratory(adapter.snapshot())?
            .evaluate(&original.terminal_condition)?,
        PredicateTruth::False
    );
    validate_goal_evidence(
        &original,
        adapter.snapshot(),
        &context(adapter.snapshot()),
        quiet,
    )?;
    let continuation =
        ProductionContinuation::new(original.digest, original.digest, original_request)?;
    let before = adapter.snapshot().clone();
    let (intent, analysis) =
        continuation.compile(IntentId::new(2), adapter.snapshot(), "restore drinks")?;
    assert_eq!(adapter.snapshot(), &before, "planning performs no effects");
    assert_eq!(intent.terminal_condition, original.terminal_condition);
    assert_eq!(analysis["requirements"][0]["stock"], 53);
    assert!(matches!(
        intent.requested_actions[0].action,
        Action::CreateWorkOrder { amount: 2, .. }
    ));
    let next = StaticPlanner::default().prepare_laboratory(
        adapter.snapshot(),
        &intent,
        &context(adapter.snapshot()),
    )?;
    assert_ne!(next.digest, original.digest);
    assert_ne!(
        next.steps[0].idempotency_key,
        original.steps[0].idempotency_key
    );
    let prepared = adapter.prepare(&next, &context(adapter.snapshot()))?;
    let receipt = adapter.commit(&next, &prepared, &context(adapter.snapshot()))?;
    adapter.advance_ticks(100)?;
    for action in receipt.actions {
        assert_eq!(
            adapter
                .poll_action(action.action_id, &context(adapter.snapshot()))?
                .state,
            CommitState::Verified
        );
    }
    assert_eq!(
        PredicateEvidence::laboratory(adapter.snapshot())?
            .evaluate(&original.terminal_condition)?,
        PredicateTruth::True
    );
    assert_eq!(
        original, immutable,
        "continuation never edits the old plan or proof"
    );
    Ok(())
}

#[test]
fn originally_satisfied_quota_is_retained_and_replanned_when_it_later_falls() -> Result<()> {
    let request = ProductionRequest::parse(
        r#"{"template":"production","quotas":[{"item":"DRINK","minimum":40},{"item":"FOOD","minimum":65}]}"#,
    )?;
    let mut adapter = MemoryAdapter::new(world()?);
    adapter.advance_ticks(1150)?;
    let original = original(adapter.snapshot(), &request)?;
    assert_eq!(original.steps.len(), 1);
    let prepared = adapter.prepare(&original, &context(adapter.snapshot()))?;
    let receipt = adapter.commit(&original, &prepared, &context(adapter.snapshot()))?;
    adapter.advance_ticks(50)?;
    for action in receipt.actions {
        adapter.poll_action(action.action_id, &context(adapter.snapshot()))?;
    }
    let source = ProductionContinuation::new(original.digest, original.digest, request)?;
    let restored = ProductionContinuation::parse(&source.canonical_json())?;
    let (intent, analysis) = restored.compile(
        IntentId::new(2),
        adapter.snapshot(),
        "both original reserves",
    )?;
    assert_eq!(intent.requested_actions.len(), 1);
    assert!(matches!(&intent.requested_actions[0].action,
        Action::CreateWorkOrder { job_token, .. } if job_token == "BREW_DRINK"));
    assert_eq!(intent.terminal_condition, original.terminal_condition);
    assert_eq!(analysis["requirements"].as_array().map(Vec::len), Some(2));
    assert_eq!(source, restored);
    Ok(())
}

#[test]
fn continuation_refuses_unknown_true_and_unresolved_original_goals() -> Result<()> {
    let mut snapshot = world()?;
    let original = original(&snapshot, &request()?)?;
    assert_eq!(
        validate_goal_evidence(&original, &snapshot, &context(&snapshot), false)
            .err()
            .map(|error| error.code),
        Some(ErrorCode::PreconditionsFailed)
    );
    let stock = snapshot
        .graph
        .entities
        .get_mut(&starter::STOCK_LEDGER)
        .and_then(|ledger| ledger.fields.get_mut(effects::STOCK_DRINK_FIELD))
        .ok_or_else(|| invalid("missing fixture stock"))?;
    stock.presence = Some(FactPresence::Unknown("not observed".to_owned()));
    snapshot.cursor = snapshot
        .cursor
        .checked_next()
        .ok_or_else(|| invalid("fixture cursor exhausted"))?;
    snapshot.refresh_hash();
    assert_eq!(
        validate_goal_evidence(&original, &snapshot, &context(&snapshot), true)
            .err()
            .map(|error| error.code),
        Some(ErrorCode::PreconditionsFailed)
    );
    let stock = snapshot
        .graph
        .entities
        .get_mut(&starter::STOCK_LEDGER)
        .and_then(|ledger| ledger.fields.get_mut(effects::STOCK_DRINK_FIELD))
        .ok_or_else(|| invalid("missing fixture stock"))?;
    stock.presence = Some(FactPresence::Known(WorldValue::U64(60)));
    stock.value = WorldValue::U64(60);
    snapshot.cursor = snapshot
        .cursor
        .checked_next()
        .ok_or_else(|| invalid("fixture cursor exhausted"))?;
    snapshot.refresh_hash();
    assert_eq!(
        validate_goal_evidence(&original, &snapshot, &context(&snapshot), true)
            .err()
            .map(|error| error.code),
        Some(ErrorCode::InvalidIntent)
    );
    Ok(())
}

#[test]
fn current_observe_and_plan_authority_are_required_without_copying_old_grants() -> Result<()> {
    let snapshot = world()?;
    let original = original(&snapshot, &request()?)?;
    for capability in [Capability::Observe, Capability::Plan] {
        let mut context = context(&snapshot);
        context
            .grants
            .retain(|grant| grant.capability != capability);
        assert_eq!(
            validate_goal_evidence(&original, &snapshot, &context, true)
                .err()
                .map(|error| error.code),
            Some(ErrorCode::CapabilityDenied)
        );
    }
    let mut stale = context(&snapshot);
    stale.anchor.tick = GameTick(stale.anchor.tick.0 + 1);
    assert_eq!(
        validate_goal_evidence(&original, &snapshot, &stale, true)
            .err()
            .map(|error| error.code),
        Some(ErrorCode::StaleAnchor)
    );
    let mut abandoned = original.clone();
    abandoned.anchor.cursor.epoch += 1;
    assert_eq!(
        validate_goal_evidence(&abandoned, &snapshot, &context(&snapshot), true)
            .err()
            .map(|error| error.code),
        Some(ErrorCode::StaleAnchor)
    );
    Ok(())
}

#[test]
fn verified_recovery_continues_real_consumed_goals_across_two_observation_epochs() -> Result<()> {
    let mut adapter = MemoryAdapter::new(world()?);
    let original = original(adapter.snapshot(), &request()?)?;
    let immutable = original.clone();
    let prepared = adapter.prepare(&original, &context(adapter.snapshot()))?;
    let receipt = adapter.commit(&original, &prepared, &context(adapter.snapshot()))?;
    adapter.advance_ticks(200)?;
    for action in receipt.actions {
        assert_eq!(
            adapter
                .poll_action(action.action_id, &context(adapter.snapshot()))?
                .state,
            CommitState::Verified
        );
    }
    adapter.advance_ticks(1_000)?;
    let mut parent = original.clone();
    let mut continuation =
        ProductionContinuation::new(original.digest, original.digest, request()?)?;
    for epoch in 1..=2 {
        let persisted = adapter.snapshot().clone();
        adapter = MemoryAdapter::recovered(persisted.clone())?;
        let recovery = RecoveredGoalLineage::verify(
            &persisted,
            persisted.anchor(),
            adapter.snapshot().anchor(),
        )?;
        assert_eq!(recovery.active_anchor().cursor.epoch, epoch);
        assert_eq!(
            validate_goal_evidence(
                &parent,
                adapter.snapshot(),
                &context(adapter.snapshot()),
                true
            )
            .err()
            .map(|error| error.code),
            Some(ErrorCode::StaleAnchor),
            "an old plan alone cannot authorize its new observation epoch"
        );
        let quiet = parent.steps.iter().try_fold(true, |quiet, step| {
            dfmcp_intent::inspect_effect_work(
                adapter.snapshot(),
                &step.action,
                &step.idempotency_key,
                true,
            )
            .map(|work| quiet && work.is_quiescent())
        })?;
        assert!(quiet);
        validate_goal_evidence_in_active_lineage(
            &parent,
            recovery.active_anchor(),
            adapter.snapshot(),
            &context(adapter.snapshot()),
            quiet,
        )?;
        let (intent, _) = continuation.compile(
            IntentId::new(u128::from(epoch) + 1),
            adapter.snapshot(),
            "recovered original quota",
        )?;
        assert_eq!(intent.terminal_condition, original.terminal_condition);
        let next = StaticPlanner::default().prepare_laboratory(
            adapter.snapshot(),
            &intent,
            &context(adapter.snapshot()),
        )?;
        let prepared = adapter.prepare(&next, &context(adapter.snapshot()))?;
        let receipt = adapter.commit(&next, &prepared, &context(adapter.snapshot()))?;
        adapter.advance_ticks(100)?;
        for action in receipt.actions {
            assert_eq!(
                adapter
                    .poll_action(action.action_id, &context(adapter.snapshot()))?
                    .state,
                CommitState::Verified
            );
        }
        assert_eq!(
            PredicateEvidence::laboratory(adapter.snapshot())?
                .evaluate(&original.terminal_condition)?,
            PredicateTruth::True
        );
        assert_eq!(
            original, immutable,
            "recovery never reseals original history"
        );
        parent = next;
        continuation = continuation.next(parent.digest)?;
        if epoch == 1 {
            adapter.advance_ticks(1_100)?;
        }
    }
    Ok(())
}

#[test]
fn recovery_binding_requires_the_exact_durable_world_and_single_epoch_reset() -> Result<()> {
    let mut adapter = MemoryAdapter::new(world()?);
    adapter.advance_ticks(100)?;
    let persisted = adapter.snapshot().clone();
    let recovered = MemoryAdapter::recovered(persisted.clone())?;
    let valid = RecoveredGoalLineage::verify(
        &persisted,
        persisted.anchor(),
        recovered.snapshot().anchor(),
    )?;
    assert_eq!(valid.previous_anchor(), persisted.anchor());
    for variant in 0..5 {
        let mut changed = recovered.snapshot().clone();
        match variant {
            0 => changed.paused = !changed.paused,
            1 => changed.tick = GameTick(changed.tick.0 + 1),
            2 => changed.cursor.epoch += 1,
            3 => changed.cursor.sequence += 1,
            _ => changed.fortress_id = FortressId::new(999),
        }
        changed.refresh_hash();
        assert_eq!(
            RecoveredGoalLineage::verify(&persisted, persisted.anchor(), changed.anchor())
                .err()
                .map(|error| error.code),
            Some(ErrorCode::CorruptLedger),
            "changed recovery variant {variant}"
        );
    }
    let mut wrong_head = persisted.anchor();
    wrong_head.tick = GameTick(wrong_head.tick.0 + 1);
    assert_eq!(
        RecoveredGoalLineage::verify(&persisted, wrong_head, recovered.snapshot().anchor())
            .err()
            .map(|error| error.code),
        Some(ErrorCode::CorruptLedger)
    );
    let mut exhausted = persisted;
    exhausted.cursor.epoch = u64::MAX;
    exhausted.refresh_hash();
    assert_eq!(
        RecoveredGoalLineage::verify(
            &exhausted,
            exhausted.anchor(),
            recovered.snapshot().anchor()
        )
        .err()
        .map(|error| error.code),
        Some(ErrorCode::CursorGap)
    );
    Ok(())
}

#[test]
fn recovered_goal_binding_preserves_authority_truth_quiescence_and_continuity_gates() -> Result<()>
{
    let mut adapter = MemoryAdapter::new(world()?);
    let original = original(adapter.snapshot(), &request()?)?;
    adapter.advance_ticks(100)?;
    let persisted = adapter.snapshot().clone();
    let mut forked_original = original.clone();
    forked_original.anchor.state_hash = Digest32::of_bytes(b"another seal at the same cursor");
    assert_eq!(
        validate_goal_evidence_in_active_lineage(
            &forked_original,
            original.anchor,
            &persisted,
            &context(&persisted),
            true,
        )
        .err()
        .map(|error| error.code),
        Some(ErrorCode::StaleAnchor),
        "a later current cursor cannot hide an original/active fork"
    );
    let recovered = MemoryAdapter::recovered(persisted.clone())?;
    let snapshot = recovered.snapshot();
    let recovery = RecoveredGoalLineage::verify(&persisted, persisted.anchor(), snapshot.anchor())?;
    let validate = |snapshot: &WorldSnapshot, context: &OperationContext, quiet| {
        validate_goal_evidence_in_active_lineage(
            &original,
            recovery.active_anchor(),
            snapshot,
            context,
            quiet,
        )
    };
    validate(snapshot, &context(snapshot), true)?;
    assert_eq!(
        validate(snapshot, &context(snapshot), false)
            .err()
            .map(|error| error.code),
        Some(ErrorCode::PreconditionsFailed)
    );
    for capability in [Capability::Observe, Capability::Plan] {
        let mut expired = context(snapshot);
        for grant in &mut expired.grants {
            if grant.capability == capability {
                grant.expires_at_tick = Some(GameTick(0));
            }
        }
        assert_eq!(
            validate(snapshot, &expired, true)
                .err()
                .map(|error| error.code),
            Some(ErrorCode::CapabilityDenied)
        );
    }
    for variant in 0..4 {
        let mut changed = snapshot.clone();
        match variant {
            0 => changed.tick = GameTick(changed.tick.0 - 1),
            1 => changed.cursor.epoch += 1,
            2 => changed.paused = !changed.paused,
            _ => changed.fortress_id = FortressId::new(999),
        }
        changed.refresh_hash();
        assert_eq!(
            validate(&changed, &context(&changed), true)
                .err()
                .map(|error| error.code),
            Some(ErrorCode::StaleAnchor),
            "unestablished current lineage variant {variant}"
        );
    }
    for (presence, code) in [
        (
            FactPresence::Unknown("not observed after recovery".to_owned()),
            ErrorCode::PreconditionsFailed,
        ),
        (
            FactPresence::Known(WorldValue::U64(60)),
            ErrorCode::InvalidIntent,
        ),
    ] {
        let mut changed = snapshot.clone();
        let stock = changed
            .graph
            .entities
            .get_mut(&starter::STOCK_LEDGER)
            .and_then(|ledger| ledger.fields.get_mut(effects::STOCK_DRINK_FIELD))
            .ok_or_else(|| invalid("missing fixture stock"))?;
        if let FactPresence::Known(value) = &presence {
            stock.value = value.clone();
        }
        stock.presence = Some(presence);
        changed.cursor = changed
            .cursor
            .checked_next()
            .ok_or_else(|| invalid("fixture cursor exhausted"))?;
        changed.refresh_hash();
        assert_eq!(
            validate(&changed, &context(&changed), true)
                .err()
                .map(|error| error.code),
            Some(code)
        );
    }
    Ok(())
}

#[test]
fn lineage_changes_seal_even_when_lowered_actions_and_caller_summary_are_identical() -> Result<()> {
    let snapshot = world()?;
    let source = source()?;
    let (first, _) = source.compile(IntentId::new(7), &snapshot, "custom summary")?;
    let next = source.next(Digest32::of_bytes(b"another parent"))?;
    let (second, _) = next.compile(IntentId::new(7), &snapshot, "custom summary")?;
    assert_eq!(first.requested_actions, second.requested_actions);
    assert_eq!(first.terminal_condition, second.terminal_condition);
    let planner = StaticPlanner::default();
    let a = planner.prepare_laboratory(&snapshot, &first, &context(&snapshot))?;
    let b = planner.prepare_laboratory(&snapshot, &second, &context(&snapshot))?;
    assert_ne!(a.digest, b.digest);
    assert_eq!(next.root, source.root);
    let reopened = ProductionContinuation::parse(&next.canonical_json())?;
    assert_eq!(
        reopened
            .compile(IntentId::new(7), &snapshot, "custom summary")?
            .0,
        second
    );
    Ok(())
}

#[test]
fn unused_original_setup_permissions_are_part_of_the_continuation_seal() -> Result<()> {
    let snapshot = world()?;
    let root = Digest32::of_bytes(b"setup goal");
    let make = |x| {
        ProductionRequest::parse(&json!({"template":"production", "quotas":[{"item":"DRINK","minimum":60}],
        "prerequisites":{"assign_labor":true,"workshops":[{"building":"workshop:Still","location":[x,0,10]}]}}).to_string())
    };
    let a = ProductionContinuation::new(root, root, make(0)?)?;
    let b = ProductionContinuation::new(root, root, make(1)?)?;
    let (first, _) = a.compile(IntentId::new(7), &snapshot, "same")?;
    let (second, _) = b.compile(IntentId::new(7), &snapshot, "same")?;
    assert_eq!(first.requested_actions, second.requested_actions);
    assert_ne!(first.summary, second.summary);
    assert_ne!(a.source_digest(), b.source_digest());
    assert_eq!(ProductionContinuation::parse(&a.canonical_json())?, a);
    Ok(())
}

#[test]
fn repeated_continuation_does_not_grow_or_rewrite_the_original_request() -> Result<()> {
    let mut source = source()?;
    let original = source.request.canonical_json();
    let root = source.root;
    let bytes = source.canonical_json().len();
    for index in 0..1000u32 {
        source = source.next(Digest32::of_bytes(&index.to_be_bytes()))?;
        source = ProductionContinuation::parse(&source.canonical_json())?;
        assert_eq!(source.canonical_json().len(), bytes);
        assert_eq!(source.root, root);
        assert_eq!(source.request.canonical_json(), original);
    }
    Ok(())
}

#[test]
fn public_request_cannot_replace_original_requirements_or_import_proof() -> Result<()> {
    let root = source()?.root.to_hex();
    let good = json!({"template":"continue_goal", "plan_digest":root});
    assert!(is_request(&good.to_string()));
    assert_eq!(parse_request(&good.to_string())?.to_hex(), root);
    for (key, value) in [
        ("quotas", json!([])),
        ("root_plan_digest", json!(root)),
        ("physical_quiescent", json!(true)),
        ("authority", json!("configure_production")),
    ] {
        let mut bad = good.clone();
        bad[key] = value;
        assert!(parse_request(&bad.to_string()).is_err());
    }
    for raw in ["0".repeat(64), "f".repeat(63), "g".repeat(64)] {
        assert!(
            parse_request(&json!({"template":"continue_goal", "plan_digest":raw}).to_string())
                .is_err()
        );
    }
    assert!(parse_request(&"x".repeat(MAX_ACTIONS_JSON_BYTES + 1)).is_err());
    let duplicated = format!(
        "{{\"template\":\"continue_goal\",\"plan_digest\":\"{root}\",\"plan_digest\":\"{root}\"}}"
    );
    assert!(parse_request(&duplicated).is_err());
    Ok(())
}

#[test]
fn retained_source_rejects_unknown_schema_fields_and_unbounded_payload() -> Result<()> {
    let source = source()?;
    let original: Value = serde_json::from_str(&source.canonical_json())
        .map_err(|error| invalid(error.to_string()))?;
    for (field, value) in [
        ("schema", json!("future")),
        ("root_plan_digest", json!("0".repeat(64))),
        ("grants", json!([])),
        ("production", json!({"template":"continue_goal"})),
    ] {
        let mut bad = original.clone();
        bad[field] = value;
        assert!(ProductionContinuation::parse(&bad.to_string()).is_err());
    }
    assert!(ProductionContinuation::parse(&"x".repeat(MAX_ACTIONS_JSON_BYTES + 1)).is_err());
    assert!(
        source
            .compile(IntentId::new(1), &world()?, &"x".repeat(256))
            .is_err()
    );
    Ok(())
}

#[test]
fn next_request_uses_the_existing_fortress_plan_blueprint_contract() -> Result<()> {
    let root = source()?.root;
    let next = request_json("session-id", root);
    assert_eq!(next["tool"], "fortress.plan");
    assert_eq!(next["arguments"]["session_id"], "session-id");
    let raw = next["arguments"]["blueprint"]
        .as_str()
        .ok_or_else(|| invalid("request absent"))?;
    assert_eq!(parse_request(raw)?, root);
    Ok(())
}

fn legacy_archive() -> String {
    format!(
        "{{\"schema\":\"dfmcp.production-continuation/1\",\"parent_plan_digest\":\"{}\",\"root_plan_digest\":\"{}\",\"production\":{{\"quotas\":[{{\"item\":\"DRINK\",\"minimum\":60}}],\"template\":\"production\"}}}}",
        "1".repeat(64),
        "2".repeat(64)
    )
}

#[test]
fn legacy_archive_preserves_exact_source_bytes_and_digest_domain() -> Result<()> {
    let raw = legacy_archive();
    let legacy = ProductionContinuation::parse(&raw)?;
    assert_eq!(legacy.canonical_json(), raw);
    let mut original_domain = b"dfmcp-production-continuation-source/1\0".to_vec();
    original_domain.extend_from_slice(raw.as_bytes());
    assert_eq!(legacy.source_digest(), Digest32::of_bytes(&original_domain));
    assert_eq!(
        legacy.source_digest().to_hex(),
        "4793ae0dd140b10f944111af4d61cb625abef0eee59f954b425860a535ef9051"
    );
    assert_eq!(legacy.version, SourceVersion::Legacy);
    assert_eq!(legacy.request.planner(), None);
    assert!(legacy.lineage_json()["planner"].is_null());
    Ok(())
}

#[test]
fn legacy_archive_replays_the_old_algorithm_while_new_pursuit_covers_consumption() -> Result<()> {
    let mut adapter = MemoryAdapter::new(world()?);
    adapter.advance_ticks(1099)?;
    let legacy = ProductionContinuation::parse(&legacy_archive())?;
    let before = legacy.compile(IntentId::new(7), adapter.snapshot(), "old continuation")?;
    assert!(matches!(
        before.0.requested_actions[0].action,
        Action::CreateWorkOrder { amount: 4, .. }
    ));
    let current = ProductionContinuation::new(legacy.parent, legacy.root, legacy.request.clone())?;
    let upgraded = current.compile(IntentId::new(7), adapter.snapshot(), "new continuation")?;
    assert!(matches!(upgraded.0.requested_actions[0].action,
        Action::CreateWorkOrder { amount, .. } if amount > 4));
    assert_eq!(before.0.terminal_condition, upgraded.0.terminal_condition);
    assert!(current.same_original_request(&legacy));
    assert_ne!(current.source_digest(), legacy.source_digest());
    assert_eq!(
        current.request.canonical_json(),
        legacy.request.canonical_json()
    );
    assert_eq!(
        current.request.planner(),
        None,
        "the original request is retained; the outer version selects the new compiler"
    );
    let reopened = ProductionContinuation::parse(&legacy.canonical_json())?;
    assert_eq!(
        reopened.compile(IntentId::new(7), adapter.snapshot(), "old continuation")?,
        before
    );
    let reopened_current = ProductionContinuation::parse(&current.canonical_json())?;
    assert_eq!(
        reopened_current.compile(IntentId::new(7), adapter.snapshot(), "new continuation")?,
        upgraded
    );
    Ok(())
}

#[test]
fn new_continuation_of_legacy_parent_keeps_original_request_and_root() -> Result<()> {
    let legacy = ProductionContinuation::parse(&legacy_archive())?;
    let parent = Digest32::of_bytes(b"next retained committed parent");
    let next = legacy.next(parent)?;
    assert_eq!(next.version, SourceVersion::ConsumptionAwareV1);
    assert_eq!(next.parent, parent);
    assert_eq!(next.root, legacy.root);
    assert_eq!(next.request, legacy.request);
    assert_eq!(next.lineage_json()["source_schema"], SOURCE_SCHEMA);
    assert_eq!(next.lineage_json()["planner"], "consumption_aware_v1");
    assert_eq!(ProductionContinuation::parse(&next.canonical_json())?, next);
    Ok(())
}

#[test]
fn retry_comparison_accepts_only_the_same_original_request_and_lineage() -> Result<()> {
    let legacy = ProductionContinuation::parse(&legacy_archive())?;
    let current = ProductionContinuation::new(legacy.parent, legacy.root, legacy.request.clone())?;
    assert!(legacy.same_original_request(&current));
    let changed_quota = ProductionContinuation::new(
        legacy.parent,
        legacy.root,
        ProductionRequest::parse(
            r#"{"template":"production","quotas":[{"item":"DRINK","minimum":61}]}"#,
        )?,
    )?;
    assert!(!legacy.same_original_request(&changed_quota));
    let changed_permission = ProductionContinuation::new(
        legacy.parent,
        legacy.root,
        ProductionRequest::parse(
            r#"{"template":"production","quotas":[{"item":"DRINK","minimum":60}],"prerequisites":{"assign_labor":true}}"#,
        )?,
    )?;
    assert!(!legacy.same_original_request(&changed_permission));
    let other = Digest32::of_bytes(b"wrong lineage");
    assert!(!legacy.same_original_request(&ProductionContinuation::new(
        other,
        legacy.root,
        legacy.request.clone()
    )?));
    assert!(!legacy.same_original_request(&ProductionContinuation::new(
        legacy.parent,
        other,
        legacy.request.clone()
    )?));
    Ok(())
}

#[test]
fn legacy_envelope_cannot_smuggle_a_later_compiler_into_archived_history() -> Result<()> {
    let mut source: Value =
        serde_json::from_str(&legacy_archive()).map_err(|error| invalid(error.to_string()))?;
    source["production"]["planner"] = json!("consumption_aware_v1");
    assert_eq!(
        ProductionContinuation::parse(&source.to_string())
            .err()
            .map(|error| error.code),
        Some(ErrorCode::InvalidRequest)
    );
    source["schema"] = json!(SOURCE_SCHEMA);
    let current = ProductionContinuation::parse(&source.to_string())?;
    assert_eq!(
        current.request.planner(),
        Some(ProductionPlanner::ConsumptionAwareV1)
    );
    assert_eq!(
        ProductionContinuation::parse(&current.canonical_json())?,
        current
    );
    Ok(())
}
