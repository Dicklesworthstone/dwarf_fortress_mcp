#![forbid(unsafe_code)]

//! Conservative spatial-blueprint intent compiler.
//!
//! The compiler bounds coordinate arithmetic and requires complete hazard coverage.
//! Terrain-completion predicates are not yet represented in the world query model, so
//! generated temporal predicates are deliberately `False`: they can never manufacture
//! success from an unrelated pause state.

use dfmcp_core::{
    DfmcpError, ErrorCode, GameTick, IntentId, MapCoord, MapCuboid, Result, RiskTier, StateAnchor,
};
use dfmcp_world::Predicate;
use dfmcp_world::TileType;
use dfmcp_world::spatial_index::{ChunkSpatialIndex, TemperatureBand};

use crate::action::{Action, DigMode};
use crate::plan::{Constraint, Intent, ObligationSpec, RequestedAction};

/// Room blueprint archetype templates.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BlueprintTemplate {
    BedroomCluster {
        rooms_count: u32,
        room_size: (u8, u8),
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
    UnsupportedCaveInRisk { span_width: u64 },
    IncompleteKnowledge { reason: String },
}

/// Automated architectural blueprint planner.
#[derive(Clone, Debug, Default)]
pub struct BlueprintPlanner;

impl BlueprintPlanner {
    /// Check a bounded excavation area against a complete one-tile hazard halo.
    #[must_use]
    pub fn assess_hazards(
        &self,
        area: &MapCuboid,
        spatial_index: &ChunkSpatialIndex,
    ) -> HazardAssessment {
        let Some(halo_min) = checked_coord_offset(area.min, -1, -1, -1) else {
            return HazardAssessment::IncompleteKnowledge {
                reason: "hazard halo crosses the coordinate boundary".to_owned(),
            };
        };
        let Some(halo_max) = checked_coord_offset(area.max, 1, 1, 1) else {
            return HazardAssessment::IncompleteKnowledge {
                reason: "hazard halo crosses the coordinate boundary".to_owned(),
            };
        };
        let Ok(halo) = MapCuboid::new(halo_min, halo_max) else {
            return HazardAssessment::IncompleteKnowledge {
                reason: "hazard halo is not a valid cuboid".to_owned(),
            };
        };
        let Some(expected_tiles) = halo.tile_count() else {
            return HazardAssessment::IncompleteKnowledge {
                reason: "hazard halo tile count overflow".to_owned(),
            };
        };
        let tiles = match spatial_index.find_cuboid(&halo) {
            Ok(tiles) => tiles,
            Err(error) => {
                return HazardAssessment::IncompleteKnowledge {
                    reason: error.message,
                };
            }
        };
        if u64::try_from(tiles.len()).ok() != Some(expected_tiles) {
            return HazardAssessment::IncompleteKnowledge {
                reason: format!(
                    "hazard scan observed {} of {expected_tiles} required halo tiles",
                    tiles.len()
                ),
            };
        }
        for (coord, properties) in tiles {
            if properties.tile_type == TileType::MagmaWall
                || properties.temperature == TemperatureBand::MagmaHot
            {
                return HazardAssessment::MagmaProximity {
                    hazard_coord: coord,
                };
            }
        }

        let width = inclusive_span(area.min.x, area.max.x);
        let height = inclusive_span(area.min.y, area.max.y);
        if width > 7 && height > 7 {
            return HazardAssessment::UnsupportedCaveInRisk { span_width: width };
        }
        HazardAssessment::Safe
    }

