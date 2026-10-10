// Shared test-only protocol encoders, storage, native source, and session fixture.
// All interactions with the production adapter use its ordinary public API.

use crate::control_effect_journal::EffectJournalStorage;
use crate::live_projection::{LiveWorldProjection, project_live_capsule};
use crate::live_routing::LiveRoutingEvidence;
use crate::semantic_workforce::store::AssociationStore;
use crate::semantic_workforce::{
    SemanticWorkforceResult, SemanticWorkforceReview, SemanticWorkforceSession, SingleLaborResult,
    WorkforceEvidenceOwner,
};
use crate::workforce_control::journal::{
    AssignmentState, WorkforceBinding, WorkforceJournal, WorkforceMode,
};
use crate::workforce_control::rpc::{CONNECT_BYTES, RPC_BYTES, WorkforceManifest, WorkforceSource};
use crate::workforce_control::{
    AssignmentEffect, AssignmentPhase, AssignmentPlan, MAX_EFFECT, WorkforceCapture,
};
use crate::workforce_session::WorkforceSession;
use crate::{
    BridgeManifest, CitizenRecord, LiveObservationCapsule, ObservationAssembler, ObservationPage,
};
use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, DfmcpError, Digest32, EntityId, ErrorCode,
    GameTick, IntentId, ObservationCursor, OperationContext, RequestId, Result, RiskTier,
    SessionId, StepId, WorkBudget,
};
use dfmcp_intent::{Action, ObligationSpec, PlanStep, PreparedPlan, derive_step_idempotency_key};
use dfmcp_world::{
    CompareOp, EntityKind, EvidenceCoverage, EvidencePolicy, EvidenceSource, FactSource, Predicate,
    PredicateEvidence, Value, WorldSnapshot,
};
use std::cell::RefCell;
use std::collections::BTreeSet;
use std::io::{self, Cursor, Read, Seek, SeekFrom, Write};
use std::net::SocketAddr;
use std::rc::Rc;
use std::time::Duration;

fn error(code: ErrorCode, message: &str) -> DfmcpError {
    DfmcpError::new(code, message)
}

// Independent encoding of the fixture receipt's fixed protocol domain.
fn fixture_receipt(bytes: &[u8]) -> Digest32 {
    let mut tagged = b"dfmcp-workforce-receipt/1\0".to_vec();
    tagged.extend_from_slice(bytes);
    Digest32::of_bytes(&tagged)
}

fn invalid(message: impl Into<String>) -> dfmcp_core::DfmcpError {
    error(ErrorCode::AdapterRejected, &message.into())
}
fn text(out: &mut Vec<u8>, value: &str) {
    out.extend_from_slice(&(value.len() as u16).to_be_bytes());
    out.extend_from_slice(value.as_bytes());
}

fn observation(ids: &[i32]) -> Result<(LiveObservationCapsule, LiveWorldProjection)> {
    let mut assembler = ObservationAssembler::new(BridgeManifest {
        bridge_version: "0.1.0".to_owned(),
        df_version: "0.51.11".to_owned(),
        dfhack_version: "0.51.11-r1".to_owned(),
        world_loaded: true,
        fortress_mode: true,
        bridge_generation: 7,
        supported_methods: BTreeSet::from(["Handshake".to_owned(), "ReadObservation".to_owned()]),
    });
    assembler.push_page(ObservationPage {
        bridge_generation: 7,
        world_loaded: true,
        fortress_mode: true,
        paused: true,
        current_year: 0,
        current_year_tick: 120,
        world_name: "Routing fixture".to_owned(),
        world_folder: "region1".to_owned(),
        site_id: 4,
        citizen_count_total: ids.len() as u32,
        citizen_offset: 0,
        complete: true,
        citizens: ids
            .iter()
            .map(|id| CitizenRecord {
                unit_id: *id,
                name: format!("Urist {id}"),
                race: "dwarf".to_owned(),
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
            })
            .collect(),
    })?;
    let capsule = assembler.finalize()?;
    let projection = project_live_capsule(
        &capsule,
        crate::workforce_control::fortress_id("region1", 4),
        ObservationCursor::ORIGIN,
    )?;
    Ok((capsule, projection))
}

