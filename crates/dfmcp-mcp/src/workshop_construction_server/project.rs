//! Parent-owned composition of the existing Workshop runtime. These methods do
//! not relax standalone profile isolation or turn a serialized handoff into
//! authority. Production must independently reopen and verify Offline custody.
use super::*;
use dfmcp_adapter::workshop_construction::{
    WorkshopProgress, WorkshopRecord,
    production_origin::{IMPORT_WORK_BYTES, WorkshopHandoffCheckpoint},
};
use std::path::Path;
use std::time::Duration;

const HANDOFF_SCHEMA: &str = "dfmcp.workshop-project-handoff/1";
const HANDOFF_BYTES: usize = 32 * 1024;

#[derive(Default)]
pub(super) struct State {
    high_generation: u64,
    high_building: u32,
    fenced: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FrontierInput {
    high_tick: u64,
    high_generation: u64,
    high_next_building_id: u32,
    fenced: bool,
}
impl State {
    pub(super) fn fence(&mut self) {
        self.fenced = true;
    }
    pub(super) fn reference(&self, high_tick: u64) -> Value {
        json!({"high_tick":high_tick,"high_generation":self.high_generation,
            "high_next_building_id":self.high_building,"fenced":self.fenced})
    }

    fn retain(
        &mut self,
        current_tick: u64,
        frontier: FrontierInput,
        binding_generation: u64,
    ) -> Result<u64> {
        if frontier.high_tick > dfmcp_adapter::bounded_run::MAX_NATIVE_TICK
            || frontier.high_generation == u64::MAX
            || frontier.high_next_building_id > i32::MAX as u32
        {
            return Err(invalid("invalid workshop project source frontier"));
        }
        // A surviving verified child descendant may already be ahead of the
        // parent's last reply. Merge rejection floors; never replace them.
        self.high_generation = self.high_generation.max(frontier.high_generation);
        self.high_building = self.high_building.max(frontier.high_next_building_id);
        self.fenced |= frontier.fenced
            || (self.high_generation != 0 && self.high_generation != binding_generation);
        Ok(current_tick.max(frontier.high_tick))
    }
}

impl Config {
    pub(crate) fn from_resource_project_environment(
        parent_check: Arc<dyn Fn() -> Result<()> + Send + Sync>,
    ) -> Result<Self> {
        runtime_io()?;
        parent_check()?;
        validate_family_environment()?;
        let mut config = Self::parse(
            &std::env::var(CONFIG)
                .map_err(|_| denied("operator workshop configuration is missing"))?,
        )?;
        config.parent_check = Some(parent_check);
        config.current()?;
        Ok(config)
    }

    pub(crate) fn with_project_deadline(mut self, deadline: u64) -> Result<Self> {
        if self.parent_check.is_none()
            || deadline == 0
            || deadline > dfmcp_adapter::bounded_run::MAX_NATIVE_TICK
            || deadline > self.input.authority_expires_tick
            || self.project_deadline.is_some_and(|old| old != deadline)
            || self.clock_deadline().is_some_and(|clock| clock != deadline)
        {
            return Err(invalid(
                "project deadline must be immutable, inside host authority and equal the configured clock horizon",
            ));
        }
        self.project_deadline = Some(deadline);
        Ok(self)
    }

    fn clock_deadline(&self) -> Option<u64> {
        self.policy.get("clock")?.get("deadline_tick")?.as_u64()
    }

    pub(super) fn project_effect_deadline(&self, tick: u64) -> Result<()> {
        if self
            .project_deadline
            .is_some_and(|deadline| tick >= deadline)
        {
            return Err(denied("original project deadline has elapsed"));
        }
        Ok(())
    }

    /// Stable policy identity. Startup controls are independently authenticated
    /// by exact child journals/ACKs and retained parent recovery checkpoints.
    pub(crate) fn project_identity(&self) -> Digest32 {
        let mut policy = self.policy.clone();
        if let Some(object) = policy.as_object_mut() {
            for name in ["initialize", "acknowledged", "recovery_generation"] {
                object.remove(name);
            }
            if let Some(clock) = object.get_mut("clock").and_then(Value::as_object_mut) {
                for name in ["initialize", "recovery", "recovery_generation"] {
                    clock.remove(name);
                }
            }
        }
        let identity = json!({"schema":"dfmcp.workshop-project-policy/1",
            "policy":policy,"project_deadline_tick":self.project_deadline});
        Digest32::of_bytes(identity.to_string().as_bytes())
    }

