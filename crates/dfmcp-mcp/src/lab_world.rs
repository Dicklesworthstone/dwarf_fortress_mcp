//! Laboratory world scenarios, semantic action requests, and bounded
//! entity/terrain projections for the process-local MCP laboratory.
//!
//! Everything here is deterministic and process-local. It lets an agent
//! exercise the full plan → commit → wait → prove loop for excavation,
//! construction, labor, logistics, military and production actions against
//! the reference action semantics in `dfmcp_intent::effects`. Nothing here is
//! evidence about Dwarf Fortress or DFHack behaviour.

use std::collections::{BTreeMap, BTreeSet};

use dfmcp_core::{
    DfmcpError, EntityId, ErrorCode, FortressId, GameTick, MapCoord, MapCuboid, ObservationCursor,
    Result,
};
use dfmcp_intent::effects;
use dfmcp_intent::{
    Action, BuildingKind, DigMode, MaterialSelector, PreparedPlan, RequestedAction,
};
use dfmcp_world::terrain::{region_tiles, uniform_chunk, validate_region};
use dfmcp_world::{
    ChunkCoord, CompareOp, EntityKind, EntityRecord, Fact, FactSource, Predicate, Value,
    WorldGraph, WorldSnapshot, tile_codes,
};
use serde::Deserialize;
use serde_json::{Value as Json, json};

/// Largest semantic action request accepted by `fortress.plan`.
pub(crate) const MAX_ACTIONS_JSON_BYTES: usize = 16 * 1024;
/// Most steps one laboratory plan may request.
pub(crate) const MAX_LAB_STEPS: usize = 16;
/// Largest query specification accepted by `fortress.query`.
pub(crate) const MAX_QUERY_JSON_BYTES: usize = 1024;
const MAX_ENTITY_PAGE: usize = 100;
const MAX_TERRAIN_QUERY_TILES: u64 = 4_096;
const MAX_NAME_BYTES: usize = 128;
const MAX_LIST_ITEMS: usize = 64;

/// Built-in laboratory scenarios.
pub(crate) const SCENARIOS: [&str; 2] = ["empty", "starter_fortress"];

/// Starter-fortress entity identities.
pub(crate) mod starter {
    use dfmcp_core::EntityId;
    pub const FIRST_DWARF: u64 = 1_001;
    pub const FOOD_STOCKPILE: EntityId = EntityId::new(2_001);
    pub const INNER_BURROW: EntityId = EntityId::new(3_001);
    pub const MILITIA_SQUAD: EntityId = EntityId::new(4_001);
    pub const STOCK_LEDGER: EntityId = EntityId::new(5_001);
    /// Opening drink and food: a few days for seven dwarves.
    pub const OPENING_DRINK: u64 = 40;
    pub const OPENING_FOOD: u64 = 60;
    /// Excavation level of the starter fortress.
    pub const LEVEL_Z: i32 = 10;
}

fn invalid(message: impl Into<String>) -> DfmcpError {
    DfmcpError::new(ErrorCode::InvalidRequest, message)
}

fn lab_fact(value: Value) -> Fact {
    Fact::known(
        value,
        GameTick(1),
        FactSource::Derived("dfmcp.lab-scenario/1".to_owned()),
        dfmcp_core::Digest32::ZERO,
    )
}

fn record(id: EntityId, kind: EntityKind, label: &str, fields: Vec<(&str, Value)>) -> EntityRecord {
    EntityRecord {
        id,
        generation: 1,
        revision: 1,
        kind,
        label: label.to_owned(),
        fields: fields
            .into_iter()
            .map(|(name, value)| (name.to_owned(), lab_fact(value)))
            .collect(),
    }
}

/// Build the opening snapshot for a named scenario.
pub(crate) fn scenario_snapshot(
    scenario: &str,
    fortress_id: FortressId,
    paused: bool,
) -> Result<WorldSnapshot> {
    let graph = match scenario {
        "empty" => WorldGraph::default(),
        "starter_fortress" => starter_graph()?,
        other => {
            return Err(invalid(format!(
                "unknown laboratory scenario {other:?}; expected one of {SCENARIOS:?}"
            )));
        }
    };
    Ok(WorldSnapshot::new(
        fortress_id,
        GameTick(1),
        ObservationCursor::ORIGIN,
        paused,
        graph,
    ))
}

