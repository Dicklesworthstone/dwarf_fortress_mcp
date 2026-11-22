//! Export one verified completed original for a later production owner.
//!
//! This is a read-only custody handoff, never current workshop readiness or
//! permission to dispatch. The importing owner must close the donor writer,
//! reopen the exact ACK Offline, and freshly qualify completion before effects.
use super::*;
use dfmcp_adapter::workshop_construction::journal::WorkshopEntry;
use std::path::Path;

const DESCRIPTOR_BYTES: u64 = 16 * 1024;
const ACK_BYTES: u64 = 1024 * 1024;

/// Metadata minted only from the retained owner after complete custody checks.
/// It contains no native credential, construction permit or fresh proof.
#[derive(Clone, Debug)]
pub(crate) struct ProductionHandoff {
    directory: PathBuf,
    binding: WorkshopBinding,
    acknowledged: WorkshopPin,
    original_key: String,
    original_plan_digest: Digest32,
    original_receipt: Digest32,
    building_id: u32,
    item_id: u32,
    completion_tick: u64,
    completion_witness: Digest32,
    authority_tick_floor: u64,
    clock: Option<clock::ProductionHandoff>,
    digest: Digest32,
}

fn append(bytes: &mut Vec<u8>, value: &[u8]) -> Result<()> {
    let length = u32::try_from(value.len()).map_err(|_| exhausted())?;
    if bytes.len().saturating_add(4).saturating_add(value.len()) > DESCRIPTOR_BYTES as usize {
        return Err(exhausted());
    }
    bytes.extend_from_slice(&length.to_be_bytes());
    bytes.extend_from_slice(value);
    Ok(())
}

impl ProductionHandoff {
    pub(crate) fn directory(&self) -> &Path {
        &self.directory
    }

    pub(crate) fn binding(&self) -> &WorkshopBinding {
        &self.binding
    }

    pub(crate) fn pin(&self) -> WorkshopPin {
        self.acknowledged
    }

    pub(crate) fn original_key(&self) -> &str {
        &self.original_key
    }

    pub(crate) fn original_plan_digest(&self) -> Digest32 {
        self.original_plan_digest
    }

    pub(crate) fn authority_tick_floor(&self) -> u64 {
        self.authority_tick_floor
    }

    pub(crate) fn digest(&self) -> Digest32 {
        self.digest
    }

    /// Existing closed combined-production operator input. No MCP request may
    /// substitute a path, source pin, building ID or caller-produced descriptor.
    pub(crate) fn prerequisite_json(&self) -> Value {
        let binding = self.binding();
        json!({
            "directory":self.directory().to_string_lossy(),
            "endpoint":binding.endpoint().to_string(),
            "world_folder":binding.fortress().folder(),
            "site_id":binding.fortress().site(),
            "df_version":binding.df_version(),
            "dfhack_version":binding.dfhack_version(),
            "generation":binding.generation(),
            "map_dimensions":binding.dimensions(),
            "acknowledged":pin_json(self.pin()),
            "original_key":self.original_key(),
            "original_plan_digest":self.original_plan_digest().to_hex(),
        })
    }

    pub(crate) fn to_json(&self) -> Value {
        json!({
            "schema":"dfmcp.workshop-production-handoff/1",
            "handoff_digest":self.digest().to_hex(),
            "workshop_prerequisite":self.prerequisite_json(),
            "original_receipt":self.original_receipt.to_hex(),
            "building_id":self.building_id,
            "material_item_id":self.item_id,
            "historical_completion":{
                "tick":self.completion_tick,
                "witness":self.completion_witness.to_hex(),
                "fresh_for_current_call":false,
            },
            "authority_tick_floor":self.authority_tick_floor(),
            "authority_tick_floor_custody":"retain separately; not an immutable descriptor or durable acknowledgement",
            "clock":self.clock.as_ref().map(clock::ProductionHandoff::to_json)
                .unwrap_or_else(||json!({"enabled":false})),
            "source_profile":"workshop/1.20",
            "retained_custody_verified":true,
            "historical_completion_proven":true,
            "current_completion_proven":false,
            "current_pause_proven":false,
            "production_ready":false,
            "mutation_authority":false,
            "native_calls":0,
            "close_workshop_session_before_import":true,
            "next_required_check":"import exact acknowledged original Offline, then freshly qualify completion at the production owner's paused source tick",
        })
    }

