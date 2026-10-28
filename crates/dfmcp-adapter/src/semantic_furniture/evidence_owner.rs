//! Fresh complete spatial/1.8 evidence for one original furniture plan.
//!
//! Reacquisition must reproduce the exact original published state. The source
//! closes before an independent policy issuer authorizes that state; neither
//! source labels nor a retained snapshot issue authority. Native furniture and
//! spatial incarnations remain independent. Rejected reads retain their floors.

use std::cell::Cell;
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::rc::Rc;
use std::time::{Duration, Instant};

use dfmcp_core::{
    Capability, DfmcpError, ErrorCode, OperationContext, Result, RiskTier, StateAnchor,
};
use dfmcp_world::{CompareOp, EntityKind, EvidencePolicy, Predicate, PredicateEvidence, Value};

use crate::build_placement::BuildCapture;
use crate::live_jobs_rpc::operations::map::spatial::citizens::{
    CitizenSpatialLimits, CitizenSpatialRpcClient,
};
use crate::live_operations::item_entity_id;
use crate::live_routing::LiveRoutingEvidence;
use crate::live_spatial::SpatialStateView;
use crate::live_spatial::citizens::{LiveSpatialCitizenObservation, LiveSpatialCitizenState};

use super::FurnitureEvidenceOwner;

const CONNECTION_BYTES: u64 = 1024 * 1024;
const AUTHORIZE_BYTES: u64 = 256 * 1024;
const LOCAL_BASE_BYTES: u64 = 64 * 1024;
const FINAL_CHECK_BYTES: u64 = 4096;
// Includes repeated canonical hashing, raw encoding, lookup and borrowed-scope
// validation. Skills have separately bounded projected fields. These are logical
// byte-work reservations, never eager allocations of the requested allowance.
const LOCAL_NODE_BYTES: u64 = 64 * 1024;
const LOCAL_SKILL_BYTES: u64 = 32 * 1024;
const ROUTING_VIEWS: u8 = 2;

fn error(code: ErrorCode, message: &str) -> DfmcpError {
    DfmcpError::new(code, message)
}
fn exhausted() -> DfmcpError {
    error(
        ErrorCode::BudgetExceeded,
        "spatial furniture evidence exhausted its shared allowance",
    )
}
fn stale() -> DfmcpError {
    error(
        ErrorCode::StaleAnchor,
        "spatial furniture evidence differs from the original paused source; prepare a new plan",
    )
}
fn unavailable() -> DfmcpError {
    error(
        ErrorCode::AdapterUnavailable,
        "spatial furniture evidence must be freshly acquired",
    )
}

/// One operation-owned complete spatial source. Implementations apply the
/// supplied shrinking deadline and byte allowance before acquisition, verify
/// complete paging and release, and never return a partial observation. The
/// owner drops the source on both success and failure, before issuing policy.
pub trait SpatialFurnitureEvidenceSource {
    fn read_observation(
        &mut self,
        context: &OperationContext,
    ) -> Result<LiveSpatialCitizenObservation>;
}

/// Injectable bounded stream for the existing fixed spatial/1.8 RPC codec.
pub trait SpatialFurnitureEvidenceIo: Read + Write {
    fn narrow_timeout(&mut self, timeout: Duration) -> io::Result<()>;
}
impl SpatialFurnitureEvidenceIo for TcpStream {
    fn narrow_timeout(&mut self, timeout: Duration) -> io::Result<()> {
        self.set_read_timeout(Some(timeout))?;
        self.set_write_timeout(Some(timeout))
    }
}