/// Three 48x48 rock levels (z 9..11) with a carved 10x3 entrance hall on
/// z=10, seven dwarves, a stockpile, a burrow and a squad.
fn starter_graph() -> Result<WorldGraph> {
    let mut graph = WorldGraph::default();
    // Solid rock one level above and below too, so blueprint hazard checks
    // can see the complete one-tile halo around work on the main level.
    for z in starter::LEVEL_Z - 1..=starter::LEVEL_Z + 1 {
        for x in -1..=1 {
            for y in -1..=1 {
                let coord = ChunkCoord { x, y, z };
                graph
                    .chunks
                    .insert(coord, uniform_chunk(coord, tile_codes::SOLID_WALL));
            }
        }
    }
    let mut snapshot = WorldSnapshot::new(
        FortressId::new(1),
        GameTick(1),
        ObservationCursor::ORIGIN,
        true,
        graph,
    );
    let hall = MapCuboid::new(
        MapCoord::new(0, 0, starter::LEVEL_Z),
        MapCoord::new(9, 2, starter::LEVEL_Z),
    )?;
    for coord in region_tiles(hall) {
        snapshot.set_tile_code(coord, tile_codes::FLOOR)?;
    }
    let mut graph = snapshot.graph;
    let dwarves = [
        ("Urist McMiner", "miner"),
        ("Kogan Hammerstone", "mason"),
        ("Ast Brewmaster", "brewer"),
        ("Domas Woodcutter", "carpenter"),
        ("Litast Shieldarm", "axedwarf"),
        ("Rigoth Seedsower", "farmer"),
        ("Momuz Quarrier", "miner"),
    ];
    for (index, (name, profession)) in (0u64..).zip(dwarves) {
        let id = EntityId::new(starter::FIRST_DWARF + index);
        let x = i32::try_from(index).map_err(|_| invalid("scenario index overflow"))?;
        graph.entities.insert(
            id,
            record(
                id,
                EntityKind::Unit,
                name,
                vec![
                    ("profession", Value::Text(profession.to_owned())),
                    ("alive", Value::Bool(true)),
                    (
                        "position",
                        Value::Coord(MapCoord::new(x, 1, starter::LEVEL_Z)),
                    ),
                ],
            ),
        );
    }
    for (id, kind, label) in [
        (
            starter::FOOD_STOCKPILE,
            EntityKind::Stockpile,
            "Food stockpile",
        ),
        (starter::INNER_BURROW, EntityKind::Burrow, "Inner fortress"),
        (
            starter::MILITIA_SQUAD,
            EntityKind::Squad,
            "The Axes of Dawn",
        ),
    ] {
        graph
            .entities
            .insert(id, record(id, kind, label, Vec::new()));
    }
    graph.entities.insert(
        starter::STOCK_LEDGER,
        record(
            starter::STOCK_LEDGER,
            EntityKind::Other(effects::STOCK_LEDGER_KIND.to_owned()),
            "Fortress stocks",
            vec![
                (
                    effects::STOCK_DRINK_FIELD,
                    Value::U64(starter::OPENING_DRINK),
                ),
                (effects::STOCK_FOOD_FIELD, Value::U64(starter::OPENING_FOOD)),
                (effects::METABOLISM_TICKS_FIELD, Value::U64(0)),
            ],
        ),
    );
    Ok(graph)
}

