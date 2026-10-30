//! Sampled completion of one entire retained sparse mining blueprint.
//!
//! Map/1.5 and dig/1.16 have independent generations. Exact source selectors
//! associate their evidence; they never establish shared-process identity,
//! mining causality, current terrain after a read, or mutation authority.
//! Progress is reconstructed from typed raw captures and fixed transitions.
//! Beads: df-dfhack-bridge-plane-c-pic.4, df-action-coordinator-exec-ero.4.

use std::collections::BTreeMap;
use std::net::SocketAddr;

use dfmcp_core::{
    Capability, DfmcpError, Digest32, ErrorCode, GameTick, MapCoord, MapCuboid, OperationContext,
    Result, RiskTier,
};
use dfmcp_world::map_region::{Cell, Region, Shape};

use super::{BatchDefinition, MAX_DEFINITION_BYTES, MAX_VOLUME};
use crate::dig_designation::journal::session::DigSessionView;
use crate::dig_designation::journal::{DigRecord, DigState};
use crate::dig_designation::{DigEffect, DigPhase, MAX_NATIVE_TICK};
use crate::live_map::LiveMapObservation;

pub mod io;
pub mod store;

pub const MAX_SPEC_BYTES: usize = 64 * 1024;
pub const MAX_CAPTURE_BYTES: usize = 24 * 1024;
pub const MAX_SOURCE_BYTES: usize = 512;
pub const MAX_ATTEMPTS: u32 = 128;
pub const MAX_REQUIRED_SAMPLES: u32 = 64;
pub const PREDICATE: &str = "visible-dry-cleared-floor/1";
const SOURCE_MAGIC: &[u8; 8] = b"DFMBMS01";
const SPEC_MAGIC: &[u8; 8] = b"DFMBMG01";
const CAPTURE_MAGIC: &[u8; 8] = b"DFMBMC01";

fn error(code: ErrorCode, message: &str) -> DfmcpError {
    DfmcpError::new(code, message)
}
fn invalid(message: &str) -> DfmcpError {
    error(ErrorCode::InvalidRequest, message)
}
fn stale(message: &str) -> DfmcpError {
    error(ErrorCode::StaleAnchor, message)
}
fn require(value: bool, message: &str) -> Result<()> {
    if value { Ok(()) } else { Err(invalid(message)) }
}
fn hash(domain: &[u8], bytes: &[u8]) -> Digest32 {
    let mut raw = domain.to_vec();
    raw.push(0);
    raw.extend_from_slice(bytes);
    Digest32::of_bytes(&raw)
}
fn text(out: &mut Vec<u8>, value: &str) {
    out.extend_from_slice(&(value.len() as u16).to_be_bytes());
    out.extend_from_slice(value.as_bytes());
}
fn blob(out: &mut Vec<u8>, value: &[u8]) {
    out.extend_from_slice(&(value.len() as u32).to_be_bytes());
    out.extend_from_slice(value);
}

/// Independently supplied map source acceptance. Stored bytes only compare this
/// identity; reopening must receive current host authority separately.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourcePin {
    endpoint: SocketAddr,
    generation: u64,
    df_version: String,
    dfhack_version: String,
}
impl SourcePin {
    pub fn new(
        endpoint: SocketAddr,
        generation: u64,
        df_version: String,
        dfhack_version: String,
    ) -> Result<Self> {
        require(
            endpoint.ip().is_loopback()
                && endpoint.port() != 0
                && generation != 0
                && generation != u64::MAX
                && [&df_version, &dfhack_version].iter().all(|value| {
                    !value.is_empty() && value.len() <= 128 && !value.chars().any(char::is_control)
                }),
            "invalid independent map source pin",
        )?;
        Ok(Self {
            endpoint,
            generation,
            df_version,
            dfhack_version,
        })
    }
    pub fn endpoint(&self) -> SocketAddr {
        self.endpoint
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn df_version(&self) -> &str {
        &self.df_version
    }
    pub fn dfhack_version(&self) -> &str {
        &self.dfhack_version
    }
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut raw = SOURCE_MAGIC.to_vec();
        text(&mut raw, &self.endpoint.to_string());
        raw.extend_from_slice(&self.generation.to_be_bytes());
        text(&mut raw, &self.df_version);
        text(&mut raw, &self.dfhack_version);
        raw
    }
    pub fn decode(raw: &[u8]) -> Result<Self> {
        require(
            raw.len() <= MAX_SOURCE_BYTES,
            "map source pin exceeds bound",
        )?;
        let mut r = Reader(raw);
        require(r.take(8)? == SOURCE_MAGIC, "wrong map source pin format")?;
        let endpoint = r
            .text(128)?
            .parse()
            .map_err(|_| invalid("invalid map endpoint"))?;
        let value = Self::new(endpoint, r.u64()?, r.text(128)?, r.text(128)?)?;
        require(
            r.0.is_empty() && value.canonical_bytes() == raw,
            "noncanonical map source pin",
        )?;
        Ok(value)
    }
}