#[derive(Clone, Copy)]
struct WireAllowance {
    deadline: Instant,
    bytes: u64,
}
struct BoundedIo<S> {
    stream: S,
    allowance: Rc<Cell<WireAllowance>>,
}
impl<S: SpatialFurnitureEvidenceIo> BoundedIo<S> {
    fn before(&mut self, count: usize) -> io::Result<()> {
        let allowance = self.allowance.get();
        let remaining = allowance
            .deadline
            .checked_duration_since(Instant::now())
            .filter(|d| !d.is_zero())
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::TimedOut, "spatial evidence deadline expired")
            })?;
        if count as u64 > allowance.bytes {
            return Err(io::Error::other(
                "spatial evidence byte allowance exhausted",
            ));
        }
        self.stream.narrow_timeout(remaining)
    }
    fn charge(&self, count: usize) {
        let mut allowance = self.allowance.get();
        allowance.bytes = allowance.bytes.saturating_sub(count as u64);
        self.allowance.set(allowance);
    }
}
impl<S: SpatialFurnitureEvidenceIo> Read for BoundedIo<S> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        self.before(output.len())?;
        let count = self.stream.read(output)?;
        self.charge(count);
        Ok(count)
    }
}
impl<S: SpatialFurnitureEvidenceIo> Write for BoundedIo<S> {
    fn write(&mut self, input: &[u8]) -> io::Result<usize> {
        self.before(input.len())?;
        let count = self.stream.write(input)?;
        self.charge(count);
        Ok(count)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.before(0)?;
        self.stream.flush()
    }
}

/// Fixed Handshake/ReadObservation spatial/1.8 acquisition, including verified
/// release, under one nonrenewable connection/acquisition deadline. There is no
/// caller-selected protocol, method, command, or policy. Dropping closes locally
/// without an additional cleanup request. Every failed call fences this source.
pub struct SpatialFurnitureRpcSource<S> {
    client: CitizenSpatialRpcClient<BoundedIo<S>>,
    allowance: Rc<Cell<WireAllowance>>,
    used: bool,
}
impl<S: SpatialFurnitureEvidenceIo> SpatialFurnitureRpcSource<S> {
    pub fn negotiate(
        stream: S,
        token: Vec<u8>,
        nonce: Vec<u8>,
        limits: CitizenSpatialLimits,
        context: &OperationContext,
    ) -> Result<Self> {
        Self::negotiate_until(stream, token, nonce, limits, context, deadline(context)?)
    }
    /// Preserve a trusted host's earlier connection deadline while negotiating
    /// a checked stream. The supplied deadline can only narrow the context.
    pub fn negotiate_until(
        stream: S,
        token: Vec<u8>,
        nonce: Vec<u8>,
        limits: CitizenSpatialLimits,
        context: &OperationContext,
        end: Instant,
    ) -> Result<Self> {
        let end = end.min(deadline(context)?);
        read_authority(context)?;
        limits.validate()?;
        remaining_millis(end)?;
        let allowance = Rc::new(Cell::new(WireAllowance {
            deadline: end,
            bytes: context.budget.max_bytes,
        }));
        let client = CitizenSpatialRpcClient::negotiate(
            BoundedIo {
                stream,
                allowance: Rc::clone(&allowance),
            },
            token,
            nonce,
            limits,
        )?;
        remaining_millis(end)?;
        read_authority(context)?;
        Ok(Self {
            client,
            allowance,
            used: false,
        })
    }
}
impl SpatialFurnitureRpcSource<TcpStream> {
    pub fn connect(
        endpoint: SocketAddr,
        token: Vec<u8>,
        nonce: Vec<u8>,
        limits: CitizenSpatialLimits,
        context: &OperationContext,
    ) -> Result<Self> {
        read_authority(context)?;
        limits.validate()?;
        if !endpoint.ip().is_loopback()
            || endpoint.port() == 0
            || !(32..=256).contains(&token.len())
            || !(16..=64).contains(&nonce.len())
        {
            return Err(error(
                ErrorCode::InvalidRequest,
                "spatial evidence needs a numeric loopback endpoint and bounded credentials",
            ));
        }
        let end = deadline(context)?;
        let remaining = end
            .checked_duration_since(Instant::now())
            .filter(|d| !d.is_zero())
            .ok_or_else(exhausted)?;
        let stream = TcpStream::connect_timeout(&endpoint, remaining).map_err(|_| {
            error(
                ErrorCode::AdapterUnavailable,
                "spatial evidence connection failed",
            )
        })?;
        stream.set_nodelay(true).map_err(|_| {
            error(
                ErrorCode::AdapterUnavailable,
                "spatial evidence socket configuration failed",
            )
        })?;
        // Connecting does not reset the absolute acquisition deadline.
        Self::negotiate_until(stream, token, nonce, limits, context, end)
    }
}
impl<S: SpatialFurnitureEvidenceIo> SpatialFurnitureEvidenceSource
    for SpatialFurnitureRpcSource<S>
{
    fn read_observation(
        &mut self,
        context: &OperationContext,
    ) -> Result<LiveSpatialCitizenObservation> {
        if self.used {
            return Err(unavailable());
        }
        self.used = true;
        read_authority(context)?;
        let prior = self.allowance.get();
        self.allowance.set(WireAllowance {
            deadline: prior.deadline.min(deadline(context)?),
            // The owning ledger prepays a distinct acquisition reservation.
            bytes: context.budget.max_bytes,
        });
        let observation = self.client.read_observation()?;
        remaining_millis(self.allowance.get().deadline)?;
        read_authority(context)?;
        Ok(observation)
    }
}

