//! One read-only current Manager sample exported with exact original custody.
use super::*;
use dfmcp_adapter::manager_appointment::production_origin::{
    EXPORT_READ_WORK_BYTES, EXPORT_WORK_BYTES, ManagerProductionHandoff, ManagerReadSource,
};

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct FloorInput {
    tick: u64,
    generation: u64,
    sequence: u64,
    next_event: u32,
    history_count: u32,
    group_next_position: u32,
    group_next_assignment: u32,
    civ_next_position: u32,
    civ_next_assignment: u32,
    fenced: bool,
}
impl FloorInput {
    pub(super) fn floor(&self) -> ManagerProductionFloor {
        ManagerProductionFloor {
            tick: self.tick,
            generation: self.generation,
            sequence: self.sequence,
            next_event: self.next_event,
            history_count: self.history_count,
            group_next_position: self.group_next_position,
            group_next_assignment: self.group_next_assignment,
            civ_next_position: self.civ_next_position,
            civ_next_assignment: self.civ_next_assignment,
            fenced: self.fenced,
        }
    }
    pub(super) fn validate(&self, generation: u64) -> Result<()> {
        self.floor().validate()?;
        if self.generation < generation || (!self.fenced && self.generation != generation) {
            return Err(denied(
                "Manager handoff recovery must retain its original generation or a sticky fence",
            ));
        }
        Ok(())
    }
}

pub(super) fn floor_json(floor: ManagerProductionFloor) -> Value {
    json!({
        "tick":floor.tick,"generation":floor.generation,"sequence":floor.sequence,
        "next_event":floor.next_event,"history_count":floor.history_count,
        "group_next_position":floor.group_next_position,
        "group_next_assignment":floor.group_next_assignment,
        "civ_next_position":floor.civ_next_position,
        "civ_next_assignment":floor.civ_next_assignment,"fenced":floor.fenced,
    })
}

/// Connection creation occurs inside the prepaid export, after the canonical
/// original and independent ACK have been verified. Every connection is opened
/// with Recover permissions: its writer callback always refuses appointment.
struct Source {
    config: Config,
    rpc: Option<ManagerRpc>,
    failed_generation: Option<u64>,
    consumed: bool,
}
impl ManagerReadSource for Source {
    fn binding(&self) -> &Binding {
        &self.config.binding
    }
    fn observed_generation(&self) -> Option<u64> {
        self.rpc
            .as_ref()
            .and_then(ManagerRpc::observed_generation)
            .or(self.failed_generation)
    }
    fn observed_floor(&self) -> Option<ManagerProductionFloor> {
        self.rpc.as_ref().and_then(ManagerRpc::observed_floor)
    }
    fn read_manager(&mut self, context: &OperationContext) -> Result<Capture> {
        if self.consumed || context.budget.max_bytes < EXPORT_READ_WORK_BYTES {
            return Err(error(
                ErrorCode::BudgetExceeded,
                "Manager handoff requires one prepaid complete source read",
            ));
        }
        self.consumed = true;
        let mut work = TurnWork::new(context, Instant::now())?;
        match self
            .config
            .connect(&work.reserve(context, CONNECT_BYTES)?, Mode::Recover)
        {
            Ok(rpc) => self.rpc = Some(rpc),
            Err(cause) => {
                self.failed_generation = cause
                    .details
                    .iter()
                    .find(|(key, _)| key == "consumed_manager_generation")
                    .and_then(|(_, value)| value.parse().ok());
                return Err(cause);
            }
        }
        let current = work.reserve(context, CONNECT_BYTES)?;
        let capture = self
            .rpc
            .as_mut()
            .ok_or_else(|| denied("Manager handoff source unavailable"))?
            .observe(self.config.binding.selection, &current)?;
        self.config.current()?;
        work.reserve(context, 0)?;
        Ok(capture)
    }
    fn fence(&mut self) {
        if let Some(rpc) = self.rpc.as_mut() {
            rpc.fence();
        }
    }
}

