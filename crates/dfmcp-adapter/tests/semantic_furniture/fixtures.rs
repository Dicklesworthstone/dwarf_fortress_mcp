//! Inject only storage and native I/O. Plans, projections, scoped evidence,
//! paired reviews, native codecs and both durable owners are production types.

use dfmcp_adapter::build_placement::journal::{
    BuildDispatch, BuildGuard, BuildInventory, BuildJournal, BuildMode, BuildSource, BuildStage,
    BuildState,
};
use dfmcp_adapter::build_placement::session::BuildSession;
use dfmcp_adapter::build_placement::{
    BuildBinding, BuildCapture, BuildItem, BuildKind, BuildNativeSummary, BuildPhase, BuildPlan,
    BuildPreparation, BuildRecord, BuildSelection,
};
use dfmcp_adapter::control_effect_journal::EffectJournalStorage;
use dfmcp_adapter::live_jobs_rpc::operations::map::spatial::{
    SpatialLimits, citizens::CitizenSpatialLimits,
};
use dfmcp_adapter::live_jobs_rpc::operations::paged::PagedOperationsLimits;
use dfmcp_adapter::live_operations::{LiveItem, OperationsProfile};
use dfmcp_adapter::live_spatial::citizens::{
    LiveSpatialCitizenObservation, LiveSpatialCitizenState,
};
use dfmcp_adapter::live_spatial::{LiveSpatialObservation, SpatialStateView};
use dfmcp_adapter::semantic_furniture::evidence_owner::{
    SpatialFurnitureEvidenceOwner, SpatialFurnitureEvidenceSource,
};
use dfmcp_adapter::semantic_furniture::store::AssociationStore;
use dfmcp_adapter::semantic_furniture::{SemanticFurnitureReview, SemanticFurnitureSession};
use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, DfmcpError, Digest32, ErrorCode, IntentId,
    MapCoord, MapCuboid, OperationContext, RequestId, Result, RiskTier, SessionId, StepId,
    WorkBudget,
};
use dfmcp_intent::{
    Action, BuildingKind, MaterialSelector, PlanStep, PreparedPlan, derive_step_idempotency_key,
    effects,
};
use dfmcp_world::{
    EvidenceCoverage, EvidencePolicy, EvidenceSource, FactSource, Predicate, WorldSnapshot,
};
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, Cursor, Read, Seek, SeekFrom, Write};
use std::net::SocketAddr;
use std::rc::Rc;
use std::time::Duration;

pub const SPATIAL_GENERATION: u64 = 801;
pub type Session = SemanticFurnitureSession<Memory, Memory, Native>;
type EvidenceFactory = Box<dyn FnMut(&OperationContext) -> Result<SpatialSource>>;
type EvidenceAuthorizer = Box<
    dyn FnMut(
        &LiveSpatialCitizenObservation,
        &LiveSpatialCitizenState,
        &OperationContext,
    ) -> Result<EvidencePolicy>,
>;
pub type Evidence = SpatialFurnitureEvidenceOwner<EvidenceFactory, EvidenceAuthorizer>;

pub fn invalid(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::InternalInvariantViolation, message)
}

fn hex(raw: &str) -> Result<Vec<u8>> {
    if !raw.len().is_multiple_of(2) || !raw.is_ascii() {
        return Err(invalid("fixture hex extent"));
    }
    raw.as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| {
            let value = std::str::from_utf8(pair).map_err(|_| invalid("fixture hex text"))?;
            u8::from_str_radix(value, 16).map_err(|_| invalid("fixture hex byte"))
        })
        .collect()
}

pub fn golden(name: &str) -> Result<Vec<u8>> {
    let source =
        include_str!("../../../../bridge/common/tests/fixtures/build_placement_v1_19.json");
    let marker = format!("\"{name}\": \"");
    let start = source.find(&marker).ok_or_else(|| invalid("fixture key"))? + marker.len();
    let value = source[start..]
        .split('"')
        .next()
        .ok_or_else(|| invalid("fixture value"))?;
    hex(value)
}