fn read_authority(context: &OperationContext) -> Result<()> {
    context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    context.authorize(Capability::Observe, RiskTier::ReadOnly, &[], None)
}
fn deadline(context: &OperationContext) -> Result<Instant> {
    context.budget.validate()?;
    Instant::now()
        .checked_add(Duration::from_millis(context.budget.max_wall_millis))
        .ok_or_else(exhausted)
}
fn remaining_millis(end: Instant) -> Result<u64> {
    let left = end
        .checked_duration_since(Instant::now())
        .ok_or_else(exhausted)?;
    u64::try_from(left.as_millis())
        .ok()
        .filter(|n| *n > 0)
        .ok_or_else(exhausted)
}
fn wire_bytes(limits: CitizenSpatialLimits) -> Result<u64> {
    let operations = limits.spatial.operations;
    let pages = operations.payload_bytes.div_ceil(operations.page_bytes) as u64;
    // Two payload passes plus bounded request/envelope/notification headroom
    // for each page and its release. Actual wire work is additionally metered.
    (operations.payload_bytes as u64)
        .checked_mul(2)
        .and_then(|n| n.checked_add((pages + 1) * (512 * 1024 + 8192)))
        .ok_or_else(exhausted)
}
fn local_bytes(state: &LiveSpatialCitizenState, limits: CitizenSpatialLimits) -> Result<u64> {
    let snapshot = state.snapshot().ok_or_else(unavailable)?;
    let observed = state.observation_full().ok_or_else(unavailable)?;
    let nodes = snapshot
        .graph
        .entities
        .len()
        .checked_add(snapshot.graph.edges.len())
        .and_then(|n| n.checked_add(snapshot.graph.chunks.len()))
        .and_then(|n| n.checked_add(snapshot.graph.events.len()))
        .ok_or_else(exhausted)?;
    let skills = observed
        .citizens()
        .iter()
        .try_fold(0usize, |count, citizen| {
            count
                .checked_add(citizen.skills.len())
                .ok_or_else(exhausted)
        })?;
    (nodes as u64)
        .checked_mul(LOCAL_NODE_BYTES)
        .and_then(|n| n.checked_add((skills as u64) * LOCAL_SKILL_BYTES))
        .and_then(|n| n.checked_add(16 * limits.spatial.operations.payload_bytes as u64))
        .and_then(|n| n.checked_add(LOCAL_BASE_BYTES))
        .ok_or_else(exhausted)
}

/// Pure preflight for a trusted host before constructing an evidence owner.
/// This is the same complete logical reservation used by every later refresh.
pub fn evidence_reserve_bytes(
    state: &LiveSpatialCitizenState,
    limits: CitizenSpatialLimits,
) -> Result<u64> {
    limits.validate()?;
    local_bytes(state, limits)?
        .checked_add(wire_bytes(limits)?)
        .and_then(|n| n.checked_add(CONNECTION_BYTES + 2 * AUTHORIZE_BYTES + FINAL_CHECK_BYTES))
        .ok_or_else(exhausted)
}

