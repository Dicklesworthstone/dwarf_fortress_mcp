#![forbid(unsafe_code)]

//! Automated Spatial Blueprint and Defensive Fortress Layout Planner.
//!
//! WP-PLN-01: Compiles high-level architectural goals (bedrooms, dining halls,
//! workshop hubs, defense perimeters) into concrete excavation and construction plans
//! with environmental hazard detection (aquifers, magma, cave-ins).

use dfmcp_core::{
    DfmcpError, ErrorCode, IntentId, MapCoord, MapCuboid, Result, RiskTier, StateAnchor,
};
use dfmcp_world::Predicate;
use dfmcp_world::TileType;
use dfmcp_world::spatial_index::{ChunkSpatialIndex, TemperatureBand};

use crate::action::{Action, DigMode};
use crate::plan::{Constraint, Intent, RequestedAction};

/// Room blueprint archetype templates.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BlueprintTemplate {
    BedroomCluster {
        rooms_count: u32,
        room_size: (u8, u8), // (width, height), e.g. (3, 3)
    },
    DiningHall {
        width: u8,
        height: u8,
    },
    WorkshopHub {
        bays_count: u32,
    },
    StockpileVault {
        width: u8,
        height: u8,
        category: String,
    },
    DefensiveMoat {
        perimeter_cuboid: MapCuboid,
        drawbridge_span: u8,
    },
}

/// Preflight environmental hazard scanner result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HazardAssessment {
    Safe,
    MagmaProximity { hazard_coord: MapCoord },
    UnsupportedCaveInRisk { span_width: u8 },
}

/// Automated Architectural Blueprint and Room Expansion Planner.
#[derive(Clone, Debug, Default)]
pub struct BlueprintPlanner;

impl BlueprintPlanner {
    /// Preflight check a planned excavation area against spatial index for environmental hazards.
    #[must_use]
    pub fn assess_hazards(
        &self,
        area: &MapCuboid,
        spatial_index: &ChunkSpatialIndex,
    ) -> HazardAssessment {
        // 1. Check for magma proximity (1-tile halo around cuboid)
        let Ok(halo) = MapCuboid::new(
            MapCoord {
                x: area.min.x - 1,
                y: area.min.y - 1,
                z: area.min.z - 1,
            },
            MapCoord {
                x: area.max.x + 1,
                y: area.max.y + 1,
                z: area.max.z + 1,
            },
        ) else {
            return HazardAssessment::Safe;
        };

        let tiles = spatial_index.find_cuboid(&halo);
        for (coord, props) in tiles {
            if props.tile_type == TileType::MagmaWall
                || props.temperature == TemperatureBand::MagmaHot
            {
                return HazardAssessment::MagmaProximity {
                    hazard_coord: coord,
                };
            }
        }

        // 2. Cave-in check: unsupported spans > 7 tiles in width and length
        let width = (area.max.x - area.min.x + 1).unsigned_abs() as u8;
        let height = (area.max.y - area.min.y + 1).unsigned_abs() as u8;

        if width > 7 && height > 7 {
            return HazardAssessment::UnsupportedCaveInRisk { span_width: width };
        }

        HazardAssessment::Safe
    }

