//! Read-only handoff from an acknowledged original workshop to production.
//!
//! Historical custody selects the original building; only a new authenticated
//! progress read can qualify its current completion. This owner grants no
//! mutation permission, canonical world fact, resource reservation, manager
//! readiness, production attribution or cross-plugin atomicity.
use super::*;
use crate::workshop_construction::acknowledgement::{
    WORK_BYTES as ACK_WORK_BYTES, WorkshopAcknowledgements,
};
use crate::workshop_construction::rpc::WorkshopManifest;
use dfmcp_core::StateAnchor;

/// Complete bounded journal replay, two ACK reads and the original descriptor.
pub const IMPORT_WORK_BYTES: u64 = 96 * 1024 * 1024;
/// One full exact journal read, two ACK reads and bounded identity checks.
pub const VERIFY_WORK_BYTES: u64 = 24 * 1024 * 1024;
/// Both local verification brackets, one progress RPC and bounded proof work.
pub const REFRESH_WORK_BYTES: u64 = 64 * 1024 * 1024;
/// Maximum bytes in the importing host's separate immutable plan association.
pub const ASSOCIATION_MAX_BYTES: usize = 64 * 1024;

fn denied(message: &str) -> dfmcp_core::DfmcpError {
    error(ErrorCode::CapabilityDenied, message)
}
fn stale(message: &str) -> dfmcp_core::DfmcpError {
    error(ErrorCode::StaleAnchor, message)
}
fn prepaid(context: &OperationContext, bytes: u64) -> Result<Work> {
    let mut work = Work::new(context)?;
    if work.bytes < bytes {
        return Err(exhausted());
    }
    work.bytes = bytes;
    Ok(work)
}

/// Open only the bounded private association used by the importing production
/// host. Creation needs current Query and Plan; reopening is descriptor-pinned
/// Offline storage. This wrapper does not encode, repair or interpret content.
pub fn open_private_association_storage(
    path: &Path,
    context: &OperationContext,
    initialize: bool,
) -> Result<(PrivateBuildFile, OperationContext)> {
    context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    if initialize {
        context.authorize(Capability::Plan, RiskTier::Guarded, &[], None)?;
    }
    let mode = if initialize {
        BuildMode::Control
    } else {
        BuildMode::Offline
    };
    let (storage, created, current) =
        open_private_storage(path, context, mode, initialize, ASSOCIATION_MAX_BYTES)?;
    check(created == initialize)?;
    current.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    if initialize {
        current.authorize(Capability::Plan, RiskTier::Guarded, &[], None)?;
    }
    Ok((storage, current))
}

/// One immutable original plus non-cloneable consumed source floors. Hold this
/// owner and both Offline files for the entire importing session. A failed
/// acquisition must not be evaded by reminting it from the historical donor.
/// Hosts persist the exact descriptor and consumed recovery floors separately;
/// this importer never writes or advances the donor.
#[derive(Debug)]
pub struct WorkshopProductionOrigin {
    binding: WorkshopBinding,
    pin: WorkshopPin,
    record: WorkshopRecord,
    canonical: Vec<u8>,
    witness: Digest32,
    owner: SessionId,
    epoch: u64,
    high_tick: u64,
    high_generation: u64,
    high_building: u32,
    fenced: bool,
}

/// Qualified completion at this exact read and caller anchor. It is evidence
/// for a host-owned prerequisite, not a canonical predicate or future promise.
/// Every later effect boundary must call `WorkshopProductionOrigin::refresh`.
#[derive(Debug)]
pub struct CompletedWorkshopProduction {
    origin: Digest32,
    anchor: StateAnchor,
    progress: WorkshopProgress,
    witness: Digest32,
}
impl CompletedWorkshopProduction {
    pub fn origin_witness(&self) -> Digest32 {
        self.origin
    }
    pub fn anchor(&self) -> StateAnchor {
        self.anchor
    }
    pub fn progress(&self) -> &WorkshopProgress {
        &self.progress
    }
    pub fn witness(&self) -> Digest32 {
        self.witness
    }
}

