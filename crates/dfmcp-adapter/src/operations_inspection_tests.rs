use super::*;
use crate::live_jobs::{LiveJob, LiveJobObservation};
use crate::live_operations::{
    JobItemAttachment, LiveBuilding, LiveItem, building_entity_id, item_entity_id,
};
use crate::operations_analysis::{
    DiagnosisScope, MAX_ANALYSIS_WORK, MaterialDemand, diagnose_production, plan_inventory,
};
use dfmcp_core::{
    CapabilityGrant, CapabilityScope, EntityId, GameTick, MapCoord, ObservationCursor, RequestId,
    WorkBudget,
};
use dfmcp_world::EntityKind;

fn item(id: u32, kind: &str, units: u32) -> LiveItem {
    LiveItem {
        native_id: id,
        item_type: 1,
        type_key: kind.to_owned(),
        subtype: -1,
        material_type: 0,
        material_index: 1,
        stack_size: units,
        raw_position: MapCoord::new(1, 2, 3),
        flags: 64,
        container_native_id: None,
        holder_building_native_id: None,
    }
}

fn observation() -> LiveOperationsObservation {
    let mut forbidden = item(12, "WOOD", 5);
    forbidden.flags |= 1;
    LiveOperationsObservation {
        jobs: LiveJobObservation {
            bridge_generation: 7,
            df_version: "df-test".to_owned(),
            dfhack_version: "dfhack-test".to_owned(),
            year: 105,
            year_tick: 3,
            paused: true,
            site_id: 1,
            world_folder: "region1".to_owned(),
            next_job_id: 100,
            jobs: vec![LiveJob {
                native_id: 7,
                job_type: 5,
                type_key: "ConstructBuilding".to_owned(),
                reaction: String::new(),
                suspended: true,
                repeating: false,
                position: MapCoord::new(1, 2, 3),
                worker_native_id: None,
                holder_native_id: Some(3),
                completion_timer: -1,
                attached_item_count: 1,
                required_item_filter_count: 1,
            }],
        },
        next_building_id: 100,
        next_item_id: 1000,
        buildings: vec![LiveBuilding {
            native_id: 3,
            building_type: 1,
            type_key: "Workshop".to_owned(),
            x1: 0,
            y1: 0,
            x2: 2,
            y2: 2,
            z: 3,
            build_stage: 1,
            max_build_stage: 3,
        }],
        items: vec![
            item(10, "WOOD", 3),
            item(11, "WOOD", 4),
            forbidden,
            item(13, "BOULDER", 2),
        ],
        attachments: vec![JobItemAttachment {
            job_native_id: 7,
            item_native_id: 10,
            role: 0,
            filter_index: 0,
        }],
    }
}

fn parent(operations: &LiveOperationsObservation, sequence: u64) -> Result<StateAnchor> {
    Ok(StateAnchor {
        fortress_id: operations.jobs.fortress_id()?,
        cursor: ObservationCursor { epoch: 9, sequence },
        tick: operations.jobs.tick(),
        state_hash: Digest32::of_bytes(&sequence.to_be_bytes()),
    })
}

fn context(anchor: StateAnchor) -> OperationContext {
    OperationContext {
        session_id: SessionId::new(17),
        request_id: RequestId::new(1),
        anchor,
        budget: WorkBudget {
            max_wall_millis: 10_000,
            max_entities: OPERATIONS_INSPECTION_MAX_ENTITIES as u32,
            max_bytes: OPERATIONS_INSPECTION_PROJECTION_BYTES,
            ..WorkBudget::default()
        },
        grants: [Capability::Query, Capability::Observe]
            .into_iter()
            .map(|capability| CapabilityGrant {
                capability,
                scope: CapabilityScope {
                    fortress_id: Some(anchor.fortress_id),
                    ..CapabilityScope::default()
                },
                max_risk: RiskTier::ReadOnly,
                expires_at_tick: None,
                remaining_uses: None,
            })
            .collect(),
        cancellation_requested: false,
    }
}

fn fixture() -> Result<(OperationsInspectionProjection, OperationContext)> {
    let operations = observation();
    let anchor = parent(&operations, 41)?;
    let context = context(anchor);
    Ok((
        OperationsInspectionProjection::new(anchor, operations, &context)?,
        context,
    ))
}

