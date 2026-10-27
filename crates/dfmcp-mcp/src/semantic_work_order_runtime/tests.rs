//! Complete runtime paths use strict native fixtures, actual canonical citizen
//! acquisition, real semantic/native journals, exact archives and goal history.
#[path = "clock_runtime_tests.rs"]
mod clock_runtime_tests;
use super::*;
use dfmcp_adapter::live_jobs::{LiveJob, LiveJobObservation};
use dfmcp_adapter::live_operations::{LiveItem, LiveOperationsObservation};
use dfmcp_adapter::semantic_workforce::evidence_owner::{
    CitizenEvidenceLimits, CitizenEvidenceSource, CitizenWorkforceEvidenceOwner,
};
use dfmcp_adapter::work_order_control::{WorkOrderJournal, WorkOrderQuerySource};
use dfmcp_adapter::work_order_progress::{ProgressManifest, ProgressObservation};
use dfmcp_adapter::work_orders::rpc::WorkOrderManifest;
use dfmcp_adapter::work_orders::{
    WorkOrderEffect, WorkOrderObservation, WorkOrderPlan, WorkOrderState,
};
use dfmcp_adapter::{BridgeManifest, CitizenRecord, ObservationAssembler, ObservationPage};
use dfmcp_core::MapCoord;
use dfmcp_world::{EntityKind, EvidenceCoverage, EvidencePolicy};
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::rc::Rc;

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
    let raw =
        include_str!("../../../dfmcp-adapter/tests/fixtures/work_order_observation_v1_10.hex")
            .trim();
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

// The canonical read profile is independent of work-orders/1.10. Keep this
// different in every prepare/commit/recovery and original-goal lifecycle test.
const CITIZEN_GENERATION: u64 = 701;

