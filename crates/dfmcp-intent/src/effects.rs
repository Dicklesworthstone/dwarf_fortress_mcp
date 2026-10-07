//! Reference semantics of every semantic action on the canonical world model.
//!
//! This module is the single definition of what an action *means* in canonical
//! state: which entities and fields it changes, which entity it creates (with
//! an identity derived only from the step's idempotency key, so a planner can
//! name it before dispatch), which postconditions prove it, and how temporal
//! work progresses with game time.
//!
//! The deterministic laboratory and the in-process mutation dispatcher both
//! execute these semantics. A live adapter must instead *observe* the native
//! effects; it uses the same field vocabulary and created-entity identities so
//! the same sealed postconditions and obligations remain checkable.
//!
//! Progress rates are a laboratory calibration, not a claim about Dwarf
//! Fortress throughput.

use std::collections::BTreeMap;

use dfmcp_core::{
    DfmcpError, Digest32, EntityId, ErrorCode, FortressId, GameTick, MapCoord, MapCuboid, Result,
};
use dfmcp_world::terrain::{region_tiles, tile_codes, validate_region};
use dfmcp_world::{
    CompareOp, EntityKind, EntityRecord, Fact, FactSource, Predicate, Value, WorldSnapshot,
};

use crate::action::{Action, BuildingKind, DigMode};
use crate::plan::ObligationSpec;

/// Field holding whether a labor is enabled on a unit: `labor.<labor>`.
pub const LABOR_FIELD_PREFIX: &str = "labor.";
/// Field holding burrow membership on a unit: `burrow.<burrow entity id>`.
pub const BURROW_FIELD_PREFIX: &str = "burrow.";
/// Field holding a unit's squad entity.
pub const SQUAD_FIELD: &str = "squad";
/// Field holding a standing order on the fortress settings entity.
pub const STANDING_ORDER_FIELD_PREFIX: &str = "standing_order.";
/// Stockpile fields.
pub const STOCKPILE_ACCEPTS_FIELD: &str = "accepts";
pub const STOCKPILE_MAX_BINS_FIELD: &str = "max_bins";
pub const STOCKPILE_MAX_BARRELS_FIELD: &str = "max_barrels";
pub const STOCKPILE_MAX_WHEELBARROWS_FIELD: &str = "max_wheelbarrows";
/// Lifecycle field on created designation, building and work-order entities.
pub const STATUS_FIELD: &str = "status";
pub const STATUS_ACTIVE: &str = "active";
pub const STATUS_COMPLETE: &str = "complete";
pub const STATUS_CANCELLED: &str = "cancelled";
/// Building fields.
pub const CONSTRUCTION_STAGE_FIELD: &str = "construction_stage";
pub const STAGE_PLANNED: &str = "planned";
pub const STAGE_UNDER_CONSTRUCTION: &str = "under_construction";
pub const STAGE_COMPLETE: &str = "complete";
pub const STAGE_CANCELLED: &str = "cancelled";
/// Work-order and designation progress fields.
pub const AMOUNT_REMAINING_FIELD: &str = "amount_remaining";
pub const TILES_REMAINING_FIELD: &str = "tiles_remaining";

/// Entity kind of a created dig designation.
pub const DIG_DESIGNATION_KIND: &str = "dig_designation";
/// Entity kind of the per-fortress settings record holding standing orders.
pub const FORTRESS_SETTINGS_KIND: &str = "fortress_settings";

/// Laboratory calibration: game ticks of labor to excavate one tile.
pub const DIG_TICKS_PER_TILE: u64 = 10;
/// Entity kind of a fortress's consumable stock ledger (laboratory economy).
pub const STOCK_LEDGER_KIND: &str = "stock_ledger";
/// Drink units held by the stock ledger.
pub const STOCK_DRINK_FIELD: &str = "stock.drink";
/// Food units held by the stock ledger.
pub const STOCK_FOOD_FIELD: &str = "stock.food";
/// Game ticks of metabolism the ledger has accounted for.
pub const METABOLISM_TICKS_FIELD: &str = "metabolism_ticks";
/// Each living dwarf drinks one unit per this many ticks (calibration).
pub const DRINK_INTERVAL_TICKS: u64 = 1_200;
/// Each living dwarf eats one unit per this many ticks (calibration).
pub const FOOD_INTERVAL_TICKS: u64 = 2_400;
/// Unit need fields: `satisfied`, or `thirsty` / `hungry` after a shortage.
pub const NEED_DRINK_FIELD: &str = "need.drink";
pub const NEED_FOOD_FIELD: &str = "need.food";

/// Hostile creature fields (laboratory threat model).
pub const HOSTILE_FIELD: &str = "hostile";
pub const HEALTH_FIELD: &str = "health";
pub const ARRIVES_AT_FIELD: &str = "arrives_at_tick";
pub const THREAT_STATUS_FIELD: &str = "threat_status";
pub const THREAT_APPROACHING: &str = "approaching";
pub const THREAT_ATTACKING: &str = "attacking";
pub const THREAT_SLAIN: &str = "slain";
/// Game ticks per combat round while a hostile is attacking.
pub const COMBAT_ROUND_TICKS: u64 = 100;
/// Damage each squad member deals per round.
pub const SOLDIER_DAMAGE_PER_ROUND: u64 = 10;
/// An unopposed hostile kills one exposed dwarf every this many rounds.
pub const ROUNDS_PER_KILL: u64 = 3;
/// Combat rounds a threat has fought (kept on the creature).
pub const COMBAT_ROUNDS_FIELD: &str = "combat_rounds";

