//! Strict, bounded custody for one original workforce goal's observation history.

use std::io::{self, SeekFrom};
use std::path::Path;

use dfmcp_core::{
    Capability, Digest32, ErrorCode, GameTick, ObservationCursor, OperationContext, Result,
    RiskTier, SessionId,
};
use dfmcp_world::{EntityKind, EvidenceCoverage, EvidencePolicy, EvidenceSource};

use crate::bounded_run::hash;
use crate::build_placement::journal::BuildMode;
use crate::build_placement::journal::private_file::{PrivateBuildFile, open_private_storage};
use crate::control_effect_journal::EffectJournalStorage;
use crate::workforce_control::{MAX_CAPTURE, WorkforceCapture};

use super::super::projection::{
    ELIGIBILITY_SOURCE, HISTORICAL_ID_SOURCE, LABOR_SOURCE, UNIT_ID_SOURCE, WorkforceGoalProjection,
};
use super::super::{Call, SemanticWorkforceReview, custody, error, exhausted};

/// Retention is explicit; no silent pruning, tail repair, or rollover occurs.
pub const MAX_GOAL_HISTORY_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_GOAL_HISTORY_EVENTS: usize = 1024;
const HEADER: &[u8; 8] = b"DFMWGH01";
const FRAME: &[u8; 8] = b"DFMWGF01";
const END: &[u8; 8] = b"DFMWGE01";
const HEADER_BYTES: usize = 112;
const OVERHEAD: usize = 92;
pub(super) const MAX_EVENT_BYTES: usize = MAX_CAPTURE + 64;

fn io_error(_: io::Error) -> dfmcp_core::DfmcpError {
    error(
        ErrorCode::CorruptLedger,
        "workforce goal history custody or I/O failed; reopen without repair",
    )
}

#[derive(Clone, Debug)]
pub(super) enum Event {
    Started {
        cursor: ObservationCursor,
        tick: GameTick,
        native: u64,
        interrupted: bool,
    },
    Captured(WorkforceCapture),
    Published([u8; 4]),
    Interrupted {
        tick: GameTick,
        native: u64,
    },
}

impl Event {
    fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        match self {
            Self::Started {
                cursor,
                tick,
                native,
                interrupted,
            } => {
                out.push(0);
                out.extend_from_slice(&cursor.epoch.to_be_bytes());
                out.extend_from_slice(&cursor.sequence.to_be_bytes());
                out.extend_from_slice(&tick.get().to_be_bytes());
                out.extend_from_slice(&native.to_be_bytes());
                out.push(u8::from(*interrupted));
            }
            Self::Captured(capture) => {
                out.push(1);
                out.extend_from_slice(capture.canonical_bytes());
            }
            Self::Published(policy) => {
                out.push(2);
                out.extend_from_slice(policy);
            }
            Self::Interrupted { tick, native } => {
                out.push(3);
                out.extend_from_slice(&tick.get().to_be_bytes());
                out.extend_from_slice(&native.to_be_bytes());
            }
        }
        out
    }

    fn decode(bytes: &[u8]) -> Result<Self> {
        let mut r = Reader(bytes);
        let event = match r.array::<1>()?[0] {
            0 => Self::Started {
                cursor: ObservationCursor {
                    epoch: u64::from_be_bytes(r.array()?),
                    sequence: u64::from_be_bytes(r.array()?),
                },
                tick: GameTick(u64::from_be_bytes(r.array()?)),
                native: u64::from_be_bytes(r.array()?),
                interrupted: match r.array::<1>()?[0] {
                    0 => false,
                    1 => true,
                    _ => return Err(custody()),
                },
            },
            1 => {
                let value = WorkforceCapture::decode(r.0).map_err(|_| custody())?;
                r.0 = &[];
                Self::Captured(value)
            }
            2 => {
                let policy = r.array()?;
                if policy[0] > 1 || policy[1] > 2 || policy[2] > 1 || policy[3] > 15 {
                    return Err(custody());
                }
                Self::Published(policy)
            }
            3 => Self::Interrupted {
                tick: GameTick(u64::from_be_bytes(r.array()?)),
                native: u64::from_be_bytes(r.array()?),
            },
            _ => return Err(custody()),
        };
        if !r.0.is_empty() {
            return Err(custody());
        }
        Ok(event)
    }
}

