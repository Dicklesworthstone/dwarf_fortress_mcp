#![forbid(unsafe_code)]

//! Conservative spatial-blueprint intent compiler.
//!
//! Geometry is bounded and connected where the template requires access. The
//! legacy index supplies only a limited magma/span preflight, not a native safety
//! proof. Terrain-completion predicates remain `False`: source geometry and a
//! transport acknowledgement cannot establish that excavation has completed.

use dfmcp_core::{
    DfmcpError, ErrorCode, GameTick, IntentId, MapCoord, MapCuboid, Result, RiskTier, StateAnchor,
};
use dfmcp_world::Predicate;
use dfmcp_world::TileType;
use dfmcp_world::spatial_index::{ChunkSpatialIndex, TemperatureBand};

use crate::action::{Action, DigMode};
use crate::plan::{Constraint, Intent, ObligationSpec, RequestedAction};

pub mod layout;
pub use layout::{BlueprintExcavation, BlueprintLayout, ExcavationRole};

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
    /// One-tile perimeter on one z-level. The optional north-edge crossing is
    /// left unexcavated; its span does not request or prove bridge construction.
    DefensiveMoat {
        perimeter_cuboid: MapCuboid,
        drawbridge_span: u8,
    },
}

/// Result of the legacy index's limited magma/span preflight.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HazardAssessment {
    Safe,
    MagmaProximity { hazard_coord: MapCoord },
    UnsupportedCaveInRisk { span_width: u64 },
    IncompleteKnowledge { reason: String },
}

#[derive(Clone, Debug, Default)]
pub struct BlueprintPlanner;

impl BlueprintPlanner {
    /// Generate inspectable geometry without treating it as a prepared plan.
    pub fn layout(&self, origin: MapCoord, template: BlueprintTemplate) -> Result<BlueprintLayout> {
        BlueprintLayout::generate(origin, template)
    }

    /// Check a bounded excavation area against a complete one-tile hazard halo.
    /// `Safe` means only that this limited index check found no listed hazard.
    #[must_use]
    pub fn assess_hazards(
        &self,
        area: &MapCuboid,
        spatial_index: &ChunkSpatialIndex,
    ) -> HazardAssessment {
        let halo = match hazard_halo(area) {
            Ok(halo) => halo,
            Err(error) => {
                return HazardAssessment::IncompleteKnowledge {
                    reason: error.message,
                };
            }
        };
        let Some(expected_tiles) = halo.tile_count() else {
            return HazardAssessment::IncompleteKnowledge {
                reason: "hazard halo tile count overflow".to_owned(),
            };
        };
        if expected_tiles > MAX_HAZARD_TILES {
            return HazardAssessment::IncompleteKnowledge {
                reason: "hazard halo exceeds the blueprint work budget".to_owned(),
            };
        }
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

    /// Compile geometry only after every part's bounded preflight succeeds.
    /// False terminal predicates preserve the existing fail-closed completion rule.
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
        let layout = self.layout(origin, template)?;
        // Sum the *actual* repeated halo work, not just unique excavated tiles.
        let mut scan_tiles = 0_u64;
        for part in layout.excavations() {
            let count = hazard_halo(&part.area)?.tile_count().ok_or_else(|| {
                DfmcpError::new(
                    ErrorCode::BudgetExceeded,
                    "blueprint hazard tile count overflow",
                )
            })?;
            scan_tiles = scan_tiles.checked_add(count).ok_or_else(|| {
                DfmcpError::new(
                    ErrorCode::BudgetExceeded,
                    "blueprint hazard tile count overflow",
                )
            })?;
            if scan_tiles > MAX_HAZARD_TILES {
                return Err(DfmcpError::new(
                    ErrorCode::BudgetExceeded,
                    "blueprint hazard scans exceed 131072 tile visits",
                ));
            }
        }
        for part in layout.excavations() {
            self.require_safe(layout.summary(), &part.area, spatial_index)?;
        }
        Ok(Intent {
            id: intent_id,
            anchor,
            summary: layout.summary().to_owned(),
            terminal_condition: Predicate::False,
            constraints: vec![Constraint::MaxRisk(RiskTier::Guarded)],
            requested_actions: layout
                .excavations()
                .iter()
                .map(|part| dig_request(part.area, part.mode, deadline))
                .collect(),
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

const MAX_HAZARD_TILES: u64 = 131_072;

fn hazard_halo(area: &MapCuboid) -> Result<MapCuboid> {
    if area.min.x > area.max.x || area.min.y > area.max.y || area.min.z > area.max.z {
        return Err(invalid("hazard scan requires an ordered cuboid"));
    }
    let low = checked_coord_offset(area.min, -1, -1, -1)
        .ok_or_else(|| invalid("hazard halo crosses the coordinate boundary"))?;
    let high = checked_coord_offset(area.max, 1, 1, 1)
        .ok_or_else(|| invalid("hazard halo crosses the coordinate boundary"))?;
    MapCuboid::new(low, high)
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
    (i64::from(maximum.max(minimum)) - i64::from(minimum.min(maximum)) + 1) as u64
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
        let intent = BlueprintPlanner.compile_blueprint_intent(
            IntentId::new(1),
            anchor(),
            MapCoord { x: 0, y: 0, z: 100 },
            BlueprintTemplate::BedroomCluster {
                rooms_count: 4,
                room_size: (3, 3),
            },
            &covered_spatial_index()?,
        )?;
        assert_eq!(intent.requested_actions.len(), 10);
        assert_eq!(intent.summary, "excavate 4 bedroom units");
        assert_eq!(intent.terminal_condition, Predicate::False);
        Ok(())
    }

    #[test]
    fn huge_dining_hall_is_rejected_for_cave_in_risk() -> Result<()> {
        assert!(
            BlueprintPlanner
                .compile_blueprint_intent(
                    IntentId::new(2),
                    anchor(),
                    MapCoord { x: 0, y: 0, z: 100 },
                    BlueprintTemplate::DiningHall {
                        width: 12,
                        height: 12
                    },
                    &covered_spatial_index()?,
                )
                .is_err()
        );
        Ok(())
    }

    #[test]
    fn coordinate_boundaries_and_zero_dimensions_are_rejected() -> Result<()> {
        let index = covered_spatial_index()?;
        assert!(
            BlueprintPlanner
                .compile_blueprint_intent(
                    IntentId::new(3),
                    anchor(),
                    MapCoord {
                        x: i32::MAX,
                        y: 0,
                        z: 100
                    },
                    BlueprintTemplate::DiningHall {
                        width: 2,
                        height: 2
                    },
                    &index,
                )
                .is_err()
        );
        assert!(
            BlueprintPlanner
                .compile_blueprint_intent(
                    IntentId::new(4),
                    anchor(),
                    MapCoord { x: 0, y: 0, z: 100 },
                    BlueprintTemplate::DiningHall {
                        width: 0,
                        height: 2
                    },
                    &index,
                )
                .is_err()
        );
        Ok(())
    }
}