/// The stock a completed work-order unit adds, if its job produces any.
#[must_use]
pub fn work_order_product(job_token: &str) -> Option<(&'static str, u64)> {
    match job_token {
        "BREW_DRINK" => Some((STOCK_DRINK_FIELD, 5)),
        "PREPARE_MEAL" | "COOK_MEAL" => Some((STOCK_FOOD_FIELD, 5)),
        _ => None,
    }
}
/// Workshop kind label and labor a laboratory job needs before it makes any
/// progress. Jobs without an entry have no modeled requirement.
#[must_use]
pub fn work_order_requirements(job_token: &str) -> Option<(&'static str, &'static str)> {
    match job_token {
        "BREW_DRINK" => Some(("workshop:Still", "BREW")),
        "PREPARE_MEAL" | "COOK_MEAL" => Some(("workshop:Kitchen", "COOK")),
        _ => None,
    }
}

/// Field a stalled work order carries naming what it is waiting for.
pub const BLOCKED_BY_FIELD: &str = "blocked_by";

/// Why `job_token` cannot progress in `snapshot`, if it cannot: no completed
/// matching workshop, or no living unit with the labor enabled.
#[must_use]
pub fn work_order_blocker(snapshot: &WorldSnapshot, job_token: &str) -> Option<String> {
    let (workshop, labor) = work_order_requirements(job_token)?;
    let entities = snapshot.graph.entities.values();
    let has_workshop = entities.clone().any(|entity| {
        entity.kind == EntityKind::Building
            && field_text(entity, "building_kind") == Some(workshop)
            && field_text(entity, CONSTRUCTION_STAGE_FIELD) == Some(STAGE_COMPLETE)
    });
    let labor_field = format!("{LABOR_FIELD_PREFIX}{labor}");
    let has_worker = entities.clone().any(|entity| {
        entity.kind == EntityKind::Unit
            && is_alive(entity)
            && field_value(entity, &labor_field) == Some(&Value::Bool(true))
    });
    match (has_workshop, has_worker) {
        (true, true) => None,
        (false, true) => Some(format!("no completed {workshop}")),
        (true, false) => Some(format!("no living unit with the {labor} labor enabled")),
        (false, false) => Some(format!(
            "no completed {workshop} and no living unit with the {labor} labor enabled"
        )),
    }
}

/// Laboratory calibration: game ticks to construct one building.
pub const BUILD_TICKS: u64 = 500;
/// Laboratory calibration: game ticks to produce one work-order unit.
pub const WORK_ORDER_TICKS_PER_UNIT: u64 = 50;
/// Largest default obligation horizon: one Dwarf Fortress year.
pub const MAX_DEFAULT_OBLIGATION_TICKS: u64 = 403_200;
/// Default obligation polling cadence.
pub const DEFAULT_POLL_INTERVAL_TICKS: u64 = 10;

const CREATED_ENTITY_NAMESPACE: u64 = 0x7E00_0000_0000_0000;
const CREATED_ENTITY_MASK: u64 = 0x00FF_FFFF_FFFF_FFFF;
const SOURCE: &str = "dfmcp.reference-effects/1";

fn namespaced(domain: &[u8], payload: &[u8]) -> EntityId {
    let mut bytes = Vec::with_capacity(domain.len() + payload.len() + 1);
    bytes.extend_from_slice(domain);
    bytes.push(0);
    bytes.extend_from_slice(payload);
    let digest = Digest32::of_bytes(&bytes);
    let mut low = [0u8; 8];
    low.copy_from_slice(&digest.as_bytes()[..8]);
    EntityId::new(CREATED_ENTITY_NAMESPACE | (u64::from_be_bytes(low) & CREATED_ENTITY_MASK))
}

/// Identity of the `ordinal`-th entity created by the step whose idempotency
/// key is `idempotency_key`. It is known before dispatch and stable across
/// retries, so sealed postconditions can refer to it.
#[must_use]
pub fn created_entity_id(idempotency_key: &str, ordinal: u32) -> EntityId {
    let mut payload = idempotency_key.as_bytes().to_vec();
    payload.extend_from_slice(&ordinal.to_be_bytes());
    namespaced(b"dfmcp-created-entity-v1", &payload)
}

/// Identity of the fortress-wide settings entity that holds standing orders.
#[must_use]
pub fn fortress_settings_entity_id(fortress: FortressId) -> EntityId {
    namespaced(b"dfmcp-fortress-settings-v1", &fortress.get().to_be_bytes())
}

/// Canonical tile code an excavation mode leaves behind.
#[must_use]
pub const fn dig_target_tile_code(mode: DigMode) -> u32 {
    match mode {
        DigMode::Mine | DigMode::RemoveConstruction => tile_codes::FLOOR,
        DigMode::Channel => tile_codes::OPEN_SPACE,
        DigMode::UpStair | DigMode::DownStair | DigMode::UpDownStair => tile_codes::STAIR,
        DigMode::Ramp => tile_codes::RAMP,
    }
}

const fn dig_mode_name(mode: DigMode) -> &'static str {
    match mode {
        DigMode::Mine => "mine",
        DigMode::Channel => "channel",
        DigMode::UpStair => "up_stair",
        DigMode::DownStair => "down_stair",
        DigMode::UpDownStair => "up_down_stair",
        DigMode::Ramp => "ramp",
        DigMode::RemoveConstruction => "remove_construction",
    }
}

/// Stable label for a building kind, e.g. `workshop:Carpenters`.
#[must_use]
pub fn building_kind_label(kind: &BuildingKind) -> String {
    match kind {
        BuildingKind::Workshop(name) => format!("workshop:{name}"),
        BuildingKind::Furnace(name) => format!("furnace:{name}"),
        BuildingKind::Furniture(name) => format!("furniture:{name}"),
        BuildingKind::Construction(name) => format!("construction:{name}"),
        BuildingKind::Trap(name) => format!("trap:{name}"),
        BuildingKind::FarmPlot => "farm_plot".to_owned(),
        BuildingKind::Bridge => "bridge".to_owned(),
        BuildingKind::Well => "well".to_owned(),
        BuildingKind::Custom(name) => format!("custom:{name}"),
    }
}

