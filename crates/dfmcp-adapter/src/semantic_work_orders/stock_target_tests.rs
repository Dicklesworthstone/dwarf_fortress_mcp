use super::*;
use crate::live_jobs::{LiveJob, LiveJobObservation};
use crate::live_operations::{
    JobItemAttachment, LiveBuilding, LiveItem, LiveOperationsObservation,
};
use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, GameTick, MapCoord, ObservationCursor, RequestId,
    RiskTier, SessionId, WorkBudget,
};
use dfmcp_world::{EvidenceCoverage, EvidenceSource, WorldGraph, WorldSnapshot};

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
        items: rows
            .iter()
            .map(|(id, item_type, units)| LiveItem {
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
            })
            .collect(),
        attachments: Vec::new(),
    }
}

fn job(type_key: &str) -> LiveJob {
    LiveJob {
        native_id: 7,
        job_type: 13,
        type_key: type_key.into(),
        reaction: String::new(),
        suspended: false,
        repeating: false,
        position: MapCoord::new(1, 1, 1),
        worker_native_id: None,
        holder_native_id: None,
        completion_timer: -1,
        attached_item_count: 0,
        required_item_filter_count: 0,
    }
}

fn context(raw: &LiveOperationsObservation) -> Result<OperationContext> {
    let snapshot = WorldSnapshot::new(
        raw.jobs.fortress_id()?,
        raw.jobs.tick(),
        ObservationCursor {
            epoch: 2,
            sequence: 7,
        },
        true,
        WorldGraph::default(),
    );
    Ok(OperationContext {
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
        grants: [Capability::Query, Capability::Observe]
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
        cancellation_requested: false,
    })
}

fn accepted(projection: &InventoryBaselineProjection) -> EvidencePolicy {
    let mut policy = EvidencePolicy::at(projection.snapshot().anchor());
    policy.paused = true;
    policy
        .entity_kinds
        .insert(EntityKind::Item, EvidenceCoverage::Complete);
    policy.sources = projection.observed_sources();
    policy
}

fn fixture(rows: &[(u32, &str, u32)]) -> Result<(InventoryBaselineProjection, OperationContext)> {
    let capture = raw(rows);
    let context = context(&capture)?;
    let projection = InventoryBaselineProjection::new(context.anchor, capture, &context)?;
    Ok((projection, context))
}

fn bed(
    projection: &InventoryBaselineProjection,
    target: u64,
    maximum: u32,
    context: &OperationContext,
) -> Result<StockTargetAssessment> {
    projection.stock_target(
        WorkOrderRecipe::WoodenBed,
        target,
        maximum,
        accepted(projection),
        context,
    )
}

#[test]
fn stock_target_qualifies_exact_deficit_and_reuses_unchanged_increase_predicate() -> Result<()> {
    let (projection, c) = fixture(&[(1, "BED", 2), (2, "DOOR", 99), (3, "BED", 3)])?;
    let assessment = bed(&projection, 7, 8, &c)?;
    assert_eq!(assessment.recipe(), WorkOrderRecipe::WoodenBed);
    assert_eq!(assessment.item_type(), "BED");
    assert_eq!(assessment.baseline_stack_units(), 5);
    assert_eq!(assessment.target_stack_units(), 7);
    assert_eq!(assessment.deficit_stack_units(), 2);
    assert_eq!(assessment.max_order_amount(), 8);
    assert_eq!(assessment.amount(), Some(2));
    assert_eq!(assessment.item_rows(), 3);
    assert_eq!(assessment.original_anchor(), c.anchor);
    assert_eq!(
        assessment.source_digest(),
        projection.operations().source_digest()?
    );
    assert_eq!(assessment.source_generation(), 601);
    assert_eq!(assessment.source_counters(), [100, 200, 300]);
    let unchanged = projection.increase("BED", 2, accepted(&projection), &c)?;
    assert_eq!(assessment.objective(), Some(&unchanged));
    assert_eq!(
        assessment.witness(),
        Digest32::of_bytes(assessment.canonical_bytes())
    );
    assert_ne!(assessment.witness(), Digest32::ZERO);
    Ok(())
}

