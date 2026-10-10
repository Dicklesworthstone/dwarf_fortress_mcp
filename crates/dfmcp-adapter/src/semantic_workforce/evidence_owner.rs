//! Fresh V1 citizen evidence for an already anchored semantic workforce plan.
//!
//! This owner acquires and validates evidence, but never issues its authority.
//! A separately trusted callback must establish compatibility and issue the
//! exact source/domain policy. No policy is inferred from native field labels.
//! Every refresh opens one source, assembles a complete paused roster, closes the
//! source, and publishes only an unchanged original capsule with a checked scope.

use std::cell::Cell;
use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::rc::Rc;
use std::time::{Duration, Instant};

use dfmcp_core::{
    Capability, DfmcpError, ErrorCode, OperationContext, Result, RiskTier, StateAnchor,
};
use dfmcp_world::{CompareOp, EntityKind, EvidencePolicy, Predicate, PredicateEvidence, Value};

use crate::live_connect::LiveConnectionConfig;
use crate::live_projection::{LiveWorldProjection, project_live_capsule, raw_unit_id_to_entity_id};
use crate::live_routing::LiveRoutingEvidence;
use crate::workforce_control::WorkforceCapture;
use crate::{
    BridgeCredentials, BridgeManifest, DfHackRpcClient, LiveObservationCapsule,
    LiveObservationSource, ObservationPage, read_complete_observation_bounded,
};

use super::WorkforceEvidenceOwner;

const MAX_CITIZENS: u32 = 4096;
const CONNECTION_BYTES: u64 = 1024 * 1024;
const AUTHORIZE_BYTES: u64 = 256 * 1024;
const LOCAL_BASE_BYTES: u64 = 64 * 1024;
// Each V1 citizen and membership edge occupy fewer than 4 KiB of canonical
// projected bytes. Reserve sixteen full passes for construction, encoding,
// hashing, source/receipt checks, and the two bounded borrowed routing views.
// This includes the smaller raw capsule passes and retained copies. It is
// byte-work rather than an eagerly allocated buffer.
const LOCAL_CITIZEN_BYTES: u64 = 64 * 1024;
const ROUTING_VIEWS: u8 = 2;

fn error(code: ErrorCode, message: &str) -> DfmcpError {
    DfmcpError::new(code, message)
}
fn exhausted() -> DfmcpError {
    error(
        ErrorCode::BudgetExceeded,
        "citizen evidence exhausted its shared allowance",
    )
}
fn stale() -> DfmcpError {
    error(
        ErrorCode::StaleAnchor,
        "citizen evidence differs from the original paused semantic source; prepare a new plan",
    )
}

/// A bounded full-roster read. Larger limits need correspondingly larger byte
/// allowances; they never truncate the original roster into a partial scope.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CitizenEvidenceLimits {
    pub page_size: u32,
    pub max_citizens: u32,
}
impl Default for CitizenEvidenceLimits {
    fn default() -> Self {
        Self {
            page_size: 256,
            max_citizens: 256,
        }
    }
}
impl CitizenEvidenceLimits {
    pub fn validate(self) -> Result<()> {
        if self.page_size == 0
            || self.page_size > MAX_CITIZENS
            || self.max_citizens == 0
            || self.max_citizens > MAX_CITIZENS
        {
            return Err(error(
                ErrorCode::InvalidRequest,
                "citizen evidence limits must be in 1..=4096",
            ));
        }
        Ok(())
    }
    fn local_bytes(self) -> u64 {
        LOCAL_BASE_BYTES + (u64::from(self.max_citizens) + 1) * LOCAL_CITIZEN_BYTES
    }
}

/// An operation-scoped source. Implementations must apply the supplied narrowing
/// deadline and byte allowance before the actual page I/O. The owner never
/// retains a source between refreshes and never retries a failed page.
pub trait CitizenEvidenceSource {
    fn bridge_manifest(&self) -> BridgeManifest;
    fn read_page(
        &mut self,
        offset: u32,
        maximum: u32,
        include_names: bool,
        context: &OperationContext,
    ) -> Result<ObservationPage>;
}

