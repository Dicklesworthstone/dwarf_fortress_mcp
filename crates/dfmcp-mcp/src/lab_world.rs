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
    WorkOrderCondition,
};
use dfmcp_world::terrain::{region_tiles, uniform_chunk, validate_region};
use dfmcp_world::{
    ChunkCoord, CompareOp, EntityKind, EntityRecord, Fact, FactSource, Predicate, Value,
    WorldGraph, WorldSnapshot, laboratory_fact_value, tile_codes,
};
use serde::Deserialize;
use serde_json::{Value as Json, json};

#[path = "production_workload.rs"]
mod production_workload;
use production_workload::ProductionWorkload;

#[cfg(test)]
#[path = "production_workload_tests.rs"]
mod production_workload_tests;

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
pub(crate) const SCENARIOS: [&str; 3] = ["empty", "starter_fortress", "besieged_fortress"];

/// Starter-fortress entity identities.
pub(crate) mod starter {
    use dfmcp_core::EntityId;
    pub const FIRST_DWARF: u64 = 1_001;
    pub const FOOD_STOCKPILE: EntityId = EntityId::new(2_001);
    pub const INNER_BURROW: EntityId = EntityId::new(3_001);
    pub const MILITIA_SQUAD: EntityId = EntityId::new(4_001);
    pub const STOCK_LEDGER: EntityId = EntityId::new(5_001);
    /// The fortress's working still and kitchen, built at the hall's far end.
    pub const STILL: EntityId = EntityId::new(7_001);
    pub const KITCHEN: EntityId = EntityId::new(7_002);
    /// The raider of `besieged_fortress`.
    pub const RAIDER: EntityId = EntityId::new(6_001);
    pub const RAIDER_ARRIVES_AT: u64 = 1_500;
    pub const RAIDER_HEALTH: u64 = 120;
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
        "besieged_fortress" => {
            let mut graph = starter_graph()?;
            graph.entities.insert(
                starter::RAIDER,
                record(
                    starter::RAIDER,
                    EntityKind::Creature,
                    "Goblin raider",
                    vec![
                        (effects::HOSTILE_FIELD, Value::Bool(true)),
                        (effects::HEALTH_FIELD, Value::U64(starter::RAIDER_HEALTH)),
                        (
                            effects::ARRIVES_AT_FIELD,
                            Value::U64(starter::RAIDER_ARRIVES_AT),
                        ),
                        (
                            effects::THREAT_STATUS_FIELD,
                            Value::Text(effects::THREAT_APPROACHING.to_owned()),
                        ),
                        (effects::COMBAT_ROUNDS_FIELD, Value::U64(0)),
                    ],
                ),
            );
            graph
        }
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
        let mut fields = vec![
            ("profession", Value::Text(profession.to_owned())),
            ("alive", Value::Bool(true)),
            (
                "position",
                Value::Coord(MapCoord::new(x, 1, starter::LEVEL_Z)),
            ),
        ];
        // The brewer brews and the farmer cooks: production needs both.
        match profession {
            "brewer" => fields.push(("labor.BREW", Value::Bool(true))),
            "farmer" => fields.push(("labor.COOK", Value::Bool(true))),
            _ => {}
        }
        graph
            .entities
            .insert(id, record(id, EntityKind::Unit, name, fields));
    }
    for (id, kind, x) in [
        (starter::STILL, "workshop:Still", 8),
        (starter::KITCHEN, "workshop:Kitchen", 9),
    ] {
        let at = MapCoord::new(x, 0, starter::LEVEL_Z);
        graph.entities.insert(
            id,
            record(
                id,
                EntityKind::Building,
                kind,
                vec![
                    ("building_kind", Value::Text(kind.to_owned())),
                    ("position", Value::Coord(at)),
                    ("footprint_min", Value::Coord(at)),
                    ("footprint_max", Value::Coord(at)),
                    ("material_tokens", Value::List(Vec::new())),
                    (
                        effects::CONSTRUCTION_STAGE_FIELD,
                        Value::Text(effects::STAGE_COMPLETE.to_owned()),
                    ),
                    ("progress_ticks", Value::U64(effects::BUILD_TICKS)),
                    ("required_ticks", Value::U64(effects::BUILD_TICKS)),
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
enum WorkOrderConditionSpec {
    ItemCountBelow {
        item_token: String,
        threshold: u32,
    },
    MaterialAvailable {
        material_token: String,
        minimum: u32,
    },
    CompletedOrder {
        order_name: String,
    },
}

fn work_order_conditions(specs: Vec<WorkOrderConditionSpec>) -> Result<Vec<WorkOrderCondition>> {
    if specs.len() > effects::MAX_WORK_ORDER_CONDITIONS {
        return Err(invalid("work order exceeds the 64-condition bound"));
    }
    specs
        .into_iter()
        .map(|spec| {
            Ok(match spec {
                WorkOrderConditionSpec::ItemCountBelow {
                    item_token,
                    threshold,
                } => WorkOrderCondition::ItemCountBelow {
                    item_token: name(item_token, "item token")?,
                    threshold,
                },
                WorkOrderConditionSpec::MaterialAvailable {
                    material_token,
                    minimum,
                } => WorkOrderCondition::MaterialAvailable {
                    material_token: name(material_token, "material token")?,
                    minimum,
                },
                WorkOrderConditionSpec::CompletedOrder { order_name } => {
                    WorkOrderCondition::CompletedOrder {
                        order_name: name(order_name, "work order dependency")?,
                    }
                }
            })
        })
        .collect()
}

fn work_order_condition_json(condition: &WorkOrderCondition) -> Json {
    match condition {
        WorkOrderCondition::ItemCountBelow {
            item_token,
            threshold,
        } => json!({
            "kind": "item_count_below", "item_token": item_token, "threshold": threshold,
        }),
        WorkOrderCondition::MaterialAvailable {
            material_token,
            minimum,
        } => json!({
            "kind": "material_available", "material_token": material_token, "minimum": minimum,
        }),
        WorkOrderCondition::CompletedOrder { order_name } => json!({
            "kind": "completed_order", "order_name": order_name,
        }),
    }
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
        #[serde(default)]
        conditions: Vec<WorkOrderConditionSpec>,
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
            conditions,
        } => Action::CreateWorkOrder {
            name: name(order, "work order name")?,
            job_token: name(job_token, "job token")?,
            amount,
            conditions: work_order_conditions(conditions)?,
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

pub(crate) use crate::observation_projection::value_json;

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

/// Whether a blueprint request is the production objective template.
pub(crate) fn is_production_objective(raw: &str) -> bool {
    raw.len() <= MAX_ACTIONS_JSON_BYTES
        && serde_json::from_str::<Json>(raw).is_ok_and(|v| v["template"] == "production")
}

/// The laboratory recipe catalog, matching the reference effects exactly:
/// a brewing batch yields 5 drink and a meal batch 5 food, from no modeled
/// material inputs. Completed workshops and eligible workers are required.
fn lab_recipes() -> dfmcp_intent::ProductionLogisticsCompiler {
    let mut compiler = dfmcp_intent::ProductionLogisticsCompiler::without_recipes();
    for (output, job, workshop) in [
        ("DRINK", "BREW_DRINK", "Still"),
        ("FOOD", "PREPARE_MEAL", "Kitchen"),
    ] {
        compiler.register_recipe(dfmcp_intent::ProductionRecipe {
            output_token: output.to_owned(),
            output_batch_size: 5,
            input_tokens: Vec::new(),
            workshop: BuildingKind::Workshop(workshop.to_owned()),
            job_token: job.to_owned(),
        });
    }
    compiler
}

/// The complete original production request, before stock-dependent lowering.
/// Duplicate quotas mean the largest minimum; their canonical order and JSON
/// are independent of the order in which equivalent goals were submitted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProductionRequest {
    quotas: BTreeMap<String, u32>,
    prerequisites: Option<ProductionPrerequisites>,
}

/// Original permission to synthesize setup, independent of the current stock.
/// Retain unused sites so replay never invents a new construction location.
#[derive(Clone, Debug, PartialEq, Eq)]
struct ProductionPrerequisites {
    assign_labor: bool,
    workshops: BTreeMap<String, ProductionWorkshopSite>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ProductionWorkshopSite {
    location: MapCoord,
    footprint: MapCuboid,
}

const MAX_PRODUCTION_WORKSHOP_SITES: usize = 2;
const MAX_PRODUCTION_WORKSHOP_TILES: u64 = 64;
const MAX_PRODUCTION_SETUP_ENTITIES: usize = 65_536;

impl ProductionRequest {
    pub(crate) fn parse(raw: &str) -> Result<Self> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Quota {
            item: String,
            minimum: u32,
        }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Workshop {
            building: String,
            location: [i32; 3],
            min: Option<[i32; 3]>,
            max: Option<[i32; 3]>,
        }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Prerequisites {
            #[serde(default)]
            assign_labor: bool,
            #[serde(default)]
            workshops: Vec<Workshop>,
        }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Objective {
            template: String,
            quotas: Vec<Quota>,
            prerequisites: Option<Prerequisites>,
        }
        if raw.len() > MAX_ACTIONS_JSON_BYTES {
            return Err(invalid("production objective exceeds its byte bound"));
        }
        let objective: Objective = serde_json::from_str(raw).map_err(|error| {
            invalid(format!(
                "production objective must be {{\"template\":\"production\",\"quotas\":[{{\"item\":\"DRINK|FOOD\",\"minimum\":n}}]}}: {error}"
            ))
        })?;
        if objective.template != "production" {
            return Err(invalid("production objective requires template=production"));
        }
        // Bound submitted records before normalization: duplicates must not
        // bypass the same input allowance as distinct requested goals.
        if objective.quotas.is_empty() || objective.quotas.len() > MAX_LIST_ITEMS {
            return Err(invalid(
                "production objective requires 1..64 submitted quotas",
            ));
        }
        let mut quotas = BTreeMap::<String, u32>::new();
        for quota in objective.quotas {
            if !matches!(quota.item.as_str(), "DRINK" | "FOOD") {
                return Err(invalid(format!(
                    "production quota token {:?} is outside the laboratory DRINK/FOOD model",
                    quota.item
                )));
            }
            let minimum = quotas.entry(quota.item).or_default();
            *minimum = (*minimum).max(quota.minimum);
        }
        let prerequisites = objective
            .prerequisites
            .map(|setup| -> Result<ProductionPrerequisites> {
                // Bound submitted sites before canonical ordering or duplicate checks.
                if setup.workshops.len() > MAX_PRODUCTION_WORKSHOP_SITES {
                    return Err(invalid("production accepts at most two workshop sites"));
                }
                let mut workshops = BTreeMap::<String, ProductionWorkshopSite>::new();
                for site in setup.workshops {
                    let output = match site.building.as_str() {
                        "workshop:Still" => "DRINK",
                        "workshop:Kitchen" => "FOOD",
                        _ => {
                            return Err(invalid(
                                "production workshop sites require workshop:Still or workshop:Kitchen",
                            ));
                        }
                    };
                    if !quotas.contains_key(output) {
                        return Err(invalid(
                            "a workshop site must serve a quota in the original request",
                        ));
                    }
                    if workshops.contains_key(&site.building) {
                        return Err(invalid("production repeats a workshop site kind"));
                    }
                    let location = coord(site.location);
                    let footprint = match (site.min, site.max) {
                        (None, None) => MapCuboid::new(location, location)?,
                        (Some(min), Some(max)) => cuboid(min, max)?,
                        _ => {
                            return Err(invalid(
                                "production workshop min and max must be supplied together",
                            ));
                        }
                    };
                    if footprint.min.z != footprint.max.z
                        || !footprint.contains(location)
                        || validate_region(footprint)? > MAX_PRODUCTION_WORKSHOP_TILES
                    {
                        return Err(invalid(
                            "production workshop footprint must contain its location on one level and have at most 64 tiles",
                        ));
                    }
                    // Check halo arithmetic before retaining an otherwise valid site.
                    production_site_halo(footprint)?;
                    if workshops
                        .values()
                        .any(|other| production_regions_overlap(other.footprint, footprint))
                    {
                        return Err(invalid("production workshop sites overlap"));
                    }
                    workshops.insert(
                        site.building,
                        ProductionWorkshopSite {
                            location,
                            footprint,
                        },
                    );
                }
                Ok(ProductionPrerequisites {
                    assign_labor: setup.assign_labor,
                    workshops,
                })
            })
            .transpose()?;
        Ok(Self {
            quotas,
            prerequisites,
        })
    }

    pub(crate) fn canonical_json(&self) -> String {
        let mut request = json!({
            "template": "production",
            "quotas": self.quotas.iter().map(|(item, minimum)| {
                json!({"item": item, "minimum": minimum})
            }).collect::<Vec<_>>(),
        });
        // Keep the established source bytes for requests without new options.
        if let Some(setup) = &self.prerequisites {
            request["prerequisites"] = json!({
                "assign_labor": setup.assign_labor,
                "workshops": setup.workshops.iter().map(|(building, site)| json!({
                    "building": building,
                    "location": [site.location.x, site.location.y, site.location.z],
                    "min": [site.footprint.min.x, site.footprint.min.y, site.footprint.min.z],
                    "max": [site.footprint.max.x, site.footprint.max.y, site.footprint.max.z],
                })).collect::<Vec<_>>(),
            });
        }
        request.to_string()
    }

    /// Recompile from the original quotas at this one observed snapshot.
    /// Work-order completion and the original stock goal remain separate.
    pub(crate) fn compile(&self, snapshot: &WorldSnapshot) -> Result<ProductionCompilation> {
        let evidence = dfmcp_world::PredicateEvidence::laboratory(snapshot)?;
        let snapshot = evidence.snapshot();
        let mut inventory = dfmcp_intent::InventoryStockpile::new();
        let ledger =
            effects::stock_ledger(snapshot).and_then(|id| snapshot.graph.entities.get(&id));
        let mut terminal = Vec::with_capacity(self.quotas.len());
        for (token, minimum) in &self.quotas {
            let field = match token.as_str() {
                "DRINK" => effects::STOCK_DRINK_FIELD,
                "FOOD" => effects::STOCK_FOOD_FIELD,
                _ => return Err(invalid("production quota escaped its closed model")),
            };
            let held = ledger
                .and_then(|ledger| ledger.fields.get(field))
                .and_then(|fact| laboratory_fact_value(fact, snapshot.tick));
            let Some(Value::U64(held)) = held else {
                return Err(DfmcpError::new(
                    ErrorCode::PreconditionsFailed,
                    format!(
                        "production quota for {token} requires an established laboratory stock count in {field}"
                    ),
                ));
            };
            let count = u32::try_from(*held).map_err(|_| {
                DfmcpError::new(
                    ErrorCode::BudgetExceeded,
                    "observed production stock exceeds the compiler's exact u32 count bound",
                )
            })?;
            inventory.set_stock(token, count);
            let ledger = ledger.ok_or_else(|| {
                DfmcpError::new(
                    ErrorCode::PreconditionsFailed,
                    "production stock ledger is not established",
                )
            })?;
            terminal.push(Predicate::FieldCompare {
                entity_id: ledger.id,
                field: field.to_owned(),
                op: CompareOp::Ge,
                value: Value::U64(u64::from(*minimum)),
            });
        }
        let quotas: Vec<dfmcp_intent::ProductionQuota> = self
            .quotas
            .iter()
            .map(|(item, minimum)| dfmcp_intent::ProductionQuota {
                item_token: item.clone(),
                minimum_stock: *minimum,
            })
            .collect();
        let plan = lab_recipes().plan_quotas(
            &quotas,
            &inventory,
            dfmcp_intent::ProductionPlanningLimits::default(),
        )?;
        let mut analysis = json!({
            "model": "laboratory recipes (5 units per batch, no modeled inputs; each job needs its completed workshop and a living worker with the labor); stock read from the stock ledger",
            "feasible": plan.model_feasible(),
            "requirements": plan.requirements().iter().map(|r| json!({
                "item": r.item_token, "minimum_stock": r.minimum_stock, "stock": r.stock_units,
                "planned": r.planned_units, "missing": r.missing_units,
            })).collect::<Vec<_>>(),
            "shortages": plan.shortages().iter().map(|s| json!({
                "item": s.item_token, "required": s.required_units, "stock": s.stock_units, "missing": s.missing_units,
            })).collect::<Vec<_>>(),
        });
        if !plan.model_feasible() {
            return Err(DfmcpError::new(
                ErrorCode::PreconditionsFailed,
                format!("production quotas are infeasible in the laboratory model: {analysis}"),
            ));
        }
        if plan.steps().is_empty() {
            return Err(DfmcpError::new(
                ErrorCode::InvalidIntent,
                "observed stock already meets every quota; nothing to produce",
            ));
        }
        let jobs: Vec<(&str, &str)> = plan
            .steps()
            .iter()
            .map(|step| (step.output_token.as_str(), step.job_token.as_str()))
            .collect();
        // Existing physical work belongs to the shared fortress, including
        // work whose original process/action handles no longer exist. Its
        // observed remaining service cannot disappear from a new deadline.
        let workload = ProductionWorkload::capture(snapshot, &jobs)?;
        let setup = match &self.prerequisites {
            Some(options) => compile_production_prerequisites(snapshot, &jobs, options)?,
            None => {
                // Preserve the existing no-setup contract and sealed actions.
                let blockers: Vec<String> = jobs
                    .iter()
                    .filter_map(|(output, job)| {
                        effects::work_order_blocker(snapshot, job)
                            .map(|why| format!("{output} ({job}): {why}"))
                    })
                    .collect();
                if !blockers.is_empty() {
                    return Err(DfmcpError::new(
                        ErrorCode::PreconditionsFailed,
                        format!(
                            "production cannot progress in the observed fortress: {}; supply explicit prerequisites to plan staffing or a workshop site",
                            blockers.join("; ")
                        ),
                    ));
                }
                ProductionSetup::default()
            }
        };
        let setup_count = setup.actions.len();
        let offset = u32::try_from(setup_count)
            .map_err(|_| invalid("production setup step index overflow"))?;
        let mut steps = setup.actions;
        let mut order_indices = BTreeMap::<String, u32>::new();
        for step in plan.steps() {
            let mut dependencies = setup
                .dependencies
                .get(&step.job_token)
                .cloned()
                .unwrap_or_default();
            if let Some(previous_job) = setup.serial_after.get(&step.job_token) {
                let previous = order_indices.get(previous_job).ok_or_else(|| {
                    invalid("production staffing dependency does not precede its consumer")
                })?;
                dependencies.insert(*previous);
            }
            for dependency in &step.depends_on {
                let dependency = u32::try_from(*dependency)
                    .map_err(|_| invalid("production dependency step index overflow"))?;
                dependencies.insert(
                    dependency
                        .checked_add(offset)
                        .ok_or_else(|| invalid("production dependency step index overflow"))?,
                );
            }
            order_indices.insert(
                step.job_token.clone(),
                u32::try_from(steps.len())
                    .map_err(|_| invalid("production order step index overflow"))?,
            );
            steps.push(json!({
                "action": {
                    "kind": "create_work_order",
                    "name": format!("{} for quota", step.output_token.to_lowercase()),
                    "job_token": step.job_token,
                    "amount": step.batches,
                    "conditions": [{
                        "kind": "item_count_below",
                        "item_token": step.output_token,
                        "threshold": step.inventory_threshold,
                    }],
                },
                "depends_on": dependencies,
            }));
        }
        let actions = Json::Array(steps).to_string();
        // The expanded program obeys the ordinary action parser's byte/step bounds.
        let parsed = parse_steps(&actions)?;
        if let Some(capacity) = workload.analysis() {
            analysis["capacity"] = capacity;
        }
        if self.prerequisites.is_some() {
            analysis["prerequisites"] = json!({
                "steps_added": setup_count,
                "workshops": setup.workshops,
                "staffing": setup.staffing,
                "selection_policy": "reuse eligible complete workshops; maximize distinct eligible living workers, minimize labor enables, then prefer known non-military, unknown, assigned workers and canonical per-job entity IDs",
                "scope": "reference laboratory only; generated setup requires its own action capabilities; jobs sharing one selected worker are serialized through completion dependencies",
            });
            analysis["required_action_capabilities"] = json!(
                parsed
                    .iter()
                    .map(|step| step.action.capability().as_str())
                    .collect::<BTreeSet<_>>()
            );
        }
        Ok(ProductionCompilation {
            actions,
            analysis,
            terminal: Predicate::All(terminal).normalized(),
            workload,
        })
    }
}

#[derive(Default)]
struct ProductionSetup {
    actions: Vec<Json>,
    dependencies: BTreeMap<String, BTreeSet<u32>>,
    serial_after: BTreeMap<String, String>,
    workshops: Vec<Json>,
    staffing: Vec<Json>,
}

fn production_fact<'a>(entity: &'a EntityRecord, field: &str, tick: GameTick) -> Option<&'a Value> {
    entity
        .fields
        .get(field)
        .and_then(|fact| laboratory_fact_value(fact, tick))
}

fn production_regions_overlap(left: MapCuboid, right: MapCuboid) -> bool {
    left.min.x <= right.max.x
        && left.max.x >= right.min.x
        && left.min.y <= right.max.y
        && left.max.y >= right.min.y
        && left.min.z <= right.max.z
        && left.max.z >= right.min.z
}

fn production_site_halo(area: MapCuboid) -> Result<MapCuboid> {
    let lower = |value: i32| {
        value
            .checked_sub(1)
            .ok_or_else(|| invalid("production site cannot represent its safety halo"))
    };
    let upper = |value: i32| {
        value
            .checked_add(1)
            .ok_or_else(|| invalid("production site cannot represent its safety halo"))
    };
    MapCuboid::new(
        MapCoord::new(lower(area.min.x)?, lower(area.min.y)?, lower(area.min.z)?),
        MapCoord::new(upper(area.max.x)?, upper(area.max.y)?, upper(area.max.z)?),
    )
}

fn production_setup_refusal(message: impl Into<String>) -> DfmcpError {
    DfmcpError::new(ErrorCode::PreconditionsFailed, message)
}

fn production_site_is_eligible(
    snapshot: &WorldSnapshot,
    site: &ProductionWorkshopSite,
) -> Result<()> {
    for at in region_tiles(site.footprint) {
        if snapshot.tile_code_at(at) != Some(tile_codes::FLOOR) {
            return Err(production_setup_refusal(format!(
                "production workshop requires established open floor at {at:?}"
            )));
        }
        let below_z =
            at.z.checked_sub(1)
                .ok_or_else(|| invalid("production site cannot represent its support level"))?;
        let below = MapCoord::new(at.x, at.y, below_z);
        if !matches!(
            snapshot.tile_code_at(below),
            Some(
                tile_codes::SOLID_WALL
                    | tile_codes::FLOOR
                    | tile_codes::STAIR
                    | tile_codes::RAMP
                    | tile_codes::FORTIFICATION
            )
        ) {
            return Err(production_setup_refusal(format!(
                "production workshop requires established support below {at:?}"
            )));
        }
    }
    // A caller-provided site is a constraint, not evidence of safety.
    for at in region_tiles(production_site_halo(site.footprint)?) {
        match snapshot.tile_code_at(at) {
            Some(
                tile_codes::OPEN_SPACE
                | tile_codes::FLOOR
                | tile_codes::SOLID_WALL
                | tile_codes::STAIR
                | tile_codes::RAMP
                | tile_codes::FORTIFICATION
                | tile_codes::TREE,
            ) => {}
            _ => {
                return Err(production_setup_refusal(format!(
                    "production workshop safety halo is unobserved, hazardous or unsupported at {at:?}"
                )));
            }
        }
    }
    for building in snapshot
        .graph
        .entities
        .values()
        .filter(|entity| entity.kind == EntityKind::Building)
    {
        let (Some(Value::Coord(min)), Some(Value::Coord(max))) = (
            production_fact(building, "footprint_min", snapshot.tick),
            production_fact(building, "footprint_max", snapshot.tick),
        ) else {
            return Err(production_setup_refusal(format!(
                "building {} has unknown footprint; production cannot establish an unoccupied site",
                building.id.get()
            )));
        };
        let footprint = MapCuboid::new(*min, *max).map_err(|_| {
            production_setup_refusal("an existing building has noncanonical footprint bounds")
        })?;
        if production_regions_overlap(footprint, site.footprint) {
            return Err(production_setup_refusal(format!(
                "production workshop site overlaps existing building {}",
                building.id.get()
            )));
        }
    }
    Ok(())
}

/// Prefer explicit non-membership to unavailable membership, and unavailable
/// membership to a known assignment. Absence is never reported as non-membership.
fn production_military_rank(unit: &EntityRecord, tick: GameTick) -> u8 {
    match production_fact(unit, effects::SQUAD_FIELD, tick) {
        Some(Value::Null) => 0,
        Some(Value::Entity(_) | Value::U64(_)) => 2,
        _ => 1,
    }
}

/// Select both jobs jointly: distinct workers first, then fewest new labor
/// enables, then military preference and canonical per-job entity identities.
/// For a fixed worker on the other job, at most one identity is excluded.
/// Its best counterpart is therefore among that role's first two candidates.
/// Four pairs suffice even for a complete 65,536-entity input.
fn select_production_workers<'a>(
    tick: GameTick,
    living: &[&'a EntityRecord],
    fields: &[&str],
    assign_labor: bool,
) -> Result<Vec<&'a EntityRecord>> {
    if fields.is_empty() || fields.len() > MAX_PRODUCTION_WORKSHOP_SITES {
        return Err(invalid(
            "production staffing requires one or two closed jobs",
        ));
    }
    let change_cost = |unit: &EntityRecord, field: &str| {
        u8::from(production_fact(unit, field, tick) != Some(&Value::Bool(true)))
    };
    let mut candidates = Vec::<Vec<&EntityRecord>>::with_capacity(fields.len());
    for field in fields {
        let mut best = Vec::with_capacity(3);
        for unit in living {
            match production_fact(unit, field, tick) {
                Some(Value::Bool(true)) => {}
                Some(Value::Bool(false)) if assign_labor => {}
                _ => continue,
            }
            best.push(*unit);
            best.sort_by_key(|unit| {
                (
                    change_cost(unit, field),
                    production_military_rank(unit, tick),
                    unit.id,
                )
            });
            best.truncate(2);
        }
        if best.is_empty() {
            return Err(production_setup_refusal(format!(
                "production requires a known living worker with {field}=true, or an explicitly false field and prerequisites.assign_labor=true; unavailable labor cannot establish staffing or compensation"
            )));
        }
        candidates.push(best);
    }
    let Some(first) = candidates.first() else {
        return Err(invalid("production staffing lost its first job"));
    };
    if fields.len() == 1 {
        return first
            .first()
            .copied()
            .map(|unit| vec![unit])
            .ok_or_else(|| invalid("production staffing lost its first candidate"));
    }
    let Some(second) = candidates.get(1) else {
        return Err(invalid("production staffing lost its second job"));
    };
    let mut best_pair = None;
    for left in first {
        for right in second {
            let left_military = production_military_rank(left, tick);
            let right_military = production_military_rank(right, tick);
            let key = (
                left.id == right.id,
                change_cost(left, fields[0]) + change_cost(right, fields[1]),
                left_military + right_military,
                left_military,
                right_military,
                left.id,
                right.id,
            );
            if best_pair.as_ref().is_none_or(|(prior, _, _)| key < *prior) {
                best_pair = Some((key, *left, *right));
            }
        }
    }
    best_pair
        .map(|(_, left, right)| vec![left, right])
        .ok_or_else(|| invalid("production staffing found no bounded candidate pair"))
}

fn compile_production_prerequisites(
    snapshot: &WorldSnapshot,
    jobs: &[(&str, &str)],
    options: &ProductionPrerequisites,
) -> Result<ProductionSetup> {
    if snapshot.graph.entities.len() > MAX_PRODUCTION_SETUP_ENTITIES
        || jobs.len() > MAX_PRODUCTION_WORKSHOP_SITES
    {
        return Err(DfmcpError::new(
            ErrorCode::BudgetExceeded,
            "production setup exceeds its complete entity or job bound",
        ));
    }
    let mut requirements = Vec::with_capacity(jobs.len());
    for (output, job) in jobs {
        let (workshop, labor) = effects::work_order_requirements(job)
            .ok_or_else(|| invalid("production setup escaped the closed recipe model"))?;
        requirements.push((*output, *job, workshop, format!("labor.{labor}"), labor));
    }
    let living: Vec<&EntityRecord> = snapshot
        .graph
        .entities
        .values()
        .filter(|unit| {
            unit.kind == EntityKind::Unit
                && production_fact(unit, "alive", snapshot.tick) == Some(&Value::Bool(true))
        })
        .collect();
    let fields: Vec<&str> = requirements
        .iter()
        .map(|(_, _, _, field, _)| field.as_str())
        .collect();
    let workers = select_production_workers(snapshot.tick, &living, &fields, options.assign_labor)?;
    let mut worker_jobs = BTreeMap::<EntityId, String>::new();
    let mut setup = ProductionSetup::default();
    for ((output, job, workshop, field, labor), worker) in requirements.iter().zip(workers) {
        let mut dependencies = BTreeSet::new();
        let already_enabled =
            production_fact(worker, field, snapshot.tick) == Some(&Value::Bool(true));
        let labor_action = if already_enabled {
            None
        } else {
            // The default inverse disables this same labor. It is an exact
            // restoration only because the eligible prior value is false.
            if production_fact(worker, field, snapshot.tick) != Some(&Value::Bool(false)) {
                return Err(production_setup_refusal(
                    "production staffing requires an established disabled labor for compensation",
                ));
            }
            let index = u32::try_from(setup.actions.len())
                .map_err(|_| invalid("production setup step index overflow"))?;
            setup.actions.push(json!({
                "action": {
                    "kind": "set_labor",
                    "units": [worker.id.get().to_string()],
                    "labor": labor,
                    "enabled": true,
                },
                "depends_on": [],
            }));
            dependencies.insert(index);
            Some(index)
        };
        let previous_job = worker_jobs.insert(worker.id, (*job).to_owned());
        if let Some(previous_job) = &previous_job {
            // Completion proof for the earlier order releases this worker.
            // The ordinary planner also offsets the later obligation by the
            // predecessor's horizon, so queue time cannot consume its own.
            setup
                .serial_after
                .insert((*job).to_owned(), previous_job.clone());
        }
        setup.staffing.push(json!({
            "job_token": job,
            "labor": labor,
            "unit": worker.id.get().to_string(),
            "evidence": if already_enabled { "observed" } else { "planned" },
            "action_index": labor_action,
            "reused_across_jobs": previous_job.is_some(),
            "serial_after_job": previous_job,
            "prior_labor": match production_fact(worker, field, snapshot.tick) {
                Some(Value::Bool(true)) => "enabled",
                Some(Value::Bool(false)) => "disabled",
                _ => "unknown",
            },
            "military_assignment": match production_military_rank(worker, snapshot.tick) {
                0 => "unassigned",
                2 => "assigned",
                _ => "unknown",
            },
        }));
        let ready = snapshot.graph.entities.values().find(|building| {
            building.kind == EntityKind::Building
                && production_fact(building, "building_kind", snapshot.tick)
                    == Some(&Value::Text((*workshop).to_owned()))
                && production_fact(building, effects::CONSTRUCTION_STAGE_FIELD, snapshot.tick)
                    == Some(&Value::Text(effects::STAGE_COMPLETE.to_owned()))
        });
        if let Some(building) = ready {
            setup.workshops.push(json!({
                "job_token": job,
                "building": workshop,
                "evidence": "observed",
                "entity_id": building.id.get().to_string(),
                "action_index": null,
            }));
        } else {
            let site = options.workshops.get(*workshop).ok_or_else(|| {
                production_setup_refusal(format!(
                    "{output} requires {workshop}; supply its explicit prerequisites.workshops site"
                ))
            })?;
            production_site_is_eligible(snapshot, site)?;
            let index = u32::try_from(setup.actions.len())
                .map_err(|_| invalid("production setup step index overflow"))?;
            setup.actions.push(json!({
                "action": {
                    "kind": "build",
                    "building": workshop,
                    "location": [site.location.x, site.location.y, site.location.z],
                    "min": [site.footprint.min.x, site.footprint.min.y, site.footprint.min.z],
                    "max": [site.footprint.max.x, site.footprint.max.y, site.footprint.max.z],
                },
                "depends_on": [],
            }));
            dependencies.insert(index);
            setup.workshops.push(json!({
                "job_token": job,
                "building": workshop,
                "evidence": "planned",
                "entity_id": null,
                "action_index": index,
                "location": [site.location.x, site.location.y, site.location.z],
                "min": [site.footprint.min.x, site.footprint.min.y, site.footprint.min.z],
                "max": [site.footprint.max.x, site.footprint.max.y, site.footprint.max.z],
            }));
        }
        setup.dependencies.insert((*job).to_owned(), dependencies);
    }
    Ok(setup)
}

pub(crate) struct ProductionCompilation {
    pub(crate) actions: String,
    pub(crate) analysis: Json,
    pub(crate) terminal: Predicate,
    workload: ProductionWorkload,
}

impl ProductionCompilation {
    /// Bind the queued-service allowance to actual sealed step identities.
    /// Call before preparing the intent; changing a deadline changes its seal.
    /// This only describes a reference-model allowance, never future proof or
    /// permission to dispatch existing or new work.
    pub(crate) fn apply_capacity_horizon(&self, intent: &mut dfmcp_intent::Intent) -> Result<()> {
        self.workload.apply_horizon(&self.actions, intent)
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
    for (stock, interval, need, deprived, noun) in [
        (
            effects::STOCK_DRINK_FIELD,
            effects::DRINK_INTERVAL_TICKS,
            effects::NEED_DRINK_FIELD,
            "thirsty",
            "drink",
        ),
        (
            effects::STOCK_FOOD_FIELD,
            effects::FOOD_INTERVAL_TICKS,
            effects::NEED_FOOD_FIELD,
            "hungry",
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
        // Keep about four rounds in stock; the production compiler sizes the
        // work orders against observed stock.
        let (token, job) = if noun == "drink" {
            ("DRINK", "BREW_DRINK")
        } else {
            ("FOOD", "PREPARE_MEAL")
        };
        let blocker = effects::work_order_blocker(snapshot, job);
        let labor = effects::work_order_requirements(job).map(|(_, labor)| labor);
        // Remedy the actual blocker first: a production plan that cannot
        // progress would only be refused.
        let remedy = match (&blocker, labor) {
            (None, _) => json!({
                "tool": "fortress.plan",
                "arguments": {"blueprint": json!({
                    "template": "production",
                    "quotas": [{"item": token, "minimum": living * 4}],
                }).to_string()},
                "requires": "configure_production",
            }),
            (Some(why), Some(labor)) if !why.contains("no completed") => {
                // Prefer a dwarf outside the militia for the missing labor.
                let worker = units
                    .iter()
                    .min_by_key(|u| (u.fields.contains_key(effects::SQUAD_FIELD), u.id));
                match worker {
                    Some(worker) => json!({
                        "tool": "fortress.plan",
                        "arguments": {"actions": json!([{"action": {
                            "kind": "set_labor",
                            "units": [worker.id.get().to_string()],
                            "labor": labor,
                            "enabled": true,
                        }}]).to_string()},
                        "requires": "configure_labor",
                        "then": "plan production once the labor is assigned",
                    }),
                    None => Json::Null,
                }
            }
            _ => Json::Null,
        };
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
            "production_blocked_by": blocker,
            "remedy": remedy,
        }));
    }
    alerts.extend(threat_alerts(snapshot, &units));
    alerts.extend(civilian_alert(snapshot, &units));
    alerts
}

