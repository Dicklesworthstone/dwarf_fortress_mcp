//! Durable exact original dig excavation evidence; no restored grants.
//!
//! This private event codec reuses the existing LiveSpatialCitizenObservation canonical bytes.
//! Framing, exclusive storage custody, sync and exact readback follow the native
//! semantic work-order history shell. A fingerprint compares independently
//! reissued policy; there is no policy decoder or mutation authority here.

use std::io::{self, SeekFrom};
use std::path::Path;

use dfmcp_core::{
    Capability, Digest32, ErrorCode, GameTick, ObservationCursor, OperationContext, Result,
    RiskTier, SessionId,
};
use dfmcp_world::EvidencePolicy;

use crate::bounded_run::hash;
use crate::build_placement::journal::BuildMode;
use crate::build_placement::journal::private_file::{PrivateBuildFile, open_private_storage};
use crate::dig_designation::{DigEffect, DigPhase, DigPlan, MAX_EFFECT_BYTES};
use crate::live_spatial::citizens::{LiveSpatialCitizenObservation, MAX_SPATIAL_CITIZEN_BYTES};
const MAX_SAMPLE: usize = MAX_SPATIAL_CITIZEN_BYTES + 4096;
use crate::control_effect_journal::EffectJournalStorage;

use super::super::goal_projection::DigGoalProjection;
use super::super::{Call, SemanticDigReview, custody, error, exhausted};

/// Explicit bounded retention, without silent pruning, repair or rollover.
pub const MAX_GOAL_HISTORY_BYTES: usize = 64 * 1024 * 1024;
// Fresh review/dispatch samples plus cadence polls retain all 64 stable samples.
pub const MAX_GOAL_HISTORY_EVENTS: usize = 1024;
const HEADER: &[u8; 8] = b"DFMDGH01";
const FRAME: &[u8; 8] = b"DFMDGF01";
const END: &[u8; 8] = b"DFMDGE01";
const HEADER_BYTES: usize = 112;
const OVERHEAD: usize = 92;
pub(super) const MAX_EVENT_BYTES: usize = MAX_SAMPLE + 1;
/// The final interruption remains prepaid even after a failed publication ack.
pub(super) const MAX_POLL_EVENTS: usize = 4;
pub(super) const MAX_POLL_BYTES: usize =
    26 + 1 + MAX_SAMPLE + 37 + MAX_EFFECT_BYTES + 10 + MAX_POLL_EVENTS * OVERHEAD;

fn io_error(_: io::Error) -> dfmcp_core::DfmcpError {
    error(
        ErrorCode::CorruptLedger,
        "dig goal history custody or I/O failed; reopen without repair",
    )
}

#[derive(Clone, Debug)]
pub(super) enum Event {
    Started {
        cursor: ObservationCursor,
        tick: GameTick,
        interrupted: bool,
    },
    Captured(Box<LiveSpatialCitizenObservation>),
    Published {
        fingerprint: Digest32,
        receipt: DigEffect,
    },
    Interrupted {
        tick: GameTick,
        fenced: bool,
    },
}

fn blob(out: &mut Vec<u8>, bytes: &[u8]) -> Result<()> {
    out.extend_from_slice(
        &u32::try_from(bytes.len())
            .map_err(|_| exhausted())?
            .to_be_bytes(),
    );
    out.extend_from_slice(bytes);
    Ok(())
}