/// A stream whose next blocking operation can be narrowed to this timeout.
/// `TcpStream` supplies the real implementation; injected streams allow the
/// exact V1 codec to be exercised without a game or another runtime.
pub trait CitizenEvidenceIo: Read + Write {
    fn narrow_timeout(&mut self, timeout: Duration) -> io::Result<()>;
}
impl CitizenEvidenceIo for TcpStream {
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
    read_timeout: Duration,
    write_timeout: Duration,
}
impl<S: CitizenEvidenceIo> BoundedIo<S> {
    fn before(&mut self, bytes: usize, reading: bool) -> io::Result<()> {
        let current = self.allowance.get();
        let left = current
            .deadline
            .checked_duration_since(Instant::now())
            .filter(|left| !left.is_zero())
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::TimedOut, "citizen RPC deadline expired")
            })?;
        if bytes as u64 > current.bytes {
            return Err(io::Error::other("citizen RPC byte allowance exhausted"));
        }
        self.stream.narrow_timeout(left.min(if reading {
            self.read_timeout
        } else {
            self.write_timeout
        }))
    }
    fn charge(&self, bytes: usize) {
        let mut current = self.allowance.get();
        current.bytes = current.bytes.saturating_sub(bytes as u64);
        self.allowance.set(current);
    }
}
impl<S: CitizenEvidenceIo> Read for BoundedIo<S> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        self.before(output.len(), true)?;
        let count = self.stream.read(output)?;
        self.charge(count);
        Ok(count)
    }
}
impl<S: CitizenEvidenceIo> Write for BoundedIo<S> {
    fn write(&mut self, input: &[u8]) -> io::Result<usize> {
        self.before(input.len(), false)?;
        let count = self.stream.write(input)?;
        self.charge(count);
        Ok(count)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.before(0, false)?;
        self.stream.flush()
    }
}

/// The existing authenticated V1 codec under one nonrenewable wall deadline.
/// Negotiation and each page consume their own explicit byte reservation from
/// the owner's shrinking ledger. Dropping closes without a cleanup RPC.
pub struct CitizenRpcSource<S> {
    client: DfHackRpcClient<BoundedIo<S>>,
    allowance: Rc<Cell<WireAllowance>>,
    fenced: bool,
}
impl<S: CitizenEvidenceIo> CitizenRpcSource<S> {
    pub fn negotiate(
        stream: S,
        credentials: BridgeCredentials,
        config: &LiveConnectionConfig,
        context: &OperationContext,
    ) -> Result<Self> {
        Self::negotiate_until(stream, credentials, config, context, deadline(context)?)
    }

    fn negotiate_until(
        stream: S,
        credentials: BridgeCredentials,
        config: &LiveConnectionConfig,
        context: &OperationContext,
        end: Instant,
    ) -> Result<Self> {
        context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
        config.validate()?;
        if Instant::now() >= end {
            return Err(exhausted());
        }
        let allowance = Rc::new(Cell::new(WireAllowance {
            deadline: end,
            bytes: context.budget.max_bytes,
        }));
        let client = DfHackRpcClient::negotiate(
            BoundedIo {
                stream,
                allowance: Rc::clone(&allowance),
                read_timeout: config.read_timeout,
                write_timeout: config.write_timeout,
            },
            credentials,
            &config.client_name,
            &config.client_version,
        )?;
        if Instant::now() >= allowance.get().deadline {
            return Err(exhausted());
        }
        Ok(Self {
            client,
            allowance,
            fenced: false,
        })
    }
}
impl CitizenRpcSource<TcpStream> {
    pub fn connect(
        config: &LiveConnectionConfig,
        credentials: BridgeCredentials,
        context: &OperationContext,
    ) -> Result<Self> {
        context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
        config.validate()?;
        let end = deadline(context)?;
        let left = end
            .checked_duration_since(Instant::now())
            .ok_or_else(exhausted)?;
        let stream = TcpStream::connect_timeout(&config.endpoint, config.connect_timeout.min(left))
            .map_err(|_| {
                error(
                    ErrorCode::AdapterUnavailable,
                    "citizen evidence connection failed",
                )
            })?;
        stream.set_nodelay(true).map_err(|_| {
            error(
                ErrorCode::AdapterUnavailable,
                "citizen evidence socket configuration failed",
            )
        })?;
        // Preserve the same absolute deadline across connect and handshake;
        // converting it back into a duration here could renew it on preemption.
        Self::negotiate_until(stream, credentials, config, context, end)
    }
}
impl<S: CitizenEvidenceIo> CitizenEvidenceSource for CitizenRpcSource<S> {
    fn bridge_manifest(&self) -> BridgeManifest {
        self.client.manifest().clone()
    }
    fn read_page(
        &mut self,
        offset: u32,
        maximum: u32,
        include_names: bool,
        context: &OperationContext,
    ) -> Result<ObservationPage> {
        if self.fenced {
            return Err(error(
                ErrorCode::AdapterUnavailable,
                "citizen evidence source is fenced",
            ));
        }
        // Every error, including pre-I/O authority failure, prevents source reuse.
        self.fenced = true;
        context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
        let old = self.allowance.get();
        self.allowance.set(WireAllowance {
            deadline: old.deadline.min(deadline(context)?),
            bytes: context.budget.max_bytes,
        });
        let page = self
            .client
            .read_observation(offset, maximum, include_names)?;
        if Instant::now() >= self.allowance.get().deadline {
            return Err(exhausted());
        }
        context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
        self.fenced = false;
        Ok(page)
    }
}

