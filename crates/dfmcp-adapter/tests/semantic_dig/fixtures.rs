//! Inject native I/O and storage only. Original plans, canonical terrain,
//! source-scoped predicates, effect codecs and durable owners are production types.
use dfmcp_adapter::control_effect_journal::EffectJournalStorage;
use dfmcp_adapter::dig_designation::journal::session::{DigSession, DigSessionGuard};
use dfmcp_adapter::dig_designation::journal::{
    DigBinding, DigGuard, DigJournal, DigMode, DigRecord, DigStage, DigState,
};
use dfmcp_adapter::dig_designation::rpc::{DigManifest, DigPreparation, DigSource};
use dfmcp_adapter::dig_designation::{
    DigEffect, DigObservation, DigPhase, DigPlan, DigRegion, DigTile,
};
use dfmcp_adapter::live_operations::{LiveOperationsObservation, OperationsProfile};
use dfmcp_adapter::live_routing::LiveRoutingEvidence;
use dfmcp_adapter::live_spatial::LiveSpatialObservation;
use dfmcp_adapter::live_spatial::citizens::LiveSpatialCitizenObservation;
use dfmcp_adapter::semantic_dig::terrain::DigTerrainProjection;
use dfmcp_adapter::semantic_dig::{
    AssociationStore, DigEvidenceOwner, SemanticDigReview, SemanticDigSession,
};
use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, DfmcpError, Digest32, ErrorCode, GameTick,
    IntentId, MapCoord, MapCuboid, ObservationCursor, OperationContext, RequestId, Result,
    RiskTier, SessionId, StateAnchor, StepId, WorkBudget,
};
use dfmcp_intent::{Action, PlanStep, PreparedPlan, derive_step_idempotency_key, effects};
use dfmcp_world::map_region::{Cell as MapCell, Region, Shape, Tile};
use dfmcp_world::{EvidencePolicy, Predicate, WorldSnapshot};
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, Cursor, Read, Seek, SeekFrom, Write};
use std::net::SocketAddr;
use std::rc::Rc;
pub const SPATIAL_GENERATION: u64 = 801;
pub type Session = SemanticDigSession<Memory, Memory, Native>;
pub fn invalid(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::InternalInvariantViolation, message)
}
pub fn hex(raw: &str) -> Result<Vec<u8>> {
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
    let raw = match name {
        "capture" | "observation" => {
            include_str!("../../../../tests/native/dig_designation/vectors/observation.hex")
        }
        "prepared" => include_str!("../../../../tests/native/dig_designation/vectors/prepared.hex"),
        "designated" => {
            include_str!("../../../../tests/native/dig_designation/vectors/designated.hex")
        }
        "cancelled" => {
            include_str!("../../../../tests/native/dig_designation/vectors/cancelled.hex")
        }
        _ => return Err(invalid("fixture key")),
    };
    hex(raw.trim())
}
pub fn capture() -> Result<DigObservation> {
    DigObservation::decode(&golden("capture")?)
}
pub fn field16(out: &mut Vec<u8>, raw: &[u8]) -> Result<()> {
    let n = u16::try_from(raw.len()).map_err(|_| invalid("fixture field extent"))?;
    out.extend_from_slice(&n.to_be_bytes());
    out.extend_from_slice(raw);
    Ok(())
}
fn field32(out: &mut Vec<u8>, raw: &[u8]) -> Result<()> {
    let n = u32::try_from(raw.len()).map_err(|_| invalid("fixture field extent"))?;
    out.extend_from_slice(&n.to_be_bytes());
    out.extend_from_slice(raw);
    Ok(())
}
/// Re-key exact native golden effects without changing their native readback.
pub fn native_record(plan: &DigPlan, phase: DigPhase) -> Result<DigEffect> {
    let name = match phase {
        DigPhase::Prepared | DigPhase::Unknown => "prepared",
        DigPhase::Designated => "designated",
        DigPhase::Refused => "cancelled",
    };
    let mut raw = golden(name)?;
    raw[117..133].copy_from_slice(plan.token());
    raw.truncate(204);
    field16(&mut raw, plan.key().as_bytes())?;
    if phase == DigPhase::Unknown {
        raw[133] = 1;
    }
    if phase.terminal() {
        let mut receipt = b"dfmcp-dig-designation-receipt/1\0".to_vec();
        receipt.extend_from_slice(&raw[8..16]);
        receipt.extend_from_slice(&raw[204..]);
        receipt.extend_from_slice(&raw[85..133]);
        receipt.extend_from_slice(&raw[133..172]);
        raw[172..204].copy_from_slice(Digest32::of_bytes(&receipt).as_bytes());
    }
    DigEffect::decode(&raw, plan)
}
pub fn compose(
    operations: &LiveOperationsObservation,
    terrain: &dfmcp_adapter::live_map::LiveMapObservation,
) -> Result<LiveSpatialCitizenObservation> {
    let mut spatial = b"DFMS1600".to_vec();
    field32(
        &mut spatial,
        &operations.encode_profile(OperationsProfile::PagedV1_4)?,
    )?;
    field32(&mut spatial, &terrain.encode_payload()?)?;
    let citizens = [b"DFMC1800".as_slice(), &0u32.to_be_bytes()].concat();
    let mut payload = b"DFMS1800".to_vec();
    field32(&mut payload, &spatial)?;
    field32(&mut payload, &citizens)?;
    LiveSpatialCitizenObservation::decode_payload(
        &payload,
        terrain.bridge_generation,
        terrain.df_version.clone(),
        terrain.dfhack_version.clone(),
    )
}
pub fn spatial_observation(
    capture: &DigObservation,
    generation: u64,
) -> Result<LiveSpatialCitizenObservation> {
    let base = LiveSpatialObservation::decode_payload(
        &hex(include_str!("../fixtures/spatial_v1_6.hex").trim())?,
        SPATIAL_GENERATION,
        "df".to_owned(),
        "dfhack".to_owned(),
    )?;
    let mut terrain = base.terrain().clone();
    let mut operations = base.operations().clone();
    terrain.bridge_generation = generation;
    terrain.year = (capture.tick() / 403_200) as u32;
    terrain.year_tick = (capture.tick() % 403_200) as u32;
    terrain.site_id = capture.site();
    terrain.world_folder = capture.folder().to_owned();
    terrain.paused = capture.paused();
    terrain.map_dimensions = capture.dimensions();
    let [x, y, z, w, h] = capture.region().coordinates();
    terrain.map.region = Region {
        origin: [x - 1, y - 1, z - 1],
        size: [w + 2, h + 2, 3],
    };
    terrain.map.cells = capture
        .tiles()
        .map(|(_, cell)| match cell {
            DigTile::Missing => MapCell::Unallocated,
            DigTile::Hidden => MapCell::Hidden,
            DigTile::Visible(t) => MapCell::Visible(Tile {
                native_tiletype: t.tiletype(),
                shape: if t.flags() & 1 != 0 {
                    Shape::Wall
                } else {
                    Shape::Floor
                },
                liquid_depth: 0,
                magma: false,
                traffic: 0,
                dig_designation: t.dig(),
                building_occupancy: 0,
                unit_occupancy: 0,
                walkable_region: 1,
                temperature_1: t.temperatures()[0],
                temperature_2: t.temperatures()[1],
            }),
        })
        .collect();
    operations.jobs.jobs.clear();
    operations.buildings.clear();
    operations.attachments.clear();
    operations.items.clear();
    operations.jobs.bridge_generation = generation;
    operations.jobs.year = terrain.year;
    operations.jobs.year_tick = terrain.year_tick;
    operations.jobs.site_id = terrain.site_id as i32;
    operations.jobs.world_folder = terrain.world_folder.clone();
    operations.jobs.paused = terrain.paused;
    compose(&operations, &terrain)
}
pub fn goal_observation(
    capture: &DigObservation,
    tick: u64,
    change: impl FnOnce(&mut dfmcp_adapter::live_map::LiveMapObservation),
) -> Result<LiveSpatialCitizenObservation> {
    let raw = spatial_observation(capture, SPATIAL_GENERATION)?;
    let mut terrain = raw.spatial().terrain().clone();
    let mut operations = raw.spatial().operations().clone();
    terrain.year = (tick / 403_200) as u32;
    terrain.year_tick = (tick % 403_200) as u32;
    change(&mut terrain);
    operations.jobs.year = terrain.year;
    operations.jobs.year_tick = terrain.year_tick;
    operations.jobs.paused = terrain.paused;
    operations.jobs.bridge_generation = terrain.bridge_generation;
    operations.jobs.world_folder = terrain.world_folder.clone();
    operations.jobs.site_id = terrain.site_id as i32;
    compose(&operations, &terrain)
}
pub fn policy(p: &DigTerrainProjection) -> EvidencePolicy {
    let mut policy = EvidencePolicy::at(p.anchor());
    policy.paused = true;
    policy.terrain_regions = p.terrain_regions().to_vec();
    policy
}
pub fn original_plan(snapshot: &WorldSnapshot, capture: &DigObservation) -> Result<PreparedPlan> {
    let [x, y, z, w, h] = capture.region().coordinates();
    let area = MapCuboid::new(
        MapCoord::new(x as i32, y as i32, z as i32),
        MapCoord::new((x + w - 1) as i32, (y + h - 1) as i32, z as i32),
    )?;
    let action = Action::DesignateDig {
        area,
        mode: dfmcp_intent::DigMode::Mine,
    };
    let intent_id = IntentId::new(741);
    let step_id = StepId::ZERO;
    let key = derive_step_idempotency_key(intent_id, snapshot.anchor(), step_id, &action);
    let postconditions = effects::default_postconditions(&action, &key, snapshot.fortress_id);
    let obligation =
        effects::default_obligation(&action, &key, snapshot.fortress_id, snapshot.tick)?;
    let deadline = obligation
        .as_ref()
        .ok_or_else(|| invalid("original excavation obligation"))?
        .deadline_tick;
    let plan = PreparedPlan::builder(
        intent_id,
        snapshot.anchor(),
        "Excavate the original Mine rectangle",
        Predicate::All(postconditions.clone()).normalized(),
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
    anchor_context(snapshot.anchor())
}
pub fn anchor_context(anchor: StateAnchor) -> OperationContext {
    OperationContext {
        session_id: SessionId::new(741),
        request_id: RequestId::new(1),
        anchor,
        budget: WorkBudget {
            max_wall_millis: 60_000,
            max_bytes: 2 * 1024 * 1024 * 1024,
            max_entities: 8192,
            max_actions: 1,
            max_output_tokens: 32768,
            max_game_ticks: 0,
        },
        grants: [
            Capability::Query,
            Capability::Observe,
            Capability::Plan,
            Capability::Designate,
        ]
        .into_iter()
        .map(|capability| CapabilityGrant {
            capability,
            scope: CapabilityScope {
                fortress_id: Some(anchor.fortress_id),
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
pub fn query_context(c: &OperationContext) -> OperationContext {
    let mut c = c.clone();
    c.grants.retain(|g| g.capability == Capability::Query);
    c
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
    pub capture: DigObservation,
    pub records: BTreeMap<String, DigEffect>,
    pub connections: usize,
    pub observes: usize,
    pub prepares: usize,
    pub commits: usize,
    pub queries: usize,
    pub cancels: usize,
    pub lost_commit_reply: bool,
    pub forget_query: bool,
    pub unknown_commit: bool,
}
pub struct Native {
    binding: DigBinding,
    disk: Memory,
    state: Rc<RefCell<SourceState>>,
}
fn native_authority(c: &OperationContext, capability: Capability) -> Result<()> {
    c.authorize(
        capability,
        if matches!(capability, Capability::Query | Capability::Observe) {
            RiskTier::ReadOnly
        } else {
            RiskTier::Guarded
        },
        &[],
        None,
    )
}
pub fn factory(
    state: Rc<RefCell<SourceState>>,
    disk: Memory,
) -> impl FnOnce(&DigBinding, DigRegion, &OperationContext) -> Result<Native> {
    move |binding, region, c| {
        native_authority(c, Capability::Query)?;
        if region != state.borrow().capture.region() {
            return Err(invalid("source region changed"));
        }
        state.borrow_mut().connections += 1;
        Ok(Native {
            binding: binding.clone(),
            disk,
            state,
        })
    }
}
pub fn durable_record(
    disk: &Memory,
    binding: &DigBinding,
    plan: &DigPlan,
    c: &OperationContext,
) -> Result<DigRecord> {
    let mut journal = DigJournal::open(
        Memory::from_bytes(disk.durable()),
        c,
        DigMode::Offline,
        Some(binding.clone()),
        None,
    )?;
    journal.get(plan.key(), plan.digest(), c)
}
impl DigSource for Native {
    fn manifest(&self) -> &DigManifest {
        self.binding.manifest()
    }
    fn endpoint(&self) -> Option<SocketAddr> {
        Some(self.binding.endpoint())
    }
    fn observe(&mut self, region: DigRegion, c: &OperationContext) -> Result<DigObservation> {
        native_authority(c, Capability::Observe)?;
        let mut state = self.state.borrow_mut();
        state.observes += 1;
        if region != state.capture.region() {
            return Err(invalid("native observation region changed"));
        }
        Ok(state.capture.clone())
    }
    fn prepare(&mut self, plan: &DigPlan, c: &OperationContext) -> Result<DigPreparation> {
        native_authority(c, Capability::Plan)?;
        let entry = durable_record(&self.disk, &self.binding, plan, c)?;
        assert_eq!(entry.state(), DigState::Intent);
        assert_eq!(entry.plan(), plan);
        let effect = native_record(plan, DigPhase::Prepared)?;
        let mut state = self.state.borrow_mut();
        state.prepares += 1;
        state.records.insert(plan.key().to_owned(), effect.clone());
        DigPreparation::decode(effect.canonical_bytes(), false, plan)
    }
    fn commit(&mut self, plan: &DigPlan, c: &OperationContext) -> Result<DigEffect> {
        native_authority(c, Capability::Designate)?;
        let entry = durable_record(&self.disk, &self.binding, plan, c)?;
        assert_eq!(entry.state(), DigState::DispatchStarted);
        assert_eq!(entry.plan(), plan);
        let mut state = self.state.borrow_mut();
        let effect = native_record(
            plan,
            if state.unknown_commit {
                DigPhase::Unknown
            } else {
                DigPhase::Designated
            },
        )?;
        state.commits += 1;
        state.records.insert(plan.key().to_owned(), effect.clone());
        if state.lost_commit_reply {
            Err(DfmcpError::new(
                ErrorCode::AdapterUnavailable,
                "designation happened; acknowledgement lost",
            ))
        } else {
            Ok(effect)
        }
    }
    fn query(&mut self, plan: &DigPlan, c: &OperationContext) -> Result<Option<DigEffect>> {
        native_authority(c, Capability::Query)?;
        let mut state = self.state.borrow_mut();
        state.queries += 1;
        if state.forget_query {
            return Ok(None);
        }
        state
            .records
            .get(plan.key())
            .map(|e| DigEffect::decode(e.canonical_bytes(), plan))
            .transpose()
    }
    fn cancel(&mut self, plan: &DigPlan, c: &OperationContext) -> Result<DigEffect> {
        native_authority(c, Capability::Designate)?;
        let mut state = self.state.borrow_mut();
        state.cancels += 1;
        if let Some(effect) = state
            .records
            .get(plan.key())
            .filter(|e| e.phase() != DigPhase::Prepared)
        {
            return Ok(effect.clone());
        }
        let effect = native_record(plan, DigPhase::Refused)?;
        state.records.insert(plan.key().to_owned(), effect.clone());
        Ok(effect)
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
pub struct Evidence {
    pub original: DigTerrainProjection,
    pub state: Rc<RefCell<EvidenceState>>,
    views: Cell<u8>,
    current: bool,
}
pub fn evidence_owner(
    original: DigTerrainProjection,
    state: Rc<RefCell<EvidenceState>>,
    c: &OperationContext,
) -> Result<Evidence> {
    native_authority(c, Capability::Observe)?;
    if original.anchor() != c.anchor {
        return Err(invalid("evidence anchor changed"));
    }
    Ok(Evidence {
        original,
        state,
        views: Cell::new(0),
        current: false,
    })
}
impl DigEvidenceOwner for Evidence {
    fn refresh_reserve_bytes(&self) -> Result<u64> {
        Ok(64 * 1024 * 1024)
    }
    fn refresh_after_capture(
        &mut self,
        capture: &DigObservation,
        c: &OperationContext,
    ) -> Result<()> {
        self.current = false;
        native_authority(c, Capability::Observe)?;
        let mut state = self.state.borrow_mut();
        state.opened += 1;
        state.reads += 1;
        state.authorizations += 2;
        if !state.permitted
            || state.active != 0
            || state.observation != *self.original.observation()
            || c.anchor != self.original.anchor()
            || capture.fortress_id() != self.original.anchor().fortress_id
            || capture.tick() != self.original.anchor().tick.get()
            || !capture.paused()
            || !capture.blockers(false).is_empty()
        {
            return Err(DfmcpError::new(
                ErrorCode::CapabilityDenied,
                "original independent terrain evidence changed",
            ));
        }
        self.current = true;
        self.views.set(0);
        Ok(())
    }
    fn routing_evidence(&self) -> Result<LiveRoutingEvidence<'_>> {
        if !self.current || self.views.get() >= 2 {
            return Err(invalid("fresh independent evidence unavailable"));
        }
        self.views.set(self.views.get() + 1);
        LiveRoutingEvidence::dig_terrain_v1_8(policy(&self.original), &self.original)
    }
}
pub struct Guard {
    pub permitted: Rc<Cell<bool>>,
    pub calls: Rc<RefCell<Vec<DigStage>>>,
}
impl Default for Guard {
    fn default() -> Self {
        Self {
            permitted: Rc::new(Cell::new(true)),
            calls: Rc::default(),
        }
    }
}
impl DigGuard for Guard {
    fn check(&mut self, stage: DigStage, _: &DigPlan, _: &OperationContext) -> Result<()> {
        self.calls.borrow_mut().push(stage);
        if stage == DigStage::Commit && !self.permitted.get() {
            return Err(DfmcpError::new(
                ErrorCode::CapabilityDenied,
                "current designation policy revoked",
            ));
        }
        Ok(())
    }
}
impl DigSessionGuard for Guard {
    fn connect(&mut self, _: &DigBinding, _: DigRegion, c: &OperationContext) -> Result<()> {
        native_authority(c, Capability::Query)
    }
    fn observe(&mut self, _: &DigBinding, _: DigRegion, c: &OperationContext) -> Result<()> {
        native_authority(c, Capability::Observe)
    }
}
pub struct Fixture {
    pub session: Session,
    pub original: PreparedPlan,
    pub context: OperationContext,
    pub capture: DigObservation,
    pub binding: DigBinding,
    pub native_disk: Memory,
    pub associations: Memory,
    pub native: Rc<RefCell<SourceState>>,
    pub evidence: Evidence,
    pub evidence_state: Rc<RefCell<EvidenceState>>,
    pub original_state: DigTerrainProjection,
    pub guard: Guard,
}
impl Fixture {
    pub fn new() -> Result<Self> {
        let capture = capture()?;
        let observation = spatial_observation(&capture, SPATIAL_GENERATION)?;
        let basis = anchor_context(StateAnchor {
            fortress_id: capture.fortress_id(),
            cursor: ObservationCursor::ORIGIN,
            tick: GameTick(capture.tick()),
            state_hash: capture.witness(),
        });
        let original_state = DigTerrainProjection::new(
            observation.clone(),
            ObservationCursor {
                epoch: 0,
                sequence: 1,
            },
            &basis,
        )?;
        let context = context(original_state.snapshot());
        let original = original_plan(original_state.snapshot(), &capture)?;
        let binding = DigBinding::new(
            SocketAddr::from(([127, 0, 0, 1], 5000)),
            DigManifest {
                generation: capture.generation(),
                df_version: "df".to_owned(),
                dfhack_version: "dfhack".to_owned(),
            },
            &capture,
            MapCuboid::new(MapCoord::new(0, 0, 0), MapCoord::new(63, 63, 7))?,
        )?;
        let native_disk = Memory::default();
        let associations = Memory::default();
        let journal = DigJournal::open(
            native_disk.clone(),
            &context,
            DigMode::Control,
            Some(binding.clone()),
            Some([27; 32]),
        )?;
        let id = journal.id();
        let native_session = DigSession::new(journal, &context)?;
        let store = AssociationStore::open(associations.clone(), id, true, false, &context)?;
        let session = Session::new(native_session, store, &context)?;
        let native = Rc::new(RefCell::new(SourceState {
            capture: capture.clone(),
            records: BTreeMap::new(),
            connections: 0,
            observes: 0,
            prepares: 0,
            commits: 0,
            queries: 0,
            cancels: 0,
            lost_commit_reply: false,
            forget_query: false,
            unknown_commit: false,
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
    pub fn observe(&mut self) -> Result<SemanticDigReview> {
        self.session.observe(
            self.original.clone(),
            &mut self.evidence,
            &self.context,
            factory(self.native.clone(), self.native_disk.clone()),
            &mut self.guard,
        )
    }
    pub fn prepared(&mut self) -> Result<SemanticDigReview> {
        let review = self.observe()?;
        let result = self.session.prepare(
            review.seal(),
            &mut self.evidence,
            &self.context,
            &mut self.guard,
        )?;
        assert_eq!(result.native_record().state(), DigState::Prepared);
        Ok(review)
    }
}
pub fn reopen(
    native_disk: Memory,
    associations: Memory,
    binding: &DigBinding,
    mode: DigMode,
    read_only: bool,
    c: &OperationContext,
) -> Result<Session> {
    let journal = DigJournal::open(native_disk, c, mode, Some(binding.clone()), None)?;
    let id = journal.id();
    let native = DigSession::new(journal, c)?;
    let store = AssociationStore::open(associations, id, false, read_only, c)?;
    Session::new(native, store, c)
}