fn policy(snapshot: &WorldSnapshot) -> EvidencePolicy {
    let mut policy = EvidencePolicy::at(snapshot.anchor());
    policy.all_entities = EvidenceCoverage::Observed;
    policy.paused = true;
    for entity in snapshot.graph.entities.values() {
        for fact in entity.fields.values() {
            if let FactSource::DfhackField(field) = &fact.source {
                policy.sources.insert(EvidenceSource::Observed {
                    field: field.clone(),
                    source_digest: fact.source_digest,
                });
            }
        }
    }
    policy
}

fn plan_with_preconditions(
    snapshot: &WorldSnapshot,
    action: Action,
    preconditions: Vec<Predicate>,
) -> PreparedPlan {
    let action = action.normalized();
    let terminal = Predicate::Paused(true);
    let intent = IntentId::new(1);
    let step = StepId::new(0);
    let deadline = GameTick(snapshot.tick.get() + 100);
    let obligation = action.naturally_temporal().then(|| ObligationSpec {
        terminal: terminal.clone(),
        failure: None,
        deadline_tick: deadline,
        poll_interval_ticks: 1,
        stable_for_observations: 1,
    });
    PreparedPlan::builder(
        intent,
        snapshot.anchor(),
        "semantic routing fixture",
        terminal.clone(),
    )
    .max_risk(action.risk())
    .required_capabilities(BTreeSet::from([action.capability()]))
    .expires_at_tick(deadline)
    .steps(vec![PlanStep {
        id: step,
        idempotency_key: derive_step_idempotency_key(intent, snapshot.anchor(), step, &action),
        required_capability: action.capability(),
        risk: action.risk(),
        action,
        preconditions,
        postconditions: vec![terminal],
        compensation: None,
        obligation,
        depends_on: Vec::new(),
    }])
    .build()
}

fn labor(units: Vec<EntityId>, enabled: bool) -> Action {
    Action::SetLabor {
        units,
        labor: "MINE".to_owned(),
        enabled,
    }
}

#[derive(Clone)]
struct DetailFixture {
    name: String,
    selected_only: bool,
    labors: Vec<u8>,
    members: Vec<u32>,
}

#[derive(Clone)]
struct CitizenFixture {
    id: u32,
    eligible: bool,
    labors: Vec<u8>,
}

impl CitizenFixture {
    fn append(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.id.to_be_bytes());
        out.extend_from_slice(&(self.id + 100).to_be_bytes());
        out.push(u8::from(self.eligible));
        out.extend_from_slice(&self.labors);
    }
}

#[derive(Clone)]
struct WorkforceFixture {
    generation: u64,
    sequence: u64,
    tick: u64,
    site: u32,
    folder: String,
    paused: bool,
    automatic: bool,
    details: Vec<DetailFixture>,
    citizens: Vec<CitizenFixture>,
}

impl Default for WorkforceFixture {
    fn default() -> Self {
        Self {
            generation: 7,
            sequence: 3,
            tick: 120,
            site: 4,
            folder: "region1".to_owned(),
            paused: true,
            automatic: true,
            details: vec![DetailFixture {
                name: "Mining only".to_owned(),
                selected_only: true,
                labors: vec![1, 0],
                members: Vec::new(),
            }],
            citizens: vec![CitizenFixture {
                id: 42,
                eligible: true,
                labors: vec![0, 0],
            }],
        }
    }
}

