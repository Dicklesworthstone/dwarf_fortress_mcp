//! Where the original named citizens actually are, in one coherent spatial/1.8 capture.
//!
//! A declared shelter is geometry, not a native burrow, safe terrain or a route.
//! Missing citizens, changed incarnations and unavailable positions stay unknown.
//! A dead citizen is not silently removed from the original arrival condition.

use std::collections::BTreeSet;
use std::time::Instant;

use dfmcp_core::{
    Capability, DfmcpError, Digest32, EntityId, ErrorCode, MapCoord, MapCuboid,
    OperationContext, Result, RiskTier, StateAnchor, GameTick,
};
use dfmcp_world::{EntityKind, EntityRecord, FactSource, PredicateTruth, Value, WorldSnapshot};

use crate::live_spatial::{SpatialStateView, citizens::{
    LiveCitizen, LiveSpatialCitizenState, citizen_entity_id,
}};

pub const POLICY: &str = "named-incarnations-coherent-citizen-position/1";
pub const MAX_CITIZENS: usize = 128;
pub const MAX_SHELTERS: usize = 16;
pub const MAX_SHELTER_TILES: u64 = 262_144;
pub const MAX_WORK: u64 = 1_000_000;
// Maximum bounded selection/report workspace. The actual requirement scales
// with the named goal; it must also fit narrowed native query-result budgets.
pub const REQUIRED_BYTES: u64 = 32 * 1024;

fn invalid(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::InvalidRequest, message)
}
fn exhausted() -> DfmcpError {
    DfmcpError::new(ErrorCode::BudgetExceeded, "shelter analysis exhausted its bounded allowance")
}
fn invariant() -> DfmcpError {
    DfmcpError::new(ErrorCode::InternalInvariantViolation, "native citizen and canonical projection disagree")
}

/// Identity selected at the original observation, including the ABA incarnation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct CitizenTarget {
    pub entity_id: EntityId,
    pub generation: u32,
}

