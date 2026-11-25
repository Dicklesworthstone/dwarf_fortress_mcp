//! Separately reviewed Run/1.13 time for an original resource-project stock watch.
//!
//! The durable target names the parent goal and its fixed production custody;
//! it is not an order, a Created receipt, or a serialized process-local monitor.
//! The parent must independently qualify the unchanged no-work goal after each
//! durable native marker. Recovery restores neither that sample nor a review.
use crate::live_production_runtime::ProjectStockGoal;
use crate::semantic_work_order_runtime::bounded_goal_clock::{
    self, ClockView, GoalClockTarget, RunRequest,
};
use dfmcp_adapter::bounded_run::journal::{RunJournal, RunState};
use dfmcp_adapter::bounded_run::rpc::RunSource;
use dfmcp_adapter::bounded_run::{MAX_NATIVE_TICK, RunObservation, RunPlan};
use dfmcp_adapter::control_effect_journal::EffectJournalStorage;
use dfmcp_core::{
    Capability, DfmcpError, Digest32, ErrorCode, FortressId, GameTick, OperationContext, Result,
    RiskTier, StateAnchor, WorkBudget,
};
use serde::Deserialize;
use serde_json::{Value, json};

const TARGET_SCHEMA: &str = "dfmcp.native-resource-stock-clock-target/1";
const CHECKPOINT_SCHEMA: &str = "dfmcp.native-resource-stock-clock/1";

fn corrupt(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::CorruptLedger, message)
}
fn denied(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::CapabilityDenied, message)
}
fn stale(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::StaleAnchor, message)
}
fn hash(domain: &[u8], fields: &[&[u8]]) -> Digest32 {
    let mut bytes = domain.to_vec();
    bytes.push(0);
    for field in fields {
        bytes.extend_from_slice(&(field.len() as u64).to_be_bytes());
        bytes.extend_from_slice(field);
    }
    Digest32::of_bytes(&bytes)
}
fn digest(raw: &str) -> Result<Digest32> {
    let value = Digest32::from_hex(raw)
        .filter(|value| *value != Digest32::ZERO && value.to_hex() == raw)
        .ok_or_else(|| corrupt("stock clock identity must be a nonzero canonical digest"))?;
    Ok(value)
}
fn goal_digest(goal: &ProjectStockGoal) -> Result<Digest32> {
    goal.validate()?;
    let bytes = serde_json::to_vec(goal)
        .map_err(|_| corrupt("original stock goal cannot be serialized"))?;
    Ok(hash(b"dfmcp-native-resource-stock-clock-goal/1", &[&bytes]))
}

/// Hash the complete acknowledged parent original using its canonical JSON
/// representation. The bounded traversal admits all bytes before encoding.
pub(in crate::native_resource_project) fn original_digest(original: &Value) -> Result<Digest32> {
    if original.get("schema").and_then(Value::as_str)
        != Some("dfmcp.native-resource-project-original/1")
    {
        return Err(corrupt("stock clock requires its complete parent original"));
    }
    let mut stack = vec![(original, 0usize)];
    let mut nodes = 0usize;
    let mut text_bytes = 0usize;
    while let Some((value, depth)) = stack.pop() {
        nodes = nodes.saturating_add(1);
        if nodes > 4096 || depth > 16 {
            return Err(corrupt(
                "stock clock parent original exceeds its shape bound",
            ));
        }
        let mut text = |value: &str| -> Result<()> {
            text_bytes = text_bytes.saturating_add(value.len());
            if value.len() > 16 * 1024 || text_bytes > 64 * 1024 || value.contains('\0') {
                return Err(corrupt(
                    "stock clock parent original exceeds its text bound",
                ));
            }
            Ok(())
        };
        match value {
            Value::String(value) => text(value)?,
            Value::Array(values) => {
                if values
                    .len()
                    .saturating_add(stack.len())
                    .saturating_add(nodes)
                    > 4096
                {
                    return Err(corrupt(
                        "stock clock parent original exceeds its node bound",
                    ));
                }
                stack.extend(values.iter().map(|value| (value, depth + 1)));
            }
            Value::Object(values) => {
                if values
                    .len()
                    .saturating_add(stack.len())
                    .saturating_add(nodes)
                    > 4096
                {
                    return Err(corrupt(
                        "stock clock parent original exceeds its node bound",
                    ));
                }
                for (key, value) in values {
                    text(key)?;
                    stack.push((value, depth + 1));
                }
            }
            Value::Number(value) if value.as_u64().is_none() && value.as_i64().is_none() => {
                return Err(corrupt(
                    "stock clock parent original has an unbounded numeric field",
                ));
            }
            _ => {}
        }
    }
    let bytes = serde_json::to_vec(original)
        .map_err(|_| corrupt("stock clock parent original cannot be serialized"))?;
    if bytes.len() > 64 * 1024 {
        return Err(corrupt(
            "stock clock parent original exceeds its encoded bound",
        ));
    }
    Ok(hash(
        b"dfmcp-native-resource-stock-clock-parent/1",
        &[&bytes],
    ))
}

