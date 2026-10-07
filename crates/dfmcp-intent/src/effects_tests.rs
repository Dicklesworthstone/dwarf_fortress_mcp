use super::*;
use dfmcp_core::ObservationCursor;
use dfmcp_world::terrain::uniform_chunk;
use dfmcp_world::{ChunkCoord, FactPresence, WorldGraph, evaluate};
use std::collections::BTreeSet;

const UNIT_A: EntityId = EntityId::new(11);
const UNIT_B: EntityId = EntityId::new(12);
const BURROW: EntityId = EntityId::new(21);
const SQUAD: EntityId = EntityId::new(31);
const STOCKPILE: EntityId = EntityId::new(41);

fn entity(id: EntityId, kind: EntityKind, label: &str) -> EntityRecord {
    EntityRecord {
        id,
        generation: 1,
        revision: 1,
        kind,
        label: label.to_owned(),
        fields: BTreeMap::new(),
    }
}

fn world() -> WorldSnapshot {
    let mut graph = WorldGraph::default();
    for (id, kind, label) in [
        (UNIT_A, EntityKind::Unit, "Urist"),
        (UNIT_B, EntityKind::Unit, "Kogan"),
        (BURROW, EntityKind::Burrow, "Refuge"),
        (SQUAD, EntityKind::Squad, "Axes"),
        (STOCKPILE, EntityKind::Stockpile, "Food"),
    ] {
        graph.entities.insert(id, entity(id, kind, label));
    }
    let rock = ChunkCoord { x: 0, y: 0, z: 10 };
    graph
        .chunks
        .insert(rock, uniform_chunk(rock, tile_codes::SOLID_WALL));
    let floor = ChunkCoord { x: 1, y: 0, z: 10 };
    graph
        .chunks
        .insert(floor, uniform_chunk(floor, tile_codes::FLOOR));
    WorldSnapshot::new(
        FortressId::new(9),
        GameTick(1_000),
        ObservationCursor::ORIGIN,
        false,
        graph,
    )
}

/// A completed still and a brewer: what a BREW_DRINK order needs to progress.
fn equip_brewery(snapshot: &mut WorldSnapshot) {
    let mut still = entity(EntityId::new(51), EntityKind::Building, "workshop:Still");
    for (field, value) in [
        ("building_kind", Value::Text("workshop:Still".to_owned())),
        (
            CONSTRUCTION_STAGE_FIELD,
            Value::Text(STAGE_COMPLETE.to_owned()),
        ),
    ] {
        still
            .fields
            .insert(field.to_owned(), known(value, GameTick(1)));
    }
    snapshot.graph.entities.insert(still.id, still);
    if let Some(brewer) = snapshot.graph.entities.get_mut(&UNIT_A) {
        brewer.fields.insert(
            format!("{LABOR_FIELD_PREFIX}BREW"),
            known(Value::Bool(true), GameTick(1)),
        );
    }
    snapshot.refresh_hash();
}

fn cuboid(a: (i32, i32, i32), b: (i32, i32, i32)) -> Result<MapCuboid> {
    MapCuboid::new(MapCoord::new(a.0, a.1, a.2), MapCoord::new(b.0, b.1, b.2))
}

fn all_hold(snapshot: &WorldSnapshot, predicates: &[Predicate]) -> bool {
    !predicates.is_empty() && predicates.iter().all(|p| evaluate(snapshot, p))
}

fn advance(snapshot: &mut WorldSnapshot, ticks: u64) -> Result<bool> {
    snapshot.tick = GameTick(snapshot.tick.0 + ticks);
    advance_effects(snapshot, ticks)
}

#[test]
fn created_identities_are_stable_namespaced_and_ordinal_distinct() {
    let a = created_entity_id("key-a", 0);
    assert_eq!(a, created_entity_id("key-a", 0));
    assert_ne!(a, created_entity_id("key-a", 1));
    assert_ne!(a, created_entity_id("key-b", 0));
    assert_eq!(a.get() & !CREATED_ENTITY_MASK, CREATED_ENTITY_NAMESPACE);
    assert_ne!(
        fortress_settings_entity_id(FortressId::new(1)),
        fortress_settings_entity_id(FortressId::new(2))
    );
}