struct Work {
    context: OperationContext,
    deadline: Instant,
    remaining: u64,
}
impl Work {
    fn new(context: &OperationContext, local: u64) -> Result<Self> {
        read_authority(context)?;
        Ok(Self {
            context: context.clone(),
            deadline: deadline(context)?,
            remaining: context
                .budget
                .max_bytes
                .checked_sub(local)
                .ok_or_else(exhausted)?,
        })
    }
    fn current(&self) -> Result<OperationContext> {
        let mut context = self.context.clone();
        context.budget.max_wall_millis = remaining_millis(self.deadline)?;
        context.budget.max_bytes = self.remaining;
        read_authority(&context)?;
        Ok(context)
    }
    fn reserve(&mut self, bytes: u64) -> Result<OperationContext> {
        let mut context = self.current()?;
        self.remaining = self.remaining.checked_sub(bytes).ok_or_else(exhausted)?;
        context.budget.max_bytes = bytes;
        Ok(context)
    }
}

struct Publication {
    policy: EvidencePolicy,
    deadline: Instant,
    views: Cell<u8>,
}

/// Own an exact already-published spatial basis, including its canonical cursor
/// and entity-generation history. The factory returns a fresh complete source;
/// the separate authorizer validates compatibility and issues exact grants. Both
/// are trusted shell callbacks, never deserializable client inputs.
pub struct SpatialFurnitureEvidenceOwner<F, A> {
    original: LiveSpatialCitizenState,
    anchor: StateAnchor,
    limits: CitizenSpatialLimits,
    factory: F,
    authorizer: A,
    furniture_generation: Option<u64>,
    furniture_sequence: u64,
    high_tick: u64,
    high_job: u32,
    high_building: u32,
    high_item: u32,
    source_changed: bool,
    local_bytes: u64,
    wire_bytes: u64,
    current: Option<Publication>,
}
impl<F, A> SpatialFurnitureEvidenceOwner<F, A> {
    pub fn new<S>(
        original: LiveSpatialCitizenState,
        limits: CitizenSpatialLimits,
        factory: F,
        authorizer: A,
        context: &OperationContext,
    ) -> Result<Self>
    where
        S: SpatialFurnitureEvidenceSource,
        F: FnMut(&OperationContext) -> Result<S>,
        A: FnMut(
            &LiveSpatialCitizenObservation,
            &LiveSpatialCitizenState,
            &OperationContext,
        ) -> Result<EvidencePolicy>,
    {
        limits.validate()?;
        let observation = original.observation_full().ok_or_else(unavailable)?;
        let snapshot = original.snapshot().ok_or_else(unavailable)?;
        if snapshot.anchor() != context.anchor || !snapshot.paused {
            return Err(stale());
        }
        let operations = observation.spatial().operations();
        let mut owner = Self {
            anchor: context.anchor,
            limits,
            factory,
            authorizer,
            furniture_generation: None,
            furniture_sequence: 0,
            high_tick: context.anchor.tick.get(),
            high_job: operations.jobs.next_job_id,
            high_building: operations.next_building_id,
            high_item: operations.next_item_id,
            source_changed: false,
            local_bytes: local_bytes(&original, limits)?,
            wire_bytes: wire_bytes(limits)?,
            original,
            current: None,
        };
        owner.refresh(context)?;
        Ok(owner)
    }
    /// Complete logical allowance prepaid by the semantic effect coordinator.
    pub fn required_bytes(&self) -> Result<u64> {
        self.local_bytes
            .checked_add(self.wire_bytes)
            .and_then(|n| n.checked_add(CONNECTION_BYTES + 2 * AUTHORIZE_BYTES + FINAL_CHECK_BYTES))
            .ok_or_else(exhausted)
    }
    /// Invalidation retires usable evidence, never consumed source floors.
    pub fn invalidate(&mut self) {
        self.current = None;
    }
    pub fn refresh<S>(&mut self, context: &OperationContext) -> Result<()>
    where
        S: SpatialFurnitureEvidenceSource,
        F: FnMut(&OperationContext) -> Result<S>,
        A: FnMut(
            &LiveSpatialCitizenObservation,
            &LiveSpatialCitizenState,
            &OperationContext,
        ) -> Result<EvidencePolicy>,
    {
        self.refresh_required::<S>(None, context)
    }
    fn retain_native(&mut self, capture: &BuildCapture) -> Result<()> {
        let original = self.original.observation_full().ok_or_else(unavailable)?;
        let terrain = original.spatial().terrain();
        if capture.fortress_id() != self.anchor.fortress_id
            || capture.folder() != terrain.world_folder
            || capture.site() != terrain.site_id
            || capture.dimensions() != terrain.map_dimensions
            || self
                .furniture_generation
                .is_some_and(|n| n != capture.generation())
        {
            self.source_changed = true;
            return Err(stale());
        }
        // Pin before any canonical acquisition or later authority failure.
        self.furniture_generation = Some(capture.generation());
        let regressed = capture.sequence() < self.furniture_sequence
            || capture.tick() < self.high_tick
            || capture.next_job_id() < self.high_job
            || capture.next_building_id() < self.high_building;
        self.furniture_sequence = self.furniture_sequence.max(capture.sequence());
        self.high_tick = self.high_tick.max(capture.tick());
        self.high_job = self.high_job.max(capture.next_job_id());
        self.high_building = self.high_building.max(capture.next_building_id());
        if regressed
            || capture.tick() != self.anchor.tick.get()
            || !capture.paused()
            || !capture.eligible()
            || self
                .limits
                .spatial
                .region
                .index(capture.selection().target())
                .is_none()
            || original
                .spatial()
                .operations()
                .items
                .binary_search_by_key(&capture.selection().item_id(), |item| item.native_id)
                .is_err()
        {
            return Err(stale());
        }
        Ok(())
    }
    fn retain_spatial(&mut self, observed: &LiveSpatialCitizenObservation) -> Result<()> {
        let original = self.original.observation_full().ok_or_else(unavailable)?;
        let expected = original.spatial().terrain();
        let terrain = observed.spatial().terrain();
        let operations = observed.spatial().operations();
        if terrain.world_folder != expected.world_folder
            || terrain.site_id != expected.site_id
            || terrain.bridge_generation != expected.bridge_generation
            || terrain.df_version != expected.df_version
            || terrain.dfhack_version != expected.dfhack_version
            || terrain.map_dimensions != expected.map_dimensions
            || terrain.map.region != expected.map.region
        {
            self.source_changed = true;
            return Err(stale());
        }
        let regressed = terrain.tick().get() < self.high_tick
            || operations.jobs.next_job_id < self.high_job
            || operations.next_building_id < self.high_building
            || operations.next_item_id < self.high_item;
        self.high_tick = self.high_tick.max(terrain.tick().get());
        self.high_job = self.high_job.max(operations.jobs.next_job_id);
        self.high_building = self.high_building.max(operations.next_building_id);
        self.high_item = self.high_item.max(operations.next_item_id);
        if observed != original {
            // Even a same-tick change cannot later disappear behind the old
            // canonical generation history after an explicit invalidation.
            self.source_changed = true;
        }
        if regressed || self.source_changed {
            return Err(stale());
        }
        Ok(())
    }
    fn refresh_required<S>(
        &mut self,
        capture: Option<&BuildCapture>,
        context: &OperationContext,
    ) -> Result<()>
    where
        S: SpatialFurnitureEvidenceSource,
        F: FnMut(&OperationContext) -> Result<S>,
        A: FnMut(
            &LiveSpatialCitizenObservation,
            &LiveSpatialCitizenState,
            &OperationContext,
        ) -> Result<EvidencePolicy>,
    {
        self.invalidate();
        if let Some(capture) = capture {
            self.retain_native(capture)?;
        }
        let original = self.original.observation_full().ok_or_else(unavailable)?;
        let snapshot = self.original.snapshot().ok_or_else(unavailable)?;
        if self.source_changed
            || context.anchor != self.anchor
            || self.high_tick > self.anchor.tick.get()
            || snapshot.anchor() != self.anchor
        {
            return Err(stale());
        }
        if context.budget.max_entities < snapshot.graph.entities.len() as u32
            || context.budget.max_bytes < self.required_bytes()?
        {
            return Err(exhausted());
        }
        let mut work = Work::new(context, self.local_bytes)?;
        validate_capture(original, self.limits)?;
        let source_context = work.reserve(CONNECTION_BYTES)?;
        let mut source = (self.factory)(&source_context)?;
        work.current()?;
        let observed = source.read_observation(&work.reserve(self.wire_bytes)?);
        drop(source);
        // A successfully consumed typed observation advances floors even if the
        // response arrived after its deadline or its policy is later refused.
        let observed = observed?;
        self.retain_spatial(&observed)?;
        work.current()?;
        validate_capture(&observed, self.limits)?;
        let policy = (self.authorizer)(&observed, &self.original, &work.reserve(AUTHORIZE_BYTES)?)?;
        validate_scope(&self.original, policy.clone())?;
        let final_policy =
            (self.authorizer)(&observed, &self.original, &work.reserve(AUTHORIZE_BYTES)?)?;
        if final_policy != policy {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "spatial evidence authority changed before publication",
            ));
        }
        work.current()?;
        self.current = Some(Publication {
            policy,
            deadline: work.deadline,
            views: Cell::new(0),
        });
        Ok(())
    }
}

