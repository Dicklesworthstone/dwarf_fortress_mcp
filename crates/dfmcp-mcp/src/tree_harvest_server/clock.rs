//! Independently authorized finite Run/1.13 intervals for the acknowledged
//! original Tree/1.21 goal. The original harvest deadline is never renewed.
use super::{Config as TreeConfig, Runtime, TurnWork, denied, digest, error, nonce};
use dfmcp_adapter::bounded_run::journal::{DurableRun, RunBinding, RunJournal, RunMode};
use dfmcp_adapter::bounded_run::private_file::{PrivateRunFile, open_private_run_journal_exact};
use dfmcp_adapter::bounded_run::rpc::{RunManifest, RunRpcClient, RunSource, RunTcpStream};
use dfmcp_adapter::bounded_run::{RunObservation, RunPlan, RunRecord, RunSpec};
use dfmcp_adapter::build_placement::journal::private_file::PrivateBuildFile;
use dfmcp_adapter::tree_harvest::clock::{
    Clock, ClockRunView, ClockView, OPERATION_BYTES, Recovery, RunRequest, Target, VIEW_BYTES,
};
use dfmcp_adapter::tree_harvest::journal::{CLOCK_ORIGIN_BYTES, Journal, Mode, TreeClockOrigin};
use dfmcp_adapter::tree_harvest::rpc::{Binding, CONNECT_BYTES as TREE_CONNECT_BYTES};
use dfmcp_adapter::tree_harvest::{MAX_SAMPLES, Progress};
use dfmcp_adapter::workshop_construction::acknowledgement::WorkshopAcknowledgements;
use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, Digest32, ErrorCode, FortressId, GameTick,
    ObservationCursor, OperationContext, Result, RiskTier, StateAnchor,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::net::SocketAddr;
use std::time::Instant;

pub(super) const OPT_IN: &str = "DFMCP_ALLOW_UNADMITTED_TREE_CLOCK";
pub(super) const TOKEN: &str = "DFMCP_RUN_TOKEN";
pub(super) const ALLOW: &str = "DFMCP_RUN_ALLOW_CLOCK";
const CONNECT_BYTES: u64 = 1024 * 1024;
const TREE_LOCAL_BYTES: u64 = 32 * 1024 * 1024;
const GUARD_BYTES: u64 = 2 * CLOCK_ORIGIN_BYTES + 2 * TREE_LOCAL_BYTES + 2 * TREE_CONNECT_BYTES;