#[test]
fn immediate_unit_actions_satisfy_their_reference_postconditions() -> Result<()> {
    let mut s = world();
    let actions = [
        Action::SetLabor {
            units: vec![UNIT_A, UNIT_B],
            labor: "MINE".to_owned(),
            enabled: true,
        },
        Action::SetBurrowMembership {
            units: vec![UNIT_A],
            burrow: BURROW,
            assigned: true,
        },
        Action::AssignSquad {
            units: vec![UNIT_B],
            squad: SQUAD,
        },
        Action::ConfigureStockpile {
            stockpile: STOCKPILE,
            accepts: BTreeSet::from(["MEAT".to_owned(), "FISH".to_owned()]),
            max_bins: Some(4),
            max_barrels: None,
            max_wheelbarrows: Some(1),
        },
        Action::SetStandingOrder {
            key: "gather_refuse".to_owned(),
            value: "off".to_owned(),
        },
    ];
    for action in &actions {
        let post = default_postconditions(action, "k", s.fortress_id);
        assert!(!all_hold(&s, &post), "{action:?} already held");
        assert!(apply_effect(&mut s, action, "k")?);
        assert!(all_hold(&s, &post), "{action:?} not proven");
        // Reapplying an immediate configuration is a no-op, not a new revision.
        assert!(!apply_effect(&mut s, action, "k")?);
    }
    assert_eq!(s.graph.entities[&UNIT_A].revision, 3);
    Ok(())
}

#[test]
fn compensation_is_the_exact_inverse_for_self_describing_actions() -> Result<()> {
    let mut s = world();
    let action = Action::SetLabor {
        units: vec![UNIT_A],
        labor: "MINE".to_owned(),
        enabled: true,
    };
    apply_effect(&mut s, &action, "k")?;
    let undo = default_compensation(&action)
        .ok_or_else(|| DfmcpError::new(ErrorCode::InternalInvariantViolation, "no compensation"))?;
    apply_effect(&mut s, &undo, "k")?;
    assert!(all_hold(
        &s,
        &default_postconditions(&undo, "k", s.fortress_id)
    ));
    assert!(
        default_compensation(&Action::AssignSquad {
            units: vec![UNIT_A],
            squad: SQUAD
        })
        .is_none()
    );
    Ok(())
}

#[test]
fn wrong_or_missing_entities_are_precondition_failures() {
    let mut s = world();
    for action in [
        Action::SetLabor {
            units: vec![BURROW],
            labor: "MINE".to_owned(),
            enabled: true,
        },
        Action::AssignSquad {
            units: vec![UNIT_A],
            squad: EntityId::new(999),
        },
        Action::ConfigureStockpile {
            stockpile: SQUAD,
            accepts: BTreeSet::new(),
            max_bins: None,
            max_barrels: None,
            max_wheelbarrows: None,
        },
    ] {
        let error = apply_effect(&mut s, &action, "k").err();
        assert_eq!(
            error.map(|e| e.code),
            Some(ErrorCode::PreconditionsFailed),
            "{action:?}"
        );
    }
}

#[test]
fn excavation_progresses_with_game_time_in_canonical_tile_order() -> Result<()> {
    let mut s = world();
    let area = cuboid((2, 3, 10), (4, 4, 10))?; // 6 tiles
    let action = Action::DesignateDig {
        area,
        mode: DigMode::Mine,
    };
    let post = default_postconditions(&action, "dig", s.fortress_id);
    assert!(apply_effect(&mut s, &action, "dig")?);
    let designation = created_entity_id("dig", 0);
    assert_eq!(
        s.graph.entities[&designation].fields[TILES_REMAINING_FIELD].value,
        Value::U64(6)
    );
    assert!(!all_hold(&s, &post));
    // 25 ticks dig two tiles and carry five ticks.
    assert!(advance(&mut s, 25)?);
    assert_eq!(
        s.tile_code_at(MapCoord::new(2, 3, 10)),
        Some(tile_codes::FLOOR)
    );
    assert_eq!(
        s.tile_code_at(MapCoord::new(3, 3, 10)),
        Some(tile_codes::FLOOR)
    );
    assert_eq!(
        s.tile_code_at(MapCoord::new(4, 3, 10)),
        Some(tile_codes::SOLID_WALL)
    );
    assert_eq!(
        s.graph.entities[&designation].fields[TILES_REMAINING_FIELD].value,
        Value::U64(4)
    );
    // Five carried plus 35 ticks dig the remaining four tiles.
    assert!(advance(&mut s, 35)?);
    assert!(all_hold(&s, &post));
    assert_eq!(
        s.graph.entities[&designation].fields[STATUS_FIELD].value,
        Value::Text(STATUS_COMPLETE.to_owned())
    );
    assert!(!advance(&mut s, 100)?);
    // Dispatching the same step twice is refused rather than duplicated.
    assert_eq!(
        apply_effect(&mut s, &action, "dig").err().map(|e| e.code),
        Some(ErrorCode::Conflict)
    );
    Ok(())
}

