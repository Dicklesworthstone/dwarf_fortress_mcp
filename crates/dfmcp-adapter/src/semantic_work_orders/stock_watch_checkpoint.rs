//! Canonical historical custody for original no-work inventory watches.
//!
//! These bytes contain no current sample or unfinished stability streak. The
//! embedding owner must authenticate them against its private journal before
//! recovery; a digest or a caller-provided checkpoint is not source authority.
use super::{StockTargetAssessment, error};
use dfmcp_core::{
    Digest32, ErrorCode, FortressId, GameTick, ObservationCursor, OperationContext, Result,
    StateAnchor,
};

const SCALAR_DOMAIN: &[u8] = b"dfmcp-no-work-stock-watch-checkpoint/1\0";
const PORTFOLIO_DOMAIN: &[u8] = b"dfmcp-no-work-stock-targets-watch-checkpoint/1\0";
pub(super) const MAX_BYTES: usize = 2048;

/// Bounded historical terms for checking a private journal envelope. These
/// primitive values are neither qualified current counts nor an active watch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StockWatchCheckpointTerms {
    pub original_anchor: StateAnchor,
    pub high_tick: GameTick,
    pub goal_witness: Digest32,
    /// Canonically ordered recipe, minimum stack units and finite order cap.
    pub targets: Vec<(crate::work_orders::WorkOrderRecipe, u64, u32)>,
    pub deadline_tick: GameTick,
    pub poll_interval_ticks: u64,
    pub stable_samples: u32,
    pub achieved_at: Option<GameTick>,
    pub expired: bool,
    pub fenced: bool,
}

pub(super) fn corrupt() -> dfmcp_core::DfmcpError {
    error(
        ErrorCode::CorruptLedger,
        "invalid or regressed original stock-watch checkpoint",
    )
}

pub(super) struct Reader<'a> {
    remaining: &'a [u8],
}

impl<'a> Reader<'a> {
    pub(super) fn new(bytes: &'a [u8]) -> Self {
        Self { remaining: bytes }
    }

    pub(super) fn take(&mut self, length: usize) -> Result<&'a [u8]> {
        let value = self.remaining.get(..length).ok_or_else(corrupt)?;
        self.remaining = &self.remaining[length..];
        Ok(value)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N]> {
        self.take(N)?.try_into().map_err(|_| corrupt())
    }

    pub(super) fn byte(&mut self) -> Result<u8> {
        Ok(self.array::<1>()?[0])
    }

    fn boolean(&mut self) -> Result<bool> {
        match self.byte()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(corrupt()),
        }
    }

    pub(super) fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_be_bytes(self.array()?))
    }

    pub(super) fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_be_bytes(self.array()?))
    }

    pub(super) fn digest(&mut self) -> Result<Digest32> {
        Ok(Digest32::from_bytes(self.array()?))
    }

    pub(super) fn anchor(&mut self) -> Result<StateAnchor> {
        Ok(StateAnchor {
            fortress_id: FortressId::new(self.u64()?),
            cursor: ObservationCursor {
                epoch: self.u64()?,
                sequence: self.u64()?,
            },
            tick: GameTick(self.u64()?),
            state_hash: self.digest()?,
        })
    }

    fn optional_tick(&mut self) -> Result<Option<GameTick>> {
        if self.boolean()? {
            Ok(Some(GameTick(self.u64()?)))
        } else {
            Ok(None)
        }
    }

    pub(super) fn finish(self) -> Result<()> {
        if self.remaining.is_empty() {
            Ok(())
        } else {
            Err(corrupt())
        }
    }
}

fn put_anchor(bytes: &mut Vec<u8>, anchor: StateAnchor) {
    bytes.extend_from_slice(&anchor.fortress_id.get().to_be_bytes());
    bytes.extend_from_slice(&anchor.cursor.epoch.to_be_bytes());
    bytes.extend_from_slice(&anchor.cursor.sequence.to_be_bytes());
    bytes.extend_from_slice(&anchor.tick.get().to_be_bytes());
    bytes.extend_from_slice(anchor.state_hash.as_bytes());
}

#[derive(Clone, Debug)]
pub(super) struct Checkpoint {
    pub(super) originals: Vec<StockTargetAssessment>,
    pub(super) deadline: GameTick,
    pub(super) cadence: u64,
    pub(super) stable: u32,
    pub(super) witness: Digest32,
    pub(super) high_tick: GameTick,
    pub(super) high_counters: [u64; 3],
    pub(super) high_anchor: StateAnchor,
    pub(super) last_counted: Option<GameTick>,
    pub(super) achieved: Option<GameTick>,
    pub(super) expired: bool,
    pub(super) fenced: bool,
}

