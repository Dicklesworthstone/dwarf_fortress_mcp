#![forbid(unsafe_code)]
//! Sealed Rust evidence for the isolated excavation-run/1.18 profile.
//!
//! A sampled floor condition is neither mining causality nor a current pause.
//! These values confer no authority, and decoding a prepared record never grants
//! permission to dispatch it. Beads: df-dfhack-bridge-plane-c-pic.4/.5.

pub mod coordinator;
pub mod private_file;
pub mod rpc;
pub mod session;
pub mod workflow;
mod record;
pub use record::{ExcavationRunRecord, ExcavationTrigger};
pub use crate::bounded_run::{RunPhase, RunReason, RunSpec};
pub use crate::order_run::FortressIdentity;
use crate::bounded_run::{Reader, RunObservation, error, hash, require, validate_key};
use crate::order_run::{field, put_field, text};
use dfmcp_core::{Digest32, ErrorCode, Result};

pub const MAX_CAPTURE_BYTES: usize = 1024;
pub const MAX_RECORD_BYTES: usize = 3072;
pub const MAX_PLAN_BYTES: usize = 1188;
pub const MAX_NATIVE_TICK: u64 = crate::bounded_run::MAX_NATIVE_TICK;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExcavationRegion([u32; 5]);
impl ExcavationRegion {
    pub fn new(values: [u32; 5]) -> Result<Self> {
        let [x, y, z, width, height] = values;
        if x >= 32768 || y >= 32768 || z >= 32768
            || !(1..=8).contains(&width) || !(1..=8).contains(&height)
            || width > 32768 - x || height > 32768 - y
        {
            return Err(error(ErrorCode::InvalidRequest, "invalid excavation rectangle"));
        }
        Ok(Self(values))
    }
    pub fn values(self) -> [u32; 5] { self.0 }
    pub fn cell_count(self) -> usize { (self.0[3] * self.0[4]) as usize }
}

