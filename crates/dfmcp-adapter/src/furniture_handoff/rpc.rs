//! One complete, authenticated operations/1.4 read for inventory allocation.
//! Only Handshake and ReadObservation are bound; this foreground owner has no
//! preparation, placement, reconnect or background polling path.

use crate::build_placement::rpc::BuildCancellation;
use crate::build_placement::rpc::codec::{Message, bytes};
use crate::build_placement::rpc::link::Link;
use crate::live_jobs_rpc::operations::paged::{
    PagedOperationsLimits, acquire_bound, handshake_bound,
};
use crate::live_operations::{LiveOperationsObservation, LiveOperationsState, OperationsProfile};
use crate::order_run::FortressIdentity;
use dfmcp_core::{Capability, DfmcpError, ErrorCode, GameTick, OperationContext, Result, RiskTier};
use std::cell::Cell;
use std::net::SocketAddr;
use std::time::{Duration, Instant};

/// The runtime must keep its pinned operator configuration and current request
/// authority valid throughout the read and subsequent handoff publication.
pub type ReadPermission = Box<dyn Fn() -> Result<()> + Send>;

/// Whole connection allowance, including bindings, headers and notifications.
pub const MAX_NETWORK_BYTES: u64 = 20 * 1024 * 1024;
/// The fortress root plus every possible job, building and item in this profile.
pub const MAX_ENTITIES: u32 = 1 + 4096 + 4096 + 65536;

fn invalid(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::AdapterRejected, message).retryable(false)
}
fn require(condition: bool, message: &str) -> Result<()> {
    if condition { Ok(()) } else { Err(invalid(message)) }
}
fn exhausted() -> DfmcpError {
    DfmcpError::new(
        ErrorCode::BudgetExceeded,
        "furniture inventory read allowance exhausted",
    )
    .retryable(false)
}

struct Work<'a> {
    context: &'a OperationContext,
    fortress: &'a FortressIdentity,
    cancellation: &'a BuildCancellation,
    permission: &'a ReadPermission,
    deadline: Instant,
    high_tick: Cell<u64>,
}
impl Work<'_> {
    fn remaining(&self) -> Result<Duration> {
        let remaining = self
            .deadline
            .checked_duration_since(Instant::now())
            .ok_or_else(exhausted)?;
        if remaining < Duration::from_millis(1) {
            return Err(exhausted());
        }
        Ok(remaining)
    }
    fn check(&self) -> Result<()> {
        self.cancellation.check()?;
        (self.permission)()?;
        let mut context = self.context.clone();
        context.budget.max_wall_millis = self.remaining()?.as_millis() as u64;
        context.anchor.tick = GameTick(self.high_tick.get());
        if context.anchor.fortress_id != self.fortress.fortress_id() {
            return Err(DfmcpError::new(
                ErrorCode::CapabilityDenied,
                "furniture inventory query belongs to another fortress",
            ));
        }
        context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)
    }
}

fn bind(link: &mut Link, check: &dyn Fn() -> Result<()>) -> Result<[i16; 2]> {
    let mut methods = [0; 2];
    for (index, method) in ["Handshake", "ReadObservation"].iter().enumerate() {
        let mut request = Vec::new();
        for (tag, value) in [
            (1, *method),
            (2, "dfmcp.operations.v1_4.Request"),
            (3, "dfmcp.operations.v1_4.Reply"),
            (4, "dfmcp_operations_v1_4"),
        ] {
            bytes(&mut request, tag, value.as_bytes());
        }
        let raw = link.frame_bounded(0, &request, 1024, check)?;
        let reply = Message::parse(&raw, 1)?;
        reply.exact(&[1])?;
        let id = reply.number(1)?;
        require(
            (2..=32767).contains(&id) && !methods[..index].contains(&(id as i16)),
            "furniture inventory read methods are invalid or aliased",
        )?;
        methods[index] = id as i16;
    }
    Ok(methods)
}

