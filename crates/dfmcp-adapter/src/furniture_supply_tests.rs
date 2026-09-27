use super::*;
use crate::furniture_allocation::Slot;
use crate::live_jobs::{LiveJob, LiveJobObservation};
use crate::live_map::LiveMapObservation;
use crate::live_operations::{
    JobItemAttachment, LiveBuilding, LiveItem, LiveOperationsObservation, LiveOperationsState,
    OperationsProfile,
};
use crate::live_spatial::{LiveSpatialObservation, LiveSpatialState};
use dfmcp_core::{
    CapabilityGrant, CapabilityScope, FortressId, GameTick, MapCoord, MapCuboid, RequestId,
    SessionId, WorkBudget,
};
use dfmcp_world::map_region::{Cell, MapRegion, Region};
use std::time::Duration;

const FOLDER: &str = "furniture-supply-test";
const SITE: u32 = 2;

fn item(id: u32, kind: Kind, position: [i32; 3]) -> LiveItem {
    LiveItem {
        native_id: id,
        item_type: match kind {
            Kind::Bed => 1,
            Kind::Chair => 2,
            Kind::Table => 3,
        },
        type_key: kind.native_key().into(),
        subtype: -1,
        material_type: 0,
        material_index: 1,
        stack_size: 1,
        raw_position: MapCoord::new(position[0], position[1], position[2]),
        flags: 64,
        container_native_id: None,
        holder_building_native_id: None,
    }
}

fn observation(items: Vec<LiveItem>) -> LiveOperationsObservation {
    LiveOperationsObservation {
        jobs: LiveJobObservation {
            bridge_generation: 7,
            df_version: "test-df".into(),
            dfhack_version: "test-dfhack".into(),
            year: 250,
            year_tick: 100,
            paused: true,
            site_id: SITE as i32,
            world_folder: FOLDER.into(),
            next_job_id: 100,
            jobs: Vec::new(),
        },
        next_building_id: 100,
        next_item_id: items.iter().map(|i| i.native_id).max().unwrap_or(0) + 1,
        buildings: Vec::new(),
        items,
        attachments: Vec::new(),
    }
}

fn slot(name: &str, kind: Kind, target: [u32; 3]) -> Slot {
    Slot {
        name: name.into(),
        kind,
        target,
        after: Vec::new(),
        material: None,
        subtype: None,
        max_distance: allocation::MAX_DISTANCE,
    }
}

fn request(slots: Vec<Slot>) -> Request {
    Request {
        slots,
        excluded_items: Vec::new(),
    }
}

fn context<S: OperationsStateView + ?Sized>(state: &S) -> OperationContext {
    OperationContext {
        session_id: SessionId::new(37),
        request_id: RequestId::new(1),
        anchor: state.operations_snapshot().unwrap().anchor(),
        budget: WorkBudget {
            max_wall_millis: 60_000,
            max_entities: 4096,
            ..WorkBudget::default()
        },
        grants: vec![CapabilityGrant {
            capability: Capability::Query,
            scope: CapabilityScope::default(),
            max_risk: RiskTier::ReadOnly,
            expires_at_tick: None,
            remaining_uses: None,
        }],
        cancellation_requested: false,
    }
}

fn published(value: LiveOperationsObservation) -> LiveOperationsState {
    let mut state = LiveOperationsState::with_profile(OperationsProfile::PagedV1_4);
    state.publish(value).unwrap();
    state
}

fn report(value: LiveOperationsObservation, requested: &Request) -> Report {
    let state = published(value);
    plan(&state, &context(&state), FOLDER, SITE, requested, MAX_WORK).unwrap()
}

fn assigned(result: &Report) -> Vec<(&str, u32)> {
    result
        .allocation
        .assignments
        .iter()
        .map(|a| (a.slot.as_str(), a.item_id))
        .collect()
}