const CIVILIAN_RESTRICTION: &str = "CIVILIAN_BURROW_RESTRICTION";

/// Render one reference action as a laboratory plan step.
fn action_step_json(action: &Action) -> Option<Json> {
    let ids = |units: &[EntityId]| {
        units
            .iter()
            .map(|u| u.get().to_string())
            .collect::<Vec<_>>()
    };
    Some(match action {
        Action::SetBurrowMembership {
            units,
            burrow,
            assigned,
        } => json!({"action": {
            "kind": "set_burrow_membership", "units": ids(units),
            "burrow": burrow.get().to_string(), "assigned": assigned}}),
        Action::SetStandingOrder { key, value } => json!({"action": {
            "kind": "set_standing_order", "key": key, "value": value}}),
        _ => return None,
    })
}

/// Civilian safety through `dfmcp_intent::CivilianAlertFsm`: the observed
/// level is derived from the world (civilians in the safe burrow and the
/// restriction order active), and the FSM's own transition supplies the
/// remedy — lockdown while a hostile is active, all-clear once none is.
fn civilian_alert(snapshot: &WorldSnapshot, units: &[&EntityRecord]) -> Vec<Json> {
    let Some(burrow) = snapshot
        .graph
        .entities
        .values()
        .find(|e| e.kind == EntityKind::Burrow)
        .map(|e| e.id)
    else {
        return Vec::new();
    };
    let squads: Vec<EntityId> = snapshot
        .graph
        .entities
        .values()
        .filter(|e| e.kind == EntityKind::Squad)
        .map(|e| e.id)
        .collect();
    let civilians: Vec<EntityId> = units
        .iter()
        .filter(|u| !u.fields.contains_key(effects::SQUAD_FIELD))
        .map(|u| u.id)
        .collect();
    if civilians.is_empty() {
        return Vec::new();
    }
    let hostile_active = snapshot.graph.entities.values().any(|e| {
        e.kind == EntityKind::Creature
            && e.fields.get(effects::HOSTILE_FIELD).map(|f| &f.value) == Some(&Value::Bool(true))
            && matches!(e.fields.get(effects::THREAT_STATUS_FIELD).map(|f| &f.value),
                Some(Value::Text(status)) if status != effects::THREAT_SLAIN)
    });
    let burrow_field = format!("{}{}", effects::BURROW_FIELD_PREFIX, burrow.get());
    let sheltered = civilians.iter().all(|id| {
        snapshot
            .graph
            .entities
            .get(id)
            .and_then(|u| u.fields.get(&burrow_field))
            .map(|f| &f.value)
            == Some(&Value::Bool(true))
    });
    let restricted = snapshot
        .graph
        .entities
        .get(&effects::fortress_settings_entity_id(snapshot.fortress_id))
        .and_then(|e| {
            e.fields.get(&format!(
                "{}{CIVILIAN_RESTRICTION}",
                effects::STANDING_ORDER_FIELD_PREFIX
            ))
        })
        .map(|f| &f.value)
        == Some(&Value::Text("ACTIVE".to_owned()));
    let observed = if sheltered && restricted {
        dfmcp_intent::ThreatLevel::EmergencyLockdown
    } else {
        dfmcp_intent::ThreatLevel::Peace
    };
    let (target, alert, severity, finding) = match (hostile_active, observed) {
        (true, dfmcp_intent::ThreatLevel::Peace) => (
            dfmcp_intent::ThreatLevel::EmergencyLockdown,
            "civilian_lockdown",
            "high",
            format!(
                "{} civilians are outside the safe burrow while a hostile is active",
                civilians.len()
            ),
        ),
        (false, dfmcp_intent::ThreatLevel::EmergencyLockdown) => (
            dfmcp_intent::ThreatLevel::Peace,
            "all_clear",
            "low",
            "no hostile is active; civilians are still confined to the safe burrow".to_owned(),
        ),
        _ => return Vec::new(),
    };
    let Ok(mut fsm) = dfmcp_intent::CivilianAlertFsm::new(burrow, squads) else {
        return Vec::new();
    };
    fsm.confirm_observed_level(observed);
    let Ok(actions) = fsm.transition_to(target, &civilians) else {
        return Vec::new();
    };
    let steps: Vec<Json> = actions.iter().filter_map(action_step_json).collect();
    vec![json!({
        "alert": alert,
        "severity": severity,
        "finding": finding,
        "remedy": {
            "tool": "fortress.plan",
            "arguments": {"actions": Json::Array(steps).to_string()},
            "requires": "configure_logistics",
        },
    })]
}

