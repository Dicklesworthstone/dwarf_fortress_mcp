//! Exact sparse mining geometry and durable-parent prefix validation.
//!
//! The geometry matches `scripts/dig_blueprint.py`; the Rust parent is a distinct
//! custody format, not an import of Python stores or an original PreparedPlan.
//! A complete prefix of historical Designated receipts unlocks one new region.
//! It never establishes excavation completion, present terrain, or authority.
//! Bead: df-dfhack-bridge-plane-c-pic.4.

use std::collections::{BTreeMap, BTreeSet};

use dfmcp_core::{DfmcpError, Digest32, ErrorCode, Result};

use crate::dig_designation::journal::session::DigSessionView;
use crate::dig_designation::journal::{DigBinding, DigRecord, DigState};
use crate::dig_designation::{DigEffect, DigObservation, DigPhase, DigReason, DigRegion};

pub mod store;

pub const SCHEMA: &str = "dfmcp.excavation-blueprint/1";
pub const MAX_PARTS: usize = 32;
pub const MAX_TARGETS: usize = 512;
pub const MAX_VOLUME: u64 = 1024;
pub const MAX_STEPS: usize = 128;
pub const MAX_BLUEPRINT_BYTES: usize = 16 * 1024;
pub const MAX_DEFINITION_BYTES: usize = 32 * 1024;
const DEFINITION_MAGIC: &[u8; 8] = b"DFMDBD01";

fn invalid(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::InvalidRequest, message)
}
fn corrupt(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::CorruptLedger, message)
}
fn require(ok: bool, message: &str) -> Result<()> {
    if ok { Ok(()) } else { Err(invalid(message)) }
}
fn hash(domain: &[u8], bytes: &[u8]) -> Digest32 {
    let mut input = domain.to_vec();
    input.push(0);
    input.extend_from_slice(bytes);
    Digest32::of_bytes(&input)
}

/// One floor-only cuboid. No shape, native key, policy or authority is imported.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BlueprintPart {
    pub origin: [u32; 3],
    pub size: [u32; 3],
}