fn spatial(
    operations: &LiveOperationsObservation,
    dimensions: [u32; 3],
    cell: Cell,
) -> LiveSpatialObservation {
    let jobs = &operations.jobs;
    let terrain = LiveMapObservation {
        bridge_generation: jobs.bridge_generation,
        df_version: jobs.df_version.clone(),
        dfhack_version: jobs.dfhack_version.clone(),
        year: jobs.year,
        year_tick: jobs.year_tick,
        paused: jobs.paused,
        site_id: SITE,
        world_folder: jobs.world_folder.clone(),
        map_dimensions: dimensions,
        map: MapRegion {
            region: Region {
                origin: [0, 0, 0],
                size: [1, 1, 1],
            },
            cells: vec![cell],
        },
    };
    let operations_bytes = operations
        .encode_profile(OperationsProfile::PagedV1_4)
        .unwrap();
    let terrain_bytes = terrain.encode_payload().unwrap();
    let mut bytes = b"DFMS1600".to_vec();
    for part in [&operations_bytes, &terrain_bytes] {
        bytes.extend_from_slice(&(part.len() as u32).to_be_bytes());
        bytes.extend_from_slice(part);
    }
    LiveSpatialObservation::decode_payload(
        &bytes,
        jobs.bridge_generation,
        jobs.df_version.clone(),
        jobs.dfhack_version.clone(),
    )
    .unwrap()
}

#[test]
fn globally_reroutes_generic_slot_to_preserve_scarce_material() {
    let first = item(10, Kind::Bed, [4, 4, 3]);
    let mut second = item(11, Kind::Bed, [8, 4, 3]);
    second.material_index = 2;
    let generic = slot("a_generic", Kind::Bed, [4, 4, 3]);
    let mut scarce = slot("z_specific", Kind::Bed, [5, 4, 3]);
    scarce.material = Some((0, 1));
    let result = report(
        observation(vec![first, second]),
        &request(vec![generic, scarce]),
    );

    assert_eq!(
        assigned(&result),
        vec![("a_generic", 11), ("z_specific", 10)]
    );
    assert_eq!(result.allocation.total_distance, Some(5));
    assert_eq!(result.allocation.maximum_assignable, 2);
    assert_eq!(result.allocation.shortage, None);
    assert_eq!(
        result.allocation.compatible_counts,
        vec![("a_generic".into(), 2), ("z_specific".into(), 1)]
    );
    assert_eq!(
        result.items.keys().copied().collect::<Vec<_>>(),
        vec![10, 11]
    );
}

#[test]
fn complete_inventory_scarcity_emits_hall_witness_without_partial_assignments() {
    let requested = request(vec![
        slot("bed_c", Kind::Bed, [6, 4, 3]),
        slot("bed_a", Kind::Bed, [4, 4, 3]),
        slot("bed_b", Kind::Bed, [5, 4, 3]),
    ]);
    let result = report(
        observation(vec![
            item(10, Kind::Bed, [4, 4, 3]),
            item(11, Kind::Bed, [5, 4, 3]),
            item(12, Kind::Chair, [6, 4, 3]),
        ]),
        &requested,
    );
    assert_eq!(result.candidate_count, 3);
    assert_eq!(result.observed_items, 3);
    assert!(result.allocation.assignments.is_empty());
    assert_eq!(result.allocation.total_distance, None);
    assert_eq!(result.allocation.maximum_assignable, 2);
    let shortage = result.allocation.shortage.as_ref().unwrap();
    assert_eq!(shortage.slots, vec!["bed_a", "bed_b", "bed_c"]);
    assert_eq!(shortage.candidate_items, vec![10, 11]);
    assert_eq!(shortage.missing, 1);
    assert_eq!(
        result.items.keys().copied().collect::<Vec<_>>(),
        vec![10, 11]
    );
}