#[test]
fn stock_target_supports_the_closed_four_recipe_catalog() -> Result<()> {
    let (projection, c) = fixture(&[
        (1, "BED", 2),
        (2, "DOOR", 3),
        (3, "TABLE", 4),
        (4, "CHAIR", 5),
        (5, "WOOD", 99),
    ])?;
    for (recipe, item_type, baseline) in [
        (WorkOrderRecipe::WoodenBed, "BED", 2),
        (WorkOrderRecipe::WoodenDoor, "DOOR", 3),
        (WorkOrderRecipe::WoodenTable, "TABLE", 4),
        (WorkOrderRecipe::WoodenChair, "CHAIR", 5),
    ] {
        let assessment =
            projection.stock_target(recipe, baseline + 2, 2, accepted(&projection), &c)?;
        assert_eq!(assessment.item_type(), item_type);
        assert_eq!(assessment.baseline_stack_units(), baseline);
        assert_eq!(assessment.amount(), Some(2));
        let objective = assessment.objective().ok_or_else(|| {
            error(
                ErrorCode::InternalInvariantViolation,
                "missing fixture increase",
            )
        })?;
        assert_eq!(objective.item_type(), item_type);
        assert_eq!(objective.target_stack_units(), baseline + 2);
    }
    Ok(())
}

#[test]
fn stock_target_satisfied_stock_returns_no_order_even_with_outstanding_producers() -> Result<()> {
    let mut capture = raw(&[(1, "BED", 7)]);
    capture.jobs.jobs.push(job("CustomReaction"));
    let c = context(&capture)?;
    let projection = InventoryBaselineProjection::new(c.anchor, capture, &c)?;
    for target in [1, 7] {
        let assessment = bed(&projection, target, 1, &c)?;
        assert_eq!(assessment.baseline_stack_units(), 7);
        assert_eq!(assessment.target_stack_units(), target);
        assert_eq!(assessment.deficit_stack_units(), 0);
        assert_eq!(assessment.amount(), None);
        assert_eq!(assessment.objective(), None);
        assessment.revalidate(&projection, accepted(&projection), &c)?;
    }
    assert_ne!(
        bed(&projection, 1, 1, &c)?.witness(),
        bed(&projection, 7, 1, &c)?.witness()
    );
    Ok(())
}

#[test]
fn stock_target_complete_empty_census_is_zero_but_unknown_is_not() -> Result<()> {
    let (projection, c) = fixture(&[])?;
    let assessment = bed(&projection, 5, 5, &c)?;
    assert_eq!(assessment.baseline_stack_units(), 0);
    assert_eq!(assessment.item_rows(), 0);
    assert_eq!(assessment.amount(), Some(5));
    assert!(
        projection
            .stock_target(
                WorkOrderRecipe::WoodenBed,
                5,
                5,
                EvidencePolicy::at(projection.snapshot().anchor()),
                &c,
            )
            .is_err()
    );
    Ok(())
}

#[test]
fn stock_target_never_clamps_deficit_or_accepts_out_of_range_limits() -> Result<()> {
    let (projection, c) = fixture(&[])?;
    for (target, maximum) in [
        (0, 1),
        (1_000_001, 100),
        (u64::MAX, 100),
        (1, 0),
        (1, 101),
        (1, u32::MAX),
    ] {
        let error = bed(&projection, target, maximum, &c).err().ok_or_else(|| {
            error(
                ErrorCode::InternalInvariantViolation,
                "invalid fixture target accepted",
            )
        })?;
        assert_eq!(error.code, ErrorCode::InvalidRequest);
    }
    for (target, maximum) in [(2, 1), (101, 100), (1_000_000, 100)] {
        let error = bed(&projection, target, maximum, &c).err().ok_or_else(|| {
            error(
                ErrorCode::InternalInvariantViolation,
                "oversized fixture deficit accepted",
            )
        })?;
        assert_eq!(error.code, ErrorCode::Conflict);
    }
    let (projection, c) = fixture(&[(1, "BED", 999_900)])?;
    let assessment = bed(&projection, 1_000_000, 100, &c)?;
    assert_eq!(assessment.amount(), Some(100));
    assert_eq!(assessment.target_stack_units(), 1_000_000);
    Ok(())
}

#[test]
fn stock_target_requires_complete_independent_type_and_quantity_policy() -> Result<()> {
    let (projection, c) = fixture(&[(1, "BED", 2), (2, "DOOR", 99)])?;
    for case in 0..8 {
        let mut policy = accepted(&projection);
        match case {
            0 => policy.entity_kinds.clear(),
            1 => { policy.entity_kinds.insert(EntityKind::Item, EvidenceCoverage::Observed); }
            2 => policy.sources.retain(|source| !matches!(source, EvidenceSource::Observed { field, .. } if field.ends_with("type_key"))),
            3 => policy.sources.retain(|source| !matches!(source, EvidenceSource::Observed { field, .. } if field.ends_with("getStackSize"))),
            4 => policy.all_entities = EvidenceCoverage::Complete,
            5 => { policy.entity_kinds.insert(EntityKind::Job, EvidenceCoverage::Complete); }
            6 => { policy.sources.insert(EvidenceSource::Observed { field: "operations/1.3.item.type_key".into(), source_digest: Digest32::of_bytes(b"unqualified source") }); }
            _ => { policy.anchor = None; }
        }
        assert!(
            projection
                .stock_target(WorkOrderRecipe::WoodenBed, 3, 5, policy, &c)
                .is_err(),
            "policy {case}"
        );
    }
    Ok(())
}

