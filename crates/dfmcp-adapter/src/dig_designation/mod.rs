#![forbid(unsafe_code)]
//! Typed evidence for the isolated dig/1.16 development protocol.
//!
//! A designation receipt proves historical configuration, not excavation, safety,
//! current terrain or authority. Hidden and missing cells have no attribute payload.

pub mod journal;
pub mod rpc;

use std::collections::BTreeSet;

use dfmcp_core::{Digest32, ErrorCode, FortressId, MapCoord, MapCuboid, Result};

use crate::bounded_run::{Reader, error, hash, require, validate_key};

pub const MAX_CAPTURE_BYTES: usize = 16 * 1024;
pub const MAX_EFFECT_BYTES: usize = 334;
pub const MAX_PLAN_BYTES: usize = MAX_CAPTURE_BYTES + 143;
pub const MAX_CELLS: usize = 300;
pub const MAX_NATIVE_TICK: u64 = u32::MAX as u64 * 403_200 + 403_199;

fn text(reader: &mut Reader<'_>, maximum: usize) -> Result<String> {
    let size = usize::from(u16::from_be_bytes(reader.array()?));
    require((1..=maximum).contains(&size), "invalid dig text length")?;
    let value = std::str::from_utf8(reader.take(size)?)
        .map_err(|_| error(ErrorCode::AdapterRejected, "invalid dig UTF-8"))?;
    require(!value.contains('\0'), "NUL in dig text")?;
    Ok(value.to_owned())
}