fn deadline(context: &OperationContext) -> Result<Instant> {
    if context.budget.max_wall_millis == 0 || context.budget.max_bytes == 0 {
        return Err(exhausted());
    }
    Instant::now()
        .checked_add(Duration::from_millis(context.budget.max_wall_millis))
        .ok_or_else(exhausted)
}
fn remaining_millis(end: Instant) -> Result<u64> {
    let millis = end
        .checked_duration_since(Instant::now())
        .ok_or_else(exhausted)?
        .as_millis();
    if millis == 0 {
        return Err(exhausted());
    }
    u64::try_from(millis).map_err(|_| exhausted())
}
struct Work {
    context: OperationContext,
    deadline: Instant,
    remaining: u64,
}
impl Work {
    fn new(
        context: &OperationContext,
        anchor: StateAnchor,
        limits: CitizenEvidenceLimits,
    ) -> Result<Self> {
        context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
        if context.anchor != anchor {
            return Err(stale());
        }
        if context.budget.max_entities < limits.max_citizens.saturating_add(1) {
            return Err(exhausted());
        }
        let remaining = context
            .budget
            .max_bytes
            .checked_sub(limits.local_bytes())
            .ok_or_else(exhausted)?;
        let out = Self {
            context: context.clone(),
            deadline: deadline(context)?,
            remaining,
        };
        // One source and at least one complete page plus both authority checks
        // must fit before even opening a connection.
        if remaining < CONNECTION_BYTES + page_bytes(limits.page_size) + 2 * AUTHORIZE_BYTES {
            return Err(exhausted());
        }
        Ok(out)
    }
    fn current(&self) -> Result<OperationContext> {
        let mut out = self.context.clone();
        out.budget.max_wall_millis = remaining_millis(self.deadline)?;
        out.budget.max_bytes = self.remaining;
        out.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
        Ok(out)
    }
    fn charge(&mut self, bytes: u64) -> Result<()> {
        self.remaining = self.remaining.checked_sub(bytes).ok_or_else(exhausted)?;
        self.current()?;
        Ok(())
    }
    fn reserve(&mut self, bytes: u64) -> Result<OperationContext> {
        let mut out = self.current()?;
        self.charge(bytes)?;
        out.budget.max_bytes = bytes;
        Ok(out)
    }
}
fn page_bytes(maximum: u32) -> u64 {
    // Include the wire's 256 KiB notification ceiling, all bounded citizen
    // fields, summaries/requests, and a second pass for decoding/assembly.
    2 * (256 * 1024 + 4096 + u64::from(maximum) * 512)
}

struct Pages<'a, S> {
    source: &'a mut S,
    work: &'a mut Work,
    expected: &'a BridgeManifest,
}
impl<S: CitizenEvidenceSource> LiveObservationSource for Pages<'_, S> {
    fn bridge_manifest(&self) -> BridgeManifest {
        self.expected.clone()
    }
    fn read_observation_page(
        &mut self,
        offset: u32,
        maximum: u32,
        names: bool,
    ) -> Result<ObservationPage> {
        if self.source.bridge_manifest() != *self.expected {
            return Err(stale());
        }
        let context = self.work.reserve(page_bytes(maximum))?;
        let page = self.source.read_page(offset, maximum, names, &context)?;
        self.work.current()?;
        if self.source.bridge_manifest() != *self.expected || page.citizens.len() > maximum as usize
        {
            return Err(stale());
        }
        Ok(page)
    }
}

