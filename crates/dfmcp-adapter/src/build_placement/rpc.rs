//! Fixed furniture/1.19 foreground TCP with current authority and one-shot dispatch.
//!
//! One connection owns its deadline, source, selection and fresh preparation.
//! Recovery connections cannot observe, prepare or commit. No runtime, thread,
//! subprocess, reconnect, arbitrary native method or production admission is added.
use super::journal::{BuildDispatch, BuildSource};
use super::*;
use dfmcp_core::{Capability, GameTick, OperationContext, RiskTier};
use std::collections::BTreeMap;
use std::io;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

mod codec;
mod link;
use codec::{Message, bytes, number};
use link::Link;

const OPT_IN: &str = "DFMCP_ALLOW_UNADMITTED_BUILD_V1_19";
const TOKEN: &str = "DFMCP_BUILD_TOKEN";
const ENDPOINT: &str = "DFMCP_BUILD_ENDPOINT";
const PLACE: &str = "DFMCP_BUILD_ALLOW_PLACE";
const METHODS: [&str; 6] = [
    "Handshake",
    "ReadPlacement",
    "PreparePlacement",
    "CommitPlacement",
    "QueryPlacement",
    "CancelPlacement",
];

fn malformed() -> dfmcp_core::DfmcpError {
    error(
        ErrorCode::AdapterRejected,
        "invalid furniture native envelope",
    )
}
fn denied() -> dfmcp_core::DfmcpError {
    error(
        ErrorCode::CapabilityDenied,
        "furniture operator permission or scope refused",
    )
}
fn budget_error() -> dfmcp_core::DfmcpError {
    error(
        ErrorCode::BudgetExceeded,
        "furniture connection deadline or work allowance exhausted",
    )
}
fn cancelled() -> dfmcp_core::DfmcpError {
    error(
        ErrorCode::CancellationRequested,
        "furniture connection cancelled; no effect outcome inferred",
    )
}
fn io_error(_: io::Error) -> dfmcp_core::DfmcpError {
    error(
        ErrorCode::AdapterUnavailable,
        "furniture transport failed; query original intent, never replay commit",
    )
    .retryable(false)
}

type CancellationCheck = Arc<dyn Fn() -> Result<()> + Send + Sync>;
/// A one-way cancellation signal plus an optional runtime-owned current-context
/// check. A dynamic check can follow successive supervised MCP calls without
/// extending the original connection deadline or retaining a completed Cx.
#[derive(Clone, Default)]
pub struct BuildCancellation {
    cancelled: Arc<AtomicBool>,
    check: Option<CancellationCheck>,
}
impl BuildCancellation {
    pub fn with_check(check: CancellationCheck) -> Self {
        Self {
            cancelled: Arc::default(),
            check: Some(check),
        }
    }
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
    fn check(&self) -> Result<()> {
        if self.is_cancelled() {
            return Err(cancelled());
        }
        if let Some(check) = &self.check {
            check()?;
        }
        Ok(())
    }
}

/// The trusted host repeats its pinned environment and runtime checks here.
/// `true` is used only for native prepare/commit, so revocation still permits
/// authenticated query and preparation retirement. Never captures an MCP input.
pub type BuildPermission = Box<dyn Fn(bool) -> Result<()> + Send>;

