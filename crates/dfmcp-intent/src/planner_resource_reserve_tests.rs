//! Exercise the real planner admission, finite-source proof and plan archive.

use super::*;

fn context(snapshot: &WorldSnapshot) -> OperationContext {
    OperationContext {
        session_id: SessionId::new(8_271),
        request_id: RequestId::new(2),
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
    }
}

fn intent(snapshot: &WorldSnapshot) -> Result<Intent> {
    let acquisition =
        ResourceAcquisitionPlan::compile(snapshot, &[target(HarvestResource::Plant, 3)], true)?;
    Ok(Intent {
        id: IntentId::new(8_272),
        anchor: snapshot.anchor(),
        summary: "gather inputs for the original future drink reserve".to_owned(),
        terminal_condition: Predicate::FieldCompare {
            entity_id: LEDGER,
            field: effects::STOCK_DRINK_FIELD.to_owned(),
            op: CompareOp::Ge,
            value: Value::U64(10_000),
        },
        constraints: vec![
            Constraint::MaxRisk(RiskTier::Guarded),
            Constraint::LaboratoryResourceProductionReserveUntil(GameTick(12_000)),
        ],
        requested_actions: acquisition.requested_actions,
    })
}

#[test]
fn reserve_harvest_keeps_original_goal_and_mandatory_finite_proof() -> Result<()> {
    let snapshot = world(30);
    let before = snapshot.canonical_bytes();
    let intent = intent(&snapshot)?;
    let plan =
        StaticPlanner::default().prepare_laboratory(&snapshot, &intent, &context(&snapshot))?;
    assert_eq!(snapshot.canonical_bytes(), before);
    assert_eq!(plan.terminal_condition, intent.terminal_condition);
    assert!(plan.requires_checkpoint);
    assert!(plan.required_capabilities.contains(&Capability::Designate));
    assert!(plan.required_capabilities.contains(&Capability::Checkpoint));
    assert!(
        plan.summary
            .ends_with("[laboratory resource production reserve until 12000]")
    );
    let evidence = dfmcp_world::PredicateEvidence::laboratory(&snapshot)?;
    assert!(evidence.establishes(&plan.terminal_condition)?);
    for step in &plan.steps {
        let obligation = step
            .obligation
            .as_ref()
            .ok_or_else(|| refused("missing harvest proof"))?;
        assert!(obligation.deadline_tick.0 < 12_000);
        assert_ne!(
            evidence.evaluate(&obligation.terminal)?,
            PredicateTruth::True
        );
    }
    let (bytes, seal) = crate::plan_archive::encode(&plan, Default::default())?;
    assert_eq!(
        crate::plan_archive::decode(&bytes, seal, Default::default())?,
        plan
    );
    Ok(())
}

#[test]
fn old_reserve_generic_planning_and_summary_lookalikes_cannot_admit_harvest() -> Result<()> {
    let snapshot = world(30);
    let planner = StaticPlanner::default();
    let context = context(&snapshot);
    let original = intent(&snapshot)?;
    assert!(planner.prepare(&snapshot, &original, &context).is_err());
    let mut old = original.clone();
    old.constraints[1] = Constraint::LaboratoryProductionReserveUntil(GameTick(12_000));
    assert!(
        planner
            .prepare_laboratory(&snapshot, &old, &context)
            .is_err()
    );
    let mut lookalike = original.clone();
    lookalike.constraints.pop();
    lookalike.summary = "[laboratory resource production reserve until 12000]".to_owned();
    assert!(
        planner
            .prepare_laboratory(&snapshot, &lookalike, &context)
            .is_err()
    );
    for extra in [
        Constraint::LaboratoryProductionReserveUntil(GameTick(12_000)),
        Constraint::LaboratoryResourceProductionReserveUntil(GameTick(12_000)),
    ] {
        let mut duplicate = original.clone();
        duplicate.constraints.push(extra);
        assert!(
            planner
                .prepare_laboratory(&snapshot, &duplicate, &context)
                .is_err()
        );
    }
    Ok(())
}

