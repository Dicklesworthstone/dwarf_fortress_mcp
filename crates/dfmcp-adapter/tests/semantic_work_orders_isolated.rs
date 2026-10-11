#![forbid(unsafe_code)]
//! Exercise the ordinary production coordinator through public APIs, strict native
//! codecs, injected acquisition/storage, and real private-file custody.

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::rc::Rc;
use std::time::Duration;

use dfmcp_adapter::control_effect_journal::EffectJournalStorage;
use dfmcp_adapter::live_projection::{LiveWorldProjection, project_live_capsule};
use dfmcp_adapter::live_routing::{LiveRoutingEvidence, resolve_work_order_step};
use dfmcp_adapter::semantic_work_orders::{
    AssociationStore, NativeOrderCreation, SemanticWorkOrderSession, WorkOrderEvidenceOwner,
    open_private_association_store,
};
use dfmcp_adapter::work_order_control::session::WorkOrderSession;
use dfmcp_adapter::work_order_control::{
    CreationState, JournalMode, WorkOrderJournal, WorkOrderQuerySource, WorkOrderSource,
};
use dfmcp_adapter::work_orders::rpc::WorkOrderManifest;
use dfmcp_adapter::work_orders::{
    WorkOrderEffect, WorkOrderObservation, WorkOrderPlan, WorkOrderSpec, WorkOrderState,
};
use dfmcp_adapter::{
    BridgeManifest, CitizenRecord, LiveObservationCapsule, ObservationAssembler, ObservationPage,
};
use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, DfmcpError, Digest32, EntityId, ErrorCode,
    GameTick, IntentId, ObservationCursor, OperationContext, RequestId, Result, RiskTier,
    SessionId, StepId, WorkBudget,
};
use dfmcp_intent::{
    Action, ObligationSpec, PlanStep, PreparedPlan, WorkOrderCondition, derive_step_idempotency_key,
};
use dfmcp_world::{
    EvidenceCoverage, EvidencePolicy, EvidenceSource, FactSource, Predicate, PredicateEvidence,
    WorldSnapshot,
};

fn error(code: ErrorCode, message: &str) -> DfmcpError {
    DfmcpError::new(code, message)
}
fn rejected() -> DfmcpError {
    error(ErrorCode::AdapterRejected, "injected fixture refusal")
}
fn text(bytes: &mut Vec<u8>, value: &str) {
    bytes.extend_from_slice(&(value.len() as u16).to_be_bytes());
    bytes.extend_from_slice(value.as_bytes());
}
fn queue() -> Result<WorkOrderObservation> {
    let raw = include_str!("fixtures/work_order_observation_v1_10.hex").trim();
    let bytes = raw
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|p| {
            let a = char::from(p[0]).to_digit(16).ok_or_else(rejected)?;
            let b = char::from(p[1]).to_digit(16).ok_or_else(rejected)?;
            Ok((a * 16 + b) as u8)
        })
        .collect::<Result<Vec<_>>>()?;
    WorkOrderObservation::decode(&bytes)
}
fn changed_clock(
    original: &WorkOrderObservation,
    generation: u64,
    sequence: u64,
    tick: u64,
) -> Result<WorkOrderObservation> {
    let mut bytes = original.canonical_bytes().to_vec();
    bytes[8..16].copy_from_slice(&generation.to_be_bytes());
    bytes[16..24].copy_from_slice(&sequence.to_be_bytes());
    bytes[24..32].copy_from_slice(&tick.to_be_bytes());
    WorkOrderObservation::decode(&bytes)
}
fn after(p: &WorkOrderPlan) -> Result<WorkOrderObservation> {
    let o = p.observation();
    let mut bytes = o.canonical_bytes().to_vec();
    bytes[16..24].copy_from_slice(&(o.sequence() + 1).to_be_bytes());
    bytes[32..36].copy_from_slice(&(o.next_order_id() + 1).to_be_bytes());
    let at = 43 + o.world_folder().len();
    bytes[at..at + 4].copy_from_slice(&((o.order_ids().len() + 1) as u32).to_be_bytes());
    bytes.extend_from_slice(&o.next_order_id().to_be_bytes());
    WorkOrderObservation::decode(&bytes)
}
fn effect(p: &WorkOrderPlan, state: WorkOrderState) -> Result<WorkOrderEffect> {
    let o = p.observation();
    let mut bytes = b"DFMWOE10".to_vec();
    for n in [o.generation(), o.sequence(), o.tick()] {
        bytes.extend_from_slice(&n.to_be_bytes());
    }
    bytes.extend_from_slice(&o.next_order_id().to_be_bytes());
    bytes.push(p.spec().recipe() as u8);
    bytes.extend_from_slice(&p.spec().amount().to_be_bytes());
    bytes.extend_from_slice(o.witness().as_bytes());
    bytes.extend_from_slice(p.digest().as_bytes());
    bytes.extend_from_slice(p.prepare_token());
    let created = state == WorkOrderState::Created;
    bytes.extend_from_slice(&[state as u8, u8::from(created)]);
    bytes.extend_from_slice(&(if created { o.tick() } else { 0 }).to_be_bytes());
    let (after, config) = if created {
        let mut config = b"DFMWOC10".to_vec();
        config.extend_from_slice(&o.next_order_id().to_be_bytes());
        config.push(p.spec().recipe() as u8);
        config.extend_from_slice(&p.spec().amount().to_be_bytes());
        (after(p)?.witness(), Digest32::of_bytes(&config))
    } else {
        (Digest32::ZERO, Digest32::ZERO)
    };
    bytes.extend_from_slice(after.as_bytes());
    bytes.extend_from_slice(config.as_bytes());
    let receipt = if state.terminal() {
        let mut proof = b"dfmcp-work-order-receipt/1\0".to_vec();
        proof.extend_from_slice(&o.generation().to_be_bytes());
        text(&mut proof, p.key());
        proof.extend_from_slice(&bytes[73..195]);
        Digest32::of_bytes(&proof)
    } else {
        Digest32::ZERO
    };
    bytes.extend_from_slice(receipt.as_bytes());
    text(&mut bytes, p.key());
    WorkOrderEffect::decode(&bytes, p)
}