/// Only stable native custody participates. Dynamic heads, source floors and
/// process-local watch witnesses cannot change this original target identity.
pub(in crate::native_resource_project) fn production_source_digest(
    reference: &Value,
) -> Result<Digest32> {
    if reference.get("schema").and_then(Value::as_str)
        != Some("dfmcp.project-production-recovery/1")
    {
        return Err(corrupt(
            "stock clock requires acknowledged production custody",
        ));
    }
    let custody = reference
        .get("native_custody")
        .and_then(Value::as_object)
        .filter(|object| object.len() == 2)
        .ok_or_else(|| corrupt("stock clock production native custody is incomplete"))?;
    let field = |name| {
        custody
            .get(name)
            .and_then(Value::as_str)
            .ok_or_else(|| corrupt("stock clock production native identity is missing"))
    };
    let workforce = digest(field("workforce_journal_id")?)?;
    let orders = digest(field("order_journal_id")?)?;
    Ok(hash(
        b"dfmcp-native-resource-stock-clock-production/1",
        &[workforce.as_bytes(), orders.as_bytes()],
    ))
}

/// Closed, compact recovery data. In particular there is no stock-watch witness,
/// sample counter, current truth, native order ID, or permission in this record.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Descriptor {
    schema: String,
    parent_journal_id: String,
    original_digest: String,
    original_goal_digest: String,
    production_source_digest: String,
    fortress_id: String,
    epoch: u64,
    original_tick: u64,
    deadline_tick: u64,
    target_digest: String,
    original_paired_seal: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::native_resource_project) struct WatchClockTarget {
    descriptor: Descriptor,
    digest: Digest32,
    original_seal: Digest32,
    fortress: FortressId,
}
impl WatchClockTarget {
    pub(in crate::native_resource_project) fn new(
        parent_journal_id: Digest32,
        original_digest: Digest32,
        goal: &ProjectStockGoal,
        production_source_digest: Digest32,
        origin: StateAnchor,
    ) -> Result<Self> {
        let mut descriptor = Descriptor {
            schema: TARGET_SCHEMA.into(),
            parent_journal_id: parent_journal_id.to_hex(),
            original_digest: original_digest.to_hex(),
            original_goal_digest: goal_digest(goal)?.to_hex(),
            production_source_digest: production_source_digest.to_hex(),
            fortress_id: origin.fortress_id.get().to_string(),
            epoch: origin.cursor.epoch,
            original_tick: origin.tick.get(),
            deadline_tick: goal.deadline_tick(),
            target_digest: String::new(),
            original_paired_seal: String::new(),
        };
        let (target, seal, _) = Self::identity(&descriptor)?;
        descriptor.target_digest = target.to_hex();
        descriptor.original_paired_seal = seal.to_hex();
        Self::from_descriptor(descriptor)
    }