impl Event {
    pub(super) fn maximum_bytes(&self) -> usize {
        match self {
            Self::Started { .. } => 26,
            Self::Captured(_) => MAX_EVENT_BYTES,
            Self::Published { .. } => 37 + MAX_EFFECT_BYTES,
            Self::Interrupted { .. } => 10,
        }
    }
    fn encode(&self) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        match self {
            Self::Started {
                cursor,
                tick,
                interrupted,
            } => {
                out.push(0);
                out.extend_from_slice(&cursor.epoch.to_be_bytes());
                out.extend_from_slice(&cursor.sequence.to_be_bytes());
                out.extend_from_slice(&tick.get().to_be_bytes());
                out.push(u8::from(*interrupted));
            }
            Self::Captured(sample) => {
                out.push(1);
                let map = sample.spatial().terrain();
                out.extend_from_slice(&map.bridge_generation.to_be_bytes());
                blob(&mut out, map.df_version.as_bytes())?;
                blob(&mut out, map.dfhack_version.as_bytes())?;
                blob(&mut out, &sample.encode_payload()?)?;
            }
            Self::Published {
                fingerprint,
                receipt,
            } => {
                out.push(2);
                out.extend_from_slice(fingerprint.as_bytes());
                blob(&mut out, receipt.canonical_bytes())?;
            }
            Self::Interrupted { tick, fenced } => {
                out.push(3);
                out.extend_from_slice(&tick.get().to_be_bytes());
                out.push(u8::from(*fenced));
            }
        }
        if out.len() > MAX_EVENT_BYTES {
            return Err(exhausted());
        }
        Ok(out)
    }
    fn decode(bytes: &[u8], plan: &DigPlan) -> Result<Self> {
        if bytes.len() > MAX_EVENT_BYTES {
            return Err(custody());
        }
        let mut r = Reader(bytes);
        let event = match r.array::<1>()?[0] {
            0 => Self::Started {
                cursor: ObservationCursor {
                    epoch: u64::from_be_bytes(r.array()?),
                    sequence: u64::from_be_bytes(r.array()?),
                },
                tick: GameTick(u64::from_be_bytes(r.array()?)),
                interrupted: r.boolean()?,
            },
            1 => {
                let generation = u64::from_be_bytes(r.array()?);
                let df = std::str::from_utf8(r.blob(128)?)
                    .map_err(|_| custody())?
                    .to_owned();
                let dfhack = std::str::from_utf8(r.blob(128)?)
                    .map_err(|_| custody())?
                    .to_owned();
                let sample = LiveSpatialCitizenObservation::decode_payload(
                    r.blob(MAX_SPATIAL_CITIZEN_BYTES)?,
                    generation,
                    df,
                    dfhack,
                )
                .map_err(|_| custody())?;
                Self::Captured(Box::new(sample))
            }
            2 => {
                let fingerprint = Digest32::from_bytes(r.array()?);
                let receipt =
                    DigEffect::decode(r.blob(MAX_EFFECT_BYTES)?, plan).map_err(|_| custody())?;
                if fingerprint == Digest32::ZERO || receipt.phase() != DigPhase::Designated {
                    return Err(custody());
                }
                Self::Published {
                    fingerprint,
                    receipt,
                }
            }
            3 => Self::Interrupted {
                tick: GameTick(u64::from_be_bytes(r.array()?)),
                fenced: r.boolean()?,
            },
            _ => return Err(custody()),
        };
        if !r.0.is_empty() {
            return Err(custody());
        }
        Ok(event)
    }
}

/// Comparison only. No bytes can reconstruct an EvidencePolicy or confer
/// historical proof. The projection rejects every domain outside its exact
/// original terrain before this complete narrow fingerprint is made.
pub(super) fn policy_fingerprint(
    projection: &DigGoalProjection,
    policy: &EvidencePolicy,
) -> Result<Digest32> {
    projection.evidence(policy.clone())?;
    let mut out = b"dfmcp-dig-goal-policy/1\0".to_vec();
    out.extend_from_slice(projection.review_seal().as_bytes());
    out.extend_from_slice(projection.source_digest().as_bytes());
    let anchor = projection.anchor();
    out.extend_from_slice(&anchor.fortress_id.get().to_be_bytes());
    out.extend_from_slice(&anchor.cursor.epoch.to_be_bytes());
    out.extend_from_slice(&anchor.cursor.sequence.to_be_bytes());
    out.extend_from_slice(&anchor.tick.get().to_be_bytes());
    out.extend_from_slice(anchor.state_hash.as_bytes());
    out.push(u8::from(policy.paused));
    out.extend_from_slice(&(policy.terrain_regions.len() as u32).to_be_bytes());
    for area in &policy.terrain_regions {
        for point in [area.min, area.max] {
            for v in [point.x, point.y, point.z] {
                out.extend_from_slice(&v.to_be_bytes());
            }
        }
    }
    Ok(Digest32::of_bytes(&out))
}

