//! Read-only verification of the acknowledged original used by a separate clock.
//! The returned snapshot does not reserve journal space or grant any permission.
use super::*;

pub const CLOCK_ORIGIN_BYTES: u64 = 64 * 1024 * 1024;

/// Exact retained original and source floors reconstructed from private bytes.
/// Mutable journal view fields are never used to define this descriptor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreeClockOrigin {
    binding: Binding,
    pin: WorkshopPin,
    original: Original,
    record: Record,
    high_tick: u64,
    high_generation: u64,
    high_sequence: u64,
    high_item: u32,
    samples: u32,
    pending_sample: bool,
    continuity_lost: bool,
    achieved_tick: Option<u64>,
    retained_bytes: usize,
}

impl TreeClockOrigin {
    pub fn binding(&self) -> &Binding {
        &self.binding
    }
    pub const fn pin(&self) -> WorkshopPin {
        self.pin
    }
    pub fn original(&self) -> &Original {
        &self.original
    }
    pub fn record(&self) -> &Record {
        &self.record
    }
    pub const fn high_tick(&self) -> u64 {
        self.high_tick
    }
    pub const fn high_generation(&self) -> u64 {
        self.high_generation
    }
    pub const fn high_sequence(&self) -> u64 {
        self.high_sequence
    }
    pub const fn high_item(&self) -> u32 {
        self.high_item
    }
    pub const fn samples(&self) -> u32 {
        self.samples
    }
    pub const fn pending_sample(&self) -> bool {
        self.pending_sample
    }
    pub const fn continuity_lost(&self) -> bool {
        self.continuity_lost
    }
    pub const fn achieved_tick(&self) -> Option<u64> {
        self.achieved_tick
    }

    /// Admit all anticipated guard and post-run samples against this verified
    /// snapshot. The exclusive owner must recheck before each effect boundary;
    /// this method performs no durable reservation and never enlarges capacity.
    pub fn require_capacity(&self, count: u32) -> Result<()> {
        let frames = count.checked_mul(2).ok_or_else(clock_capacity)?;
        let bytes = usize::try_from(frames)
            .ok()
            .and_then(|n| n.checked_mul(MAX_BODY + 84))
            .ok_or_else(clock_capacity)?;
        if count == 0
            || count > MAX_SAMPLES
            || self.pending_sample
            || self
                .samples
                .checked_add(count)
                .is_none_or(|n| n > MAX_SAMPLES)
            || self
                .pin
                .frames
                .checked_add(frames)
                .is_none_or(|n| n > MAX_FRAMES)
            || self
                .retained_bytes
                .checked_add(bytes)
                .is_none_or(|n| n > MAX_BYTES)
        {
            return Err(clock_capacity());
        }
        Ok(())
    }
}

fn clock_capacity() -> dfmcp_core::DfmcpError {
    error(
        ErrorCode::BudgetExceeded,
        "original Tree sample, frame or byte capacity cannot cover the complete clock interval",
    )
}

impl<S: EffectJournalStorage> Journal<S> {
    /// Verify the held Tree chain and independent ACK without native I/O or a
    /// custody write. Control/recovery owners keep their existing mode; no
    /// designation permit, clock grant or fresh completion proof is restored.
    pub fn clock_origin<A: EffectJournalStorage>(
        &mut self,
        acknowledgement: &mut WorkshopAcknowledgements<A>,
        context: &OperationContext,
    ) -> Result<TreeClockOrigin> {
        let result: Result<TreeClockOrigin> = (|| {
            check(!self.fenced && self.owner == context.session_id)?;
            let mut work = Work::new(context)?;
            if context.budget.max_bytes < CLOCK_ORIGIN_BYTES {
                return Err(clock_capacity());
            }
            Self::authorize(
                &work.current()?,
                &self.binding,
                false,
                false,
                context.anchor.tick.get(),
            )?;
            let origin = self.clock_projection(&mut work)?;
            work.context.anchor.tick = GameTick(origin.high_tick.max(context.anchor.tick.get()));
            Self::authorize(
                &work.current()?,
                &self.binding,
                false,
                false,
                origin.high_tick,
            )?;
            let scope = self.binding.read_scope()?;
            acknowledgement.verify_tree(
                &work
                    .reserve(crate::workshop_construction::acknowledgement::WORK_BYTES as usize)?,
                scope,
            )?;
            check(acknowledgement.latest() == Some(origin.pin))?;
            let raw = read(&mut self.storage, &mut work)?;
            check(raw == self.raw)?;
            acknowledgement.verify_tree(
                &work
                    .reserve(crate::workshop_construction::acknowledgement::WORK_BYTES as usize)?,
                scope,
            )?;
            check(acknowledgement.latest() == Some(origin.pin))?;
            Self::authorize(
                &work.current()?,
                &self.binding,
                false,
                false,
                origin.high_tick,
            )?;
            Ok(origin)
        })();
        if result
            .as_ref()
            .is_err_and(|e| e.code == ErrorCode::CorruptLedger)
        {
            self.fenced = true;
            self.permit = false;
            self.sample_permit = false;
        }
        result
    }

