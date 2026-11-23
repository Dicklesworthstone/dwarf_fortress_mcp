//! One foreground owner for real Tree, Workshop and combined-production runtimes.
//! Child custody remains effect authority; the parent owns stage order and stop policy.
use super::config::{Config, runtime_io};
use super::continuity;
use super::error;
use super::journal::{self, Checkpoint, Mode, Stage};
use crate::live_production_runtime::{
    Config as ProductionConfig, Mode as ProductionMode, ObserveRequest, ProjectProductionIntent,
    Runtime as ProductionRuntime, StockTargetRequest,
};
use crate::semantic_work_order_runtime::clock::RunRequest;
use crate::semantic_work_order_runtime::inventory_clock::RunRequest as InventoryRunRequest;
use crate::tree_harvest_server::{
    ProjectTreeAction as TreeAction, ProjectTreeRuntime as TreeRuntime, TreeProjectHandoff,
};
use crate::workshop_construction_server::{
    ProjectWorkshopAction as WorkshopAction, ProjectWorkshopRuntime as WorkshopRuntime,
    ProjectWorkshopTemplate, WorkshopProjectHandoff,
};
use dfmcp_adapter::tree_harvest::journal::Mode as TreeMode;
use dfmcp_adapter::workshop_construction::journal::WorkshopMode;
use dfmcp_core::{
    Capability, Digest32, ErrorCode, GameTick, OperationContext, RequestId, Result, RiskTier,
    SessionId, WorkBudget,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::time::Instant;

pub(super) const RESPONSE_BYTES: u64 = 256 * 1024;
pub(super) const MAX_BYTES: u64 = 32 * 1024 * 1024 * 1024;
const WORKSHOP_BYTES: u64 = 512 * 1024 * 1024;

#[path = "tree.rs"]
mod tree;

fn invalid(message: &str) -> dfmcp_core::DfmcpError {
    error(ErrorCode::InvalidRequest, message)
}
fn denied(message: &str) -> dfmcp_core::DfmcpError {
    error(ErrorCode::CapabilityDenied, message)
}
fn exhausted() -> dfmcp_core::DfmcpError {
    error(
        ErrorCode::BudgetExceeded,
        "resource project work or output bound exhausted",
    )
}
fn digest(raw: &str) -> Result<Digest32> {
    if raw.len() != 64
        || !raw
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(invalid(
            "project digest requires 64 lowercase hexadecimal characters",
        ));
    }
    Digest32::from_hex(raw)
        .filter(|d| *d != Digest32::ZERO)
        .ok_or_else(|| invalid("invalid project digest"))
}
fn workshop_mode(mode: Mode) -> WorkshopMode {
    match mode {
        Mode::Control => WorkshopMode::Control,
        Mode::Recover => WorkshopMode::Recover,
        Mode::Offline => WorkshopMode::Offline,
    }
}
fn tree_mode(mode: Mode) -> TreeMode {
    match mode {
        Mode::Control => TreeMode::Control,
        Mode::Recover => TreeMode::Recover,
        Mode::Offline => TreeMode::Offline,
    }
}
fn production_mode(mode: Mode) -> ProductionMode {
    match mode {
        Mode::Control => ProductionMode::Control,
        Mode::Recover => ProductionMode::Reconcile,
        Mode::Offline => ProductionMode::Offline,
    }
}

pub(super) enum Action {
    Observe(Option<String>),
    Inspect,
    Plan(String),
    Commit {
        seal: String,
        confirmed: bool,
        scope: Option<String>,
    },
    Wait(Option<String>),
    Cancel(Option<String>),
}

struct Work {
    start: Instant,
    ceiling: WorkBudget,
    bytes: u64,
}
impl Work {
    fn new(ceiling: WorkBudget, start: Instant) -> Result<Self> {
        ceiling.validate()?;
        Ok(Self {
            start,
            ceiling,
            bytes: ceiling
                .max_bytes
                .checked_sub(RESPONSE_BYTES)
                .ok_or_else(exhausted)?,
        })
    }
    fn remaining(&self) -> Result<WorkBudget> {
        runtime_io()?;
        let elapsed = u64::try_from(self.start.elapsed().as_millis()).map_err(|_| exhausted())?;
        let mut budget = self.ceiling;
        budget.max_wall_millis = budget
            .max_wall_millis
            .checked_sub(elapsed)
            .filter(|millis| *millis > 0)
            .ok_or_else(exhausted)?;
        budget.max_bytes = self.bytes;
        Ok(budget)
    }
    fn reserve(&mut self, base: &OperationContext, bytes: u64) -> Result<OperationContext> {
        let mut context = base.clone();
        context.budget = self.remaining()?;
        self.bytes = self.bytes.checked_sub(bytes).ok_or_else(exhausted)?;
        context.budget.max_bytes = bytes;
        Ok(context)
    }
    fn child(&mut self, maximum: u64, reserve_after: u64) -> Result<WorkBudget> {
        let bytes = self
            .bytes
            .checked_sub(reserve_after)
            .filter(|n| *n > 0)
            .ok_or_else(exhausted)?
            .min(maximum);
        let mut budget = self.remaining()?;
        self.bytes -= bytes;
        budget.max_bytes = bytes;
        Ok(budget)
    }
    fn accounting(&self) -> Value {
        json!({
            "max_wall_millis":self.ceiling.max_wall_millis,
            "max_bytes":self.ceiling.max_bytes,
            "max_output_tokens":self.ceiling.max_output_tokens,
            "reserved_byte_work":self.ceiling.max_bytes - self.bytes,
            "remaining_byte_work":self.bytes,
            "output_bytes_reserved":RESPONSE_BYTES,
            "max_game_ticks":self.ceiling.max_game_ticks,
            "accounting":"child work is prepaid at its narrowed ceiling; unused reservations are not recycled",
        })
    }
}