/// Approaching or attacking hostiles, with a squad-assignment remedy when
/// the fortress has a squad and dwarves not yet in it.
fn threat_alerts(snapshot: &WorldSnapshot, units: &[&EntityRecord]) -> Vec<Json> {
    let squad = snapshot
        .graph
        .entities
        .values()
        .find(|e| e.kind == EntityKind::Squad)
        .map(|e| e.id);
    let soldiers = units
        .iter()
        .filter(|u| {
            matches!(
                u.fields.get(effects::SQUAD_FIELD).map(|f| &f.value),
                Some(Value::Entity(_))
            )
        })
        .count();
    let recruits: Vec<String> = units
        .iter()
        .filter(|u| !u.fields.contains_key(effects::SQUAD_FIELD))
        .take(4)
        .map(|u| u.id.get().to_string())
        .collect();
    snapshot
        .graph
        .entities
        .values()
        .filter(|e| {
            e.kind == EntityKind::Creature
                && e.fields.get(effects::HOSTILE_FIELD).map(|f| &f.value) == Some(&Value::Bool(true))
        })
        .filter_map(|hostile| {
            let status = match hostile.fields.get(effects::THREAT_STATUS_FIELD).map(|f| &f.value) {
                Some(Value::Text(status)) if status != effects::THREAT_SLAIN => status.clone(),
                _ => return None,
            };
            let arrives = match hostile.fields.get(effects::ARRIVES_AT_FIELD).map(|f| &f.value) {
                Some(Value::U64(tick)) => *tick,
                _ => 0,
            };
            let attacking = status == effects::THREAT_ATTACKING;
            let remedy = squad.filter(|_| soldiers < 4 && !recruits.is_empty()).map(|squad| {
                json!({
                    "tool": "fortress.plan",
                    "arguments": {"actions": format!(
                        r#"[{{"action":{{"kind":"assign_squad","units":{},"squad":"{}"}}}}]"#,
                        json!(recruits),
                        squad.get()
                    )},
                    "requires": "configure_military",
                })
            });
            Some(json!({
                "alert": "hostile",
                "severity": if attacking { "critical" } else { "high" },
                "finding": if attacking {
                    format!("{} is attacking; {soldiers} dwarves are in a squad", hostile.label)
                } else {
                    format!("{} arrives at tick {arrives}; {soldiers} dwarves are in a squad", hostile.label)
                },
                "subject": hostile.id.get().to_string(),
                "remedy": remedy,
            }))
        })
        .collect()
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
                let mut row = json!({
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
                });
                if let Action::CreateWorkOrder { conditions, .. } = &step.action {
                    row["conditions"] = Json::Array(conditions.iter().map(work_order_condition_json).collect());
                }
                row
            })
            .collect(),
    )
}