#[test]
fn stock_target_authority_is_current_read_authority_not_plan_or_control() -> Result<()> {
    let (projection, c) = fixture(&[(1, "BED", 2)])?;
    // The fixture intentionally has only fortress-scoped Query and Observe.
    let original = bed(&projection, 3, 5, &c)?;
    for case in 0..7 {
        let mut changed = c.clone();
        match case {
            0 => changed
                .grants
                .retain(|grant| grant.capability != Capability::Query),
            1 => changed
                .grants
                .retain(|grant| grant.capability != Capability::Observe),
            2 => changed.cancellation_requested = true,
            3 => {
                for grant in &mut changed.grants {
                    grant.expires_at_tick = Some(GameTick(119));
                }
            }
            4 => changed.anchor.cursor.sequence += 1,
            5 => changed.anchor.state_hash = Digest32::of_bytes(b"another canonical observation"),
            _ => {
                for grant in &mut changed.grants {
                    grant.scope.fortress_id = Some(dfmcp_core::FortressId::new(1));
                }
            }
        }
        assert!(
            bed(&projection, 3, 5, &changed).is_err(),
            "authority {case}"
        );
        assert!(
            original
                .revalidate(&projection, accepted(&projection), &changed)
                .is_err(),
            "revalidation authority {case}"
        );
    }
    Ok(())
}

#[test]
fn stock_target_qualification_admits_complete_work_before_analysis() -> Result<()> {
    let (projection, c) = fixture(&[(1, "BED", 2), (2, "DOOR", 99)])?;
    let original = bed(&projection, 3, 5, &c)?;
    for case in 0..3 {
        let mut changed = c.clone();
        match case {
            0 => changed.budget.max_bytes = INVENTORY_BASELINE_QUALIFY_BYTES - 1,
            1 => {
                changed.budget.max_entities = projection.snapshot().graph.entities.len() as u32 - 1
            }
            _ => changed.budget.max_wall_millis = 0,
        }
        assert!(bed(&projection, 3, 5, &changed).is_err(), "budget {case}");
        assert!(
            original
                .revalidate(&projection, accepted(&projection), &changed)
                .is_err(),
            "revalidation budget {case}"
        );
    }
    Ok(())
}

#[test]
fn stock_target_malformed_or_partial_native_captures_cannot_supply_a_baseline() -> Result<()> {
    let good = raw(&[(1, "BED", 2), (2, "DOOR", 99)]);
    let c = context(&good)?;
    for case in 0..7 {
        let mut changed = good.clone();
        match case {
            0 => changed.items[1].native_id = changed.items[0].native_id,
            1 => changed.items[0].stack_size = u32::MAX,
            2 => changed.items[0].flags = u32::MAX,
            3 => changed.items[0].type_key.clear(),
            4 => changed.items[0].container_native_id = Some(8),
            5 => changed.jobs.paused = false,
            _ => changed.jobs.bridge_generation = u64::MAX,
        }
        assert!(
            InventoryBaselineProjection::new(c.anchor, changed, &c).is_err(),
            "source {case}"
        );
    }
    Ok(())
}

#[test]
fn stock_target_blocks_matching_jobs_even_suspended_repeating_or_unassigned() -> Result<()> {
    for (recipe, type_key) in [
        (WorkOrderRecipe::WoodenBed, "ConstructBed"),
        (WorkOrderRecipe::WoodenDoor, "ConstructDoor"),
        (WorkOrderRecipe::WoodenTable, "ConstructTable"),
        (WorkOrderRecipe::WoodenChair, "ConstructThrone"),
    ] {
        for (suspended, repeating) in [(false, false), (true, false), (false, true), (true, true)] {
            let mut capture = raw(&[]);
            let mut outstanding = job(type_key);
            outstanding.suspended = suspended;
            outstanding.repeating = repeating;
            capture.jobs.jobs.push(outstanding);
            let c = context(&capture)?;
            let projection = InventoryBaselineProjection::new(c.anchor, capture, &c)?;
            let result = projection.stock_target(recipe, 1, 1, accepted(&projection), &c);
            assert!(
                result.is_err(),
                "{recipe:?}, suspended={suspended}, repeating={repeating}"
            );
        }
    }
    Ok(())
}

