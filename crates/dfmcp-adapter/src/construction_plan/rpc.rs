//! One foreground read-only receipt bracket around a complete operations/1.4
//! capture. This client binds exactly four existing native methods and owns no
//! preparation, effect, reconnect, polling, or publication permission.

use super::{Goal, LinkedSample, Manifest};
use crate::build_placement::rpc::BuildCancellation;
use crate::build_placement::rpc::codec::{Message, bytes, number};
use crate::build_placement::rpc::link::Link;
use crate::build_placement::{BuildBinding, BuildNativeSummary, BuildRecord};
use crate::live_jobs_rpc::operations::paged::{
    PagedOperationsLimits, SourceManifest, acquire_bound, handshake_bound,
};
use dfmcp_core::{Capability, DfmcpError, ErrorCode, GameTick, OperationContext, Result, RiskTier};
use std::cell::Cell;
use std::time::{Duration, Instant};

/// The trusted host pins operator configuration and current runtime authority.
/// This callback has no write mode: none of the bound methods can place, retire,
/// cancel a game job, or change game time. Credentials never come from a goal.
pub type ReadPermission = Box<dyn Fn() -> Result<()> + Send>;

const BINDINGS: [(&str, &str, &str); 4] = [
    ("Handshake", "dfmcp.build.v1_19", "dfmcp_build_v1_19"),
    ("QueryPlacement", "dfmcp.build.v1_19", "dfmcp_build_v1_19"),
    (
        "Handshake",
        "dfmcp.operations.v1_4",
        "dfmcp_operations_v1_4",
    ),
    (
        "ReadObservation",
        "dfmcp.operations.v1_4",
        "dfmcp_operations_v1_4",
    ),
];

fn invalid(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::AdapterRejected, message).retryable(false)
}
fn require(condition: bool, message: &str) -> Result<()> {
    if condition {
        Ok(())
    } else {
        Err(invalid(message))
    }
}
fn exhausted() -> DfmcpError {
    DfmcpError::new(
        ErrorCode::BudgetExceeded,
        "construction acquisition allowance exhausted",
    )
    .retryable(false)
}

struct Work<'a> {
    context: &'a OperationContext,
    original: &'a BuildBinding,
    cancellation: &'a BuildCancellation,
    permission: &'a ReadPermission,
    deadline: Instant,
    high_tick: Cell<u64>,
}
impl Work<'_> {
    fn remaining(&self) -> Result<Duration> {
        let duration = self
            .deadline
            .checked_duration_since(Instant::now())
            .ok_or_else(exhausted)?;
        if duration < Duration::from_millis(1) {
            return Err(exhausted());
        }
        Ok(duration)
    }
    fn current(&self) -> Result<OperationContext> {
        let mut context = self.context.clone();
        context.anchor.tick = GameTick(context.anchor.tick.get().max(self.high_tick.get()));
        context.budget.max_wall_millis = self.remaining()?.as_millis() as u64;
        Ok(context)
    }
    fn check(&self) -> Result<()> {
        self.cancellation.check()?;
        (self.permission)()?;
        let current = self.current()?;
        if current.anchor.fortress_id != self.original.fortress().fortress_id() {
            return Err(DfmcpError::new(
                ErrorCode::CapabilityDenied,
                "construction query belongs to another fortress",
            ));
        }
        current.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)
    }
}

fn bind(link: &mut Link, check: &dyn Fn() -> Result<()>) -> Result<[i16; 4]> {
    let mut methods = [0; 4];
    for (index, (name, package, plugin)) in BINDINGS.iter().enumerate() {
        let mut request = Vec::new();
        for (tag, value) in [
            (1, (*name).to_owned()),
            (2, format!("{package}.Request")),
            (3, format!("{package}.Reply")),
            (4, (*plugin).to_owned()),
        ] {
            bytes(&mut request, tag, value.as_bytes());
        }
        let raw = link.frame_bounded(0, &request, 1024, check)?;
        let reply = Message::parse(&raw, 1)?;
        reply.exact(&[1])?;
        let id = reply.number(1)?;
        require(
            (2..=32767).contains(&id) && !methods[..index].contains(&(id as i16)),
            "construction read methods are invalid or aliased",
        )?;
        methods[index] = id as i16;
    }
    Ok(methods)
}