/// Fixed inclusive deadline and whole-mask sampled stability. There is no
/// implicit game advancement or minimum polling cadence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Policy {
    deadline_tick: u64,
    stable_ticks: u64,
    required_samples: u32,
    max_gap_ticks: u64,
    max_samples: u32,
}
impl Policy {
    pub fn new(
        deadline_tick: u64,
        stable_ticks: u64,
        required_samples: u32,
        max_gap_ticks: u64,
        max_samples: u32,
    ) -> Result<Self> {
        require(
            deadline_tick <= MAX_NATIVE_TICK
                && stable_ticks <= 403_200
                && (1..=MAX_REQUIRED_SAMPLES).contains(&required_samples)
                && (1..=403_200).contains(&max_gap_ticks)
                && (1..=MAX_ATTEMPTS).contains(&max_samples)
                && required_samples <= max_samples,
            "invalid bounded blueprint monitoring policy",
        )?;
        Ok(Self {
            deadline_tick,
            stable_ticks,
            required_samples,
            max_gap_ticks,
            max_samples,
        })
    }
    pub fn deadline_tick(&self) -> u64 {
        self.deadline_tick
    }
    pub fn stable_ticks(&self) -> u64 {
        self.stable_ticks
    }
    pub fn required_samples(&self) -> u32 {
        self.required_samples
    }
    pub fn max_gap_ticks(&self) -> u64 {
        self.max_gap_ticks
    }
    pub fn max_samples(&self) -> u32 {
        self.max_samples
    }
    fn append(&self, raw: &mut Vec<u8>) {
        raw.extend_from_slice(&self.deadline_tick.to_be_bytes());
        raw.extend_from_slice(&self.stable_ticks.to_be_bytes());
        raw.extend_from_slice(&self.required_samples.to_be_bytes());
        raw.extend_from_slice(&self.max_gap_ticks.to_be_bytes());
        raw.extend_from_slice(&self.max_samples.to_be_bytes());
    }
    fn read(r: &mut Reader<'_>) -> Result<Self> {
        Self::new(r.u64()?, r.u64()?, r.u32()?, r.u64()?, r.u32()?)
    }
    fn feasible(&self, start_tick: u64) -> Result<()> {
        let horizon = self
            .deadline_tick
            .checked_sub(start_tick)
            .ok_or_else(|| invalid("blueprint monitor deadline precedes its fixed start"))?;
        require(
            self.stable_ticks <= horizon
                && u64::from(self.required_samples - 1) <= horizon
                && self.stable_ticks <= self.max_gap_ticks * u64::from(self.max_samples - 1),
            "blueprint monitoring stability cannot fit its fixed horizon and attempts",
        )
    }
}