type Trace = Rc<RefCell<Vec<String>>>;
#[derive(Clone)]
struct Memory {
    data: Rc<RefCell<MemoryData>>,
    offset: u64,
}
struct MemoryData {
    bytes: Vec<u8>,
    name: &'static str,
    trace: Trace,
    syncs: usize,
    fail_sync: Option<usize>,
    deny_sync: Option<(usize, Rc<Cell<bool>>)>,
    valid: bool,
}
impl Memory {
    fn new(name: &'static str, trace: Trace) -> Self {
        Self {
            data: Rc::new(RefCell::new(MemoryData {
                bytes: Vec::new(),
                name,
                trace,
                syncs: 0,
                fail_sync: None,
                deny_sync: None,
                valid: true,
            })),
            offset: 0,
        }
    }
    fn stored_bytes(&self) -> Vec<u8> {
        self.data.borrow().bytes.clone()
    }
    fn with_stored_bytes(&self, bytes: Vec<u8>) -> Self {
        let out = Self::new(
            self.data.borrow().name,
            Rc::clone(&self.data.borrow().trace),
        );
        out.data.borrow_mut().bytes = bytes;
        out
    }
}
impl Read for Memory {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let d = self.data.borrow();
        let start = usize::try_from(self.offset).map_err(|_| io::Error::other("offset"))?;
        let src = d.bytes.get(start..).unwrap_or_default();
        let len = src.len().min(out.len());
        out[..len].copy_from_slice(&src[..len]);
        self.offset += len as u64;
        Ok(len)
    }
}
impl Write for Memory {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let mut d = self.data.borrow_mut();
        let start = usize::try_from(self.offset).map_err(|_| io::Error::other("offset"))?;
        let end = start
            .checked_add(bytes.len())
            .ok_or_else(|| io::Error::other("length"))?;
        let extent = end.max(d.bytes.len());
        d.bytes.resize(extent, 0);
        d.bytes[start..end].copy_from_slice(bytes);
        d.trace.borrow_mut().push(format!("{}:write", d.name));
        self.offset = end as u64;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
impl Seek for Memory {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let next = match pos {
            SeekFrom::Start(n) => i128::from(n),
            SeekFrom::Current(n) => i128::from(self.offset) + i128::from(n),
            SeekFrom::End(n) => self.data.borrow().bytes.len() as i128 + i128::from(n),
        };
        self.offset = u64::try_from(next).map_err(|_| io::Error::other("negative seek"))?;
        Ok(self.offset)
    }
}
impl EffectJournalStorage for Memory {
    fn sync(&mut self) -> io::Result<()> {
        let mut d = self.data.borrow_mut();
        d.syncs += 1;
        d.trace
            .borrow_mut()
            .push(format!("{}:sync:{}", d.name, d.syncs));
        if let Some((at, flag)) = &d.deny_sync
            && *at == d.syncs
        {
            flag.set(true);
        }
        if d.fail_sync == Some(d.syncs) {
            return Err(io::Error::other("injected sync failure"));
        }
        Ok(())
    }
    fn truncate(&mut self, _: u64) -> io::Result<()> {
        Err(io::Error::other("repair forbidden"))
    }
    fn validate_identity(&self) -> io::Result<()> {
        if self.data.borrow().valid {
            Ok(())
        } else {
            Err(io::Error::other("lost custody"))
        }
    }
}

struct NativeState {
    queue: WorkOrderObservation,
    retained: BTreeMap<String, WorkOrderEffect>,
    reads: usize,
    prepares: usize,
    commits: usize,
    queries: usize,
    lose_commit: bool,
    commit_state: WorkOrderState,
    deny_read: bool,
    trace: Trace,
}
struct Native {
    state: Rc<RefCell<NativeState>>,
    manifest: WorkOrderManifest,
    fenced: bool,
}
impl Native {
    fn new(state: &Rc<RefCell<NativeState>>) -> Self {
        Self {
            state: Rc::clone(state),
            manifest: WorkOrderManifest {
                generation: state.borrow().queue.generation(),
                df_version: "0.51.11".into(),
                dfhack_version: "0.51.11-r1".into(),
            },
            fenced: false,
        }
    }
    fn valid(&self) -> Result<()> {
        if self.fenced { Err(rejected()) } else { Ok(()) }
    }
}
impl WorkOrderQuerySource for Native {
    fn manifest(&self) -> &WorkOrderManifest {
        &self.manifest
    }
    fn fence(&mut self) {
        self.fenced = true;
    }
    fn read_orders(&mut self, timeout: Duration) -> Result<WorkOrderObservation> {
        self.valid()?;
        assert!(timeout <= Duration::from_secs(60));
        let mut s = self.state.borrow_mut();
        s.reads += 1;
        s.trace.borrow_mut().push("native:read".into());
        if s.deny_read {
            return Err(rejected());
        }
        Ok(s.queue.clone())
    }
    fn query(&mut self, plan: &WorkOrderPlan, _: Duration) -> Result<Option<WorkOrderEffect>> {
        self.valid()?;
        let mut s = self.state.borrow_mut();
        s.queries += 1;
        s.trace.borrow_mut().push("native:query".into());
        Ok(s.retained.get(plan.key()).cloned())
    }
}
impl WorkOrderSource for Native {
    fn prepare(&mut self, plan: &WorkOrderPlan, _: Duration) -> Result<WorkOrderEffect> {
        self.valid()?;
        let mut s = self.state.borrow_mut();
        s.prepares += 1;
        s.trace.borrow_mut().push("native:prepare".into());
        if &s.queue != plan.observation() {
            return Err(rejected());
        }
        let value = effect(plan, WorkOrderState::Prepared)?;
        s.retained.insert(plan.key().into(), value.clone());
        Ok(value)
    }
    fn commit(
        &mut self,
        plan: &WorkOrderPlan,
        prepared: &WorkOrderEffect,
        _: Duration,
    ) -> Result<WorkOrderEffect> {
        self.valid()?;
        assert_eq!(prepared.state(), WorkOrderState::Prepared);
        let mut s = self.state.borrow_mut();
        s.commits += 1;
        s.trace.borrow_mut().push("native:commit".into());
        if &s.queue != plan.observation() {
            return Err(rejected());
        }
        let value = effect(plan, s.commit_state)?;
        if s.commit_state == WorkOrderState::Created {
            s.queue = after(plan)?;
        }
        s.retained.insert(plan.key().into(), value.clone());
        if s.lose_commit {
            Err(error(ErrorCode::AdapterUnavailable, "lost reply"))
        } else {
            Ok(value)
        }
    }
}