    fn identity(descriptor: &Descriptor) -> Result<(Digest32, Digest32, FortressId)> {
        let parent = digest(&descriptor.parent_journal_id)?;
        let original = digest(&descriptor.original_digest)?;
        let goal = digest(&descriptor.original_goal_digest)?;
        let production = digest(&descriptor.production_source_digest)?;
        let fortress = descriptor
            .fortress_id
            .parse::<u64>()
            .map_err(|_| corrupt("invalid stock clock fortress identity"))?;
        if descriptor.schema != TARGET_SCHEMA
            || fortress == 0
            || fortress.to_string() != descriptor.fortress_id
            || descriptor.epoch >= u64::from(u32::MAX)
            || descriptor.original_tick >= descriptor.deadline_tick
            || descriptor.deadline_tick > MAX_NATIVE_TICK
        {
            return Err(corrupt(
                "stock clock changed its original scope or deadline",
            ));
        }
        let fields: &[&[u8]] = &[
            parent.as_bytes(),
            original.as_bytes(),
            goal.as_bytes(),
            production.as_bytes(),
            &fortress.to_be_bytes(),
            &descriptor.epoch.to_be_bytes(),
            &descriptor.original_tick.to_be_bytes(),
            &descriptor.deadline_tick.to_be_bytes(),
        ];
        Ok((
            hash(b"dfmcp-native-resource-stock-clock-target/1", fields),
            hash(b"dfmcp-native-resource-stock-clock-original/1", fields),
            FortressId::new(fortress),
        ))
    }

    fn from_descriptor(descriptor: Descriptor) -> Result<Self> {
        let (target, seal, fortress) = Self::identity(&descriptor)?;
        if digest(&descriptor.target_digest)? != target
            || digest(&descriptor.original_paired_seal)? != seal
        {
            return Err(corrupt(
                "stock clock target commitment does not match its descriptor",
            ));
        }
        Ok(Self {
            descriptor,
            digest: target,
            original_seal: seal,
            fortress,
        })
    }

    pub(in crate::native_resource_project) fn from_json(value: &Value) -> Result<Self> {
        let object = value
            .as_object()
            .filter(|object| object.len() == 11)
            .ok_or_else(|| corrupt("invalid closed stock clock target descriptor"))?;
        // Admit the entire small primitive record before cloning/deserializing.
        if object.iter().any(|(key, value)| {
            key.len() > 64
                || match value {
                    Value::String(text) => text.len() > 128,
                    Value::Number(number) => number.as_u64().is_none(),
                    _ => true,
                }
        }) {
            return Err(corrupt(
                "stock clock target descriptor exceeds its closed shape",
            ));
        }
        let descriptor: Descriptor = serde_json::from_value(value.clone())
            .map_err(|_| corrupt("invalid closed stock clock target fields"))?;
        Self::from_descriptor(descriptor)
    }

    pub(in crate::native_resource_project) fn to_json(&self) -> Value {
        let d = &self.descriptor;
        json!({
            "schema":d.schema, "parent_journal_id":d.parent_journal_id,
            "original_digest":d.original_digest, "original_goal_digest":d.original_goal_digest,
            "production_source_digest":d.production_source_digest, "fortress_id":d.fortress_id,
            "epoch":d.epoch, "original_tick":d.original_tick, "deadline_tick":d.deadline_tick,
            "target_digest":d.target_digest, "original_paired_seal":d.original_paired_seal,
        })
    }

    pub(in crate::native_resource_project) fn matches_goal(
        &self,
        goal: &ProjectStockGoal,
    ) -> Result<()> {
        if self.descriptor.original_goal_digest != goal_digest(goal)?.to_hex()
            || self.descriptor.deadline_tick != goal.deadline_tick()
        {
            return Err(denied(
                "stock clock cannot replace any original stock goal term",
            ));
        }
        Ok(())
    }

    pub(in crate::native_resource_project) fn matches_original(
        &self,
        parent: Digest32,
        original: Digest32,
        goal: &ProjectStockGoal,
        production: Digest32,
    ) -> Result<()> {
        self.matches_goal(goal)?;
        if self.descriptor.parent_journal_id != parent.to_hex()
            || self.descriptor.original_digest != original.to_hex()
            || self.descriptor.production_source_digest != production.to_hex()
        {
            return Err(denied(
                "stock clock differs from its acknowledged parent or production custody",
            ));
        }
        Ok(())
    }