impl Checkpoint {
    pub(super) fn terms(&self) -> Result<StockWatchCheckpointTerms> {
        Ok(StockWatchCheckpointTerms {
            original_anchor: self
                .originals
                .first()
                .ok_or_else(corrupt)?
                .original_anchor(),
            high_tick: self.high_tick,
            goal_witness: self.witness,
            targets: self
                .originals
                .iter()
                .map(|original| {
                    (
                        original.recipe(),
                        original.target_stack_units(),
                        original.max_order_amount(),
                    )
                })
                .collect(),
            deadline_tick: self.deadline,
            poll_interval_ticks: self.cadence,
            stable_samples: self.stable,
            achieved_at: self.achieved,
            expired: self.expired,
            fenced: self.fenced,
        })
    }

    pub(super) fn encode(&self, portfolio: bool) -> Vec<u8> {
        let mut bytes = if portfolio {
            PORTFOLIO_DOMAIN
        } else {
            SCALAR_DOMAIN
        }
        .to_vec();
        bytes.push(self.originals.len() as u8);
        for original in &self.originals {
            let encoded = original.canonical_bytes();
            bytes.extend_from_slice(&(encoded.len() as u32).to_be_bytes());
            bytes.extend_from_slice(encoded);
        }
        bytes.extend_from_slice(&self.deadline.get().to_be_bytes());
        bytes.extend_from_slice(&self.cadence.to_be_bytes());
        bytes.extend_from_slice(&self.stable.to_be_bytes());
        bytes.extend_from_slice(self.witness.as_bytes());
        bytes.extend_from_slice(&self.high_tick.get().to_be_bytes());
        for counter in self.high_counters {
            bytes.extend_from_slice(&counter.to_be_bytes());
        }
        put_anchor(&mut bytes, self.high_anchor);
        for tick in [self.last_counted, self.achieved] {
            bytes.push(u8::from(tick.is_some()));
            if let Some(tick) = tick {
                bytes.extend_from_slice(&tick.get().to_be_bytes());
            }
        }
        bytes.push(u8::from(self.expired));
        bytes.push(u8::from(self.fenced));
        bytes
    }

    pub(super) fn decode(bytes: &[u8], portfolio: bool) -> Result<Self> {
        if bytes.len() > MAX_BYTES {
            return Err(corrupt());
        }
        let mut reader = Reader::new(bytes);
        let domain = if portfolio {
            PORTFOLIO_DOMAIN
        } else {
            SCALAR_DOMAIN
        };
        if reader.take(domain.len())? != domain {
            return Err(corrupt());
        }
        let count = usize::from(reader.byte()?);
        if count == 0 || count > 4 || (!portfolio && count != 1) {
            return Err(corrupt());
        }
        let mut originals = Vec::with_capacity(count);
        for _ in 0..count {
            let length = reader.u32()? as usize;
            if length > 512 {
                return Err(corrupt());
            }
            originals.push(StockTargetAssessment::recover_satisfied(
                reader.take(length)?,
            )?);
        }
        let checkpoint = Self {
            originals,
            deadline: GameTick(reader.u64()?),
            cadence: reader.u64()?,
            stable: reader.u32()?,
            witness: reader.digest()?,
            high_tick: GameTick(reader.u64()?),
            high_counters: [reader.u64()?, reader.u64()?, reader.u64()?],
            high_anchor: reader.anchor()?,
            last_counted: reader.optional_tick()?,
            achieved: reader.optional_tick()?,
            expired: reader.boolean()?,
            fenced: reader.boolean()?,
        };
        reader.finish()?;
        checkpoint.validate(portfolio)?;
        if checkpoint.encode(portfolio) != bytes {
            return Err(corrupt());
        }
        Ok(checkpoint)
    }