enum Child {
    Tree(Box<TreeRuntime>),
    Workshop(Box<WorkshopRuntime>),
    Production(Box<ProductionRuntime>),
}
impl Child {
    fn reference(&self) -> Value {
        match self {
            Self::Tree(child) => child.project_recovery_reference(),
            Self::Workshop(child) => child.project_recovery_reference(),
            Self::Production(child) => child.project_recovery_reference(),
        }
    }
    fn withdraw(&mut self) {
        match self {
            Self::Tree(child) => child.withdraw_permission(),
            Self::Workshop(child) => child.withdraw(),
            Self::Production(child) => child.abandon_preparation(),
        }
    }
}

pub(super) struct Runtime {
    config: Config,
    workshop_config: ProjectWorkshopTemplate,
    journal: journal::PrivateJournal,
    child: Option<Child>,
    mode: Mode,
    pub(super) session: SessionId,
    budget: WorkBudget,
    request: u128,
    high_tick: u64,
    fenced: bool,
    last_child_reference: Value,
    current_goal: Value,
    accounting: Value,
    publication_started: Instant,
}

impl Runtime {
    pub(super) fn open(
        config: Config,
        workshop_config: ProjectWorkshopTemplate,
        mode: Mode,
        session: SessionId,
        budget: WorkBudget,
        start: Instant,
    ) -> Result<(Self, Result<Value>)> {
        if config.initialize && mode != Mode::Control {
            return Err(denied(
                "initializing a resource project requires explicit control mode",
            ));
        }
        let mut work = Work::new(budget, start)?;
        let mut context = workshop_config.project_context(
            workshop_mode(mode),
            session,
            RequestId::new(1),
            work.remaining()?,
        )?;
        context.anchor.tick = GameTick(
            config
                .acknowledged
                .as_ref()
                .map_or(0, |pin| pin.authority_tick_floor),
        );
        let ledger_context = work.reserve(&context, journal::WORK_BYTES)?;
        // The durable opening marker precedes even the first child open.
        let journal = journal::open_private(
            &config.directory,
            &ledger_context,
            mode,
            config.initialize,
            config.original.clone(),
            config.acknowledged.clone(),
            if config.tree.is_some() {
                Stage::TreeOpening
            } else {
                Stage::WorkshopOpening
            },
        )?;
        let high_tick = journal.pin().authority_tick_floor;
        let mut runtime = Self {
            config,
            workshop_config,
            journal,
            child: None,
            mode,
            session,
            budget,
            request: 1,
            high_tick,
            fenced: false,
            last_child_reference: Value::Null,
            current_goal: Value::Null,
            accounting: Value::Null,
            publication_started: start,
        };
        let opened = runtime.open_current(&mut work);
        runtime.accounting = work.accounting();
        let value = match opened {
            Ok(child) => match work.remaining().and_then(|_| runtime.config.current()) {
                Ok(()) => Ok(runtime.result(child)),
                Err(e) => {
                    runtime.withdraw();
                    Err(e)
                }
            },
            Err(e) => {
                runtime.fenced = true;
                runtime.withdraw();
                Err(e)
            }
        };
        // Keep the owner after a child-opening failure. Its durable marker and
        // any surviving child custody must remain inspectable and closeable.
        Ok((runtime, value))
    }

    pub(super) fn last_request_id(&self) -> RequestId {
        RequestId::new(self.request)
    }

    pub(super) fn publication_current(&self) -> Result<()> {
        self.config.current()?;
        if self.publication_started.elapsed().as_millis() >= u128::from(self.budget.max_wall_millis)
        {
            return Err(exhausted());
        }
        Ok(())
    }

    fn context(&self, work: &Work) -> Result<OperationContext> {
        self.config.current()?;
        let mut context = self.workshop_config.project_context(
            workshop_mode(self.mode),
            self.session,
            self.last_request_id(),
            work.remaining()?,
        )?;
        context.anchor.tick = GameTick(self.high_tick.max(self.journal.pin().authority_tick_floor));
        Ok(context)
    }