// No Debug: credentials and environment values must not enter diagnostics.
struct Settings {
    endpoint: SocketAddr,
    secret: Vec<u8>,
    placement: bool,
}
impl Settings {
    fn environment() -> Result<Self> {
        let mut values = BTreeMap::new();
        for (key, value) in std::env::vars_os() {
            if key.to_string_lossy().starts_with("DFMCP_") {
                let key = key.to_str().ok_or_else(denied)?;
                if ![OPT_IN, TOKEN, ENDPOINT, PLACE].contains(&key) {
                    return Err(denied());
                }
                let value = value.to_str().ok_or_else(denied)?;
                if value.len() > 256 {
                    return Err(denied());
                }
                values.insert(key.to_owned(), value.to_owned());
            }
        }
        Self::parse(&values)
    }
    fn parse(values: &BTreeMap<String, String>) -> Result<Self> {
        if values
            .keys()
            .any(|key| ![OPT_IN, TOKEN, ENDPOINT, PLACE].contains(&key.as_str()))
            || values.get(OPT_IN).map(String::as_str) != Some("1")
        {
            return Err(denied());
        }
        let secret = values.get(TOKEN).ok_or_else(denied)?.as_bytes().to_vec();
        let endpoint = values
            .get(ENDPOINT)
            .map_or("127.0.0.1:5000", String::as_str);
        let address: SocketAddr = endpoint.parse().map_err(|_| denied())?;
        validate_settings(address, &secret)?;
        if address.to_string() != endpoint {
            return Err(denied());
        }
        let placement = match values.get(PLACE).map(String::as_str) {
            None | Some("0") => false,
            Some("1") => true,
            _ => return Err(denied()),
        };
        Ok(Self {
            endpoint: address,
            secret,
            placement,
        })
    }
}
fn validate_settings(endpoint: SocketAddr, secret: &[u8]) -> Result<()> {
    if !endpoint.is_ipv4()
        || !endpoint.ip().is_loopback()
        || endpoint.port() == 0
        || !(32..=256).contains(&secret.len())
    {
        return Err(denied());
    }
    Ok(())
}
fn operator() -> Result<(Settings, BuildPermission)> {
    let settings = Settings::environment()?;
    let endpoint = settings.endpoint;
    let secret = settings.secret.clone();
    let permission = Box::new(move |placement| {
        let current = Settings::environment()?;
        if current.endpoint != endpoint
            || current.secret != secret
            || (placement && !current.placement)
        {
            return Err(denied());
        }
        Ok(())
    });
    Ok((settings, permission))
}