fn validate_capture(
    observation: &LiveSpatialCitizenObservation,
    limits: CitizenSpatialLimits,
) -> Result<()> {
    observation.validate()?;
    let operations = observation.spatial().operations();
    let bound = limits.spatial.operations;
    if !observation.spatial().terrain().paused
        || observation.spatial().terrain().map.region != limits.spatial.region
        || operations.jobs.jobs.len() > bound.jobs as usize
        || operations.buildings.len() > bound.buildings as usize
        || operations.items.len() > bound.items as usize
        || observation.citizens().len() > limits.citizens as usize
        || observation.encode_payload()?.len() > bound.payload_bytes
    {
        return Err(stale());
    }
    Ok(())
}
fn validate_scope(state: &LiveSpatialCitizenState, policy: EvidencePolicy) -> Result<()> {
    let snapshot = state.snapshot().ok_or_else(unavailable)?;
    let observed = state.observation_full().ok_or_else(unavailable)?;
    let scope = PredicateEvidence::scoped(snapshot, policy)?;
    if !scope.establishes(&Predicate::Paused(true))? {
        return Err(error(
            ErrorCode::CapabilityDenied,
            "spatial source has no independently authorized pause evidence",
        ));
    }
    for item in &observed.spatial().operations().items {
        let entity_id = item_entity_id(item.native_id);
        if !scope.establishes(&Predicate::EntityKind {
            entity_id,
            kind: EntityKind::Item,
        })? || !scope.establishes(&Predicate::FieldCompare {
            entity_id,
            field: "native_item_id".to_owned(),
            op: CompareOp::Eq,
            value: Value::U64(u64::from(item.native_id)),
        })? {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "spatial source has no independently authorized item identity evidence",
            ));
        }
    }
    Ok(())
}

