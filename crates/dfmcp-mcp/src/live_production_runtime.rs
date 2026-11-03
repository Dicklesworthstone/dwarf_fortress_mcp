//! One original labor -> finite-order plan, owned by one development session.
//! Native effect and durable frontier decisions remain in their existing owners.
use crate::{semantic_work_order_runtime as orders, semantic_workforce_runtime as workforce};
use dfmcp_adapter::LiveObservationCapsule;
use dfmcp_adapter::build_placement::journal::BuildMode;
use dfmcp_adapter::live_dispatch::durable::{self, Coordinator, Phase};
use dfmcp_adapter::live_production::{self, Executor};
use dfmcp_adapter::live_projection::{LiveWorldProjection, project_live_capsule};
use dfmcp_adapter::plan_archive::{create_private_plan_archive, open_private_plan_archive};
use dfmcp_adapter::semantic_work_orders::SemanticWorkOrderSession;
use dfmcp_adapter::semantic_workforce::SemanticWorkforceSession;
use dfmcp_adapter::workforce_control::journal::WorkforceBinding;
use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, Digest32, GameTick, ObservationCursor,
    OperationContext, RequestId, Result, RiskTier, SessionId, StateAnchor, WorkBudget,
};
use dfmcp_intent::plan_archive::PlanArchiveSeal;
use dfmcp_intent::{Action, PreparedPlan};
use dfmcp_world::PredicateEvidence;
use serde_json::{Value, json};
use std::time::{Duration, Instant};

mod config;
mod custody;
mod request;
mod review;
pub(crate) use config::Config;
use custody::{GuardedEvidence, SharedCustody};
use orders::Shell as OrderShell;
pub(crate) use request::Request;
pub(crate) use workforce::Mode;
use workforce::Shell as WorkforceShell;
use workforce::{denied, exhausted, invalid};

pub(crate) const MAX_BYTES: u64 = 32 * 1024 * 1024 * 1024;
pub(crate) const MAX_WALL_MILLIS: u64 = 60_000;
pub(crate) const MAX_OUTPUT_TOKENS: u32 = 32_768;
const MIB: u64 = 1024 * 1024;
const MAX_FRONTIER_BYTES: u64 = 2 * 1024 * MIB;
type W = workforce::native::NativeShell;
type O = orders::native::NativeShell;
type File = dfmcp_adapter::build_placement::journal::private_file::PrivateBuildFile;
type Labor =
    SemanticWorkforceSession<<W as WorkforceShell>::Journal, <W as WorkforceShell>::Association>;
type Order = SemanticWorkOrderSession<
    <O as OrderShell>::Journal,
    <O as OrderShell>::Association,
    <O as OrderShell>::Native,
>;
type Factory = Box<
    dyn FnMut(&WorkforceBinding, &OperationContext) -> Result<<W as WorkforceShell>::Native> + Send,
>;
type Execution = Executor<
    <W as WorkforceShell>::Journal,
    <W as WorkforceShell>::Association,
    <O as OrderShell>::Journal,
    <O as OrderShell>::Association,
    <O as OrderShell>::Native,
    GuardedEvidence<workforce::native::NativeEvidence>,
    Factory,
>;

struct Work {
    context: OperationContext,
    until: Instant,
    bytes: u64,
}

#[cfg(test)]
#[path = "live_production_runtime/budget_tests.rs"]
mod budget_tests;
impl Work {
    fn new(c: &OperationContext) -> Result<Self> {
        c.budget.validate()?;
        Ok(Self {
            context: c.clone(),
            bytes: c.budget.max_bytes,
            until: Instant::now()
                .checked_add(Duration::from_millis(c.budget.max_wall_millis))
                .ok_or_else(|| exhausted("combined deadline overflow"))?,
        })
    }
    fn current(&self) -> Result<OperationContext> {
        let left = self
            .until
            .checked_duration_since(Instant::now())
            .filter(|v| v.as_millis() > 0)
            .ok_or_else(|| exhausted("combined operation deadline expired"))?;
        let mut c = self.context.clone();
        c.budget.max_wall_millis =
            u64::try_from(left.as_millis()).map_err(|_| exhausted("combined deadline overflow"))?;
        c.budget.max_bytes = self.bytes;
        Ok(c)
    }
    fn take(&mut self, amount: u64) -> Result<OperationContext> {
        let mut c = self.current()?;
        self.bytes = self
            .bytes
            .checked_sub(amount)
            .ok_or_else(|| exhausted("combined byte-work exhausted"))?;
        c.budget.max_bytes = amount;
        Ok(c)
    }
    fn rest(&mut self, ceiling: u64) -> Result<OperationContext> {
        self.take(self.bytes.min(ceiling))
    }
}