#[test]
fn excavation_refuses_unobserved_terrain() -> Result<()> {
    let mut s = world();
    let action = Action::DesignateDig {
        area: cuboid((0, 0, 10), (0, 0, 11))?,
        mode: DigMode::Channel,
    };
    assert_eq!(
        apply_effect(&mut s, &action, "dig").err().map(|e| e.code),
        Some(ErrorCode::PreconditionsFailed)
    );
    Ok(())
}

#[test]
fn buildings_need_open_floor_and_complete_after_construction_time() -> Result<()> {
    let mut s = world();
    let blocked = Action::Build {
        kind: BuildingKind::Workshop("Carpenters".to_owned()),
        location: MapCoord::new(1, 1, 10),
        footprint: cuboid((0, 0, 10), (2, 2, 10))?,
        material: crate::MaterialSelector::default(),
    };
    assert_eq!(
        apply_effect(&mut s, &blocked, "b").err().map(|e| e.code),
        Some(ErrorCode::PreconditionsFailed)
    );
    let build = Action::Build {
        kind: BuildingKind::Workshop("Carpenters".to_owned()),
        location: MapCoord::new(17, 1, 10),
        footprint: cuboid((16, 0, 10), (18, 2, 10))?,
        material: crate::MaterialSelector::default(),
    };
    let post = default_postconditions(&build, "b", s.fortress_id);
    apply_effect(&mut s, &build, "b")?;
    let id = created_entity_id("b", 0);
    assert_eq!(s.graph.entities[&id].kind, EntityKind::Building);
    assert_eq!(s.graph.entities[&id].label, "workshop:Carpenters");
    advance(&mut s, BUILD_TICKS - 1)?;
    assert_eq!(
        s.graph.entities[&id].fields[CONSTRUCTION_STAGE_FIELD].value,
        Value::Text(STAGE_UNDER_CONSTRUCTION.to_owned())
    );
    assert!(!all_hold(&s, &post));
    advance(&mut s, 1)?;
    assert!(all_hold(&s, &post));
    Ok(())
}

#[test]
fn work_orders_count_down_and_complete() -> Result<()> {
    let mut s = world();
    equip_brewery(&mut s);
    let order = Action::CreateWorkOrder {
        name: "brew".to_owned(),
        job_token: "BREW_DRINK".to_owned(),
        amount: 3,
        conditions: Vec::new(),
    };
    let post = default_postconditions(&order, "o", s.fortress_id);
    apply_effect(&mut s, &order, "o")?;
    let id = created_entity_id("o", 0);
    advance(&mut s, WORK_ORDER_TICKS_PER_UNIT * 2 + 7)?;
    assert_eq!(
        s.graph.entities[&id].fields[AMOUNT_REMAINING_FIELD].value,
        Value::U64(1)
    );
    advance(&mut s, WORK_ORDER_TICKS_PER_UNIT - 7)?;
    assert!(all_hold(&s, &post));
    assert_eq!(
        s.graph.entities[&id].fields[STATUS_FIELD].value,
        Value::Text(STATUS_COMPLETE.to_owned())
    );
    Ok(())
}

#[test]
fn default_obligations_cover_exactly_the_temporal_families() -> Result<()> {
    let now = GameTick(1_000);
    let fortress = FortressId::new(9);
    let dig = Action::DesignateDig {
        area: cuboid((0, 0, 0), (9, 9, 0))?,
        mode: DigMode::Mine,
    };
    let obligation = default_obligation(&dig, "d", fortress, now)?.ok_or_else(|| {
        DfmcpError::new(ErrorCode::InternalInvariantViolation, "missing obligation")
    })?;
    assert_eq!(obligation.deadline_tick, GameTick(1_000 + 100 + 2_000));
    assert_eq!(
        vec![obligation.terminal],
        default_postconditions(&dig, "d", fortress)
    );
    let labor = Action::SetLabor {
        units: vec![UNIT_A],
        labor: "MINE".to_owned(),
        enabled: true,
    };
    assert!(default_obligation(&labor, "l", fortress, now)?.is_none());
    let huge = Action::CreateWorkOrder {
        name: "x".to_owned(),
        job_token: "X".to_owned(),
        amount: u32::MAX,
        conditions: Vec::new(),
    };
    let capped = default_obligation(&huge, "w", fortress, now)?.ok_or_else(|| {
        DfmcpError::new(ErrorCode::InternalInvariantViolation, "missing obligation")
    })?;
    assert_eq!(
        capped.deadline_tick,
        GameTick(1_000 + MAX_DEFAULT_OBLIGATION_TICKS)
    );
    Ok(())
}

