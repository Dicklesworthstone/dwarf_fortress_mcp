//! Read-only handoff from one acknowledged Manager/1.23 appointment.
//!
//! The original transaction selects an exact citizen, historical figure and
//! assignment. Only a new authenticated ReadManager can qualify that assignment
//! and its production cache at the importing host's current paused tick. This
//! module cannot appoint a Manager, update an order, advance time or prove that
//! production will occur. Historical custody never becomes current readiness.
use super::*;
use crate::manager_appointment::rpc::ManagerRpc;
use crate::workshop_construction::acknowledgement::WORK_BYTES as ACK_WORK_BYTES;
use dfmcp_core::StateAnchor;

pub const IMPORT_WORK_BYTES: u64 = 96 * 1024 * 1024;
pub const VERIFY_WORK_BYTES: u64 = 24 * 1024 * 1024;
pub const REFRESH_WORK_BYTES: u64 = 64 * 1024 * 1024;
pub const MAX_ORIGINAL_FRAMES: u32 = MAX_FRAMES;
/// Full native records remain in the original donor. This descriptor commits
/// their complete bytes without copying up to 129 KiB into a host association.
pub const DESCRIPTOR_MAX_BYTES: usize = 4096;
const JOURNAL_WORK_BYTES: usize = 2 * MAX_BYTES + 1024;
const READ_WORK_BYTES: usize = 8 * 1024 * 1024;
const MAX_NATIVE_TICK: u64 = u32::MAX as u64 * 403200 + 403199;

fn denied(message: &str) -> dfmcp_core::DfmcpError {
    error(ErrorCode::CapabilityDenied, message)
}
fn stale(message: &str) -> dfmcp_core::DfmcpError {
    error(ErrorCode::StaleAnchor, message)
}
fn prepaid(context: &OperationContext, bytes: u64) -> Result<Work> {
    let mut work = Work::new(context)?;
    if work.left < bytes {
        return Err(error(
            ErrorCode::BudgetExceeded,
            "complete Manager production prerequisite work must be prepaid",
        ));
    }
    work.left = bytes;
    Ok(work)
}

/// An independently authenticated, host-pinned reader. No writer is part of
/// this interface. Connection/setup failures before refresh must fence the
/// origin; a successfully decoded source binding is retained immediately.
pub trait ManagerReadSource {
    fn binding(&self) -> &Binding;
    fn observed_generation(&self) -> Option<u64> {
        None
    }
    fn observed_floor(&self) -> Option<ManagerProductionFloor> {
        None
    }
    fn read_manager(&mut self, context: &OperationContext) -> Result<Capture>;
    fn fence(&mut self);
}
impl ManagerReadSource for ManagerRpc {
    fn binding(&self) -> &Binding {
        ManagerRpc::binding(self)
    }
    fn observed_generation(&self) -> Option<u64> {
        ManagerRpc::observed_generation(self)
    }
    fn observed_floor(&self) -> Option<ManagerProductionFloor> {
        ManagerRpc::observed_floor(self)
    }
    fn read_manager(&mut self, context: &OperationContext) -> Result<Capture> {
        let selection = self.binding().selection;
        self.observe(selection, context)
    }
    fn fence(&mut self) {
        ManagerRpc::fence(self);
    }
}

