use super::*;
use dfmcp_core::{EdgeId, ObservationCursor};
use dfmcp_world::terrain::uniform_chunk;
use dfmcp_world::{ChunkCoord, EdgeKind, EdgeRecord, WorldGraph};

const CITIZEN: EntityId = EntityId::new(11);
const BREWER: EntityId = EntityId::new(12);
const STILL: EntityId = EntityId::new(51);
const LEDGER: EntityId = EntityId::new(91);

fn put(
    snapshot: &mut WorldSnapshot,
    id: EntityId,
    kind: EntityKind,
    fields: Vec<(&str, Value)>,
) -> Result<()> {
    let label = kind.as_str().to_owned();
    create_entity(
        snapshot,
        id,
        kind,
        label,
        fields
            .into_iter()
            .map(|(name, value)| (name.to_owned(), value))
            .collect(),
    )?;
    snapshot.refresh_hash();
    Ok(())
}

fn world() -> Result<WorldSnapshot> {
    let mut snapshot = WorldSnapshot::new(
        FortressId::new(9),
        GameTick(1_000),
        ObservationCursor::ORIGIN,
        false,
        WorldGraph::default(),
    );
    for (id, brews) in [(CITIZEN, false), (BREWER, true)] {
        put(
            &mut snapshot,
            id,
            EntityKind::Unit,
            vec![
                ("alive", Value::Bool(true)),
                ("labor.BREW", Value::Bool(brews)),
                (SQUAD_FIELD, Value::Null),
                (NEED_DRINK_FIELD, Value::Text("satisfied".to_owned())),
                (NEED_FOOD_FIELD, Value::Text("satisfied".to_owned())),
            ],
        )?;
    }
    put(
        &mut snapshot,
        STILL,
        EntityKind::Building,
        vec![
            ("building_kind", Value::Text("workshop:Still".to_owned())),
            (
                CONSTRUCTION_STAGE_FIELD,
                Value::Text(STAGE_COMPLETE.to_owned()),
            ),
        ],
    )?;
    put(
        &mut snapshot,
        LEDGER,
        EntityKind::Other(STOCK_LEDGER_KIND.to_owned()),
        vec![
            (STOCK_DRINK_FIELD, Value::U64(0)),
            (STOCK_FOOD_FIELD, Value::U64(100)),
            (METABOLISM_TICKS_FIELD, Value::U64(0)),
        ],
    )?;
    Ok(snapshot)
}

fn set(snapshot: &mut WorldSnapshot, id: EntityId, name: &str, value: Value) -> Result<()> {
    write_fields(snapshot, id, vec![(name.to_owned(), value)])?;
    snapshot.refresh_hash();
    Ok(())
}

fn order(
    snapshot: &mut WorldSnapshot,
    id: EntityId,
    name: &str,
    job: &str,
    amount: u64,
    partial: u64,
    conditions: Vec<WorkOrderCondition>,
) -> Result<()> {
    put(
        snapshot,
        id,
        EntityKind::WorkOrder,
        vec![
            (WORK_ORDER_NAME_FIELD, Value::Text(name.to_owned())),
            ("job_token", Value::Text(job.to_owned())),
            (
                WORK_ORDER_CONDITIONS_FIELD,
                work_order_conditions_value(&conditions)?,
            ),
            ("amount_total", Value::U64(amount)),
            (AMOUNT_REMAINING_FIELD, Value::U64(amount)),
            ("work_ticks", Value::U64(partial)),
            (STATUS_FIELD, Value::Text(STATUS_ACTIVE.to_owned())),
        ],
    )
}

fn threat(snapshot: &mut WorldSnapshot, id: EntityId, delay: u64) -> Result<()> {
    let arrival = snapshot.tick.0 + delay;
    put(
        snapshot,
        id,
        EntityKind::Creature,
        vec![
            (HOSTILE_FIELD, Value::Bool(true)),
            (HEALTH_FIELD, Value::U64(100)),
            (ARRIVES_AT_FIELD, Value::U64(arrival)),
            (
                THREAT_STATUS_FIELD,
                Value::Text(THREAT_APPROACHING.to_owned()),
            ),
            (COMBAT_ROUNDS_FIELD, Value::U64(0)),
        ],
    )
}

fn advance_with(
    snapshot: &mut WorldSnapshot,
    ticks: u64,
    limits: EffectAdvanceLimits,
) -> Result<bool> {
    // Exercise the public shadow contract, including refusal after an earlier
    // internal event. No event result is published until the whole call succeeds.
    let mut shadow = snapshot.clone();
    shadow.tick = GameTick(
        snapshot
            .tick
            .0
            .checked_add(ticks)
            .ok_or_else(|| advance_budget_error("test clock would overflow"))?,
    );
    let changed = advance_effects_with_limits(&mut shadow, ticks, limits)?;
    shadow.refresh_hash();
    *snapshot = shadow;
    Ok(changed)
}

fn after(source: &WorldSnapshot, cuts: &[u64]) -> Result<WorldSnapshot> {
    let mut snapshot = source.clone();
    for ticks in cuts {
        advance_with(&mut snapshot, *ticks, EffectAdvanceLimits::default())?;
    }
    Ok(snapshot)
}