/// A comparison fingerprint only. Decoding this never constructs EvidencePolicy
/// or PredicateEvidence. Recovery requires a newly and independently issued exact
/// policy; previously unavailable facts cannot gain retrospective proof authority.
pub(super) fn policy_fingerprint(
    p: &WorkforceGoalProjection,
    policy: &EvidencePolicy,
) -> Result<[u8; 4]> {
    p.evidence(policy.clone())?;
    let coverage = match policy.all_entities {
        EvidenceCoverage::Unknown => 0,
        EvidenceCoverage::Observed => 1,
        EvidenceCoverage::Complete => return Err(custody()),
    };
    let units = match policy.entity_kinds.get(&EntityKind::Unit) {
        None => 0,
        Some(EvidenceCoverage::Unknown) => 1,
        Some(EvidenceCoverage::Observed) => 2,
        Some(EvidenceCoverage::Complete) => return Err(custody()),
    };
    let mut mask = 0;
    for (index, field) in [
        UNIT_ID_SOURCE,
        HISTORICAL_ID_SOURCE,
        ELIGIBILITY_SOURCE,
        LABOR_SOURCE,
    ]
    .into_iter()
    .enumerate()
    {
        if policy.sources.contains(&EvidenceSource::Observed {
            field: field.to_owned(),
            source_digest: p.source_digest(),
        }) {
            mask |= 1 << index;
        }
    }
    Ok([coverage, units, u8::from(policy.paused), mask])
}

struct Reader<'a>(&'a [u8]);
impl Reader<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8]> {
        let out = self.0.get(..n).ok_or_else(custody)?;
        self.0 = &self.0[n..];
        Ok(out)
    }
    fn array<const N: usize>(&mut self) -> Result<[u8; N]> {
        let mut out = [0; N];
        out.copy_from_slice(self.take(N)?);
        Ok(out)
    }
}

