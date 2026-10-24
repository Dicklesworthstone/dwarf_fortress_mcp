use super::*;
use dfmcp_core::ObservationCursor;
use dfmcp_world::terrain::uniform_chunk;
use dfmcp_world::{ChunkCoord, FactPresence, WorldGraph};

const WORKER: EntityId = EntityId::new(11);
const LEDGER: EntityId = EntityId::new(91);
const DIG_TILE: MapCoord = MapCoord::new(4, 4, 10);
const BUILD_TILE: MapCoord = MapCoord::new(6, 6, 10);
const LABORS: [&str; 5] = ["MINING", "CONSTRUCTION", "BREW", "COOK", "CARPENTRY"];

fn put(
    snapshot: &mut WorldSnapshot,
    id: EntityId,
    kind: EntityKind,
    fields: Vec<(&str, Value)>,
) -> Result<()> {
    create_entity(
        snapshot,
        id,
        kind.clone(),
        kind.as_str().to_owned(),
        fields
            .into_iter()
            .map(|(field, value)| (field.to_owned(), value))
            .collect(),
    )?;
    snapshot.refresh_hash();
    Ok(())
}

fn world(shared: bool) -> Result<WorldSnapshot> {
    let mut graph = WorldGraph::default();
    let chunk = ChunkCoord { x: 0, y: 0, z: 10 };
    graph
        .chunks
        .insert(chunk, uniform_chunk(chunk, tile_codes::FLOOR));
    let mut snapshot = WorldSnapshot::new(
        FortressId::new(9),
        GameTick(1_000),
        ObservationCursor::ORIGIN,
        false,
        graph,
    );
    for (id, kind, position) in [
        (51, "Still", MapCoord::new(0, 0, 10)),
        (52, "Kitchen", MapCoord::new(1, 0, 10)),
        (53, "Carpenters", MapCoord::new(2, 0, 10)),
    ] {
        put(
            &mut snapshot,
            EntityId::new(id),
            EntityKind::Building,
            vec![
                ("building_kind", Value::Text(format!("workshop:{kind}"))),
                (
                    CONSTRUCTION_STAGE_FIELD,
                    Value::Text(STAGE_COMPLETE.to_owned()),
                ),
                ("position", Value::Coord(position)),
                ("footprint_min", Value::Coord(position)),
                ("footprint_max", Value::Coord(position)),
            ],
        )?;
    }
    put(
        &mut snapshot,
        LEDGER,
        EntityKind::Other(STOCK_LEDGER_KIND.to_owned()),
        vec![
            (STOCK_DRINK_FIELD, Value::U64(100)),
            (STOCK_FOOD_FIELD, Value::U64(100)),
            (METABOLISM_TICKS_FIELD, Value::U64(0)),
            (STOCK_WOOD_FIELD, Value::U64(20)),
            (STOCK_BED_FIELD, Value::U64(0)),
            (STOCK_DOOR_FIELD, Value::U64(0)),
            (STOCK_TABLE_FIELD, Value::U64(0)),
            (STOCK_CHAIR_FIELD, Value::U64(0)),
        ],
    )?;
    if shared {
        let settings = fortress_settings_entity_id(snapshot.fortress_id);
        put(
            &mut snapshot,
            settings,
            EntityKind::Other(FORTRESS_SETTINGS_KIND.to_owned()),
            vec![(
                LABORATORY_WORKFORCE_MODEL_FIELD,
                Value::Text(LABORATORY_WORKFORCE_MODEL.to_owned()),
            )],
        )?;
    }
    Ok(snapshot)
}

fn worker(snapshot: &mut WorldSnapshot, id: EntityId, labors: &[&str]) -> Result<()> {
    let fields = std::iter::once(("alive".to_owned(), Value::Bool(true)))
        .chain(std::iter::once((SQUAD_FIELD.to_owned(), Value::Null)))
        .chain(LABORS.into_iter().map(|labor| {
            (
                format!("{LABOR_FIELD_PREFIX}{labor}"),
                Value::Bool(labors.contains(&labor)),
            )
        }))
        .collect();
    create_entity(snapshot, id, EntityKind::Unit, "worker".to_owned(), fields)?;
    snapshot.refresh_hash();
    Ok(())
}

fn set(snapshot: &mut WorldSnapshot, id: EntityId, field: &str, value: Value) -> Result<()> {
    write_fields(snapshot, id, vec![(field.to_owned(), value)])?;
    snapshot.refresh_hash();
    Ok(())
}

fn record_mut(snapshot: &mut WorldSnapshot, id: EntityId) -> Result<&mut EntityRecord> {
    snapshot
        .graph
        .entities
        .get_mut(&id)
        .ok_or_else(|| precondition("workforce test record missing"))
}

fn fact_mut<'a>(
    snapshot: &'a mut WorldSnapshot,
    id: EntityId,
    field: &str,
) -> Result<&'a mut Fact> {
    record_mut(snapshot, id)?
        .fields
        .get_mut(field)
        .ok_or_else(|| precondition("workforce test fact missing"))
}