    /// Accepted host pins only; this is not an observation or native admission.
    pub(crate) fn project_binding(&self) -> Result<WorkshopBinding> {
        WorkshopBinding::from_source_pins(
            self.endpoint,
            self.identity.clone(),
            self.input.generation,
            self.input.map_dimensions,
            &self.input.df_version,
            &self.input.dfhack_version,
        )
    }
    pub(crate) fn project_directory(&self) -> &Path {
        &self.input.directory
    }
    /// The stable policy omits restart selectors only because the exact parent
    /// checkpoint binds them before any child open. Actual child replay then
    /// verifies custody and accepts only its authenticated descendants.
    pub(crate) fn validate_project_recovery(&self, reference: &Value) -> Result<()> {
        self.current()?;
        self.project_recovery_pins(reference)
    }

    fn project_recovery_pins(&self, reference: &Value) -> Result<()> {
        if self.parent_check.is_none() || self.project_deadline.is_none() || self.input.initialize {
            return Err(denied(
                "project recovery requires an existing original Workshop child",
            ));
        }
        let retained: PinInput =
            serde_json::from_value(reference.get("acknowledged").cloned().ok_or_else(|| {
                denied("parent checkpoint has no independent Workshop acknowledgement")
            })?)
            .map_err(|_| denied("parent Workshop acknowledgement is missing or invalid"))?;
        if self.pin != Some(retained.decode()?) {
            return Err(denied(
                "Workshop restart acknowledgement differs from the parent checkpoint",
            ));
        }
        match (
            self.clock.is_some(),
            reference.get("clock").filter(|clock| !clock.is_null()),
        ) {
            (false, None) => {}
            (true, Some(clock)) => {
                let configured = &self.policy["clock"];
                let retained = clock
                    .get("recovery")
                    .filter(|pin| pin.is_object())
                    .ok_or_else(|| {
                        denied("parent checkpoint lost the original Workshop clock pin")
                    })?;
                if configured["initialize"] != false
                    || configured["recovery"].is_null()
                    || &configured["recovery"] != retained
                {
                    return Err(denied(
                        "Workshop clock restart pin differs from the parent checkpoint",
                    ));
                }
            }
            _ => {
                return Err(denied(
                    "Workshop clock custody differs from the parent checkpoint",
                ));
            }
        }
        Ok(())
    }

    pub(crate) fn project_initializing(&self) -> bool {
        self.input.initialize
    }
    pub(crate) fn project_clock_configured(&self) -> bool {
        self.clock.is_some()
    }