fn handoff_json(directory: &std::path::Path, handoff: &ManagerProductionHandoff) -> Result<Value> {
    let directory = directory
        .to_str()
        .ok_or_else(|| denied("Manager handoff path must be UTF-8"))?;
    let binding = handoff.binding();
    let pin = handoff.pin();
    let mut checkpoint = floor_json(handoff.floor());
    checkpoint["origin_witness"] = json!(handoff.origin_witness().to_hex());
    Ok(json!({
        "schema":"dfmcp.manager-production-handoff/1",
        "handoff_digest":handoff.witness().to_hex(),
        "manager_prerequisite":{
            "directory":directory,"endpoint":binding.endpoint.to_string(),
            "world_folder":binding.source.folder(),"site_id":binding.source.site(),
            "df_version":binding.df_version,"dfhack_version":binding.dfhack_version,
            "generation":binding.generation,"map_dimensions":binding.dimensions,
            "unit_id":binding.selection.unit,"histfig_id":binding.selection.figure,
            "acknowledged":{"journal_id":pin.journal_id.to_hex(),"head":pin.head.to_hex(),"frames":pin.frames},
            "original_key":handoff.plan().key(),"original_plan_digest":handoff.plan().digest.to_hex(),
        },
        "origin_witness":handoff.origin_witness().to_hex(),
        "manager_prerequisite_checkpoint":checkpoint,
        "origin_descriptor":{"encoding":"hex","byte_length":handoff.origin_canonical_bytes().len(),
            "sha256":handoff.origin_witness().to_hex(),"bytes":hex(handoff.origin_canonical_bytes())},
        "original_review_seal":handoff.original_seal().to_hex(),
        "original_receipt":handoff.receipt().to_hex(),
        "source_profile":"manager/1.23","retained_custody_verified":true,
        "historical_appointment_proven":true,
        "current_manager_sample":capture_json(handoff.capture()),
        "fresh_for_current_call":true,"production_handoff_floor":floor_json(handoff.floor()),
        "floor_custody":"retain separately in recovery configuration; not a new journal acknowledgement",
        "canonical_world_anchor":null,"production_ready":false,"mutation_authority":false,
        "native_observations":1,"native_mutations":0,"clock_advanced":false,
        "close_manager_session_before_import":true,
        "next_required_check":"close this Manager session; import the exact acknowledged original Offline; freshly qualify Manager readiness at the production owner's own paused source tick",
    }))
}

impl Runtime {
    pub(super) fn production_handoff(
        &mut self,
        context: &OperationContext,
        work: &mut TurnWork,
    ) -> Result<Value> {
        self.config.current()?;
        if self.config.binding.protocol != ManagerProtocol::V1_23 || self.mode == Mode::Offline {
            return Err(denied(
                "current production handoff requires read-enabled Manager 1.23; Offline custody remains locally inspectable",
            ));
        }
        // Preserve the complete closing custody check before acquiring a source.
        if work.left < EXPORT_WORK_BYTES + 32 * 1024 * 1024 {
            return Err(error(
                ErrorCode::BudgetExceeded,
                "Manager handoff and closing custody checks must be prepaid together",
            ));
        }
        let current = work.reserve(context, EXPORT_WORK_BYTES)?;
        let mut source = Source {
            config: self.config.clone(),
            rpc: None,
            failed_generation: None,
            consumed: false,
        };
        let result = self.journal.export_production_handoff(
            &mut self.acks,
            &mut source,
            &mut self.handoff_floor,
            &current,
        );
        if let Some(floor) = self.handoff_floor {
            self.high_tick = self.high_tick.max(floor.tick);
            self.high_sequence = self.high_sequence.max(floor.sequence);
        }
        self.config.current()?;
        let handoff = result?;
        let value = handoff_json(&self.config.input.directory, &handoff)?;
        if value.to_string().len() as u64 > RESPONSE_BYTES {
            return Err(error(
                ErrorCode::BudgetExceeded,
                "complete Manager handoff exceeds output allowance",
            ));
        }
        work.reserve(context, 0)?;
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handoff_recovery_floor_is_closed_monotone_and_never_readiness() -> Result<()> {
        let floor = ManagerProductionFloor {
            tick: 40,
            generation: 7,
            sequence: 4,
            next_event: 9,
            history_count: 9,
            group_next_position: 2,
            group_next_assignment: 3,
            civ_next_position: 4,
            civ_next_assignment: 5,
            fenced: false,
        };
        let value = floor_json(floor);
        let parsed: FloorInput = serde_json::from_value(value.clone())
            .map_err(|_| denied("test floor failed to decode"))?;
        parsed.validate(7)?;
        assert_eq!(parsed.floor(), floor);
        assert!(parsed.validate(8).is_err());
        for key in ["ready", "production_ready", "source", "permit"] {
            let mut forged = value.clone();
            forged[key] = json!(true);
            assert!(serde_json::from_value::<FloorInput>(forged).is_err());
        }
        let mut changed = value;
        changed["generation"] = json!(8);
        let newer: FloorInput = serde_json::from_value(changed.clone())
            .map_err(|_| denied("test floor failed to decode"))?;
        assert!(newer.validate(7).is_err());
        changed["fenced"] = json!(true);
        let fenced: FloorInput =
            serde_json::from_value(changed).map_err(|_| denied("test floor failed to decode"))?;
        fenced.validate(7)?;
        assert!(fenced.floor().fenced);
        Ok(())
    }
}
