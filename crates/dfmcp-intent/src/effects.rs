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

use std::collections::{BTreeMap, BTreeSet};

use dfmcp_core::{
    DfmcpError, Digest32, EntityId, ErrorCode, FortressId, GameTick, MapCoord, MapCuboid, Result,
};
use dfmcp_world::terrain::{region_tiles, tile_codes, validate_region};
use dfmcp_world::{
    CompareOp, EntityKind, EntityRecord, Fact, FactSource, Predicate, Value, WorldSnapshot,
    laboratory_fact_value,
};

use crate::action::{Action, BuildingKind, DigMode, WorkOrderCondition};
use crate::plan::ObligationSpec;

#[path = "effects_capacity.rs"]
mod capacity;

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
/// Explicit semantic name used to resolve a unique work-order dependency.
/// An entity's display label does not establish this identity.
pub const WORK_ORDER_NAME_FIELD: &str = "order_name";
/// Canonical typed condition list. An explicit empty list is unconditional;
/// absence cannot recover conditions discarded by an older implementation.
pub const WORK_ORDER_CONDITIONS_FIELD: &str = "conditions";
/// Bound checked before allocating or evaluating a work-order condition list.
pub const MAX_WORK_ORDER_CONDITIONS: usize = 64;
/// Reference condition token/name bound, independent of a planner's policy.
pub const MAX_WORK_ORDER_CONDITION_BYTES: usize = 256;
/// Exact generic inventory token namespaces on the reference stock ledger.
pub const STOCK_ITEM_FIELD_PREFIX: &str = "stock.item.";
pub const STOCK_MATERIAL_FIELD_PREFIX: &str = "stock.material.";

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
    capacity::requirements(job_token)
}

/// Field a stalled work order carries naming what it is waiting for.
pub const BLOCKED_BY_FIELD: &str = "blocked_by";