fn field_eq(entity_id: EntityId, field: impl Into<String>, value: Value) -> Predicate {
    Predicate::FieldCompare {
        entity_id,
        field: field.into(),
        op: CompareOp::Eq,
        value,
    }
}

fn accepts_value(accepts: &std::collections::BTreeSet<String>) -> Value {
    Value::List(accepts.iter().cloned().map(Value::Text).collect())
}

/// Postconditions that prove `action` took effect, or empty when the action
/// family has no reference semantics (extensions must state their own).
#[must_use]
pub fn default_postconditions(
    action: &Action,
    idempotency_key: &str,
    fortress: FortressId,
) -> Vec<Predicate> {
    match action {
        Action::Pause { paused } => vec![Predicate::Paused(*paused)],
        Action::DesignateDig { area, mode } => vec![Predicate::RegionTerrain {
            area: *area,
            tile_code: dig_target_tile_code(*mode),
        }],
        Action::Build { .. } => vec![field_eq(
            created_entity_id(idempotency_key, 0),
            CONSTRUCTION_STAGE_FIELD,
            Value::Text(STAGE_COMPLETE.to_owned()),
        )],
        Action::CreateWorkOrder { .. } => vec![field_eq(
            created_entity_id(idempotency_key, 0),
            AMOUNT_REMAINING_FIELD,
            Value::U64(0),
        )],
        Action::SetLabor {
            units,
            labor,
            enabled,
        } => units
            .iter()
            .map(|unit| {
                field_eq(
                    *unit,
                    format!("{LABOR_FIELD_PREFIX}{labor}"),
                    Value::Bool(*enabled),
                )
            })
            .collect(),
        Action::SetBurrowMembership {
            units,
            burrow,
            assigned,
        } => units
            .iter()
            .map(|unit| {
                field_eq(
                    *unit,
                    format!("{BURROW_FIELD_PREFIX}{}", burrow.get()),
                    Value::Bool(*assigned),
                )
            })
            .collect(),
        Action::AssignSquad { units, squad } => units
            .iter()
            .map(|unit| field_eq(*unit, SQUAD_FIELD, Value::Entity(*squad)))
            .collect(),
        Action::ConfigureStockpile {
            stockpile, accepts, ..
        } => vec![field_eq(
            *stockpile,
            STOCKPILE_ACCEPTS_FIELD,
            accepts_value(accepts),
        )],
        Action::SetStandingOrder { key, value } => vec![field_eq(
            fortress_settings_entity_id(fortress),
            format!("{STANDING_ORDER_FIELD_PREFIX}{key}"),
            Value::Text(value.clone()),
        )],
        Action::Extension { .. } => Vec::new(),
    }
}

/// Default bounded obligation for naturally temporal actions. The terminal is
/// the action's reference postcondition; the deadline is an explicit, sealed
/// game-tick horizon scaled to the requested work and capped at one year.
pub fn default_obligation(
    action: &Action,
    idempotency_key: &str,
    fortress: FortressId,
    now: GameTick,
) -> Result<Option<ObligationSpec>> {
    let horizon = match action {
        Action::DesignateDig { area, .. } => {
            let tiles = validate_region(*area)?;
            DIG_TICKS_PER_TILE
                .saturating_mul(tiles)
                .saturating_mul(2)
                .saturating_add(100)
        }
        Action::Build { .. } => BUILD_TICKS.saturating_mul(4),
        Action::CreateWorkOrder { amount, .. } => WORK_ORDER_TICKS_PER_UNIT
            .saturating_mul(u64::from(*amount))
            .saturating_mul(2)
            .saturating_add(100),
        _ => return Ok(None),
    }
    .min(MAX_DEFAULT_OBLIGATION_TICKS);
    let postconditions = default_postconditions(action, idempotency_key, fortress);
    let terminal = match postconditions.len() {
        1 => postconditions
            .into_iter()
            .next()
            .unwrap_or(Predicate::False),
        _ => Predicate::All(postconditions),
    };
    let deadline_tick = now.checked_add(horizon).ok_or_else(|| {
        DfmcpError::new(
            ErrorCode::BudgetExceeded,
            "default obligation deadline exceeds the game-tick horizon",
        )
    })?;
    Ok(Some(ObligationSpec {
        terminal,
        failure: None,
        deadline_tick,
        poll_interval_ticks: DEFAULT_POLL_INTERVAL_TICKS,
        stable_for_observations: 1,
    }))
}

/// Exact inverse of an action whose prior state is fully determined by the
/// action itself. Actions that overwrite unobserved prior configuration
/// (stockpiles, squads, standing orders) have no default compensation.
#[must_use]
pub fn default_compensation(action: &Action) -> Option<Action> {
    match action {
        Action::Pause { paused } => Some(Action::Pause { paused: !*paused }),
        Action::SetLabor {
            units,
            labor,
            enabled,
        } => Some(Action::SetLabor {
            units: units.clone(),
            labor: labor.clone(),
            enabled: !*enabled,
        }),
        Action::SetBurrowMembership {
            units,
            burrow,
            assigned,
        } => Some(Action::SetBurrowMembership {
            units: units.clone(),
            burrow: *burrow,
            assigned: !*assigned,
        }),
        _ => None,
    }
}

fn precondition(message: impl Into<String>) -> DfmcpError {
    DfmcpError::new(ErrorCode::PreconditionsFailed, message)
}

fn known(value: Value, tick: GameTick) -> Fact {
    Fact::known(
        value,
        tick,
        FactSource::Derived(SOURCE.to_owned()),
        Digest32::ZERO,
    )
}