    /// The parent uses this context to retain its start intent before opening
    /// either child. Original Query/Observe and clock safety grants keep their
    /// host expiry; only positive construction edges use the project deadline.
    pub(crate) fn project_context(
        &self,
        mode: WorkshopMode,
        session: SessionId,
        request: RequestId,
        budget: WorkBudget,
    ) -> Result<OperationContext> {
        self.current()?;
        budget.validate()?;
        if self.parent_check.is_none() || self.project_deadline.is_none() {
            return Err(denied(
                "workshop project context requires its original parent policy",
            ));
        }
        let control = mode == WorkshopMode::Control
            && self.input.initialize
            && std::env::var(BUILD).ok().as_deref() == Some("1");
        let mut grants = Runtime::grants(
            self,
            control,
            self.clock.as_ref().is_some_and(|clock| clock.control(mode)),
        );
        // Parent Control may retain its own plan/transition ledger during a
        // recovered child session. This context is never used for child native
        // dispatch, and recovering the child still cannot restore Construct.
        if mode == WorkshopMode::Control
            && !grants
                .iter()
                .any(|grant| grant.capability == Capability::Plan)
        {
            grants.push(CapabilityGrant {
                capability: Capability::Plan,
                scope: CapabilityScope {
                    fortress_id: Some(self.identity.fortress_id()),
                    ..CapabilityScope::default()
                },
                max_risk: RiskTier::Guarded,
                expires_at_tick: Some(GameTick(self.input.authority_expires_tick)),
                remaining_uses: None,
            });
        }
        Ok(OperationContext {
            session_id: session,
            request_id: request,
            anchor: StateAnchor {
                fortress_id: self.identity.fortress_id(),
                cursor: ObservationCursor::ORIGIN,
                tick: GameTick(0),
                state_hash: Digest32::ZERO,
            },
            grants,
            budget,
            cancellation_requested: false,
        })
    }
}

fn narrow(ceiling: WorkBudget, remaining: WorkBudget) -> Result<WorkBudget> {
    let budget = WorkBudget {
        max_wall_millis: ceiling.max_wall_millis.min(remaining.max_wall_millis),
        max_game_ticks: ceiling.max_game_ticks.min(remaining.max_game_ticks),
        max_entities: ceiling.max_entities.min(remaining.max_entities),
        max_bytes: ceiling.max_bytes.min(remaining.max_bytes),
        max_output_tokens: ceiling.max_output_tokens.min(remaining.max_output_tokens),
        max_actions: ceiling.max_actions.min(remaining.max_actions),
    };
    budget.validate()?;
    Ok(budget)
}

fn narrow_current(
    ceiling: WorkBudget,
    remaining: WorkBudget,
    started: Instant,
) -> Result<WorkBudget> {
    let mut budget = narrow(ceiling, remaining)?;
    budget.max_wall_millis = budget
        .max_wall_millis
        .checked_sub(started.elapsed().as_millis() as u64)
        .filter(|left| *left > 0)
        .ok_or_else(exhausted)?;
    Ok(budget)
}

/// Historical, immutable transition data. Its fresh read was qualified only at
/// export time. Deserialization supplies no current proof, permit or new source
/// authority; the production owner must independently import and refresh it.
#[derive(Clone, Debug)]
pub(crate) struct WorkshopProjectHandoff {
    directory: PathBuf,
    binding: WorkshopBinding,
    acknowledged: WorkshopPin,
    record: WorkshopRecord,
    progress: WorkshopProgress,
    origin_witness: Digest32,
    origin_high_tick: u64,
    origin_high_building: u32,
    high_tick: u64,
    high_generation: u64,
    high_building: u32,
    project_deadline_tick: u64,
    authority_expires_tick: u64,
    clock_deadline_tick: Option<u64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HandoffInput {
    schema: String,
    directory: PathBuf,
    binding_hex: String,
    acknowledged: PinInput,
    record_hex: String,
    progress_hex: String,
    origin_witness: String,
    origin_high_tick: u64,
    origin_high_next_building_id: u32,
    high_tick: u64,
    high_generation: u64,
    high_next_building_id: u32,
    project_deadline_tick: u64,
    authority_expires_tick: u64,
    clock_deadline_tick: Option<u64>,
}

fn bytes(raw: &str, limit: usize) -> Result<Vec<u8>> {
    if raw.is_empty() || raw.len() > 2 * limit || raw.len() % 2 != 0 {
        return Err(invalid("bounded canonical handoff bytes required"));
    }
    raw.as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let digit = |b: u8| match b {
                b'0'..=b'9' => Ok(b - b'0'),
                b'a'..=b'f' => Ok(b - b'a' + 10),
                _ => Err(invalid("handoff bytes require lowercase hexadecimal")),
            };
            Ok(digit(pair[0])? * 16 + digit(pair[1])?)
        })
        .collect()
}

impl WorkshopProjectHandoff {
    pub(crate) fn directory(&self) -> &Path {
        &self.directory
    }
    pub(crate) fn binding(&self) -> &WorkshopBinding {
        &self.binding
    }
    pub(crate) fn acknowledged(&self) -> WorkshopPin {
        self.acknowledged
    }
    pub(crate) fn original_key(&self) -> &str {
        self.record.plan().key()
    }
    pub(crate) fn original_plan_digest(&self) -> Digest32 {
        self.record.plan().digest()
    }
    pub(crate) fn origin_witness(&self) -> Digest32 {
        self.origin_witness
    }
    pub(crate) fn high_tick(&self) -> u64 {
        self.high_tick
    }
    pub(crate) fn high_generation(&self) -> u64 {
        self.high_generation
    }
    pub(crate) fn high_next_building_id(&self) -> u32 {
        self.high_building
    }
    pub(crate) fn project_deadline_tick(&self) -> u64 {
        self.project_deadline_tick
    }
    pub(crate) fn authority_expires_tick(&self) -> u64 {
        self.authority_expires_tick
    }
    pub(crate) fn clock_deadline_tick(&self) -> Option<u64> {
        self.clock_deadline_tick
    }