#[test]
fn all_512_native_flag_words_leave_only_exclusively_ground_supply() {
    let items = (0..512)
        .map(|flags| {
            let mut value = item(flags + 1, Kind::Bed, [4, 4, 3]);
            value.flags = flags;
            value
        })
        .collect();
    let result = report(
        observation(items),
        &request(vec![slot("bed", Kind::Bed, [4, 4, 3])]),
    );
    assert_eq!(result.observed_items, 512);
    assert_eq!(result.candidate_count, 1);
    assert_eq!(assigned(&result), vec![("bed", 65)]);
    assert_eq!(result.item_counts["not_exclusively_ground_flags"], 511);
    assert_eq!(result.item_counts["candidate_furniture"], 1);
}

#[test]
fn actual_job_attachment_excludes_item_even_without_native_in_job_flag() {
    let mut value = observation(vec![item(10, Kind::Bed, [4, 4, 3])]);
    value.jobs.jobs = vec![LiveJob {
        native_id: 5,
        job_type: 1,
        type_key: "StoreItemInStockpile".into(),
        reaction: String::new(),
        suspended: false,
        repeating: false,
        position: MapCoord::new(4, 4, 3),
        worker_native_id: None,
        holder_native_id: None,
        completion_timer: -1,
        attached_item_count: 1,
        required_item_filter_count: 0,
    }];
    value.attachments = vec![JobItemAttachment {
        job_native_id: 5,
        item_native_id: 10,
        role: 0,
        filter_index: -1,
    }];
    assert_eq!(value.items[0].flags, 64);
    let result = report(value, &request(vec![slot("bed", Kind::Bed, [4, 4, 3])]));
    assert_eq!(result.candidate_count, 0);
    assert_eq!(result.item_counts["job_attached_item"], 1);
    assert!(result.items.is_empty());
    assert_eq!(result.allocation.maximum_assignable, 0);
}

#[test]
fn classifies_every_item_and_never_guesses_material_holder_or_bounded_position() {
    let mut items = vec![
        item(1, Kind::Bed, [4, 4, 3]),
        item(2, Kind::Chair, [5, 4, 3]),
        item(3, Kind::Table, [6, 4, 3]),
    ];
    for id in 4..=18 {
        let mut value = item(id, Kind::Bed, [4, 4, 3]);
        match id {
            4 => value.type_key = "bed".into(),
            5 => value.stack_size = 0,
            6 => value.stack_size = 2,
            7 => value.material_type = -1,
            8 => value.raw_position.x = -1,
            9 => value.raw_position.x = 32_768,
            10 => value.raw_position.y = -1,
            11 => value.raw_position.y = 32_768,
            12 => value.raw_position.z = -1,
            13 => value.raw_position.z = 32_768,
            14 => value.holder_building_native_id = Some(20),
            15 => value.container_native_id = Some(17),
            16 => value.flags = 65,
            17 => value.type_key = "BAR".into(),
            _ => {} // Explicitly excluded below despite otherwise eligible state.
        }
        items.push(value);
    }
    let mut value = observation(items);
    value.buildings = vec![LiveBuilding {
        native_id: 20,
        building_type: 1,
        type_key: "Bed".into(),
        x1: 4,
        y1: 4,
        x2: 4,
        y2: 4,
        z: 3,
        build_stage: 3,
        max_build_stage: 3,
    }];
    let mut requested = request(vec![
        slot("bed", Kind::Bed, [4, 4, 3]),
        slot("chair", Kind::Chair, [5, 4, 3]),
        slot("table", Kind::Table, [6, 4, 3]),
    ]);
    requested.excluded_items = vec![18];
    let result = report(value, &requested);
    assert_eq!(
        assigned(&result),
        vec![("bed", 1), ("chair", 2), ("table", 3)]
    );
    assert_eq!(result.candidate_count, 3);
    assert_eq!(result.item_counts.values().sum::<u64>(), 18);
    for (reason, count) in [
        ("candidate_furniture", 3),
        ("unsupported_furniture_type", 2),
        ("not_singleton", 2),
        ("unknown_material", 1),
        ("unestablished_bounded_position", 6),
        ("building_held_item", 1),
        ("contained_item", 1),
        ("not_exclusively_ground_flags", 1),
        ("explicitly_excluded_item", 1),
    ] {
        assert_eq!(result.item_counts[reason], count, "{reason}");
    }
}