fn field_value<'a>(entity: &'a EntityRecord, field: &str) -> Option<&'a Value> {
    entity.fields.get(field).and_then(Fact::known_value)
}

fn field_u64(entity: &EntityRecord, field: &str) -> Result<u64> {
    match field_value(entity, field) {
        Some(Value::U64(value)) => Ok(*value),
        _ => Err(precondition(format!(
            "entity {} field {field} requires a known unsigned value before reference progress",
            entity.id.get()
        ))),
    }
}

fn field_text<'a>(entity: &'a EntityRecord, field: &str) -> Option<&'a str> {
    match field_value(entity, field) {
        Some(Value::Text(value)) => Some(value),
        _ => None,
    }
}

/// Write fields onto one entity, advancing its revision once if anything
/// changed. Returns whether the entity changed.
fn write_fields(
    snapshot: &mut WorldSnapshot,
    entity_id: EntityId,
    fields: Vec<(String, Value)>,
) -> Result<bool> {
    let tick = snapshot.tick;
    let entity = snapshot
        .graph
        .entities
        .get_mut(&entity_id)
        .ok_or_else(|| precondition(format!("entity {} is not observed", entity_id.get())))?;
    let mut changed = false;
    for (name, value) in fields {
        if entity.fields.get(&name).map(|fact| &fact.value) != Some(&value)
            || entity
                .fields
                .get(&name)
                .is_some_and(|fact| fact.presence.is_some())
        {
            entity.fields.insert(name, known(value, tick));
            changed = true;
        }
    }
    if changed {
        entity.revision = entity.revision.checked_add(1).ok_or_else(|| {
            DfmcpError::new(ErrorCode::BudgetExceeded, "entity revision is exhausted")
        })?;
    }
    Ok(changed)
}

fn require_kind(snapshot: &WorldSnapshot, entity_id: EntityId, kind: &EntityKind) -> Result<()> {
    match snapshot.graph.entities.get(&entity_id) {
        Some(entity) if &entity.kind == kind => Ok(()),
        Some(entity) => Err(precondition(format!(
            "entity {} is a {}, not a {}",
            entity_id.get(),
            entity.kind.as_str(),
            kind.as_str()
        ))),
        None => Err(precondition(format!(
            "entity {} is not observed",
            entity_id.get()
        ))),
    }
}

fn require_units(snapshot: &WorldSnapshot, units: &[EntityId]) -> Result<()> {
    if units.is_empty() {
        return Err(DfmcpError::new(
            ErrorCode::InvalidRequest,
            "action names no units",
        ));
    }
    for unit in units {
        require_kind(snapshot, *unit, &EntityKind::Unit)?;
    }
    Ok(())
}

fn create_entity(
    snapshot: &mut WorldSnapshot,
    id: EntityId,
    kind: EntityKind,
    label: String,
    fields: Vec<(String, Value)>,
) -> Result<()> {
    if snapshot.graph.entities.contains_key(&id) {
        return Err(DfmcpError::new(
            ErrorCode::Conflict,
            "the step's created entity already exists; an effect is dispatched once",
        ));
    }
    let tick = snapshot.tick;
    let fields: BTreeMap<String, Fact> = fields
        .into_iter()
        .map(|(name, value)| (name, known(value, tick)))
        .collect();
    snapshot.graph.entities.insert(
        id,
        EntityRecord {
            id,
            generation: 1,
            revision: 1,
            kind,
            label,
            fields,
        },
    );
    Ok(())
}

fn count_remaining(snapshot: &WorldSnapshot, area: MapCuboid, target: u32) -> u64 {
    region_tiles(area)
        .filter(|coord| snapshot.tile_code_at(*coord) != Some(target))
        .count() as u64
}

