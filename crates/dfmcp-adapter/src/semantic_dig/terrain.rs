//! Independently scoped terrain from one complete spatial/1.8 capture.
//!
//! This is a new projection. Existing spatial/1.8 entity projections and their
//! hashes remain unchanged. Padding, hidden and unsupported cells are UNKNOWN;
//! a hash alone never grants observation authority or terrain coverage.
use crate::live_spatial::citizens::LiveSpatialCitizenObservation;
use dfmcp_core::{
    Capability, Digest32, EntityId, ErrorCode, MapCoord, MapCuboid, ObservationCursor,
    OperationContext, Result, RiskTier, StateAnchor,
};
use dfmcp_world::map_region::{Cell, Shape};
use dfmcp_world::terrain::{tile_codes, tile_slot};
use dfmcp_world::{
    ChunkCoord, EntityKind, EntityRecord, EvidenceCoverage, EvidencePolicy, EvidenceSource, Fact,
    FactSource, MapChunk, PredicateEvidence, TerrainRun, Value, WorldGraph, WorldSnapshot,
};
use std::collections::{BTreeMap, BTreeSet};

pub const DIG_TERRAIN_SCHEMA: &str = "dfmcp.original_dig_terrain/1";
pub const MAX_PROJECTION_BYTES: u64 = 64 * 1024 * 1024;
fn error(code: ErrorCode, text: &str) -> dfmcp_core::DfmcpError {
    dfmcp_core::DfmcpError::new(code, text)
}
fn invalid(text: &str) -> dfmcp_core::DfmcpError {
    error(ErrorCode::AdapterRejected, text)
}

/// Exact source incarnation, software, native world and configured read region.
/// This checksum is custody evidence, never a restored source-policy grant.
pub fn source_identity_digest(raw: &LiveSpatialCitizenObservation) -> Result<Digest32> {
    raw.validate()?;
    let map = raw.spatial().terrain();
    let mut out = b"dfmcp.original-dig-spatial-source/1\0".to_vec();
    out.extend_from_slice(&map.bridge_generation.to_be_bytes());
    for value in [&map.df_version, &map.dfhack_version, &map.world_folder] {
        out.extend_from_slice(&(value.len() as u32).to_be_bytes());
        out.extend_from_slice(value.as_bytes());
    }
    out.extend_from_slice(&map.site_id.to_be_bytes());
    for value in map
        .map_dimensions
        .into_iter()
        .chain(map.map.region.origin)
        .chain(map.map.region.size)
    {
        out.extend_from_slice(&value.to_be_bytes());
    }
    Ok(Digest32::of_bytes(&out))
}

