use super::*;
use crate::live_jobs::LiveJobObservation;
use crate::live_operations::LiveItem;
use crate::semantic_work_orders::{ObservedInventoryGoal, ObservedInventoryObjective};
use dfmcp_core::{
    CapabilityGrant, CapabilityScope, GameTick, IntentId, MapCoord, ObservationCursor,
    RequestId, SessionId, StepId, WorkBudget,
};
use dfmcp_intent::{
    Action, Constraint, Intent, ObligationSpec, PreparedPlan, RequestedAction, StaticPlanner,
};
use dfmcp_world::{CompareOp, Predicate, PredicateTruth, WorldGraph};

fn raw(rows: &[(u32, &str, u32)]) -> LiveOperationsObservation {
    LiveOperationsObservation {
        jobs: LiveJobObservation {
            bridge_generation: 601,
            df_version: "0.51.11".into(),
            dfhack_version: "0.51.11-r1".into(),
            year: 0,
            year_tick: 120,
            paused: true,
            site_id: 4,
            world_folder: "region1".into(),
            next_job_id: 100,
            jobs: Vec::new(),
        },
        next_building_id: 200,
        next_item_id: 300,
        buildings: Vec::new(),
        items: rows.iter().map(|(id, item_type, units)| LiveItem {
            native_id: *id,
            item_type: 5,
            type_key: (*item_type).to_owned(),
            subtype: -1,
            material_type: 420,
            material_index: 0,
            stack_size: *units,
            raw_position: MapCoord::new(1, 1, 1),
            flags: 64,
            container_native_id: None,
            holder_building_native_id: None,
        }).collect(),
        attachments: Vec::new(),
    }
}

fn context(raw: &LiveOperationsObservation) -> Result<(WorldSnapshot, OperationContext)> {
    let snapshot = WorldSnapshot::new(
        raw.jobs.fortress_id()?,
        raw.jobs.tick(),
        ObservationCursor { epoch: 2, sequence: 7 },
        true,
        WorldGraph::default(),
    );
    let context = OperationContext {
        session_id: SessionId::new(1),
        request_id: RequestId::new(1),
        anchor: snapshot.anchor(),
        budget: WorkBudget {
            max_wall_millis: 10000,
            max_bytes: 256 * 1024 * 1024,
            max_output_tokens: 65536,
            max_entities: 4096,
            max_actions: 1,
            max_game_ticks: 0,
        },
        grants: [
            Capability::Query, Capability::Observe, Capability::Plan,
            Capability::ConfigureProduction,
        ].into_iter().map(|capability| CapabilityGrant {
            capability,
            scope: CapabilityScope {
                fortress_id: Some(snapshot.fortress_id),
                ..CapabilityScope::default()
            },
            max_risk: RiskTier::Guarded,
            expires_at_tick: None,
            remaining_uses: None,
        }).collect(),
        cancellation_requested: false,
    };
    Ok((snapshot, context))
}

fn accepted(projection: &InventoryBaselineProjection) -> EvidencePolicy {
    let mut policy = EvidencePolicy::at(projection.snapshot().anchor());
    policy.paused = true;
    policy.entity_kinds.insert(EntityKind::Item, EvidenceCoverage::Complete);
    policy.sources = projection.observed_sources();
    policy
}

fn plan(
    snapshot: &WorldSnapshot,
    context: &OperationContext,
    terminal: Predicate,
) -> Result<PreparedPlan> {
    let mut policy = EvidencePolicy::at(snapshot.anchor());
    policy.paused = true;
    StaticPlanner::default().prepare_with_evidence(
        &PredicateEvidence::scoped(snapshot, policy)?,
        &Intent {
            id: IntentId::new(71),
            anchor: snapshot.anchor(),
            summary: "Observe a sealed original net inventory increase".into(),
            terminal_condition: terminal.clone(),
            constraints: vec![Constraint::MaxRisk(RiskTier::Guarded)],
            requested_actions: vec![RequestedAction {
                action: Action::CreateWorkOrder {
                    name: "original finite bed goal".into(),
                    job_token: "CONSTRUCT_BED".into(),
                    amount: 2,
                    conditions: Vec::new(),
                },
                preconditions: vec![Predicate::Paused(true)],
                postconditions: vec![terminal.clone()],
                compensation: None,
                obligation: Some(ObligationSpec {
                    terminal,
                    failure: None,
                    deadline_tick: GameTick(220),
                    poll_interval_ticks: 5,
                    stable_for_observations: 2,
                }),
                depends_on: Vec::new(),
            }],
        },
        context,
    )
}