    fn preflight(&mut self, work: &mut Work, read_only: bool) -> Result<()> {
        let base = self.context(work)?;
        let current = work.reserve(&base, journal::WORK_BYTES)?;
        if self.mode == Mode::Offline || read_only {
            self.journal.verify(&current)
        } else {
            self.journal.preflight_append(&current)
        }
    }

    fn effective_stage(&self) -> Stage {
        let snapshot = self.journal.snapshot();
        snapshot.cancelled_from.unwrap_or(snapshot.stage)
    }

    fn cancelled(&self) -> bool {
        self.journal.snapshot().stage == Stage::Cancelled
    }

    fn authorize_forward(&self, work: &Work) -> Result<()> {
        if self.mode != Mode::Control || self.cancelled() || self.fenced || self.journal.is_fenced()
        {
            return Err(denied("project is stopped, fenced or not in control mode"));
        }
        let current = self.context(work)?;
        if current.anchor.tick.get() >= self.config.goal.deadline_tick {
            return Err(denied("original resource project deadline has passed"));
        }
        current.authorize(Capability::Plan, RiskTier::Guarded, &[], None)
    }

    fn handoff(&self) -> Result<WorkshopProjectHandoff> {
        let value = self
            .journal
            .snapshot()
            .checkpoint
            .handoff
            .as_ref()
            .ok_or_else(|| denied("project has no acknowledged Workshop handoff"))?;
        let handoff = WorkshopProjectHandoff::from_json(value)?;
        if handoff.project_deadline_tick() != self.config.goal.deadline_tick
            || handoff
                .clock_deadline_tick()
                .is_some_and(|deadline| deadline != self.config.goal.deadline_tick)
            || self
                .config
                .handoff
                .as_ref()
                .is_some_and(|configured| configured.to_json() != *value)
        {
            return Err(denied(
                "project handoff or original deadline changed on recovery",
            ));
        }
        Ok(handoff)
    }

    fn append(&mut self, work: &mut Work, stage: Stage, checkpoint: Checkpoint) -> Result<()> {
        let base = self.context(work)?;
        let current = work.reserve(&base, journal::WORK_BYTES)?;
        self.journal.advance(&current, stage, checkpoint)?;
        Ok(())
    }

    fn save_child(&mut self, work: &mut Work, activation: Option<Stage>) -> Result<()> {
        let Some(child) = &self.child else {
            return Ok(());
        };
        let reference = child.reference();
        self.high_tick = self.high_tick.max(continuity::tick(&reference));
        self.last_child_reference = reference.clone();
        // Inspection after a refused recovery still reaches this method. Keep
        // rejection floors in memory, but never acknowledge a foreign Tree
        // original whose deadline was absent from the earlier checkpoint.
        if let Child::Tree(child) = child {
            child.validate_project_original_policy()?;
        }
        let mut checkpoint = self.journal.snapshot().checkpoint.clone();
        let slot = match child {
            Child::Tree(_) => &mut checkpoint.tree,
            Child::Workshop(_) => &mut checkpoint.workshop,
            Child::Production(_) => &mut checkpoint.production,
        };
        if let Some(retained) = slot.as_ref()
            && let Err(e) = continuity::same_original(retained, &reference)
        {
            self.fenced = true;
            return Err(e);
        }
        *slot = Some(reference);
        let stage = if self.cancelled() {
            Stage::Cancelled
        } else {
            activation.unwrap_or(self.journal.snapshot().stage)
        };
        if self.mode != Mode::Offline
            && (checkpoint != self.journal.snapshot().checkpoint
                || stage != self.journal.snapshot().stage)
        {
            self.append(work, stage, checkpoint)?;
        }
        Ok(())
    }

    fn workshop_budget(&self, work: &mut Work, reserve_after: u64) -> Result<WorkBudget> {
        let mut budget = work.child(WORKSHOP_BYTES, reserve_after)?;
        budget.max_actions = 1;
        budget.max_entities = budget.max_entities.min(4096);
        budget.max_output_tokens = budget.max_output_tokens.min(32_768);
        if !self.workshop_config.project_clock_configured() {
            budget.max_game_ticks = 0;
        }
        Ok(budget)
    }