#[derive(Clone, Debug)]
pub struct DigTerrainProjection {
    snapshot: WorldSnapshot,
    raw: LiveSpatialCitizenObservation,
    source: Digest32,
    identity: Digest32,
    terrain_regions: Vec<MapCuboid>,
}
impl DigTerrainProjection {
    pub fn new(
        raw: LiveSpatialCitizenObservation,
        cursor: ObservationCursor,
        c: &OperationContext,
    ) -> Result<Self> {
        Self::project(raw, cursor, c, false)
    }
    pub(super) fn historical(
        raw: LiveSpatialCitizenObservation,
        cursor: ObservationCursor,
        c: &OperationContext,
    ) -> Result<Self> {
        // Historical interpretation retains current authority and its high tick;
        // only the typed source cursor may name older retained evidence.
        Self::project(raw, cursor, c, true)
    }
    fn project(
        raw: LiveSpatialCitizenObservation,
        cursor: ObservationCursor,
        c: &OperationContext,
        historical: bool,
    ) -> Result<Self> {
        c.budget.validate()?;
        c.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
        c.authorize(Capability::Observe, RiskTier::ReadOnly, &[], None)?;
        if c.budget.max_bytes < MAX_PROJECTION_BYTES {
            return Err(error(
                ErrorCode::BudgetExceeded,
                "original dig terrain projection requires its complete bounded allowance",
            ));
        }
        raw.validate()?;
        let map = raw.spatial().terrain();
        if !map.paused
            || map.fortress_id()? != c.anchor.fortress_id
            || cursor.epoch != c.anchor.cursor.epoch
            || (!historical
                && (cursor.sequence < c.anchor.cursor.sequence || map.tick() < c.anchor.tick))
        {
            return Err(error(
                ErrorCode::StaleAnchor,
                "original excavation requires the same paused named fortress and monotonic canonical cursor/tick",
            ));
        }
        let operations = raw.spatial().operations();
        let observed_nodes = 1usize
            .saturating_add(map.map.cells.len())
            .saturating_add(raw.citizens().len())
            .saturating_add(operations.jobs.jobs.len())
            .saturating_add(operations.buildings.len())
            .saturating_add(operations.items.len())
            .saturating_add(operations.attachments.len());
        if observed_nodes > c.budget.max_entities as usize {
            return Err(error(
                ErrorCode::BudgetExceeded,
                "complete spatial terrain and rosters exceed the operation entity allowance",
            ));
        }
        let source = raw.source_digest()?;
        let identity = source_identity_digest(&raw)?;
        let mut chunks: BTreeMap<ChunkCoord, Vec<u32>> = BTreeMap::new();
        let mut eligible = BTreeSet::new();
        for (index, cell) in map.map.cells.iter().enumerate() {
            let position = map
                .map
                .region
                .position(index)
                .ok_or_else(|| invalid("terrain capture index is outside its exact region"))?;
            let coord = MapCoord::new(position[0] as i32, position[1] as i32, position[2] as i32);
            let Cell::Visible(tile) = cell else {
                continue;
            };
            let code = match tile.shape {
                Shape::Empty => tile_codes::OPEN_SPACE,
                Shape::Wall => tile_codes::SOLID_WALL,
                Shape::Floor => tile_codes::FLOOR,
                Shape::Ramp => tile_codes::RAMP,
                Shape::StairUp | Shape::StairDown | Shape::StairUpDown => tile_codes::STAIR,
                Shape::Other | Shape::RampTop => tile_codes::UNKNOWN,
            };
            if code == tile_codes::UNKNOWN {
                continue;
            }
            let slot = tile_slot(coord);
            chunks
                .entry(slot.chunk)
                .or_insert_with(|| vec![tile_codes::UNKNOWN; 256])[slot.offset as usize] = code;
            // RegionTerrain is a shape predicate. Do not let a wet or still
            // designated floor stand in for completed, cleared mining work.
            if tile.liquid_depth == 0 && !tile.magma && tile.dig_designation == 0 {
                eligible.insert((coord.z, coord.y, coord.x));
            }
        }
        let terrain_regions = domains(&eligible)?;
        let mut graph = WorldGraph::default();
        for (coord, codes) in chunks {
            let mut runs: Vec<TerrainRun> = Vec::new();
            for code in codes {
                match runs.last_mut() {
                    Some(run) if run.tile_code == code => run.length += 1,
                    _ => runs.push(TerrainRun {
                        tile_code: code,
                        length: 1,
                    }),
                }
            }
            graph.chunks.insert(
                coord,
                MapChunk {
                    coord,
                    revision: cursor.sequence,
                    width: 16,
                    height: 16,
                    terrain_runs: runs,
                    sparse_overlays: BTreeMap::new(),
                },
            );
        }
        // Canonical content seals the full raw capture and source identity even
        // when two observations have identical projected shape codes.
        let entity = EntityId::new(1);
        let fact = |v| {
            Fact::known(
                v,
                map.tick(),
                FactSource::Derived(DIG_TERRAIN_SCHEMA.to_owned()),
                source,
            )
        };
        graph.entities.insert(
            entity,
            EntityRecord {
                id: entity,
                generation: 1,
                revision: cursor.sequence,
                kind: EntityKind::Fortress,
                label: DIG_TERRAIN_SCHEMA.to_owned(),
                fields: BTreeMap::from([
                    (
                        "source_digest".to_owned(),
                        fact(Value::Text(source.to_hex())),
                    ),
                    (
                        "source_identity".to_owned(),
                        fact(Value::Text(identity.to_hex())),
                    ),
                ]),
            },
        );
        let snapshot = WorldSnapshot::new(map.fortress_id()?, map.tick(), cursor, true, graph);
        c.authorize(Capability::Observe, RiskTier::ReadOnly, &[], None)?;
        Ok(Self {
            snapshot,
            raw,
            source,
            identity,
            terrain_regions,
        })
    }
    pub fn snapshot(&self) -> &WorldSnapshot {
        &self.snapshot
    }
    pub fn anchor(&self) -> StateAnchor {
        self.snapshot.anchor()
    }
    pub fn observation(&self) -> &LiveSpatialCitizenObservation {
        &self.raw
    }
    pub fn raw(&self) -> &LiveSpatialCitizenObservation {
        &self.raw
    }
    pub fn source_digest(&self) -> Digest32 {
        self.source
    }
    pub fn source_identity_digest(&self) -> Digest32 {
        self.identity
    }
    pub fn terrain_regions(&self) -> &[MapCuboid] {
        &self.terrain_regions
    }
    /// Terrain authority is granted by bounded domains, not entity source labels.
    pub fn evidence_sources(&self) -> BTreeSet<EvidenceSource> {
        BTreeSet::new()
    }
    pub fn evidence(&self, policy: EvidencePolicy) -> Result<PredicateEvidence<'_>> {
        if policy.all_entities != EvidenceCoverage::Unknown
            || !policy.entity_kinds.is_empty()
            || policy.all_edges != EvidenceCoverage::Unknown
            || !policy.edge_kinds.is_empty()
            || !policy.sources.is_empty()
            || policy.terrain_regions.iter().any(|area| {
                !self
                    .terrain_regions
                    .iter()
                    .any(|known| known.contains_cuboid(*area))
            })
        {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "dig terrain policy exceeds independently qualifiable observed dry, undesignated cells",
            ));
        }
        PredicateEvidence::scoped(&self.snapshot, policy)
    }
}