/// Fixed nonempty goal. Ordering is canonical; overlapping regions are a union.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShelterSelection {
    citizens: Vec<CitizenTarget>,
    shelters: Vec<MapCuboid>,
    digest: Digest32,
}
impl ShelterSelection {
    pub fn new(citizens: &[CitizenTarget], shelters: &[MapCuboid]) -> Result<Self> {
        if citizens.is_empty() || citizens.len() > MAX_CITIZENS
            || shelters.is_empty() || shelters.len() > MAX_SHELTERS
        {
            return Err(invalid("select 1..128 original citizens and 1..16 shelter regions"));
        }
        let mut ids = BTreeSet::new();
        for target in citizens {
            let raw = target.entity_id.get().checked_sub(citizen_entity_id(0).get());
            if target.generation == 0 || raw.is_none_or(|id| id > i32::MAX as u64)
                || !ids.insert(target.entity_id)
            {
                return Err(invalid("shelter citizens require unique spatial/1.8 IDs and nonzero generations"));
            }
        }
        let mut tiles = 0u64;
        for area in shelters {
            MapCuboid::new(area.min, area.max)?;
            if [area.min.x, area.min.y, area.min.z].iter().any(|n| *n < 0)
                || [area.max.x, area.max.y, area.max.z].iter().any(|n| *n >= 32_768)
            {
                return Err(invalid("shelter coordinates must be within the native coordinate domain"));
            }
            tiles = tiles.checked_add(area.tile_count().ok_or_else(exhausted)?).ok_or_else(exhausted)?;
            if tiles > MAX_SHELTER_TILES { return Err(exhausted()); }
        }
        let mut citizens = citizens.to_vec();
        citizens.sort_unstable();
        let mut shelters = shelters.to_vec();
        shelters.sort_unstable();
        if shelters.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(invalid("duplicate shelter region"));
        }
        let mut bytes = b"dfmcp-shelter-selection/1\0".to_vec();
        bytes.extend_from_slice(&(citizens.len() as u32).to_be_bytes());
        for target in &citizens {
            bytes.extend_from_slice(&target.entity_id.get().to_be_bytes());
            bytes.extend_from_slice(&target.generation.to_be_bytes());
        }
        bytes.extend_from_slice(&(shelters.len() as u32).to_be_bytes());
        for area in &shelters {
            for n in [area.min.x, area.min.y, area.min.z, area.max.x, area.max.y, area.max.z] {
                bytes.extend_from_slice(&n.to_be_bytes());
            }
        }
        Ok(Self { citizens, shelters, digest: Digest32::of_bytes(&bytes) })
    }
    pub fn citizens(&self) -> &[CitizenTarget] { &self.citizens }
    pub fn shelters(&self) -> &[MapCuboid] { &self.shelters }
    pub const fn digest(&self) -> Digest32 { self.digest }
    pub fn required_bytes(&self) -> u64 {
        1_024 + self.citizens.len() as u64 * 128 + self.shelters.len() as u64 * 64
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArrivalStatus {
    Inside,
    Outside,
    NotObserved,
    IncarnationChanged,
    NotAlive,
    PositionUnknown,
}
impl ArrivalStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Inside => "inside",
            Self::Outside => "outside",
            Self::NotObserved => "not_observed",
            Self::IncarnationChanged => "incarnation_changed",
            Self::NotAlive => "not_alive",
            Self::PositionUnknown => "position_unknown",
        }
    }
    pub const fn truth(self) -> PredicateTruth {
        match self {
            Self::Inside => PredicateTruth::True,
            Self::Outside | Self::NotAlive => PredicateTruth::False,
            Self::NotObserved | Self::IncarnationChanged | Self::PositionUnknown => PredicateTruth::Unknown,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArrivalRow {
    pub target: CitizenTarget,
    pub observed_generation: Option<u32>,
    pub observed_revision: Option<u64>,
    pub status: ArrivalStatus,
    pub position: Option<MapCoord>,
    /// First canonical containing region, not an additive occupancy claim.
    pub shelter_index: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShelterReport {
    pub anchor: StateAnchor,
    pub source_digest: Digest32,
    pub bridge_generation: u64,
    pub selection: ShelterSelection,
    pub rows: Vec<ArrivalRow>,
    pub arrival_truth: PredicateTruth,
    pub inside: usize,
    pub outside: usize,
    pub not_alive: usize,
    pub unresolved: usize,
    pub work_units: u64,
}

fn position_status(citizen: &LiveCitizen, dimensions: [u32; 3], shelters: &[MapCuboid])
    -> (ArrivalStatus, Option<MapCoord>, Option<usize>)
{
    if !citizen.alive() { return (ArrivalStatus::NotAlive, None, None); }
    let position = citizen.position;
    let coordinates = [position.x, position.y, position.z];
    if !citizen.active() || !citizen.visible()
        || coordinates.iter().zip(dimensions).any(|(&n, max)| n < 0 || n as u32 >= max)
    {
        return (ArrivalStatus::PositionUnknown, None, None);
    }
    match shelters.iter().position(|area| area.contains(position)) {
        Some(index) => (ArrivalStatus::Inside, Some(position), Some(index)),
        None => (ArrivalStatus::Outside, Some(position), None),
    }
}

// The native owner publishes this digest with its immutable projection. Reuse
// it instead of re-encoding megabytes of unrelated items and skill histories for
// a small positional query. Source admission remains the observing shell's duty.
fn fact_matches(entity: &EntityRecord, field: &str, native: &str, expected: &Value,
    source: Digest32, tick: GameTick) -> bool
{
    entity.fields.get(field).is_some_and(|fact| {
        fact.source_digest == source && fact.observed_at == tick
            && matches!(&fact.source, FactSource::DfhackField(name) if name == native)
            && fact.known_value() == Some(expected)
    })
}
fn published_digest(snapshot: &WorldSnapshot, count: usize) -> Result<Digest32> {
    let root = snapshot.graph.entities.get(&EntityId::new(1)).ok_or_else(invariant)?;
    let profile = root.fields.get("observation_profile").ok_or_else(invariant)?;
    let source = profile.source_digest;
    if source == Digest32::ZERO
        || !fact_matches(root, "observation_profile", "spatial/1.8.capture",
            &Value::Text("spatial/1.8".to_owned()), source, snapshot.tick)
        || !fact_matches(root, "strict_citizen_count", "spatial/1.8.citizens.count",
            &Value::U64(count as u64), source, snapshot.tick)
    { return Err(invariant()); }
    Ok(source)
}
fn verify_citizen(entity: &EntityRecord, citizen: &LiveCitizen, source: Digest32, tick: GameTick) -> Result<()> {
    for (field, native, expected) in [
        ("native_unit_id", "spatial/1.8.citizen.unit_id", Value::U64(u64::from(citizen.native_id))),
        ("position", "spatial/1.8.citizen.position", Value::Coord(citizen.position)),
        ("alive", "spatial/1.8.alive", Value::Bool(citizen.alive())),
        ("active", "spatial/1.8.active", Value::Bool(citizen.active())),
        ("visible", "spatial/1.8.visible", Value::Bool(citizen.visible())),
    ] {
        if !fact_matches(entity, field, native, &expected, source, tick) { return Err(invariant()); }
    }
    Ok(())
}

/// Analyze actual native positions, not reference assignments or caller facts.
/// This does no I/O and creates neither an action nor a mutation permission.
pub fn assess(state: &LiveSpatialCitizenState, context: &OperationContext,
    selection: &ShelterSelection, maximum_work: u64) -> Result<ShelterReport>
{
    let started = Instant::now();
    context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    if maximum_work == 0 || maximum_work > MAX_WORK || context.budget.max_bytes < selection.required_bytes() {
        return Err(exhausted());
    }
    let snapshot = state.snapshot().ok_or_else(|| invalid("coherent citizen capture is absent"))?;
    if snapshot.anchor() != context.anchor {
        return Err(DfmcpError::new(ErrorCode::StaleAnchor, "shelter context names another capture"));
    }
    if snapshot.graph.entities.len() > context.budget.max_entities as usize { return Err(exhausted()); }
    let source = state.observation_full().ok_or_else(invariant)?;
    let dimensions = source.spatial().terrain().map_dimensions;
    for area in &selection.shelters {
        if [area.max.x, area.max.y, area.max.z].iter().zip(dimensions).any(|(&n, max)| n as u32 >= max) {
            return Err(invalid("shelter exceeds the observed fortress map dimensions"));
        }
    }
    let mut work = 0u64;
    let mut charge = |units: u64| -> Result<()> {
        work = work.checked_add(units).ok_or_else(exhausted)?;
        if work > maximum_work || started.elapsed().as_millis() >= u128::from(context.budget.max_wall_millis) {
            return Err(exhausted());
        }
        Ok(())
    };
    // Charge comparisons before inspecting the retained roster. No full-source
    // encoding, canonical-world clone, or native acquisition occurs here.
    charge(source.citizens().len() as u64 + selection.shelters.len() as u64 + 1)?;
    let source_digest = published_digest(snapshot, source.citizens().len())?;
    let mut rows = Vec::with_capacity(selection.citizens.len());
    let (mut inside, mut outside, mut not_alive, mut unresolved) = (0, 0, 0, 0);
    for target in &selection.citizens {
        charge(32 + selection.shelters.len() as u64)?;
        let native = (target.entity_id.get() - citizen_entity_id(0).get()) as u32;
        let mut row = ArrivalRow { target: *target, observed_generation: None,
            observed_revision: None, status: ArrivalStatus::NotObserved, position: None, shelter_index: None };
        if let Ok(index) = source.citizens().binary_search_by_key(&native, |citizen| citizen.native_id) {
            let citizen = &source.citizens()[index];
            let entity = snapshot.graph.entities.get(&target.entity_id).ok_or_else(invariant)?;
            if entity.id != target.entity_id || entity.kind != EntityKind::Unit { return Err(invariant()); }
            verify_citizen(entity, citizen, source_digest, snapshot.tick)?;
            row.observed_generation = Some(entity.generation);
            row.observed_revision = Some(entity.revision);
            if entity.generation != target.generation {
                row.status = ArrivalStatus::IncarnationChanged;
            } else {
                (row.status, row.position, row.shelter_index) = position_status(citizen, dimensions, &selection.shelters);
            }
        }
        match row.status {
            ArrivalStatus::Inside => inside += 1,
            ArrivalStatus::Outside => outside += 1,
            ArrivalStatus::NotAlive => not_alive += 1,
            _ => unresolved += 1,
        }
        rows.push(row);
    }
    charge(1)?;
    charge(0)?;
    context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    let arrival_truth = if outside + not_alive > 0 { PredicateTruth::False }
        else if unresolved > 0 { PredicateTruth::Unknown } else { PredicateTruth::True };
    Ok(ShelterReport { anchor: snapshot.anchor(), source_digest,
        bridge_generation: source.spatial().terrain().bridge_generation,
        selection: selection.clone(), rows, arrival_truth, inside, outside, not_alive, unresolved, work_units: work })
}