/// Most active-work entries and units a briefing lists explicitly.
const MAX_BRIEFING_ITEMS: usize = 16;

fn text_field<'a>(entity: &'a EntityRecord, name: &str) -> Option<&'a str> {
    match entity.fields.get(name).and_then(Fact::known_value) {
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
                entity.fields.get(*name).map(|fact| {
                    (
                        (*name).to_owned(),
                        crate::observation_projection::fact_value(fact),
                    )
                })
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
                    name.starts_with(effects::LABOR_FIELD_PREFIX)
                        && fact.known_value() == Some(&Value::Bool(true))
                })
                .map(|(name, _)| &name[effects::LABOR_FIELD_PREFIX.len()..])
                .collect();
            json!({
                "entity_id": unit.id.get().to_string(),
                "name": unit.label,
                "profession": text_field(unit, "profession"),
                "enabled_labors": labors,
                "squad": unit.fields.get(effects::SQUAD_FIELD).map(crate::observation_projection::fact_value),
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

/// A page always names exactly one collection of a profiled observation.
#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ObservationSection {
    Entities,
    Relations,
    Chunks,
    Events,
}

impl ObservationSection {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Entities => "entities",
            Self::Relations => "relations",
            Self::Chunks => "chunks",
            Self::Events => "events",
        }
    }

    const fn domain(self) -> &'static str {
        match self {
            Self::Entities => "laboratory.entity_records",
            Self::Relations => "laboratory.relation_records",
            Self::Chunks => "laboratory.chunk_records",
            Self::Events => "laboratory.retained_event_records",
        }
    }
}

#[derive(Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
enum QuerySpec {
    Observation {
        completeness_profile: String,
        section: Option<ObservationSection>,
        limit: Option<usize>,
        offset: Option<usize>,
    },
    Entities {
        kind: Option<String>,
        limit: Option<usize>,
        offset: Option<usize>,
        /// Row filter: `{"field","op","value"}`, `{"all":[..]}`, `{"any":[..]}`
        /// or `{"not":{..}}`; unknown or absent facts never match.
        #[serde(rename = "where")]
        filter: Option<Json>,
    },
    /// Walkability route between two tiles (six-neighbour, observed dry
    /// floors and complementary stairs; no digging, ramps or hidden tiles).
    Path {
        from: [i32; 3],
        to: [i32; 3],
    },
    /// Ranked lexical search over entity labels, kinds and field values.
    Search {
        text: String,
        limit: Option<usize>,
    },
    Terrain {
        min: [i32; 3],
        max: [i32; 3],
    },
}

/// Answer the structured laboratory query modes. `raw` is either the bare
/// word `entities` or a closed JSON object naming one of the query modes.
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
            filter: None,
        }
    } else {
        serde_json::from_str(raw).map_err(|error| {
            invalid(format!(
                "query mode must be \"summary\", \"entities\", or a JSON object with mode entities|observation|terrain|search|path: {error}"
            ))
        })?
    };
    match spec {
        QuerySpec::Entities {
            kind,
            limit,
            offset,
            filter,
        } => {
            let predicate = filter
                .as_ref()
                .map(|raw| filter_predicate(raw, 0))
                .transpose()?;
            entities_page(
                snapshot,
                kind.as_deref(),
                limit,
                offset,
                predicate.as_ref(),
                filter.as_ref(),
            )
        }
        QuerySpec::Observation {
            completeness_profile,
            section,
            limit,
            offset,
        } => observation_page(snapshot, &completeness_profile, section, limit, offset),
        QuerySpec::Search { text, limit } => search(snapshot, &text, limit),
        QuerySpec::Path { from, to } => path(snapshot, from, to),
        QuerySpec::Terrain { min, max } => terrain(snapshot, coord(min), coord(max)),
    }
}

