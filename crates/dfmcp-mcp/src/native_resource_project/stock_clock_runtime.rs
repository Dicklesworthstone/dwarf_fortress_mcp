//! Foreground integration of the parent's separate no-order stock-watch clock.
//! Every reservation belongs to one public call; a native marker never grants
//! another interval or restores a process-local stock sample.
use super::stock_clock::{ClockCheckpoint, StockClock, WatchClockTarget};
use super::*;
use crate::live_production_runtime::StockWatchClockBasis;
use crate::semantic_work_order_runtime::bounded_goal_clock::{ClockView, RunRequest};
use crate::semantic_work_order_runtime::{Mode as OrderMode, Shell as _};
use dfmcp_adapter::bounded_run::private_file::PrivateRunFile;
use dfmcp_adapter::bounded_run::rpc::RunSource;

const MIB: u64 = 1024 * 1024;
pub(super) const OPEN_BYTES: u64 = 128 * MIB;
const SAMPLE_BYTES: u64 = 1024 * MIB;
const ACTION_BYTES: u64 = 256 * MIB;
const VIEW_BYTES: u64 = 8 * MIB;

pub(super) type NativeStockClock = StockClock<PrivateRunFile>;

pub(super) enum StockClockAction {
    Plan(RunRequest),
    Commit { seal: Digest32, confirmed: bool },
    Wait,
    Cancel,
}
impl StockClockAction {
    pub(super) fn from_action(action: &Action) -> Result<Option<Self>> {
        Ok(match action {
            Action::Plan(raw) if raw.len() <= 4096 => {
                let value: Value = serde_json::from_str(raw)
                    .map_err(|_| invalid("invalid resource project plan request"))?;
                if value["kind"] == "stock_watch_run" {
                    Some(Self::Plan(RunRequest::parse(raw, "stock_watch_run")?))
                } else {
                    None
                }
            }
            Action::Commit {
                seal,
                confirmed,
                scope,
                order_step_id,
            } if scope.as_deref() == Some("stock_clock") => {
                if order_step_id.is_some() {
                    return Err(invalid("a no-order stock watch has no order selector"));
                }
                Some(Self::Commit {
                    seal: digest(seal)?,
                    confirmed: *confirmed,
                })
            }
            Action::Wait(scope) if scope.as_deref() == Some("stock_clock") => Some(Self::Wait),
            Action::Cancel {
                scope,
                order_step_id,
            } if scope.as_deref() == Some("stock_clock") => {
                if order_step_id.is_some() {
                    return Err(invalid("a no-order stock watch has no order selector"));
                }
                Some(Self::Cancel)
            }
            _ => None,
        })
    }
}

fn narrowed(a: WorkBudget, b: WorkBudget) -> WorkBudget {
    WorkBudget {
        max_bytes: a.max_bytes.min(b.max_bytes),
        max_wall_millis: a.max_wall_millis.min(b.max_wall_millis),
        max_game_ticks: a.max_game_ticks.min(b.max_game_ticks),
        max_entities: a.max_entities.min(b.max_entities),
        max_actions: a.max_actions.min(b.max_actions),
        max_output_tokens: a.max_output_tokens.min(b.max_output_tokens),
    }
}
fn order_mode(mode: Mode) -> OrderMode {
    match mode {
        Mode::Control => OrderMode::Control,
        Mode::Recover => OrderMode::Reconcile,
        Mode::Offline => OrderMode::Offline,
    }
}

impl Runtime {
    fn stock_child(&mut self) -> Result<&mut ProductionRuntime> {
        match self.child.as_mut() {
            Some(Child::Production(child)) => Ok(child),
            _ => Err(denied(
                "stock-watch time requires the acknowledged production child",
            )),
        }
    }

    fn stock_context(
        &mut self,
        work: &mut Work,
        bytes: u64,
        observation: bool,
    ) -> Result<OperationContext> {
        let floor = self.high_tick.max(self.journal.pin().authority_tick_floor);
        let child = self.stock_child()?;
        child.retain_project_authority_tick(floor)?;
        let ceiling = child.budget();
        let mut context = child.context(false)?;
        if !observation {
            // Authorization/replay floor only. This is never returned as a
            // source observation or fed into a stock sampling operation.
            context.anchor.tick = context.anchor.tick.max(GameTick(floor));
        }
        context = work.reserve(&context, bytes)?;
        context.budget = narrowed(context.budget, ceiling);
        if context.budget.max_bytes != bytes {
            return Err(exhausted());
        }
        Ok(context)
    }