#[test]
fn original_free_inspection_keeps_the_complete_source_and_parent_distinct() -> Result<()> {
    let (projection, context) = fixture()?;
    assert_eq!(projection.parent_anchor(), context.anchor);
    assert_ne!(projection.anchor(), projection.parent_anchor());
    assert_eq!(projection.anchor().tick, projection.parent_anchor().tick);
    assert_eq!(projection.operations()?, &observation());
    assert_eq!(projection.source_digest(), observation().source_digest()?);
    assert_eq!(projection.snapshot()?.anchor(), projection.anchor());
    assert!(projection.snapshot()?.hash_is_valid());
    assert_eq!(projection.snapshot()?.graph.entities.len(), 7);
    assert!(
        !projection
            .snapshot()?
            .graph
            .entities
            .values()
            .any(|entity| {
                entity.kind == EntityKind::WorkOrder || entity.kind == EntityKind::Unit
            })
    );
    assert!(
        projection
            .snapshot()?
            .graph
            .entities
            .values()
            .all(|entity| {
                entity
                    .fields
                    .values()
                    .all(|fact| fact.source_digest == projection.source_digest())
            })
    );
    assert!(
        context
            .grants
            .iter()
            .all(|grant| grant.max_risk == RiskTier::ReadOnly)
    );
    Ok(())
}

#[test]
fn preflight_diagnosis_and_joint_allocation_use_the_real_shared_consumers() -> Result<()> {
    let (projection, mut context) = fixture()?;
    context.anchor = projection.anchor();
    let diagnosis = diagnose_production(
        &projection,
        &context,
        DiagnosisScope::default(),
        MAX_ANALYSIS_WORK,
    )?;
    assert_eq!(diagnosis.jobs_considered, 1);
    assert_eq!(diagnosis.rows[0].job.entity_id, EntityId::new(9));
    assert!(diagnosis.rows[0].suspended);
    assert!(!diagnosis.rows[0].worker_assigned);
    assert_eq!(diagnosis.rows[0].holder_stage, Some((1, 3)));
    assert_eq!(diagnosis.rows[0].distinct_attached_items, 1);
    assert_eq!(diagnosis.anchor, projection.anchor());
    assert_eq!(diagnosis.source_digest, projection.source_digest());
    let demand = |key: &str, units| MaterialDemand {
        key: key.to_owned(),
        units,
        item_types: vec!["WOOD".to_owned()],
        subtype: None,
        material_type: None,
        material_index: None,
    };
    let allocation = plan_inventory(
        &projection,
        &context,
        &[demand("beds", 4), demand("doors", 3)],
        MAX_ANALYSIS_WORK,
    )?;
    assert_eq!(allocation.candidate_items, 1);
    assert_eq!(allocation.allocation.requested_units, 7);
    assert_eq!(allocation.allocation.allocated_units, 4);
    assert_eq!(allocation.allocation.cut_capacity, 4);
    assert_eq!(
        allocation
            .allocation
            .shortage
            .as_ref()
            .map(|shortage| shortage.deficit),
        Some(3)
    );
    assert_eq!(
        allocation.excluded_items.get("attached_to_job_in_chain"),
        Some(&1)
    );
    assert_eq!(
        allocation.excluded_items.get("forbidden_in_chain"),
        Some(&1)
    );
    assert_eq!(allocation.source_digest, projection.source_digest());

    context.anchor = projection.parent_anchor();
    assert_eq!(
        diagnose_production(
            &projection,
            &context,
            DiagnosisScope::default(),
            MAX_ANALYSIS_WORK,
        )
        .unwrap_err()
        .code,
        ErrorCode::StaleAnchor
    );
    context.anchor = projection.anchor();
    context
        .grants
        .retain(|grant| grant.capability != Capability::Query);
    assert_eq!(
        diagnose_production(
            &projection,
            &context,
            DiagnosisScope::default(),
            MAX_ANALYSIS_WORK,
        )
        .unwrap_err()
        .code,
        ErrorCode::CapabilityDenied
    );
    Ok(())
}

#[test]
fn heartbeat_does_not_invent_new_operations_identity_for_a_new_parent() -> Result<()> {
    let (first, context) = fixture()?;
    let heartbeat = first.refresh(first.parent_anchor(), observation(), &context)?;
    assert_eq!(heartbeat.anchor(), first.anchor());
    assert_eq!(heartbeat.source_digest(), first.source_digest());
    let parent = parent(&observation(), 42)?;
    let refreshed = first.refresh(parent, observation(), &super::tests::context(parent))?;
    assert_eq!(refreshed.anchor(), first.anchor());
    assert_eq!(refreshed.parent_anchor(), parent);
    assert_eq!(first.parent_anchor(), context.anchor);
    Ok(())
}

#[test]
fn same_tick_operations_changes_advance_only_their_own_source_cursor() -> Result<()> {
    let (first, context) = fixture()?;
    let mut next = observation();
    next.items[1].stack_size += 1;
    let refreshed = first.refresh(first.parent_anchor(), next, &context)?;
    assert_eq!(refreshed.parent_anchor(), first.parent_anchor());
    assert_eq!(
        refreshed.anchor().cursor.sequence,
        first.anchor().cursor.sequence + 1
    );
    assert_ne!(refreshed.anchor().state_hash, first.anchor().state_hash);
    assert_eq!(
        refreshed.snapshot()?.graph.entities[&item_entity_id(11)].generation,
        1
    );
    assert_eq!(first.operations()?.items[1].stack_size, 4);
    Ok(())
}