/// Immutable original parts plus the deterministic exact-cover native layout.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    parts: Vec<BlueprintPart>,
    targets: Vec<[u32; 3]>,
    regions: Vec<DigRegion>,
    bytes: Vec<u8>,
    blueprint_digest: Digest32,
    digest: Digest32,
}
impl Layout {
    pub fn new(mut parts: Vec<BlueprintPart>) -> Result<Self> {
        require(
            (1..=MAX_PARTS).contains(&parts.len()),
            "mining blueprint requires 1..32 floor parts",
        )?;
        let mut count = 0u64;
        let mut low = [u32::MAX; 3];
        let mut high = [0u32; 3];
        // Complete extent admission precedes expansion of any target cell.
        for part in &parts {
            for axis in 0..3 {
                require(
                    (1..=32766).contains(&part.origin[axis])
                        && (1..=128).contains(&part.size[axis])
                        && part.origin[axis]
                            .checked_add(part.size[axis])
                            .is_some_and(|end| end <= 32767),
                    "blueprint part exceeds bounded coordinates or complete native halo",
                )?;
                low[axis] = low[axis].min(part.origin[axis]);
                high[axis] = high[axis].max(part.origin[axis] + part.size[axis]);
            }
            count += part.size.into_iter().map(u64::from).product::<u64>();
            require(count <= MAX_TARGETS as u64, "blueprint exceeds 512 targets")?;
        }
        let size = std::array::from_fn::<_, 3, _>(|axis| high[axis] - low[axis]);
        require(
            size.iter().all(|n| *n <= 128)
                && size.into_iter().map(u64::from).product::<u64>() <= MAX_VOLUME,
            "blueprint exceeds its coherent 1024-cell capture domain",
        )?;
        let mut selected = BTreeSet::new();
        for part in &parts {
            let [x, y, z] = part.origin;
            let [w, h, levels] = part.size;
            for pz in z..z + levels {
                for py in y..y + h {
                    for px in x..x + w {
                        require(
                            selected.insert((pz, py, px)),
                            "mining blueprint parts overlap",
                        )?;
                    }
                }
            }
        }
        let targets: Vec<_> = selected.iter().map(|&(z, y, x)| [x, y, z]).collect();
        let mut remaining = selected;
        let mut regions = Vec::new();
        while let Some(&(z, y, x)) = remaining.first() {
            let mut width = 1;
            while width < 8 && remaining.contains(&(z, y, x + width)) {
                width += 1;
            }
            let mut height = 1;
            while height < 8 && (x..x + width).all(|px| remaining.contains(&(z, y + height, px))) {
                height += 1;
            }
            require(
                regions.len() < MAX_STEPS,
                "blueprint exceeds 128 native regions",
            )?;
            regions.push(DigRegion::new(x, y, z, width, height)?);
            for py in y..y + height {
                for px in x..x + width {
                    remaining.remove(&(z, py, px));
                }
            }
        }
        parts.sort_by_key(|part| {
            let [x, y, z] = part.origin;
            let [w, h, levels] = part.size;
            (z, y, x, levels, h, w)
        });
        let mut canonical = String::from("{\"parts\":[");
        for (index, part) in parts.iter().enumerate() {
            if index > 0 {
                canonical.push(',');
            }
            let [x, y, z] = part.origin;
            let [w, h, levels] = part.size;
            canonical.push_str(&format!(
                "{{\"region\":{{\"origin\":[{x},{y},{z}],\"size\":[{w},{h},{levels}]}},\"shape\":\"floor\"}}"
            ));
        }
        canonical.push_str(&format!("],\"schema\":\"{SCHEMA}\"}}"));
        require(
            canonical.len() <= MAX_BLUEPRINT_BYTES,
            "normalized blueprint exceeds 16 KiB",
        )?;
        let [x, y, z] = low;
        let [w, h, levels] = size;
        let mut mask = format!(
            "{{\"region\":{{\"origin\":[{x},{y},{z}],\"size\":[{w},{h},{levels}]}},\"targets\":["
        );
        for (index, [px, py, pz]) in targets.iter().copied().enumerate() {
            if index > 0 {
                mask.push(',');
            }
            let cell = ((pz - z) * h + py - y) * w + px - x;
            mask.push_str(&format!("[{cell},3]"));
        }
        mask.push_str("]}");
        let blueprint_digest = hash(b"dfmcp-excavation-blueprint/1", mask.as_bytes());
        let mut layout = format!(
            "{{\"blueprint_digest\":\"{blueprint_digest}\",\"dig_mode\":\"normal\",\"native_profile\":\"dig/1.16\",\"rectangles\":["
        );
        for (index, region) in regions.iter().enumerate() {
            if index > 0 {
                layout.push(',');
            }
            let [x, y, z, w, h] = region.coordinates();
            layout.push_str(&format!("[{x},{y},{z},{w},{h}]"));
        }
        layout.push_str("]}");
        Ok(Self {
            parts,
            targets,
            regions,
            bytes: canonical.into_bytes(),
            blueprint_digest,
            digest: hash(b"dfmcp-dig-blueprint-layout/1", layout.as_bytes()),
        })
    }
    pub fn parts(&self) -> &[BlueprintPart] {
        &self.parts
    }
    /// Exact selected cells in z/y/x order, excluding every unselected gap.
    pub fn targets(&self) -> &[[u32; 3]] {
        &self.targets
    }
    pub fn regions(&self) -> &[DigRegion] {
        &self.regions
    }
    pub fn canonical_bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub fn blueprint_digest(&self) -> Digest32 {
        self.blueprint_digest
    }
    pub fn digest(&self) -> Digest32 {
        self.digest
    }
    fn validate_scope(&self, binding: &DigBinding) -> Result<()> {
        require(
            self.regions.iter().all(|region| {
                binding.scope().contains_cuboid(region.halo())
                    && binding.scope().contains_cuboid(region.write_area())
            }),
            "blueprint halo or shared scheduling block exceeds the original scope",
        )
    }
    fn validate_dimensions(&self, dimensions: [u32; 3]) -> Result<()> {
        require(
            self.regions.iter().all(|region| {
                let [x, y, z, w, h] = region.coordinates();
                x + w < dimensions[0] && y + h < dimensions[1] && z + 1 < dimensions[2]
            }),
            "a future blueprint region or complete halo lies outside the original map",
        )
    }
}

