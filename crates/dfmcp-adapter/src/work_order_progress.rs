#![forbid(unsafe_code)]
//! Selected read-only work-order progress. Counters are observations, not goods
//! produced, creation receipts, native identity across reload, or mutation authority.

pub mod rpc;

use std::time::{Duration, Instant};
use dfmcp_core::{Capability, DfmcpError, Digest32, ErrorCode, FortressId,
    GameTick, OperationContext, Result, RiskTier, SessionId};

pub const MAX_TARGETS: usize = 32;
pub const MAX_QUEUE: usize = 4096;
pub const MAX_OBSERVATION_BYTES: usize = 16 * 1024;
pub const RPC_BYTE_RESERVE: u64 = 384 * 1024;
pub const BOOTSTRAP_BYTE_RESERVE: u64 = 1024 * 1024;
const MAX_TICK: u64 = u32::MAX as u64 * 403_200 + 403_199;

fn error(code: ErrorCode, text: &str) -> DfmcpError { DfmcpError::new(code, text) }
fn require(valid: bool, text: &str) -> Result<()> {
    if valid { Ok(()) } else { Err(error(ErrorCode::AdapterRejected, text)) }
}
pub fn validate_targets(ids: &[u32]) -> Result<()> {
    if ids.is_empty() || ids.len() > MAX_TARGETS || ids.iter().any(|id| *id > i32::MAX as u32)
        || ids.windows(2).any(|pair| pair[0] >= pair[1])
    { return Err(error(ErrorCode::InvalidRequest, "progress selection must contain 1..32 sorted unique native order IDs")); }
    Ok(())
}
struct Reader<'a> { bytes: &'a [u8] }
impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let out = self.bytes.get(..n).ok_or_else(|| error(ErrorCode::AdapterRejected, "truncated work-order progress"))?;
        self.bytes = &self.bytes[n..]; Ok(out)
    }
    fn array<const N: usize>(&mut self) -> Result<[u8; N]> {
        self.take(N)?.try_into().map_err(|_| error(ErrorCode::AdapterRejected, "invalid progress scalar width"))
    }
    fn byte(&mut self) -> Result<u8> { Ok(self.array::<1>()?[0]) }
    fn boolean(&mut self) -> Result<bool> {
        match self.byte()? { 0 => Ok(false), 1 => Ok(true), _ => Err(error(ErrorCode::AdapterRejected, "noncanonical progress boolean")) }
    }
    fn u32(&mut self) -> Result<u32> { Ok(u32::from_be_bytes(self.array()?)) }
    fn i32(&mut self) -> Result<i32> { Ok(i32::from_be_bytes(self.array()?)) }
    fn u64(&mut self) -> Result<u64> { Ok(u64::from_be_bytes(self.array()?)) }
    fn text(&mut self, maximum: usize, empty: bool) -> Result<String> {
        let n = usize::from(u16::from_be_bytes(self.array()?));
        require(n <= maximum && (empty || n > 0), "invalid progress string bound")?;
        let s = std::str::from_utf8(self.take(n)?).map_err(|_| error(ErrorCode::AdapterRejected, "progress text is not UTF-8"))?;
        require(!s.contains('\0'), "progress text contains NUL")?; Ok(s.to_owned())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OrderStatus {
    pub job_type: i32, pub recipe: u8, pub remaining: i32, pub total: i32,
    pub status_bits: u32, pub frequency: i32, pub workshop_id: i32, pub max_workshops: i32,
    pub next_check_year: i32, pub next_check_tick: i32,
    pub item_conditions: u32, pub order_conditions: u32,
    pub type_key: String, pub reaction: String,
}
impl OrderStatus {
    pub fn validated(&self) -> bool { self.status_bits & 1 != 0 }
    pub fn active(&self) -> bool { self.status_bits & 2 != 0 }
    pub fn recipe_name(&self) -> Option<&'static str> {
        match self.recipe { 1 => Some("wooden_bed"), 2 => Some("wooden_door"),
            3 => Some("wooden_table"), 4 => Some("wooden_chair"), _ => None }
    }
    pub fn phase(&self) -> &'static str {
        if self.recipe == 0 { "unrecognized_or_modified" }
        else if !self.validated() { "awaiting_validation" }
        else if self.remaining == 0 { "reported_zero_remaining" }
        else if self.active() { "active" } else { "validated_inactive" }
    }
    fn same_configuration(&self, other: &Self) -> bool {
        self.job_type == other.job_type && self.recipe == other.recipe && self.total == other.total
            && self.frequency == other.frequency && self.workshop_id == other.workshop_id
            && self.max_workshops == other.max_workshops && self.item_conditions == other.item_conditions
            && self.order_conditions == other.order_conditions && self.type_key == other.type_key
            && self.reaction == other.reaction
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProgressRow { pub native_order_id: u32, pub order: Option<OrderStatus> }
impl ProgressRow {
    pub fn phase(&self) -> &'static str { self.order.as_ref().map_or("absent", OrderStatus::phase) }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProgressObservation {
    bytes: Vec<u8>, generation: u64, sequence: u64, tick: u64,
    site: u32, horizon: u32, queue_count: u32, paused: bool, folder: String,
    rows: Vec<ProgressRow>,
}
impl ProgressObservation {
    /// A complete exact selection is mandatory: omitted IDs are not absences.
    pub fn decode(bytes: &[u8], expected_ids: &[u32]) -> Result<Self> {
        validate_targets(expected_ids)?;
        require(bytes.len() <= MAX_OBSERVATION_BYTES, "progress observation exceeds 16 KiB")?;
        let mut r = Reader { bytes };
        require(r.take(8)? == b"DFMWP012", "not work-order-progress/1.12")?;
        let generation = r.u64()?; let sequence = r.u64()?; let tick = r.u64()?;
        let site = r.u32()?; let horizon = r.u32()?; let queue_count = r.u32()?; let paused = r.boolean()?;
        require(generation > 0 && generation < u64::MAX && sequence > 0 && sequence < u64::MAX
            && tick <= MAX_TICK && site <= i32::MAX as u32 && horizon <= i32::MAX as u32
            && queue_count <= MAX_QUEUE as u32 && queue_count <= horizon, "invalid progress identity or queue bound")?;
        let folder = r.text(512, false)?;
        require(r.u32()? as usize == expected_ids.len(), "progress selection has omitted or extra IDs")?;
        let mut rows = Vec::with_capacity(expected_ids.len());
        let mut present_count = 0;
        for expected in expected_ids {
            let native_order_id = r.u32()?;
            require(native_order_id == *expected, "progress response reordered or substituted a selected ID")?;
            let order = if r.boolean()? {
                require(native_order_id < horizon, "present order lies beyond allocation horizon")?;
                present_count += 1;
                let job_type = r.i32()?; let recipe = r.byte()?;
                let remaining = r.i32()?; let total = r.i32()?; let status_bits = r.u32()?;
                let frequency = r.i32()?; let workshop_id = r.i32()?; let max_workshops = r.i32()?;
                let next_check_year = r.i32()?; let next_check_tick = r.i32()?;
                let item_conditions = r.u32()?; let order_conditions = r.u32()?;
                let type_key = r.text(128, false)?; let reaction = r.text(128, true)?;
                require(job_type >= 0 && recipe <= 4 && i16::try_from(remaining).is_ok() && i16::try_from(total).is_ok()
                    && item_conditions <= MAX_QUEUE as u32 && order_conditions <= MAX_QUEUE as u32,
                    "invalid progress counters, type or condition bounds")?;
                if recipe != 0 {
                    let expected_type = match recipe { 1 => "ConstructBed", 2 => "ConstructDoor", 3 => "ConstructTable", _ => "ConstructThrone" };
                    require((1..=100).contains(&total) && (0..=total).contains(&remaining)
                        && status_bits & !3 == 0 && frequency == 0 && workshop_id == -1 && max_workshops == 1
                        && item_conditions == 0 && order_conditions == 0 && reaction.is_empty() && type_key == expected_type,
                        "recognized recipe contradicts progress evidence")?;
                }
                Some(OrderStatus { job_type, recipe, remaining, total, status_bits, frequency,
                    workshop_id, max_workshops, next_check_year, next_check_tick, item_conditions, order_conditions, type_key, reaction })
            } else { None };
            rows.push(ProgressRow { native_order_id, order });
        }
        require(present_count <= queue_count && r.bytes.is_empty(), "progress presence count or record extent is invalid")?;
        Ok(Self { bytes: bytes.to_vec(), generation, sequence, tick, site, horizon, queue_count, paused, folder, rows })
    }
    pub fn canonical_bytes(&self) -> &[u8] { &self.bytes }
    pub fn witness(&self) -> Digest32 { Digest32::of_bytes(&self.bytes) }
    pub fn generation(&self) -> u64 { self.generation }
    pub fn sequence(&self) -> u64 { self.sequence }
    pub fn tick(&self) -> u64 { self.tick }
    pub fn site(&self) -> u32 { self.site }
    pub fn next_order_id(&self) -> u32 { self.horizon }
    pub fn queue_count(&self) -> u32 { self.queue_count }
    pub fn paused(&self) -> bool { self.paused }
    pub fn world_folder(&self) -> &str { &self.folder }
    pub fn rows(&self) -> &[ProgressRow] { &self.rows }
    pub fn ids(&self) -> Vec<u32> { self.rows.iter().map(|r| r.native_order_id).collect() }
    pub fn fortress_id(&self) -> FortressId {
        let mut bytes = b"dfmcp-live-fortress-id-v1\0".to_vec();
        bytes.extend_from_slice(self.folder.as_bytes()); bytes.push(0); bytes.extend_from_slice(&self.site.to_be_bytes());
        let digest = Digest32::of_bytes(&bytes); let mut id = [0; 8]; id.copy_from_slice(&digest.as_bytes()[..8]);
        FortressId::new(u64::from_be_bytes(id) | 1)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProgressChange {
    pub native_order_id: u32, pub kind: &'static str, pub before_phase: &'static str,
    pub after_phase: &'static str, pub remaining_decrease: Option<u32>, pub remaining_increase: Option<u32>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProgressComparison {
    pub status: &'static str, pub reset_reason: Option<&'static str>, pub baseline: Option<Digest32>,
    pub elapsed_game_ticks: Option<u64>, pub changes: Vec<ProgressChange>,
}
pub fn compare(before: Option<&ProgressObservation>, after: &ProgressObservation) -> Result<ProgressComparison> {
    let mut out = ProgressComparison { status: "bootstrap", reset_reason: None, baseline: None,
        elapsed_game_ticks: None, changes: Vec::new() };
    let Some(before) = before else { return Ok(out); };
    if before.generation == after.generation && after.sequence <= before.sequence {
        return Err(error(ErrorCode::StaleAnchor, "progress capture was replayed or reordered; acquire fresh evidence"));
    }
    let reset = if before.generation != after.generation || before.folder != after.folder || before.site != after.site {
        Some("incarnation_or_fortress_changed")
    } else if before.ids() != after.ids() { Some("selection_changed") }
    else if after.tick < before.tick || after.horizon < before.horizon { Some("clock_or_allocation_horizon_regressed") }
    else { None };
    if let Some(reason) = reset { out.status = "reset"; out.reset_reason = Some(reason); return Ok(out); }
    out.status = "compared"; out.baseline = Some(before.witness()); out.elapsed_game_ticks = Some(after.tick - before.tick);
    for (old, new) in before.rows.iter().zip(&after.rows) {
        if old == new { continue; }
        let mut change = ProgressChange { native_order_id: new.native_order_id, kind: "status_changed",
            before_phase: old.phase(), after_phase: new.phase(), remaining_decrease: None, remaining_increase: None };
        match (&old.order, &new.order) {
            (Some(_), None) => change.kind = "disappeared_outcome_unknown",
            (None, Some(_)) => change.kind = "appeared_identity_unlinked",
            (Some(a), Some(b)) => {
                if !a.same_configuration(b) { change.kind = "configuration_changed"; }
                else if a.recipe == 0 { change.kind = "unrecognized_order_changed"; }
                else if b.remaining < a.remaining {
                    change.kind = "remaining_counter_decreased";
                    change.remaining_decrease = Some((a.remaining - b.remaining) as u32);
                } else if b.remaining > a.remaining {
                    change.kind = "remaining_counter_increased";
                    change.remaining_increase = Some((b.remaining - a.remaining) as u32);
                }
            }
            (None, None) => continue,
        }
        out.changes.push(change);
    }
    Ok(out)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProgressManifest { pub generation: u64, pub df_version: String, pub dfhack_version: String }
/// Trusted read-only injected boundary. No prepare, commit or generic method exists.
pub trait ProgressSource {
    fn manifest(&self) -> &ProgressManifest;
    fn fence(&mut self);
    fn read(&mut self, ids: &[u32], timeout: Duration) -> Result<ProgressObservation>;
}
pub struct ProgressSession<S> {
    session_id: SessionId, fortress_id: FortressId, source: S,
    current: Option<ProgressObservation>, comparison: Option<ProgressComparison>,
}
impl<S: ProgressSource> ProgressSession<S> {
    pub fn new(source: S, context: &OperationContext) -> Result<Self> {
        context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
        if context.session_id == SessionId::NIL || context.anchor.fortress_id == FortressId::NIL {
            return Err(error(ErrorCode::InvalidRequest, "progress requires a concrete session and fortress"));
        }
        let manifest = source.manifest();
        require(manifest.generation > 0 && manifest.generation < u64::MAX
            && [&manifest.df_version, &manifest.dfhack_version].iter().all(|v| !v.is_empty() && v.len() <= 128 && !v.contains('\0')),
            "progress source manifest is invalid")?;
        Ok(Self { session_id: context.session_id, fortress_id: context.anchor.fortress_id,
            source, current: None, comparison: None })
    }
    fn access(&self, context: &OperationContext) -> Result<()> {
        if context.session_id != self.session_id || context.anchor.fortress_id != self.fortress_id {
            return Err(error(ErrorCode::CapabilityDenied, "progress belongs to another session or fortress"));
        }
        self.current_context(context).authorize(Capability::Query, RiskTier::ReadOnly, &[], None)
    }
    fn current_context(&self, context: &OperationContext) -> OperationContext {
        let mut current = context.clone();
        if let Some(capture) = &self.current {
            current.anchor.tick = GameTick(current.anchor.tick.get().max(capture.tick));
        }
        current
    }
    pub fn current(&self, context: &OperationContext) -> Result<Option<&ProgressObservation>> {
        self.access(context)?; Ok(self.current.as_ref())
    }
    pub fn comparison(&self, context: &OperationContext) -> Result<Option<&ProgressComparison>> {
        self.access(context)?; Ok(self.comparison.as_ref())
    }
    pub fn refresh(&mut self, ids: &[u32], context: &OperationContext) -> Result<()> {
        self.access(context)?;
        self.current_context(context).authorize(Capability::Observe, RiskTier::ReadOnly, &[], None)?;
        validate_targets(ids)?;
        if context.budget.max_entities < MAX_QUEUE as u32 || context.budget.max_bytes < RPC_BYTE_RESERVE {
            return Err(error(ErrorCode::BudgetExceeded, "progress requires full-queue work and complete RPC capacity"));
        }
        let before = self.current.take(); self.comparison = None;
        let started = Instant::now();
        let timeout = Duration::from_millis(context.budget.max_wall_millis.min(60_000));
        let manifest = self.source.manifest().clone();
        let result = (|| {
            let capture = self.source.read(ids, timeout)?;
            let capture = ProgressObservation::decode(capture.canonical_bytes(), ids)?;
            require(self.source.manifest() == &manifest && capture.generation == manifest.generation
                && capture.fortress_id() == self.fortress_id, "progress source changed incarnation, software or fortress")?;
            let mut fresh = context.clone(); fresh.anchor.tick = GameTick(capture.tick);
            fresh.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
            fresh.authorize(Capability::Observe, RiskTier::ReadOnly, &[], None)?;
            let comparison = compare(before.as_ref(), &capture)?;
            if started.elapsed() >= timeout { return Err(error(ErrorCode::BudgetExceeded, "progress capture exceeded its deadline")); }
            Ok((capture, comparison))
        })();
        match result {
            Ok((capture, comparison)) => { self.current = Some(capture); self.comparison = Some(comparison); Ok(()) }
            Err(cause) => { self.source.fence(); Err(cause) } // Never serve an old selection as a successful refresh.
        }
    }
}

#[cfg(test)]
#[path = "work_order_progress/tests.rs"]
mod tests;