// ---------------------------------------------------------------------------
// Semantic action requests
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StepSpec {
    action: ActionSpec,
    #[serde(default)]
    depends_on: Vec<u32>,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum ActionSpec {
    Pause {
        paused: bool,
    },
    DesignateDig {
        min: [i32; 3],
        max: [i32; 3],
        mode: String,
    },
    Build {
        building: String,
        location: [i32; 3],
        min: [i32; 3],
        max: [i32; 3],
        #[serde(default)]
        materials: Vec<String>,
    },
    SetLabor {
        units: Vec<String>,
        labor: String,
        enabled: bool,
    },
    CreateWorkOrder {
        name: String,
        job_token: String,
        amount: u32,
    },
    ConfigureStockpile {
        stockpile: String,
        accepts: Vec<String>,
        max_bins: Option<u32>,
        max_barrels: Option<u32>,
        max_wheelbarrows: Option<u32>,
    },
    AssignSquad {
        units: Vec<String>,
        squad: String,
    },
    SetBurrowMembership {
        units: Vec<String>,
        burrow: String,
        assigned: bool,
    },
    SetStandingOrder {
        key: String,
        value: String,
    },
}

fn name(value: String, what: &str) -> Result<String> {
    if value.is_empty() || value.len() > MAX_NAME_BYTES || value.chars().any(char::is_control) {
        return Err(invalid(format!(
            "{what} must contain 1..={MAX_NAME_BYTES} non-control bytes"
        )));
    }
    Ok(value)
}

fn entity(value: &str) -> Result<EntityId> {
    if value.is_empty()
        || value.len() > 20
        || value.starts_with('+')
        || (value.len() > 1 && value.starts_with('0'))
    {
        return Err(invalid("entity IDs are canonical decimal u64 strings"));
    }
    value
        .parse::<u64>()
        .map(EntityId::new)
        .map_err(|_| invalid("entity IDs are canonical decimal u64 strings"))
}

fn entities(values: Vec<String>) -> Result<Vec<EntityId>> {
    if values.is_empty() || values.len() > MAX_LIST_ITEMS {
        return Err(invalid(format!(
            "unit lists must name 1..={MAX_LIST_ITEMS} entities"
        )));
    }
    values.iter().map(|value| entity(value)).collect()
}

fn coord(value: [i32; 3]) -> MapCoord {
    MapCoord::new(value[0], value[1], value[2])
}

fn cuboid(min: [i32; 3], max: [i32; 3]) -> Result<MapCuboid> {
    let area = MapCuboid::new(coord(min), coord(max))?;
    validate_region(area)?;
    Ok(area)
}

fn dig_mode(value: &str) -> Result<DigMode> {
    Ok(match value {
        "mine" => DigMode::Mine,
        "channel" => DigMode::Channel,
        "up_stair" => DigMode::UpStair,
        "down_stair" => DigMode::DownStair,
        "up_down_stair" => DigMode::UpDownStair,
        "ramp" => DigMode::Ramp,
        "remove_construction" => DigMode::RemoveConstruction,
        other => return Err(invalid(format!("unknown dig mode {other:?}"))),
    })
}

fn building_kind(value: &str) -> Result<BuildingKind> {
    let (family, detail) = value.split_once(':').unwrap_or((value, ""));
    let subtype = || name(detail.to_owned(), "building subtype");
    Ok(match family {
        "workshop" => BuildingKind::Workshop(subtype()?),
        "furnace" => BuildingKind::Furnace(subtype()?),
        "furniture" => BuildingKind::Furniture(subtype()?),
        "construction" => BuildingKind::Construction(subtype()?),
        "trap" => BuildingKind::Trap(subtype()?),
        "custom" => BuildingKind::Custom(subtype()?),
        "farm_plot" if detail.is_empty() => BuildingKind::FarmPlot,
        "bridge" if detail.is_empty() => BuildingKind::Bridge,
        "well" if detail.is_empty() => BuildingKind::Well,
        _ => {
            return Err(invalid(format!(
                "unknown building {value:?}; use e.g. workshop:Still, furniture:Bed, farm_plot"
            )));
        }
    })
}

fn action(spec: ActionSpec) -> Result<Action> {
    Ok(match spec {
        ActionSpec::Pause { paused } => Action::Pause { paused },
        ActionSpec::DesignateDig { min, max, mode } => Action::DesignateDig {
            area: cuboid(min, max)?,
            mode: dig_mode(&mode)?,
        },
        ActionSpec::Build {
            building,
            location,
            min,
            max,
            materials,
        } => {
            if materials.len() > MAX_LIST_ITEMS {
                return Err(invalid("too many material tokens"));
            }
            Action::Build {
                kind: building_kind(&building)?,
                location: coord(location),
                footprint: cuboid(min, max)?,
                material: MaterialSelector {
                    required_tokens: materials
                        .into_iter()
                        .map(|token| name(token, "material token"))
                        .collect::<Result<BTreeSet<_>>>()?,
                    ..MaterialSelector::default()
                },
            }
        }
        ActionSpec::SetLabor {
            units,
            labor,
            enabled,
        } => Action::SetLabor {
            units: entities(units)?,
            labor: name(labor, "labor")?,
            enabled,
        },
        ActionSpec::CreateWorkOrder {
            name: order,
            job_token,
            amount,
        } => Action::CreateWorkOrder {
            name: name(order, "work order name")?,
            job_token: name(job_token, "job token")?,
            amount,
            conditions: Vec::new(),
        },
        ActionSpec::ConfigureStockpile {
            stockpile,
            accepts,
            max_bins,
            max_barrels,
            max_wheelbarrows,
        } => {
            if accepts.len() > MAX_LIST_ITEMS {
                return Err(invalid("too many accepted stockpile categories"));
            }
            Action::ConfigureStockpile {
                stockpile: entity(&stockpile)?,
                accepts: accepts
                    .into_iter()
                    .map(|token| name(token, "stockpile category"))
                    .collect::<Result<BTreeSet<_>>>()?,
                max_bins,
                max_barrels,
                max_wheelbarrows,
            }
        }
        ActionSpec::AssignSquad { units, squad } => Action::AssignSquad {
            units: entities(units)?,
            squad: entity(&squad)?,
        },
        ActionSpec::SetBurrowMembership {
            units,
            burrow,
            assigned,
        } => Action::SetBurrowMembership {
            units: entities(units)?,
            burrow: entity(&burrow)?,
            assigned,
        },
        ActionSpec::SetStandingOrder { key, value } => Action::SetStandingOrder {
            key: name(key, "standing order key")?,
            value: name(value, "standing order value")?,
        },
    })
}

/// Parse a JSON array of `{ "action": {...}, "depends_on": [step indices] }`.
/// Postconditions, obligations and compensations come from the reference
/// action model when the planner seals the plan.
pub(crate) fn parse_steps(raw: &str) -> Result<Vec<RequestedAction>> {
    if raw.len() > MAX_ACTIONS_JSON_BYTES {
        return Err(DfmcpError::new(
            ErrorCode::BudgetExceeded,
            format!("actions exceed {MAX_ACTIONS_JSON_BYTES} bytes"),
        ));
    }
    let steps: Vec<StepSpec> = serde_json::from_str(raw).map_err(|error| {
        invalid(format!(
            "actions must be a JSON array of {{\"action\":{{\"kind\":...}},\"depends_on\":[...]}}: {error}"
        ))
    })?;
    if steps.is_empty() || steps.len() > MAX_LAB_STEPS {
        return Err(invalid(format!(
            "a laboratory plan has 1..={MAX_LAB_STEPS} steps"
        )));
    }
    steps
        .into_iter()
        .map(|step| {
            Ok(RequestedAction {
                action: action(step.action)?,
                preconditions: Vec::new(),
                postconditions: Vec::new(),
                compensation: None,
                obligation: None,
                depends_on: step.depends_on,
            })
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Blueprints
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(tag = "template", rename_all = "snake_case", deny_unknown_fields)]
enum BlueprintSpec {
    BedroomCluster {
        origin: [i32; 3],
        rooms: u32,
        room_size: [u8; 2],
    },
    DiningHall {
        origin: [i32; 3],
        width: u8,
        height: u8,
    },
    WorkshopHub {
        origin: [i32; 3],
        bays: u32,
    },
    StockpileVault {
        origin: [i32; 3],
        width: u8,
        height: u8,
        category: String,
    },
}

/// Parse `{"template": "bedroom_cluster"|"dining_hall"|"workshop_hub"|
/// "stockpile_vault", "origin": [x,y,z], ...}` into a blueprint template.
pub(crate) fn parse_blueprint(raw: &str) -> Result<(MapCoord, dfmcp_intent::BlueprintTemplate)> {
    if raw.len() > MAX_QUERY_JSON_BYTES {
        return Err(DfmcpError::new(
            ErrorCode::BudgetExceeded,
            "blueprint request exceeds its byte bound",
        ));
    }
    let spec: BlueprintSpec = serde_json::from_str(raw).map_err(|error| {
        invalid(format!(
            "blueprint must be {{\"template\":\"bedroom_cluster|dining_hall|workshop_hub|stockpile_vault\",\"origin\":[x,y,z],...}}: {error}"
        ))
    })?;
    use dfmcp_intent::BlueprintTemplate as T;
    Ok(match spec {
        BlueprintSpec::BedroomCluster {
            origin,
            rooms,
            room_size,
        } => (
            coord(origin),
            T::BedroomCluster {
                rooms_count: rooms,
                room_size: (room_size[0], room_size[1]),
            },
        ),
        BlueprintSpec::DiningHall {
            origin,
            width,
            height,
        } => (coord(origin), T::DiningHall { width, height }),
        BlueprintSpec::WorkshopHub { origin, bays } => {
            (coord(origin), T::WorkshopHub { bays_count: bays })
        }
        BlueprintSpec::StockpileVault {
            origin,
            width,
            height,
            category,
        } => (
            coord(origin),
            T::StockpileVault {
                width,
                height,
                category: name(category, "stockpile category")?,
            },
        ),
    })
}

/// The spatial index the blueprint hazard preflight reads, built from every
/// canonical chunk of the observed world.
pub(crate) fn spatial_index(
    snapshot: &WorldSnapshot,
) -> Result<dfmcp_world::spatial_index::ChunkSpatialIndex> {
    let mut index = dfmcp_world::spatial_index::ChunkSpatialIndex::new();
    for chunk in snapshot.graph.chunks.values() {
        index.insert_or_update_chunk(chunk)?;
    }
    Ok(index)
}

// ---------------------------------------------------------------------------
// JSON projections
// ---------------------------------------------------------------------------

const JSON_SAFE_INTEGER: u64 = (1 << 53) - 1;

pub(crate) fn value_json(value: &Value) -> Json {
    match value {
        Value::Null => Json::Null,
        Value::Bool(value) => json!(value),
        Value::I64(value) if value.unsigned_abs() <= JSON_SAFE_INTEGER => json!(value),
        Value::I64(value) => json!(value.to_string()),
        Value::U64(value) if *value <= JSON_SAFE_INTEGER => json!(value),
        Value::U64(value) => json!(value.to_string()),
        Value::Fixed { units, scale } => json!({"units": units.to_string(), "scale": scale}),
        Value::Text(value) => json!(value),
        Value::Entity(id) => json!({"entity_id": id.get().to_string()}),
        Value::Coord(c) => json!([c.x, c.y, c.z]),
        Value::Bytes(bytes) => json!({"bytes": bytes.len()}),
        Value::List(values) => Json::Array(values.iter().map(value_json).collect()),
        Value::Object(values) => Json::Object(
            values
                .iter()
                .map(|(key, value)| (key.clone(), value_json(value)))
                .collect(),
        ),
    }
}

const fn op_name(op: CompareOp) -> &'static str {
    match op {
        CompareOp::Eq => "eq",
        CompareOp::Ne => "ne",
        CompareOp::Lt => "lt",
        CompareOp::Le => "le",
        CompareOp::Gt => "gt",
        CompareOp::Ge => "ge",
    }
}

pub(crate) fn predicate_json(predicate: &Predicate) -> Json {
    match predicate {
        Predicate::True => json!(true),
        Predicate::False => json!(false),
        Predicate::EntityExists(id) => json!({"entity_exists": id.get().to_string()}),
        Predicate::EntityKind { entity_id, kind } => {
            json!({"entity_kind": {"entity_id": entity_id.get().to_string(), "kind": kind.as_str()}})
        }
        Predicate::FieldCompare {
            entity_id,
            field,
            op,
            value,
        } => json!({"field": {
            "entity_id": entity_id.get().to_string(),
            "field": field,
            "op": op_name(*op),
            "value": value_json(value),
        }}),
        Predicate::EdgeExists { edge_id, kind } => json!({"edge_exists": {
            "edge_id": edge_id.get().to_string(),
            "kind": kind.as_ref().map(|kind| format!("{kind:?}")),
        }}),
        Predicate::Paused(paused) => json!({"paused": paused}),
        Predicate::RegionTerrain { area, tile_code } => json!({"region_terrain": {
            "min": [area.min.x, area.min.y, area.min.z],
            "max": [area.max.x, area.max.y, area.max.z],
            "tile": tile_name(*tile_code),
        }}),
        Predicate::All(children) => {
            json!({"all": children.iter().map(predicate_json).collect::<Vec<_>>()})
        }
        Predicate::Any(children) => {
            json!({"any": children.iter().map(predicate_json).collect::<Vec<_>>()})
        }
        Predicate::Not(child) => json!({"not": predicate_json(child)}),
    }
}

pub(crate) const fn tile_name(code: u32) -> &'static str {
    match code {
        tile_codes::OPEN_SPACE => "open_space",
        tile_codes::FLOOR => "floor",
        tile_codes::SOLID_WALL => "wall",
        tile_codes::STAIR => "stair",
        tile_codes::RAMP => "ramp",
        tile_codes::FORTIFICATION => "fortification",
        tile_codes::TREE => "tree",
        tile_codes::MAGMA_WALL => "magma",
        tile_codes::CHASM => "chasm",
        _ => "other",
    }
}

const fn tile_glyph(code: Option<u32>) -> char {
    match code {
        None => '?',
        Some(tile_codes::OPEN_SPACE) => ' ',
        Some(tile_codes::FLOOR) => '.',
        Some(tile_codes::SOLID_WALL) => '#',
        Some(tile_codes::STAIR) => 'X',
        Some(tile_codes::RAMP) => '^',
        Some(tile_codes::FORTIFICATION) => '%',
        Some(tile_codes::TREE) => 'T',
        Some(tile_codes::MAGMA_WALL) => '~',
        Some(tile_codes::CHASM) => '_',
        Some(_) => '*',
    }
}

pub(crate) const fn action_kind(action: &Action) -> &'static str {
    match action {
        Action::Pause { .. } => "pause",
        Action::DesignateDig { .. } => "designate_dig",
        Action::Build { .. } => "build",
        Action::SetLabor { .. } => "set_labor",
        Action::CreateWorkOrder { .. } => "create_work_order",
        Action::ConfigureStockpile { .. } => "configure_stockpile",
        Action::AssignSquad { .. } => "assign_squad",
        Action::SetBurrowMembership { .. } => "set_burrow_membership",
        Action::SetStandingOrder { .. } => "set_standing_order",
        Action::Extension { .. } => "extension",
    }
}