struct Basis {
    capsule: LiveObservationCapsule,
    projection: LiveWorldProjection,
    seal: Digest32,
}

pub(crate) struct Runtime {
    config: Config,
    mode: Mode,
    session: SessionId,
    request: u128,
    budget: WorkBudget,
    anchor: StateAnchor,
    opening_labor: bool,
    opening_orders: bool,
    parts: Option<(Labor, Order)>,
    execution: Option<Execution>,
    frontier: Option<Coordinator<File>>,
    custody: Option<SharedCustody>,
    original: Option<PreparedPlan>,
    seal: Option<PlanArchiveSeal>,
    identity_digest: Option<Digest32>,
    basis: Option<Basis>,
    workforce_journal: Digest32,
    order_journal: Digest32,
    initialization_failed: bool,
    fenced: bool,
}

pub(crate) struct View {
    value: Value,
}
impl View {
    pub(crate) fn to_json(&self) -> Value {
        self.value.clone()
    }
}

fn build_mode(mode: Mode) -> BuildMode {
    match mode {
        Mode::Control => BuildMode::Control,
        Mode::Reconcile => BuildMode::Recover,
        Mode::Offline => BuildMode::Offline,
    }
}
fn order_mode(mode: Mode) -> orders::Mode {
    match mode {
        Mode::Control => orders::Mode::Control,
        Mode::Reconcile => orders::Mode::Reconcile,
        Mode::Offline => orders::Mode::Offline,
    }
}
fn host_plan(plan: &PreparedPlan) -> Result<PlanArchiveSeal> {
    let seal = live_production::validate_plan(plan)?;
    if plan.steps.len() != 2
        || !matches!(plan.steps[0].action, Action::SetLabor { .. })
        || !matches!(plan.steps[1].action, Action::CreateWorkOrder { .. })
        || !plan.steps[0].depends_on.is_empty()
        || plan.steps[1].depends_on != [plan.steps[0].id]
    {
        return Err(invalid(
            "combined host requires its original labor then dependent order plan",
        ));
    }
    Ok(seal)
}
fn observation_seal(
    capsule: &LiveObservationCapsule,
    projection: &LiveWorldProjection,
) -> Digest32 {
    let mut bytes = b"dfmcp-combined-production-observation/1\0".to_vec();
    bytes.extend_from_slice(capsule.content_digest.as_bytes());
    bytes.extend_from_slice(projection.snapshot.state_hash.as_bytes());
    bytes.extend_from_slice(&projection.snapshot.cursor.epoch.to_be_bytes());
    bytes.extend_from_slice(&projection.snapshot.cursor.sequence.to_be_bytes());
    Digest32::of_bytes(&bytes)
}
pub(crate) fn anchor_json(a: StateAnchor) -> Value {
    json!({"fortress_id":a.fortress_id.get().to_string(),"epoch":a.cursor.epoch,
        "sequence":a.cursor.sequence,"tick":a.tick.get(),"state_hash":a.state_hash.to_hex()})
}
fn frontier_json(v: &durable::View) -> Value {
    json!({"journal_id":v.recovery.journal_id.to_hex(),"head":v.recovery.head.to_hex(),
        "authority_tick_floor":v.high_tick,"all_applied":v.all_applied(),
        "needs_reconciliation":v.needs_reconciliation(),"next_ready":v.next_ready.map(|s|s.get()),
        "review":v.review.as_ref().map(|r|json!({"step_id":r.step.get(),"family":r.family.protocol(),
            "paired_seal":r.seal.to_hex(),"preparation_digest":r.preparation.digest.to_hex()})),
        "entries":v.entries.iter().map(|e|json!({"step_id":e.step.get(),"family":e.family.protocol(),
            "idempotency_key":e.idempotency_key,"depends_on":e.depends_on.iter().map(|s|s.get()).collect::<Vec<_>>(),
            "phase":e.phase.as_str(),"dispatch_attempted":e.dispatch_attempted,
            "preparation_digest":e.preparation.map(|p|p.digest.to_hex()),
            "receipt_digest":e.receipt.map(|r|r.digest.to_hex())})).collect::<Vec<_>>()})
}
fn plan_json(plan: &PreparedPlan) -> Value {
    json!({"plan_digest":plan.digest.to_hex(),"intent_id":plan.intent_id.to_string(),
        "anchor":anchor_json(plan.anchor),"summary":plan.summary,"expires_at_tick":plan.expires_at_tick.get(),
        "terminal":crate::lab_world::predicate_json(&plan.terminal_condition),
        "steps":plan.steps.iter().map(|s|{
            let action = match &s.action {
                Action::SetLabor {units,labor,enabled} => json!({"kind":"set_labor","units":units.iter().map(|u|u.get().to_string()).collect::<Vec<_>>(),"labor":labor,"enabled":enabled}),
                Action::CreateWorkOrder {name,job_token,amount,..} => json!({"kind":"create_work_order","name":name,"job_token":job_token,"amount":amount}),
                _ => json!({"kind":"unsupported"}),
            };
            json!({"step_id":s.id.get(),"key":s.idempotency_key,"action":action,
                "depends_on":s.depends_on.iter().map(|d|d.get()).collect::<Vec<_>>(),
                "preconditions":s.preconditions.iter().map(crate::lab_world::predicate_json).collect::<Vec<_>>(),
                "postconditions":s.postconditions.iter().map(crate::lab_world::predicate_json).collect::<Vec<_>>(),
                "obligation":s.obligation.as_ref().map(|o|json!({"deadline_tick":o.deadline_tick.get(),"poll_interval_ticks":o.poll_interval_ticks,"stable_samples":o.stable_for_observations}))})
        }).collect::<Vec<_>>()})
}

