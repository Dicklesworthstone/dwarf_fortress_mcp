#![forbid(unsafe_code)]

//! Conservative spatial-blueprint intent compiler.
//!
//! Geometry is bounded and connected where the template requires access. The
//! legacy index supplies only a limited magma/span preflight, not a native safety
//! proof. Each excavation seals a region-terrain postcondition and obligation:
//! completion is proven only by later observation of the target terrain, never by
//! source geometry or a transport acknowledgement.

use dfmcp_core::{
    DfmcpError, ErrorCode, GameTick, IntentId, MapCoord, MapCuboid, Result, RiskTier, StateAnchor,
    StepId,
};
use dfmcp_world::spatial_index::{ChunkSpatialIndex, TemperatureBand};
use dfmcp_world::{CompareOp, EntityKind, Predicate, TileType, Value};

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
    Dormitory {
        bed_count: u8,
        columns: u8,
    },
    DiningPairs {
        table_count: u8,
        columns: u8,
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
    /// This original excavation compiler retains its sealed deadline policy.
    pub fn compile_blueprint_intent(
        &self,
        intent_id: IntentId,
        anchor: StateAnchor,
        origin: MapCoord,
        template: BlueprintTemplate,
        spatial_index: &ChunkSpatialIndex,
    ) -> Result<Intent> {
        let deadline = legacy_deadline(anchor)?;
        let layout = self.layout(origin, template)?;
        self.compile_layout_intent(intent_id, anchor, &layout, spatial_index, Some(deadline))
    }

    fn compile_layout_intent(
        &self,
        intent_id: IntentId,
        anchor: StateAnchor,
        layout: &BlueprintLayout,
        spatial_index: &ChunkSpatialIndex,
        deadline: Option<GameTick>,
    ) -> Result<Intent> {
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
            terminal_condition: Predicate::All(
                layout
                    .excavations()
                    .iter()
                    .map(|part| Predicate::RegionTerrain {
                        area: part.area,
                        tile_code: crate::effects::dig_target_tile_code(part.mode),
                    })
                    .collect(),
            )
            .normalized(),
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

impl BlueprintPlanner {
    /// Compile a blueprint and furnish its rooms: a bed in every bedroom, and a
    /// table with a chair in a dining hall. The original goal requires both the
    /// complete terrain and each exact completed furnishing. Omitted obligations
    /// let the ordinary planner account for workload and prerequisite deadlines.
    pub fn compile_furnished_blueprint_intent(
        &self,
        intent_id: IntentId,
        anchor: StateAnchor,
        origin: MapCoord,
        template: BlueprintTemplate,
        spatial_index: &ChunkSpatialIndex,
    ) -> Result<Intent> {
        self.compile_furnished(intent_id, anchor, origin, template, spatial_index, false)
    }

    /// Reconstruct already retained, unversioned blueprint requests exactly.
    /// New requests must use the current compiler; old goals cannot be silently
    /// strengthened or their sealed deadlines extended during recovery.
    pub fn compile_furnished_blueprint_intent_legacy(
        &self,
        intent_id: IntentId,
        anchor: StateAnchor,
        origin: MapCoord,
        template: BlueprintTemplate,
        spatial_index: &ChunkSpatialIndex,
    ) -> Result<Intent> {
        self.compile_furnished(intent_id, anchor, origin, template, spatial_index, true)
    }

    fn compile_furnished(
        &self,
        intent_id: IntentId,
        anchor: StateAnchor,
        origin: MapCoord,
        template: BlueprintTemplate,
        spatial_index: &ChunkSpatialIndex,
        legacy: bool,
    ) -> Result<Intent> {
        let furniture = match &template {
            BlueprintTemplate::BedroomCluster { .. } => vec!["Bed"],
            BlueprintTemplate::DiningHall { .. } => vec!["Table", "Chair"],
            BlueprintTemplate::DiningPairs { table_count, .. } => {
                ["Chair", "Table"].repeat(usize::from(*table_count))
            }
            BlueprintTemplate::Dormitory { bed_count, .. } => vec!["Bed"; usize::from(*bed_count)],
            _ => Vec::new(),
        };
        let layout = self.layout(origin, template)?;
        let deadline = legacy.then(|| legacy_deadline(anchor)).transpose()?;
        let mut intent =
            self.compile_layout_intent(intent_id, anchor, &layout, spatial_index, deadline)?;
        let mut added = Vec::new();
        let mut terminal = vec![intent.terminal_condition.clone()];
        for (index, part) in layout.excavations().iter().enumerate() {
            if part.role != ExcavationRole::Room {
                continue;
            }
            let dependency = u32::try_from(index)
                .map_err(|_| invalid("blueprint step index exceeds its representation"))?;
            let centre = MapCoord {
                x: part.area.min.x + (part.area.max.x - part.area.min.x) / 2,
                y: part.area.min.y + (part.area.max.y - part.area.min.y) / 2,
                z: part.area.min.z,
            };
            let mut occupied = Vec::new();
            for (offset, name) in furniture.iter().enumerate() {
                let shift = i32::try_from(offset).map_err(|_| invalid("furniture offset"))?;
                let location = if let Some(&bed) = layout.dormitory_beds().get(offset) {
                    bed
                } else if let Some(&(chair, table)) = layout.dining_pairs().get(offset / 2) {
                    if *name == "Chair" { chair } else { table }
                } else if legacy {
                    MapCoord {
                        x: if centre.x + shift <= part.area.max.x {
                            centre.x + shift
                        } else {
                            centre.x - shift
                        },
                        ..centre
                    }
                } else {
                    furniture_location(part.area, centre, &occupied)?
                };
                occupied.push(location);
                let action = Action::Build {
                    kind: crate::action::BuildingKind::Furniture((*name).to_owned()),
                    location,
                    footprint: MapCuboid::new(location, location)?,
                    material: crate::action::MaterialSelector::default(),
                };
                if !legacy {
                    let step_number = u32::try_from(intent.requested_actions.len() + added.len())
                        .map_err(|_| {
                        invalid("blueprint step index exceeds its representation")
                    })?;
                    let key = crate::derive_step_idempotency_key(
                        intent_id,
                        anchor,
                        StepId::new(step_number),
                        &action,
                    );
                    terminal.extend(furnishing_goal(&action, &key, anchor));
                }
                added.push(RequestedAction {
                    action,
                    preconditions: Vec::new(),
                    postconditions: Vec::new(),
                    compensation: None,
                    obligation: None,
                    depends_on: vec![dependency],
                });
            }
        }
        if intent.requested_actions.len() + added.len() > layout::MAX_BLUEPRINT_ACTIONS {
            return Err(DfmcpError::new(
                ErrorCode::BudgetExceeded,
                "furnished blueprint exceeds 64 steps",
            ));
        }
        intent.requested_actions.extend(added);
        if !legacy {
            intent.terminal_condition = Predicate::All(terminal).normalized();
            intent.terminal_condition.validate_shape()?;
        }
        Ok(intent)
    }
}

fn legacy_deadline(anchor: StateAnchor) -> Result<GameTick> {
    anchor.tick.checked_add(1_000).ok_or_else(|| {
        DfmcpError::new(
            ErrorCode::BudgetExceeded,
            "blueprint obligation deadline overflows the game tick range",
        )
    })
}

/// Preserve ordinary east/west placement, then use the vertical room axis for
/// narrow halls. Every selected tile is distinct and inside the room footprint.
fn furniture_location(
    area: MapCuboid,
    centre: MapCoord,
    occupied: &[MapCoord],
) -> Result<MapCoord> {
    [(0, 0), (1, 0), (-1, 0), (0, 1), (0, -1)]
        .into_iter()
        .filter_map(|(dx, dy)| checked_coord_offset(centre, dx, dy, 0))
        .find(|point| area.contains(*point) && !occupied.contains(point))
        .ok_or_else(|| invalid("furnished room cannot fit its distinct furniture footprints"))
}

/// A completed unrelated building or a moved/mistyped furnishing cannot satisfy
/// this original blueprint. These are laboratory construction predicates, not
/// native item custody or room ownership claims.
fn furnishing_goal(action: &Action, key: &str, anchor: StateAnchor) -> Vec<Predicate> {
    let mut predicates = crate::effects::default_postconditions(action, key, anchor.fortress_id);
    if let Action::Build {
        kind,
        location,
        footprint,
        ..
    } = action
    {
        let entity_id = crate::effects::created_entity_id(key, 0);
        predicates.push(Predicate::EntityKind {
            entity_id,
            kind: EntityKind::Building,
        });
        for (field, value) in [
            (
                "building_kind",
                Value::Text(crate::effects::building_kind_label(kind)),
            ),
            ("position", Value::Coord(*location)),
            ("footprint_min", Value::Coord(footprint.min)),
            ("footprint_max", Value::Coord(footprint.max)),
        ] {
            predicates.push(Predicate::FieldCompare {
                entity_id,
                field: field.to_owned(),
                op: CompareOp::Eq,
                value,
            });
        }
    }
    predicates
}

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

fn dig_request(area: MapCuboid, mode: DigMode, deadline_tick: Option<GameTick>) -> RequestedAction {
    RequestedAction {
        action: Action::DesignateDig { area, mode },
        preconditions: Vec::new(),
        postconditions: vec![Predicate::RegionTerrain {
            area,
            tile_code: crate::effects::dig_target_tile_code(mode),
        }],
        compensation: None,
        obligation: deadline_tick.map(|deadline_tick| ObligationSpec {
            terminal: Predicate::RegionTerrain {
                area,
                tile_code: crate::effects::dig_target_tile_code(mode),
            },
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
    fn every_dining_pair_has_distinct_interior_tiles_and_a_connected_aisle() -> Result<()> {
        let origin = MapCoord::new(2, 6, 10);
        for table_count in 1..=8 {
            for columns in 1..=4 {
                let layout = BlueprintPlanner.layout(
                    origin,
                    BlueprintTemplate::DiningPairs {
                        table_count,
                        columns,
                    },
                )?;
                let area = layout.excavations()[0].area;
                let occupied: std::collections::BTreeSet<_> = layout
                    .dining_pairs()
                    .iter()
                    .flat_map(|&(chair, table)| [chair, table])
                    .collect();
                assert_eq!(occupied.len(), usize::from(table_count) * 2);
                for &(chair, table) in layout.dining_pairs() {
                    assert_eq!(table, MapCoord::new(chair.x + 1, chair.y, chair.z));
                    assert!(area.contains(chair) && area.contains(table));
                    assert!((origin.x..=table.x).all(|x| {
                        !occupied.contains(&MapCoord::new(x, table.y - 1, table.z))
                    }));
                    assert!((origin.y..table.y).all(|y| {
                        !occupied.contains(&MapCoord::new(origin.x, y, table.z))
                    }));
                }
            }
        }
        for (table_count, columns) in [(0, 4), (9, 4), (1, 0), (1, 5)] {
            assert!(
                BlueprintPlanner
                    .layout(origin, BlueprintTemplate::DiningPairs { table_count, columns })
                    .is_err()
            );
        }
        Ok(())
    }

    #[test]
    fn shared_dormitory_seals_distinct_beds_and_approach_aisles() -> Result<()> {
        let origin = MapCoord::new(2, 6, 10);
        for beds in 1..=16 {
            for columns in 1..=8 {
                let layout = BlueprintPlanner.layout(
                    origin,
                    BlueprintTemplate::Dormitory {
                        bed_count: beds,
                        columns,
                    },
                )?;
                assert_eq!(layout.excavations().len(), 1);
                assert_eq!(layout.access_point(), Some(origin));
                let area = layout.excavations()[0].area;
                let targets: std::collections::BTreeSet<_> =
                    layout.dormitory_beds().iter().copied().collect();
                assert_eq!(targets.len(), usize::from(beds));
                for bed in &targets {
                    assert!(area.contains(*bed));
                    let approach = MapCoord::new(bed.x, bed.y - 1, bed.z);
                    assert!(area.contains(approach));
                    assert!(
                        (origin.x..=bed.x)
                            .all(|x| !targets.contains(&MapCoord::new(x, approach.y, bed.z)))
                    );
                    assert!(
                        (origin.y..=approach.y)
                            .all(|y| !targets.contains(&MapCoord::new(origin.x, y, bed.z)))
                    );
                }
            }
        }
        for (bed_count, columns) in [(0, 4), (17, 4), (1, 0), (1, 9)] {
            assert!(
                BlueprintPlanner
                    .layout(origin, BlueprintTemplate::Dormitory { bed_count, columns })
                    .is_err()
            );
        }
        Ok(())
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
        // The intent is complete only when every excavated region shows its
        // target terrain; each action seals the same region predicate.
        let Predicate::All(regions) = &intent.terminal_condition else {
            return Err(DfmcpError::new(
                ErrorCode::InternalInvariantViolation,
                "blueprint terminal is not a region conjunction",
            ));
        };
        assert_eq!(regions.len(), 10);
        for requested in &intent.requested_actions {
            assert!(regions.contains(&requested.postconditions[0]));
            assert_eq!(
                requested.obligation.as_ref().map(|o| &o.terminal),
                Some(&requested.postconditions[0])
            );
        }
        Ok(())
    }

    #[test]
    fn furnished_bedrooms_get_a_bed_each_after_their_room_is_dug() -> Result<()> {
        let intent = BlueprintPlanner.compile_furnished_blueprint_intent(
            IntentId::new(1),
            anchor(),
            MapCoord { x: 0, y: 0, z: 100 },
            BlueprintTemplate::BedroomCluster {
                rooms_count: 4,
                room_size: (3, 3),
            },
            &covered_spatial_index()?,
        )?;
        let beds: Vec<&RequestedAction> = intent
            .requested_actions
            .iter()
            .filter(|requested| matches!(requested.action, Action::Build { .. }))
            .collect();
        assert_eq!(beds.len(), 4);
        for bed in beds {
            let room = &intent.requested_actions[bed.depends_on[0] as usize];
            let (Action::Build { location, .. }, Action::DesignateDig { area, .. }) =
                (&bed.action, &room.action)
            else {
                return Err(invalid("unexpected blueprint step shapes"));
            };
            assert!(area.contains(*location));
        }
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