/// Observed economy alerts: stocks that will run out soon, are exhausted, or
/// dwarves left thirsty or hungry. Each names a concrete remedy plan.
pub(crate) fn world_alerts(snapshot: &WorldSnapshot) -> Vec<Json> {
    let Some(ledger) =
        effects::stock_ledger(snapshot).and_then(|id| snapshot.graph.entities.get(&id))
    else {
        return Vec::new();
    };
    let units: Vec<&EntityRecord> = snapshot
        .graph
        .entities
        .values()
        .filter(|e| {
            e.kind == EntityKind::Unit
                && e.fields.get("alive").map(|f| &f.value) != Some(&Value::Bool(false))
        })
        .collect();
    if units.is_empty() {
        return Vec::new();
    }
    let mut alerts = Vec::new();
    for (stock, interval, need, deprived, job, noun) in [
        (
            effects::STOCK_DRINK_FIELD,
            effects::DRINK_INTERVAL_TICKS,
            effects::NEED_DRINK_FIELD,
            "thirsty",
            "BREW_DRINK",
            "drink",
        ),
        (
            effects::STOCK_FOOD_FIELD,
            effects::FOOD_INTERVAL_TICKS,
            effects::NEED_FOOD_FIELD,
            "hungry",
            "PREPARE_MEAL",
            "food",
        ),
    ] {
        let held = match ledger.fields.get(stock).map(|f| &f.value) {
            Some(Value::U64(held)) => *held,
            _ => continue,
        };
        let living = units.len() as u64;
        let ticks_left = held / living * interval;
        let starving = units
            .iter()
            .filter(|u| {
                matches!(u.fields.get(need).map(|f| &f.value), Some(Value::Text(t)) if t == deprived)
            })
            .count();
        let remedy = json!({
            "tool": "fortress.plan",
            "arguments": {"actions": format!(
                r#"[{{"action":{{"kind":"create_work_order","name":"{noun} supply","job_token":"{job}","amount":{}}}}}]"#,
                (living * 4).div_ceil(5)
            )},
            "requires": "configure_production",
        });
        let (severity, finding) = if starving > 0 {
            (
                "critical",
                format!("{starving} dwarves are {deprived}: the fortress is out of {noun}"),
            )
        } else if held == 0 {
            ("critical", format!("the fortress has no {noun} left"))
        } else if ticks_left < 4 * interval {
            (
                "high",
                format!(
                    "{noun} runs out in about {ticks_left} game ticks ({held} units for {living} dwarves)"
                ),
            )
        } else {
            continue;
        };
        alerts.push(json!({
            "alert": format!("{noun}_supply"),
            "severity": severity,
            "finding": finding,
            "stock": held,
            "ticks_until_exhausted": ticks_left,
            "deprived_units": starving,
            "remedy": remedy,
        }));
    }
    alerts
}