    fn validate_stock_target(&self, target: &WatchClockTarget) -> Result<()> {
        let production = self
            .journal
            .snapshot()
            .checkpoint
            .production
            .as_ref()
            .ok_or_else(|| denied("stock clock lost its acknowledged production custody"))?;
        if !self.config.stock_watch_clock || self.effective_stage() != Stage::Production {
            return Err(denied(
                "this original project did not opt in to stock-watch time",
            ));
        }
        target.matches_original(
            digest(&self.journal.pin().journal_id)?,
            stock_clock::original_digest(self.journal.original())?,
            &self.config.goal,
            stock_clock::production_source_digest(production)?,
        )
    }

    /// An explicit no-work stock Plan may initialize this one fixed sibling.
    /// Recovery only reopens the exact prior file, even if initialization failed.
    pub(super) fn open_stock_clock(&mut self, work: &mut Work, initialize: bool) -> Result<()> {
        if self.stock_clock.is_some() {
            return Ok(());
        }
        let retained = self.journal.snapshot().checkpoint.stock_clock.clone();
        if retained.is_none() && (!initialize || !self.config.stock_watch_clock) {
            return Ok(());
        }
        let goal = self.config.goal.clone();
        let mut context = self.stock_context(work, 64 * MIB, retained.is_none())?;
        let shell = self.stock_child()?.project_stock_clock_shell(&goal)?;
        let (checkpoint, creating) = if let Some(value) = retained {
            (ClockCheckpoint::from_json(&value)?, false)
        } else {
            self.authorize_forward(work)?;
            if !self.stock_child()?.project_has_stock_watch(&goal)? {
                return Ok(());
            }
            let production = self
                .journal
                .snapshot()
                .checkpoint
                .production
                .as_ref()
                .ok_or_else(|| {
                    denied("stock clock needs acknowledged no-order production custody")
                })?;
            let target = WatchClockTarget::new(
                digest(&self.journal.pin().journal_id)?,
                stock_clock::original_digest(self.journal.original())?,
                &goal,
                stock_clock::production_source_digest(production)?,
                context.anchor,
            )?;
            let id =
                shell.prospective_resource_stock_clock_journal(&self.config.directory, &context)?;
            let checkpoint = ClockCheckpoint::prospective(target, id)?;
            let mut next = self.journal.snapshot().checkpoint.clone();
            next.stock_clock = Some(checkpoint.to_json());
            // ACK before the child header can be created, not merely before RPC.
            self.append(work, Stage::Production, next)?;
            (checkpoint, true)
        };
        self.validate_stock_target(checkpoint.target())?;
        let external = self.config.stock_clock_recovery.clone();
        if let Some(value) = &external {
            let external = ClockCheckpoint::from_json(value)?;
            if checkpoint.target() != external.target()
                || checkpoint.journal_id() != external.journal_id()
            {
                return Err(denied(
                    "external stock-clock recovery names another original owner",
                ));
            }
        }
        context.budget.max_wall_millis = context
            .budget
            .max_wall_millis
            .min(work.remaining()?.max_wall_millis);
        let journal = shell.open_resource_stock_clock(
            &self.config.directory,
            order_mode(self.mode),
            creating,
            checkpoint.journal_id(),
            &context,
        )?;
        // Do not publish a partially verified owner. Any failure before both
        // ancestry checks drops this local handle and preserves the stricter
        // acknowledged/external recovery pins for every subsequent reopen.
        let mut clock = StockClock::new(journal, checkpoint.target().clone());
        let current = self.stock_context(work, VIEW_BYTES, false)?;
        let native = shell.inventory_clock_context(&current, false)?;
        let verified = clock.verify_recovery(&checkpoint.to_json(), &current, &native);
        self.high_tick = self.high_tick.max(clock.high_tick());
        verified?;
        if let Some(external) = external {
            // Both retained heads must occur in the actual surviving journal.
            // An older external pin cannot weaken a newer acknowledged one;
            // verify_recovery merges rejection floors monotonically.
            let current = self.stock_context(work, VIEW_BYTES, false)?;
            let native = shell.inventory_clock_context(&current, false)?;
            let verified = clock.verify_recovery(&external, &current, &native);
            self.high_tick = self.high_tick.max(clock.high_tick());
            verified?;
        }
        self.stock_clock = Some(clock);
        let current = self.stock_context(work, VIEW_BYTES, false)?;
        let native = shell.inventory_clock_context(&current, false)?;
        let clock = self
            .stock_clock
            .as_mut()
            .ok_or_else(|| denied("stock clock owner was not retained"))?;
        let inspected = clock.inspect(&current, &native);
        self.high_tick = self.high_tick.max(clock.high_tick());
        inspected.map(|_| ())
    }