fn apply(snapshot: &mut WorldSnapshot, action: &Action, key: &str) -> Result<()> {
    let mut shadow = snapshot.clone();
    apply_effect(&mut shadow, action, key)?;
    shadow.refresh_hash();
    *snapshot = shadow;
    Ok(())
}

fn cancel(snapshot: &mut WorldSnapshot, action: &Action, key: &str) -> Result<()> {
    let mut shadow = snapshot.clone();
    cancel_effect(&mut shadow, action, key)?;
    shadow.refresh_hash();
    *snapshot = shadow;
    Ok(())
}

fn advance_with(
    snapshot: &mut WorldSnapshot,
    ticks: u64,
    limits: EffectAdvanceLimits,
) -> Result<()> {
    // A foreground publication owns a shadow. A later refused internal event
    // must discard every earlier event in the same requested advance.
    let mut shadow = snapshot.clone();
    shadow.tick = GameTick(
        snapshot
            .tick
            .0
            .checked_add(ticks)
            .ok_or_else(|| advance_budget_error("workforce test clock overflow"))?,
    );
    advance_effects_with_limits(&mut shadow, ticks, limits)?;
    shadow.refresh_hash();
    *snapshot = shadow;
    Ok(())
}

fn advance(snapshot: &mut WorldSnapshot, ticks: u64) -> Result<()> {
    advance_with(snapshot, ticks, EffectAdvanceLimits::default())
}

fn after(source: &WorldSnapshot, partitions: &[u64]) -> Result<WorldSnapshot> {
    let mut result = source.clone();
    for ticks in partitions {
        advance(&mut result, *ticks)?;
    }
    Ok(result)
}