/// Most nested filter levels.
const MAX_FILTER_DEPTH: usize = 6;
/// Most children of one `all`/`any` filter.
const MAX_FILTER_CHILDREN: usize = 16;

fn filter_value(raw: &Json) -> Result<Value> {
    Ok(match raw {
        Json::Bool(value) => Value::Bool(*value),
        Json::String(text) if text.len() <= MAX_NAME_BYTES => Value::Text(text.clone()),
        Json::Number(number) => match (number.as_u64(), number.as_i64()) {
            (Some(value), _) => Value::U64(value),
            (None, Some(value)) => Value::I64(value),
            _ => return Err(invalid("filter numbers must be integers")),
        },
        _ => {
            return Err(invalid(
                "filter values are booleans, integers or short strings",
            ));
        }
    })
}

/// Compile a row filter into a predicate whose entity references stand for
/// the row (`EntityId::NIL`).
pub(crate) fn filter_predicate(raw: &Json, depth: usize) -> Result<Predicate> {
    if depth > MAX_FILTER_DEPTH {
        return Err(invalid("filter nesting exceeds its bound"));
    }
    let children = |items: &Json| -> Result<Vec<Predicate>> {
        let items = items
            .as_array()
            .filter(|items| !items.is_empty() && items.len() <= MAX_FILTER_CHILDREN)
            .ok_or_else(|| invalid(format!("all/any take 1..={MAX_FILTER_CHILDREN} filters")))?;
        items
            .iter()
            .map(|item| filter_predicate(item, depth + 1))
            .collect()
    };
    let object = raw
        .as_object()
        .ok_or_else(|| invalid("a filter is a JSON object"))?;
    match (object.get("all"), object.get("any"), object.get("not")) {
        (Some(items), None, None) if object.len() == 1 => {
            return Ok(Predicate::All(children(items)?));
        }
        (None, Some(items), None) if object.len() == 1 => {
            return Ok(Predicate::Any(children(items)?));
        }
        (None, None, Some(inner)) if object.len() == 1 => {
            return Ok(Predicate::Not(Box::new(filter_predicate(
                inner,
                depth + 1,
            )?)));
        }
        _ => {}
    }
    let field = object
        .get("field")
        .and_then(Json::as_str)
        .filter(|field| !field.is_empty() && field.len() <= MAX_NAME_BYTES)
        .ok_or_else(|| invalid("a comparison filter names a field"))?;
    let op = match object.get("op").and_then(Json::as_str).unwrap_or("eq") {
        "eq" => CompareOp::Eq,
        "ne" => CompareOp::Ne,
        "lt" => CompareOp::Lt,
        "le" => CompareOp::Le,
        "gt" => CompareOp::Gt,
        "ge" => CompareOp::Ge,
        other => {
            return Err(invalid(format!(
                "unknown filter op {other:?}; use eq|ne|lt|le|gt|ge"
            )));
        }
    };
    let value = filter_value(
        object
            .get("value")
            .ok_or_else(|| invalid("a comparison filter carries a value"))?,
    )?;
    if object
        .keys()
        .any(|key| !matches!(key.as_str(), "field" | "op" | "value"))
    {
        return Err(invalid("a comparison filter has only field, op and value"));
    }
    Ok(Predicate::FieldCompare {
        entity_id: EntityId::NIL,
        field: field.to_owned(),
        op,
        value,
    })
}

/// Largest x/y margin around a path query's endpoints.
const PATH_MARGIN: i32 = 8;
/// Most path points returned.
const MAX_PATH_POINTS: usize = 64;
/// Largest coordinate magnitude a path endpoint may name.
const MAX_PATH_COORDINATE: u32 = 1 << 20;

fn path(snapshot: &WorldSnapshot, from: [i32; 3], to: [i32; 3]) -> Result<Json> {
    use dfmcp_world::map_region::{Cell, MapRegion, Region, Shape, Tile};
    // The bounded route region is the endpoints' bounding box plus a margin; z gets
    // a one-level margin so a single-level route does not sit on the region boundary.
    if from
        .iter()
        .chain(&to)
        .any(|c| c.unsigned_abs() > MAX_PATH_COORDINATE)
    {
        return Err(invalid(format!(
            "path coordinates are bounded by +/-{MAX_PATH_COORDINATE}"
        )));
    }
    let margin = [PATH_MARGIN, PATH_MARGIN, 1];
    let origin: [i32; 3] = std::array::from_fn(|axis| from[axis].min(to[axis]) - margin[axis]);
    let mut size = [0_u32; 3];
    for axis in 0..3 {
        let extent = i64::from(from[axis].max(to[axis])) + i64::from(margin[axis])
            - i64::from(origin[axis])
            + 1;
        size[axis] = u32::try_from(extent)
            .map_err(|_| invalid("path endpoints are too far apart for one bounded route query"))?;
    }
    // Region coordinates are local: the lab map may use negative world coordinates.
    let region = Region {
        origin: [0, 0, 0],
        size,
    };
    let volume = region
        .volume()
        .map_err(|_| invalid("path endpoints are too far apart for one bounded route query"))?;
    let world = |local: [u32; 3]| -> [i32; 3] {
        std::array::from_fn(|axis| origin[axis].saturating_add_unsigned(local[axis]))
    };
    let mut cells = Vec::with_capacity(volume);
    for index in 0..volume {
        let Some(local) = region.position(index) else {
            return Err(invalid("path region indexing failed"));
        };
        let [x, y, z] = world(local);
        cells.push(match snapshot.tile_code_at(MapCoord::new(x, y, z)) {
            None => Cell::Hidden,
            Some(code) => {
                let shape = match code {
                    tile_codes::FLOOR => Shape::Floor,
                    tile_codes::SOLID_WALL | tile_codes::MAGMA_WALL => Shape::Wall,
                    tile_codes::OPEN_SPACE | tile_codes::CHASM => Shape::Empty,
                    tile_codes::STAIR => Shape::StairUpDown,
                    tile_codes::RAMP => Shape::Ramp,
                    _ => Shape::Other,
                };
                Cell::Visible(Tile {
                    native_tiletype: code,
                    shape,
                    liquid_depth: 0,
                    magma: code == tile_codes::MAGMA_WALL,
                    traffic: 0,
                    dig_designation: 0,
                    building_occupancy: 0,
                    unit_occupancy: 0,
                    walkable_region: u32::from(matches!(shape, Shape::Floor | Shape::StairUpDown)),
                    temperature_1: 0,
                    temperature_2: 0,
                })
            }
        });
    }
    let map = MapRegion { region, cells };
    let local =
        |c: [i32; 3]| -> [u32; 3] { std::array::from_fn(|axis| c[axis].abs_diff(origin[axis])) };
    let route = map
        .route(
            local(from),
            local(to),
            dfmcp_world::map_region::MAX_ROUTE_WORK,
        )
        .map_err(|error| invalid(format!("path query failed: {error:?}")))?;
    let reachable = !route.path.is_empty();
    // An unobserved tile can only open a route if some walkable tile could step into
    // it: horizontally always, vertically only through a stair-like shape.
    let borders_unobserved = (0..volume).any(|index| {
        let (Some(Cell::Visible(tile)), Some(position)) =
            (map.cells.get(index), region.position(index))
        else {
            return false;
        };
        if !map.candidate(index) {
            return false;
        }
        (0..3).any(|axis| {
            [false, true].into_iter().any(|increase| {
                if axis == 2
                    && !(if increase {
                        tile.shape.up()
                    } else {
                        tile.shape.down()
                    })
                {
                    return false;
                }
                let mut next = position;
                let moved = if increase {
                    next[axis].checked_add(1)
                } else {
                    next[axis].checked_sub(1)
                };
                moved.is_some_and(|value| {
                    next[axis] = value;
                    region
                        .index(next)
                        .is_some_and(|n| matches!(map.cells.get(n), Some(Cell::Hidden)))
                })
            })
        })
    });
    let certified = reachable
        || route.endpoint_excluded
        || !(route.touched_region_boundary || borders_unobserved);
    Ok(json!({
        "mode": "path",
        "from": from,
        "to": to,
        "reachable": reachable,
        "endpoint_not_walkable": route.endpoint_excluded,
        "path_length": route.path.len().saturating_sub(1),
        "path": route.path.iter().take(MAX_PATH_POINTS).map(|p| world(*p)).collect::<Vec<_>>(),
        "path_truncated": route.path.len() > MAX_PATH_POINTS,
        "visited_tiles": route.visited_tiles,
        "policy": dfmcp_world::map_region::ROUTE_POLICY,
        "region": {"origin": origin, "size": size},
        "touched_region_boundary": route.touched_region_boundary,
        "borders_unobserved_tiles": borders_unobserved,
        "epistemic_state": if certified { "certified_derived" } else { "unknown" },
        "note": if certified {
            "derived from observed terrain under the stated walking policy"
        } else {
            "no route inside the bounded, observed region; a route outside it or through unobserved tiles is not ruled out"
        },
    }))
}

fn search(snapshot: &WorldSnapshot, text: &str, limit: Option<usize>) -> Result<Json> {
    let limit = limit.unwrap_or(10);
    if limit == 0
        || limit > MAX_ENTITY_PAGE
        || text.trim().is_empty()
        || text.len() > MAX_NAME_BYTES
    {
        return Err(invalid(format!(
            "search needs non-empty text of at most {MAX_NAME_BYTES} bytes and a limit of 1..={MAX_ENTITY_PAGE}"
        )));
    }
    let mut engine = dfmcp_world::FrankenSearchEngine::new();
    engine.index_snapshot(snapshot)?;
    let hits = engine.search(text, limit)?;
    Ok(json!({
        "mode": "search",
        "text": text,
        "returned": hits.len(),
        "ranking": "lexical (BM25-style) over labels, kinds and field values; ties by identity",
        "hits": hits.iter().map(|hit| json!({
            "entity_id": hit.entity_id.map(|id| id.get().to_string()),
            "event_id": hit.event_id.map(|id| id.get().to_string()),
            "title": hit.title,
            "snippet": hit.snippet,
            "score_micros": hit.score_micros,
        })).collect::<Vec<_>>(),
    }))
}