struct Publication {
    capsule: LiveObservationCapsule,
    projection: LiveWorldProjection,
    policy: EvidencePolicy,
    deadline: Instant,
    views: Cell<u8>,
}

/// An original-plan observation owner. The factory must return a fresh
/// operation-owned source; the authorizer must independently validate source
/// compatibility and issue its exact policy. Neither callback is deserializable
/// from MCP input. A changed capsule requires a new original plan/owner.
pub struct CitizenWorkforceEvidenceOwner<F, A> {
    original: LiveObservationCapsule,
    anchor: StateAnchor,
    limits: CitizenEvidenceLimits,
    factory: F,
    authorizer: A,
    current: Option<Publication>,
}
impl<F, A> CitizenWorkforceEvidenceOwner<F, A> {
    /// Adopt an exact already known basis, then independently reacquire it before
    /// exposing any routing scope. Supplying a capsule alone grants nothing.
    pub fn new<S>(
        original: LiveObservationCapsule,
        limits: CitizenEvidenceLimits,
        factory: F,
        authorizer: A,
        context: &OperationContext,
    ) -> Result<Self>
    where
        S: CitizenEvidenceSource,
        F: FnMut(&OperationContext) -> Result<S>,
        A: FnMut(
            &LiveObservationCapsule,
            &LiveWorldProjection,
            &OperationContext,
        ) -> Result<EvidencePolicy>,
    {
        limits.validate()?;
        let mut out = Self {
            original,
            anchor: context.anchor,
            limits,
            factory,
            authorizer,
            current: None,
        };
        out.refresh(context)?;
        Ok(out)
    }
    /// Discard usable evidence immediately, including when authorization or
    /// budget validation refuses before opening a source.
    pub fn invalidate(&mut self) {
        self.current = None;
    }

    pub fn refresh<S>(&mut self, context: &OperationContext) -> Result<()>
    where
        S: CitizenEvidenceSource,
        F: FnMut(&OperationContext) -> Result<S>,
        A: FnMut(
            &LiveObservationCapsule,
            &LiveWorldProjection,
            &OperationContext,
        ) -> Result<EvidencePolicy>,
    {
        self.refresh_required::<S>(None, context)
    }
    fn refresh_required<S>(
        &mut self,
        capture: Option<&WorkforceCapture>,
        context: &OperationContext,
    ) -> Result<()>
    where
        S: CitizenEvidenceSource,
        F: FnMut(&OperationContext) -> Result<S>,
        A: FnMut(
            &LiveObservationCapsule,
            &LiveWorldProjection,
            &OperationContext,
        ) -> Result<EvidencePolicy>,
    {
        self.invalidate();
        if self.original.citizens.len() > self.limits.max_citizens as usize
            || self.original.canonical_bytes.len() > 4096 + self.limits.max_citizens as usize * 512
            || !self.original.paused
        {
            return Err(stale());
        }
        // An accepted refresh must contain exactly this already pinned roster.
        // Bound acquisition and all local work by that exact count. A reported
        // larger roster is rejected by the complete reader on its first page;
        // unused capacity in the operator ceiling is never an extra read grant.
        let roster_count = self.original.citizens.len() as u32;
        let required = CitizenEvidenceLimits {
            page_size: self.limits.page_size.min(roster_count.max(1)),
            max_citizens: roster_count,
        };
        let mut work = Work::new(context, self.anchor, required)?;
        self.original.validate()?;
        let original_projection =
            project_live_capsule(&self.original, self.anchor.fortress_id, self.anchor.cursor)?;
        if original_projection.snapshot.anchor() != self.anchor {
            return Err(stale());
        }
        let native_site = u32::try_from(self.original.site_id).map_err(|_| stale())?;
        if self.anchor.fortress_id
            != crate::workforce_control::fortress_id(&self.original.world_folder, native_site)
        {
            return Err(stale());
        }
        if let Some(capture) = capture
            && (capture.fortress_id() != self.anchor.fortress_id
                || capture.folder() != self.original.world_folder
                || capture.site() != native_site
                || capture.generation() != self.original.bridge.bridge_generation
                || capture.tick() != self.anchor.tick.get()
                || !capture.paused()
                || capture.ids().iter().any(|id| {
                    self.original
                        .citizens
                        .binary_search_by_key(id, |u| u.unit_id as u32)
                        .is_err()
                }))
        {
            return Err(stale());
        }
        work.current()?;
        let source_context = work.reserve(CONNECTION_BYTES)?;
        let mut source = (self.factory)(&source_context)?;
        work.current()?;
        if source.bridge_manifest() != self.original.bridge {
            return Err(stale());
        }
        let mut pages = Pages {
            source: &mut source,
            work: &mut work,
            expected: &self.original.bridge,
        };
        let capsule = read_complete_observation_bounded(
            &mut pages,
            required.page_size,
            self.original.names_included,
            required.max_citizens,
        )?;
        drop(source);
        work.current()?;
        // Do not invent a later cursor or use changed source bytes to validate
        // the old plan. Pagination may differ; canonical identity may not.
        if capsule != self.original {
            return Err(stale());
        }
        // Exact equality permits reuse of the projection validated in this
        // call. Rebuilding it would repeat an entire canonical graph pass.
        let projection = original_projection;
        let scope_context = work.reserve(AUTHORIZE_BYTES)?;
        let policy = (self.authorizer)(&capsule, &projection, &scope_context)?;
        validate_scope(&capsule, &projection, policy.clone())?;
        let final_context = work.reserve(AUTHORIZE_BYTES)?;
        let final_policy = (self.authorizer)(&capsule, &projection, &final_context)?;
        if final_policy != policy {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "citizen evidence authority changed before publication",
            ));
        }
        work.current()?;
        self.current = Some(Publication {
            capsule,
            projection,
            policy,
            deadline: work.deadline,
            views: Cell::new(0),
        });
        Ok(())
    }
}