struct Reader<'a>(&'a [u8]);
impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let out = self.0.get(..n).ok_or_else(custody)?;
        self.0 = &self.0[n..];
        Ok(out)
    }
    fn array<const N: usize>(&mut self) -> Result<[u8; N]> {
        let mut out = [0; N];
        out.copy_from_slice(self.take(N)?);
        Ok(out)
    }
    fn boolean(&mut self) -> Result<bool> {
        match self.array::<1>()?[0] {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(custody()),
        }
    }
    fn blob(&mut self, maximum: usize) -> Result<&'a [u8]> {
        let length = u32::from_be_bytes(self.array()?) as usize;
        if length > maximum {
            return Err(custody());
        }
        self.take(length)
    }
}

#[derive(Clone, Copy)]
struct Frontier {
    cursor: ObservationCursor,
    tick: GameTick,
    phase: u8,
    source_fenced: bool,
}
impl Frontier {
    fn apply(&mut self, event: &Event) -> Result<()> {
        match event {
            Event::Started { cursor, tick, .. } => {
                if self.source_fenced
                    || cursor.epoch != self.cursor.epoch
                    || cursor.sequence <= self.cursor.sequence
                    || *tick < self.tick
                {
                    return Err(custody());
                }
                self.cursor = *cursor;
                self.tick = *tick;
                self.phase = 1;
            }
            Event::Captured(sample) => {
                if self.phase != 1 {
                    return Err(custody());
                }
                self.phase = 2;
                // Complete raw observations retain a high clock even when
                // later source identity or policy validation refuses proof.
                sample.validate().map_err(|_| custody())?;
                self.tick = self.tick.max(sample.spatial().terrain().tick());
            }
            Event::Published { .. } => {
                if self.phase != 2 || self.source_fenced {
                    return Err(custody());
                }
                self.phase = 0;
            }
            Event::Interrupted { tick, fenced } => {
                if *tick < self.tick {
                    return Err(custody());
                }
                self.tick = *tick;
                self.source_fenced |= *fenced;
                self.phase = 0;
            }
        }
        Ok(())
    }
}

/// Operator-owned append-only history. A file belongs to one exact original
/// review and native journal. Its raw captures are retained evidence candidates,
/// not grants, current observations, or restored mutation authority.
pub struct GoalHistoryStore<S> {
    storage: S,
    raw: Vec<u8>,
    events: Vec<Event>,
    head: Digest32,
    native_journal: Digest32,
    seal: Digest32,
    native_plan: DigPlan,
    session: SessionId,
    fortress: dfmcp_core::FortressId,
    frontier: Frontier,
    read_only: bool,
    fenced: bool,
}
pub type PrivateGoalHistoryStore = GoalHistoryStore<PrivateBuildFile>;

/// The path is operator configuration, never an MCP argument. Uses the existing
/// descriptor-pinned, exclusively locked 0700-parent/0600-file custody shell.
pub fn open_private_goal_history_store(
    path: &Path,
    native_journal: Digest32,
    review: &SemanticDigReview,
    initialize: bool,
    read_only: bool,
    context: &OperationContext,
) -> Result<PrivateGoalHistoryStore> {
    validate_binding(native_journal, review, context)?;
    if initialize && read_only {
        return Err(error(
            ErrorCode::CapabilityDenied,
            "read-only goal history cannot initialize",
        ));
    }
    if initialize
        && context.budget.max_bytes <= super::super::LOCAL_BYTES + (2 * HEADER_BYTES) as u64 + 8192
    {
        return Err(exhausted());
    }
    let mode = if read_only {
        BuildMode::Offline
    } else if initialize {
        BuildMode::Control
    } else {
        BuildMode::Recover
    };
    let (storage, created, current) =
        open_private_storage(path, context, mode, initialize, MAX_GOAL_HISTORY_BYTES)?;
    if created != initialize {
        return Err(custody());
    }
    GoalHistoryStore::open(
        storage,
        native_journal,
        review,
        initialize,
        read_only,
        &current,
    )
}