fn reseal(plan: &mut PreparedPlan) {
    plan.digest = plan.compute_digest();
    plan.id = plan.expected_id();
}

#[test]
fn inventory_baseline_qualifies_stack_units_and_seals_exact_original_source() -> Result<()> {
    let observed = raw(&[(1, "BED", 2), (2, "DOOR", 99), (3, "BED", 3)]);
    let (snapshot, c) = context(&observed)?;
    let projection = InventoryBaselineProjection::new(snapshot.anchor(), observed.clone(), &c)?;
    let objective = projection.increase("BED", 2, accepted(&projection), &c)?;
    assert_eq!(objective.baseline_stack_units(), 5);
    assert_eq!(objective.increase_stack_units(), 2);
    assert_eq!(objective.target_stack_units(), 7);
    assert_eq!(objective.baseline_anchor(), snapshot.anchor());
    assert_eq!(objective.baseline_source_digest(), observed.source_digest()?);
    assert_eq!(objective.source_generation(), 601);
    assert_eq!(objective.baseline_source_counters(), [100, 200, 300]);
    assert_ne!(projection.snapshot().anchor(), objective.baseline_anchor());
    let original = plan(&snapshot, &c, objective.predicate())?;
    assert_eq!(ObservedInventoryIncreaseObjective::from_plan(&original), Some(objective.clone()));
    assert_eq!(
        ObservedInventoryGoal::from_selected_plan(&original, StepId::ZERO),
        Some(ObservedInventoryGoal::Increase(objective)),
    );
    assert!(ObservedInventoryObjective::from_plan(&original).is_none());
    Ok(())
}

#[test]
fn inventory_baseline_never_turns_unqualified_or_incomplete_evidence_into_zero() -> Result<()> {
    let observed = raw(&[(1, "BED", 2), (2, "DOOR", 7)]);
    let (snapshot, c) = context(&observed)?;
    let projection = InventoryBaselineProjection::new(snapshot.anchor(), observed, &c)?;
    for case in 0..6 {
        let mut policy = accepted(&projection);
        match case {
            0 => { policy.entity_kinds.clear(); }
            1 => { policy.entity_kinds.insert(EntityKind::Item, EvidenceCoverage::Observed); }
            2 => { policy.sources.retain(|source| !matches!(source, EvidenceSource::Observed { field, .. } if field == INVENTORY_TYPE_SOURCE)); }
            3 => { policy.sources.retain(|source| !matches!(source, EvidenceSource::Observed { field, .. } if field == INVENTORY_QUANTITY_SOURCE)); }
            4 => { policy.all_entities = EvidenceCoverage::Complete; }
            _ => { policy.sources.insert(EvidenceSource::Observed { field: INVENTORY_TYPE_SOURCE.into(), source_digest: Digest32::of_bytes(b"another census") }); }
        }
        assert!(projection.increase("BED", 1, policy, &c).is_err(), "case {case}");
    }

    let empty = raw(&[]);
    let projection = InventoryBaselineProjection::new(snapshot.anchor(), empty, &c)?;
    assert!(projection.increase("BED", 1, EvidencePolicy::at(projection.snapshot().anchor()), &c).is_err());
    let objective = projection.increase("BED", 1, accepted(&projection), &c)?;
    assert_eq!(objective.baseline_stack_units(), 0);
    assert_eq!(objective.target_stack_units(), 1);
    Ok(())
}