impl Runtime {
    pub(crate) fn open(
        config: Config,
        mode: Mode,
        session: SessionId,
        budget: WorkBudget,
    ) -> Result<Self> {
        config.current()?;
        budget.validate()?;
        if session == SessionId::NIL
            || budget.max_bytes > MAX_BYTES
            || budget.max_wall_millis > MAX_WALL_MILLIS
            || budget.max_actions != 2
            || budget.max_entities < 4096
            || budget.max_output_tokens > MAX_OUTPUT_TOKENS
            || budget.max_game_ticks != 0
        {
            return Err(exhausted(
                "combined runtime requires two actions, 4096 entities and zero game ticks",
            ));
        }
        let opening_labor = mode == Mode::Control && config.workforce.labor_enabled();
        let opening_orders = mode == Mode::Control && config.orders.production_enabled();
        if config.initialize && (!opening_labor || !opening_orders) {
            return Err(denied(
                "initializing both native custodians requires both explicit family authorities",
            ));
        }
        let anchor = config.recovery.map(|r| r.anchor).unwrap_or(StateAnchor {
            fortress_id: config.workforce.acceptance().fortress(),
            cursor: ObservationCursor::ORIGIN,
            tick: GameTick(0),
            state_hash: Digest32::ZERO,
        });
        let mut out = Self {
            config,
            mode,
            session,
            request: 1,
            budget,
            anchor,
            opening_labor,
            opening_orders,
            parts: None,
            execution: None,
            frontier: None,
            custody: None,
            original: None,
            seal: None,
            identity_digest: None,
            basis: None,
            workforce_journal: Digest32::ZERO,
            order_journal: Digest32::ZERO,
            initialization_failed: false,
            fenced: false,
        };
        let c = out.context(false)?;
        let mut work = Work::new(&c)?;
        let (w, wa, wid) = out
            .config
            .workforce
            .open_control(mode, &work.take(512 * MIB)?)?;
        let labor = SemanticWorkforceSession::new(w, wa, &work.take(256 * MIB)?)?;
        let (o, oa, oid) = out
            .config
            .orders
            .open_control(order_mode(mode), &work.take(128 * MIB)?)?;
        let order = SemanticWorkOrderSession::new(o, oa, &work.take(64 * MIB)?)?;
        out.workforce_journal = wid;
        out.order_journal = oid;
        out.parts = Some((labor, order));
        if let Some(recovery) = out.config.recovery {
            let mut archive = open_private_plan_archive(
                &out.config.path("original.plan"),
                recovery.seal,
                live_production::archive_limits(),
                &work.take(4 * MIB)?,
            )?;
            let original = archive.recover(&work.take(4 * MIB)?)?;
            host_plan(&original)?;
            if original.anchor != recovery.anchor {
                return Err(denied(
                    "combined original anchor differs from external recovery",
                ));
            }
            let mut identity = workforce::identity::open_private_capsule(
                &out.config.path("original.citizens"),
                recovery.identity_digest,
                &work.take(32 * MIB)?,
            )?;
            let capsule = identity.recover(&work.take(32 * MIB)?)?;
            let projection = project_live_capsule(
                &capsule,
                original.anchor.fortress_id,
                original.anchor.cursor,
            )?;
            if projection.snapshot.anchor() != original.anchor {
                return Err(denied(
                    "combined original identity differs from archived plan",
                ));
            }
            out.basis = Some(Basis {
                seal: observation_seal(&capsule, &projection),
                capsule,
                projection,
            });
            out.seal = Some(recovery.seal);
            out.identity_digest = Some(recovery.identity_digest);
            out.custody = Some(SharedCustody::new(archive, identity, original.clone()));
            out.original = Some(original.clone());
            out.frontier = Some(durable::open_private_coordinator(
                &out.config.path("execution.journal"),
                &work.take(16 * MIB)?,
                build_mode(mode),
                Some(original),
                recovery.frontier,
                false,
            )?);
            out.create_executor(&mut work)?;
        } else {
            let (labor, order) = out
                .parts
                .as_mut()
                .ok_or_else(|| invalid("combined native owners absent"))?;
            if !labor.inventory(&work.take(144 * MIB)?)?.records.is_empty()
                || !order.inventory(&work.take(16 * MIB)?)?.is_empty()
            {
                return Err(denied(
                    "existing native work requires the exact combined original recovery",
                ));
            }
        }
        out.config.current()?;
        work.current()?;
        Ok(out)
    }
    pub(crate) fn session_id(&self) -> SessionId {
        self.session
    }
    pub(crate) fn mode(&self) -> Mode {
        self.mode
    }
    pub(crate) fn budget(&self) -> WorkBudget {
        self.budget
    }
    pub(crate) fn anchor(&self) -> StateAnchor {
        self.anchor
    }
    fn labor_enabled(&self) -> bool {
        self.opening_labor && self.config.workforce.labor_enabled()
    }
    fn orders_enabled(&self) -> bool {
        self.opening_orders && self.config.orders.production_enabled()
    }
    fn grants(&self) -> Vec<CapabilityGrant> {
        let mut capabilities = vec![Capability::Query, Capability::Observe];
        if self.labor_enabled() || self.orders_enabled() {
            capabilities.push(Capability::Plan);
        }
        if self.labor_enabled() {
            capabilities.push(Capability::ConfigureLabor);
        }
        if self.orders_enabled() {
            capabilities.push(Capability::ConfigureProduction);
        }
        capabilities
            .into_iter()
            .map(|capability| CapabilityGrant {
                capability,
                scope: CapabilityScope {
                    fortress_id: Some(self.anchor.fortress_id),
                    ..CapabilityScope::default()
                },
                max_risk: RiskTier::Guarded,
                expires_at_tick: None,
                remaining_uses: None,
            })
            .collect()
    }
    pub(crate) fn context(&mut self, cancelled: bool) -> Result<OperationContext> {
        self.config.current()?;
        self.request = self
            .request
            .checked_add(1)
            .ok_or_else(|| exhausted("combined request IDs exhausted"))?;
        Ok(OperationContext {
            session_id: self.session,
            request_id: RequestId::new(self.request),
            anchor: self.anchor,
            budget: self.budget,
            grants: self.grants(),
            cancellation_requested: cancelled,
        })
    }
    fn check(&self, c: &OperationContext, control: bool) -> Result<()> {
        self.config.current()?;
        if c.session_id != self.session
            || c.anchor != self.anchor
            || c.grants != self.grants()
            || c.budget.max_bytes > self.budget.max_bytes
            || c.budget.max_wall_millis > self.budget.max_wall_millis
            || c.budget.max_entities > self.budget.max_entities
            || c.budget.max_actions > 2
            || c.budget.max_output_tokens > self.budget.max_output_tokens
            || c.budget.max_game_ticks != 0
        {
            return Err(denied(
                "combined operation differs from retained scope or widens its budget",
            ));
        }
        c.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
        if control {
            if self.mode != Mode::Control || self.initialization_failed || self.fenced {
                return Err(denied(
                    "combined control is unavailable; retain exact recovery custody",
                ));
            }
            c.authorize(Capability::Plan, RiskTier::Guarded, &[], None)?;
        }
        Ok(())
    }
    fn arm(&self, work: &mut Work) -> Result<()> {
        if let Some(custody) = &self.custody {
            let allowance = (work.bytes / 2).min(512 * MIB);
            custody.arm(&work.take(allowance)?)?;
        }
        Ok(())
    }
    fn create_executor(&mut self, work: &mut Work) -> Result<()> {
        let result = self.create_executor_inner(work);
        if result.is_err() {
            self.abandon_preparation();
        }
        result
    }
    fn create_executor_inner(&mut self, work: &mut Work) -> Result<()> {
        let original = self
            .original
            .clone()
            .ok_or_else(|| invalid("combined original absent"))?;
        let custody = self
            .custody
            .as_ref()
            .ok_or_else(|| invalid("combined parent custody absent"))?
            .clone();
        self.arm(work)?;
        let evidence = if self.mode == Mode::Offline {
            None
        } else {
            let capsule = self
                .basis
                .as_ref()
                .ok_or_else(|| invalid("combined identity absent"))?
                .capsule
                .clone();
            Some(GuardedEvidence::new(
                self.config
                    .workforce
                    .evidence(capsule, &work.take(32 * MIB)?)?,
                custody.clone(),
            ))
        };
        let shell = self.config.workforce.clone();
        let factory: Factory = Box::new(move |binding, c| shell.native(binding, c));
        let (mut w, mut o) = self
            .parts
            .take()
            .ok_or_else(|| invalid("combined native owners already consumed"))?;
        let labor_keys = w
            .inventory(&work.take(144 * MIB)?)?
            .records
            .iter()
            .map(|r| r.plan().key().to_owned())
            .collect::<std::collections::BTreeSet<_>>();
        let order_keys = o
            .inventory(&work.take(16 * MIB)?)?
            .iter()
            .map(|r| r.plan().key().to_owned())
            .collect::<std::collections::BTreeSet<_>>();
        if let Some(frontier) = &mut self.frontier {
            for entry in frontier.inspect(&work.take(8 * MIB)?)?.entries {
                if entry.preparation.is_some()
                    || entry.receipt.is_some()
                    || entry.dispatch_attempted
                {
                    let found = match entry.family {
                        dfmcp_adapter::live_routing::LiveFamily::WorkforceV1_17 => {
                            labor_keys.contains(&entry.idempotency_key)
                        }
                        dfmcp_adapter::live_routing::LiveFamily::WorkOrdersV1_10 => {
                            order_keys.contains(&entry.idempotency_key)
                        }
                        _ => false,
                    };
                    if !found {
                        return Err(denied(
                            "combined frontier lost its original native effect custody",
                        ));
                    }
                }
            }
        }
        let result = Executor::new(original, w, o, evidence, factory, &work.take(768 * MIB)?);
        custody.disarm()?;
        self.execution = Some(result?);
        Ok(())
    }
    pub(crate) fn abandon_preparation(&mut self) {
        if let Some(execution) = &mut self.execution {
            execution.abandon_preparation();
        }
        if let Some(frontier) = &mut self.frontier {
            frontier.abandon_review();
        }
        if let Some(custody) = &self.custody
            && custody.disarm().is_err()
        {
            self.fenced = true;
        }
    }
    fn recovery_json(&self) -> Value {
        match (
            self.original.as_ref(),
            self.seal,
            self.identity_digest,
            self.frontier.as_ref(),
        ) {
            (Some(p), Some(seal), Some(identity), Some(frontier)) => {
                let r = frontier.recovery();
                json!({"plan_digest":seal.plan_digest.to_hex(),"object_digest":seal.object_digest.to_hex(),
                    "anchor":anchor_json(p.anchor),"identity_digest":identity.to_hex(),
                    "frontier":{"journal_id":r.journal_id.to_hex(),"head":r.head.to_hex(),"authority_tick_floor":r.authority_tick_floor}})
            }
            _ => Value::Null,
        }
    }
    pub(crate) fn opening_recovery_reference(&self) -> Value {
        json!({"mode":self.mode.as_str(),"anchor":anchor_json(self.anchor),"recovery":self.recovery_json(),
            "original_plan":self.original.as_ref().map(plan_json),"runtime_admitted":false,
            "current_state_proven":false,"original_goal_proven":false,"automatic_redispatch":false})
    }
    fn view(&mut self, work: &mut Work) -> Result<View> {
        let result = self.view_inner(work);
        if result.is_err() {
            self.abandon_preparation();
        }
        result
    }
    fn view_inner(&mut self, work: &mut Work) -> Result<View> {
        self.config.current()?;
        self.arm(work)?;
        if let Some(custody) = &self.custody {
            custody.verify(&work.current()?)?;
        }
        let frontier = if let Some(frontier) = &mut self.frontier {
            Some(frontier.inspect(&work.take(8 * MIB)?)?)
        } else {
            None
        };
        if let Some((w, o)) = &mut self.parts
            && (!w.inventory(&work.take(144 * MIB)?)?.records.is_empty()
                || !o.inventory(&work.take(16 * MIB)?)?.is_empty())
        {
            return Err(denied(
                "unattached native records require exact original recovery",
            ));
        }
        self.config.current()?;
        work.current()?;
        let review = match frontier.as_ref().and_then(|v| v.review.as_ref()) {
            Some(current) => {
                let evidence = self
                    .execution
                    .as_ref()
                    .and_then(Execution::latest_step_evidence)
                    .ok_or_else(|| denied("current frontier review has no native effect review"))?;
                let mut value = review::project(evidence)?;
                if value["step_id"] != json!(current.step.get())
                    || value["family"] != json!(current.family.protocol())
                    || value["preparation_digest"] != json!(current.preparation.digest.to_hex())
                    || value["original_plan_digest"]
                        != json!(
                            self.original
                                .as_ref()
                                .ok_or_else(|| invalid("native review lost its original plan"))?
                                .digest
                                .to_hex()
                        )
                {
                    return Err(denied("current frontier and native effect reviews differ"));
                }
                value["paired_seal"] = json!(current.seal.to_hex());
                value["frontier_head"] = json!(current.head.to_hex());
                value["confirmation_required"] = json!(true);
                value
            }
            None => Value::Null,
        };
        let frontier = frontier.as_ref().map(frontier_json);
        let observation=self.basis.as_ref().map(|b|json!({"seal":b.seal.to_hex(),"source_digest":b.capsule.content_digest.to_hex(),
            "paused":true,"citizens":b.projection.snapshot.graph.entities.iter().filter(|(_,r)|r.kind==dfmcp_world::EntityKind::Unit).map(|(id,_)|json!({"entity_id":id.get().to_string()})).collect::<Vec<_>>() }));
        let value = json!({"ok":true,"mode":self.mode.as_str(),"anchor":anchor_json(self.anchor),
            "runtime_admitted":false,"observation":observation,"original_plan":self.original.as_ref().map(plan_json),
            "frontier":frontier,"review":review,
            "recovery":self.recovery_json(),"labor_control_enabled":self.labor_enabled(),"production_control_enabled":self.orders_enabled(),
            "native_custody":{"workforce_journal_id":self.workforce_journal.to_hex(),"order_journal_id":self.order_journal.to_hex()},
            "current_state_proven":false,"original_goal_proven":false,"clock_advanced":false,
            "automatic_redispatch":false,"custody_initialization_may_be_partial":self.initialization_failed,
            "coverage_omissions":["parent goal completion","produced goods","current labor after its receipt","workshop/material feasibility","game clock control"]});
        if let Some(custody) = &self.custody {
            custody.disarm()?;
        }
        Ok(View { value })
    }
    pub(crate) fn inspect(&mut self, c: &OperationContext) -> Result<View> {
        self.check(c, false)?;
        let result = self.view(&mut Work::new(c)?);
        if result.is_err() {
            self.abandon_preparation();
        }
        result
    }
    pub(crate) fn observe(&mut self, c: &OperationContext) -> Result<View> {
        self.check(c, false)?;
        if self.mode == Mode::Offline {
            return Err(denied(
                "offline combined recovery cannot acquire live observations",
            ));
        }
        if self.original.is_some() {
            return self.inspect(c);
        }
        self.basis = None;
        let mut work = Work::new(c)?;
        c.authorize(Capability::Observe, RiskTier::ReadOnly, &[], None)?;
        let capsule = self.config.workforce.acquire(&work.take(16 * MIB)?)?;
        let cursor = ObservationCursor {
            epoch: self.anchor.cursor.epoch,
            sequence: self
                .anchor
                .cursor
                .sequence
                .checked_add(1)
                .ok_or_else(|| exhausted("combined observation cursor exhausted"))?,
        };
        let projection = project_live_capsule(&capsule, self.anchor.fortress_id, cursor)?;
        if projection.snapshot.tick < self.anchor.tick {
            return Err(denied("combined observation regressed"));
        }
        work.context.anchor = projection.snapshot.anchor();
        self.config
            .workforce
            .acceptance()
            .citizens(&capsule, &projection, &work.take(MIB)?)?;
        self.config
            .orders
            .acceptance()
            .citizens(&capsule, &projection, &work.take(MIB)?)?;
        self.anchor = projection.snapshot.anchor();
        self.basis = Some(Basis {
            seal: observation_seal(&capsule, &projection),
            capsule,
            projection,
        });
        self.view(&mut work)
    }
    pub(crate) fn plan(&mut self, request: Request, c: &OperationContext) -> Result<View> {
        self.check(c, true)?;
        if self.original.is_some() {
            return Err(invalid(
                "retain original combined plan; prepare its next step without recompiling",
            ));
        }
        let basis = self
            .basis
            .as_ref()
            .ok_or_else(|| invalid("observe the canonical roster before combined planning"))?;
        if request.observation()? != basis.seal {
            return Err(denied("combined request names another observation seal"));
        }
        let mut work = Work::new(c)?;
        let policy = self.config.workforce.acceptance().citizens(
            &basis.capsule,
            &basis.projection,
            &work.take(MIB)?,
        )?;
        let evidence = PredicateEvidence::scoped(&basis.projection.snapshot, policy)?;
        let plan = request.compile(&evidence, &work.take(8 * MIB)?)?;
        let expected = host_plan(&plan)?;
        // From this point failures preserve partial publication and forbid another
        // plan in this owner. No native prepare/commit has been reached.
        self.initialization_failed = true;
        let archive = create_private_plan_archive(
            &self.config.path("original.plan"),
            &plan,
            live_production::archive_limits(),
            &work.take(4 * MIB)?,
        )?;
        if archive.seal() != expected {
            return Err(denied("combined archive differs from original seal"));
        }
        let identity = workforce::identity::create_private_capsule(
            &self.config.path("original.citizens"),
            &basis.capsule,
            &work.take(32 * MIB)?,
        )?;
        let identity_digest = identity.seal();
        let recovery = durable::initial_recovery(&plan, &work.take(2 * MIB)?)?;
        self.seal = Some(expected);
        self.identity_digest = Some(identity_digest);
        self.original = Some(plan.clone());
        self.custody = Some(SharedCustody::new(archive, identity, plan.clone()));
        self.frontier = Some(durable::open_private_coordinator(
            &self.config.path("execution.journal"),
            &work.take(16 * MIB)?,
            BuildMode::Control,
            Some(plan),
            recovery,
            true,
        )?);
        self.create_executor(&mut work)?;
        self.initialization_failed = false;
        self.view(&mut Work::new(&work.current()?)?)
    }
    pub(crate) fn prepare_next(
        &mut self,
        expected: Digest32,
        c: &OperationContext,
    ) -> Result<View> {
        self.check(c, true)?;
        if self.original.as_ref().map(|p| p.digest) != Some(expected) {
            return Err(denied("next step must name the exact original plan"));
        }
        self.delegate(c, true, |frontier, execution, c| {
            frontier.prepare_next(execution, c).map(|_| ())
        })
    }
    pub(crate) fn commit(
        &mut self,
        seal: Digest32,
        confirmed: bool,
        c: &OperationContext,
    ) -> Result<View> {
        self.check(c, true)?;
        self.delegate(c, false, |frontier, execution, c| {
            frontier.commit(seal, confirmed, execution, c).map(|_| ())
        })
    }
    pub(crate) fn wait(&mut self, c: &OperationContext) -> Result<View> {
        self.check(c, false)?;
        if self.mode == Mode::Offline {
            return self.inspect(c);
        }
        self.delegate(c, false, |frontier, execution, c| {
            let view = frontier.inspect(c)?;
            if let Some(entry) = view.entries.iter().find(|e| {
                e.phase.uncertain() || (e.phase == Phase::Prepared && view.review.is_none())
            }) {
                frontier.reconcile(entry.step, execution, c)?;
            }
            Ok(())
        })
    }
    pub(crate) fn cancel(&mut self, c: &OperationContext) -> Result<View> {
        self.check(c, true)?;
        self.delegate(c, false, |frontier, execution, c| {
            let step = frontier
                .inspect(c)?
                .entries
                .iter()
                .find(|entry| entry.phase == Phase::Prepared)
                .map(|entry| entry.step)
                .ok_or_else(|| {
                    invalid("no retained prepared step to cancel; reconcile uncertain work")
                })?;
            frontier.cancel_prepared(step, execution, c).map(|_| ())
        })
    }
    fn delegate(
        &mut self,
        c: &OperationContext,
        prime_labor: bool,
        action: impl FnOnce(&mut Coordinator<File>, &mut Execution, &OperationContext) -> Result<()>,
    ) -> Result<View> {
        let result = self.delegate_inner(c, prime_labor, action);
        if result.is_err() {
            self.abandon_preparation();
        }
        result
    }
    fn delegate_inner(
        &mut self,
        c: &OperationContext,
        prime_labor: bool,
        action: impl FnOnce(&mut Coordinator<File>, &mut Execution, &OperationContext) -> Result<()>,
    ) -> Result<View> {
        let mut work = Work::new(c)?;
        let output = work.take(256 * MIB)?;
        self.arm(&mut work)?;
        if let Some(custody) = &self.custody {
            custody.verify(&work.current()?)?;
        }
        let frontier = self
            .frontier
            .as_mut()
            .ok_or_else(|| invalid("no original combined execution"))?;
        let execution = self
            .execution
            .as_mut()
            .ok_or_else(|| invalid("combined executor is unavailable"))?;
        // The labor owner resolves canonical unit IDs before its first native
        // capture. Its current-operation route must therefore be primed through
        // a fresh authorized citizen read, never by reviving a prior generation.
        if prime_labor
            && frontier.inspect(&work.take(8 * MIB)?)?.next_ready == Some(dfmcp_core::StepId::ZERO)
        {
            execution.refresh_original_evidence(&work.take(32 * MIB)?, |evidence, current| {
                evidence.prime(current)
            })?;
        }
        let result = action(frontier, execution, &work.rest(MAX_FRONTIER_BYTES)?);
        if let Some(custody) = &self.custody {
            custody.disarm()?;
        }
        if let Err(cause) = result {
            self.abandon_preparation();
            return Err(cause);
        }
        self.config.current()?;
        let mut output = output;
        output.budget.max_wall_millis = output
            .budget
            .max_wall_millis
            .min(work.current()?.budget.max_wall_millis);
        self.view(&mut Work::new(&output)?)
    }
}