fn capsule(
    o: &WorkOrderObservation,
) -> Result<(LiveObservationCapsule, LiveWorldProjection, ObservationPage)> {
    let bridge = BridgeManifest {
        bridge_version: "0.1.0".into(),
        df_version: "0.51.11".into(),
        dfhack_version: "0.51.11-r1".into(),
        bridge_generation: o.generation(),
        world_loaded: true,
        fortress_mode: true,
        supported_methods: BTreeSet::from(["Handshake".into(), "ReadObservation".into()]),
    };
    let page = ObservationPage {
        bridge_generation: o.generation(),
        world_loaded: true,
        fortress_mode: true,
        paused: true,
        current_year: (o.tick() / 403_200) as u32,
        current_year_tick: (o.tick() % 403_200) as u32,
        world_name: "Work-order fixture".into(),
        world_folder: o.world_folder().into(),
        site_id: o.site_id() as i32,
        citizen_count_total: 1,
        citizen_offset: 0,
        complete: true,
        citizens: vec![CitizenRecord {
            unit_id: 42,
            name: "Urist".into(),
            race: "dwarf".into(),
            profession: 4,
            x: 10,
            y: 11,
            z: 2,
            alive: true,
            sane: true,
            active: true,
            visible: true,
            citizen: true,
            resident: false,
            baby: false,
            child: false,
            adult: true,
        }],
    };
    let mut assembler = ObservationAssembler::new(bridge);
    assembler.push_page(page.clone())?;
    let capsule = assembler.finalize()?;
    let projection = project_live_capsule(
        &capsule,
        o.fortress_id(),
        ObservationCursor {
            epoch: 4,
            sequence: 20,
        },
    )?;
    Ok((capsule, projection, page))
}
fn policy(snapshot: &WorldSnapshot) -> EvidencePolicy {
    let mut p = EvidencePolicy::at(snapshot.anchor());
    p.paused = true;
    p.all_entities = EvidenceCoverage::Observed;
    for entity in snapshot.graph.entities.values() {
        for f in entity.fields.values() {
            if let FactSource::DfhackField(field) = &f.source {
                p.sources.insert(EvidenceSource::Observed {
                    field: field.clone(),
                    source_digest: f.source_digest,
                });
            }
        }
    }
    p
}
struct Observer {
    capsule: LiveObservationCapsule,
    projection: LiveWorldProjection,
    denied: Rc<Cell<bool>>,
    refreshed: bool,
    refreshes: usize,
    trace: Trace,
}
impl WorkOrderEvidenceOwner for Observer {
    fn refresh_after_observation(
        &mut self,
        o: &WorkOrderObservation,
        c: &OperationContext,
    ) -> Result<()> {
        self.refreshed = false;
        self.refreshes += 1;
        self.trace.borrow_mut().push("canonical:refresh".into());
        c.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
        assert!(c.budget.max_bytes <= 16 * 1024 * 1024);
        assert_eq!(c.anchor, self.projection.snapshot.anchor());
        if self.denied.get() || o.tick() != c.anchor.tick.get() {
            return Err(rejected());
        }
        self.refreshed = true;
        Ok(())
    }
    fn routing_evidence(&self) -> Result<LiveRoutingEvidence<'_>> {
        if !self.refreshed || self.denied.get() {
            return Err(rejected());
        }
        LiveRoutingEvidence::citizens_v1(
            PredicateEvidence::scoped(
                &self.projection.snapshot,
                policy(&self.projection.snapshot),
            )?,
            &self.projection,
            &self.capsule,
        )
    }
}
fn plan(snapshot: &WorldSnapshot, job: &str, intent: u128) -> PreparedPlan {
    let action = Action::CreateWorkOrder {
        name: "original furniture quota".into(),
        job_token: job.into(),
        amount: 5,
        conditions: Vec::new(),
    }
    .normalized();
    let id = IntentId::new(intent);
    let step = StepId::ZERO;
    let goal = Predicate::EntityExists(EntityId::new(9999));
    PreparedPlan::builder(
        id,
        snapshot.anchor(),
        "Create furniture without losing its original goal",
        goal.clone(),
    )
    .steps(vec![PlanStep {
        id: step,
        risk: action.risk(),
        required_capability: action.capability(),
        idempotency_key: derive_step_idempotency_key(id, snapshot.anchor(), step, &action),
        action,
        preconditions: vec![Predicate::Paused(true)],
        postconditions: vec![goal.clone()],
        compensation: None,
        depends_on: Vec::new(),
        obligation: Some(ObligationSpec {
            terminal: goal,
            failure: None,
            deadline_tick: GameTick(snapshot.tick.get() + 100),
            poll_interval_ticks: 10,
            stable_for_observations: 2,
        }),
    }])
    .max_risk(RiskTier::Reversible)
    .required_capabilities(BTreeSet::from([Capability::ConfigureProduction]))
    .expires_at_tick(GameTick(snapshot.tick.get() + 80))
    .build()
}
fn reseal(mut p: PreparedPlan) -> PreparedPlan {
    p.digest = p.compute_digest();
    p.id = p.expected_id();
    p
}
type Session = SemanticWorkOrderSession<Memory, Memory, Native>;
struct Fixture {
    context: OperationContext,
    original: PreparedPlan,
    native: Rc<RefCell<NativeState>>,
    observer: Observer,
    journal: Memory,
    associations: Memory,
    trace: Trace,
}
impl Fixture {
    fn new() -> Result<Self> {
        let queue = queue()?;
        let (capsule, projection, _) = capsule(&queue)?;
        let original = plan(&projection.snapshot, "CONSTRUCT_BED", 1);
        original.validate_structure()?;
        let trace = Trace::default();
        let context = OperationContext {
            session_id: SessionId::new(1),
            request_id: RequestId::new(1),
            anchor: projection.snapshot.anchor(),
            budget: WorkBudget {
                max_bytes: 128 * 1024 * 1024,
                max_entities: 8192,
                max_wall_millis: 60_000,
                max_output_tokens: 65_536,
                max_game_ticks: 0,
                max_actions: 1,
            },
            grants: [
                Capability::Query,
                Capability::Plan,
                Capability::ConfigureProduction,
            ]
            .into_iter()
            .map(|capability| CapabilityGrant {
                capability,
                scope: CapabilityScope {
                    fortress_id: Some(queue.fortress_id()),
                    ..CapabilityScope::default()
                },
                max_risk: RiskTier::Guarded,
                expires_at_tick: None,
                remaining_uses: None,
            })
            .collect(),
            cancellation_requested: false,
        };
        let native = Rc::new(RefCell::new(NativeState {
            queue,
            retained: BTreeMap::new(),
            reads: 0,
            prepares: 0,
            commits: 0,
            queries: 0,
            lose_commit: false,
            commit_state: WorkOrderState::Created,
            deny_read: false,
            trace: Rc::clone(&trace),
        }));
        let observer = Observer {
            capsule,
            projection,
            denied: Rc::new(Cell::new(false)),
            refreshed: false,
            refreshes: 0,
            trace: Rc::clone(&trace),
        };
        Ok(Self {
            context,
            original,
            native,
            observer,
            journal: Memory::new("native-journal", Rc::clone(&trace)),
            associations: Memory::new("semantic-store", Rc::clone(&trace)),
            trace,
        })
    }
    fn session(&self, initialize: bool, mode: JournalMode, read_only: bool) -> Result<Session> {
        let j = WorkOrderJournal::open(self.journal.clone(), &self.context, mode, initialize)?;
        let id = j.summary(&self.context)?.journal_id;
        let a = AssociationStore::open(
            self.associations.clone(),
            id,
            initialize,
            read_only,
            &self.context,
        )?;
        let source = if mode == JournalMode::Offline {
            None
        } else {
            Some(Native::new(&self.native))
        };
        SemanticWorkOrderSession::new(
            WorkOrderSession::new(j, source, &self.context)?,
            a,
            &self.context,
        )
    }
    fn advance_source(
        &mut self,
        observation: WorkOrderObservation,
        cursor: ObservationCursor,
        intent: u128,
    ) -> Result<()> {
        let (capsule, _, _) = capsule(&observation)?;
        let projection = project_live_capsule(&capsule, observation.fortress_id(), cursor)?;
        self.original = plan(&projection.snapshot, "CONSTRUCT_DOOR", intent);
        self.context.anchor = projection.snapshot.anchor();
        self.observer.capsule = capsule;
        self.observer.projection = projection;
        self.observer.refreshed = false;
        self.native.borrow_mut().queue = observation;
        Ok(())
    }
}