#[test]
fn reserve_harvest_rejects_mixed_effects_unrelated_labor_and_setup_without_harvest() -> Result<()> {
    let snapshot = world(30);
    let original = intent(&snapshot)?;
    for action in [
        Action::Pause { paused: true },
        Action::CreateWorkOrder {
            name: "unfunded".to_owned(),
            job_token: "BREW_DRINK".to_owned(),
            amount: 3,
            conditions: vec![],
        },
        Action::SetLabor {
            units: vec![WORKER],
            labor: "CARPENTRY".to_owned(),
            enabled: true,
        },
        Action::SetLabor {
            units: vec![WORKER],
            labor: "WOODCUTTING".to_owned(),
            enabled: true,
        },
        Action::SetLabor {
            units: vec![WORKER],
            labor: "PLANT_GATHERING".to_owned(),
            enabled: false,
        },
    ] {
        let mut mixed = original.clone();
        let mut extra = mixed.requested_actions[0].clone();
        extra.action = action;
        mixed.requested_actions.push(extra);
        assert!(
            StaticPlanner::default()
                .prepare_laboratory(&snapshot, &mixed, &context(&snapshot))
                .is_err()
        );
    }
    let mut setup = original;
    setup.requested_actions[0].action = Action::SetLabor {
        units: vec![WORKER],
        labor: "PLANT_GATHERING".to_owned(),
        enabled: true,
    };
    assert!(
        StaticPlanner::default()
            .prepare_laboratory(&snapshot, &setup, &context(&snapshot))
            .is_err()
    );
    Ok(())
}

#[test]
fn reserve_harvest_accepts_only_qualified_original_stock_and_finite_horizon() -> Result<()> {
    let snapshot = world(30);
    let original = intent(&snapshot)?;
    let planner = StaticPlanner::default();
    for until in [
        0,
        snapshot.tick.0,
        snapshot.tick.0 + effects::MAX_DEFAULT_OBLIGATION_TICKS + 1,
        u64::MAX,
    ] {
        let mut invalid = original.clone();
        invalid.constraints[1] =
            Constraint::LaboratoryResourceProductionReserveUntil(GameTick(until));
        assert!(
            planner
                .prepare_laboratory(&snapshot, &invalid, &context(&snapshot))
                .is_err()
        );
    }
    for terminal in [
        Predicate::True,
        Predicate::Paused(false),
        Predicate::FieldCompare {
            entity_id: LEDGER,
            field: effects::STOCK_PLANT_FIELD.to_owned(),
            op: CompareOp::Ge,
            value: Value::U64(0),
        },
    ] {
        let mut invalid = original.clone();
        invalid.terminal_condition = terminal;
        assert!(
            planner
                .prepare_laboratory(&snapshot, &invalid, &context(&snapshot))
                .is_err()
        );
    }
    let mut unavailable = snapshot.clone();
    let fact = unavailable
        .graph
        .entities
        .get_mut(&LEDGER)
        .and_then(|ledger| ledger.fields.get_mut(effects::STOCK_DRINK_FIELD))
        .ok_or_else(|| refused("stock fixture missing"))?;
    fact.source = FactSource::Derived("untrusted".to_owned());
    unavailable.refresh_hash();
    let mut invalid = original;
    invalid.anchor = unavailable.anchor();
    assert!(
        planner
            .prepare_laboratory(&unavailable, &invalid, &context(&unavailable))
            .is_err()
    );
    Ok(())
}

#[test]
fn reserve_harvest_preserves_labor_source_generation_budget_and_authority_checks() -> Result<()> {
    let mut snapshot = world(30);
    set(
        &mut snapshot,
        WORKER,
        "labor.PLANT_GATHERING",
        Value::Bool(false),
    )?;
    let original = intent(&snapshot)?;
    let planner = StaticPlanner::default();
    let valid = planner.prepare_laboratory(&snapshot, &original, &context(&snapshot))?;
    assert_eq!(valid.steps.len(), 2);
    assert!(
        valid
            .required_capabilities
            .contains(&Capability::ConfigureLabor)
    );
    let mut no_authority = context(&snapshot);
    no_authority.grants.clear();
    assert!(
        planner
            .prepare_laboratory(&snapshot, &original, &no_authority)
            .is_err()
    );
    let mut too_small = context(&snapshot);
    too_small.budget.max_actions = 1;
    assert!(
        planner
            .prepare_laboratory(&snapshot, &original, &too_small)
            .is_err()
    );
    let mut changed = original;
    for requested in &mut changed.requested_actions {
        if let Action::HarvestResource {
            source_generation, ..
        } = &mut requested.action
        {
            *source_generation += 1;
        }
    }
    assert!(
        planner
            .prepare_laboratory(&snapshot, &changed, &context(&snapshot))
            .is_err()
    );
    Ok(())
}