    fn validate(&self, portfolio: bool) -> Result<()> {
        let first = self.originals.first().ok_or_else(corrupt)?;
        let original = first.original_anchor();
        let span = self
            .deadline
            .get()
            .checked_sub(original.tick.get())
            .ok_or_else(corrupt)?;
        let maximum = dfmcp_intent::effects::MAX_DEFAULT_OBLIGATION_TICKS;
        if span == 0
            || span > maximum
            || !(1..=maximum).contains(&self.cadence)
            || !(1..=64).contains(&self.stable)
            || self
                .cadence
                .checked_mul(u64::from(self.stable))
                .is_none_or(|v| v > span)
            || self
                .originals
                .windows(2)
                .any(|pair| pair[0].recipe().as_str() >= pair[1].recipe().as_str())
            || self.originals.iter().any(|item| {
                item.original_anchor() != original
                    || item.source_digest() != first.source_digest()
                    || item.source_generation() != first.source_generation()
                    || item.source_counters() != first.source_counters()
                    || item.item_rows() != first.item_rows()
            })
            || self.high_tick < original.tick
            || self.high_tick.get() > crate::work_orders::MAX_NATIVE_TICK
            || self.high_anchor.fortress_id != original.fortress_id
            || self.high_anchor.cursor.epoch != original.cursor.epoch
            || self.high_anchor.cursor.sequence < original.cursor.sequence
            || self.high_anchor.tick < original.tick
            || self.high_anchor.tick > self.high_tick
            || (self.high_anchor.cursor == original.cursor && self.high_anchor != original)
            || self
                .high_counters
                .iter()
                .zip(first.source_counters())
                .any(|(high, start)| *high < start || *high > u64::from(u32::MAX))
            || self.last_counted.is_some_and(|tick| {
                tick.get() < original.tick.get().saturating_add(self.cadence)
                    || tick > self.high_anchor.tick
                    || tick > self.deadline
            })
            || self.achieved.is_some_and(|tick| {
                self.last_counted != Some(tick)
                    || tick > self.deadline
                    || tick.get().saturating_sub(original.tick.get())
                        < self.cadence.saturating_mul(u64::from(self.stable))
            })
            || (self.expired && (self.achieved.is_some() || self.high_tick < self.deadline))
        {
            return Err(corrupt());
        }
        let mut identity = if portfolio {
            b"dfmcp-no-work-stock-targets-watch/1\0".to_vec()
        } else {
            b"dfmcp-no-work-stock-watch/1\0".to_vec()
        };
        if portfolio {
            identity.extend_from_slice(&(self.originals.len() as u32).to_be_bytes());
        }
        for original in &self.originals {
            identity.extend_from_slice(original.witness().as_bytes());
        }
        identity.extend_from_slice(&self.deadline.get().to_be_bytes());
        identity.extend_from_slice(&self.cadence.to_be_bytes());
        identity.extend_from_slice(&self.stable.to_be_bytes());
        if Digest32::of_bytes(&identity) != self.witness {
            return Err(corrupt());
        }
        Ok(())
    }

    /// The vector's private guard retains only shared source rejection floors.
    /// Its individual achievement is never a portfolio achievement.
    pub(super) fn scalar_guard(&self) -> Result<Self> {
        let original = self.originals.first().ok_or_else(corrupt)?.clone();
        let mut identity = b"dfmcp-no-work-stock-watch/1\0".to_vec();
        identity.extend_from_slice(original.witness().as_bytes());
        identity.extend_from_slice(&self.deadline.get().to_be_bytes());
        identity.extend_from_slice(&self.cadence.to_be_bytes());
        identity.extend_from_slice(&self.stable.to_be_bytes());
        Ok(Self {
            originals: vec![original],
            witness: Digest32::of_bytes(&identity),
            last_counted: None,
            achieved: None,
            expired: false,
            ..self.clone()
        })
    }

    /// Compare authenticated sequential records; no record grants authority.
    pub(super) fn follows(&self, previous: &Self) -> Result<()> {
        if self.originals != previous.originals
            || self.deadline != previous.deadline
            || self.cadence != previous.cadence
            || self.stable != previous.stable
            || self.witness != previous.witness
            || self.high_tick < previous.high_tick
            || self
                .high_counters
                .iter()
                .zip(previous.high_counters)
                .any(|(next, high)| *next < high)
            || self.high_anchor.cursor.sequence < previous.high_anchor.cursor.sequence
            || self.high_anchor.tick < previous.high_anchor.tick
            || (self.high_anchor.cursor == previous.high_anchor.cursor
                && self.high_anchor != previous.high_anchor)
            || previous
                .last_counted
                .is_some_and(|tick| self.last_counted.is_none_or(|next| next < tick))
            || previous
                .achieved
                .is_some_and(|tick| self.achieved != Some(tick))
            || (previous.expired && !self.expired)
            || (previous.fenced && !self.fenced)
        {
            return Err(corrupt());
        }
        Ok(())
    }

    pub(super) fn authorize(&self, context: &OperationContext) -> Result<()> {
        let original = self
            .originals
            .first()
            .ok_or_else(corrupt)?
            .original_anchor();
        if context.anchor.fortress_id != original.fortress_id
            || context.anchor.cursor.epoch != original.cursor.epoch
        {
            return Err(error(
                ErrorCode::StaleAnchor,
                "stock-watch recovery belongs to another fortress lineage",
            ));
        }
        // Rejection floors may exceed the last published canonical anchor.
        // Check grant expiry at the consumed floor without creating evidence.
        let mut current = context.clone();
        current.anchor.tick = current.anchor.tick.max(self.high_tick);
        current.authorize(
            dfmcp_core::Capability::Query,
            dfmcp_core::RiskTier::ReadOnly,
            &[],
            None,
        )
    }
}