#[test]
fn extensions_have_no_reference_semantics() {
    let mut s = world();
    let extension = Action::Extension {
        namespace: "ns".to_owned(),
        name: "x".to_owned(),
        parameters: BTreeMap::new(),
    };
    assert!(default_postconditions(&extension, "e", s.fortress_id).is_empty());
    assert_eq!(
        apply_effect(&mut s, &extension, "e").err().map(|e| e.code),
        Some(ErrorCode::AdapterRejected)
    );
}

#[test]
fn cancellation_stops_temporal_work_without_undoing_progress() -> Result<()> {
    let mut s = world();
    let dig = Action::DesignateDig {
        area: cuboid((0, 0, 10), (3, 0, 10))?,
        mode: DigMode::Mine,
    };
    apply_effect(&mut s, &dig, "d")?;
    advance(&mut s, DIG_TICKS_PER_TILE)?;
    assert!(cancel_effect(&mut s, &dig, "d")?);
    assert!(!cancel_effect(&mut s, &dig, "d")?);
    assert!(!advance(&mut s, 1_000)?);
    assert_eq!(
        s.tile_code_at(MapCoord::new(0, 0, 10)),
        Some(tile_codes::FLOOR)
    );
    assert_eq!(
        s.tile_code_at(MapCoord::new(1, 0, 10)),
        Some(tile_codes::SOLID_WALL)
    );
    let labor = Action::SetLabor {
        units: vec![UNIT_A],
        labor: "MINE".to_owned(),
        enabled: true,
    };
    assert!(!cancel_effect(&mut s, &labor, "l")?);
    Ok(())
}

fn with_ledger(drink: u64, food: u64) -> WorldSnapshot {
    let mut snapshot = world();
    let mut ledger = entity(
        EntityId::new(91),
        EntityKind::Other(STOCK_LEDGER_KIND.to_owned()),
        "stocks",
    );
    for (field, value) in [
        (STOCK_DRINK_FIELD, drink),
        (STOCK_FOOD_FIELD, food),
        (METABOLISM_TICKS_FIELD, 0),
    ] {
        ledger
            .fields
            .insert(field.to_owned(), known(Value::U64(value), GameTick(1)));
    }
    snapshot.graph.entities.insert(ledger.id, ledger);
    snapshot.refresh_hash();
    snapshot
}

fn ledger_u64(snapshot: &WorldSnapshot, field: &str) -> Result<u64> {
    field_u64(&snapshot.graph.entities[&EntityId::new(91)], field)
}

fn need(snapshot: &WorldSnapshot, unit: EntityId, field: &str) -> Option<String> {
    field_text(&snapshot.graph.entities[&unit], field).map(str::to_owned)
}

#[test]
fn dwarves_drink_and_eat_on_schedule_and_shortages_are_explicit() -> Result<()> {
    let mut snapshot = with_ledger(3, 10);
    // Less than one interval: nothing is consumed, only time is accounted.
    advance(&mut snapshot, DRINK_INTERVAL_TICKS - 1)?;
    assert_eq!(ledger_u64(&snapshot, STOCK_DRINK_FIELD)?, 3);
    assert_eq!(need(&snapshot, UNIT_A, NEED_DRINK_FIELD), None);
    // One drinking round for two dwarves.
    advance(&mut snapshot, 1)?;
    assert_eq!(ledger_u64(&snapshot, STOCK_DRINK_FIELD)?, 1);
    assert_eq!(
        need(&snapshot, UNIT_B, NEED_DRINK_FIELD).as_deref(),
        Some("satisfied")
    );
    // The next round has one unit for two dwarves: the later one goes without.
    advance(&mut snapshot, DRINK_INTERVAL_TICKS)?;
    assert_eq!(ledger_u64(&snapshot, STOCK_DRINK_FIELD)?, 0);
    assert_eq!(
        need(&snapshot, UNIT_A, NEED_DRINK_FIELD).as_deref(),
        Some("satisfied")
    );
    assert_eq!(
        need(&snapshot, UNIT_B, NEED_DRINK_FIELD).as_deref(),
        Some("thirsty")
    );
    // Food every second drinking round: 10 - 2 = 8.
    assert_eq!(ledger_u64(&snapshot, STOCK_FOOD_FIELD)?, 8);
    Ok(())
}