/// Monotone rejection custody for the importing host. It contains no source
/// grant, current proof or durable acknowledgement of subsequent native reads.
/// A restored owner cannot detect rollback of every external copy of this
/// floor beyond the independently acknowledged donor's immutable seed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ManagerProductionFloor {
    pub tick: u64,
    pub generation: u64,
    pub sequence: u64,
    pub next_event: u32,
    pub history_count: u32,
    pub group_next_position: u32,
    pub group_next_assignment: u32,
    pub civ_next_position: u32,
    pub civ_next_assignment: u32,
    pub fenced: bool,
}
impl ManagerProductionFloor {
    pub fn validate(&self) -> Result<()> {
        if self.tick > MAX_NATIVE_TICK
            || self.generation == 0
            || self.generation == u64::MAX
            || self.sequence == u64::MAX
            || self.next_event >= i32::MAX as u32
            || self.history_count > 65536
            || [
                self.group_next_position,
                self.group_next_assignment,
                self.civ_next_position,
                self.civ_next_assignment,
            ]
            .iter()
            .any(|value| *value > i32::MAX as u32)
        {
            return Err(error(
                ErrorCode::InvalidRequest,
                "invalid Manager production recovery frontier",
            ));
        }
        Ok(())
    }
    pub(crate) fn capture(capture: &Capture) -> Self {
        Self {
            tick: capture.tick,
            generation: capture.generation,
            sequence: capture.sequence,
            next_event: capture.next_event,
            history_count: capture.history_count,
            group_next_position: capture.group.next_position as u32,
            group_next_assignment: capture.group.next_assignment as u32,
            civ_next_position: capture.civ.next_position as u32,
            civ_next_assignment: capture.civ.next_assignment as u32,
            fenced: false,
        }
    }
    fn dominates(&self, old: &Self) -> bool {
        self.tick >= old.tick
            && self.generation >= old.generation
            && self.sequence >= old.sequence
            && self.next_event >= old.next_event
            && self.history_count >= old.history_count
            && self.group_next_position >= old.group_next_position
            && self.group_next_assignment >= old.group_next_assignment
            && self.civ_next_position >= old.civ_next_position
            && self.civ_next_assignment >= old.civ_next_assignment
    }
    pub(crate) fn retain(&mut self, next: Self) {
        self.tick = self.tick.max(next.tick);
        self.generation = self.generation.max(next.generation);
        self.sequence = self.sequence.max(next.sequence);
        self.next_event = self.next_event.max(next.next_event);
        self.history_count = self.history_count.max(next.history_count);
        self.group_next_position = self.group_next_position.max(next.group_next_position);
        self.group_next_assignment = self.group_next_assignment.max(next.group_next_assignment);
        self.civ_next_position = self.civ_next_position.max(next.civ_next_position);
        self.civ_next_assignment = self.civ_next_assignment.max(next.civ_next_assignment);
        self.fenced |= next.fenced;
    }
    fn append(&self, bytes: &mut Vec<u8>) {
        for value in [self.tick, self.generation, self.sequence] {
            bytes.extend_from_slice(&value.to_be_bytes());
        }
        for value in [
            self.next_event,
            self.history_count,
            self.group_next_position,
            self.group_next_assignment,
            self.civ_next_position,
            self.civ_next_assignment,
        ] {
            bytes.extend_from_slice(&value.to_be_bytes());
        }
        bytes.push(u8::from(self.fenced));
    }
}

/// One immutable donor and its exclusive process-local observation history.
/// Keep this owner and both Offline files alive throughout the importing
/// session. Reopening never restores a cached readiness observation.
#[derive(Debug)]
pub struct ManagerProductionOrigin {
    binding: Binding,
    pin: WorkshopPin,
    original: Original,
    record: Record,
    canonical: Vec<u8>,
    witness: Digest32,
    owner: SessionId,
    epoch: u64,
    floor: ManagerProductionFloor,
}

/// Readiness at one exact authenticated capture and caller anchor. This value
/// is not a reservation or permission for a later native effect boundary.
#[derive(Debug)]
pub struct ReadyManagerProduction {
    origin: Digest32,
    anchor: StateAnchor,
    capture: Capture,
    witness: Digest32,
}
impl ReadyManagerProduction {
    pub fn origin_witness(&self) -> Digest32 {
        self.origin
    }
    pub fn anchor(&self) -> StateAnchor {
        self.anchor
    }
    pub fn capture(&self) -> &Capture {
        &self.capture
    }
    pub fn witness(&self) -> Digest32 {
        self.witness
    }
}

