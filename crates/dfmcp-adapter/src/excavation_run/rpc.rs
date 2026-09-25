//! Fixed excavation-run/1.18 TCP source for the durable coordinator.
//!
//! Public construction requires the isolated operator environment. Recovery
//! negotiates without reading terrain and cannot prepare or commit. No runtime,
//! thread, reconnect, replay, arbitrary RPC selector or subprocess is introduced.
use super::coordinator::{ExcavationBinding, ExcavationDispatch, ExcavationRunSource};
use super::*;
use dfmcp_core::{Capability, OperationContext, RiskTier};
use std::collections::BTreeMap;
use std::io;
use std::net::SocketAddr;
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
use std::time::Duration;

mod codec;
mod link;
use codec::{Message, bytes, number};
use link::{Link, RPC_BYTES};

const OPT_IN: &str = "DFMCP_ALLOW_UNADMITTED_EXCAVATION_RUN_V1_18";
const TOKEN: &str = "DFMCP_EXCAVATION_RUN_TOKEN";
const ENDPOINT: &str = "DFMCP_EXCAVATION_RUN_ENDPOINT";
const CLOCK: &str = "DFMCP_EXCAVATION_RUN_ALLOW_CLOCK";
const METHODS: [&str; 6] = ["Handshake", "ObserveRun", "PrepareRun", "CommitRun", "QueryRun", "CancelRun"];
const MAX_SEEN: usize = 256;

fn malformed() -> dfmcp_core::DfmcpError {
    error(ErrorCode::AdapterRejected, "invalid excavation-run native envelope")
}
fn denied() -> dfmcp_core::DfmcpError {
    error(ErrorCode::CapabilityDenied, "excavation-run operator permission or scope refused")
}
fn budget_error() -> dfmcp_core::DfmcpError {
    error(ErrorCode::BudgetExceeded, "excavation connection deadline or work allowance exhausted")
}
fn cancelled() -> dfmcp_core::DfmcpError {
    error(ErrorCode::CancellationRequested, "excavation connection cancelled; no effect outcome inferred")
}
fn io_error(_: io::Error) -> dfmcp_core::DfmcpError {
    error(ErrorCode::AdapterUnavailable, "excavation transport failed; query original intent, never replay commit")
}

/// The supervising caller can signal this handle while a socket read is blocked.
/// Cancellation is one-way and closes the client, not the independent native stop
/// owner. This handle is not a Cx region, global lease or authority grant.
#[derive(Clone, Default)]
pub struct ExcavationCancellation(Arc<AtomicBool>);
impl ExcavationCancellation {
    pub fn cancel(&self) { self.0.store(true, Ordering::Release); }
    pub fn is_cancelled(&self) -> bool { self.0.load(Ordering::Acquire) }
}

