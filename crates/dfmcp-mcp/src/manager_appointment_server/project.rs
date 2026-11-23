//! Embedded Manager/1.23 custody for one immutable native resource project.
//! The parent owns stage transitions; appointment permission remains inside
//! the original independently acknowledged Manager owner.
use super::*;
use dfmcp_adapter::bounded_run::MAX_NATIVE_TICK;
use dfmcp_adapter::manager_appointment::production_origin::{
    DESCRIPTOR_MAX_BYTES, MAX_ORIGINAL_FRAMES, ManagerProductionOrigin,
};
use std::path::Path;

const HANDOFF_SCHEMA: &str = "dfmcp.manager-project-handoff/1";
const HANDOFF_BYTES: usize = 16 * 1024;

fn invalid(message: &str) -> DfmcpError {
    error(ErrorCode::InvalidRequest, message)
}
fn exhausted() -> DfmcpError {
    error(
        ErrorCode::BudgetExceeded,
        "original Manager project work allowance exhausted",
    )
}

fn pin_json(pin: WorkshopPin) -> Value {
    json!({"journal_id":pin.journal_id.to_hex(),"head":pin.head.to_hex(),"frames":pin.frames})
}
fn parse_pin(value: &Value, allow_empty: bool) -> Result<WorkshopPin> {
    let input: PinInput = serde_json::from_value(value.clone())
        .map_err(|_| invalid("invalid retained Manager acknowledgement"))?;
    let pin = WorkshopPin {
        journal_id: digest(&input.journal_id)?,
        head: digest(&input.head)?,
        frames: input.frames,
    };
    if pin.journal_id == Digest32::ZERO
        || pin.head == Digest32::ZERO
        || pin.frames > MAX_ORIGINAL_FRAMES
        || (!allow_empty && pin.frames == 0)
    {
        return Err(invalid("invalid retained Manager custody identity"));
    }
    Ok(pin)
}

fn private_path(path: &Path) -> Result<()> {
    let raw = path
        .to_str()
        .ok_or_else(|| invalid("Manager custody path must be UTF-8"))?;
    if !path.is_absolute()
        || !(2..=3800).contains(&raw.len())
        || raw[1..].split('/').any(|part| {
            part.is_empty() || part == "." || part == ".." || part.chars().any(char::is_control)
        })
    {
        return Err(invalid(
            "Manager project custody needs a canonical absolute directory",
        ));
    }
    Ok(())
}

impl Config {
    pub(crate) fn from_resource_project_environment(
        parent_check: Arc<dyn Fn() -> Result<()> + Send + Sync>,
    ) -> Result<Self> {
        runtime_io()?;
        parent_check()?;
        family_environment(ManagerProtocol::V1_23)?;
        let raw = std::env::var(config_name_for(ManagerProtocol::V1_23))
            .map_err(|_| denied("resource project Manager configuration is missing"))?;
        let mut config = Self::parse_protocol(&raw, ManagerProtocol::V1_23)?;
        config.parent_check = Some(parent_check);
        config.current()?;
        Ok(config)
    }

    pub(crate) fn with_project_deadline(mut self, deadline: u64) -> Result<Self> {
        if self.parent_check.is_none()
            || self.binding.protocol != ManagerProtocol::V1_23
            || deadline == 0
            || deadline > MAX_NATIVE_TICK
            || deadline > self.input.authority_expires_tick
            || self.project_deadline.is_some_and(|old| old != deadline)
        {
            return Err(invalid(
                "Manager project deadline must be fixed inside its original host authority",
            ));
        }
        self.project_deadline = Some(deadline);
        Ok(self)
    }

    pub(super) fn project_effect_deadline(&self, tick: u64) -> Result<()> {
        if self
            .project_deadline
            .is_some_and(|deadline| tick >= deadline)
        {
            return Err(denied("original resource project deadline has elapsed"));
        }
        Ok(())
    }