/// Complete parent intent, exact native source, original journal and bootstrap.
/// The separate parent bytes never create a native dispatch permit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BatchDefinition {
    layout: Layout,
    binding: DigBinding,
    journal_id: Digest32,
    bootstrap: DigObservation,
    id: Digest32,
}
impl BatchDefinition {
    pub fn new(
        layout: Layout,
        binding: DigBinding,
        journal_id: Digest32,
        bootstrap: DigObservation,
    ) -> Result<Self> {
        layout.validate_scope(&binding)?;
        layout.validate_dimensions(bootstrap.dimensions())?;
        require(
            journal_id != Digest32::ZERO,
            "blueprint requires an original native journal identity",
        )?;
        require(
            bootstrap.paused()
                && layout.regions.first() == Some(&bootstrap.region())
                && bootstrap.generation() == binding.manifest().generation
                && bootstrap.folder() == binding.folder()
                && bootstrap.site() == binding.site(),
            "blueprint bootstrap must capture the first region at its exact paused source",
        )?;
        let mut out = Self {
            layout,
            binding,
            journal_id,
            bootstrap,
            id: Digest32::ZERO,
        };
        let bytes = out.canonical_bytes();
        require(
            bytes.len() <= MAX_DEFINITION_BYTES,
            "blueprint definition exceeds bound",
        )?;
        out.id = hash(b"dfmcp-dig-blueprint-batch-rust/1", &bytes);
        Ok(out)
    }
    pub fn layout(&self) -> &Layout {
        &self.layout
    }
    pub fn binding(&self) -> &DigBinding {
        &self.binding
    }
    pub fn journal_id(&self) -> Digest32 {
        self.journal_id
    }
    pub fn bootstrap(&self) -> &DigObservation {
        &self.bootstrap
    }
    pub fn id(&self) -> Digest32 {
        self.id
    }
    pub fn key(&self, index: usize) -> Result<String> {
        require(
            index < self.layout.regions.len(),
            "blueprint child index is outside its original layout",
        )?;
        Ok(format!("db-{}-{index:03}", self.id))
    }
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut out = DEFINITION_MAGIC.to_vec();
        out.extend_from_slice(&(self.layout.parts.len() as u16).to_be_bytes());
        for part in &self.layout.parts {
            for value in part.origin.into_iter().chain(part.size) {
                out.extend_from_slice(&value.to_be_bytes());
            }
        }
        let binding = self.binding.canonical_bytes();
        for bytes in [binding.as_slice(), self.bootstrap.canonical_bytes()] {
            out.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
            out.extend_from_slice(bytes);
        }
        out.extend_from_slice(self.journal_id.as_bytes());
        out
    }
    pub fn decode(raw: &[u8]) -> Result<Self> {
        require(
            raw.len() <= MAX_DEFINITION_BYTES,
            "blueprint definition exceeds bound",
        )?;
        let mut reader = Bytes(raw);
        require(
            reader.take(8)? == DEFINITION_MAGIC,
            "invalid blueprint parent format",
        )?;
        let count = u16::from_be_bytes(reader.array()?) as usize;
        require(
            (1..=MAX_PARTS).contains(&count),
            "invalid original blueprint part count",
        )?;
        let mut parts = Vec::with_capacity(count);
        for _ in 0..count {
            parts.push(BlueprintPart {
                origin: [reader.u32()?, reader.u32()?, reader.u32()?],
                size: [reader.u32()?, reader.u32()?, reader.u32()?],
            });
        }
        let layout = Layout::new(parts)?;
        let binding = DigBinding::from_canonical_bytes(reader.field(1024)?)?;
        let bootstrap =
            DigObservation::decode(reader.field(crate::dig_designation::MAX_CAPTURE_BYTES)?)?;
        let journal_id = Digest32::from_bytes(reader.array()?);
        require(reader.0.is_empty(), "trailing blueprint parent bytes")?;
        let out = Self::new(layout, binding, journal_id, bootstrap)?;
        require(
            out.canonical_bytes() == raw,
            "noncanonical blueprint parent bytes",
        )?;
        Ok(out)
    }
    fn check_capture(
        &self,
        region: DigRegion,
        capture: &DigObservation,
        tick: u64,
        sequence: u64,
    ) -> Result<()> {
        if capture.region() != region
            || capture.generation() != self.binding.manifest().generation
            || capture.folder() != self.binding.folder()
            || capture.site() != self.binding.site()
            || capture.dimensions() != self.bootstrap.dimensions()
            || capture.tick() < tick
            || capture.sequence() < sequence
        {
            return Err(corrupt(
                "blueprint child changed original source, geometry, dimensions or clock",
            ));
        }
        Ok(())
    }
    /// Audit a complete inventory acquired under current native-journal custody.
    /// The caller must also compare the opened journal's complete source binding.
    pub fn audit(
        &self,
        view: &DigSessionView,
        records: &[DigRecord],
        stopped: bool,
    ) -> Result<BatchProgress> {
        if view.journal_id != self.journal_id
            || view.head == Digest32::ZERO
            || view.total_records != records.len()
            || records.len() > self.layout.regions.len()
        {
            return Err(corrupt(
                "blueprint requires its complete original native journal inventory",
            ));
        }
        let expected: BTreeSet<_> = (0..self.layout.regions.len())
            .map(|index| self.key(index))
            .collect::<Result<_>>()?;
        let mut by_key = BTreeMap::new();
        for record in records {
            if !expected.contains(record.plan().key())
                || by_key.insert(record.plan().key(), record).is_some()
            {
                return Err(corrupt(
                    "foreign or duplicate child in original blueprint journal",
                ));
            }
        }
        let mut pending = records.iter().filter(|record| !record.state().terminal());
        let actual_pending = pending.next();
        if pending.next().is_some()
            || !match (actual_pending, &view.pending) {
                (None, None) => true,
                (Some(record), Some(summary)) => {
                    record.plan().key() == summary.key
                        && record.plan().digest() == summary.plan_digest
                        && record.state() == summary.state
                        && record.effect().map(DigEffect::phase) == summary.native_phase
                        && record.effect().and_then(DigEffect::receipt) == summary.receipt
                        && (!summary.dispatchable || record.state() == DigState::Prepared)
                }
                _ => false,
            }
        {
            return Err(corrupt(
                "blueprint pending identity differs from the complete inventory",
            ));
        }
        let mut out = BatchProgress {
            status: "ready",
            next_index: None,
            pending_index: None,
            designated: 0,
            total: self.layout.regions.len(),
            stopped,
            rows: Vec::with_capacity(self.layout.regions.len()),
            journal_id: view.journal_id,
            head: view.head,
        };
        let mut prefix_open = true;
        let mut tick = self.bootstrap.tick();
        let mut sequence = self.bootstrap.sequence();
        for (index, region) in self.layout.regions.iter().copied().enumerate() {
            let key = self.key(index)?;
            let record = by_key.get(key.as_str()).copied();
            if record.is_some() && !prefix_open {
                return Err(corrupt(
                    "blueprint records are not the complete original ordered prefix",
                ));
            }
            if let Some(record) = record {
                let plan = record.plan();
                self.check_capture(region, plan.before(), tick, sequence)?;
                if plan.allow_hidden_neighbors() {
                    return Err(corrupt(
                        "blueprint child relaxed the original visible-neighbor policy",
                    ));
                }
                if let Some(effect) = record.effect() {
                    DigEffect::decode(effect.canonical_bytes(), plan)?;
                }
            }
            let phase = record.and_then(DigRecord::effect).map(DigEffect::phase);
            let designated = record.is_some_and(|record| record.state() == DigState::Terminal)
                && phase == Some(DigPhase::Designated);
            if designated {
                let before = record
                    .ok_or_else(|| corrupt("missing designated blueprint child"))?
                    .plan()
                    .before();
                tick = before.tick();
                sequence = before
                    .sequence()
                    .checked_add(1)
                    .ok_or_else(|| corrupt("blueprint native sequence exhausted"))?;
                out.designated += 1;
            } else if prefix_open {
                prefix_open = false;
                match record {
                    None if !stopped => out.next_index = Some(index),
                    None => {}
                    Some(record)
                        if record.state() == DigState::Terminal
                            && phase == Some(DigPhase::Refused) =>
                    {
                        out.status = if record
                            .effect()
                            .is_some_and(|e| e.reason() == DigReason::CancelledBeforeDispatch)
                        {
                            "halted_cancelled"
                        } else {
                            "halted_refused"
                        };
                    }
                    Some(record) if record.permanent_unknown() => {
                        out.status = "halted_unknown";
                        out.pending_index = Some(index);
                    }
                    Some(_) => {
                        out.status = "pending_recovery";
                        out.pending_index = Some(index);
                    }
                }
            }
            out.rows.push(BatchRow {
                index,
                region,
                key,
                native_plan_digest: record.map(|r| r.plan().digest()),
                state: record.map_or("not_started", |r| r.state().as_str()),
                outcome: phase.map(DigPhase::as_str),
                receipt: record
                    .and_then(DigRecord::effect)
                    .and_then(DigEffect::receipt),
            });
        }
        if prefix_open {
            out.status = "all_designated";
        } else if stopped && out.status == "ready" {
            out.status = "stopped";
        }
        Ok(out)
    }
    /// An additional guard around one original next child; never a permit.
    pub fn validate_next(
        &self,
        view: &DigSessionView,
        records: &[DigRecord],
        stopped: bool,
        key: &str,
        region: DigRegion,
        capture: Option<&DigObservation>,
    ) -> Result<()> {
        let progress = self.audit(view, records, stopped)?;
        require(!stopped, "blueprint advancement is permanently stopped")?;
        require(
            progress.status != "halted_unknown",
            "unknown blueprint outcome cannot advance",
        )?;
        let index = progress
            .next_index
            .or(progress.pending_index)
            .ok_or_else(|| invalid("original blueprint has no next designation"))?;
        require(
            self.key(index)? == key && self.layout.regions[index] == region,
            "mining request substitutes the original next blueprint child",
        )?;
        if let Some(record) = records.iter().find(|r| r.plan().key() == key) {
            require(
                matches!(
                    record.state(),
                    DigState::Intent | DigState::Prepared | DigState::DispatchStarted
                ),
                "retained blueprint child requires original-key recovery",
            )?;
            if let Some(capture) = capture {
                require(
                    capture == record.plan().before(),
                    "prepared blueprint child capture changed",
                )?;
            }
        }
        if let Some(capture) = capture {
            let mut tick = self.bootstrap.tick();
            let mut sequence = self.bootstrap.sequence();
            if index > 0 {
                let previous_key = self.key(index - 1)?;
                let previous = records
                    .iter()
                    .find(|r| r.plan().key() == previous_key)
                    .ok_or_else(|| corrupt("blueprint predecessor is absent"))?;
                tick = previous.plan().before().tick();
                sequence = previous
                    .plan()
                    .before()
                    .sequence()
                    .checked_add(1)
                    .ok_or_else(|| corrupt("blueprint native sequence exhausted"))?;
            }
            self.check_capture(region, capture, tick, sequence)?;
            require(
                capture.blockers(false).is_empty(),
                "blueprint next region is not eligible under its original policy",
            )?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BatchRow {
    pub index: usize,
    pub region: DigRegion,
    pub key: String,
    pub native_plan_digest: Option<Digest32>,
    pub state: &'static str,
    pub outcome: Option<&'static str>,
    pub receipt: Option<Digest32>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BatchProgress {
    pub status: &'static str,
    pub next_index: Option<usize>,
    pub pending_index: Option<usize>,
    pub designated: usize,
    pub total: usize,
    pub stopped: bool,
    pub rows: Vec<BatchRow>,
    pub journal_id: Digest32,
    pub head: Digest32,
}

struct Bytes<'a>(&'a [u8]);
impl<'a> Bytes<'a> {
    fn take(&mut self, length: usize) -> Result<&'a [u8]> {
        require(length <= self.0.len(), "truncated blueprint parent bytes")?;
        let (value, remaining) = self.0.split_at(length);
        self.0 = remaining;
        Ok(value)
    }
    fn array<const N: usize>(&mut self) -> Result<[u8; N]> {
        self.take(N)?
            .try_into()
            .map_err(|_| invalid("invalid blueprint field width"))
    }
    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_be_bytes(self.array()?))
    }
    fn field(&mut self, maximum: usize) -> Result<&'a [u8]> {
        let length = self.u32()? as usize;
        require(length <= maximum, "blueprint parent field exceeds bound")?;
        self.take(length)
    }
}

#[cfg(test)]
#[path = "dig_blueprint_tests.rs"]
mod tests;