fn enclosure(parent: &BatchDefinition) -> Result<Region> {
    let mut low = [u32::MAX; 3];
    let mut high = [0u32; 3];
    for part in parent.layout().parts() {
        for axis in 0..3 {
            low[axis] = low[axis].min(part.origin[axis]);
            high[axis] = high[axis].max(part.origin[axis] + part.size[axis]);
        }
    }
    let region = Region {
        origin: low,
        size: std::array::from_fn(|axis| high[axis] - low[axis]),
    };
    require(
        region.volume().is_ok_and(|n| n as u64 <= MAX_VOLUME),
        "blueprint monitor requires one complete bounded enclosing capture",
    )?;
    Ok(region)
}
fn cuboid(region: Region) -> Result<MapCuboid> {
    region.volume().map_err(crate::live_map::map_error)?;
    MapCuboid::new(
        MapCoord::new(
            region.origin[0] as i32,
            region.origin[1] as i32,
            region.origin[2] as i32,
        ),
        MapCoord::new(
            (region.origin[0] + region.size[0] - 1) as i32,
            (region.origin[1] + region.size[1] - 1) as i32,
            (region.origin[2] + region.size[2] - 1) as i32,
        ),
    )
}

fn designation_set(
    parent: &BatchDefinition,
    view: &DigSessionView,
    records: &[DigRecord],
) -> Result<(Digest32, u64)> {
    let progress = parent.audit(view, records, false)?;
    if progress.designated != progress.total
        || progress.pending_index.is_some()
        || progress.next_index.is_some()
        || records.len() != parent.layout().regions().len()
    {
        return Err(error(
            ErrorCode::EffectIndeterminate,
            "whole blueprint monitoring requires every original terminal designation receipt",
        ));
    }
    let by_key: BTreeMap<_, _> = records
        .iter()
        .map(|record| (record.plan().key(), record))
        .collect();
    let mut raw = parent.id().as_bytes().to_vec();
    raw.extend_from_slice(parent.journal_id().as_bytes());
    raw.extend_from_slice(parent.layout().digest().as_bytes());
    raw.extend_from_slice(&(records.len() as u16).to_be_bytes());
    let mut floor = parent.bootstrap().tick();
    for index in 0..parent.layout().regions().len() {
        let key = parent.key(index)?;
        let record = by_key
            .get(key.as_str())
            .ok_or_else(|| invalid("original designation record missing"))?;
        let effect = record
            .effect()
            .filter(|effect| {
                record.state() == DigState::Terminal && effect.phase() == DigPhase::Designated
            })
            .ok_or_else(|| invalid("original designation receipt missing"))?;
        DigEffect::decode(effect.canonical_bytes(), record.plan())?;
        let receipt = effect
            .receipt()
            .ok_or_else(|| invalid("terminal designation has no receipt"))?;
        text(&mut raw, &key);
        raw.extend_from_slice(record.plan().digest().as_bytes());
        raw.extend_from_slice(receipt.as_bytes());
        floor = floor.max(record.plan().before().tick());
    }
    Ok((
        hash(b"dfmcp-blueprint-monitor-designation-set/1", &raw),
        floor,
    ))
}