    pub(crate) fn to_json(&self) -> Value {
        json!({"schema":HANDOFF_SCHEMA,"directory":self.directory,
            "binding_hex":hex(&self.binding.encode()),"acknowledged":pin_json(self.acknowledged),
            "record_hex":hex(self.record.canonical_bytes()),"progress_hex":hex(self.progress.canonical_bytes()),
            "origin_witness":self.origin_witness.to_hex(),"origin_high_tick":self.origin_high_tick,
            "origin_high_next_building_id":self.origin_high_building,"high_tick":self.high_tick,
            "high_generation":self.high_generation,"high_next_building_id":self.high_building,
            "project_deadline_tick":self.project_deadline_tick,"authority_expires_tick":self.authority_expires_tick,
            "clock_deadline_tick":self.clock_deadline_tick})
    }

    pub(crate) fn from_json(value: &Value) -> Result<Self> {
        if value.to_string().len() > HANDOFF_BYTES {
            return Err(invalid("workshop project handoff exceeds 32 KiB"));
        }
        let input: HandoffInput = serde_json::from_value(value.clone())
            .map_err(|_| invalid("invalid closed workshop project handoff"))?;
        if input.schema != HANDOFF_SCHEMA {
            return Err(invalid("unsupported workshop project handoff"));
        }
        let handoff = Self {
            directory: input.directory,
            binding: WorkshopBinding::decode(&bytes(&input.binding_hex, 1024)?)?,
            acknowledged: input.acknowledged.decode()?,
            record: WorkshopRecord::decode(&bytes(&input.record_hex, 6144)?)?,
            progress: WorkshopProgress::decode(&bytes(&input.progress_hex, 1024)?)?,
            origin_witness: digest(&input.origin_witness)?,
            origin_high_tick: input.origin_high_tick,
            origin_high_building: input.origin_high_next_building_id,
            high_tick: input.high_tick,
            high_generation: input.high_generation,
            high_building: input.high_next_building_id,
            project_deadline_tick: input.project_deadline_tick,
            authority_expires_tick: input.authority_expires_tick,
            clock_deadline_tick: input.clock_deadline_tick,
        };
        handoff.validate()?;
        Ok(handoff)
    }

    fn validate(&self) -> Result<()> {
        let path = self
            .directory
            .to_str()
            .ok_or_else(|| invalid("handoff path is not UTF-8"))?;
        if !self.directory.is_absolute()
            || !(2..=3800).contains(&path.len())
            || path[1..]
                .split('/')
                .any(|p| p.is_empty() || p == "." || p == ".." || p.chars().any(char::is_control))
            || self.acknowledged.frames == 0
            || self.acknowledged.frames > dfmcp_adapter::workshop_construction::journal::MAX_FRAMES
            || !self.binding.capture_matches(self.record.plan().before())
            || WorkshopGoal::evaluate(&self.record, &self.progress) != WorkshopGoal::Complete
            || self.origin_high_tick < self.record.after().map_or(u64::MAX, WorkshopCapture::tick)
            || self.origin_high_building
                < self
                    .record
                    .after()
                    .map_or(u32::MAX, WorkshopCapture::next_building_id)
            || self.high_tick < self.origin_high_tick.max(self.progress.tick())
            || self.high_tick > dfmcp_adapter::bounded_run::MAX_NATIVE_TICK
            || self.high_generation != self.binding.generation()
            || self.high_building
                < self
                    .origin_high_building
                    .max(self.progress.next_building_id())
            || self.high_building > i32::MAX as u32
            || self.project_deadline_tick <= self.high_tick
            || self.project_deadline_tick > self.authority_expires_tick
            || self.project_deadline_tick > dfmcp_adapter::bounded_run::MAX_NATIVE_TICK
            || self
                .clock_deadline_tick
                .is_some_and(|clock| clock != self.project_deadline_tick)
        {
            return Err(invalid(
                "workshop project handoff lost original source, completion, deadline or floors",
            ));
        }
        // This is the existing adapter origin identity, not a new authority
        // format. An independently reopened importer must match it exactly.
        let mut canonical = b"dfmcp-workshop-production-origin/1\0".to_vec();
        let binding = self.binding.encode();
        canonical.extend_from_slice(&(binding.len() as u32).to_be_bytes());
        canonical.extend_from_slice(&binding);
        canonical.extend_from_slice(self.acknowledged.journal_id.as_bytes());
        canonical.extend_from_slice(self.acknowledged.head.as_bytes());
        canonical.extend_from_slice(&self.acknowledged.frames.to_be_bytes());
        let record = self.record.canonical_bytes();
        canonical.extend_from_slice(&(record.len() as u32).to_be_bytes());
        canonical.extend_from_slice(record);
        canonical.extend_from_slice(&self.origin_high_tick.to_be_bytes());
        canonical.extend_from_slice(&self.origin_high_building.to_be_bytes());
        if Digest32::of_bytes(&canonical) != self.origin_witness {
            return Err(invalid(
                "workshop project handoff changed its acknowledged origin identity",
            ));
        }
        Ok(())
    }
}