#[test]
fn reserve_harvest_needs_qualified_remaining_consumption_at_the_original_tick() -> Result<()> {
    let mut snapshot = world(30);
    let remaining = 12_000 - snapshot.tick.0;
    let allowance = remaining / effects::DRINK_INTERVAL_TICKS;
    set(
        &mut snapshot,
        LEDGER,
        effects::STOCK_DRINK_FIELD,
        Value::U64(10_000 + allowance),
    )?;
    let funded = intent(&snapshot)?;
    let before = snapshot.canonical_bytes();
    assert!(
        StaticPlanner::default()
            .prepare_laboratory(&snapshot, &funded, &context(&snapshot))
            .is_err_and(|error| error.code == ErrorCode::InvalidIntent
                && error.message.contains("already covers"))
    );
    assert_eq!(snapshot.canonical_bytes(), before);

    // The remaining absolute duration is unchanged, but this observed
    // metabolism phase includes one additional consumption boundary.
    set(
        &mut snapshot,
        LEDGER,
        effects::METABOLISM_TICKS_FIELD,
        Value::U64(effects::DRINK_INTERVAL_TICKS - 1),
    )?;
    let phase = intent(&snapshot)?;
    let prepared =
        StaticPlanner::default().prepare_laboratory(&snapshot, &phase, &context(&snapshot))?;
    assert_eq!(prepared.terminal_condition, phase.terminal_condition);
    assert!(
        prepared
            .summary
            .ends_with("[laboratory resource production reserve until 12000]")
    );
    Ok(())
}

#[test]
fn reserve_harvest_keeps_zero_minima_duplicate_normalization_and_exact_archive_bytes() -> Result<()>
{
    let mut snapshot = world(30);
    set(
        &mut snapshot,
        LEDGER,
        effects::STOCK_DRINK_FIELD,
        Value::U64(0),
    )?;
    let mut original = intent(&snapshot)?;
    original.terminal_condition = Predicate::FieldCompare {
        entity_id: LEDGER,
        field: effects::STOCK_DRINK_FIELD.to_owned(),
        op: CompareOp::Ge,
        value: Value::U64(0),
    };
    let planner = StaticPlanner::default();
    let expected = planner.prepare_laboratory(&snapshot, &original, &context(&snapshot))?;
    let mut repeated = original.clone();
    repeated.terminal_condition = Predicate::All(vec![
        original.terminal_condition.clone(),
        Predicate::All(vec![original.terminal_condition.clone()]),
    ]);
    let actual = planner.prepare_laboratory(&snapshot, &repeated, &context(&snapshot))?;
    assert_eq!(actual, expected);
    assert_eq!(
        crate::plan_archive::encode(&actual, Default::default())?,
        crate::plan_archive::encode(&expected, Default::default())?,
        "published normalization and the exact typed archive must remain unchanged"
    );
    assert_eq!(
        expected.summary,
        format!(
            "{} [laboratory resource production reserve until 12000]",
            original.summary
        ),
        "do not rename the already published summary seal"
    );
    Ok(())
}

#[test]
fn reserve_harvest_cannot_hide_unrelated_effects_in_compensation() -> Result<()> {
    let snapshot = world(30);
    let original = intent(&snapshot)?;
    for compensation in [
        Action::Pause { paused: true },
        Action::SetStandingOrder {
            key: "reserve-side-effect".to_owned(),
            value: "true".to_owned(),
        },
        Action::SetLabor {
            units: vec![WORKER],
            labor: "PLANT_GATHERING".to_owned(),
            enabled: false,
        },
    ] {
        let mut changed = original.clone();
        changed.requested_actions[0].compensation = Some(compensation);
        assert!(
            StaticPlanner::default()
                .prepare_laboratory(&snapshot, &changed, &context(&snapshot))
                .is_err_and(|error| error.code == ErrorCode::InvalidIntent
                    && error.message.contains("compensation"))
        );
    }
    Ok(())
}