/// Apply the immediate part of `action` to canonical state. Temporal actions
/// create their tracking entity here and progress in [`advance_effects`].
/// Returns whether canonical state changed; the caller owns the cursor and
/// state hash. On error the snapshot may be partially modified, so callers
/// must apply effects to a transaction shadow.
pub fn apply_effect(
    snapshot: &mut WorldSnapshot,
    action: &Action,
    idempotency_key: &str,
) -> Result<bool> {
    match action {
        Action::Pause { paused } => {
            let changed = snapshot.paused != *paused;
            snapshot.paused = *paused;
            Ok(changed)
        }
        Action::SetLabor {
            units,
            labor,
            enabled,
        } => {
            require_units(snapshot, units)?;
            let mut changed = false;
            for unit in units {
                changed |= write_fields(
                    snapshot,
                    *unit,
                    vec![(
                        format!("{LABOR_FIELD_PREFIX}{labor}"),
                        Value::Bool(*enabled),
                    )],
                )?;
            }
            Ok(changed)
        }
        Action::SetBurrowMembership {
            units,
            burrow,
            assigned,
        } => {
            require_units(snapshot, units)?;
            require_kind(snapshot, *burrow, &EntityKind::Burrow)?;
            let mut changed = false;
            for unit in units {
                changed |= write_fields(
                    snapshot,
                    *unit,
                    vec![(
                        format!("{BURROW_FIELD_PREFIX}{}", burrow.get()),
                        Value::Bool(*assigned),
                    )],
                )?;
            }
            Ok(changed)
        }
        Action::AssignSquad { units, squad } => {
            require_units(snapshot, units)?;
            require_kind(snapshot, *squad, &EntityKind::Squad)?;
            let mut changed = false;
            for unit in units {
                changed |= write_fields(
                    snapshot,
                    *unit,
                    vec![(SQUAD_FIELD.to_owned(), Value::Entity(*squad))],
                )?;
            }
            Ok(changed)
        }
        Action::ConfigureStockpile {
            stockpile,
            accepts,
            max_bins,
            max_barrels,
            max_wheelbarrows,
        } => {
            require_kind(snapshot, *stockpile, &EntityKind::Stockpile)?;
            let mut fields = vec![(STOCKPILE_ACCEPTS_FIELD.to_owned(), accepts_value(accepts))];
            for (name, value) in [
                (STOCKPILE_MAX_BINS_FIELD, max_bins),
                (STOCKPILE_MAX_BARRELS_FIELD, max_barrels),
                (STOCKPILE_MAX_WHEELBARROWS_FIELD, max_wheelbarrows),
            ] {
                if let Some(value) = value {
                    fields.push((name.to_owned(), Value::U64(u64::from(*value))));
                }
            }
            write_fields(snapshot, *stockpile, fields)
        }
        Action::SetStandingOrder { key, value } => {
            let settings = fortress_settings_entity_id(snapshot.fortress_id);
            let field = (
                format!("{STANDING_ORDER_FIELD_PREFIX}{key}"),
                Value::Text(value.clone()),
            );
            if snapshot.graph.entities.contains_key(&settings) {
                require_kind(
                    snapshot,
                    settings,
                    &EntityKind::Other(FORTRESS_SETTINGS_KIND.to_owned()),
                )?;
                write_fields(snapshot, settings, vec![field])
            } else {
                create_entity(
                    snapshot,
                    settings,
                    EntityKind::Other(FORTRESS_SETTINGS_KIND.to_owned()),
                    "fortress settings".to_owned(),
                    vec![field],
                )?;
                Ok(true)
            }
        }
        Action::DesignateDig { area, mode } => {
            let tiles = validate_region(*area)?;
            if let Some(unknown) = region_tiles(*area).find(|c| snapshot.tile_code_at(*c).is_none())
            {
                return Err(precondition(format!(
                    "excavation region includes unobserved terrain at {unknown:?}"
                )));
            }
            let target = dig_target_tile_code(*mode);
            let remaining = count_remaining(snapshot, *area, target);
            create_entity(
                snapshot,
                created_entity_id(idempotency_key, 0),
                EntityKind::Other(DIG_DESIGNATION_KIND.to_owned()),
                format!("{} designation", dig_mode_name(*mode)),
                vec![
                    (
                        "mode".to_owned(),
                        Value::Text(dig_mode_name(*mode).to_owned()),
                    ),
                    ("area_min".to_owned(), Value::Coord(area.min)),
                    ("area_max".to_owned(), Value::Coord(area.max)),
                    ("target_tile_code".to_owned(), Value::U64(u64::from(target))),
                    ("tiles_total".to_owned(), Value::U64(tiles)),
                    (TILES_REMAINING_FIELD.to_owned(), Value::U64(remaining)),
                    ("work_ticks".to_owned(), Value::U64(0)),
                    (
                        STATUS_FIELD.to_owned(),
                        Value::Text(
                            if remaining == 0 {
                                STATUS_COMPLETE
                            } else {
                                STATUS_ACTIVE
                            }
                            .to_owned(),
                        ),
                    ),
                ],
            )?;
            Ok(true)
        }
        Action::Build {
            kind,
            location,
            footprint,
            material,
        } => {
            if !footprint.contains(*location) {
                return Err(DfmcpError::new(
                    ErrorCode::InvalidRequest,
                    "building location lies outside its footprint",
                ));
            }
            validate_region(*footprint)?;
            for coord in region_tiles(*footprint) {
                match snapshot.tile_code_at(coord) {
                    Some(tile_codes::FLOOR) => {}
                    Some(_) => {
                        return Err(precondition(format!(
                            "building footprint tile {coord:?} is not open floor"
                        )));
                    }
                    None => {
                        return Err(precondition(format!(
                            "building footprint tile {coord:?} is not observed"
                        )));
                    }
                }
            }
            let label = building_kind_label(kind);
            create_entity(
                snapshot,
                created_entity_id(idempotency_key, 0),
                EntityKind::Building,
                label.clone(),
                vec![
                    ("building_kind".to_owned(), Value::Text(label)),
                    ("position".to_owned(), Value::Coord(*location)),
                    ("footprint_min".to_owned(), Value::Coord(footprint.min)),
                    ("footprint_max".to_owned(), Value::Coord(footprint.max)),
                    (
                        "material_tokens".to_owned(),
                        Value::List(
                            material
                                .required_tokens
                                .iter()
                                .cloned()
                                .map(Value::Text)
                                .collect(),
                        ),
                    ),
                    (
                        CONSTRUCTION_STAGE_FIELD.to_owned(),
                        Value::Text(STAGE_PLANNED.to_owned()),
                    ),
                    ("progress_ticks".to_owned(), Value::U64(0)),
                    ("required_ticks".to_owned(), Value::U64(BUILD_TICKS)),
                ],
            )?;
            Ok(true)
        }
        Action::CreateWorkOrder {
            name,
            job_token,
            amount,
            ..
        } => {
            if *amount == 0 {
                return Err(DfmcpError::new(
                    ErrorCode::InvalidRequest,
                    "work order amount must be positive",
                ));
            }
            create_entity(
                snapshot,
                created_entity_id(idempotency_key, 0),
                EntityKind::WorkOrder,
                name.clone(),
                vec![
                    ("job_token".to_owned(), Value::Text(job_token.clone())),
                    ("amount_total".to_owned(), Value::U64(u64::from(*amount))),
                    (
                        AMOUNT_REMAINING_FIELD.to_owned(),
                        Value::U64(u64::from(*amount)),
                    ),
                    ("work_ticks".to_owned(), Value::U64(0)),
                    (
                        STATUS_FIELD.to_owned(),
                        Value::Text(STATUS_ACTIVE.to_owned()),
                    ),
                ],
            )?;
            Ok(true)
        }
        Action::Extension {
            namespace, name, ..
        } => Err(DfmcpError::new(
            ErrorCode::AdapterRejected,
            format!("extension action {namespace}.{name} has no reference semantics"),
        )),
    }
}