#[derive(Clone, Copy)]
struct Frontier {
    cursor: ObservationCursor,
    tick: GameTick,
    native: u64,
    phase: u8,
}
impl Frontier {
    fn apply(&mut self, event: &Event) -> Result<()> {
        match event {
            Event::Started {
                cursor,
                tick,
                native,
                ..
            } => {
                if cursor.epoch != self.cursor.epoch
                    || cursor.sequence <= self.cursor.sequence
                    || *tick < self.tick
                    || *native < self.native
                {
                    return Err(custody());
                }
                self.cursor = *cursor;
                self.tick = *tick;
                self.native = *native;
                self.phase = 1;
            }
            Event::Captured(capture) => {
                if self.phase != 1
                    || capture.tick() < self.tick.get()
                    || capture.sequence() < self.native
                {
                    return Err(custody());
                }
                self.tick = GameTick(capture.tick());
                self.native = capture.sequence();
                self.phase = 2;
            }
            Event::Published(_) => {
                if self.phase != 2 {
                    return Err(custody());
                }
                self.phase = 0;
            }
            Event::Interrupted { tick, native } => {
                if *tick < self.tick || *native < self.native {
                    return Err(custody());
                }
                self.tick = *tick;
                self.native = *native;
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
    review: &SemanticWorkforceReview,
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
        && context.budget.max_bytes
            <= super::super::LOCAL_RESERVE + (2 * HEADER_BYTES) as u64 + 8192
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
    review: &SemanticWorkforceReview,
    c: &OperationContext,
) -> Result<()> {
    c.budget.validate()?;
    c.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    if native == Digest32::ZERO
        || review.seal != review.association.seal(native)
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
        review: &SemanticWorkforceReview,
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
        let mut call = Call::new(context)?;
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
            let head = hash(b"dfmcp-workforce-goal-history/1", &raw);
            raw.extend_from_slice(head.as_bytes());
            storage.validate_identity().map_err(io_error)?;
            call.context()?;
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
        if head != hash(b"dfmcp-workforce-goal-history/1", &raw[..HEADER_BYTES - 32]) {
            return Err(custody());
        }
        let mut frontier = Frontier {
            cursor: review.original.anchor.cursor,
            tick: review.original.anchor.tick,
            native: review.native.before().sequence(),
            phase: 0,
        };
        let mut events = Vec::new();
        while !reader.0.is_empty() {
            call.context()?;
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
            let event = Event::decode(reader.take(count)?)?;
            frontier.apply(&event)?;
            let end = raw.len() - reader.0.len();
            head = Digest32::from_bytes(reader.array()?);
            if head != hash(b"dfmcp-workforce-goal-frame/1", &raw[start..end])
                || reader.take(8)? != END
            {
                return Err(custody());
            }
            events.push(event);
        }
        if !initialize && !read_only {
            call.context()?;
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
            session: context.session_id,
            fortress: context.anchor.fortress_id,
            frontier,
            read_only,
            fenced: false,
        })
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
    pub fn high_native_sequence(&self) -> u64 {
        self.frontier.native
    }
    pub(super) fn events(&self) -> &[Event] {
        &self.events
    }
    pub(super) fn binding(&self, native: Digest32, review: &SemanticWorkforceReview) -> Result<()> {
        if self.fenced || self.native_journal != native || self.seal != review.seal {
            return Err(custody());
        }
        Ok(())
    }
    pub(super) fn preflight_poll(&self, cursor: ObservationCursor) -> Result<()> {
        if self.fenced {
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
        if self.events.len() + 4 > MAX_GOAL_HISTORY_EVENTS
            || self.raw.len() + MAX_EVENT_BYTES + 4 * OVERHEAD + 100 > MAX_GOAL_HISTORY_BYTES
        {
            return Err(exhausted());
        }
        Ok(())
    }

    fn access(&self, context: &OperationContext, write: bool) -> Result<()> {
        context.budget.validate()?;
        if self.fenced {
            return Err(custody());
        }
        if context.session_id != self.session
            || context.anchor.fortress_id != self.fortress
            || (write && self.read_only)
        {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "goal history session or access mode differs",
            ));
        }
        context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)
    }
    fn verify_bytes(storage: &mut S, bytes: &[u8], call: &Call) -> Result<()> {
        call.context()?;
        storage.validate_identity().map_err(io_error)?;
        if storage.seek(SeekFrom::End(0)).map_err(io_error)? != bytes.len() as u64 {
            return Err(custody());
        }
        storage.seek(SeekFrom::Start(0)).map_err(io_error)?;
        let mut buffer = [0u8; 4096];
        for chunk in bytes.chunks(4096) {
            call.context()?;
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
        call.context()?;
        Ok(())
    }
    pub fn verify(&mut self, context: &OperationContext) -> Result<()> {
        self.access(context, false)?;
        let mut call = Call::new(context)?;
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
        self.access(context, true)?;
        let mut frontier = self.frontier;
        frontier.apply(&event)?;
        let mut call = Call::new(context)?;
        let expected = match &event {
            Event::Captured(c) => c.canonical_bytes().len() + 1,
            _ => 34,
        };
        let next_len = self.raw.len() + OVERHEAD + expected;
        if next_len > MAX_GOAL_HISTORY_BYTES
            || self.events.len() >= MAX_GOAL_HISTORY_EVENTS
            || self.events.len() >= context.budget.max_entities as usize
        {
            return Err(exhausted());
        }
        call.reserve((6 * next_len + 4096) as u64)?;
        let verification = Self::verify_bytes(&mut self.storage, &self.raw, &call);
        if verification.is_err() {
            self.fenced = true;
        }
        verification?;
        let body = event.encode();
        let mut frame = Vec::new();
        frame
            .try_reserve_exact(body.len() + OVERHEAD)
            .map_err(|_| exhausted())?;
        frame.extend_from_slice(FRAME);
        frame.extend_from_slice(&(body.len() as u32).to_be_bytes());
        frame.extend_from_slice(&(self.events.len() as u64 + 1).to_be_bytes());
        frame.extend_from_slice(self.head.as_bytes());
        frame.extend_from_slice(&body);
        let head = hash(b"dfmcp-workforce-goal-frame/1", &frame);
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
            call.context()?;
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