fn tick_oracle(source: &WorldSnapshot, ticks: u64) -> Result<WorldSnapshot> {
    let mut snapshot = source.clone();
    for _ in 0..ticks {
        advance_with(&mut snapshot, 1, EffectAdvanceLimits::default())?;
    }
    Ok(snapshot)
}

fn physical_graph(snapshot: &WorldSnapshot) -> WorldGraph {
    let mut graph = snapshot.graph.clone();
    for entity in graph.entities.values_mut() {
        entity.revision = 0;
        for fact in entity.fields.values_mut() {
            fact.observed_at = GameTick(0);
        }
    }
    for chunk in graph.chunks.values_mut() {
        chunk.revision = 0;
    }
    graph
}

fn assert_physical(left: &WorldSnapshot, right: &WorldSnapshot) {
    assert_eq!(left.tick, right.tick);
    assert_eq!(left.paused, right.paused);
    // Include progress counters, blockers, lifecycle, needs, life state and all
    // terrain values. Only publication revisions and observation stamps differ.
    assert_eq!(physical_graph(left), physical_graph(right));
}

fn number(snapshot: &WorldSnapshot, id: EntityId, name: &str) -> Result<u64> {
    field_u64(entity(snapshot, id)?, name, snapshot.tick)
}

fn below(threshold: u32) -> WorkOrderCondition {
    WorkOrderCondition::ItemCountBelow {
        item_token: "DRINK".to_owned(),
        threshold,
    }
}

#[test]
fn prerequisite_completion_only_releases_later_intervals_in_either_entity_order() -> Result<()> {
    for (first, second) in [(100, 200), (200, 100)] {
        let mut source = world()?;
        let first = EntityId::new(first);
        let second = EntityId::new(second);
        order(
            &mut source,
            first,
            "prerequisite",
            "MAKE_BARREL",
            1,
            0,
            Vec::new(),
        )?;
        order(
            &mut source,
            second,
            "dependent",
            "MAKE_BARREL",
            1,
            0,
            vec![WorkOrderCondition::CompletedOrder {
                order_name: "prerequisite".to_owned(),
            }],
        )?;
        let whole = after(&source, &[75])?;
        assert_eq!(number(&whole, first, AMOUNT_REMAINING_FIELD)?, 0);
        assert_eq!(number(&whole, second, AMOUNT_REMAINING_FIELD)?, 1);
        assert_eq!(number(&whole, second, "work_ticks")?, 25);
        assert_eq!(
            whole.graph.entities[&first].fields[STATUS_FIELD].observed_at,
            GameTick(1_050)
        );
        assert_physical(&whole, &after(&source, &[17, 8, 25, 1, 24])?);
        assert_physical(&whole, &tick_oracle(&source, 75)?);
    }
    Ok(())
}

#[test]
fn workshop_completion_does_not_retroactively_award_brewing_time() -> Result<()> {
    let mut source = world()?;
    set(
        &mut source,
        STILL,
        CONSTRUCTION_STAGE_FIELD,
        Value::Text(STAGE_PLANNED.to_owned()),
    )?;
    set(&mut source, STILL, "progress_ticks", Value::U64(0))?;
    set(
        &mut source,
        STILL,
        "required_ticks",
        Value::U64(BUILD_TICKS),
    )?;
    let brew = EntityId::new(100);
    order(&mut source, brew, "brew", "BREW_DRINK", 3, 0, Vec::new())?;
    let boundary = after(&source, &[BUILD_TICKS])?;
    assert_eq!(number(&boundary, brew, "work_ticks")?, 0);
    assert_eq!(number(&boundary, LEDGER, STOCK_DRINK_FIELD)?, 0);
    assert_eq!(
        boundary.graph.entities[&STILL].fields[CONSTRUCTION_STAGE_FIELD].observed_at,
        GameTick(1_500)
    );
    let whole = after(&source, &[650])?;
    assert_eq!(number(&whole, LEDGER, STOCK_DRINK_FIELD)?, 15);
    assert_eq!(
        whole.graph.entities[&brew].fields[STATUS_FIELD].observed_at,
        GameTick(1_650)
    );
    assert_physical(&whole, &after(&source, &[100, 399, 1, 50, 100])?);
    assert_physical(&whole, &tick_oracle(&source, 650)?);
    Ok(())
}

#[test]
fn consumption_releases_stock_conditions_at_the_actual_meal_boundary() -> Result<()> {
    let mut source = world()?;
    set(&mut source, LEDGER, STOCK_DRINK_FIELD, Value::U64(10))?;
    let brew = EntityId::new(100);
    order(
        &mut source,
        brew,
        "stock-gated brew",
        "BREW_DRINK",
        2,
        0,
        vec![below(10)],
    )?;
    let meal = after(&source, &[1_200])?;
    assert_eq!(number(&meal, LEDGER, STOCK_DRINK_FIELD)?, 8);
    assert_eq!(number(&meal, brew, "work_ticks")?, 0);
    let whole = after(&source, &[1_250])?;
    assert_eq!(number(&whole, LEDGER, STOCK_DRINK_FIELD)?, 13);
    assert_eq!(number(&whole, brew, AMOUNT_REMAINING_FIELD)?, 1);
    assert_physical(&whole, &after(&source, &[1_199, 1, 13, 37])?);
    assert_physical(&whole, &tick_oracle(&source, 1_250)?);
    Ok(())
}