#[test]
fn nested_container_descendants_never_become_direct_ground_supply() {
    let mut items = Vec::new();
    for id in 1..=200 {
        let mut value = item(id, Kind::Bed, [4, 4, 3]);
        if id < 200 {
            value.container_native_id = Some(id + 1);
        } else {
            value.type_key = "BIN".into();
        }
        items.push(value);
    }
    items.push(item(201, Kind::Bed, [5, 4, 3]));
    let result = report(
        observation(items),
        &request(vec![slot("bed", Kind::Bed, [4, 4, 3])]),
    );
    assert_eq!(assigned(&result), vec![("bed", 201)]);
    assert_eq!(result.candidate_count, 1);
    assert_eq!(result.item_counts["contained_item"], 199);
    assert_eq!(result.item_counts["unsupported_furniture_type"], 1);
}

#[test]
fn ground_furniture_containing_an_observed_payload_is_excluded_from_supply() {
    let container = item(10, Kind::Bed, [4, 4, 3]);
    let mut payload = item(11, Kind::Bed, [4, 4, 3]);
    payload.item_type = 4;
    payload.type_key = "BAR".into();
    payload.container_native_id = Some(10);
    let alternative = item(12, Kind::Bed, [5, 4, 3]);

    let result = report(
        observation(vec![container, payload, alternative]),
        &request(vec![slot("bed", Kind::Bed, [4, 4, 3])]),
    );
    assert_eq!(assigned(&result), vec![("bed", 12)]);
    assert_eq!(result.observed_items, 3);
    assert_eq!(result.candidate_count, 1);
    assert_eq!(result.item_counts["contains_observed_items"], 1);
    assert_eq!(result.item_counts["unsupported_furniture_type"], 1);
    assert_eq!(result.item_counts["candidate_furniture"], 1);
    assert_eq!(result.items.keys().copied().collect::<Vec<_>>(), vec![12]);
}

#[test]
fn exact_material_subtype_same_level_and_distance_apply_after_supply_projection() {
    let mut items: Vec<_> = (1..=6)
        .map(|id| {
            let mut value = item(id, Kind::Bed, [3, 4, 3]);
            value.subtype = 7;
            value
        })
        .collect();
    items[1].material_index = 2;
    items[2].subtype = 8;
    items[3].raw_position.z = 4;
    items[4].raw_position.x = 6;
    items[5].type_key = "CHAIR".into();
    items[5].item_type = 2;
    let mut wanted = slot("specific", Kind::Bed, [4, 4, 3]);
    wanted.material = Some((0, 1));
    wanted.subtype = Some(7);
    wanted.max_distance = 1;
    let result = report(observation(items), &request(vec![wanted]));
    assert_eq!(result.candidate_count, 6);
    assert_eq!(
        result.allocation.compatible_counts,
        vec![("specific".into(), 1)]
    );
    assert_eq!(assigned(&result), vec![("specific", 1)]);
    assert_eq!(result.allocation.total_distance, Some(1));
    assert_eq!(result.items.len(), 1);
    assert_eq!(result.items[&1].candidate.subtype, 7);
    assert_eq!(result.items[&1].candidate.material_index, 1);
}

#[test]
fn bounded_position_edges_and_inorganic_material_index_survive_projection() {
    let mut first = item(1, Kind::Bed, [0, 0, 0]);
    first.material_index = -1;
    let second = item(2, Kind::Chair, [32_767, 32_767, 32_767]);
    let result = report(
        observation(vec![first, second]),
        &request(vec![
            slot("bed", Kind::Bed, [1, 1, 0]),
            slot("chair", Kind::Chair, [32_766, 32_766, 32_767]),
        ]),
    );
    assert_eq!(assigned(&result), vec![("bed", 1), ("chair", 2)]);
    assert_eq!(result.allocation.total_distance, Some(4));
    assert_eq!(result.items[&1].candidate.position, [0, 0, 0]);
    assert_eq!(result.items[&1].candidate.material_index, -1);
    assert_eq!(result.items[&2].candidate.position, [32_767; 3]);
}