impl ManagerProductionOrigin {
    pub fn binding(&self) -> &Binding {
        &self.binding
    }
    pub fn pin(&self) -> WorkshopPin {
        self.pin
    }
    pub fn plan(&self) -> &Plan {
        &self.original.plan
    }
    pub fn original_seal(&self) -> Digest32 {
        self.original.seal()
    }
    pub fn original_deadline_tick(&self) -> u64 {
        self.original.deadline_tick
    }
    pub fn historical_record(&self) -> &Record {
        &self.record
    }
    pub fn receipt(&self) -> Digest32 {
        self.record.receipt
    }
    pub fn canonical_bytes(&self) -> &[u8] {
        &self.canonical
    }
    pub fn witness(&self) -> Digest32 {
        self.witness
    }
    pub fn floor(&self) -> ManagerProductionFloor {
        self.floor
    }
    pub fn high_tick(&self) -> u64 {
        self.floor.tick
    }
    pub fn is_fenced(&self) -> bool {
        self.floor.fenced
    }
    pub fn fence(&mut self) {
        self.floor.fenced = true;
    }

    fn access(&self, context: &OperationContext, observe: bool) -> Result<OperationContext> {
        if self.owner != context.session_id || self.epoch != context.anchor.cursor.epoch {
            return Err(denied(
                "Manager production origin belongs to another session or epoch",
            ));
        }
        let mut current = context.clone();
        current.anchor.tick = GameTick(current.anchor.tick.get().max(self.floor.tick));
        Journal::<PrivateBuildFile>::authorize(
            &current,
            &self.binding,
            false,
            observe,
            self.floor.tick,
        )?;
        Ok(current)
    }

    /// Raise every retained frontier; never clear a fence or renew authority.
    pub fn retain_recovery_floor(
        &mut self,
        floor: ManagerProductionFloor,
        context: &OperationContext,
    ) -> Result<()> {
        let work = Work::new(context)?;
        self.access(&work.current()?, false)?;
        floor.validate()?;
        if !floor.dominates(&self.floor) {
            return Err(stale(
                "Manager recovery cannot lower consumed source floors",
            ));
        }
        self.floor.retain(floor);
        self.floor.fenced |= floor.generation != self.binding.generation;
        self.access(&work.current()?, false)?;
        Ok(())
    }

    /// Consume an already authenticated source binding before later fallible
    /// budget or custody checks. A mismatch permanently withdraws readiness.
    pub fn retain_source_binding(&mut self, binding: &Binding) -> Result<()> {
        self.retain_source_generation(binding.generation)?;
        if binding != &self.binding {
            self.fence();
            return Err(stale("Manager production source binding changed"));
        }
        Ok(())
    }

    /// Retain a generation authenticated by the exact RPC reply profile and
    /// nonce even when it contradicts the expected operator pin.
    pub fn retain_source_generation(&mut self, generation: u64) -> Result<()> {
        if generation == 0 || generation == u64::MAX {
            self.fence();
            return Err(stale("Manager source returned an invalid generation"));
        }
        self.floor.generation = self.floor.generation.max(generation);
        if generation != self.binding.generation {
            self.fence();
            return Err(stale("Manager source generation changed"));
        }
        Ok(())
    }

    /// A failed connection cannot produce readiness. Preserve any authenticated
    /// generation it decoded before refusing the expected source pin.
    pub fn retain_connection_error(&mut self, cause: &dfmcp_core::DfmcpError) -> Result<()> {
        self.fence();
        for (key, value) in &cause.details {
            if key == "consumed_manager_generation" {
                let generation = value
                    .parse::<u64>()
                    .map_err(|_| stale("invalid retained Manager connection generation"))?;
                self.retain_source_generation(generation)?;
            }
        }
        Ok(())
    }

    /// Consume decoded native state before any later budget, authority or
    /// custody failure. Expected binding fields alone are not observations.
    pub fn retain_source_state<N: ManagerReadSource>(&mut self, source: &N) -> Result<()> {
        let mut invalid = false;
        if let Some(next) = source.observed_floor() {
            if next.validate().is_err() {
                self.fence();
                return Err(stale("invalid decoded Manager source frontier"));
            }
            invalid |= !next.dominates(&self.floor);
            invalid |= next.generation != self.binding.generation;
            self.floor.retain(next);
        }
        if let Some(generation) = source.observed_generation() {
            invalid |= self.retain_source_generation(generation).is_err();
        }
        invalid |= self.retain_source_binding(source.binding()).is_err();
        if invalid {
            self.fence();
            return Err(stale(
                "Manager source identity or retained frontier changed",
            ));
        }
        Ok(())
    }