/// Immutable original geometry, independent source pin and receipt-set link.
/// Decoding restores retained identity, never verified current original custody;
/// callers must call `verify_original` before a combined evidence claim.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MonitorSpec {
    parent: BatchDefinition,
    source: SourcePin,
    policy: Policy,
    start_tick: u64,
    minimum_native_tick: u64,
    native_head: Digest32,
    receipt_digest: Digest32,
    region: Region,
    id: Digest32,
}
impl MonitorSpec {
    pub fn new(
        parent: BatchDefinition,
        view: &DigSessionView,
        records: &[DigRecord],
        source: SourcePin,
        policy: Policy,
        start_tick: u64,
    ) -> Result<Self> {
        let (receipt_digest, minimum_native_tick) = designation_set(&parent, view, records)?;
        Self::retained(
            parent,
            source,
            policy,
            start_tick,
            minimum_native_tick,
            view.head,
            receipt_digest,
        )
    }
    fn retained(
        parent: BatchDefinition,
        source: SourcePin,
        policy: Policy,
        start_tick: u64,
        minimum_native_tick: u64,
        native_head: Digest32,
        receipt_digest: Digest32,
    ) -> Result<Self> {
        let binding = parent.binding();
        require(
            source.endpoint == binding.endpoint()
                && source.df_version == binding.manifest().df_version
                && source.dfhack_version == binding.manifest().dfhack_version
                && native_head != Digest32::ZERO
                && receipt_digest != Digest32::ZERO
                && minimum_native_tick >= parent.bootstrap().tick()
                && start_tick >= minimum_native_tick
                && start_tick <= MAX_NATIVE_TICK,
            "monitor source selectors, original receipts or starting clock differ",
        )?;
        policy.feasible(start_tick)?;
        let region = enclosure(&parent)?;
        let mut value = Self {
            parent,
            source,
            policy,
            start_tick,
            minimum_native_tick,
            native_head,
            receipt_digest,
            region,
            id: Digest32::ZERO,
        };
        let raw = value.canonical_bytes();
        require(
            raw.len() <= MAX_SPEC_BYTES,
            "blueprint monitor definition exceeds bound",
        )?;
        value.id = hash(b"dfmcp-blueprint-map-monitor/1", &raw);
        Ok(value)
    }
    pub fn parent(&self) -> &BatchDefinition {
        &self.parent
    }
    pub fn source(&self) -> &SourcePin {
        &self.source
    }
    pub fn policy(&self) -> &Policy {
        &self.policy
    }
    pub fn region(&self) -> Region {
        self.region
    }
    pub fn id(&self) -> Digest32 {
        self.id
    }
    pub fn native_head(&self) -> Digest32 {
        self.native_head
    }
    pub fn receipt_digest(&self) -> Digest32 {
        self.receipt_digest
    }
    pub fn start_tick(&self) -> u64 {
        self.start_tick
    }
    pub fn minimum_native_tick(&self) -> u64 {
        self.minimum_native_tick
    }
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut raw = SPEC_MAGIC.to_vec();
        blob(&mut raw, &self.parent.canonical_bytes());
        blob(&mut raw, &self.source.canonical_bytes());
        self.policy.append(&mut raw);
        raw.extend_from_slice(&self.start_tick.to_be_bytes());
        raw.extend_from_slice(&self.minimum_native_tick.to_be_bytes());
        raw.extend_from_slice(self.native_head.as_bytes());
        raw.extend_from_slice(self.receipt_digest.as_bytes());
        raw
    }
    pub fn decode(raw: &[u8]) -> Result<Self> {
        require(
            raw.len() <= MAX_SPEC_BYTES,
            "blueprint monitor definition exceeds bound",
        )?;
        let mut r = Reader(raw);
        require(
            r.take(8)? == SPEC_MAGIC,
            "wrong blueprint monitor definition format",
        )?;
        let parent = BatchDefinition::decode(r.blob(MAX_DEFINITION_BYTES)?)?;
        let source = SourcePin::decode(r.blob(MAX_SOURCE_BYTES)?)?;
        let policy = Policy::read(&mut r)?;
        let value = Self::retained(
            parent,
            source,
            policy,
            r.u64()?,
            r.u64()?,
            Digest32::from_bytes(r.array()?),
            Digest32::from_bytes(r.array()?),
        )?;
        require(
            r.0.is_empty() && value.canonical_bytes() == raw,
            "noncanonical blueprint monitor definition",
        )?;
        Ok(value)
    }
    /// Verify actual complete native custody again. Parent stop state is not
    /// part of this immutable receipt set and does not cancel observation.
    pub fn verify_original(
        &self,
        parent: &BatchDefinition,
        view: &DigSessionView,
        records: &[DigRecord],
    ) -> Result<()> {
        if parent != &self.parent || view.head != self.native_head {
            return Err(error(
                ErrorCode::CorruptLedger,
                "original blueprint or complete native head changed",
            ));
        }
        let (receipt_digest, minimum_native_tick) = designation_set(parent, view, records)?;
        if receipt_digest != self.receipt_digest || minimum_native_tick != self.minimum_native_tick
        {
            return Err(error(
                ErrorCode::CorruptLedger,
                "original blueprint designation evidence changed",
            ));
        }
        Ok(())
    }
    pub fn validate_capture(&self, capture: &LiveMapObservation) -> Result<()> {
        capture.validate()?;
        let binding = self.parent.binding();
        if capture.bridge_generation != self.source.generation
            || capture.df_version != self.source.df_version
            || capture.dfhack_version != self.source.dfhack_version
            || capture.world_folder != binding.folder()
            || capture.site_id != binding.site()
            || capture.map_dimensions != self.parent.bootstrap().dimensions()
            || capture.map.region != self.region
            || !capture.paused
            || capture.tick().get() < self.start_tick
        {
            return Err(stale(
                "map sample changed original monitor source, region, pause or clock",
            ));
        }
        Ok(())
    }
    /// Scoped current read authority only. The owner supplies any newer consumed
    /// clock floor; retained source bytes cannot reconstruct capability grants.
    pub fn authorize(&self, context: &OperationContext, observe: bool) -> Result<()> {
        context.budget.validate()?;
        if context.anchor.fortress_id != self.parent.binding().fortress_id() {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "blueprint monitor belongs to another fortress",
            ));
        }
        let mut current = context.clone();
        current.anchor.tick = current.anchor.tick.max(GameTick(self.start_tick));
        current.authorize(
            Capability::Query,
            RiskTier::ReadOnly,
            &[],
            Some(self.parent.binding().scope()),
        )?;
        if observe {
            current.authorize(
                Capability::Observe,
                RiskTier::ReadOnly,
                &[],
                Some(cuboid(self.region)?),
            )?;
            if self.region.volume().map_err(crate::live_map::map_error)?
                > current.budget.max_entities as usize
            {
                return Err(error(
                    ErrorCode::BudgetExceeded,
                    "complete blueprint map capture exceeds entity allowance",
                ));
            }
        }
        Ok(())
    }
    pub fn classify(&self, capture: &LiveMapObservation) -> Result<Counts> {
        self.validate_capture(capture)?;
        let mut counts = Counts::default();
        for position in self.parent.layout().targets() {
            let index = self
                .region
                .index(*position)
                .ok_or_else(|| invalid("original target lies outside its enclosing capture"))?;
            match capture
                .map
                .cells
                .get(index)
                .ok_or_else(|| invalid("complete blueprint map cell missing"))?
            {
                Cell::Hidden => counts.hidden += 1,
                Cell::Unallocated => counts.missing += 1,
                Cell::Visible(tile) => {
                    let wrong_shape = tile.shape != Shape::Floor;
                    let wet = tile.liquid_depth != 0 || tile.magma;
                    let designated = tile.dig_designation != 0;
                    counts.wrong_shape += u32::from(wrong_shape);
                    counts.wet += u32::from(wet);
                    counts.designated += u32::from(designated);
                    if wrong_shape || wet || designated {
                        counts.mismatched += 1;
                    } else {
                        counts.matched += 1;
                    }
                }
            }
        }
        Ok(counts)
    }
}