#[test]
fn completed_brewing_restocks_and_one_long_wait_equals_many_short_ones() -> Result<()> {
    let mut a = with_ledger(0, 100);
    equip_brewery(&mut a);
    let order = Action::CreateWorkOrder {
        name: "brew".to_owned(),
        job_token: "BREW_DRINK".to_owned(),
        amount: 4,
        conditions: Vec::new(),
    };
    apply_effect(&mut a, &order, "brew-key")?;
    a.refresh_hash();
    let mut b = a.clone();
    advance(&mut a, 3_000)?;
    for _ in 0..30 {
        advance(&mut b, 100)?;
    }
    // Four units of five drinks, minus two rounds for two dwarves.
    assert_eq!(ledger_u64(&a, STOCK_DRINK_FIELD)?, 20 - 4);
    // Fact stamps record when each value was written; the values agree.
    let values = |s: &WorldSnapshot| -> Vec<(EntityId, String, Value)> {
        s.graph
            .entities
            .values()
            .flat_map(|e| {
                e.fields
                    .iter()
                    .filter(|(name, _)| {
                        name.as_str() != "work_ticks" && name.as_str() != METABOLISM_TICKS_FIELD
                    })
                    .map(|(name, fact)| (e.id, name.clone(), fact.value.clone()))
            })
            .collect()
    };
    assert_eq!(values(&a), values(&b));
    Ok(())
}

#[test]
fn work_orders_stall_without_their_workshop_or_worker_and_say_why() -> Result<()> {
    let mut s = world();
    let order = Action::CreateWorkOrder {
        name: "brew".to_owned(),
        job_token: "BREW_DRINK".to_owned(),
        amount: 1,
        conditions: Vec::new(),
    };
    apply_effect(&mut s, &order, "o")?;
    let id = created_entity_id("o", 0);
    advance(&mut s, WORK_ORDER_TICKS_PER_UNIT * 10)?;
    let order_fields = &s.graph.entities[&id].fields;
    assert_eq!(order_fields[AMOUNT_REMAINING_FIELD].value, Value::U64(1));
    assert_eq!(
        order_fields[BLOCKED_BY_FIELD].value,
        Value::Text(
            "no completed workshop:Still and no living unit with the BREW labor enabled".to_owned()
        )
    );
    // Equipping the brewery unblocks it; only time after that counts.
    equip_brewery(&mut s);
    advance(&mut s, WORK_ORDER_TICKS_PER_UNIT)?;
    let order_fields = &s.graph.entities[&id].fields;
    assert_eq!(order_fields[AMOUNT_REMAINING_FIELD].value, Value::U64(0));
    assert_eq!(order_fields[BLOCKED_BY_FIELD].value, Value::Null);
    Ok(())
}

fn unavailable_variants(original: &Fact, anchor: dfmcp_core::StateAnchor) -> Vec<Option<Fact>> {
    let mut variants = vec![
        None,
        Some(known(Value::Null, original.observed_at)),
        Some(known(Value::Bool(true), original.observed_at)),
    ];
    for presence in [
        FactPresence::Absent,
        FactPresence::Unknown("not observed".to_owned()),
        FactPresence::Unsupported("not supported".to_owned()),
        FactPresence::Omitted("outside projection".to_owned()),
        FactPresence::Redacted("withheld".to_owned()),
        FactPresence::Stale(anchor),
        FactPresence::Known(Value::Text("inconsistent compatibility value".to_owned())),
    ] {
        // Keep the old compatibility value populated to catch consumers that
        // bypass presence and read it directly.
        let mut fact = original.clone();
        fact.presence = Some(presence);
        variants.push(Some(fact));
    }
    variants
}