    fn verify_with<S: EffectJournalStorage, A: EffectJournalStorage>(
        &mut self,
        journal: &mut Journal<S>,
        acknowledgement: &mut WorkshopAcknowledgements<A>,
        work: &mut Work,
    ) -> Result<()> {
        work.context = self.access(&work.current()?, false)?;
        if journal.mode != Mode::Offline
            || journal.owner != self.owner
            || journal.binding != self.binding
            || journal.pin() != self.pin
        {
            self.fence();
            return Err(corrupt());
        }
        let scope = self.binding.read_scope()?;
        acknowledgement
            .verify_manager_v1_23_offline(&work.reserve(ACK_WORK_BYTES as usize)?, scope)?;
        check(acknowledgement.latest() == Some(self.pin))?;
        journal.verify(&work.reserve(JOURNAL_WORK_BYTES)?)?;
        acknowledgement
            .verify_manager_v1_23_offline(&work.reserve(ACK_WORK_BYTES as usize)?, scope)?;
        check(acknowledgement.latest() == Some(self.pin))?;
        self.access(&work.current()?, false)?;
        Ok(())
    }

    /// Exact local historical custody only, including after a source fence.
    /// No credential, native read, donor append or readiness claim is created.
    pub fn verify<S: EffectJournalStorage, A: EffectJournalStorage>(
        &mut self,
        journal: &mut Journal<S>,
        acknowledgement: &mut WorkshopAcknowledgements<A>,
        context: &OperationContext,
    ) -> Result<()> {
        let mut work = prepaid(context, VERIFY_WORK_BYTES)?;
        let result = self.verify_with(journal, acknowledgement, &mut work);
        if result
            .as_ref()
            .is_err_and(|cause| cause.code == ErrorCode::CorruptLedger)
        {
            self.fence();
        }
        result
    }

    fn original_identity_matches(&self, current: &Capture) -> bool {
        let Some(after) = self.record.after.as_ref() else {
            return false;
        };
        let Some((_, original_assignment)) = after.target() else {
            return false;
        };
        let manager_links: Vec<_> = current
            .links
            .iter()
            .filter(|link| {
                link.kind == 10
                    && link.entity == after.group.id
                    && link.assignment == original_assignment.id
            })
            .collect();
        let original_link = after.links.iter().find(|link| {
            link.kind == 10
                && link.entity == after.group.id
                && link.assignment == original_assignment.id
        });
        let Some((index, assignment)) = current.target() else {
            return false;
        };
        let link_matches = manager_links
            .first()
            .copied()
            .zip(original_link)
            .is_some_and(|(link, original)| {
                link.entity_cache == current.group.index
                    && link.assignment_cache == index as i32
                    && link.strength == original.strength
                    && link.start_year == original.start_year
            });
        current.identity
            && current.member
            && current.role_eligible
            && current.dual_getters
            && current.group.id == after.group.id
            && current.civ.id == after.civ.id
            && current.manager_position == after.manager_position
            && current.manager_index >= 0
            && assignment.id == original_assignment.id
            && assignment.position == original_assignment.position
            && assignment.figure == original_assignment.figure
            && assignment.last_figure == original_assignment.last_figure
            && assignment.cache == current.manager_index
            && assignment.ordinary_active()
            && current.manager_links == 1
            && current.lookup_unit == after.selection.unit as i32
            && manager_links.len() == 1
            && link_matches
    }

    fn ready(&self, current: &Capture) -> bool {
        let Some((_, assignment)) = current.target() else {
            return false;
        };
        current.paused
            && current.ui_quiescent
            && current.active
            && current.citizen
            && current.adult
            && current.sane
            && current
                .group
                .role_buckets
                .get(MANAGE_PRODUCTION)
                .is_some_and(|bucket| bucket.contains(&assignment.id))
    }