fn append_text(out: &mut Vec<u8>, value: &str) {
    // Only private validated key/folder fields reach this bounded serializer.
    out.extend_from_slice(&(value.len() as u16).to_be_bytes());
    out.extend_from_slice(value.as_bytes());
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DigRegion {
    x: u32,
    y: u32,
    z: u32,
    width: u32,
    height: u32,
}

impl DigRegion {
    pub fn new(x: u32, y: u32, z: u32, width: u32, height: u32) -> Result<Self> {
        if !(1..=8).contains(&width) || !(1..=8).contains(&height)
            || !(1..=32766).contains(&z) || x == 0 || y == 0
            || x > 32767 - width || y > 32767 - height
        {
            return Err(error(ErrorCode::InvalidRequest, "dig requires a bounded single-level rectangle and complete halo"));
        }
        Ok(Self { x, y, z, width, height })
    }
    pub fn coordinates(self) -> [u32; 5] { [self.x, self.y, self.z, self.width, self.height] }
    pub fn target_count(self) -> u32 { self.width * self.height }
    pub fn halo_count(self) -> usize { ((self.width + 2) * (self.height + 2) * 3) as usize }
    pub fn halo(self) -> MapCuboid {
        MapCuboid {
            min: MapCoord::new(self.x as i32 - 1, self.y as i32 - 1, self.z as i32 - 1),
            max: MapCoord::new((self.x + self.width) as i32, (self.y + self.height) as i32, self.z as i32 + 1),
        }
    }
    /// Scheduling writes affect complete map blocks, not merely target tiles.
    /// This conservative scope never under-authorizes those shared block writes.
    pub fn write_area(self) -> MapCuboid {
        MapCuboid {
            min: MapCoord::new(((self.x >> 4) << 4) as i32, ((self.y >> 4) << 4) as i32, self.z as i32),
            max: MapCoord::new((((self.x + self.width - 1) >> 4) * 16 + 15) as i32,
                (((self.y + self.height - 1) >> 4) * 16 + 15) as i32, self.z as i32),
        }
    }
    fn append(self, out: &mut Vec<u8>) {
        for n in self.coordinates() { out.extend_from_slice(&n.to_be_bytes()); }
    }
    fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        Self::new(reader.u32()?, reader.u32()?, reader.u32()?, reader.u32()?, reader.u32()?)
    }
    fn position(self, index: usize) -> [u32; 3] {
        let width = (self.width + 2) as usize;
        let layer = width * (self.height + 2) as usize;
        [self.x - 1 + (index % width) as u32,
            self.y - 1 + ((index % layer) / width) as u32, self.z - 1 + (index / layer) as u32]
    }
    fn target(self, [x, y, z]: [u32; 3]) -> bool {
        z == self.z && x >= self.x && x < self.x + self.width && y >= self.y && y < self.y + self.height
    }
    fn target_block(self, [x, y, z]: [u32; 3]) -> bool {
        z == self.z && ((self.x >> 4)..=((self.x + self.width - 1) >> 4)).contains(&(x >> 4))
            && ((self.y >> 4)..=((self.y + self.height - 1) >> 4)).contains(&(y >> 4))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VisibleTile {
    tiletype: u32,
    designation_other: u32,
    occupancy: u32,
    priority: u32,
    cooldown: u32,
    block_other: u32,
    temperatures: [u16; 2],
    dig: u8,
    hazards: u8,
    flags: u8,
}
impl VisibleTile {
    pub fn tiletype(&self) -> u32 { self.tiletype }
    pub fn designation_other(&self) -> u32 { self.designation_other }
    pub fn occupancy(&self) -> u32 { self.occupancy }
    pub fn priority(&self) -> u32 { self.priority }
    pub fn cooldown(&self) -> u32 { self.cooldown }
    pub fn block_other(&self) -> u32 { self.block_other }
    pub fn temperatures(&self) -> [u16; 2] { self.temperatures }
    pub fn dig(&self) -> u8 { self.dig }
    pub fn hazards(&self) -> u8 { self.hazards }
    pub fn flags(&self) -> u8 { self.flags }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DigTile { Missing, Hidden, Visible(VisibleTile) }

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum DigBlocker {
    Unpaused, UnobservedTarget, NotNaturalWall, ExistingDesignation,
    OccupiedOrJob, KnownHazard, MissingContext, HiddenContext, SequenceExhausted,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DigObservation {
    bytes: Vec<u8>,
    generation: u64,
    sequence: u64,
    tick: u64,
    site: u32,
    dimensions: [u32; 3],
    region: DigRegion,
    paused: bool,
    folder: String,
    tiles: Vec<DigTile>,
    offsets: Vec<usize>,
}
impl DigObservation {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        require(bytes.len() <= MAX_CAPTURE_BYTES, "dig capture exceeds 16 KiB")?;
        let mut r = Reader(bytes);
        require(r.take(8)? == b"DFMDG016", "not a dig/1.16 capture")?;
        let generation = r.u64()?;
        let sequence = r.u64()?;
        let tick = r.u64()?;
        let site = r.u32()?;
        require(generation > 0 && generation < u64::MAX && sequence < u64::MAX
            && tick <= MAX_NATIVE_TICK && site <= i32::MAX as u32, "invalid dig source or clock")?;
        let dimensions = [r.u32()?, r.u32()?, r.u32()?];
        let region = DigRegion::decode(&mut r)?;
        require(dimensions[0] > region.x + region.width && dimensions[0] <= 32768
            && dimensions[1] > region.y + region.height && dimensions[1] <= 32768
            && dimensions[2] > region.z + 1 && dimensions[2] <= 32768, "dig halo is outside the map")?;
        let paused = r.boolean()?;
        let folder = text(&mut r, 512)?;
        let count = usize::from(u16::from_be_bytes(r.array()?));
        require(count == region.halo_count() && count <= MAX_CELLS, "incomplete dig halo")?;
        let mut tiles = Vec::with_capacity(count);
        let mut offsets = Vec::with_capacity(count);
        for _ in 0..count {
            offsets.push(bytes.len() - r.0.len());
            tiles.push(match r.byte()? {
                0 => DigTile::Missing,
                1 => DigTile::Hidden,
                2 => {
                    let tile = VisibleTile {
                        tiletype: r.u32()?, designation_other: r.u32()?, occupancy: r.u32()?,
                        priority: r.u32()?, cooldown: r.u32()?, block_other: r.u32()?,
                        temperatures: [u16::from_be_bytes(r.array()?), u16::from_be_bytes(r.array()?)],
                        dig: r.byte()?, hazards: r.byte()?, flags: r.byte()?,
                    };
                    require(tile.priority <= 7000 && tile.dig <= 7 && tile.hazards <= 15 && tile.flags <= 31,
                        "invalid visible dig tile attributes")?;
                    DigTile::Visible(tile)
                }
                _ => return Err(error(ErrorCode::AdapterRejected, "unknown dig tile presence")),
            });
        }
        r.finish()?;
        Ok(Self { bytes: bytes.to_vec(), generation, sequence, tick, site, dimensions,
            region, paused, folder, tiles, offsets })
    }
    pub fn canonical_bytes(&self) -> &[u8] { &self.bytes }
    pub fn witness(&self) -> Digest32 { Digest32::of_bytes(&self.bytes) }
    pub fn generation(&self) -> u64 { self.generation }
    pub fn sequence(&self) -> u64 { self.sequence }
    pub fn tick(&self) -> u64 { self.tick }
    pub fn site(&self) -> u32 { self.site }
    pub fn folder(&self) -> &str { &self.folder }
    pub fn dimensions(&self) -> [u32; 3] { self.dimensions }
    pub fn region(&self) -> DigRegion { self.region }
    pub fn paused(&self) -> bool { self.paused }
    pub fn tiles(&self) -> impl Iterator<Item = ([u32; 3], &DigTile)> {
        self.tiles.iter().enumerate().map(|(i, tile)| (self.region.position(i), tile))
    }
    pub fn fortress_id(&self) -> FortressId { crate::workforce_control::fortress_id(&self.folder, self.site) }
    pub fn blockers(&self, allow_hidden_neighbors: bool) -> BTreeSet<DigBlocker> {
        let mut out = BTreeSet::new();
        if !self.paused { out.insert(DigBlocker::Unpaused); }
        if self.sequence >= u64::MAX - 1 { out.insert(DigBlocker::SequenceExhausted); }
        for (position, tile) in self.tiles() {
            match tile {
                DigTile::Missing => { out.insert(DigBlocker::MissingContext); }
                DigTile::Hidden if !allow_hidden_neighbors => { out.insert(DigBlocker::HiddenContext); }
                DigTile::Visible(v) if v.hazards != 0 => { out.insert(DigBlocker::KnownHazard); }
                _ => {}
            }
            if !self.region.target(position) { continue; }
            match tile {
                DigTile::Visible(v) => {
                    if v.flags & 1 == 0 { out.insert(DigBlocker::NotNaturalWall); }
                    if v.dig != 0 || v.flags & 2 != 0 { out.insert(DigBlocker::ExistingDesignation); }
                    if v.flags & 12 != 0 { out.insert(DigBlocker::OccupiedOrJob); }
                }
                _ => { out.insert(DigBlocker::UnobservedTarget); }
            }
        }
        out
    }
    fn expected_witness(&self) -> Result<Digest32> {
        require(self.sequence < u64::MAX - 1, "dig intervention sequence exhausted")?;
        let mut out = self.bytes.clone();
        out[16..24].copy_from_slice(&(self.sequence + 1).to_be_bytes());
        for (i, tile) in self.tiles.iter().enumerate() {
            if !matches!(tile, DigTile::Visible(_)) { continue; }
            let offset = self.offsets[i];
            let position = self.region.position(i);
            if self.region.target(position) {
                out[offset + 13..offset + 17].copy_from_slice(&4000u32.to_be_bytes());
                out[offset + 29] = 1;
            }
            if self.region.target_block(position) {
                out[offset + 17..offset + 21].fill(0);
                out[offset + 31] |= 16;
            }
        }
        Ok(Digest32::of_bytes(&out))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DigPlan {
    key: String,
    allow_hidden: bool,
    before: DigObservation,
    digest: Digest32,
    token: [u8; 16],
}
impl DigPlan {
    pub fn new(key: &str, allow_hidden: bool, before: DigObservation) -> Result<Self> {
        validate_key(key)?;
        require(before.blockers(allow_hidden).is_empty(), "dig capture is not eligible for designation")?;
        let mut bytes = Vec::new();
        before.region.append(&mut bytes);
        bytes.push(u8::from(allow_hidden));
        bytes.extend_from_slice(before.witness().as_bytes());
        let digest = hash(b"dfmcp-dig-designation-plan/1", &bytes);
        let mut bytes = before.generation.to_be_bytes().to_vec();
        append_text(&mut bytes, key);
        bytes.extend_from_slice(digest.as_bytes());
        let full = hash(b"dfmcp-dig-designation-token/1", &bytes);
        let mut token = [0; 16];
        token.copy_from_slice(&full.as_bytes()[..16]);
        Ok(Self { key: key.to_owned(), allow_hidden, before, digest, token })
    }
    pub fn key(&self) -> &str { &self.key }
    pub fn allow_hidden_neighbors(&self) -> bool { self.allow_hidden }
    pub fn before(&self) -> &DigObservation { &self.before }
    pub fn digest(&self) -> Digest32 { self.digest }
    pub fn token(&self) -> &[u8; 16] { &self.token }
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut out = b"DFMDGP16".to_vec();
        append_text(&mut out, &self.key);
        out.push(u8::from(self.allow_hidden));
        out.extend_from_slice(&(self.before.bytes.len() as u32).to_be_bytes());
        out.extend_from_slice(&self.before.bytes);
        out
    }
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        require(bytes.len() <= MAX_PLAN_BYTES, "dig plan exceeds its bound")?;
        let mut r = Reader(bytes);
        require(r.take(8)? == b"DFMDGP16", "invalid retained dig plan")?;
        let key = text(&mut r, 128)?;
        let allow_hidden = r.boolean()?;
        let size = r.u32()? as usize;
        require(size <= MAX_CAPTURE_BYTES, "retained dig observation exceeds its bound")?;
        let before = DigObservation::decode(r.take(size)?)?;
        r.finish()?;
        Self::new(&key, allow_hidden, before)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DigPhase { Prepared, Unknown, Designated, Refused }
impl DigPhase {
    pub fn terminal(self) -> bool { matches!(self, Self::Designated | Self::Refused) }
    pub fn as_str(self) -> &'static str {
        match self { Self::Prepared => "prepared", Self::Unknown => "unknown",
            Self::Designated => "designated", Self::Refused => "refused" }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DigReason { None, Stale, CancelledBeforeDispatch }

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DigEffect {
    bytes: Vec<u8>,
    phase: DigPhase,
    reason: DigReason,
    after: Option<Digest32>,
    count: Option<u32>,
    receipt: Option<Digest32>,
}
impl DigEffect {
    pub fn decode(bytes: &[u8], plan: &DigPlan) -> Result<Self> {
        require((207..=MAX_EFFECT_BYTES).contains(&bytes.len()), "invalid dig effect extent")?;
        let mut prefix = b"DFMDGE16".to_vec();
        for n in [plan.before.generation, plan.before.sequence, plan.before.tick] { prefix.extend_from_slice(&n.to_be_bytes()); }
        plan.before.region.append(&mut prefix);
        prefix.push(u8::from(plan.allow_hidden));
        prefix.extend_from_slice(plan.before.witness().as_bytes());
        prefix.extend_from_slice(plan.digest.as_bytes());
        prefix.extend_from_slice(&plan.token);
        let mut r = Reader(bytes);
        require(r.take(prefix.len())? == prefix.as_slice(), "dig effect differs from retained plan")?;
        let phase = match r.byte()? {
            0 => DigPhase::Prepared, 1 => DigPhase::Unknown, 2 => DigPhase::Designated, 4 => DigPhase::Refused,
            _ => return Err(error(ErrorCode::AdapterRejected, "unknown dig effect phase")),
        };
        let reason = match r.byte()? {
            0 => DigReason::None, 1 => DigReason::Stale, 2 => DigReason::CancelledBeforeDispatch,
            _ => return Err(error(ErrorCode::AdapterRejected, "unknown dig effect reason")),
        };
        let known = r.boolean()?;
        let count = r.u32()?;
        let after = Digest32::from_bytes(r.array()?);
        let receipt = Digest32::from_bytes(r.array()?);
        let key = text(&mut r, 128)?;
        r.finish()?;
        require(key == plan.key && known == (phase == DigPhase::Designated)
            && (phase == DigPhase::Refused) == (reason != DigReason::None), "contradictory dig effect shape")?;
        if known {
            require(count == plan.before.region.target_count() && after == plan.before.expected_witness()?,
                "designation lacks exact terrain, priority and scheduling readback")?;
        } else {
            require(count == 0 && after == Digest32::from_bytes([0; 32]), "absent dig readback is not zero")?;
        }
        let mut proof = plan.before.generation.to_be_bytes().to_vec();
        append_text(&mut proof, &plan.key);
        proof.extend_from_slice(plan.digest.as_bytes());
        proof.extend_from_slice(&plan.token);
        proof.extend_from_slice(&bytes[133..172]);
        let expected = if phase.terminal() { hash(b"dfmcp-dig-designation-receipt/1", &proof) }
            else { Digest32::from_bytes([0; 32]) };
        require(receipt == expected, "invalid dig terminal receipt")?;
        Ok(Self { bytes: bytes.to_vec(), phase, reason, after: known.then_some(after),
            count: known.then_some(count), receipt: phase.terminal().then_some(receipt) })
    }
    pub fn canonical_bytes(&self) -> &[u8] { &self.bytes }
    pub fn phase(&self) -> DigPhase { self.phase }
    pub fn reason(&self) -> DigReason { self.reason }
    pub fn after_witness(&self) -> Option<Digest32> { self.after }
    pub fn designated_count(&self) -> Option<u32> { self.count }
    pub fn receipt(&self) -> Option<Digest32> { self.receipt }
}

#[cfg(test)]
mod tests;