#[test]
fn exact_original_plan_reaches_native_insertion_but_never_becomes_goal_proof() -> Result<()> {
    let mut f = Fixture::new()?;
    let mut s = f.session(true, JournalMode::Control, false)?;
    let r = s.observe(f.original.clone(), &mut f.observer, &f.context)?;
    assert_eq!(r.original_plan(), &f.original);
    assert_eq!(r.native_plan().key(), f.original.steps[0].idempotency_key);
    assert_ne!(r.seal(), r.native_plan().digest());
    assert_eq!(f.native.borrow().prepares, 0);
    let prepared = s.prepare(r.seal(), &mut f.observer, &f.context)?;
    assert_eq!(prepared.native_record().state(), CreationState::Prepared);
    assert_eq!(prepared.creation_result(), &NativeOrderCreation::Unverified);
    assert!(
        s.commit(r.seal(), false, &mut f.observer, &f.context)
            .is_err()
    );
    let created = s.commit(r.seal(), true, &mut f.observer, &f.context)?;
    assert!(matches!(
        created.creation_result(),
        NativeOrderCreation::Inserted { .. }
    ));
    assert!(!created.original_goal_proven());
    assert_eq!(
        created.pending_obligation(),
        f.original.steps[0].obligation.as_ref()
    );
    assert_eq!(created.review().original_plan(), &f.original);
    let count = f.observer.refreshes;
    let replay = s.commit(r.seal(), true, &mut f.observer, &f.context)?;
    assert_eq!(replay.native_record(), created.native_record());
    assert_eq!(f.native.borrow().prepares, 1);
    assert_eq!(f.native.borrow().commits, 1);
    assert_eq!(f.observer.refreshes, count);
    let trace = f.trace.borrow();
    let at = |needle: &str| trace.iter().position(|v| v == needle).ok_or_else(rejected);
    assert!(at("semantic-store:sync:2")? < at("native:prepare")?);
    assert!(at("native-journal:sync:3")? < at("native:commit")?);
    Ok(())
}

#[test]
fn all_closed_recipes_use_the_original_key_and_finite_quantity() -> Result<()> {
    for (job, code) in [
        ("CONSTRUCT_BED", 1),
        ("CONSTRUCT_DOOR", 2),
        ("CONSTRUCT_TABLE", 3),
        ("CONSTRUCT_THRONE", 4),
    ] {
        let mut f = Fixture::new()?;
        f.original = plan(&f.observer.projection.snapshot, job, code);
        let mut s = f.session(true, JournalMode::Control, false)?;
        let r = s.observe(f.original.clone(), &mut f.observer, &f.context)?;
        assert_eq!(r.native_plan().spec().recipe() as u128, code);
        assert_eq!(r.native_plan().spec().amount(), 5);
        s.prepare(r.seal(), &mut f.observer, &f.context)?;
        assert!(matches!(
            s.commit(r.seal(), true, &mut f.observer, &f.context)?
                .creation_result(),
            NativeOrderCreation::Inserted { .. }
        ));
    }
    Ok(())
}

#[test]
fn independent_evidence_loss_after_dispatch_sync_never_calls_insertion() -> Result<()> {
    let mut f = Fixture::new()?;
    let mut s = f.session(true, JournalMode::Control, false)?;
    let r = s.observe(f.original.clone(), &mut f.observer, &f.context)?;
    s.prepare(r.seal(), &mut f.observer, &f.context)?;
    f.journal.data.borrow_mut().deny_sync = Some((3, Rc::clone(&f.observer.denied)));
    assert_eq!(
        s.commit(r.seal(), true, &mut f.observer, &f.context)
            .err()
            .ok_or_else(rejected)?
            .code,
        ErrorCode::EffectIndeterminate
    );
    assert_eq!(f.native.borrow().commits, 0);
    assert_eq!(
        s.inspect(r.seal(), &f.context)?.native_record().state(),
        CreationState::DispatchStarted
    );
    f.observer.denied.set(false);
    assert!(
        s.commit(r.seal(), true, &mut f.observer, &f.context)
            .is_err()
    );
    assert_eq!(f.native.borrow().commits, 0);
    Ok(())
}

#[test]
fn lost_created_reply_reconciles_after_restart_without_another_insertion() -> Result<()> {
    let mut f = Fixture::new()?;
    f.native.borrow_mut().lose_commit = true;
    let mut s = f.session(true, JournalMode::Control, false)?;
    let r = s.observe(f.original.clone(), &mut f.observer, &f.context)?;
    s.prepare(r.seal(), &mut f.observer, &f.context)?;
    assert_eq!(
        s.commit(r.seal(), true, &mut f.observer, &f.context)?
            .native_record()
            .state(),
        CreationState::Indeterminate
    );
    drop(s);
    f.context.session_id = SessionId::new(2);
    f.context
        .grants
        .retain(|g| g.capability == Capability::Query);
    f.context.anchor.tick = GameTick(f.context.anchor.tick.get() + 50);
    let mut recovered = f.session(false, JournalMode::Reconcile, true)?;
    let attached = recovered.reattach(f.original.clone(), &f.context)?;
    assert_eq!(attached.seal(), r.seal());
    let result = recovered.reconcile(attached.seal(), &f.context)?;
    assert!(matches!(
        result.creation_result(),
        NativeOrderCreation::Inserted { .. }
    ));
    assert!(!result.original_goal_proven());
    assert_eq!(f.native.borrow().commits, 1);
    assert_eq!(f.native.borrow().queries, 1);
    assert!(
        recovered
            .commit(r.seal(), true, &mut f.observer, &f.context)
            .is_err()
    );
    Ok(())
}