impl WorkshopProductionOrigin {
    pub fn binding(&self) -> &WorkshopBinding {
        &self.binding
    }
    pub fn pin(&self) -> WorkshopPin {
        self.pin
    }
    pub fn plan(&self) -> &WorkshopPlan {
        self.record.plan()
    }
    pub fn historical_record(&self) -> &WorkshopRecord {
        &self.record
    }
    pub fn receipt(&self) -> Digest32 {
        self.record.receipt()
    }
    /// Stable across reopening under another authorized session. Native reads
    /// never rewrite these exact donor bytes or their original identity.
    pub fn canonical_bytes(&self) -> &[u8] {
        &self.canonical
    }
    pub fn witness(&self) -> Digest32 {
        self.witness
    }
    pub fn high_tick(&self) -> u64 {
        self.high_tick
    }
    pub fn high_generation(&self) -> u64 {
        self.high_generation
    }
    pub fn high_next_building_id(&self) -> u32 {
        self.high_building
    }
    pub fn is_fenced(&self) -> bool {
        self.fenced
    }
    /// Withdraw future qualification after an ambiguous connection/setup read
    /// outside `refresh`. This one-way restriction needs no surviving grant;
    /// local historical verification remains available under current Query.
    pub fn fence(&mut self) {
        self.fenced = true;
    }

    /// Narrow this owner using the importing host's externally retained
    /// recovery custody. These maxima are rejection evidence, not fresh native
    /// facts or a donor acknowledgement. They can never lower a consumed floor,
    /// clear a fence, change the immutable descriptor or restore permission.
    /// Reissue current Query authority at the retained tick independently.
    pub fn retain_recovery_floor(
        &mut self,
        tick: u64,
        generation: u64,
        next_building_id: u32,
        fenced: bool,
        context: &OperationContext,
    ) -> Result<()> {
        let work = Work::new(context)?;
        self.access(&work.current()?, false)?;
        if tick > crate::workshop_construction::MAX_NATIVE_TICK
            || generation == 0
            || generation == u64::MAX
            || next_building_id > crate::workshop_construction::MAX_ID
        {
            return Err(error(
                ErrorCode::InvalidRequest,
                "invalid workshop recovery frontier",
            ));
        }
        if tick < self.high_tick
            || generation < self.high_generation
            || next_building_id < self.high_building
        {
            return Err(stale(
                "workshop recovery frontier cannot lower consumed source floors",
            ));
        }
        self.high_tick = tick;
        self.high_generation = generation;
        self.high_building = next_building_id;
        self.fenced |= fenced || generation != self.binding.generation();
        // An expired grant at this newly retained tick cannot erase the floor.
        self.access(&work.current()?, false)?;
        Ok(())
    }

    fn access(&self, context: &OperationContext, observe: bool) -> Result<OperationContext> {
        if self.owner != context.session_id || self.epoch != context.anchor.cursor.epoch {
            return Err(denied(
                "workshop production origin belongs to another session or epoch",
            ));
        }
        authorize(
            context,
            &self.binding,
            self.high_tick,
            if observe {
                WorkshopStage::Sample
            } else {
                WorkshopStage::Query
            },
        )
    }

    fn verify_with<S: EffectJournalStorage, A: EffectJournalStorage>(
        &mut self,
        journal: &mut WorkshopJournal<S>,
        acknowledgement: &mut WorkshopAcknowledgements<A>,
        work: &mut Work,
    ) -> Result<()> {
        work.context = self.access(&work.current()?, false)?;
        if journal.mode != WorkshopMode::Offline
            || journal.owner != self.owner
            || journal.binding != self.binding
            || journal.pin() != self.pin
        {
            self.fenced = true;
            return Err(corrupt());
        }
        acknowledgement.verify_workshop_offline(&work.reserve(ACK_WORK_BYTES)?)?;
        check(acknowledgement.latest() == Some(self.pin))?;
        journal.verify(work)?;
        acknowledgement.verify_workshop_offline(&work.reserve(ACK_WORK_BYTES)?)?;
        check(acknowledgement.latest() == Some(self.pin))?;
        self.access(&work.current()?, false)?;
        Ok(())
    }