impl WorkforceFixture {
    fn capture(&self) -> Result<WorkforceCapture> {
        let mut out = b"DFMWF017".to_vec();
        for n in [self.generation, self.sequence, self.tick] {
            out.extend_from_slice(&n.to_be_bytes());
        }
        out.extend_from_slice(&self.site.to_be_bytes());
        text(&mut out, &self.folder);
        out.extend_from_slice(&[u8::from(self.paused), u8::from(self.automatic)]);
        out.extend_from_slice(&2u16.to_be_bytes());
        text(&mut out, "MINE");
        text(&mut out, "CARPENTRY");
        out.extend_from_slice(&(self.details.len() as u16).to_be_bytes());
        for detail in &self.details {
            text(&mut out, &detail.name);
            out.extend_from_slice(&0u32.to_be_bytes());
            out.push(u8::from(detail.selected_only));
            out.extend_from_slice(&detail.labors);
            out.extend_from_slice(&(detail.members.len() as u16).to_be_bytes());
            for id in &detail.members {
                out.extend_from_slice(&id.to_be_bytes());
            }
        }
        out.extend_from_slice(&(self.citizens.len() as u16).to_be_bytes());
        for citizen in &self.citizens {
            citizen.append(&mut out);
        }
        WorkforceCapture::decode(&out)
    }

    fn effect(
        &self,
        plan: &AssignmentPlan,
        labors: &[u8],
        applied: bool,
    ) -> Result<AssignmentEffect> {
        let mut out = b"DFMWE017".to_vec();
        text(&mut out, plan.key());
        out.extend_from_slice(plan.digest().as_bytes());
        out.extend_from_slice(plan.token());
        out.extend_from_slice(&plan.spec().detail().to_be_bytes());
        out.push(u8::from(plan.spec().assigned()));
        out.extend_from_slice(plan.before().witness().as_bytes());
        for n in [self.generation, self.sequence, self.tick] {
            out.extend_from_slice(&n.to_be_bytes());
        }
        out.push(if applied { 2 } else { 1 });
        let mut after = self.clone();
        if applied {
            after.sequence += 1;
            let members = &mut after.details[plan.spec().detail() as usize].members;
            for citizen in &self.citizens {
                if plan.spec().assigned() {
                    members.push(citizen.id);
                } else {
                    members.retain(|id| *id != citizen.id);
                }
            }
            members.sort_unstable();
            members.dedup();
            for citizen in &mut after.citizens {
                citizen.labors = labors.to_vec();
            }
            out.extend_from_slice(after.capture()?.witness().as_bytes());
        } else {
            out.extend_from_slice(Digest32::ZERO.as_bytes());
        }
        out.extend_from_slice(&2u16.to_be_bytes());
        out.extend_from_slice(
            &(if applied {
                after.citizens.len() as u16
            } else {
                0
            })
            .to_be_bytes(),
        );
        if applied {
            for citizen in &after.citizens {
                citizen.append(&mut out);
            }
        }
        let receipt = fixture_receipt(&out);
        out.extend_from_slice(receipt.as_bytes());
        AssignmentEffect::decode(&out, plan)
    }
}