/// Complete native building/job registries and the distant exact item are part
/// of the footprint. This profile requires fortress-wide grants; it cannot
/// pretend that a target-tile grant covers the global ID and item relation writes.
fn authority(
    context: &OperationContext,
    fortress: &FortressIdentity,
    observe: bool,
    place: bool,
    tick: u64,
) -> Result<()> {
    if context.anchor.fortress_id != fortress.fortress_id() {
        return Err(denied());
    }
    let mut current = context.clone();
    current.anchor.tick = GameTick(tick.max(current.anchor.tick.get()));
    current.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    if observe || place {
        current.authorize(Capability::Observe, RiskTier::ReadOnly, &[], None)?;
    }
    if place {
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

#[derive(Clone, Debug, PartialEq, Eq)]
struct Manifest {
    generation: u64,
    df: String,
    dfhack: String,
}
#[derive(Clone, Copy)]
enum Request<'a> {
    Handshake,
    Observe(BuildSelection),
    Prepare(&'a BuildPlan),
    Commit(&'a BuildPlan),
    Query(&'a BuildPlan),
    Cancel(&'a BuildPlan),
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
        }
    }
    fn place(self) -> bool {
        matches!(self, Self::Prepare(_) | Self::Commit(_))
    }
    fn observe(self) -> bool {
        matches!(self, Self::Observe(_))
    }
    fn plan(self) -> Option<&'a BuildPlan> {
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
        number(&mut out, 4, 19);
        let selection = match self {
            Self::Observe(s) => Some(s),
            Self::Prepare(p) => Some(p.before().selection()),
            _ => None,
        };
        if let Some(s) = selection {
            for (tag, n) in (5..=9).zip(s.values()) {
                number(&mut out, tag, u64::from(n));
            }
        }
        if let Some(plan) = self.plan() {
            bytes(&mut out, 10, plan.key().as_bytes());
            if matches!(self, Self::Prepare(_)) {
                bytes(&mut out, 11, plan.before().witness().as_bytes());
            }
            bytes(&mut out, 12, plan.digest().as_bytes());
            if matches!(self, Self::Commit(_) | Self::Cancel(_)) {
                bytes(&mut out, 13, plan.token());
            }
        }
        out
    }
}
struct Reply {
    capture: Option<BuildCapture>,
    record: Option<BuildRecord>,
    replayed: Option<bool>,
}
struct Wire {
    link: Link,
    settings: Settings,
    permission: BuildPermission,
    nonce: [u8; 32],
    fortress: FortressIdentity,
    methods: [i16; 6],
    manifest: Option<Manifest>,
    retained: u64,
    unresolved: bool,
}
impl Wire {
    fn negotiate(
        settings: Settings,
        permission: BuildPermission,
        nonce: [u8; 32],
        fortress: FortressIdentity,
        context: &OperationContext,
        cancellation: BuildCancellation,
    ) -> Result<Self> {
        validate_settings(settings.endpoint, &settings.secret)?;
        let allowance = timeout(
            context,
            Duration::from_millis(context.budget.max_wall_millis),
        )?;
        let check = || {
            permission(false)?;
            authority(context, &fortress, false, false, context.anchor.tick.get())
        };
        let mut link = Link::connect(
            settings.endpoint,
            allowance,
            context.budget.max_bytes,
            cancellation,
            &check,
        )?;
        link.greeting(&check)?;
        let mut methods = [0; 6];
        for (index, name) in METHODS.iter().enumerate() {
            let mut request = Vec::new();
            bytes(&mut request, 1, name.as_bytes());
            bytes(&mut request, 2, b"dfmcp.build.v1_19.Request");
            bytes(&mut request, 3, b"dfmcp.build.v1_19.Reply");
            bytes(&mut request, 4, b"dfmcp_build_v1_19");
            let raw = link.frame(0, &request, &check)?;
            let reply = Message::parse(&raw, 1)?;
            reply.exact(&[1])?;
            let method = reply.number(1)?;
            require(
                (2..=32767).contains(&method) && !methods[..index].contains(&(method as i16)),
                "invalid or aliased furniture method binding",
            )?;
            methods[index] = method as i16;
        }
        let mut out = Self {
            link,
            settings,
            permission,
            nonce,
            fortress,
            methods,
            manifest: None,
            retained: 0,
            unresolved: false,
        };
        out.call(Request::Handshake, context, allowance)?;
        Ok(out)
    }
    fn call(
        &mut self,
        request: Request<'_>,
        context: &OperationContext,
        duration: Duration,
    ) -> Result<Reply> {
        self.link
            .narrow(timeout(context, duration)?, context.budget.max_bytes)?;
        if (request.observe() || request.place()) && context.budget.max_entities < 10 {
            return Err(budget_error());
        }
        let permission = &self.permission;
        let fortress = &self.fortress;
        let check = || {
            permission(request.place())?;
            authority(
                context,
                fortress,
                request.observe(),
                request.place(),
                request
                    .plan()
                    .map_or(context.anchor.tick.get(), |p| p.before().tick()),
            )
        };
        let raw = self.link.frame(
            self.methods[request.index()],
            &request.encode(&self.settings.secret, &self.nonce),
            &check,
        )?;
        let reply = Message::parse(&raw, 13)?;
        require(
            reply.bytes(3)? == self.nonce && reply.number(4)? == 1 && reply.number(5)? == 19,
            "furniture reply nonce or profile differs",
        )?;
        if !reply.boolean(1)? {
            reply.exact(&[1, 2, 3, 4, 5, 6, 7, 8])?;
            require(
                (1..=8).contains(&reply.number(2)?)
                    && reply.number(6)? == 0
                    && reply.bytes(7)?.is_empty()
                    && reply.bytes(8)?.is_empty(),
                "noncanonical furniture refusal",
            )?;
            return Err(error(
                ErrorCode::AdapterRejected,
                "native furniture request refused; no effect outcome inferred",
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
        reply.exact(&fields)?;
        require(reply.number(2)? == 0, "furniture success has failure code")?;
        let manifest = Manifest {
            generation: reply.number(6)?,
            df: text(reply.bytes(7)?, 128)?,
            dfhack: text(reply.bytes(8)?, 128)?,
        };
        require(
            manifest.generation > 0 && manifest.generation < u64::MAX,
            "invalid furniture generation",
        )?;
        if let Some(old) = &self.manifest {
            require(old == &manifest, "furniture connection source changed")?;
        }
        let unresolved = reply.boolean(12)?;
        let retained = reply.number(13)?;
        require(
            retained <= 256
                && retained >= self.retained
                && (retained != 0 || !unresolved)
                && (!self.unresolved || unresolved),
            "invalid furniture native retention or uncertainty fence",
        )?;
        let capture = if let Request::Observe(selection) = request {
            let capture = BuildCapture::decode(reply.bytes(9)?)?;
            require(
                capture.generation() == manifest.generation
                    && capture.fortress() == fortress
                    && capture.selection() == selection,
                "furniture observation scope differs",
            )?;
            authority(context, fortress, true, false, capture.tick())?;
            Some(capture)
        } else {
            None
        };
        let record = if reply.has(10) {
            let record = BuildRecord::decode(reply.bytes(10)?)?;
            require(
                Some(record.plan()) == request.plan()
                    && retained > 0
                    && record.plan().before().generation() <= manifest.generation,
                "native furniture record belongs to another intent",
            )?;
            require(
                record.phase() != BuildPhase::Indeterminate || unresolved,
                "indeterminate furniture effect lost native fence",
            )?;
            require(
                !matches!(request, Request::Commit(_) | Request::Cancel(_))
                    || record.phase() != BuildPhase::Prepared,
                "furniture operation did not retire preparation",
            )?;
            Some(record)
        } else {
            None
        };
        let replayed = if matches!(request, Request::Prepare(_)) {
            let replayed = reply.boolean(11)?;
            if !replayed {
                require(
                    record.as_ref().is_some_and(|r| {
                        r.phase() == BuildPhase::Prepared
                            && r.plan().before().generation() == manifest.generation
                    }) && !unresolved,
                    "fresh furniture preparation contains historical or unresolved work",
                )?;
            }
            Some(replayed)
        } else {
            None
        };
        (self.permission)(false)?;
        self.link
            .narrow(timeout(context, duration)?, context.budget.max_bytes)?;
        self.manifest = Some(manifest);
        self.retained = retained;
        self.unresolved = unresolved;
        Ok(Reply {
            capture,
            record,
            replayed,
        })
    }
}

/// Foreground native source. The trusted host owns current authority, durable
/// journal custody and the supervising runtime region. Nonces must be fresh per
/// connection and are explicit for runtime entropy and deterministic test labs.
pub struct BuildRpc {
    wire: Wire,
    binding: BuildBinding,
    selection: BuildSelection,
    initial: Option<BuildCapture>,
    control: bool,
    fenced: bool,
    prepare_attempted: bool,
    prepared: Option<BuildPlan>,
    seen: BTreeMap<String, BuildRecord>,
}
impl BuildRpc {
    pub fn connect_control(
        fortress: FortressIdentity,
        selection: BuildSelection,
        nonce: [u8; 32],
        context: &OperationContext,
        cancellation: BuildCancellation,
    ) -> Result<Self> {
        let (settings, permission) = operator()?;
        Self::connect_inner(
            settings,
            permission,
            nonce,
            fortress,
            selection,
            context,
            cancellation,
            None,
        )
    }
    pub fn connect_recovery(
        original: &BuildBinding,
        selection: BuildSelection,
        nonce: [u8; 32],
        context: &OperationContext,
        cancellation: BuildCancellation,
    ) -> Result<Self> {
        let (settings, permission) = operator()?;
        require(
            settings.endpoint == original.endpoint(),
            "furniture recovery endpoint differs",
        )?;
        Self::connect_inner(
            settings,
            permission,
            nonce,
            original.fortress().clone(),
            selection,
            context,
            cancellation,
            Some(original),
        )
    }
    /// Trusted composition seam for a stricter host profile. The callback must
    /// reject production admission state and revalidate operator authorization.
    #[allow(clippy::too_many_arguments)]
    pub fn connect_trusted(
        endpoint: SocketAddr,
        token: Vec<u8>,
        nonce: [u8; 32],
        fortress: FortressIdentity,
        selection: BuildSelection,
        context: &OperationContext,
        cancellation: BuildCancellation,
        permission: BuildPermission,
    ) -> Result<Self> {
        Self::connect_inner(
            Settings {
                endpoint,
                secret: token,
                placement: false,
            },
            permission,
            nonce,
            fortress,
            selection,
            context,
            cancellation,
            None,
        )
    }
    #[allow(clippy::too_many_arguments)]
    pub fn connect_trusted_recovery(
        original: &BuildBinding,
        token: Vec<u8>,
        nonce: [u8; 32],
        selection: BuildSelection,
        context: &OperationContext,
        cancellation: BuildCancellation,
        permission: BuildPermission,
    ) -> Result<Self> {
        Self::connect_inner(
            Settings {
                endpoint: original.endpoint(),
                secret: token,
                placement: false,
            },
            permission,
            nonce,
            original.fortress().clone(),
            selection,
            context,
            cancellation,
            Some(original),
        )
    }
    #[allow(clippy::too_many_arguments)]
    fn connect_inner(
        settings: Settings,
        permission: BuildPermission,
        nonce: [u8; 32],
        fortress: FortressIdentity,
        selection: BuildSelection,
        context: &OperationContext,
        cancellation: BuildCancellation,
        original: Option<&BuildBinding>,
    ) -> Result<Self> {
        let mut wire =
            Wire::negotiate(settings, permission, nonce, fortress, context, cancellation)?;
        let m = wire.manifest.as_ref().ok_or_else(malformed)?.clone();
        let (binding, initial) = if let Some(original) = original {
            require(
                m.df == original.df_version() && m.dfhack == original.dfhack_version(),
                "furniture recovery software differs",
            )?;
            (original.recovery_generation(m.generation)?, None)
        } else {
            let reply = wire.call(
                Request::Observe(selection),
                context,
                Duration::from_millis(context.budget.max_wall_millis),
            )?;
            let capture = reply.capture.ok_or_else(malformed)?;
            (
                BuildBinding::new(wire.settings.endpoint, &m.df, &m.dfhack, &capture)?,
                Some(capture),
            )
        };
        Ok(Self {
            wire,
            binding,
            selection,
            initial,
            control: original.is_none(),
            fenced: false,
            prepare_attempted: false,
            prepared: None,
            seen: BTreeMap::new(),
        })
    }
    pub fn initial_capture(&self) -> Option<&BuildCapture> {
        self.initial.as_ref()
    }
    pub fn is_fenced(&self) -> bool {
        self.fenced
    }
    pub fn native_unresolved(&self) -> bool {
        self.wire.unresolved
    }
    pub fn retained_records(&self) -> u64 {
        self.wire.retained
    }
    fn finish<T>(&mut self, result: Result<T>) -> Result<T> {
        if result.is_err() {
            self.fence();
        }
        result
    }
    fn valid_plan(&self, plan: &BuildPlan, exact: bool) -> Result<()> {
        require(
            !self.fenced
                && plan.before().selection() == self.selection
                && plan.before().fortress() == self.binding.fortress()
                && plan.before().dimensions() == self.binding.dimensions()
                && plan.before().generation() <= self.binding.generation()
                && (!exact || plan.before().generation() == self.binding.generation()),
            "furniture plan outside connection scope",
        )
    }
    fn keyed(
        &mut self,
        request: Request<'_>,
        context: &OperationContext,
        duration: Duration,
    ) -> Result<Reply> {
        let plan = request.plan().ok_or_else(malformed)?;
        self.valid_plan(plan, request.place())?;
        let reply = self.wire.call(request, context, duration)?;
        if let Some(record) = &reply.record {
            if let Some(old) = self.seen.get(plan.key()) {
                old.validate_successor(record)?;
            } else {
                require(
                    self.seen.len() < 32,
                    "furniture connection record allowance exhausted",
                )?;
            }
            self.seen.insert(plan.key().to_owned(), record.clone());
            require(
                self.seen.len() as u64 <= self.wire.retained,
                "inconsistent furniture native retained count",
            )?;
        } else {
            require(
                !self.seen.contains_key(plan.key()),
                "known native furniture record disappeared",
            )?;
        }
        Ok(reply)
    }
}
impl BuildSource for BuildRpc {
    fn binding(&self) -> &BuildBinding {
        &self.binding
    }
    fn native_summary(&self) -> BuildNativeSummary {
        // Wire::call validates the complete reply and the 256-record bound
        // before publishing either field.
        BuildNativeSummary {
            unresolved: self.wire.unresolved,
            retained_records: self.wire.retained as u16,
        }
    }
    fn fence(&mut self) {
        self.fenced = true;
        self.prepared = None;
        self.wire.link.fence();
    }
    fn observe(
        &mut self,
        selection: BuildSelection,
        context: &OperationContext,
        duration: Duration,
    ) -> Result<BuildCapture> {
        let result = (|| {
            require(
                self.control && !self.fenced && self.selection == selection,
                "furniture observation mode or selection refused",
            )?;
            let capture = self
                .wire
                .call(Request::Observe(selection), context, duration)?
                .capture
                .ok_or_else(malformed)?;
            require(
                self.binding.capture_matches(&capture),
                "furniture observed source changed",
            )?;
            Ok(capture)
        })();
        self.finish(result)
    }
    fn prepare(
        &mut self,
        plan: &BuildPlan,
        context: &OperationContext,
        duration: Duration,
    ) -> Result<BuildPreparation> {
        let result = (|| {
            require(
                self.control
                    && !self.prepare_attempted
                    && self.prepared.is_none()
                    && self.seen.is_empty()
                    && !self.wire.unresolved,
                "furniture preparation requires a fresh unencumbered connection",
            )?;
            self.prepare_attempted = true;
            let reply = self.keyed(Request::Prepare(plan), context, duration)?;
            let record = reply.record.ok_or_else(malformed)?;
            let replayed = reply.replayed.ok_or_else(malformed)?;
            if !replayed {
                self.prepared = Some(plan.clone());
            }
            BuildPreparation::new(record, replayed)
        })();
        self.finish(result)
    }
    fn commit(
        &mut self,
        permit: BuildDispatch<'_>,
        context: &OperationContext,
        duration: Duration,
    ) -> Result<BuildRecord> {
        let prepared = self.prepared.take();
        let result = (|| {
            require(
                self.control
                    && prepared.as_ref() == Some(permit.plan())
                    && permit.journal_head() != Digest32::ZERO,
                "furniture commit needs same-connection preparation and durable one-use dispatch",
            )?;
            self.keyed(Request::Commit(permit.plan()), context, duration)?
                .record
                .ok_or_else(malformed)
        })();
        self.finish(result)
    }
    fn query(
        &mut self,
        plan: &BuildPlan,
        context: &OperationContext,
        duration: Duration,
    ) -> Result<Option<BuildRecord>> {
        self.prepared = None;
        let result = self
            .keyed(Request::Query(plan), context, duration)
            .map(|r| r.record);
        self.finish(result)
    }
    fn cancel(
        &mut self,
        plan: &BuildPlan,
        context: &OperationContext,
        duration: Duration,
    ) -> Result<BuildRecord> {
        self.prepared = None;
        let result = self
            .keyed(Request::Cancel(plan), context, duration)
            .and_then(|r| r.record.ok_or_else(malformed));
        self.finish(result)
    }
}

#[cfg(test)]
mod tests;