/// Sealed plan steps with the entities they will create and the exact
/// predicates that must be observed before they count as done.
pub(crate) fn plan_steps_json(plan: &PreparedPlan) -> Json {
    Json::Array(
        plan.steps
            .iter()
            .map(|step| {
                let creates = matches!(
                    step.action,
                    Action::DesignateDig { .. } | Action::Build { .. } | Action::CreateWorkOrder { .. }
                )
                .then(|| effects::created_entity_id(&step.idempotency_key, 0).get().to_string());
                json!({
                    "step": step.id.get(),
                    "kind": action_kind(&step.action),
                    "capability": step.required_capability.as_str(),
                    "risk": step.risk.as_str(),
                    "depends_on": step.depends_on.iter().map(|id| id.get()).collect::<Vec<_>>(),
                    "creates_entity_id": creates,
                    "postconditions": step.postconditions.iter().map(predicate_json).collect::<Vec<_>>(),
                    "obligation": step.obligation.as_ref().map(|obligation| json!({
                        "terminal": predicate_json(&obligation.terminal),
                        "deadline_tick": obligation.deadline_tick.0,
                        "poll_interval_ticks": obligation.poll_interval_ticks,
                        "stable_observations": obligation.stable_for_observations,
                    })),
                    "compensable": step.compensation.is_some(),
                })
            })
            .collect(),
    )
}