    pub(super) fn stock_clock_withholds_goal(&self) -> bool {
        self.journal.snapshot().checkpoint.stock_clock.is_some()
            && self
                .stock_clock
                .as_ref()
                .is_none_or(|clock| clock.pending() || clock.may_be_running())
    }

    pub(super) fn refresh_stock_clock(&mut self, work: &mut Work) -> Result<()> {
        if self.journal.snapshot().checkpoint.stock_clock.is_none() {
            return Ok(());
        }
        self.open_stock_clock(work, false)?;
        let current = self.stock_context(work, VIEW_BYTES, false)?;
        let goal = self.config.goal.clone();
        let control_enabled = self.stock_child()?.inventory_clock_control_enabled();
        let shell = self.stock_child()?.project_stock_clock_shell(&goal)?;
        let native = shell.inventory_clock_context(&current, false)?;
        let clock = self
            .stock_clock
            .as_mut()
            .ok_or_else(|| denied("retained stock clock custody is unavailable"))?;
        if !control_enabled {
            clock.withdraw_review();
        }
        let result = clock.inspect(&current, &native);
        self.high_tick = self.high_tick.max(clock.high_tick());
        result.map(|_| ())
    }

    fn fresh_stock_clock_basis(
        &mut self,
        goal: &ProjectStockGoal,
        context: &OperationContext,
    ) -> Result<StockWatchClockBasis> {
        let child = self.stock_child()?;
        let result = child.project_sample_stock_clock(goal, context);
        let consumed = continuity::tick(&child.project_recovery_reference());
        self.high_tick = self.high_tick.max(consumed);
        result
    }

