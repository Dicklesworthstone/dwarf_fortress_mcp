use std::collections::{BTreeMap, BTreeSet};

use dfmcp_core::{Capability, EntityId, MapCoord, MapCuboid, RiskTier};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DigMode {
    Mine,
    Channel,
    UpStair,
    DownStair,
    UpDownStair,
    Ramp,
    RemoveConstruction,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BuildingKind {
    Workshop(String),
    Furnace(String),
    Furniture(String),
    Construction(String),
    Trap(String),
    FarmPlot,
    Bridge,
    Well,
    Custom(String),
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct MaterialSelector {
    pub required_tokens: BTreeSet<String>,
    pub forbidden_tokens: BTreeSet<String>,
    pub prefer_nearest: bool,
    pub reserve_count: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkOrderCondition {
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Pause {
        paused: bool,
    },
    DesignateDig {
        area: MapCuboid,
        mode: DigMode,
    },
    Build {
        kind: BuildingKind,
        location: MapCoord,
        footprint: MapCuboid,
        material: MaterialSelector,
    },
    SetLabor {
        units: Vec<EntityId>,
        labor: String,
        enabled: bool,
    },
    CreateWorkOrder {
        name: String,
        job_token: String,
        amount: u32,
        conditions: Vec<WorkOrderCondition>,
    },
    ConfigureStockpile {
        stockpile: EntityId,
        accepts: BTreeSet<String>,
        max_bins: Option<u32>,
        max_barrels: Option<u32>,
        max_wheelbarrows: Option<u32>,
    },
    AssignSquad {
        units: Vec<EntityId>,
        squad: EntityId,
    },
    SetBurrowMembership {
        units: Vec<EntityId>,
        burrow: EntityId,
        assigned: bool,
    },
    SetStandingOrder {
        key: String,
        value: String,
    },
    Extension {
        namespace: String,
        name: String,
        parameters: BTreeMap<String, String>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct ActionScope {
    pub entity_ids: Vec<EntityId>,
    pub map_area: Option<MapCuboid>,
}

impl Action {
    #[must_use]
    pub const fn risk(&self) -> RiskTier {
        match self {
            Self::Pause { .. }
            | Self::SetLabor { .. }
            | Self::CreateWorkOrder { .. }
            | Self::ConfigureStockpile { .. }
            | Self::SetBurrowMembership { .. } => RiskTier::Reversible,
            Self::DesignateDig { .. }
            | Self::Build { .. }
            | Self::AssignSquad { .. }
            | Self::SetStandingOrder { .. }
            | Self::Extension { .. } => RiskTier::Guarded,
        }
    }

    #[must_use]
    pub const fn capability(&self) -> Capability {
        match self {
            Self::Pause { .. } => Capability::ControlClock,
            Self::DesignateDig { .. } => Capability::Designate,
            Self::Build { .. } => Capability::Construct,
            Self::SetLabor { .. } => Capability::ConfigureLabor,
            Self::CreateWorkOrder { .. } => Capability::ConfigureProduction,
            Self::ConfigureStockpile { .. } => Capability::ConfigureLogistics,
            Self::AssignSquad { .. } => Capability::ConfigureMilitary,
            Self::SetBurrowMembership { .. } => Capability::ConfigureLogistics,
            Self::SetStandingOrder { .. } => Capability::ConfigureLogistics,
            Self::Extension { .. } => Capability::Extension,
        }
    }

    #[must_use]
    pub fn scope(&self) -> ActionScope {
        match self {
            Self::Pause { .. }
            | Self::CreateWorkOrder { .. }
            | Self::SetStandingOrder { .. }
            | Self::Extension { .. } => ActionScope::default(),
            Self::DesignateDig { area, .. } => ActionScope {
                entity_ids: Vec::new(),
                map_area: Some(*area),
            },
            Self::Build {
                location: _,
                footprint,
                ..
            } => ActionScope {
                entity_ids: Vec::new(),
                map_area: Some(*footprint),
            },
            Self::SetLabor { units, .. } => ActionScope {
                entity_ids: units.clone(),
                map_area: None,
            },
            Self::ConfigureStockpile { stockpile, .. } => ActionScope {
                entity_ids: vec![*stockpile],
                map_area: None,
            },
            Self::AssignSquad { units, squad } => {
                let mut entity_ids = units.clone();
                entity_ids.push(*squad);
                entity_ids.sort_unstable();
                entity_ids.dedup();
                ActionScope {
                    entity_ids,
                    map_area: None,
                }
            }
            Self::SetBurrowMembership { units, burrow, .. } => {
                let mut entity_ids = units.clone();
                entity_ids.push(*burrow);
                entity_ids.sort_unstable();
                entity_ids.dedup();
                ActionScope {
                    entity_ids,
                    map_area: None,
                }
            }
        }
    }

    #[must_use]
    pub const fn naturally_temporal(&self) -> bool {
        matches!(
            self,
            Self::DesignateDig { .. } | Self::Build { .. } | Self::CreateWorkOrder { .. }
        )
    }

    #[must_use]
    pub fn normalized(&self) -> Self {
        match self {
            Self::SetLabor {
                units,
                labor,
                enabled,
            } => Self::SetLabor {
                units: normalized_ids(units),
                labor: labor.clone(),
                enabled: *enabled,
            },
            Self::AssignSquad { units, squad } => Self::AssignSquad {
                units: normalized_ids(units),
                squad: *squad,
            },
            Self::SetBurrowMembership {
                units,
                burrow,
                assigned,
            } => Self::SetBurrowMembership {
                units: normalized_ids(units),
                burrow: *burrow,
                assigned: *assigned,
            },
            Self::CreateWorkOrder {
                name,
                job_token,
                amount,
                conditions,
            } => {
                let mut conditions = conditions.clone();
                conditions.sort_by_key(condition_key);
                Self::CreateWorkOrder {
                    name: name.clone(),
                    job_token: job_token.clone(),
                    amount: *amount,
                    conditions,
                }
            }
            other => other.clone(),
        }
    }

    pub(crate) fn encode(&self, output: &mut Vec<u8>) {
        match self {
            Self::Pause { paused } => {
                output.push(0);
                output.push(u8::from(*paused));
            }
            Self::DesignateDig { area, mode } => {
                output.push(1);
                put_cuboid(output, *area);
                output.push(match mode {
                    DigMode::Mine => 0,
                    DigMode::Channel => 1,
                    DigMode::UpStair => 2,
                    DigMode::DownStair => 3,
                    DigMode::UpDownStair => 4,
                    DigMode::Ramp => 5,
                    DigMode::RemoveConstruction => 6,
                });
            }
            Self::Build {
                kind,
                location,
                footprint,
                material,
            } => {
                output.push(2);
                put_building_kind(output, kind);
                put_coord(output, *location);
                put_cuboid(output, *footprint);
                put_string_set(output, &material.required_tokens);
                put_string_set(output, &material.forbidden_tokens);
                output.push(u8::from(material.prefer_nearest));
                put_u32(output, material.reserve_count);
            }
            Self::SetLabor {
                units,
                labor,
                enabled,
            } => {
                output.push(3);
                put_ids(output, units);
                put_str(output, labor);
                output.push(u8::from(*enabled));
            }
            Self::CreateWorkOrder {
                name,
                job_token,
                amount,
                conditions,
            } => {
                output.push(4);
                put_str(output, name);
                put_str(output, job_token);
                put_u32(output, *amount);
                put_u64(output, conditions.len() as u64);
                for condition in conditions {
                    put_condition(output, condition);
                }
            }
            Self::ConfigureStockpile {
                stockpile,
                accepts,
                max_bins,
                max_barrels,
                max_wheelbarrows,
            } => {
                output.push(5);
                put_u64(output, stockpile.get());
                put_string_set(output, accepts);
                put_option_u32(output, *max_bins);
                put_option_u32(output, *max_barrels);
                put_option_u32(output, *max_wheelbarrows);
            }
            Self::AssignSquad { units, squad } => {
                output.push(6);
                put_ids(output, units);
                put_u64(output, squad.get());
            }
            Self::SetBurrowMembership {
                units,
                burrow,
                assigned,
            } => {
                output.push(7);
                put_ids(output, units);
                put_u64(output, burrow.get());
                output.push(u8::from(*assigned));
            }
            Self::SetStandingOrder { key, value } => {
                output.push(8);
                put_str(output, key);
                put_str(output, value);
            }
            Self::Extension {
                namespace,
                name,
                parameters,
            } => {
                output.push(9);
                put_str(output, namespace);
                put_str(output, name);
                put_u64(output, parameters.len() as u64);
                for (key, value) in parameters {
                    put_str(output, key);
                    put_str(output, value);
                }
            }
        }
    }
}

fn normalized_ids(ids: &[EntityId]) -> Vec<EntityId> {
    let mut output = ids.to_vec();
    output.sort_unstable();
    output.dedup();
    output
}

fn condition_key(condition: &WorkOrderCondition) -> String {
    match condition {
        WorkOrderCondition::ItemCountBelow {
            item_token,
            threshold,
        } => format!("0:{item_token}:{threshold:010}"),
        WorkOrderCondition::MaterialAvailable {
            material_token,
            minimum,
        } => format!("1:{material_token}:{minimum:010}"),
        WorkOrderCondition::CompletedOrder { order_name } => format!("2:{order_name}"),
    }
}

fn put_condition(output: &mut Vec<u8>, condition: &WorkOrderCondition) {
    match condition {
        WorkOrderCondition::ItemCountBelow {
            item_token,
            threshold,
        } => {
            output.push(0);
            put_str(output, item_token);
            put_u32(output, *threshold);
        }
        WorkOrderCondition::MaterialAvailable {
            material_token,
            minimum,
        } => {
            output.push(1);
            put_str(output, material_token);
            put_u32(output, *minimum);
        }
        WorkOrderCondition::CompletedOrder { order_name } => {
            output.push(2);
            put_str(output, order_name);
        }
    }
}

fn put_building_kind(output: &mut Vec<u8>, kind: &BuildingKind) {
    match kind {
        BuildingKind::Workshop(value) => {
            output.push(0);
            put_str(output, value);
        }
        BuildingKind::Furnace(value) => {
            output.push(1);
            put_str(output, value);
        }
        BuildingKind::Furniture(value) => {
            output.push(2);
            put_str(output, value);
        }
        BuildingKind::Construction(value) => {
            output.push(3);
            put_str(output, value);
        }
        BuildingKind::Trap(value) => {
            output.push(4);
            put_str(output, value);
        }
        BuildingKind::FarmPlot => output.push(5),
        BuildingKind::Bridge => output.push(6),
        BuildingKind::Well => output.push(7),
        BuildingKind::Custom(value) => {
            output.push(8);
            put_str(output, value);
        }
    }
}

fn put_ids(output: &mut Vec<u8>, ids: &[EntityId]) {
    let ids = normalized_ids(ids);
    put_u64(output, ids.len() as u64);
    for id in ids {
        put_u64(output, id.get());
    }
}

fn put_coord(output: &mut Vec<u8>, coord: MapCoord) {
    output.extend_from_slice(&coord.x.to_be_bytes());
    output.extend_from_slice(&coord.y.to_be_bytes());
    output.extend_from_slice(&coord.z.to_be_bytes());
}

fn put_cuboid(output: &mut Vec<u8>, cuboid: MapCuboid) {
    put_coord(output, cuboid.min);
    put_coord(output, cuboid.max);
}

fn put_string_set(output: &mut Vec<u8>, values: &BTreeSet<String>) {
    put_u64(output, values.len() as u64);
    for value in values {
        put_str(output, value);
    }
}

fn put_option_u32(output: &mut Vec<u8>, value: Option<u32>) {
    match value {
        Some(value) => {
            output.push(1);
            put_u32(output, value);
        }
        None => output.push(0),
    }
}

fn put_u32(output: &mut Vec<u8>, value: u32) {
    output.extend_from_slice(&value.to_be_bytes());
}

fn put_u64(output: &mut Vec<u8>, value: u64) {
    output.extend_from_slice(&value.to_be_bytes());
}

fn put_str(output: &mut Vec<u8>, value: &str) {
    put_u64(output, value.len() as u64);
    output.extend_from_slice(value.as_bytes());
}