#[test]
fn request_order_is_canonical_and_explicit_exclusions_remove_supply() {
    let state = published(observation(vec![
        item(1, Kind::Bed, [4, 4, 3]),
        item(2, Kind::Chair, [5, 4, 3]),
        item(3, Kind::Bed, [6, 4, 3]),
        item(4, Kind::Chair, [7, 4, 3]),
    ]));
    let mut a = request(vec![
        slot("z_chair", Kind::Chair, [5, 4, 3]),
        slot("a_bed", Kind::Bed, [4, 4, 3]),
    ]);
    a.slots[0].after = vec!["a_bed".into()];
    a.excluded_items = vec![4, 1];
    let mut b = a.clone();
    b.slots.reverse();
    b.excluded_items.reverse();
    let c = context(&state);
    let first = plan(&state, &c, FOLDER, SITE, &a, MAX_WORK).unwrap();
    let second = plan(&state, &c, FOLDER, SITE, &b, MAX_WORK).unwrap();
    assert_eq!(first, second);
    assert_eq!(assigned(&first), vec![("a_bed", 3), ("z_chair", 2)]);
    assert_eq!(first.request.excluded_items, vec![1, 4]);
    assert_eq!(first.request.slots[0].name, "a_bed");
    assert_eq!(first.item_counts["explicitly_excluded_item"], 2);
}

#[test]
fn whole_inventory_query_requires_authority_without_entity_or_map_scope_restrictions() {
    let state = published(observation(vec![item(1, Kind::Bed, [4, 4, 3])]));
    let requested = request(vec![slot("bed", Kind::Bed, [4, 4, 3])]);
    for variant in 0..8 {
        let mut c = context(&state);
        match variant {
            0 => c.grants.clear(),
            1 => c.grants[0].capability = Capability::Observe,
            2 => {
                c.grants[0].scope.entity_ids.insert(item_entity_id(1));
            }
            3 => {
                c.grants[0].scope.map_area =
                    Some(MapCuboid::new(MapCoord::new(0, 0, 0), MapCoord::new(63, 63, 7)).unwrap());
            }
            4 => c.grants[0].scope.fortress_id = Some(FortressId::new(0)),
            5 => c.grants[0].expires_at_tick = Some(GameTick(0)),
            6 => c.grants[0].remaining_uses = Some(0),
            _ => c.grants[0].remaining_uses = Some(1),
        }
        assert_eq!(
            plan(&state, &c, FOLDER, SITE, &requested, MAX_WORK)
                .unwrap_err()
                .code,
            ErrorCode::CapabilityDenied,
            "authority variant {variant}"
        );
    }
    let mut c = context(&state);
    c.grants[0].scope.fortress_id = Some(c.anchor.fortress_id);
    assert!(plan(&state, &c, FOLDER, SITE, &requested, MAX_WORK).is_ok());
    c.cancellation_requested = true;
    assert_eq!(
        plan(&state, &c, FOLDER, SITE, &requested, MAX_WORK)
            .unwrap_err()
            .code,
        ErrorCode::CancellationRequested
    );
}