/// Most active-work entries and units a briefing lists explicitly.
const MAX_BRIEFING_ITEMS: usize = 16;

fn text_field<'a>(entity: &'a EntityRecord, name: &str) -> Option<&'a str> {
    match entity.fields.get(name).map(|fact| &fact.value) {
        Some(Value::Text(value)) => Some(value),
        _ => None,
    }
}

/// A bounded situational briefing of the whole laboratory world: entity counts
/// by kind (complete), active work with progress, and the dwarves. Lists are
/// capped and say how many were omitted; counts are always complete.
pub(crate) fn briefing(snapshot: &WorldSnapshot) -> Json {
    let mut counts: BTreeMap<&str, u64> = BTreeMap::new();
    for entity in snapshot.graph.entities.values() {
        *counts.entry(entity.kind.as_str()).or_insert(0) += 1;
    }
    let active: Vec<&EntityRecord> = snapshot
        .graph
        .entities
        .values()
        .filter(|entity| {
            matches!(
                text_field(entity, effects::STATUS_FIELD),
                Some(effects::STATUS_ACTIVE)
            ) || matches!(
                text_field(entity, effects::CONSTRUCTION_STAGE_FIELD),
                Some(effects::STAGE_PLANNED | effects::STAGE_UNDER_CONSTRUCTION)
            )
        })
        .collect();
    let active_work: Vec<Json> = active
        .iter()
        .take(MAX_BRIEFING_ITEMS)
        .map(|entity| {
            let progress: BTreeMap<String, Json> = [
                effects::TILES_REMAINING_FIELD,
                effects::AMOUNT_REMAINING_FIELD,
                effects::CONSTRUCTION_STAGE_FIELD,
                "progress_ticks",
                "required_ticks",
            ]
            .iter()
            .filter_map(|name| {
                entity
                    .fields
                    .get(*name)
                    .map(|fact| ((*name).to_owned(), value_json(&fact.value)))
            })
            .collect();
            json!({
                "entity_id": entity.id.get().to_string(),
                "kind": entity.kind.as_str(),
                "label": entity.label,
                "progress": progress,
            })
        })
        .collect();
    let units: Vec<&EntityRecord> = snapshot
        .graph
        .entities
        .values()
        .filter(|entity| entity.kind == EntityKind::Unit)
        .collect();
    let dwarves: Vec<Json> = units
        .iter()
        .take(MAX_BRIEFING_ITEMS)
        .map(|unit| {
            let labors: Vec<&str> = unit
                .fields
                .iter()
                .filter(|(name, fact)| {
                    name.starts_with(effects::LABOR_FIELD_PREFIX) && fact.value == Value::Bool(true)
                })
                .map(|(name, _)| &name[effects::LABOR_FIELD_PREFIX.len()..])
                .collect();
            json!({
                "entity_id": unit.id.get().to_string(),
                "name": unit.label,
                "profession": text_field(unit, "profession"),
                "enabled_labors": labors,
                "squad": unit.fields.get(effects::SQUAD_FIELD).map(|fact| value_json(&fact.value)),
            })
        })
        .collect();
    json!({
        "counts_by_kind": counts,
        "terrain_chunks_observed": snapshot.graph.chunks.len(),
        "active_work": active_work,
        "active_work_omitted": active.len().saturating_sub(MAX_BRIEFING_ITEMS),
        "dwarves": dwarves,
        "dwarves_omitted": units.len().saturating_sub(MAX_BRIEFING_ITEMS),
        "drill_down": "fortress.query with {\"mode\":\"entities\",\"kind\":...} or {\"mode\":\"terrain\",...}",
    })
}