    fn clock_projection(&self, work: &mut Work) -> Result<TreeClockOrigin> {
        work.charge(self.raw.len())?;
        // Open/appends already validated every immutable frame. Re-read the
        // retained original and each consumed sample rather than public caches.
        let mut reader = Reader(&self.raw);
        check(reader.take(8)? == b"DFMTJ021")?;
        check(read_blob(&mut reader, 2048)? == self.binding.encode())?;
        check(coord(&mut reader)? == self.root && reader.take(32)? != [0; 32])?;
        check(reader.take(32)? == self.id.as_bytes())?;
        let mut original_bytes = None;
        let mut last_record = &[][..];
        let mut state = 0;
        let mut pending_sample = false;
        let mut samples = Vec::with_capacity(MAX_SAMPLES as usize);
        let mut frames = 0;
        let mut head = self.id;
        while !reader.0.is_empty() {
            work.current()?;
            check(frames < MAX_FRAMES && reader.take(8)? == b"DFMTF021")?;
            let length = reader.u32()? as usize;
            check(length <= MAX_BODY)?;
            check(reader.u32()? == frames + 1 && reader.take(32)? == head.as_bytes())?;
            let mut body = Reader(reader.take(length)?);
            state = body.byte()?;
            check(state <= State::Terminal as u8)?;
            let current_original = read_blob(&mut body, MAX_CAPTURE_BYTES + 512)?;
            if let Some(original) = original_bytes {
                check(original == current_original)?;
            } else {
                original_bytes = Some(current_original);
            }
            last_record = read_blob(&mut body, MAX_RECORD_BYTES)?;
            let sample = read_blob(&mut body, 4096)?;
            let credit = body.boolean()?;
            let sampling = body.boolean()?;
            body.finish()?;
            check((!sample.is_empty() || !credit) && !(sampling && !sample.is_empty()))?;
            if sampling {
                check(!pending_sample)?;
                pending_sample = true;
            }
            if !sample.is_empty() {
                check(pending_sample && samples.len() < MAX_SAMPLES as usize)?;
                pending_sample = false;
                samples.push((sample, credit));
            }
            head = Digest32::from_bytes(reader.array()?);
            check(reader.take(8)? == b"DFMTEND1")?;
            frames += 1;
        }
        check(frames == self.frames && head == self.head)?;
        require(
            state == State::Terminal as u8,
            "Tree clock requires a resolved original designation",
        )?;
        let original_bytes = original_bytes.ok_or_else(corrupt)?;
        work.charge(2 * (original_bytes.len() + last_record.len() + 4096 * samples.len()))?;
        let original = Original::decode(original_bytes)?;
        self.binding.capture(&original.plan.before)?;
        check(original.plan.before.root == self.root)?;
        let record = Record::decode(last_record, &original.plan)?;
        require(
            record.phase == 2,
            "Tree clock requires the exact original designation receipt",
        )?;
        let after = record.after.as_ref().ok_or_else(corrupt)?;
        self.binding.capture(after)?;
        let mut high_tick = after.tick.max(original.plan.before.tick);
        let mut high_generation = original.plan.before.generation;
        let mut high_sequence = after.sequence.max(original.plan.before.sequence);
        let mut high_item = after.next_item.max(original.plan.before.next_item);
        let mut continuity_lost = false;
        let mut achieved_tick = None;
        let count = samples.len() as u32;
        for (bytes, credit) in samples {
            work.current()?;
            let sample = Progress::decode(bytes, &original.plan)?;
            check(
                sample.generation >= high_generation
                    && sample.tick >= high_tick
                    && sample.next_item >= high_item
                    && (sample.generation != high_generation || sample.sequence >= high_sequence),
            )?;
            continuity_lost |= sample.generation != original.plan.before.generation;
            high_tick = sample.tick;
            high_generation = sample.generation;
            high_sequence = sample.sequence;
            high_item = sample.next_item;
            if achieved_tick.is_none()
                && !continuity_lost
                && credit
                && sample.tick <= original.deadline_tick
                && sample.supply_observed()
            {
                achieved_tick = Some(sample.tick);
            }
        }
        work.current()?;
        Ok(TreeClockOrigin {
            binding: self.binding.clone(),
            pin: self.pin(),
            original,
            record,
            high_tick,
            high_generation,
            high_sequence,
            high_item,
            samples: count,
            pending_sample,
            continuity_lost,
            achieved_tick,
            retained_bytes: self.raw.len(),
        })
    }
}

#[cfg(all(test, target_os = "linux"))]
#[path = "clock_origin_tests.rs"]
mod tests;