// First merge consecutive x cells, then equal spans in y, then equal rectangles
// in z. Ordinary large, complete rectangular captures require one policy domain.
fn domains(cells: &BTreeSet<(i32, i32, i32)>) -> Result<Vec<MapCuboid>> {
    let mut rows: Vec<MapCuboid> = Vec::new();
    for &(z, y, x) in cells {
        if let Some(last) = rows.last_mut()
            && last.min.z == z
            && last.min.y == y
            && last.max.x + 1 == x
        {
            last.max.x = x;
            continue;
        }
        let p = MapCoord::new(x, y, z);
        rows.push(MapCuboid::new(p, p)?);
    }
    let mut rectangles: Vec<MapCuboid> = Vec::new();
    for row in rows {
        if let Some(old) = rectangles.iter_mut().rev().find(|old| {
            old.min.z == row.min.z
                && old.min.x == row.min.x
                && old.max.x == row.max.x
                && old.max.y + 1 == row.min.y
        }) {
            old.max.y = row.max.y;
        } else {
            rectangles.push(row);
        }
    }
    let mut volumes: Vec<MapCuboid> = Vec::new();
    for rectangle in rectangles {
        if let Some(old) = volumes.iter_mut().rev().find(|old| {
            old.min.x == rectangle.min.x
                && old.max.x == rectangle.max.x
                && old.min.y == rectangle.min.y
                && old.max.y == rectangle.max.y
                && old.max.z + 1 == rectangle.min.z
        }) {
            old.max.z = rectangle.max.z;
        } else {
            volumes.push(rectangle);
        }
    }
    if volumes.len() > 64 {
        return Err(error(
            ErrorCode::BudgetExceeded,
            "observed eligible terrain exceeds the 64 exact policy-domain bound",
        ));
    }
    Ok(volumes)
}