impl Runtime {
    pub(crate) fn project_has_original_history(&self) -> bool {
        self.journal.pin().frames != 0
            || self
                .clock
                .as_ref()
                .is_some_and(clock::Owner::project_has_history)
    }
    /// Historical recovery selectors even after the final independent ACK
    /// write or authority check failed. No field here certifies current custody.
    pub(crate) fn project_recovery_reference(&self) -> Value {
        let mut reference = self.failure_reference();
        reference["acknowledged"] = self
            .guard
            .acks
            .try_lock()
            .ok()
            .and_then(|acknowledgement| acknowledgement.latest())
            .map(pin_json)
            .unwrap_or(Value::Null);
        reference["acknowledgement_currently_verified"] = json!(false);
        let high_tick = self
            .authority_tick
            .max(self.journal.high_tick())
            .max(self.clock.as_ref().map_or(0, clock::Owner::high_tick));
        reference["project_source_frontier"] = self.project.reference(high_tick);
        if let Some(original) = reference.get_mut("original").and_then(Value::as_object_mut) {
            original.remove("review_seal");
        }
        reference
    }

    /// Restore all rejection state after reopening locally verified child
    /// custody. The Config pin check must also run before that open.
    pub(crate) fn retain_project_recovery(&mut self, reference: &Value) -> Result<()> {
        self.config.validate_project_recovery(reference)?;
        self.retain_project_frontier(
            reference
                .get("project_source_frontier")
                .ok_or_else(|| denied("parent checkpoint lost the Workshop source frontier"))?,
        )?;
        if let Some(owner) = &mut self.clock {
            let fenced = reference["clock"]["advancement_fenced"]
                .as_bool()
                .ok_or_else(|| denied("parent checkpoint lost the Workshop clock fence"))?;
            if fenced {
                owner.fence_advancement();
            }
        }
        self.withdraw();
        Ok(())
    }

    /// Restore rejection floors from the parent's exact immutable child
    /// checkpoint. The caller must verify its ledger/root binding first. This
    /// method performs no native contact, creates no permit, and only narrows.
    pub(crate) fn retain_project_frontier(&mut self, value: &Value) -> Result<()> {
        self.config.current()?;
        if self.config.parent_check.is_none() || self.config.project_deadline.is_none() {
            return Err(denied(
                "workshop frontier requires its original parent policy",
            ));
        }
        if value.to_string().len() > 512 {
            return Err(invalid("workshop project frontier exceeds its bound"));
        }
        let frontier: FrontierInput = serde_json::from_value(value.clone())
            .map_err(|_| invalid("invalid closed workshop project frontier"))?;
        let current_tick = self
            .authority_tick
            .max(self.journal.high_tick())
            .max(self.clock.as_ref().map_or(0, clock::Owner::high_tick));
        self.authority_tick =
            self.project
                .retain(current_tick, frontier, self.config.input.generation)?;
        self.withdraw();
        Ok(())
    }