/// The first four counts partition the exact selected set. Deficit categories
/// may overlap. Unselected holes are never counted as targets.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub matched: u32,
    pub mismatched: u32,
    pub hidden: u32,
    pub missing: u32,
    pub wrong_shape: u32,
    pub wet: u32,
    pub designated: u32,
}
impl Counts {
    pub fn total(self) -> u32 {
        self.matched + self.mismatched + self.hidden + self.missing
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Pending,
    Unknown,
    Stabilizing,
    Satisfied,
    Expired,
    Invalidated,
    Cancelled,
}
impl Status {
    pub fn terminal(self) -> bool {
        matches!(
            self,
            Self::Satisfied | Self::Expired | Self::Invalidated | Self::Cancelled
        )
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Unknown => "unknown",
            Self::Stabilizing => "stabilizing",
            Self::Satisfied => "satisfied",
            Self::Expired => "expired",
            Self::Invalidated => "invalidated",
            Self::Cancelled => "cancelled",
        }
    }
}

/// Pure replay state. A satisfied sample is historical; this type does not
/// assert that any caller just acquired it or currently holds original custody.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MonitorState {
    spec: MonitorSpec,
    high_tick: u64,
    attempts: u32,
    latest: Option<LiveMapObservation>,
    first_satisfied: Option<LiveMapObservation>,
    status: Status,
    streak: u32,
    since_tick: Option<u64>,
    pending: bool,
}
impl MonitorState {
    pub fn new(spec: MonitorSpec) -> Self {
        Self {
            high_tick: spec.start_tick,
            spec,
            attempts: 0,
            latest: None,
            first_satisfied: None,
            status: Status::Pending,
            streak: 0,
            since_tick: None,
            pending: false,
        }
    }
    pub fn spec(&self) -> &MonitorSpec {
        &self.spec
    }
    pub fn high_tick(&self) -> u64 {
        self.high_tick
    }
    pub fn attempts(&self) -> u32 {
        self.attempts
    }
    pub fn latest(&self) -> Option<&LiveMapObservation> {
        self.latest.as_ref()
    }
    pub fn first_satisfied(&self) -> Option<&LiveMapObservation> {
        self.first_satisfied.as_ref()
    }
    pub fn status(&self) -> Status {
        self.status
    }
    pub fn streak(&self) -> u32 {
        self.streak
    }
    pub fn since_tick(&self) -> Option<u64> {
        self.since_tick
    }
    pub fn pending(&self) -> bool {
        self.pending
    }
    pub fn counts(&self) -> Result<Option<Counts>> {
        self.latest
            .as_ref()
            .map(|raw| self.spec.classify(raw))
            .transpose()
    }
    fn unfinished(&self) -> Result<()> {
        require(
            !self.status.terminal(),
            "terminal blueprint monitor history is immutable",
        )
    }
    fn reset(&mut self, status: Status) {
        self.status = status;
        self.streak = 0;
        self.since_tick = None;
        self.pending = false;
    }
    /// The durable shell calls this before publishing read intent or connecting.
    /// Failed and unfinished reads consume the same finite attempt allowance.
    pub fn begin_attempt(&self) -> Result<Self> {
        self.unfinished()?;
        require(
            !self.pending,
            "blueprint monitoring read is already pending",
        )?;
        if self.attempts >= self.spec.policy.max_samples {
            return Err(error(
                ErrorCode::BudgetExceeded,
                "blueprint monitoring attempt allowance exhausted",
            ));
        }
        let mut next = self.clone();
        next.attempts += 1;
        next.pending = true;
        Ok(next)
    }
    pub fn sample(&self, capture: LiveMapObservation) -> Result<Self> {
        self.unfinished()?;
        capture.validate()?;
        require(
            capture
                .map
                .region
                .volume()
                .is_ok_and(|n| n as u64 <= MAX_VOLUME),
            "monitor sample exceeds complete capture bound",
        )?;
        let mut next = if self.pending {
            self.clone()
        } else {
            self.begin_attempt()?
        };
        let tick = capture.tick().get();
        let regressed = tick < self.high_tick;
        next.high_tick = next.high_tick.max(tick);
        next.pending = false;
        if self.spec.validate_capture(&capture).is_err() || regressed {
            next.reset(Status::Invalidated);
            return Ok(next); // Preserve the last accepted original-source sample.
        }
        let counts = self.spec.classify(&capture)?;
        let advanced = self
            .latest
            .as_ref()
            .is_none_or(|old| capture.tick() > old.tick());
        let gap = self
            .latest
            .as_ref()
            .is_some_and(|old| tick - old.tick().get() > self.spec.policy.max_gap_ticks);
        next.latest = Some(capture);
        if gap {
            next.streak = 0;
            next.since_tick = None;
        }
        if tick > self.spec.policy.deadline_tick {
            next.reset(Status::Expired);
        } else if counts.hidden != 0 || counts.missing != 0 {
            next.reset(Status::Unknown);
        } else if counts.mismatched != 0 {
            next.reset(Status::Pending);
        } else {
            if next.streak == 0 {
                next.streak = 1;
                next.since_tick = Some(tick);
            } else if advanced {
                next.streak += 1;
            }
            let since = next
                .since_tick
                .ok_or_else(|| invalid("matching blueprint streak lost its start"))?;
            if next.streak >= self.spec.policy.required_samples
                && tick - since >= self.spec.policy.stable_ticks
            {
                next.status = Status::Satisfied;
                if next.first_satisfied.is_none() {
                    next.first_satisfied = next.latest.clone();
                }
            } else {
                next.status = Status::Stabilizing;
            }
        }
        Ok(next)
    }
    pub fn interrupt(&self) -> Result<Self> {
        self.unfinished()?;
        let mut next = self.clone();
        next.reset(Status::Unknown);
        Ok(next)
    }
    pub fn invalidate(&self) -> Result<Self> {
        self.unfinished()?;
        let mut next = self.clone();
        next.reset(Status::Invalidated);
        Ok(next)
    }
    pub fn cancel(&self) -> Result<Self> {
        self.unfinished()?;
        let mut next = self.clone();
        next.reset(Status::Cancelled);
        Ok(next)
    }
    /// Reopening keeps terminal historical evidence, never a fresh proof flag.
    /// Unfinished stability cannot silently bridge a lost process owner.
    pub fn reopened(&self) -> Result<Self> {
        if self.status.terminal() {
            Ok(self.clone())
        } else {
            self.interrupt()
        }
    }
}