    pub(in crate::native_resource_project) fn fortress(&self) -> FortressId {
        self.fortress
    }
    pub(in crate::native_resource_project) fn epoch(&self) -> u64 {
        self.descriptor.epoch
    }
    pub(in crate::native_resource_project) fn digest(&self) -> Digest32 {
        self.digest
    }
    pub(in crate::native_resource_project) fn original_paired_seal(&self) -> Digest32 {
        self.original_seal
    }
    pub(in crate::native_resource_project) fn original_tick(&self) -> u64 {
        self.descriptor.original_tick
    }
    pub(in crate::native_resource_project) fn deadline_tick(&self) -> u64 {
        self.descriptor.deadline_tick
    }
}
impl GoalClockTarget for WatchClockTarget {
    const REQUEST_KIND: &'static str = "stock_watch_run";
    const CANDIDATE_KEY: &'static str = "stock-watch-clock-candidate";
    const PURPOSE: &'static str = "advance_original_resource_stock_stability";

    fn digest(&self) -> Digest32 {
        self.digest
    }
    fn original_paired_seal(&self) -> Digest32 {
        self.original_seal
    }
    fn original_tick(&self) -> u64 {
        self.descriptor.original_tick
    }
    fn context(&self, c: &OperationContext) -> Result<()> {
        if c.anchor.fortress_id != self.fortress
            || c.anchor.cursor.epoch != self.descriptor.epoch
            || c.anchor.tick.get() < self.descriptor.original_tick
        {
            return Err(stale(
                "stock clock cannot cross its original fortress, epoch or tick floor",
            ));
        }
        // Do not apply the goal deadline here: overdue reconciliation and a
        // separately authorized safety pause must retain their original target.
        c.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)
    }
    fn capture(&self, capture: &RunObservation) -> Result<u64> {
        let tick = capture
            .tick()
            .filter(|tick| *tick >= self.descriptor.original_tick)
            .ok_or_else(|| stale("stock clock source precedes its original tick"))?;
        if !capture.eligible() {
            return Err(stale("stock clock requires a loaded paused source"));
        }
        Ok(tick)
    }
    fn plan(&self, plan: &RunPlan) -> Result<()> {
        let tick = self.capture(plan.before())?;
        if tick
            .checked_add(u64::from(plan.spec().game_ticks()))
            .is_none_or(|end| end > self.descriptor.deadline_tick)
            || plan.key() != self.key(plan.digest())
        {
            return Err(corrupt(
                "stock clock run changes the original target or deadline",
            ));
        }
        Ok(())
    }
    fn key(&self, plan: Digest32) -> String {
        let digest = hash(
            b"dfmcp-native-resource-stock-clock-key/1",
            &[self.digest.as_bytes(), plan.as_bytes()],
        );
        format!("stock-watch-clock-{}", digest.to_hex())
    }
    fn seal(&self, journal: Digest32, plan: &RunPlan) -> Digest32 {
        hash(
            b"dfmcp-native-resource-stock-clock-review/1",
            &[
                self.digest.as_bytes(),
                journal.as_bytes(),
                &plan.canonical_bytes(),
            ],
        )
    }
}

/// Exact parent-held journal pin. Its counters are rejection floors, never
/// restored pause, stock, stability or review evidence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::native_resource_project) struct ClockCheckpoint {
    target: WatchClockTarget,
    journal_id: Digest32,
    head: Digest32,
    high_tick: u64,
    high_sequence: u64,
}
impl ClockCheckpoint {
    pub(in crate::native_resource_project) fn prospective(
        target: WatchClockTarget,
        journal_id: Digest32,
    ) -> Result<Self> {
        if journal_id == Digest32::ZERO {
            return Err(corrupt(
                "stock clock requires its exact prospective journal identity",
            ));
        }
        let high_tick = target.original_tick();
        Ok(Self {
            target,
            journal_id,
            head: journal_id,
            high_tick,
            high_sequence: 0,
        })
    }

    pub(in crate::native_resource_project) fn from_json(value: &Value) -> Result<Self> {
        let object = value
            .as_object()
            .filter(|object| object.len() == 6)
            .ok_or_else(|| corrupt("invalid closed stock clock checkpoint"))?;
        if object.get("schema").and_then(Value::as_str) != Some(CHECKPOINT_SCHEMA) {
            return Err(corrupt("invalid stock clock checkpoint schema"));
        }
        let target = WatchClockTarget::from_json(
            object
                .get("target")
                .ok_or_else(|| corrupt("stock clock target is missing"))?,
        )?;
        let field = |name| {
            object
                .get(name)
                .and_then(Value::as_str)
                .ok_or_else(|| corrupt("stock clock checkpoint identity is missing"))
        };
        let journal_id = digest(field("journal_id")?)?;
        let head = digest(field("head")?)?;
        let high_tick = object
            .get("authority_tick_floor")
            .and_then(Value::as_u64)
            .filter(|tick| *tick >= target.original_tick() && *tick <= MAX_NATIVE_TICK)
            .ok_or_else(|| corrupt("stock clock checkpoint tick floor is invalid"))?;
        let high_sequence = object
            .get("high_source_sequence")
            .and_then(Value::as_u64)
            .ok_or_else(|| corrupt("stock clock checkpoint source floor is invalid"))?;
        Ok(Self {
            target,
            journal_id,
            head,
            high_tick,
            high_sequence,
        })
    }

