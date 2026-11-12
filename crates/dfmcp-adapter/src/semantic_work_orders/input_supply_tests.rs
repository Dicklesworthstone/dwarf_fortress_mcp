use super::*;
use crate::live_jobs::{LiveJob, LiveJobObservation};
use crate::live_operations::{JobItemAttachment, LiveBuilding, LiveItem, item_entity_id};
use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, GameTick, MapCoord, ObservationCursor, RequestId,
    RiskTier, SessionId, WorkBudget,
};

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
        next_item_id: 5000,
        buildings: Vec::new(),
        items: rows
            .iter()
            .map(|(id, kind, units)| LiveItem {
                native_id: *id,
                item_type: 5,
                type_key: (*kind).to_owned(),
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

fn source() -> Result<InputSupplySource> {
    InputSupplySource::new(
        "region1".into(),
        4,
        "0.51.11".into(),
        "0.51.11-r1".into(),
        601,
    )
}

fn context(raw: &LiveOperationsObservation) -> Result<OperationContext> {
    let anchor = StateAnchor {
        fortress_id: raw.jobs.fortress_id()?,
        cursor: ObservationCursor {
            epoch: 2,
            sequence: 7,
        },
        tick: raw.jobs.tick(),
        state_hash: Digest32::of_bytes(b"original citizen observation"),
    };
    Ok(OperationContext {
        session_id: SessionId::new(1),
        request_id: RequestId::new(1),
        anchor,
        budget: WorkBudget {
            max_wall_millis: 10_000,
            max_bytes: INPUT_SUPPLY_QUALIFY_BYTES,
            max_output_tokens: 8192,
            max_entities: INPUT_SUPPLY_MAX_ENTITIES as u32,
            max_actions: 1,
            max_game_ticks: 0,
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
    })
}

fn requirements(
    orders: &[(WorkOrderRecipe, u32)],
    reserve: u64,
) -> Result<FurnitureInputRequirements> {
    FurnitureInputRequirements::new(
        &orders
            .iter()
            .map(|(recipe, amount)| WorkOrderSpec::new(*recipe, *amount))
            .collect::<Result<Vec<_>>>()?,
        reserve,
    )
}

fn assess(
    raw: &LiveOperationsObservation,
    requirements: &FurnitureInputRequirements,
) -> Result<InputSupplyAssessment> {
    let c = context(raw)?;
    InputSupplyAssessment::qualify(requirements, &source()?, c.anchor, raw, &c)
}

fn job() -> LiveJob {
    LiveJob {
        native_id: 7,
        job_type: 13,
        type_key: "Haul".into(),
        reaction: String::new(),
        suspended: true,
        repeating: false,
        position: MapCoord::new(1, 1, 1),
        worker_native_id: None,
        holder_native_id: None,
        completion_timer: -1,
        attached_item_count: 0,
        required_item_filter_count: 1,
    }
}

fn building() -> LiveBuilding {
    LiveBuilding {
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
    }
}

#[test]
fn input_supply_allocates_all_original_orders_jointly_without_reusing_wood() -> Result<()> {
    let capture = raw(&[(1, "WOOD", 6), (2, "BOULDER", 100)]);
    let orders = [
        (WorkOrderRecipe::WoodenBed, 3),
        (WorkOrderRecipe::WoodenDoor, 4),
    ];
    for order in orders {
        assert!(assess(&capture, &requirements(&[order], 0)?)?.ready());
    }
    let demand = requirements(&orders, 0)?;
    let assessment = assess(&capture, &demand)?;
    assert!(!assessment.ready());
    assert_eq!(assessment.required_stack_units(), 7);
    assert_eq!(assessment.eligible_stack_units(), 6);
    assert_eq!(assessment.shortfall_stack_units(), 1);
    assert_eq!(assessment.eligible_items(), 1);
    assert_eq!(assessment.unmatched_items(), 1);
    assert_eq!(assessment.allocation().allocated_units, 6);
    assert_eq!(assessment.allocation().requested_units, 7);
    assert_eq!(assessment.allocation().cut_capacity, 6);
    let allocated: u64 = assessment
        .allocation()
        .assignments
        .iter()
        .map(|row| {
            assert_eq!(row.supply_id, item_entity_id(1).get());
            row.units
        })
        .sum();
    assert_eq!(allocated, 6);
    assert_eq!(assessment.demands().len(), 2);
    assert_eq!(assessment.requirements(), &demand);
    assert_eq!(assessment.source_generation(), 601);
    assert_eq!(assessment.source_counters(), [100, 200, 5000]);
    assert_eq!(assessment.item_rows(), 2);
    assert_eq!(assessment.source_digest(), capture.source_digest()?);
    assert_eq!(assessment.source_witness(), source()?.witness());
    assert_eq!(assessment.original_anchor(), context(&capture)?.anchor);
    assert_eq!(
        assessment.witness(),
        Digest32::of_bytes(assessment.canonical_bytes())
    );
    assert!(assessment.work_units() > 0);
    Ok(())
}

#[test]
fn input_supply_retained_reserve_competes_with_every_original_order() -> Result<()> {
    let demand = requirements(&[(WorkOrderRecipe::WoodenBed, 2)], 3)?;
    let shortage = assess(&raw(&[(1, "WOOD", 4)]), &demand)?;
    assert_eq!(demand.order_stack_units(), 2);
    assert_eq!(demand.reserve_stack_units(), 3);
    assert_eq!(demand.required_stack_units(), 5);
    assert!(!shortage.ready());
    assert_eq!(shortage.shortfall_stack_units(), 1);
    assert_eq!(shortage.demands()[0].key, "retained_wood_reserve");
    assert_eq!(shortage.demands()[1].key, "wooden_bed");
    let ready = assess(&raw(&[(1, "WOOD", 5)]), &demand)?;
    assert!(ready.ready());
    assert_eq!(ready.shortfall_stack_units(), 0);
    assert_eq!(ready.allocation().allocated_by_demand, [3, 2]);
    assert_ne!(ready.witness(), shortage.witness());
    Ok(())
}

#[test]
fn input_supply_requirements_are_closed_bounded_canonical_and_complete() -> Result<()> {
    let all = [
        (WorkOrderRecipe::WoodenBed, 100),
        (WorkOrderRecipe::WoodenDoor, 100),
        (WorkOrderRecipe::WoodenTable, 100),
        (WorkOrderRecipe::WoodenChair, 100),
    ];
    let first = requirements(&all, INPUT_SUPPLY_MAX_RESERVE_STACK_UNITS)?;
    let mut reverse = all;
    reverse.reverse();
    assert_eq!(
        first,
        requirements(&reverse, INPUT_SUPPLY_MAX_RESERVE_STACK_UNITS)?
    );
    assert_eq!(first.orders().len(), 4);
    assert_eq!(first.required_stack_units(), 1_000_400);
    assert_eq!(first.witness(), Digest32::of_bytes(first.canonical_bytes()));
    assert_ne!(first.witness(), requirements(&all, 999_999)?.witness());
    let mut changed = all;
    changed[0].1 = 99;
    assert_ne!(
        first.witness(),
        requirements(&changed, 1_000_000)?.witness()
    );
    assert!(requirements(&[], 0).is_err());
    assert!(requirements(&[all[0]; 5], 0).is_err());
    assert!(requirements(&[all[0]; 2], 0).is_err());
    for reserve in [1_000_001, u64::MAX] {
        assert!(requirements(&all, reserve).is_err());
    }
    assert!(WorkOrderSpec::new(WorkOrderRecipe::WoodenBed, 0).is_err());
    assert!(WorkOrderSpec::new(WorkOrderRecipe::WoodenBed, 101).is_err());
    Ok(())
}

#[test]
fn input_supply_counts_empty_and_large_captures_without_saturating_or_graph_projection()
-> Result<()> {
    let demand = requirements(&[(WorkOrderRecipe::WoodenChair, 100)], 1_000_000)?;
    let absent = assess(&raw(&[]), &demand)?;
    assert!(!absent.ready());
    assert_eq!(absent.eligible_items(), 0);
    assert_eq!(absent.eligible_stack_units(), 0);
    assert_eq!(absent.shortfall_stack_units(), 1_000_100);
    let large = assess(
        &raw(&[(1, "WOOD", i32::MAX as u32), (2, "WOOD", i32::MAX as u32)]),
        &demand,
    )?;
    assert!(large.ready());
    assert_eq!(large.eligible_stack_units(), 2 * i32::MAX as u64);
    assert_eq!(large.allocation().allocated_units, 1_000_100);
    // The raw assessment fits its 16 MiB admission; no 128 MiB projection is needed.
    assert!(INPUT_SUPPLY_QUALIFY_BYTES < super::super::INVENTORY_BASELINE_PROJECTION_BYTES);
    Ok(())
}

#[test]
fn input_supply_excludes_every_existing_conservative_flag_and_explicit_holder() -> Result<()> {
    let demand = requirements(&[(WorkOrderRecipe::WoodenBed, 1)], 0)?;
    for (flag, reason) in [
        (1, "forbidden_in_chain"),
        (2, "in_job_in_chain"),
        (4, "dump_in_chain"),
        (8, "removed_in_chain"),
        (16, "rotten_in_chain"),
        (32, "trader_in_chain"),
        (128, "in_inventory_in_chain"),
        (256, "in_building_in_chain"),
    ] {
        let mut capture = raw(&[(1, "WOOD", 100)]);
        capture.items[0].flags |= flag;
        let assessment = assess(&capture, &demand)?;
        assert!(!assessment.ready(), "flag {flag}");
        assert_eq!(assessment.eligible_stack_units(), 0);
        assert_eq!(assessment.excluded_items().get(reason), Some(&1));
    }
    let mut held = raw(&[(1, "WOOD", 100)]);
    held.buildings.push(building());
    held.items[0].holder_building_native_id = Some(10);
    let assessment = assess(&held, &demand)?;
    assert!(!assessment.ready());
    assert_eq!(
        assessment.excluded_items().get("building_holder_in_chain"),
        Some(&1)
    );
    let empty = assess(&raw(&[(1, "WOOD", 0)]), &demand)?;
    assert_eq!(empty.excluded_items().get("zero_stack_size"), Some(&1));
    Ok(())
}

#[test]
fn input_supply_inherits_flags_and_actual_job_attachments_through_containers() -> Result<()> {
    let demand = requirements(&[(WorkOrderRecipe::WoodenBed, 2)], 0)?;
    let mut capture = raw(&[(1, "WOOD", 8), (2, "BOX", 1), (3, "BARREL", 1)]);
    capture.items[0].container_native_id = Some(2);
    capture.items[1].container_native_id = Some(3);
    assert!(assess(&capture, &demand)?.ready());
    capture.items[2].flags |= 1;
    let forbidden = assess(&capture, &demand)?;
    assert!(!forbidden.ready());
    assert_eq!(
        forbidden.excluded_items().get("forbidden_in_chain"),
        Some(&3)
    );
    capture.items[2].flags = 64;
    let mut attached = job();
    attached.attached_item_count = 1;
    capture.jobs.jobs.push(attached);
    capture.attachments.push(JobItemAttachment {
        job_native_id: 7,
        item_native_id: 3,
        role: 0,
        filter_index: 0,
    });
    let reserved = assess(&capture, &demand)?;
    assert!(!reserved.ready());
    assert_eq!(reserved.eligible_stack_units(), 0);
    assert_eq!(
        reserved.excluded_items().get("attached_to_job_in_chain"),
        Some(&3)
    );
    // Clearing a diagnostic in_job bit never hides an actual native attachment.
    assert!(capture.items.iter().all(|item| item.flags == 64));
    Ok(())
}

#[test]
fn input_supply_rejects_missing_relations_duplicate_rosters_cycles_and_invalid_flags() -> Result<()>
{
    let good = raw(&[(1, "WOOD", 2), (2, "BOX", 1)]);
    let demand = requirements(&[(WorkOrderRecipe::WoodenBed, 1)], 0)?;
    for case in 0..10 {
        let mut capture = good.clone();
        match case {
            0 => capture.items[1].native_id = 1,
            1 => capture.items.reverse(),
            2 => capture.items[0].container_native_id = Some(99),
            3 => {
                capture.items[0].container_native_id = Some(2);
                capture.items[1].container_native_id = Some(1);
            }
            4 => capture.items[0].holder_building_native_id = Some(10),
            5 => capture.items[0].flags |= 512,
            6 => capture.items[0].stack_size = u32::MAX,
            7 => capture.items[0].type_key.clear(),
            8 => {
                let mut attached = job();
                attached.attached_item_count = 1;
                capture.jobs.jobs.push(attached);
            }
            _ => capture.attachments.push(JobItemAttachment {
                job_native_id: 7,
                item_native_id: 1,
                role: 0,
                filter_index: 0,
            }),
        }
        assert!(assess(&capture, &demand).is_err(), "malformed {case}");
    }
    Ok(())
}

#[test]
fn input_supply_independent_source_pins_cannot_be_issued_from_matching_counts() -> Result<()> {
    let good = raw(&[(1, "WOOD", 8)]);
    let c = context(&good)?;
    let policy = source()?;
    let demand = requirements(&[(WorkOrderRecipe::WoodenBed, 1)], 0)?;
    for case in 0..7 {
        let mut capture = good.clone();
        match case {
            0 => capture.jobs.world_folder.push('x'),
            1 => capture.jobs.site_id += 1,
            2 => capture.jobs.df_version.push('x'),
            3 => capture.jobs.dfhack_version.push('x'),
            4 => capture.jobs.bridge_generation += 1,
            5 => capture.jobs.bridge_generation = 0,
            _ => capture.jobs.bridge_generation = u64::MAX,
        }
        assert!(!policy.matches_capture_identity(&capture));
        assert!(InputSupplyAssessment::qualify(&demand, &policy, c.anchor, &capture, &c).is_err());
    }
    for generation in [0, u64::MAX] {
        assert!(
            InputSupplySource::new(
                "region1".into(),
                4,
                "df".into(),
                "dfhack".into(),
                generation
            )
            .is_err()
        );
    }
    assert!(InputSupplySource::new("".into(), 4, "df".into(), "dfhack".into(), 1).is_err());
    assert!(
        InputSupplySource::new("region1".into(), u32::MAX, "df".into(), "dfhack".into(), 1)
            .is_err()
    );
    assert!(
        InputSupplySource::new("region1".into(), 4, "df\0".into(), "dfhack".into(), 1).is_err()
    );
    Ok(())
}

#[test]
fn input_supply_requires_current_query_observe_and_exact_paused_original_anchor() -> Result<()> {
    let capture = raw(&[(1, "WOOD", 8)]);
    let c = context(&capture)?;
    let policy = source()?;
    let demand = requirements(&[(WorkOrderRecipe::WoodenBed, 1)], 0)?;
    // Read-only grants suffice to assess; there is deliberately no Plan/effect grant.
    assert!(InputSupplyAssessment::qualify(&demand, &policy, c.anchor, &capture, &c)?.ready());
    for case in 0..9 {
        let mut current = c.clone();
        match case {
            0 => current
                .grants
                .retain(|grant| grant.capability != Capability::Query),
            1 => current
                .grants
                .retain(|grant| grant.capability != Capability::Observe),
            2 => current.cancellation_requested = true,
            3 => current
                .grants
                .iter_mut()
                .for_each(|grant| grant.expires_at_tick = Some(GameTick(119))),
            4 => current.anchor.cursor.epoch += 1,
            5 => current.anchor.cursor.sequence += 1,
            6 => current.anchor.state_hash = Digest32::of_bytes(b"another parent"),
            7 => current.anchor.tick.0 += 1,
            _ => current
                .grants
                .iter_mut()
                .for_each(|grant| grant.scope.fortress_id = Some(dfmcp_core::FortressId::new(1))),
        }
        assert!(
            InputSupplyAssessment::qualify(&demand, &policy, c.anchor, &capture, &current).is_err(),
            "authority {case}"
        );
    }
    for case in 0..3 {
        let mut changed = capture.clone();
        match case {
            0 => changed.jobs.paused = false,
            1 => changed.jobs.year_tick += 1,
            _ => changed.jobs.year_tick -= 1,
        }
        assert!(policy.matches_capture_identity(&changed));
        assert!(
            InputSupplyAssessment::qualify(&demand, &policy, c.anchor, &changed, &c).is_err(),
            "tick {case}"
        );
    }
    Ok(())
}

#[test]
fn input_supply_admits_whole_census_work_before_validation_and_checks_all_edges() -> Result<()> {
    let mut capture = raw(&[(1, "WOOD", 8), (2, "BOX", 1)]);
    let c = context(&capture)?;
    let policy = source()?;
    let demand = requirements(&[(WorkOrderRecipe::WoodenBed, 1)], 0)?;
    for case in 0..3 {
        let mut small = c.clone();
        match case {
            0 => small.budget.max_bytes = INPUT_SUPPLY_QUALIFY_BYTES - 1,
            1 => small.budget.max_entities = 3,
            _ => small.budget.max_wall_millis = 0,
        }
        assert!(
            InputSupplyAssessment::qualify(&demand, &policy, c.anchor, &capture, &small).is_err(),
            "budget {case}"
        );
    }
    let mut attached = job();
    attached.attached_item_count = 8190;
    attached.holder_native_id = Some(10);
    capture.jobs.jobs.push(attached);
    capture.buildings.push(building());
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
    capture.validate()?;
    assert!(InputSupplyAssessment::qualify(&demand, &policy, c.anchor, &capture, &c).is_err());
    capture.attachments.pop();
    capture.jobs.jobs[0].attached_item_count -= 1;
    let accepted = InputSupplyAssessment::qualify(&demand, &policy, c.anchor, &capture, &c)?;
    assert!(!accepted.ready());
    let mut too_many = raw(&[]);
    too_many.items = (1..4096)
        .map(|id| {
            let mut item = capture.items[1].clone();
            item.native_id = id;
            item
        })
        .collect();
    assert!(InputSupplyAssessment::qualify(&demand, &policy, c.anchor, &too_many, &c).is_err());
    Ok(())
}

#[test]
fn input_supply_revalidation_retains_demand_but_allows_unrelated_growth_and_new_supply()
-> Result<()> {
    let capture = raw(&[(1, "WOOD", 4), (2, "BOULDER", 100)]);
    let c = context(&capture)?;
    let policy = source()?;
    let demand = requirements(&[(WorkOrderRecipe::WoodenBed, 2)], 2)?;
    let original = InputSupplyAssessment::qualify(&demand, &policy, c.anchor, &capture, &c)?;
    let mut changed = capture.clone();
    changed.items[1].stack_size += 1;
    changed.items[0].stack_size += 3;
    changed.jobs.next_job_id += 1;
    changed.next_building_id += 1;
    changed.next_item_id += 1;
    let fresh = original.revalidate(&policy, &changed, &c)?;
    assert!(fresh.ready());
    assert_eq!(fresh.requirements(), original.requirements());
    assert_eq!(fresh.required_stack_units(), 4);
    assert_eq!(fresh.eligible_stack_units(), 7);
    assert_ne!(fresh.source_digest(), original.source_digest());
    assert_ne!(fresh.witness(), original.witness());
    changed.items[0].flags |= 2;
    let lost = original.revalidate(&policy, &changed, &c)?;
    assert!(!lost.ready());
    assert_eq!(lost.shortfall_stack_units(), 4);
    assert_eq!(lost.requirements(), original.requirements());
    assert!(original.ready()); // A historical value was not silently rewritten.
    Ok(())
}

#[test]
fn input_supply_revalidation_cannot_rewind_allocators_or_substitute_original_source() -> Result<()>
{
    let capture = raw(&[(1, "WOOD", 4)]);
    let c = context(&capture)?;
    let policy = source()?;
    let demand = requirements(&[(WorkOrderRecipe::WoodenBed, 2)], 0)?;
    let original = InputSupplyAssessment::qualify(&demand, &policy, c.anchor, &capture, &c)?;
    for counter in 0..3 {
        let mut changed = capture.clone();
        match counter {
            0 => changed.jobs.next_job_id -= 1,
            1 => changed.next_building_id -= 1,
            _ => changed.next_item_id -= 1,
        }
        changed.validate()?;
        assert!(
            original.revalidate(&policy, &changed, &c).is_err(),
            "allocator {counter}"
        );
    }
    let foreign = InputSupplySource::new(
        "region1".into(),
        4,
        "0.51.11".into(),
        "0.51.11-r1".into(),
        602,
    )?;
    let mut changed = capture.clone();
    changed.jobs.bridge_generation = 602;
    assert!(original.revalidate(&foreign, &changed, &c).is_err());
    assert!(original.revalidate(&policy, &changed, &c).is_err());
    assert_eq!(original, original.revalidate(&policy, &capture, &c)?);
    Ok(())
}

#[test]
fn input_supply_uses_the_existing_allocator_policy_not_raw_observed_stock_totals() -> Result<()> {
    let mut capture = raw(&[(1, "WOOD", 1), (2, "WOOD", 500), (3, "WOOD", 400)]);
    capture.items[1].flags |= 8;
    capture.items[2].flags |= 16;
    let demand = requirements(&[(WorkOrderRecipe::WoodenBed, 2)], 0)?;
    let assessment = assess(&capture, &demand)?;
    assert!(!assessment.ready());
    assert_eq!(assessment.eligible_stack_units(), 1);
    assert_eq!(assessment.shortfall_stack_units(), 1);
    let mut state = crate::live_operations::LiveOperationsState::default();
    state.publish(capture)?;
    let mut c = context(
        state
            .observation()
            .ok_or_else(|| invalid("fixture observation missing"))?,
    )?;
    c.anchor = state
        .snapshot()
        .ok_or_else(|| invalid("fixture snapshot missing"))?
        .anchor();
    let query = crate::operations_analysis::plan_inventory(
        &state,
        &c,
        assessment.demands(),
        MAX_ANALYSIS_WORK,
    )?;
    assert_eq!(assessment.allocation(), &query.allocation);
    assert_eq!(assessment.excluded_items(), &query.excluded_items);
    assert_eq!(
        assessment.eligible_stack_units(),
        query.candidate_stack_units
    );
    Ok(())
}
