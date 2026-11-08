//! Independent bounded-run custody for the one retained Carpenter workshop.
//! Run/1.13 does not identify a fortress; native NIL grants are issued separately.
use super::{
    Config as WorkshopConfig, Guard, INSPECT_BYTES, Runtime, Work, denied, digest, error,
    exhausted, invalid, nonce,
};
use dfmcp_adapter::bounded_run::journal::{DurableRun, RunBinding, RunJournal, RunMode};
use dfmcp_adapter::bounded_run::private_file::{PrivateRunFile, open_private_run_journal_exact};
use dfmcp_adapter::bounded_run::rpc::{RunManifest, RunRpcClient, RunSource, RunTcpStream};
use dfmcp_adapter::bounded_run::{RunObservation, RunPlan, RunRecord, RunSpec};
use dfmcp_adapter::build_placement::journal::private_file::PrivateBuildFile;
use dfmcp_adapter::workshop_construction::clock::{
    Clock, ClockView, OPERATION_BYTES, Recovery, RunRequest, Target, VIEW_BYTES,
};
use dfmcp_adapter::workshop_construction::journal::{
    WorkshopGuard, WorkshopInventory, WorkshopJournal, WorkshopMode,
};
use dfmcp_adapter::workshop_construction::{WorkshopBinding, WorkshopGoal, WorkshopProgress};
use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, Digest32, ErrorCode, FortressId, GameTick,
    ObservationCursor, OperationContext, Result, RiskTier, StateAnchor,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::net::SocketAddr;
use std::time::Instant;