fn physical(snapshot: &WorldSnapshot) -> WorldGraph {
    let mut graph = snapshot.graph.clone();
    for record in graph.entities.values_mut() {
        record.revision = 0;
        for fact in record.fields.values_mut() {
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
    assert_eq!(physical(left), physical(right));
}

fn number(snapshot: &WorldSnapshot, id: EntityId, field: &str) -> Result<u64> {
    field_u64(entity(snapshot, id)?, field, snapshot.tick)
}

fn dig(snapshot: &mut WorldSnapshot, key: &str, from: MapCoord, to: MapCoord) -> Result<EntityId> {
    let area = MapCuboid::new(from, to)?;
    for position in region_tiles(area) {
        snapshot.set_tile_code(position, tile_codes::SOLID_WALL)?;
    }
    apply(
        snapshot,
        &Action::DesignateDig {
            area,
            mode: DigMode::Mine,
        },
        key,
    )?;
    Ok(created_entity_id(key, 0))
}

fn building_action(position: MapCoord) -> Result<Action> {
    Ok(Action::Build {
        kind: BuildingKind::Furniture("Door".to_owned()),
        location: position,
        footprint: MapCuboid::new(position, position)?,
        material: crate::action::MaterialSelector::default(),
    })
}

fn build(snapshot: &mut WorldSnapshot, key: &str, position: MapCoord) -> Result<EntityId> {
    apply(snapshot, &building_action(position)?, key)?;
    Ok(created_entity_id(key, 0))
}

fn order(snapshot: &mut WorldSnapshot, key: &str, job: &str, amount: u32) -> Result<EntityId> {
    apply(
        snapshot,
        &Action::CreateWorkOrder {
            name: key.to_owned(),
            job_token: job.to_owned(),
            amount,
            conditions: Vec::new(),
        },
        key,
    )?;
    Ok(created_entity_id(key, 0))
}

fn assert_model_stamp(snapshot: &WorldSnapshot, id: EntityId) -> Result<()> {
    assert_eq!(
        field_text(
            entity(snapshot, id)?,
            LABORATORY_WORKFORCE_TASK_MODEL_FIELD,
            snapshot.tick,
        ),
        Some(LABORATORY_WORKFORCE_MODEL)
    );
    Ok(())
}

#[test]
fn opted_in_excavation_and_construction_wait_for_actual_labor_without_banking() -> Result<()> {
    let mut snapshot = world(true)?;
    let dig_id = dig(&mut snapshot, "idle-dig", DIG_TILE, DIG_TILE)?;
    let build_id = build(&mut snapshot, "idle-build", BUILD_TILE)?;
    assert_model_stamp(&snapshot, dig_id)?;
    assert_model_stamp(&snapshot, build_id)?;
    advance(&mut snapshot, BUILD_TICKS)?;
    assert_eq!(number(&snapshot, dig_id, "work_ticks")?, 0);
    assert_eq!(number(&snapshot, dig_id, TILES_REMAINING_FIELD)?, 1);
    assert_eq!(number(&snapshot, build_id, "progress_ticks")?, 0);
    for (id, labor) in [(dig_id, "MINING"), (build_id, "CONSTRUCTION")] {
        assert!(
            field_text(entity(&snapshot, id)?, BLOCKED_BY_FIELD, snapshot.tick)
                .is_some_and(|reason| reason.contains(labor))
        );
    }
    worker(&mut snapshot, WORKER, &["MINING", "CONSTRUCTION"])?;
    advance(&mut snapshot, BUILD_TICKS + DIG_TICKS_PER_TILE - 1)?;
    let unfinished = [
        number(&snapshot, dig_id, TILES_REMAINING_FIELD)? > 0,
        number(&snapshot, build_id, "progress_ticks")? < BUILD_TICKS,
    ];
    assert_eq!(unfinished.into_iter().filter(|value| *value).count(), 1);
    advance(&mut snapshot, 1)?;
    assert_eq!(number(&snapshot, dig_id, TILES_REMAINING_FIELD)?, 0);
    assert_eq!(number(&snapshot, build_id, "progress_ticks")?, BUILD_TICKS);
    Ok(())
}

#[test]
fn unknown_life_and_unqualified_or_inexact_labors_never_create_workers() -> Result<()> {
    for variant in 0..12 {
        let mut snapshot = world(true)?;
        // Without metabolism, unavailable population evidence blocks these
        // activities directly instead of independently refusing the meal census.
        snapshot.graph.entities.remove(&LEDGER);
        worker(&mut snapshot, WORKER, &["MINING", "CONSTRUCTION"])?;
        match variant {
            0 => {
                record_mut(&mut snapshot, WORKER)?.fields.remove("alive");
            }
            1 => set(&mut snapshot, WORKER, "alive", Value::Bool(false))?,
            2 => {
                fact_mut(&mut snapshot, WORKER, "alive")?.presence =
                    Some(FactPresence::Omitted("life not observed".to_owned()));
            }
            3 => {
                fact_mut(&mut snapshot, WORKER, "alive")?.source =
                    FactSource::AgentAssertion("assumed alive".to_owned());
            }
            4 => {
                fact_mut(&mut snapshot, WORKER, "alive")?.presence =
                    Some(FactPresence::Known(Value::Bool(false)));
            }
            _ => {
                for field in ["labor.MINING", "labor.CONSTRUCTION"] {
                    match variant {
                        5 => {
                            record_mut(&mut snapshot, WORKER)?.fields.remove(field);
                        }
                        6 => set(&mut snapshot, WORKER, field, Value::Bool(false))?,
                        7 => set(&mut snapshot, WORKER, field, Value::Text("true".to_owned()))?,
                        8 => {
                            fact_mut(&mut snapshot, WORKER, field)?.presence =
                                Some(FactPresence::Unknown("labor not observed".to_owned()));
                        }
                        9 => {
                            fact_mut(&mut snapshot, WORKER, field)?.source = FactSource::Replay;
                        }
                        10 => {
                            fact_mut(&mut snapshot, WORKER, field)?.presence =
                                Some(FactPresence::Known(Value::Bool(false)));
                        }
                        _ => {
                            fact_mut(&mut snapshot, WORKER, field)?.source_digest =
                                Digest32::of_bytes(b"unqualified labor");
                        }
                    }
                }
            }
        }
        let dig_id = dig(&mut snapshot, "unknown-dig", DIG_TILE, DIG_TILE)?;
        let build_id = build(&mut snapshot, "unknown-build", BUILD_TILE)?;
        let before_report = snapshot.clone();
        let report = laboratory_workforce_report(&snapshot)?
            .ok_or_else(|| precondition("opted-in workforce report missing"))?;
        assert_eq!(
            snapshot, before_report,
            "report mutated source, variant {variant}"
        );
        assert!(report.assignments.is_empty(), "variant {variant}");
        assert_eq!(report.blocked.len(), 2, "variant {variant}");
        advance(&mut snapshot, 25)?;
        assert_eq!(
            number(&snapshot, dig_id, "work_ticks")?,
            0,
            "variant {variant}"
        );
        assert_eq!(
            number(&snapshot, build_id, "progress_ticks")?,
            0,
            "variant {variant}"
        );
    }
    Ok(())
}

#[test]
fn all_five_labor_families_share_one_conserved_worker_slot() -> Result<()> {
    let mut snapshot = world(true)?;
    worker(&mut snapshot, WORKER, &LABORS)?;
    let dig_id = dig(&mut snapshot, "five-dig", DIG_TILE, DIG_TILE)?;
    let build_id = build(&mut snapshot, "five-build", BUILD_TILE)?;
    let brew = order(&mut snapshot, "five-brew", "BREW_DRINK", 1)?;
    let cook = order(&mut snapshot, "five-cook", "PREPARE_MEAL", 1)?;
    let carpentry = order(&mut snapshot, "five-carpentry", "CONSTRUCT_BED", 1)?;
    let before = snapshot.clone();
    let report = laboratory_workforce_report(&snapshot)?
        .ok_or_else(|| precondition("workforce report missing"))?;
    assert_eq!(snapshot, before);
    assert_eq!(report.active_activities, 5);
    assert_eq!(report.assignments.len(), 1);
    assert_eq!(report.assignments[0].worker, WORKER);
    assert_eq!(report.blocked.len(), 4);
    for id in [dig_id, build_id, brew, cook, carpentry] {
        assert_model_stamp(&snapshot, id)?;
    }
    advance(&mut snapshot, 1)?;
    let earned = number(&snapshot, dig_id, "work_ticks")?
        + number(&snapshot, build_id, "progress_ticks")?
        + number(&snapshot, brew, "work_ticks")?
        + number(&snapshot, cook, "work_ticks")?
        + number(&snapshot, carpentry, "work_ticks")?;
    assert_eq!(earned, 1);
    advance(&mut snapshot, 659)?;
    assert_eq!(number(&snapshot, dig_id, TILES_REMAINING_FIELD)?, 0);
    assert_eq!(number(&snapshot, build_id, "progress_ticks")?, BUILD_TICKS);
    for id in [brew, cook, carpentry] {
        assert_eq!(number(&snapshot, id, AMOUNT_REMAINING_FIELD)?, 0);
    }
    assert_eq!(number(&snapshot, LEDGER, STOCK_DRINK_FIELD)?, 105);
    assert_eq!(number(&snapshot, LEDGER, STOCK_FOOD_FIELD)?, 105);
    assert_eq!(number(&snapshot, LEDGER, STOCK_WOOD_FIELD)?, 19);
    assert_eq!(number(&snapshot, LEDGER, STOCK_BED_FIELD)?, 1);
    Ok(())
}

// Independent tiny exhaustive oracle: try leaving each activity idle and each
// permitted worker assignment. It does not use the engine's augmenting paths.
fn maximum_three_family_matching(masks: [u8; 3], family: u8, used: u8) -> usize {
    if family == 3 {
        return 0;
    }
    let mut best = maximum_three_family_matching(masks, family + 1, used);
    for (worker, mask) in masks.into_iter().enumerate() {
        let bit = 1u8 << worker;
        if used & bit == 0 && mask & (1u8 << family) != 0 {
            best = best.max(1 + maximum_three_family_matching(masks, family + 1, used | bit));
        }
    }
    best
}

#[test]
fn cross_family_matching_reaches_independent_optimum_for_all_512_small_graphs() -> Result<()> {
    for first in 0u8..8 {
        for second in 0u8..8 {
            for third in 0u8..8 {
                let masks = [first, second, third];
                let mut snapshot = world(true)?;
                for (id, mask) in [11, 12, 13].into_iter().zip(masks) {
                    let labors: Vec<_> = ["MINING", "CONSTRUCTION", "BREW"]
                        .into_iter()
                        .enumerate()
                        .filter_map(|(index, labor)| (mask & (1u8 << index) != 0).then_some(labor))
                        .collect();
                    worker(&mut snapshot, EntityId::new(id), &labors)?;
                }
                let dig_id = dig(&mut snapshot, "oracle-dig", DIG_TILE, DIG_TILE)?;
                let build_id = build(&mut snapshot, "oracle-build", BUILD_TILE)?;
                let brew = order(&mut snapshot, "oracle-brew", "BREW_DRINK", 1)?;
                let expected = maximum_three_family_matching(masks, 0, 0);
                let report = laboratory_workforce_report(&snapshot)?
                    .ok_or_else(|| precondition("workforce report missing"))?;
                assert_eq!(report.assignments.len(), expected, "graph {masks:?}");
                let assigned_workers: BTreeSet<_> = report
                    .assignments
                    .iter()
                    .map(|assignment| assignment.worker)
                    .collect();
                let assigned_tasks: BTreeSet<_> = report
                    .assignments
                    .iter()
                    .map(|assignment| assignment.work)
                    .collect();
                assert_eq!(assigned_workers.len(), expected, "graph {masks:?}");
                assert_eq!(assigned_tasks.len(), expected, "graph {masks:?}");
                advance(&mut snapshot, 1)?;
                let earned = number(&snapshot, dig_id, "work_ticks")?
                    + number(&snapshot, build_id, "progress_ticks")?
                    + number(&snapshot, brew, "work_ticks")?;
                assert_eq!(earned, expected as u64, "graph {masks:?}");
            }
        }
    }
    Ok(())
}

#[test]
fn canonical_ties_and_earned_progress_survive_partitions_and_reopen() -> Result<()> {
    let mut source = world(true)?;
    worker(&mut source, WORKER, &["CONSTRUCTION"])?;
    let mut keys = ["tie-left", "tie-right"];
    keys.sort_by_key(|key| created_entity_id(key, 0));
    // Older earned work is protected even when a newly admitted task has the
    // smaller canonical identity. When both are fresh, that smaller ID wins.
    let older = build(&mut source, keys[1], BUILD_TILE)?;
    advance(&mut source, 73)?;
    let newer = build(&mut source, keys[0], MapCoord::new(7, 6, 10))?;
    let report = laboratory_workforce_report(&source)?
        .ok_or_else(|| precondition("workforce report missing"))?;
    assert_eq!(report.assignments.len(), 1);
    assert_eq!(report.assignments[0].work, older);
    let whole = after(&source, &[600])?;
    assert_eq!(number(&whole, older, "progress_ticks")?, BUILD_TICKS);
    assert_eq!(number(&whole, newer, "progress_ticks")?, 173);
    assert_physical(&whole, &after(&source, &[1, 72, 177, 250, 100])?);
    let checkpoint = after(&source, &[83])?;
    let reopened = WorldSnapshot::from_canonical_bytes(&checkpoint.canonical_bytes())?;
    assert_physical(&whole, &after(&reopened, &[517])?);
    let mut each_tick = source.clone();
    for _ in 0..600 {
        advance(&mut each_tick, 1)?;
    }
    assert_physical(&whole, &each_tick);

    let mut fresh = world(true)?;
    worker(&mut fresh, WORKER, &["CONSTRUCTION"])?;
    let high = build(&mut fresh, keys[1], BUILD_TILE)?;
    let low = build(&mut fresh, keys[0], MapCoord::new(7, 6, 10))?;
    advance(&mut fresh, 1)?;
    assert_eq!(number(&fresh, low, "progress_ticks")?, 1);
    assert_eq!(number(&fresh, high, "progress_ticks")?, 0);
    Ok(())
}

#[test]
fn disabling_and_cancelling_labor_reallocates_without_resurrecting_waited_time() -> Result<()> {
    let mut snapshot = world(true)?;
    worker(&mut snapshot, WORKER, &["CONSTRUCTION"])?;
    let first_action = building_action(BUILD_TILE)?;
    apply(&mut snapshot, &first_action, "cancel-first")?;
    let first = created_entity_id("cancel-first", 0);
    advance(&mut snapshot, 137)?;
    let second = build(&mut snapshot, "cancel-second", MapCoord::new(7, 6, 10))?;
    set(
        &mut snapshot,
        WORKER,
        "labor.CONSTRUCTION",
        Value::Bool(false),
    )?;
    advance(&mut snapshot, 1_000)?;
    assert_eq!(number(&snapshot, first, "progress_ticks")?, 137);
    assert_eq!(number(&snapshot, second, "progress_ticks")?, 0);
    set(
        &mut snapshot,
        WORKER,
        "labor.CONSTRUCTION",
        Value::Bool(true),
    )?;
    advance(&mut snapshot, 1)?;
    assert_eq!(number(&snapshot, first, "progress_ticks")?, 138);
    assert_eq!(number(&snapshot, second, "progress_ticks")?, 0);
    cancel(&mut snapshot, &first_action, "cancel-first")?;
    assert_eq!(
        field_value(entity(&snapshot, first)?, BLOCKED_BY_FIELD, snapshot.tick),
        Some(&Value::Null)
    );
    assert_model_stamp(&snapshot, first)?;
    advance(&mut snapshot, BUILD_TICKS - 1)?;
    assert_eq!(number(&snapshot, first, "progress_ticks")?, 138);
    assert_eq!(
        number(&snapshot, second, "progress_ticks")?,
        BUILD_TICKS - 1
    );
    advance(&mut snapshot, 1)?;
    assert_eq!(number(&snapshot, second, "progress_ticks")?, BUILD_TICKS);
    assert_eq!(
        field_text(
            entity(&snapshot, first)?,
            CONSTRUCTION_STAGE_FIELD,
            snapshot.tick
        ),
        Some(STAGE_CANCELLED)
    );
    Ok(())
}

#[test]
fn worker_death_ends_construction_service_at_the_causal_combat_boundary() -> Result<()> {
    let mut source = world(true)?;
    worker(&mut source, WORKER, &["CONSTRUCTION"])?;
    let building = build(&mut source, "death-build", BUILD_TILE)?;
    let arrival = source.tick.0;
    put(
        &mut source,
        EntityId::new(81),
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
    )?;
    let whole = after(&source, &[700])?;
    assert_eq!(number(&whole, building, "progress_ticks")?, 300);
    assert_eq!(
        field_value(entity(&whole, WORKER)?, "alive", whole.tick),
        Some(&Value::Bool(false))
    );
    assert_physical(&whole, &after(&source, &[299, 1, 1, 399])?);
    let mut each_tick = source.clone();
    for _ in 0..700 {
        advance(&mut each_tick, 1)?;
    }
    assert_physical(&whole, &each_tick);
    Ok(())
}

#[test]
fn overlapping_designations_settle_once_and_release_capacity_at_the_same_tick() -> Result<()> {
    let mut snapshot = world(true)?;
    worker(&mut snapshot, WORKER, &["MINING", "CONSTRUCTION"])?;
    let first = dig(&mut snapshot, "overlap-first", DIG_TILE, DIG_TILE)?;
    let second = dig(&mut snapshot, "overlap-second", DIG_TILE, DIG_TILE)?;
    // Give excavation earned progress before admitting construction so the
    // assertion does not depend on hash-derived identity ordering.
    advance(&mut snapshot, 1)?;
    let building = build(&mut snapshot, "overlap-build", BUILD_TILE)?;
    advance(&mut snapshot, 9)?;
    assert_eq!(snapshot.tile_code_at(DIG_TILE), Some(tile_codes::FLOOR));
    for id in [first, second] {
        assert_eq!(number(&snapshot, id, TILES_REMAINING_FIELD)?, 0);
        assert_eq!(number(&snapshot, id, "work_ticks")?, 0);
        assert_eq!(
            field_text(entity(&snapshot, id)?, STATUS_FIELD, snapshot.tick),
            Some(STATUS_COMPLETE)
        );
    }
    assert_eq!(number(&snapshot, building, "progress_ticks")?, 0);
    advance(&mut snapshot, 1)?;
    assert_eq!(number(&snapshot, building, "progress_ticks")?, 1);
    Ok(())
}

#[test]
fn real_furniture_installation_and_abstract_building_share_construction_capacity() -> Result<()> {
    let mut snapshot = world(true)?;
    worker(&mut snapshot, WORKER, &["CARPENTRY", "CONSTRUCTION"])?;
    let production = order(&mut snapshot, "finite-bed", "CONSTRUCT_BED", 1)?;
    advance(&mut snapshot, WORK_ORDER_TICKS_PER_UNIT)?;
    let item = produced_item_id(production, 1);
    let installation = Action::PlaceFurniture {
        kind: FurnitureKind::Bed,
        item,
        location: MapCoord::new(8, 6, 10),
    };
    apply(&mut snapshot, &installation, "finite-place")?;
    let placed = created_entity_id("finite-place", 0);
    let abstract_build = build(&mut snapshot, "finite-abstract", BUILD_TILE)?;
    assert_model_stamp(&snapshot, placed)?;
    assert_eq!(number(&snapshot, LEDGER, STOCK_BED_FIELD)?, 0);
    let source = snapshot.clone();
    advance(&mut snapshot, BUILD_TICKS)?;
    assert_eq!(
        number(&snapshot, placed, "progress_ticks")?
            + number(&snapshot, abstract_build, "progress_ticks")?,
        BUILD_TICKS
    );
    let stages = [placed, abstract_build]
        .into_iter()
        .filter(|id| {
            snapshot.graph.entities.get(id).is_some_and(|record| {
                field_text(record, CONSTRUCTION_STAGE_FIELD, snapshot.tick) == Some(STAGE_COMPLETE)
            })
        })
        .count();
    assert_eq!(stages, 1);
    advance(&mut snapshot, BUILD_TICKS)?;
    validate_furniture_completion(
        &snapshot,
        placed,
        FurnitureKind::Bed,
        item,
        MapCoord::new(8, 6, 10),
    )?;
    assert_eq!(
        number(&snapshot, abstract_build, "progress_ticks")?,
        BUILD_TICKS
    );
    assert_eq!(number(&snapshot, LEDGER, STOCK_BED_FIELD)?, 0);
    assert_eq!(number(&snapshot, LEDGER, STOCK_WOOD_FIELD)?, 19);
    assert_physical(&snapshot, &after(&source, &[19, 481, 143, 357])?);
    Ok(())
}

#[test]
fn completing_a_workshop_unlocks_production_only_after_its_construction_interval() -> Result<()> {
    let mut source = world(true)?;
    source.graph.entities.remove(&EntityId::new(51));
    worker(&mut source, WORKER, &["CONSTRUCTION", "BREW"])?;
    apply(
        &mut source,
        &Action::Build {
            kind: BuildingKind::Workshop("Still".to_owned()),
            location: BUILD_TILE,
            footprint: MapCuboid::new(BUILD_TILE, BUILD_TILE)?,
            material: crate::action::MaterialSelector::default(),
        },
        "workforce-new-still",
    )?;
    let workshop = created_entity_id("workforce-new-still", 0);
    let brew = order(&mut source, "workforce-waiting-brew", "BREW_DRINK", 1)?;
    let boundary = after(&source, &[BUILD_TICKS])?;
    assert_eq!(number(&boundary, workshop, "progress_ticks")?, BUILD_TICKS);
    assert_eq!(number(&boundary, brew, "work_ticks")?, 0);
    assert_eq!(number(&boundary, brew, AMOUNT_REMAINING_FIELD)?, 1);
    let before_completion = after(&boundary, &[WORK_ORDER_TICKS_PER_UNIT - 1])?;
    assert_eq!(number(&before_completion, brew, AMOUNT_REMAINING_FIELD)?, 1);
    let completed = after(&before_completion, &[1])?;
    assert_eq!(number(&completed, brew, AMOUNT_REMAINING_FIELD)?, 0);
    assert_eq!(number(&completed, LEDGER, STOCK_DRINK_FIELD)?, 105);
    assert_physical(&completed, &after(&source, &[550])?);
    assert_physical(&completed, &after(&source, &[17, 282, 200, 1, 49, 1])?);
    Ok(())
}

#[test]
fn cancelled_furniture_returns_its_exact_item_and_releases_shared_labor() -> Result<()> {
    let mut snapshot = world(true)?;
    worker(&mut snapshot, WORKER, &["CARPENTRY", "CONSTRUCTION"])?;
    let production = order(&mut snapshot, "cancel-bed", "CONSTRUCT_BED", 1)?;
    advance(&mut snapshot, WORK_ORDER_TICKS_PER_UNIT)?;
    let item = produced_item_id(production, 1);
    let installation = Action::PlaceFurniture {
        kind: FurnitureKind::Bed,
        item,
        location: MapCoord::new(8, 6, 10),
    };
    apply(&mut snapshot, &installation, "cancel-placement")?;
    let placed = created_entity_id("cancel-placement", 0);
    advance(&mut snapshot, 123)?;
    set(
        &mut snapshot,
        WORKER,
        "labor.CONSTRUCTION",
        Value::Bool(false),
    )?;
    advance(&mut snapshot, BUILD_TICKS)?;
    assert_eq!(number(&snapshot, placed, "progress_ticks")?, 123);
    assert!(field_text(entity(&snapshot, placed)?, BLOCKED_BY_FIELD, snapshot.tick).is_some());
    cancel(&mut snapshot, &installation, "cancel-placement")?;
    assert_model_stamp(&snapshot, placed)?;
    assert_eq!(
        field_value(entity(&snapshot, placed)?, BLOCKED_BY_FIELD, snapshot.tick),
        Some(&Value::Null)
    );
    assert_eq!(
        field_text(
            entity(&snapshot, item)?,
            FURNITURE_INVENTORY_STATE_FIELD,
            snapshot.tick
        ),
        Some("loose")
    );
    assert_eq!(
        field_value(
            entity(&snapshot, item)?,
            FURNITURE_ALLOCATED_TO_FIELD,
            snapshot.tick
        ),
        Some(&Value::Null)
    );
    assert_eq!(number(&snapshot, LEDGER, STOCK_BED_FIELD)?, 1);
    let replacement = build(&mut snapshot, "cancel-replacement", BUILD_TILE)?;
    set(
        &mut snapshot,
        WORKER,
        "labor.CONSTRUCTION",
        Value::Bool(true),
    )?;
    advance(&mut snapshot, BUILD_TICKS)?;
    assert_eq!(number(&snapshot, placed, "progress_ticks")?, 123);
    assert_eq!(
        number(&snapshot, replacement, "progress_ticks")?,
        BUILD_TICKS
    );
    assert_eq!(number(&snapshot, LEDGER, STOCK_BED_FIELD)?, 1);
    Ok(())
}

#[test]
fn malformed_or_unqualified_model_markers_refuse_atomically() -> Result<()> {
    for variant in 0..9 {
        let mut snapshot = world(true)?;
        worker(&mut snapshot, WORKER, &["CONSTRUCTION"])?;
        build(&mut snapshot, "bad-model-build", BUILD_TILE)?;
        let settings = fortress_settings_entity_id(snapshot.fortress_id);
        match variant {
            0 => set(
                &mut snapshot,
                settings,
                LABORATORY_WORKFORCE_MODEL_FIELD,
                Value::Bool(true),
            )?,
            1 => set(
                &mut snapshot,
                settings,
                LABORATORY_WORKFORCE_MODEL_FIELD,
                Value::Text("shared-workforce/999".to_owned()),
            )?,
            2 => {
                fact_mut(&mut snapshot, settings, LABORATORY_WORKFORCE_MODEL_FIELD)?.presence =
                    Some(FactPresence::Omitted("model unavailable".to_owned()));
            }
            3 => {
                fact_mut(&mut snapshot, settings, LABORATORY_WORKFORCE_MODEL_FIELD)?.source =
                    FactSource::AgentAssertion("enable model".to_owned());
            }
            4 => {
                fact_mut(&mut snapshot, settings, LABORATORY_WORKFORCE_MODEL_FIELD)?.source =
                    FactSource::Replay;
            }
            5 => {
                record_mut(&mut snapshot, settings)?.kind = EntityKind::Unit;
            }
            6 | 7 => {
                let mut duplicate = entity(&snapshot, settings)?.clone();
                duplicate.id = EntityId::new(95);
                snapshot.graph.entities.insert(duplicate.id, duplicate);
                if variant == 6 {
                    snapshot.graph.entities.remove(&settings);
                }
            }
            _ => {
                let future = GameTick(snapshot.tick.0 + 1);
                fact_mut(&mut snapshot, settings, LABORATORY_WORKFORCE_MODEL_FIELD)?.observed_at =
                    future;
            }
        }
        snapshot.refresh_hash();
        let before = snapshot.clone();
        assert!(
            laboratory_workforce_model(&snapshot).is_err(),
            "variant {variant}"
        );
        assert!(
            laboratory_workforce_report(&snapshot).is_err(),
            "variant {variant}"
        );
        assert!(advance(&mut snapshot, 20).is_err(), "variant {variant}");
        assert_eq!(snapshot, before, "variant {variant}");
    }
    Ok(())
}

#[test]
fn model_history_cannot_silently_downgrade_or_import_unstamped_active_work() -> Result<()> {
    let mut source = world(true)?;
    worker(&mut source, WORKER, &["CONSTRUCTION"])?;
    let building = build(&mut source, "stamp-build", BUILD_TILE)?;
    for variant in 0..5 {
        let mut snapshot = source.clone();
        let settings = fortress_settings_entity_id(snapshot.fortress_id);
        match variant {
            0 => {
                snapshot.graph.entities.remove(&settings);
            }
            1 => {
                record_mut(&mut snapshot, building)?
                    .fields
                    .remove(LABORATORY_WORKFORCE_TASK_MODEL_FIELD);
            }
            2 => set(
                &mut snapshot,
                building,
                LABORATORY_WORKFORCE_TASK_MODEL_FIELD,
                Value::Text("shared-workforce/999".to_owned()),
            )?,
            3 => {
                fact_mut(
                    &mut snapshot,
                    building,
                    LABORATORY_WORKFORCE_TASK_MODEL_FIELD,
                )?
                .presence = Some(FactPresence::Unknown("task model unavailable".to_owned()));
            }
            _ => {
                fact_mut(
                    &mut snapshot,
                    building,
                    LABORATORY_WORKFORCE_TASK_MODEL_FIELD,
                )?
                .source = FactSource::AgentAssertion("new task".to_owned());
            }
        }
        snapshot.refresh_hash();
        let before = snapshot.clone();
        assert!(
            laboratory_workforce_report(&snapshot).is_err(),
            "variant {variant}"
        );
        assert!(advance(&mut snapshot, 20).is_err(), "variant {variant}");
        assert_eq!(snapshot, before, "variant {variant}");
    }
    // Even completed stamped history prevents a later unmarked invocation
    // from changing the model under which this world earned its results.
    advance(&mut source, BUILD_TICKS)?;
    let settings = fortress_settings_entity_id(source.fortress_id);
    source.graph.entities.remove(&settings);
    source.refresh_hash();
    let before = source.clone();
    assert!(advance(&mut source, 1).is_err());
    assert_eq!(source, before);
    Ok(())
}

#[test]
fn exhausted_advance_budgets_publish_no_prefix_of_shared_service() -> Result<()> {
    let mut source = world(true)?;
    worker(&mut source, WORKER, &["MINING", "CONSTRUCTION"])?;
    dig(&mut source, "budget-dig", DIG_TILE, MapCoord::new(5, 4, 10))?;
    build(&mut source, "budget-build", BUILD_TILE)?;
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
        let mut snapshot = source.clone();
        assert!(advance_with(&mut snapshot, 600, limits).is_err());
        assert_eq!(snapshot, source);
    }
    let completed = after(&source, &[600])?;
    assert_ne!(completed.state_hash, source.state_hash);
    Ok(())
}

#[test]
fn unmarked_legacy_worlds_keep_parallel_construction_and_old_worker_defaults() -> Result<()> {
    let mut snapshot = world(false)?;
    let dig_id = dig(&mut snapshot, "legacy-dig", DIG_TILE, DIG_TILE)?;
    let first = build(&mut snapshot, "legacy-first", BUILD_TILE)?;
    let second = build(&mut snapshot, "legacy-second", MapCoord::new(7, 6, 10))?;
    assert_eq!(laboratory_workforce_model(&snapshot)?, None);
    assert!(laboratory_workforce_report(&snapshot)?.is_none());
    advance(&mut snapshot, BUILD_TICKS)?;
    assert_eq!(number(&snapshot, dig_id, TILES_REMAINING_FIELD)?, 0);
    assert_eq!(number(&snapshot, first, "progress_ticks")?, BUILD_TICKS);
    assert_eq!(number(&snapshot, second, "progress_ticks")?, BUILD_TICKS);
    for id in [dig_id, first, second] {
        let record = entity(&snapshot, id)?;
        assert!(
            !record
                .fields
                .contains_key(LABORATORY_WORKFORCE_TASK_MODEL_FIELD)
        );
        assert!(!record.fields.contains_key(BLOCKED_BY_FIELD));
    }
    worker(&mut snapshot, WORKER, &["BREW"])?;
    record_mut(&mut snapshot, WORKER)?.fields.remove("alive");
    let brew = order(&mut snapshot, "legacy-brew", "BREW_DRINK", 1)?;
    advance(&mut snapshot, WORK_ORDER_TICKS_PER_UNIT)?;
    assert_eq!(number(&snapshot, brew, AMOUNT_REMAINING_FIELD)?, 0);
    assert_eq!(number(&snapshot, LEDGER, STOCK_DRINK_FIELD)?, 105);
    assert_eq!(laboratory_workforce_model(&snapshot)?, None);
    assert!(snapshot.graph.entities.values().all(|record| {
        !record.fields.contains_key(LABORATORY_WORKFORCE_MODEL_FIELD)
            && !record
                .fields
                .contains_key(LABORATORY_WORKFORCE_TASK_MODEL_FIELD)
    }));
    Ok(())
}
