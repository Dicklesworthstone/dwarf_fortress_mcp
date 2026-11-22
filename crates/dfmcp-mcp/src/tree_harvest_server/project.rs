//! Embedded Tree owner for one immutable resource project. The standalone
//! environment, native permissions, original journal and clock remain separate.
use super::*;
use dfmcp_adapter::bounded_run::MAX_NATIVE_TICK;
use std::path::Path;

#[path = "project_handoff.rs"]
mod handoff;
pub(crate) use handoff::TreeProjectHandoff;

#[derive(Default)]
pub(super) struct State {
    high_generation: u64,
    high_item: u32,
    pub(super) fenced: bool,
}
impl State {
    pub(super) fn from_observation(capture: Option<&Capture>) -> Self {
        Self {
            high_generation: capture.map_or(0, |c| c.generation),
            high_item: capture.map_or(0, |c| c.next_item),
            fenced: false,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Frontier {
    high_tick: u64,
    high_generation: u64,
    high_sequence: u64,
    high_next_item_id: u32,
    fenced: bool,
}

fn invalid(message: &str) -> DfmcpError {
    error(ErrorCode::InvalidRequest, message)
}
fn exhausted() -> DfmcpError {
    error(
        ErrorCode::BudgetExceeded,
        "original Tree project work allowance exhausted",
    )
}
fn pin_json(pin: WorkshopPin) -> Value {
    json!({"journal_id":pin.journal_id.to_hex(),"head":pin.head.to_hex(),"frames":pin.frames})
}
fn pin(value: &Value) -> Result<WorkshopPin> {
    let p: PinInput = serde_json::from_value(value.clone())
        .map_err(|_| invalid("invalid retained Tree acknowledgement"))?;
    let pin = WorkshopPin {
        journal_id: digest(&p.journal_id)?,
        head: digest(&p.head)?,
        frames: p.frames,
    };
    if pin.journal_id == Digest32::ZERO || pin.head == Digest32::ZERO || pin.frames > 40 {
        return Err(invalid("invalid retained Tree acknowledgement identity"));
    }
    Ok(pin)
}

impl Config {
    pub(crate) fn from_resource_project_environment(
        parent_check: Arc<dyn Fn() -> Result<()> + Send + Sync>,
    ) -> Result<Self> {
        runtime_io()?;
        parent_check()?;
        family_environment()?;
        let mut config = Self::parse(
            &std::env::var(CONFIG)
                .map_err(|_| denied("resource project Tree configuration is missing"))?,
        )?;
        if config.clock.is_some()
            && config.policy["clock"]["initialize"] != json!(config.input.initialize)
        {
            return Err(invalid(
                "project Tree and clock must share initialization or retained recovery",
            ));
        }
        config.parent_check = Some(parent_check);
        config.current()?;
        Ok(config)
    }

    pub(crate) fn with_project_deadline(mut self, deadline: u64) -> Result<Self> {
        if self.parent_check.is_none()
            || deadline == 0
            || deadline > MAX_NATIVE_TICK
            || deadline > self.input.authority_expires_tick
            || self.project_deadline.is_some_and(|old| old != deadline)
        {
            return Err(invalid(
                "Tree project deadline must be immutable and inside original host authority",
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
        let mut policy = self.policy.clone();
        if let Some(object) = policy.as_object_mut() {
            for field in ["initialize", "acknowledged", "recovery_generation"] {
                object.remove(field);
            }
            if let Some(clock) = object.get_mut("clock").and_then(Value::as_object_mut) {
                for field in ["initialize", "recovery", "recovery_generation"] {
                    clock.remove(field);
                }
            }
        }
        Digest32::of_bytes(
            json!({"schema":"dfmcp.tree-project-policy/1",
            "policy":policy,"deadline_tick":self.project_deadline})
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
    pub(crate) fn project_root(&self) -> [u32; 3] {
        self.input.root
    }
    pub(crate) fn project_deadline_tick(&self) -> Option<u64> {
        self.project_deadline
    }
    pub(crate) fn project_initializing(&self) -> bool {
        self.input.initialize
    }
    pub(crate) fn project_clock_configured(&self) -> bool {
        self.clock.is_some()
    }

    pub(crate) fn validate_project_recovery(&self, reference: &Value) -> Result<()> {
        self.current()?;
        self.project_recovery_pins(reference)
    }

    fn project_recovery_pins(&self, reference: &Value) -> Result<()> {
        if self.parent_check.is_none() || self.project_deadline.is_none() || self.input.initialize {
            return Err(denied(
                "project Tree recovery requires existing original custody",
            ));
        }
        if self.external != Some(pin(&reference["acknowledged"])?) {
            return Err(denied(
                "Tree restart acknowledgement differs from the parent checkpoint",
            ));
        }
        match (
            self.clock.is_some(),
            reference.get("clock").filter(|value| !value.is_null()),
        ) {
            (false, None) => {}
            (true, Some(clock)) => {
                let configured = &self.policy["clock"];
                if configured["initialize"] != false
                    || !configured["recovery"].is_object()
                    || configured["recovery"] != clock["recovery"]
                {
                    return Err(denied(
                        "Tree clock recovery pin differs from the parent checkpoint",
                    ));
                }
            }
            _ => return Err(denied("Tree clock custody changed across project recovery")),
        }
        Ok(())
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
        max_game_ticks: ceiling.max_game_ticks.min(remaining.max_game_ticks),
        max_bytes: ceiling.max_bytes.min(remaining.max_bytes),
        max_entities: ceiling.max_entities.min(remaining.max_entities),
        max_actions: ceiling.max_actions.min(remaining.max_actions),
        max_output_tokens: ceiling.max_output_tokens.min(remaining.max_output_tokens),
    };
    budget.validate()?;
    Ok(budget)
}

impl Runtime {
    pub(crate) fn project_has_original_history(&self) -> bool {
        self.journal.pin().frames != 0
            || self
                .clock
                .as_ref()
                .is_some_and(clock::Owner::project_has_history)
    }
    fn project_current(&self) -> Result<()> {
        self.config.current()?;
        self.validate_project_original_policy()
    }

    /// Check the locally replayed original, including a valid journal descendant
    /// that appeared after the parent last retained an empty Tree checkpoint.
    /// This does not grant a review or consume the owner's recovery evidence.
    pub(crate) fn validate_project_original_policy(&self) -> Result<()> {
        if self.config.parent_check.is_none() || self.config.project_deadline.is_none() {
            return Err(denied(
                "Tree child is not bound to its original resource project",
            ));
        }
        if self
            .journal
            .original
            .as_ref()
            .is_some_and(|original| Some(original.deadline_tick) != self.config.project_deadline)
        {
            return Err(denied(
                "recovered Tree original deadline differs from the immutable project goal",
            ));
        }
        Ok(())
    }

    pub(super) fn retain_project_capture(&mut self, capture: &Capture) {
        if self.config.parent_check.is_none() {
            return;
        }
        self.project.fenced |= capture.generation != self.config.binding.generation
            || capture.tick < self.high_tick
            || capture.sequence < self.high_sequence
            || capture.next_item < self.project.high_item;
        self.project.high_generation = self.project.high_generation.max(capture.generation);
        self.project.high_item = self.project.high_item.max(capture.next_item);
        self.high_tick = self.high_tick.max(capture.tick);
        self.high_sequence = self.high_sequence.max(capture.sequence);
    }

    pub(super) fn retain_project_progress(&mut self, progress: &Progress) {
        if self.config.parent_check.is_none() {
            return;
        }
        self.project.fenced |= progress.generation != self.config.binding.generation
            || progress.tick < self.high_tick.max(self.journal.high_tick)
            || progress.sequence < self.high_sequence.max(self.journal.high_sequence)
            || progress.next_item < self.project.high_item.max(self.journal.high_item);
        self.project.high_generation = self.project.high_generation.max(progress.generation);
        self.project.high_item = self.project.high_item.max(progress.next_item);
        self.high_tick = self.high_tick.max(progress.tick);
        self.high_sequence = self.high_sequence.max(progress.sequence);
    }

    fn project_frontier(&self) -> Value {
        let capture = self.observation.as_ref();
        json!({
            "high_tick":self.high_tick.max(self.journal.high_tick)
                .max(self.clock.as_ref().map_or(0,clock::Owner::high_tick)),
            "high_generation":self.project.high_generation.max(self.journal.high_generation)
                .max(capture.map_or(0,|value|value.generation)),
            "high_sequence":self.high_sequence.max(self.journal.high_sequence),
            "high_next_item_id":self.project.high_item.max(self.journal.high_item)
                .max(capture.map_or(0,|value|value.next_item)),
            "fenced":self.project.fenced || self.journal.is_fenced() || self.journal.continuity_lost,
        })
    }

    pub(crate) fn project_recovery_reference(&self) -> Value {
        let mut reference = self.reference();
        if let Some(object) = reference.as_object_mut() {
            object.remove("original_review_seal");
        }
        reference["original"] = self.journal.original.as_ref().map(|original| json!({
            "key":original.plan.key(),"plan_digest":original.plan.digest.to_hex(),
            "deadline_tick":original.deadline_tick,"dispatch_started":self.journal.dispatch_started,
            "unresolved":self.journal.effect_uncertain || self.journal.sample_pending,
            "historical_supply_observed_tick":self.journal.achieved_tick,
            "currently_verified":false,
        })).unwrap_or(Value::Null);
        reference["project_source_frontier"] = self.project_frontier();
        reference["acknowledgement_currently_verified"] = json!(false);
        reference
    }

    pub(crate) fn retain_project_recovery(&mut self, reference: &Value) -> Result<()> {
        self.config.validate_project_recovery(reference)?;
        self.retain_project_recovery_state(reference)
    }

    fn retain_project_recovery_state(&mut self, reference: &Value) -> Result<()> {
        let value = &reference["project_source_frontier"];
        if value.to_string().len() > 1024 {
            return Err(invalid("Tree project frontier exceeds its bound"));
        }
        let frontier: Frontier = serde_json::from_value(value.clone())
            .map_err(|_| invalid("invalid closed Tree project frontier"))?;
        self.retain_frontier(frontier)?;
        if let Some(clock) = &mut self.clock {
            if reference["clock"]["advancement_fenced"]
                .as_bool()
                .ok_or_else(|| denied("parent checkpoint lost the Tree clock fence"))?
            {
                clock.fence_advancement();
            }
        }
        self.withdraw();
        self.validate_project_original_policy()
    }

    fn retain_frontier(&mut self, frontier: Frontier) -> Result<()> {
        if frontier.high_tick > MAX_NATIVE_TICK
            || frontier.high_generation == u64::MAX
            || frontier.high_next_item_id > i32::MAX as u32
        {
            return Err(invalid("invalid Tree project source floors"));
        }
        self.high_tick = self.high_tick.max(frontier.high_tick);
        self.high_sequence = self.high_sequence.max(frontier.high_sequence);
        self.project.high_generation = self.project.high_generation.max(frontier.high_generation);
        self.project.high_item = self.project.high_item.max(frontier.high_next_item_id);
        self.project.fenced |= frontier.fenced
            || (self.project.high_generation != 0
                && self.project.high_generation != self.config.binding.generation);
        Ok(())
    }

    pub(crate) fn perform_project(
        &mut self,
        action: Action,
        remaining: WorkBudget,
    ) -> Result<Value> {
        let start = Instant::now();
        let ceiling = self.budget;
        let result = (|| {
            self.project_current()?;
            self.budget = narrow(ceiling, remaining, start)?;
            if self.project.fenced
                && matches!(
                    &action,
                    Action::Observe
                        | Action::Plan(_)
                        | Action::Commit(_, _)
                        | Action::ClockPlan(_)
                        | Action::ClockCommit(_, _)
                )
            {
                return Err(denied(
                    "Tree source continuity is fenced; retain and reconcile its original custody",
                ));
            }
            let mut value = self.perform(action)?;
            value["custody"] = self.project_recovery_reference();
            value["project_source_frontier"] = self.project_frontier();
            Ok(value)
        })();
        self.budget = ceiling;
        if result.is_err() {
            self.withdraw();
        }
        result
    }

    /// Full action review, bounded independently of the native capsule size.
    /// The exact capsule remains either in this owner or in original custody;
    /// no descriptor here claims that those raw bytes were delivered inline.
    pub(super) fn capture_view(&self, capture: &Capture, in_custody: bool) -> Value {
        let mut value = capture_json(capture);
        if self.config.parent_check.is_none() {
            return value;
        }
        if let Some(object) = value.as_object_mut() {
            object.remove("native_capture_hex");
        }
        value["native_capture_commitment"] = json!({
            "schema":"dfmcp.tree-project-capture-commitment/1",
            "byte_length":capture.bytes().len(),
            "sha256":Digest32::of_bytes(capture.bytes()).to_hex(),
            "raw_bytes_inline":false,
            "retained_by":if in_custody {"original_tree_journal"} else {"current_tree_owner"},
        });
        let cuboid = |area: MapCuboid| {
            [
                area.min.x, area.min.y, area.min.z, area.max.x, area.max.y, area.max.z,
            ]
        };
        value["approval"] = json!({
            "root":capture.root,"species":capture.species,"tree_dimensions":capture.size,
            "computed_designation":capture.designation,
            "effect_scope":capture.effect_scope().ok().map(cuboid),
            "host_effect_scope":cuboid(self.config.scope),
            "protected":self.config.protected.iter().copied().map(cuboid).collect::<Vec<_>>(),
            "visible_domain_complete":capture.complete,"visible_cell_count":capture.cells.len(),
            "body_cell_count":capture.body.len(),"belowground_roots_observed":false,
            "paused_at_anchor":capture.paused,"mature":capture.mature,"alive":capture.alive,
            "existing_felling_job":capture.existing_job,"eligible_at_anchor":capture.eligible(),
            "workers":capture.workers.iter().map(|worker|json!({
                "id":worker.id,"position":worker.position,"visible":worker.facts[0],
                "adult":worker.facts[1],"sane":worker.facts[2],"woodcutting_labor_enabled":worker.facts[3],
                "labor_valid":worker.facts[4],"job_available":worker.facts[5],"walk_group_connected":worker.facts[6],
                "eligible_at_anchor":worker.eligible(),
            })).collect::<Vec<_>>(),
            "tools":capture.tools.iter().map(|tool|json!({
                "item_id":tool.id,"position":tool.position,"holder_id":tool.holder,
                "role":tool.role,"flags":tool.flags,"flags2":tool.flags2,
                "axe":tool.facts[0],"visible":tool.facts[1],"reciprocal_holder":tool.facts[2],"walk_group_connected":tool.facts[3],
                "offered_at_anchor":tool.offered(),"equipment_or_scheduling_proven":false,
            })).collect::<Vec<_>>(),
            "checkpoint":self.config.input.checkpoint,
            "lease_ticks":self.config.input.lease_ticks,
            "authority_expires_tick":self.config.input.authority_expires_tick,
            "project_deadline_tick":self.config.project_deadline,
            "future_falling_safety_proven":false,"global_controller_fence":false,
        });
        value
    }
}

#[cfg(all(test, target_os = "linux"))]
#[path = "project_tests.rs"]
mod tests;