/// Stop the temporal work a dispatched step started, without undoing any
/// progress already made (excavated tiles stay excavated). Immediate actions
/// have no ongoing work. Returns whether canonical state changed.
pub fn cancel_effect(
    snapshot: &mut WorldSnapshot,
    action: &Action,
    idempotency_key: &str,
) -> Result<bool> {
    let (field, active, cancelled): (&str, &[&str], &str) = match action {
        Action::DesignateDig { .. } | Action::CreateWorkOrder { .. } => {
            (STATUS_FIELD, &[STATUS_ACTIVE], STATUS_CANCELLED)
        }
        Action::Build { .. } => (
            CONSTRUCTION_STAGE_FIELD,
            &[STAGE_PLANNED, STAGE_UNDER_CONSTRUCTION],
            STAGE_CANCELLED,
        ),
        _ => return Ok(false),
    };
    let id = created_entity_id(idempotency_key, 0);
    let Some(created) = snapshot.graph.entities.get(&id) else {
        // Never dispatched (for example, still waiting on a dependency).
        return Ok(false);
    };
    if !field_text(created, field).is_some_and(|value| active.contains(&value)) {
        return Ok(false);
    }
    write_fields(
        snapshot,
        id,
        vec![(field.to_owned(), Value::Text(cancelled.to_owned()))],
    )
}

fn coord_field(entity: &EntityRecord, field: &str) -> Option<MapCoord> {
    match field_value(entity, field) {
        Some(Value::Coord(coord)) => Some(*coord),
        _ => None,
    }
}

/// Progress all active temporal work by `elapsed` game ticks, deterministically
/// in ascending entity order. Call after advancing `snapshot.tick`. Returns
/// whether canonical state changed; the caller owns the cursor and hash.
/// Unavailable progress inputs refuse advancement; they are never zero work
/// or completed work. As with [`apply_effect`], use a transaction shadow so a
/// later refusal does not publish earlier effects from the same advancement.
pub fn advance_effects(snapshot: &mut WorldSnapshot, elapsed: u64) -> Result<bool> {
    if elapsed == 0 {
        return Ok(false);
    }
    let designation_kind = EntityKind::Other(DIG_DESIGNATION_KIND.to_owned());
    let active: Vec<(EntityId, EntityKind)> = snapshot
        .graph
        .entities
        .values()
        .filter(|entity| match &entity.kind {
            EntityKind::WorkOrder => field_text(entity, STATUS_FIELD) == Some(STATUS_ACTIVE),
            EntityKind::Building => matches!(
                field_text(entity, CONSTRUCTION_STAGE_FIELD),
                Some(STAGE_PLANNED | STAGE_UNDER_CONSTRUCTION)
            ),
            kind if kind == &designation_kind => {
                field_text(entity, STATUS_FIELD) == Some(STATUS_ACTIVE)
            }
            _ => false,
        })
        .map(|entity| (entity.id, entity.kind.clone()))
        .collect();
    let mut changed = false;
    for (id, kind) in active {
        changed |= match kind {
            EntityKind::WorkOrder => advance_work_order(snapshot, id, elapsed)?,
            EntityKind::Building => advance_building(snapshot, id, elapsed)?,
            _ => advance_designation(snapshot, id, elapsed)?,
        };
    }
    changed |= advance_metabolism(snapshot, elapsed)?;
    changed |= advance_threats(snapshot, elapsed)?;
    Ok(changed)
}

fn is_alive(entity: &EntityRecord) -> bool {
    // Older reference fixtures omit this optional field for living units.
    // An explicit unavailable value must not inherit that legacy default.
    entity
        .fields
        .get("alive")
        .is_none_or(|fact| fact.known_value() == Some(&Value::Bool(true)))
}

// Legacy reference worlds omit optional defaults, but an explicit unavailable
// selector cannot establish a population count or deterministic victim order.
fn require_population_fields(snapshot: &WorldSnapshot, combat: bool) -> Result<()> {
    for unit in snapshot
        .graph
        .entities
        .values()
        .filter(|unit| unit.kind == EntityKind::Unit)
    {
        if let Some(fact) = unit.fields.get("alive")
            && !matches!(fact.known_value(), Some(Value::Bool(_)))
        {
            return Err(precondition(format!(
                "unit {} has unavailable life state; population progress requires a known census",
                unit.id.get()
            )));
        }
        if !combat || !is_alive(unit) {
            continue;
        }
        if let Some(fact) = unit.fields.get(SQUAD_FIELD)
            && !matches!(fact.known_value(), Some(Value::Entity(_) | Value::Null))
        {
            return Err(precondition(format!(
                "unit {} has unavailable squad membership before combat progress",
                unit.id.get()
            )));
        }
        if unit.fields.iter().any(|(name, fact)| {
            name.starts_with(BURROW_FIELD_PREFIX)
                && !matches!(fact.known_value(), Some(Value::Bool(_)))
        }) {
            return Err(precondition(format!(
                "unit {} has unavailable burrow membership before combat progress",
                unit.id.get()
            )));
        }
    }
    Ok(())
}

