//! One bounded map/1.5 read for an original mining blueprint monitor.
//!
//! The existing client owns the closed native protocol and typed decoding.
//! This shell adds current authority, one absolute deadline and a shared wire
//! allowance at every stream boundary. It never supplies mutation methods or
//! interprets a raw native tiletype as an excavation result.
//!
//! A complete decoded capture is returned even if its source or newly learned
//! tick differs from the retained monitor. The caller must durably retain that
//! evidence before validating it or performing a final runtime check. Discarding
//! a changed-source capture would allow a later read to hide invalidation.

use super::MonitorSpec;
use crate::live_jobs_rpc::operations::map::MapRpcClient;
use crate::live_map::LiveMapObservation;
use dfmcp_core::{DfmcpError, ErrorCode, GameTick, OperationContext, Result};
use std::io::{self, Read, Write};
use std::time::{Duration, Instant};

/// The whole original enclosure is at most 1,024 cells, including holes.
pub const MAX_CAPTURE_CELLS: usize = 1024;
pub const MAX_PAYLOAD_BYTES: usize = 32 * 1024;
/// Covers the four fixed native calls, including their bounded notifications.
pub const WIRE_BYTES: u64 = 2 * 1024 * 1024;
/// Complete wire, decoding and bounded local capture work, reserved up front.
pub const READ_BYTES: u64 = 4 * 1024 * 1024;

/// An owned stream whose blocking operations honor the supplied absolute limit.
///
/// The host must connect it to `spec.source().endpoint()` in the same supervised
/// operation, with its own current authority and connection budget. Implementors
/// must narrow an existing earlier deadline, never extend it, and must enforce
/// the remaining time on read, write and flush. Setting the deadline performs no
/// native protocol work. This adapter creates no ambient socket or worker.
pub trait ProofStream: Read + Write {
    fn set_deadline(&mut self, deadline: Instant) -> io::Result<()>;
}

fn error(code: ErrorCode, message: &str) -> DfmcpError {
    DfmcpError::new(code, message).retryable(false)
}
fn exhausted() -> DfmcpError {
    error(
        ErrorCode::BudgetExceeded,
        "complete blueprint map read exceeds its bounded allowance",
    )
}
fn retained_context(spec: &MonitorSpec, context: &OperationContext) -> OperationContext {
    let mut current = context.clone();
    current.anchor.tick = GameTick(
        current
            .anchor
            .tick
            .get()
            .max(spec.start_tick())
            .max(spec.minimum_native_tick()),
    );
    current
}

/// Validate the entire read allowance before the host connects or any byte is
/// written. This is only a preflight; `read_map` repeats authority at each I/O.
pub fn preflight(spec: &MonitorSpec, context: &OperationContext) -> Result<()> {
    context.budget.validate()?;
    let cells = spec.region().volume().map_err(|_| {
        error(
            ErrorCode::InvalidRequest,
            "invalid blueprint capture enclosure",
        )
    })?;
    if context.budget.max_wall_millis > 60_000
        || context.budget.max_bytes < READ_BYTES
        || cells > MAX_CAPTURE_CELLS
        || cells > context.budget.max_entities as usize
    {
        return Err(exhausted());
    }
    spec.authorize(&retained_context(spec, context), true)
}