    pub(in crate::native_resource_project) fn to_json(&self) -> Value {
        json!({
            "schema":CHECKPOINT_SCHEMA, "target":self.target.to_json(),
            "journal_id":self.journal_id.to_hex(), "head":self.head.to_hex(),
            "authority_tick_floor":self.high_tick, "high_source_sequence":self.high_sequence,
        })
    }

    /// The actual journal still verifies ancestry. Parent monotonicity cannot
    /// authorize new data or substitute for the retained private file.
    pub(in crate::native_resource_project) fn validate_successor(&self, next: &Self) -> Result<()> {
        if self.target != next.target
            || self.journal_id != next.journal_id
            || self.high_tick > next.high_tick
            || self.high_sequence > next.high_sequence
            || (self.head != self.journal_id && next.head == next.journal_id)
        {
            return Err(corrupt(
                "stock clock recovery replaced its target or regressed journal/source custody",
            ));
        }
        Ok(())
    }

    pub(in crate::native_resource_project) fn is_initial(&self) -> bool {
        self.head == self.journal_id
            && self.high_sequence == 0
            && self.high_tick == self.target.original_tick()
    }
    pub(in crate::native_resource_project) fn target(&self) -> &WatchClockTarget {
        &self.target
    }
    pub(in crate::native_resource_project) fn journal_id(&self) -> Digest32 {
        self.journal_id
    }
    pub(in crate::native_resource_project) fn head(&self) -> Digest32 {
        self.head
    }
    pub(in crate::native_resource_project) fn high_tick(&self) -> u64 {
        self.high_tick
    }
    pub(in crate::native_resource_project) fn high_sequence(&self) -> u64 {
        self.high_sequence
    }
}

pub(in crate::native_resource_project) struct StockClock<J: EffectJournalStorage> {
    inner: bounded_goal_clock::Clock<J, WatchClockTarget>,
    target: WatchClockTarget,
    latest: Option<ClockView>,
    inspection_required: bool,
    current_review: Option<Digest32>,
}
impl<J: EffectJournalStorage> StockClock<J> {
    pub(in crate::native_resource_project) fn new(
        journal: RunJournal<J>,
        target: WatchClockTarget,
    ) -> Self {
        Self {
            inner: bounded_goal_clock::Clock::new(journal),
            target,
            latest: None,
            inspection_required: true,
            current_review: None,
        }
    }
    pub(in crate::native_resource_project) fn target(&self) -> &WatchClockTarget {
        &self.target
    }
    pub(in crate::native_resource_project) fn latest(&self) -> Option<&ClockView> {
        self.latest.as_ref()
    }
    pub(in crate::native_resource_project) fn is_control(&self) -> bool {
        self.inner.is_control()
    }
    pub(in crate::native_resource_project) fn high_tick(&self) -> u64 {
        self.inner.high_tick().max(self.target.original_tick())
    }
    pub(in crate::native_resource_project) fn pending(&self) -> bool {
        self.inspection_required || self.latest.as_ref().is_none_or(|view| view.pending)
    }
    pub(in crate::native_resource_project) fn may_be_running(&self) -> bool {
        self.inspection_required || self.latest.as_ref().is_none_or(ClockView::may_be_running)
    }
    pub(in crate::native_resource_project) fn current_review(&self) -> Option<Digest32> {
        if self.inspection_required {
            return None;
        }
        let active = self.latest.as_ref()?.active()?;
        self.current_review.filter(|seal| {
            *seal == active.review_seal
                && active.entry.state() == RunState::Prepared
                && !active.entry.unresolved()
        })
    }
    pub(in crate::native_resource_project) fn withdraw_review(&mut self) {
        self.current_review = None;
    }

