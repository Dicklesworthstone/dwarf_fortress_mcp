//! Sampled completion of one complete original room intent and its furnishings.
//!
//! Equal paused map endpoints bracket one receipt-linked operations capture.
//! These are independently scoped sources, not an atomic world snapshot,
//! native room assignment, pathfinding result or causal effect proof.
pub mod store;
mod wire;

use std::ops::Deref;
use std::time::Instant;

use dfmcp_core::{
    Capability, DfmcpError, Digest32, ErrorCode, GameTick, OperationContext, Result, RiskTier,
};
use dfmcp_world::map_region::{Cell, Shape};

use crate::construction_plan::origin::{MonitorDefinition, Origin};
use crate::construction_plan::{self as construction, Goal, LinkedSample, Manifest};
use crate::live_map::LiveMapObservation;
use crate::live_operations::{LiveOperationsObservation, OperationsProfile};
use crate::room_intent::RoomIntent;

pub const MAX_MAP_CAPTURE: usize = 4096 * 20 + 1024;
pub const MAX_SAMPLE: usize = construction::MAX_SAMPLE + 2 * MAX_MAP_CAPTURE + 1024;
pub const MAX_DEFINITION_SIZE: usize = construction::origin::MAX_DEFINITION_SIZE + 12;
pub const MAX_DEFICITS: usize = 64;
pub const POLICY: &str = "complete-original-room-and-furnishings/1";

fn invalid(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::AdapterRejected, message)
}
fn bounded() -> DfmcpError {
    DfmcpError::new(
        ErrorCode::BudgetExceeded,
        "joint room observation exceeds its shared allowance",
    )
}
fn require(value: bool, message: &str) -> Result<()> {
    if value { Ok(()) } else { Err(invalid(message)) }
}
fn hash(domain: &[u8], bytes: &[u8]) -> Digest32 {
    let mut input = Vec::with_capacity(domain.len() + bytes.len());
    input.extend_from_slice(domain);
    input.extend_from_slice(bytes);
    Digest32::of_bytes(&input)
}

/// The complete original room-backed placement origin and fixed timing policy.
/// Roomless furniture histories cannot acquire a retrospective room intention.
#[derive(Clone, Debug)]
pub struct Definition {
    original: MonitorDefinition,
    room: RoomIntent,
    bytes: Vec<u8>,
    id: Digest32,
}
impl Definition {
    pub fn new(original: MonitorDefinition) -> Result<Self> {
        let room = original
            .origin()
            .definition()
            .room()
            .cloned()
            .ok_or_else(|| {
                invalid("joint room completion requires the original room-backed furnishing batch")
            })?;
        let mut bytes = b"DFMRCG01".to_vec();
        bytes.extend_from_slice(&(original.canonical_bytes().len() as u32).to_be_bytes());
        bytes.extend_from_slice(original.canonical_bytes());
        require(
            bytes.len() <= MAX_DEFINITION_SIZE,
            "joint room definition exceeds its bound",
        )?;
        let id = hash(b"dfmcp.original-room-condition/1\0", &bytes);
        Ok(Self {
            original,
            room,
            bytes,
            id,
        })
    }
    pub fn original(&self) -> &MonitorDefinition {
        &self.original
    }
    pub fn origin(&self) -> &Origin {
        self.original.origin()
    }
    pub fn goal(&self) -> &Goal {
        self.original.goal()
    }
    pub fn room(&self) -> &RoomIntent {
        &self.room
    }
    pub fn id(&self) -> Digest32 {
        self.id
    }
    pub fn canonical_bytes(&self) -> &[u8] {
        &self.bytes
    }
}