#[test]
fn refuses_missing_published_source_wrong_fortress_and_every_stale_anchor_component() {
    let state = published(observation(vec![item(1, Kind::Bed, [4, 4, 3])]));
    let c = context(&state);
    let requested = request(vec![slot("bed", Kind::Bed, [4, 4, 3])]);
    let absent = LiveOperationsState::default();
    assert_eq!(
        plan(&absent, &c, FOLDER, SITE, &requested, MAX_WORK)
            .unwrap_err()
            .code,
        ErrorCode::InvalidRequest
    );
    for (folder, site) in [("another-fortress", SITE), (FOLDER, SITE + 1)] {
        assert_eq!(
            plan(&state, &c, folder, site, &requested, MAX_WORK)
                .unwrap_err()
                .code,
            ErrorCode::StaleAnchor
        );
    }
    for (folder, site) in [("", SITE), ("bad\0folder", SITE), (FOLDER, u32::MAX)] {
        assert_eq!(
            plan(&state, &c, folder, site, &requested, MAX_WORK)
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }
    for variant in 0..5 {
        let mut changed = c.clone();
        match variant {
            0 => changed.anchor.state_hash = Digest32::of_bytes(b"another canonical snapshot"),
            1 => changed.anchor.cursor.sequence += 1,
            2 => changed.anchor.cursor.epoch += 1,
            3 => changed.anchor.tick = GameTick(c.anchor.tick.0 + 1),
            _ => changed.anchor.fortress_id = FortressId::new(0),
        }
        assert_eq!(
            plan(&state, &changed, FOLDER, SITE, &requested, MAX_WORK)
                .unwrap_err()
                .code,
            ErrorCode::StaleAnchor,
            "anchor variant {variant}"
        );
    }
}

#[test]
fn full_graph_scan_and_one_shared_work_allowance_bound_all_planning_phases() {
    let state = published(observation(vec![
        item(1, Kind::Bed, [4, 4, 3]),
        item(2, Kind::Bed, [5, 4, 3]),
    ]));
    let requested = request(vec![
        slot("first", Kind::Bed, [4, 4, 3]),
        slot("second", Kind::Bed, [5, 4, 3]),
    ]);
    let mut c = context(&state);
    let whole_graph_count = state.snapshot().unwrap().graph.entities.len() as u32;
    assert!(whole_graph_count > 2); // The fortress root also consumes the scan allowance.
    c.budget.max_entities = whole_graph_count - 1;
    assert_eq!(
        plan(&state, &c, FOLDER, SITE, &requested, MAX_WORK)
            .unwrap_err()
            .code,
        ErrorCode::BudgetExceeded
    );
    c.budget.max_entities = whole_graph_count;
    let complete = plan(&state, &c, FOLDER, SITE, &requested, MAX_WORK).unwrap();
    assert!(complete.work_units > 1);
    for maximum in [0, 1, complete.work_units - 1, MAX_WORK + 1] {
        assert_eq!(
            plan(&state, &c, FOLDER, SITE, &requested, maximum)
                .unwrap_err()
                .code,
            ErrorCode::BudgetExceeded,
            "work allowance {maximum}"
        );
    }
    assert_eq!(
        plan(&state, &c, FOLDER, SITE, &requested, complete.work_units).unwrap(),
        complete
    );
    c.budget.max_wall_millis = 0;
    assert_eq!(
        plan(&state, &c, FOLDER, SITE, &requested, MAX_WORK)
            .unwrap_err()
            .code,
        ErrorCode::InvalidRequest
    );

    // Expire the same wall clock without a timing-sensitive sleep or a huge fixture.
    let mut work = Work::new(100, 60_000).unwrap();
    work.charge().unwrap();
    work.wall_millis = 1;
    work.started = Instant::now() - Duration::from_millis(2);
    assert_eq!(work.charge().unwrap_err().code, ErrorCode::BudgetExceeded);
    assert_eq!(work.charge().unwrap_err().code, ErrorCode::BudgetExceeded);
}

#[test]
fn item_reappearance_and_native_source_reset_preserve_original_canonical_generations() {
    let original = observation(vec![item(10, Kind::Bed, [4, 4, 3])]);
    let mut state = published(original.clone());
    let requested = request(vec![slot("bed", Kind::Bed, [4, 4, 3])]);
    let old_context = context(&state);
    let first = plan(&state, &old_context, FOLDER, SITE, &requested, MAX_WORK).unwrap();
    assert_eq!(first.items[&10].handle.generation, 1);
    assert_eq!(first.items[&10].handle.revision, 1);
    assert_eq!(first.anchor, state.snapshot().unwrap().anchor());
    assert_eq!(first.source_digest, state.source_digest().unwrap());
    assert_ne!(first.anchor.state_hash, first.source_digest);

    let mut removed = original.clone();
    removed.items.clear();
    removed.jobs.year_tick += 1;
    state.publish(removed).unwrap();
    let empty = plan(&state, &context(&state), FOLDER, SITE, &requested, MAX_WORK).unwrap();
    assert_eq!(empty.observed_items, 0);
    assert!(empty.items.is_empty());
    let mut returned = original.clone();
    returned.jobs.year_tick += 2;
    state.publish(returned.clone()).unwrap();
    let second = plan(&state, &context(&state), FOLDER, SITE, &requested, MAX_WORK).unwrap();
    assert_eq!(second.items[&10].handle.generation, 2);
    assert_eq!(second.items[&10].handle.revision, 3);
    assert_eq!(
        second.items[&10].handle.entity_id,
        first.items[&10].handle.entity_id
    );
    assert_eq!(
        plan(&state, &old_context, FOLDER, SITE, &requested, MAX_WORK)
            .unwrap_err()
            .code,
        ErrorCode::StaleAnchor
    );

    returned.jobs.bridge_generation += 1;
    state.publish(returned).unwrap();
    let reset = plan(&state, &context(&state), FOLDER, SITE, &requested, MAX_WORK).unwrap();
    assert_eq!(reset.anchor.cursor.epoch, second.anchor.cursor.epoch + 1);
    assert_eq!(reset.items[&10].handle.generation, 3);
    assert_eq!(reset.items[&10].handle.revision, 1);
    assert_ne!(reset.source_digest, second.source_digest);
}

#[test]
fn enclosing_spatial_source_and_generation_survive_map_change_reset_and_item_reuse() {
    let operations = observation(vec![item(10, Kind::Bed, [4, 4, 3])]);
    let requested = request(vec![slot("bed", Kind::Bed, [4, 4, 3])]);
    let mut state = LiveSpatialState::default();
    let original = spatial(&operations, [64, 64, 8], Cell::Hidden);
    state.publish(original.clone()).unwrap();
    let old_context = context(&state);
    let first = plan(&state, &old_context, FOLDER, SITE, &requested, MAX_WORK).unwrap();
    assert_eq!(first.source_digest, original.source_digest().unwrap());
    assert_ne!(
        first.source_digest,
        operations
            .source_digest_profile(OperationsProfile::PagedV1_4)
            .unwrap()
    );

    state
        .publish(spatial(&operations, [64, 64, 8], Cell::Unallocated))
        .unwrap();
    let changed = plan(&state, &context(&state), FOLDER, SITE, &requested, MAX_WORK).unwrap();
    assert_eq!(changed.items[&10].handle.generation, 1);
    assert_eq!(changed.items[&10].handle.revision, 2);
    assert_ne!(changed.source_digest, first.source_digest);
    assert_ne!(changed.anchor, first.anchor);

    state
        .publish(spatial(&operations, [65, 64, 8], Cell::Hidden))
        .unwrap();
    let reset = plan(&state, &context(&state), FOLDER, SITE, &requested, MAX_WORK).unwrap();
    assert_eq!(reset.items[&10].handle.generation, 2);
    assert_eq!(reset.items[&10].handle.revision, 1);
    assert_eq!(reset.anchor.cursor.epoch, 1);
    assert_eq!(reset.anchor, state.snapshot().unwrap().anchor());
    assert_eq!(reset.source_digest, state.source_digest().unwrap());

    let mut removed = operations.clone();
    removed.items.clear();
    removed.jobs.year_tick += 1;
    state
        .publish(spatial(&removed, [65, 64, 8], Cell::Hidden))
        .unwrap();
    let mut returned = operations;
    returned.jobs.year_tick += 2;
    state
        .publish(spatial(&returned, [65, 64, 8], Cell::Hidden))
        .unwrap();
    let reused = plan(&state, &context(&state), FOLDER, SITE, &requested, MAX_WORK).unwrap();
    assert_eq!(reused.items[&10].handle.generation, 3);
    assert_eq!(reused.items[&10].handle.revision, 3);
    assert_eq!(reused.anchor.cursor.epoch, 1);
    assert_eq!(
        plan(&state, &old_context, FOLDER, SITE, &requested, MAX_WORK)
            .unwrap_err()
            .code,
        ErrorCode::StaleAnchor
    );
}

#[test]
fn thirty_two_slots_use_complete_supply_and_emit_only_thirty_two_original_handles() {
    let mut items = Vec::new();
    let mut slots = Vec::new();
    for index in 0..32 {
        let kind = [Kind::Bed, Kind::Chair, Kind::Table][index % 3];
        items.push(item(index as u32 + 1, kind, [index as i32 + 1, 1, 3]));
        let mut wanted = slot(&format!("slot_{index:02}"), kind, [index as u32 + 1, 1, 3]);
        if index > 0 {
            wanted.after.push(format!("slot_{:02}", index - 1));
        }
        slots.push(wanted);
    }
    for index in 0..32 {
        let kind = [Kind::Bed, Kind::Chair, Kind::Table][index % 3];
        items.push(item(index as u32 + 101, kind, [50, 50, 3]));
    }
    let state = published(observation(items));
    let requested = request(slots);
    let result = plan(&state, &context(&state), FOLDER, SITE, &requested, MAX_WORK).unwrap();
    assert_eq!(result.observed_items, 64);
    assert_eq!(result.candidate_count, 64);
    assert_eq!(result.allocation.maximum_assignable, 32);
    assert_eq!(result.allocation.total_distance, Some(0));
    assert_eq!(result.items.len(), 32);
    assert_eq!(result.allocation.assignments.len(), 32);
    for (index, assignment) in result.allocation.assignments.iter().enumerate() {
        let id = index as u32 + 1;
        assert_eq!(assignment.item_id, id);
        let canonical = &state.snapshot().unwrap().graph.entities[&item_entity_id(id)];
        assert_eq!(result.items[&id].handle.entity_id, canonical.id);
        assert_eq!(result.items[&id].handle.generation, canonical.generation);
        assert_eq!(result.items[&id].handle.revision, canonical.revision);
    }
    let mut oversized = requested;
    oversized
        .slots
        .push(slot("thirty_third", Kind::Bed, [33, 1, 3]));
    assert_eq!(
        plan(&state, &context(&state), FOLDER, SITE, &oversized, MAX_WORK)
            .unwrap_err()
            .code,
        ErrorCode::InvalidRequest
    );
}

#[test]
fn paged_native_roster_accepts_exactly_65536_items_and_refuses_one_more() {
    // Exercise the complete native boundary without allocating a million-fact world graph.
    let mut value = observation(
        (1..=65_536)
            .map(|id| item(id, Kind::Bed, [4, 4, 3]))
            .collect(),
    );
    value
        .validate_profile(OperationsProfile::PagedV1_4)
        .unwrap();
    assert_eq!(
        value
            .validate_profile(OperationsProfile::V1_3)
            .unwrap_err()
            .code,
        ErrorCode::BudgetExceeded
    );
    let bytes = value.encode_profile(OperationsProfile::PagedV1_4).unwrap();
    let decoded = LiveOperationsObservation::decode_profile(
        &bytes,
        value.jobs.bridge_generation,
        value.jobs.df_version.clone(),
        value.jobs.dfhack_version.clone(),
        OperationsProfile::PagedV1_4,
    )
    .unwrap();
    assert_eq!(decoded, value);
    value.items.push(item(65_537, Kind::Bed, [4, 4, 3]));
    value.next_item_id = 65_538;
    assert_eq!(
        value
            .validate_profile(OperationsProfile::PagedV1_4)
            .unwrap_err()
            .code,
        ErrorCode::BudgetExceeded
    );
}