    /// Copy an unchanged, undispatched interval; never manufacture a new key,
    /// shorten a retained interval or turn its journal into stock evidence.
    pub(in crate::native_resource_project) fn retained_review_request(
        &self,
        tick: u64,
        budget: WorkBudget,
    ) -> Option<RunRequest> {
        if self.inspection_required || !self.is_control() {
            return None;
        }
        let view = self.latest.as_ref()?;
        let run = view.active()?;
        let plan = run.entry.plan();
        let ticks = plan.spec().game_ticks();
        let wall = plan.spec().wall_ms();
        let valid = run.entry.state() == RunState::Prepared
            && !view.unresolved
            && !run.entry.unresolved()
            && !view.may_be_running()
            && plan.before().tick() == Some(tick)
            && tick >= self.high_tick()
            && plan.before().sequence() >= view.high_sequence
            && u64::from(ticks) <= budget.max_game_ticks
            && budget.max_wall_millis > 0
            && self.target.plan(plan).is_ok();
        valid.then(|| RunRequest {
            kind: WatchClockTarget::REQUEST_KIND.into(),
            original_paired_seal: self.target.original_paired_seal().to_hex(),
            game_ticks: ticks,
            wall_millis: wall,
        })
    }
    pub(in crate::native_resource_project) fn recovery_checkpoint(&self) -> Value {
        let (journal_id, head, high_tick, high_sequence) = self.inner.recovery_checkpoint();
        ClockCheckpoint {
            target: self.target.clone(),
            journal_id,
            head,
            high_tick: high_tick.max(self.target.original_tick()),
            high_sequence,
        }
        .to_json()
    }
    pub(in crate::native_resource_project) fn verify_recovery(
        &mut self,
        value: &Value,
        c: &OperationContext,
        n: &OperationContext,
    ) -> Result<()> {
        self.current_review = None;
        self.inspection_required = true;
        let checkpoint = ClockCheckpoint::from_json(value)?;
        if checkpoint.target != self.target {
            return Err(corrupt(
                "stock clock recovery changed its immutable original target",
            ));
        }
        self.inner.verify_recovery(
            checkpoint.journal_id,
            checkpoint.head,
            checkpoint.high_tick,
            checkpoint.high_sequence,
            c,
            n,
        )
    }
    fn remember(&mut self, result: &Result<ClockView>) {
        if let Ok(view) = result {
            self.latest = Some(view.clone());
            self.inspection_required = false;
        }
    }
    pub(in crate::native_resource_project) fn inspect(
        &mut self,
        c: &OperationContext,
        n: &OperationContext,
    ) -> Result<ClockView> {
        self.inspection_required = true;
        let result = self.inner.inspect(Some(&self.target), c, n);
        self.remember(&result);
        if result.as_ref().is_err()
            || self
                .latest
                .as_ref()
                .and_then(ClockView::active)
                .is_none_or(|run| {
                    run.entry.state() != RunState::Prepared
                        || run.entry.plan().before().tick() != Some(c.anchor.tick.get())
                })
        {
            self.current_review = None;
        }
        result
    }
    pub(in crate::native_resource_project) fn prepare_guarded<N, F>(
        &mut self,
        request: &RunRequest,
        source: &mut N,
        c: &OperationContext,
        n: &OperationContext,
        mut guard: F,
    ) -> Result<ClockView>
    where
        N: RunSource,
        F: FnMut(&OperationContext) -> Result<()>,
    {
        self.current_review = None;
        // The shared coordinator deliberately reuses an identical preparation
        // without another native PrepareRun. Recheck the complete current
        // authority and the parent's fresh stock/custody guard on that path too.
        if self
            .latest
            .as_ref()
            .and_then(ClockView::active)
            .is_some_and(|run| run.entry.state() == RunState::Prepared)
        {
            let retained = self
                .retained_review_request(c.anchor.tick.get(), c.budget)
                .ok_or_else(|| denied("retained stock clock needs its exact eligible interval"))?;
            if retained != *request
                || self
                    .retained_review_request(c.anchor.tick.get(), n.budget)
                    .as_ref()
                    != Some(request)
                || n.anchor.tick > c.anchor.tick
            {
                return Err(denied(
                    "retained stock clock cannot change its interval or allowance",
                ));
            }
            self.target.context(c)?;
            c.authorize(Capability::Observe, RiskTier::ReadOnly, &[], None)?;
            let end = c
                .anchor
                .tick
                .get()
                .checked_add(u64::from(request.game_ticks))
                .ok_or_else(|| denied("retained stock clock horizon overflow"))?;
            for context in [c, n] {
                context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
                context.authorize(Capability::Plan, RiskTier::Guarded, &[], None)?;
                let mut horizon = context.clone();
                horizon.anchor.tick = GameTick(end);
                horizon.authorize(Capability::ControlClock, RiskTier::Guarded, &[], None)?;
            }
            guard(c)?;
        }
        self.inspection_required = true;
        let result = self
            .inner
            .prepare_guarded(&self.target, request, source, c, n, guard);
        self.remember(&result);
        if let Ok(view) = &result {
            self.current_review = view
                .active()
                .filter(|run| run.entry.state() == RunState::Prepared && !run.entry.unresolved())
                .map(|run| run.review_seal);
        }
        result
    }
    pub(in crate::native_resource_project) fn commit_guarded<N, F>(
        &mut self,
        seal: Digest32,
        confirmed: bool,
        source: &mut N,
        c: &OperationContext,
        n: &OperationContext,
        guard: F,
    ) -> Result<ClockView>
    where
        N: RunSource,
        F: FnMut(&OperationContext) -> Result<()>,
    {
        let current = self.current_review();
        self.current_review = None;
        self.inspection_required = true;
        if !confirmed || current != Some(seal) {
            return Err(denied(
                "prepare and confirm this owner's current stock clock review",
            ));
        }
        let result = self
            .inner
            .commit_guarded(&self.target, seal, true, source, c, n, guard);
        self.remember(&result);
        result
    }
    pub(in crate::native_resource_project) fn reconcile<N: RunSource>(
        &mut self,
        source: &mut N,
        c: &OperationContext,
        n: &OperationContext,
    ) -> Result<ClockView> {
        self.current_review = None;
        self.inspection_required = true;
        let result = self.inner.reconcile(&self.target, source, c, n);
        self.remember(&result);
        result
    }
    pub(in crate::native_resource_project) fn cancel(
        &mut self,
        source: Option<&mut dyn RunSource>,
        c: &OperationContext,
        n: &OperationContext,
    ) -> Result<ClockView> {
        self.current_review = None;
        self.inspection_required = true;
        let result = self.inner.cancel(&self.target, source, c, n);
        self.remember(&result);
        result
    }
    pub(in crate::native_resource_project) fn to_json(&self) -> Value {
        let mut value = self.latest.as_ref().map_or_else(
            || {
                json!({
                    "journal_id":self.inner.recovery_checkpoint().0.to_hex(),
                    "head":self.inner.recovery_checkpoint().1.to_hex(),
                    "active_run":null, "runs":[], "run_count":0,
                    "current_pause_proven":false, "original_goal_proven":false,
                })
            },
            ClockView::to_json,
        );
        value["schema"] = json!("dfmcp.native-resource-stock-clock-view/1");
        value["target"] = self.target.to_json();
        value["target_digest"] = json!(self.target.digest().to_hex());
        value["original_paired_seal"] = json!(self.target.original_paired_seal().to_hex());
        value["target_scope"] = json!("original_resource_project_stock_goal");
        value["source"] = json!("run/1.13");
        value["inspection_required"] = json!(self.inspection_required);
        value["pending"] = json!(self.pending());
        value["unresolved"] = json!(self.may_be_running());
        value["may_be_running"] = json!(self.may_be_running());
        value["review_available"] = json!(self.current_review().is_some());
        value["high_tick"] = json!(self.high_tick());
        value["high_source_sequence"] = json!(self.inner.recovery_checkpoint().3);
        // A refused operation can advance the actual journal or rejection
        // floors without publishing a newer inspected history view.
        value["recovery_checkpoint"] = self.recovery_checkpoint();
        value["sample_history_durable"] = json!(false);
        value["recovery_restores_current_proof"] = json!(false);
        value
    }
}

#[cfg(test)]
#[path = "stock_clock_tests.rs"]
mod tests;