fn entities_page(
    snapshot: &WorldSnapshot,
    kind: Option<&str>,
    limit: Option<usize>,
    offset: Option<usize>,
    filter: Option<&Predicate>,
    raw_filter: Option<&Json>,
) -> Result<Json> {
    let limit = limit.unwrap_or(25);
    if limit == 0 || limit > MAX_ENTITY_PAGE {
        return Err(invalid(format!(
            "entity page limit must be 1..={MAX_ENTITY_PAGE}"
        )));
    }
    let offset = offset.unwrap_or(0);
    let mut matching = Vec::new();
    let mut unknown_filter_rows = 0usize;
    for entity in snapshot
        .graph
        .entities
        .values()
        .filter(|entity| kind.is_none_or(|kind| entity.kind.as_str() == kind))
    {
        let truth = filter.map_or(dfmcp_world::PredicateTruth::True, |predicate| {
            dfmcp_world::evaluate_truth_for(snapshot, entity.id, predicate)
        });
        match truth {
            dfmcp_world::PredicateTruth::True => matching.push(entity),
            dfmcp_world::PredicateTruth::False => {}
            dfmcp_world::PredicateTruth::Unknown => unknown_filter_rows += 1,
        }
    }
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
        .map(|entity| crate::observation_projection::entity_json(entity))
        .collect();
    let end = offset + rows.len();
    let page_complete = offset == 0 && end == matching.len();
    let complete_domain = page_complete && unknown_filter_rows == 0;
    let domain = json!({"domain": "laboratory.entities", "kind": kind, "where": raw_filter});
    let mut query = json!({"mode": "entities", "limit": limit, "offset": offset,
        "at": snapshot.state_hash.to_hex()});
    if let Some(kind) = kind {
        query["kind"] = json!(kind);
    }
    if let Some(filter) = raw_filter {
        query["where"] = filter.clone();
    }
    let continuation = (end < matching.len()).then(|| {
        let mut next = query.clone();
        next["offset"] = json!(end);
        next
    });
    Ok(json!({
        "mode": "entities",
        "kind": kind,
        "filtered": filter.is_some(),
        "total": matching.len(),
        "offset": offset,
        "returned": rows.len(),
        "next_offset": (end < matching.len()).then_some(end),
        "truncated": !page_complete,
        "source_complete": true,
        "unknown_filter_rows": unknown_filter_rows,
        "complete_domain": complete_domain,
        "absence_proven": matching.is_empty() && complete_domain,
        "query": query,
        "continuation": continuation,
        "observation_coverage": {
            "status": if complete_domain { "complete_for_named_projection" } else { "partial" },
            "complete_domains": if complete_domain { vec![domain.clone()] } else { vec![] },
            "partial_domains": if complete_domain { vec![] } else { vec![domain.clone()] },
            "source_complete": true,
            "page_complete": page_complete,
            "unknown_filter_rows": unknown_filter_rows,
            "absence_proof_scope": if complete_domain { vec![domain] } else { vec![] },
            "continuation": continuation,
        },
        "rows": rows,
    }))
}