#[derive(Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
enum QuerySpec {
    Entities {
        kind: Option<String>,
        limit: Option<usize>,
        offset: Option<usize>,
    },
    Terrain {
        min: [i32; 3],
        max: [i32; 3],
    },
}

/// Answer the structured laboratory query modes. `raw` is either the bare
/// word `entities` or a JSON object with a `mode` of `entities` or `terrain`.
pub(crate) fn query(snapshot: &WorldSnapshot, raw: &str) -> Result<Json> {
    if raw.len() > MAX_QUERY_JSON_BYTES {
        return Err(DfmcpError::new(
            ErrorCode::BudgetExceeded,
            "query specification exceeds its byte bound",
        ));
    }
    let spec = if raw == "entities" {
        QuerySpec::Entities {
            kind: None,
            limit: None,
            offset: None,
        }
    } else {
        serde_json::from_str(raw).map_err(|error| {
            invalid(format!(
                "query mode must be \"summary\", \"entities\", or a JSON object with mode entities|terrain: {error}"
            ))
        })?
    };
    match spec {
        QuerySpec::Entities {
            kind,
            limit,
            offset,
        } => entities_page(snapshot, kind.as_deref(), limit, offset),
        QuerySpec::Terrain { min, max } => terrain(snapshot, coord(min), coord(max)),
    }
}

fn entities_page(
    snapshot: &WorldSnapshot,
    kind: Option<&str>,
    limit: Option<usize>,
    offset: Option<usize>,
) -> Result<Json> {
    let limit = limit.unwrap_or(25);
    if limit == 0 || limit > MAX_ENTITY_PAGE {
        return Err(invalid(format!(
            "entity page limit must be 1..={MAX_ENTITY_PAGE}"
        )));
    }
    let offset = offset.unwrap_or(0);
    let matching: Vec<&EntityRecord> = snapshot
        .graph
        .entities
        .values()
        .filter(|entity| kind.is_none_or(|kind| entity.kind.as_str() == kind))
        .collect();
    if offset > matching.len() {
        return Err(DfmcpError::new(
            ErrorCode::CursorGap,
            "entity page offset is beyond the result",
        ));
    }
    let rows: Vec<Json> = matching
        .iter()
        .skip(offset)
        .take(limit)
        .map(|entity| {
            json!({
                "entity_id": entity.id.get().to_string(),
                "kind": entity.kind.as_str(),
                "label": entity.label,
                "generation": entity.generation,
                "revision": entity.revision,
                "fields": entity.fields.iter()
                    .map(|(name, fact)| (name.clone(), value_json(&fact.value)))
                    .collect::<BTreeMap<_, _>>(),
            })
        })
        .collect();
    let end = offset + rows.len();
    Ok(json!({
        "mode": "entities",
        "kind": kind,
        "total": matching.len(),
        "offset": offset,
        "returned": rows.len(),
        "next_offset": (end < matching.len()).then_some(end),
        "complete_domain": true,
        "rows": rows,
    }))
}