#[test]
fn inventory_baseline_refuses_wrong_frontier_authority_and_work_before_qualification() -> Result<()> {
    let observed = raw(&[(1, "BED", 2)]);
    let (snapshot, c) = context(&observed)?;
    for case in 0..6 {
        let mut changed = observed.clone();
        match case {
            0 => changed.jobs.year_tick += 1,
            1 => changed.jobs.year_tick -= 1,
            2 => changed.jobs.paused = false,
            3 => changed.jobs.site_id += 1,
            4 => changed.jobs.world_folder = "another-world".into(),
            _ => changed.jobs.bridge_generation = u64::MAX,
        }
        assert!(InventoryBaselineProjection::new(snapshot.anchor(), changed, &c).is_err(), "case {case}");
    }
    for case in 0..5 {
        let mut current = c.clone();
        match case {
            0 => current.cancellation_requested = true,
            1 => current.grants.retain(|grant| grant.capability != Capability::Observe),
            2 => current.budget.max_bytes = INVENTORY_BASELINE_PROJECTION_BYTES - 1,
            3 => current.budget.max_entities = 1,
            _ => current.anchor.cursor.sequence += 1,
        }
        assert!(InventoryBaselineProjection::new(snapshot.anchor(), observed.clone(), &current).is_err(), "authority {case}");
    }
    let projection = InventoryBaselineProjection::new(snapshot.anchor(), observed, &c)?;
    let mut expired = c.clone();
    for grant in &mut expired.grants { grant.expires_at_tick = Some(GameTick(119)); }
    assert!(projection.increase("BED", 1, accepted(&projection), &expired).is_err());
    let mut small = c.clone();
    small.budget.max_bytes = INVENTORY_BASELINE_QUALIFY_BYTES - 1;
    assert!(projection.increase("BED", 1, accepted(&projection), &small).is_err());
    Ok(())
}

#[test]
fn inventory_increase_recognition_binds_baseline_anchor_source_bounds_and_all_goal_copies() -> Result<()> {
    let observed = raw(&[(1, "BED", 5)]);
    let (snapshot, c) = context(&observed)?;
    let projection = InventoryBaselineProjection::new(snapshot.anchor(), observed, &c)?;
    let goal = projection.increase("BED", 2, accepted(&projection), &c)?.predicate();
    for case in 0..10 {
        let mut changed = goal.clone();
        if let Predicate::EntityQuantityIncrease {
            kind, filter_field, quantity_field, increase, baseline_anchor,
            baseline_source_digest, source_generation, baseline_source_counters, ..
        } = &mut changed {
            match case {
                0 => *kind = EntityKind::Building,
                1 => *filter_field = "label".into(),
                2 => *quantity_field = None,
                3 => *increase = 0,
                4 => *increase = ObservedInventoryIncreaseObjective::MAX_INCREASE_STACK_UNITS + 1,
                5 => baseline_anchor.cursor.sequence += 1,
                6 => *baseline_source_digest = Digest32::ZERO,
                7 => *source_generation = 0,
                8 => *source_generation = u64::MAX,
                _ => baseline_source_counters[2] = i32::MAX as u64 + 1,
            }
        }
        // Build once using the valid predicate, then make a structurally bounded
        // adversarial archive. Invalid predicates may be rejected by validation.
        let mut original = plan(&snapshot, &c, goal.clone())?;
        original.terminal_condition = changed.clone();
        original.steps[0].postconditions = vec![changed.clone()];
        original.steps[0].obligation.as_mut().ok_or_else(|| error(ErrorCode::InternalInvariantViolation, "missing fixture obligation"))?.terminal = changed;
        reseal(&mut original);
        assert!(ObservedInventoryIncreaseObjective::from_plan(&original).is_none(), "case {case}");
    }
    let mut original = plan(&snapshot, &c, goal.clone())?;
    original.steps[0].postconditions = vec![Predicate::True];
    reseal(&mut original);
    assert!(ObservedInventoryIncreaseObjective::from_plan(&original).is_none());
    let mut original = plan(&snapshot, &c, goal)?;
    original.steps[0].obligation.as_mut().ok_or_else(|| error(ErrorCode::InternalInvariantViolation, "missing fixture obligation"))?.failure = Some(Predicate::False);
    reseal(&mut original);
    assert!(ObservedInventoryIncreaseObjective::from_plan(&original).is_none());
    Ok(())
}

