//! Standalone recovery of an existing excavation-monitor history.
//!
//! This owner has no native session, source, parent file, lease or preparation.
//! Original native evidence is named by the sealed history, never reverified
//! here. Inspection and local monitor cancellation remain available when those
//! other files or their original native process cannot be recovered.
use super::{Action, Query, Work, error, exhausted, monitor, presentation, runtime};
use dfmcp_adapter::control_effect_journal::EffectJournalStorage;
use dfmcp_adapter::dig_blueprint::monitor::store::MonitorStore;
use dfmcp_adapter::dig_designation::journal::{DigBinding, DigMode};
use dfmcp_core::{
    Capability, CapabilityGrant, Digest32, ErrorCode, GameTick, OperationContext, RequestId,
    Result, SessionId, WorkBudget,
};
use serde_json::{Value, json};
use std::time::Instant;

pub(super) struct RecoveryState<B: EffectJournalStorage> {
    store: MonitorStore<B>,
    id: SessionId,
    request: u128,
    budget: WorkBudget,
    grants: Vec<CapabilityGrant>,
    authority_tick: u64,
    mode: DigMode,
}

impl<B: EffectJournalStorage> RecoveryState<B> {
    /// The supplied store has already checked the operator's exact recovery
    /// identity and acknowledged-head ancestry. This constructor additionally
    /// checks current configuration and attenuates this owner to Query only.
    pub(super) fn new(
        mut store: MonitorStore<B>,
        c: &OperationContext,
        config: &runtime::Config,
        mode: DigMode,
    ) -> Result<Self> {
        if !matches!(mode, DigMode::Offline | DigMode::Recover) {
            return Err(denied());
        }
        config.matches(store.spec().parent().binding())?;
        let selected = config.monitor.as_ref().ok_or_else(denied)?;
        if selected.initialize
            || selected
                .recovery
                .is_none_or(|pin| pin.monitor_id != store.spec().id())
        {
            return Err(denied());
        }
        selected.matches(store.spec())?;
        let mut current = c.clone();
        current.anchor.tick = current.anchor.tick.max(GameTick(store.high_tick()));
        current
            .grants
            .retain(|grant| grant.capability == Capability::Query);
        store.spec().authorize(&current, false)?;
        store.verify(&current)?;
        Ok(Self {
            authority_tick: current.anchor.tick.get(),
            store,
            id: current.session_id,
            request: current.request_id.get(),
            budget: current.budget,
            grants: current.grants,
            mode,
        })
    }
    pub(super) fn id(&self) -> SessionId {
        self.id
    }
    pub(super) fn binding(&self) -> &DigBinding {
        self.store.spec().parent().binding()
    }
    pub(super) fn mode(&self) -> DigMode {
        self.mode
    }
    pub(super) fn high_tick(&self) -> u64 {
        self.authority_tick.max(self.store.high_tick())
    }
    /// Restore the already admitted session allowance after prepaid opening.
    /// This changes no authority, identity, temporal goal or sampling allowance.
    pub(super) fn set_budget(&mut self, budget: WorkBudget) {
        self.budget = budget;
    }
    pub(super) fn context(&mut self, wall: Option<u64>) -> Result<OperationContext> {
        self.request = self.request.checked_add(1).ok_or_else(exhausted)?;
        let mut budget = self.budget;
        if let Some(wall) = wall {
            budget.max_wall_millis = budget.max_wall_millis.min(wall);
        }
        budget.validate()?;
        let mut current = super::context(
            self.id,
            RequestId::new(self.request),
            self.binding(),
            self.high_tick(),
            budget,
            false,
        );
        current.grants = self.grants.clone();
        Ok(current)
    }
    pub(super) fn reference(&self) -> Value {
        let mut value = monitor::reference(&self.store);
        value["original_designations_verified"] = json!(false);
        value["historical_evidence_only"] = json!(true);
        value["authority_tick_floor"] = json!(self.high_tick());
        value
    }
    fn current(&self, c: &OperationContext) -> Result<OperationContext> {
        if c.session_id != self.id {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "monitor recovery belongs to another session owner",
            ));
        }
        let mut current = c.clone();
        current.anchor.tick = current.anchor.tick.max(GameTick(self.high_tick()));
        current
            .grants
            .retain(|grant| grant.capability == Capability::Query);
        Ok(current)
    }
    fn authorize(&self, c: &OperationContext) -> Result<OperationContext> {
        let current = self.current(c)?;
        self.store.spec().authorize(&current, false)?;
        // An injected operation context cannot renew the session's original
        // grants. A new owner must be explicitly opened with fresh authority.
        let mut original = current.clone();
        original.grants = self.grants.clone();
        self.store.spec().authorize(&original, false)?;
        Ok(current)
    }
    fn retain(&self, mut result: Value) -> Value {
        if result.get("blueprint_monitor").is_none() {
            result["blueprint_monitor"] = self.reference();
        }
        result["monitor_only"] = json!(true);
        result["profile"] = json!(match self.mode {
            DigMode::Offline => "monitor-offline",
            _ => "monitor-recovery",
        });
        result["evidence_scope"] = json!("this_blueprint_monitor_history_only");
        result["native_calls"] = json!(0);
        result["game_mutations"] = json!(0);
        result["original_inventory_verified"] = json!(false);
        result["original_designations_verified"] = json!(false);
        result["fresh_sample_acquired"] = json!(false);
        result["sampled_excavation_condition_proven"] = json!(false);
        result["retry_commit_permitted"] = json!(false);
        result["dispatch_authorized"] = json!(false);
        result
    }
    fn packet(&self, op: &str, c: &OperationContext, result: Value) -> String {
        presentation::packet(
            op,
            self.retain(result),
            Some(c),
            Some(self.binding()),
            None,
            None,
            None,
        )
    }
    /// Opening has its own prepaid allowance and enclosing request deadline.
    pub(super) fn inspect(&mut self, c: &OperationContext) -> Result<Value> {
        let started = Instant::now();
        let current = self.authorize(c)?;
        self.store.verify(&current)?;
        let value = monitor::display(&self.store, false, false, 0, 16)?;
        if started.elapsed().as_millis() >= u128::from(current.budget.max_wall_millis) {
            return Err(exhausted());
        }
        Ok(value)
    }
    fn exact(&self, id: Digest32) -> Result<()> {
        if id == self.store.spec().id() {
            Ok(())
        } else {
            Err(error(
                ErrorCode::StaleAnchor,
                "monitor recovery names another original obligation",
            ))
        }
    }
    fn operation(
        &self,
        work: &Work,
        c: &OperationContext,
        reserved: &OperationContext,
    ) -> Result<OperationContext> {
        let mut current = self.authorize(c)?;
        current.budget = reserved.budget;
        current.budget.max_wall_millis = work.current(&current)?.budget.max_wall_millis;
        Ok(current)
    }
    fn perform(
        &mut self,
        c: &OperationContext,
        work: &mut Work,
        action: Action,
        checkpoint: &mut impl FnMut(&OperationContext) -> Result<()>,
    ) -> Result<Value> {
        match action {
            Action::Query(Query::BlueprintMonitor {
                monitor_id,
                expected_head,
                offset,
                limit,
            }) => {
                self.exact(presentation::digest(&monitor_id)?)?;
                let check = work.take(c, monitor::FINAL_BYTES)?;
                checkpoint(&work.current(c)?)?;
                let check = self.operation(work, c, &check)?;
                self.store.verify(&check)?;
                if expected_head
                    .as_deref()
                    .map(presentation::digest)
                    .transpose()?
                    .is_some_and(|head| head != self.store.head())
                {
                    return Err(error(
                        ErrorCode::StaleAnchor,
                        "monitor page belongs to another history head",
                    ));
                }
                let value = monitor::display(
                    &self.store,
                    false,
                    false,
                    offset.unwrap_or(0),
                    limit.unwrap_or(16),
                )?;
                checkpoint(&work.current(c)?)?;
                Ok(json!({"ok":true,"blueprint_monitor":value}))
            }
            Action::Inventory => {
                let check = work.take(c, monitor::FINAL_BYTES)?;
                checkpoint(&work.current(c)?)?;
                let check = self.operation(work, c, &check)?;
                let value = self.inspect(&check)?;
                checkpoint(&work.current(c)?)?;
                Ok(json!({"ok":true,"blueprint_monitor":value,"live_health_checked":false}))
            }
            Action::Query(Query::Schema {}) => {
                checkpoint(&work.current(c)?)?;
                Ok(json!({"ok":true,"query_schema":query_schema()?}))
            }
            Action::CancelMonitor(id) => {
                self.exact(id)?;
                if self.mode != DigMode::Recover {
                    return Err(error(
                        ErrorCode::CapabilityDenied,
                        "offline monitor history permits inspection only",
                    ));
                }
                let operation = work.take(c, self.store.operation_reserve())?;
                let check = work.take(c, monitor::FINAL_BYTES)?;
                checkpoint(&work.current(c)?)?;
                let operation = self.operation(work, c, &operation)?;
                let result = self.store.cancel(&operation);
                self.authority_tick = self.high_tick();
                result?;
                let check = self.operation(work, c, &check)?;
                self.store.verify(&check)?;
                checkpoint(&work.current(c)?)?;
                Ok(json!({"ok":true,"scope":"monitor",
                    "blueprint_monitor":monitor::display(&self.store,false,false,0,16)?,
                    "effects_cancelled":false,"history_erased":false}))
            }
            _ => Err(denied()),
        }
    }
    pub(super) fn run_action(
        &mut self,
        c: OperationContext,
        op: &str,
        action: Result<Action>,
        started: Instant,
        mut checkpoint: impl FnMut(&OperationContext) -> Result<()>,
    ) -> String {
        let current = match self.authorize(&c) {
            Ok(current) => current,
            Err(cause) => return self.packet(op, &c, presentation::failure(&cause)),
        };
        self.authority_tick = current.anchor.tick.get();
        let mut work = match Work::new(&current, started) {
            Ok(work) => work,
            Err(cause) => return self.packet(op, &current, presentation::failure(&cause)),
        };
        let outcome =
            action.and_then(|action| self.perform(&current, &mut work, action, &mut checkpoint));
        self.authority_tick = self.high_tick();
        let mut final_context = current;
        final_context.anchor.tick = GameTick(self.high_tick());
        let result = outcome.unwrap_or_else(|cause| presentation::failure(&cause));
        let rendered = self.packet(op, &final_context, result);
        let final_check = (|| {
            if rendered.len() as u64 > presentation::OUTPUT_BYTES {
                return Err(exhausted());
            }
            checkpoint(&work.current(&final_context)?)?;
            self.authorize(&final_context)?;
            work.current(&final_context)?;
            Ok(())
        })();
        match final_check {
            Ok(()) => rendered,
            Err(cause) => self.packet(op, &final_context, presentation::failure(&cause)),
        }
    }
    /// Closing only releases this file owner. It never cancels an observation,
    /// erases history or settles a native effect. Explicit recovery release
    /// permits a damaged file or changed configuration to be retained as an
    /// unverified handoff; its callback checks runtime cancellation only.
    pub(super) fn close(
        &mut self,
        c: OperationContext,
        release: bool,
        started: Instant,
        mut checkpoint: impl FnMut(&OperationContext) -> Result<()>,
    ) -> String {
        let op = "fortress.cancel";
        let current = match self.current(&c) {
            Ok(current) => current,
            Err(cause) => return self.packet(op, &c, presentation::failure(&cause)),
        };
        let mut work = match Work::new(&current, started) {
            Ok(work) => work,
            Err(cause) => return self.packet(op, &current, presentation::failure(&cause)),
        };
        let result = (|| {
            if current.cancellation_requested {
                return Err(error(
                    ErrorCode::CancellationRequested,
                    "monitor recovery close cancelled",
                ));
            }
            checkpoint(&work.current(&current)?)?;
            let proof = if release {
                self.reference()
            } else {
                self.authorize(&current)?;
                let check = work.take(&current, monitor::FINAL_BYTES)?;
                let check = self.operation(&work, &current, &check)?;
                self.inspect(&check)?
            };
            checkpoint(&work.current(&current)?)?;
            Ok(json!({"ok":true,"scope":"session","closed":true,
                "release_for_recovery":release,"blueprint_monitor":proof,
                "monitor_cancelled":false,"effects_cancelled":false,
                "history_erased":false,"native_quiescence_proven":false}))
        })();
        let output = self.packet(
            op,
            &current,
            result.unwrap_or_else(|cause| presentation::failure(&cause)),
        );
        let final_check = (|| {
            if output.len() as u64 > presentation::OUTPUT_BYTES {
                return Err(exhausted());
            }
            checkpoint(&work.current(&current)?)?;
            work.current(&current)?;
            Ok(())
        })();
        match final_check {
            Ok(()) => output,
            Err(cause) => self.packet(op, &current, presentation::failure(&cause)),
        }
    }
}

fn denied() -> dfmcp_core::DfmcpError {
    error(
        ErrorCode::CapabilityDenied,
        "monitor recovery permits retained history queries and local monitor cancellation only",
    )
}

fn query_schema() -> Result<Value> {
    let mut schema: Value = serde_json::from_str(include_str!(
        "../../../../schemas/dig_control_query.json"
    ))
    .map_err(|_| {
        error(
            ErrorCode::InternalInvariantViolation,
            "invalid embedded monitor query schema",
        )
    })?;
    let alternatives = schema
        .get_mut("oneOf")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| {
            error(
                ErrorCode::InternalInvariantViolation,
                "monitor query alternatives absent",
            )
        })?;
    alternatives.retain(|value| {
        matches!(
            value["properties"]["mode"]["const"].as_str(),
            Some("schema" | "blueprint_monitor")
        )
    });
    if alternatives.len() != 2 {
        return Err(error(
            ErrorCode::InternalInvariantViolation,
            "monitor query alternatives changed",
        ));
    }
    schema["$id"] = json!("dfmcp.blueprint-monitor-recovery-query/1");
    Ok(schema)
}