#[test]
fn stock_target_refuses_unknown_reaction_outputs_but_allows_unrelated_known_jobs() -> Result<()> {
    for (type_key, reaction, blocked) in [
        ("CustomReaction", "", true),
        ("Haul", "MAKE_FURNITURE", true),
        ("ConstructDoor", "", false),
        ("Haul", "", false),
    ] {
        let mut capture = raw(&[]);
        let mut outstanding = job(type_key);
        outstanding.reaction = reaction.into();
        capture.jobs.jobs.push(outstanding);
        let c = context(&capture)?;
        let projection = InventoryBaselineProjection::new(c.anchor, capture, &c)?;
        assert_eq!(
            bed(&projection, 1, 1, &c).is_err(),
            blocked,
            "{type_key}/{reaction}"
        );
    }
    Ok(())
}

#[test]
fn stock_target_counts_observed_units_without_material_or_usability_claims() -> Result<()> {
    let mut capture = raw(&[(1, "BED", 2), (2, "BED", 3), (3, "DOOR", 50)]);
    capture.items[0].flags = 1 | 8 | 16;
    capture.items[1].material_type = 0;
    capture.items[1].material_index = -1;
    let c = context(&capture)?;
    let projection = InventoryBaselineProjection::new(c.anchor, capture, &c)?;
    let assessment = bed(&projection, 6, 1, &c)?;
    assert_eq!(assessment.baseline_stack_units(), 5);
    assert_eq!(assessment.amount(), Some(1));
    Ok(())
}

#[test]
fn stock_target_exact_fresh_revalidation_reissues_policy_without_renewing_intent() -> Result<()> {
    let (projection, c) = fixture(&[(1, "BED", 2)])?;
    let original = bed(&projection, 4, 3, &c)?;
    let fresh = InventoryBaselineProjection::new(c.anchor, projection.operations().clone(), &c)?;
    assert_eq!(projection.source_digest(), fresh.source_digest());
    original.revalidate(&fresh, accepted(&fresh), &c)?;
    let mut unknown = accepted(&fresh);
    unknown.sources.clear();
    assert!(original.revalidate(&fresh, unknown, &c).is_err());
    original.revalidate(&fresh, accepted(&fresh), &c)?;
    assert_eq!(original, bed(&fresh, 4, 3, &c)?);
    assert_ne!(original.witness(), bed(&fresh, 4, 4, &c)?.witness());
    assert_ne!(original.witness(), bed(&fresh, 3, 3, &c)?.witness());
    Ok(())
}

#[test]
fn stock_target_changed_matching_stock_refuses_instead_of_resizing_or_returning_noop() -> Result<()>
{
    let (projection, c) = fixture(&[(1, "BED", 2)])?;
    let original = bed(&projection, 4, 3, &c)?;
    for units in [0, 1, 3, 4, 5] {
        let mut capture = projection.operations().clone();
        capture.items[0].stack_size = units;
        let fresh = InventoryBaselineProjection::new(c.anchor, capture, &c)?;
        assert!(
            original.revalidate(&fresh, accepted(&fresh), &c).is_err(),
            "units {units}"
        );
    }
    assert_eq!(original.baseline_stack_units(), 2);
    assert_eq!(original.target_stack_units(), 4);
    assert_eq!(original.amount(), Some(2));
    Ok(())
}

#[test]
fn stock_target_unrelated_rows_remain_part_of_the_original_capture_witness() -> Result<()> {
    let (projection, c) = fixture(&[(1, "BED", 2), (2, "DOOR", 50)])?;
    let original = bed(&projection, 4, 3, &c)?;
    for case in 0..6 {
        let mut capture = projection.operations().clone();
        match case {
            0 => capture.items[1].stack_size += 1,
            1 => capture.items[1].flags ^= 1,
            2 => capture.items[1].raw_position.x += 1,
            3 => capture.jobs.jobs.push(job("Haul")),
            4 => {
                capture.items.pop();
            }
            _ => capture.items[0].material_index += 1,
        }
        let fresh = InventoryBaselineProjection::new(c.anchor, capture, &c)?;
        let new_assessment = bed(&fresh, 4, 3, &c)?;
        assert_eq!(
            new_assessment.baseline_stack_units(),
            original.baseline_stack_units()
        );
        assert_ne!(
            new_assessment.witness(),
            original.witness(),
            "identity {case}"
        );
        assert!(
            original.revalidate(&fresh, accepted(&fresh), &c).is_err(),
            "capture {case}"
        );
    }
    Ok(())
}