fn invalid(message: &str) -> dfmcp_core::DfmcpError {
    error(ErrorCode::InvalidRequest, message)
}
fn exhausted() -> dfmcp_core::DfmcpError {
    error(
        ErrorCode::BudgetExceeded,
        "original tree clock allowance exhausted",
    )
}
fn pin_digest(raw: &str) -> Result<Digest32> {
    let value = digest(raw)?;
    if value == Digest32::ZERO {
        return Err(invalid("tree clock identity cannot be zero"));
    }
    Ok(value)
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Input {
    initialize: bool,
    generation: u64,
    recovery_generation: Option<u64>,
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
    recovery: Option<Recovery>,
}
impl Config {
    pub(super) fn parse(input: Input, tree: &super::Input, endpoint: SocketAddr) -> Result<Self> {
        if input.initialize == input.recovery.is_some()
            || input.generation == 0
            || input.generation == u64::MAX
            || input
                .recovery_generation
                .is_some_and(|generation| generation < input.generation || generation == u64::MAX)
        {
            return Err(invalid(
                "tree clock requires exact initialization/recovery and independent source generations",
            ));
        }
        let recovery = input
            .recovery
            .map(|pin| -> Result<Recovery> {
                Ok(Recovery {
                    journal_id: pin_digest(&pin.journal_id)?,
                    head: pin_digest(&pin.head)?,
                })
            })
            .transpose()?;
        Ok(Self {
            initialize: input.initialize,
            binding: RunBinding::new(
                endpoint,
                RunManifest {
                    generation: input.generation,
                    df_version: tree.df_version.clone(),
                    dfhack_version: tree.dfhack_version.clone(),
                },
            )?,
            recovery_generation: input.recovery_generation.unwrap_or(input.generation),
            recovery,
        })
    }
    pub(super) fn control(&self, mode: Mode) -> bool {
        self.initialize && mode == Mode::Control
    }
}

fn native_grants(config: &TreeConfig, control: bool) -> Vec<CapabilityGrant> {
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
            expires_at_tick: if capability == Capability::Query {
                None
            } else {
                Some(GameTick(config.input.authority_expires_tick))
            },
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
    retained_tick: u64,
}
impl Owner {
    pub(super) fn open(
        config: &TreeConfig,
        mode: Mode,
        work: &mut TurnWork,
        c: &OperationContext,
    ) -> Result<Self> {
        let selected = config
            .clock
            .clone()
            .ok_or_else(|| denied("tree clock is not configured"))?;
        let control = selected.control(mode);
        if selected.initialize && !control {
            return Err(denied(
                "only an authorized control session can initialize tree clock custody",
            ));
        }
        if control && std::env::var(ALLOW).ok().as_deref() != Some("1") {
            return Err(denied(
                "new tree clock custody requires independent clock permission",
            ));
        }
        let mode = if control {
            RunMode::Control
        } else if mode == Mode::Offline {
            RunMode::Offline
        } else {
            RunMode::Recover
        };
        let grants = native_grants(config, control);
        let opening = native_context(&work.reserve(c, 3 * VIEW_BYTES)?, &grants);
        let mut journal = open_private_run_journal_exact(
            &config.input.directory.join("tree-harvest.clock.journal"),
            &opening,
            mode,
            Some(selected.binding.clone()),
            selected.initialize,
        )?;
        if let Some(pin) = selected.recovery {
            let current = native_context(&work.reserve(c, VIEW_BYTES)?, &grants);
            journal.verify_recovery(pin.journal_id, pin.head, &current)?;
        }
        let recovery = Recovery {
            journal_id: journal.id(),
            head: journal.head(),
        };
        let unbound_active = journal
            .cached_records()
            .find(|entry| !entry.terminal() || entry.unresolved())
            .cloned();
        let retained_tick = journal
            .cached_records()
            .fold(c.anchor.tick.get(), |floor, entry| {
                floor.max(entry.plan().before().tick().unwrap_or(0)).max(
                    entry
                        .native()
                        .and_then(RunRecord::observed_tick)
                        .unwrap_or(0),
                )
            });
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
            retained_tick,
        })
    }
    fn contexts(&self, c: &OperationContext) -> (OperationContext, OperationContext) {
        let mut current = c.clone();
        current.anchor.tick = current.anchor.tick.max(GameTick(self.high_tick()));
        let native = native_context(&current, &self.grants);
        (current, native)
    }
    pub(super) fn high_tick(&self) -> u64 {
        self.retained_tick
            .max(self.clock.as_ref().map_or(0, Clock::high_tick))
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
        self.retained_tick = self.high_tick();
        if let Some(clock) = &self.clock {
            self.recovery = clock.recovery();
        } else if let Some(journal) = &self.journal {
            self.recovery = Recovery {
                journal_id: journal.id(),
                head: journal.head(),
            };
        }
    }
    /// The caller verifies current Tree bytes and the independent ACK before
    /// supplying its journal. A new target additionally requires the typed
    /// origin API; already-bound inspection cannot replace that immutable target.
    fn inspect(
        &mut self,
        binding: &Binding,
        tree: &Journal<PrivateBuildFile>,
        origin: Option<TreeClockOrigin>,
        c: &OperationContext,
    ) -> Result<Option<ClockView>> {
        self.verified = false;
        if let Some(clock) = &self.clock {
            if clock.target().tree_journal_id() != tree.pin().journal_id
                || clock.target().binding() != binding
                || tree.original.as_ref() != Some(clock.target().original())
                || tree.native.as_ref() != Some(clock.target().record())
            {
                return Err(error(
                    ErrorCode::CorruptLedger,
                    "clock custody differs from the original tree designation",
                ));
            }
        } else if let Some(origin) = origin {
            self.retained_tick = self.retained_tick.max(origin.high_tick());
            let target = Target::new(origin)?;
            let (current, native) = self.contexts(c);
            let journal = self
                .journal
                .take()
                .ok_or_else(|| denied("tree clock owner was lost; reopen its exact custody"))?;
            self.clock = Some(Clock::new(
                journal,
                target,
                self.config.recovery,
                &current,
                &native,
            )?);
            self.unbound_active = None;
        }
        let (current, native) = self.contexts(c);
        let view = if let Some(clock) = &mut self.clock {
            Some(clock.inspect(&current, &native)?)
        } else {
            let journal = self
                .journal
                .as_mut()
                .ok_or_else(|| denied("tree clock custody is unavailable"))?;
            if !journal.records(&native)?.is_empty() {
                return Err(error(
                    ErrorCode::CorruptLedger,
                    "clock history has no matching acknowledged tree original",
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
        let unbound_run = self.unbound_active.as_ref().map(|entry| ClockRunView {
            entry: entry.clone(),
            review_seal: None,
        });
        let owner_lost = self.clock.is_none() && self.journal.is_none();
        json!({"enabled":true,"recovery":recovery_json(self.recovery),"verified":false,
            "target_id":view.as_ref().map(|view|view.target_id.to_hex()),
            "deadline_tick":self.clock.as_ref().map(|clock|clock.target().deadline_tick()),
            "advancement_fenced":self.advancement_fenced,
            "pending":view.as_ref().map_or(owner_lost || unbound_run.is_some(),|view|view.pending),
            "unresolved":view.as_ref().map_or(owner_lost || self.unbound_active.as_ref().is_some_and(DurableRun::unresolved),|view|view.unresolved),
            "active_run":view.as_ref().and_then(|view|view.active()).or(unbound_run.as_ref()).map(|run|run_json(run,false)),
            "consumed_tick_floor":self.high_tick(),"current_pause_proven":false,
            "harvest_goal_proven":false,"automatic_redispatch":false})
    }
}

fn run_json(run: &ClockRunView, verified: bool) -> Value {
    let plan = run.entry.plan();
    let native = run.entry.native();
    json!({"key":plan.key(),"plan_digest":plan.digest().to_hex(),
        "review_seal":if verified {run.review_seal.map(|seal|seal.to_hex())} else {None},
        "state":run.entry.state().as_str(),"native_phase":native.map(|record|record.phase().as_str()),
        "game_ticks":plan.spec().game_ticks(),"wall_millis":plan.spec().wall_ms(),
        "start_tick":plan.before().tick(),"observed_tick":native.and_then(RunRecord::observed_tick),
        "native_unpause_attempted":native.is_some_and(RunRecord::unpause_attempted),
        "stopped_with_pause":native.is_some_and(RunRecord::pause_verified),
        "unresolved":run.entry.unresolved(),"canonical_plan_hex":super::hex(&plan.canonical_bytes()),
        "native_record_hex":native.map(|record|super::hex(record.canonical_bytes()))})
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
        serde_json::from_str(raw).map_err(|_| invalid("invalid closed tree request"))?;
    Ok(value["kind"] == "run_tree")
}
fn request(raw: &str) -> Result<RunRequest> {
    if raw.len() > 2048 {
        return Err(exhausted());
    }
    let value: PlanInput =
        serde_json::from_str(raw).map_err(|_| invalid("invalid closed tree clock request"))?;
    if value.kind != "run_tree" {
        return Err(invalid("clock request must select run_tree"));
    }
    RunSpec::new(value.game_ticks, value.wall_millis)?;
    Ok(RunRequest {
        target_id: pin_digest(&value.target_id)?,
        game_ticks: value.game_ticks,
        wall_millis: value.wall_millis,
    })
}

struct Native {
    source: RunRpcClient<RunTcpStream>,
    config: TreeConfig,
    token: Vec<u8>,
    control: bool,
    high_tick: u64,
}
impl Native {
    fn connect(
        config: &TreeConfig,
        selected: &Config,
        control: bool,
        c: &OperationContext,
    ) -> Result<Self> {
        config.current()?;
        c.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
        let token = std::env::var(TOKEN)
            .map_err(|_| denied("independent tree clock credential unavailable"))?
            .into_bytes();
        let source = RunRpcClient::connect(
            selected.binding.endpoint(),
            token.clone(),
            nonce(c, config.scope)?.to_vec(),
            c,
        )?;
        let generation = if control {
            selected.binding.manifest().generation
        } else {
            selected.recovery_generation
        };
        if source.manifest().generation != generation
            || !source.manifest().same_software(selected.binding.manifest())
        {
            return Err(denied(
                "tree clock source differs from its independent generation/software pins",
            ));
        }
        let value = Self {
            source,
            config: config.clone(),
            token,
            control,
            high_tick: c.anchor.tick.get(),
        };
        value.check(false)?;
        Ok(value)
    }
    fn check(&self, write: bool) -> Result<()> {
        self.config.current()?;
        if std::env::var(TOKEN).ok().map(String::into_bytes).as_ref() != Some(&self.token) {
            return Err(denied("tree clock credential changed"));
        }
        if write && (!self.control || std::env::var(ALLOW).ok().as_deref() != Some("1")) {
            return Err(denied("independent tree clock permission denied"));
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
        self.source.observe(c)
    }
    fn prepare(&mut self, plan: &RunPlan, c: &OperationContext) -> Result<RunRecord> {
        self.check(true)?;
        self.source.restrict_deadline(c)?;
        let record = self.source.prepare(plan, c)?;
        self.high_tick = self.high_tick.max(record.observed_tick().unwrap_or(0));
        self.check(true)?;
        Ok(record)
    }
    fn commit(&mut self, plan: &RunPlan, c: &OperationContext) -> Result<RunRecord> {
        self.check(true)?;
        self.source.restrict_deadline(c)?;
        let record = self.source.commit(plan, c)?;
        self.high_tick = self.high_tick.max(record.observed_tick().unwrap_or(0));
        self.check(true)?;
        Ok(record)
    }
    fn query(&mut self, plan: &RunPlan, c: &OperationContext) -> Result<Option<RunRecord>> {
        self.check(false)?;
        self.source.restrict_deadline(c)?;
        let record = self.source.query(plan, c)?;
        self.high_tick = self.high_tick.max(
            record
                .as_ref()
                .and_then(RunRecord::observed_tick)
                .unwrap_or(0),
        );
        self.check(false)?;
        Ok(record)
    }
    fn cancel(&mut self, plan: &RunPlan, c: &OperationContext) -> Result<RunRecord> {
        self.check(true)?;
        self.source.restrict_deadline(c)?;
        let record = self.source.cancel(plan, c)?;
        self.high_tick = self.high_tick.max(record.observed_tick().unwrap_or(0));
        self.check(true)?;
        Ok(record)
    }
}

pub(super) enum Action<'a> {
    Plan(&'a str),
    Commit { seal: Digest32, confirmed: bool },
    Wait,
    Cancel,
}

impl Runtime {
    fn clock_context(&self, c: &OperationContext) -> OperationContext {
        let mut current = c.clone();
        current.anchor.tick = GameTick(
            self.high_tick
                .max(self.journal.high_tick)
                .max(self.clock.as_ref().map_or(0, Owner::high_tick)),
        );
        current
    }

    fn exact_tree_ack(&self) -> Result<()> {
        if self.acks.latest() != Some(self.journal.pin()) {
            return Err(error(
                ErrorCode::CorruptLedger,
                "original Tree journal and independent acknowledgement differ",
            ));
        }
        Ok(())
    }

    /// Uses the caller's just-verified Tree journal/ACK and separately verifies
    /// the Run chain. First binding additionally reconstructs the typed original.
    fn inspect_clock(
        &mut self,
        work: &mut TurnWork,
        c: &OperationContext,
    ) -> Result<Option<ClockView>> {
        if self.clock.is_none() {
            return Ok(None);
        }
        self.exact_tree_ack()?;
        let current = self.clock_context(c);
        let origin = if self
            .clock
            .as_ref()
            .is_some_and(|owner| owner.clock.is_none())
            && self
                .journal
                .native
                .as_ref()
                .is_some_and(|record| record.phase == 2)
        {
            Some(
                self.journal
                    .clock_origin(&mut self.acks, &work.reserve(&current, CLOCK_ORIGIN_BYTES)?)?,
            )
        } else {
            None
        };
        let view = self
            .clock
            .as_mut()
            .ok_or_else(|| denied("tree clock unavailable"))?
            .inspect(
                &self.config.binding,
                &self.journal,
                origin,
                &work.reserve(&current, 3 * VIEW_BYTES)?,
            )?;
        self.high_tick = self
            .high_tick
            .max(self.journal.high_tick)
            .max(self.clock.as_ref().map_or(0, Owner::high_tick));
        Ok(view)
    }

    /// Final publication includes the original Tree/ACK guard. This is also the
    /// legacy final guard when no optional clock owner is configured.
    pub(super) fn clock_view(
        &mut self,
        work: &mut TurnWork,
        c: &OperationContext,
        fresh: bool,
    ) -> Result<Value> {
        let current = self.clock_context(c);
        self.guard(&work.reserve(&current, TREE_LOCAL_BYTES)?, false)?;
        if self.clock.is_none() {
            return Ok(json!({"enabled":false}));
        }
        let view = self.inspect_clock(work, &current)?;
        let owner = self
            .clock
            .as_ref()
            .ok_or_else(|| denied("tree clock unavailable"))?;
        let ready = fresh
            && !self.journal.sample_pending
            && !self.journal.continuity_lost
            && self.journal.achieved_tick.is_none()
            && !self.journal.effect_uncertain
            && self.journal.samples <= MAX_SAMPLES - 3
            && self.journal.latest.as_ref().is_some_and(|progress| {
                progress.paused
                    && progress.generation == self.config.binding.generation
                    && progress.tick == self.high_tick
                    && !progress.supply_observed()
                    && (progress.root_predicate == 1
                        || (progress.root_predicate == 2 && progress.nearby_felling_jobs > 0))
            });
        let deadline = owner
            .clock
            .as_ref()
            .map(|clock| clock.target().deadline_tick());
        let remaining = deadline.map_or(0, |tick| tick.saturating_sub(self.high_tick));
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
            "recovery":recovery_json(owner.recovery),"deadline_tick":deadline,
            "pending":view.as_ref().is_some_and(|view|view.pending),
            "unresolved":view.as_ref().is_some_and(|view|view.unresolved),
            "active_run":view.as_ref().and_then(|view|view.active()).map(|run|run_json(run,true)),
            "run_count":view.as_ref().map_or(0,|view|view.runs.len()),
            "recent_runs":rows,"omitted_run_count":omitted,
            "commit_available":commit_available,"clock_ready":ready,"new_run_available":available,
            "cancel_available":owner.mode==RunMode::Control && std::env::var(ALLOW).ok().as_deref()==Some("1"),
            "advancement_fenced":owner.advancement_fenced,
            "remaining_tree_samples":MAX_SAMPLES.saturating_sub(self.journal.samples),
            "remaining_horizon_ticks":remaining,"recommended_game_ticks":if available {ticks} else {0},
            "high_tick":owner.high_tick(),"consumed_tick_floor":self.high_tick,
            "readiness":view.as_ref().and_then(|view|view.readiness.as_ref()).map(|ready|json!({
                "tick":ready.tick(),"witness":ready.digest().to_hex(),"root_predicate":ready.root_predicate(),
                "nearby_felling_jobs":ready.nearby_felling_jobs(),"wood_rows":ready.wood_rows(),
                "meaning":"exact_original_root_or_felling_observed_at_anchor","worker_readiness_proven":false})),
            "current_pause_proven":false,"harvest_goal_proven":false,"causal_wood_provenance":"unknown",
            "shared_process_identity_proven":false,
            "source_identity_semantics":"independently_pinned_run_incarnation_without_named_fortress_identity",
            "automatic_redispatch":false}),
        )
    }

    pub(super) fn clock_allows_goal_sample(
        &mut self,
        work: &mut TurnWork,
        c: &OperationContext,
    ) -> Result<()> {
        if self.clock.is_none() {
            return Ok(());
        }
        let current = self.clock_context(c);
        self.guard(&work.reserve(&current, TREE_LOCAL_BYTES)?, false)?;
        if self
            .inspect_clock(work, &current)?
            .is_some_and(|view| !view.can_sample_goal())
        {
            return Err(error(
                ErrorCode::EffectIndeterminate,
                "settle the original clock interval before sampling tree supply",
            ));
        }
        Ok(())
    }

    /// Reserve the entire independent clock and durable Tree sample work before
    /// either native source is entered. The owner is restored on every result.
    pub(super) fn clock_action(
        &mut self,
        action: Action<'_>,
        work: &mut TurnWork,
        c: &OperationContext,
    ) -> Result<bool> {
        let mut owner = self
            .clock
            .take()
            .ok_or_else(|| denied("tree clock is not configured"))?;
        let mut native_source_entered = false;
        let result = (|| {
            self.config.current()?;
            self.exact_tree_ack()?;
            let controls = matches!(action, Action::Plan(_) | Action::Commit { .. });
            let parsed = match &action {
                Action::Plan(raw) => Some(request(raw)?),
                _ => None,
            };
            // Include final Tree/ACK and Run publication in admission before
            // consuming an interval or contacting either native source.
            if controls
                && work.left
                    < CLOCK_ORIGIN_BYTES
                        + 6 * VIEW_BYTES
                        + GUARD_BYTES
                        + OPERATION_BYTES
                        + CONNECT_BYTES
                        + TREE_LOCAL_BYTES
            {
                return Err(exhausted());
            }
            if matches!(
                action,
                Action::Commit {
                    confirmed: false,
                    ..
                }
            ) {
                return Err(denied(
                    "review and confirm the exact independent tree clock interval",
                ));
            }
            let mut current = c.clone();
            current.anchor.tick = GameTick(
                self.high_tick
                    .max(self.journal.high_tick)
                    .max(owner.high_tick()),
            );
            let origin = if controls || owner.clock.is_none() {
                if self
                    .journal
                    .native
                    .as_ref()
                    .is_none_or(|record| record.phase != 2)
                {
                    return Err(denied(
                        "clock requires the exact acknowledged original tree designation",
                    ));
                }
                let origin = self
                    .journal
                    .clock_origin(&mut self.acks, &work.reserve(&current, CLOCK_ORIGIN_BYTES)?)?;
                self.high_tick = self.high_tick.max(origin.high_tick());
                if controls {
                    if owner.mode != RunMode::Control
                        || owner.advancement_fenced
                        || std::env::var(ALLOW).ok().as_deref() != Some("1")
                        || origin.pending_sample()
                        || origin.continuity_lost()
                        || origin.achieved_tick().is_some()
                        || self.journal.effect_uncertain
                    {
                        return Err(denied(
                            "clock requires a continuous unfinished original and independent current authority",
                        ));
                    }
                    origin.require_capacity(if matches!(action, Action::Plan(_)) {
                        3
                    } else {
                        2
                    })?;
                }
                Some(origin)
            } else {
                None
            };
            current.anchor.tick = GameTick(
                self.high_tick
                    .max(self.journal.high_tick)
                    .max(owner.high_tick()),
            );
            let before = owner
                .inspect(
                    &self.config.binding,
                    &self.journal,
                    origin,
                    &work.reserve(&current, 3 * VIEW_BYTES)?,
                )?
                .ok_or_else(|| denied("original tree clock binding unavailable"))?;
            self.high_tick = self
                .high_tick
                .max(owner.high_tick())
                .max(self.journal.high_tick);
            current.anchor.tick = GameTick(self.high_tick);
            if let Some(request) = &parsed {
                if request.target_id != before.target_id {
                    return Err(error(
                        ErrorCode::StaleAnchor,
                        "clock request names another original tree",
                    ));
                }
                if u64::from(request.game_ticks) > c.budget.max_game_ticks
                    || self
                        .high_tick
                        .checked_add(u64::from(request.game_ticks))
                        .is_none_or(|end| end > before.deadline_tick)
                {
                    return Err(exhausted());
                }
            }
            // Read-only settled recovery is available without either credential.
            if matches!(action, Action::Wait) && !before.needs_reconciliation() {
                return Ok(before.can_sample_goal());
            }
            if self.mode == Mode::Offline {
                return Err(denied("offline tree clock cannot contact native sources"));
            }
            let guard_reservation = if controls {
                Some(work.reserve(&current, GUARD_BYTES)?)
            } else {
                None
            };
            let operation = work.reserve(&current, OPERATION_BYTES)?;
            let guard_started = Instant::now();
            let connect = work.reserve(&current, CONNECT_BYTES)?;
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
            let mut current = work.reserve(&current, 0)?;
            // The prepaid operation keeps its bytes; only the original deadline
            // is narrowed here. A zero-byte context is never authorized.
            current.budget.max_bytes = operation.budget.max_bytes;
            current.anchor.tick = GameTick(self.high_tick.max(owner.high_tick()));
            let native_context = native_context(&current, &owner.grants);
            let clock = owner
                .clock
                .as_mut()
                .ok_or_else(|| denied("original clock binding unavailable"))?;
            let target = clock.target().clone();
            let outcome = match action {
                Action::Plan(_) => clock.prepare_guarded(
                    native
                        .as_mut()
                        .ok_or_else(|| denied("clock source absent"))?,
                    &parsed.ok_or_else(|| invalid("clock request absent"))?,
                    &current,
                    &native_context,
                    |_, plan, edge| {
                        original_progress(
                            &self.config,
                            &mut self.journal,
                            &mut self.acks,
                            &mut self.high_tick,
                            &mut self.high_sequence,
                            &target,
                            plan,
                            edge,
                            guard_reservation.as_ref(),
                            guard_started,
                        )
                    },
                ),
                Action::Commit { seal, confirmed } => clock.commit_guarded(
                    native
                        .as_mut()
                        .ok_or_else(|| denied("clock source absent"))?,
                    seal,
                    confirmed,
                    &current,
                    &native_context,
                    |_, plan, edge| {
                        original_progress(
                            &self.config,
                            &mut self.journal,
                            &mut self.acks,
                            &mut self.high_tick,
                            &mut self.high_sequence,
                            &target,
                            plan,
                            edge,
                            guard_reservation.as_ref(),
                            guard_started,
                        )
                    },
                ),
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
            self.high_tick = self
                .high_tick
                .max(clock.high_tick())
                .max(self.journal.high_tick)
                .max(native.as_ref().map_or(0, |source| source.high_tick));
            let view = outcome?;
            owner.verified = view.verified;
            Ok(view.can_sample_goal())
        })();
        if result.is_err() && native_source_entered {
            owner.fence_advancement();
        }
        owner.retain();
        self.high_tick = self
            .high_tick
            .max(owner.high_tick())
            .max(self.journal.high_tick);
        self.clock = Some(owner);
        result
    }
}

fn same_original(origin: &TreeClockOrigin, target: &Target) -> bool {
    origin.pin().journal_id == target.tree_journal_id()
        && origin.binding() == target.binding()
        && origin.original() == target.original()
        && origin.record() == target.record()
}

/// Each Run boundary writes a real original progress sample and independent
/// acknowledgement. A failed publication retains its pending latch/floors.
#[allow(clippy::too_many_arguments)]
fn original_progress(
    config: &TreeConfig,
    journal: &mut Journal<PrivateBuildFile>,
    acks: &mut WorkshopAcknowledgements<PrivateBuildFile>,
    high_tick: &mut u64,
    high_sequence: &mut u64,
    target: &Target,
    plan: &RunPlan,
    c: &OperationContext,
    reserved: Option<&OperationContext>,
    started: Instant,
) -> Result<Progress> {
    let reserved = reserved.ok_or_else(exhausted)?;
    let mut remaining = reserved.budget.max_bytes;
    let mut part = |bytes: u64, floor: u64| -> Result<OperationContext> {
        remaining = remaining.checked_sub(bytes).ok_or_else(exhausted)?;
        let mut current = c.clone();
        current.anchor.tick = GameTick(current.anchor.tick.get().max(floor));
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
            "independent clock permission revoked before original tree guard",
        ));
    }
    let before = journal.clock_origin(acks, &part(CLOCK_ORIGIN_BYTES, *high_tick)?)?;
    *high_tick = (*high_tick).max(before.high_tick());
    if !same_original(&before, target)
        || before.pending_sample()
        || before.continuity_lost()
        || before.achieved_tick().is_some()
        || plan.before().tick().is_none_or(|tick| tick < *high_tick)
    {
        return Err(denied(
            "clock guard requires the same continuous unfinished original tree",
        ));
    }
    before.require_capacity(1)?;
    journal.begin_sample(&part(TREE_LOCAL_BYTES, *high_tick)?, acks)?;
    let mut source = config.connect(&part(TREE_CONNECT_BYTES, *high_tick)?, Mode::Recover)?;
    source.require_progress_at_or_after((*high_tick).max(plan.before().tick().unwrap_or(0)));
    let progress = match source.progress(
        &target.original().plan,
        &part(TREE_CONNECT_BYTES, *high_tick)?,
    ) {
        Ok(progress) => progress,
        Err(cause) => {
            if let Some(progress) = source.consumed_progress.take() {
                *high_tick = (*high_tick).max(progress.tick);
                *high_sequence = (*high_sequence).max(progress.sequence);
                journal.consume_uncredited(progress, &part(TREE_LOCAL_BYTES, *high_tick)?, acks)?;
            }
            return Err(cause);
        }
    };
    *high_tick = (*high_tick).max(progress.tick);
    *high_sequence = (*high_sequence).max(progress.sequence);
    journal.sample(progress.clone(), &part(TREE_LOCAL_BYTES, *high_tick)?, acks)?;
    let after = journal.clock_origin(acks, &part(CLOCK_ORIGIN_BYTES, *high_tick)?)?;
    config.current()?;
    if !same_original(&after, target)
        || after.samples() != before.samples() + 1
        || after.pending_sample()
        || after.continuity_lost()
        || after.achieved_tick().is_some()
    {
        return Err(denied(
            "fresh original tree evidence no longer supports clock advancement",
        ));
    }
    Ok(progress)
}

#[cfg(test)]
#[path = "clock_config_tests.rs"]
mod clock_config_tests;