#[test]
fn original_preparation_can_reacquire_unchanged_queue_without_lifetime_renewal() -> Result<()> {
    let mut f = Fixture::new()?;
    let mut s = f.session(true, JournalMode::Control, false)?;
    let r = s.observe(f.original.clone(), &mut f.observer, &f.context)?;
    let prepared = s.prepare(r.seal(), &mut f.observer, &f.context)?;
    drop(s);
    let mut recovered = f.session(false, JournalMode::Control, false)?;
    let attached = recovered.reattach(f.original.clone(), &f.context)?;
    assert_eq!(attached.seal(), r.seal());
    assert!(
        recovered
            .commit(r.seal(), true, &mut f.observer, &f.context)
            .is_err()
    );
    let refreshed = recovered.refresh_prepared(r.seal(), &mut f.observer, &f.context)?;
    assert_eq!(refreshed.seal(), r.seal());
    assert_eq!(
        recovered
            .prepare(r.seal(), &mut f.observer, &f.context)?
            .native_record(),
        prepared.native_record()
    );
    recovered.commit(r.seal(), true, &mut f.observer, &f.context)?;
    assert_eq!(f.native.borrow().prepares, 1);
    assert_eq!(f.native.borrow().commits, 1);
    Ok(())
}

#[test]
fn changed_original_goal_cannot_attach_to_the_same_native_creation_key() -> Result<()> {
    let mut f = Fixture::new()?;
    let mut s = f.session(true, JournalMode::Control, false)?;
    let r = s.observe(f.original.clone(), &mut f.observer, &f.context)?;
    s.prepare(r.seal(), &mut f.observer, &f.context)?;
    let mut changed = f.original.clone();
    changed.terminal_condition = Predicate::EntityExists(EntityId::new(8888));
    let changed = reseal(changed);
    assert_eq!(
        changed.steps[0].idempotency_key,
        f.original.steps[0].idempotency_key
    );
    assert!(s.reattach(changed, &f.context).is_err());
    assert!(
        s.commit(r.seal(), true, &mut f.observer, &f.context)
            .is_err()
    );
    assert_eq!(f.native.borrow().commits, 0);
    Ok(())
}

#[test]
fn unassociated_native_keys_cannot_be_adopted_by_an_empty_semantic_store() -> Result<()> {
    let f = Fixture::new()?;
    let j = WorkOrderJournal::open(f.journal.clone(), &f.context, JournalMode::Control, true)?;
    let id = j.summary(&f.context)?.journal_id;
    let mut native = WorkOrderSession::new(j, Some(Native::new(&f.native)), &f.context)?;
    let o = native.observe(&f.context)?;
    let mut native_context = f.context.clone();
    native_context.anchor.state_hash = o.witness();
    native.plan(
        &f.original.steps[0].idempotency_key,
        WorkOrderSpec::new(dfmcp_adapter::work_orders::WorkOrderRecipe::WoodenBed, 5)?,
        o.witness(),
        &native_context,
    )?;
    let associations = AssociationStore::open(f.associations.clone(), id, true, false, &f.context)?;
    assert!(SemanticWorkOrderSession::new(native, associations, &f.context).is_err());
    assert_eq!(f.native.borrow().commits, 0);
    Ok(())
}

#[test]
fn unknown_native_creation_stays_unknown_and_blocks_other_original_pursuits() -> Result<()> {
    let mut f = Fixture::new()?;
    f.native.borrow_mut().commit_state = WorkOrderState::Unknown;
    let mut s = f.session(true, JournalMode::Control, false)?;
    let r = s.observe(f.original.clone(), &mut f.observer, &f.context)?;
    s.prepare(r.seal(), &mut f.observer, &f.context)?;
    let unknown = s.commit(r.seal(), true, &mut f.observer, &f.context)?;
    assert_eq!(unknown.creation_result(), &NativeOrderCreation::Unverified);
    assert!(!unknown.original_goal_proven());
    let next = plan(&f.observer.projection.snapshot, "CONSTRUCT_DOOR", 2);
    let next = s.observe(next, &mut f.observer, &f.context)?;
    let retained = f.associations.stored_bytes();
    assert!(s.prepare(next.seal(), &mut f.observer, &f.context).is_err());
    assert_eq!(f.associations.stored_bytes(), retained);
    assert_eq!(f.native.borrow().prepares, 1);
    assert_eq!(f.native.borrow().commits, 1);
    drop(s);
    let mut recovered = f.session(false, JournalMode::Reconcile, true)?;
    recovered.reattach(f.original.clone(), &f.context)?;
    assert_eq!(
        recovered.reconcile(r.seal(), &f.context)?.creation_result(),
        &NativeOrderCreation::Unverified
    );
    assert_eq!(f.native.borrow().commits, 1);
    Ok(())
}

#[test]
fn source_and_condition_refusals_happen_before_native_preparation() -> Result<()> {
    for variant in 0..6 {
        let mut f = Fixture::new()?;
        let mut p = f.original.clone();
        match variant {
            0 => p.steps[0].preconditions = vec![Predicate::EntityExists(EntityId::new(43))],
            1 => p.requires_checkpoint = true,
            2 => p.steps[0].compensation = Some(Action::Pause { paused: true }),
            3 => {
                p = plan(&f.observer.projection.snapshot, "BREW_DRINK", 1);
            }
            4 => {
                f.observer.denied.set(true);
            }
            _ => {
                p.steps[0].preconditions = vec![Predicate::Paused(false)];
            }
        }
        let mut s = f.session(true, JournalMode::Control, false)?;
        assert!(s.observe(reseal(p), &mut f.observer, &f.context).is_err());
        assert_eq!(f.native.borrow().prepares, 0);
        assert_eq!(f.native.borrow().commits, 0);
    }
    Ok(())
}