    /// Compile a high-level blueprint into a bounded, fail-closed intent preview.
    pub fn compile_blueprint_intent(
        &self,
        intent_id: IntentId,
        anchor: StateAnchor,
        origin: MapCoord,
        template: BlueprintTemplate,
        spatial_index: &ChunkSpatialIndex,
    ) -> Result<Intent> {
        let deadline = GameTick(anchor.tick.0.checked_add(1_000).ok_or_else(|| {
            DfmcpError::new(
                ErrorCode::BudgetExceeded,
                "blueprint obligation deadline overflows the game tick range",
            )
        })?);

        let (summary, requested_actions) = match template {
            BlueprintTemplate::BedroomCluster {
                rooms_count,
                room_size,
            } => {
                if rooms_count == 0 {
                    return Err(invalid("bedroom count must be nonzero"));
                }
                validate_dimensions(room_size.0, room_size.1)?;
                let mut actions = Vec::new();
                for index in 0..rooms_count {
                    let row = index / 4;
                    let column = index % 4;
                    let x_offset = i64::from(column) * (i64::from(room_size.0) + 1);
                    let y_offset = i64::from(row) * (i64::from(room_size.1) + 1);
                    let room_origin = checked_coord_offset(origin, x_offset, y_offset, 0)
                        .ok_or_else(|| invalid("bedroom coordinate arithmetic overflow"))?;
                    let area = rectangle(room_origin, room_size.0, room_size.1)?;
                    self.require_safe("bedroom", &area, spatial_index)?;
                    actions.push(dig_request(area, DigMode::Mine, deadline));
                }
                (format!("excavate {rooms_count} bedroom units"), actions)
            }
            BlueprintTemplate::DiningHall { width, height } => {
                validate_dimensions(width, height)?;
                let area = rectangle(origin, width, height)?;
                self.require_safe("dining hall", &area, spatial_index)?;
                (
                    format!("excavate grand dining hall ({width}x{height})"),
                    vec![dig_request(area, DigMode::Mine, deadline)],
                )
            }
            BlueprintTemplate::WorkshopHub { bays_count } => {
                if bays_count == 0 {
                    return Err(invalid("workshop bay count must be nonzero"));
                }
                let mut actions = Vec::new();
                for index in 0..bays_count {
                    let x_offset = i64::from(index) * 6;
                    let bay_origin = checked_coord_offset(origin, x_offset, 0, 0)
                        .ok_or_else(|| invalid("workshop coordinate arithmetic overflow"))?;
                    let area = rectangle(bay_origin, 5, 5)?;
                    self.require_safe("workshop", &area, spatial_index)?;
                    actions.push(dig_request(area, DigMode::Mine, deadline));
                }
                (format!("excavate {bays_count} workshop bays"), actions)
            }
            BlueprintTemplate::StockpileVault {
                width,
                height,
                category,
            } => {
                validate_dimensions(width, height)?;
                if category.trim().is_empty() {
                    return Err(invalid("stockpile category must be nonempty"));
                }
                let area = rectangle(origin, width, height)?;
                self.require_safe("stockpile vault", &area, spatial_index)?;
                (
                    format!("stockpile vault ({category})"),
                    vec![dig_request(area, DigMode::Mine, deadline)],
                )
            }
            BlueprintTemplate::DefensiveMoat {
                perimeter_cuboid,
                drawbridge_span,
            } => {
                if drawbridge_span != 0 {
                    return Err(DfmcpError::new(
                        ErrorCode::CompatibilityUnknown,
                        "drawbridge gaps are not representable by the current single-cuboid moat action",
                    ));
                }
                self.require_safe("defensive moat", &perimeter_cuboid, spatial_index)?;
                (
                    "excavate defensive moat".to_owned(),
                    vec![dig_request(perimeter_cuboid, DigMode::Channel, deadline)],
                )
            }
        };

        Ok(Intent {
            id: intent_id,
            anchor,
            summary,
            // The world model cannot yet prove excavation completion.
            terminal_condition: Predicate::False,
            constraints: vec![Constraint::MaxRisk(RiskTier::Guarded)],
            requested_actions,
        })
    }

    fn require_safe(
        &self,
        label: &str,
        area: &MapCuboid,
        spatial_index: &ChunkSpatialIndex,
    ) -> Result<()> {
        match self.assess_hazards(area, spatial_index) {
            HazardAssessment::Safe => Ok(()),
            HazardAssessment::MagmaProximity { hazard_coord } => Err(invalid(format!(
                "{label} plan rejected: magma hazard at {hazard_coord:?}"
            ))),
            HazardAssessment::UnsupportedCaveInRisk { span_width } => Err(invalid(format!(
                "{label} plan rejected: unsupported span {span_width}"
            ))),
            HazardAssessment::IncompleteKnowledge { reason } => Err(DfmcpError::new(
                ErrorCode::PreconditionsFailed,
                format!("{label} hazard assessment is incomplete: {reason}"),
            )),
        }
    }
}

fn invalid(message: impl Into<String>) -> DfmcpError {
    DfmcpError::new(ErrorCode::InvalidRequest, message)
}

fn validate_dimensions(width: u8, height: u8) -> Result<()> {
    if width == 0 || height == 0 {
        return Err(invalid("blueprint width and height must be nonzero"));
    }
    Ok(())
}

fn rectangle(origin: MapCoord, width: u8, height: u8) -> Result<MapCuboid> {
    validate_dimensions(width, height)?;
    let max = checked_coord_offset(origin, i64::from(width) - 1, i64::from(height) - 1, 0)
        .ok_or_else(|| invalid("blueprint rectangle coordinate overflow"))?;
    MapCuboid::new(origin, max)
}