    /// Independently qualify one current ReadManager between exact donor
    /// custody checks. Equal native sequences are permitted: the native counter
    /// records interventions, not read calls. All decoded floors survive errors.
    pub fn refresh<S: EffectJournalStorage, A: EffectJournalStorage, N: ManagerReadSource>(
        &mut self,
        journal: &mut Journal<S>,
        acknowledgement: &mut WorkshopAcknowledgements<A>,
        source: &mut N,
        expected_tick: GameTick,
        context: &OperationContext,
    ) -> Result<ReadyManagerProduction> {
        let binding = self.retain_source_state(source);
        let result = (|| {
            binding?;
            let mut work = prepaid(context, REFRESH_WORK_BYTES)?;
            if self.is_fenced() {
                return Err(stale(
                    "Manager production origin is fenced; retain original custody",
                ));
            }
            work.context = self.access(&work.current()?, true)?;
            if expected_tick.get() < self.floor.tick || expected_tick != context.anchor.tick {
                return Err(stale(
                    "Manager prerequisite tick differs from the current host anchor",
                ));
            }
            self.verify_with(journal, acknowledgement, &mut work)?;
            self.retain_source_state(source)?;
            // Prepay canonical decoding before the native call. A returned
            // frontier must survive even if the following deadline expires.
            work.charge(4 * MAX_CAPTURE_BYTES)?;
            let current = self.access(&work.reserve(READ_WORK_BYTES)?, true)?;
            let read = source.read_manager(&current);
            let retained = self.retain_source_state(source);
            let captured = match read {
                Ok(capture) => capture,
                Err(cause) => {
                    // The RPC may have decoded a later capture before a final
                    // authority check failed. No unreturned floor is reusable.
                    self.fence();
                    return Err(cause);
                }
            };
            let capture = match Capture::decode(captured.bytes()) {
                Ok(capture) => capture,
                Err(cause) => {
                    self.fence();
                    return Err(cause);
                }
            };
            let next = ManagerProductionFloor::capture(&capture);
            let regressed = !next.dominates(&self.floor);
            self.floor.retain(next);
            retained?;
            if regressed || capture != captured || self.binding.capture(&capture).is_err() {
                self.fence();
                return Err(stale(
                    "Manager source, original identity or allocation frontier changed",
                ));
            }
            if !self.original_identity_matches(&capture) {
                self.fence();
                return Err(stale(
                    "the original Manager assignment, link or definition was lost",
                ));
            }
            self.retain_source_binding(source.binding())?;
            work.context = self.access(&work.current()?, true)?;
            self.verify_with(journal, acknowledgement, &mut work)?;
            if capture.tick != expected_tick.get() {
                return Err(stale(
                    "Manager observation differs from the fresh paused host tick",
                ));
            }
            if !self.ready(&capture) {
                return Err(error(
                    ErrorCode::Conflict,
                    "the original Manager is not currently ready for production",
                ));
            }
            work.charge(DESCRIPTOR_MAX_BYTES + 512)?;
            let mut bytes = b"dfmcp-manager-production-readiness/1\0".to_vec();
            bytes.extend_from_slice(self.witness.as_bytes());
            field(&mut bytes, context.session_id.to_string().as_bytes());
            bytes.extend_from_slice(&context.anchor.cursor.epoch.to_be_bytes());
            bytes.extend_from_slice(&context.anchor.cursor.sequence.to_be_bytes());
            bytes.extend_from_slice(&context.anchor.tick.get().to_be_bytes());
            bytes.extend_from_slice(context.anchor.state_hash.as_bytes());
            bytes.extend_from_slice(capture.witness().as_bytes());
            self.access(&work.current()?, true)?;
            Ok(ReadyManagerProduction {
                origin: self.witness,
                anchor: context.anchor,
                capture,
                witness: Digest32::of_bytes(&bytes),
            })
        })();
        source.fence();
        if result
            .as_ref()
            .is_err_and(|cause| cause.code == ErrorCode::CorruptLedger)
        {
            self.fence();
        }
        result
    }
}