fn capsule(
    o: &WorkOrderObservation,
) -> Result<(LiveObservationCapsule, LiveWorldProjection, ObservationPage)> {
    let bridge = BridgeManifest {
        bridge_version: "0.1.0".into(),
        df_version: "0.51.11".into(),
        dfhack_version: "0.51.11-r1".into(),
        bridge_generation: CITIZEN_GENERATION,
        world_loaded: true,
        fortress_mode: true,
        supported_methods: BTreeSet::from(["Handshake".into(), "ReadObservation".into()]),
    };
    let page = ObservationPage {
        bridge_generation: CITIZEN_GENERATION,
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
struct Citizens {
    bridge: BridgeManifest,
    page: ObservationPage,
}
impl CitizenEvidenceSource for Citizens {
    fn bridge_manifest(&self) -> BridgeManifest {
        self.bridge.clone()
    }
    fn read_page(
        &mut self,
        offset: u32,
        maximum: u32,
        _: bool,
        _: &OperationContext,
    ) -> Result<ObservationPage> {
        if offset != 0 || maximum < self.page.citizen_count_total {
            return Err(rejected());
        }
        Ok(self.page.clone())
    }
}
type Evidence = CitizenWorkforceEvidenceOwner<
    Box<dyn FnMut(&OperationContext) -> Result<Citizens>>,
    Box<
        dyn FnMut(
            &LiveObservationCapsule,
            &LiveWorldProjection,
            &OperationContext,
        ) -> Result<EvidencePolicy>,
    >,
>;

fn operations(queue: &WorkOrderObservation, items: bool, jobs: bool) -> LiveOperationsObservation {
    LiveOperationsObservation {
        jobs: LiveJobObservation {
            bridge_generation: 601,
            df_version: "0.51.11".into(),
            dfhack_version: "0.51.11-r1".into(),
            year: (queue.tick() / 403_200) as u32,
            year_tick: (queue.tick() % 403_200) as u32,
            paused: true,
            site_id: queue.site_id() as i32,
            world_folder: queue.world_folder().into(),
            next_job_id: 100,
            jobs: if jobs {
                vec![LiveJob {
                    native_id: 3,
                    job_type: 77,
                    type_key: "ConstructBed".into(),
                    reaction: String::new(),
                    suspended: false,
                    repeating: false,
                    position: MapCoord::new(1, 1, 1),
                    worker_native_id: None,
                    holder_native_id: None,
                    completion_timer: 20,
                    attached_item_count: 0,
                    required_item_filter_count: 0,
                }]
            } else {
                Vec::new()
            },
        },
        next_building_id: 100,
        next_item_id: 100,
        buildings: Vec::new(),
        items: if items {
            vec![LiveItem {
                native_id: 2,
                item_type: 5,
                type_key: "BED".into(),
                subtype: -1,
                material_type: 420,
                material_index: 0,
                stack_size: 1,
                raw_position: MapCoord::new(1, 1, 1),
                flags: 64,
                container_native_id: None,
                holder_building_native_id: None,
            }]
        } else {
            Vec::new()
        },
        attachments: Vec::new(),
    }
}
fn progress(
    queue: &WorkOrderObservation,
    id: u32,
    sequence: u64,
    remaining: i32,
    recipe: u8,
    status: u32,
) -> Result<ProgressObservation> {
    let mut bytes = b"DFMWP012".to_vec();
    for n in [501u64, sequence, queue.tick()] {
        bytes.extend_from_slice(&n.to_be_bytes());
    }
    for n in [
        queue.site_id(),
        queue.next_order_id(),
        queue.order_ids().len() as u32,
    ] {
        bytes.extend_from_slice(&n.to_be_bytes());
    }
    bytes.push(1);
    text(&mut bytes, queue.world_folder());
    bytes.extend_from_slice(&1u32.to_be_bytes());
    bytes.extend_from_slice(&id.to_be_bytes());
    let present = queue.order_ids().contains(&id);
    bytes.push(u8::from(present));
    if present {
        bytes.extend_from_slice(&77i32.to_be_bytes());
        bytes.push(recipe);
        for n in [remaining, 5] {
            bytes.extend_from_slice(&n.to_be_bytes());
        }
        bytes.extend_from_slice(&status.to_be_bytes());
        for n in [0i32, -1, 1, 0, 0] {
            bytes.extend_from_slice(&n.to_be_bytes());
        }
        bytes.extend_from_slice(&0u32.to_be_bytes());
        bytes.extend_from_slice(&0u32.to_be_bytes());
        text(
            &mut bytes,
            match recipe {
                2 => "ConstructDoor",
                3 => "ConstructTable",
                4 => "ConstructThrone",
                _ => "ConstructBed",
            },
        );
        text(&mut bytes, "");
    }
    ProgressObservation::decode(&bytes, &[id])
}
struct ProgressFixture {
    manifest: ProgressManifest,
    captures: Vec<ProgressObservation>,
    closed: Rc<Cell<usize>>,
}
impl ProgressSource for ProgressFixture {
    fn manifest(&self) -> &ProgressManifest {
        &self.manifest
    }
    fn fence(&mut self) {}
    fn read(&mut self, ids: &[u32], timeout: Duration) -> Result<ProgressObservation> {
        assert!(timeout <= Duration::from_secs(60));
        if self.captures.is_empty() {
            return Err(rejected());
        }
        let raw = self.captures.remove(0);
        ProgressObservation::decode(raw.canonical_bytes(), ids)
    }
}
impl Drop for ProgressFixture {
    fn drop(&mut self) {
        self.closed.set(self.closed.get() + 1);
    }
}
struct OperationsFixture {
    raw: LiveOperationsObservation,
    closed: Rc<Cell<usize>>,
}
impl OperationsGoalSource for OperationsFixture {
    fn read_operations(&mut self, c: &OperationContext) -> Result<LiveOperationsObservation> {
        assert!(c.budget.max_bytes <= 16 * 1024 * 1024);
        let raw = self.raw.encode_payload()?;
        LiveOperationsObservation::decode_payload(
            &raw,
            self.raw.jobs.bridge_generation,
            self.raw.jobs.df_version.clone(),
            self.raw.jobs.dfhack_version.clone(),
        )
    }
}
impl Drop for OperationsFixture {
    fn drop(&mut self) {
        self.closed.set(self.closed.get() + 1);
    }
}
fn native_progress(captures: Vec<ProgressObservation>, closed: Rc<Cell<usize>>) -> ProgressFixture {
    ProgressFixture {
        manifest: ProgressManifest {
            generation: 501,
            df_version: "0.51.11".into(),
            dfhack_version: "0.51.11-r1".into(),
        },
        captures,
        closed,
    }
}

#[derive(Clone)]
struct Resources {
    pins: policy::SourceAcceptance,
    native: Rc<RefCell<NativeState>>,
    journal: Memory,
    associations: Memory,
    archive: Memory,
    history: Memory,
    enabled: Rc<Cell<bool>>,
    policy_denied: Rc<Cell<bool>>,
    remaining: Rc<Cell<i32>>,
    jobs: Rc<Cell<bool>>,
    sequence: Rc<Cell<u64>>,
    recovery: Option<Recovery>,
    initialize: bool,
}
impl Resources {
    fn new() -> Result<Self> {
        let queue = queue()?;
        let trace = Trace::default();
        let pins = policy::SourceAcceptance {
            kind: policy::ACCEPTANCE_KIND.into(),
            world_folder: queue.world_folder().into(),
            site_id: queue.site_id(),
            df_version: "0.51.11".into(),
            dfhack_version: "0.51.11-r1".into(),
            citizen_bridge_version: "0.1.0".into(),
            citizen_generation: CITIZEN_GENERATION,
            work_order_generation: queue.generation(),
            progress_generation: 501,
            operations_generation: 601,
        };
        Ok(Self {
            pins,
            native: Rc::new(RefCell::new(NativeState {
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
            })),
            journal: Memory::new("native-journal", Rc::clone(&trace)),
            associations: Memory::new("semantic-associations", Rc::clone(&trace)),
            archive: Memory::new("original-archive", Rc::clone(&trace)),
            history: Memory::new("goal-history", trace),
            enabled: Rc::new(Cell::new(true)),
            policy_denied: Rc::new(Cell::new(false)),
            remaining: Rc::new(Cell::new(5)),
            jobs: Rc::new(Cell::new(false)),
            sequence: Rc::new(Cell::new(1)),
            recovery: None,
            initialize: true,
        })
    }
    fn advance(&self, ticks: u64, remaining: i32) -> Result<()> {
        let old = self.native.borrow().queue.clone();
        self.native.borrow_mut().queue =
            changed_clock(&old, old.generation(), old.sequence(), old.tick() + ticks)?;
        self.remaining.set(remaining);
        Ok(())
    }
    fn recover(&self, view: &View) -> Self {
        let mut next = self.clone();
        next.initialize = false;
        next.recovery = Some(Recovery {
            seal: view.archive.expect("retained archive"),
            anchor: view.original.as_ref().expect("original").anchor,
            history_present: view.history_present,
        });
        next
    }
}
impl Shell for Resources {
    type ClockJournal = Memory;
    type ClockSource =
        dfmcp_adapter::order_run::rpc::OrderRunRpc<dfmcp_adapter::order_run::rpc::OrderRunTcp>;
    fn clock_enabled(&self) -> bool {
        false
    }
    fn open_clock(
        &self,
        _: Mode,
        _: &OperationContext,
    ) -> Result<Option<OrderRunJournal<Self::ClockJournal>>> {
        Ok(None)
    }
    fn clock_source(&self, _: &OperationContext) -> Result<Self::ClockSource> {
        Err(rejected())
    }
    type Journal = Memory;
    type Association = Memory;
    type Native = Native;
    type Archive = Memory;
    type History = Memory;
    type Evidence = Evidence;
    type Progress = ProgressFixture;
    type Operations = OperationsFixture;
    fn acceptance(&self) -> &policy::SourceAcceptance {
        &self.pins
    }
    fn recovery(&self) -> Option<Recovery> {
        self.recovery
    }
    fn check_environment(&self) -> Result<()> {
        Ok(())
    }
    fn production_enabled(&self) -> bool {
        self.enabled.get()
    }
    fn open_control(
        &self,
        mode: Mode,
        c: &OperationContext,
    ) -> Result<(
        WorkOrderSession<Memory, Native>,
        AssociationStore<Memory>,
        Digest32,
    )> {
        let j = WorkOrderJournal::open(self.journal.clone(), c, mode.journal(), self.initialize)?;
        let id = j.summary(c)?.journal_id;
        let a = AssociationStore::open(
            self.associations.clone(),
            id,
            self.initialize,
            mode != Mode::Control,
            c,
        )?;
        Ok((
            WorkOrderSession::new(
                j,
                (mode != Mode::Offline).then(|| Native::new(&self.native)),
                c,
            )?,
            a,
            id,
        ))
    }
    fn acquire(&self, _: &OperationContext) -> Result<LiveObservationCapsule> {
        Ok(capsule(&self.native.borrow().queue)?.0)
    }
    fn evidence(&self, original: LiveObservationCapsule, c: &OperationContext) -> Result<Evidence> {
        let native = Rc::clone(&self.native);
        let denied = Rc::clone(&self.policy_denied);
        let pins = self.pins.clone();
        let factory: Box<dyn FnMut(&OperationContext) -> Result<Citizens>> = Box::new(move |_| {
            let (capsule, _, page) = capsule(&native.borrow().queue)?;
            Ok(Citizens {
                bridge: capsule.bridge,
                page,
            })
        });
        let authorizer: Box<
            dyn FnMut(
                &LiveObservationCapsule,
                &LiveWorldProjection,
                &OperationContext,
            ) -> Result<EvidencePolicy>,
        > = Box::new(move |capsule, projection, c| {
            if denied.get() {
                return Err(super::denied("operator source acceptance revoked"));
            }
            pins.citizens(capsule, projection, c)
        });
        CitizenWorkforceEvidenceOwner::new::<Citizens>(
            original,
            CitizenEvidenceLimits {
                page_size: 128,
                max_citizens: 128,
            },
            factory,
            authorizer,
            c,
        )
    }
    fn create_archive(
        &self,
        p: &PreparedPlan,
        c: &OperationContext,
    ) -> Result<PlanArchiveStore<Memory>> {
        PlanArchiveStore::create(self.archive.clone(), p, archive_limits(), c)
    }
    fn recover_archive(
        &self,
        r: Recovery,
        c: &OperationContext,
    ) -> Result<PlanArchiveStore<Memory>> {
        PlanArchiveStore::open(self.archive.clone(), r.seal, archive_limits(), c)
    }
    fn history(
        &self,
        j: Digest32,
        r: &SemanticWorkOrderReview,
        initialize: bool,
        read_only: bool,
        c: &OperationContext,
    ) -> Result<GoalHistoryStore<Memory>> {
        GoalHistoryStore::open(self.history.clone(), j, r, initialize, read_only, c)
    }
    fn progress(&self, _: &OperationContext) -> Result<ProgressFixture> {
        let q = self.native.borrow().queue.clone();
        let id = q.next_order_id() - 1;
        let seq = self.sequence.get();
        self.sequence.set(seq + 2);
        let remaining = self.remaining.get();
        Ok(native_progress(
            vec![
                progress(
                    &q,
                    id,
                    seq,
                    remaining,
                    1,
                    if remaining == 0 { 1 } else { 3 },
                )?,
                progress(
                    &q,
                    id,
                    seq + 1,
                    remaining,
                    1,
                    if remaining == 0 { 1 } else { 3 },
                )?,
            ],
            Rc::new(Cell::new(0)),
        ))
    }
    fn operations(&self, _: &OperationContext) -> Result<OperationsFixture> {
        Ok(OperationsFixture {
            raw: operations(&self.native.borrow().queue, false, self.jobs.get()),
            closed: Rc::new(Cell::new(0)),
        })
    }
}
fn budget() -> WorkBudget {
    WorkBudget {
        max_bytes: MAX_BYTES,
        max_entities: 4096,
        max_actions: 1,
        max_game_ticks: 0,
        max_wall_millis: 60_000,
        max_output_tokens: MAX_OUTPUT_TOKENS,
    }
}
fn start(resources: &Resources) -> Result<Runtime<Resources>> {
    Runtime::from_shell(
        resources.clone(),
        Mode::Control,
        SessionId::new(10),
        budget(),
    )
}
fn prepare(runtime: &mut Runtime<Resources>, recipe: &str) -> Result<View> {
    let c = runtime.context(false)?;
    let observed = runtime.observe(&c)?;
    let request = Request::parse(
        &json!({"expected_observation":observed.observation.expect("observation").0.to_hex(),
        "recipe":recipe,"amount":5,"deadline_tick":runtime.anchor().tick.get()+100,
        "poll_interval_ticks":10,"stable_samples":2})
        .to_string(),
    )?;
    let c = runtime.context(false)?;
    runtime.plan(request, &c)
}
#[test]
fn closed_requests_use_native_observable_goals_and_reject_unbounded_or_unknown_input() -> Result<()>
{
    for recipe in ["wooden_bed", "wooden_door", "wooden_table", "wooden_chair"] {
        let resources = Resources::new()?;
        let mut runtime = start(&resources)?;
        let view = prepare(&mut runtime, recipe)?;
        let original = view.original.as_ref().expect("original");
        assert_eq!(original.steps.len(), 1);
        assert!(!format!("{:?}", original.terminal_condition).contains("job_token"));
        assert!(
            !format!("{:?}", original.steps[0].postconditions).contains("work_order_conditions")
        );
        assert_eq!(
            view.native.as_ref().expect("prepared").state(),
            CreationState::Prepared
        );
        assert_eq!(resources.native.borrow().commits, 0);
        assert!(view.to_json().to_string().len() < 16 * 1024);
    }
    let raw = json!({"expected_observation":"11".repeat(32),"recipe":"BREW_DRINK","amount":5,"deadline_tick":100});
    assert!(Request::parse(&raw.to_string()).is_err());
    let mut raw = raw;
    raw["recipe"] = json!("wooden_bed");
    raw["authority"] = json!("all");
    assert!(Request::parse(&raw.to_string()).is_err());
    Ok(())
}
#[test]
fn actual_runtime_prepares_once_inserts_once_and_proves_original_goal_after_two_samples()
-> Result<()> {
    let resources = Resources::new()?;
    let mut runtime = start(&resources)?;
    let prepared = prepare(&mut runtime, "wooden_bed")?;
    let seal = prepared.review.as_ref().expect("review").seal();
    let c = runtime.context(false)?;
    assert!(runtime.commit(seal, false, &c).is_err());
    let c = runtime.context(false)?;
    let inserted = runtime.commit(seal, true, &c)?;
    assert!(matches!(
        inserted.creation,
        Some(NativeOrderCreation::Inserted { .. })
    ));
    assert!(inserted.goal.is_none());
    let c = runtime.context(false)?;
    runtime.commit(seal, true, &c)?;
    assert_eq!(resources.native.borrow().commits, 1);
    resources.advance(10, 0)?;
    let c = runtime.context(false)?;
    let first = runtime.wait(&c)?;
    assert!(!first.goal.as_ref().expect("sample").current_goal_proven());
    resources.advance(10, 0)?;
    let c = runtime.context(false)?;
    let done = runtime.wait(&c)?;
    assert!(done.goal.as_ref().expect("sample").current_goal_proven());
    assert!(
        done.goal
            .as_ref()
            .expect("sample")
            .physical_work
            .is_quiescent()
    );
    assert_eq!(
        done.to_json()["current_goal"]["semantic_completion_proven"],
        true
    );
    assert_eq!(resources.native.borrow().commits, 1);
    Ok(())
}
#[test]
fn exact_restart_preserves_historical_achievement_without_current_truth_or_dispatch() -> Result<()>
{
    let resources = Resources::new()?;
    let mut runtime = start(&resources)?;
    let prepared = prepare(&mut runtime, "wooden_bed")?;
    let c = runtime.context(false)?;
    runtime.commit(prepared.review.as_ref().expect("review").seal(), true, &c)?;
    resources.advance(10, 0)?;
    let c = runtime.context(false)?;
    runtime.wait(&c)?;
    resources.advance(10, 0)?;
    let c = runtime.context(false)?;
    let done = runtime.wait(&c)?;
    let handoff = resources.recover(&done);
    drop(runtime);
    let mut reopened =
        Runtime::from_shell(handoff.clone(), Mode::Offline, SessionId::new(11), budget())?;
    let c = reopened.context(false)?;
    let history = reopened.inspect(&c)?;
    assert_eq!(history.first_satisfied_anchor, done.first_satisfied_anchor);
    assert_eq!(history.original, done.original);
    assert!(history.goal.is_none());
    assert_eq!(history.to_json()["current_goal_available"], false);
    assert!(
        reopened
            .commit(prepared.review.as_ref().expect("review").seal(), true, &c)
            .is_err()
    );
    assert_eq!(resources.native.borrow().commits, 1);
    drop(reopened);
    let mut online = Runtime::from_shell(handoff, Mode::Reconcile, SessionId::new(12), budget())?;
    resources.advance(10, 0)?;
    let c = online.context(false)?;
    assert!(online.wait(&c)?.goal.is_some());
    assert_eq!(resources.native.borrow().commits, 1);
    Ok(())
}
#[test]
fn lost_insertion_reply_recovers_exact_receipt_without_another_insertion() -> Result<()> {
    let resources = Resources::new()?;
    let mut runtime = start(&resources)?;
    let prepared = prepare(&mut runtime, "wooden_bed")?;
    resources.native.borrow_mut().lose_commit = true;
    let c = runtime.context(false)?;
    let _ = runtime.commit(prepared.review.as_ref().expect("review").seal(), true, &c);
    let c = runtime.context(false)?;
    let uncertain = runtime.inspect(&c)?;
    assert!(
        uncertain
            .native
            .as_ref()
            .expect("native")
            .state()
            .unresolved()
    );
    let handoff = resources.recover(&uncertain);
    drop(runtime);
    let queries = resources.native.borrow().queries;
    let mut offline =
        Runtime::from_shell(handoff.clone(), Mode::Offline, SessionId::new(19), budget())?;
    let c = offline.context(false)?;
    assert!(
        offline
            .wait(&c)?
            .native
            .as_ref()
            .expect("retained uncertainty")
            .state()
            .unresolved()
    );
    assert_eq!(resources.native.borrow().queries, queries);
    drop(offline);
    let mut reopened = Runtime::from_shell(handoff, Mode::Reconcile, SessionId::new(20), budget())?;
    resources.advance(10, 0)?;
    let c = reopened.context(false)?;
    let recovered = reopened.wait(&c)?;
    assert!(matches!(
        recovered.creation,
        Some(NativeOrderCreation::Inserted { .. })
    ));
    assert_eq!(resources.native.borrow().commits, 1);
    Ok(())
}
#[test]
fn source_authority_budget_and_archive_mismatch_refuse_without_effects() -> Result<()> {
    let resources = Resources::new()?;
    let mut runtime = start(&resources)?;
    let prepared = prepare(&mut runtime, "wooden_bed")?;
    let seal = prepared.review.as_ref().expect("review").seal();
    resources.enabled.set(false);
    let c = runtime.context(false)?;
    assert!(runtime.commit(seal, true, &c).is_err());
    resources.enabled.set(true);
    let mut c = runtime.context(false)?;
    c.budget.max_bytes = 1024;
    assert!(runtime.commit(seal, true, &c).is_err());
    assert_eq!(resources.native.borrow().commits, 0);
    resources.policy_denied.set(true);
    let c = runtime.context(false)?;
    assert!(runtime.commit(seal, true, &c).is_err());
    assert_eq!(resources.native.borrow().commits, 0);
    let mut handoff = resources.recover(&prepared);
    handoff
        .recovery
        .as_mut()
        .expect("handoff")
        .seal
        .object_digest = Digest32::of_bytes(b"wrong");
    drop(runtime);
    assert!(Runtime::from_shell(handoff, Mode::Offline, SessionId::new(30), budget()).is_err());
    Ok(())
}
#[test]
fn changed_original_archive_stops_preparation_retry_before_native_contact() -> Result<()> {
    let resources = Resources::new()?;
    let mut runtime = start(&resources)?;
    let c = runtime.context(false)?;
    let observed = runtime.observe(&c)?;
    let raw = json!({
        "expected_observation": observed.observation.expect("observation").0.to_hex(),
        "recipe": "wooden_bed", "amount": 5,
        "deadline_tick": runtime.anchor().tick.get() + 100,
        "poll_interval_ticks": 10, "stable_samples": 2
    })
    .to_string();
    resources.native.borrow_mut().deny_read = true;
    let c = runtime.context(false)?;
    assert!(runtime.plan(Request::parse(&raw)?, &c).is_err());
    assert!(runtime.original.is_some());
    assert!(runtime.archive.is_some());
    resources.native.borrow_mut().deny_read = false;
    resources.archive.data.borrow_mut().valid = false;
    let reads = resources.native.borrow().reads;
    let prepares = resources.native.borrow().prepares;
    let c = runtime.context(false)?;
    assert!(runtime.plan(Request::parse(&raw)?, &c).is_err());
    assert_eq!(resources.native.borrow().reads, reads);
    assert_eq!(resources.native.borrow().prepares, prepares);
    assert_eq!(resources.native.borrow().commits, 0);
    Ok(())
}

#[test]
fn source_policy_is_exact_minimal_and_never_treats_independent_generations_as_equal() -> Result<()>
{
    let resources = Resources::new()?;
    let q = resources.native.borrow().queue.clone();
    let (capsule, projection, _) = capsule(&q)?;
    let c = OperationContext {
        session_id: SessionId::new(1),
        request_id: RequestId::new(1),
        anchor: projection.snapshot.anchor(),
        budget: budget(),
        grants: grants(Mode::Control, q.fortress_id(), true, false),
        cancellation_requested: false,
    };
    let policy = resources.pins.citizens(&capsule, &projection, &c)?;
    assert_eq!(policy.sources.len(), 1);
    assert_eq!(policy.all_entities, EvidenceCoverage::Unknown);
    assert_eq!(
        policy.entity_kinds.get(&EntityKind::Unit),
        Some(&EvidenceCoverage::Observed)
    );
    assert!(!policy.entity_kinds.contains_key(&EntityKind::Item));
    let mut wrong = resources.pins.clone();
    wrong.citizen_generation += 1;
    assert!(wrong.citizens(&capsule, &projection, &c).is_err());
    assert_ne!(
        resources.pins.citizen_generation,
        resources.pins.work_order_generation
    );
    Ok(())
}
#[test]
fn original_goal_and_global_job_quiescence_are_independent_and_deadline_never_renews() -> Result<()>
{
    let resources = Resources::new()?;
    let mut runtime = start(&resources)?;
    let prepared = prepare(&mut runtime, "wooden_bed")?;
    let c = runtime.context(false)?;
    runtime.commit(prepared.review.as_ref().expect("review").seal(), true, &c)?;
    resources.jobs.set(true);
    resources.advance(10, 0)?;
    let c = runtime.context(false)?;
    runtime.wait(&c)?;
    resources.advance(10, 0)?;
    let c = runtime.context(false)?;
    let view = runtime.wait(&c)?;
    assert!(view.goal.as_ref().expect("goal").current_goal_proven());
    assert!(
        !view
            .goal
            .as_ref()
            .expect("goal")
            .physical_work
            .is_quiescent()
    );
    assert_eq!(view.original, prepared.original);
    let c = runtime.context(false)?;
    assert!(runtime.observe(&c).is_err());
    assert_eq!(resources.native.borrow().commits, 1);
    Ok(())
}

#[test]
fn operator_config_is_closed_pinned_and_distinct_from_production_admission() -> Result<()> {
    let resources = Resources::new()?;
    let raw = json!({"schema":"dfmcp.semantic-work-order-development/1","directory":"/tmp/private-semantic-goal",
        "endpoint":"127.0.0.1:5000","initialize":true,"sources":{
            "kind":policy::ACCEPTANCE_KIND,"world_folder":resources.pins.world_folder,"site_id":resources.pins.site_id,
            "df_version":"0.51.11","dfhack_version":"0.51.11-r1","citizen_bridge_version":"0.1.0",
            "citizen_generation":701,"work_order_generation":resources.pins.work_order_generation,
            "progress_generation":501,"operations_generation":601}});
    assert!(Config::parse(&raw.to_string()).is_ok());
    for (field, value) in [
        ("endpoint", json!("192.0.2.1:5000")),
        ("directory", json!("/tmp/../unsafe")),
        ("schema", json!("registry-admitted")),
        ("authority", json!({"all_entities":"complete"})),
    ] {
        let mut bad = raw.clone();
        bad[field] = value;
        assert!(Config::parse(&bad.to_string()).is_err(), "{field}");
    }
    let mut bad = raw.clone();
    bad["sources"]["kind"] = json!("registry-admitted");
    assert!(Config::parse(&bad.to_string()).is_err());
    let mut bad = raw;
    bad["sources"]["progress_generation"] = json!(0);
    assert!(Config::parse(&bad.to_string()).is_err());
    Ok(())
}

#[cfg(unix)]
#[test]
fn typed_native_opener_never_creates_on_recovery_or_reinitializes_existing_custody() -> Result<()> {
    use dfmcp_adapter::work_order_control::open_private_work_orders_exact;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let directory = std::env::temp_dir().join(format!(
        "dfmcp-semantic-exact-open-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir(&directory).map_err(|_| rejected())?;
    std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))
        .map_err(|_| rejected())?;
    let path = directory.join("creation.journal");
    let resources = Resources::new()?;
    let q = resources.native.borrow().queue.clone();
    let c = OperationContext {
        session_id: SessionId::new(77),
        request_id: RequestId::new(1),
        anchor: StateAnchor {
            fortress_id: q.fortress_id(),
            cursor: ObservationCursor::ORIGIN,
            tick: GameTick(0),
            state_hash: Digest32::ZERO,
        },
        budget: budget(),
        grants: grants(Mode::Control, q.fortress_id(), true, false),
        cancellation_requested: false,
    };
    assert!(open_private_work_orders_exact(&path, &c, JournalMode::Control, false).is_err());
    assert!(!path.exists());
    let journal = open_private_work_orders_exact(&path, &c, JournalMode::Control, true)?;
    let identity = journal.summary(&c)?.journal_id;
    drop(journal);
    let original = std::fs::read(&path).map_err(|_| rejected())?;
    assert!(open_private_work_orders_exact(&path, &c, JournalMode::Control, true).is_err());
    assert_eq!(std::fs::read(&path).map_err(|_| rejected())?, original);
    let recovered = open_private_work_orders_exact(&path, &c, JournalMode::Offline, false)?;
    assert_eq!(recovered.summary(&c)?.journal_id, identity);
    drop(recovered);
    std::fs::remove_file(&path).map_err(|_| rejected())?;
    assert!(open_private_work_orders_exact(&path, &c, JournalMode::Reconcile, false).is_err());
    assert!(!path.exists());
    std::fs::remove_dir(&directory).map_err(|_| rejected())?;
    Ok(())
}
