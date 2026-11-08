#![forbid(unsafe_code)]
//! Sealed evidence for one Carpenter's workshop in the isolated workshop/1.20 profile.
//!
//! This generation is separate from furniture/1.19: it shares no capture,
//! plan, token, receipt or native writer bytes. `Placed` proves historical
//! stage-zero registration of the original building and its construction job,
//! never completion. Completion is only a fresh `ReadWorkshopProgress` sample
//! evaluated against the original receipt under exact source continuity.
//! Decoding evidence never recreates dispatch authority.
//! Bead: dfmcp-reality-wiring-proof-4bf.14.

pub mod acknowledgement;
pub mod clock;
pub mod journal;
pub mod rpc;
pub use journal::production_origin;
mod tree_material;
pub use tree_material::{WorkshopMaterialDiscovery, WorkshopMaterialOffer};

use crate::bounded_run::{Reader, error, hash, require, validate_key};
pub use crate::order_run::FortressIdentity;
use crate::order_run::{field, put_field, text};
use dfmcp_core::{Digest32, ErrorCode, FortressId, Result};
use std::net::SocketAddr;

pub const MAX_CAPTURE_BYTES: usize = 2048;
pub const MAX_RECORD_BYTES: usize = 6144;
pub const MAX_PROGRESS_BYTES: usize = 1024;
pub const MAX_PLAN_BYTES: usize = 8 + 2 + 128 + 2 + MAX_CAPTURE_BYTES;
pub const MAX_WORKERS: usize = 32;
pub const MAX_NATIVE_RECORDS: u16 = 64;
pub const INSERTION_BYTES: usize = 77;
pub const ALL_TILES: u16 = 0x1ff;
const MAX_ID: u32 = i32::MAX as u32;
const COMPUTED_ITEM_FLAGS: u32 = (1 << 28) | (1 << 29);
const MAX_NATIVE_TICK: u64 = crate::bounded_run::MAX_NATIVE_TICK;

fn invalid() -> dfmcp_core::DfmcpError {
    error(ErrorCode::AdapterRejected, "invalid workshop/1.20 evidence")
}
fn coordinates(r: &mut Reader<'_>) -> Result<[u32; 3]> {
    let out = [r.u32()?, r.u32()?, r.u32()?];
    require(
        out.iter().all(|n| *n < 32768),
        "workshop coordinate outside bounds",
    )?;
    Ok(out)
}
fn put(out: &mut Vec<u8>, values: impl IntoIterator<Item = u32>) {
    for n in values {
        out.extend_from_slice(&n.to_be_bytes());
    }
}
fn flags(out: &mut Vec<u8>, values: impl IntoIterator<Item = bool>) {
    out.extend(values.into_iter().map(u8::from));
}

/// Global retention status from the latest validated native reply. Historical
/// source metadata only; never a grant or proof that a key is absent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WorkshopNativeSummary {
    unresolved: bool,
    retained_records: u16,
}
impl WorkshopNativeSummary {
    pub fn new(unresolved: bool, retained_records: u16) -> Result<Self> {
        require(
            retained_records <= MAX_NATIVE_RECORDS && (!unresolved || retained_records > 0),
            "invalid workshop native retention summary",
        )?;
        Ok(Self {
            unresolved,
            retained_records,
        })
    }
    pub fn unresolved(self) -> bool {
        self.unresolved
    }
    pub fn retained_records(self) -> u16 {
        self.retained_records
    }
    pub fn prepare_available(self) -> bool {
        !self.unresolved && self.retained_records < MAX_NATIVE_RECORDS
    }
}