fn temporal_cases() -> Result<Vec<(Action, Vec<&'static str>)>> {
    Ok(vec![
        (
            Action::CreateWorkOrder {
                name: "reference work".to_owned(),
                job_token: "MAKE_TEST_ITEM".to_owned(),
                amount: 1,
                conditions: Vec::new(),
            },
            vec![AMOUNT_REMAINING_FIELD, "work_ticks", "job_token"],
        ),
        (
            Action::Build {
                kind: BuildingKind::Workshop("Carpenters".to_owned()),
                location: MapCoord::new(17, 1, 10),
                footprint: cuboid((16, 0, 10), (18, 2, 10))?,
                material: crate::MaterialSelector::default(),
            },
            vec!["required_ticks", "progress_ticks"],
        ),
        (
            Action::DesignateDig {
                area: cuboid((1, 1, 10), (1, 1, 10))?,
                mode: DigMode::Mine,
            },
            vec!["target_tile_code", "work_ticks"],
        ),
    ])
}

#[test]
fn unavailable_progress_inputs_never_create_completion() -> Result<()> {
    for (action, fields) in temporal_cases()? {
        let mut original = world();
        apply_effect(&mut original, &action, "uncertain")?;
        original.refresh_hash();
        let id = created_entity_id("uncertain", 0);
        let postconditions = default_postconditions(&action, "uncertain", original.fortress_id);
        for field in fields {
            for replacement in unavailable_variants(
                &original.graph.entities[&id].fields[field],
                original.anchor(),
            ) {
                let mut shadow = original.clone();
                let record = shadow.graph.entities.get_mut(&id).unwrap();
                match replacement {
                    Some(fact) => {
                        record.fields.insert(field.to_owned(), fact);
                    }
                    None => {
                        record.fields.remove(field);
                    }
                }
                let before = shadow.graph.clone();
                assert_eq!(
                    advance(&mut shadow, BUILD_TICKS * 10)
                        .err()
                        .map(|error| error.code),
                    Some(ErrorCode::PreconditionsFailed),
                    "{action:?}: {field}",
                );
                assert_eq!(shadow.graph, before, "{action:?}: {field}");
                assert!(!all_hold(&shadow, &postconditions), "{action:?}: {field}");
            }
        }
    }
    Ok(())
}

#[test]
fn explicit_known_progress_inputs_still_complete() -> Result<()> {
    for (action, _) in temporal_cases()? {
        let mut snapshot = world();
        apply_effect(&mut snapshot, &action, "known")?;
        let id = created_entity_id("known", 0);
        for fact in snapshot
            .graph
            .entities
            .get_mut(&id)
            .unwrap()
            .fields
            .values_mut()
        {
            fact.presence = Some(FactPresence::Known(fact.value.clone()));
        }
        let postconditions = default_postconditions(&action, "known", snapshot.fortress_id);
        advance(&mut snapshot, BUILD_TICKS * 10)?;
        assert!(all_hold(&snapshot, &postconditions), "{action:?}");
    }
    Ok(())
}

#[test]
fn unavailable_workshop_worker_and_life_facts_cannot_release_production() -> Result<()> {
    let action = Action::CreateWorkOrder {
        name: "brew".to_owned(),
        job_token: "BREW_DRINK".to_owned(),
        amount: 1,
        conditions: Vec::new(),
    };
    for (subject, field) in [
        (EntityId::new(51), "building_kind"),
        (EntityId::new(51), CONSTRUCTION_STAGE_FIELD),
        (UNIT_A, "labor.BREW"),
        (UNIT_A, "alive"),
    ] {
        let mut snapshot = world();
        equip_brewery(&mut snapshot);
        snapshot
            .graph
            .entities
            .get_mut(&UNIT_A)
            .unwrap()
            .fields
            .insert("alive".to_owned(), known(Value::Bool(true), snapshot.tick));
        apply_effect(&mut snapshot, &action, "blocked")?;
        let original = snapshot.graph.entities[&subject].fields[field].clone();
        let mut unavailable = original.clone();
        unavailable.presence = Some(FactPresence::Omitted("outside projection".to_owned()));
        snapshot
            .graph
            .entities
            .get_mut(&subject)
            .unwrap()
            .fields
            .insert(field.to_owned(), unavailable);
        advance(&mut snapshot, WORK_ORDER_TICKS_PER_UNIT * 10)?;
        let id = created_entity_id("blocked", 0);
        assert_eq!(
            field_u64(&snapshot.graph.entities[&id], AMOUNT_REMAINING_FIELD)?,
            1
        );
        assert!(field_text(&snapshot.graph.entities[&id], BLOCKED_BY_FIELD).is_some());
        assert_eq!(field_u64(&snapshot.graph.entities[&id], "work_ticks")?, 0);
        snapshot
            .graph
            .entities
            .get_mut(&subject)
            .unwrap()
            .fields
            .insert(field.to_owned(), original);
        advance(&mut snapshot, WORK_ORDER_TICKS_PER_UNIT)?;
        assert!(all_hold(
            &snapshot,
            &default_postconditions(&action, "blocked", snapshot.fortress_id)
        ));
    }
    Ok(())
}

