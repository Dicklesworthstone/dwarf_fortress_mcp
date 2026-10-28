//! Exact original terrain reacquisition before designation preparation and dispatch.
//! Native and canonical generations are independent and never issue one another authority.
use super::*;
use dfmcp_world::Predicate;
use std::cell::Cell;
const POLICY_BYTES: u64 = 256 * 1024;
const CONNECTION_BYTES: u64 = MIB;
const FINAL_BYTES: u64 = 4096;
fn cost(p: &DigTerrainProjection, limits: CitizenSpatialLimits) -> Result<(u64, u64)> {
    limits.validate()?;
    let graph = &p.snapshot().graph;
    let nodes = graph
        .entities
        .len()
        .checked_add(graph.edges.len())
        .and_then(|n| n.checked_add(graph.chunks.len()))
        .and_then(|n| n.checked_add(graph.events.len()))
        .ok_or_else(|| exhausted("dig evidence node count overflow"))?;
    let skills = p.observation().citizens().iter().try_fold(0usize, |n, c| {
        n.checked_add(c.skills.len())
            .ok_or_else(|| exhausted("dig evidence skill count overflow"))
    })?;
    let local = (nodes as u64)
        .checked_mul(64 * 1024)
        .and_then(|n| n.checked_add((skills as u64) * 32 * 1024))
        .and_then(|n| n.checked_add(16 * limits.spatial.operations.payload_bytes as u64))
        .and_then(|n| n.checked_add(64 * 1024))
        .ok_or_else(|| exhausted("dig evidence local allowance overflow"))?;
    let operations = limits.spatial.operations;
    let pages = operations.payload_bytes.div_ceil(operations.page_bytes) as u64;
    let wire = (operations.payload_bytes as u64)
        .checked_mul(2)
        .and_then(|n| n.checked_add((pages + 1) * (512 * 1024 + 8192)))
        .ok_or_else(|| exhausted("dig evidence wire allowance overflow"))?;
    Ok((local, wire))
}
pub(super) fn reserve_bytes(p: &DigTerrainProjection, limits: CitizenSpatialLimits) -> Result<u64> {
    let (local, wire) = cost(p, limits)?;
    local
        .checked_add(wire)
        .and_then(|n| n.checked_add(CONNECTION_BYTES + 2 * POLICY_BYTES + FINAL_BYTES))
        .ok_or_else(|| exhausted("dig evidence complete allowance overflow"))
}
struct Publication {
    policy: EvidencePolicy,
    until: Instant,
    views: Cell<u8>,
}
pub struct NativeEvidence {
    original: DigTerrainProjection,
    config: Arc<Config>,
    native_generation: Option<u64>,
    high_sequence: u64,
    high_tick: u64,
    source_changed: bool,
    local_bytes: u64,
    wire_bytes: u64,
    current: Option<Publication>,
}
impl NativeEvidence {
    pub(super) fn new(
        original: DigTerrainProjection,
        config: Arc<Config>,
        c: &OperationContext,
    ) -> Result<Self> {
        if original.anchor() != c.anchor || !original.snapshot().paused {
            return Err(denied(
                "dig evidence needs its exact original paused publication",
            ));
        }
        let (local_bytes, wire_bytes) = cost(&original, config.limits())?;
        let mut owner = Self {
            original,
            config,
            native_generation: None,
            high_sequence: 0,
            high_tick: c.anchor.tick.get(),
            source_changed: false,
            local_bytes,
            wire_bytes,
            current: None,
        };
        owner.refresh(None, c)?;
        Ok(owner)
    }
    fn required(&self) -> Result<u64> {
        self.local_bytes
            .checked_add(self.wire_bytes)
            .and_then(|n| n.checked_add(CONNECTION_BYTES + 2 * POLICY_BYTES + FINAL_BYTES))
            .ok_or_else(|| exhausted("dig evidence allowance overflow"))
    }
    fn retain_native(&mut self, capture: &DigObservation) -> Result<()> {
        if self.config.sources.capture(capture).is_err()
            || capture.fortress_id() != self.original.anchor().fortress_id
            || self
                .native_generation
                .is_some_and(|n| n != capture.generation())
        {
            self.source_changed = true;
            return Err(denied(
                "native dig source differs from original terrain identity",
            ));
        }
        self.native_generation = Some(capture.generation());
        let regressed = capture.sequence() < self.high_sequence || capture.tick() < self.high_tick;
        self.high_sequence = self.high_sequence.max(capture.sequence());
        self.high_tick = self.high_tick.max(capture.tick());
        self.config.selection(capture.region())?;
        if regressed
            || capture.tick() != self.original.anchor().tick.get()
            || !capture.paused()
            || !capture.blockers(false).is_empty()
        {
            return Err(denied(
                "native mining witness no longer matches the original paused terrain",
            ));
        }
        Ok(())
    }
    fn retain_spatial(&mut self, observed: &LiveSpatialCitizenObservation) -> Result<()> {
        let tick = observed.spatial().terrain().tick().get();
        let regressed = tick < self.high_tick;
        self.high_tick = self.high_tick.max(tick);
        if observed != self.original.observation() {
            self.source_changed = true;
        }
        if regressed || self.source_changed {
            return Err(denied(
                "complete terrain source changed after original planning; retain recovery without redispatch",
            ));
        }
        Ok(())
    }
    fn policy(&self, c: &OperationContext) -> Result<EvidencePolicy> {
        self.config.current()?;
        let policy = self
            .config
            .sources
            .spatial(&self.original, self.config.region, c)?;
        if !self
            .original
            .evidence(policy.clone())?
            .establishes(&Predicate::Paused(true))?
        {
            return Err(denied(
                "original terrain pause has no independent source authority",
            ));
        }
        Ok(policy)
    }
    fn refresh(&mut self, capture: Option<&DigObservation>, c: &OperationContext) -> Result<()> {
        self.current = None;
        if let Some(capture) = capture {
            self.retain_native(capture)?;
        }
        if self.source_changed
            || c.anchor != self.original.anchor()
            || self.high_tick > self.original.anchor().tick.get()
        {
            return Err(denied("original dig evidence is stale or changed lineage"));
        }
        if c.budget.max_bytes < self.required()?
            || c.budget.max_entities < self.original.snapshot().graph.entities.len() as u32
        {
            return Err(exhausted(
                "complete dig evidence exceeds its shared allowance",
            ));
        }
        self.config.current()?;
        let mut work = Work::new(c)?;
        work.reserve(self.local_bytes)?;
        let connection = work.reserve(CONNECTION_BYTES)?;
        let read = work.reserve(self.wire_bytes)?;
        let mut source = self.config.spatial(&work.narrow(&connection)?)?;
        let observation = source.read_observation(&work.narrow(&read)?);
        drop(source);
        let observation = observation?;
        self.retain_spatial(&observation)?;
        work.current()?;
        self.config
            .sources
            .observation(&observation, self.config.region)?;
        let policy = self.policy(&work.reserve(POLICY_BYTES)?)?;
        let final_policy = self.policy(&work.reserve(POLICY_BYTES)?)?;
        if policy != final_policy {
            return Err(denied(
                "original terrain source authority changed before publication",
            ));
        }
        work.current()?;
        self.current = Some(Publication {
            policy,
            until: work.until,
            views: Cell::new(0),
        });
        Ok(())
    }
}
impl DigEvidenceOwner for NativeEvidence {
    fn refresh_reserve_bytes(&self) -> Result<u64> {
        self.required()
    }
    fn refresh_after_capture(
        &mut self,
        capture: &DigObservation,
        c: &OperationContext,
    ) -> Result<()> {
        self.refresh(Some(capture), c)
    }
    fn routing_evidence(&self) -> Result<LiveRoutingEvidence<'_>> {
        let current = self
            .current
            .as_ref()
            .ok_or_else(|| denied("dig evidence must be freshly acquired"))?;
        self.config.current()?;
        if Instant::now() >= current.until || current.views.get() >= 2 {
            return Err(exhausted("dig evidence publication expired or consumed"));
        }
        current.views.set(current.views.get() + 1);
        let evidence =
            LiveRoutingEvidence::dig_terrain_v1_8(current.policy.clone(), &self.original)?;
        if Instant::now() >= current.until {
            return Err(exhausted("dig evidence publication expired"));
        }
        Ok(evidence)
    }
}
