//! The real original-goal publisher supplies analysis input; these tests never
//! reconstruct an operations-only state or synthesize a goal projection.
use super::*;
use dfmcp_adapter::live_operations::{
    JobItemAttachment, LiveBuilding, building_entity_id, item_entity_id,
};
use dfmcp_adapter::operations_analysis::{
    self as analysis, DiagnosisScope, MAX_ANALYSIS_WORK, MaterialDemand, OperationsStateView,
};

fn production_capture(queue: &WorkOrderObservation) -> LiveOperationsObservation {
    let mut raw = operations(queue, true, true);
    let job = &mut raw.jobs.jobs[0];
    job.suspended = true;
    job.holder_native_id = Some(7);
    job.attached_item_count = 2;
    job.required_item_filter_count = 2;
    let mut second = job.clone();
    second.native_id = 4;
    second.suspended = false;
    second.worker_native_id = Some(42);
    second.attached_item_count = 1;
    second.required_item_filter_count = 1;
    raw.jobs.jobs.push(second);
    raw.buildings.push(LiveBuilding {
        native_id: 7,
        building_type: 17,
        type_key: "Workshop".into(),
        x1: 1,
        y1: 1,
        x2: 3,
        y2: 3,
        z: 1,
        build_stage: 1,
        max_build_stage: 3,
    });
    let input = &mut raw.items[0];
    input.type_key = "WOOD".into();
    input.flags = 16; // Direct rotten flag and separately forbidden container.
    input.container_native_id = Some(3);
    let mut container = input.clone();
    container.native_id = 3;
    container.type_key = "BIN".into();
    container.flags = 65;
    container.container_native_id = None;
    let mut supply = container.clone();
    supply.native_id = 4;
    supply.type_key = "WOOD".into();
    supply.flags = 64;
    supply.stack_size = 7;
    let mut removed = supply.clone();
    removed.native_id = 5;
    removed.flags = 72;
    raw.items.extend([container, supply, removed]);
    raw.attachments = vec![
        JobItemAttachment {
            job_native_id: 3,
            item_native_id: 2,
            role: 0,
            filter_index: 0,
        },
        JobItemAttachment {
            job_native_id: 3,
            item_native_id: 2,
            role: 1,
            filter_index: 0,
        },
        JobItemAttachment {
            job_native_id: 4,
            item_native_id: 2,
            role: 0,
            filter_index: 0,
        },
    ];
    raw
}

fn capture_projection(
    fixture: &Fixture,
    session: &mut Session,
    monitor: &mut WorkOrderGoalMonitor,
    sequence: u64,
    present: bool,
) -> Result<WorkOrderGoalProjection> {
    let mut observed = None;
    poll_with_operations(
        fixture,
        session,
        monitor,
        sequence,
        10 * sequence,
        5,
        |queue| {
            if present {
                production_capture(queue)
            } else {
                operations(queue, false, false)
            }
        },
        |projection| {
            observed = Some(projection.clone());
            Ok(grant(projection))
        },
    )?;
    observed.ok_or_else(rejected)
}

fn captured() -> Result<(Fixture, WorkOrderGoalProjection)> {
    let (fixture, mut session, review) = goal_fixture()?;
    let mut monitor = session.begin_goal_monitor(review.seal(), &fixture.context)?;
    let projection = capture_projection(&fixture, &mut session, &mut monitor, 1, true)?;
    Ok((fixture, projection))
}

fn context(fixture: &Fixture, projection: &WorkOrderGoalProjection) -> OperationContext {
    let mut context = fixture.context.clone();
    context.anchor = projection.anchor();
    context
        .grants
        .retain(|grant| grant.capability == Capability::Query);
    context
}

fn demands() -> Vec<MaterialDemand> {
    ["first", "second"]
        .into_iter()
        .map(|key| MaterialDemand {
            key: key.into(),
            units: 4,
            item_types: vec!["WOOD".into()],
            subtype: None,
            material_type: None,
            material_index: None,
        })
        .collect()
}