    fn seal(&self) -> Result<Digest32> {
        let binding = self.binding();
        let directory = self.directory().to_str()
            .ok_or_else(|| invalid("workshop handoff custody path must be UTF-8"))?;
        let mut bytes = b"dfmcp-workshop-production-handoff/1\0".to_vec();
        append(&mut bytes, directory.as_bytes())?;
        append(&mut bytes, binding.endpoint().to_string().as_bytes())?;
        append(&mut bytes, binding.fortress().folder().as_bytes())?;
        bytes.extend_from_slice(&binding.fortress().site().to_be_bytes());
        bytes.extend_from_slice(&binding.generation().to_be_bytes());
        for dimension in binding.dimensions() {
            bytes.extend_from_slice(&dimension.to_be_bytes());
        }
        append(&mut bytes, binding.df_version().as_bytes())?;
        append(&mut bytes, binding.dfhack_version().as_bytes())?;
        bytes.extend_from_slice(self.acknowledged.journal_id.as_bytes());
        bytes.extend_from_slice(self.acknowledged.head.as_bytes());
        bytes.extend_from_slice(&self.acknowledged.frames.to_be_bytes());
        append(&mut bytes, self.original_key().as_bytes())?;
        bytes.extend_from_slice(self.original_plan_digest.as_bytes());
        bytes.extend_from_slice(self.original_receipt.as_bytes());
        bytes.extend_from_slice(&self.building_id.to_be_bytes());
        bytes.extend_from_slice(&self.item_id.to_be_bytes());
        bytes.extend_from_slice(&self.completion_tick.to_be_bytes());
        bytes.extend_from_slice(self.completion_witness.as_bytes());
        match &self.clock {
            None => bytes.push(0),
            Some(clock) => {
                bytes.push(1);
                clock.append_identity(&mut bytes);
            }
        }
        // The moving authority floor is deliberately separate from immutable
        // custody identity. A later production tick cannot change this donor.
        if bytes.len() > DESCRIPTOR_BYTES as usize {
            return Err(exhausted());
        }
        Ok(Digest32::of_bytes(&bytes))
    }
}

fn complete_original(entry: &WorkshopEntry, binding: &WorkshopBinding) -> Result<()> {
    if !entry.placed()
        || entry.unresolved()
        || entry.needs_reconciliation()
        || !entry.goal_complete()
        || entry.high_generation() != binding.generation()
        || entry.plan().before().generation() != binding.generation()
        || entry.latest_sample().is_none()
    {
        return Err(error(
            ErrorCode::Conflict,
            "production handoff requires the resolved original Carpenter and its continuous completed-construction observation",
        ));
    }
    Ok(())
}

impl Runtime {
    fn handoff_ack(&self, pin: WorkshopPin, c: &OperationContext) -> Result<()> {
        self.config.current()?;
        let mut acknowledgements = self.guard.acks.try_lock()
            .map_err(|_| denied("workshop handoff acknowledgement is unavailable"))?;
        acknowledgements.verify(c)?;
        if acknowledgements.latest() != Some(pin) {
            return Err(error(
                ErrorCode::CorruptLedger,
                "workshop handoff requires the exact independently acknowledged journal head",
            ));
        }
        self.config.current()
    }