    fn open_current(&mut self, work: &mut Work) -> Result<Value> {
        self.preflight(work, self.cancelled())?;
        if self.cancelled()
            && matches!(
                self.effective_stage(),
                Stage::TreeOpening | Stage::WorkshopOpening | Stage::ProductionOpening
            )
        {
            // Opening never dispatches a native effect. A cancelled opening is
            // a terminal local marker; recovery must not activate that child.
            return Ok(json!({"opened":true,"local_parent_only":true,
                "project_cancelled":true,"child_initialization_not_replayed":true}));
        }
        // An optional Tree prefix is part of the immutable original, including
        // when reopening a later child. Validate its retained handoff before
        // touching Workshop or production custody.
        if self.config.tree.is_some()
            && !matches!(self.effective_stage(), Stage::TreeOpening | Stage::Tree)
        {
            self.tree_handoff()?;
        }
        match self.effective_stage() {
            Stage::TreeOpening | Stage::Tree => self.open_tree(work)?,
            Stage::TreeTransition => {
                self.tree_handoff()?;
            }
            Stage::WorkshopOpening | Stage::Workshop => {
                let tree_handoff = if self.config.tree.is_some() {
                    Some(self.tree_handoff()?)
                } else {
                    None
                };
                let config = self.workshop_config.activate(tree_handoff.as_ref())?;
                if (!self.config.initialize && config.project_initializing())
                    || self.config.handoff.is_some()
                {
                    return Err(denied(
                        "recovery cannot initialize or substitute the original Workshop child",
                    ));
                }
                let prior = self.journal.snapshot().checkpoint.workshop.clone();
                if let Some(reference) = &prior {
                    config.validate_project_recovery(reference)?;
                }
                let budget = self.workshop_budget(work, journal::WORK_BYTES)?;
                let child = WorkshopRuntime::open_for_resource_project(
                    config,
                    workshop_mode(self.mode),
                    self.session,
                    budget,
                    self.high_tick,
                )?;
                self.child = Some(Child::Workshop(Box::new(child)));
                if prior.is_none()
                    && let Some(Child::Workshop(child)) = &self.child
                    && child.project_has_original_history()
                {
                    return Err(denied(
                        "an unacknowledged Workshop opening cannot adopt preexisting original or clock history",
                    ));
                }
                if let Some(reference) = &prior
                    && let Some(Child::Workshop(child)) = &mut self.child
                {
                    // Recovery open performs local custody reads only. Merge
                    // retained source floors and sticky clock fences before
                    // permitting any native contact; a newer descendant wins.
                    child.retain_project_recovery(reference)?;
                    continuity::same_original(reference, &child.project_recovery_reference())?;
                }
                self.save_child(work, Some(Stage::Workshop))?;
            }
            Stage::Transition => {
                self.handoff()?;
            }
            Stage::ProductionOpening | Stage::Production => {
                let handoff = self.handoff()?;
                let config = ProductionConfig::from_project_environment(
                    self.config.guard(),
                    runtime_io,
                    &handoff,
                )?;
                if config.initialize {
                    return Err(denied(
                        "a retained production opening requires existing custody; initialization is never replayed",
                    ));
                }
                self.open_production(config, work)?;
            }
            Stage::Cancelled => {
                return Err(denied("cancelled project lost its original active stage"));
            }
        }
        Ok(json!({
            "opened":true, "native_effect_dispatched":false,
            "preparation_restored":false, "fresh_review_required":true,
            "child":self.last_child_reference,
        }))
    }

    fn open_production(&mut self, mut config: ProductionConfig, work: &mut Work) -> Result<()> {
        let intent = self
            .journal
            .snapshot()
            .checkpoint
            .production_intent
            .as_ref()
            .map(ProjectProductionIntent::from_json)
            .transpose()?;
        if let Some(intent) = &intent {
            intent.validate_goal(&self.config.goal)?;
        }
        if let Some(reference) = self.journal.snapshot().checkpoint.production.as_ref() {
            if let Some(intent) = &intent {
                // The independently acknowledged intent supplies the exact
                // original selectors when a child write outlived its parent
                // checkpoint. Missing or partial files still refuse to open.
                config.recover_project_publication(reference, intent)?;
            } else {
                config.validate_project_recovery(reference)?;
            }
        } else if config.recovery.is_some() || intent.is_some() {
            return Err(denied(
                "an unacknowledged production opening cannot adopt a selected preexisting original",
            ));
        }
        let mut budget = work.child(MAX_BYTES, journal::WORK_BYTES)?;
        budget.max_output_tokens = budget.max_output_tokens.min(32_768);
        if config.clock.is_none() && config.inventory_clock.is_none() {
            budget.max_game_ticks = 0;
        }
        let child =
            ProductionRuntime::open(config, production_mode(self.mode), self.session, budget)?;
        // Retain the real owner before any parent comparison can fail.
        self.child = Some(Child::Production(Box::new(child)));
        let Some(Child::Production(child)) = &self.child else {
            return Err(error(
                ErrorCode::InternalInvariantViolation,
                "production owner not retained",
            ));
        };
        // Authenticate the actual recovered original and fixed parent goal
        // before acknowledging its adoption, including legacy checkpoints.
        let status = child.project_status(&self.config.goal)?;
        if let Some(intent) = &intent {
            child.validate_project_publication(&self.config.goal, intent)?;
        }
        self.save_child(work, Some(Stage::Production))?;
        self.current_goal = status.to_json();
        Ok(())
    }