/// What `job_token` still needs to establish in the laboratory before it can
/// progress: a completed matching workshop and a living eligible worker.
#[must_use]
pub fn work_order_blocker(snapshot: &WorldSnapshot, job_token: &str) -> Option<String> {
    let (workshop, labor) = work_order_requirements(job_token)?;
    let entities = snapshot.graph.entities.values();
    let has_workshop = entities.clone().any(|entity| {
        entity.kind == EntityKind::Building
            && field_text(entity, "building_kind", snapshot.tick) == Some(workshop)
            && field_text(entity, CONSTRUCTION_STAGE_FIELD, snapshot.tick) == Some(STAGE_COMPLETE)
    });
    let labor_field = format!("{LABOR_FIELD_PREFIX}{labor}");
    let has_worker = entities.clone().any(|entity| {
        entity.kind == EntityKind::Unit
            && is_alive(entity, snapshot.tick)
            && field_value(entity, &labor_field, snapshot.tick) == Some(&Value::Bool(true))
    });
    match (has_workshop, has_worker) {
        (true, true) => None,
        (false, true) => Some(format!("completed {workshop} is not established")),
        (true, false) => Some(format!(
            "living unit with the {labor} labor enabled is not established"
        )),
        (false, false) => Some(format!(
            "completed {workshop} and living unit with the {labor} labor enabled are not established"
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

fn validate_condition_text(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > MAX_WORK_ORDER_CONDITION_BYTES
        || value.chars().any(char::is_control)
    {
        return Err(DfmcpError::new(
            ErrorCode::InvalidRequest,
            "work-order condition names and tokens require 1..=256 non-control bytes",
        ));
    }
    Ok(())
}

fn condition_key(condition: &WorkOrderCondition) -> (u8, &str, u32) {
    match condition {
        WorkOrderCondition::ItemCountBelow {
            item_token,
            threshold,
        } => (0, item_token, *threshold),
        WorkOrderCondition::MaterialAvailable {
            material_token,
            minimum,
        } => (1, material_token, *minimum),
        WorkOrderCondition::CompletedOrder { order_name } => (2, order_name, 0),
    }
}

fn canonical_conditions(conditions: &[WorkOrderCondition]) -> Result<Vec<WorkOrderCondition>> {
    if conditions.len() > MAX_WORK_ORDER_CONDITIONS {
        return Err(DfmcpError::new(
            ErrorCode::BudgetExceeded,
            "work order exceeds the 64-condition reference bound",
        ));
    }
    for condition in conditions {
        validate_condition_text(condition_key(condition).1)?;
    }
    let mut normalized = conditions.to_vec();
    normalized.sort_by(|left, right| condition_key(left).cmp(&condition_key(right)));
    normalized.dedup();
    Ok(normalized)
}

fn condition_value(condition: &WorkOrderCondition) -> Value {
    let fields = match condition {
        WorkOrderCondition::ItemCountBelow {
            item_token,
            threshold,
        } => vec![
            ("kind", Value::Text("item_count_below".to_owned())),
            ("item_token", Value::Text(item_token.clone())),
            ("threshold", Value::U64(u64::from(*threshold))),
        ],
        WorkOrderCondition::MaterialAvailable {
            material_token,
            minimum,
        } => vec![
            ("kind", Value::Text("material_available".to_owned())),
            ("material_token", Value::Text(material_token.clone())),
            ("minimum", Value::U64(u64::from(*minimum))),
        ],
        WorkOrderCondition::CompletedOrder { order_name } => vec![
            ("kind", Value::Text("completed_order".to_owned())),
            ("order_name", Value::Text(order_name.clone())),
        ],
    };
    Value::Object(
        fields
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect(),
    )
}

/// Bounded canonical condition records used by persistence and sealed proof.
/// Conditions are conjunctive; reordering and exact duplicates have no effect.
pub fn work_order_conditions_value(conditions: &[WorkOrderCondition]) -> Result<Value> {
    Ok(Value::List(
        canonical_conditions(conditions)?
            .iter()
            .map(condition_value)
            .collect(),
    ))
}

fn decode_conditions(value: &Value) -> Result<Vec<WorkOrderCondition>> {
    let invalid =
        || precondition("work-order conditions require canonical typed condition records");
    let Value::List(values) = value else {
        return Err(invalid());
    };
    if values.len() > MAX_WORK_ORDER_CONDITIONS {
        return Err(precondition(
            "work-order condition record exceeds its 64-condition bound",
        ));
    }
    let mut conditions = Vec::with_capacity(values.len());
    for value in values {
        let Value::Object(fields) = value else {
            return Err(invalid());
        };
        let text = |key: &str| -> Result<String> {
            let Some(Value::Text(value)) = fields.get(key) else {
                return Err(invalid());
            };
            validate_condition_text(value).map_err(|_| invalid())?;
            Ok(value.clone())
        };
        let amount = |key: &str| -> Result<u32> {
            let Some(Value::U64(value)) = fields.get(key) else {
                return Err(invalid());
            };
            u32::try_from(*value).map_err(|_| invalid())
        };
        let condition = match fields.get("kind") {
            Some(Value::Text(kind)) if kind == "item_count_below" && fields.len() == 3 => {
                WorkOrderCondition::ItemCountBelow {
                    item_token: text("item_token")?,
                    threshold: amount("threshold")?,
                }
            }
            Some(Value::Text(kind)) if kind == "material_available" && fields.len() == 3 => {
                WorkOrderCondition::MaterialAvailable {
                    material_token: text("material_token")?,
                    minimum: amount("minimum")?,
                }
            }
            Some(Value::Text(kind)) if kind == "completed_order" && fields.len() == 2 => {
                WorkOrderCondition::CompletedOrder {
                    order_name: text("order_name")?,
                }
            }
            _ => return Err(invalid()),
        };
        conditions.push(condition);
    }
    // Stored order and multiplicity are canonical semantic state, not a second
    // accepted encoding that can change under an idempotent retry.
    if work_order_conditions_value(&conditions)? != *value {
        return Err(invalid());
    }
    Ok(conditions)
}

fn item_stock_field(token: &str) -> String {
    match token {
        "DRINK" => STOCK_DRINK_FIELD.to_owned(),
        "FOOD" => STOCK_FOOD_FIELD.to_owned(),
        _ => format!("{STOCK_ITEM_FIELD_PREFIX}{token}"),
    }
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
        Action::CreateWorkOrder {
            name,
            job_token,
            conditions,
            ..
        } => {
            if validate_condition_text(name).is_err() || validate_condition_text(job_token).is_err()
            {
                return vec![Predicate::False];
            }
            let Ok(conditions) = work_order_conditions_value(conditions) else {
                // This infallible template must never turn a refused condition
                // shape into an unconditional completion claim.
                return vec![Predicate::False];
            };
            let id = created_entity_id(idempotency_key, 0);
            vec![
                field_eq(id, AMOUNT_REMAINING_FIELD, Value::U64(0)),
                field_eq(id, STATUS_FIELD, Value::Text(STATUS_COMPLETE.to_owned())),
                field_eq(id, WORK_ORDER_NAME_FIELD, Value::Text(name.clone())),
                field_eq(id, "job_token", Value::Text(job_token.clone())),
                field_eq(id, WORK_ORDER_CONDITIONS_FIELD, conditions),
            ]
        }
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

fn field_value<'a>(entity: &'a EntityRecord, field: &str, tick: GameTick) -> Option<&'a Value> {
    entity
        .fields
        .get(field)
        .and_then(|fact| laboratory_fact_value(fact, tick))
}

fn field_u64(entity: &EntityRecord, field: &str, tick: GameTick) -> Result<u64> {
    match field_value(entity, field, tick) {
        Some(Value::U64(value)) => Ok(*value),
        _ => Err(precondition(format!(
            "entity {} field {field} requires an eligible known unsigned value before reference progress",
            entity.id.get()
        ))),
    }
}

fn field_text<'a>(entity: &'a EntityRecord, field: &str, tick: GameTick) -> Option<&'a str> {
    match field_value(entity, field, tick) {
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
        // An explicit action establishes its result even when the old
        // compatibility value agrees but has unavailable or untrusted provenance.
        if entity
            .fields
            .get(&name)
            .and_then(|fact| laboratory_fact_value(fact, tick))
            != Some(&value)
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
            conditions,
        } => {
            if *amount == 0 {
                return Err(DfmcpError::new(
                    ErrorCode::InvalidRequest,
                    "work order amount must be positive",
                ));
            }
            validate_condition_text(name)?;
            validate_condition_text(job_token)?;
            let conditions = work_order_conditions_value(conditions)?;
            create_entity(
                snapshot,
                created_entity_id(idempotency_key, 0),
                EntityKind::WorkOrder,
                name.clone(),
                vec![
                    (WORK_ORDER_NAME_FIELD.to_owned(), Value::Text(name.clone())),
                    (WORK_ORDER_CONDITIONS_FIELD.to_owned(), conditions),
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
    match field_text(created, field, snapshot.tick) {
        Some(value) if active.contains(&value) => {}
        Some(STATUS_COMPLETE | STATUS_CANCELLED) => return Ok(false),
        _ => {
            return Err(precondition(format!(
                "entity {} has no eligible lifecycle state for cancellation",
                id.get()
            )));
        }
    }
    write_fields(
        snapshot,
        id,
        vec![(field.to_owned(), Value::Text(cancelled.to_owned()))],
    )
}

fn coord_field(entity: &EntityRecord, field: &str, tick: GameTick) -> Option<MapCoord> {
    match field_value(entity, field, tick) {
        Some(Value::Coord(coord)) => Some(*coord),
        _ => None,
    }
}

// A lifecycle or hostile selector is an input to the workload and census,
// not permission to silently drop an unavailable contributor from the model.
// Ordinary buildings and creatures without model fields remain valid fixtures.
fn require_progress_selectors(snapshot: &WorldSnapshot) -> Result<()> {
    for entity in snapshot.graph.entities.values() {
        let lifecycle = match &entity.kind {
            EntityKind::WorkOrder => Some((STATUS_FIELD, false)),
            EntityKind::Building
                if entity.fields.contains_key("progress_ticks")
                    || entity.fields.contains_key("required_ticks") =>
            {
                Some((CONSTRUCTION_STAGE_FIELD, true))
            }
            EntityKind::Other(kind) if kind == DIG_DESIGNATION_KIND => Some((STATUS_FIELD, false)),
            _ => None,
        };
        if let Some((field, construction)) = lifecycle {
            let eligible = match field_text(entity, field, snapshot.tick) {
                Some(STATUS_COMPLETE | STATUS_CANCELLED) => true,
                Some(STAGE_PLANNED | STAGE_UNDER_CONSTRUCTION) => construction,
                Some(STATUS_ACTIVE) => !construction,
                _ => false,
            };
            if !eligible {
                return Err(precondition(format!(
                    "entity {} has no eligible lifecycle selector before reference progress",
                    entity.id.get()
                )));
            }
        }
        if entity.kind == EntityKind::Creature
            && (entity.fields.contains_key(HOSTILE_FIELD)
                || entity.fields.contains_key(THREAT_STATUS_FIELD))
        {
            match field_value(entity, HOSTILE_FIELD, snapshot.tick) {
                Some(Value::Bool(false)) => {}
                Some(Value::Bool(true))
                    if matches!(
                        field_text(entity, THREAT_STATUS_FIELD, snapshot.tick),
                        Some(THREAT_APPROACHING | THREAT_ATTACKING | THREAT_SLAIN)
                    ) => {}
                _ => {
                    return Err(precondition(format!(
                        "creature {} has no eligible hostile selector before reference progress",
                        entity.id.get()
                    )));
                }
            }
        }
    }
    Ok(())
}

/// Hard limits for one reference advancement. A caller may reduce these limits,
/// but cannot turn a large request into an unbounded simulation loop.
pub const MAX_EFFECT_ADVANCE_TICKS: u64 = 1_000_000;
pub const MAX_EFFECT_ADVANCE_EVENTS: u64 = 200_000;
pub const MAX_EFFECT_ADVANCE_WORK_UNITS: u64 = 100_000_000;
/// Aggregate excavation footprint admitted before allocating any pending-tile
/// sets. A work budget alone must not permit enormous simultaneous caches.
pub const MAX_EFFECT_ADVANCE_CACHED_DIG_TILES: u64 = 1_048_576;
/// Canonical entity domain admitted before allocating production capacity
/// matching state. Each source unit and completed workshop has capacity one.
pub const MAX_EFFECT_ADVANCE_PRODUCTION_ENTITIES: usize = 65_536;

/// Explicit CPU-work budget for the reference event timeline. Work units charge
/// source facts, entity and condition scans, and terrain visits before the work.
/// They are a deterministic algorithmic bound, not elapsed wall-clock time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EffectAdvanceLimits {
    pub max_game_ticks: u64,
    pub max_events: u64,
    pub max_work_units: u64,
}

impl Default for EffectAdvanceLimits {
    fn default() -> Self {
        Self {
            max_game_ticks: MAX_EFFECT_ADVANCE_TICKS,
            max_events: MAX_EFFECT_ADVANCE_EVENTS,
            max_work_units: MAX_EFFECT_ADVANCE_WORK_UNITS,
        }
    }
}

struct EffectAdvanceBudget {
    limits: EffectAdvanceLimits,
    events: u64,
    work_units: u64,
}

fn advance_budget_error(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::BudgetExceeded, message)
}

impl EffectAdvanceBudget {
    fn new(limits: EffectAdvanceLimits, elapsed: u64) -> Result<Self> {
        if limits.max_game_ticks > MAX_EFFECT_ADVANCE_TICKS
            || limits.max_events > MAX_EFFECT_ADVANCE_EVENTS
            || limits.max_work_units > MAX_EFFECT_ADVANCE_WORK_UNITS
        {
            return Err(DfmcpError::new(
                ErrorCode::InvalidRequest,
                "reference advancement limits exceed the supported hard bounds",
            ));
        }
        if elapsed > limits.max_game_ticks {
            return Err(advance_budget_error(
                "reference advancement exceeds its game-tick budget",
            ));
        }
        Ok(Self {
            limits,
            events: 0,
            work_units: 0,
        })
    }

    fn charge(&mut self, amount: u64) -> Result<()> {
        let next = self.work_units.checked_add(amount).ok_or_else(|| {
            advance_budget_error("reference advancement work accounting overflowed")
        })?;
        if next > self.limits.max_work_units {
            return Err(advance_budget_error(
                "reference advancement exhausted its source/entity/terrain work budget",
            ));
        }
        self.work_units = next;
        Ok(())
    }

    fn event(&mut self) -> Result<()> {
        if self.events >= self.limits.max_events {
            return Err(advance_budget_error(
                "reference advancement exhausted its event budget",
            ));
        }
        self.events += 1;
        Ok(())
    }
}

/// Advance physical effects on their causal timeline. The caller has already
/// advanced snapshot.tick by elapsed and owns the cursor and hash. Eligibility
/// is evaluated at the beginning of each interval; newly completed workshops,
/// prerequisites, and newly consumed inventory affect only subsequent work.
///
/// At a shared tick, construction and excavation settle first, then production
/// units in ascending entity order, then drink/food consumption, then arrivals
/// and combat in ascending hostile order. Registered production jobs share a
/// finite pool: one eligible living unit and one completed matching workshop
/// per running order. A worker killed at that tick cannot contribute to a later
/// interval. Internal event boundaries are not foreground
/// observations and do not poll obligations or manufacture proof samples.
///
/// Successful advances have identical physical values under different wait
/// partitions. Publication cursors, revisions and observation metadata still
/// belong to each caller's actual observation cadence. Unavailable source facts
/// never become zero or completed work. As with apply_effect, use a transaction
/// shadow: any later refusal must discard the entire requested advance.
pub fn advance_effects(snapshot: &mut WorldSnapshot, elapsed: u64) -> Result<bool> {
    advance_effects_with_limits(snapshot, elapsed, EffectAdvanceLimits::default())
}

/// The explicitly budgeted form of advance_effects. The original destination
/// tick is restored on success or refusal; callers must discard the shadow's
/// other changes after a refusal.
pub fn advance_effects_with_limits(
    snapshot: &mut WorldSnapshot,
    elapsed: u64,
    limits: EffectAdvanceLimits,
) -> Result<bool> {
    if elapsed == 0 {
        return Ok(false);
    }
    let mut budget = EffectAdvanceBudget::new(limits, elapsed)?;
    let destination = snapshot.tick;
    let start = destination.0.checked_sub(elapsed).ok_or_else(|| {
        DfmcpError::new(
            ErrorCode::InvalidRequest,
            "reference advancement elapsed ticks precede the source timeline",
        )
    })?;
    // The caller advanced only the clock. Validate original authority before
    // rewinding it: a fact from inside the proposed interval must not become
    // legitimate simply because the destination clock has reached it.
    budget.charge(snapshot.graph.entities.len() as u64 + snapshot.graph.edges.len() as u64)?;
    for facts in snapshot
        .graph
        .entities
        .values()
        .map(|record| &record.fields)
        .chain(snapshot.graph.edges.values().map(|record| &record.fields))
    {
        budget.charge(facts.len() as u64)?;
        if facts.values().any(|fact| {
            fact.observed_at.0 > start && laboratory_fact_value(fact, fact.observed_at).is_some()
        }) {
            return Err(precondition(
                "reference advancement cannot promote a future-dated known source fact",
            ));
        }
    }
    snapshot.tick = GameTick(start);
    let result = advance_timeline(snapshot, destination, elapsed, &mut budget);
    snapshot.tick = destination;
    result
}

enum ConditionRecord {
    Known(Vec<WorkOrderCondition>),
    Unavailable(String),
}

struct TimelineProduction {
    ready: Vec<usize>,
    blockers: BTreeMap<usize, Option<String>>,
}

struct TimelineOrder {
    id: EntityId,
    job: String,
    product: Option<(&'static str, u64)>,
    conditions: ConditionRecord,
}

struct TimelineDig {
    area: MapCuboid,
    target: u32,
    // Coordinate tuples preserve canonical z/y/x excavation order. These sets
    // are built once, then updated for intersecting designations after a tile
    // changes; completing a large region does not rescan it for every tile.
    pending: BTreeSet<(i32, i32, i32)>,
}

struct EffectTimeline {
    orders: Vec<TimelineOrder>,
    buildings: Vec<EntityId>,
    digs: BTreeMap<EntityId, TimelineDig>,
    hostiles: Vec<EntityId>,
    ledger: Option<EntityId>,
    entity_count: u64,
    population_width: u64,
}

fn active_order(snapshot: &WorldSnapshot, id: EntityId) -> Result<bool> {
    Ok(field_text(entity(snapshot, id)?, STATUS_FIELD, snapshot.tick) == Some(STATUS_ACTIVE))
}

fn active_building(snapshot: &WorldSnapshot, id: EntityId) -> Result<bool> {
    Ok(matches!(
        field_text(
            entity(snapshot, id)?,
            CONSTRUCTION_STAGE_FIELD,
            snapshot.tick
        ),
        Some(STAGE_PLANNED | STAGE_UNDER_CONSTRUCTION)
    ))
}

fn designation_area(record: &EntityRecord, tick: GameTick) -> Result<MapCuboid> {
    let (Some(min), Some(max)) = (
        coord_field(record, "area_min", tick),
        coord_field(record, "area_max", tick),
    ) else {
        return Err(precondition(
            "dig designation requires eligible area coordinates before reference progress",
        ));
    };
    MapCuboid::new(min, max)
}

impl EffectTimeline {
    fn new(
        snapshot: &WorldSnapshot,
        elapsed: u64,
        budget: &mut EffectAdvanceBudget,
    ) -> Result<Self> {
        let entity_count = snapshot.graph.entities.len() as u64;
        budget.charge(entity_count.saturating_mul(3))?;
        require_progress_selectors(snapshot)?;
        // Count complete requested footprints before allocating even the first
        // cache. This also refuses oversized overlapping regions before terrain
        // reads, so a malicious source cannot allocate up to the CPU-work cap.
        budget.charge(entity_count)?;
        let mut cached_tiles = 0u64;
        for record in snapshot.graph.entities.values() {
            if matches!(&record.kind, EntityKind::Other(kind) if kind == DIG_DESIGNATION_KIND)
                && active_order(snapshot, record.id)?
            {
                let tiles = validate_region(designation_area(record, snapshot.tick)?)?;
                cached_tiles = cached_tiles.checked_add(tiles).ok_or_else(|| {
                    advance_budget_error("aggregate cached excavation footprint overflowed")
                })?;
                if cached_tiles > MAX_EFFECT_ADVANCE_CACHED_DIG_TILES {
                    return Err(advance_budget_error(
                        "aggregate cached excavation footprint exceeds its explicit tile bound",
                    ));
                }
            }
        }
        let mut timeline = Self {
            orders: Vec::new(),
            buildings: Vec::new(),
            digs: BTreeMap::new(),
            hostiles: Vec::new(),
            ledger: stock_ledger(snapshot),
            entity_count,
            population_width: 0,
        };
        for record in snapshot.graph.entities.values() {
            if record.kind == EntityKind::Unit {
                timeline.population_width = timeline
                    .population_width
                    .checked_add(record.fields.len() as u64 + 1)
                    .ok_or_else(|| advance_budget_error("population scan accounting overflowed"))?;
            }
            match &record.kind {
                EntityKind::WorkOrder if active_order(snapshot, record.id)? => {
                    let job = field_text(record, "job_token", snapshot.tick).ok_or_else(|| {
                        precondition("work order requires an eligible known job token before reference progress")
                    })?;
                    validate_condition_text(job)?;
                    let work = field_u64(record, "work_ticks", snapshot.tick)?;
                    let remaining = field_u64(record, AMOUNT_REMAINING_FIELD, snapshot.tick)?;
                    if work >= WORK_ORDER_TICKS_PER_UNIT || remaining == 0 {
                        return Err(precondition(
                            "active work order has noncanonical progress counters",
                        ));
                    }
                    let conditions = match field_value(record, WORK_ORDER_CONDITIONS_FIELD, snapshot.tick) {
                        Some(stored) => match decode_conditions(stored) {
                            Ok(conditions) => ConditionRecord::Known(conditions),
                            Err(error) => ConditionRecord::Unavailable(error.message),
                        },
                        None => ConditionRecord::Unavailable(
                            "work-order conditions are not established; legacy orders require an explicit condition record".to_owned(),
                        ),
                    };
                    timeline.orders.push(TimelineOrder {
                        id: record.id,
                        job: job.to_owned(),
                        product: work_order_product(job),
                        conditions,
                    });
                }
                EntityKind::Building if active_building(snapshot, record.id)? => {
                    let required = field_u64(record, "required_ticks", snapshot.tick)?;
                    let progress = field_u64(record, "progress_ticks", snapshot.tick)?;
                    if required == 0 || progress >= required {
                        return Err(precondition(
                            "active building has noncanonical progress counters",
                        ));
                    }
                    timeline.buildings.push(record.id);
                }
                EntityKind::Other(kind)
                    if kind == DIG_DESIGNATION_KIND && active_order(snapshot, record.id)? =>
                {
                    let area = designation_area(record, snapshot.tick)?;
                    let tiles = validate_region(area)?;
                    budget.charge(tiles)?;
                    let target =
                        u32::try_from(field_u64(record, "target_tile_code", snapshot.tick)?)
                            .map_err(|_| {
                                precondition("dig designation target tile code is invalid")
                            })?;
                    if field_u64(record, "work_ticks", snapshot.tick)? >= DIG_TICKS_PER_TILE {
                        return Err(precondition(
                            "active dig designation has noncanonical progress counters",
                        ));
                    }
                    let mut pending = BTreeSet::new();
                    for coord in region_tiles(area) {
                        let code = snapshot.tile_code_at(coord).ok_or_else(|| {
                            precondition("active dig designation includes unobserved terrain")
                        })?;
                        if code != target {
                            pending.insert((coord.z, coord.y, coord.x));
                        }
                    }
                    timeline.digs.insert(
                        record.id,
                        TimelineDig {
                            area,
                            target,
                            pending,
                        },
                    );
                }
                EntityKind::Creature
                    if field_value(record, HOSTILE_FIELD, snapshot.tick)
                        == Some(&Value::Bool(true))
                        && matches!(
                            field_text(record, THREAT_STATUS_FIELD, snapshot.tick),
                            Some(THREAT_APPROACHING | THREAT_ATTACKING)
                        ) =>
                {
                    field_u64(record, ARRIVES_AT_FIELD, snapshot.tick)?;
                    timeline.hostiles.push(record.id);
                }
                _ => {}
            }
        }
        if let Some(ledger) = timeline.ledger {
            field_u64(
                entity(snapshot, ledger)?,
                METABOLISM_TICKS_FIELD,
                snapshot.tick,
            )?
            .checked_add(elapsed)
            .ok_or_else(|| {
                advance_budget_error("metabolism clock would overflow during reference advancement")
            })?;
        }
        Ok(timeline)
    }

    fn gate(
        &self,
        snapshot: &WorldSnapshot,
        order: &TimelineOrder,
        budget: &mut EffectAdvanceBudget,
    ) -> Result<ProductionGate> {
        let conditions = match &order.conditions {
            ConditionRecord::Known(conditions) => conditions,
            ConditionRecord::Unavailable(reason) => {
                return Ok(ProductionGate::Blocked(reason.clone()));
            }
        };
        budget.charge(
            self.entity_count
                .saturating_mul(conditions.len() as u64 + 3)
                .saturating_add(conditions.len() as u64 + 1),
        )?;
        if let Some(blocker) = work_order_blocker(snapshot, &order.job) {
            return Ok(ProductionGate::Blocked(blocker));
        }
        Ok(production_gate(
            snapshot,
            order.id,
            conditions,
            order.product,
        ))
    }

    fn production_schedule(
        &self,
        snapshot: &WorldSnapshot,
        budget: &mut EffectAdvanceBudget,
    ) -> Result<TimelineProduction> {
        budget.charge(self.orders.len() as u64 + 1)?;
        if snapshot.graph.entities.len() > MAX_EFFECT_ADVANCE_PRODUCTION_ENTITIES
            && self.orders.iter().any(|order| work_order_requirements(&order.job).is_some())
        {
            return Err(advance_budget_error(
                "production capacity exceeds its explicit canonical entity bound",
            ));
        }
        let mut ready = Vec::new();
        let mut blockers = BTreeMap::new();
        for (index, order) in self.orders.iter().enumerate() {
            if active_order(snapshot, order.id)? {
                let blocker = match self.gate(snapshot, order, budget)? {
                    ProductionGate::Ready { .. } => {
                        ready.push(index);
                        None
                    }
                    ProductionGate::Blocked(reason) => Some(reason),
                };
                blockers.insert(index, blocker);
            }
        }
        let selected = capacity::select(snapshot, &self.orders, ready, budget)?;
        for (index, reason) in selected.waiting {
            blockers.insert(index, Some(reason));
        }
        Ok(TimelineProduction {
            ready: selected.ready,
            blockers,
        })
    }

    fn next_interval(
        &self,
        snapshot: &WorldSnapshot,
        remaining: u64,
        budget: &mut EffectAdvanceBudget,
    ) -> Result<(u64, Vec<usize>)> {
        budget.charge(
            (self.orders.len() + self.buildings.len() + self.digs.len() + self.hostiles.len())
                as u64
                + 1,
        )?;
        let mut delta = remaining;
        let ready = self.production_schedule(snapshot, budget)?.ready;
        for index in &ready {
            let work = field_u64(
                entity(snapshot, self.orders[*index].id)?,
                "work_ticks",
                snapshot.tick,
            )?;
            delta = delta.min(WORK_ORDER_TICKS_PER_UNIT - work);
        }
        for id in &self.buildings {
            if active_building(snapshot, *id)? {
                let building = entity(snapshot, *id)?;
                delta = delta.min(
                    field_u64(building, "required_ticks", snapshot.tick)?
                        - field_u64(building, "progress_ticks", snapshot.tick)?,
                );
            }
        }
        for (id, dig) in &self.digs {
            let work = field_u64(entity(snapshot, *id)?, "work_ticks", snapshot.tick)?;
            delta = delta.min(if dig.pending.is_empty() {
                1
            } else {
                DIG_TICKS_PER_TILE - work
            });
        }
        if let Some(ledger) = self.ledger {
            let metabolism = field_u64(
                entity(snapshot, ledger)?,
                METABOLISM_TICKS_FIELD,
                snapshot.tick,
            )?;
            delta = delta.min(DRINK_INTERVAL_TICKS - metabolism % DRINK_INTERVAL_TICKS);
            delta = delta.min(FOOD_INTERVAL_TICKS - metabolism % FOOD_INTERVAL_TICKS);
        }
        for id in &self.hostiles {
            let hostile = entity(snapshot, *id)?;
            if field_text(hostile, THREAT_STATUS_FIELD, snapshot.tick) == Some(THREAT_SLAIN) {
                continue;
            }
            let arrival = field_u64(hostile, ARRIVES_AT_FIELD, snapshot.tick)?;
            delta = delta.min(if snapshot.tick.0 < arrival {
                arrival - snapshot.tick.0
            } else {
                COMBAT_ROUND_TICKS - (snapshot.tick.0 - arrival) % COMBAT_ROUND_TICKS
            });
        }
        Ok((delta, ready))
    }

    fn advance_digs(
        &mut self,
        snapshot: &mut WorldSnapshot,
        elapsed: u64,
        budget: &mut EffectAdvanceBudget,
    ) -> Result<bool> {
        budget.charge(self.digs.len() as u64 * 2)?;
        let mut earned = Vec::with_capacity(self.digs.len());
        let mut changed = false;
        let ids: Vec<EntityId> = self.digs.keys().copied().collect();
        for id in ids {
            let work = field_u64(entity(snapshot, id)?, "work_ticks", snapshot.tick)? + elapsed;
            earned.push((id, work));
            if work < DIG_TICKS_PER_TILE {
                continue;
            }
            let Some((coord, target)) = self.digs.get(&id).and_then(|dig| {
                dig.pending
                    .first()
                    .map(|(z, y, x)| (MapCoord::new(*x, *y, *z), dig.target))
            }) else {
                continue;
            };
            // Tile replacement decodes a bounded 256-tile chunk. Every active
            // overlapping designation sees the resulting terrain at this tick.
            budget.charge(self.digs.len() as u64 + 256)?;
            changed |= snapshot.set_tile_code(coord, target)?;
            for dig in self.digs.values_mut() {
                if dig.area.contains(coord) {
                    let key = (coord.z, coord.y, coord.x);
                    if dig.target == target {
                        dig.pending.remove(&key);
                    } else {
                        dig.pending.insert(key);
                    }
                }
            }
        }
        for (id, work) in earned {
            let remaining = self
                .digs
                .get(&id)
                .ok_or_else(|| {
                    DfmcpError::new(
                        ErrorCode::InternalInvariantViolation,
                        "active designation disappeared from timeline",
                    )
                })?
                .pending
                .len() as u64;
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
        }
        self.digs.retain(|id, _| {
            snapshot.graph.entities.get(id).is_some_and(|record| {
                field_text(record, STATUS_FIELD, snapshot.tick) == Some(STATUS_ACTIVE)
            })
        });
        Ok(changed)
    }

    fn charge_population(&self, budget: &mut EffectAdvanceBudget, combat: bool) -> Result<()> {
        let visits = self
            .population_width
            .saturating_add(self.entity_count.saturating_mul(5));
        budget.charge(if combat {
            visits.saturating_mul(self.hostiles.len() as u64 + 1)
        } else {
            visits
        })
    }

    fn refresh_blockers(
        &self,
        snapshot: &mut WorldSnapshot,
        budget: &mut EffectAdvanceBudget,
    ) -> Result<bool> {
        let mut changed = false;
        let schedule = self.production_schedule(snapshot, budget)?;
        for (index, blocker) in schedule.blockers {
            changed |= write_production_blocker(snapshot, self.orders[index].id, blocker)?;
        }
        Ok(changed)
    }
}

fn advance_timeline(
    snapshot: &mut WorldSnapshot,
    destination: GameTick,
    elapsed: u64,
    budget: &mut EffectAdvanceBudget,
) -> Result<bool> {
    let mut timeline = EffectTimeline::new(snapshot, elapsed, budget)?;
    let mut changed = false;
    if !timeline.hostiles.is_empty() {
        timeline.charge_population(budget, true)?;
        // A threat already at its arrival boundary is attacking at the source
        // tick. This does not award a combat round or consume elapsed work.
        changed |= advance_threats(snapshot, 0)?;
    }
    while snapshot.tick < destination {
        budget.event()?;
        let (delta, ready) =
            timeline.next_interval(snapshot, destination.0 - snapshot.tick.0, budget)?;
        if delta == 0 {
            return Err(DfmcpError::new(
                ErrorCode::InternalInvariantViolation,
                "reference event timeline did not advance",
            ));
        }
        snapshot.tick = GameTick(snapshot.tick.0 + delta);
        for id in &timeline.buildings {
            if active_building(snapshot, *id)? {
                changed |= advance_building(snapshot, *id, delta)?;
            }
        }
        changed |= timeline.advance_digs(snapshot, delta, budget)?;
        for index in ready {
            changed |= advance_ready_work_order(
                snapshot,
                &timeline,
                &timeline.orders[index],
                delta,
                budget,
            )?;
        }
        if timeline.ledger.is_some() {
            timeline.charge_population(budget, false)?;
            changed |= advance_metabolism(snapshot, delta)?;
        }
        if !timeline.hostiles.is_empty() {
            timeline.charge_population(budget, true)?;
            changed |= advance_threats(snapshot, delta)?;
        }
        changed |= timeline.refresh_blockers(snapshot, budget)?;
    }
    Ok(changed)
}

fn is_alive(entity: &EntityRecord, tick: GameTick) -> bool {
    // Older reference fixtures omit this optional field for living units.
    // An explicit unavailable value must not inherit that legacy default.
    entity
        .fields
        .get("alive")
        .is_none_or(|fact| laboratory_fact_value(fact, tick) == Some(&Value::Bool(true)))
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
            && !matches!(
                laboratory_fact_value(fact, snapshot.tick),
                Some(Value::Bool(_))
            )
        {
            return Err(precondition(format!(
                "unit {} has unavailable life state; population progress requires a known census",
                unit.id.get()
            )));
        }
        if !combat || !is_alive(unit, snapshot.tick) {
            continue;
        }
        if let Some(fact) = unit.fields.get(SQUAD_FIELD)
            && !matches!(
                laboratory_fact_value(fact, snapshot.tick),
                Some(Value::Entity(_) | Value::Null)
            )
        {
            return Err(precondition(format!(
                "unit {} has unavailable squad membership before combat progress",
                unit.id.get()
            )));
        }
        if unit.fields.iter().any(|(name, fact)| {
            name.starts_with(BURROW_FIELD_PREFIX)
                && !matches!(
                    laboratory_fact_value(fact, snapshot.tick),
                    Some(Value::Bool(_))
                )
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
                && field_value(entity, HOSTILE_FIELD, snapshot.tick) == Some(&Value::Bool(true))
                && matches!(
                    field_text(entity, THREAT_STATUS_FIELD, snapshot.tick),
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
        let arrives = field_u64(entity(snapshot, hostile)?, ARRIVES_AT_FIELD, snapshot.tick)?;
        if now < arrives {
            continue;
        }
        if field_text(
            entity(snapshot, hostile)?,
            THREAT_STATUS_FIELD,
            snapshot.tick,
        ) != Some(THREAT_ATTACKING)
        {
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
                        && is_alive(unit, snapshot.tick)
                        && matches!(
                            field_value(unit, SQUAD_FIELD, snapshot.tick),
                            Some(Value::Entity(_))
                        )
                })
                .count() as u64;
            let creature = entity(snapshot, hostile)?;
            let fought = field_u64(creature, COMBAT_ROUNDS_FIELD, snapshot.tick)?
                .checked_add(1)
                .ok_or_else(|| advance_budget_error("combat round counter would overflow"))?;
            let health = field_u64(creature, HEALTH_FIELD, snapshot.tick)?
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
                            && is_alive(unit, snapshot.tick)
                            && unit.fields.iter().all(|(name, fact)| {
                                !name.starts_with(BURROW_FIELD_PREFIX)
                                    || laboratory_fact_value(fact, snapshot.tick)
                                        == Some(&Value::Bool(false))
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
/// settled at the same tick is available for that meal.
/// When a stock runs short the dwarves served last (highest id) go without
/// and their need turns `thirsty` / `hungry`; a later full meal satisfies
/// everyone again. A world without a stock ledger has no metabolism.
fn advance_metabolism(snapshot: &mut WorldSnapshot, elapsed: u64) -> Result<bool> {
    let Some(ledger) = stock_ledger(snapshot) else {
        return Ok(false);
    };
    require_population_fields(snapshot, false)?;
    let before = field_u64(
        entity(snapshot, ledger)?,
        METABOLISM_TICKS_FIELD,
        snapshot.tick,
    )?;
    let after = before
        .checked_add(elapsed)
        .ok_or_else(|| advance_budget_error("metabolism clock would overflow"))?;
    let living: Vec<EntityId> = snapshot
        .graph
        .entities
        .values()
        .filter(|entity| entity.kind == EntityKind::Unit && is_alive(entity, snapshot.tick))
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
        let mut held = field_u64(entity(snapshot, ledger)?, stock_field, snapshot.tick)?;
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

enum ProductionGate {
    Ready { unit_limit: u64 },
    Blocked(String),
}

fn completed_order_blocker(
    snapshot: &WorldSnapshot,
    dependent: EntityId,
    name: &str,
) -> Option<String> {
    let mut matching = None;
    for order in snapshot
        .graph
        .entities
        .values()
        .filter(|entity| entity.kind.as_str() == EntityKind::WorkOrder.as_str())
    {
        let Some(observed_name) = field_text(order, WORK_ORDER_NAME_FIELD, snapshot.tick) else {
            return Some(format!(
                "completed order {name:?} cannot be resolved uniquely: a work-order name is not established"
            ));
        };
        if observed_name != name {
            continue;
        }
        if matching.is_some() {
            return Some(format!("completed order {name:?} is ambiguous"));
        }
        matching = Some(order);
    }
    let Some(order) = matching else {
        return Some(format!("completed order {name:?} is not established"));
    };
    if order.id == dependent {
        return Some(format!(
            "completed order {name:?} refers to this work order itself"
        ));
    }
    if field_text(order, STATUS_FIELD, snapshot.tick) != Some(STATUS_COMPLETE)
        || field_value(order, AMOUNT_REMAINING_FIELD, snapshot.tick) != Some(&Value::U64(0))
    {
        return Some(format!(
            "order {name:?} completion with zero remaining work is not established"
        ));
    }
    None
}

fn production_gate(
    snapshot: &WorldSnapshot,
    id: EntityId,
    conditions: &[WorkOrderCondition],
    product: Option<(&str, u64)>,
) -> ProductionGate {
    if conditions.is_empty() {
        return ProductionGate::Ready {
            unit_limit: u64::MAX,
        };
    }
    // Named dependency resolution needs a complete bounded domain, never a
    // truncated first match. The empty conjunction performs no domain scan.
    if snapshot.graph.entities.len() > 65_536 {
        return ProductionGate::Blocked(
            "work-order condition domain exceeds the 65536-entity reference bound".to_owned(),
        );
    }
    let ledger = stock_ledger(snapshot).and_then(|id| snapshot.graph.entities.get(&id));
    let mut unit_limit = u64::MAX;
    for condition in conditions {
        match condition {
            WorkOrderCondition::ItemCountBelow {
                item_token,
                threshold,
            } => {
                let field = item_stock_field(item_token);
                let Some(Value::U64(held)) =
                    ledger.and_then(|e| field_value(e, &field, snapshot.tick))
                else {
                    return ProductionGate::Blocked(format!(
                        "item {item_token:?} count is not established in {field}"
                    ));
                };
                let threshold = u64::from(*threshold);
                if *held >= threshold {
                    return ProductionGate::Blocked(format!(
                        "item {item_token:?} count {held} is not below {threshold}"
                    ));
                }
                if let Some((produced_field, per_unit)) = product
                    && produced_field == field
                {
                    // Check before each indivisible production unit. At most
                    // one unit may cross the threshold; no per-unit loop or
                    // large elapsed interval can bypass the next check.
                    unit_limit = unit_limit.min((threshold - held).div_ceil(per_unit));
                }
            }
            WorkOrderCondition::MaterialAvailable {
                material_token,
                minimum,
            } => {
                let field = format!("{STOCK_MATERIAL_FIELD_PREFIX}{material_token}");
                let Some(Value::U64(held)) =
                    ledger.and_then(|e| field_value(e, &field, snapshot.tick))
                else {
                    return ProductionGate::Blocked(format!(
                        "material {material_token:?} availability is not established in {field}"
                    ));
                };
                if *held < u64::from(*minimum) {
                    return ProductionGate::Blocked(format!(
                        "material {material_token:?} count {held} is below required {minimum}"
                    ));
                }
                // A condition is an availability gate, not an unstated recipe
                // or reservation. No material is consumed by this predicate.
            }
            WorkOrderCondition::CompletedOrder { order_name } => {
                if let Some(blocker) = completed_order_blocker(snapshot, id, order_name) {
                    return ProductionGate::Blocked(blocker);
                }
            }
        }
    }
    ProductionGate::Ready { unit_limit }
}

fn write_production_blocker(
    snapshot: &mut WorldSnapshot,
    id: EntityId,
    blocker: Option<String>,
) -> Result<bool> {
    write_fields(
        snapshot,
        id,
        vec![(
            BLOCKED_BY_FIELD.to_owned(),
            blocker.map_or(Value::Null, Value::Text),
        )],
    )
}

fn advance_ready_work_order(
    snapshot: &mut WorldSnapshot,
    timeline: &EffectTimeline,
    order: &TimelineOrder,
    elapsed: u64,
    budget: &mut EffectAdvanceBudget,
) -> Result<bool> {
    let record = entity(snapshot, order.id)?;
    let work = field_u64(record, "work_ticks", snapshot.tick)? + elapsed;
    if work < WORK_ORDER_TICKS_PER_UNIT {
        // Eligibility was frozen at the source boundary. Another order's
        // completion cannot revoke work already earned through this interval.
        return write_fields(
            snapshot,
            order.id,
            vec![("work_ticks".to_owned(), Value::U64(work))],
        );
    }
    if work != WORK_ORDER_TICKS_PER_UNIT {
        return Err(DfmcpError::new(
            ErrorCode::InternalInvariantViolation,
            "production interval crossed a unit boundary",
        ));
    }
    // A completion is admitted against stock after earlier simultaneous
    // completions. A losing order keeps all 49 pre-completion ticks; only the
    // rejected final tick is discarded. Retaining the interval's starting
    // counter instead would make arbitrary caller cuts change physical work.
    if !matches!(
        timeline.gate(snapshot, order, budget)?,
        ProductionGate::Ready { unit_limit: 1.. }
    ) {
        return write_fields(
            snapshot,
            order.id,
            vec![(
                "work_ticks".to_owned(),
                Value::U64(WORK_ORDER_TICKS_PER_UNIT - 1),
            )],
        );
    }
    let remaining = field_u64(
        entity(snapshot, order.id)?,
        AMOUNT_REMAINING_FIELD,
        snapshot.tick,
    )?
    .checked_sub(1)
    .ok_or_else(|| {
        DfmcpError::new(
            ErrorCode::InternalInvariantViolation,
            "production completed an order with no remaining units",
        )
    })?;
    let mut changed = false;
    if let Some((stock_field, per_unit)) = order.product
        && let Some(ledger) = timeline.ledger
    {
        let held = field_u64(entity(snapshot, ledger)?, stock_field, snapshot.tick)?;
        let produced = held
            .checked_add(per_unit)
            .ok_or_else(|| advance_budget_error("production would overflow its stock counter"))?;
        changed |= write_fields(
            snapshot,
            ledger,
            vec![(stock_field.to_owned(), Value::U64(produced))],
        )?;
    }
    let mut fields = vec![
        (AMOUNT_REMAINING_FIELD.to_owned(), Value::U64(remaining)),
        ("work_ticks".to_owned(), Value::U64(0)),
    ];
    if remaining == 0 {
        fields.push((
            STATUS_FIELD.to_owned(),
            Value::Text(STATUS_COMPLETE.to_owned()),
        ));
        fields.push((BLOCKED_BY_FIELD.to_owned(), Value::Null));
    }
    changed |= write_fields(snapshot, order.id, fields)?;
    Ok(changed)
}

fn advance_building(snapshot: &mut WorldSnapshot, id: EntityId, elapsed: u64) -> Result<bool> {
    let building = entity(snapshot, id)?;
    let required = field_u64(building, "required_ticks", snapshot.tick)?;
    if required == 0 {
        return Err(precondition(
            "building duration must be positive before reference progress",
        ));
    }
    let progress = field_u64(building, "progress_ticks", snapshot.tick)?
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

#[cfg(test)]
#[path = "effects_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "effects_timeline_tests.rs"]
mod timeline_tests;