fn terrain(snapshot: &WorldSnapshot, min: MapCoord, max: MapCoord) -> Result<Json> {
    let area = MapCuboid::new(min, max)?;
    let tiles = validate_region(area)?;
    if tiles > MAX_TERRAIN_QUERY_TILES {
        return Err(DfmcpError::new(
            ErrorCode::BudgetExceeded,
            format!("terrain queries cover at most {MAX_TERRAIN_QUERY_TILES} tiles"),
        ));
    }
    let mut counts: BTreeMap<&str, u64> = BTreeMap::new();
    let mut levels = Vec::new();
    for z in area.min.z..=area.max.z {
        let mut rows = Vec::new();
        for y in area.min.y..=area.max.y {
            let row: String = (area.min.x..=area.max.x)
                .map(|x| {
                    let code = snapshot.tile_code_at(MapCoord::new(x, y, z));
                    *counts.entry(code.map_or("unknown", tile_name)).or_insert(0) += 1;
                    tile_glyph(code)
                })
                .collect();
            rows.push(row);
        }
        levels.push(json!({"z": z, "rows": rows}));
    }
    Ok(json!({
        "mode": "terrain",
        "min": [min.x, min.y, min.z],
        "max": [max.x, max.y, max.z],
        "legend": {"#": "wall", ".": "floor", " ": "open_space", "X": "stair", "^": "ramp",
                    "~": "magma", "_": "chasm", "T": "tree", "%": "fortification", "?": "unobserved"},
        "row_order": "y ascending; each row is x ascending",
        "counts": counts,
        "levels": levels,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starter_fortress_has_rock_a_hall_and_named_entities() -> Result<()> {
        let snapshot = scenario_snapshot("starter_fortress", FortressId::new(3), true)?;
        assert!(snapshot.hash_is_valid());
        assert_eq!(
            snapshot.tile_code_at(MapCoord::new(9, 2, starter::LEVEL_Z)),
            Some(tile_codes::FLOOR)
        );
        assert_eq!(
            snapshot.tile_code_at(MapCoord::new(10, 2, starter::LEVEL_Z)),
            Some(tile_codes::SOLID_WALL)
        );
        assert_eq!(
            snapshot.tile_code_at(MapCoord::new(-16, -16, starter::LEVEL_Z)),
            Some(tile_codes::SOLID_WALL)
        );
        assert_eq!(
            snapshot.tile_code_at(MapCoord::new(0, 0, 9)),
            Some(tile_codes::SOLID_WALL)
        );
        assert_eq!(snapshot.tile_code_at(MapCoord::new(0, 0, 12)), None);
        assert_eq!(snapshot.graph.entities.len(), 11);
        assert!(scenario_snapshot("moon_base", FortressId::new(3), true).is_err());
        Ok(())
    }

    #[test]
    fn action_requests_are_closed_bounded_and_typed() -> Result<()> {
        let steps = parse_steps(
            r#"[{"action":{"kind":"designate_dig","min":[0,3,10],"max":[4,5,10],"mode":"mine"}},
                {"action":{"kind":"build","building":"workshop:Still","location":[2,4,10],"min":[1,3,10],"max":[3,5,10]},"depends_on":[0]},
                {"action":{"kind":"set_labor","units":["1001"],"labor":"BREW","enabled":true}}]"#,
        )?;
        assert_eq!(steps.len(), 3);
        assert_eq!(steps[1].depends_on, vec![0]);
        for bad in [
            r#"[]"#,
            r#"[{"action":{"kind":"designate_dig","min":[0,0,0],"max":[1,1,0],"mode":"mine","extra":1}}]"#,
            r#"[{"action":{"kind":"designate_dig","min":[0,0,0],"max":[1,1,0],"mode":"blast"}}]"#,
            r#"[{"action":{"kind":"set_labor","units":["01"],"labor":"MINE","enabled":true}}]"#,
            r#"[{"action":{"kind":"set_labor","units":[],"labor":"MINE","enabled":true}}]"#,
            r#"[{"action":{"kind":"build","building":"spaceship","location":[0,0,0],"min":[0,0,0],"max":[0,0,0]}}]"#,
            r#"[{"action":{"kind":"extension"}}]"#,
            r#"[{"action":{"kind":"pause","paused":true},"note":"x"}]"#,
        ] {
            assert!(parse_steps(bad).is_err(), "{bad}");
        }
        Ok(())
    }

    #[test]
    fn briefing_counts_everything_and_lists_active_work_and_dwarves() -> Result<()> {
        let mut snapshot = scenario_snapshot("starter_fortress", FortressId::new(3), false)?;
        let dig = Action::DesignateDig {
            area: cuboid([0, 3, 10], [1, 3, 10])?,
            mode: DigMode::Mine,
        };
        effects::apply_effect(&mut snapshot, &dig, "k")?;
        let briefing = briefing(&snapshot);
        assert_eq!(briefing["counts_by_kind"]["unit"], 7);
        assert_eq!(briefing["counts_by_kind"]["dig_designation"], 1);
        assert_eq!(briefing["active_work"][0]["progress"]["tiles_remaining"], 2);
        assert_eq!(briefing["dwarves"].as_array().map(Vec::len), Some(7));
        assert_eq!(briefing["dwarves"][0]["profession"], "miner");
        Ok(())
    }

    #[test]
    fn queries_page_entities_and_render_terrain() -> Result<()> {
        let snapshot = scenario_snapshot("starter_fortress", FortressId::new(3), true)?;
        let page = query(&snapshot, r#"{"mode":"entities","kind":"unit","limit":5}"#)?;
        assert_eq!(page["total"], 7);
        assert_eq!(page["returned"], 5);
        assert_eq!(page["next_offset"], 5);
        assert_eq!(page["rows"][0]["entity_id"], "1001");
        let terrain = query(
            &snapshot,
            r#"{"mode":"terrain","min":[8,0,10],"max":[11,3,10]}"#,
        )?;
        assert_eq!(terrain["levels"][0]["rows"][0], "..##");
        assert_eq!(terrain["levels"][0]["rows"][3], "####");
        assert!(
            query(
                &snapshot,
                r#"{"mode":"terrain","min":[0,0,0],"max":[99,99,0]}"#
            )
            .is_err()
        );
        assert!(query(&snapshot, "everything").is_err());
        Ok(())
    }
}