/// Redacted cells have no backing attributes; zero is not fabricated terrain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExcavationCell {
    Missing,
    Hidden,
    Visible { shape: u8, liquid: u8, dig: u8 },
}
impl ExcavationCell {
    pub fn floor_observed(self) -> bool {
        self == Self::Visible { shape: 3, liquid: 0, dig: 0 }
    }
    pub fn visible_dry(self) -> bool {
        matches!(self, Self::Visible { liquid: 0, .. })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExcavationCapture {
    bytes: Vec<u8>,
    clock: RunObservation,
    tick: u64,
    fortress: FortressIdentity,
    dimensions: [u32; 3],
    region: ExcavationRegion,
    cells: Vec<ExcavationCell>,
}
impl ExcavationCapture {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        require(bytes.len() <= MAX_CAPTURE_BYTES, "excavation capture exceeds 1 KiB")?;
        let mut r = Reader(bytes);
        require(r.take(8)? == b"DFMEC018", "not an excavation-run/1.18 capture")?;
        let clock = RunObservation::decode(r.take(35)?)?;
        require(clock.loaded(), "excavation capture has no loaded source")?;
        let tick = clock.tick().ok_or_else(|| error(ErrorCode::AdapterRejected,
            "excavation capture has no valid clock"))?;
        let site = r.u32()?;
        let dimensions = [r.u32()?, r.u32()?, r.u32()?];
        require(dimensions.iter().all(|n| (1..=32768).contains(n)), "invalid map size")?;
        let folder = text(field(&mut r, 512)?, 512)?;
        let fortress = FortressIdentity::new(&folder, site)?;
        let region = ExcavationRegion::new([r.u32()?, r.u32()?, r.u32()?, r.u32()?, r.u32()?])?;
        let [x, y, z, width, height] = region.values();
        require(x + width <= dimensions[0] && y + height <= dimensions[1]
            && z < dimensions[2], "excavation region outside map")?;
        let count = usize::from(u16::from_be_bytes(r.array()?));
        require(count == region.cell_count(), "excavation cell count mismatch")?;
        let mut cells = Vec::with_capacity(count); // Count validated before allocation, at most 64.
        for _ in 0..count {
            cells.push(match r.byte()? {
                0 => ExcavationCell::Missing,
                1 => ExcavationCell::Hidden,
                2 => {
                    let shape = r.byte()?;
                    let liquid = r.byte()?;
                    let dig = r.byte()?;
                    require(shape <= 8 && liquid <= 7 && dig <= 7, "invalid visible cell")?;
                    ExcavationCell::Visible { shape, liquid, dig }
                }
                _ => return Err(error(ErrorCode::AdapterRejected, "unknown cell presence")),
            });
        }
        r.finish()?;
        Ok(Self { bytes: bytes.to_vec(), clock, tick, fortress, dimensions, region, cells })
    }
    pub fn canonical_bytes(&self) -> &[u8] { &self.bytes }
    pub fn witness(&self) -> Digest32 { Digest32::of_bytes(&self.bytes) }
    pub fn fortress(&self) -> &FortressIdentity { &self.fortress }
    pub fn generation(&self) -> u64 { self.clock.generation() }
    pub fn sequence(&self) -> u64 { self.clock.sequence() }
    pub fn tick(&self) -> u64 { self.tick }
    pub fn paused(&self) -> bool { self.clock.paused() }
    pub fn dimensions(&self) -> [u32; 3] { self.dimensions }
    pub fn region(&self) -> ExcavationRegion { self.region }
    pub fn cells(&self) -> &[ExcavationCell] { &self.cells }
    pub fn floor_observed(&self) -> bool { self.cells.iter().all(|c| c.floor_observed()) }
    pub fn same_source(&self, other: &Self) -> bool {
        self.generation() == other.generation() && self.fortress == other.fortress
            && self.dimensions == other.dimensions
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExcavationRunSpec {
    clock: RunSpec,
    samples: u32,
    stable_ticks: u32,
    interval: u32,
    max_gap: u32,
}
impl ExcavationRunSpec {
    pub fn new(clock: RunSpec, samples: u32, stable_ticks: u32, interval: u32,
        max_gap: u32) -> Result<Self>
    {
        let ticks = clock.game_ticks();
        if !(1..=128).contains(&samples) || !(1..=ticks).contains(&interval)
            || stable_ticks > ticks || max_gap < interval || max_gap > 1200
        {
            return Err(error(ErrorCode::InvalidRequest, "invalid excavation sampling limits"));
        }
        // Arithmetic occurs only after all operands are bounded.
        if interval + ((samples - 1) * interval).max(stable_ticks) >= ticks {
            return Err(error(ErrorCode::InvalidRequest, "floor window cannot fit before tick limit"));
        }
        Ok(Self { clock, samples, stable_ticks, interval, max_gap })
    }
    pub fn values(self) -> [u32; 6] {
        [self.clock.game_ticks(), self.clock.wall_ms(), self.samples, self.stable_ticks,
            self.interval, self.max_gap]
    }
    pub fn clock(self) -> RunSpec { self.clock }
    pub fn samples(self) -> u32 { self.samples }
    pub fn stable_ticks(self) -> u32 { self.stable_ticks }
    pub fn interval(self) -> u32 { self.interval }
    pub fn max_gap(self) -> u32 { self.max_gap }
    pub(super) fn append(self, bytes: &mut Vec<u8>) {
        for n in self.values() { bytes.extend_from_slice(&n.to_be_bytes()); }
    }
    pub(super) fn read(r: &mut Reader<'_>) -> Result<Self> {
        Self::new(RunSpec::new(r.u32()?, r.u32()?)?, r.u32()?, r.u32()?, r.u32()?, r.u32()?)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExcavationRunPlan {
    key: String,
    spec: ExcavationRunSpec,
    before: ExcavationCapture,
    digest: Digest32,
    token: [u8; 16],
}
impl ExcavationRunPlan {
    pub fn new(key: &str, spec: ExcavationRunSpec, before: ExcavationCapture) -> Result<Self> {
        validate_key(key)?;
        if !before.paused() || before.floor_observed() || before.sequence() == u64::MAX
            || before.tick() > MAX_NATIVE_TICK - u64::from(spec.clock.game_ticks())
            || !before.cells.iter().all(|c| c.visible_dry())
        {
            return Err(error(ErrorCode::StaleAnchor,
                "run requires an unsatisfied visible dry region and an eligible paused clock"));
        }
        let mut data = Vec::new();
        spec.append(&mut data);
        data.extend_from_slice(before.canonical_bytes());
        let digest = hash(b"dfmcp-excavation-run-plan/1", &data);
        let mut keyed = Vec::new();
        put_field(&mut keyed, key.as_bytes());
        keyed.extend_from_slice(digest.as_bytes());
        let full = hash(b"dfmcp-excavation-run-token/1", &keyed);
        let mut token = [0; 16];
        token.copy_from_slice(&full.as_bytes()[..16]);
        Ok(Self { key: key.to_owned(), spec, before, digest, token })
    }
    pub fn key(&self) -> &str { &self.key }
    pub fn spec(&self) -> ExcavationRunSpec { self.spec }
    pub fn before(&self) -> &ExcavationCapture { &self.before }
    pub fn digest(&self) -> Digest32 { self.digest }
    pub fn token(&self) -> &[u8; 16] { &self.token }
    /// Reconstructible Rust journal intent; NOT the native plan hash preimage.
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut out = b"DFMEP018".to_vec();
        put_field(&mut out, self.key.as_bytes());
        self.spec.append(&mut out);
        put_field(&mut out, self.before.canonical_bytes());
        out
    }
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        require(bytes.len() <= MAX_PLAN_BYTES, "excavation intent exceeds bound")?;
        let mut r = Reader(bytes);
        require(r.take(8)? == b"DFMEP018", "not an excavation-run intent")?;
        let key = text(field(&mut r, 128)?, 128)?;
        let spec = ExcavationRunSpec::read(&mut r)?;
        let before = ExcavationCapture::decode(field(&mut r, MAX_CAPTURE_BYTES)?)?;
        r.finish()?;
        Self::new(&key, spec, before)
    }
}

#[cfg(test)]
mod tests;
