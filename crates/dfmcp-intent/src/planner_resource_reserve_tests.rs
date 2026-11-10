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
