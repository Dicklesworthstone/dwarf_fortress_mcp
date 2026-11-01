//! Fixed workshop/1.20 foreground TCP with current authority and one-shot dispatch.
//!
//! Reuses only the closed protobuf envelope and bounded framing primitives of
//! the furniture client; no furniture wire value, method or field is reused.
//! No runtime, thread, subprocess, reconnect, arbitrary native method or
//! production admission is added.
use super::journal::WorkshopDispatch;
use super::*;
use crate::build_placement::rpc::BuildCancellation;
use crate::build_placement::rpc::codec::{Message, bytes, number};
use crate::build_placement::rpc::link::Link;
use dfmcp_core::{Capability, GameTick, OperationContext, RiskTier};
use std::collections::BTreeMap;
use std::time::Duration;

pub const METHODS: [&str; 7] = [
    "Handshake",
    "ReadWorkshop",
    "PrepareWorkshop",
    "CommitWorkshop",
    "QueryWorkshop",
    "CancelWorkshop",
    "ReadWorkshopProgress",
];
/// Conservative per-connection work accounting reservation.
pub const CONNECT_BYTES: u64 = 1024 * 1024;

fn malformed() -> dfmcp_core::DfmcpError {
    error(
        ErrorCode::AdapterRejected,
        "invalid workshop native envelope",
    )
}
fn denied() -> dfmcp_core::DfmcpError {
    error(
        ErrorCode::CapabilityDenied,
        "workshop operator permission or authority refused",
    )
}
fn budget_error() -> dfmcp_core::DfmcpError {
    error(
        ErrorCode::BudgetExceeded,
        "workshop connection deadline or work allowance exhausted",
    )
}

/// The trusted host repeats its pinned environment and runtime checks. `true`
/// is passed only for native prepare/commit, so build revocation still permits
/// authenticated query, cancellation and progress reads. Never captures MCP input.
pub type WorkshopPermission = Box<dyn Fn(bool) -> Result<()> + Send>;

/// Native requests validate complete building/job/unit registries and a log
/// that may be distant from the footprint: fortress-wide grants are required.
/// Spatial scope, protected regions and leases are enforced by the host guard.
pub fn authority(
    context: &OperationContext,
    fortress: &FortressIdentity,
    observe: bool,
    build: bool,
    tick: u64,
) -> Result<()> {
    if context.anchor.fortress_id != fortress.fortress_id() {
        return Err(denied());
    }
    let mut current = context.clone();
    current.anchor.tick = GameTick(tick.max(current.anchor.tick.get()));
    current.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    if observe || build {
        current.authorize(Capability::Observe, RiskTier::ReadOnly, &[], None)?;
    }
    if build {
        current.authorize(Capability::Plan, RiskTier::Guarded, &[], None)?;
        current.authorize(Capability::Construct, RiskTier::Guarded, &[], None)?;
    }
    Ok(())
}
fn timeout(context: &OperationContext, requested: Duration) -> Result<Duration> {
    context.budget.validate()?;
    if !(1..=60_000).contains(&context.budget.max_wall_millis)
        || requested < Duration::from_millis(1)
        || requested > Duration::from_secs(60)
    {
        return Err(budget_error());
    }
    Ok(requested.min(Duration::from_millis(context.budget.max_wall_millis)))
}

/// Exact native software and incarnation seen on one connection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkshopManifest {
    pub endpoint: SocketAddr,
    pub fortress: FortressIdentity,
    pub generation: u64,
    pub df_version: String,
    pub dfhack_version: String,
}