#[test]
fn killed_workers_stop_production_after_their_last_live_interval() -> Result<()> {
    let mut source = world()?;
    let brew = EntityId::new(100);
    order(
        &mut source,
        brew,
        "brew before attack",
        "BREW_DRINK",
        20,
        0,
        Vec::new(),
    )?;
    threat(&mut source, EntityId::new(300), 0)?;
    threat(&mut source, EntityId::new(301), 50)?;
    let whole = after(&source, &[1_000])?;
    assert_eq!(number(&whole, LEDGER, STOCK_DRINK_FIELD)?, 30);
    assert_eq!(number(&whole, brew, AMOUNT_REMAINING_FIELD)?, 14);
    assert_eq!(number(&whole, brew, "work_ticks")?, 0);
    assert_eq!(
        whole.graph.entities[&BREWER].fields["alive"].value,
        Value::Bool(false)
    );
    assert_eq!(
        whole.graph.entities[&BREWER].fields["alive"].observed_at,
        GameTick(1_300)
    );
    assert_eq!(
        whole.graph.entities[&CITIZEN].fields["alive"].observed_at,
        GameTick(1_350)
    );
    assert!(
        field_text(entity(&whole, brew)?, BLOCKED_BY_FIELD, whole.tick)
            .is_some_and(|reason| reason.contains("living unit"))
    );
    assert_physical(&whole, &after(&source, &[49, 1, 249, 1, 1, 699])?);
    assert_physical(&whole, &tick_oracle(&source, 1_000)?);
    Ok(())
}

#[test]
fn competing_simultaneous_units_keep_partial_work_without_overshooting_stock_gate() -> Result<()> {
    let mut source = world()?;
    parallel_brewing_capacity(&mut source)?;
    let first = EntityId::new(100);
    let second = EntityId::new(200);
    order(
        &mut source,
        first,
        "first brew",
        "BREW_DRINK",
        4,
        40,
        vec![below(5)],
    )?;
    order(
        &mut source,
        second,
        "second brew",
        "BREW_DRINK",
        4,
        40,
        vec![below(5)],
    )?;
    let first_meeting = after(&source, &[50])?;
    assert_eq!(number(&first_meeting, LEDGER, STOCK_DRINK_FIELD)?, 5);
    assert_eq!(number(&first_meeting, first, AMOUNT_REMAINING_FIELD)?, 3);
    assert_eq!(number(&first_meeting, second, AMOUNT_REMAINING_FIELD)?, 4);
    assert_eq!(number(&first_meeting, second, "work_ticks")?, 49);
    assert_physical(&first_meeting, &after(&source, &[1, 8, 1, 17, 23])?);
    assert_physical(&first_meeting, &tick_oracle(&source, 50)?);
    let resumed = after(&source, &[1_250])?;
    // The meal opens the gate at 1200. The losing order needs only its final
    // tick; its competitor retains the one tick earned before stock fills.
    assert_eq!(number(&resumed, LEDGER, STOCK_DRINK_FIELD)?, 8);
    assert_eq!(number(&resumed, first, "work_ticks")?, 1);
    assert_eq!(number(&resumed, second, "work_ticks")?, 0);
    assert_eq!(number(&resumed, second, AMOUNT_REMAINING_FIELD)?, 3);
    assert_physical(&resumed, &after(&source, &[3, 7, 1_190, 1, 49])?);
    assert_physical(&resumed, &tick_oracle(&source, 1_250)?);
    Ok(())
}

#[test]
fn competing_orders_with_unequal_partial_work_finish_in_time_order() -> Result<()> {
    let mut source = world()?;
    parallel_brewing_capacity(&mut source)?;
    let first = EntityId::new(100);
    let second = EntityId::new(200);
    order(
        &mut source,
        first,
        "earlier id",
        "BREW_DRINK",
        4,
        20,
        vec![below(5)],
    )?;
    order(
        &mut source,
        second,
        "earlier completion",
        "BREW_DRINK",
        4,
        40,
        vec![below(5)],
    )?;
    let first_meeting = after(&source, &[50])?;
    assert_eq!(number(&first_meeting, first, AMOUNT_REMAINING_FIELD)?, 4);
    assert_eq!(number(&first_meeting, first, "work_ticks")?, 30);
    assert_eq!(number(&first_meeting, second, AMOUNT_REMAINING_FIELD)?, 3);
    assert_eq!(number(&first_meeting, LEDGER, STOCK_DRINK_FIELD)?, 5);
    assert_physical(&first_meeting, &tick_oracle(&source, 50)?);
    let resumed = after(&source, &[1_250])?;
    assert_eq!(number(&resumed, first, AMOUNT_REMAINING_FIELD)?, 3);
    assert_eq!(number(&resumed, first, "work_ticks")?, 0);
    assert_eq!(number(&resumed, second, "work_ticks")?, 20);
    assert_eq!(number(&resumed, LEDGER, STOCK_DRINK_FIELD)?, 8);
    assert_physical(&resumed, &after(&source, &[4, 5, 1, 1_191, 19, 30])?);
    assert_physical(&resumed, &tick_oracle(&source, 1_250)?);
    Ok(())
}

