#![forbid(unsafe_code)]
//! Sealed evidence for one exact furniture item in the isolated build/1.19 profile.
//!
//! Placed proves historical stage-zero construction-job registration, never a
//! completed or usable building. Decoding evidence never recreates dispatch authority.
//! Beads: df-dfhack-bridge-plane-c-pic.4/.5.

pub mod journal;
pub mod rpc;
pub mod session;

use crate::bounded_run::{Reader, error, hash, require, validate_key};
pub use crate::order_run::FortressIdentity;
use crate::order_run::{field, put_field, text};
use dfmcp_core::{Digest32, ErrorCode, FortressId, Result};
use std::net::SocketAddr;

pub const MAX_CAPTURE_BYTES: usize = 2048;
pub const MAX_RECORD_BYTES: usize = 6144;
pub const MAX_PLAN_BYTES: usize = 2190;
pub const MAX_NATIVE_TICK: u64 = crate::bounded_run::MAX_NATIVE_TICK;
const MAX_ID: u32 = i32::MAX as u32;

/// Global retention status from the source's latest validated native reply.
/// This is historical source metadata, not a canonical capture, an authority
/// grant, or proof that a particular placement key is absent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BuildNativeSummary {
    unresolved: bool,
    retained_records: u16,
}
impl BuildNativeSummary {
    pub fn new(unresolved: bool, retained_records: u16) -> Result<Self> {
        require(
            retained_records <= 256 && (!unresolved || retained_records > 0),
            "invalid furniture native retention summary",
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
    /// Availability reported by the latest reply. Native authority and state
    /// still require fresh checks before every preparation and dispatch.
    pub fn prepare_available(self) -> bool {
        !self.unresolved && self.retained_records < 256
    }
}

fn invalid() -> dfmcp_core::DfmcpError {
    error(ErrorCode::AdapterRejected, "invalid furniture evidence")
}
fn coordinates(r: &mut Reader<'_>) -> Result<[u32; 3]> {
    let out = [r.u32()?, r.u32()?, r.u32()?];
    require(
        out.iter().all(|n| *n < 32768),
        "furniture coordinate outside bounds",
    )?;
    Ok(out)
}
fn put_numbers(out: &mut Vec<u8>, values: impl IntoIterator<Item = u32>) {
    for n in values {
        out.extend_from_slice(&n.to_be_bytes());
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum BuildKind {
    Bed = 1,
    Chair = 2,
    Table = 3,
}
impl BuildKind {
    fn read(value: u8) -> Result<Self> {
        match value {
            1 => Ok(Self::Bed),
            2 => Ok(Self::Chair),
            3 => Ok(Self::Table),
            _ => Err(invalid()),
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Bed => "bed",
            Self::Chair => "chair",
            Self::Table => "table",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BuildSelection {
    kind: BuildKind,
    item: u32,
    target: [u32; 3],
}
impl BuildSelection {
    pub fn new(kind: BuildKind, item: u32, target: [u32; 3]) -> Result<Self> {
        require(
            item < MAX_ID
                && (1..32767).contains(&target[0])
                && (1..32767).contains(&target[1])
                && target[2] < 32768,
            "furniture selection needs an exact item and complete same-level halo",
        )?;
        Ok(Self { kind, item, target })
    }
    pub fn kind(self) -> BuildKind {
        self.kind
    }
    pub fn item_id(self) -> u32 {
        self.item
    }
    pub fn target(self) -> [u32; 3] {
        self.target
    }
    pub fn values(self) -> [u32; 5] {
        [
            self.kind as u32,
            self.item,
            self.target[0],
            self.target[1],
            self.target[2],
        ]
    }
    pub fn canonical_bytes(self) -> Vec<u8> {
        let mut out = vec![self.kind as u8];
        put_numbers(
            &mut out,
            [self.item, self.target[0], self.target[1], self.target[2]],
        );
        out
    }
    fn read(r: &mut Reader<'_>) -> Result<Self> {
        Self::new(BuildKind::read(r.byte()?)?, r.u32()?, coordinates(r)?)
    }
}

/// Missing/hidden variants contain no native attributes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BuildTile {
    Missing,
    Hidden,
    Visible(VisibleBuildTile),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VisibleBuildTile {
    tiletype: u32,
    shape: u8,
    liquid: u8,
    dig: u8,
    occupancy_other: u32,
    occupied: bool,
    building: Option<u32>,
}
impl VisibleBuildTile {
    pub fn tiletype(&self) -> u32 {
        self.tiletype
    }
    pub fn shape(&self) -> u8 {
        self.shape
    }
    pub fn liquid(&self) -> u8 {
        self.liquid
    }
    pub fn dig(&self) -> u8 {
        self.dig
    }
    pub fn occupancy_other(&self) -> u32 {
        self.occupancy_other
    }
    pub fn occupied(&self) -> bool {
        self.occupied
    }
    pub fn building_id(&self) -> Option<u32> {
        self.building
    }
}
impl BuildTile {
    fn read(r: &mut Reader<'_>) -> Result<Self> {
        match r.byte()? {
            0 => Ok(Self::Missing),
            1 => Ok(Self::Hidden),
            2 => {
                let tiletype = r.u32()?;
                let shape = r.byte()?;
                let liquid = r.byte()?;
                let dig = r.byte()?;
                let occupancy_other = r.u32()?;
                let occupied = r.boolean()?;
                let building = if r.boolean()? { Some(r.u32()?) } else { None };
                require(
                    tiletype <= MAX_ID
                        && shape <= 8
                        && liquid <= 7
                        && dig <= 7
                        && building.is_none_or(|n| n < MAX_ID),
                    "invalid visible furniture tile",
                )?;
                Ok(Self::Visible(VisibleBuildTile {
                    tiletype,
                    shape,
                    liquid,
                    dig,
                    occupancy_other,
                    occupied,
                    building,
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
                put_numbers(out, [v.tiletype]);
                out.extend_from_slice(&[v.shape, v.liquid, v.dig]);
                put_numbers(out, [v.occupancy_other]);
                out.extend_from_slice(&[u8::from(v.occupied), u8::from(v.building.is_some())]);
                if let Some(n) = v.building {
                    put_numbers(out, [n]);
                }
            }
        }
    }
    pub fn dry(&self) -> bool {
        matches!(self, Self::Visible(v) if v.liquid == 0)
    }
    pub fn empty_floor(&self) -> bool {
        matches!(self, Self::Visible(v) if v.liquid == 0 && v.shape == 3 && v.dig == 0
            && !v.occupied && v.building.is_none() && v.occupancy_other == 0)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BuildItem {
    Missing,
    Hidden,
    Visible(VisibleBuildItem),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VisibleBuildItem {
    position: [u32; 3],
    kind: Option<BuildKind>,
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
    ground: BuildTile,
}
impl VisibleBuildItem {
    pub fn position(&self) -> [u32; 3] {
        self.position
    }
    pub fn kind(&self) -> Option<BuildKind> {
        self.kind
    }
    pub fn native_type(&self) -> u32 {
        self.native_type
    }
    pub fn subtype(&self) -> i32 {
        self.subtype
    }
    pub fn material(&self) -> i32 {
        self.material
    }
    pub fn material_index(&self) -> i32 {
        self.material_index
    }
    pub fn quality(&self) -> u32 {
        self.quality
    }
    pub fn wear(&self) -> u32 {
        self.wear
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
    pub fn ground(&self) -> &BuildTile {
        &self.ground
    }
}
impl BuildItem {
    fn read(r: &mut Reader<'_>) -> Result<Self> {
        match r.byte()? {
            0 => Ok(Self::Missing),
            1 => Ok(Self::Hidden),
            2 => {
                let position = coordinates(r)?;
                let kind = match r.byte()? {
                    0 => None,
                    v => Some(BuildKind::read(v)?),
                };
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
                    "invalid selected furniture item",
                )?;
                let mut jobs = Vec::with_capacity(usize::from(n));
                for _ in 0..n {
                    let id = r.u32()?;
                    require(
                        id < MAX_ID && jobs.last().is_none_or(|old| *old < id),
                        "unordered or invalid furniture item job references",
                    )?;
                    jobs.push(id);
                }
                let ground = BuildTile::read(r)?;
                require(
                    matches!(ground, BuildTile::Visible(_)),
                    "visible furniture item needs visible ground",
                )?;
                Ok(Self::Visible(VisibleBuildItem {
                    position,
                    kind,
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
                put_numbers(out, v.position);
                out.push(v.kind.map_or(0, |kind| kind as u8));
                put_numbers(
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
                out.extend_from_slice(&[
                    u8::from(v.on_ground),
                    u8::from(v.in_job),
                    v.jobs.len() as u8,
                ]);
                put_numbers(out, v.jobs.iter().copied());
                v.ground.append(out);
            }
        }
    }
    pub fn available(&self, kind: BuildKind) -> bool {
        matches!(self, Self::Visible(v) if v.kind == Some(kind) && v.on_ground && !v.in_job
            && v.other_flags & !((1 << 28) | (1 << 29)) == 0 && v.other_refs == 0
            && v.jobs.is_empty() && v.wear == 0 && v.material >= 0
            && matches!(&v.ground, BuildTile::Visible(t) if t.liquid == 0 && t.shape == 3 && !t.occupied))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuildBlocker {
    GameNotPaused,
    TargetNotFree,
    TargetNotSupported,
    SequenceExhausted,
    BuildingIdExhausted,
    JobIdExhausted,
    BuildingCapacityExhausted,
    TargetNotEmptyFloor,
    ContextNotFullyVisible,
    ContextLiquid,
    NoEmptyFloorNeighbor,
    ItemUnavailable,
}
impl BuildBlocker {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::GameNotPaused => "game_not_paused",
            Self::TargetNotFree => "target_not_free",
            Self::TargetNotSupported => "target_not_supported",
            Self::SequenceExhausted => "sequence_exhausted",
            Self::BuildingIdExhausted => "building_id_exhausted",
            Self::JobIdExhausted => "job_id_exhausted",
            Self::BuildingCapacityExhausted => "building_capacity_exhausted",
            Self::TargetNotEmptyFloor => "target_not_empty_floor",
            Self::ContextNotFullyVisible => "context_not_fully_visible",
            Self::ContextLiquid => "context_liquid",
            Self::NoEmptyFloorNeighbor => "no_empty_floor_neighbor",
            Self::ItemUnavailable => "item_unavailable",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BuildCapture {
    bytes: Vec<u8>,
    generation: u64,
    sequence: u64,
    tick: u64,
    fortress: FortressIdentity,
    dimensions: [u32; 3],
    next_building: u32,
    next_job: u32,
    building_count: u32,
    paused: bool,
    free_tile: bool,
    supported: bool,
    selection: BuildSelection,
    tiles: Vec<BuildTile>,
    item: BuildItem,
}
impl BuildCapture {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        require(
            bytes.len() <= MAX_CAPTURE_BYTES,
            "furniture capture exceeds 2 KiB",
        )?;
        let mut r = Reader(bytes);
        require(r.take(8)? == b"DFMBC019", "not a furniture/1.19 capture")?;
        let generation = r.u64()?;
        let sequence = r.u64()?;
        let tick = r.u64()?;
        let site = r.u32()?;
        let dimensions = [r.u32()?, r.u32()?, r.u32()?];
        let next_building = r.u32()?;
        let next_job = r.u32()?;
        let building_count = r.u32()?;
        let folder = text(field(&mut r, 512)?, 512)?;
        let fortress = FortressIdentity::new(&folder, site)?;
        let paused = r.boolean()?;
        let free_tile = r.boolean()?;
        let supported = r.boolean()?;
        let selection = BuildSelection::read(&mut r)?;
        require(
            generation > 0
                && generation < u64::MAX
                && sequence < u64::MAX
                && tick <= MAX_NATIVE_TICK
                && dimensions.iter().all(|n| (1..=32768).contains(n))
                && next_building <= MAX_ID
                && next_job <= MAX_ID
                && building_count <= 65536,
            "invalid furniture source, clock, map or ID horizon",
        )?;
        let [x, y, z] = selection.target;
        require(
            x + 1 < dimensions[0] && y + 1 < dimensions[1] && z < dimensions[2],
            "furniture context outside map",
        )?;
        let mut tiles = Vec::with_capacity(9);
        for _ in 0..9 {
            tiles.push(BuildTile::read(&mut r)?);
        }
        let item = BuildItem::read(&mut r)?;
        if let BuildItem::Visible(v) = &item {
            require(
                v.position.iter().zip(dimensions).all(|(p, d)| *p < d),
                "furniture item outside map",
            )?;
        }
        r.finish()?;
        Ok(Self {
            bytes: bytes.to_vec(),
            generation,
            sequence,
            tick,
            fortress,
            dimensions,
            next_building,
            next_job,
            building_count,
            paused,
            free_tile,
            supported,
            selection,
            tiles,
            item,
        })
    }
    fn encode(&self) -> Vec<u8> {
        let mut out = b"DFMBC019".to_vec();
        for n in [self.generation, self.sequence, self.tick] {
            out.extend_from_slice(&n.to_be_bytes());
        }
        put_numbers(
            &mut out,
            [
                self.site(),
                self.dimensions[0],
                self.dimensions[1],
                self.dimensions[2],
                self.next_building,
                self.next_job,
                self.building_count,
            ],
        );
        put_field(&mut out, self.folder().as_bytes());
        out.extend_from_slice(&[
            u8::from(self.paused),
            u8::from(self.free_tile),
            u8::from(self.supported),
        ]);
        out.extend_from_slice(&self.selection.canonical_bytes());
        for tile in &self.tiles {
            tile.append(&mut out);
        }
        self.item.append(&mut out);
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
    pub fn paused(&self) -> bool {
        self.paused
    }
    pub fn free_tile(&self) -> bool {
        self.free_tile
    }
    pub fn supported(&self) -> bool {
        self.supported
    }
    pub fn selection(&self) -> BuildSelection {
        self.selection
    }
    pub fn tiles(&self) -> &[BuildTile] {
        &self.tiles
    }
    pub fn item(&self) -> &BuildItem {
        &self.item
    }
    pub fn item_position(&self) -> Option<[i32; 3]> {
        match &self.item {
            BuildItem::Visible(item) => Some(item.position.map(|n| n as i32)),
            _ => None,
        }
    }
    pub fn same_source(&self, other: &Self) -> bool {
        self.generation == other.generation
            && self.fortress == other.fortress
            && self.dimensions == other.dimensions
    }
    pub fn blockers(&self) -> Vec<BuildBlocker> {
        use BuildBlocker::*;
        [
            (!self.paused, GameNotPaused),
            (!self.free_tile, TargetNotFree),
            (!self.supported, TargetNotSupported),
            (self.sequence >= u64::MAX - 1, SequenceExhausted),
            (self.next_building >= MAX_ID, BuildingIdExhausted),
            (self.next_job >= MAX_ID, JobIdExhausted),
            (self.building_count >= 65536, BuildingCapacityExhausted),
            (!self.tiles[4].empty_floor(), TargetNotEmptyFloor),
            (
                !self
                    .tiles
                    .iter()
                    .all(|t| matches!(t, BuildTile::Visible(_))),
                ContextNotFullyVisible,
            ),
            (
                self.tiles
                    .iter()
                    .any(|t| matches!(t, BuildTile::Visible(v) if v.liquid != 0)),
                ContextLiquid,
            ),
            (
                ![1, 3, 5, 7].iter().any(|i| self.tiles[*i].empty_floor()),
                NoEmptyFloorNeighbor,
            ),
            (!self.item.available(self.selection.kind), ItemUnavailable),
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
        require(self.eligible(), "furniture plan has unmet preconditions")?;
        let mut out = self.clone();
        out.sequence += 1;
        out.next_building += 1;
        out.next_job += 1;
        out.building_count += 1;
        out.free_tile = false;
        if let BuildTile::Visible(v) = &mut out.tiles[4] {
            v.occupied = true;
            v.building = Some(self.next_building);
        }
        if let BuildItem::Visible(v) = &mut out.item {
            v.in_job = true;
            v.jobs = vec![self.next_job];
        }
        Self::decode(&out.encode())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BuildPlan {
    key: String,
    before: BuildCapture,
    digest: Digest32,
    token: [u8; 16],
}
impl BuildPlan {
    pub fn new(key: &str, before: BuildCapture) -> Result<Self> {
        validate_key(key)?;
        require(before.eligible(), "ineligible furniture plan")?;
        let mut out = before.selection.canonical_bytes();
        out.extend_from_slice(before.witness().as_bytes());
        let digest = hash(b"dfmcp-build-plan/1", &out);
        let mut out = Vec::new();
        put_field(&mut out, key.as_bytes());
        out.extend_from_slice(digest.as_bytes());
        let full = hash(b"dfmcp-build-token/1", &out);
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
    pub fn before(&self) -> &BuildCapture {
        &self.before
    }
    pub fn digest(&self) -> Digest32 {
        self.digest
    }
    pub fn token(&self) -> &[u8; 16] {
        &self.token
    }
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut out = b"DFMBP019".to_vec();
        put_field(&mut out, self.key.as_bytes());
        put_field(&mut out, self.before.canonical_bytes());
        out
    }
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        require(
            bytes.len() <= MAX_PLAN_BYTES,
            "furniture plan exceeds bound",
        )?;
        let mut r = Reader(bytes);
        require(r.take(8)? == b"DFMBP019", "invalid retained furniture plan")?;
        let key = text(field(&mut r, 128)?, 128)?;
        let before = BuildCapture::decode(field(&mut r, MAX_CAPTURE_BYTES)?)?;
        r.finish()?;
        Self::new(&key, before)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BuildInsertion {
    bytes: Vec<u8>,
    building: u32,
    job: u32,
    item: u32,
    kind: BuildKind,
    position: [u32; 3],
    material: i32,
    material_index: i32,
    stage: u32,
    max_stage: u32,
    linked: bool,
    construct_job: bool,
    exact_item_link: bool,
    suspended: bool,
}
impl BuildInsertion {
    fn decode(bytes: &[u8]) -> Result<Self> {
        require(bytes.len() == 53, "invalid furniture insertion width")?;
        let mut r = Reader(bytes);
        require(
            r.take(8)? == b"DFMBI019",
            "invalid furniture insertion profile",
        )?;
        let building = r.u32()?;
        let job = r.u32()?;
        let item = r.u32()?;
        let kind = BuildKind::read(r.byte()?)?;
        let position = coordinates(&mut r)?;
        let material = r.u32()? as i32;
        let material_index = r.u32()? as i32;
        let stage = r.u32()?;
        let max_stage = r.u32()?;
        let linked = r.boolean()?;
        let construct_job = r.boolean()?;
        let exact_item_link = r.boolean()?;
        let suspended = r.boolean()?;
        r.finish()?;
        require(
            [building, job, item].iter().all(|id| *id < MAX_ID)
                && (1..=32).contains(&max_stage)
                && stage <= max_stage,
            "invalid furniture insertion IDs or stages",
        )?;
        Ok(Self {
            bytes: bytes.to_vec(),
            building,
            job,
            item,
            kind,
            position,
            material,
            material_index,
            stage,
            max_stage,
            linked,
            construct_job,
            exact_item_link,
            suspended,
        })
    }
    fn matches(&self, before: &BuildCapture) -> bool {
        let BuildItem::Visible(item) = &before.item else {
            return false;
        };
        self.building == before.next_building
            && self.job == before.next_job
            && self.item == before.selection.item
            && self.kind == before.selection.kind
            && self.position == before.selection.target
            && self.material == item.material
            && self.material_index == item.material_index
            && self.stage == 0
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
    pub fn kind(&self) -> BuildKind {
        self.kind
    }
    pub fn position(&self) -> [u32; 3] {
        self.position
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
    pub fn linked(&self) -> bool {
        self.linked
    }
    pub fn construct_job(&self) -> bool {
        self.construct_job
    }
    pub fn exact_item_link(&self) -> bool {
        self.exact_item_link
    }
    pub fn suspended(&self) -> bool {
        self.suspended
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum BuildPhase {
    Prepared = 0,
    Indeterminate = 1,
    Placed = 2,
    Refused = 3,
    Cancelled = 4,
}
impl BuildPhase {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Prepared => "prepared",
            Self::Indeterminate => "indeterminate",
            Self::Placed => "placed",
            Self::Refused => "refused",
            Self::Cancelled => "cancelled",
        }
    }
    pub fn terminal(self) -> bool {
        matches!(self, Self::Placed | Self::Refused | Self::Cancelled)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum BuildReason {
    None = 0,
    Stale = 1,
    Expired = 2,
    SourceChanged = 3,
    Cancelled = 4,
    NativeFailure = 5,
}
impl BuildReason {
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
pub struct BuildRecord {
    bytes: Vec<u8>,
    plan: BuildPlan,
    phase: BuildPhase,
    reason: BuildReason,
    after: Option<BuildCapture>,
    insertion: Option<BuildInsertion>,
    receipt: Digest32,
}
impl BuildRecord {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        require(
            bytes.len() <= MAX_RECORD_BYTES,
            "furniture receipt exceeds 6 KiB",
        )?;
        let mut r = Reader(bytes);
        require(
            r.take(8)? == b"DFMBR019",
            "invalid furniture receipt profile",
        )?;
        let key = text(field(&mut r, 128)?, 128)?;
        let before = BuildCapture::decode(field(&mut r, MAX_CAPTURE_BYTES)?)?;
        let plan = BuildPlan::new(&key, before)?;
        require(
            r.take(32)? == plan.digest.as_bytes() && r.take(16)? == plan.token,
            "furniture receipt plan or token differs",
        )?;
        let phase = match r.byte()? {
            0 => BuildPhase::Prepared,
            1 => BuildPhase::Indeterminate,
            2 => BuildPhase::Placed,
            3 => BuildPhase::Refused,
            4 => BuildPhase::Cancelled,
            _ => return Err(invalid()),
        };
        let reason = match r.byte()? {
            0 => BuildReason::None,
            1 => BuildReason::Stale,
            2 => BuildReason::Expired,
            3 => BuildReason::SourceChanged,
            4 => BuildReason::Cancelled,
            5 => BuildReason::NativeFailure,
            _ => return Err(invalid()),
        };
        let attempted = r.boolean()?;
        let has_after = r.boolean()?;
        let valid_reason = match phase {
            BuildPhase::Prepared | BuildPhase::Placed => reason == BuildReason::None,
            BuildPhase::Indeterminate => reason == BuildReason::NativeFailure,
            BuildPhase::Refused => matches!(
                reason,
                BuildReason::Stale | BuildReason::Expired | BuildReason::SourceChanged
            ),
            BuildPhase::Cancelled => reason == BuildReason::Cancelled,
        };
        require(
            valid_reason
                && attempted == matches!(phase, BuildPhase::Placed | BuildPhase::Indeterminate)
                && has_after == (phase == BuildPhase::Placed),
            "contradictory furniture receipt state",
        )?;
        let (after, insertion) = if has_after {
            let after = BuildCapture::decode(field(&mut r, MAX_CAPTURE_BYTES)?)?;
            let insertion = BuildInsertion::decode(field(&mut r, 53)?)?;
            require(
                after == plan.before.expected_after()? && insertion.matches(&plan.before),
                "furniture receipt lacks exact complete native effect readback",
            )?;
            (Some(after), Some(insertion))
        } else {
            (None, None)
        };
        let receipt = Digest32::from_bytes(r.array()?);
        r.finish()?;
        require(
            receipt == hash(b"dfmcp-build-receipt/1", &bytes[..bytes.len() - 32]),
            "furniture receipt integrity failed",
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
    pub fn plan(&self) -> &BuildPlan {
        &self.plan
    }
    pub fn phase(&self) -> BuildPhase {
        self.phase
    }
    pub fn reason(&self) -> BuildReason {
        self.reason
    }
    pub fn attempted(&self) -> bool {
        matches!(self.phase, BuildPhase::Indeterminate | BuildPhase::Placed)
    }
    pub fn resolved(&self) -> bool {
        self.phase.terminal()
    }
    pub fn terminal(&self) -> bool {
        self.resolved()
    }
    pub fn after(&self) -> Option<&BuildCapture> {
        self.after.as_ref()
    }
    pub fn insertion(&self) -> Option<&BuildInsertion> {
        self.insertion.as_ref()
    }
    pub fn receipt(&self) -> Digest32 {
        self.receipt
    }
    pub fn verify_plan(&self, plan: &BuildPlan) -> Result<()> {
        require(
            &self.plan == plan,
            "furniture record belongs to another plan",
        )
    }
    pub fn validate_successor(&self, next: &Self) -> Result<()> {
        require(
            self.plan == next.plan && (self.phase == BuildPhase::Prepared || self == next),
            "immutable furniture outcome changed; preserve uncertainty",
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BuildPreparation {
    record: BuildRecord,
    replayed: bool,
}
impl BuildPreparation {
    pub fn new(record: BuildRecord, replayed: bool) -> Result<Self> {
        require(
            replayed || record.phase == BuildPhase::Prepared,
            "fresh furniture preparation contains history",
        )?;
        Ok(Self { record, replayed })
    }
    pub fn record(&self) -> &BuildRecord {
        &self.record
    }
    pub fn effect(&self) -> &BuildRecord {
        &self.record
    }
    pub fn replayed(&self) -> bool {
        self.replayed
    }
}

/// Exact native endpoint/software/source identity retained by the durable owner.
/// Recovery retains the original fortress/map as expected historical scope.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BuildBinding {
    endpoint: SocketAddr,
    generation: u64,
    fortress: FortressIdentity,
    dimensions: [u32; 3],
    df_version: String,
    dfhack_version: String,
}
impl BuildBinding {
    pub fn new(
        endpoint: SocketAddr,
        df: &str,
        dfhack: &str,
        capture: &BuildCapture,
    ) -> Result<Self> {
        require(
            endpoint.is_ipv4() && endpoint.ip().is_loopback() && endpoint.port() != 0,
            "furniture requires numeric IPv4 loopback",
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
    pub(crate) fn recovery_generation(&self, generation: u64) -> Result<Self> {
        require(
            generation >= self.generation && generation < u64::MAX,
            "furniture recovery generation regressed",
        )?;
        let mut out = self.clone();
        out.generation = generation;
        Ok(out)
    }
    pub fn capture_matches(&self, capture: &BuildCapture) -> bool {
        self.generation == capture.generation
            && self.fortress == capture.fortress
            && self.dimensions == capture.dimensions
    }
    pub fn source_matches(&self, current: &Self, exact: bool) -> Result<()> {
        require(
            self.endpoint == current.endpoint
                && self.fortress == current.fortress
                && self.dimensions == current.dimensions
                && self.df_version == current.df_version
                && self.dfhack_version == current.dfhack_version
                && current.generation >= self.generation
                && (!exact || current.generation == self.generation),
            "furniture source binding differs",
        )
    }
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        put_field(&mut out, self.endpoint.to_string().as_bytes());
        out.extend_from_slice(&self.generation.to_be_bytes());
        put_numbers(&mut out, [self.fortress.site()]);
        put_field(&mut out, self.fortress.folder().as_bytes());
        put_numbers(&mut out, self.dimensions);
        put_field(&mut out, self.df_version.as_bytes());
        put_field(&mut out, self.dfhack_version.as_bytes());
        out
    }
    pub fn canonical_bytes(&self) -> Vec<u8> {
        self.encode()
    }
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        require(bytes.len() <= 1024, "furniture binding exceeds bound")?;
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
            "invalid furniture binding",
        )?;
        let out = Self {
            endpoint,
            generation,
            fortress,
            dimensions,
            df_version,
            dfhack_version,
        };
        require(out.encode() == bytes, "noncanonical furniture binding")?;
        Ok(out)
    }
}

#[cfg(test)]
mod tests;