    pub(crate) fn perform_project(
        &mut self,
        action: Action,
        remaining: WorkBudget,
    ) -> Result<Value> {
        let started = Instant::now();
        let ceiling = self.budget;
        let result = (|| {
            if self.config.parent_check.is_none() || self.config.project_deadline.is_none() {
                return Err(denied("workshop child is not bound to a parent project"));
            }
            self.budget = narrow_current(ceiling, remaining, started)?;
            if self.project.fenced
                && matches!(
                    &action,
                    Action::Observe | Action::Plan(_) | Action::Commit { .. }
                )
            {
                return Err(denied(
                    "workshop project source is fenced; preserve original recovery custody",
                ));
            }
            let context = self.context()?;
            let mut view = self.perform(action, &context)?;
            view["project_source_frontier"] = self.project.reference(self.authority_tick);
            Ok(view)
        })();
        self.budget = ceiling;
        if result.is_err() {
            self.withdraw();
        }
        result
    }

    fn project_checkpoint(
        &mut self,
        work: &mut Work,
        key: &str,
        digest: Digest32,
    ) -> Result<WorkshopHandoffCheckpoint> {
        self.config.current()?;
        let current = work.reserve(IMPORT_WORK_BYTES)?;
        let mut acknowledgement = self
            .guard
            .acks
            .try_lock()
            .map_err(|_| denied("independent workshop acknowledgement busy or poisoned"))?;
        self.journal
            .handoff_checkpoint(&mut *acknowledgement, key, digest, &current)
    }

    /// Reuse the same typed settled-clock export as the public historical
    /// handoff. It additionally rejects a previous unknown native clock read's
    /// advancement fence; JSON presentation flags never authorize transition.
    fn project_clock_settled(&mut self, work: &mut Work) -> Result<()> {
        if self.clock.is_none() {
            return Ok(());
        }
        let inventory = self.journal.inventory(&work.reserve(INSPECT_BYTES)?)?;
        let current = work.reserve(clock::HANDOFF_BYTES)?;
        let owner = self
            .clock
            .as_mut()
            .ok_or_else(|| denied("original project clock owner is unavailable"))?;
        let result = owner.production_handoff(self.journal.binding(), &inventory, &current);
        self.authority_tick = self.authority_tick.max(owner.high_tick());
        work.advance_authorization_tick(self.authority_tick);
        result.map(|_| ())
    }