#[test]
fn original_goal_analysis_uses_exact_selected_observation_identity() -> Result<()> {
    let (fixture, projection) = captured()?;
    let context = context(&fixture, &projection);
    let trace = fixture.trace.borrow().clone();
    let journal = fixture.journal.stored_bytes();
    let associations = fixture.associations.stored_bytes();
    let snapshot = projection.snapshot().clone();
    let diagnosis = analysis::diagnose_production(
        &projection,
        &context,
        DiagnosisScope::default(),
        MAX_ANALYSIS_WORK,
    )?;
    let allocation =
        analysis::plan_inventory(&projection, &context, &demands(), MAX_ANALYSIS_WORK)?;
    assert_eq!(diagnosis.anchor, projection.anchor());
    assert_eq!(allocation.anchor, diagnosis.anchor);
    assert_eq!(diagnosis.source_digest, projection.source_digest());
    assert_eq!(allocation.source_digest, projection.source_digest());
    assert_ne!(
        diagnosis.source_digest,
        projection.operations().source_digest()?,
        "raw operations alone cannot replace the selected original-order source identity"
    );
    assert!(std::ptr::eq(
        projection.operations_snapshot().ok_or_else(rejected)?,
        projection.snapshot()
    ));
    assert!(std::ptr::eq(
        projection.operations_observation().ok_or_else(rejected)?,
        projection.operations()
    ));
    assert_eq!(projection.original_anchor(), fixture.original.anchor);
    for row in &diagnosis.rows {
        assert_eq!(row.job.revision, projection.anchor().cursor.sequence);
        assert_eq!(row.job.generation, 1);
        assert_eq!(row.holder.map(|h| h.revision), Some(row.job.revision));
    }
    assert_eq!(projection.snapshot(), &snapshot);
    assert_eq!(*fixture.trace.borrow(), trace);
    assert_eq!(fixture.journal.stored_bytes(), journal);
    assert_eq!(fixture.associations.stored_bytes(), associations);
    Ok(())
}

#[test]
fn original_goal_diagnostics_and_joint_inventory_share_observed_conditions() -> Result<()> {
    let (fixture, projection) = captured()?;
    let context = context(&fixture, &projection);
    let diagnosis = analysis::diagnose_production(
        &projection,
        &context,
        DiagnosisScope::default(),
        MAX_ANALYSIS_WORK,
    )?;
    assert_eq!(diagnosis.jobs_considered, 2);
    assert_eq!(diagnosis.jobs_with_findings, 2);
    let first = diagnosis.rows.first().ok_or_else(rejected)?;
    assert_eq!(first.native_job_id, 3);
    assert!(first.suspended);
    assert!(!first.worker_assigned);
    assert_eq!(
        first.holder.map(|h| h.entity_id),
        Some(building_entity_id(7))
    );
    assert_eq!(first.holder_stage, Some((1, 3)));
    assert_eq!(first.attachment_records, 2);
    assert_eq!(first.distinct_attached_items, 1);
    assert_eq!(first.filters_without_indexed_attachments, 1);
    for row in &diagnosis.rows {
        assert_eq!(row.direct_item_flags.get("rotten"), Some(&1));
        assert_eq!(row.container_item_flags.get("forbidden"), Some(&1));
        assert_eq!(row.shared_attached_items, 1);
        assert_eq!(row.affected_item_count, 1);
        assert!(row.findings.contains(&"holder_under_construction"));
    }
    let allocation =
        analysis::plan_inventory(&projection, &context, &demands(), MAX_ANALYSIS_WORK)?;
    assert_eq!(allocation.candidate_items, 1);
    assert_eq!(allocation.candidate_stack_units, 7);
    assert_eq!(
        allocation.excluded_items.get("forbidden_in_chain"),
        Some(&2)
    );
    assert_eq!(allocation.excluded_items.get("removed_in_chain"), Some(&1));
    assert_eq!(allocation.allocation.requested_units, 8);
    assert_eq!(allocation.allocation.allocated_units, 7);
    assert_eq!(allocation.allocation.cut_capacity, 7);
    assert_eq!(
        allocation
            .allocation
            .shortage
            .as_ref()
            .map(|cut| cut.deficit),
        Some(1)
    );
    assert!(
        allocation
            .allocation
            .assignments
            .iter()
            .all(|assignment| assignment.supply_id == item_entity_id(4).get())
    );
    assert_eq!(
        allocation
            .allocation
            .assignments
            .iter()
            .map(|assignment| assignment.units)
            .sum::<u64>(),
        7,
        "joint demands cannot count the same observed stack twice"
    );
    Ok(())
}