#[test]
fn meal_then_combat_phase_order_is_the_same_at_a_shared_boundary() -> Result<()> {
    let mut source = world()?;
    set(&mut source, LEDGER, STOCK_DRINK_FIELD, Value::U64(1))?;
    let hostile = EntityId::new(300);
    threat(&mut source, hostile, 900)?;
    let whole = after(&source, &[1_250])?;
    assert_eq!(
        whole.graph.entities[&hostile].fields[THREAT_STATUS_FIELD].observed_at,
        GameTick(1_900)
    );
    assert_eq!(number(&whole, hostile, COMBAT_ROUNDS_FIELD)?, 3);
    assert_eq!(number(&whole, LEDGER, STOCK_DRINK_FIELD)?, 0);
    assert_eq!(
        whole.graph.entities[&CITIZEN].fields[NEED_DRINK_FIELD].value,
        Value::Text("satisfied".to_owned())
    );
    assert_eq!(
        whole.graph.entities[&BREWER].fields[NEED_DRINK_FIELD].value,
        Value::Text("thirsty".to_owned())
    );
    assert_eq!(
        whole.graph.entities[&BREWER].fields["alive"].observed_at,
        GameTick(2_200)
    );
    assert_physical(&whole, &after(&source, &[899, 1, 100, 200, 50])?);
    assert_physical(&whole, &tick_oracle(&source, 1_250)?);
    Ok(())
}

#[test]
fn overlapping_excavations_keep_terrain_and_remaining_counts_partition_invariant() -> Result<()> {
    let mut source = world()?;
    let chunk = ChunkCoord { x: 0, y: 0, z: 0 };
    source
        .graph
        .chunks
        .insert(chunk, uniform_chunk(chunk, tile_codes::SOLID_WALL));
    for (key, min, max) in [("west", 0, 7), ("east", 4, 11)] {
        apply_effect(
            &mut source,
            &Action::DesignateDig {
                area: MapCuboid::new(MapCoord::new(min, 0, 0), MapCoord::new(max, 0, 0))?,
                mode: DigMode::Mine,
            },
            key,
        )?;
    }
    set(
        &mut source,
        created_entity_id("east", 0),
        "work_ticks",
        Value::U64(5),
    )?;
    let whole = after(&source, &[87])?;
    for key in ["west", "east"] {
        let id = created_entity_id(key, 0);
        assert_eq!(number(&whole, id, TILES_REMAINING_FIELD)?, 0);
        assert_eq!(
            whole.graph.entities[&id].fields[STATUS_FIELD].value,
            Value::Text(STATUS_COMPLETE.to_owned())
        );
    }
    for x in 0..12 {
        assert_eq!(
            whole.tile_code_at(MapCoord::new(x, 0, 0)),
            Some(tile_codes::FLOOR)
        );
    }
    assert_physical(&whole, &after(&source, &[4, 1, 9, 1, 17, 55])?);
    assert_physical(&whole, &tick_oracle(&source, 87)?);
    Ok(())
}

#[test]
fn excavation_budget_scales_with_changed_tiles_instead_of_repeated_region_scans() -> Result<()> {
    let mut source = world()?;
    for x in 0..2 {
        for y in 0..2 {
            let chunk = ChunkCoord { x, y, z: 0 };
            source
                .graph
                .chunks
                .insert(chunk, uniform_chunk(chunk, tile_codes::SOLID_WALL));
        }
    }
    let area = MapCuboid::new(MapCoord::new(0, 0, 0), MapCoord::new(31, 31, 0))?;
    apply_effect(
        &mut source,
        &Action::DesignateDig {
            area,
            mode: DigMode::Mine,
        },
        "large dig",
    )?;
    let mut advanced = source.clone();
    advance_with(
        &mut advanced,
        10_240,
        EffectAdvanceLimits {
            max_game_ticks: 20_000,
            max_events: 2_000,
            max_work_units: 500_000,
        },
    )?;
    assert_eq!(
        number(
            &advanced,
            created_entity_id("large dig", 0),
            TILES_REMAINING_FIELD
        )?,
        0
    );
    assert!(
        region_tiles(area).all(|coord| advanced.tile_code_at(coord) == Some(tile_codes::FLOOR))
    );
    Ok(())
}