#[test]
fn reserve_labor_keeps_its_exact_inverse_and_ordinary_default_plan_bytes() -> Result<()> {
    let mut snapshot = world(30);
    set(
        &mut snapshot,
        WORKER,
        "labor.PLANT_GATHERING",
        Value::Bool(false),
    )?;
    let original = intent(&snapshot)?;
    let planner = StaticPlanner::default();
    let expected = planner.prepare_laboratory(&snapshot, &original, &context(&snapshot))?;
    assert_eq!(
        expected.steps[0].compensation,
        Some(Action::SetLabor {
            units: vec![WORKER],
            labor: "PLANT_GATHERING".to_owned(),
            enabled: false,
        })
    );

    let mut ordinary_default = original.clone();
    ordinary_default.requested_actions[0].compensation = None;
    let actual = planner.prepare_laboratory(&snapshot, &ordinary_default, &context(&snapshot))?;
    assert_eq!(actual, expected);
    assert_eq!(
        crate::plan_archive::encode(&actual, Default::default())?,
        crate::plan_archive::encode(&expected, Default::default())?
    );

    for compensation in [
        Action::Pause { paused: true },
        Action::SetLabor {
            units: vec![EntityId::new(12)],
            labor: "PLANT_GATHERING".to_owned(),
            enabled: false,
        },
        Action::SetLabor {
            units: vec![WORKER],
            labor: "WOODCUTTING".to_owned(),
            enabled: false,
        },
        Action::SetLabor {
            units: vec![WORKER],
            labor: "PLANT_GATHERING".to_owned(),
            enabled: true,
        },
    ] {
        let mut changed = original.clone();
        changed.requested_actions[0].compensation = Some(compensation);
        assert!(
            planner
                .prepare_laboratory(&snapshot, &changed, &context(&snapshot))
                .is_err_and(|error| error.code == ErrorCode::InvalidIntent
                    && error.message.contains("compensation"))
        );
    }
    Ok(())
}

#[test]
fn reserve_harvest_requires_the_complete_source_qualified_consumption_census() -> Result<()> {
    let snapshot = world(30);
    let original = intent(&snapshot)?;
    for variant in 0..8 {
        let mut changed = snapshot.clone();
        match variant {
            0 => {
                changed
                    .graph
                    .entities
                    .get_mut(&WORKER)
                    .expect("worker")
                    .fields
                    .remove("alive");
            }
            1 => {
                changed
                    .graph
                    .entities
                    .get_mut(&WORKER)
                    .expect("worker")
                    .fields
                    .get_mut("alive")
                    .expect("life state")
                    .source = FactSource::Derived("untrusted".to_owned());
            }
            2 => {
                changed
                    .graph
                    .entities
                    .get_mut(&WORKER)
                    .expect("worker")
                    .fields
                    .get_mut("alive")
                    .expect("life state")
                    .observed_at = GameTick(snapshot.tick.0 + 1);
            }
            3 => {
                changed
                    .graph
                    .entities
                    .get_mut(&WORKER)
                    .expect("worker")
                    .fields
                    .get_mut("alive")
                    .expect("life state")
                    .source_digest = Digest32::of_bytes(b"not the registered laboratory producer");
            }
            4 => {
                changed
                    .graph
                    .entities
                    .get_mut(&LEDGER)
                    .expect("ledger")
                    .fields
                    .remove(effects::METABOLISM_TICKS_FIELD);
            }
            5 => {
                changed
                    .graph
                    .entities
                    .get_mut(&LEDGER)
                    .expect("ledger")
                    .fields
                    .get_mut(effects::METABOLISM_TICKS_FIELD)
                    .expect("metabolism")
                    .source = FactSource::Derived("untrusted".to_owned());
            }
            6 => {
                changed
                    .graph
                    .entities
                    .get_mut(&WORKER)
                    .expect("worker")
                    .generation = 0;
            }
            _ => {
                changed
                    .graph
                    .entities
                    .get_mut(&LEDGER)
                    .expect("ledger")
                    .generation = 0;
            }
        }
        changed.refresh_hash();
        let mut request = original.clone();
        request.anchor = changed.anchor();
        assert!(
            StaticPlanner::default()
                .prepare_laboratory(&changed, &request, &context(&changed))
                .is_err_and(|error| error.code == ErrorCode::PreconditionsFailed
                    && error.message.starts_with("resource reserve")),
            "variant {variant}"
        );
    }
    Ok(())
}