    /// One reviewed prepare/commit or one bounded reconciliation/cancellation.
    /// Error paths keep the actual clock owner and all consumed source floors.
    pub(super) fn perform_stock_clock(
        &mut self,
        action: StockClockAction,
        work: &mut Work,
    ) -> Result<Value> {
        let forward = matches!(
            &action,
            StockClockAction::Plan(_) | StockClockAction::Commit { .. }
        );
        if forward {
            self.authorize_forward(work)?;
        }
        self.refresh_stock_clock(work)?;
        let target = self
            .stock_clock
            .as_ref()
            .ok_or_else(|| {
                denied("plan the original no-work stock goal before requesting stock-watch time")
            })?
            .target()
            .clone();
        self.validate_stock_target(&target)?;
        if let StockClockAction::Commit { seal, confirmed } = &action {
            if !confirmed
                || self
                    .stock_clock
                    .as_ref()
                    .and_then(StockClock::current_review)
                    != Some(*seal)
            {
                return Err(denied(
                    "prepare and confirm the current stock-watch clock review",
                ));
            }
        }
        let goal = self.config.goal.clone();
        // Reserve both complete censuses, final parent verification and native
        // work before any live contact. Unused reservations are not recycled.
        let initial_sample = if forward {
            Some(self.stock_context(work, SAMPLE_BYTES, true)?)
        } else {
            None
        };
        let mut guarded_sample = if forward {
            Some(self.stock_context(work, SAMPLE_BYTES, true)?)
        } else {
            None
        };
        let mut guarded_parent = if forward {
            Some(self.stock_context(work, journal::WORK_BYTES, false)?)
        } else {
            None
        };
        let mut canonical = self.stock_context(work, ACTION_BYTES, forward)?;
        let mut source_context = self.stock_context(work, 4 * MIB, forward)?;
        if work.remaining()?.max_bytes < journal::WORK_BYTES {
            return Err(exhausted());
        }
        if let Some(mut sample) = initial_sample {
            sample.budget.max_wall_millis = sample
                .budget
                .max_wall_millis
                .min(work.remaining()?.max_wall_millis);
            let basis = self.fresh_stock_clock_basis(&goal, &sample)?;
            if basis.anchor.fortress_id != target.fortress()
                || basis.anchor.cursor.epoch != target.epoch()
                || basis.anchor.tick.get() < target.original_tick()
            {
                return Err(denied(
                    "stock clock fresh basis differs from its original source",
                ));
            }
            canonical.anchor = basis.anchor;
            source_context.anchor = basis.anchor;
        }
        canonical.budget.max_wall_millis = canonical
            .budget
            .max_wall_millis
            .min(work.remaining()?.max_wall_millis);
        source_context.request_id = canonical.request_id;
        source_context.budget.max_wall_millis = source_context
            .budget
            .max_wall_millis
            .min(canonical.budget.max_wall_millis);
        let shell = self.stock_child()?.project_stock_clock_shell(&goal)?;
        let control = forward || matches!(&action, StockClockAction::Cancel);
        let mut native = shell.inventory_clock_context(&canonical, control)?;
        let offline = self.mode == Mode::Offline;
        let needs_source = !offline
            && match &action {
                StockClockAction::Cancel => self
                    .stock_clock
                    .as_ref()
                    .and_then(StockClock::latest)
                    .is_some_and(ClockView::cancellation_needs_source),
                StockClockAction::Wait => self
                    .stock_clock
                    .as_ref()
                    .and_then(StockClock::latest)
                    .is_some_and(ClockView::needs_reconciliation),
                _ => true,
            };
        let mut source = if needs_source {
            let source_context = shell.inventory_clock_context(&source_context, control)?;
            Some(shell.inventory_clock_source(&source_context)?)
        } else {
            None
        };
        canonical.budget.max_wall_millis = canonical
            .budget
            .max_wall_millis
            .min(work.remaining()?.max_wall_millis);
        native.budget.max_wall_millis = native
            .budget
            .max_wall_millis
            .min(canonical.budget.max_wall_millis);
        let mut clock = self
            .stock_clock
            .take()
            .ok_or_else(|| denied("stock clock owner is unavailable"))?;
        let result = (|| {
            let mut guard = |edge: &OperationContext| -> Result<()> {
                self.config.current()?;
                self.validate_stock_target(&target)?;
                if self.cancelled() || self.fenced || self.high_tick >= goal.deadline_tick() {
                    return Err(denied(
                        "stock clock lost its original active parent horizon",
                    ));
                }
                let mut parent = guarded_parent
                    .take()
                    .ok_or_else(|| denied("stock-clock parent verification cannot repeat"))?;
                parent.anchor = edge.anchor;
                parent.budget.max_wall_millis = parent
                    .budget
                    .max_wall_millis
                    .min(edge.budget.max_wall_millis)
                    .min(work.remaining()?.max_wall_millis);
                self.journal.verify(&parent)?;
                let mut sample = guarded_sample
                    .take()
                    .ok_or_else(|| denied("stock-clock final census cannot repeat"))?;
                sample.anchor = self.stock_child()?.anchor();
                sample.budget.max_wall_millis = sample
                    .budget
                    .max_wall_millis
                    .min(edge.budget.max_wall_millis)
                    .min(work.remaining()?.max_wall_millis);
                let basis = self.fresh_stock_clock_basis(&goal, &sample)?;
                if basis.anchor.fortress_id != edge.anchor.fortress_id
                    || basis.anchor.cursor.epoch != edge.anchor.cursor.epoch
                    || basis.anchor.tick != edge.anchor.tick
                {
                    return Err(denied(
                        "native clock and fresh complete stock census do not share one paused tick",
                    ));
                }
                self.config.current()?;
                work.remaining()?;
                Ok(())
            };
            match action {
                StockClockAction::Plan(request) => clock.prepare_guarded(
                    &request,
                    source
                        .as_mut()
                        .ok_or_else(|| denied("stock clock source absent"))?,
                    &canonical,
                    &native,
                    &mut guard,
                ),
                StockClockAction::Commit { seal, confirmed } => clock.commit_guarded(
                    seal,
                    confirmed,
                    source
                        .as_mut()
                        .ok_or_else(|| denied("stock clock source absent"))?,
                    &canonical,
                    &native,
                    &mut guard,
                ),
                StockClockAction::Wait if needs_source => clock.reconcile(
                    source
                        .as_mut()
                        .ok_or_else(|| denied("stock clock source absent"))?,
                    &canonical,
                    &native,
                ),
                StockClockAction::Wait => clock.inspect(&canonical, &native),
                StockClockAction::Cancel => clock.cancel(
                    source.as_mut().map(|source| source as &mut dyn RunSource),
                    &canonical,
                    &native,
                ),
            }
        })();
        self.high_tick = self.high_tick.max(clock.high_tick());
        self.stock_clock = Some(clock);
        // A pre-effect census is historical after an attempted unpause. Even
        // prepare/review leaves no current proof available beside a pending run.
        let floor = self.high_tick;
        let child = self.stock_child()?;
        if result.is_ok() {
            child.retain_project_stock_clock_tick(floor)?;
        } else {
            child.abandon_preparation();
            child.retain_project_authority_tick(floor)?;
        }
        self.current_goal = Value::Null;
        result?;
        Ok(self.stock_child()?.project_stock_clock_view())
    }