    /// Exact local historical custody only: no credential, native call, append,
    /// current-completion claim or permission restoration, even after fencing.
    pub fn verify<S: EffectJournalStorage, A: EffectJournalStorage>(
        &mut self,
        journal: &mut WorkshopJournal<S>,
        acknowledgement: &mut WorkshopAcknowledgements<A>,
        context: &OperationContext,
    ) -> Result<()> {
        let mut work = prepaid(context, VERIFY_WORK_BYTES)?;
        let result = self.verify_with(journal, acknowledgement, &mut work);
        if result
            .as_ref()
            .is_err_and(|e| e.code == ErrorCode::CorruptLedger)
        {
            self.fenced = true;
        }
        result
    }

    /// Retain an already decoded handshake before any later fallible budget,
    /// authority or custody check. This only raises rejection floors or fences
    /// the owner: it performs no I/O, needs no surviving grant and establishes
    /// neither current completion nor permission. A matching manifest cannot
    /// clear an existing fence or lower the retained source generation.
    pub fn retain_source_manifest(&mut self, current: &WorkshopManifest) -> Result<()> {
        self.high_generation = self.high_generation.max(current.generation);
        if current.endpoint != self.binding.endpoint()
            || current.fortress != *self.binding.fortress()
            || current.generation != self.binding.generation()
            || current.df_version != self.binding.df_version()
            || current.dfhack_version != self.binding.dfhack_version()
        {
            self.fenced = true;
            return Err(stale("workshop production source binding changed"));
        }
        Ok(())
    }

    /// Read the exact original building once between independently reverified
    /// donor custody. `expected_tick` must be the host's freshly observed paused
    /// tick, not a historical tick copied from this donor. The existing native
    /// profile enforces paused fortress reads; its generation remains separate
    /// from the caller's canonical cursor and other native families.
    pub fn refresh<S: EffectJournalStorage, A: EffectJournalStorage, N: WorkshopSource>(
        &mut self,
        journal: &mut WorkshopJournal<S>,
        acknowledgement: &mut WorkshopAcknowledgements<A>,
        source: &mut N,
        expected_tick: GameTick,
        context: &OperationContext,
    ) -> Result<CompletedWorkshopProduction> {
        // The caller may already have consumed this manifest while connecting.
        // Losing the remaining deadline or authority cannot erase that source
        // generation before the bounded local and native read work begins.
        if let Err(cause) = self.retain_source_manifest(source.manifest()) {
            source.fence();
            return Err(cause);
        }
        let mut work = prepaid(context, REFRESH_WORK_BYTES)?;
        if self.fenced {
            return Err(stale(
                "workshop production origin is fenced; retain original custody",
            ));
        }
        work.context = self.access(&work.current()?, true)?;
        if expected_tick.get() < self.high_tick || expected_tick != context.anchor.tick {
            return Err(stale(
                "workshop prerequisite tick differs from the current host anchor",
            ));
        }
        let result = (|| {
            self.verify_with(journal, acknowledgement, &mut work)?;
            self.retain_source_manifest(source.manifest())?;
            let insertion = self.record.insertion().ok_or_else(corrupt)?;
            let (building, item) = (insertion.building_id(), insertion.item_id());
            let current = self.access(&work.reserve(RPC_RESERVE)?, true)?;
            // A native error may have consumed an unreturned source frontier.
            // Fence until independently recovered rather than forget it.
            let progress = match source.progress(building, item, &current, work.remaining()?) {
                Ok(progress) => progress,
                Err(cause) => {
                    self.fenced = true;
                    return Err(cause);
                }
            };
            let regressed = progress.tick() < self.high_tick
                || progress.next_building_id() < self.high_building
                || progress.generation() != self.binding.generation();
            // Preserve every decoded frontier BEFORE any later policy,
            // cross-source, custody or deadline check can fail.
            self.high_tick = self.high_tick.max(progress.tick());
            self.high_generation = self.high_generation.max(progress.generation());
            self.high_building = self.high_building.max(progress.next_building_id());
            if regressed
                || progress.fortress() != self.binding.fortress()
                || progress.dimensions() != self.binding.dimensions()
                || progress.building_id() != building
                || progress.item_id() != item
            {
                self.fenced = true;
                return Err(stale(
                    "workshop prerequisite source or allocator continuity broke",
                ));
            }
            // Absence or a changed original identity is consumed just like a
            // higher allocator frontier, even if later authority expires.
            let goal = WorkshopGoal::evaluate(&self.record, &progress);
            if !matches!(
                goal,
                WorkshopGoal::Complete | WorkshopGoal::InProgress { .. }
            ) {
                self.fenced = true;
                return Err(stale(
                    "original workshop is absent, contradicted or unqualified",
                ));
            }
            self.retain_source_manifest(source.manifest())?;
            work.context = self.access(&work.current()?, true)?;
            self.verify_with(journal, acknowledgement, &mut work)?;
            if progress.tick() != expected_tick.get() {
                return Err(stale(
                    "workshop progress differs from the fresh paused host tick",
                ));
            }
            if goal != WorkshopGoal::Complete {
                return Err(error(
                    ErrorCode::Conflict,
                    "original workshop construction is incomplete",
                ));
            }
            work.charge(2 * MAX_PROGRESS_BYTES + 256)?;
            let mut identity = b"dfmcp-workshop-production-completion/1\0".to_vec();
            identity.extend_from_slice(self.witness.as_bytes());
            identity.extend_from_slice(context.session_id.to_string().as_bytes());
            identity.extend_from_slice(&context.anchor.cursor.epoch.to_be_bytes());
            identity.extend_from_slice(&context.anchor.cursor.sequence.to_be_bytes());
            identity.extend_from_slice(context.anchor.state_hash.as_bytes());
            identity.extend_from_slice(progress.witness().as_bytes());
            self.access(&work.current()?, true)?;
            Ok(CompletedWorkshopProduction {
                origin: self.witness,
                anchor: context.anchor,
                progress,
                witness: Digest32::of_bytes(&identity),
            })
        })();
        source.fence();
        if result
            .as_ref()
            .is_err_and(|e| e.code == ErrorCode::CorruptLedger)
        {
            self.fenced = true;
        }
        result
    }
}