#[test]
fn original_goal_analysis_preserves_retirement_and_reappearance_generations() -> Result<()> {
    let (fixture, mut session, review) = goal_fixture()?;
    let mut monitor = session.begin_goal_monitor(review.seal(), &fixture.context)?;
    let first = capture_projection(&fixture, &mut session, &mut monitor, 1, true)?;
    let retired = capture_projection(&fixture, &mut session, &mut monitor, 2, false)?;
    let returned = capture_projection(&fixture, &mut session, &mut monitor, 3, true)?;
    let empty = analysis::diagnose_production(
        &retired,
        &context(&fixture, &retired),
        DiagnosisScope::default(),
        MAX_ANALYSIS_WORK,
    )?;
    assert_eq!(empty.jobs_considered, 0);
    assert!(empty.rows.is_empty());
    assert_ne!(first.source_digest(), returned.source_digest());
    let context = context(&fixture, &returned);
    let focused = |generation| DiagnosisScope {
        job: Some((EntityId::new(5), generation)),
        ..DiagnosisScope::default()
    };
    assert_eq!(
        analysis::diagnose_production(&returned, &context, focused(1), MAX_ANALYSIS_WORK)
            .err()
            .ok_or_else(rejected)?
            .code,
        ErrorCode::Conflict
    );
    let diagnosis =
        analysis::diagnose_production(&returned, &context, focused(2), MAX_ANALYSIS_WORK)?;
    let row = diagnosis.rows.first().ok_or_else(rejected)?;
    assert_eq!(row.job.generation, 2);
    assert_eq!(row.holder.map(|holder| holder.generation), Some(2));
    assert_eq!(row.job.revision, returned.anchor().cursor.sequence);
    assert_eq!(row.affected_item_examples.len(), 1);
    assert_eq!(row.affected_item_examples[0].generation, 2);
    assert_eq!(
        returned.snapshot().graph.entities[&item_entity_id(4)].generation,
        2
    );
    assert_eq!(returned.anchor().cursor.epoch, first.anchor().cursor.epoch);
    Ok(())
}

#[test]
fn original_goal_analysis_rechecks_anchor_scope_authority_and_work() -> Result<()> {
    let (fixture, projection) = captured()?;
    let current = context(&fixture, &projection);
    let snapshot = projection.snapshot().clone();
    for case in 0..8 {
        let mut context = current.clone();
        let expected = match case {
            0 => {
                context.grants.clear();
                ErrorCode::CapabilityDenied
            }
            1 => {
                context.cancellation_requested = true;
                ErrorCode::CancellationRequested
            }
            2 => {
                context.anchor.cursor.sequence += 1;
                ErrorCode::StaleAnchor
            }
            3 => {
                context.anchor.state_hash = Digest32::of_bytes(b"another source");
                ErrorCode::StaleAnchor
            }
            4 => {
                context.budget.max_entities = snapshot.graph.entities.len() as u32 - 1;
                ErrorCode::BudgetExceeded
            }
            5 => {
                context.grants[0].scope.entity_ids = BTreeSet::from([EntityId::new(5)]);
                ErrorCode::CapabilityDenied
            }
            6 => {
                context.grants[0].expires_at_tick = Some(GameTick(context.anchor.tick.get() - 1));
                ErrorCode::CapabilityDenied
            }
            _ => {
                context.grants[0].remaining_uses = Some(1);
                ErrorCode::CapabilityDenied
            }
        };
        assert_eq!(
            analysis::diagnose_production(
                &projection,
                &context,
                DiagnosisScope::default(),
                MAX_ANALYSIS_WORK
            )
            .err()
            .ok_or_else(rejected)?
            .code,
            expected,
            "diagnosis case {case}"
        );
        assert_eq!(
            analysis::plan_inventory(&projection, &context, &demands(), MAX_ANALYSIS_WORK)
                .err()
                .ok_or_else(rejected)?
                .code,
            expected,
            "inventory case {case}"
        );
    }
    for max_work in [0, 1, MAX_ANALYSIS_WORK + 1] {
        assert_eq!(
            analysis::diagnose_production(
                &projection,
                &current,
                DiagnosisScope::default(),
                max_work
            )
            .err()
            .ok_or_else(rejected)?
            .code,
            ErrorCode::BudgetExceeded
        );
        assert_eq!(
            analysis::plan_inventory(&projection, &current, &demands(), max_work)
                .err()
                .ok_or_else(rejected)?
                .code,
            ErrorCode::BudgetExceeded
        );
    }
    assert_eq!(projection.snapshot(), &snapshot);
    Ok(())
}