fn text(raw: &[u8]) -> Result<String> {
    require(
        !raw.is_empty() && raw.len() <= 128 && !raw.contains(&0),
        "invalid native software identity",
    )?;
    std::str::from_utf8(raw)
        .map(str::to_owned)
        .map_err(|_| invalid("native software identity is not UTF-8"))
}
fn original_manifest(original: &BuildBinding) -> Manifest {
    Manifest {
        generation: original.generation(),
        df_version: original.df_version().to_owned(),
        dfhack_version: original.dfhack_version().to_owned(),
    }
}
fn operations_manifest(source: SourceManifest) -> Manifest {
    Manifest {
        generation: source.generation,
        df_version: source.df,
        dfhack_version: source.dfhack,
    }
}

/// The request and success envelope are the existing furniture/1.19 query
/// subset. Exact immutable record equality is stronger than matching an ID or
/// trusting a projected phase string; BuildRecord decoding remains shared.
fn build_read(
    link: &mut Link,
    method: i16,
    token: &[u8],
    nonce: &[u8; 32],
    record: Option<&BuildRecord>,
    original: &Manifest,
    check: &dyn Fn() -> Result<()>,
) -> Result<(BuildNativeSummary, Option<Vec<u8>>)> {
    let mut request = Vec::new();
    bytes(&mut request, 1, token);
    bytes(&mut request, 2, nonce);
    number(&mut request, 3, 1);
    number(&mut request, 4, 19);
    if let Some(record) = record {
        bytes(&mut request, 10, record.plan().key().as_bytes());
        bytes(&mut request, 12, record.plan().digest().as_bytes());
    }
    let raw = link.frame_bounded(method, &request, 8192, check)?;
    let reply = Message::parse(&raw, 13)?;
    let fields: &[u32] = if record.is_some() {
        &[1, 2, 3, 4, 5, 6, 7, 8, 10, 12, 13]
    } else {
        &[1, 2, 3, 4, 5, 6, 7, 8, 12, 13]
    };
    reply.exact(fields)?;
    require(
        reply.boolean(1)?
            && reply.number(2)? == 0
            && reply.bytes(3)? == nonce
            && reply.number(4)? == 1
            && reply.number(5)? == 19,
        "native receipt query refused or changed nonce/profile",
    )?;
    let source = Manifest {
        generation: reply.number(6)?,
        df_version: text(reply.bytes(7)?)?,
        dfhack_version: text(reply.bytes(8)?)?,
    };
    source.canonical_bytes()?;
    require(
        &source == original,
        "original furniture source is no longer retained",
    )?;
    let summary = BuildNativeSummary::new(
        reply.boolean(12)?,
        u16::try_from(reply.number(13)?)
            .map_err(|_| invalid("invalid native receipt retention"))?,
    )?;
    let retained = if let Some(record) = record {
        let raw = reply.bytes(10)?;
        // The original record has already passed the shared complete canonical
        // decoder when Goal was created. Equality preserves every sealed byte.
        require(
            raw == record.canonical_bytes(),
            "original placed receipt changed or disappeared",
        )?;
        Some(raw.to_vec())
    } else {
        None
    };
    check()?;
    Ok((summary, retained))
}

fn receipts(
    link: &mut Link,
    method: i16,
    token: &[u8],
    nonce: &[u8; 32],
    goal: &Goal,
    original: &Manifest,
    check: &dyn Fn() -> Result<()>,
) -> Result<Vec<Vec<u8>>> {
    let mut records = Vec::with_capacity(goal.records().len());
    for record in goal.records() {
        let (summary, raw) = build_read(link, method, token, nonce, Some(record), original, check)?;
        require(
            usize::from(summary.retained_records()) >= goal.records().len(),
            "native receipt retention does not cover the complete goal",
        )?;
        records.push(raw.ok_or_else(|| invalid("original receipt response is absent"))?);
    }
    Ok(records)
}