pub fn field16(out: &mut Vec<u8>, raw: &[u8]) -> Result<()> {
    let length = u16::try_from(raw.len()).map_err(|_| invalid("fixture field extent"))?;
    out.extend_from_slice(&length.to_be_bytes());
    out.extend_from_slice(raw);
    Ok(())
}

fn field32(out: &mut Vec<u8>, raw: &[u8]) -> Result<()> {
    let length = u32::try_from(raw.len()).map_err(|_| invalid("fixture payload extent"))?;
    out.extend_from_slice(&length.to_be_bytes());
    out.extend_from_slice(raw);
    Ok(())
}

/// Independent receipt encoder for the original plan key. Native placement
/// readback comes from the unchanged golden fixture, never a fabricated proof.
pub fn native_record(plan: &BuildPlan, phase: BuildPhase) -> Result<BuildRecord> {
    let mut bytes = b"DFMBR019".to_vec();
    field16(&mut bytes, plan.key().as_bytes())?;
    field16(&mut bytes, plan.before().canonical_bytes())?;
    bytes.extend_from_slice(plan.digest().as_bytes());
    bytes.extend_from_slice(plan.token());
    match phase {
        BuildPhase::Prepared => bytes.extend_from_slice(&[0, 0, 0, 0]),
        BuildPhase::Indeterminate => bytes.extend_from_slice(&[1, 5, 1, 0]),
        BuildPhase::Placed => {
            bytes.extend_from_slice(&[2, 0, 1, 1]);
            let receipt = BuildRecord::decode(&golden("placed")?)?;
            let insertion = receipt
                .insertion()
                .ok_or_else(|| invalid("golden placement insertion"))?;
            field16(
                &mut bytes,
                plan.before().expected_after()?.canonical_bytes(),
            )?;
            field16(&mut bytes, insertion.canonical_bytes())?;
        }
        BuildPhase::Cancelled => bytes.extend_from_slice(&[4, 4, 0, 0]),
        BuildPhase::Refused => bytes.extend_from_slice(&[3, 1, 0, 0]),
    }
    let mut domain = b"dfmcp-build-receipt/1\0".to_vec();
    domain.extend_from_slice(&bytes);
    bytes.extend_from_slice(Digest32::of_bytes(&domain).as_bytes());
    BuildRecord::decode(&bytes)
}

/// Reuse the versioned spatial fixture's map encoding, while supplying the
/// exact selected native item and independent spatial source incarnation.
pub fn spatial_observation(
    capture: &BuildCapture,
    generation: u64,
) -> Result<LiveSpatialCitizenObservation> {
    let base = LiveSpatialObservation::decode_payload(
        &hex(include_str!("../fixtures/spatial_v1_6.hex").trim())?,
        SPATIAL_GENERATION,
        "df".to_owned(),
        "dfhack".to_owned(),
    )?;
    let mut operations = base.operations().clone();
    let mut terrain = base.terrain().clone();
    terrain.bridge_generation = generation;
    terrain.year = (capture.tick() / 403_200) as u32;
    terrain.year_tick = (capture.tick() % 403_200) as u32;
    terrain.site_id = capture.site();
    terrain.world_folder = capture.folder().to_owned();
    terrain.paused = capture.paused();
    terrain.map_dimensions = capture.dimensions();
    terrain.map.region.origin = capture.selection().target();
    operations.jobs.jobs.clear();
    operations.buildings.clear();
    operations.attachments.clear();
    operations.items.clear();
    operations.next_building_id = capture.next_building_id();
    operations.next_item_id = capture.selection().item_id() + 1;
    operations.jobs.next_job_id = capture.next_job_id();
    let BuildItem::Visible(item) = capture.item() else {
        return Err(invalid("golden furniture item must be visible"));
    };
    let kind = match capture.selection().kind() {
        BuildKind::Bed => "Bed",
        BuildKind::Chair => "Chair",
        BuildKind::Table => "Table",
    };
    operations.items.push(LiveItem {
        native_id: capture.selection().item_id(),
        item_type: item.native_type() as i32,
        type_key: kind.to_owned(),
        subtype: item.subtype(),
        material_type: item.material(),
        material_index: item.material_index(),
        stack_size: 1,
        raw_position: MapCoord::new(
            item.position()[0] as i32,
            item.position()[1] as i32,
            item.position()[2] as i32,
        ),
        flags: 1 << 6,
        container_native_id: None,
        holder_building_native_id: None,
    });
    operations.jobs.bridge_generation = generation;
    operations.jobs.year = terrain.year;
    operations.jobs.year_tick = terrain.year_tick;
    operations.jobs.site_id = terrain.site_id as i32;
    operations
        .jobs
        .world_folder
        .clone_from(&terrain.world_folder);
    operations.jobs.paused = terrain.paused;
    let mut spatial = b"DFMS1600".to_vec();
    field32(
        &mut spatial,
        &operations.encode_profile(OperationsProfile::PagedV1_4)?,
    )?;
    field32(&mut spatial, &terrain.encode_payload()?)?;
    let mut citizens = b"DFMC1800".to_vec();
    citizens.extend_from_slice(&1u32.to_be_bytes());
    citizens.extend_from_slice(&42u32.to_be_bytes());
    field16(&mut citizens, b"Urist")?;
    field16(&mut citizens, b"dwarf")?;
    citizens.extend_from_slice(&3i32.to_be_bytes());
    for position in [1i32, 1, 5] {
        citizens.extend_from_slice(&position.to_be_bytes());
    }
    citizens.extend_from_slice(&0b1_0101_1111u16.to_be_bytes());
    citizens.extend_from_slice(&0i32.to_be_bytes());
    citizens.extend_from_slice(&[1, 1]);
    citizens.extend_from_slice(&0u16.to_be_bytes());
    let mut payload = b"DFMS1800".to_vec();
    field32(&mut payload, &spatial)?;
    field32(&mut payload, &citizens)?;
    LiveSpatialCitizenObservation::decode_payload(
        &payload,
        generation,
        terrain.df_version,
        terrain.dfhack_version,
    )
}