#[test]
fn reserve_harvest_rejects_a_unit_kind_alias_even_when_its_anchor_is_unchanged() -> Result<()> {
    let mut snapshot = world(30);
    let original = intent(&snapshot)?;
    let anchor = snapshot.anchor();
    snapshot
        .graph
        .entities
        .get_mut(&WORKER)
        .expect("worker")
        .kind = EntityKind::Other(EntityKind::Unit.as_str().to_owned());
    snapshot.refresh_hash();
    assert_eq!(snapshot.anchor(), anchor, "legacy canonical kind alias");
    assert!(
        StaticPlanner::default()
            .prepare_laboratory(&snapshot, &original, &context(&snapshot))
            .is_err_and(|error| error.code == ErrorCode::PreconditionsFailed
                && error.message.contains("unit kind alias"))
    );
    Ok(())
}

#[test]
fn reserve_harvest_refuses_ambiguous_ledger_and_overflow_instead_of_inventing_no_work() -> Result<()>
{
    let snapshot = world(30);
    let original = intent(&snapshot)?;
    let mut duplicate = snapshot.clone();
    let mut ledger = duplicate
        .graph
        .entities
        .get(&LEDGER)
        .expect("ledger")
        .clone();
    ledger.id = EntityId::new(992);
    duplicate.graph.entities.insert(ledger.id, ledger);
    duplicate.refresh_hash();
    let mut request = original.clone();
    request.anchor = duplicate.anchor();
    assert!(
        StaticPlanner::default()
            .prepare_laboratory(&duplicate, &request, &context(&duplicate))
            .is_err_and(|error| error.code == ErrorCode::PreconditionsFailed
                && error.message.contains("ledger custody"))
    );

    for target_overflow in [false, true] {
        let mut changed = snapshot.clone();
        let mut request = original.clone();
        if target_overflow {
            set(
                &mut changed,
                LEDGER,
                effects::STOCK_DRINK_FIELD,
                Value::U64(u64::MAX),
            )?;
            request.terminal_condition = Predicate::FieldCompare {
                entity_id: LEDGER,
                field: effects::STOCK_DRINK_FIELD.to_owned(),
                op: CompareOp::Ge,
                value: Value::U64(u64::MAX),
            };
        } else {
            set(
                &mut changed,
                LEDGER,
                effects::METABOLISM_TICKS_FIELD,
                Value::U64(u64::MAX),
            )?;
        }
        request.anchor = changed.anchor();
        assert!(
            StaticPlanner::default()
                .prepare_laboratory(&changed, &request, &context(&changed))
                .is_err_and(|error| error.code == ErrorCode::BudgetExceeded)
        );
    }
    Ok(())
}

#[test]
fn reserve_harvest_bounds_the_complete_source_even_for_one_small_action() -> Result<()> {
    let mut snapshot = world(30);
    let mut request = intent(&snapshot)?;
    for index in 0..65_537 {
        let id = EntityId::new(1_000_000 + index);
        snapshot.graph.entities.insert(
            id,
            EntityRecord {
                id,
                generation: 1,
                revision: 1,
                kind: EntityKind::Item,
                label: String::new(),
                fields: BTreeMap::new(),
            },
        );
    }
    snapshot.refresh_hash();
    request.anchor = snapshot.anchor();
    assert!(
        StaticPlanner::default()
            .prepare_laboratory(&snapshot, &request, &context(&snapshot))
            .is_err_and(|error| error.code == ErrorCode::BudgetExceeded
                && error.message.starts_with("resource reserve"))
    );
    Ok(())
}