#[test]
fn source_resolution_requires_independent_pause_scope_and_exact_canonical_anchor() -> Result<()> {
    let f = Fixture::new()?;
    let no_grant = LiveRoutingEvidence::citizens_v1(
        PredicateEvidence::untrusted(&f.observer.projection.snapshot)?,
        &f.observer.projection,
        &f.observer.capsule,
    )?;
    assert!(resolve_work_order_step(&f.original, StepId::ZERO, &no_grant, &queue()?).is_err());
    let mut wrong = f.original.clone();
    wrong.anchor.cursor.sequence += 1;
    wrong.steps[0].idempotency_key = derive_step_idempotency_key(
        wrong.intent_id,
        wrong.anchor,
        StepId::ZERO,
        &wrong.steps[0].action,
    );
    let evidence = LiveRoutingEvidence::citizens_v1(
        PredicateEvidence::scoped(
            &f.observer.projection.snapshot,
            policy(&f.observer.projection.snapshot),
        )?,
        &f.observer.projection,
        &f.observer.capsule,
    )?;
    assert!(resolve_work_order_step(&reseal(wrong), StepId::ZERO, &evidence, &queue()?).is_err());
    Ok(())
}

#[test]
fn authority_and_budget_refuse_before_any_new_persistence_or_native_contact() -> Result<()> {
    let mut f = Fixture::new()?;
    let mut s = f.session(true, JournalMode::Control, false)?;
    let before = f.trace.borrow().len();
    let mut narrow = f.context.clone();
    narrow.budget.max_bytes = 4 * 1024 * 1024;
    assert_eq!(
        s.observe(f.original.clone(), &mut f.observer, &narrow)
            .err()
            .ok_or_else(rejected)?
            .code,
        ErrorCode::BudgetExceeded
    );
    assert_eq!(f.trace.borrow().len(), before);
    for capability in [Capability::Plan, Capability::ConfigureProduction] {
        let mut denied = f.context.clone();
        denied.grants.retain(|g| g.capability != capability);
        assert!(
            s.observe(f.original.clone(), &mut f.observer, &denied)
                .is_err()
        );
        assert_eq!(f.native.borrow().reads, 0);
    }
    let r = s.observe(f.original.clone(), &mut f.observer, &f.context)?;
    let before = f.trace.borrow().len();
    assert!(s.prepare(r.seal(), &mut f.observer, &narrow).is_err());
    assert_eq!(f.trace.borrow().len(), before);
    Ok(())
}

#[test]
fn association_and_dispatch_sync_failures_cannot_restore_dispatch_permission() -> Result<()> {
    for dispatch in [false, true] {
        let mut f = Fixture::new()?;
        let mut s = f.session(true, JournalMode::Control, false)?;
        let r = s.observe(f.original.clone(), &mut f.observer, &f.context)?;
        if dispatch {
            s.prepare(r.seal(), &mut f.observer, &f.context)?;
            f.journal.data.borrow_mut().fail_sync = Some(3);
            assert!(
                s.commit(r.seal(), true, &mut f.observer, &f.context)
                    .is_err()
            );
            assert_eq!(f.native.borrow().commits, 0);
        } else {
            f.associations.data.borrow_mut().fail_sync = Some(2);
            assert!(s.prepare(r.seal(), &mut f.observer, &f.context).is_err());
            assert_eq!(f.native.borrow().prepares, 0);
        }
        drop(s);
        f.journal.data.borrow_mut().fail_sync = None;
        f.associations.data.borrow_mut().fail_sync = None;
        let mut recovered = f.session(false, JournalMode::Control, false)?;
        if dispatch {
            recovered.reattach(f.original.clone(), &f.context)?;
            assert!(
                recovered
                    .commit(r.seal(), true, &mut f.observer, &f.context)
                    .is_err()
            );
            assert_eq!(f.native.borrow().commits, 0);
        } else {
            assert!(recovered.reattach(f.original.clone(), &f.context).is_err());
            let fresh = recovered.observe(f.original.clone(), &mut f.observer, &f.context)?;
            assert_eq!(fresh.seal(), r.seal());
            recovered.prepare(fresh.seal(), &mut f.observer, &f.context)?;
            assert_eq!(f.native.borrow().prepares, 1);
        }
    }
    Ok(())
}

#[test]
fn cancellation_after_expiry_retires_only_undispatched_native_preparation() -> Result<()> {
    let mut f = Fixture::new()?;
    let mut s = f.session(true, JournalMode::Control, false)?;
    let r = s.observe(f.original.clone(), &mut f.observer, &f.context)?;
    s.prepare(r.seal(), &mut f.observer, &f.context)?;
    let mut later = f.context.clone();
    later.anchor.tick = GameTick(f.original.expires_at_tick.get() + 1);
    assert!(s.commit(r.seal(), true, &mut f.observer, &later).is_err());
    let cancelled = s.cancel(r.seal(), &later)?;
    assert_eq!(
        cancelled.native_record().state(),
        CreationState::CancelledBeforeDispatch
    );
    assert_eq!(
        cancelled.creation_result(),
        &NativeOrderCreation::Unverified
    );
    assert_eq!(f.native.borrow().commits, 0);
    assert_eq!(
        s.cancel(r.seal(), &later)?.native_record(),
        cancelled.native_record()
    );
    Ok(())
}

#[test]
fn changed_association_bytes_and_torn_frames_refuse_without_repair() -> Result<()> {
    let mut f = Fixture::new()?;
    let mut s = f.session(true, JournalMode::Control, false)?;
    let r = s.observe(f.original.clone(), &mut f.observer, &f.context)?;
    s.prepare(r.seal(), &mut f.observer, &f.context)?;
    let pristine = f.associations.stored_bytes();
    f.associations.data.borrow_mut().bytes[80] ^= 1;
    assert!(
        s.commit(r.seal(), true, &mut f.observer, &f.context)
            .is_err()
    );
    assert_eq!(f.native.borrow().commits, 0);
    drop(s);
    let native =
        WorkOrderJournal::open(f.journal.clone(), &f.context, JournalMode::Reconcile, false)?;
    let id = native.summary(&f.context)?.journal_id;
    for end in [0, 79, 80, pristine.len() - 1] {
        let bytes = pristine[..end].to_vec();
        let storage = f.associations.with_stored_bytes(bytes.clone());
        let result = AssociationStore::open(storage.clone(), id, false, true, &f.context);
        if end == 80 {
            // Complete empty header cannot itself adopt the already retained key.
            let store = result?;
            let j = WorkOrderJournal::open(
                f.journal.clone(),
                &f.context,
                JournalMode::Reconcile,
                false,
            )?;
            let n = WorkOrderSession::new(j, Some(Native::new(&f.native)), &f.context)?;
            assert!(SemanticWorkOrderSession::new(n, store, &f.context).is_err());
        } else {
            assert!(result.is_err());
        }
        assert_eq!(storage.stored_bytes(), bytes);
    }
    Ok(())
}