#[test]
fn future_source_facts_cannot_be_promoted_by_the_destination_clock() -> Result<()> {
    for on_edge in [false, true] {
        let mut source = world()?;
        let future = known(Value::Bool(true), GameTick(source.tick.0 + 25));
        if on_edge {
            let edge = EdgeId::new(1);
            source.graph.edges.insert(
                edge,
                EdgeRecord {
                    id: edge,
                    revision: 1,
                    kind: EdgeKind::AssignedTo,
                    from: CITIZEN,
                    to: BREWER,
                    fields: BTreeMap::from([("future".to_owned(), future)]),
                },
            );
        } else {
            source
                .graph
                .entities
                .get_mut(&CITIZEN)
                .ok_or_else(|| precondition("test citizen is missing"))?
                .fields
                .insert("future".to_owned(), future);
        }
        source.refresh_hash();
        let before = source.clone();
        let error = advance_with(&mut source, 50, EffectAdvanceLimits::default()).err();
        assert_eq!(
            error.map(|error| error.code),
            Some(ErrorCode::PreconditionsFailed)
        );
        assert_eq!(source, before);
    }
    Ok(())
}

#[test]
fn aggregate_excavation_cache_bound_refuses_before_reading_or_allocating_regions() -> Result<()> {
    let mut source = world()?;
    let tiles = 65_536;
    let count = MAX_EFFECT_ADVANCE_CACHED_DIG_TILES / tiles + 1;
    // No terrain chunks are installed. A per-region allocation/read before
    // aggregate admission would fail with unknown terrain instead of this
    // budget refusal; the regression does not allocate a million-tile cache.
    for ordinal in 0..count {
        put(
            &mut source,
            EntityId::new(200 + ordinal),
            EntityKind::Other(DIG_DESIGNATION_KIND.to_owned()),
            vec![
                ("area_min", Value::Coord(MapCoord::new(0, 0, 0))),
                ("area_max", Value::Coord(MapCoord::new(255, 255, 0))),
                ("target_tile_code", Value::U64(u64::from(tile_codes::FLOOR))),
                ("work_ticks", Value::U64(0)),
                (TILES_REMAINING_FIELD, Value::U64(tiles)),
                (STATUS_FIELD, Value::Text(STATUS_ACTIVE.to_owned())),
            ],
        )?;
    }
    let before = source.clone();
    let error = advance_with(&mut source, 1, EffectAdvanceLimits::default())
        .err()
        .ok_or_else(|| precondition("oversized excavation cache was admitted"))?;
    assert_eq!(error.code, ErrorCode::BudgetExceeded);
    assert!(
        error
            .message
            .contains("aggregate cached excavation footprint")
    );
    assert_eq!(source, before);
    Ok(())
}

#[test]
fn timeline_budgets_and_clock_arithmetic_refuse_without_publishing_partial_events() -> Result<()> {
    let mut source = world()?;
    order(
        &mut source,
        EntityId::new(100),
        "bounded",
        "MAKE_BARREL",
        3,
        0,
        Vec::new(),
    )?;
    let before = source.clone();
    for limits in [
        EffectAdvanceLimits {
            max_events: 1,
            ..EffectAdvanceLimits::default()
        },
        EffectAdvanceLimits {
            max_work_units: 1,
            ..EffectAdvanceLimits::default()
        },
        EffectAdvanceLimits {
            max_game_ticks: 99,
            ..EffectAdvanceLimits::default()
        },
    ] {
        let error = advance_with(&mut source, 100, limits).err();
        assert_eq!(
            error.map(|error| error.code),
            Some(ErrorCode::BudgetExceeded)
        );
        assert_eq!(source, before);
    }
    let error = advance_with(
        &mut source,
        MAX_EFFECT_ADVANCE_TICKS + 1,
        EffectAdvanceLimits::default(),
    )
    .err();
    assert_eq!(
        error.map(|error| error.code),
        Some(ErrorCode::BudgetExceeded)
    );
    assert_eq!(source, before);
    let mut underflow = source.clone();
    underflow.tick = GameTick(10);
    let graph = underflow.graph.clone();
    let error = advance_effects(&mut underflow, 11).err();
    assert_eq!(
        error.map(|error| error.code),
        Some(ErrorCode::InvalidRequest)
    );
    assert_eq!(underflow.graph, graph);
    assert_eq!(underflow.tick, GameTick(10));
    set(
        &mut source,
        LEDGER,
        METABOLISM_TICKS_FIELD,
        Value::U64(u64::MAX - 3),
    )?;
    let before_overflow = source.clone();
    let error = advance_with(&mut source, 10, EffectAdvanceLimits::default()).err();
    assert_eq!(
        error.map(|error| error.code),
        Some(ErrorCode::BudgetExceeded)
    );
    assert_eq!(source, before_overflow);
    Ok(())
}


fn production_workshop(
    snapshot: &mut WorldSnapshot,
    id: EntityId,
    kind: &str,
) -> Result<()> {
    put(
        snapshot,
        id,
        EntityKind::Building,
        vec![
            ("building_kind", Value::Text(kind.to_owned())),
            (
                CONSTRUCTION_STAGE_FIELD,
                Value::Text(STAGE_COMPLETE.to_owned()),
            ),
        ],
    )
}