    pub(crate) fn project_identity(&self) -> Digest32 {
        let i = &self.input;
        // Initialization, ACK and rejection floors are custody selectors, not
        // replacement source/authority policy on an authenticated restart.
        Digest32::of_bytes(
            json!({
                "schema":"dfmcp.manager-project-policy/1",
                "protocol":"1.23","directory":i.directory,"endpoint":i.endpoint,
                "world_folder":i.world_folder,"site_id":i.site_id,
                "df_version":i.df_version,"dfhack_version":i.dfhack_version,
                "generation":i.generation,"map_dimensions":i.map_dimensions,
                "unit_id":i.unit_id,"histfig_id":i.histfig_id,
                "read_scope":i.read_scope,"scope":i.scope,
                "authority_expires_tick":i.authority_expires_tick,
                "lease_ticks":i.lease_ticks,"checkpoint":i.checkpoint,
                "deadline_tick":self.project_deadline,
            })
            .to_string()
            .as_bytes(),
        )
    }
    pub(crate) fn project_binding(&self) -> &Binding {
        &self.binding
    }
    pub(crate) fn project_directory(&self) -> &Path {
        &self.input.directory
    }
    pub(crate) fn project_initializing(&self) -> bool {
        self.input.initialize
    }
    pub(crate) fn project_deadline_tick(&self) -> Option<u64> {
        self.project_deadline
    }

    pub(crate) fn validate_project_handoff(&self, handoff: &ManagerProjectHandoff) -> Result<()> {
        if self.parent_check.is_none()
            || self.binding != handoff.binding
            || self.input.directory != handoff.directory
            || self.project_deadline != Some(handoff.deadline_tick)
        {
            return Err(denied(
                "Manager handoff differs from the original project policy",
            ));
        }
        Ok(())
    }

    pub(crate) fn validate_project_recovery(&self, reference: &Value) -> Result<()> {
        self.current()?;
        self.project_recovery_pins(reference)
    }

    fn project_recovery_pins(&self, reference: &Value) -> Result<()> {
        if self.parent_check.is_none()
            || self.project_deadline.is_none()
            || self.input.initialize
            || self.external != Some(parse_pin(&reference["acknowledged"], true)?)
        {
            return Err(denied(
                "Manager recovery must select the exact parent-held original acknowledgement",
            ));
        }
        Ok(())
    }
}