#[test]
fn real_private_store_reopens_exact_bytes_and_rejects_workforce_family_aliasing() -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new()?;
    let base = std::env::temp_dir().join(format!("dfmcp-semantic-orders-{}", std::process::id()));
    std::fs::create_dir(&base).map_err(|_| rejected())?;
    std::fs::set_permissions(&base, std::fs::Permissions::from_mode(0o700))
        .map_err(|_| rejected())?;
    let path = base.join("history.bin");
    let id = Digest32::of_bytes(b"native-work-order-journal");
    let store = open_private_association_store(&path, id, true, false, &f.context)?;
    assert!(open_private_association_store(&path, id, false, true, &f.context).is_err());
    drop(store);
    let store = open_private_association_store(&path, id, false, true, &f.context)?;
    assert!(store.is_read_only());
    assert_eq!(store.native_journal(), id);
    drop(store);
    let stored = std::fs::read(&path).map_err(|_| rejected())?;
    assert_eq!(&stored[..8], b"DFMSOJ01");
    assert!(
        dfmcp_adapter::semantic_workforce::store::AssociationStore::open(
            f.associations.with_stored_bytes(stored),
            id,
            false,
            true,
            &f.context
        )
        .is_err()
    );
    std::fs::remove_file(path).map_err(|_| rejected())?;
    std::fs::remove_dir(base).map_err(|_| rejected())?;
    Ok(())
}

#[test]
fn every_original_condition_is_retained_and_refused_instead_of_dropped() -> Result<()> {
    for condition in [
        WorkOrderCondition::ItemCountBelow {
            item_token: "BED".into(),
            threshold: 8,
        },
        WorkOrderCondition::MaterialAvailable {
            material_token: "WOOD".into(),
            minimum: 1,
        },
        WorkOrderCondition::CompletedOrder {
            order_name: "preceding furniture".into(),
        },
    ] {
        let mut f = Fixture::new()?;
        let mut p = f.original.clone();
        if let Action::CreateWorkOrder { conditions, .. } = &mut p.steps[0].action {
            conditions.push(condition);
        }
        p.steps[0].action = p.steps[0].action.normalized();
        p.steps[0].idempotency_key =
            derive_step_idempotency_key(p.intent_id, p.anchor, StepId::ZERO, &p.steps[0].action);
        let p = reseal(p);
        p.validate_structure()?;
        let mut s = f.session(true, JournalMode::Control, false)?;
        let before = f.trace.borrow().len();
        assert!(s.observe(p, &mut f.observer, &f.context).is_err());
        assert_eq!(f.trace.borrow().len(), before);
    }
    Ok(())
}

#[test]
fn concrete_complete_citizen_acquisition_owner_drives_native_prepare_and_commit() -> Result<()> {
    use dfmcp_adapter::semantic_workforce::evidence_owner::{
        CitizenEvidenceLimits, CitizenEvidenceSource, CitizenWorkforceEvidenceOwner,
    };
    struct Source {
        bridge: BridgeManifest,
        page: ObservationPage,
        active: Rc<Cell<u32>>,
    }
    impl Drop for Source {
        fn drop(&mut self) {
            self.active.set(self.active.get() - 1);
        }
    }
    impl CitizenEvidenceSource for Source {
        fn bridge_manifest(&self) -> BridgeManifest {
            self.bridge.clone()
        }
        fn read_page(
            &mut self,
            offset: u32,
            maximum: u32,
            _: bool,
            c: &OperationContext,
        ) -> Result<ObservationPage> {
            c.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
            assert_eq!(offset, 0);
            assert_eq!(maximum, 1);
            Ok(self.page.clone())
        }
    }
    let f = Fixture::new()?;
    let active = Rc::new(Cell::new(0));
    let opened = Rc::new(Cell::new(0));
    let calls = Rc::new(Cell::new(0));
    let bridge = f.observer.capsule.bridge.clone();
    let (_, _, page) = capsule(&queue()?)?;
    let active_source = Rc::clone(&active);
    let opened_source = Rc::clone(&opened);
    let factory = move |c: &OperationContext| {
        c.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
        assert_eq!(active_source.get(), 0);
        active_source.set(1);
        opened_source.set(opened_source.get() + 1);
        Ok(Source {
            bridge: bridge.clone(),
            page: page.clone(),
            active: Rc::clone(&active_source),
        })
    };
    let active_policy = Rc::clone(&active);
    let policy_calls = Rc::clone(&calls);
    let authorizer = move |_: &LiveObservationCapsule,
                           projection: &LiveWorldProjection,
                           c: &OperationContext| {
        c.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
        assert_eq!(
            active_policy.get(),
            0,
            "acquisition must close before source authorization"
        );
        policy_calls.set(policy_calls.get() + 1);
        Ok(policy(&projection.snapshot))
    };
    let mut evidence = CitizenWorkforceEvidenceOwner::new::<Source>(
        f.observer.capsule.clone(),
        CitizenEvidenceLimits {
            page_size: 1,
            max_citizens: 1,
        },
        factory,
        authorizer,
        &f.context,
    )?;
    let mut s = f.session(true, JournalMode::Control, false)?;
    let review = s.observe(f.original.clone(), &mut evidence, &f.context)?;
    s.prepare(review.seal(), &mut evidence, &f.context)?;
    let created = s.commit(review.seal(), true, &mut evidence, &f.context)?;
    assert!(matches!(
        created.creation_result(),
        NativeOrderCreation::Inserted { .. }
    ));
    assert_eq!(opened.get(), 4); // Constructor, observation, prepare, dispatch.
    assert_eq!(calls.get(), 8); // Exact independent policy is checked twice each time.
    assert_eq!(active.get(), 0);
    assert_eq!(f.native.borrow().commits, 1);
    Ok(())
}