impl<S: EffectJournalStorage> WorkshopJournal<S> {
    /// Mint one source selector from exact independently acknowledged Offline
    /// custody. A Placed receipt is required; its historical completion is not.
    /// The caller must retain this owner, not repeatedly remint it after reads.
    pub fn production_origin<A: EffectJournalStorage>(
        &mut self,
        acknowledgement: &mut WorkshopAcknowledgements<A>,
        key: &str,
        digest: Digest32,
        context: &OperationContext,
    ) -> Result<WorkshopProductionOrigin> {
        let mut work = prepaid(context, IMPORT_WORK_BYTES)?;
        if self.mode != WorkshopMode::Offline {
            return Err(denied(
                "workshop production import requires Offline donor custody",
            ));
        }
        work.context = self.access(&work.current()?)?;
        acknowledgement.verify_workshop_offline(&work.reserve(ACK_WORK_BYTES)?)?;
        check(acknowledgement.latest() == Some(self.pin()))?;
        self.verify(&mut work)?;
        work.charge(MAX_BODY_BYTES)?;
        let entry = self.known(key, digest)?;
        if !entry.placed() || entry.needs_reconciliation() || entry.continuity_lost {
            return Err(stale(
                "workshop production import needs the resolved original Placed receipt",
            ));
        }
        let (high_tick, high_building) = self.production_floors(key, &mut work)?;
        work.context.anchor.tick = GameTick(high_tick.max(context.anchor.tick.get()));
        self.access(&work.current()?)?;
        acknowledgement.verify_workshop_offline(&work.reserve(ACK_WORK_BYTES)?)?;
        check(acknowledgement.latest() == Some(self.pin()))?;
        let record = entry.native.ok_or_else(corrupt)?;
        work.charge(2 * MAX_BODY_BYTES + MAX_BINDING_BYTES)?;
        let pin = self.pin();
        let mut canonical = b"dfmcp-workshop-production-origin/1\0".to_vec();
        field(&mut canonical, &self.binding.encode());
        canonical.extend_from_slice(pin.journal_id.as_bytes());
        canonical.extend_from_slice(pin.head.as_bytes());
        canonical.extend_from_slice(&pin.frames.to_be_bytes());
        field(&mut canonical, record.canonical_bytes());
        canonical.extend_from_slice(&high_tick.to_be_bytes());
        canonical.extend_from_slice(&high_building.to_be_bytes());
        self.access(&work.current()?)?;
        Ok(WorkshopProductionOrigin {
            binding: self.binding.clone(),
            pin,
            record,
            witness: Digest32::of_bytes(&canonical),
            canonical,
            owner: context.session_id,
            epoch: context.anchor.cursor.epoch,
            high_tick,
            high_generation: self.binding.generation(),
            high_building,
            fenced: false,
        })
    }