fn parallel_brewing_capacity(snapshot: &mut WorldSnapshot) -> Result<()> {
    // Reuse the existing citizen so simultaneous-production fixtures retain
    // their original population and exact consumption behavior.
    set(snapshot, CITIZEN, "labor.BREW", Value::Bool(true))?;
    production_workshop(snapshot, EntityId::new(52), "workshop:Still")
}

#[test]
fn production_worker_and_workshop_capacity_independently_limit_simultaneous_orders() -> Result<()> {
    for workers in 1..=2 {
        for workshops in 1..=2 {
            let mut source = world()?;
            if workers == 2 {
                set(&mut source, CITIZEN, "labor.BREW", Value::Bool(true))?;
            }
            if workshops == 2 {
                production_workshop(&mut source, EntityId::new(52), "workshop:Still")?;
            }
            for id in [100, 200, 300] {
                order(
                    &mut source,
                    EntityId::new(id),
                    &format!("brew {id}"),
                    "BREW_DRINK",
                    1,
                    0,
                    Vec::new(),
                )?;
            }
            let service = workers.min(workshops);
            let middle = after(&source, &[25])?;
            for (index, id) in [100, 200, 300].into_iter().enumerate() {
                let id = EntityId::new(id);
                assert_eq!(
                    number(&middle, id, "work_ticks")?,
                    if index < service { 25 } else { 0 }
                );
                if index >= service {
                    let blocker = field_text(entity(&middle, id)?, BLOCKED_BY_FIELD, middle.tick);
                    assert!(blocker.is_some_and(|reason| reason.contains("production capacity busy")));
                    assert!(blocker.is_some_and(|reason| reason.contains(
                        if workshops <= workers { "workshop:Still" } else { "living units" }
                    )));
                }
            }
            let whole = after(&source, &[75])?;
            assert_eq!(
                number(&whole, LEDGER, STOCK_DRINK_FIELD)?,
                service as u64 * 5
            );
            assert_eq!(
                number(&whole, EntityId::new(if service == 1 { 200 } else { 300 }), "work_ticks")?,
                25
            );
            assert_physical(&whole, &after(&source, &[1, 24, 24, 1, 25])?);
            assert_physical(&whole, &tick_oracle(&source, 75)?);
            let complete = after(&source, &[150])?;
            assert_eq!(number(&complete, LEDGER, STOCK_DRINK_FIELD)?, 15);
        }
    }
    Ok(())
}

#[test]
fn one_multiskilled_worker_cannot_brew_and_cook_in_the_same_interval() -> Result<()> {
    let mut source = world()?;
    set(&mut source, BREWER, "labor.COOK", Value::Bool(true))?;
    production_workshop(&mut source, EntityId::new(52), "workshop:Kitchen")?;
    order(&mut source, EntityId::new(100), "brew", "BREW_DRINK", 1, 0, Vec::new())?;
    order(&mut source, EntityId::new(200), "cook", "COOK_MEAL", 1, 0, Vec::new())?;
    let middle = after(&source, &[25])?;
    assert_eq!(number(&middle, EntityId::new(100), "work_ticks")?, 25);
    assert_eq!(number(&middle, EntityId::new(200), "work_ticks")?, 0);
    assert!(
        field_text(entity(&middle, EntityId::new(200))?, BLOCKED_BY_FIELD, middle.tick)
            .is_some_and(|reason| reason.contains("living units with COOK"))
    );
    let whole = after(&source, &[75])?;
    assert_eq!(number(&whole, LEDGER, STOCK_DRINK_FIELD)?, 5);
    assert_eq!(number(&whole, LEDGER, STOCK_FOOD_FIELD)?, 100);
    assert_eq!(number(&whole, EntityId::new(200), "work_ticks")?, 25);
    assert_physical(&whole, &after(&source, &[17, 32, 1, 25])?);
    assert_physical(&whole, &tick_oracle(&source, 75)?);
    assert_eq!(number(&after(&source, &[100])?, LEDGER, STOCK_FOOD_FIELD)?, 105);
    Ok(())
}

#[test]
fn production_matching_moves_a_flexible_worker_to_keep_a_specialist_productive() -> Result<()> {
    let mut source = world()?;
    // The lower-ID citizen is considered first for brewing, but is the only
    // cook. The matching must move brewing to the other citizen.
    set(&mut source, CITIZEN, "labor.BREW", Value::Bool(true))?;
    set(&mut source, CITIZEN, "labor.COOK", Value::Bool(true))?;
    production_workshop(&mut source, EntityId::new(52), "workshop:Kitchen")?;
    order(&mut source, EntityId::new(100), "brew", "BREW_DRINK", 1, 0, Vec::new())?;
    order(&mut source, EntityId::new(200), "cook", "PREPARE_MEAL", 1, 0, Vec::new())?;
    let whole = after(&source, &[50])?;
    assert_eq!(number(&whole, LEDGER, STOCK_DRINK_FIELD)?, 5);
    assert_eq!(number(&whole, LEDGER, STOCK_FOOD_FIELD)?, 105);
    assert_eq!(number(&whole, EntityId::new(100), AMOUNT_REMAINING_FIELD)?, 0);
    assert_eq!(number(&whole, EntityId::new(200), AMOUNT_REMAINING_FIELD)?, 0);
    assert_physical(&whole, &after(&source, &[1, 17, 31, 1])?);
    Ok(())
}