#[derive(Default)]
struct Memory {
    file: Cursor<Vec<u8>>,
    synced: Vec<u8>,
    fail_sync: bool,
    fail_write: bool,
}
#[derive(Clone, Default)]
struct Storage(Rc<RefCell<Memory>>);
impl Storage {
    fn from_bytes(bytes: Vec<u8>) -> Self {
        Self(Rc::new(RefCell::new(Memory {
            file: Cursor::new(bytes.clone()),
            synced: bytes,
            ..Memory::default()
        })))
    }
    fn crash(&self) {
        let mut memory = self.0.borrow_mut();
        memory.file = Cursor::new(memory.synced.clone());
        memory.fail_sync = false;
        memory.fail_write = false;
    }
}
impl Read for Storage {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        self.0.borrow_mut().file.read(out)
    }
}
impl Seek for Storage {
    fn seek(&mut self, at: SeekFrom) -> io::Result<u64> {
        self.0.borrow_mut().file.seek(at)
    }
}
impl Write for Storage {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let mut memory = self.0.borrow_mut();
        if memory.fail_write {
            return Err(io::Error::other("injected association write failure"));
        }
        memory.file.write(bytes)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
impl EffectJournalStorage for Storage {
    fn sync(&mut self) -> io::Result<()> {
        let mut memory = self.0.borrow_mut();
        if memory.fail_sync {
            return Err(io::Error::other("injected association sync failure"));
        }
        memory.synced = memory.file.get_ref().clone();
        Ok(())
    }
    fn truncate(&mut self, _: u64) -> io::Result<()> {
        Err(io::Error::other("no repair"))
    }
}

fn context(snapshot: &WorldSnapshot) -> OperationContext {
    OperationContext {
        session_id: SessionId::new(1),
        request_id: RequestId::new(1),
        anchor: snapshot.anchor(),
        budget: WorkBudget {
            max_wall_millis: 10000,
            max_bytes: 256 * 1024 * 1024,
            max_output_tokens: 65536,
            max_entities: 8192,
            max_actions: 1,
            max_game_ticks: 0,
        },
        grants: [
            Capability::Query,
            Capability::Plan,
            Capability::ConfigureLabor,
        ]
        .into_iter()
        .map(|capability| CapabilityGrant {
            capability,
            scope: CapabilityScope {
                fortress_id: Some(snapshot.fortress_id),
                ..CapabilityScope::default()
            },
            max_risk: RiskTier::Guarded,
            expires_at_tick: None,
            remaining_uses: None,
        })
        .collect(),
        cancellation_requested: false,
    }
}
struct Observer {
    capsule: LiveObservationCapsule,
    projection: LiveWorldProjection,
    refreshes: usize,
    fail_at: Option<usize>,
    deny: bool,
    corrupt_at: Option<(usize, Storage)>,
    delay_at: Option<usize>,
}
impl WorkforceEvidenceOwner for Observer {
    fn refresh_after_capture(
        &mut self,
        capture: &WorkforceCapture,
        c: &OperationContext,
    ) -> Result<()> {
        assert!(c.budget.max_bytes <= 16 * 1024 * 1024);
        assert!(c.budget.max_wall_millis <= 10000);
        assert_eq!(capture.tick(), self.projection.snapshot.tick.get());
        self.refreshes += 1;
        if self.delay_at == Some(self.refreshes) {
            std::thread::sleep(Duration::from_millis(30));
        }
        if self.fail_at == Some(self.refreshes) {
            self.deny = true;
        }
        if let Some((at, storage)) = &self.corrupt_at
            && *at == self.refreshes
        {
            let mut memory = storage.0.borrow_mut();
            if let Some(last) = memory.file.get_mut().last_mut() {
                *last ^= 1;
            }
        }
        Ok(())
    }
    fn routing_evidence(&self) -> Result<LiveRoutingEvidence<'_>> {
        let snapshot = &self.projection.snapshot;
        let evidence = if self.deny {
            PredicateEvidence::untrusted(snapshot)?
        } else {
            PredicateEvidence::scoped(snapshot, policy(snapshot))?
        };
        LiveRoutingEvidence::citizens_v1(evidence, &self.projection, &self.capsule)
    }
}
fn effect(
    fixture: &WorkforceFixture,
    plan: &AssignmentPlan,
    phase: AssignmentPhase,
    labors: &[u8],
) -> Result<AssignmentEffect> {
    if phase == AssignmentPhase::Applied {
        return fixture.effect(plan, labors, true);
    }
    let unknown = fixture.effect(plan, &[], false)?;
    let mut bytes = unknown.canonical_bytes().to_vec();
    bytes[8 + 2 + plan.key().len() + 32 + 16 + 4 + 1 + 32 + 24] = phase as u8;
    bytes.truncate(bytes.len() - 32);
    let receipt = fixture_receipt(&bytes);
    bytes.extend_from_slice(receipt.as_bytes());
    assert!(bytes.len() <= MAX_EFFECT);
    AssignmentEffect::decode(&bytes, plan)
}
struct NativeState {
    fixture: WorkforceFixture,
    calls: [usize; 5],
    effect: Option<AssignmentEffect>,
    labors: Vec<u8>,
    unknown: bool,
    lose_commit: bool,
    fail_connect: bool,
    edges: Vec<(u8, u64, u64, GameTick)>,
}
#[derive(Clone)]
struct Native {
    binding: WorkforceBinding,
    journal: Storage,
    associations: Storage,
    data: Rc<RefCell<NativeState>>,
    context: OperationContext,
}
impl Native {
    fn record_edge(&self, kind: u8, c: &OperationContext) {
        assert_eq!(
            c.budget.max_bytes,
            if kind == 5 { CONNECT_BYTES } else { RPC_BYTES }
        );
        assert!(c.budget.max_wall_millis > 0 && c.budget.max_wall_millis <= 10000);
        self.data.borrow_mut().edges.push((
            kind,
            c.budget.max_wall_millis,
            c.budget.max_bytes,
            c.anchor.tick,
        ));
    }
    fn connect(&self, binding: &WorkforceBinding, c: &OperationContext) -> Result<Self> {
        self.record_edge(5, c);
        assert_eq!(binding, &self.binding);
        if self.data.borrow().fail_connect {
            return Err(invalid("injected connection loss"));
        }
        Ok(self.clone())
    }
    fn durable(&self, plan: &AssignmentPlan) -> Result<AssignmentState> {
        let storage = Storage::from_bytes(self.journal.0.borrow().synced.clone());
        let mut journal =
            WorkforceJournal::open(storage, &self.context, WorkforceMode::Offline, None)?;
        let view = journal.view(&self.context)?;
        let store = AssociationStore::open(
            Storage::from_bytes(self.associations.0.borrow().synced.clone()),
            view.id,
            false,
            true,
            &self.context,
        )?;
        let state = journal
            .get(plan.key(), plan.digest(), &self.context)?
            .state();
        // Exercise the public coordinator's real custody checks against only
        // synced bytes before allowing the injected native effect. Its opening
        // view verifies the native digest and witness of every association.
        let (session, _) = WorkforceSession::new(journal, &self.context)?;
        SemanticWorkforceSession::new(session, store, &self.context)?;
        Ok(state)
    }
}
impl WorkforceSource for Native {
    fn manifest(&self) -> &WorkforceManifest {
        self.binding.manifest()
    }
    fn endpoint(&self) -> Option<SocketAddr> {
        Some(self.binding.endpoint())
    }
    fn observe(&mut self, ids: &[u32], c: &OperationContext) -> Result<WorkforceCapture> {
        self.record_edge(0, c);
        let mut data = self.data.borrow_mut();
        data.calls[0] += 1;
        let capture = data.fixture.capture()?;
        assert_eq!(ids, capture.ids());
        Ok(capture)
    }
    fn prepare(&mut self, plan: &AssignmentPlan, c: &OperationContext) -> Result<AssignmentEffect> {
        self.record_edge(1, c);
        assert_eq!(self.durable(plan)?, AssignmentState::Intent);
        let mut data = self.data.borrow_mut();
        data.calls[1] += 1;
        let effect = effect(&data.fixture, plan, AssignmentPhase::Prepared, &[])?;
        data.effect = Some(effect.clone());
        Ok(effect)
    }
    fn commit(&mut self, plan: &AssignmentPlan, c: &OperationContext) -> Result<AssignmentEffect> {
        self.record_edge(2, c);
        assert_eq!(self.durable(plan)?, AssignmentState::DispatchStarted);
        let mut data = self.data.borrow_mut();
        data.calls[2] += 1;
        let effect = effect(
            &data.fixture,
            plan,
            if data.unknown {
                AssignmentPhase::Unknown
            } else {
                AssignmentPhase::Applied
            },
            &data.labors,
        )?;
        data.effect = Some(effect.clone());
        if data.lose_commit {
            return Err(invalid("lost assignment acknowledgement"));
        }
        Ok(effect)
    }
    fn query(
        &mut self,
        _: &AssignmentPlan,
        c: &OperationContext,
    ) -> Result<Option<AssignmentEffect>> {
        self.record_edge(3, c);
        let mut data = self.data.borrow_mut();
        data.calls[3] += 1;
        Ok(data.effect.clone())
    }
    fn cancel(&mut self, plan: &AssignmentPlan, c: &OperationContext) -> Result<AssignmentEffect> {
        self.record_edge(4, c);
        let mut data = self.data.borrow_mut();
        data.calls[4] += 1;
        let effect = effect(&data.fixture, plan, AssignmentPhase::Cancelled, &[])?;
        data.effect = Some(effect.clone());
        Ok(effect)
    }
}
fn never(_: &WorkforceBinding, _: &OperationContext) -> Result<Native> {
    Err(invalid("unexpected native connection"))
}