    pub(super) fn stock_clock_recovery(&self) -> Value {
        self.stock_clock.as_ref().map_or_else(
            || {
                self.config
                    .stock_clock_recovery
                    .clone()
                    .or_else(|| self.journal.snapshot().checkpoint.stock_clock.clone())
                    .unwrap_or(Value::Null)
            },
            StockClock::recovery_checkpoint,
        )
    }

    pub(super) fn stock_clock_json(&self) -> Value {
        if let Some(clock) = &self.stock_clock {
            return clock.to_json();
        }
        self.journal.snapshot().checkpoint.stock_clock.as_ref().map_or(Value::Null, |pin| {
            json!({
                "schema":"dfmcp.native-resource-stock-clock-view/1",
                "target":pin["target"], "target_digest":pin["target"]["target_digest"],
                "original_paired_seal":pin["target"]["original_paired_seal"],
                "journal_id":pin["journal_id"], "head":pin["head"],
                "high_tick":pin["authority_tick_floor"], "high_source_sequence":pin["high_source_sequence"],
                "inspection_required":true, "pending":true, "unresolved":true,
                "may_be_running":true, "review_available":false, "runs":[],
                "current_pause_proven":false, "original_goal_proven":false,
                "recovery_restores_current_proof":false,
            })
        })
    }

    pub(super) fn stock_clock_guidance(&self) -> Value {
        let Some(clock) = &self.stock_clock else {
            return Value::Null;
        };
        let target = clock.target();
        let mut guidance = json!({
            "schema":"dfmcp.resource-stock-clock-guidance/1",
            "target_digest":target.digest().to_hex(),
            "original_goal_digest":target.to_json()["original_goal_digest"],
            "goal_witness":null, "plan_request":null, "blocker":null,
        });
        let blocker = if self.cancelled() || self.fenced || self.journal.is_fenced() {
            Some((
                "project_stopped",
                "The original project is stopped or its custody needs recovery.",
                "operator",
            ))
        } else if clock.current_review().is_some() {
            None
        } else if clock
            .latest()
            .and_then(ClockView::active)
            .is_some_and(|run| {
                run.entry.state() == dfmcp_adapter::bounded_run::journal::RunState::Prepared
                    && !run.entry.unresolved()
            })
        {
            Some((
                "stock_clock_prepared_requires_review",
                "The original interval is prepared; request a fresh review or explicitly cancel it before rebuilding a recovered stock watch.",
                "inspect",
            ))
        } else if clock.pending() || clock.may_be_running() {
            Some((
                "stock_clock_reconciliation_required",
                "Inspect or reconcile the retained stock-watch run before sampling stock or starting another action.",
                "wait",
            ))
        } else if self.mode != Mode::Control || !clock.is_control() {
            Some((
                "stock_clock_control_unavailable",
                "A new interval requires the original independent inventory-clock control authority.",
                "operator",
            ))
        } else {
            match &self.child {
                Some(Child::Production(child)) if !child.inventory_clock_control_enabled() => {
                    Some((
                        "stock_clock_control_unavailable",
                        "The original independent inventory-clock authority is currently unavailable.",
                        "operator",
                    ))
                }
                Some(Child::Production(child)) => {
                    match child.project_stock_clock_basis(&self.config.goal) {
                        Ok(basis) => {
                            let ticks = basis
                                .next_sample_tick
                                .saturating_sub(basis.anchor.tick.get())
                                .min(self.budget.max_game_ticks)
                                .min(1200);
                            let wall = self.budget.max_wall_millis.min(60_000);
                            if ticks == 0
                                || wall == 0
                                || clock.latest().is_none_or(|view| view.runs.len() >= 64)
                            {
                                Some((
                                    "stock_clock_allowance_exhausted",
                                    "No original bounded clock interval remains.",
                                    "operator",
                                ))
                            } else {
                                guidance["goal_witness"] = json!(basis.goal_witness.to_hex());
                                guidance["plan_request"] = json!({
                                    "kind":"stock_watch_run",
                                    "original_paired_seal":target.original_paired_seal().to_hex(),
                                    "game_ticks":ticks, "wall_millis":wall,
                                });
                                None
                            }
                        }
                        Err(_) => Some((
                            "stock_watch_sample_required",
                            "Take a fresh complete stock sample; only True stock with unfinished stability can justify time.",
                            "wait",
                        )),
                    }
                }
                _ => Some((
                    "stock_clock_child_unavailable",
                    "Recover the exact acknowledged production and clock custody.",
                    "operator",
                )),
            }
        };
        if let Some((code, message, next_action)) = blocker {
            guidance["blocker"] = json!({"code":code,"message":message,"next_action":next_action});
        }
        guidance
    }
}