/// Acquire once on one foreground connection. The trusted host must first own
/// synchronized read intent and later reverify original source custody before
/// publication. Returned bytes are evidence, never effect or publication
/// authority. The host owns the supplied credentials, nonce and permission check.
#[allow(clippy::too_many_arguments)]
pub fn acquire_trusted(
    original: &BuildBinding,
    goal: &Goal,
    operations_token: Vec<u8>,
    build_token: Vec<u8>,
    nonce: [u8; 32],
    context: &OperationContext,
    cancellation: BuildCancellation,
    permission: ReadPermission,
) -> Result<LinkedSample> {
    context.budget.validate()?;
    require(
        (1..=60_000).contains(&context.budget.max_wall_millis)
            && original.endpoint().is_ipv4()
            && original.endpoint().ip().is_loopback()
            && original.endpoint().port() != 0
            && nonce != [0; 32]
            && [&operations_token, &build_token]
                .iter()
                .all(|value| (32..=256).contains(&value.len()) && !value.contains(&0)),
        "invalid trusted construction endpoint, credentials, nonce, or deadline",
    )?;
    let deadline = Instant::now()
        .checked_add(Duration::from_millis(context.budget.max_wall_millis))
        .ok_or_else(exhausted)?;
    let work = Work {
        context,
        original,
        cancellation: &cancellation,
        permission: &permission,
        deadline,
        high_tick: Cell::new(context.anchor.tick.get()),
    };
    work.check()?;
    require(
        goal.records().len() <= context.budget.max_entities as usize,
        "construction goal exceeds the entity allowance",
    )?;
    for record in goal.records() {
        work.check()?;
        require(
            original.capture_matches(record.plan().before()),
            "construction goal differs from the original source binding",
        )?;
        work.high_tick
            .set(work.high_tick.get().max(record.plan().before().tick()));
    }
    work.check()?;
    let check = || work.check();
    let mut link = Link::connect_construction(
        original.endpoint(),
        work.remaining()?,
        context.budget.max_bytes,
        cancellation.clone(),
        &check,
    )?;
    link.greeting(&check)?;
    let methods = bind(&mut link, &check)?;
    let before = original_manifest(original);
    build_read(
        &mut link,
        methods[0],
        &build_token,
        &nonce,
        None,
        &before,
        &check,
    )?;
    let limits = PagedOperationsLimits::default();
    let source = handshake_bound(&operations_token, &nonce, limits, |request, maximum| {
        link.frame_bounded(methods[2], request, maximum, &check)
    })?;
    require(
        source.df == before.df_version && source.dfhack == before.dfhack_version,
        "native software families disagree on the construction connection",
    )?;
    let before_records = receipts(
        &mut link,
        methods[1],
        &build_token,
        &nonce,
        goal,
        &before,
        &check,
    )?;
    let capture = acquire_bound(
        &operations_token,
        &nonce,
        limits,
        &source,
        true,
        |request, maximum| link.frame_bounded(methods[3], request, maximum, &check),
        |observation| {
            let count = 1usize
                .saturating_add(observation.jobs.jobs.len())
                .saturating_add(observation.buildings.len())
                .saturating_add(observation.items.len());
            if count > context.budget.max_entities as usize {
                return Err(exhausted());
            }
            work.high_tick
                .set(work.high_tick.get().max(observation.jobs.tick().get()));
            work.check()
        },
    )?;
    // The shared pager has fully decoded this roster before release. Do not
    // retain a second semantic copy while querying the trailing receipts.
    drop(capture.observation);
    let after_records = receipts(
        &mut link,
        methods[1],
        &build_token,
        &nonce,
        goal,
        &before,
        &check,
    )?;
    let sample = LinkedSample {
        before: before.clone(),
        before_records,
        operations: operations_manifest(capture.manifest),
        capture: capture.payload,
        after: before,
        after_records,
    };
    // Every record and manifest was checked above and the raw capture passed the
    // shared strict decoder. The durable owner separately validates its complete
    // state transition before publication; acquisition need not decode twice.
    work.check()?;
    Ok(sample)
}

#[cfg(test)]
#[path = "rpc_tests.rs"]
mod tests;