/// Full raw evidence acquired in one fixed query-only transport bracket.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sample {
    pub before: Manifest,
    pub before_capture: Vec<u8>,
    pub furnishings: LinkedSample,
    pub after: Manifest,
    pub after_capture: Vec<u8>,
}
impl Sample {
    fn map(&self, before: bool) -> Result<LiveMapObservation> {
        let (manifest, bytes) = if before {
            (&self.before, &self.before_capture)
        } else {
            (&self.after, &self.after_capture)
        };
        manifest.validate()?;
        require(
            !bytes.is_empty() && bytes.len() <= MAX_MAP_CAPTURE,
            "room map capture exceeds its bound",
        )?;
        let map = LiveMapObservation::decode_payload(
            bytes,
            manifest.generation,
            manifest.df_version.clone(),
            manifest.dfhack_version.clone(),
        )?;
        require(
            map.map.cells.len() <= 4096,
            "room map exceeds 4096 captured cells",
        )?;
        Ok(map)
    }
    fn operations(&self) -> Result<LiveOperationsObservation> {
        self.furnishings.operations.validate()?;
        require(
            self.furnishings.capture.len() <= construction::MAX_CAPTURE,
            "room operations capture exceeds its bound",
        )?;
        LiveOperationsObservation::decode_profile(
            &self.furnishings.capture,
            self.furnishings.operations.generation,
            self.furnishings.operations.df_version.clone(),
            self.furnishings.operations.dfhack_version.clone(),
            OperationsProfile::PagedV1_4,
        )
    }
    /// Retain every decoded clock before later custody or semantic checks.
    /// This is not acceptance of the sample as joint room evidence.
    pub fn observed_tick(&self) -> Result<u64> {
        let mut tick = 0;
        self.retain_observed_tick(&mut tick)?;
        Ok(tick)
    }
    /// Retain each consumed source clock before decoding the next component.
    /// Even an incomplete bracket cannot make an already learned tick disappear.
    pub fn retain_observed_tick(&self, tick: &mut u64) -> Result<()> {
        *tick = (*tick).max(self.map(true)?.tick().get());
        *tick = (*tick).max(self.operations()?.jobs.tick().get());
        *tick = (*tick).max(self.map(false)?.tick().get());
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceIdentity {
    pub furniture: construction::SourceIdentity,
    pub map: Manifest,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub total: u32,
    pub matched: u32,
    pub hidden: u32,
    pub missing: u32,
    pub wrong_shape: u32,
    pub liquid: u32,
    pub designated: u32,
    pub occupied: u32,
    pub unoccupied: u32,
}
impl Counts {
    fn record(&mut self, status: &'static str) {
        self.total += 1;
        match status {
            "matched" => self.matched += 1,
            "hidden" => self.hidden += 1,
            "missing" => self.missing += 1,
            "wrong_shape" => self.wrong_shape += 1,
            "liquid" => self.liquid += 1,
            "designated" => self.designated += 1,
            "occupied" => self.occupied += 1,
            _ => self.unoccupied += 1,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Deficit {
    pub position: [u32; 3],
    pub requirement: &'static str,
    pub status: &'static str,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TerrainAssessment {
    pub floors: Counts,
    pub walls: Counts,
    pub furnishings: Counts,
    pub deficits: Vec<Deficit>,
}
impl TerrainAssessment {
    pub fn condition_met(&self) -> bool {
        self.floors.total > 0
            && self.floors.total == self.floors.matched
            && self.walls.total == self.walls.matched
            && self.furnishings.total > 0
            && self.furnishings.total == self.furnishings.matched
    }
    pub fn deficit_count(&self) -> u32 {
        [&self.floors, &self.walls, &self.furnishings]
            .iter()
            .map(|counts| counts.total - counts.matched)
            .sum()
    }
    pub fn unknown(&self) -> bool {
        [&self.floors, &self.walls, &self.furnishings]
            .iter()
            .any(|counts| counts.hidden > 0 || counts.missing > 0)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Progress {
    pub definition_id: Digest32,
    pub construction: construction::Progress,
    pub terrain: TerrainAssessment,
    pub source: Option<SourceIdentity>,
    /// Monotonic authority floor, including a later invalidating observation.
    pub observed_tick_floor: u64,
}
impl Deref for Progress {
    type Target = construction::Progress;
    fn deref(&self) -> &Self::Target {
        &self.construction
    }
}
impl Progress {
    pub fn new(definition: &Definition) -> Self {
        let floor = definition
            .origin()
            .receipts()
            .iter()
            .fold(0, |tick, record| {
                tick.max(record.plan().before().tick())
                    .max(record.after().map_or(0, |after| after.tick()))
            });
        Self {
            definition_id: definition.id(),
            construction: construction::Progress::new(definition.goal()),
            terrain: TerrainAssessment::default(),
            source: None,
            observed_tick_floor: floor,
        }
    }
    pub fn terminal(&self) -> bool {
        self.construction.terminal()
    }
    pub fn begin_read(&self) -> Result<Self> {
        let mut next = self.clone();
        next.construction = next.construction.begin_read()?;
        Ok(next)
    }
    pub fn cancel(&self) -> Self {
        let mut next = self.clone();
        next.construction = next.construction.cancel();
        next
    }
    pub fn advance(
        &self,
        definition: &Definition,
        sample: &Sample,
        context: &OperationContext,
    ) -> Result<Self> {
        advance(self, definition, sample, context)
    }
}

fn cell_status(cell: Option<&Cell>, requirement: &'static str) -> &'static str {
    match cell {
        None | Some(Cell::Unallocated) => "missing",
        Some(Cell::Hidden) => "hidden",
        Some(Cell::Visible(tile)) => {
            if tile.liquid_depth != 0 || tile.magma {
                "liquid"
            } else if tile.dig_designation != 0 {
                "designated"
            } else if tile.shape
                != if requirement == "wall" {
                    Shape::Wall
                } else {
                    Shape::Floor
                }
            {
                "wrong_shape"
            } else if requirement == "wall"
                && (tile.building_occupancy != 0 || tile.unit_occupancy != 0)
            {
                "occupied"
            } else if requirement == "furnishing" && tile.building_occupancy == 0 {
                "unoccupied"
            } else {
                "matched"
            }
        }
    }
}

pub fn advance(
    state: &Progress,
    definition: &Definition,
    sample: &Sample,
    context: &OperationContext,
) -> Result<Progress> {
    advance_with_counter(state, definition, sample, context, &mut 0)
}

pub(crate) fn advance_with_counter(
    state: &Progress,
    definition: &Definition,
    sample: &Sample,
    context: &OperationContext,
    used: &mut u64,
) -> Result<Progress> {
    let start = Instant::now();
    require(
        state.definition_id == definition.id() && state.reading && !state.terminal(),
        "room sample belongs to another original goal or unowned transition",
    )?;
    let mut current = context.clone();
    current.anchor.tick = GameTick(current.anchor.tick.get().max(state.observed_tick_floor));
    if current.anchor.fortress_id
        != definition
            .origin()
            .definition()
            .binding()
            .fortress()
            .fortress_id()
    {
        return Err(DfmcpError::new(
            ErrorCode::CapabilityDenied,
            "joint room monitor belongs to another fortress",
        ));
    }
    current.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    let raw = sample.canonical_bytes()?;
    if current.budget.max_bytes < (raw.len() as u64).saturating_mul(5).saturating_add(16_384) {
        return Err(bounded());
    }
    let before = sample.map(true)?;
    let after = sample.map(false)?;
    let observed = sample.operations()?;
    let tick = before
        .tick()
        .get()
        .max(after.tick().get())
        .max(observed.jobs.tick().get());
    current.anchor.tick = GameTick(current.anchor.tick.get().max(tick));
    current.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    let map_entities = before.map.cells.len() + after.map.cells.len();
    let entities = 1 + observed.jobs.jobs.len() + observed.buildings.len() + observed.items.len();
    if map_entities + entities > current.budget.max_entities as usize {
        return Err(bounded());
    }
    *used = used
        .checked_add((map_entities + 6 * (entities + observed.attachments.len())) as u64)
        .ok_or_else(bounded)?;
    if *used > construction::MAX_WORK {
        return Err(bounded());
    }
    require(
        sample.before == sample.after && sample.before_capture == sample.after_capture,
        "joint room map endpoints must have identical complete bytes and source",
    )?;
    require(
        before.tick() == observed.jobs.tick(),
        "room map and operations must share one sampled game tick",
    )?;
    require(
        before.map.region == definition.room().terrain().region(),
        "room sample changed the complete original capture region",
    )?;
    let binding = definition.origin().definition().binding();
    let source = SourceIdentity {
        furniture: construction::SourceIdentity {
            furniture_generation: sample.furnishings.before.generation,
            operations: sample.furnishings.operations.clone(),
        },
        map: sample.before.clone(),
    };
    let invalidation = if before.world_folder != binding.fortress().folder()
        || before.site_id != binding.fortress().site()
        || before.map_dimensions != binding.dimensions()
    {
        Some("room_world_identity_changed")
    } else if sample.before.df_version != binding.df_version()
        || sample.before.dfhack_version != binding.dfhack_version()
        || sample.furnishings.before.df_version != binding.df_version()
        || sample.furnishings.before.dfhack_version != binding.dfhack_version()
        || sample.before.df_version != sample.furnishings.operations.df_version
        || sample.before.dfhack_version != sample.furnishings.operations.dfhack_version
    {
        Some("room_software_changed")
    } else if tick < state.observed_tick_floor {
        Some("room_game_clock_regressed")
    } else if state.source.as_ref().is_some_and(|prior| prior != &source) {
        Some("room_native_source_changed")
    } else {
        None
    };
    let mut terrain = TerrainAssessment::default();
    for (requirement, positions) in [
        ("floor", definition.room().terrain().floors()),
        ("wall", definition.room().terrain().walls()),
    ] {
        for &position in positions {
            let cell = before
                .map
                .region
                .index(position)
                .and_then(|index| before.map.cells.get(index));
            let status = cell_status(cell, requirement);
            if requirement == "floor" {
                terrain.floors.record(status);
            } else {
                terrain.walls.record(status);
            }
            if status != "matched" && terrain.deficits.len() < MAX_DEFICITS {
                terrain.deficits.push(Deficit {
                    position,
                    requirement,
                    status,
                });
            }
        }
    }
    for record in definition.goal().records() {
        let position = record
            .insertion()
            .ok_or_else(|| invalid("room receipt lacks its original placement"))?
            .position();
        let cell = before
            .map
            .region
            .index(position)
            .and_then(|index| before.map.cells.get(index));
        let status = cell_status(cell, "furnishing");
        terrain.furnishings.record(status);
        if status != "matched" && terrain.deficits.len() < MAX_DEFICITS {
            terrain.deficits.push(Deficit {
                position,
                requirement: "furnishing",
                status,
            });
        }
    }
    let unmet = if !before.paused || !observed.jobs.paused {
        Some("room_capture_unpaused")
    } else if terrain.unknown() {
        Some("room_terrain_unknown")
    } else if !terrain.condition_met() {
        Some("room_terrain_not_met")
    } else {
        None
    };
    current.budget.max_entities = current
        .budget
        .max_entities
        .saturating_sub(map_entities as u32);
    current.budget.max_wall_millis = current
        .budget
        .max_wall_millis
        .checked_sub(start.elapsed().as_millis() as u64)
        .filter(|remaining| *remaining > 0)
        .ok_or_else(bounded)?;
    let mut construction = construction::advance_joint(
        &state.construction,
        definition.goal(),
        &sample.furnishings,
        &current,
        used,
        construction::JointCondition {
            capture: hash(b"dfmcp.joint-room-sample/1\0", &raw),
            unmet,
            invalidation,
        },
    )?;
    if construction.phase == "satisfied" {
        construction.reason = "joint_original_room_sampled_condition";
    }
    Ok(Progress {
        definition_id: definition.id(),
        construction,
        terrain,
        source: Some(source),
        observed_tick_floor: state.observed_tick_floor.max(tick),
    })
}

#[cfg(test)]
mod tests;