struct Work<'a, G> {
    spec: &'a MonitorSpec,
    context: OperationContext,
    guard: G,
    deadline: Instant,
    bytes: u64,
    failed: Option<DfmcpError>,
}
impl<'a, G: FnMut(&OperationContext) -> Result<()>> Work<'a, G> {
    fn new(spec: &'a MonitorSpec, context: &OperationContext, guard: G) -> Result<Self> {
        let started = Instant::now();
        preflight(spec, context)?;
        let deadline = started
            .checked_add(Duration::from_millis(context.budget.max_wall_millis))
            .ok_or_else(exhausted)?;
        let work = Self {
            spec,
            context: retained_context(spec, context),
            guard,
            deadline,
            bytes: WIRE_BYTES,
            failed: None,
        };
        work.current()?;
        Ok(work)
    }
    fn current(&self) -> Result<OperationContext> {
        if let Some(cause) = &self.failed {
            return Err(cause.clone());
        }
        let remaining = self
            .deadline
            .checked_duration_since(Instant::now())
            .map(|duration| duration.as_millis())
            .filter(|millis| *millis > 0)
            .ok_or_else(exhausted)?;
        if self.bytes == 0 {
            return Err(exhausted());
        }
        let mut current = self.context.clone();
        current.budget.max_wall_millis = u64::try_from(remaining).map_err(|_| exhausted())?;
        current.budget.max_bytes = self.bytes;
        Ok(current)
    }
    fn before<S: ProofStream>(&mut self, stream: &mut S) -> Result<()> {
        let current = self.current()?;
        self.spec.authorize(&current, true)?;
        (self.guard)(&current)?;
        // A guard can consume time. No native I/O may begin after its allowance.
        self.current()?;
        stream.set_deadline(self.deadline).map_err(|_| {
            error(
                ErrorCode::AdapterUnavailable,
                "blueprint map stream could not enforce its deadline",
            )
        })?;
        self.current()?;
        Ok(())
    }
    fn fail(&mut self, cause: DfmcpError) -> io::Error {
        if self.failed.is_none() {
            self.failed = Some(cause);
        }
        io::Error::other("blueprint map I/O boundary refused")
    }
    fn consumed(&mut self, count: usize, maximum: usize) -> io::Result<usize> {
        if count > maximum {
            return Err(self.fail(error(
                ErrorCode::AdapterRejected,
                "blueprint map stream returned an invalid transfer length",
            )));
        }
        self.bytes = self
            .bytes
            .checked_sub(count as u64)
            .ok_or_else(|| self.fail(exhausted()))?;
        Ok(count)
    }
}

struct GuardedStream<'a, 'b, S, G> {
    stream: S,
    work: &'a mut Work<'b, G>,
}
impl<S: ProofStream, G: FnMut(&OperationContext) -> Result<()>> Read
    for GuardedStream<'_, '_, S, G>
{
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if let Err(cause) = self.work.before(&mut self.stream) {
            return Err(self.work.fail(cause));
        }
        let maximum = out.len().min(self.work.bytes as usize);
        let count = self.stream.read(&mut out[..maximum])?;
        // Do not run another authority/runtime check after a completed read:
        // the final bytes may be the very evidence the caller must retain.
        self.work.consumed(count, maximum)
    }
}
impl<S: ProofStream, G: FnMut(&OperationContext) -> Result<()>> Write
    for GuardedStream<'_, '_, S, G>
{
    fn write(&mut self, input: &[u8]) -> io::Result<usize> {
        if let Err(cause) = self.work.before(&mut self.stream) {
            return Err(self.work.fail(cause));
        }
        let maximum = input.len().min(self.work.bytes as usize);
        let count = self.stream.write(&input[..maximum])?;
        self.work.consumed(count, maximum)
    }
    fn flush(&mut self) -> io::Result<()> {
        if let Err(cause) = self.work.before(&mut self.stream) {
            return Err(self.work.fail(cause));
        }
        self.stream.flush()
    }
}

/// Execute the existing closed map/1.5 handshake and exactly one whole-enclosure
/// read. The stream is consumed and dropped; no reconnect or mutation is possible.
///
/// Success means complete typed acquisition only. In particular, source-pin
/// mismatch, expiry at the newly learned game tick and runtime revocation after
/// the last read are for the durable monitor to evaluate after retaining these
/// bytes. Callers must not label this return value a published goal result.
pub fn read_map<S, G>(
    stream: S,
    spec: &MonitorSpec,
    token: Vec<u8>,
    nonce: Vec<u8>,
    context: &OperationContext,
    guard: G,
) -> Result<LiveMapObservation>
where
    S: ProofStream,
    G: FnMut(&OperationContext) -> Result<()>,
{
    let mut work = Work::new(spec, context, guard)?;
    let acquired = (|| {
        let mut client = MapRpcClient::negotiate(
            GuardedStream {
                stream,
                work: &mut work,
            },
            token,
            nonce,
            spec.region(),
            MAX_PAYLOAD_BYTES,
        )?;
        client.read_observation()
    })();
    match (work.failed, acquired) {
        (Some(cause), _) => Err(cause),
        (None, result) => result,
    }
}

#[cfg(test)]
mod tests;