/// Hostile creatures arrive at their scheduled tick and then fight in
/// rounds. Living squad members wound the hostile; with no soldiers it kills
/// one exposed dwarf (alive and in no burrow) every few rounds, highest id
/// first. A slain hostile stops. Everything is a deterministic function of
/// the snapshot and elapsed ticks (one long wait equals many short ones up to
/// round boundaries).
fn advance_threats(snapshot: &mut WorldSnapshot, elapsed: u64) -> Result<bool> {
    let hostiles: Vec<EntityId> = snapshot
        .graph
        .entities
        .values()
        .filter(|entity| {
            entity.kind == EntityKind::Creature
                && field_value(entity, HOSTILE_FIELD) == Some(&Value::Bool(true))
                && matches!(
                    field_text(entity, THREAT_STATUS_FIELD),
                    Some(THREAT_APPROACHING | THREAT_ATTACKING)
                )
        })
        .map(|entity| entity.id)
        .collect();
    if !hostiles.is_empty() {
        require_population_fields(snapshot, true)?;
    }
    let now = snapshot.tick.0;
    let start = now.saturating_sub(elapsed);
    let mut changed = false;
    for hostile in hostiles {
        let arrives = field_u64(entity(snapshot, hostile)?, ARRIVES_AT_FIELD)?;
        if now < arrives {
            continue;
        }
        if field_text(entity(snapshot, hostile)?, THREAT_STATUS_FIELD) != Some(THREAT_ATTACKING) {
            changed |= write_fields(
                snapshot,
                hostile,
                vec![(
                    THREAT_STATUS_FIELD.to_owned(),
                    Value::Text(THREAT_ATTACKING.to_owned()),
                )],
            )?;
        }
        // Rounds completed since arrival, before and after this advance.
        let since = |tick: u64| tick.saturating_sub(arrives) / COMBAT_ROUND_TICKS;
        let rounds = since(now) - since(start.max(arrives));
        for _ in 0..rounds {
            let soldiers = snapshot
                .graph
                .entities
                .values()
                .filter(|unit| {
                    unit.kind == EntityKind::Unit
                        && is_alive(unit)
                        && matches!(field_value(unit, SQUAD_FIELD), Some(Value::Entity(_)))
                })
                .count() as u64;
            let creature = entity(snapshot, hostile)?;
            let fought = field_u64(creature, COMBAT_ROUNDS_FIELD)?.saturating_add(1);
            let health = field_u64(creature, HEALTH_FIELD)?
                .saturating_sub(soldiers * SOLDIER_DAMAGE_PER_ROUND);
            let mut fields = vec![
                (COMBAT_ROUNDS_FIELD.to_owned(), Value::U64(fought)),
                (HEALTH_FIELD.to_owned(), Value::U64(health)),
            ];
            if health == 0 {
                fields.push((
                    THREAT_STATUS_FIELD.to_owned(),
                    Value::Text(THREAT_SLAIN.to_owned()),
                ));
            }
            changed |= write_fields(snapshot, hostile, fields)?;
            if health == 0 {
                break;
            }
            if soldiers == 0 && fought.is_multiple_of(ROUNDS_PER_KILL) {
                let victim = snapshot
                    .graph
                    .entities
                    .values()
                    .rev()
                    .find(|unit| {
                        unit.kind == EntityKind::Unit
                            && is_alive(unit)
                            && unit.fields.iter().all(|(name, fact)| {
                                !name.starts_with(BURROW_FIELD_PREFIX)
                                    || fact.known_value() == Some(&Value::Bool(false))
                            })
                    })
                    .map(|unit| unit.id);
                if let Some(victim) = victim {
                    changed |= write_fields(
                        snapshot,
                        victim,
                        vec![
                            ("alive".to_owned(), Value::Bool(false)),
                            (
                                "cause_of_death".to_owned(),
                                Value::Text("hostile attack".to_owned()),
                            ),
                        ],
                    )?;
                }
            }
        }
    }
    Ok(changed)
}

/// The fortress's stock ledger, if the world has one (lowest id wins).
#[must_use]
pub fn stock_ledger(snapshot: &WorldSnapshot) -> Option<EntityId> {
    snapshot
        .graph
        .entities
        .values()
        .find(|entity| matches!(&entity.kind, EntityKind::Other(kind) if kind == STOCK_LEDGER_KIND))
        .map(|entity| entity.id)
}

/// Living dwarves consume drink and food as game time passes. Production
/// (above) is applied first, so a meal finished in the same interval feeds.
/// When a stock runs short the dwarves served last (highest id) go without
/// and their need turns `thirsty` / `hungry`; a later full meal satisfies
/// everyone again. A world without a stock ledger has no metabolism.
fn advance_metabolism(snapshot: &mut WorldSnapshot, elapsed: u64) -> Result<bool> {
    let Some(ledger) = stock_ledger(snapshot) else {
        return Ok(false);
    };
    require_population_fields(snapshot, false)?;
    let before = field_u64(entity(snapshot, ledger)?, METABOLISM_TICKS_FIELD)?;
    let after = before.saturating_add(elapsed);
    let living: Vec<EntityId> = snapshot
        .graph
        .entities
        .values()
        .filter(|entity| entity.kind == EntityKind::Unit && is_alive(entity))
        .map(|entity| entity.id)
        .collect();
    let mut changed = write_fields(
        snapshot,
        ledger,
        vec![(METABOLISM_TICKS_FIELD.to_owned(), Value::U64(after))],
    )?;
    for (stock_field, interval, need_field, deprived) in [
        (
            STOCK_DRINK_FIELD,
            DRINK_INTERVAL_TICKS,
            NEED_DRINK_FIELD,
            "thirsty",
        ),
        (
            STOCK_FOOD_FIELD,
            FOOD_INTERVAL_TICKS,
            NEED_FOOD_FIELD,
            "hungry",
        ),
    ] {
        let meals = after / interval - before / interval;
        if meals == 0 || living.is_empty() {
            continue;
        }
        let mut held = field_u64(entity(snapshot, ledger)?, stock_field)?;
        let mut served = living.len();
        for _ in 0..meals {
            let wanted = living.len() as u64;
            served = usize::try_from(held.min(wanted)).unwrap_or(living.len());
            held -= held.min(wanted);
        }
        changed |= write_fields(
            snapshot,
            ledger,
            vec![(stock_field.to_owned(), Value::U64(held))],
        )?;
        for (index, unit) in living.iter().enumerate() {
            let need = if index < served {
                "satisfied"
            } else {
                deprived
            };
            changed |= write_fields(
                snapshot,
                *unit,
                vec![(need_field.to_owned(), Value::Text(need.to_owned()))],
            )?;
        }
    }
    Ok(changed)
}