    fn plan_production_target(&mut self, raw: &str, work: &mut Work) -> Result<Value> {
        let request = StockTargetRequest::parse(raw)?;
        self.config.goal.validate_request(&request)?;
        if self.journal.snapshot().stage != Stage::Production
            || self
                .journal
                .snapshot()
                .checkpoint
                .production_intent
                .is_some()
        {
            return Err(denied(
                "retain or recover the recorded production original; never compile a replacement",
            ));
        }
        let prior = self
            .journal
            .snapshot()
            .checkpoint
            .production
            .as_ref()
            .ok_or_else(|| denied("production planning requires acknowledged native custody"))?;
        if !prior["recovery"].is_null() {
            return Err(denied(
                "the project already owns its original production plan",
            ));
        }
        let prior = prior.clone();
        let base = self.context(work)?;
        // Both parent publications are prepaid before any child source read.
        // The first reservation cannot be reused by the child or final save.
        let reserved = work.reserve(&base, journal::WORK_BYTES)?;
        let mut budget = work.child(MAX_BYTES, journal::WORK_BYTES)?;
        let Some(Child::Production(child)) = &mut self.child else {
            return Err(denied("original production owner is unavailable"));
        };
        let ceiling = child.budget();
        budget.max_wall_millis = budget.max_wall_millis.min(ceiling.max_wall_millis);
        budget.max_bytes = budget.max_bytes.min(ceiling.max_bytes);
        budget.max_entities = budget.max_entities.min(ceiling.max_entities);
        budget.max_actions = budget.max_actions.min(ceiling.max_actions);
        budget.max_output_tokens = budget.max_output_tokens.min(ceiling.max_output_tokens);
        budget.max_game_ticks = budget.max_game_ticks.min(ceiling.max_game_ticks);
        let mut current = child.context(false)?;
        current.budget = budget;
        let config = &self.config;
        let journal = &mut self.journal;
        let high_tick = &mut self.high_tick;
        let expected_head = journal.pin().head;
        let mut record = |intent: &ProjectProductionIntent, consumed: &OperationContext| {
            *high_tick = (*high_tick)
                .max(intent.authority_tick_floor())
                .max(consumed.anchor.tick.get());
            config.current()?;
            intent.validate_goal(&config.goal)?;
            intent.validate_checkpoint(&prior)?;
            if journal.pin().head != expected_head
                || journal.snapshot().stage != Stage::Production
                || journal.snapshot().checkpoint.production_intent.is_some()
                || *high_tick >= config.goal.deadline_tick
            {
                return Err(denied(
                    "production publication lost its exact parent head, original deadline or opening slot",
                ));
            }
            let mut context = reserved.clone();
            context.budget.max_wall_millis = context
                .budget
                .max_wall_millis
                .min(work.remaining()?.max_wall_millis);
            context.anchor.tick = context.anchor.tick.max(GameTick(*high_tick));
            context.authorize(Capability::Plan, RiskTier::Guarded, &[], None)?;
            let mut checkpoint = journal.snapshot().checkpoint.clone();
            checkpoint.production_intent = Some(intent.to_json());
            journal.advance(&context, Stage::Production, checkpoint)?;
            config.current()?;
            work.remaining()?;
            Ok(())
        };
        let view = child.project_plan_stock_target_recorded(
            &config.goal,
            request,
            &current,
            &mut record,
        )?;
        self.current_goal = child.project_status(&config.goal)?.to_json();
        Ok(view.to_json())
    }