#[test]
fn retirement_and_reappearance_preserve_history_and_reject_old_handles() -> Result<()> {
    let (first, _) = fixture()?;
    let mut empty = observation();
    empty.jobs.jobs.clear();
    empty.buildings.clear();
    empty.items.clear();
    empty.attachments.clear();
    empty.jobs.year_tick += 1;
    let empty_parent = parent(&empty, 42)?;
    let retired = first.refresh(empty_parent, empty, &context(empty_parent))?;
    let mut next = observation();
    next.jobs.year_tick += 2;
    let next_parent = parent(&next, 43)?;
    let reappeared = retired.refresh(next_parent, next, &context(next_parent))?;
    for id in [EntityId::new(9), building_entity_id(3), item_entity_id(11)] {
        assert_eq!(first.snapshot()?.graph.entities[&id].generation, 1);
        assert!(!retired.snapshot()?.graph.entities.contains_key(&id));
        assert_eq!(reappeared.snapshot()?.graph.entities[&id].generation, 2);
    }
    let query = context(reappeared.anchor());
    assert_eq!(
        diagnose_production(
            &reappeared,
            &query,
            DiagnosisScope {
                job: Some((EntityId::new(9), 1)),
                ..DiagnosisScope::default()
            },
            MAX_ANALYSIS_WORK
        )
        .unwrap_err()
        .code,
        ErrorCode::Conflict
    );
    assert_eq!(
        diagnose_production(
            &reappeared,
            &query,
            DiagnosisScope {
                job: Some((EntityId::new(9), 2)),
                ..DiagnosisScope::default()
            },
            MAX_ANALYSIS_WORK
        )?
        .jobs_considered,
        1
    );
    assert_eq!(
        first.anchor().cursor.epoch,
        reappeared.anchor().cursor.epoch
    );
    Ok(())
}

#[test]
fn source_restart_version_drift_and_each_allocator_rewind_refuse() -> Result<()> {
    let (first, context) = fixture()?;
    for case in 0..8 {
        let mut next = observation();
        match case {
            0 => next.jobs.bridge_generation += 1,
            1 => next.jobs.df_version.push_str("-changed"),
            2 => next.jobs.dfhack_version.push_str("-changed"),
            3 => next.jobs.world_folder.push_str("-changed"),
            4 => next.jobs.site_id += 1,
            5 => next.jobs.next_job_id -= 1,
            6 => next.next_building_id -= 1,
            _ => next.next_item_id -= 1,
        }
        assert_eq!(
            first
                .refresh(first.parent_anchor(), next, &context)
                .unwrap_err()
                .code,
            ErrorCode::StaleAnchor,
            "case {case}"
        );
    }
    assert_eq!(
        first
            .refresh(first.parent_anchor(), observation(), &context)?
            .anchor(),
        first.anchor()
    );
    assert_eq!(first.operations()?, &observation());
    Ok(())
}

#[test]
fn canonical_parent_must_advance_without_replacing_or_rewinding_an_anchor() -> Result<()> {
    let (first, _) = fixture()?;
    for case in 0..5 {
        let mut next_parent = first.parent_anchor();
        let mut next = observation();
        match case {
            0 => next_parent.cursor.epoch += 1,
            1 => next_parent.cursor.sequence -= 1,
            2 => next_parent.state_hash = Digest32::of_bytes(b"replacement"),
            3 => {
                next.jobs.year_tick -= 1;
                next_parent.tick = next.jobs.tick();
                next_parent.cursor.sequence += 1;
            }
            _ => {
                next.jobs.year_tick += 1;
                next_parent.tick = next.jobs.tick();
            }
        }
        assert_eq!(
            first
                .refresh(next_parent, next, &context(next_parent))
                .unwrap_err()
                .code,
            ErrorCode::StaleAnchor,
            "case {case}"
        );
    }
    let mut other = context(first.parent_anchor());
    other.session_id = SessionId::new(18);
    assert_eq!(
        first
            .refresh(first.parent_anchor(), observation(), &other)
            .unwrap_err()
            .code,
        ErrorCode::CapabilityDenied
    );
    Ok(())
}