    pub(crate) fn project_handoff(
        &mut self,
        remaining: WorkBudget,
    ) -> Result<WorkshopProjectHandoff> {
        let started = Instant::now();
        let ceiling = self.budget;
        let result = (|| {
            self.config.current()?;
            let deadline = self
                .config
                .project_deadline
                .filter(|_| self.config.parent_check.is_some())
                .ok_or_else(|| denied("workshop handoff requires its original parent policy"))?;
            if self.mode == WorkshopMode::Offline || self.project.fenced {
                return Err(denied(
                    "workshop handoff requires an unfenced live original owner",
                ));
            }
            self.budget = narrow_current(ceiling, remaining, started)?;
            let context = self.context()?;
            let mut work = Work::new(&context, self.config.view_bytes())?;
            self.config
                .project_effect_deadline(work.context.anchor.tick.get())?;
            self.project_clock_settled(&mut work)?;
            let inventory = self.journal.inventory(&work.reserve(INSPECT_BYTES)?)?;
            if inventory.entries().len() != 1 {
                return Err(denied(
                    "workshop handoff needs exactly one retained original",
                ));
            }
            let entry = &inventory.entries()[0];
            let key = entry.plan().key().to_owned();
            let digest = entry.plan().digest();
            if self.journal.has_permit(&key) {
                return Err(denied("workshop preparation remains unresolved"));
            }
            let checkpoint = self.project_checkpoint(&mut work, &key, digest)?;
            self.authority_tick = self.authority_tick.max(checkpoint.high_tick());
            self.project.high_generation = self
                .project
                .high_generation
                .max(checkpoint.high_generation());
            self.project.high_building = self
                .project
                .high_building
                .max(checkpoint.high_next_building_id());
            work.advance_authorization_tick(self.authority_tick);
            self.config.binding(checkpoint.binding())?;
            self.config.capture(checkpoint.record().plan().before())?;
            self.source = None;
            self.observation = None;
            let connect = work.reserve(CONNECT_BYTES)?;
            let mut source = match self.config.source(&connect, WorkshopMode::Recover) {
                Ok(source) => source,
                Err(cause) => {
                    self.project.fenced = true;
                    return Err(cause);
                }
            };
            let manifest = source.manifest();
            self.project.high_generation = self.project.high_generation.max(manifest.generation);
            if manifest.generation != checkpoint.binding().generation() {
                self.project.fenced = true;
                source.fence();
                return Err(denied(
                    "workshop handoff lost its original native incarnation",
                ));
            }
            source.require_progress_at_or_after(self.authority_tick);
            let insertion = checkpoint
                .record()
                .insertion()
                .ok_or_else(|| denied("original workshop insertion is absent"))?;
            let current = work.reserve(CONNECT_BYTES)?;
            let progress = source.progress(
                insertion.building_id(),
                insertion.item_id(),
                &current,
                Duration::from_millis(current.budget.max_wall_millis),
            );
            // A reply can consume another incarnation before any subsequent
            // authority, custody or budget check. Retain that manifest even
            // when progress decoding failed, and never erase a decoded frontier.
            let manifest = source.manifest();
            self.project.high_generation = self.project.high_generation.max(manifest.generation);
            let binding = checkpoint.binding();
            let source_changed = manifest.endpoint != binding.endpoint()
                || &manifest.fortress != binding.fortress()
                || manifest.generation != binding.generation()
                || manifest.df_version != binding.df_version()
                || manifest.dfhack_version != binding.dfhack_version();
            source.fence();
            let progress = match progress {
                Ok(progress) => progress,
                Err(cause) => {
                    self.project.fenced = true;
                    return Err(cause);
                }
            };
            let regressed = progress.tick() < self.authority_tick
                || progress.generation() != self.project.high_generation
                || progress.next_building_id() < self.project.high_building;
            self.authority_tick = self.authority_tick.max(progress.tick());
            self.project.high_generation = self.project.high_generation.max(progress.generation());
            self.project.high_building =
                self.project.high_building.max(progress.next_building_id());
            work.advance_authorization_tick(self.authority_tick);
            let goal = WorkshopGoal::evaluate(checkpoint.record(), &progress);
            if source_changed
                || regressed
                || !matches!(
                    goal,
                    WorkshopGoal::Complete | WorkshopGoal::InProgress { .. }
                )
            {
                self.project.fenced = true;
                return Err(denied(
                    "workshop handoff source, allocation or original identity changed",
                ));
            }
            let after = self.project_checkpoint(&mut work, &key, digest)?;
            if checkpoint.canonical_bytes() != after.canonical_bytes() {
                self.project.fenced = true;
                return Err(denied(
                    "workshop handoff custody changed during its native read",
                ));
            }
            self.project_clock_settled(&mut work)?;
            self.config.current()?;
            if progress.tick() != self.authority_tick {
                return Err(denied(
                    "workshop completion precedes the settled source floor",
                ));
            }
            self.config.project_effect_deadline(self.authority_tick)?;
            work.current(1)?
                .authorize(Capability::Observe, RiskTier::ReadOnly, &[], None)?;
            if goal != WorkshopGoal::Complete {
                return Err(error(
                    ErrorCode::Conflict,
                    "original workshop construction is incomplete",
                ));
            }
            let handoff = WorkshopProjectHandoff {
                directory: self.config.input.directory.clone(),
                binding: checkpoint.binding().clone(),
                acknowledged: checkpoint.pin(),
                record: checkpoint.record().clone(),
                progress,
                origin_witness: checkpoint.origin_witness(),
                origin_high_tick: checkpoint.high_tick(),
                origin_high_building: checkpoint.high_next_building_id(),
                high_tick: self.authority_tick,
                high_generation: self.project.high_generation,
                high_building: self.project.high_building,
                project_deadline_tick: deadline,
                authority_expires_tick: self.config.input.authority_expires_tick,
                clock_deadline_tick: self.config.clock_deadline(),
            };
            handoff.validate()?;
            Ok(handoff)
        })();
        self.budget = ceiling;
        if result.is_err() {
            if self.project.fenced
                && let Some(clock) = &mut self.clock
            {
                clock.fence_advancement();
            }
            self.withdraw();
        }
        result
    }
}

#[cfg(test)]
#[path = "project_tests.rs"]
mod tests;