    /// Compile a high-level blueprint template into a concrete, validated `Intent`.
    pub fn compile_blueprint_intent(
        &self,
        intent_id: IntentId,
        anchor: StateAnchor,
        origin: MapCoord,
        template: BlueprintTemplate,
        spatial_index: &ChunkSpatialIndex,
    ) -> Result<Intent> {
        match template {
            BlueprintTemplate::BedroomCluster {
                rooms_count,
                room_size,
            } => {
                let rooms_per_row = 4u32;
                let mut requested_actions = Vec::new();

                for i in 0..rooms_count {
                    let row = i / rooms_per_row;
                    let col = i % rooms_per_row;

                    let room_origin = MapCoord {
                        x: origin.x + (col as i32) * (room_size.0 as i32 + 1),
                        y: origin.y + (row as i32) * (room_size.1 as i32 + 1),
                        z: origin.z,
                    };

                    let room_cuboid = MapCuboid::new(
                        room_origin,
                        MapCoord {
                            x: room_origin.x + (room_size.0 as i32 - 1),
                            y: room_origin.y + (room_size.1 as i32 - 1),
                            z: room_origin.z,
                        },
                    )?;

                    match self.assess_hazards(&room_cuboid, spatial_index) {
                        HazardAssessment::Safe => {}
                        HazardAssessment::MagmaProximity { hazard_coord } => {
                            return Err(DfmcpError::new(
                                ErrorCode::InvalidRequest,
                                format!(
                                    "bedroom plan rejected: magma hazard detected at {:?}",
                                    hazard_coord
                                ),
                            ));
                        }
                        HazardAssessment::UnsupportedCaveInRisk { span_width } => {
                            return Err(DfmcpError::new(
                                ErrorCode::InvalidRequest,
                                format!(
                                    "bedroom plan rejected: cave-in span {} exceeds safe limit",
                                    span_width
                                ),
                            ));
                        }
                    }

                    requested_actions.push(RequestedAction {
                        action: Action::DesignateDig {
                            area: room_cuboid,
                            mode: DigMode::Mine,
                        },
                        preconditions: Vec::new(),
                        postconditions: Vec::new(),
                        compensation: None,
                        obligation: None,
                        depends_on: Vec::new(),
                    });
                }

                Ok(Intent {
                    id: intent_id,
                    anchor,
                    summary: format!("excavate {} bedroom units", rooms_count),
                    terminal_condition: Predicate::Paused(true),
                    constraints: vec![Constraint::MaxRisk(RiskTier::Guarded)],
                    requested_actions,
                })
            }
            BlueprintTemplate::DiningHall { width, height } => {
                let cuboid = MapCuboid::new(
                    origin,
                    MapCoord {
                        x: origin.x + (width as i32 - 1),
                        y: origin.y + (height as i32 - 1),
                        z: origin.z,
                    },
                )?;

                match self.assess_hazards(&cuboid, spatial_index) {
                    HazardAssessment::Safe => {}
                    HazardAssessment::MagmaProximity { hazard_coord } => {
                        return Err(DfmcpError::new(
                            ErrorCode::InvalidRequest,
                            format!(
                                "dining hall plan rejected: magma hazard at {:?}",
                                hazard_coord
                            ),
                        ));
                    }
                    HazardAssessment::UnsupportedCaveInRisk { span_width } => {
                        return Err(DfmcpError::new(
                            ErrorCode::InvalidRequest,
                            format!("dining hall plan rejected: unsupported span {}", span_width),
                        ));
                    }
                }

                Ok(Intent {
                    id: intent_id,
                    anchor,
                    summary: format!("excavate grand dining hall ({}x{})", width, height),
                    terminal_condition: Predicate::Paused(true),
                    constraints: vec![Constraint::MaxRisk(RiskTier::Guarded)],
                    requested_actions: vec![RequestedAction {
                        action: Action::DesignateDig {
                            area: cuboid,
                            mode: DigMode::Mine,
                        },
                        preconditions: Vec::new(),
                        postconditions: Vec::new(),
                        compensation: None,
                        obligation: None,
                        depends_on: Vec::new(),
                    }],
                })
            }
            BlueprintTemplate::WorkshopHub { bays_count } => {
                let mut requested_actions = Vec::new();
                for i in 0..bays_count {
                    let bay_origin = MapCoord {
                        x: origin.x + (i as i32) * 6,
                        y: origin.y,
                        z: origin.z,
                    };
                    let bay_cuboid = MapCuboid::new(
                        bay_origin,
                        MapCoord {
                            x: bay_origin.x + 4,
                            y: bay_origin.y + 4,
                            z: bay_origin.z,
                        },
                    )?;

                    requested_actions.push(RequestedAction {
                        action: Action::DesignateDig {
                            area: bay_cuboid,
                            mode: DigMode::Mine,
                        },
                        preconditions: Vec::new(),
                        postconditions: Vec::new(),
                        compensation: None,
                        obligation: None,
                        depends_on: Vec::new(),
                    });
                }

                Ok(Intent {
                    id: intent_id,
                    anchor,
                    summary: format!("excavate {} workshop bays", bays_count),
                    terminal_condition: Predicate::Paused(true),
                    constraints: vec![Constraint::MaxRisk(RiskTier::Guarded)],
                    requested_actions,
                })
            }
            BlueprintTemplate::StockpileVault {
                width,
                height,
                category,
            } => {
                let cuboid = MapCuboid::new(
                    origin,
                    MapCoord {
                        x: origin.x + (width as i32 - 1),
                        y: origin.y + (height as i32 - 1),
                        z: origin.z,
                    },
                )?;

                Ok(Intent {
                    id: intent_id,
                    anchor,
                    summary: format!("stockpile vault ({})", category),
                    terminal_condition: Predicate::Paused(true),
                    constraints: vec![Constraint::MaxRisk(RiskTier::Guarded)],
                    requested_actions: vec![RequestedAction {
                        action: Action::DesignateDig {
                            area: cuboid,
                            mode: DigMode::Mine,
                        },
                        preconditions: Vec::new(),
                        postconditions: Vec::new(),
                        compensation: None,
                        obligation: None,
                        depends_on: Vec::new(),
                    }],
                })
            }
            BlueprintTemplate::DefensiveMoat {
                perimeter_cuboid, ..
            } => Ok(Intent {
                id: intent_id,
                anchor,
                summary: "excavate defensive moat".to_owned(),
                terminal_condition: Predicate::Paused(true),
                constraints: vec![Constraint::MaxRisk(RiskTier::Guarded)],
                requested_actions: vec![RequestedAction {
                    action: Action::DesignateDig {
                        area: perimeter_cuboid,
                        mode: DigMode::Channel,
                    },
                    preconditions: Vec::new(),
                    postconditions: Vec::new(),
                    compensation: None,
                    obligation: None,
                    depends_on: Vec::new(),
                }],
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dfmcp_core::{Digest32, FortressId, GameTick, ObservationCursor};
    use dfmcp_world::{ChunkCoord, MapChunk, TerrainRun};
    use std::collections::BTreeMap;

    fn empty_spatial_index() -> ChunkSpatialIndex {
        let mut index = ChunkSpatialIndex::new();
        let chunk = MapChunk {
            coord: ChunkCoord { x: 0, y: 0, z: 100 },
            revision: 1,
            width: 16,
            height: 16,
            terrain_runs: vec![TerrainRun {
                tile_code: 2, // SolidWall
                length: 256,
            }],
            sparse_overlays: BTreeMap::new(),
        };
        index.insert_or_update_chunk(&chunk);
        index
    }

    #[test]
    fn test_bedroom_cluster_compilation() -> Result<()> {
        let planner = BlueprintPlanner;
        let index = empty_spatial_index();
        let anchor = StateAnchor {
            fortress_id: FortressId::new(1),
            cursor: ObservationCursor::ORIGIN,
            tick: GameTick(100),
            state_hash: Digest32::ZERO,
        };

        let intent = planner.compile_blueprint_intent(
            IntentId::new(1),
            anchor,
            MapCoord { x: 0, y: 0, z: 100 },
            BlueprintTemplate::BedroomCluster {
                rooms_count: 4,
                room_size: (3, 3),
            },
            &index,
        )?;

        assert_eq!(intent.requested_actions.len(), 4);
        assert_eq!(intent.summary, "excavate 4 bedroom units");

        Ok(())
    }

    #[test]
    fn test_cave_in_rejection_for_huge_dining_hall() {
        let planner = BlueprintPlanner;
        let index = empty_spatial_index();
        let anchor = StateAnchor {
            fortress_id: FortressId::new(1),
            cursor: ObservationCursor::ORIGIN,
            tick: GameTick(100),
            state_hash: Digest32::ZERO,
        };

        // 12x12 dining hall without columns triggers cave in risk (> 7x7 span)
        let result = planner.compile_blueprint_intent(
            IntentId::new(2),
            anchor,
            MapCoord { x: 0, y: 0, z: 100 },
            BlueprintTemplate::DiningHall {
                width: 12,
                height: 12,
            },
            &index,
        );

        assert!(result.is_err());
    }
}