#[test]
fn acquisition_requires_one_exact_paused_fortress_and_complete_context() -> Result<()> {
    let operations = observation();
    let parent = parent(&operations, 41)?;
    let context = context(parent);
    for case in 0..4 {
        let mut next = operations.clone();
        let mut caller = context.clone();
        match case {
            0 => next.jobs.paused = false,
            1 => next.jobs.year_tick += 1,
            2 => next.jobs.site_id += 1,
            _ => caller.anchor.state_hash = Digest32::of_bytes(b"different-context"),
        }
        assert_eq!(
            OperationsInspectionProjection::new(parent, next, &caller)
                .unwrap_err()
                .code,
            ErrorCode::StaleAnchor,
            "case {case}"
        );
    }
    Ok(())
}

#[test]
fn whole_domain_query_and_observe_authority_is_required_without_plan_authority() -> Result<()> {
    let operations = observation();
    let parent = parent(&operations, 41)?;
    for case in 0..6 {
        let mut context = context(parent);
        match case {
            0 => context
                .grants
                .retain(|grant| grant.capability != Capability::Query),
            1 => context
                .grants
                .retain(|grant| grant.capability != Capability::Observe),
            2 => context.grants[1].expires_at_tick = Some(GameTick(parent.tick.get() - 1)),
            3 => context.grants[1].remaining_uses = Some(1),
            4 => {
                context.grants[1]
                    .scope
                    .entity_ids
                    .insert(item_entity_id(11));
            }
            _ => context.cancellation_requested = true,
        }
        let expected = if case == 5 {
            ErrorCode::CancellationRequested
        } else {
            ErrorCode::CapabilityDenied
        };
        assert_eq!(
            OperationsInspectionProjection::new(parent, operations.clone(), &context)
                .unwrap_err()
                .code,
            expected,
            "case {case}"
        );
    }
    assert!(OperationsInspectionProjection::new(parent, operations, &context(parent)).is_ok());
    Ok(())
}

#[test]
fn complete_source_and_work_bounds_are_independent_of_output_page_width() -> Result<()> {
    let operations = observation();
    let parent = parent(&operations, 41)?;
    let mut context = context(parent);
    context.budget.max_output_tokens = 1;
    assert!(OperationsInspectionProjection::new(parent, operations.clone(), &context).is_ok());
    context.budget.max_bytes -= 1;
    assert_eq!(
        OperationsInspectionProjection::new(parent, operations.clone(), &context)
            .unwrap_err()
            .code,
        ErrorCode::BudgetExceeded
    );
    context.budget.max_bytes += 1;
    context.budget.max_entities = 6;
    assert_eq!(
        OperationsInspectionProjection::new(parent, operations, &context)
            .unwrap_err()
            .code,
        ErrorCode::BudgetExceeded
    );
    Ok(())
}

#[test]
fn hard_projection_limits_cover_entities_and_all_relation_types() -> Result<()> {
    let mut operations = observation();
    operations.jobs.jobs.clear();
    operations.buildings.clear();
    operations.attachments.clear();
    operations.items = (0..OPERATIONS_INSPECTION_MAX_ENTITIES as u32)
        .map(|id| item(id, "WOOD", 1))
        .collect();
    operations.next_item_id = OPERATIONS_INSPECTION_MAX_ENTITIES as u32;
    let parent = parent(&operations, 41)?;
    let mut context = context(parent);
    context.budget.max_entities = 10_000;
    assert_eq!(
        OperationsInspectionProjection::new(parent, operations, &context)
            .unwrap_err()
            .code,
        ErrorCode::BudgetExceeded
    );

    let mut operations = observation();
    operations.jobs.jobs[0].attached_item_count = OPERATIONS_INSPECTION_MAX_EDGES as u32;
    operations.attachments = (0..OPERATIONS_INSPECTION_MAX_EDGES as i32)
        .map(|role| JobItemAttachment {
            job_native_id: 7,
            item_native_id: 10,
            role,
            filter_index: 0,
        })
        .collect();
    // Attachments fit their own ceiling; the observed holder adds another edge.
    assert_eq!(
        OperationsInspectionProjection::new(parent, operations, &context)
            .unwrap_err()
            .code,
        ErrorCode::BudgetExceeded
    );
    Ok(())
}

#[test]
fn malformed_successor_cannot_change_a_retained_inspection() -> Result<()> {
    let (first, context) = fixture()?;
    let expected = first.anchor();
    let mut next = observation();
    next.attachments[0].item_native_id = 99;
    assert_eq!(
        first
            .refresh(first.parent_anchor(), next, &context)
            .unwrap_err()
            .code,
        ErrorCode::AdapterRejected
    );
    assert_eq!(first.anchor(), expected);
    assert_eq!(first.operations()?, &observation());
    assert!(first.snapshot()?.hash_is_valid());
    assert_eq!(
        first
            .refresh(first.parent_anchor(), observation(), &context)?
            .anchor(),
        expected
    );
    Ok(())
}