impl<S: EffectJournalStorage> Journal<S> {
    /// Read the already validated private canonical chain, not publicly mutable
    /// diagnostic fields. Its exact bytes are independently reverified below.
    fn production_record(&self, work: &mut Work) -> Result<(Original, Record)> {
        work.charge(self.raw.len().saturating_mul(2) + 8 * MAX_RECORD_BYTES)?;
        let mut reader = Reader(&self.raw);
        check(reader.take(8)? == ManagerProtocol::V1_23.magic(b'J'))?;
        check(read_blob(&mut reader, 2048)? == self.binding.encode())?;
        check(Selection::read(&mut reader)? == self.selection)?;
        reader.take(64)?; // Private, previously validated nonce and journal ID.
        let mut frames = 0;
        let mut last = None;
        while !reader.0.is_empty() {
            work.current()?;
            check(frames < MAX_FRAMES && reader.take(8)? == ManagerProtocol::V1_23.magic(b'F'))?;
            let length = reader.u32()? as usize;
            check(length <= MAX_BODY)?;
            reader.take(36)?; // Validated u32 sequence and previous frame digest.
            last = Some(reader.take(length)?);
            reader.take(40)?; // Validated frame digest and terminator.
            frames += 1;
        }
        check(frames == self.frames && frames > 0)?;
        let mut body = Reader(last.ok_or_else(corrupt)?);
        check(body.byte()? == State::Terminal as u8)?;
        let original = Original::decode(read_blob(&mut body, MAX_CAPTURE_BYTES + 512)?)?;
        let record = Record::decode(read_blob(&mut body, MAX_RECORD_BYTES)?, &original.plan)?;
        body.finish()?;
        check(record.complete())?;
        self.binding.capture(&original.plan.before)?;
        self.binding
            .capture(record.after.as_ref().ok_or_else(corrupt)?)?;
        Ok((original, record))
    }

    /// Import only exact, resolved, independently acknowledged Offline custody
    /// of a cache-bearing original. No credential, native read, file write or
    /// appointment permission is used or returned.
    pub fn production_origin<A: EffectJournalStorage>(
        &mut self,
        acknowledgement: &mut WorkshopAcknowledgements<A>,
        key: &str,
        digest: Digest32,
        context: &OperationContext,
    ) -> Result<ManagerProductionOrigin> {
        let mut work = prepaid(context, IMPORT_WORK_BYTES)?;
        if self.mode != Mode::Offline
            || self.binding.protocol != ManagerProtocol::V1_23
            || self.owner != context.session_id
            || self.fenced
        {
            return Err(denied(
                "Manager production import requires exact Offline Manager/1.23 custody",
            ));
        }
        Self::authorize(
            &work.current()?,
            &self.binding,
            false,
            false,
            self.high_tick,
        )?;
        let (original, record) = self.production_record(&mut work)?;
        if original.plan.key() != key || original.plan.digest != digest {
            return Err(denied(
                "Manager production import names another original appointment",
            ));
        }
        let floor = ManagerProductionFloor::capture(record.after.as_ref().ok_or_else(corrupt)?);
        floor.validate()?;
        work.context.anchor.tick = GameTick(work.context.anchor.tick.get().max(floor.tick));
        Self::authorize(&work.current()?, &self.binding, false, false, floor.tick)?;
        work.charge(2 * MAX_RECORD_BYTES + DESCRIPTOR_MAX_BYTES)?;
        let pin = self.pin();
        let mut canonical = b"dfmcp-manager-production-origin/1\0".to_vec();
        field(&mut canonical, &self.binding.encode());
        canonical.extend_from_slice(pin.journal_id.as_bytes());
        canonical.extend_from_slice(pin.head.as_bytes());
        canonical.extend_from_slice(&pin.frames.to_be_bytes());
        canonical.extend_from_slice(original.seal().as_bytes());
        canonical.extend_from_slice(record.receipt.as_bytes());
        canonical.extend_from_slice(Digest32::of_bytes(record.bytes()).as_bytes());
        floor.append(&mut canonical);
        check(canonical.len() <= DESCRIPTOR_MAX_BYTES)?;
        let mut origin = ManagerProductionOrigin {
            binding: self.binding.clone(),
            pin,
            original,
            record,
            witness: Digest32::of_bytes(&canonical),
            canonical,
            owner: context.session_id,
            epoch: context.anchor.cursor.epoch,
            floor,
        };
        origin.verify_with(self, acknowledgement, &mut work)?;
        Ok(origin)
    }
}

#[cfg(all(test, target_os = "linux"))]
#[path = "production_origin_tests.rs"]
mod tests;