fn validate_scope(
    capsule: &LiveObservationCapsule,
    projection: &LiveWorldProjection,
    policy: EvidencePolicy,
) -> Result<()> {
    let scope = PredicateEvidence::scoped(&projection.snapshot, policy)?;
    if !scope.establishes(&Predicate::Paused(true))? {
        return Err(error(
            ErrorCode::CapabilityDenied,
            "citizen source has no independently authorized pause evidence",
        ));
    }
    for citizen in &capsule.citizens {
        let id = raw_unit_id_to_entity_id(citizen.unit_id)?;
        if !scope.establishes(&Predicate::EntityKind {
            entity_id: id,
            kind: EntityKind::Unit,
        })? || !scope.establishes(&Predicate::FieldCompare {
            entity_id: id,
            field: "raw_unit_id".to_owned(),
            op: CompareOp::Eq,
            value: Value::I64(i64::from(citizen.unit_id)),
        })? {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "citizen source has no independently authorized identity evidence",
            ));
        }
    }
    Ok(())
}

impl<F, A, S> WorkforceEvidenceOwner for CitizenWorkforceEvidenceOwner<F, A>
where
    S: CitizenEvidenceSource,
    F: FnMut(&OperationContext) -> Result<S>,
    A: FnMut(
        &LiveObservationCapsule,
        &LiveWorldProjection,
        &OperationContext,
    ) -> Result<EvidencePolicy>,
{
    fn refresh_after_capture(
        &mut self,
        capture: &WorkforceCapture,
        context: &OperationContext,
    ) -> Result<()> {
        self.refresh_required::<S>(Some(capture), context)
    }
    fn routing_evidence(&self) -> Result<LiveRoutingEvidence<'_>> {
        let current = self.current.as_ref().ok_or_else(|| {
            error(
                ErrorCode::AdapterUnavailable,
                "citizen evidence must be refreshed",
            )
        })?;
        if Instant::now() >= current.deadline || current.views.get() >= ROUTING_VIEWS {
            return Err(exhausted());
        }
        current.views.set(current.views.get() + 1);
        let evidence = LiveRoutingEvidence::citizens_v1(
            PredicateEvidence::scoped(&current.projection.snapshot, current.policy.clone())?,
            &current.projection,
            &current.capsule,
        )?;
        if Instant::now() >= current.deadline {
            return Err(exhausted());
        }
        Ok(evidence)
    }
}