fn entity(snapshot: &WorldSnapshot, id: EntityId) -> Result<&EntityRecord> {
    snapshot.graph.entities.get(&id).ok_or_else(|| {
        DfmcpError::new(
            ErrorCode::InternalInvariantViolation,
            "active effect entity disappeared during progress",
        )
    })
}

fn advance_work_order(snapshot: &mut WorldSnapshot, id: EntityId, elapsed: u64) -> Result<bool> {
    let order = entity(snapshot, id)?;
    let job = field_text(order, "job_token").ok_or_else(|| {
        precondition(format!(
            "work order {} requires a known job token before reference progress",
            id.get()
        ))
    })?;
    let work = field_u64(order, "work_ticks")?.saturating_add(elapsed);
    let remaining = field_u64(order, AMOUNT_REMAINING_FIELD)?;
    let product = work_order_product(job);
    let blocker = work_order_blocker(snapshot, job);
    // A stalled order accrues no work; it says what it is waiting for.
    let blocked = write_fields(
        snapshot,
        id,
        vec![(
            BLOCKED_BY_FIELD.to_owned(),
            blocker.clone().map_or(Value::Null, Value::Text),
        )],
    )?;
    if blocker.is_some() {
        return Ok(blocked);
    }
    let produced = (work / WORK_ORDER_TICKS_PER_UNIT).min(remaining);
    let remaining = remaining - produced;
    if produced > 0
        && let Some((stock_field, per_unit)) = product
        && let Some(ledger) = stock_ledger(snapshot)
    {
        let held = field_u64(entity(snapshot, ledger)?, stock_field)?;
        write_fields(
            snapshot,
            ledger,
            vec![(
                stock_field.to_owned(),
                Value::U64(held.saturating_add(produced.saturating_mul(per_unit))),
            )],
        )?;
    }
    let mut fields = vec![
        (AMOUNT_REMAINING_FIELD.to_owned(), Value::U64(remaining)),
        (
            "work_ticks".to_owned(),
            Value::U64(if remaining == 0 {
                0
            } else {
                work % WORK_ORDER_TICKS_PER_UNIT
            }),
        ),
    ];
    if remaining == 0 {
        fields.push((
            STATUS_FIELD.to_owned(),
            Value::Text(STATUS_COMPLETE.to_owned()),
        ));
    }
    Ok(write_fields(snapshot, id, fields)? | blocked)
}

fn advance_building(snapshot: &mut WorldSnapshot, id: EntityId, elapsed: u64) -> Result<bool> {
    let building = entity(snapshot, id)?;
    let required = field_u64(building, "required_ticks")?;
    if required == 0 {
        return Err(precondition(
            "building duration must be positive before reference progress",
        ));
    }
    let progress = field_u64(building, "progress_ticks")?
        .saturating_add(elapsed)
        .min(required);
    let stage = if progress >= required {
        STAGE_COMPLETE
    } else {
        STAGE_UNDER_CONSTRUCTION
    };
    write_fields(
        snapshot,
        id,
        vec![
            ("progress_ticks".to_owned(), Value::U64(progress)),
            (
                CONSTRUCTION_STAGE_FIELD.to_owned(),
                Value::Text(stage.to_owned()),
            ),
        ],
    )
}

fn advance_designation(snapshot: &mut WorldSnapshot, id: EntityId, elapsed: u64) -> Result<bool> {
    let designation = entity(snapshot, id)?;
    let (Some(min), Some(max)) = (
        coord_field(designation, "area_min"),
        coord_field(designation, "area_max"),
    ) else {
        return Err(DfmcpError::new(
            ErrorCode::InternalInvariantViolation,
            "dig designation lost its area",
        ));
    };
    let area = MapCuboid::new(min, max)?;
    let target = u32::try_from(field_u64(designation, "target_tile_code")?).map_err(|_| {
        DfmcpError::new(
            ErrorCode::InternalInvariantViolation,
            "dig designation target tile code is invalid",
        )
    })?;
    let work = field_u64(designation, "work_ticks")?.saturating_add(elapsed);
    let budget = work / DIG_TICKS_PER_TILE;
    let mut changed = false;
    if budget > 0 {
        let pending: Vec<MapCoord> = region_tiles(area)
            .filter(|coord| {
                snapshot
                    .tile_code_at(*coord)
                    .is_some_and(|code| code != target)
            })
            .take(usize::try_from(budget).unwrap_or(usize::MAX))
            .collect();
        for coord in pending {
            changed |= snapshot.set_tile_code(coord, target)?;
        }
    }
    let remaining = count_remaining(snapshot, area, target);
    let mut fields = vec![
        (TILES_REMAINING_FIELD.to_owned(), Value::U64(remaining)),
        (
            "work_ticks".to_owned(),
            Value::U64(if remaining == 0 {
                0
            } else {
                work % DIG_TICKS_PER_TILE
            }),
        ),
    ];
    if remaining == 0 {
        fields.push((
            STATUS_FIELD.to_owned(),
            Value::Text(STATUS_COMPLETE.to_owned()),
        ));
    }
    changed |= write_fields(snapshot, id, fields)?;
    Ok(changed)
}

#[cfg(test)]
#[path = "effects_tests.rs"]
mod tests;