type Owner = SemanticWorkforceSession<Storage, Storage>;
struct Fixture {
    owner: Owner,
    native: Native,
    observer: Observer,
    context: OperationContext,
    plan: PreparedPlan,
}
fn fixture() -> Result<Fixture> {
    let (capsule, projection) = observation(&[42])?;
    let c = context(&projection.snapshot);
    let binding = WorkforceBinding::new(
        SocketAddr::from(([127, 0, 0, 1], 5000)),
        WorkforceManifest {
            generation: 7,
            df_version: capsule.bridge.df_version.clone(),
            dfhack_version: capsule.bridge.dfhack_version.clone(),
        },
        "region1".to_owned(),
        4,
    )?;
    let journal_storage = Storage::default();
    let journal = WorkforceJournal::open(
        journal_storage.clone(),
        &c,
        WorkforceMode::Control,
        Some((binding.clone(), [1; 32])),
    )?;
    let (session, view) = WorkforceSession::new(journal, &c)?;
    let association_storage = Storage::default();
    let store = AssociationStore::open(association_storage.clone(), view.id, true, false, &c)?;
    let owner = SemanticWorkforceSession::new(session, store, &c)?;
    let native = Native {
        binding,
        journal: journal_storage,
        associations: association_storage,
        data: Rc::new(RefCell::new(NativeState {
            fixture: WorkforceFixture::default(),
            calls: [0; 5],
            effect: None,
            labors: vec![1, 0],
            unknown: false,
            lose_commit: false,
            fail_connect: false,
            edges: Vec::new(),
        })),
        context: c.clone(),
    };
    let mut plan = plan_with_preconditions(
        &projection.snapshot,
        labor(vec![EntityId::new(43)], true),
        vec![Predicate::Paused(true)],
    );
    plan.steps[0].obligation = Some(ObligationSpec {
        terminal: Predicate::FieldCompare {
            entity_id: EntityId::new(43),
            field: "productive".to_owned(),
            op: CompareOp::Eq,
            value: Value::Bool(true),
        },
        failure: None,
        deadline_tick: GameTick(200),
        poll_interval_ticks: 5,
        stable_for_observations: 3,
    });
    reseal(&mut plan);
    Ok(Fixture {
        owner,
        native,
        observer: Observer {
            capsule,
            projection,
            refreshes: 0,
            fail_at: None,
            deny: false,
            corrupt_at: None,
            delay_at: None,
        },
        context: c,
        plan,
    })
}
fn reseal(plan: &mut PreparedPlan) {
    plan.digest = plan.compute_digest();
    plan.id = plan.expected_id();
}
fn review(f: &mut Fixture) -> Result<SemanticWorkforceReview> {
    f.owner
        .observe(f.plan.clone(), &mut f.observer, &f.context, |binding, c| {
            f.native.connect(binding, c)
        })
}
fn prepared(f: &mut Fixture) -> Result<SemanticWorkforceReview> {
    let review = review(f)?;
    let result = f
        .owner
        .prepare(review.seal(), &mut f.observer, &f.context, |binding, c| {
            f.native.connect(binding, c)
        })?;
    assert_eq!(result.native_record().state(), AssignmentState::Prepared);
    Ok(review)
}
fn reopen(native: &Native, c: &OperationContext, store: Storage, read_only: bool) -> Result<Owner> {
    let journal = WorkforceJournal::open(native.journal.clone(), c, WorkforceMode::Control, None)?;
    let (session, view) = WorkforceSession::new(journal, c)?;
    let associations = AssociationStore::open(store, view.id, false, read_only, c)?;
    SemanticWorkforceSession::new(session, associations, c)
}