/// Own the native connection only through acquisition and verified release.
/// Returning owned semantic data drops the link before any canonical graph
/// construction. Local projection must not keep an idle native source alive.
/// The caller retains Work, so ending network ownership renews neither its
/// deadline nor its authority and does not grant publication permission.
fn read_observation(
    endpoint: SocketAddr,
    token: &[u8],
    nonce: &[u8; 32],
    work: &Work<'_>,
) -> Result<LiveOperationsObservation> {
    let check = || work.check();
    let mut link = Link::connect_furniture_allocation(
        endpoint,
        work.remaining()?,
        work.context.budget.max_bytes.min(MAX_NETWORK_BYTES),
        work.cancellation.clone(),
        &check,
    )?;
    link.greeting(&check)?;
    let methods = bind(&mut link, &check)?;
    let limits = PagedOperationsLimits::default();
    let source = handshake_bound(token, nonce, limits, |request, maximum| {
        link.frame_bounded(methods[0], request, maximum, &check)
    })?;
    let capture = acquire_bound(
        token,
        nonce,
        limits,
        &source,
        true,
        |request, maximum| link.frame_bounded(methods[1], request, maximum, &check),
        |observation| {
            let entities = 1usize
                .saturating_add(observation.jobs.jobs.len())
                .saturating_add(observation.buildings.len())
                .saturating_add(observation.items.len());
            if entities > work.context.budget.max_entities.min(MAX_ENTITIES) as usize {
                return Err(exhausted());
            }
            require(
                observation.jobs.world_folder == work.fortress.folder()
                    && observation.jobs.site_id == work.fortress.site() as i32
                    && observation.jobs.fortress_id()? == work.fortress.fortress_id(),
                "furniture inventory capture belongs to another fortress",
            )?;
            let tick = observation.jobs.tick().get();
            require(
                tick >= work.context.anchor.tick.get(),
                "furniture inventory capture regressed before the current tick floor",
            )?;
            work.high_tick.set(tick);
            work.check()
        },
    )?;
    Ok(capture.observation)
}

/// Acquire and publish one immutable native capture into a fresh paged state.
/// The source generation is never rewritten. Complete strict decoding and a
/// verified capture release precede projection. Returning this historical state
/// grants no reservation, placement permission, or enduring publication right.
///
/// The host supplies a shrinking portion of its complete operation budget and
/// retains the same permission/cancellation checks through allocation, durable
/// handoff and final result rendering. Network bytes are bounded by the lesser
/// of the supplied allowance and MAX_NETWORK_BYTES; the fixed profile contains
/// at most MAX_ENTITIES entities, including the fortress root.
pub fn acquire_trusted(
    endpoint: SocketAddr,
    fortress: &FortressIdentity,
    token: Vec<u8>,
    nonce: [u8; 32],
    context: &OperationContext,
    cancellation: BuildCancellation,
    permission: ReadPermission,
) -> Result<LiveOperationsState> {
    context.budget.validate()?;
    require(
        endpoint.is_ipv4()
            && endpoint.ip().is_loopback()
            && endpoint.port() != 0
            && (1..=60_000).contains(&context.budget.max_wall_millis)
            && (32..=256).contains(&token.len())
            && !token.contains(&0)
            && nonce != [0; 32],
        "invalid trusted furniture inventory endpoint, credential, nonce or deadline",
    )?;
    let work = Work {
        context,
        fortress,
        cancellation: &cancellation,
        permission: &permission,
        deadline: Instant::now()
            .checked_add(Duration::from_millis(context.budget.max_wall_millis))
            .ok_or_else(exhausted)?,
        high_tick: Cell::new(context.anchor.tick.get()),
    };
    work.check()?;
    let observation = read_observation(endpoint, &token, &nonce, &work)?;
    // Network ownership ended, not request ownership. Recheck current authority
    // and the original shrinking deadline before and after local projection.
    work.check()?;
    let mut state = LiveOperationsState::with_profile(OperationsProfile::PagedV1_4);
    state.publish(observation)?;
    work.check()?;
    Ok(state)
}

#[cfg(test)]
#[path = "rpc_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "rpc_lifecycle_tests.rs"]
mod lifecycle_tests;