#[test]
fn zero_building_duration_is_invalid_instead_of_one_tick_completion() -> Result<()> {
    let (action, _) = temporal_cases()?.remove(1);
    let mut snapshot = world();
    apply_effect(&mut snapshot, &action, "zero")?;
    let id = created_entity_id("zero", 0);
    snapshot.graph.entities.get_mut(&id).unwrap().fields.insert(
        "required_ticks".to_owned(),
        known(Value::U64(0), snapshot.tick),
    );
    assert_eq!(
        advance(&mut snapshot, 1).err().map(|error| error.code),
        Some(ErrorCode::PreconditionsFailed)
    );
    assert!(!all_hold(
        &snapshot,
        &default_postconditions(&action, "zero", snapshot.fortress_id)
    ));
    Ok(())
}

fn with_active_threat() -> WorldSnapshot {
    let mut snapshot = world();
    let mut creature = entity(EntityId::new(99), EntityKind::Creature, "raider");
    for (field, value) in [
        (HOSTILE_FIELD, Value::Bool(true)),
        (HEALTH_FIELD, Value::U64(100)),
        (ARRIVES_AT_FIELD, Value::U64(snapshot.tick.0)),
        (
            THREAT_STATUS_FIELD,
            Value::Text(THREAT_APPROACHING.to_owned()),
        ),
        (COMBAT_ROUNDS_FIELD, Value::U64(0)),
    ] {
        creature
            .fields
            .insert(field.to_owned(), known(value, snapshot.tick));
    }
    snapshot.graph.entities.insert(creature.id, creature);
    snapshot.refresh_hash();
    snapshot
}

#[test]
fn unavailable_life_census_refuses_stock_and_combat_progress() -> Result<()> {
    for (original, ticks) in [
        (with_ledger(10, 10), DRINK_INTERVAL_TICKS),
        (with_active_threat(), COMBAT_ROUND_TICKS),
    ] {
        let alive = known(Value::Bool(true), original.tick);
        for unavailable in unavailable_variants(&alive, original.anchor())
            .into_iter()
            .flatten()
            .filter(|fact| !matches!(fact.known_value(), Some(Value::Bool(_))))
        {
            let mut shadow = original.clone();
            shadow
                .graph
                .entities
                .get_mut(&UNIT_A)
                .unwrap()
                .fields
                .insert("alive".to_owned(), unavailable);
            let before = shadow.graph.clone();
            assert_eq!(
                advance(&mut shadow, ticks).err().map(|error| error.code),
                Some(ErrorCode::PreconditionsFailed)
            );
            assert_eq!(shadow.graph, before);
        }
        // The documented missing-field default for older reference fixtures
        // remains valid, so a fully available world still advances.
        let mut legacy = original;
        assert!(advance(&mut legacy, ticks)?);
    }
    Ok(())
}

#[test]
fn unavailable_military_selectors_cannot_determine_combat_outcomes() -> Result<()> {
    for (subject, field, value, ticks) in [
        (
            UNIT_A,
            SQUAD_FIELD.to_owned(),
            Value::Entity(SQUAD),
            COMBAT_ROUND_TICKS,
        ),
        (
            UNIT_B,
            format!("{BURROW_FIELD_PREFIX}{}", BURROW.get()),
            Value::Bool(false),
            COMBAT_ROUND_TICKS * ROUNDS_PER_KILL,
        ),
    ] {
        let mut snapshot = with_active_threat();
        let mut unavailable = known(value, snapshot.tick);
        unavailable.presence = Some(FactPresence::Omitted("outside projection".to_owned()));
        snapshot
            .graph
            .entities
            .get_mut(&subject)
            .unwrap()
            .fields
            .insert(field, unavailable);
        let before = snapshot.graph.clone();
        assert_eq!(
            advance(&mut snapshot, ticks).err().map(|error| error.code),
            Some(ErrorCode::PreconditionsFailed)
        );
        assert_eq!(snapshot.graph, before);
    }
    Ok(())
}