    fn production_handoff_inner(&mut self, work: &mut Work) -> Result<ProductionHandoff> {
        self.config.current()?;
        self.authority_tick = self.authority_tick.max(self.journal.high_tick());
        if let Some(clock) = &self.clock {
            self.authority_tick = self.authority_tick.max(clock.high_tick());
        }
        work.advance_authorization_tick(self.authority_tick);
        let inventory = self.journal.inventory(&work.reserve(INSPECT_BYTES)?)?;
        if inventory.entries().len() != 1 || self.journal.is_fenced() {
            return Err(denied("production handoff requires one intact original workshop"));
        }
        let entry = &inventory.entries()[0];
        self.config.binding(self.journal.binding())?;
        complete_original(entry, self.journal.binding())?;
        let pin = self.journal.pin();
        self.handoff_ack(pin, &work.reserve(ACK_BYTES)?)?;
        let clock = if let Some(owner) = &mut self.clock {
            let result = owner.production_handoff(
                self.journal.binding(),
                &inventory,
                &work.reserve(clock::HANDOFF_BYTES)?,
            );
            self.authority_tick = self.authority_tick.max(owner.high_tick());
            work.advance_authorization_tick(self.authority_tick);
            Some(result?)
        } else {
            None
        };
        // A typed descriptor is published only after both custody stores have
        // survived the complete clock inspection under the same request budget.
        let final_inventory = self.journal.inventory(&work.reserve(INSPECT_BYTES)?)?;
        if self.journal.pin() != pin
            || final_inventory.entries() != inventory.entries()
            || self.journal.is_fenced()
        {
            return Err(error(
                ErrorCode::CorruptLedger,
                "original workshop changed while exporting production custody",
            ));
        }
        self.handoff_ack(pin, &work.reserve(ACK_BYTES)?)?;
        let current = work.reserve(2 * DESCRIPTOR_BYTES)?;
        current.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
        let record = entry.native()
            .ok_or_else(|| denied("original placed workshop receipt is absent"))?;
        let insertion = record.insertion()
            .ok_or_else(|| denied("original placed workshop insertion is absent"))?;
        let completion = entry.latest_sample()
            .ok_or_else(|| denied("original completed workshop observation is absent"))?;
        let mut handoff = ProductionHandoff {
            directory: self.config.input.directory.clone(),
            binding: self.journal.binding().clone(),
            acknowledged: pin,
            original_key: entry.plan().key().to_owned(),
            original_plan_digest: entry.plan().digest(),
            original_receipt: record.receipt(),
            building_id: insertion.building_id(),
            item_id: insertion.item_id(),
            completion_tick: completion.tick(),
            completion_witness: completion.witness(),
            authority_tick_floor: self.authority_tick.max(entry.high_tick()).max(work.context.anchor.tick.get()),
            clock,
            digest: Digest32::ZERO,
        };
        handoff.digest = handoff.seal()?;
        self.config.current()?;
        work.current(1)?.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
        Ok(handoff)
    }

    /// Local authenticated reads only. Failures withdraw any process-local
    /// preparation and preserve all original journals and consumed clocks.
    pub(crate) fn production_handoff(&mut self, c: &OperationContext) -> Result<ProductionHandoff> {
        let result = (|| {
            let mut work = Work::new(c, self.config.view_bytes())?;
            self.production_handoff_inner(&mut work)
        })();
        if result.is_err() {
            self.withdraw();
        }
        result
    }

    pub(crate) fn production_handoff_view(&mut self, c: &OperationContext) -> Result<Value> {
        let started = Instant::now();
        let result = (|| {
            // production_handoff prepays the ordinary final view and output
            // reserve as well as its complete local custody work.
            let handoff = self.production_handoff(c)?;
            let mut current = c.clone();
            current.budget.max_wall_millis = c.budget.max_wall_millis
                .checked_sub(started.elapsed().as_millis() as u64)
                .filter(|left| *left > 0)
                .ok_or_else(exhausted)?;
            current.budget.max_bytes = self.config.view_bytes();
            current.anchor.tick = GameTick(current.anchor.tick.get().max(handoff.authority_tick_floor()));
            // ACK already equals the current journal pin. This ordinary view
            // cannot append a new acknowledgement for an unchanged head.
            let mut view = self.view(&current, false)?;
            view["production_handoff"] = handoff.to_json();
            if view.to_string().len() as u64 > RESPONSE_BYTES
                || started.elapsed().as_millis() >= u128::from(c.budget.max_wall_millis)
            {
                return Err(exhausted());
            }
            current.anchor.tick = GameTick(current.anchor.tick.get().max(self.authority_tick));
            current.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
            Ok(view)
        })();
        if result.is_err() {
            self.withdraw();
        }
        result
    }
}