fn checked_coord_offset(origin: MapCoord, dx: i64, dy: i64, dz: i64) -> Option<MapCoord> {
    Some(MapCoord {
        x: i32::try_from(i64::from(origin.x).checked_add(dx)?).ok()?,
        y: i32::try_from(i64::from(origin.y).checked_add(dy)?).ok()?,
        z: i32::try_from(i64::from(origin.z).checked_add(dz)?).ok()?,
    })
}

fn inclusive_span(minimum: i32, maximum: i32) -> u64 {
    let low = i64::from(minimum.min(maximum));
    let high = i64::from(minimum.max(maximum));
    // Any pair of i32 coordinates spans at most 2^32 points, so this
    // conversion is infallible on every Rust target that supports u64.
    match u64::try_from(high - low + 1) {
        Ok(span) => span,
        Err(_) => u64::MAX,
    }
}

fn dig_request(area: MapCuboid, mode: DigMode, deadline_tick: GameTick) -> RequestedAction {
    RequestedAction {
        action: Action::DesignateDig { area, mode },
        preconditions: Vec::new(),
        postconditions: vec![Predicate::True],
        compensation: None,
        obligation: Some(ObligationSpec {
            terminal: Predicate::False,
            failure: None,
            deadline_tick,
            poll_interval_ticks: 10,
            stable_for_observations: 1,
        }),
        depends_on: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dfmcp_core::{Digest32, FortressId, ObservationCursor};
    use dfmcp_world::{ChunkCoord, MapChunk, TerrainRun};
    use std::collections::BTreeMap;

    fn covered_spatial_index() -> Result<ChunkSpatialIndex> {
        let mut index = ChunkSpatialIndex::new();
        for z in 99..=101 {
            for y in -1..=1 {
                for x in -1..=1 {
                    index.insert_or_update_chunk(&MapChunk {
                        coord: ChunkCoord { x, y, z },
                        revision: 1,
                        width: 16,
                        height: 16,
                        terrain_runs: vec![TerrainRun {
                            tile_code: 2,
                            length: 256,
                        }],
                        sparse_overlays: BTreeMap::new(),
                    })?;
                }
            }
        }
        Ok(index)
    }

    fn anchor() -> StateAnchor {
        StateAnchor {
            fortress_id: FortressId::new(1),
            cursor: ObservationCursor::ORIGIN,
            tick: GameTick(100),
            state_hash: Digest32::ZERO,
        }
    }

    #[test]
    fn bedroom_cluster_compilation_is_bounded_and_fail_closed() -> Result<()> {
        let planner = BlueprintPlanner;
        let intent = planner.compile_blueprint_intent(
            IntentId::new(1),
            anchor(),
            MapCoord { x: 0, y: 0, z: 100 },
            BlueprintTemplate::BedroomCluster {
                rooms_count: 4,
                room_size: (3, 3),
            },
            &covered_spatial_index()?,
        )?;

        assert_eq!(intent.requested_actions.len(), 4);
        assert_eq!(intent.summary, "excavate 4 bedroom units");
        assert_eq!(intent.terminal_condition, Predicate::False);
        Ok(())
    }

    #[test]
    fn huge_dining_hall_is_rejected_for_cave_in_risk() -> Result<()> {
        let result = BlueprintPlanner.compile_blueprint_intent(
            IntentId::new(2),
            anchor(),
            MapCoord { x: 0, y: 0, z: 100 },
            BlueprintTemplate::DiningHall {
                width: 12,
                height: 12,
            },
            &covered_spatial_index()?,
        );
        assert!(result.is_err());
        Ok(())
    }

    #[test]
    fn coordinate_boundaries_and_zero_dimensions_are_rejected() -> Result<()> {
        let index = covered_spatial_index()?;
        let overflow = BlueprintPlanner.compile_blueprint_intent(
            IntentId::new(3),
            anchor(),
            MapCoord {
                x: i32::MAX,
                y: 0,
                z: 100,
            },
            BlueprintTemplate::DiningHall {
                width: 2,
                height: 2,
            },
            &index,
        );
        assert!(overflow.is_err());
        let zero = BlueprintPlanner.compile_blueprint_intent(
            IntentId::new(4),
            anchor(),
            MapCoord { x: 0, y: 0, z: 100 },
            BlueprintTemplate::DiningHall {
                width: 0,
                height: 2,
            },
            &index,
        );
        assert!(zero.is_err());
        Ok(())
    }
}