#[derive(Clone, Copy)]
enum Request<'a> {
    Handshake,
    Observe(WorkshopSelection),
    Prepare(&'a WorkshopPlan),
    Commit(&'a WorkshopPlan),
    Query(&'a WorkshopPlan),
    Cancel(&'a WorkshopPlan),
    Progress { building: u32, item: u32 },
}
impl<'a> Request<'a> {
    fn index(self) -> usize {
        match self {
            Self::Handshake => 0,
            Self::Observe(_) => 1,
            Self::Prepare(_) => 2,
            Self::Commit(_) => 3,
            Self::Query(_) => 4,
            Self::Cancel(_) => 5,
            Self::Progress { .. } => 6,
        }
    }
    fn build(self) -> bool {
        matches!(self, Self::Prepare(_) | Self::Commit(_))
    }
    fn observe(self) -> bool {
        matches!(self, Self::Observe(_) | Self::Progress { .. })
    }
    fn plan(self) -> Option<&'a WorkshopPlan> {
        match self {
            Self::Prepare(p) | Self::Commit(p) | Self::Query(p) | Self::Cancel(p) => Some(p),
            _ => None,
        }
    }
    fn encode(self, secret: &[u8], nonce: &[u8; 32]) -> Vec<u8> {
        let mut out = Vec::new();
        bytes(&mut out, 1, secret);
        bytes(&mut out, 2, nonce);
        number(&mut out, 3, 1);
        number(&mut out, 4, 20);
        let selection = match self {
            Self::Observe(s) => Some(s),
            Self::Prepare(p) => Some(p.before().selection()),
            _ => None,
        };
        if let Some(s) = selection {
            let [x, y, z] = s.center();
            for (tag, n) in [(5, s.item_id()), (6, x), (7, y), (8, z)] {
                number(&mut out, tag, u64::from(n));
            }
        }
        if let Some(plan) = self.plan() {
            bytes(&mut out, 9, plan.key().as_bytes());
            if matches!(self, Self::Prepare(_)) {
                bytes(&mut out, 10, plan.before().witness().as_bytes());
            }
            bytes(&mut out, 11, plan.digest().as_bytes());
            if matches!(self, Self::Commit(_) | Self::Cancel(_)) {
                bytes(&mut out, 12, plan.token());
            }
        }
        if let Self::Progress { building, item } = self {
            number(&mut out, 5, u64::from(item));
            number(&mut out, 13, u64::from(building));
        }
        out
    }
}
struct Reply {
    observation: Option<Vec<u8>>,
    record: Option<WorkshopRecord>,
    replayed: Option<bool>,
}

/// Fixed native operations behind the durable coordinator. The effect shell
/// enforces credentials, cancellation, absolute connection bounds and the
/// shrinking per-operation deadline at send time.
pub trait WorkshopSource {
    fn manifest(&self) -> &WorkshopManifest;
    fn native_summary(&self) -> WorkshopNativeSummary;
    fn fence(&mut self);
    fn observe(
        &mut self,
        selection: WorkshopSelection,
        context: &OperationContext,
        timeout: Duration,
    ) -> Result<WorkshopCapture>;
    fn prepare(
        &mut self,
        plan: &WorkshopPlan,
        context: &OperationContext,
        timeout: Duration,
    ) -> Result<WorkshopPreparation>;
    fn commit(
        &mut self,
        dispatch: WorkshopDispatch<'_>,
        context: &OperationContext,
        timeout: Duration,
    ) -> Result<WorkshopRecord>;
    fn query(
        &mut self,
        plan: &WorkshopPlan,
        context: &OperationContext,
        timeout: Duration,
    ) -> Result<Option<WorkshopRecord>>;
    fn cancel(
        &mut self,
        plan: &WorkshopPlan,
        context: &OperationContext,
        timeout: Duration,
    ) -> Result<WorkshopRecord>;
    fn progress(
        &mut self,
        building: u32,
        item: u32,
        context: &OperationContext,
        timeout: Duration,
    ) -> Result<WorkshopProgress>;
}

/// One authenticated foreground connection. Nonces must be fresh per connection
/// and are explicit so runtime entropy and deterministic tests stay distinct.
pub struct WorkshopRpc {
    link: Link,
    secret: Vec<u8>,
    nonce: [u8; 32],
    fortress: FortressIdentity,
    permission: WorkshopPermission,
    methods: [i16; 7],
    manifest: WorkshopManifest,
    retained: u64,
    unresolved: bool,
    fenced: bool,
    prepare_attempted: bool,
    prepared: Option<WorkshopPlan>,
    seen: BTreeMap<String, WorkshopRecord>,
}
impl WorkshopRpc {
    #[allow(clippy::too_many_arguments)]
    pub fn connect(
        endpoint: SocketAddr,
        secret: Vec<u8>,
        nonce: [u8; 32],
        fortress: FortressIdentity,
        context: &OperationContext,
        cancellation: BuildCancellation,
        permission: WorkshopPermission,
    ) -> Result<Self> {
        if !endpoint.is_ipv4()
            || !endpoint.ip().is_loopback()
            || endpoint.port() == 0
            || !(32..=256).contains(&secret.len())
            || nonce == [0; 32]
        {
            return Err(denied());
        }
        let allowance = timeout(
            context,
            Duration::from_millis(context.budget.max_wall_millis),
        )?;
        let check = || {
            permission(false)?;
            authority(context, &fortress, false, false, context.anchor.tick.get())
        };
        let mut link = Link::connect_workshop(
            endpoint,
            allowance,
            context.budget.max_bytes,
            cancellation,
            &check,
        )?;
        link.greeting(&check)?;
        let mut methods = [0; 7];
        for (index, name) in METHODS.iter().enumerate() {
            let mut request = Vec::new();
            bytes(&mut request, 1, name.as_bytes());
            bytes(&mut request, 2, b"dfmcp.workshop.v1_20.Request");
            bytes(&mut request, 3, b"dfmcp.workshop.v1_20.Reply");
            bytes(&mut request, 4, b"dfmcp_workshop_v1_20");
            let raw = link.frame_bounded(0, &request, 64, &check)?;
            let reply = Message::parse(&raw, 1)?;
            reply.exact(&[1])?;
            let method = reply.number(1)?;
            require(
                (2..=32767).contains(&method) && !methods[..index].contains(&(method as i16)),
                "invalid or aliased workshop method binding",
            )?;
            methods[index] = method as i16;
        }
        let mut out = Self {
            link,
            secret,
            nonce,
            fortress: fortress.clone(),
            permission,
            methods,
            manifest: WorkshopManifest {
                endpoint,
                fortress,
                generation: 0,
                df_version: String::new(),
                dfhack_version: String::new(),
            },
            retained: 0,
            unresolved: false,
            fenced: false,
            prepare_attempted: false,
            prepared: None,
            seen: BTreeMap::new(),
        };
        let result = out.call(Request::Handshake, context, allowance);
        out.finish(result)?;
        Ok(out)
    }
    pub fn is_fenced(&self) -> bool {
        self.fenced
    }
    fn finish<T>(&mut self, result: Result<T>) -> Result<T> {
        if result.is_err() {
            self.fence();
        }
        result
    }
    fn call(
        &mut self,
        request: Request<'_>,
        context: &OperationContext,
        duration: Duration,
    ) -> Result<Reply> {
        require(!self.fenced, "workshop connection is fenced")?;
        self.link
            .narrow(timeout(context, duration)?, context.budget.max_bytes)?;
        let permission = &self.permission;
        let fortress = &self.fortress;
        let check = || {
            permission(request.build())?;
            authority(
                context,
                fortress,
                request.observe(),
                request.build(),
                request
                    .plan()
                    .map_or(context.anchor.tick.get(), |p| p.before().tick()),
            )
        };
        let raw = self.link.frame_bounded(
            self.methods[request.index()],
            &request.encode(&self.secret, &self.nonce),
            8192,
            &check,
        )?;
        let reply = Message::parse(&raw, 13)?;
        require(
            reply.bytes(3)? == self.nonce && reply.number(4)? == 1 && reply.number(5)? == 20,
            "workshop reply nonce or profile differs",
        )?;
        if !reply.boolean(1)? {
            reply.exact(&[1, 2, 3, 4, 5, 6, 7, 8])?;
            require(
                (1..=8).contains(&reply.number(2)?)
                    && reply.number(6)? == 0
                    && reply.bytes(7)?.is_empty()
                    && reply.bytes(8)?.is_empty(),
                "noncanonical workshop refusal",
            )?;
            return Err(error(
                ErrorCode::AdapterRejected,
                "native workshop request refused; no effect outcome inferred",
            )
            .retryable(false));
        }
        let mut fields = vec![1, 2, 3, 4, 5, 6, 7, 8, 12, 13];
        if request.observe() {
            fields.push(9);
        }
        if matches!(
            request,
            Request::Prepare(_) | Request::Commit(_) | Request::Cancel(_)
        ) || (matches!(request, Request::Query(_)) && reply.has(10))
        {
            fields.push(10);
        }
        if matches!(request, Request::Prepare(_)) {
            fields.push(11);
        }
        fields.sort_unstable();
        reply.exact(&fields)?;
        require(reply.number(2)? == 0, "workshop success has failure code")?;
        let manifest = WorkshopManifest {
            endpoint: self.manifest.endpoint,
            fortress: self.fortress.clone(),
            generation: reply.number(6)?,
            df_version: text(reply.bytes(7)?, 128)?,
            dfhack_version: text(reply.bytes(8)?, 128)?,
        };
        require(
            manifest.generation > 0 && manifest.generation < u64::MAX,
            "invalid workshop generation",
        )?;
        if self.manifest.generation != 0 {
            require(
                self.manifest == manifest,
                "workshop connection source changed",
            )?;
        }
        let unresolved = reply.boolean(12)?;
        let retained = reply.number(13)?;
        require(
            retained <= u64::from(MAX_NATIVE_RECORDS)
                && retained >= self.retained
                && (retained != 0 || !unresolved),
            "invalid workshop native retention or uncertainty fence",
        )?;
        let observation = if request.observe() {
            Some(reply.bytes(9)?.to_vec())
        } else {
            None
        };
        let record = if reply.has(10) {
            let record = WorkshopRecord::decode(reply.bytes(10)?)?;
            require(
                Some(record.plan()) == request.plan()
                    && retained > 0
                    && record.plan().before().generation() <= manifest.generation,
                "native workshop record belongs to another intent",
            )?;
            require(
                record.phase() != WorkshopPhase::Indeterminate || unresolved,
                "indeterminate workshop effect lost native fence",
            )?;
            require(
                !matches!(request, Request::Commit(_) | Request::Cancel(_))
                    || record.phase() != WorkshopPhase::Prepared,
                "workshop operation did not retire preparation",
            )?;
            Some(record)
        } else {
            None
        };
        // A cleared flag alone never discharges uncertainty on this connection.
        require(
            !self.unresolved
                || unresolved
                || matches!(
                    (request, record.as_ref()),
                    (Request::Query(plan), Some(r)) if r.phase() == WorkshopPhase::Placed
                        && r.plan() == plan && plan.before().generation() == manifest.generation
                ),
            "native workshop uncertainty cleared without original query proof",
        )?;
        let replayed = if matches!(request, Request::Prepare(_)) {
            let replayed = reply.boolean(11)?;
            if !replayed {
                require(
                    record.as_ref().is_some_and(|r| {
                        r.phase() == WorkshopPhase::Prepared
                            && r.plan().before().generation() == manifest.generation
                    }) && !unresolved,
                    "fresh workshop preparation contains historical or unresolved work",
                )?;
            }
            Some(replayed)
        } else {
            None
        };
        (self.permission)(false)?;
        self.link
            .narrow(timeout(context, duration)?, context.budget.max_bytes)?;
        self.manifest = manifest;
        self.retained = retained;
        self.unresolved = unresolved;
        Ok(Reply {
            observation,
            record,
            replayed,
        })
    }
    fn keyed(
        &mut self,
        request: Request<'_>,
        context: &OperationContext,
        duration: Duration,
    ) -> Result<Reply> {
        let plan = request.plan().ok_or_else(malformed)?;
        require(
            plan.before().fortress() == &self.fortress
                && plan.before().generation() <= self.manifest.generation
                && (!request.build() || plan.before().generation() == self.manifest.generation),
            "workshop plan outside connection scope",
        )?;
        let prior = (self.retained, self.unresolved);
        let reply = self.call(request, context, duration)?;
        let checked: Result<()> = match (&reply.record, self.seen.get(plan.key())) {
            (Some(record), Some(old)) => old.validate_successor(record),
            (Some(_), None) => require(
                self.seen.len() < 8,
                "workshop connection record allowance exhausted",
            ),
            (None, Some(_)) => Err(error(
                ErrorCode::AdapterRejected,
                "known native workshop record disappeared",
            )),
            (None, None) => Ok(()),
        };
        if let Err(cause) = checked {
            self.retained = prior.0;
            self.unresolved = prior.1;
            return Err(cause);
        }
        if let Some(record) = &reply.record {
            self.seen.insert(plan.key().to_owned(), record.clone());
        }
        Ok(reply)
    }
}
impl WorkshopSource for WorkshopRpc {
    fn manifest(&self) -> &WorkshopManifest {
        &self.manifest
    }
    fn native_summary(&self) -> WorkshopNativeSummary {
        // call() validated the complete reply and the retention bound.
        WorkshopNativeSummary {
            unresolved: self.unresolved,
            retained_records: self.retained as u16,
        }
    }
    fn fence(&mut self) {
        self.fenced = true;
        self.prepared = None;
        self.link.fence();
    }
    fn observe(
        &mut self,
        selection: WorkshopSelection,
        context: &OperationContext,
        duration: Duration,
    ) -> Result<WorkshopCapture> {
        let result = (|| {
            let raw = self
                .call(Request::Observe(selection), context, duration)?
                .observation
                .ok_or_else(malformed)?;
            let capture = WorkshopCapture::decode(&raw)?;
            require(
                capture.generation() == self.manifest.generation
                    && capture.fortress() == &self.fortress
                    && capture.selection() == selection,
                "workshop observation scope differs",
            )?;
            authority(context, &self.fortress, true, false, capture.tick())?;
            Ok(capture)
        })();
        self.finish(result)
    }
    fn prepare(
        &mut self,
        plan: &WorkshopPlan,
        context: &OperationContext,
        duration: Duration,
    ) -> Result<WorkshopPreparation> {
        let result = (|| {
            require(
                !self.prepare_attempted
                    && self.prepared.is_none()
                    && self.seen.is_empty()
                    && !self.unresolved,
                "workshop preparation requires a fresh unencumbered connection",
            )?;
            self.prepare_attempted = true;
            let reply = self.keyed(Request::Prepare(plan), context, duration)?;
            let record = reply.record.ok_or_else(malformed)?;
            let replayed = reply.replayed.ok_or_else(malformed)?;
            if !replayed {
                self.prepared = Some(plan.clone());
            }
            WorkshopPreparation::new(record, replayed)
        })();
        self.finish(result)
    }
    fn commit(
        &mut self,
        permit: WorkshopDispatch<'_>,
        context: &OperationContext,
        duration: Duration,
    ) -> Result<WorkshopRecord> {
        let result = (|| {
            require(
                self.prepared.take().as_ref() == Some(permit.plan()),
                "workshop commit requires this connection's exact fresh preparation",
            )?;
            require(
                permit.journal_head() != Digest32::ZERO,
                "workshop commit needs a synchronized one-use dispatch",
            )?;
            self.keyed(Request::Commit(permit.plan()), context, duration)?
                .record
                .ok_or_else(malformed)
        })();
        self.prepared = None;
        self.finish(result)
    }
    fn query(
        &mut self,
        plan: &WorkshopPlan,
        context: &OperationContext,
        duration: Duration,
    ) -> Result<Option<WorkshopRecord>> {
        self.prepared = None;
        let result = self
            .keyed(Request::Query(plan), context, duration)
            .map(|r| r.record);
        self.finish(result)
    }
    fn cancel(
        &mut self,
        plan: &WorkshopPlan,
        context: &OperationContext,
        duration: Duration,
    ) -> Result<WorkshopRecord> {
        self.prepared = None;
        let result = self
            .keyed(Request::Cancel(plan), context, duration)
            .and_then(|r| r.record.ok_or_else(malformed));
        self.finish(result)
    }
    fn progress(
        &mut self,
        building: u32,
        item: u32,
        context: &OperationContext,
        duration: Duration,
    ) -> Result<WorkshopProgress> {
        let result = (|| {
            require(
                building < MAX_ID && item < MAX_ID,
                "invalid workshop progress request",
            )?;
            let raw = self
                .call(Request::Progress { building, item }, context, duration)?
                .observation
                .ok_or_else(malformed)?;
            let progress = WorkshopProgress::decode(&raw)?;
            require(
                progress.generation() == self.manifest.generation
                    && progress.fortress() == &self.fortress
                    && progress.building_id() == building
                    && progress.item_id() == item,
                "workshop progress scope differs",
            )?;
            authority(context, &self.fortress, true, false, progress.tick())?;
            Ok(progress)
        })();
        self.finish(result)
    }
}

#[cfg(test)]
#[path = "rpc_tests.rs"]
mod tests;
