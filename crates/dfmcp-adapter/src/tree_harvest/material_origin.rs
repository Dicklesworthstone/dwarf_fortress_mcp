//! Read-only import of one acknowledged original Tree designation.
//! This descriptor cannot prepare, dispatch, sample or advance the Tree owner.
use super::*;

/// Exact historical custody used only to select a bounded fresh supply read.
/// It proves neither current material availability nor causal tree provenance.
/// The importing host must retain and reverify both held files around native I/O.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreeMaterialOrigin {
    binding: Binding,
    pin: WorkshopPin,
    original: Original,
    high_tick: u64,
    high_sequence: u64,
    high_item: u32,
    witness: Digest32,
}

impl TreeMaterialOrigin {
    pub fn binding(&self) -> &Binding {
        &self.binding
    }
    pub fn original(&self) -> &Original {
        &self.original
    }
    pub const fn pin(&self) -> WorkshopPin {
        self.pin
    }
    pub const fn high_tick(&self) -> u64 {
        self.high_tick
    }
    pub const fn high_sequence(&self) -> u64 {
        self.high_sequence
    }
    pub const fn high_item(&self) -> u32 {
        self.high_item
    }
    pub const fn witness(&self) -> Digest32 {
        self.witness
    }
}

impl<S: EffectJournalStorage> Journal<S> {
    /// Import the original only from an independently acknowledged, exclusively
    /// held Offline owner. This performs no native call or custody write and
    /// never derives its result from the journal's mutable public view fields.
    pub fn material_origin<A: EffectJournalStorage>(
        &mut self,
        acknowledgement: &mut WorkshopAcknowledgements<A>,
        context: &OperationContext,
    ) -> Result<TreeMaterialOrigin> {
        let result: Result<TreeMaterialOrigin> = (|| {
            check(self.mode == Mode::Offline && !self.fenced && self.owner == context.session_id)?;
            let mut work = Work::new(context)?;
            Self::authorize(&work.current()?, &self.binding, false, false, context.anchor.tick.get())?;
            // The retained chain was validated on open. Recover its actual
            // source floor before any new I/O, even if public view fields were
            // changed by an embedding after that open.
            let origin = self.material_projection(&self.raw, &mut work)?;
            work.context.anchor.tick = GameTick(origin.high_tick.max(context.anchor.tick.get()));
            Self::authorize(&work.current()?, &self.binding, false, false, origin.high_tick)?;
            let scope = self.binding.read_scope()?;
            acknowledgement.verify_tree(
                &work.reserve(crate::workshop_construction::acknowledgement::WORK_BYTES as usize)?,
                scope,
            )?;
            check(acknowledgement.latest() == Some(self.pin()))?;
            let raw = read(&mut self.storage, &mut work)?;
            check(raw == self.raw)?;
            acknowledgement.verify_tree(
                &work.reserve(crate::workshop_construction::acknowledgement::WORK_BYTES as usize)?,
                scope,
            )?;
            check(acknowledgement.latest() == Some(origin.pin))?;
            Self::authorize(&work.current()?, &self.binding, false, false, origin.high_tick)?;
            Ok(origin)
        })();
        if result.as_ref().is_err_and(|e| e.code == ErrorCode::CorruptLedger) {
            self.fenced = true;
            self.permit = false;
            self.sample_permit = false;
        }
        result
    }

    fn material_projection(&self, raw: &[u8], work: &mut Work) -> Result<TreeMaterialOrigin> {
        // Prepay the complete retained-byte traversal before following frames.
        work.charge(raw.len())?;
        // open_work/appends already validated the complete private byte chain.
        // The exact reread pins that chain. Decode its retained frames, not
        // writable public caches, without allocating or replaying large
        // captures per frame.
        let mut reader = Reader(raw);
        check(reader.take(8)? == b"DFMTJ021")?;
        check(read_blob(&mut reader, 2048)? == self.binding.encode())?;
        check(coord(&mut reader)? == self.root && reader.take(32)? != [0; 32])?;
        check(reader.take(32)? == self.id.as_bytes())?;
        let mut original_bytes = None;
        let mut last_record = &[][..];
        let mut last_state = 0;
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
            last_state = body.byte()?;
            check(last_state <= State::Terminal as u8)?;
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
                samples.push(sample);
            }
            head = Digest32::from_bytes(reader.array()?);
            check(reader.take(8)? == b"DFMTEND1")?;
            frames += 1;
        }
        check(frames == self.frames && head == self.head)?;
        require(
            last_state == State::Terminal as u8 && !pending_sample,
            "Tree material import requires resolved original custody with no pending sample",
        )?;
        let original_bytes = original_bytes.ok_or_else(corrupt)?;
        work.charge(2 * (original_bytes.len() + last_record.len() + 4096 * samples.len()))?;
        let original = Original::decode(original_bytes)?;
        self.binding.capture(&original.plan.before)?;
        check(original.plan.before.root == self.root)?;
        let record = Record::decode(last_record, &original.plan)?;
        require(record.phase == 2, "Tree material import requires the original designation receipt")?;
        let after = record.after.as_ref().ok_or_else(corrupt)?;
        let mut high_tick = after.tick.max(original.plan.before.tick);
        let mut high_sequence = after.sequence.max(original.plan.before.sequence);
        let mut high_item = after.next_item.max(original.plan.before.next_item);
        for raw_sample in samples {
            work.current()?;
            let sample = Progress::decode(raw_sample, &original.plan)?;
            require(
                sample.generation == self.binding.generation,
                "Tree material import cannot cross a retained native incarnation break",
            )?;
            high_tick = high_tick.max(sample.tick);
            high_sequence = high_sequence.max(sample.sequence);
            high_item = high_item.max(sample.next_item);
        }
        let pin = self.pin();
        let mut identity = b"dfmcp-tree-material-origin/1\0".to_vec();
        blob(&mut identity, &self.binding.encode());
        for digest in [pin.journal_id, pin.head, original.seal(), record.receipt] {
            identity.extend_from_slice(digest.as_bytes());
        }
        identity.extend_from_slice(&pin.frames.to_be_bytes());
        identity.extend_from_slice(&high_tick.to_be_bytes());
        identity.extend_from_slice(&high_sequence.to_be_bytes());
        identity.extend_from_slice(&high_item.to_be_bytes());
        work.current()?;
        Ok(TreeMaterialOrigin {
            binding: self.binding.clone(),
            pin,
            original,
            high_tick,
            high_sequence,
            high_item,
            witness: Digest32::of_bytes(&identity),
        })
    }
}