// Intentionally no Debug: neither credentials nor caller environment enter diagnostics.
struct Settings { endpoint: SocketAddr, secret: Vec<u8>, clock: bool }
impl Settings {
    fn environment() -> Result<Self> {
        let mut values = BTreeMap::new();
        for (key, value) in std::env::vars_os() {
            if key.to_string_lossy().starts_with("DFMCP_") {
                let key = key.to_str().ok_or_else(denied)?;
                if ![OPT_IN, TOKEN, ENDPOINT, CLOCK].contains(&key) { return Err(denied()); }
                let value = value.to_str().ok_or_else(denied)?;
                if value.len() > 256 { return Err(denied()); }
                values.insert(key.to_owned(), value.to_owned());
            }
        }
        Self::parse(&values)
    }
    fn parse(values: &BTreeMap<String, String>) -> Result<Self> {
        if values.keys().any(|key| ![OPT_IN, TOKEN, ENDPOINT, CLOCK].contains(&key.as_str()))
            || values.get(OPT_IN).map(String::as_str) != Some("1")
        { return Err(denied()); }
        let token = values.get(TOKEN).ok_or_else(denied)?.as_bytes();
        if !(32..=256).contains(&token.len()) { return Err(denied()); }
        let endpoint = values.get(ENDPOINT).map_or("127.0.0.1:5000", String::as_str);
        let address: SocketAddr = endpoint.parse().map_err(|_| denied())?;
        if !address.is_ipv4() || !address.ip().is_loopback() || address.port() == 0
            || address.to_string() != endpoint { return Err(denied()); }
        let clock = match values.get(CLOCK).map(String::as_str) {
            None | Some("0") => false, Some("1") => true, _ => return Err(denied()),
        };
        Ok(Self { endpoint: address, secret: token.to_vec(), clock })
    }
}
type Permission = Box<dyn Fn(bool) -> Result<()> + Send>;
fn operator() -> Result<(Settings, Permission)> {
    let settings = Settings::environment()?;
    let endpoint = settings.endpoint;
    let secret = settings.secret.clone();
    let permission = Box::new(move |clock| {
        let current = Settings::environment()?;
        if current.endpoint != endpoint || current.secret != secret || (clock && !current.clock) {
            return Err(denied());
        }
        Ok(())
    });
    Ok((settings, permission))
}
fn query_authority(c: &OperationContext, fortress: &FortressIdentity) -> Result<()> {
    if c.anchor.fortress_id != fortress.fortress_id() { return Err(denied()); }
    c.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)
}
fn timeout(c: &OperationContext, requested: Duration) -> Result<Duration> {
    c.budget.validate()?;
    if !(1..=60000).contains(&c.budget.max_wall_millis)
        || requested < Duration::from_millis(1) || requested > Duration::from_secs(60)
    { return Err(budget_error()); }
    Ok(requested.min(Duration::from_millis(c.budget.max_wall_millis)))
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Manifest { generation: u64, df: String, dfhack: String }
#[derive(Clone, Copy)]
enum Request<'a> {
    Handshake, Observe(ExcavationRegion), Prepare(&'a ExcavationRunPlan),
    Commit(&'a ExcavationRunPlan), Query(&'a ExcavationRunPlan), Cancel(&'a ExcavationRunPlan),
}
impl<'a> Request<'a> {
    fn index(self) -> usize {
        match self { Self::Handshake => 0, Self::Observe(_) => 1, Self::Prepare(_) => 2,
            Self::Commit(_) => 3, Self::Query(_) => 4, Self::Cancel(_) => 5 }
    }
    fn clock(self) -> bool { matches!(self, Self::Prepare(_) | Self::Commit(_) | Self::Cancel(_)) }
    fn plan(self) -> Option<&'a ExcavationRunPlan> {
        match self { Self::Prepare(p) | Self::Commit(p) | Self::Query(p) | Self::Cancel(p) => Some(p), _ => None }
    }
    fn encode(self, secret: &[u8], nonce: &[u8; 32]) -> Vec<u8> {
        let mut out = Vec::new();
        bytes(&mut out, 1, secret); bytes(&mut out, 2, nonce);
        number(&mut out, 3, 1); number(&mut out, 4, 18);
        if let Some(plan) = self.plan() {
            bytes(&mut out, 5, plan.key().as_bytes());
            if matches!(self, Self::Prepare(_)) {
                number(&mut out, 6, u64::from(plan.spec().clock().game_ticks()));
                number(&mut out, 7, u64::from(plan.spec().clock().wall_ms()));
                bytes(&mut out, 8, plan.before().canonical_bytes());
            }
            bytes(&mut out, 9, plan.digest().as_bytes());
            if matches!(self, Self::Commit(_) | Self::Cancel(_)) { bytes(&mut out, 10, plan.token()); }
        }
        let region = match self { Self::Observe(r) => Some(r), Self::Prepare(p) => Some(p.before().region()), _ => None };
        if let Some(region) = region {
            for (tag, n) in (11..=15).zip(region.values()) { number(&mut out, tag, u64::from(n)); }
        }
        if let Self::Prepare(p) = self {
            for (tag, n) in (16..=19).zip([p.spec().samples(), p.spec().stable_ticks(), p.spec().interval(), p.spec().max_gap()]) {
                number(&mut out, tag, u64::from(n));
            }
        }
        out
    }
}
struct Reply { capture: Option<ExcavationCapture>, record: Option<ExcavationRunRecord> }
struct Wire {
    link: Link, settings: Settings, permission: Permission, nonce: [u8; 32],
    fortress: FortressIdentity, methods: [i16; 6], manifest: Option<Manifest>, retained: u64,
}
impl Wire {
    fn negotiate(settings: Settings, permission: Permission, nonce: [u8; 32], fortress: FortressIdentity,
        c: &OperationContext, cancellation: ExcavationCancellation, control: bool) -> Result<Self>
    {
        let allowance = timeout(c, Duration::from_millis(c.budget.max_wall_millis))?;
        if c.budget.max_bytes < (if control { 8 } else { 7 }) * RPC_BYTES + 24 { return Err(budget_error()); }
        let check = || { permission(false)?; query_authority(c, &fortress) };
        let mut link = Link::connect(settings.endpoint, allowance, c.budget.max_bytes, cancellation, &check)?;
        link.greeting(&check)?;
        let mut methods = [0; 6];
        for (index, name) in METHODS.iter().enumerate() {
            let mut request = Vec::new();
            bytes(&mut request, 1, name.as_bytes());
            bytes(&mut request, 2, b"dfmcp.excavation_run.v1_18.Request");
            bytes(&mut request, 3, b"dfmcp.excavation_run.v1_18.Reply");
            bytes(&mut request, 4, b"dfmcp_excavation_run_v1_18");
            let raw = link.frame(0, &request, &check)?;
            let reply = Message::parse(&raw, 1)?;
            reply.exact(&[1])?;
            let method = reply.number(1)?;
            require((2..=32767).contains(&method) && !methods[..index].contains(&(method as i16)),
                "invalid or aliased excavation method binding")?;
            methods[index] = method as i16;
        }
        let mut out = Self { link, settings, permission, nonce, fortress, methods, manifest: None, retained: 0 };
        out.call(Request::Handshake, c, allowance, None)?;
        Ok(out)
    }
    fn call(&mut self, request: Request<'_>, c: &OperationContext, duration: Duration,
        binding: Option<&ExcavationBinding>) -> Result<Reply>
    {
        self.link.narrow(timeout(c, duration)?, c.budget.max_bytes)?;
        let permission = &self.permission;
        let fortress = &self.fortress;
        let check = || {
            // Revoking unpause permission must leave authorized safety cancellation available.
            permission(matches!(request, Request::Prepare(_) | Request::Commit(_)))?;
            query_authority(c, fortress)?;
            if request.clock() { c.authorize(Capability::ControlClock, RiskTier::Guarded, &[], None)?; }
            if let Request::Prepare(plan) | Request::Commit(plan) = request {
                super::coordinator::authorize_start(c, binding.ok_or_else(denied)?, plan)?;
            }
            Ok(())
        };
        let raw = self.link.frame(self.methods[request.index()], &request.encode(&self.settings.secret, &self.nonce), &check)?;
        let reply = Message::parse(&raw, 12)?;
        require(reply.bytes(3)? == self.nonce && reply.number(4)? == 1 && reply.number(5)? == 18,
            "excavation reply nonce or profile differs")?;
        if !reply.boolean(1)? {
            reply.exact(&[1, 2, 3, 4, 5, 6, 7, 8])?;
            require((1..=8).contains(&reply.number(2)?) && reply.number(6)? == 0
                && reply.bytes(7)?.is_empty() && reply.bytes(8)?.is_empty(), "noncanonical native refusal")?;
            return Err(error(ErrorCode::AdapterRejected, "native excavation request refused; no effect outcome inferred"));
        }
        let mut fields = vec![1, 2, 3, 4, 5, 6, 7, 8, 11, 12];
        if matches!(request, Request::Observe(_)) { fields.push(9); }
        if matches!(request, Request::Prepare(_) | Request::Commit(_) | Request::Cancel(_))
            || (matches!(request, Request::Query(_)) && reply.has(10)) { fields.push(10); }
        reply.exact(&fields)?;
        require(reply.number(2)? == 0, "native success has failure code")?;
        let manifest = Manifest { generation: reply.number(6)?, df: text(reply.bytes(7)?, 128)?,
            dfhack: text(reply.bytes(8)?, 128)? };
        require(manifest.generation > 0 && manifest.generation < u64::MAX, "invalid native generation")?;
        if let Some(old) = &self.manifest { require(old == &manifest, "excavation connection source changed")?; }
        let owner = reply.boolean(11)?;
        let retained = reply.number(12)?;
        require(retained <= 256 && retained >= self.retained && (retained != 0 || !owner),
            "invalid native retention or ownership")?;
        let capture = if let Request::Observe(region) = request {
            let capture = ExcavationCapture::decode(reply.bytes(9)?)?;
            require(capture.generation() == manifest.generation && capture.fortress() == &self.fortress
                && capture.region() == region, "excavation observation scope differs")?;
            Some(capture)
        } else { None };
        let record = if reply.has(10) {
            let record = ExcavationRunRecord::decode(reply.bytes(10)?)?;
            require(Some(record.plan()) == request.plan() && retained > 0
                && record.plan().before().generation() <= manifest.generation,
                "native excavation record belongs to another intent")?;
            require(record.terminal() || record.plan().before().generation() == manifest.generation,
                "old-source active excavation evidence")?;
            require(!matches!(record.phase(), RunPhase::Running | RunPhase::Stopping) || owner,
                "unowned active excavation record")?;
            Some(record)
        } else { None };
        self.manifest = Some(manifest); self.retained = retained;
        Ok(Reply { capture, record })
    }
}

/// A caller-owned foreground source. The nonce must be fresh for each connection;
/// it is explicit so the runtime can use its own entropy provider and replay lab.
pub struct ExcavationRpc {
    wire: Wire,
    binding: ExcavationBinding,
    region: ExcavationRegion,
    initial: Option<ExcavationCapture>,
    control: bool,
    fenced: bool,
    prepare_attempted: bool,
    prepared: Option<ExcavationRunPlan>,
    seen: BTreeMap<String, ExcavationRunRecord>,
}
impl ExcavationRpc {
    pub fn connect_control(fortress: FortressIdentity, region: ExcavationRegion, nonce: [u8; 32],
        c: &OperationContext, cancellation: ExcavationCancellation) -> Result<Self>
    {
        let (settings, permission) = operator()?;
        let wire = Wire::negotiate(settings, permission, nonce, fortress, c, cancellation, true)?;
        Self::bootstrap(wire, region, None, c)
    }
    pub fn connect_recovery(original: &ExcavationBinding, region: ExcavationRegion, nonce: [u8; 32],
        c: &OperationContext, cancellation: ExcavationCancellation) -> Result<Self>
    {
        let (settings, permission) = operator()?;
        if settings.endpoint != original.endpoint() { return Err(denied()); }
        let wire = Wire::negotiate(settings, permission, nonce, original.fortress().clone(), c, cancellation, false)?;
        Self::bootstrap(wire, region, Some(original), c)
    }
    fn bootstrap(mut wire: Wire, region: ExcavationRegion, original: Option<&ExcavationBinding>,
        c: &OperationContext) -> Result<Self>
    {
        let m = wire.manifest.as_ref().ok_or_else(malformed)?.clone();
        let (binding, initial) = if let Some(original) = original {
            require(m.df == original.df_version() && m.dfhack == original.dfhack_version(),
                "excavation recovery software differs")?;
            // Expected historical folder/site/map scope, NOT a new terrain observation.
            // This mode never enables observe, prepare or commit.
            (original.recovery_generation(m.generation)?, None)
        } else {
            let reply = wire.call(Request::Observe(region), c, Duration::from_millis(c.budget.max_wall_millis), None)?;
            let capture = reply.capture.ok_or_else(malformed)?;
            (ExcavationBinding::new(wire.settings.endpoint, &m.df, &m.dfhack, &capture)?, Some(capture))
        };
        Ok(Self { wire, binding, region, initial, control: original.is_none(), fenced: false,
            prepare_attempted: false, prepared: None, seen: BTreeMap::new() })
    }
    pub fn initial_capture(&self) -> Option<&ExcavationCapture> { self.initial.as_ref() }
    pub fn is_fenced(&self) -> bool { self.fenced }
    fn finish<T>(&mut self, result: Result<T>) -> Result<T> {
        if result.is_err() { self.fence(); }
        result
    }
    fn valid_plan(&self, plan: &ExcavationRunPlan, exact: bool) -> Result<()> {
        if self.fenced { return Err(io_error(io::Error::from(io::ErrorKind::NotConnected))); }
        let before = plan.before();
        require(before.region() == self.region && before.fortress() == self.binding.fortress()
            && before.dimensions() == self.binding.dimensions() && before.generation() <= self.binding.generation()
            && (!exact || before.generation() == self.binding.generation()), "excavation plan outside connection scope")
    }
    fn keyed(&mut self, request: Request<'_>, c: &OperationContext, duration: Duration) -> Result<Option<ExcavationRunRecord>> {
        let plan = request.plan().ok_or_else(malformed)?;
        self.valid_plan(plan, !matches!(request, Request::Query(_)))?;
        let reply = self.wire.call(request, c, duration, Some(&self.binding))?;
        if let Some(record) = &reply.record {
            if let Some(old) = self.seen.get(plan.key()) { old.validate_successor(record)?; }
            else { require(self.seen.len() < MAX_SEEN, "excavation retained-history allowance exhausted")?; }
            self.seen.insert(plan.key().to_owned(), record.clone());
            require(self.seen.len() as u64 <= self.wire.retained, "inconsistent native retained count")?;
        } else { require(!self.seen.contains_key(plan.key()), "retained native excavation record disappeared")?; }
        Ok(reply.record)
    }
}
impl ExcavationRunSource for ExcavationRpc {
    fn binding(&self) -> &ExcavationBinding { &self.binding }
    fn fence(&mut self) { self.fenced = true; self.prepared = None; self.wire.link.fence(); }
    fn observe(&mut self, region: ExcavationRegion, c: &OperationContext, duration: Duration) -> Result<ExcavationCapture> {
        let result = (|| {
            require(self.control && !self.fenced && self.region == region, "excavation observation mode or region refused")?;
            let capture = self.wire.call(Request::Observe(region), c, duration, Some(&self.binding))?.capture.ok_or_else(malformed)?;
            require(capture.dimensions() == self.binding.dimensions(), "excavation map dimensions changed")?;
            Ok(capture)
        })();
        self.finish(result)
    }
    fn prepare(&mut self, plan: &ExcavationRunPlan, c: &OperationContext, duration: Duration) -> Result<ExcavationRunRecord> {
        let result = (|| {
            require(self.control && !self.prepare_attempted, "excavation preparation is single-use and control-only")?;
            self.prepare_attempted = true;
            let record = self.keyed(Request::Prepare(plan), c, duration)?.ok_or_else(malformed)?;
            require(record.phase() == RunPhase::Prepared, "native excavation intent was not prepared")?;
            self.prepared = Some(plan.clone());
            Ok(record)
        })();
        self.finish(result)
    }
    fn commit(&mut self, permit: ExcavationDispatch<'_>, c: &OperationContext, duration: Duration) -> Result<ExcavationRunRecord> {
        let prepared = self.prepared.take(); // Consume BEFORE any authorization check or socket I/O.
        let result = (|| {
            require(self.control && prepared.as_ref() == Some(permit.plan()) && permit.journal_head() != Digest32::ZERO,
                "no same-connection preparation and durable one-use excavation dispatch")?;
            self.keyed(Request::Commit(permit.plan()), c, duration)?.ok_or_else(malformed)
        })();
        self.finish(result)
    }
    fn query(&mut self, plan: &ExcavationRunPlan, c: &OperationContext, duration: Duration) -> Result<Option<ExcavationRunRecord>> {
        let result = self.keyed(Request::Query(plan), c, duration);
        self.finish(result)
    }
    fn cancel(&mut self, plan: &ExcavationRunPlan, c: &OperationContext, duration: Duration) -> Result<ExcavationRunRecord> {
        self.prepared = None;
        let result = (|| {
            let record = self.keyed(Request::Cancel(plan), c, duration)?.ok_or_else(malformed)?;
            require(record.phase() != RunPhase::Prepared, "native cancellation retained preparation")?;
            Ok(record)
        })();
        self.finish(result)
    }
}

#[cfg(test)]
mod tests;