/// One exact log and the center of the fixed 3x3 Carpenter's workshop footprint.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WorkshopSelection {
    item: u32,
    center: [u32; 3],
}
impl WorkshopSelection {
    pub fn new(item: u32, center: [u32; 3]) -> Result<Self> {
        require(
            item < MAX_ID
                && (1..32767).contains(&center[0])
                && (1..32767).contains(&center[1])
                && center[2] < 32768,
            "workshop selection needs one exact item and a complete 3x3 footprint",
        )?;
        Ok(Self { item, center })
    }
    pub fn item_id(self) -> u32 {
        self.item
    }
    pub fn center(self) -> [u32; 3] {
        self.center
    }
    /// Inclusive footprint bounds `[x1, y1, x2, y2, z]`.
    pub fn footprint(self) -> [u32; 5] {
        let [x, y, z] = self.center;
        [x - 1, y - 1, x + 1, y + 1, z]
    }
    pub fn contains(self, p: [u32; 3]) -> bool {
        let [x1, y1, x2, y2, z] = self.footprint();
        p[2] == z && (x1..=x2).contains(&p[0]) && (y1..=y2).contains(&p[1])
    }
    pub fn canonical_bytes(self) -> Vec<u8> {
        let mut out = vec![1];
        put(
            &mut out,
            [self.item, self.center[0], self.center[1], self.center[2]],
        );
        out
    }
    fn read(r: &mut Reader<'_>) -> Result<Self> {
        require(r.byte()? == 1, "workshop kind must be Carpenters")?;
        Self::new(r.u32()?, coordinates(r)?)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VisibleWorkshopTile {
    tiletype: u32,
    shape: u8,
    liquid: u8,
    dig: u8,
    pile: bool,
    smooth: u8,
    occupancy_other: u32,
    occupied: bool,
    building: Option<u32>,
    zone: bool,
}
impl VisibleWorkshopTile {
    pub fn shape(&self) -> u8 {
        self.shape
    }
    pub fn liquid(&self) -> u8 {
        self.liquid
    }
    pub fn occupied(&self) -> bool {
        self.occupied
    }
    pub fn building_id(&self) -> Option<u32> {
        self.building
    }
    pub fn zone(&self) -> bool {
        self.zone
    }
    pub fn occupancy_other(&self) -> u32 {
        self.occupancy_other
    }
}
/// Missing/hidden variants contain no native attributes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkshopTile {
    Missing,
    Hidden,
    Visible(VisibleWorkshopTile),
}
impl WorkshopTile {
    fn read(r: &mut Reader<'_>) -> Result<Self> {
        match r.byte()? {
            0 => Ok(Self::Missing),
            1 => Ok(Self::Hidden),
            2 => {
                let tiletype = r.u32()?;
                let shape = r.byte()?;
                let liquid = r.byte()?;
                let dig = r.byte()?;
                let pile = r.boolean()?;
                let smooth = r.byte()?;
                let occupancy_other = r.u32()?;
                let occupied = r.boolean()?;
                let building = if r.boolean()? { Some(r.u32()?) } else { None };
                let zone = r.boolean()?;
                require(
                    tiletype <= MAX_ID
                        && shape <= 8
                        && liquid <= 7
                        && dig <= 7
                        && smooth <= 3
                        && building.is_none_or(|n| n < MAX_ID),
                    "invalid visible workshop tile",
                )?;
                Ok(Self::Visible(VisibleWorkshopTile {
                    tiletype,
                    shape,
                    liquid,
                    dig,
                    pile,
                    smooth,
                    occupancy_other,
                    occupied,
                    building,
                    zone,
                }))
            }
            _ => Err(invalid()),
        }
    }
    fn append(&self, out: &mut Vec<u8>) {
        match self {
            Self::Missing => out.push(0),
            Self::Hidden => out.push(1),
            Self::Visible(v) => {
                out.push(2);
                put(out, [v.tiletype]);
                out.extend_from_slice(&[v.shape, v.liquid, v.dig, u8::from(v.pile), v.smooth]);
                put(out, [v.occupancy_other]);
                flags(out, [v.occupied, v.building.is_some()]);
                if let Some(n) = v.building {
                    put(out, [n]);
                }
                out.push(u8::from(v.zone));
            }
        }
    }
    pub fn visible(&self) -> bool {
        matches!(self, Self::Visible(_))
    }
    pub fn dry(&self) -> bool {
        matches!(self, Self::Visible(v) if v.liquid == 0)
    }
    pub fn empty_floor(&self) -> bool {
        matches!(self, Self::Visible(v) if v.liquid == 0 && v.shape == 3 && v.dig == 0 && !v.pile
            && v.smooth == 0 && !v.occupied && v.building.is_none() && v.occupancy_other == 0 && !v.zone)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VisibleWorkshopItem {
    position: [u32; 3],
    log: bool,
    wood: bool,
    native_type: u32,
    subtype: i32,
    material: i32,
    material_index: i32,
    quality: u32,
    wear: u32,
    other_flags: u32,
    other_refs: u32,
    on_ground: bool,
    in_job: bool,
    jobs: Vec<u32>,
    ground: WorkshopTile,
    reachable: bool,
}
impl VisibleWorkshopItem {
    pub fn position(&self) -> [u32; 3] {
        self.position
    }
    pub fn log(&self) -> bool {
        self.log
    }
    pub fn wood(&self) -> bool {
        self.wood
    }
    pub fn native_type(&self) -> u32 {
        self.native_type
    }
    pub fn material(&self) -> i32 {
        self.material
    }
    pub fn material_index(&self) -> i32 {
        self.material_index
    }
    pub fn other_flags(&self) -> u32 {
        self.other_flags
    }
    pub fn other_refs(&self) -> u32 {
        self.other_refs
    }
    pub fn on_ground(&self) -> bool {
        self.on_ground
    }
    pub fn in_job(&self) -> bool {
        self.in_job
    }
    pub fn jobs(&self) -> &[u32] {
        &self.jobs
    }
    pub fn reachable(&self) -> bool {
        self.reachable
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkshopItem {
    Missing,
    Hidden,
    Visible(VisibleWorkshopItem),
}
impl WorkshopItem {
    fn read(r: &mut Reader<'_>) -> Result<Self> {
        match r.byte()? {
            0 => Ok(Self::Missing),
            1 => Ok(Self::Hidden),
            2 => {
                let position = coordinates(r)?;
                let log = r.boolean()?;
                let wood = r.boolean()?;
                let native_type = r.u32()?;
                let subtype = r.u32()? as i32;
                let material = r.u32()? as i32;
                let material_index = r.u32()? as i32;
                let quality = r.u32()?;
                let wear = r.u32()?;
                let other_flags = r.u32()?;
                let other_refs = r.u32()?;
                let on_ground = r.boolean()?;
                let in_job = r.boolean()?;
                let n = r.byte()?;
                require(
                    n <= 8
                        && native_type <= MAX_ID
                        && quality <= MAX_ID
                        && wear <= MAX_ID
                        && other_refs <= 4096,
                    "invalid selected workshop item",
                )?;
                let mut jobs = Vec::with_capacity(usize::from(n));
                for _ in 0..n {
                    let id = r.u32()?;
                    require(
                        id < MAX_ID && jobs.last().is_none_or(|old| *old < id),
                        "unordered or invalid workshop item job references",
                    )?;
                    jobs.push(id);
                }
                let ground = WorkshopTile::read(r)?;
                require(
                    ground.visible(),
                    "visible workshop item needs visible ground",
                )?;
                let reachable = r.boolean()?;
                Ok(Self::Visible(VisibleWorkshopItem {
                    position,
                    log,
                    wood,
                    native_type,
                    subtype,
                    material,
                    material_index,
                    quality,
                    wear,
                    other_flags,
                    other_refs,
                    on_ground,
                    in_job,
                    jobs,
                    ground,
                    reachable,
                }))
            }
            _ => Err(invalid()),
        }
    }
    fn append(&self, out: &mut Vec<u8>) {
        match self {
            Self::Missing => out.push(0),
            Self::Hidden => out.push(1),
            Self::Visible(v) => {
                out.push(2);
                put(out, v.position);
                flags(out, [v.log, v.wood]);
                put(
                    out,
                    [
                        v.native_type,
                        v.subtype as u32,
                        v.material as u32,
                        v.material_index as u32,
                        v.quality,
                        v.wear,
                        v.other_flags,
                        v.other_refs,
                    ],
                );
                flags(out, [v.on_ground, v.in_job]);
                out.push(v.jobs.len() as u8);
                put(out, v.jobs.iter().copied());
                v.ground.append(out);
                out.push(u8::from(v.reachable));
            }
        }
    }
    /// One unclaimed, unforbidden, unhidden plant-WOOD log on dry visible floor.
    pub fn available(&self) -> bool {
        matches!(self, Self::Visible(v) if v.log && v.wood && v.on_ground && !v.in_job
            && v.other_flags & !COMPUTED_ITEM_FLAGS == 0 && v.other_refs == 0 && v.jobs.is_empty()
            && v.wear == 0 && v.material >= 0 && v.reachable
            && matches!(&v.ground, WorkshopTile::Visible(t) if t.liquid == 0 && t.shape == 3
                && !t.occupied && t.building.is_none()))
    }
}

/// One living active citizen. Every field is an observed SDK predicate, not a
/// scheduling certificate; no assignment is predicted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WorkshopWorker {
    id: u32,
    position: [u32; 3],
    visible: bool,
    adult: bool,
    sane: bool,
    labor: bool,
    labor_valid: bool,
    job_available: bool,
    reachable: bool,
}
impl WorkshopWorker {
    fn read(r: &mut Reader<'_>) -> Result<Self> {
        let id = r.u32()?;
        require(id < MAX_ID, "invalid workshop worker ID")?;
        Ok(Self {
            id,
            position: coordinates(r)?,
            visible: r.boolean()?,
            adult: r.boolean()?,
            sane: r.boolean()?,
            labor: r.boolean()?,
            labor_valid: r.boolean()?,
            job_available: r.boolean()?,
            reachable: r.boolean()?,
        })
    }
    fn append(&self, out: &mut Vec<u8>) {
        put(out, [self.id]);
        put(out, self.position);
        flags(
            out,
            [
                self.visible,
                self.adult,
                self.sane,
                self.labor,
                self.labor_valid,
                self.job_available,
                self.reachable,
            ],
        );
    }
    pub fn id(&self) -> u32 {
        self.id
    }
    pub fn position(&self) -> [u32; 3] {
        self.position
    }
    pub fn carpenter_labor_enabled(&self) -> bool {
        self.labor
    }
    pub fn labor_valid(&self) -> bool {
        self.labor_valid
    }
    pub fn job_available(&self) -> bool {
        self.job_available
    }
    pub fn reachable(&self) -> bool {
        self.reachable
    }
    pub fn adult(&self) -> bool {
        self.adult
    }
    pub fn sane(&self) -> bool {
        self.sane
    }
    pub fn visible(&self) -> bool {
        self.visible
    }
    pub fn eligible(&self) -> bool {
        self.visible
            && self.adult
            && self.sane
            && self.labor
            && self.labor_valid
            && self.job_available
            && self.reachable
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkshopBlocker {
    GameNotPaused,
    FootprintNotFree,
    FootprintNotSupported,
    WorkerCensusIncomplete,
    SequenceExhausted,
    BuildingIdExhausted,
    JobIdExhausted,
    BuildingCapacityExhausted,
    FootprintNotFullyVisible,
    FootprintNotEmptyFloor,
    ItemUnavailable,
    ItemInsideFootprint,
    NoEligibleWorker,
}
impl WorkshopBlocker {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::GameNotPaused => "game_not_paused",
            Self::FootprintNotFree => "footprint_not_free",
            Self::FootprintNotSupported => "footprint_not_supported",
            Self::WorkerCensusIncomplete => "worker_census_incomplete",
            Self::SequenceExhausted => "sequence_exhausted",
            Self::BuildingIdExhausted => "building_id_exhausted",
            Self::JobIdExhausted => "job_id_exhausted",
            Self::BuildingCapacityExhausted => "building_capacity_exhausted",
            Self::FootprintNotFullyVisible => "footprint_not_fully_visible",
            Self::FootprintNotEmptyFloor => "footprint_not_empty_floor",
            Self::ItemUnavailable => "material_unavailable",
            Self::ItemInsideFootprint => "material_inside_footprint",
            Self::NoEligibleWorker => "no_observed_available_carpenter",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkshopCapture {
    bytes: Vec<u8>,
    generation: u64,
    sequence: u64,
    tick: u64,
    fortress: FortressIdentity,
    dimensions: [u32; 3],
    next_building: u32,
    next_job: u32,
    building_count: u32,
    zone_count: u32,
    paused: bool,
    free_footprint: bool,
    supported: bool,
    census_complete: bool,
    selection: WorkshopSelection,
    tiles: Vec<WorkshopTile>,
    item: WorkshopItem,
    workers: Vec<WorkshopWorker>,
}
impl WorkshopCapture {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        require(
            bytes.len() <= MAX_CAPTURE_BYTES,
            "workshop capture exceeds 2 KiB",
        )?;
        let mut r = Reader(bytes);
        require(r.take(8)? == b"DFMWC020", "not a workshop/1.20 capture")?;
        let generation = r.u64()?;
        let sequence = r.u64()?;
        let tick = r.u64()?;
        let site = r.u32()?;
        let dimensions = [r.u32()?, r.u32()?, r.u32()?];
        let next_building = r.u32()?;
        let next_job = r.u32()?;
        let building_count = r.u32()?;
        let zone_count = r.u32()?;
        let folder = text(field(&mut r, 512)?, 512)?;
        let fortress = FortressIdentity::new(&folder, site)?;
        let paused = r.boolean()?;
        let free_footprint = r.boolean()?;
        let supported = r.boolean()?;
        let census_complete = r.boolean()?;
        let selection = WorkshopSelection::read(&mut r)?;
        require(
            generation > 0
                && generation < u64::MAX
                && sequence < u64::MAX
                && tick <= MAX_NATIVE_TICK
                && dimensions.iter().all(|n| (1..=32768).contains(n))
                && next_building <= MAX_ID
                && next_job <= MAX_ID
                && building_count <= 65536
                && zone_count <= building_count,
            "invalid workshop source, clock, map or ID horizon",
        )?;
        let [x, y, z] = selection.center;
        require(
            x + 1 < dimensions[0] && y + 1 < dimensions[1] && z < dimensions[2],
            "workshop footprint outside map",
        )?;
        let mut tiles = Vec::with_capacity(9);
        for _ in 0..9 {
            tiles.push(WorkshopTile::read(&mut r)?);
        }
        let item = WorkshopItem::read(&mut r)?;
        if let WorkshopItem::Visible(v) = &item {
            require(
                v.position.iter().zip(dimensions).all(|(p, d)| *p < d),
                "workshop item outside map",
            )?;
        }
        let n = usize::from(r.byte()?);
        require(
            n <= MAX_WORKERS && (census_complete || n == 0),
            "invalid workshop worker census",
        )?;
        let mut workers: Vec<WorkshopWorker> = Vec::with_capacity(n);
        for _ in 0..n {
            let w = WorkshopWorker::read(&mut r)?;
            require(
                workers.last().is_none_or(|old| old.id < w.id)
                    && w.position.iter().zip(dimensions).all(|(p, d)| *p < d),
                "unordered or out-of-map workshop worker",
            )?;
            workers.push(w);
        }
        r.finish()?;
        let out = Self {
            bytes: bytes.to_vec(),
            generation,
            sequence,
            tick,
            fortress,
            dimensions,
            next_building,
            next_job,
            building_count,
            zone_count,
            paused,
            free_footprint,
            supported,
            census_complete,
            selection,
            tiles,
            item,
            workers,
        };
        require(out.encode() == bytes, "noncanonical workshop capture")?;
        Ok(out)
    }
    fn encode(&self) -> Vec<u8> {
        let mut out = b"DFMWC020".to_vec();
        for n in [self.generation, self.sequence, self.tick] {
            out.extend_from_slice(&n.to_be_bytes());
        }
        put(
            &mut out,
            [
                self.site(),
                self.dimensions[0],
                self.dimensions[1],
                self.dimensions[2],
                self.next_building,
                self.next_job,
                self.building_count,
                self.zone_count,
            ],
        );
        put_field(&mut out, self.folder().as_bytes());
        flags(
            &mut out,
            [
                self.paused,
                self.free_footprint,
                self.supported,
                self.census_complete,
            ],
        );
        out.extend_from_slice(&self.selection.canonical_bytes());
        for tile in &self.tiles {
            tile.append(&mut out);
        }
        self.item.append(&mut out);
        out.push(self.workers.len() as u8);
        for w in &self.workers {
            w.append(&mut out);
        }
        out
    }
    pub fn canonical_bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub fn witness(&self) -> Digest32 {
        Digest32::of_bytes(&self.bytes)
    }
    pub fn fortress(&self) -> &FortressIdentity {
        &self.fortress
    }
    pub fn fortress_id(&self) -> FortressId {
        self.fortress.fortress_id()
    }
    pub fn folder(&self) -> &str {
        self.fortress.folder()
    }
    pub fn site(&self) -> u32 {
        self.fortress.site()
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn sequence(&self) -> u64 {
        self.sequence
    }
    pub fn tick(&self) -> u64 {
        self.tick
    }
    pub fn dimensions(&self) -> [u32; 3] {
        self.dimensions
    }
    pub fn next_building_id(&self) -> u32 {
        self.next_building
    }
    pub fn next_job_id(&self) -> u32 {
        self.next_job
    }
    pub fn building_count(&self) -> u32 {
        self.building_count
    }
    pub fn zone_count(&self) -> u32 {
        self.zone_count
    }
    pub fn paused(&self) -> bool {
        self.paused
    }
    pub fn free_footprint(&self) -> bool {
        self.free_footprint
    }
    pub fn supported(&self) -> bool {
        self.supported
    }
    pub fn census_complete(&self) -> bool {
        self.census_complete
    }
    pub fn selection(&self) -> WorkshopSelection {
        self.selection
    }
    pub fn tiles(&self) -> &[WorkshopTile] {
        &self.tiles
    }
    pub fn item(&self) -> &WorkshopItem {
        &self.item
    }
    pub fn workers(&self) -> &[WorkshopWorker] {
        &self.workers
    }
    pub fn item_position(&self) -> Option<[u32; 3]> {
        match &self.item {
            WorkshopItem::Visible(v) => Some(v.position),
            _ => None,
        }
    }
    pub fn item_inside_footprint(&self) -> bool {
        self.item_position()
            .is_some_and(|p| self.selection.contains(p))
    }
    pub fn same_source(&self, other: &Self) -> bool {
        self.generation == other.generation
            && self.fortress == other.fortress
            && self.dimensions == other.dimensions
    }
    pub fn blockers(&self) -> Vec<WorkshopBlocker> {
        use WorkshopBlocker::*;
        [
            (!self.paused, GameNotPaused),
            (!self.free_footprint, FootprintNotFree),
            (!self.supported, FootprintNotSupported),
            (!self.census_complete, WorkerCensusIncomplete),
            (self.sequence >= u64::MAX - 1, SequenceExhausted),
            (self.next_building >= MAX_ID, BuildingIdExhausted),
            (self.next_job >= MAX_ID, JobIdExhausted),
            (self.building_count >= 65536, BuildingCapacityExhausted),
            (
                !self.tiles.iter().all(WorkshopTile::visible),
                FootprintNotFullyVisible,
            ),
            (
                !self.tiles.iter().all(WorkshopTile::empty_floor),
                FootprintNotEmptyFloor,
            ),
            (!self.item.available(), ItemUnavailable),
            (self.item_inside_footprint(), ItemInsideFootprint),
            (
                !self.workers.iter().any(WorkshopWorker::eligible),
                NoEligibleWorker,
            ),
        ]
        .into_iter()
        .filter_map(|(blocked, reason)| blocked.then_some(reason))
        .collect()
    }
    pub fn eligible(&self) -> bool {
        self.blockers().is_empty()
    }
    /// Exact historical stage-zero effect required by the native contract.
    pub fn expected_after(&self) -> Result<Self> {
        require(self.eligible(), "workshop plan has unmet preconditions")?;
        let mut out = self.clone();
        out.sequence += 1;
        out.next_building += 1;
        out.next_job += 1;
        out.building_count += 1;
        out.free_footprint = false;
        for tile in &mut out.tiles {
            if let WorkshopTile::Visible(v) = tile {
                v.occupied = true;
                v.building = Some(self.next_building);
            }
        }
        if let WorkshopItem::Visible(v) = &mut out.item {
            v.in_job = true;
            v.jobs = vec![self.next_job];
        }
        Self::decode(&out.encode())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkshopPlan {
    key: String,
    before: WorkshopCapture,
    digest: Digest32,
    token: [u8; 16],
}
impl WorkshopPlan {
    pub fn new(key: &str, before: WorkshopCapture) -> Result<Self> {
        validate_key(key)?;
        require(before.eligible(), "ineligible workshop plan")?;
        let mut out = before.selection.canonical_bytes();
        out.extend_from_slice(before.witness().as_bytes());
        let digest = hash(b"dfmcp-workshop-plan/1", &out);
        let mut out = Vec::new();
        put_field(&mut out, key.as_bytes());
        out.extend_from_slice(digest.as_bytes());
        let full = hash(b"dfmcp-workshop-token/1", &out);
        let mut token = [0; 16];
        token.copy_from_slice(&full.as_bytes()[..16]);
        Ok(Self {
            key: key.to_owned(),
            before,
            digest,
            token,
        })
    }
    pub fn key(&self) -> &str {
        &self.key
    }
    pub fn before(&self) -> &WorkshopCapture {
        &self.before
    }
    pub fn digest(&self) -> Digest32 {
        self.digest
    }
    pub fn token(&self) -> &[u8; 16] {
        &self.token
    }
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut out = b"DFMWL020".to_vec();
        put_field(&mut out, self.key.as_bytes());
        put_field(&mut out, self.before.canonical_bytes());
        out
    }
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        require(bytes.len() <= MAX_PLAN_BYTES, "workshop plan exceeds bound")?;
        let mut r = Reader(bytes);
        require(r.take(8)? == b"DFMWL020", "invalid retained workshop plan")?;
        let key = text(field(&mut r, 128)?, 128)?;
        let before = WorkshopCapture::decode(field(&mut r, MAX_CAPTURE_BYTES)?)?;
        r.finish()?;
        Self::new(&key, before)
    }
}

/// Independent native readback of the original building, job, log and links.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkshopInsertion {
    bytes: Vec<u8>,
    building: u32,
    job: u32,
    item: u32,
    geometry: [u32; 7],
    material: i32,
    material_index: i32,
    stage: u32,
    max_stage: u32,
    building_flags: u32,
    has_design: bool,
    setting_occupancy: bool,
    planned_tiles: u16,
    linked: bool,
    construct_job: bool,
    exact_item_link: bool,
    suspended: bool,
}
impl WorkshopInsertion {
    fn decode(bytes: &[u8]) -> Result<Self> {
        require(
            bytes.len() == INSERTION_BYTES,
            "invalid workshop insertion width",
        )?;
        let mut r = Reader(bytes);
        require(
            r.take(8)? == b"DFMWI020",
            "invalid workshop insertion profile",
        )?;
        let building = r.u32()?;
        let job = r.u32()?;
        let item = r.u32()?;
        require(r.byte()? == 1, "workshop insertion kind differs")?;
        let mut geometry = [0; 7];
        for n in &mut geometry {
            *n = r.u32()?;
        }
        let material = r.u32()? as i32;
        let material_index = r.u32()? as i32;
        let stage = r.u32()?;
        let max_stage = r.u32()?;
        let building_flags = r.u32()?;
        let has_design = r.boolean()?;
        let setting_occupancy = r.boolean()?;
        let planned_tiles = u16::from_be_bytes(r.array()?);
        let linked = r.boolean()?;
        let construct_job = r.boolean()?;
        let exact_item_link = r.boolean()?;
        let suspended = r.boolean()?;
        r.finish()?;
        require(
            [building, job, item].iter().all(|id| *id < MAX_ID)
                && geometry.iter().all(|n| *n < 32768)
                && (1..=32).contains(&max_stage)
                && stage <= max_stage
                && planned_tiles <= ALL_TILES,
            "invalid workshop insertion IDs, geometry or stages",
        )?;
        Ok(Self {
            bytes: bytes.to_vec(),
            building,
            job,
            item,
            geometry,
            material,
            material_index,
            stage,
            max_stage,
            building_flags,
            has_design,
            setting_occupancy,
            planned_tiles,
            linked,
            construct_job,
            exact_item_link,
            suspended,
        })
    }
    fn matches(&self, before: &WorkshopCapture) -> bool {
        let WorkshopItem::Visible(item) = &before.item else {
            return false;
        };
        let [x1, y1, x2, y2, z] = before.selection.footprint();
        let [cx, cy, _] = before.selection.center;
        self.building == before.next_building
            && self.job == before.next_job
            && self.item == before.selection.item
            && self.geometry == [x1, y1, x2, y2, z, cx, cy]
            && self.material == item.material
            && self.material_index == item.material_index
            && self.stage == 0
            && self.setting_occupancy
            && self.planned_tiles == ALL_TILES
            && self.linked
            && self.construct_job
            && self.exact_item_link
            && !self.suspended
    }
    pub fn canonical_bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub fn building_id(&self) -> u32 {
        self.building
    }
    pub fn job_id(&self) -> u32 {
        self.job
    }
    pub fn item_id(&self) -> u32 {
        self.item
    }
    /// `[x1, y1, x2, y2, z, center_x, center_y]`.
    pub fn geometry(&self) -> [u32; 7] {
        self.geometry
    }
    pub fn material(&self) -> i32 {
        self.material
    }
    pub fn material_index(&self) -> i32 {
        self.material_index
    }
    pub fn stage(&self) -> u32 {
        self.stage
    }
    pub fn max_stage(&self) -> u32 {
        self.max_stage
    }
    pub fn building_flags(&self) -> u32 {
        self.building_flags
    }
    pub fn has_design(&self) -> bool {
        self.has_design
    }
    pub fn setting_occupancy(&self) -> bool {
        self.setting_occupancy
    }
    pub fn planned_tiles(&self) -> u16 {
        self.planned_tiles
    }
    pub fn suspended(&self) -> bool {
        self.suspended
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum WorkshopPhase {
    Prepared = 0,
    Indeterminate = 1,
    Placed = 2,
    Refused = 3,
    Cancelled = 4,
}
impl WorkshopPhase {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Prepared => "prepared",
            Self::Indeterminate => "indeterminate",
            Self::Placed => "placed",
            Self::Refused => "refused",
            Self::Cancelled => "cancelled",
        }
    }
    pub fn resolved(self) -> bool {
        matches!(self, Self::Placed | Self::Refused | Self::Cancelled)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum WorkshopReason {
    None = 0,
    Stale = 1,
    Expired = 2,
    SourceChanged = 3,
    Cancelled = 4,
    NativeFailure = 5,
}
impl WorkshopReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Stale => "stale",
            Self::Expired => "expired",
            Self::SourceChanged => "source_changed",
            Self::Cancelled => "cancelled",
            Self::NativeFailure => "native_failure",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkshopRecord {
    bytes: Vec<u8>,
    plan: WorkshopPlan,
    phase: WorkshopPhase,
    reason: WorkshopReason,
    after: Option<WorkshopCapture>,
    insertion: Option<WorkshopInsertion>,
    receipt: Digest32,
}
impl WorkshopRecord {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        require(
            bytes.len() <= MAX_RECORD_BYTES,
            "workshop receipt exceeds 6 KiB",
        )?;
        let mut r = Reader(bytes);
        require(
            r.take(8)? == b"DFMWR020",
            "invalid workshop receipt profile",
        )?;
        let key = text(field(&mut r, 128)?, 128)?;
        let before = WorkshopCapture::decode(field(&mut r, MAX_CAPTURE_BYTES)?)?;
        let plan = WorkshopPlan::new(&key, before)?;
        require(
            r.take(32)? == plan.digest.as_bytes() && r.take(16)? == plan.token,
            "workshop receipt plan or token differs",
        )?;
        let phase = match r.byte()? {
            0 => WorkshopPhase::Prepared,
            1 => WorkshopPhase::Indeterminate,
            2 => WorkshopPhase::Placed,
            3 => WorkshopPhase::Refused,
            4 => WorkshopPhase::Cancelled,
            _ => return Err(invalid()),
        };
        let reason = match r.byte()? {
            0 => WorkshopReason::None,
            1 => WorkshopReason::Stale,
            2 => WorkshopReason::Expired,
            3 => WorkshopReason::SourceChanged,
            4 => WorkshopReason::Cancelled,
            5 => WorkshopReason::NativeFailure,
            _ => return Err(invalid()),
        };
        let attempted = r.boolean()?;
        let has_after = r.boolean()?;
        let valid_reason = match phase {
            WorkshopPhase::Prepared | WorkshopPhase::Placed => reason == WorkshopReason::None,
            WorkshopPhase::Indeterminate => reason == WorkshopReason::NativeFailure,
            WorkshopPhase::Refused => matches!(
                reason,
                WorkshopReason::Stale | WorkshopReason::Expired | WorkshopReason::SourceChanged
            ),
            WorkshopPhase::Cancelled => reason == WorkshopReason::Cancelled,
        };
        require(
            valid_reason
                && attempted
                    == matches!(phase, WorkshopPhase::Placed | WorkshopPhase::Indeterminate)
                && has_after == (phase == WorkshopPhase::Placed),
            "contradictory workshop receipt state",
        )?;
        let (after, insertion) = if has_after {
            let after = WorkshopCapture::decode(field(&mut r, MAX_CAPTURE_BYTES)?)?;
            let insertion = WorkshopInsertion::decode(field(&mut r, INSERTION_BYTES)?)?;
            require(
                after == plan.before.expected_after()? && insertion.matches(&plan.before),
                "workshop receipt lacks exact complete native effect readback",
            )?;
            (Some(after), Some(insertion))
        } else {
            (None, None)
        };
        let receipt = Digest32::from_bytes(r.array()?);
        r.finish()?;
        require(
            receipt == hash(b"dfmcp-workshop-receipt/1", &bytes[..bytes.len() - 32]),
            "workshop receipt integrity failed",
        )?;
        Ok(Self {
            bytes: bytes.to_vec(),
            plan,
            phase,
            reason,
            after,
            insertion,
            receipt,
        })
    }
    pub fn canonical_bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub fn plan(&self) -> &WorkshopPlan {
        &self.plan
    }
    pub fn phase(&self) -> WorkshopPhase {
        self.phase
    }
    pub fn reason(&self) -> WorkshopReason {
        self.reason
    }
    pub fn attempted(&self) -> bool {
        matches!(
            self.phase,
            WorkshopPhase::Indeterminate | WorkshopPhase::Placed
        )
    }
    pub fn resolved(&self) -> bool {
        self.phase.resolved()
    }
    pub fn after(&self) -> Option<&WorkshopCapture> {
        self.after.as_ref()
    }
    pub fn insertion(&self) -> Option<&WorkshopInsertion> {
        self.insertion.as_ref()
    }
    pub fn receipt(&self) -> Digest32 {
        self.receipt
    }
    pub fn verify_plan(&self, plan: &WorkshopPlan) -> Result<()> {
        require(
            &self.plan == plan,
            "workshop record belongs to another plan",
        )
    }
    /// Prepared may become any outcome; Indeterminate may become only the exact
    /// original Placed receipt; every other outcome is immutable.
    pub fn validate_successor(&self, next: &Self) -> Result<()> {
        require(
            self.plan == next.plan
                && (self.phase == WorkshopPhase::Prepared
                    || self == next
                    || (self.phase == WorkshopPhase::Indeterminate
                        && next.phase == WorkshopPhase::Placed)),
            "immutable workshop outcome changed; preserve uncertainty",
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkshopPreparation {
    record: WorkshopRecord,
    replayed: bool,
}
impl WorkshopPreparation {
    pub fn new(record: WorkshopRecord, replayed: bool) -> Result<Self> {
        require(
            replayed || record.phase == WorkshopPhase::Prepared,
            "fresh workshop preparation contains history",
        )?;
        Ok(Self { record, replayed })
    }
    pub fn record(&self) -> &WorkshopRecord {
        &self.record
    }
    pub fn replayed(&self) -> bool {
        self.replayed
    }
}

/// Present-building facts from one fresh ReadWorkshopProgress reply.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WorkshopBuildingFacts {
    carpenters: bool,
    subtype: i32,
    custom: i32,
    geometry: [u32; 7],
    material: i32,
    material_index: i32,
    stage: u32,
    max_stage: u32,
    exists: bool,
    jobs: u8,
    construct_jobs: u8,
    contained: u8,
    item_contained: bool,
}
impl WorkshopBuildingFacts {
    pub fn carpenters(&self) -> bool {
        self.carpenters
    }
    pub fn subtype(&self) -> i32 {
        self.subtype
    }
    pub fn custom(&self) -> i32 {
        self.custom
    }
    pub fn geometry(&self) -> [u32; 7] {
        self.geometry
    }
    pub fn material(&self) -> i32 {
        self.material
    }
    pub fn material_index(&self) -> i32 {
        self.material_index
    }
    pub fn stage(&self) -> u32 {
        self.stage
    }
    pub fn max_stage(&self) -> u32 {
        self.max_stage
    }
    pub fn exists(&self) -> bool {
        self.exists
    }
    pub fn jobs(&self) -> u8 {
        self.jobs
    }
    pub fn construct_jobs(&self) -> u8 {
        self.construct_jobs
    }
    pub fn contained(&self) -> u8 {
        self.contained
    }
    pub fn item_contained(&self) -> bool {
        self.item_contained
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkshopProgress {
    bytes: Vec<u8>,
    generation: u64,
    tick: u64,
    fortress: FortressIdentity,
    dimensions: [u32; 3],
    next_building: u32,
    building: u32,
    item: u32,
    facts: Option<WorkshopBuildingFacts>,
}
impl WorkshopProgress {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        require(
            bytes.len() <= MAX_PROGRESS_BYTES,
            "workshop progress exceeds 1 KiB",
        )?;
        let mut r = Reader(bytes);
        require(
            r.take(8)? == b"DFMWP020",
            "not a workshop/1.20 progress read",
        )?;
        let generation = r.u64()?;
        let tick = r.u64()?;
        let site = r.u32()?;
        let dimensions = [r.u32()?, r.u32()?, r.u32()?];
        let folder = text(field(&mut r, 512)?, 512)?;
        let fortress = FortressIdentity::new(&folder, site)?;
        let next_building = r.u32()?;
        let building = r.u32()?;
        let item = r.u32()?;
        require(
            generation > 0
                && generation < u64::MAX
                && tick <= MAX_NATIVE_TICK
                && dimensions.iter().all(|n| (1..=32768).contains(n))
                && next_building <= MAX_ID
                && building < MAX_ID
                && item < MAX_ID,
            "invalid workshop progress source or IDs",
        )?;
        let facts = if r.boolean()? {
            let carpenters = r.boolean()?;
            let subtype = r.u32()? as i32;
            let custom = r.u32()? as i32;
            let mut geometry = [0; 7];
            for n in &mut geometry {
                *n = r.u32()?;
            }
            let material = r.u32()? as i32;
            let material_index = r.u32()? as i32;
            let stage = r.u32()?;
            let max_stage = r.u32()?;
            let exists = r.boolean()?;
            let jobs = r.byte()?;
            let construct_jobs = r.byte()?;
            let contained = r.byte()?;
            let item_contained = r.boolean()?;
            require(
                building < next_building
                    && geometry.iter().all(|n| *n < 32768)
                    && (1..=32).contains(&max_stage)
                    && stage <= max_stage
                    && jobs <= 8
                    && construct_jobs <= jobs
                    && contained <= 64
                    && (!item_contained || contained > 0),
                "invalid workshop progress facts",
            )?;
            Some(WorkshopBuildingFacts {
                carpenters,
                subtype,
                custom,
                geometry,
                material,
                material_index,
                stage,
                max_stage,
                exists,
                jobs,
                construct_jobs,
                contained,
                item_contained,
            })
        } else {
            None
        };
        r.finish()?;
        Ok(Self {
            bytes: bytes.to_vec(),
            generation,
            tick,
            fortress,
            dimensions,
            next_building,
            building,
            item,
            facts,
        })
    }
    pub fn canonical_bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub fn witness(&self) -> Digest32 {
        Digest32::of_bytes(&self.bytes)
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn tick(&self) -> u64 {
        self.tick
    }
    pub fn fortress(&self) -> &FortressIdentity {
        &self.fortress
    }
    pub fn dimensions(&self) -> [u32; 3] {
        self.dimensions
    }
    pub fn next_building_id(&self) -> u32 {
        self.next_building
    }
    pub fn building_id(&self) -> u32 {
        self.building
    }
    pub fn item_id(&self) -> u32 {
        self.item
    }
    pub fn facts(&self) -> Option<&WorkshopBuildingFacts> {
        self.facts.as_ref()
    }
}

/// Original-goal verdict from one fresh progress read against the immutable
/// original receipt. Only `Complete` satisfies the goal. Absence is observed
/// absence of the original ID; `Unknown` covers any source or epoch break.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkshopGoal {
    Complete,
    InProgress {
        stage: u32,
        max_stage: u32,
        construct_jobs: u8,
    },
    Absent,
    Contradicted,
    Unknown(WorkshopUnknown),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkshopUnknown {
    NoPlacedReceipt,
    SourceIdentityDiffers,
    NativeGenerationChanged,
    ClockRegressed,
    RequestDiffers,
}
impl WorkshopUnknown {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NoPlacedReceipt => "no_placed_receipt",
            Self::SourceIdentityDiffers => "source_identity_differs",
            Self::NativeGenerationChanged => "native_generation_changed",
            Self::ClockRegressed => "clock_regressed",
            Self::RequestDiffers => "progress_request_differs",
        }
    }
}
impl WorkshopGoal {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Complete => "complete",
            Self::InProgress { .. } => "in_progress",
            Self::Absent => "original_building_absent",
            Self::Contradicted => "original_identity_contradicted",
            Self::Unknown(_) => "unknown",
        }
    }
    pub fn satisfied(self) -> bool {
        self == Self::Complete
    }
    /// Evaluate one fresh sample. The original native generation must be
    /// unchanged: after a plugin restart, reload or restore, a same-ID building
    /// with the same type, geometry and material is NOT continuity.
    pub fn evaluate(record: &WorkshopRecord, progress: &WorkshopProgress) -> Self {
        let (Some(after), Some(insertion)) = (record.after(), record.insertion()) else {
            return Self::Unknown(WorkshopUnknown::NoPlacedReceipt);
        };
        if record.phase() != WorkshopPhase::Placed {
            return Self::Unknown(WorkshopUnknown::NoPlacedReceipt);
        }
        let before = record.plan().before();
        if progress.fortress != *before.fortress() || progress.dimensions != before.dimensions() {
            return Self::Unknown(WorkshopUnknown::SourceIdentityDiffers);
        }
        if progress.generation != before.generation() {
            return Self::Unknown(WorkshopUnknown::NativeGenerationChanged);
        }
        if progress.tick < after.tick() || progress.next_building < after.next_building_id() {
            return Self::Unknown(WorkshopUnknown::ClockRegressed);
        }
        if progress.building != insertion.building_id() || progress.item != insertion.item_id() {
            return Self::Unknown(WorkshopUnknown::RequestDiffers);
        }
        let Some(facts) = progress.facts() else {
            return Self::Absent;
        };
        if !facts.carpenters
            || facts.custom != -1
            || facts.geometry != insertion.geometry()
            || facts.material != insertion.material()
            || facts.material_index != insertion.material_index()
            || facts.max_stage != insertion.max_stage()
        {
            return Self::Contradicted;
        }
        if facts.stage == facts.max_stage && facts.construct_jobs == 0 {
            Self::Complete
        } else {
            Self::InProgress {
                stage: facts.stage,
                max_stage: facts.max_stage,
                construct_jobs: facts.construct_jobs,
            }
        }
    }
}

/// Exact native endpoint/software/source identity retained by the durable owner.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkshopBinding {
    endpoint: SocketAddr,
    generation: u64,
    fortress: FortressIdentity,
    dimensions: [u32; 3],
    df_version: String,
    dfhack_version: String,
}
impl WorkshopBinding {
    /// Independently accepted operator pins, not an observed capture or source
    /// grant. The caller must still qualify every authenticated native read.
    pub fn from_source_pins(
        endpoint: SocketAddr,
        fortress: FortressIdentity,
        generation: u64,
        dimensions: [u32; 3],
        df: &str,
        dfhack: &str,
    ) -> Result<Self> {
        require(
            endpoint.is_ipv4()
                && endpoint.ip().is_loopback()
                && endpoint.port() != 0
                && generation > 0
                && generation < u64::MAX
                && dimensions.iter().all(|n| (1..=32768).contains(n)),
            "invalid workshop source pins",
        )?;
        Ok(Self {
            endpoint,
            generation,
            fortress,
            dimensions,
            df_version: text(df.as_bytes(), 128)?,
            dfhack_version: text(dfhack.as_bytes(), 128)?,
        })
    }

    pub fn new(
        endpoint: SocketAddr,
        df: &str,
        dfhack: &str,
        capture: &WorkshopCapture,
    ) -> Result<Self> {
        require(
            endpoint.is_ipv4() && endpoint.ip().is_loopback() && endpoint.port() != 0,
            "workshop requires numeric IPv4 loopback",
        )?;
        Ok(Self {
            endpoint,
            generation: capture.generation,
            fortress: capture.fortress.clone(),
            dimensions: capture.dimensions,
            df_version: text(df.as_bytes(), 128)?,
            dfhack_version: text(dfhack.as_bytes(), 128)?,
        })
    }
    pub fn endpoint(&self) -> SocketAddr {
        self.endpoint
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn fortress(&self) -> &FortressIdentity {
        &self.fortress
    }
    pub fn dimensions(&self) -> [u32; 3] {
        self.dimensions
    }
    pub fn df_version(&self) -> &str {
        &self.df_version
    }
    pub fn dfhack_version(&self) -> &str {
        &self.dfhack_version
    }
    pub fn capture_matches(&self, capture: &WorkshopCapture) -> bool {
        self.generation == capture.generation
            && self.fortress == capture.fortress
            && self.dimensions == capture.dimensions
    }
    /// Exact source identity for effects. Recovery may observe a later native
    /// generation for history lookups; it never inherits effect permission.
    pub fn source_matches(&self, current: &Self, exact: bool) -> Result<()> {
        require(
            self.endpoint == current.endpoint
                && self.fortress == current.fortress
                && self.dimensions == current.dimensions
                && self.df_version == current.df_version
                && self.dfhack_version == current.dfhack_version
                && current.generation >= self.generation
                && (!exact || current.generation == self.generation),
            "workshop source binding differs",
        )
    }
    #[cfg(test)]
    pub(crate) fn recovery_generation(&self, generation: u64) -> Result<Self> {
        require(
            generation >= self.generation && generation < u64::MAX,
            "workshop recovery generation regressed",
        )?;
        let mut out = self.clone();
        out.generation = generation;
        Ok(out)
    }
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        put_field(&mut out, self.endpoint.to_string().as_bytes());
        out.extend_from_slice(&self.generation.to_be_bytes());
        put(&mut out, [self.fortress.site()]);
        put_field(&mut out, self.fortress.folder().as_bytes());
        put(&mut out, self.dimensions);
        put_field(&mut out, self.df_version.as_bytes());
        put_field(&mut out, self.dfhack_version.as_bytes());
        out
    }
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        require(bytes.len() <= 1024, "workshop binding exceeds bound")?;
        let mut r = Reader(bytes);
        let endpoint: SocketAddr = text(field(&mut r, 128)?, 128)?
            .parse()
            .map_err(|_| invalid())?;
        let generation = r.u64()?;
        let site = r.u32()?;
        let fortress = FortressIdentity::new(&text(field(&mut r, 512)?, 512)?, site)?;
        let dimensions = [r.u32()?, r.u32()?, r.u32()?];
        let df_version = text(field(&mut r, 128)?, 128)?;
        let dfhack_version = text(field(&mut r, 128)?, 128)?;
        r.finish()?;
        require(
            endpoint.is_ipv4()
                && endpoint.ip().is_loopback()
                && endpoint.port() != 0
                && generation > 0
                && generation < u64::MAX
                && dimensions.iter().all(|n| (1..=32768).contains(n)),
            "invalid workshop binding",
        )?;
        let out = Self {
            endpoint,
            generation,
            fortress,
            dimensions,
            df_version,
            dfhack_version,
        };
        require(out.encode() == bytes, "noncanonical workshop binding")?;
        Ok(out)
    }
}

#[cfg(test)]
mod tests;