/// Render a bounded page from an immutable typed profile. The canonical
/// envelope and the source snapshot have separate hashes. Only the source
/// hash is a retained history address and may appear in a continuation.
fn observation_page(
    snapshot: &WorldSnapshot,
    profile: &str,
    section: Option<ObservationSection>,
    limit: Option<usize>,
    offset: Option<usize>,
) -> Result<Json> {
    use crate::observation_projection::{
        anchor_json, entity_json, fact_value, presence_json, value_presence,
    };
    use dfmcp_world::{CompletenessProfile, ProfiledSnapshot, ProjectionProvenance};

    let profile = CompletenessProfile::parse(profile)?;
    let section = section.unwrap_or(ObservationSection::Entities);
    let limit = limit.unwrap_or(25);
    if limit == 0 || limit > MAX_ENTITY_PAGE {
        return Err(invalid(format!(
            "observation page limit must be 1..={MAX_ENTITY_PAGE}"
        )));
    }
    let offset = offset.unwrap_or(0);
    let provenance = ProjectionProvenance {
        source_schema: "dfmcp-world-snapshot-v1".to_owned(),
        source_manifest: dfmcp_core::Digest32::of_bytes(
            b"dfmcp.lab-observation-source/1;schema=dfmcp-world-snapshot-v1;engine=dfmcp_intent::effects;scope=process-local-reference",
        ),
    };
    let projected = ProfiledSnapshot::project(snapshot, profile, provenance, BTreeMap::new())?;
    let graph = &projected.snapshot().graph;
    let included = match section {
        ObservationSection::Entities | ObservationSection::Relations => true,
        ObservationSection::Chunks => profile.includes_map_chunks(),
        ObservationSection::Events => profile.includes_events(),
    };
    let (total, source_total) = match section {
        ObservationSection::Entities => (graph.entities.len(), snapshot.graph.entities.len()),
        ObservationSection::Relations => (graph.edges.len(), snapshot.graph.edges.len()),
        ObservationSection::Chunks => (graph.chunks.len(), snapshot.graph.chunks.len()),
        ObservationSection::Events => (graph.events.len(), snapshot.graph.events.len()),
    };
    if offset > total {
        return Err(DfmcpError::new(
            ErrorCode::CursorGap,
            "observation page offset is beyond the selected profile section",
        ));
    }
    let rows: Vec<Json> = match section {
        ObservationSection::Entities => graph.entities.values().skip(offset).take(limit)
            .map(entity_json).collect(),
        ObservationSection::Relations => graph.edges.values().skip(offset).take(limit).map(|edge| {
            json!({
                "edge_id": edge.id.to_string(), "revision": edge.revision, "kind": edge.kind.as_str(),
                "from": edge.from.to_string(), "to": edge.to.to_string(),
                "fields": edge.fields.iter().map(|(name, fact)| (name.clone(), fact_value(fact)))
                    .collect::<BTreeMap<_, _>>(),
                "field_presence": edge.fields.iter().map(|(name, fact)| (name.clone(), presence_json(fact)))
                    .collect::<BTreeMap<_, _>>(),
            })
        }).collect(),
        ObservationSection::Chunks => graph.chunks.values().skip(offset).take(limit).map(|chunk| {
            json!({
                "coord": [chunk.coord.x, chunk.coord.y, chunk.coord.z],
                "revision": chunk.revision, "width": chunk.width, "height": chunk.height,
                "chunk_hash": chunk.compute_hash().to_hex(),
                "terrain_runs": chunk.terrain_runs.iter().map(|run| {
                    json!({"tile_code": run.tile_code, "length": run.length})
                }).collect::<Vec<_>>(),
                "sparse_overlays": chunk.sparse_overlays.iter().map(|(at, fields)| (at.to_string(),
                    json!({
                        "fields": fields.iter().map(|(name, value)| (name.clone(), value_json(value)))
                            .collect::<BTreeMap<_, _>>(),
                        "field_presence": fields.iter().map(|(name, value)| (name.clone(), value_presence(value)))
                            .collect::<BTreeMap<_, _>>(),
                    })
                )).collect::<BTreeMap<_, _>>(),
            })
        }).collect(),
        ObservationSection::Events => graph.events.values().skip(offset).take(limit).map(|event| {
            json!({
                "event_id": event.id.to_string(), "tick": event.tick.0, "kind": event.kind.as_str(),
                "subject": event.subject.map(|id| id.to_string()), "summary": event.summary,
                "fields": event.fields.iter().map(|(name, value)| (name.clone(), value_json(value)))
                    .collect::<BTreeMap<_, _>>(),
                "field_presence": event.fields.iter().map(|(name, value)| (name.clone(), value_presence(value)))
                    .collect::<BTreeMap<_, _>>(),
            })
        }).collect(),
    };
    let end = offset + rows.len();
    let page_complete = offset == 0 && end == total;
    let complete_domain = included && page_complete;
    let domain = json!({
        "domain": section.domain(), "scope": "record_membership",
        "completeness_profile": profile.as_str(), "section": section.as_str(),
    });
    let query = json!({
        "mode": "observation", "completeness_profile": profile.as_str(), "section": section.as_str(),
        "limit": limit, "offset": offset, "at": snapshot.state_hash.to_hex(),
    });
    let continuation = (included && end < total).then(|| {
        let mut next = query.clone();
        next["offset"] = json!(end);
        next
    });
    let omitted_domains = if included {
        vec![]
    } else {
        vec![
            json!({"domain": section.domain(), "reason": "excluded_by_completeness_profile",
            "completeness_profile": profile.as_str()}),
        ]
    };
    Ok(json!({
        "mode": "observation", "completeness_profile": profile.as_str(), "section": section.as_str(),
        "anchor": anchor_json(&projected.source_anchor()),
        "source_anchor": anchor_json(&projected.source_anchor()),
        "projected_anchor": anchor_json(&projected.snapshot().anchor()),
        "profile_digest": projected.digest().to_hex(),
        "provenance": {
            "source_schema": projected.provenance().source_schema,
            "source_manifest": projected.provenance().source_manifest.to_hex(),
            "source": "process_local_laboratory_snapshot",
        },
        "rendering": {
            "format": "bounded_json_records", "canonical_envelope": false,
            "binary_payloads": "length_summary_with_explicit_omission",
            "field_knowledge": "consult_each_field_presence",
            "action_authority": "a profile does not authorize an action",
        },
        "included_sections": {
            "entities": true, "relations": true,
            "chunks": profile.includes_map_chunks(), "events": profile.includes_events(),
        },
        "section_included": included, "source_total": source_total, "total": total,
        "offset": offset, "returned": rows.len(), "next_offset": (included && end < total).then_some(end),
        "source_complete": included, "complete_domain": complete_domain,
        "absence_proven": included && total == 0 && page_complete,
        "truncated": !complete_domain,
        "query": query, "continuation": continuation, "rows": rows,
        "observation_coverage": {
            "status": if complete_domain { "complete_for_named_projection" } else { "partial" },
            "anchor": anchor_json(&projected.source_anchor()),
            "complete_domains": if complete_domain { vec![domain.clone()] } else { vec![] },
            "partial_domains": if !complete_domain && included { vec![domain.clone()] } else { vec![] },
            "omitted_domains": omitted_domains, "page_complete": included && page_complete,
            "absence_proof_scope": if complete_domain { vec![domain] } else { vec![] },
            "continuation": continuation,
        },
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
#[path = "production_prerequisite_tests.rs"]
mod production_prerequisite_tests;

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
        assert_eq!(snapshot.graph.entities.len(), 13);
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
    fn conditional_work_orders_parse_and_render_every_typed_condition() -> Result<()> {
        let conditions = json!([
            {"kind":"item_count_below","item_token":"DRINK","threshold":50},
            {"kind":"material_available","material_token":"PLANT","minimum":2},
            {"kind":"completed_order","order_name":"first batch"},
        ]);
        let request = json!([{"action":{
            "kind":"create_work_order","name":"conditional","job_token":"BREW_DRINK",
            "amount":2,"conditions":conditions,
        }}]);
        let steps = parse_steps(&request.to_string())?;
        let Action::CreateWorkOrder {
            conditions: parsed, ..
        } = &steps[0].action
        else {
            return Err(invalid("wrong parsed action"));
        };
        assert_eq!(
            parsed
                .iter()
                .map(work_order_condition_json)
                .collect::<Vec<_>>(),
            conditions
                .as_array()
                .cloned()
                .ok_or_else(|| invalid("missing test conditions"))?
        );
        let empty = parse_steps(
            r#"[{"action":{"kind":"create_work_order","name":"legacy request","job_token":"MAKE_TEST","amount":1}}]"#,
        )?;
        assert!(
            matches!(&empty[0].action, Action::CreateWorkOrder { conditions, .. } if conditions.is_empty())
        );
        Ok(())
    }

    #[test]
    fn condition_requests_reject_unknown_shapes_and_excessive_bounds() {
        let mut bad = vec![
            json!([{"kind":"unknown","item_token":"DRINK","threshold":1}]),
            json!([{"kind":"item_count_below","item_token":"DRINK","threshold":1,"extra":true}]),
            json!([{"kind":"item_count_below","item_token":"DRINK","threshold":-1}]),
            json!([{"kind":"material_available","material_token":"WOOD","minimum":u64::from(u32::MAX)+1}]),
            json!([{"kind":"completed_order","order_name":""}]),
            json!([{"kind":"completed_order","order_name":"x".repeat(MAX_NAME_BYTES+1)}]),
        ];
        bad.push(Json::Array(vec![
            json!({"kind":"completed_order","order_name":"x"});
            effects::MAX_WORK_ORDER_CONDITIONS + 1
        ]));
        for conditions in bad {
            let request = json!([{"action":{
                "kind":"create_work_order","name":"bad","job_token":"MAKE_TEST",
                "amount":1,"conditions":conditions,
            }}]);
            assert!(parse_steps(&request.to_string()).is_err(), "{request}");
        }
    }

    #[test]
    fn production_objectives_preserve_stock_thresholds_in_executable_actions() -> Result<()> {
        let snapshot = scenario_snapshot("starter_fortress", FortressId::new(3), false)?;
        let compiled = ProductionRequest::parse(
            r#"{"template":"production","quotas":[{"item":"DRINK","minimum":46},{"item":"FOOD","minimum":66}]}"#,
        )?.compile(&snapshot)?;
        assert_eq!(compiled.analysis["feasible"], true);
        let steps = parse_steps(&compiled.actions)?;
        assert_eq!(steps.len(), 2);
        for step in &steps {
            let Action::CreateWorkOrder {
                job_token,
                amount,
                conditions,
                ..
            } = &step.action
            else {
                return Err(invalid("wrong production action"));
            };
            assert_eq!(*amount, 2);
            let (item, threshold) = if job_token == "BREW_DRINK" {
                ("DRINK", 46)
            } else {
                ("FOOD", 66)
            };
            assert_eq!(
                conditions,
                &vec![WorkOrderCondition::ItemCountBelow {
                    item_token: item.to_owned(),
                    threshold,
                }]
            );
        }
        Ok(())
    }

    #[test]
    fn production_objectives_require_current_source_qualified_stock_counts() -> Result<()> {
        let original = scenario_snapshot("starter_fortress", FortressId::new(3), false)?;
        let request = r#"{"template":"production","quotas":[{"item":"DRINK","minimum":50}]}"#;
        let fact = original.graph.entities[&starter::STOCK_LEDGER].fields
            [effects::STOCK_DRINK_FIELD]
            .clone();
        let mut variants = vec![None];
        for source in [
            FactSource::Replay,
            FactSource::AgentAssertion("unproved stock".to_owned()),
            FactSource::Derived("unregistered forecast".to_owned()),
        ] {
            let mut changed = fact.clone();
            changed.source = source;
            variants.push(Some(changed));
        }
        for presence in [
            dfmcp_world::FactPresence::Absent,
            dfmcp_world::FactPresence::Unknown("not captured".to_owned()),
            dfmcp_world::FactPresence::Omitted("not in profile".to_owned()),
            dfmcp_world::FactPresence::Stale(original.anchor()),
            dfmcp_world::FactPresence::Known(Value::U64(0)),
        ] {
            let mut changed = fact.clone();
            changed.presence = Some(presence);
            variants.push(Some(changed));
        }
        let mut future = fact.clone();
        future.observed_at = GameTick(u64::MAX);
        variants.push(Some(future));
        for replacement in variants {
            let mut snapshot = original.clone();
            let ledger = snapshot
                .graph
                .entities
                .get_mut(&starter::STOCK_LEDGER)
                .ok_or_else(|| invalid("missing fixture ledger"))?;
            match replacement {
                Some(fact) => {
                    ledger
                        .fields
                        .insert(effects::STOCK_DRINK_FIELD.to_owned(), fact);
                }
                None => {
                    ledger.fields.remove(effects::STOCK_DRINK_FIELD);
                }
            }
            snapshot.refresh_hash();
            assert!(
                ProductionRequest::parse(request)?
                    .compile(&snapshot)
                    .is_err_and(|e| e.code == ErrorCode::PreconditionsFailed)
            );
        }
        let mut no_ledger = original.clone();
        no_ledger.graph.entities.remove(&starter::STOCK_LEDGER);
        no_ledger.refresh_hash();
        assert!(
            ProductionRequest::parse(request)?
                .compile(&no_ledger)
                .is_err_and(|e| e.code == ErrorCode::PreconditionsFailed)
        );
        assert!(
            ProductionRequest::parse(request)?
                .compile(&original)
                .is_ok()
        );
        Ok(())
    }

    #[test]
    fn production_requests_normalize_complete_quotas_before_lowering() -> Result<()> {
        let first = ProductionRequest::parse(
            r#"{"template":"production","quotas":[{"item":"FOOD","minimum":70},{"item":"DRINK","minimum":40},{"item":"FOOD","minimum":65}]}"#,
        )?;
        let second = ProductionRequest::parse(
            r#"{"quotas":[{"minimum":40,"item":"DRINK"},{"minimum":70,"item":"FOOD"}],"template":"production"}"#,
        )?;
        assert_eq!(first, second);
        assert_eq!(first.canonical_json(), second.canonical_json());
        assert_eq!(ProductionRequest::parse(&first.canonical_json())?, first);
        let world = scenario_snapshot("starter_fortress", FortressId::new(314), false)?;
        let compiled = first.compile(&world)?;
        let steps = parse_steps(&compiled.actions)?;
        assert_eq!(steps.len(), 1, "the drink quota already holds");
        assert!(matches!(
            &steps[0].action,
            Action::CreateWorkOrder { job_token, amount: 2, .. } if job_token == "PREPARE_MEAL"
        ));
        assert_eq!(
            compiled.terminal,
            Predicate::All(vec![
                Predicate::FieldCompare {
                    entity_id: starter::STOCK_LEDGER,
                    field: effects::STOCK_DRINK_FIELD.to_owned(),
                    op: CompareOp::Ge,
                    value: Value::U64(40),
                },
                Predicate::FieldCompare {
                    entity_id: starter::STOCK_LEDGER,
                    field: effects::STOCK_FOOD_FIELD.to_owned(),
                    op: CompareOp::Ge,
                    value: Value::U64(70),
                },
            ])
            .normalized(),
        );
        Ok(())
    }

    #[test]
    fn production_goal_retains_initially_satisfied_stock_and_unknowns() -> Result<()> {
        let mut world = scenario_snapshot("starter_fortress", FortressId::new(315), false)?;
        let request = ProductionRequest::parse(
            r#"{"template":"production","quotas":[{"item":"DRINK","minimum":40},{"item":"FOOD","minimum":65}]}"#,
        )?;
        let terminal = request.compile(&world)?.terminal;
        let ledger = world
            .graph
            .entities
            .get_mut(&starter::STOCK_LEDGER)
            .ok_or_else(|| invalid("fixture ledger missing"))?;
        ledger.fields.insert(
            effects::STOCK_FOOD_FIELD.to_owned(),
            lab_fact(Value::U64(65)),
        );
        ledger.fields.insert(
            effects::STOCK_DRINK_FIELD.to_owned(),
            lab_fact(Value::U64(39)),
        );
        world.refresh_hash();
        assert_eq!(
            dfmcp_world::PredicateEvidence::laboratory(&world)?.evaluate(&terminal)?,
            dfmcp_world::PredicateTruth::False
        );
        let ledger = world
            .graph
            .entities
            .get_mut(&starter::STOCK_LEDGER)
            .ok_or_else(|| invalid("fixture ledger missing"))?;
        let mut unknown = lab_fact(Value::U64(40));
        unknown.source = FactSource::AgentAssertion("assumed stock".to_owned());
        ledger
            .fields
            .insert(effects::STOCK_DRINK_FIELD.to_owned(), unknown);
        world.refresh_hash();
        assert_eq!(
            dfmcp_world::PredicateEvidence::laboratory(&world)?.evaluate(&terminal)?,
            dfmcp_world::PredicateTruth::Unknown
        );
        assert!(
            request
                .compile(&world)
                .is_err_and(|error| error.code == ErrorCode::PreconditionsFailed)
        );
        Ok(())
    }

    #[test]
    fn production_request_rejects_bad_templates_unknown_zero_quotas_and_input_overflow() {
        for raw in [
            r#"{"template":"blueprint","quotas":[{"item":"DRINK","minimum":50}]}"#,
            r#"{"template":"production","quotas":[]}"#,
            r#"{"template":"production","quotas":[{"item":"STEEL","minimum":0},{"item":"DRINK","minimum":50}]}"#,
            r#"{"template":"production","quotas":[{"item":"DRINK","minimum":4294967296}]}"#,
            r#"{"template":"production","quotas":[{"item":"DRINK","minimum":50,"discard":true}]}"#,
        ] {
            assert!(ProductionRequest::parse(raw).is_err(), "{raw}");
        }
        let maximum = json!({"template":"production",
            "quotas": vec![json!({"item":"DRINK","minimum":50}); MAX_LIST_ITEMS]});
        assert!(ProductionRequest::parse(&maximum.to_string()).is_ok());
        let over = json!({"template":"production",
            "quotas": vec![json!({"item":"DRINK","minimum":50}); MAX_LIST_ITEMS + 1]});
        assert!(ProductionRequest::parse(&over.to_string()).is_err());
        assert!(ProductionRequest::parse(&" ".repeat(MAX_ACTIONS_JSON_BYTES + 1)).is_err());
    }

    #[test]
    fn production_goal_and_order_proof_diverge_on_actual_consumption_timeline() -> Result<()> {
        for (start, elapsed, quotas, expected_drink, expected_food) in [
            (1100, 200, json!([{"item":"DRINK","minimum":60}]), 53, 60),
            (
                1151,
                50,
                json!([{"item":"DRINK","minimum":40},{"item":"FOOD","minimum":65}]),
                33,
                65,
            ),
        ] {
            let mut snapshot = scenario_snapshot("starter_fortress", FortressId::new(316), false)?;
            let to_start = start - snapshot.tick.0;
            snapshot.tick = GameTick(start);
            effects::advance_effects(&mut snapshot, to_start)?;
            snapshot.refresh_hash();
            let request = ProductionRequest::parse(
                &json!({"template":"production","quotas":quotas}).to_string(),
            )?;
            let compiled = request.compile(&snapshot)?;
            let steps = parse_steps(&compiled.actions)?;
            assert_eq!(steps.len(), 1);
            let key = "original-production-timeline";
            let action_proof = Predicate::All(effects::default_postconditions(
                &steps[0].action,
                key,
                snapshot.fortress_id,
            ))
            .normalized();
            effects::apply_effect(&mut snapshot, &steps[0].action, key)?;
            snapshot.tick = GameTick(start + elapsed);
            effects::advance_effects(&mut snapshot, elapsed)?;
            snapshot.refresh_hash();
            let evidence = dfmcp_world::PredicateEvidence::laboratory(&snapshot)?;
            assert_eq!(
                evidence.evaluate(&action_proof)?,
                dfmcp_world::PredicateTruth::True
            );
            assert_eq!(
                evidence.evaluate(&compiled.terminal)?,
                dfmcp_world::PredicateTruth::False
            );
            let ledger = snapshot
                .graph
                .entities
                .get(&starter::STOCK_LEDGER)
                .ok_or_else(|| invalid("fixture ledger missing"))?;
            for (field, expected) in [
                (effects::STOCK_DRINK_FIELD, expected_drink),
                (effects::STOCK_FOOD_FIELD, expected_food),
            ] {
                assert_eq!(
                    ledger.fields.get(field).and_then(Fact::known_value),
                    Some(&Value::U64(expected))
                );
            }
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

#[cfg(test)]
mod observation_coverage_tests {
    use super::*;
    use dfmcp_world::FactPresence;

    #[test]
    fn entity_pages_keep_exact_filter_and_anchor_without_claiming_complete_results() -> Result<()> {
        let snapshot = scenario_snapshot("starter_fortress", FortressId::new(71), true)?;
        let page = query(
            &snapshot,
            r#"{"mode":"entities","kind":"unit","limit":2,"where":{"field":"alive","value":true}}"#,
        )?;
        assert_eq!(page["returned"], 2);
        assert_eq!(page["total"], 7);
        assert_eq!(page["complete_domain"], false);
        assert_eq!(page["absence_proven"], false);
        assert_eq!(page["observation_coverage"]["status"], "partial");
        let mut next = page["continuation"].clone();
        assert_eq!(next["at"], snapshot.state_hash.to_hex());
        assert_eq!(next["offset"], 2);
        assert_eq!(next["where"], json!({"field":"alive","value":true}));
        // The server resolves and strips the historical anchor before routing.
        if let Some(object) = next.as_object_mut() {
            object.remove("at");
        }
        let second = query(&snapshot, &next.to_string())?;
        assert_eq!(second["rows"][0]["entity_id"], "1003");
        assert_eq!(second["complete_domain"], false);
        Ok(())
    }

    #[test]
    fn unresolved_filters_cannot_certify_empty_results_even_under_negation() -> Result<()> {
        let snapshot = scenario_snapshot("starter_fortress", FortressId::new(72), true)?;
        let unknown = query(
            &snapshot,
            r#"{"mode":"entities","kind":"unit","where":{"not":{"field":"future.unknown","value":true}}}"#,
        )?;
        assert_eq!(unknown["total"], 0);
        assert_eq!(unknown["unknown_filter_rows"], 7);
        assert_eq!(unknown["complete_domain"], false);
        assert_eq!(unknown["absence_proven"], false);
        assert_eq!(
            unknown["observation_coverage"]["absence_proof_scope"],
            json!([])
        );
        let absent = query(
            &snapshot,
            r#"{"mode":"entities","kind":"unit","where":{"field":"alive","value":false}}"#,
        )?;
        assert_eq!(absent["total"], 0);
        assert_eq!(absent["unknown_filter_rows"], 0);
        assert_eq!(absent["complete_domain"], true);
        assert_eq!(absent["absence_proven"], true);
        Ok(())
    }

    #[test]
    fn entity_and_briefing_views_do_not_reveal_unavailable_retained_values() -> Result<()> {
        let mut snapshot = scenario_snapshot("starter_fortress", FortressId::new(73), true)?;
        let Some(unit) = snapshot
            .graph
            .entities
            .get_mut(&EntityId::new(starter::FIRST_DWARF))
        else {
            return Err(invalid("fixture unit missing"));
        };
        let mut redacted = Fact::with_presence(
            FactPresence::Redacted("test policy".into()),
            snapshot.tick,
            FactSource::Replay,
            dfmcp_core::Digest32::ZERO,
        );
        redacted.value = Value::Text("retained hidden assignment".into());
        unit.fields.insert("squad".into(), redacted.clone());
        unit.fields.insert("profession".into(), redacted);
        snapshot.refresh_hash();
        let page = query(&snapshot, r#"{"mode":"entities","kind":"unit","limit":1}"#)?;
        assert!(page["rows"][0]["fields"]["squad"].is_null());
        assert_eq!(
            page["rows"][0]["field_presence"]["squad"]["state"],
            "redacted"
        );
        assert!(!page.to_string().contains("retained hidden assignment"));
        assert!(
            !briefing(&snapshot)
                .to_string()
                .contains("retained hidden assignment")
        );
        Ok(())
    }
}

#[cfg(test)]
mod profiled_observation_query_tests {
    use super::*;
    use dfmcp_core::{EdgeId, EventId};
    use dfmcp_world::{CompletenessProfile, EdgeKind, EdgeRecord, WorldEvent, WorldEventKind};

    fn fixture() -> Result<WorldSnapshot> {
        let mut snapshot = scenario_snapshot("starter_fortress", FortressId::new(81), true)?;
        let binary = Value::List(vec![Value::Null, Value::Bytes(vec![1, 2, 3, 4])]);
        snapshot.graph.edges.insert(
            EdgeId::new(1),
            EdgeRecord {
                id: EdgeId::new(1),
                revision: 1,
                kind: EdgeKind::AssignedTo,
                from: EntityId::new(starter::FIRST_DWARF),
                to: starter::STILL,
                fields: BTreeMap::from([("future.binary".into(), lab_fact(binary.clone()))]),
            },
        );
        snapshot.graph.events.insert(
            EventId::new(1),
            WorldEvent {
                id: EventId::new(1),
                tick: snapshot.tick,
                kind: WorldEventKind::AdapterNotice,
                subject: Some(starter::STILL),
                summary: "reference observation".into(),
                fields: BTreeMap::from([
                    ("future.binary".into(), binary.clone()),
                    ("known_null".into(), Value::Null),
                ]),
            },
        );
        let Some(chunk) = snapshot.graph.chunks.values_mut().next() else {
            return Err(invalid("fixture chunk missing"));
        };
        chunk
            .sparse_overlays
            .insert(0, BTreeMap::from([("future.binary".into(), binary)]));
        snapshot.refresh_hash();
        Ok(snapshot)
    }

    fn request(
        snapshot: &WorldSnapshot,
        profile: &str,
        section: &str,
        limit: usize,
    ) -> Result<Json> {
        query(
            snapshot,
            &json!({"mode":"observation", "completeness_profile":profile,
            "section":section, "limit":limit})
            .to_string(),
        )
    }

    #[test]
    fn all_five_profiles_keep_the_source_anchor_separate_from_the_projection() -> Result<()> {
        let snapshot = fixture()?;
        for profile in CompletenessProfile::ALL {
            let page = request(&snapshot, profile.as_str(), "entities", 2)?;
            assert_eq!(
                page["source_anchor"]["state_hash"],
                snapshot.state_hash.to_hex()
            );
            assert_eq!(page["anchor"], page["source_anchor"]);
            assert_eq!(page["continuation"]["at"], snapshot.state_hash.to_hex());
            assert_eq!(
                page["continuation"]["completeness_profile"],
                profile.as_str()
            );
            assert_eq!(page["continuation"]["section"], "entities");
            assert_eq!(
                page["rows"][0]["entity_id"],
                starter::FIRST_DWARF.to_string()
            );
            assert_eq!(page["returned"], 2);
            assert_eq!(page["complete_domain"], false);
            assert_eq!(page["rendering"]["canonical_envelope"], false);
            if profile == CompletenessProfile::ResearchFull {
                assert_eq!(page["projected_anchor"], page["source_anchor"]);
            } else {
                assert_ne!(
                    page["projected_anchor"]["state_hash"],
                    page["source_anchor"]["state_hash"]
                );
            }
        }
        let historical = request(&snapshot, "historical", "entities", 2)?;
        assert!(historical["rows"][0]["fields"]["profession"].is_null());
        assert_eq!(
            historical["rows"][0]["field_presence"]["profession"]["state"],
            "omitted"
        );
        assert_eq!(
            historical["rows"][0]["field_presence"]["profession"]["projection"],
            "historical"
        );
        Ok(())
    }

    #[test]
    fn continuations_keep_the_profile_and_section_and_resume_at_the_next_record() -> Result<()> {
        let snapshot = fixture()?;
        let first = request(&snapshot, "spatial", "chunks", 2)?;
        let mut continuation = first["continuation"].clone();
        assert_eq!(continuation["offset"], 2);
        assert_eq!(continuation["at"], snapshot.state_hash.to_hex());
        assert_eq!(continuation["completeness_profile"], "spatial");
        assert_eq!(continuation["section"], "chunks");
        let Some(object) = continuation.as_object_mut() else {
            return Err(invalid("expected continuation"));
        };
        object.remove("at"); // Resolved by the server's existing retained-history router.
        let second = query(&snapshot, &continuation.to_string())?;
        let Some(expected) = snapshot.graph.chunks.values().nth(2) else {
            return Err(invalid("fixture third chunk missing"));
        };
        assert_eq!(
            second["rows"][0]["coord"],
            json!([expected.coord.x, expected.coord.y, expected.coord.z])
        );
        assert_ne!(second["rows"][0]["coord"], first["rows"][1]["coord"]);
        assert_eq!(second["complete_domain"], false);
        Ok(())
    }

    #[test]
    fn excluded_sections_do_not_turn_present_records_into_proven_absence() -> Result<()> {
        let snapshot = fixture()?;
        for (profile, section) in [
            ("operations", "chunks"),
            ("control-minimum", "events"),
            ("spatial", "events"),
            ("historical", "chunks"),
        ] {
            let page = request(&snapshot, profile, section, 100)?;
            assert_eq!(page["section_included"], false);
            assert!(page["source_total"].as_u64().is_some_and(|total| total > 0));
            assert_eq!(page["total"], 0);
            assert_eq!(page["rows"], json!([]));
            assert_eq!(page["complete_domain"], false);
            assert_eq!(page["absence_proven"], false);
            assert_eq!(page["observation_coverage"]["status"], "partial");
            assert_eq!(
                page["observation_coverage"]["absence_proof_scope"],
                json!([])
            );
            assert_eq!(
                page["observation_coverage"]["omitted_domains"][0]["reason"],
                "excluded_by_completeness_profile"
            );
        }
        let empty = scenario_snapshot("empty", FortressId::new(82), true)?;
        let included = request(&empty, "research-full", "events", 100)?;
        assert_eq!(included["absence_proven"], true);
        let omitted = request(&empty, "control-minimum", "events", 100)?;
        assert_eq!(omitted["absence_proven"], false);
        Ok(())
    }

    #[test]
    fn relations_chunks_and_events_preserve_structure_and_mark_binary_omission() -> Result<()> {
        let snapshot = fixture()?;
        let relations = request(&snapshot, "research-full", "relations", 100)?;
        assert_eq!(
            relations["rows"][0]["from"],
            starter::FIRST_DWARF.to_string()
        );
        assert_eq!(relations["rows"][0]["to"], starter::STILL.to_string());
        assert_eq!(
            relations["rows"][0]["fields"]["future.binary"],
            json!([null, {"bytes":4}])
        );
        assert_eq!(
            relations["rows"][0]["field_presence"]["future.binary"]["state"],
            "omitted"
        );
        let chunks = request(&snapshot, "spatial", "chunks", 100)?;
        assert_eq!(chunks["rows"][0]["terrain_runs"][0]["length"], 256);
        assert_eq!(
            chunks["rows"][0]["sparse_overlays"]["0"]["fields"]["future.binary"],
            json!([null, {"bytes":4}])
        );
        assert_eq!(
            chunks["rows"][0]["sparse_overlays"]["0"]["field_presence"]["future.binary"]["state"],
            "omitted"
        );
        let events = request(&snapshot, "historical", "events", 100)?;
        assert_eq!(
            events["rows"][0]["field_presence"]["future.binary"]["state"],
            "omitted"
        );
        assert!(events["rows"][0]["fields"].get("known_null").is_some());
        assert_eq!(
            events["rows"][0]["field_presence"]["known_null"]["state"],
            "known"
        );
        assert_eq!(events["complete_domain"], true);
        assert_eq!(
            events["observation_coverage"]["complete_domains"][0]["scope"],
            "record_membership"
        );
        Ok(())
    }

    #[test]
    fn observation_requests_refuse_unknown_profiles_sections_and_invalid_pages() -> Result<()> {
        let snapshot = fixture()?;
        for raw in [
            r#"{"mode":"observation","completeness_profile":"everything"}"#,
            r#"{"mode":"observation","completeness_profile":"research-full","section":"memory"}"#,
            r#"{"mode":"observation","completeness_profile":"spatial","limit":0}"#,
            r#"{"mode":"observation","completeness_profile":"spatial","limit":101}"#,
            r#"{"mode":"observation","completeness_profile":"historical","offset":1000}"#,
        ] {
            assert!(query(&snapshot, raw).is_err(), "{raw}");
        }
        Ok(())
    }
}