pub(super) const OPT_IN: &str = "DFMCP_ALLOW_UNADMITTED_WORKSHOP_CLOCK";
pub(super) const TOKEN: &str = "DFMCP_RUN_TOKEN";
pub(super) const ALLOW: &str = "DFMCP_RUN_ALLOW_CLOCK";
const CONNECT_BYTES: u64 = 1024 * 1024;
const GUARD_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Input {
    initialize: bool,
    generation: u64,
    recovery_generation: Option<u64>,
    deadline_tick: u64,
    recovery: Option<RecoveryInput>,
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct RecoveryInput {
    journal_id: String,
    head: String,
}
#[derive(Clone)]
pub(super) struct Config {
    initialize: bool,
    binding: RunBinding,
    recovery_generation: u64,
    deadline_tick: u64,
    recovery: Option<Recovery>,
}
impl Config {
    pub(super) fn parse(
        input: Input,
        workshop: &super::Input,
        endpoint: SocketAddr,
    ) -> Result<Self> {
        if input.initialize == input.recovery.is_some()
            || input.generation == 0
            || input.generation == u64::MAX
            || input
                .recovery_generation
                .is_some_and(|g| g < input.generation || g == u64::MAX)
            || input.deadline_tick == 0
            || input.deadline_tick > dfmcp_adapter::bounded_run::MAX_NATIVE_TICK
            || input.deadline_tick > workshop.authority_expires_tick
        {
            return Err(invalid(
                "clock requires exact initialization/recovery, independent source generation and a finite horizon within authority",
            ));
        }
        let recovery = input
            .recovery
            .map(|pin| -> Result<Recovery> {
                Ok(Recovery {
                    journal_id: digest(&pin.journal_id)?,
                    head: digest(&pin.head)?,
                })
            })
            .transpose()?;
        let binding = RunBinding::new(
            endpoint,
            RunManifest {
                generation: input.generation,
                df_version: workshop.df_version.clone(),
                dfhack_version: workshop.dfhack_version.clone(),
            },
        )?;
        Ok(Self {
            initialize: input.initialize,
            binding,
            recovery_generation: input.recovery_generation.unwrap_or(input.generation),
            deadline_tick: input.deadline_tick,
            recovery,
        })
    }
    pub(super) fn control(&self, mode: WorkshopMode) -> bool {
        self.initialize && mode == WorkshopMode::Control
    }
}

fn native_grants(config: &WorkshopConfig, control: bool) -> Vec<CapabilityGrant> {
    let mut capabilities = vec![Capability::Query];
    if control {
        capabilities.extend([Capability::Plan, Capability::ControlClock]);
    }
    capabilities
        .into_iter()
        .map(|capability| CapabilityGrant {
            capability,
            scope: CapabilityScope {
                fortress_id: Some(FortressId::NIL),
                ..CapabilityScope::default()
            },
            max_risk: if capability == Capability::Query {
                RiskTier::ReadOnly
            } else {
                RiskTier::Guarded
            },
            expires_at_tick: Some(GameTick(config.input.authority_expires_tick)),
            remaining_uses: None,
        })
        .collect()
}
fn native_context(c: &OperationContext, grants: &[CapabilityGrant]) -> OperationContext {
    OperationContext {
        anchor: StateAnchor {
            fortress_id: FortressId::NIL,
            cursor: ObservationCursor::ORIGIN,
            tick: c.anchor.tick,
            state_hash: Digest32::ZERO,
        },
        grants: grants.to_vec(),
        ..c.clone()
    }
}
fn recovery_json(pin: Recovery) -> Value {
    json!({"journal_id":pin.journal_id.to_hex(),"head":pin.head.to_hex()})
}

/// Keeps an opened empty clock until the exact original Placed receipt exists.
/// A failed binding consumes the owner; no missing file is silently recreated.
pub(super) struct Owner {
    config: Config,
    mode: RunMode,
    grants: Vec<CapabilityGrant>,
    journal: Option<RunJournal<PrivateRunFile>>,
    clock: Option<Clock<PrivateRunFile>>,
    recovery: Recovery,
    verified: bool,
    advancement_fenced: bool,
    unbound_active: Option<DurableRun>,
}
impl Owner {
    pub(super) fn open(
        config: &WorkshopConfig,
        mode: WorkshopMode,
        work: &mut Work,
    ) -> Result<Self> {
        let selected = config
            .clock
            .clone()
            .ok_or_else(|| denied("workshop clock configuration absent"))?;
        let control = selected.control(mode);
        if selected.initialize && !control {
            return Err(denied(
                "only an explicitly authorized control session can initialize workshop clock custody",
            ));
        }
        if control && std::env::var(ALLOW).ok().as_deref() != Some("1") {
            return Err(denied(
                "new workshop clock custody requires independent clock permission",
            ));
        }
        let mode = if control {
            RunMode::Control
        } else if mode == WorkshopMode::Offline {
            RunMode::Offline
        } else {
            RunMode::Recover
        };
        let grants = native_grants(config, control);
        let opening = native_context(&work.reserve(3 * VIEW_BYTES)?, &grants);
        let mut journal = open_private_run_journal_exact(
            &config.input.directory.join("workshop.clock.journal"),
            &opening,
            mode,
            Some(selected.binding.clone()),
            selected.initialize,
        )?;
        if let Some(pin) = selected.recovery {
            let current = native_context(&work.reserve(VIEW_BYTES)?, &grants);
            journal.verify_recovery(pin.journal_id, pin.head, &current)?;
        }
        let recovery = Recovery {
            journal_id: journal.id(),
            head: journal.head(),
        };
        let unbound_active = journal
            .cached_records()
            .find(|run| !run.terminal() || run.unresolved())
            .cloned();
        Ok(Self {
            config: selected,
            mode,
            grants,
            journal: Some(journal),
            clock: None,
            recovery,
            verified: false,
            advancement_fenced: false,
            unbound_active,
        })
    }
    fn contexts(&self, c: &OperationContext) -> (OperationContext, OperationContext) {
        let mut current = c.clone();
        current.anchor.tick = GameTick(current.anchor.tick.get().max(self.high_tick()));
        let native = native_context(&current, &self.grants);
        (current, native)
    }
    pub(super) fn high_tick(&self) -> u64 {
        self.clock.as_ref().map_or(0, Clock::high_tick)
    }
    pub(super) fn withdraw(&mut self) {
        self.verified = false;
        if let Some(clock) = &mut self.clock {
            clock.abandon_preparation();
        }
    }
    pub(super) fn fence_advancement(&mut self) {
        self.advancement_fenced = true;
        self.withdraw();
    }
    pub(super) fn prepared_seal(&self) -> Option<Digest32> {
        self.clock.as_ref().and_then(Clock::prepared_seal)
    }
    fn retain(&mut self) {
        if let Some(clock) = &self.clock {
            self.recovery = clock.recovery();
        } else if let Some(journal) = &self.journal {
            self.recovery = Recovery {
                journal_id: journal.id(),
                head: journal.head(),
            };
        }
    }
    fn inspect(
        &mut self,
        binding: &WorkshopBinding,
        inventory: &WorkshopInventory,
        c: &OperationContext,
    ) -> Result<Option<ClockView>> {
        self.verified = false;
        let entry = inventory.entries().first();
        if let Some(clock) = &self.clock {
            if clock.target().workshop_journal_id() != inventory.journal_id
                || clock.target().binding() != binding
                || entry.and_then(|entry| entry.native()) != Some(clock.target().record())
            {
                return Err(error(
                    ErrorCode::CorruptLedger,
                    "clock custody differs from the retained original workshop",
                ));
            }
        } else if let Some(record) = entry
            .filter(|entry| entry.placed())
            .and_then(|entry| entry.native())
        {
            let target = Target::new(
                inventory.journal_id,
                binding.clone(),
                record.clone(),
                self.config.deadline_tick,
            )?;
            let (current, native) = self.contexts(c);
            let journal = self
                .journal
                .take()
                .ok_or_else(|| denied("workshop clock owner was lost; reopen exact custody"))?;
            // The external pin is mandatory on recovery. Fresh empty custody has
            // only this process's creation proof and cannot inherit authority.
            let clock = Clock::new(journal, target, self.config.recovery, &current, &native)?;
            self.clock = Some(clock);
            self.unbound_active = None;
        }
        let (current, native) = self.contexts(c);
        let view = if let Some(clock) = &mut self.clock {
            Some(clock.inspect(&current, &native)?)
        } else {
            let journal = self
                .journal
                .as_mut()
                .ok_or_else(|| denied("workshop clock custody is not available"))?;
            if !journal.records(&native)?.is_empty() {
                return Err(error(
                    ErrorCode::CorruptLedger,
                    "clock history has no matching original workshop insertion",
                ));
            }
            None
        };
        self.retain();
        self.verified = true;
        Ok(view)
    }
    pub(super) fn reference(&self) -> Value {
        let view = self.clock.as_ref().map(Clock::cached_view);
        let unbound_run = self.unbound_active.as_ref().map(|entry| {
            dfmcp_adapter::workshop_construction::clock::ClockRunView {
                entry: entry.clone(),
                review_seal: None,
            }
        });
        let owner_lost = self.clock.is_none() && self.journal.is_none();
        json!({"enabled":true,"recovery":recovery_json(self.recovery),"verified":false,
            "target_id":view.as_ref().map(|view|view.target_id.to_hex()),
            "deadline_tick":self.config.deadline_tick,
            "advancement_fenced":self.advancement_fenced,
            "pending":view.as_ref().map_or(owner_lost || unbound_run.is_some(),|view|view.pending),
            "unresolved":view.as_ref().map_or(owner_lost || self.unbound_active.as_ref().is_some_and(DurableRun::unresolved),|view|view.unresolved),
            "active_run":view.as_ref().and_then(|view|view.active()).or(unbound_run.as_ref()).map(|run|run_json(run,false)),
            "current_pause_proven":false,"workshop_completion_proven":false,"automatic_redispatch":false})
    }
}

fn run_json(
    run: &dfmcp_adapter::workshop_construction::clock::ClockRunView,
    verified: bool,
) -> Value {
    let plan = run.entry.plan();
    let native = run.entry.native();
    json!({"key":plan.key(),"plan_digest":plan.digest().to_hex(),
        "review_seal":if verified {run.review_seal.map(|value|value.to_hex())} else {None},
        "state":run.entry.state().as_str(),"native_phase":native.map(|value|value.phase().as_str()),
        "game_ticks":plan.spec().game_ticks(),"wall_millis":plan.spec().wall_ms(),
        "start_tick":plan.before().tick(),"observed_tick":native.and_then(RunRecord::observed_tick),
        "native_unpause_attempted":native.is_some_and(RunRecord::unpause_attempted),
        "stopped_with_pause":native.is_some_and(RunRecord::pause_verified),
        "unresolved":run.entry.unresolved(),"canonical_plan_hex":super::hex(&plan.canonical_bytes()),
        "native_record_hex":native.map(|value|super::hex(value.canonical_bytes()))})
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PlanInput {
    kind: String,
    target_id: String,
    game_ticks: u32,
    wall_millis: u32,
}
pub(super) fn is_request(raw: &str) -> Result<bool> {
    if raw.len() > 2048 {
        return Err(exhausted());
    }
    let value: Value =
        serde_json::from_str(raw).map_err(|_| invalid("invalid closed workshop request"))?;
    Ok(value["kind"] == "run_workshop")
}
fn request(raw: &str) -> Result<RunRequest> {
    if raw.len() > 2048 {
        return Err(exhausted());
    }
    let input: PlanInput =
        serde_json::from_str(raw).map_err(|_| invalid("invalid closed workshop clock request"))?;
    if input.kind != "run_workshop" {
        return Err(invalid("clock request must select run_workshop"));
    }
    RunSpec::new(input.game_ticks, input.wall_millis)?;
    Ok(RunRequest {
        target_id: digest(&input.target_id)?,
        game_ticks: input.game_ticks,
        wall_millis: input.wall_millis,
    })
}

struct Native {
    source: RunRpcClient<RunTcpStream>,
    config: WorkshopConfig,
    token: Vec<u8>,
    control: bool,
    high_tick: u64,
}
impl Native {
    fn connect(
        config: &WorkshopConfig,
        selected: &Config,
        control: bool,
        c: &OperationContext,
    ) -> Result<Self> {
        config.current()?;
        c.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
        let token = std::env::var(TOKEN)
            .map_err(|_| denied("independent workshop clock credential unavailable"))?
            .into_bytes();
        let source = RunRpcClient::connect(
            selected.binding.endpoint(),
            token.clone(),
            nonce(c)?.to_vec(),
            c,
        )?;
        let expected_generation = if control {
            selected.binding.manifest().generation
        } else {
            selected.recovery_generation
        };
        if source.manifest().generation != expected_generation
            || !source.manifest().same_software(selected.binding.manifest())
        {
            return Err(denied(
                "workshop clock source differs from its independent generation/software pins",
            ));
        }
        let native = Self {
            source,
            config: config.clone(),
            token,
            control,
            high_tick: c.anchor.tick.get(),
        };
        native.check(false)?;
        Ok(native)
    }
    fn check(&self, write: bool) -> Result<()> {
        self.config.current()?;
        if std::env::var(TOKEN).ok().map(String::into_bytes).as_ref() != Some(&self.token) {
            return Err(denied("workshop clock credential changed"));
        }
        if write && (!self.control || std::env::var(ALLOW).ok().as_deref() != Some("1")) {
            return Err(denied(
                "independent workshop clock control permission denied",
            ));
        }
        Ok(())
    }
}
impl RunSource for Native {
    fn manifest(&self) -> &RunManifest {
        self.source.manifest()
    }
    fn endpoint(&self) -> Option<SocketAddr> {
        self.source.endpoint()
    }
    fn observe(&mut self, c: &OperationContext) -> Result<RunObservation> {
        self.check(false)?;
        self.source.restrict_deadline(c)?;
        // Return every decoded observation to the owner before a later policy
        // check can fail, so its consumed clock/sequence floor is never lost.
        self.source.observe(c)
    }
    fn prepare(&mut self, plan: &RunPlan, c: &OperationContext) -> Result<RunRecord> {
        self.check(true)?;
        self.source.restrict_deadline(c)?;
        let value = self.source.prepare(plan, c)?;
        self.high_tick = self.high_tick.max(value.observed_tick().unwrap_or(0));
        self.check(true)?;
        Ok(value)
    }
    fn commit(&mut self, plan: &RunPlan, c: &OperationContext) -> Result<RunRecord> {
        self.check(true)?;
        self.source.restrict_deadline(c)?;
        let value = self.source.commit(plan, c)?;
        self.high_tick = self.high_tick.max(value.observed_tick().unwrap_or(0));
        self.check(true)?;
        Ok(value)
    }
    fn query(&mut self, plan: &RunPlan, c: &OperationContext) -> Result<Option<RunRecord>> {
        self.check(false)?;
        self.source.restrict_deadline(c)?;
        let value = self.source.query(plan, c)?;
        self.high_tick = self.high_tick.max(
            value
                .as_ref()
                .and_then(RunRecord::observed_tick)
                .unwrap_or(0),
        );
        self.check(false)?;
        Ok(value)
    }
    fn cancel(&mut self, plan: &RunPlan, c: &OperationContext) -> Result<RunRecord> {
        self.check(true)?;
        self.source.restrict_deadline(c)?;
        let value = self.source.cancel(plan, c)?;
        self.high_tick = self.high_tick.max(value.observed_tick().unwrap_or(0));
        self.check(true)?;
        Ok(value)
    }
}

pub(super) enum Action<'a> {
    Plan(&'a str),
    Commit { seal: Digest32, confirmed: bool },
    Wait,
    Cancel,
}

impl Runtime {
    pub(super) fn clock_view(
        &mut self,
        inventory: &WorkshopInventory,
        c: &OperationContext,
        fresh: bool,
    ) -> Result<Value> {
        let Some(owner) = &mut self.clock else {
            return Ok(json!({"enabled":false}));
        };
        let mut clock_context = c.clone();
        clock_context.budget.max_bytes = 3 * VIEW_BYTES;
        let view = owner.inspect(self.journal.binding(), inventory, &clock_context)?;
        self.authority_tick = self.authority_tick.max(owner.high_tick());
        let entry = inventory.entries().first();
        let ready = fresh
            && entry.is_some_and(|entry| {
                matches!(entry.goal(), Some(WorkshopGoal::InProgress { .. }))
                    && !entry.unresolved()
                    && entry.high_generation() == entry.plan().before().generation()
                    && entry.samples()
                        <= dfmcp_adapter::workshop_construction::journal::MAX_SAMPLES - 3
            });
        let remaining = owner
            .config
            .deadline_tick
            .saturating_sub(self.authority_tick);
        let ticks = remaining.min(c.budget.max_game_ticks).min(1200);
        let available = ready
            && ticks > 0
            && !owner.advancement_fenced
            && owner.mode == RunMode::Control
            && std::env::var(ALLOW).ok().as_deref() == Some("1")
            && view
                .as_ref()
                .is_some_and(|view| view.verified && !view.pending && !view.unresolved);
        let commit_available = view.as_ref().is_some_and(|view| view.verified)
            && !owner.advancement_fenced
            && owner.prepared_seal().is_some()
            && std::env::var(ALLOW).ok().as_deref() == Some("1");
        let mut recent = view
            .as_ref()
            .map(|view| view.runs.iter().collect::<Vec<_>>())
            .unwrap_or_default();
        recent.sort_by_key(|run| {
            (
                run.entry.plan().before().tick(),
                run.entry.plan().before().sequence(),
            )
        });
        let omitted = recent.len().saturating_sub(4);
        let rows = recent
            .iter()
            .skip(omitted)
            .map(|run| run_json(run, true))
            .collect::<Vec<_>>();
        Ok(
            json!({"enabled":true,"mode":owner.mode.as_str(),"verified":true,
            "target_id":view.as_ref().map(|view|view.target_id.to_hex()),
            "original_plan_digest":view.as_ref().map(|view|view.original_plan_digest.to_hex()),
            "recovery":recovery_json(owner.recovery),"deadline_tick":owner.config.deadline_tick,
            "pending":view.as_ref().is_some_and(|view|view.pending),
            "unresolved":view.as_ref().is_some_and(|view|view.unresolved),
            "active_run":view.as_ref().and_then(|view|view.active()).map(|run|run_json(run,true)),
            "run_count":view.as_ref().map_or(0,|view|view.runs.len()),
            "recent_runs":rows,"omitted_run_count":omitted,
            "commit_available":commit_available,"clock_ready":ready,"new_run_available":available,
            "cancel_available":owner.mode==RunMode::Control && std::env::var(ALLOW).ok().as_deref()==Some("1"),
            "advancement_fenced":owner.advancement_fenced,
            "remaining_workshop_samples":entry.map(|entry|dfmcp_adapter::workshop_construction::journal::MAX_SAMPLES-entry.samples()),
            "recommended_game_ticks":if available {ticks} else {0},
            "remaining_horizon_ticks":remaining,"high_tick":owner.high_tick(),
            "readiness":view.as_ref().and_then(|view|view.readiness.as_ref()).map(|ready|json!({
                "tick":ready.tick(),"witness":ready.digest().to_hex(),"stage":ready.stage(),
                "max_stage":ready.max_stage(),"construct_jobs":ready.construct_jobs(),
                "meaning":"exact_original_incomplete_at_anchor","worker_readiness_proven":false})),
            "current_pause_proven":false,"workshop_completion_proven":false,
            "construction_causality_proven":false,"shared_process_identity_proven":false,
            "source_identity_semantics":"independently_pinned_run_incarnation_without_named_fortress_identity",
            "automatic_redispatch":false}),
        )
    }

    pub(super) fn clock_allows_goal_sample(&mut self, work: &mut Work) -> Result<()> {
        let Some(owner) = &mut self.clock else {
            return Ok(());
        };
        let inventory = self.journal.inventory(&work.reserve(INSPECT_BYTES)?)?;
        let c = work.reserve(3 * VIEW_BYTES)?;
        let view = owner.inspect(self.journal.binding(), &inventory, &c)?;
        self.authority_tick = self.authority_tick.max(owner.high_tick());
        work.advance_authorization_tick(self.authority_tick);
        if view.is_some_and(|view| view.pending || view.unresolved || !view.verified) {
            return Err(error(
                ErrorCode::EffectIndeterminate,
                "settle the exact original clock before sampling workshop completion",
            ));
        }
        Ok(())
    }

    /// Prepay the full clock operation and the external original-custody guard
    /// before contacting either native source. Every owner is restored even on
    /// error so original run recovery references survive a lost reply.
    pub(super) fn clock_action(&mut self, action: Action<'_>, work: &mut Work) -> Result<bool> {
        let mut owner = self
            .clock
            .take()
            .ok_or_else(|| denied("workshop clock is not configured"))?;
        let mut native_source_entered = false;
        let result = (|| {
            self.config.current()?;
            if self.mode == WorkshopMode::Offline {
                return Err(denied(
                    "offline workshop clock cannot contact native sources",
                ));
            }
            let inventory = self.journal.inventory(&work.reserve(INSPECT_BYTES)?)?;
            let before = owner
                .inspect(
                    self.journal.binding(),
                    &inventory,
                    &work.reserve(3 * VIEW_BYTES)?,
                )?
                .ok_or_else(|| {
                    denied("clock requires the original acknowledged Placed workshop")
                })?;
            self.guard
                .acknowledge(self.journal.pin(), &work.reserve(1024 * 1024)?)?;
            self.authority_tick = self
                .authority_tick
                .max(owner.high_tick())
                .max(self.journal.high_tick());
            work.advance_authorization_tick(self.authority_tick);
            let controls = matches!(action, Action::Plan(_) | Action::Commit { .. });
            let parsed = match &action {
                Action::Plan(raw) => Some(request(raw)?),
                _ => None,
            };
            if let Some(request) = &parsed {
                if request.target_id != before.target_id {
                    return Err(error(
                        ErrorCode::StaleAnchor,
                        "clock request names another original workshop",
                    ));
                }
                if u64::from(request.game_ticks) > work.context.budget.max_game_ticks
                    || self
                        .authority_tick
                        .checked_add(u64::from(request.game_ticks))
                        .is_none_or(|end| end > owner.config.deadline_tick)
                {
                    return Err(exhausted());
                }
            }
            if matches!(
                action,
                Action::Commit {
                    confirmed: false,
                    ..
                }
            ) {
                return Err(denied(
                    "review and confirm this exact independent clock interval",
                ));
            }
            if controls {
                let entry = inventory
                    .entries()
                    .first()
                    .ok_or_else(|| denied("original workshop missing"))?;
                if owner.mode != RunMode::Control
                    || owner.advancement_fenced
                    || std::env::var(ALLOW).ok().as_deref() != Some("1")
                    || self.config.input.checkpoint != "disposable-fortress-no-checkpoint"
                    || entry.unresolved()
                    || entry.goal_complete()
                    || entry.high_generation() != entry.plan().before().generation()
                    || matches!(
                        entry.goal(),
                        Some(
                            WorkshopGoal::Absent
                                | WorkshopGoal::Contradicted
                                | WorkshopGoal::Unknown(_)
                        )
                    )
                    || entry.samples()
                        > dfmcp_adapter::workshop_construction::journal::MAX_SAMPLES
                            - if matches!(action, Action::Plan(_)) {
                                3
                            } else {
                                2
                            }
                {
                    return Err(denied(
                        "clock requires current independent authority and a continuous unfinished original with remaining observation capacity",
                    ));
                }
            }
            // A settled local wait does not acquire credentials or a connection.
            if matches!(action, Action::Wait) && !before.needs_reconciliation() {
                return Ok(!before.pending && !before.unresolved);
            }
            let guard_reservation = if controls {
                Some(work.reserve(GUARD_BYTES)?)
            } else {
                None
            };
            let operation = work.reserve(OPERATION_BYTES)?;
            let guard_started = Instant::now();
            let connect = work.reserve(CONNECT_BYTES)?;
            let native_connect = native_context(&connect, &owner.grants);
            let needs_source =
                !matches!(action, Action::Cancel) || before.cancellation_needs_source();
            native_source_entered = needs_source;
            let mut native = if needs_source {
                Some(Native::connect(
                    &self.config,
                    &owner.config,
                    owner.mode == RunMode::Control,
                    &native_connect,
                )?)
            } else {
                None
            };
            let mut current = work.current(operation.budget.max_bytes)?;
            current.anchor.tick = GameTick(self.authority_tick.max(owner.high_tick()));
            let native_context = native_context(&current, &owner.grants);
            let clock = owner
                .clock
                .as_mut()
                .ok_or_else(|| denied("original clock binding unavailable"))?;
            let target = clock.target().clone();
            let outcome = match action {
                Action::Plan(_) => {
                    let source = native
                        .as_mut()
                        .ok_or_else(|| denied("clock source absent"))?;
                    let request = parsed.ok_or_else(|| invalid("clock request absent"))?;
                    clock.prepare_guarded(
                        source,
                        &request,
                        &current,
                        &native_context,
                        |_, plan, c| {
                            original_progress(
                                &self.config,
                                &mut self.journal,
                                &mut self.guard,
                                &mut self.authority_tick,
                                &target,
                                plan,
                                c,
                                guard_reservation.as_ref(),
                                guard_started,
                            )
                        },
                    )
                }
                Action::Commit { seal, confirmed } => {
                    let source = native
                        .as_mut()
                        .ok_or_else(|| denied("clock source absent"))?;
                    clock.commit_guarded(
                        source,
                        seal,
                        confirmed,
                        &current,
                        &native_context,
                        |_, plan, c| {
                            original_progress(
                                &self.config,
                                &mut self.journal,
                                &mut self.guard,
                                &mut self.authority_tick,
                                &target,
                                plan,
                                c,
                                guard_reservation.as_ref(),
                                guard_started,
                            )
                        },
                    )
                }
                Action::Wait => clock.reconcile(
                    native
                        .as_mut()
                        .ok_or_else(|| denied("clock source absent"))?,
                    &current,
                    &native_context,
                ),
                Action::Cancel => clock.cancel(
                    native.as_mut().map(|source| source as &mut dyn RunSource),
                    &current,
                    &native_context,
                ),
            };
            self.authority_tick = self
                .authority_tick
                .max(clock.high_tick())
                .max(self.journal.high_tick())
                .max(native.as_ref().map_or(0, |source| source.high_tick));
            work.advance_authorization_tick(self.authority_tick);
            let view = outcome?;
            owner.verified = view.verified;
            Ok(!view.pending && !view.unresolved && view.verified)
        })();
        // A rejected native handshake/progress read may have consumed a source
        // generation or tick that its RPC cannot return. Never revive fresh
        // advancement after such a failed control path. Local retirement,
        // native safety stop and query-only recovery remain available.
        if result.is_err() && native_source_entered {
            owner.fence_advancement();
        }
        owner.retain();
        self.authority_tick = self
            .authority_tick
            .max(owner.high_tick())
            .max(self.journal.high_tick());
        work.advance_authorization_tick(self.authority_tick);
        self.clock = Some(owner);
        result
    }
}

/// This is a durable original progress sample, never a readiness claim about a
/// worker or scheduler. Both journal and independent ACK are rechecked before
/// the run writer, and source discontinuity survives process recovery.
#[allow(clippy::too_many_arguments)]
fn original_progress(
    config: &WorkshopConfig,
    journal: &mut WorkshopJournal<PrivateBuildFile>,
    guard: &mut Guard,
    authority_tick: &mut u64,
    target: &Target,
    plan: &RunPlan,
    c: &OperationContext,
    reserved: Option<&OperationContext>,
    started: Instant,
) -> Result<WorkshopProgress> {
    let reserved = reserved.ok_or_else(exhausted)?;
    let mut remaining = reserved.budget.max_bytes;
    let mut part = |bytes: u64| -> Result<OperationContext> {
        remaining = remaining.checked_sub(bytes).ok_or_else(exhausted)?;
        let mut current = c.clone();
        current.budget.max_wall_millis = current.budget.max_wall_millis.min(
            reserved
                .budget
                .max_wall_millis
                .checked_sub(started.elapsed().as_millis() as u64)
                .filter(|wall| *wall > 0)
                .ok_or_else(exhausted)?,
        );
        current.budget.max_bytes = bytes;
        Ok(current)
    };
    config.current()?;
    if std::env::var(ALLOW).ok().as_deref() != Some("1") {
        return Err(denied(
            "independent clock permission revoked before original progress guard",
        ));
    }
    let before = journal.inventory(&part(INSPECT_BYTES)?)?;
    let entry = before
        .entries()
        .first()
        .ok_or_else(|| denied("original workshop unavailable"))?;
    if before.entries().len() != 1
        || before.journal_id != target.workshop_journal_id()
        || journal.binding() != target.binding()
        || entry.native() != Some(target.record())
        || entry.unresolved()
        || entry.goal_complete()
        || entry.high_generation() != entry.plan().before().generation()
        || plan
            .before()
            .tick()
            .is_none_or(|tick| tick < entry.high_tick().max(*authority_tick))
    {
        return Err(denied(
            "clock guard requires the same continuous unfinished original workshop",
        ));
    }
    guard.acknowledge(journal.pin(), &part(1024 * 1024)?)?;
    let mut source = config.source(&part(CONNECT_BYTES)?, WorkshopMode::Recover)?;
    source.require_progress_at_or_after((*authority_tick).max(plan.before().tick().unwrap_or(0)));
    let sampled = journal.sample(
        &mut source,
        entry.plan().key(),
        entry.plan().digest(),
        &part(16 * 1024 * 1024)?,
        guard,
    )?;
    *authority_tick = (*authority_tick).max(sampled.high_tick());
    guard.acknowledge(journal.pin(), &part(1024 * 1024)?)?;
    let after = journal.inventory(&part(INSPECT_BYTES)?)?;
    config.current()?;
    if after.entries().first().is_none_or(|latest| {
        latest.native() != Some(target.record()) || latest.samples() != sampled.samples()
    }) || sampled.samples() != entry.samples() + 1
        || !matches!(sampled.goal(), Some(WorkshopGoal::InProgress { .. }))
    {
        return Err(denied(
            "fresh workshop evidence no longer supports this unfinished original",
        ));
    }
    sampled
        .latest_sample()
        .cloned()
        .ok_or_else(|| denied("fresh original workshop progress absent"))
}

#[cfg(test)]
#[path = "clock_config_tests.rs"]
mod clock_config_tests;