/// Exact map/1.5 source capsule used by the history owner. The map decoder
/// validates every field, including unselected cells, before replay.
pub fn encode_capture(capture: &LiveMapObservation) -> Result<Vec<u8>> {
    capture.validate()?;
    require(
        capture
            .map
            .region
            .volume()
            .is_ok_and(|n| n as u64 <= MAX_VOLUME),
        "blueprint monitor capture exceeds 1024 cells",
    )?;
    let mut raw = CAPTURE_MAGIC.to_vec();
    raw.extend_from_slice(&capture.bridge_generation.to_be_bytes());
    text(&mut raw, &capture.df_version);
    text(&mut raw, &capture.dfhack_version);
    blob(&mut raw, &capture.encode_payload()?);
    require(
        raw.len() <= MAX_CAPTURE_BYTES,
        "blueprint monitor capture exceeds byte bound",
    )?;
    Ok(raw)
}
pub fn decode_capture(raw: &[u8]) -> Result<LiveMapObservation> {
    require(
        raw.len() <= MAX_CAPTURE_BYTES,
        "blueprint monitor capture exceeds byte bound",
    )?;
    let mut r = Reader(raw);
    require(
        r.take(8)? == CAPTURE_MAGIC,
        "wrong blueprint monitor capture format",
    )?;
    let generation = r.u64()?;
    let df = r.text(128)?;
    let dfhack = r.text(128)?;
    let capture =
        LiveMapObservation::decode_payload(r.blob(MAX_CAPTURE_BYTES)?, generation, df, dfhack)?;
    require(
        r.0.is_empty() && encode_capture(&capture)? == raw,
        "noncanonical blueprint monitor capture",
    )?;
    Ok(capture)
}
pub fn capture_digest(capture: &LiveMapObservation) -> Result<Digest32> {
    Ok(hash(
        b"dfmcp-blueprint-monitor-map-capture/1",
        &encode_capture(capture)?,
    ))
}

struct Reader<'a>(&'a [u8]);
impl<'a> Reader<'a> {
    fn take(&mut self, count: usize) -> Result<&'a [u8]> {
        let part = self
            .0
            .get(..count)
            .ok_or_else(|| invalid("truncated blueprint monitor bytes"))?;
        self.0 = &self.0[count..];
        Ok(part)
    }
    fn array<const N: usize>(&mut self) -> Result<[u8; N]> {
        self.take(N)?
            .try_into()
            .map_err(|_| invalid("truncated blueprint monitor field"))
    }
    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_be_bytes(self.array()?))
    }
    fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_be_bytes(self.array()?))
    }
    fn text(&mut self, maximum: usize) -> Result<String> {
        let count = u16::from_be_bytes(self.array()?) as usize;
        require(count <= maximum, "blueprint monitor text exceeds bound")?;
        std::str::from_utf8(self.take(count)?)
            .map(str::to_owned)
            .map_err(|_| invalid("blueprint monitor text is not UTF-8"))
    }
    fn blob(&mut self, maximum: usize) -> Result<&'a [u8]> {
        let count = self.u32()? as usize;
        require(count <= maximum, "blueprint monitor field exceeds bound")?;
        self.take(count)
    }
}

#[cfg(test)]
mod tests;