pub(super) fn capture_floor(capture: &Capture) -> ManagerProductionFloor {
    ManagerProductionFloor {
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
fn dominates(next: ManagerProductionFloor, old: ManagerProductionFloor) -> bool {
    next.tick >= old.tick
        && next.generation >= old.generation
        && next.sequence >= old.sequence
        && next.next_event >= old.next_event
        && next.history_count >= old.history_count
        && next.group_next_position >= old.group_next_position
        && next.group_next_assignment >= old.group_next_assignment
        && next.civ_next_position >= old.civ_next_position
        && next.civ_next_assignment >= old.civ_next_assignment
}
pub(super) fn retain_floor(
    retained: &mut Option<ManagerProductionFloor>,
    next: ManagerProductionFloor,
) {
    if let Some(old) = retained {
        old.tick = old.tick.max(next.tick);
        old.generation = old.generation.max(next.generation);
        old.sequence = old.sequence.max(next.sequence);
        old.next_event = old.next_event.max(next.next_event);
        old.history_count = old.history_count.max(next.history_count);
        old.group_next_position = old.group_next_position.max(next.group_next_position);
        old.group_next_assignment = old.group_next_assignment.max(next.group_next_assignment);
        old.civ_next_position = old.civ_next_position.max(next.civ_next_position);
        old.civ_next_assignment = old.civ_next_assignment.max(next.civ_next_assignment);
        old.fenced |= next.fenced;
    } else {
        *retained = Some(next);
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Frontier {
    high_tick: u64,
    high_generation: u64,
    high_sequence: u64,
    next_event: u32,
    history_count: u32,
    group_next_position: u32,
    group_next_assignment: u32,
    civ_next_position: u32,
    civ_next_assignment: u32,
    fenced: bool,
}
impl Frontier {
    fn floor(self) -> ManagerProductionFloor {
        ManagerProductionFloor {
            tick: self.high_tick,
            generation: self.high_generation,
            sequence: self.high_sequence,
            next_event: self.next_event,
            history_count: self.history_count,
            group_next_position: self.group_next_position,
            group_next_assignment: self.group_next_assignment,
            civ_next_position: self.civ_next_position,
            civ_next_assignment: self.civ_next_assignment,
            fenced: self.fenced,
        }
    }
}

fn narrow(ceiling: WorkBudget, remaining: WorkBudget, start: Instant) -> Result<WorkBudget> {
    let budget = WorkBudget {
        max_wall_millis: ceiling
            .max_wall_millis
            .min(remaining.max_wall_millis)
            .checked_sub(start.elapsed().as_millis() as u64)
            .filter(|left| *left > 0)
            .ok_or_else(exhausted)?,
        max_game_ticks: 0,
        max_bytes: ceiling.max_bytes.min(remaining.max_bytes),
        max_entities: ceiling.max_entities.min(remaining.max_entities),
        max_actions: ceiling.max_actions.min(remaining.max_actions).min(1),
        max_output_tokens: ceiling.max_output_tokens.min(remaining.max_output_tokens),
    };
    budget.validate()?;
    Ok(budget)
}

impl Runtime {
    pub(crate) fn open_project(
        config: Config,
        mode: Mode,
        session: SessionId,
        budget: WorkBudget,
        start: Instant,
        parent_tick_floor: u64,
    ) -> Result<(Self, Value)> {
        if config.parent_check.is_none()
            || config.project_deadline.is_none()
            || config.binding.protocol != ManagerProtocol::V1_23
            || parent_tick_floor > MAX_NATIVE_TICK
        {
            return Err(denied(
                "Manager project opening needs its fixed 1.23 source, goal and valid frontier",
            ));
        }
        let (mut runtime, _) =
            Self::open_at(config, mode, session, budget, start, parent_tick_floor)?;
        // The parent retains this owner before validating and saving its
        // original policy. A conflicting recovered descendant must remain
        // inspectable and must never gain a restored preparation or be adopted.
        runtime.raise_project_tick_floor(parent_tick_floor)?;
        let view = runtime.project_view(runtime.view(false));
        Ok((runtime, view))
    }

    pub(crate) fn project_has_original_history(&self) -> bool {
        self.journal.pin().frames != 0
    }

    pub(crate) fn validate_project_original_policy(&self) -> Result<()> {
        if self.config.parent_check.is_none()
            || self.config.project_deadline.is_none()
            || self.config.binding.protocol != ManagerProtocol::V1_23
            || self.journal.original.as_ref().is_some_and(|original| {
                Some(original.deadline_tick) != self.config.project_deadline
                    || original.plan.before.protocol != ManagerProtocol::V1_23
                    || self.config.binding.capture(&original.plan.before).is_err()
            })
        {
            return Err(denied(
                "Manager original differs from its fixed project source or deadline",
            ));
        }
        Ok(())
    }

    pub(crate) fn raise_project_tick_floor(&mut self, floor: u64) -> Result<()> {
        if floor > MAX_NATIVE_TICK {
            return Err(invalid("invalid Manager project rejection tick"));
        }
        self.high_tick = self.high_tick.max(floor);
        if let Some(retained) = &mut self.handoff_floor {
            retained.tick = retained.tick.max(floor);
        }
        Ok(())
    }

    fn project_floor(&self) -> ManagerProductionFloor {
        let mut floor = self.handoff_floor;
        for capture in [
            self.journal.original.as_ref().map(|o| &o.plan.before),
            self.journal.native.as_ref().and_then(|n| n.after.as_ref()),
            self.observation.as_ref(),
        ]
        .into_iter()
        .flatten()
        {
            retain_floor(&mut floor, capture_floor(capture));
        }
        let mut floor = floor.unwrap_or(ManagerProductionFloor {
            tick: 0,
            generation: self.config.binding.generation,
            sequence: 0,
            next_event: 0,
            history_count: 0,
            group_next_position: 0,
            group_next_assignment: 0,
            civ_next_position: 0,
            civ_next_assignment: 0,
            fenced: false,
        });
        floor.tick = floor.tick.max(self.high_tick).max(self.journal.high_tick);
        floor.generation = floor.generation.max(self.journal.high_generation);
        floor.sequence = floor
            .sequence
            .max(self.high_sequence)
            .max(self.journal.high_sequence);
        floor.fenced |= self.project_fenced || floor.generation != self.config.binding.generation;
        floor
    }

    fn project_frontier(&self) -> Value {
        let f = self.project_floor();
        json!({"high_tick":f.tick,"high_generation":f.generation,"high_sequence":f.sequence,
            "next_event":f.next_event,"history_count":f.history_count,
            "group_next_position":f.group_next_position,"group_next_assignment":f.group_next_assignment,
            "civ_next_position":f.civ_next_position,"civ_next_assignment":f.civ_next_assignment,"fenced":f.fenced})
    }

    pub(super) fn retain_project_capture(&mut self, capture: &Capture) -> Result<()> {
        if self.config.parent_check.is_none() {
            return Ok(());
        }
        let next = capture_floor(capture);
        let previous = self.project_floor();
        self.project_fenced |=
            !dominates(next, previous) || self.config.binding.capture(capture).is_err();
        retain_floor(&mut self.handoff_floor, next);
        self.high_tick = self.high_tick.max(capture.tick);
        self.high_sequence = self.high_sequence.max(capture.sequence);
        if self.project_fenced {
            return Err(denied(
                "Manager project source or retained frontier changed",
            ));
        }
        Ok(())
    }

    /// A decoded source can be rejected after consuming its counters. Retain
    /// those counters before dropping the local connection or returning error.
    pub(super) fn retain_project_source(&mut self, source: &ManagerRpc, rejected: bool) {
        if self.config.parent_check.is_none() {
            return;
        }
        if let Some(mut next) = source.observed_floor() {
            next.fenced |= rejected
                || !dominates(next, self.project_floor())
                || next.generation != self.config.binding.generation;
            self.project_fenced |= next.fenced;
            self.high_tick = self.high_tick.max(next.tick);
            self.high_sequence = self.high_sequence.max(next.sequence);
            retain_floor(&mut self.handoff_floor, next);
        }
        if let Some(generation) = source.observed_generation() {
            let mut floor = self.project_floor();
            floor.generation = floor.generation.max(generation);
            floor.fenced |= generation != self.config.binding.generation;
            self.project_fenced |= floor.fenced;
            retain_floor(&mut self.handoff_floor, floor);
        }
    }

    pub(crate) fn project_recovery_reference(&self) -> Value {
        let mut reference = self.reference();
        if let Some(object) = reference.as_object_mut() {
            object.remove("original_review_seal");
        }
        reference["original"] = self.journal.original.as_ref().map(|o|json!({
            "key":o.plan.key(),"plan_digest":o.plan.digest.to_hex(),"deadline_tick":o.deadline_tick,
            "dispatch_started":self.journal.dispatch_started,"unresolved":self.journal.effect_uncertain,
            "native_phase":self.journal.native.as_ref().map(|record|record.phase),
            "currently_verified":false,
        })).unwrap_or(Value::Null);
        reference["project_source_frontier"] = self.project_frontier();
        reference["production_handoff_floor"] =
            production_handoff::floor_json(self.project_floor());
        reference["acknowledgement_currently_verified"] = json!(false);
        reference
    }

    pub(crate) fn retain_project_recovery(&mut self, reference: &Value) -> Result<()> {
        self.config.validate_project_recovery(reference)?;
        self.retain_project_recovery_state(reference)
    }

    fn retain_project_recovery_state(&mut self, reference: &Value) -> Result<()> {
        if reference.to_string().len() > 32 * 1024 {
            return Err(invalid("Manager project reference exceeds its bound"));
        }
        let raw = &reference["project_source_frontier"];
        if raw.to_string().len() > 1024 {
            return Err(invalid("Manager project frontier exceeds its bound"));
        }
        let frontier: Frontier = serde_json::from_value(raw.clone())
            .map_err(|_| invalid("invalid closed Manager project frontier"))?;
        let floor = frontier.floor();
        floor.validate()?;
        if floor.generation < self.config.binding.generation {
            return Err(denied(
                "Manager project frontier erased its pinned generation",
            ));
        }
        self.project_fenced |= floor.fenced || floor.generation != self.config.binding.generation;
        retain_floor(&mut self.handoff_floor, floor);
        self.high_tick = self.high_tick.max(floor.tick);
        self.high_sequence = self.high_sequence.max(floor.sequence);
        if let Some(saved) = reference
            .get("production_handoff_floor")
            .filter(|v| !v.is_null())
        {
            let retained: production_handoff::FloorInput = serde_json::from_value(saved.clone())
                .map_err(|_| invalid("invalid retained Manager export frontier"))?;
            retained.validate(self.config.binding.generation)?;
            let retained = retained.floor();
            retain_floor(&mut self.handoff_floor, retained);
            self.high_tick = self.high_tick.max(retained.tick);
            self.high_sequence = self.high_sequence.max(retained.sequence);
            self.project_fenced |=
                retained.fenced || retained.generation != self.config.binding.generation;
        }
        self.withdraw();
        if reference.get("original").is_some_and(Value::is_object)
            && self.journal.original.is_none()
        {
            return Err(denied("Manager recovery lost its parent-held original"));
        }
        if let Some(original) = self.journal.original.as_ref()
            && reference.get("original").is_some_and(Value::is_object)
            && (reference["original"]["key"] != original.plan.key()
                || reference["original"]["plan_digest"] != original.plan.digest.to_hex()
                || reference["original"]["deadline_tick"] != original.deadline_tick)
        {
            return Err(denied(
                "Manager recovery selected another original appointment",
            ));
        }
        self.validate_project_original_policy()
    }

    fn project_capture_view(&self, capture: &Capture, retained_by: &str) -> Value {
        let mut value = capture_json(capture);
        if let Some(object) = value.as_object_mut() {
            object.remove("native_capture_hex");
        }
        value["native_capture_commitment"] = json!({
            "schema":"dfmcp.manager-project-capture-commitment/1",
            "byte_length":capture.bytes().len(),"sha256":Digest32::of_bytes(capture.bytes()).to_hex(),
            "raw_bytes_inline":false,"retained_by":retained_by,
        });
        value["approval"] = json!({
            "risk":"irreversible","role":"MANAGER","unit_id":capture.selection.unit,
            "histfig_id":capture.selection.figure,"manager_position_id":capture.manager_position,
            "manager_position_index":capture.manager_index,"assignment_id":capture.target().map(|(_,a)|a.id),
            "assignment_index":capture.target().map(|(index,_)|index),
            "assignment_flags_hex":capture.target().map(|(_,a)|hex(&a.flags)),
            "effect_scope":self.config.input.scope,"selected_historical_figure_link":"POSITION",
            "history_event":"one_history_event_add_hf_entity_linkst",
            "production_responsibility":"MANAGE_PRODUCTION","clock_advanced":false,
            "lease_ticks":self.config.input.lease_ticks,
            "authority_expires_tick":self.config.input.authority_expires_tick,
            "checkpoint":self.config.input.checkpoint,"project_deadline_tick":self.config.project_deadline,
            "local_lease_global_fence":false,
        });
        value
    }

    fn project_view(&self, mut value: Value) -> Value {
        value["custody"] = self.project_recovery_reference();
        value["project_source_frontier"] = self.project_frontier();
        value["project_deadline_tick"] = json!(self.config.project_deadline);
        // The root deadline is immutable, but the native Manager offer still
        // has its own maximum horizon. Never suggest an impossible original.
        value["recommended_deadline_tick"] = json!(
            value["recommended_deadline_tick"]
                .as_u64()
                .zip(self.config.project_deadline)
                .map(|(native, root)| native.min(root))
        );
        if self.project_floor().fenced
            || self
                .config
                .project_deadline
                .is_some_and(|deadline| self.project_floor().tick >= deadline)
        {
            value["production_handoff_available"] = json!(false);
            value["recommended_deadline_tick"] = Value::Null;
        }
        if self.journal.original.is_none() {
            value["observation"] = self
                .observation
                .as_ref()
                .map(|c| self.project_capture_view(c, "current_manager_owner"))
                .unwrap_or(Value::Null);
        }
        if let Some(original) = self.journal.original.as_ref() {
            value["original"]["original_source"] =
                self.project_capture_view(&original.plan.before, "original_manager_journal");
            value["original"]["after"] = self
                .journal
                .native
                .as_ref()
                .and_then(|n| n.after.as_ref())
                .map(|c| self.project_capture_view(c, "original_manager_journal"))
                .unwrap_or(Value::Null);
        }
        value
    }

    pub(crate) fn perform_project(
        &mut self,
        action: Action,
        remaining: WorkBudget,
    ) -> Result<Value> {
        let start = Instant::now();
        let ceiling = self.budget;
        let result = (|| {
            self.config.current()?;
            self.validate_project_original_policy()?;
            self.budget = narrow(ceiling, remaining, start)?;
            if self.project_floor().fenced
                && matches!(
                    &action,
                    Action::Observe
                        | Action::Plan(_)
                        | Action::Commit(_, _)
                        | Action::Query(Some(_))
                )
            {
                return Err(denied(
                    "Manager project source continuity is fenced; retain and reconcile original custody",
                ));
            }
            let action = if matches!(action, Action::Cancel) && self.journal.original.is_none() {
                self.withdraw();
                Action::Query(None)
            } else {
                action
            };
            let value = self.perform(action)?;
            self.validate_project_original_policy()?;
            Ok(self.project_view(value))
        })();
        self.budget = ceiling;
        if let Err(cause) = &result {
            for (name, value) in &cause.details {
                if name == "consumed_manager_generation"
                    && let Ok(generation) = value.parse::<u64>()
                {
                    let mut floor = self.project_floor();
                    floor.generation = floor.generation.max(generation);
                    floor.fenced |= generation != self.config.binding.generation;
                    retain_floor(&mut self.handoff_floor, floor);
                    self.project_fenced |= floor.fenced;
                }
            }
            self.withdraw();
        }
        result
    }
}

#[cfg(all(test, target_os = "linux"))]
#[path = "project_tests.rs"]
mod tests;

#[derive(Clone, Debug)]
pub(crate) struct ManagerProjectHandoff {
    directory: PathBuf,
    binding: Binding,
    acknowledged: WorkshopPin,
    original_key: String,
    original_plan_digest: Digest32,
    deadline_tick: u64,
    origin_witness: Digest32,
    origin_descriptor: Vec<u8>,
    original_seal: Digest32,
    original_receipt: Digest32,
    historical_appointment_tick: u64,
    floor: ManagerProductionFloor,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceInput {
    endpoint: String,
    world_folder: String,
    site_id: u32,
    df_version: String,
    dfhack_version: String,
    generation: u64,
    map_dimensions: [u32; 3],
    unit_id: u32,
    histfig_id: u32,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HandoffInput {
    schema: String,
    directory: PathBuf,
    binding: SourceInput,
    acknowledged: Value,
    original_key: String,
    original_plan_digest: String,
    deadline_tick: u64,
    origin_witness: String,
    origin_descriptor_hex: String,
    original_review_seal: String,
    original_receipt: String,
    historical_appointment_tick: u64,
    floor: production_handoff::FloorInput,
}

impl ManagerProjectHandoff {
    pub(crate) fn directory(&self) -> &Path {
        &self.directory
    }
    pub(crate) fn project_deadline_tick(&self) -> u64 {
        self.deadline_tick
    }
    pub(crate) fn origin_witness(&self) -> Digest32 {
        self.origin_witness
    }
    pub(crate) fn high_tick(&self) -> u64 {
        self.floor.tick
    }

    pub(crate) fn to_json(&self) -> Value {
        let b = &self.binding;
        json!({"schema":HANDOFF_SCHEMA,"directory":self.directory,
            "binding":{"endpoint":b.endpoint.to_string(),"world_folder":b.source.folder(),"site_id":b.source.site(),
                "df_version":b.df_version,"dfhack_version":b.dfhack_version,"generation":b.generation,
                "map_dimensions":b.dimensions,"unit_id":b.selection.unit,"histfig_id":b.selection.figure},
            "acknowledged":pin_json(self.acknowledged),"original_key":self.original_key,
            "original_plan_digest":self.original_plan_digest.to_hex(),"deadline_tick":self.deadline_tick,
            "origin_witness":self.origin_witness.to_hex(),"origin_descriptor_hex":hex(&self.origin_descriptor),
            "original_review_seal":self.original_seal.to_hex(),"original_receipt":self.original_receipt.to_hex(),
            "historical_appointment_tick":self.historical_appointment_tick,"floor":production_handoff::floor_json(self.floor)})
    }

    /// Bounded historical parsing creates neither custody nor current readiness.
    /// The production owner must independently import the exact donor Offline.
    pub(crate) fn from_json(value: &Value) -> Result<Self> {
        if value.to_string().len() > HANDOFF_BYTES {
            return Err(invalid("Manager project handoff exceeds its bound"));
        }
        let input: HandoffInput = serde_json::from_value(value.clone())
            .map_err(|_| invalid("invalid closed Manager project handoff"))?;
        private_path(&input.directory)?;
        let floor = input.floor.floor();
        input.floor.validate(input.binding.generation)?;
        dfmcp_adapter::bounded_run::validate_key(&input.original_key)?;
        let d = input.binding.map_dimensions;
        if input.schema != HANDOFF_SCHEMA
            || d.iter().any(|n| !(1..=32768).contains(n))
            || input.deadline_tick == 0
            || input.deadline_tick > MAX_NATIVE_TICK
            || floor.tick >= input.deadline_tick
            || floor.fenced
            || floor.generation != input.binding.generation
            || input.historical_appointment_tick > floor.tick
            || input.origin_descriptor_hex.is_empty()
            || input.origin_descriptor_hex.len() > 2 * DESCRIPTOR_MAX_BYTES
            || input.origin_descriptor_hex.len() % 2 != 0
            || !input
                .origin_descriptor_hex
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(invalid(
                "invalid Manager handoff source, deadline or descriptor bounds",
            ));
        }
        let endpoint = dfmcp_adapter::parse_loopback_endpoint(&input.binding.endpoint)?;
        if endpoint.to_string() != input.binding.endpoint {
            return Err(invalid("noncanonical Manager handoff endpoint"));
        }
        let binding = Binding {
            protocol: ManagerProtocol::V1_23,
            endpoint,
            source: FortressIdentity::new(&input.binding.world_folder, input.binding.site_id)?,
            generation: input.binding.generation,
            dimensions: d,
            scope: area([0, 0, 0, d[0] - 1, d[1] - 1, d[2] - 1], d)?,
            selection: Selection::new(input.binding.unit_id, input.binding.histfig_id)?,
            df_version: input.binding.df_version,
            dfhack_version: input.binding.dfhack_version,
        };
        binding.validate()?;
        let acknowledged = parse_pin(&input.acknowledged, false)?;
        let original_plan_digest = digest(&input.original_plan_digest)?;
        let origin_witness = digest(&input.origin_witness)?;
        let original_seal = digest(&input.original_review_seal)?;
        let original_receipt = digest(&input.original_receipt)?;
        let origin_descriptor: Vec<u8> = input
            .origin_descriptor_hex
            .as_bytes()
            .chunks_exact(2)
            .map(|pair| {
                let nibble = |b: u8| {
                    if b.is_ascii_digit() {
                        b - b'0'
                    } else {
                        b - b'a' + 10
                    }
                };
                nibble(pair[0]) * 16 + nibble(pair[1])
            })
            .collect();
        if [
            original_plan_digest,
            origin_witness,
            original_seal,
            original_receipt,
        ]
        .contains(&Digest32::ZERO)
            || Digest32::of_bytes(&origin_descriptor) != origin_witness
        {
            return Err(invalid("Manager handoff lost its exact immutable origin"));
        }
        Ok(Self {
            directory: input.directory,
            binding,
            acknowledged,
            original_key: input.original_key,
            original_plan_digest,
            deadline_tick: input.deadline_tick,
            origin_witness,
            origin_descriptor,
            original_seal,
            original_receipt,
            historical_appointment_tick: input.historical_appointment_tick,
            floor,
        })
    }

    pub(crate) fn production_configuration(&self) -> Value {
        let b = &self.binding;
        let mut checkpoint = production_handoff::floor_json(self.floor);
        checkpoint["origin_witness"] = json!(self.origin_witness.to_hex());
        json!({"manager_prerequisite":{"directory":self.directory,"endpoint":b.endpoint.to_string(),
            "world_folder":b.source.folder(),"site_id":b.source.site(),"df_version":b.df_version,
            "dfhack_version":b.dfhack_version,"generation":b.generation,"map_dimensions":b.dimensions,
            "unit_id":b.selection.unit,"histfig_id":b.selection.figure,"acknowledged":pin_json(self.acknowledged),
            "original_key":self.original_key,"original_plan_digest":self.original_plan_digest.to_hex()},
            "manager_prerequisite_checkpoint":checkpoint})
    }

    pub(crate) fn validate_project_checkpoint(&self, reference: &Value) -> Result<()> {
        if reference.to_string().len() > 32 * 1024 {
            return Err(invalid("Manager project checkpoint exceeds its bound"));
        }
        let source = &reference["source_identity"];
        let original = &reference["original"];
        let b = &self.binding;
        if parse_pin(&reference["acknowledged"], false)? != self.acknowledged
            || parse_pin(&reference["journal_pin_unverified"], false)? != self.acknowledged
            || original["key"] != self.original_key
            || original["plan_digest"] != self.original_plan_digest.to_hex()
            || original["deadline_tick"] != self.deadline_tick
            || reference["original_key"] != self.original_key
            || reference["original_plan_digest"] != self.original_plan_digest.to_hex()
            || reference["native_phase"] != 2
            || original["native_phase"] != 2
            || original["dispatch_started"] != true
            || original["unresolved"] != false
            || reference["dispatch_started"] != true
            || reference["uncertain_effect"] != false
            || source["protocol"] != "1.23"
            || source["endpoint"] != b.endpoint.to_string()
            || source["world_folder"] != b.source.folder()
            || source["site_id"] != b.source.site()
            || source["generation"] != b.generation
            || source["df_version"] != b.df_version
            || source["dfhack_version"] != b.dfhack_version
            || source["unit_id"] != b.selection.unit
            || source["histfig_id"] != b.selection.figure
        {
            return Err(denied(
                "Manager handoff changed its acknowledged parent original",
            ));
        }
        let frontier: Frontier =
            serde_json::from_value(reference["project_source_frontier"].clone())
                .map_err(|_| invalid("Manager handoff checkpoint lost its source frontier"))?;
        let floor = frontier.floor();
        floor.validate()?;
        let retained: production_handoff::FloorInput =
            serde_json::from_value(reference["production_handoff_floor"].clone())
                .map_err(|_| invalid("Manager handoff checkpoint lost its export frontier"))?;
        retained.validate(b.generation)?;
        if floor.fenced
            || retained.floor().fenced
            || floor.generation != b.generation
            || !dominates(floor, self.floor)
            || !dominates(retained.floor(), self.floor)
            || reference["consumed_tick_floor"]
                .as_u64()
                .is_none_or(|tick| tick < self.floor.tick)
        {
            return Err(denied(
                "Manager handoff checkpoint regressed or fenced its exported source",
            ));
        }
        Ok(())
    }

    pub(crate) fn verify_origin(&self, origin: &ManagerProductionOrigin) -> Result<()> {
        if origin.binding() != &self.binding
            || origin.pin() != self.acknowledged
            || origin.plan().key() != self.original_key
            || origin.plan().digest != self.original_plan_digest
            || origin.original_deadline_tick() != self.deadline_tick
            || origin.witness() != self.origin_witness
            || origin.canonical_bytes() != self.origin_descriptor
            || origin.original_seal() != self.original_seal
            || origin.receipt() != self.original_receipt
            || origin
                .historical_record()
                .after
                .as_ref()
                .map(|capture| capture.tick)
                != Some(self.historical_appointment_tick)
            || !dominates(self.floor, origin.floor())
            || self.floor.fenced
            || origin.is_fenced()
        {
            return Err(denied(
                "Manager project handoff differs from its independently verified original custody",
            ));
        }
        Ok(())
    }
}

impl Runtime {
    pub(crate) fn project_handoff(
        &mut self,
        remaining: WorkBudget,
    ) -> Result<ManagerProjectHandoff> {
        let start = Instant::now();
        let ceiling = self.budget;
        let result = (|| {
            self.config.current()?;
            self.validate_project_original_policy()?;
            self.budget = narrow(ceiling, remaining, start)?;
            if self.mode == Mode::Offline
                || self.project_floor().fenced
                || self.journal.dispatchable()
            {
                return Err(denied(
                    "Manager transfer requires a continuous resolved read-enabled original",
                ));
            }
            self.config
                .project_effect_deadline(self.project_floor().tick)?;
            let mut current = self.current()?;
            let mut work = TurnWork::new(&current, start)?;
            self.guard(&work.reserve(&current, 32 * 1024 * 1024)?, false)?;
            // The existing exporter independently validates canonical journal,
            // ACK and current Manager/cache through one read-only connection.
            let exported = self.acquire_production_handoff(&current, &mut work)?;
            current.anchor.tick = GameTick(self.high_tick.max(self.journal.high_tick));
            self.config
                .project_effect_deadline(current.anchor.tick.get())?;
            self.guard(&work.reserve(&current, 32 * 1024 * 1024)?, false)?;
            let original = self
                .journal
                .original
                .as_ref()
                .ok_or_else(|| denied("original Manager appointment unavailable"))?;
            let historical = self
                .journal
                .native
                .as_ref()
                .filter(|record| record.complete())
                .and_then(|record| record.after.as_ref())
                .ok_or_else(|| {
                    denied("Manager appointment has no complete historical poststate")
                })?;
            let handoff = ManagerProjectHandoff {
                directory: self.config.input.directory.clone(),
                binding: exported.binding().clone(),
                acknowledged: exported.pin(),
                original_key: exported.plan().key().to_owned(),
                original_plan_digest: exported.plan().digest,
                deadline_tick: original.deadline_tick,
                origin_witness: exported.origin_witness(),
                origin_descriptor: exported.origin_canonical_bytes().to_vec(),
                original_seal: exported.original_seal(),
                original_receipt: exported.receipt(),
                historical_appointment_tick: historical.tick,
                floor: exported.floor(),
            };
            ManagerProjectHandoff::from_json(&handoff.to_json())?;
            if handoff.acknowledged != self.journal.pin()
                || handoff.original_seal != original.seal()
                || handoff.high_tick() != current.anchor.tick.get()
                || handoff.project_deadline_tick()
                    != self
                        .config
                        .project_deadline
                        .ok_or_else(|| denied("Manager project deadline unavailable"))?
            {
                return Err(denied("Manager project changed during current handoff"));
            }
            self.validate_project_original_policy()?;
            self.config.current()?;
            work.reserve(&current, 0)?;
            Ok(handoff)
        })();
        self.budget = ceiling;
        if result.is_err() {
            self.withdraw();
        }
        result
    }
}