    fn advance_to_production(&mut self, raw: &str, work: &mut Work) -> Result<Value> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Advance {
            kind: String,
            expected_head: String,
        }
        let request: Advance = serde_json::from_str(raw)
            .map_err(|_| invalid("invalid closed project transition request"))?;
        if request.kind != "advance_to_production"
            || request.expected_head != self.journal.pin().head
        {
            return Err(denied(
                "project transition requires the exact current parent head",
            ));
        }
        self.authorize_forward(work)?;
        if self.effective_stage() == Stage::Workshop {
            let budget = self.workshop_budget(work, 4 * journal::WORK_BYTES)?;
            let Some(Child::Workshop(child)) = &mut self.child else {
                return Err(denied("original Workshop owner is unavailable"));
            };
            child.retain_project_authority_tick(self.high_tick)?;
            let handoff = child.project_handoff(budget)?;
            // Handoff consumed a fresh native sample. Retain its floors before
            // freezing Workshop custody in the next durable stage.
            self.save_child(work, None)?;
            self.authorize_forward(work)?;
            let mut checkpoint = self.journal.snapshot().checkpoint.clone();
            checkpoint.handoff = Some(handoff.to_json());
            self.append(work, Stage::Transition, checkpoint)?;
            // The acknowledged transition is durable before dropping the
            // exclusive writer. Production reopens that exact donor Offline.
            if let Some(child) = &mut self.child {
                child.withdraw();
            }
            self.child = None;
        }
        if self.effective_stage() != Stage::Transition {
            return Err(denied(
                "project cannot skip, repeat or replace a child stage",
            ));
        }
        let handoff = self.handoff()?;
        let config =
            ProductionConfig::from_project_environment(self.config.guard(), runtime_io, &handoff)?;
        if !config.initialize {
            return Err(denied(
                "first production entry requires explicit fresh custody; an opening marker is recovered separately",
            ));
        }
        self.authorize_forward(work)?;
        self.append(
            work,
            Stage::ProductionOpening,
            self.journal.snapshot().checkpoint.clone(),
        )?;
        // A lost opening reply leaves this marker. It never authorizes another
        // initialization attempt, even if no native effect was requested.
        self.open_production(config, work)?;
        Ok(json!({
            "advanced_to":"production", "native_effect_dispatched":false,
            "observe_required":true, "fresh_review_required":true,
            "child":self.last_child_reference,
        }))
    }

    fn child_call(&mut self, action: Action, work: &mut Work) -> Result<Value> {
        let mut stock_target = false;
        if let Action::Plan(raw) = &action {
            if raw.len() > 4096 {
                return Err(exhausted());
            }
            let value: Value =
                serde_json::from_str(raw).map_err(|_| invalid("invalid project plan request"))?;
            if value["kind"] == "advance_to_production" {
                return self.advance_to_production(raw, work);
            }
            if value["kind"] == "advance_to_workshop" {
                return self.advance_to_workshop(raw, work);
            }
            stock_target = value["kind"] == "stock_target";
        }
        if matches!(&action, Action::Plan(_) | Action::Commit { .. }) {
            self.authorize_forward(work)?;
        }
        if self.fenced {
            if matches!(&action, Action::Inspect) {
                return Ok(
                    json!({"local_parent_only":true,"child_custody_currently_verified":false}),
                );
            }
            return Err(denied(
                "project owner is fenced; preserve exact custody and reopen recovery",
            ));
        }
        if self.child.is_none() {
            return match action {
                Action::Inspect | Action::Wait(_) | Action::Cancel(_) => Ok(json!({
                    "local_parent_only":true, "child_opening_may_be_incomplete":!matches!(self.effective_stage(), Stage::TreeTransition | Stage::Transition),
                    "native_effects_cancelled":false,
                })),
                _ => Err(denied(
                    "project has no active child; inspect its acknowledged stage",
                )),
            };
        }
        if stock_target && matches!(&self.child, Some(Child::Production(_))) {
            return self.plan_production_target(
                match &action {
                    Action::Plan(raw) => raw,
                    _ => {
                        return Err(error(
                            ErrorCode::InternalInvariantViolation,
                            "production plan selector lost its request",
                        ));
                    }
                },
                work,
            );
        }
        let mut budget = work.child(
            if matches!(&self.child, Some(Child::Tree(_) | Child::Workshop(_))) {
                WORKSHOP_BYTES
            } else {
                MAX_BYTES
            },
            journal::WORK_BYTES,
        )?;
        budget.max_output_tokens = budget.max_output_tokens.min(32_768);
        let goal = self.config.goal.clone();
        let offline = self.mode == Mode::Offline;
        match self.child.as_mut() {
            Some(Child::Tree(child)) => {
                child.retain_project_authority_tick(self.high_tick)?;
                budget.max_actions = 1;
                budget.max_entities = budget.max_entities.min(4096);
                if !self
                    .config
                    .tree
                    .as_ref()
                    .is_some_and(|config| config.project_clock_configured())
                {
                    budget.max_game_ticks = 0;
                }
                child.perform_project(tree::action(action, offline)?, budget)
            }
            Some(Child::Workshop(child)) => {
                child.retain_project_authority_tick(self.high_tick)?;
                budget.max_actions = 1;
                if !self.workshop_config.project_clock_configured() {
                    budget.max_game_ticks = 0;
                }
                let action = match action {
                    Action::Observe(None) => WorkshopAction::Observe,
                    Action::Observe(Some(_)) => {
                        return Err(invalid(
                            "Workshop observation has no caller-selected source",
                        ));
                    }
                    Action::Inspect => WorkshopAction::Inspect,
                    Action::Plan(raw) => WorkshopAction::Plan(raw),
                    Action::Commit {
                        seal,
                        confirmed,
                        scope,
                    } => {
                        if !matches!(scope.as_deref().unwrap_or("goal"), "goal" | "clock") {
                            return Err(invalid("unknown Workshop commit scope"));
                        }
                        WorkshopAction::Commit { seal, confirmed }
                    }
                    Action::Wait(scope) => match scope.as_deref().unwrap_or("goal") {
                        "goal" | "project" if offline => WorkshopAction::Inspect,
                        "goal" | "project" => WorkshopAction::Wait,
                        "clock" => WorkshopAction::WaitClock,
                        _ => return Err(invalid("unknown Workshop wait scope")),
                    },
                    Action::Cancel(scope) => match scope.as_deref().unwrap_or("project") {
                        "goal" | "project" => WorkshopAction::Cancel,
                        "clock" => WorkshopAction::CancelClock,
                        _ => return Err(invalid("unknown Workshop cancellation scope")),
                    },
                };
                child.perform_project(action, budget)
            }
            Some(Child::Production(child)) => {
                let ceiling = child.budget();
                budget.max_wall_millis = budget.max_wall_millis.min(ceiling.max_wall_millis);
                budget.max_bytes = budget.max_bytes.min(ceiling.max_bytes);
                budget.max_entities = budget.max_entities.min(ceiling.max_entities);
                budget.max_actions = budget.max_actions.min(ceiling.max_actions);
                budget.max_output_tokens = budget.max_output_tokens.min(ceiling.max_output_tokens);
                budget.max_game_ticks = budget.max_game_ticks.min(ceiling.max_game_ticks);
                let mut current = child.context(false)?;
                current.budget = budget;
                let view = match action {
                    Action::Observe(raw) => child.observe(
                        raw.as_deref()
                            .map(ObserveRequest::parse)
                            .transpose()?
                            .unwrap_or_default(),
                        &current,
                    ),
                    Action::Inspect => child.inspect(&current),
                    Action::Plan(raw) => {
                        let value: Value = serde_json::from_str(&raw)
                            .map_err(|_| invalid("invalid production project request"))?;
                        match value["kind"].as_str() {
                            Some("stock_target") => {
                                return Err(denied(
                                    "stock targets require their parent publication intent",
                                ));
                            }
                            Some("run") => child.plan_run(RunRequest::parse(&raw)?, &current),
                            Some("inventory_run") => child
                                .plan_inventory_run(InventoryRunRequest::parse(&raw)?, &current),
                            Some("step" | "review_prepared") => {
                                #[derive(Deserialize)]
                                #[serde(deny_unknown_fields)]
                                struct Step {
                                    kind: String,
                                    original_plan_digest: String,
                                }
                                let request: Step = serde_json::from_str(&raw)
                                    .map_err(|_| invalid("invalid closed original-step request"))?;
                                let original = digest(&request.original_plan_digest)?;
                                if child.project_status(&goal)?.original_plan_digest
                                    != Some(original)
                                {
                                    return Err(denied(
                                        "project request names another original production goal",
                                    ));
                                }
                                if request.kind == "step" {
                                    child.prepare_next(original, &current)
                                } else {
                                    child.review_prepared(original, &current)
                                }
                            }
                            _ => {
                                return Err(invalid(
                                    "project permits only its fixed stock target, original steps and independently reviewed clocks",
                                ));
                            }
                        }
                    }
                    Action::Commit {
                        seal,
                        confirmed,
                        scope,
                    } => {
                        let seal = digest(&seal)?;
                        match scope.as_deref().unwrap_or("goal") {
                            "goal" => child.commit(seal, confirmed, &current),
                            "clock" => child.commit_run(seal, confirmed, &current),
                            "inventory_clock" => {
                                child.commit_inventory_run(seal, confirmed, &current)
                            }
                            _ => return Err(invalid("unknown production commit scope")),
                        }
                    }
                    Action::Wait(scope) => {
                        if !matches!(
                            scope.as_deref().unwrap_or("goal"),
                            "goal" | "project" | "clock" | "inventory_clock"
                        ) {
                            return Err(invalid("unknown production wait scope"));
                        }
                        if offline {
                            child.inspect(&current)
                        } else {
                            child.wait(&current)
                        }
                    }
                    Action::Cancel(scope) => match scope.as_deref().unwrap_or("project") {
                        "goal" | "project" => child.cancel(&current),
                        "clock" => child.cancel_run(&current),
                        "inventory_clock" => child.cancel_inventory_run(&current),
                        _ => return Err(invalid("unknown production cancellation scope")),
                    },
                }?;
                self.current_goal = child.project_status(&goal)?.to_json();
                Ok(view.to_json())
            }
            None => Err(denied("project child unavailable")),
        }
    }

    pub(super) fn perform(&mut self, action: Action) -> Result<Value> {
        self.request = self.request.checked_add(1).ok_or_else(exhausted)?;
        self.current_goal = Value::Null;
        self.publication_started = Instant::now();
        let mut work = Work::new(self.budget, self.publication_started)?;
        let outcome = (|| {
            let stopping = matches!(&action, Action::Cancel(scope)
                if matches!(scope.as_deref().unwrap_or("project"), "goal" | "project"));
            // The reserved terminal frame must remain usable even when there
            // is no space for another active-stage checkpoint.
            self.preflight(&mut work, stopping || matches!(&action, Action::Inspect))?;
            if stopping {
                let base = self.context(&work)?;
                let current = work.reserve(&base, journal::WORK_BYTES)?;
                // Persist the root stop before asking the current child to
                // retire its preparation. This cannot erase a native effect.
                self.journal.cancel(&current)?;
            }
            self.child_call(action, &mut work)
        })();
        if outcome.is_err() {
            self.withdraw();
        }
        let retained = self.save_child(&mut work, None);
        self.accounting = work.accounting();
        if let Err(e) = retained {
            self.fenced = true;
            self.withdraw();
            return Err(e);
        }
        if let Err(e) = work.remaining().and_then(|_| self.config.current()) {
            self.withdraw();
            return Err(e);
        }
        outcome.map(|child| self.result(child))
    }

    pub(super) fn withdraw(&mut self) {
        self.current_goal = Value::Null;
        if let Some(child) = &mut self.child {
            child.withdraw();
            self.last_child_reference = child.reference();
            self.high_tick = self
                .high_tick
                .max(continuity::tick(&self.last_child_reference));
        }
    }

    pub(super) fn reference(&self) -> Value {
        let mut pin = self.journal.pin();
        // This is a rejection floor, not a claim that a failed append was
        // acknowledged. Preserve it even when a child reply cannot be retained.
        pin.authority_tick_floor = pin.authority_tick_floor.max(self.high_tick);
        json!({
            "schema":"dfmcp.native-resource-project-recovery/1",
            "original":self.journal.original(),
            "parent_acknowledged":pin,
            "retained_stage":self.journal.snapshot(),
            "child_reference":self.last_child_reference,
            "authority_tick_floor":self.high_tick,
            "controller_fenced":self.fenced || self.journal.is_fenced(),
            "current_goal_available":false,
            "preparation_restored":false,
            "automatic_redispatch":false,
            "recovery":"Retain parent journal and ACK, every selected child custody family and each exact handoff. Reopen existing files only; an opening marker never grants initialization.",
        })
    }

    fn result(&self, child: Value) -> Value {
        json!({
            "ok":true, "profile":"native-resource-project",
            "mode":match self.mode { Mode::Control=>"control", Mode::Recover=>"recover", Mode::Offline=>"offline" },
            "original":self.journal.original(),
            "stage":self.journal.snapshot().stage,
            "project_cancelled":self.cancelled(),
            "current_stock_goal":self.current_goal,
            "original_goal_proven":!self.cancelled() && self.current_goal["original_goal_proven"] == true,
            "project_custody_currently_verified":true,
            "native_effects_cancelled":false,
            "automatic_stage_effects":false,
            "runtime_admitted":false,
            "canonical_cross_family_anchor_available":false,
            "budget":self.accounting,
            "child":child,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_work_reserves_final_custody_before_child_io()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        crate::run_with_runtime_cx(|_| async {
            let ceiling = WorkBudget {
                max_bytes: 1024 * 1024 * 1024,
                max_wall_millis: 60_000,
                max_entities: 4096,
                max_actions: 5,
                max_output_tokens: 65_536,
                max_game_ticks: 100,
            };
            let mut work = Work::new(ceiling, Instant::now())?;
            let child = work.child(WORKSHOP_BYTES, journal::WORK_BYTES)?;
            assert_eq!(child.max_bytes, WORKSHOP_BYTES);
            assert_eq!(child.max_game_ticks, ceiling.max_game_ticks);
            assert!(child.max_wall_millis <= ceiling.max_wall_millis);
            let remaining = work.remaining()?;
            assert_eq!(
                remaining.max_bytes,
                ceiling.max_bytes - RESPONSE_BYTES - WORKSHOP_BYTES
            );
            let last = work.child(MAX_BYTES, journal::WORK_BYTES)?;
            assert_eq!(last.max_bytes + journal::WORK_BYTES, remaining.max_bytes);
            assert!(work.child(1, journal::WORK_BYTES).is_err());
            assert_eq!(work.bytes, journal::WORK_BYTES);
            Ok::<_, dfmcp_core::DfmcpError>(())
        })??;
        Ok(())
    }

    #[test]
    fn parent_modes_preserve_child_recovery_semantics() {
        assert_eq!(tree_mode(Mode::Recover), TreeMode::Recover);
        assert_eq!(tree_mode(Mode::Offline), TreeMode::Offline);
        assert_eq!(workshop_mode(Mode::Recover), WorkshopMode::Recover);
        assert_eq!(production_mode(Mode::Recover), ProductionMode::Reconcile);
        assert_eq!(production_mode(Mode::Offline), ProductionMode::Offline);
    }
}