#[test]
fn stock_target_source_seed_and_each_allocator_cannot_be_replaced_or_rewound() -> Result<()> {
    let (projection, c) = fixture(&[(1, "BED", 2)])?;
    let original = bed(&projection, 4, 3, &c)?;
    for case in 0..10 {
        let mut capture = projection.operations().clone();
        match case {
            0 => capture.jobs.bridge_generation += 1,
            1 => capture.jobs.bridge_generation -= 1,
            2 => capture.jobs.next_job_id += 1,
            3 => capture.jobs.next_job_id -= 1,
            4 => capture.next_building_id += 1,
            5 => capture.next_building_id -= 1,
            6 => capture.next_item_id += 1,
            7 => capture.next_item_id -= 1,
            8 => capture.jobs.df_version.push('x'),
            _ => capture.jobs.dfhack_version.push('x'),
        }
        let fresh = InventoryBaselineProjection::new(c.anchor, capture, &c)?;
        assert!(
            original.revalidate(&fresh, accepted(&fresh), &c).is_err(),
            "source floor {case}"
        );
        assert_ne!(
            original.witness(),
            bed(&fresh, 4, 3, &c)?.witness(),
            "source identity {case}"
        );
    }
    Ok(())
}

#[test]
fn stock_target_original_canonical_anchor_and_paused_tick_are_not_replaceable() -> Result<()> {
    let (projection, c) = fixture(&[(1, "BED", 2)])?;
    let original = bed(&projection, 4, 3, &c)?;
    for case in 0..5 {
        let mut capture = projection.operations().clone();
        let mut changed = c.clone();
        match case {
            0 => changed.anchor.cursor.epoch += 1,
            1 => changed.anchor.cursor.sequence += 1,
            2 => {
                changed.anchor.state_hash =
                    Digest32::of_bytes(b"different original canonical state")
            }
            3 => {
                capture.jobs.year_tick += 1;
                changed = context(&capture)?;
            }
            _ => {
                capture.jobs.year_tick -= 1;
                changed = context(&capture)?;
            }
        }
        let fresh = InventoryBaselineProjection::new(changed.anchor, capture, &changed)?;
        assert!(
            original.revalidate(&fresh, accepted(&fresh), &c).is_err(),
            "anchor {case}"
        );
        assert_ne!(
            original.witness(),
            bed(&fresh, 4, 3, &changed)?.witness(),
            "anchor identity {case}"
        );
    }
    Ok(())
}

#[test]
fn stock_target_complete_census_caps_all_projected_edges_before_publication() -> Result<()> {
    let mut capture = raw(&[(1, "BED", 2), (2, "BOX", 1)]);
    let mut hauling = job("Haul");
    hauling.holder_native_id = Some(10);
    hauling.attached_item_count = 8190;
    capture.jobs.jobs.push(hauling);
    capture.buildings.push(LiveBuilding {
        native_id: 10,
        building_type: 0,
        type_key: "Workshop".into(),
        x1: 1,
        y1: 1,
        x2: 3,
        y2: 3,
        z: 1,
        build_stage: 1,
        max_build_stage: 1,
    });
    capture.items[0].container_native_id = Some(2);
    capture.items[0].holder_building_native_id = Some(10);
    capture.attachments = (0..8190)
        .map(|role| JobItemAttachment {
            job_native_id: 7,
            item_native_id: 1,
            role,
            filter_index: -1,
        })
        .collect();
    // All native rows are valid and the attachment count alone is below8192.
    // Three holder/container relations make the actual graph8193 edges.
    capture.validate()?;
    let c = context(&capture)?;
    let refusal = InventoryBaselineProjection::new(c.anchor, capture.clone(), &c)
        .err()
        .ok_or_else(|| {
            error(
                ErrorCode::InternalInvariantViolation,
                "oversized edge census accepted",
            )
        })?;
    assert_eq!(refusal.code, ErrorCode::BudgetExceeded);

    capture.attachments.pop();
    capture.jobs.jobs[0].attached_item_count -= 1;
    let projection = InventoryBaselineProjection::new(c.anchor, capture, &c)?;
    assert_eq!(projection.snapshot().graph.edges.len(), 8192);
    assert_eq!(bed(&projection, 3, 1, &c)?.amount(), Some(1));
    Ok(())
}