impl<F, A, S> FurnitureEvidenceOwner for SpatialFurnitureEvidenceOwner<F, A>
where
    S: SpatialFurnitureEvidenceSource,
    F: FnMut(&OperationContext) -> Result<S>,
    A: FnMut(
        &LiveSpatialCitizenObservation,
        &LiveSpatialCitizenState,
        &OperationContext,
    ) -> Result<EvidencePolicy>,
{
    fn refresh_reserve_bytes(&self) -> Result<u64> {
        self.required_bytes()
    }
    fn refresh_after_capture(
        &mut self,
        capture: &BuildCapture,
        context: &OperationContext,
    ) -> Result<()> {
        self.refresh_required::<S>(Some(capture), context)
    }
    fn routing_evidence(&self) -> Result<LiveRoutingEvidence<'_>> {
        let publication = self.current.as_ref().ok_or_else(unavailable)?;
        if Instant::now() >= publication.deadline || publication.views.get() >= ROUTING_VIEWS {
            return Err(exhausted());
        }
        publication.views.set(publication.views.get() + 1);
        let snapshot = self.original.snapshot().ok_or_else(unavailable)?;
        let evidence = LiveRoutingEvidence::spatial_v1_8(
            PredicateEvidence::scoped(snapshot, publication.policy.clone())?,
            &self.original,
        )?;
        remaining_millis(publication.deadline)?;
        Ok(evidence)
    }
}

#[cfg(test)]
mod tests;