#[test]
fn production_capacity_preserves_earned_partial_work_and_does_not_bank_waiting_ticks() -> Result<()> {
    let mut source = world()?;
    order(&mut source, EntityId::new(100), "new", "BREW_DRINK", 1, 0, Vec::new())?;
    order(&mut source, EntityId::new(200), "started", "BREW_DRINK", 1, 40, Vec::new())?;
    let boundary = after(&source, &[10])?;
    assert_eq!(number(&boundary, EntityId::new(200), AMOUNT_REMAINING_FIELD)?, 0);
    assert_eq!(number(&boundary, EntityId::new(100), "work_ticks")?, 0);
    let whole = after(&source, &[35])?;
    assert_eq!(number(&whole, EntityId::new(100), "work_ticks")?, 25);
    assert_eq!(number(&whole, LEDGER, STOCK_DRINK_FIELD)?, 5);
    assert_physical(&whole, &after(&source, &[1, 8, 1, 25])?);
    assert_physical(&whole, &tick_oracle(&source, 35)?);

    let mut blocked = world()?;
    order(&mut blocked, EntityId::new(100), "gated", "BREW_DRINK", 1, 40, vec![below(0)])?;
    order(&mut blocked, EntityId::new(200), "ready", "BREW_DRINK", 1, 0, Vec::new())?;
    let completed = after(&blocked, &[50])?;
    assert_eq!(number(&completed, EntityId::new(100), "work_ticks")?, 40);
    assert_eq!(number(&completed, EntityId::new(200), AMOUNT_REMAINING_FIELD)?, 0);
    assert_eq!(number(&completed, LEDGER, STOCK_DRINK_FIELD)?, 5);
    assert_physical(&completed, &tick_oracle(&blocked, 50)?);
    Ok(())
}

#[test]
fn another_workshop_adds_capacity_only_after_its_actual_completion() -> Result<()> {
    let mut source = world()?;
    set(&mut source, CITIZEN, "labor.BREW", Value::Bool(true))?;
    let building = EntityId::new(52);
    production_workshop(&mut source, building, "workshop:Still")?;
    set(&mut source, building, CONSTRUCTION_STAGE_FIELD, Value::Text(STAGE_PLANNED.to_owned()))?;
    set(&mut source, building, "required_ticks", Value::U64(25))?;
    set(&mut source, building, "progress_ticks", Value::U64(0))?;
    for id in [100, 200] {
        order(&mut source, EntityId::new(id), &format!("brew {id}"), "BREW_DRINK", 1, 0, Vec::new())?;
    }
    let whole = after(&source, &[50])?;
    assert_eq!(number(&whole, EntityId::new(100), AMOUNT_REMAINING_FIELD)?, 0);
    assert_eq!(number(&whole, EntityId::new(200), "work_ticks")?, 25);
    assert_eq!(number(&whole, LEDGER, STOCK_DRINK_FIELD)?, 5);
    assert_physical(&whole, &after(&source, &[24, 1, 24, 1])?);
    assert_physical(&whole, &tick_oracle(&source, 50)?);
    assert_eq!(number(&after(&source, &[75])?, LEDGER, STOCK_DRINK_FIELD)?, 10);
    Ok(())
}

#[test]
fn worker_death_reduces_joint_production_capacity_at_the_next_interval() -> Result<()> {
    let mut source = world()?;
    parallel_brewing_capacity(&mut source)?;
    for id in [100, 200] {
        order(&mut source, EntityId::new(id), &format!("brew {id}"), "BREW_DRINK", 10, 0, Vec::new())?;
    }
    threat(&mut source, EntityId::new(300), 0)?;
    let whole = after(&source, &[350])?;
    assert_eq!(whole.graph.entities[&BREWER].fields["alive"].value, Value::Bool(false));
    assert_eq!(whole.graph.entities[&BREWER].fields["alive"].observed_at, GameTick(1_300));
    assert_eq!(number(&whole, EntityId::new(100), AMOUNT_REMAINING_FIELD)?, 3);
    assert_eq!(number(&whole, EntityId::new(200), AMOUNT_REMAINING_FIELD)?, 4);
    assert_eq!(number(&whole, LEDGER, STOCK_DRINK_FIELD)?, 65);
    assert_physical(&whole, &after(&source, &[49, 1, 249, 1, 1, 49])?);
    assert_physical(&whole, &tick_oracle(&source, 350)?);
    Ok(())
}