fn validate_binding(
    native: Digest32,
    review: &SemanticDigReview,
    c: &OperationContext,
) -> Result<()> {
    c.budget.validate()?;
    let mut current = c.clone();
    current.anchor.tick = current.anchor.tick.max(review.original.anchor.tick);
    current.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    let object = super::super::original(&review.original, &Call::new(&current)?)?;
    if native == Digest32::ZERO
        || review.seal != review.association.seal(native)
        || review.association.semantic != review.original.digest
        || review.association.original_object != object
        || review.association.anchor != review.original.anchor
        || review.association.native != review.native.digest()
        || review.association.witness != review.native.before().witness()
        || review.association.key != review.native.key()
        || review.association.key != review.original.steps[0].idempotency_key
        || review.association.step != dfmcp_core::StepId::ZERO
        || review.association.source == Digest32::ZERO
        || review.association.source_identity == Digest32::ZERO
        || c.anchor.fortress_id != review.original.anchor.fortress_id
        || c.anchor.cursor.epoch != review.original.anchor.cursor.epoch
    {
        return Err(custody());
    }
    Ok(())
}

impl<S: EffectJournalStorage> GoalHistoryStore<S> {
    pub fn open(
        mut storage: S,
        native: Digest32,
        review: &SemanticDigReview,
        initialize: bool,
        read_only: bool,
        context: &OperationContext,
    ) -> Result<Self> {
        validate_binding(native, review, context)?;
        if initialize && read_only {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "read-only goal history cannot initialize",
            ));
        }
        let mut current = context.clone();
        current.anchor.tick = current.anchor.tick.max(review.original.anchor.tick);
        let mut call = Call::new(&current)?;
        storage.validate_identity().map_err(io_error)?;
        let len = storage.seek(SeekFrom::End(0)).map_err(io_error)?;
        if len > MAX_GOAL_HISTORY_BYTES as u64 {
            return Err(exhausted());
        }
        call.reserve(4 * len + (2 * HEADER_BYTES) as u64 + 4096)?;
        let mut raw = Vec::new();
        raw.try_reserve_exact(len as usize)
            .map_err(|_| exhausted())?;
        raw.resize(len as usize, 0);
        storage.seek(SeekFrom::Start(0)).map_err(io_error)?;
        storage.read_exact(&mut raw).map_err(io_error)?;
        if initialize {
            if !raw.is_empty() {
                return Err(custody());
            }
            raw.extend_from_slice(HEADER);
            raw.extend_from_slice(native.as_bytes());
            raw.extend_from_slice(review.seal.as_bytes());
            raw.extend_from_slice(&context.anchor.fortress_id.get().to_be_bytes());
            let head = hash(b"dfmcp-dig-goal-history/1", &raw);
            raw.extend_from_slice(head.as_bytes());
            storage.validate_identity().map_err(io_error)?;
            call.current()?;
            if storage.seek(SeekFrom::End(0)).map_err(io_error)? != 0 {
                return Err(custody());
            }
            storage
                .write_all(&raw)
                .and_then(|_| storage.flush())
                .and_then(|_| storage.sync())
                .map_err(io_error)?;
        }
        let mut reader = Reader(&raw);
        if reader.take(8)? != HEADER
            || reader.array::<32>()? != *native.as_bytes()
            || reader.array::<32>()? != *review.seal.as_bytes()
            || u64::from_be_bytes(reader.array()?) != context.anchor.fortress_id.get()
        {
            return Err(custody());
        }
        let mut head = Digest32::from_bytes(reader.array()?);
        if head != hash(b"dfmcp-dig-goal-history/1", &raw[..HEADER_BYTES - 32]) {
            return Err(custody());
        }
        let mut frontier = Frontier {
            cursor: review.original.anchor.cursor,
            tick: review.original.anchor.tick,
            source_fenced: false,
            phase: 0,
        };
        let mut events = Vec::new();
        while !reader.0.is_empty() {
            call.current()?;
            if events.len() >= MAX_GOAL_HISTORY_EVENTS
                || events.len() >= context.budget.max_entities as usize
            {
                return Err(exhausted());
            }
            let start = raw.len() - reader.0.len();
            if reader.take(8)? != FRAME {
                return Err(custody());
            }
            let count = u32::from_be_bytes(reader.array()?) as usize;
            if count > MAX_EVENT_BYTES
                || u64::from_be_bytes(reader.array()?) != events.len() as u64 + 1
                || Digest32::from_bytes(reader.array()?) != head
            {
                return Err(custody());
            }
            let event = Event::decode(reader.take(count)?, &review.native)?;
            frontier.apply(&event)?;
            let end = raw.len() - reader.0.len();
            head = Digest32::from_bytes(reader.array()?);
            if head != hash(b"dfmcp-dig-goal-frame/1", &raw[start..end]) || reader.take(8)? != END {
                return Err(custody());
            }
            events.push(event);
        }
        call.context.anchor.tick = call.context.anchor.tick.max(frontier.tick);
        call.current()?;
        if !initialize && !read_only {
            call.current()?;
            storage.validate_identity().map_err(io_error)?;
            storage.sync().map_err(io_error)?;
        }
        Self::verify_bytes(&mut storage, &raw, &call)?;
        Ok(Self {
            storage,
            raw,
            events,
            head,
            native_journal: native,
            seal: review.seal,
            native_plan: review.native.clone(),
            session: context.session_id,
            fortress: context.anchor.fortress_id,
            frontier,
            read_only,
            fenced: false,
        })
    }

    pub fn head(&self) -> Digest32 {
        self.head
    }
    pub fn native_journal_id(&self) -> Digest32 {
        self.native_journal
    }
    pub fn byte_len(&self) -> usize {
        self.raw.len()
    }
    pub fn event_count(&self) -> usize {
        self.events.len()
    }
    pub fn is_read_only(&self) -> bool {
        self.read_only
    }
    pub fn is_fenced(&self) -> bool {
        self.fenced
    }
    pub fn review_seal(&self) -> Digest32 {
        self.seal
    }
    pub fn high_tick(&self) -> GameTick {
        self.frontier.tick
    }
    pub fn high_cursor(&self) -> ObservationCursor {
        self.frontier.cursor
    }
    /// A retained source reset blocks fresh observations without erasing readable history.
    pub fn source_fenced(&self) -> bool {
        self.frontier.source_fenced
    }
    pub(super) fn events(&self) -> &[Event] {
        &self.events
    }
    pub(super) fn binding(&self, native: Digest32, review: &SemanticDigReview) -> Result<()> {
        if self.fenced
            || self.native_journal != native
            || self.seal != review.seal
            || self.native_plan != review.native
        {
            return Err(custody());
        }
        Ok(())
    }
    pub(super) fn preflight_poll(&self, cursor: ObservationCursor) -> Result<()> {
        if self.fenced || self.frontier.source_fenced {
            return Err(custody());
        }
        if self.read_only {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "read-only goal history cannot publish observations",
            ));
        }
        if cursor.epoch != self.frontier.cursor.epoch
            || cursor.sequence <= self.frontier.cursor.sequence
        {
            return Err(error(
                ErrorCode::StaleAnchor,
                "durable goal observation requires an unused later canonical cursor",
            ));
        }
        if self.events.len() + MAX_POLL_EVENTS > MAX_GOAL_HISTORY_EVENTS
            || self.raw.len() + MAX_POLL_BYTES > MAX_GOAL_HISTORY_BYTES
        {
            return Err(exhausted());
        }
        Ok(())
    }

    fn access(&self, context: &OperationContext, write: bool) -> Result<OperationContext> {
        context.budget.validate()?;
        if self.fenced {
            return Err(custody());
        }
        if context.session_id != self.session
            || context.anchor.fortress_id != self.fortress
            || context.anchor.cursor.epoch != self.frontier.cursor.epoch
            || (write && self.read_only)
        {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "goal history session, epoch or access mode differs",
            ));
        }
        let mut current = context.clone();
        current.anchor.tick = current.anchor.tick.max(self.frontier.tick);
        current.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
        Ok(current)
    }
    fn verify_bytes(storage: &mut S, bytes: &[u8], call: &Call) -> Result<()> {
        call.current()?;
        storage.validate_identity().map_err(io_error)?;
        if storage.seek(SeekFrom::End(0)).map_err(io_error)? != bytes.len() as u64 {
            return Err(custody());
        }
        storage.seek(SeekFrom::Start(0)).map_err(io_error)?;
        let mut buffer = [0u8; 4096];
        for chunk in bytes.chunks(4096) {
            call.current()?;
            storage
                .read_exact(&mut buffer[..chunk.len()])
                .map_err(io_error)?;
            if chunk != &buffer[..chunk.len()] {
                return Err(custody());
            }
        }
        storage.validate_identity().map_err(io_error)?;
        if storage.seek(SeekFrom::End(0)).map_err(io_error)? != bytes.len() as u64 {
            return Err(custody());
        }
        call.current()?;
        Ok(())
    }
    pub fn verify(&mut self, context: &OperationContext) -> Result<()> {
        let current = self.access(context, false)?;
        let mut call = Call::new(&current)?;
        call.reserve(self.raw.len() as u64 + 4096)?;
        let outcome = Self::verify_bytes(&mut self.storage, &self.raw, &call);
        if outcome
            .as_ref()
            .is_err_and(|e| e.code == ErrorCode::CorruptLedger)
        {
            self.fenced = true;
        }
        outcome
    }
    pub(super) fn append(&mut self, event: Event, context: &OperationContext) -> Result<()> {
        let current = self.access(context, true)?;
        let mut call = Call::new(&current)?;
        // Charge raw decoding, bounded encoding and candidate copies before
        // constructing them or deriving an authority floor from the capture.
        let bound = self.raw.len() + OVERHEAD + event.maximum_bytes();
        call.reserve((6 * bound + 4096) as u64)?;
        let mut frontier = self.frontier;
        frontier.apply(&event)?;
        call.context.anchor.tick = call.context.anchor.tick.max(frontier.tick);
        call.current()?;
        let body = event.encode()?;
        // Closed native receipts are meaningful only for this exact original plan.
        Event::decode(&body, &self.native_plan)?;
        let next_len = self.raw.len() + OVERHEAD + body.len();
        if next_len > MAX_GOAL_HISTORY_BYTES
            || self.events.len() >= MAX_GOAL_HISTORY_EVENTS
            || self.events.len() >= context.budget.max_entities as usize
        {
            return Err(exhausted());
        }
        let verification = Self::verify_bytes(&mut self.storage, &self.raw, &call);
        if verification.is_err() {
            self.fenced = true;
        }
        verification?;
        let mut frame = Vec::new();
        frame
            .try_reserve_exact(body.len() + OVERHEAD)
            .map_err(|_| exhausted())?;
        frame.extend_from_slice(FRAME);
        frame.extend_from_slice(&(body.len() as u32).to_be_bytes());
        frame.extend_from_slice(&(self.events.len() as u64 + 1).to_be_bytes());
        frame.extend_from_slice(self.head.as_bytes());
        frame.extend_from_slice(&body);
        let head = hash(b"dfmcp-dig-goal-frame/1", &frame);
        frame.extend_from_slice(head.as_bytes());
        frame.extend_from_slice(END);
        let mut raw = Vec::new();
        raw.try_reserve_exact(self.raw.len() + frame.len())
            .map_err(|_| exhausted())?;
        raw.extend_from_slice(&self.raw);
        raw.extend_from_slice(&frame);
        self.events.try_reserve(1).map_err(|_| exhausted())?;
        self.access(context, true)?;
        let result = (|| {
            call.current()?;
            self.storage.validate_identity().map_err(io_error)?;
            if self.storage.seek(SeekFrom::End(0)).map_err(io_error)? != self.raw.len() as u64 {
                return Err(custody());
            }
            self.storage
                .write_all(&frame)
                .and_then(|_| self.storage.flush())
                .and_then(|_| self.storage.sync())
                .map_err(io_error)?;
            Self::verify_bytes(&mut self.storage, &raw, &call)
        })();
        if result.is_err() {
            self.fenced = true;
        }
        result?;
        self.raw = raw;
        self.events.push(event);
        self.head = head;
        self.frontier = frontier;
        Ok(())
    }
}