    /// The journal's current entry retains tick/generation floors, but an older
    /// progress sample may contain the largest native allocation horizon. Walk
    /// the complete bounded immutable chain rather than trust only the last
    /// sample. Do not relabel repeated old receipt bytes as fresh observations.
    fn production_floors(&self, selected: &str, work: &mut Work) -> Result<(u64, u32)> {
        work.charge(self.raw.len().checked_mul(3).ok_or_else(exhausted)?)?;
        let mut r = Reader(&self.raw);
        check(r.take(8)? == MAGIC)?;
        check(r.field(MAX_BINDING_BYTES)? == self.binding.encode())?;
        r.take(64)?; // Already validated nonce and journal identity.
        let mut seen = BTreeMap::<String, (Option<Digest32>, u16)>::new();
        let mut high_tick = 0;
        let mut high_building = 0;
        let mut consume = |tick: u64, generation: u64, horizon: u32| -> Result<()> {
            if generation != self.binding.generation()
                || tick < high_tick
                || horizon < high_building
            {
                return Err(stale(
                    "retained workshop history lost source or allocator continuity",
                ));
            }
            high_tick = tick;
            high_building = horizon;
            Ok(())
        };
        let mut frames = 0;
        while !r.0.is_empty() {
            work.current()?;
            check(frames < MAX_FRAMES && r.take(8)? == FRAME)?;
            let length = r.number()? as usize;
            check(length <= MAX_BODY_BYTES)?;
            r.take(40)?; // Existing open verified sequence and prior digest.
            let entry = WorkshopEntry::decode(r.take(length)?, &self.binding)?;
            r.take(40)?; // Existing open verified frame digest and terminator.
            frames += 1;
            if entry.continuity_lost || entry.high_generation != self.binding.generation() {
                return Err(stale(
                    "retained workshop history crossed a native incarnation",
                ));
            }
            let prior = seen.get(entry.plan.key()).copied();
            if prior.is_none() {
                let before = entry.plan.before();
                consume(
                    before.tick(),
                    before.generation(),
                    before.next_building_id(),
                )?;
            }
            let after = entry.native.as_ref().and_then(WorkshopRecord::after);
            let after_witness = after.map(|capture| capture.witness());
            if after_witness.is_some() && prior.and_then(|p| p.0) != after_witness {
                let after = after.ok_or_else(corrupt)?;
                consume(after.tick(), after.generation(), after.next_building_id())?;
            }
            if entry.samples > prior.map_or(0, |p| p.1) {
                let sample = entry.latest.as_ref().ok_or_else(corrupt)?;
                if sample.fortress() != self.binding.fortress()
                    || sample.dimensions() != self.binding.dimensions()
                {
                    return Err(stale("retained workshop sample belongs to another source"));
                }
                consume(
                    sample.tick(),
                    sample.generation(),
                    sample.next_building_id(),
                )?;
                if entry.plan.key() == selected {
                    let record = entry.native.as_ref().ok_or_else(corrupt)?;
                    if !matches!(
                        WorkshopGoal::evaluate(record, sample),
                        WorkshopGoal::Complete | WorkshopGoal::InProgress { .. }
                    ) {
                        return Err(stale("retained original workshop identity was lost"));
                    }
                }
            }
            seen.insert(entry.plan.key().to_owned(), (after_witness, entry.samples));
        }
        check(frames == self.frames)?;
        Ok((high_tick, high_building))
    }
}

#[cfg(test)]
#[path = "production_origin_tests.rs"]
mod tests;