#[test]
fn inventory_increase_checks_original_native_floors_before_any_progress_history() -> Result<()> {
    let observed = raw(&[(1, "BED", 5)]);
    let (snapshot, c) = context(&observed)?;
    let projection = InventoryBaselineProjection::new(snapshot.anchor(), observed.clone(), &c)?;
    let objective = projection.increase("BED", 1, accepted(&projection), &c)?;
    objective.validate_operations(&observed)?;
    for case in 0..7 {
        let mut changed = observed.clone();
        match case {
            0 => changed.jobs.bridge_generation += 1,
            1 => changed.jobs.year_tick -= 1,
            2 => changed.jobs.next_job_id -= 1,
            3 => changed.next_building_id -= 1,
            4 => changed.next_item_id -= 1,
            5 => changed.jobs.site_id += 1,
            _ => changed.jobs.paused = false,
        }
        assert!(objective.validate_operations(&changed).is_err(), "case {case}");
    }
    let mut advanced = observed;
    advanced.jobs.year_tick += 5;
    advanced.jobs.next_job_id += 1;
    advanced.next_building_id += 1;
    advanced.next_item_id += 1;
    advanced.items[0].stack_size = 6;
    objective.validate_operations(&advanced)?;
    Ok(())
}

#[test]
fn inventory_increase_remains_distinct_from_an_absolute_minimum_and_cannot_overflow() -> Result<()> {
    let observed = raw(&[(1, "BED", 5)]);
    let (snapshot, c) = context(&observed)?;
    let projection = InventoryBaselineProjection::new(snapshot.anchor(), observed, &c)?;
    let objective = projection.increase("BED", 2, accepted(&projection), &c)?;
    let minimum = Predicate::EntityQuantity {
        kind: EntityKind::Item,
        filter_field: "type_key".into(),
        filter_value: Value::Text("BED".into()),
        quantity_field: Some("stack_size".into()),
        op: CompareOp::Ge,
        value: 7,
    };
    let minimum = plan(&snapshot, &c, minimum)?;
    let increase = plan(&snapshot, &c, objective.predicate())?;
    assert_ne!(minimum.digest, increase.digest);
    assert!(matches!(ObservedInventoryGoal::from_plan(&minimum), Some(ObservedInventoryGoal::Minimum(_))));
    assert!(ObservedInventoryIncreaseObjective::from_plan(&minimum).is_none());
    for (kind, units) in [("BAR", 1), ("BED", 0), ("BED", 1_000_001)] {
        assert!(projection.increase(kind, units, accepted(&projection), &c).is_err());
    }
    assert!(ObservedInventoryIncreaseObjective::qualified(
        "BED".into(), u64::MAX, 1, c.anchor, projection.source_digest(), 601, [100, 200, 300],
    ).is_err());

    // A later source-local projection is rebased only for this evaluator test;
    // the baseline stored inside the goal remains the original citizen anchor.
    let mut later = projection.snapshot().clone();
    later.cursor = ObservationCursor { epoch: c.anchor.cursor.epoch, sequence: c.anchor.cursor.sequence + 1 };
    later.refresh_hash();
    let mut policy = accepted(&projection);
    policy.anchor = Some(later.anchor());
    let evidence = PredicateEvidence::scoped(&later, policy)?;
    assert_eq!(evidence.evaluate(&objective.predicate())?, PredicateTruth::False);
    Ok(())
}