#[test]
fn later_original_sources_survive_restart_and_block_older_control_without_hiding_history()
-> Result<()> {
    let mut f = Fixture::new()?;
    let old_plan = f.original.clone();
    let old_context = f.context.clone();
    let mut s = f.session(true, JournalMode::Control, false)?;
    let old_review = s.observe(old_plan.clone(), &mut f.observer, &f.context)?;
    s.prepare(old_review.seal(), &mut f.observer, &f.context)?;
    let before = queue()?;
    f.advance_source(
        changed_clock(
            &before,
            before.generation(),
            before.sequence(),
            before.tick() + 10,
        )?,
        ObservationCursor {
            epoch: 4,
            sequence: 21,
        },
        2,
    )?;
    let recent = s.observe(f.original.clone(), &mut f.observer, &f.context)?;
    s.prepare(recent.seal(), &mut f.observer, &f.context)?;
    let reads = f.native.borrow().reads;
    assert_eq!(
        s.observe(old_plan.clone(), &mut f.observer, &old_context)
            .err()
            .ok_or_else(rejected)?
            .code,
        ErrorCode::StaleAnchor,
    );
    assert_eq!(f.native.borrow().reads, reads);
    drop(s);
    let mut reopened = f.session(false, JournalMode::Control, false)?;
    assert!(
        reopened
            .observe(old_plan.clone(), &mut f.observer, &old_context)
            .is_err()
    );
    assert_eq!(f.native.borrow().reads, reads);
    let historical = reopened.reattach(old_plan.clone(), &f.context)?;
    assert_eq!(historical.seal(), old_review.seal());
    assert_eq!(
        reopened
            .inspect(historical.seal(), &f.context)?
            .native_record()
            .state(),
        CreationState::Prepared,
    );
    drop(reopened);
    // The original caller tick cannot resurrect a Query grant that expired after
    // an already retained newer source, even while reopening only for history.
    f.context = old_context;
    for grant in &mut f.context.grants {
        grant.expires_at_tick = Some(GameTick(before.tick() + 5));
    }
    assert!(f.session(false, JournalMode::Reconcile, true).is_err());
    assert_eq!(f.native.borrow().commits, 0);
    Ok(())
}

#[test]
fn fresh_source_owners_cannot_fork_a_cursor_or_cross_an_unestablished_restore_epoch() -> Result<()>
{
    let mut f = Fixture::new()?;
    let mut s = f.session(true, JournalMode::Control, false)?;
    let old = s.observe(f.original.clone(), &mut f.observer, &f.context)?;
    s.prepare(old.seal(), &mut f.observer, &f.context)?;
    let before = f.native.borrow().reads;
    // Altering the complete anchor while retaining its canonical cursor is a
    // fork even if a replacement owner would endorse that different capsule.
    let mut fork = f.original.clone();
    fork.anchor.state_hash = Digest32::of_bytes(b"different same-cursor world");
    fork.steps[0].idempotency_key = derive_step_idempotency_key(
        fork.intent_id,
        fork.anchor,
        StepId::ZERO,
        &fork.steps[0].action,
    );
    let fork = reseal(fork);
    let mut context = f.context.clone();
    context.anchor = fork.anchor;
    assert!(s.observe(fork, &mut f.observer, &context).is_err());
    assert_eq!(f.native.borrow().reads, before);
    let previous = f.original.clone();
    let o = queue()?;
    f.advance_source(
        changed_clock(&o, o.generation(), o.sequence(), 1)?,
        ObservationCursor {
            epoch: 5,
            sequence: 0,
        },
        3,
    )?;
    // Restore clocks can be lower. Current Query authority can inspect old
    // custody without being evaluated against the old incarnation's larger tick.
    for grant in &mut f.context.grants {
        grant.expires_at_tick = Some(GameTick(10));
    }
    let historical = s.reattach(previous, &f.context)?;
    assert_eq!(
        s.inspect(historical.seal(), &f.context)?
            .native_record()
            .state(),
        CreationState::Prepared
    );
    assert_eq!(
        s.observe(f.original.clone(), &mut f.observer, &f.context)
            .err()
            .ok_or_else(rejected)?
            .code,
        ErrorCode::StaleAnchor,
    );
    assert_eq!(f.native.borrow().reads, before);
    assert_eq!(f.native.borrow().prepares, 1);
    Ok(())
}

#[test]
fn fresh_native_observations_advance_authority_even_when_original_evidence_is_stale() -> Result<()>
{
    let mut f = Fixture::new()?;
    let mut s = f.session(true, JournalMode::Control, false)?;
    let original = queue()?;
    f.native.borrow_mut().queue = changed_clock(
        &original,
        original.generation(),
        original.sequence(),
        original.tick() + 10,
    )?;
    assert!(
        s.observe(f.original.clone(), &mut f.observer, &f.context)
            .is_err()
    );
    assert_eq!(f.native.borrow().reads, 1);
    f.native.borrow_mut().queue = original.clone();
    assert!(
        s.observe(f.original.clone(), &mut f.observer, &f.context)
            .is_err()
    );
    assert_eq!(
        f.native.borrow().reads,
        1,
        "old caller anchor must fail before another source read"
    );
    let mut old_authority = f.context.clone();
    for grant in &mut old_authority.grants {
        grant.expires_at_tick = Some(GameTick(original.tick() + 5));
    }
    assert!(s.inventory(&old_authority).is_err());
    assert_eq!(f.native.borrow().prepares, 0);
    assert_eq!(f.observer.refreshes, 0);
    Ok(())
}

#[test]
fn created_native_sequence_survives_restart_and_rejects_replayed_or_reincarnated_capture()
-> Result<()> {
    for reincarnated in [false, true] {
        let mut f = Fixture::new()?;
        let mut s = f.session(true, JournalMode::Control, false)?;
        let first = s.observe(f.original.clone(), &mut f.observer, &f.context)?;
        s.prepare(first.seal(), &mut f.observer, &f.context)?;
        s.commit(first.seal(), true, &mut f.observer, &f.context)?;
        drop(s);
        let original = queue()?;
        f.native.borrow_mut().queue = if reincarnated {
            changed_clock(
                &original,
                original.generation() + 1,
                original.sequence() + 1,
                original.tick(),
            )?
        } else {
            original
        };
        let mut reopened = f.session(false, JournalMode::Control, false)?;
        let next = plan(&f.observer.projection.snapshot, "CONSTRUCT_DOOR", 2);
        assert_eq!(
            reopened
                .observe(next, &mut f.observer, &f.context)
                .err()
                .ok_or_else(rejected)?
                .code,
            ErrorCode::StaleAnchor,
        );
        assert_eq!(f.native.borrow().prepares, 1);
        assert_eq!(f.native.borrow().commits, 1);
        // Historical proof is still exact and accessible after the rejected read.
        let original = reopened.reattach(f.original.clone(), &f.context)?;
        assert!(matches!(
            reopened
                .inspect(original.seal(), &f.context)?
                .creation_result(),
            NativeOrderCreation::Inserted { .. }
        ));
    }
    Ok(())
}