#[test]
fn ineligible_labor_and_workshop_facts_do_not_inflate_production_capacity() -> Result<()> {
    for (subject, field) in [
        (CITIZEN, "labor.BREW"),
        (EntityId::new(52), "building_kind"),
        (EntityId::new(52), CONSTRUCTION_STAGE_FIELD),
    ] {
        for asserted in [false, true] {
            let mut source = world()?;
            parallel_brewing_capacity(&mut source)?;
            for id in [100, 200] {
                order(&mut source, EntityId::new(id), &format!("brew {id}"), "BREW_DRINK", 1, 0, Vec::new())?;
            }
            let fact = source.graph.entities.get_mut(&subject).unwrap().fields.get_mut(field).unwrap();
            if asserted {
                fact.source = FactSource::AgentAssertion("invented extra capacity".to_owned());
            } else {
                fact.presence = Some(dfmcp_world::FactPresence::Omitted("not observed".to_owned()));
            }
            source.refresh_hash();
            let whole = after(&source, &[50])?;
            assert_eq!(number(&whole, LEDGER, STOCK_DRINK_FIELD)?, 5);
            assert_eq!(number(&whole, EntityId::new(200), "work_ticks")?, 0);
            assert_physical(&whole, &tick_oracle(&source, 50)?);
        }
    }
    Ok(())
}

fn exhaustive_production_capacity(
    masks: &[u8; 3],
    families: &[usize; 3],
    index: usize,
    used: u8,
    workshops: [usize; 2],
) -> usize {
    if index == families.len() {
        return 0;
    }
    let family = families[index];
    let mut best = exhaustive_production_capacity(masks, families, index + 1, used, workshops);
    if workshops[family] == 0 {
        return best;
    }
    for worker in 0..masks.len() {
        if used & (1 << worker) == 0 && masks[worker] & (1 << family) != 0 {
            let mut remaining = workshops;
            remaining[family] -= 1;
            best = best.max(1 + exhaustive_production_capacity(
                masks,
                families,
                index + 1,
                used | (1 << worker),
                remaining,
            ));
        }
    }
    best
}

#[test]
fn production_capacity_matches_exhaustive_assignment_for_every_three_worker_skill_graph() -> Result<()> {
    // Each graph assigns either/both/neither registered labor to three people.
    // Independent exhaustive assignment includes unfilled orders and explicit
    // workshop slots; it does not use augmenting paths or the production code.
    let families = [0, 1, 0];
    for encoded in 0..64u8 {
        let masks = [encoded & 3, (encoded >> 2) & 3, (encoded >> 4) & 3];
        for workshops in [[0, 0], [0, 2], [2, 0], [1, 1], [1, 2], [2, 1], [2, 2], [3, 3]] {
            let mut source = world()?;
            source.graph.entities.remove(&STILL);
            put(
                &mut source,
                EntityId::new(13),
                EntityKind::Unit,
                vec![("alive", Value::Bool(true)), (SQUAD_FIELD, Value::Null)],
            )?;
            for (worker, mask) in [CITIZEN, BREWER, EntityId::new(13)].into_iter().zip(masks) {
                set(&mut source, worker, "labor.BREW", Value::Bool(mask & 1 != 0))?;
                set(&mut source, worker, "labor.COOK", Value::Bool(mask & 2 != 0))?;
            }
            for (family, count) in workshops.into_iter().enumerate() {
                for slot in 0..count {
                    production_workshop(
                        &mut source,
                        EntityId::new(50 + family as u128 * 10 + slot as u128),
                        if family == 0 { "workshop:Still" } else { "workshop:Kitchen" },
                    )?;
                }
            }
            for (index, family) in families.into_iter().enumerate() {
                let id = EntityId::new(100 + index as u128);
                order(
                    &mut source,
                    id,
                    &format!("order {index}"),
                    if family == 0 { "BREW_DRINK" } else { "PREPARE_MEAL" },
                    1,
                    0,
                    Vec::new(),
                )?;
            }
            let expected = exhaustive_production_capacity(&masks, &families, 0, 0, workshops);
            let result = after(&source, &[50])?;
            let completed = [100, 101, 102].into_iter().filter(|id| {
                number(&result, EntityId::new(*id), AMOUNT_REMAINING_FIELD) == Ok(0)
            }).count();
            assert_eq!(completed, expected, "masks={masks:?}, workshops={workshops:?}");
            assert_eq!(
                number(&result, LEDGER, STOCK_DRINK_FIELD)?
                    + number(&result, LEDGER, STOCK_FOOD_FIELD)? - 100,
                expected as u64 * 5,
            );
        }
    }
    Ok(())
}

#[test]
fn production_capacity_budget_refusal_discards_already_completed_internal_work() -> Result<()> {
    let mut source = world()?;
    for id in [100, 200] {
        order(&mut source, EntityId::new(id), &format!("brew {id}"), "BREW_DRINK", 1, 0, Vec::new())?;
    }
    let before = source.clone();
    for limits in [
        EffectAdvanceLimits {
            max_events: 1,
            ..EffectAdvanceLimits::default()
        },
        EffectAdvanceLimits {
            max_work_units: 1,
            ..EffectAdvanceLimits::default()
        },
    ] {
        let error = advance_with(&mut source, 100, limits).err();
        assert_eq!(error.map(|error| error.code), Some(ErrorCode::BudgetExceeded));
        assert_eq!(source, before);
    }
    let complete = after(&source, &[100])?;
    assert_eq!(number(&complete, LEDGER, STOCK_DRINK_FIELD)?, 10);
    Ok(())
}