pub fn policy(snapshot: &WorldSnapshot) -> EvidencePolicy {
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

pub fn original_plan(snapshot: &WorldSnapshot, capture: &BuildCapture) -> Result<PreparedPlan> {
    let target = capture.selection().target();
    let location = MapCoord::new(target[0] as i32, target[1] as i32, target[2] as i32);
    let action = Action::Build {
        kind: BuildingKind::Furniture("bed".to_owned()),
        location,
        footprint: MapCuboid {
            min: location,
            max: location,
        },
        material: MaterialSelector::default(),
    }
    .normalized();
    let intent_id = IntentId::new(741);
    let step_id = StepId::new(0);
    let key = derive_step_idempotency_key(intent_id, snapshot.anchor(), step_id, &action);
    let postconditions = effects::default_postconditions(&action, &key, snapshot.fortress_id);
    let obligation =
        effects::default_obligation(&action, &key, snapshot.fortress_id, snapshot.tick)?;
    let deadline = obligation
        .as_ref()
        .ok_or_else(|| invalid("Build must retain its temporal obligation"))?
        .deadline_tick;
    let terminal = Predicate::All(postconditions.clone()).normalized();
    let plan = PreparedPlan::builder(
        intent_id,
        snapshot.anchor(),
        "Build the original bed",
        terminal,
    )
    .max_risk(action.risk())
    .required_capabilities(BTreeSet::from([action.capability()]))
    .expires_at_tick(deadline)
    .steps(vec![PlanStep {
        id: step_id,
        idempotency_key: key,
        required_capability: action.capability(),
        risk: action.risk(),
        action,
        preconditions: vec![Predicate::Paused(true)],
        postconditions,
        compensation: None,
        obligation,
        depends_on: Vec::new(),
    }])
    .build();
    plan.validate_structure()?;
    Ok(plan)
}

pub fn reseal(original: &PreparedPlan, change: impl FnOnce(&mut PreparedPlan)) -> PreparedPlan {
    let mut changed = original.clone();
    change(&mut changed);
    PreparedPlan::builder(
        changed.intent_id,
        changed.anchor,
        changed.summary,
        changed.terminal_condition,
    )
    .max_risk(changed.max_risk)
    .required_capabilities(changed.required_capabilities)
    .requires_checkpoint(changed.requires_checkpoint)
    .expires_at_tick(changed.expires_at_tick)
    .steps(changed.steps)
    .build()
}

pub fn context(snapshot: &WorldSnapshot) -> OperationContext {
    OperationContext {
        session_id: SessionId::new(741),
        request_id: RequestId::new(1),
        anchor: snapshot.anchor(),
        budget: WorkBudget {
            max_wall_millis: 60_000,
            max_bytes: 512 * 1024 * 1024,
            max_entities: 4096,
            max_actions: 1,
            max_output_tokens: 8192,
            max_game_ticks: 0,
        },
        grants: [
            Capability::Query,
            Capability::Observe,
            Capability::Plan,
            Capability::Construct,
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

pub fn query_context(context: &OperationContext) -> OperationContext {
    let mut query = context.clone();
    query
        .grants
        .retain(|grant| grant.capability == Capability::Query);
    query
}

#[derive(Default)]
struct Disk {
    cursor: Cursor<Vec<u8>>,
    durable: Vec<u8>,
    syncs: usize,
    fail_sync: bool,
    lost_identity: bool,
    on_sync: Option<Box<dyn FnOnce()>>,
}

#[derive(Clone, Default)]
pub struct Memory(Rc<RefCell<Disk>>);

impl Memory {
    pub fn from_bytes(bytes: Vec<u8>) -> Self {
        Self(Rc::new(RefCell::new(Disk {
            cursor: Cursor::new(bytes.clone()),
            durable: bytes,
            ..Disk::default()
        })))
    }
    pub fn bytes(&self) -> Vec<u8> {
        self.0.borrow().cursor.get_ref().clone()
    }
    pub fn durable(&self) -> Vec<u8> {
        self.0.borrow().durable.clone()
    }
    pub fn syncs(&self) -> usize {
        self.0.borrow().syncs
    }
    pub fn lose_identity(&self) {
        self.0.borrow_mut().lost_identity = true;
    }
    pub fn fail_next_sync(&self) {
        self.0.borrow_mut().fail_sync = true;
    }
    pub fn after_next_sync(&self, callback: impl FnOnce() + 'static) {
        self.0.borrow_mut().on_sync = Some(Box::new(callback));
    }
    pub fn corrupt(&self) {
        if let Some(first) = self.0.borrow_mut().cursor.get_mut().first_mut() {
            *first ^= 1;
        }
    }
}

impl Read for Memory {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        self.0.borrow_mut().cursor.read(bytes)
    }
}
impl Write for Memory {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.borrow_mut().cursor.write(bytes)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
impl Seek for Memory {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.0.borrow_mut().cursor.seek(position)
    }
}
impl EffectJournalStorage for Memory {
    fn sync(&mut self) -> io::Result<()> {
        let callback = {
            let mut disk = self.0.borrow_mut();
            if disk.fail_sync {
                disk.fail_sync = false;
                return Err(io::Error::other("injected sync failure"));
            }
            disk.syncs += 1;
            disk.durable = disk.cursor.get_ref().clone();
            disk.on_sync.take()
        };
        if let Some(callback) = callback {
            callback();
        }
        Ok(())
    }
    fn truncate(&mut self, _: u64) -> io::Result<()> {
        Err(io::Error::other("repair forbidden"))
    }
    fn validate_identity(&self) -> io::Result<()> {
        if self.0.borrow().lost_identity {
            Err(io::Error::other("injected custody loss"))
        } else {
            Ok(())
        }
    }
}

pub struct SourceState {
    pub capture: BuildCapture,
    pub summary: BuildNativeSummary,
    pub records: BTreeMap<String, BuildRecord>,
    pub connections: usize,
    pub observes: usize,
    pub prepares: usize,
    pub commits: usize,
    pub queries: usize,
    pub cancels: usize,
    pub fenced: usize,
    pub lost_commit_reply: bool,
    pub forget_query: bool,
}

pub struct Native {
    binding: BuildBinding,
    disk: Memory,
    state: Rc<RefCell<SourceState>>,
}

fn native_authority(context: &OperationContext, capability: Capability) -> Result<()> {
    let risk = if matches!(capability, Capability::Query | Capability::Observe) {
        RiskTier::ReadOnly
    } else {
        RiskTier::Guarded
    };
    context.authorize(capability, risk, &[], None)
}

pub fn factory(
    state: Rc<RefCell<SourceState>>,
    disk: Memory,
) -> impl FnOnce(&BuildBinding, &OperationContext, Duration) -> Result<Native> {
    move |expected, context, timeout| {
        native_authority(context, Capability::Query)?;
        if timeout.is_zero() {
            return Err(invalid("source needs remaining time"));
        }
        let binding = BuildBinding::new(
            expected.endpoint(),
            expected.df_version(),
            expected.dfhack_version(),
            &state.borrow().capture,
        )?;
        state.borrow_mut().connections += 1;
        Ok(Native {
            binding,
            disk,
            state,
        })
    }
}

pub fn durable_inventory(
    disk: &Memory,
    binding: &BuildBinding,
    context: &OperationContext,
) -> Result<BuildInventory> {
    let mut journal = BuildJournal::open(
        Memory::from_bytes(disk.durable()),
        context,
        BuildMode::Offline,
        Some(binding.clone()),
        None,
    )?;
    journal.inventory(context)
}

impl BuildSource for Native {
    fn binding(&self) -> &BuildBinding {
        &self.binding
    }
    fn native_summary(&self) -> BuildNativeSummary {
        self.state.borrow().summary
    }
    fn fence(&mut self) {
        self.state.borrow_mut().fenced += 1;
    }
    fn observe(
        &mut self,
        selection: BuildSelection,
        context: &OperationContext,
        _: Duration,
    ) -> Result<BuildCapture> {
        native_authority(context, Capability::Observe)?;
        let mut state = self.state.borrow_mut();
        state.observes += 1;
        if selection != state.capture.selection() {
            return Err(invalid("source selection changed"));
        }
        Ok(state.capture.clone())
    }
    fn prepare(
        &mut self,
        plan: &BuildPlan,
        context: &OperationContext,
        _: Duration,
    ) -> Result<BuildPreparation> {
        native_authority(context, Capability::Plan)?;
        let inventory = durable_inventory(&self.disk, &self.binding, context)?;
        let entry = inventory
            .entry(plan.key())
            .ok_or_else(|| invalid("native prepare preceded durable intent"))?;
        assert_eq!(entry.state(), BuildState::Intent);
        assert_eq!(entry.plan(), plan);
        let record = native_record(plan, BuildPhase::Prepared)?;
        let mut state = self.state.borrow_mut();
        state.prepares += 1;
        state.records.insert(plan.key().to_owned(), record.clone());
        state.summary = BuildNativeSummary::new(true, state.records.len() as u16)?;
        BuildPreparation::new(record, false)
    }
    fn commit(
        &mut self,
        dispatch: BuildDispatch<'_>,
        context: &OperationContext,
        _: Duration,
    ) -> Result<BuildRecord> {
        native_authority(context, Capability::Construct)?;
        let plan = dispatch.plan();
        let inventory = durable_inventory(&self.disk, &self.binding, context)?;
        let entry = inventory
            .entry(plan.key())
            .ok_or_else(|| invalid("native writer preceded durable dispatch marker"))?;
        assert_eq!(entry.state(), BuildState::DispatchStarted);
        assert_eq!(entry.plan(), plan);
        assert_eq!(inventory.head, dispatch.journal_head());
        let record = native_record(plan, BuildPhase::Placed)?;
        let mut state = self.state.borrow_mut();
        state.commits += 1;
        state.records.insert(plan.key().to_owned(), record.clone());
        state.summary = BuildNativeSummary::new(false, state.records.len() as u16)?;
        if state.lost_commit_reply {
            Err(DfmcpError::new(
                ErrorCode::AdapterUnavailable,
                "placement happened; acknowledgement lost",
            ))
        } else {
            Ok(record)
        }
    }
    fn query(
        &mut self,
        plan: &BuildPlan,
        context: &OperationContext,
        _: Duration,
    ) -> Result<Option<BuildRecord>> {
        native_authority(context, Capability::Query)?;
        let mut state = self.state.borrow_mut();
        state.queries += 1;
        if state.forget_query {
            return Ok(None);
        }
        if let Some(record) = state.records.get(plan.key()) {
            record.verify_plan(plan)?;
        }
        Ok(state.records.get(plan.key()).cloned())
    }
    fn cancel(
        &mut self,
        plan: &BuildPlan,
        context: &OperationContext,
        _: Duration,
    ) -> Result<BuildRecord> {
        native_authority(context, Capability::Query)?;
        let record = native_record(plan, BuildPhase::Cancelled)?;
        let mut state = self.state.borrow_mut();
        state.cancels += 1;
        state.records.insert(plan.key().to_owned(), record.clone());
        state.summary = BuildNativeSummary::new(false, state.records.len() as u16)?;
        Ok(record)
    }
}

pub struct EvidenceState {
    pub observation: LiveSpatialCitizenObservation,
    pub permitted: bool,
    pub opened: usize,
    pub active: usize,
    pub reads: usize,
    pub authorizations: usize,
}

pub struct SpatialSource(Rc<RefCell<EvidenceState>>);
impl SpatialFurnitureEvidenceSource for SpatialSource {
    fn read_observation(
        &mut self,
        context: &OperationContext,
    ) -> Result<LiveSpatialCitizenObservation> {
        native_authority(context, Capability::Observe)?;
        let mut state = self.0.borrow_mut();
        state.reads += 1;
        Ok(state.observation.clone())
    }
}
impl Drop for SpatialSource {
    fn drop(&mut self) {
        let mut state = self.0.borrow_mut();
        state.active = state.active.saturating_sub(1);
    }
}

pub fn evidence_owner(
    original: LiveSpatialCitizenState,
    shared: Rc<RefCell<EvidenceState>>,
    context: &OperationContext,
) -> Result<Evidence> {
    let region = original
        .observation_full()
        .ok_or_else(|| invalid("original spatial capture"))?
        .spatial()
        .terrain()
        .map
        .region;
    let limits = CitizenSpatialLimits {
        spatial: SpatialLimits {
            operations: PagedOperationsLimits {
                jobs: 16,
                buildings: 16,
                items: 128,
                payload_bytes: 64 * 1024,
                page_bytes: 64 * 1024,
            },
            region,
        },
        citizens: 16,
    };
    let source = shared.clone();
    let authorizer = shared;
    let factory: EvidenceFactory = Box::new(move |context| {
        native_authority(context, Capability::Observe)?;
        let mut state = source.borrow_mut();
        state.opened += 1;
        state.active += 1;
        drop(state);
        Ok(SpatialSource(source.clone()))
    });
    let authorizer: EvidenceAuthorizer = Box::new(move |_, state, _| {
        let mut shared = authorizer.borrow_mut();
        if shared.active != 0 {
            return Err(invalid("source must be released before authority issuance"));
        }
        shared.authorizations += 1;
        if !shared.permitted {
            return Err(DfmcpError::new(
                ErrorCode::CapabilityDenied,
                "spatial policy revoked",
            ));
        }
        Ok(policy(
            state
                .snapshot()
                .ok_or_else(|| invalid("canonical snapshot"))?,
        ))
    });
    SpatialFurnitureEvidenceOwner::new(original, limits, factory, authorizer, context)
}

pub struct Guard {
    pub permitted: Rc<Cell<bool>>,
    pub calls: Rc<RefCell<Vec<BuildStage>>>,
    pub dispatches: Rc<Cell<usize>>,
}
impl Default for Guard {
    fn default() -> Self {
        Self {
            permitted: Rc::new(Cell::new(true)),
            calls: Rc::default(),
            dispatches: Rc::default(),
        }
    }
}
impl BuildGuard for Guard {
    fn check(
        &mut self,
        stage: BuildStage,
        _: &BuildBinding,
        _: Option<&BuildPlan>,
        _: BuildSelection,
        _: &OperationContext,
    ) -> Result<()> {
        self.calls.borrow_mut().push(stage);
        if stage == BuildStage::Commit && !self.permitted.get() {
            return Err(DfmcpError::new(
                ErrorCode::CapabilityDenied,
                "current construction policy revoked",
            ));
        }
        Ok(())
    }
    fn before_dispatch(
        &mut self,
        binding: &BuildBinding,
        plan: &BuildPlan,
        context: &OperationContext,
    ) -> Result<()> {
        self.dispatches.set(self.dispatches.get() + 1);
        self.check(
            BuildStage::Commit,
            binding,
            Some(plan),
            plan.before().selection(),
            context,
        )
    }
}

pub struct Fixture {
    pub session: Session,
    pub original: PreparedPlan,
    pub context: OperationContext,
    pub capture: BuildCapture,
    pub binding: BuildBinding,
    pub native_disk: Memory,
    pub associations: Memory,
    pub native: Rc<RefCell<SourceState>>,
    pub evidence: Evidence,
    pub evidence_state: Rc<RefCell<EvidenceState>>,
    pub original_state: LiveSpatialCitizenState,
    pub guard: Guard,
}

impl Fixture {
    pub fn new() -> Result<Self> {
        let capture = BuildCapture::decode(&golden("capture")?)?;
        let observation = spatial_observation(&capture, SPATIAL_GENERATION)?;
        let mut original_state = LiveSpatialCitizenState::default();
        original_state.publish(observation.clone())?;
        let snapshot = original_state
            .snapshot()
            .ok_or_else(|| invalid("fixture canonical snapshot"))?;
        let context = context(snapshot);
        let original = original_plan(snapshot, &capture)?;
        let binding = BuildBinding::new(
            SocketAddr::from(([127, 0, 0, 1], 5000)),
            "df",
            "dfhack",
            &capture,
        )?;
        let native_disk = Memory::default();
        let associations = Memory::default();
        let mut journal = BuildJournal::open(
            native_disk.clone(),
            &context,
            BuildMode::Control,
            Some(binding.clone()),
            Some([27; 32]),
        )?;
        let id = journal.inventory(&context)?.journal_id;
        let native_session = BuildSession::new(journal, &context)?;
        let store = AssociationStore::open(associations.clone(), id, true, false, &context)?;
        let session = Session::new(native_session, store, &context)?;
        let native = Rc::new(RefCell::new(SourceState {
            capture: capture.clone(),
            summary: BuildNativeSummary::new(false, 0)?,
            records: BTreeMap::new(),
            connections: 0,
            observes: 0,
            prepares: 0,
            commits: 0,
            queries: 0,
            cancels: 0,
            fenced: 0,
            lost_commit_reply: false,
            forget_query: false,
        }));
        let evidence_state = Rc::new(RefCell::new(EvidenceState {
            observation,
            permitted: true,
            opened: 0,
            active: 0,
            reads: 0,
            authorizations: 0,
        }));
        let evidence = evidence_owner(original_state.clone(), evidence_state.clone(), &context)?;
        Ok(Self {
            session,
            original,
            context,
            capture,
            binding,
            native_disk,
            associations,
            native,
            evidence,
            evidence_state,
            original_state,
            guard: Guard::default(),
        })
    }

    pub fn observe(&mut self) -> Result<SemanticFurnitureReview> {
        self.session.observe(
            self.original.clone(),
            self.capture.selection(),
            &mut self.evidence,
            &self.context,
            factory(self.native.clone(), self.native_disk.clone()),
            &mut self.guard,
        )
    }
    pub fn prepared(&mut self) -> Result<SemanticFurnitureReview> {
        let review = self.observe()?;
        let result = self.session.prepare(
            review.seal(),
            &mut self.evidence,
            &self.context,
            &mut self.guard,
        )?;
        assert_eq!(result.native_record().state(), BuildState::Prepared);
        Ok(review)
    }
}

pub fn reopen(
    native_disk: Memory,
    associations: Memory,
    binding: &BuildBinding,
    mode: BuildMode,
    read_only: bool,
    context: &OperationContext,
) -> Result<Session> {
    let mut journal = BuildJournal::open(native_disk, context, mode, Some(binding.clone()), None)?;
    let id = journal.inventory(context)?.journal_id;
    let native = BuildSession::new(journal, context)?;
    let store = AssociationStore::open(associations, id, false, read_only, context)?;
    Session::new(native, store, context)
}
