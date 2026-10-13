//! Lossless, bounded custody of an original prepared plan (not effect authority).
//!
//! Archive identity is separate from the existing semantic plan digest. In
//! particular, tagged Other/Custom kinds retain their Rust variant even where
//! the older semantic encoding has a reserved-name alias. Decoding requires
//! BOTH externally selected identities; a self-checksummed file is not its own
//! expected identity. No current anchor, expiry, key or obligation is rewritten.

use std::collections::{BTreeMap, BTreeSet};

use dfmcp_core::{
    Capability, DfmcpError, Digest32, EdgeId, EntityId, ErrorCode, FortressId, GameTick,
    IntentId, MapCoord, MapCuboid, ObservationCursor, PlanId, Result, RiskTier, StateAnchor,
    StepId,
};
use dfmcp_world::{CompareOp, EdgeKind, EntityKind, Predicate, Value};

use crate::{Action, BuildingKind, DigMode, MaterialSelector, ObligationSpec, PlanStep,
    PreparedPlan, WorkOrderCondition};

mod codec;

const MAGIC: &[u8; 8] = b"DFMPAR01";
const DOMAIN: &[u8] = b"dfmcp-exact-plan-archive/1\0";
const OVERHEAD: usize = 8 + 4 + 32;
pub const MAX_ARCHIVE_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_ARCHIVE_STEPS: usize = 256;
const MAX_NODES: usize = 65_536;
const MAX_DEPTH: usize = 64;
const MAX_COLLECTION: usize = 4_096;
const MAX_TEXT: usize = 65_536;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ArchiveLimits {
    /// Entire framed object, including the checksum.
    pub max_bytes: usize,
    pub max_steps: usize,
    /// Aggregate collection members and recursive predicate/value nodes.
    pub max_nodes: usize,
    /// Combined predicate/value nesting, not two separately replenished limits.
    pub max_depth: usize,
}
impl Default for ArchiveLimits {
    fn default() -> Self {
        Self { max_bytes: MAX_ARCHIVE_BYTES, max_steps: MAX_ARCHIVE_STEPS,
            max_nodes: MAX_NODES, max_depth: MAX_DEPTH }
    }
}
impl ArchiveLimits {
    fn validate(self) -> Result<()> {
        if !(OVERHEAD + 1..=MAX_ARCHIVE_BYTES).contains(&self.max_bytes)
            || !(1..=MAX_ARCHIVE_STEPS).contains(&self.max_steps)
            || !(1..=MAX_NODES).contains(&self.max_nodes)
            || !(1..=MAX_DEPTH).contains(&self.max_depth)
        {
            return Err(exhausted());
        }
        Ok(())
    }
}

/// Retain this small pair in the operator's original handoff. Neither identity
/// grants authority, authenticates an operator, or proves that dispatch occurred.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlanArchiveSeal {
    pub plan_digest: Digest32,
    pub object_digest: Digest32,
}

fn invalid(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::InvalidPlan, message).retryable(false)
}
fn exhausted() -> DfmcpError {
    DfmcpError::new(ErrorCode::BudgetExceeded, "prepared-plan archive exceeds its bounded profile")
}
fn object_digest(bytes: &[u8]) -> Digest32 {
    let mut preimage = Vec::with_capacity(DOMAIN.len() + bytes.len());
    preimage.extend_from_slice(DOMAIN);
    preimage.extend_from_slice(bytes);
    Digest32::of_bytes(&preimage)
}

/// Encode every original field, then validate its semantic seal. The bounded
/// visitor runs BEFORE the older validator can normalize/clone/hash any inputs.
/// Oversized actions, text, collections and recursive values therefore refuse
/// before those unbounded helpers are called. No canonical plan bytes change.
pub fn encode(plan: &PreparedPlan, limits: ArchiveLimits) -> Result<(Vec<u8>, PlanArchiveSeal)> {
    encode_with_check(plan, limits, || Ok(()))
}

/// A caller may supply cancellation/deadline checks. Checks run at entry, each
/// 128 visited nodes and each bounded raw/string operation, and before return.
/// Legacy structural validation and hashing are byte/node-bounded, not internally
/// interrupted. The callback has no authority to change the borrowed plan.
pub fn encode_with_check<F: FnMut() -> Result<()>>(
    plan: &PreparedPlan,
    limits: ArchiveLimits,
    mut check: F,
) -> Result<(Vec<u8>, PlanArchiveSeal)> {
    limits.validate()?;
    check()?;
    let mut writer = Writer { bytes: Vec::new(), limits, nodes: 0, check: &mut check };
    writer.raw(MAGIC)?;
    writer.u32(0)?;
    writer.plan(plan)?;
    let mut bytes = writer.bytes;
    check()?;
    plan.validate_structure()?;
    check()?;
    let length = u32::try_from(bytes.len() - 12).map_err(|_| exhausted())?;
    bytes[8..12].copy_from_slice(&length.to_be_bytes());
    let object_digest = object_digest(&bytes);
    bytes.extend_from_slice(object_digest.as_bytes());
    check()?;
    Ok((bytes, PlanArchiveSeal { plan_digest: plan.digest, object_digest }))
}

/// Decode exact original content, not a recompiled/rebased plan. Current game,
/// policy, capability, checkpoint and idempotency checks remain the coordinator's
/// responsibility even when this function succeeds on an expired historical plan.
pub fn decode(bytes: &[u8], expected: PlanArchiveSeal, limits: ArchiveLimits) -> Result<PreparedPlan> {
    decode_with_check(bytes, expected, limits, || Ok(()))
}

pub fn decode_with_check<F: FnMut() -> Result<()>>(
    bytes: &[u8],
    expected: PlanArchiveSeal,
    limits: ArchiveLimits,
    mut check: F,
) -> Result<PreparedPlan> {
    limits.validate()?;
    check()?;
    if bytes.len() > limits.max_bytes { return Err(exhausted()); }
    if bytes.len() <= OVERHEAD || &bytes[..8] != MAGIC
        || expected.plan_digest == Digest32::ZERO || expected.object_digest == Digest32::ZERO
    {
        return Err(invalid("invalid prepared-plan archive header or expected identity"));
    }
    let mut length = [0; 4];
    length.copy_from_slice(&bytes[8..12]);
    if u32::from_be_bytes(length) as usize != bytes.len() - OVERHEAD {
        return Err(invalid("prepared-plan archive length differs"));
    }
    let end = bytes.len() - 32;
    if &bytes[end..] != expected.object_digest.as_bytes()
        || object_digest(&bytes[..end]) != expected.object_digest
    {
        return Err(invalid("prepared-plan archive object identity differs"));
    }
    check()?;
    let mut reader = Reader { bytes: &bytes[12..end], limits, nodes: 0, check: &mut check };
    let plan = reader.plan()?;
    if !reader.bytes.is_empty() || plan.digest != expected.plan_digest {
        return Err(invalid("prepared-plan archive has trailing fields or a different semantic identity"));
    }
    check()?;
    // Bounded decode has finished before invoking recursive canonical validation.
    plan.validate_structure()?;
    check()?;
    // Reject alternate encodings as well as any future encoder/decoder drift.
    let (roundtrip, seal) = encode_with_check(&plan, limits, &mut check)?;
    if roundtrip != bytes || seal != expected {
        return Err(invalid("prepared-plan archive is not losslessly canonical"));
    }
    check()?;
    Ok(plan)
}

struct Writer<'a, F> {
    bytes: Vec<u8>, limits: ArchiveLimits, nodes: usize, check: &'a mut F,
}
impl<F: FnMut() -> Result<()>> Writer<'_, F> {
    fn raw(&mut self, bytes: &[u8]) -> Result<()> {
        (self.check)()?;
        if self.bytes.len().checked_add(bytes.len()).is_none_or(|n| n > self.limits.max_bytes - 32) {
            return Err(exhausted());
        }
        self.bytes.try_reserve(bytes.len()).map_err(|_| exhausted())?;
        self.bytes.extend_from_slice(bytes);
        Ok(())
    }
    fn node(&mut self, depth: usize) -> Result<()> {
        if depth > self.limits.max_depth { return Err(exhausted()); }
        self.claim(1)
    }
    fn claim(&mut self, count: usize) -> Result<()> {
        self.nodes = self.nodes.checked_add(count).ok_or_else(exhausted)?;
        if self.nodes > self.limits.max_nodes { return Err(exhausted()); }
        (self.check)()
    }
    fn count(&mut self, count: usize, maximum: usize) -> Result<()> {
        if count > maximum { return Err(exhausted()); }
        self.claim(count)?;
        self.u32(u32::try_from(count).map_err(|_| exhausted())?)
    }
    fn text(&mut self, text: &str, maximum: usize) -> Result<()> {
        if text.len() > maximum || text.as_bytes().contains(&0) { return Err(exhausted()); }
        self.u32(u32::try_from(text.len()).map_err(|_| exhausted())?)?;
        self.raw(text.as_bytes())
    }
    fn blob(&mut self, bytes: &[u8]) -> Result<()> {
        if bytes.len() > MAX_TEXT { return Err(exhausted()); }
        self.u32(u32::try_from(bytes.len()).map_err(|_| exhausted())?)?;
        self.raw(bytes)
    }
    fn byte(&mut self, value: u8) -> Result<()> { self.raw(&[value]) }
    fn boolean(&mut self, value: bool) -> Result<()> { self.byte(u8::from(value)) }
    fn u32(&mut self, value: u32) -> Result<()> { self.raw(&value.to_be_bytes()) }
    fn u64(&mut self, value: u64) -> Result<()> { self.raw(&value.to_be_bytes()) }
    fn u128(&mut self, value: u128) -> Result<()> { self.raw(&value.to_be_bytes()) }
    fn i32(&mut self, value: i32) -> Result<()> { self.raw(&value.to_be_bytes()) }
    fn i64(&mut self, value: i64) -> Result<()> { self.raw(&value.to_be_bytes()) }
}

struct Reader<'a, 'c, F> {
    bytes: &'a [u8], limits: ArchiveLimits, nodes: usize, check: &'c mut F,
}
impl<'a, F: FnMut() -> Result<()>> Reader<'a, '_, F> {
    fn raw(&mut self, count: usize) -> Result<&'a [u8]> {
        (self.check)()?;
        let value = self.bytes.get(..count).ok_or_else(|| invalid("truncated prepared-plan archive"))?;
        self.bytes = &self.bytes[count..];
        Ok(value)
    }
    fn array<const N: usize>(&mut self) -> Result<[u8; N]> {
        let mut value = [0; N]; value.copy_from_slice(self.raw(N)?); Ok(value)
    }
    fn node(&mut self, depth: usize) -> Result<()> {
        if depth > self.limits.max_depth { return Err(exhausted()); }
        self.claim(1)
    }
    fn claim(&mut self, count: usize) -> Result<()> {
        self.nodes = self.nodes.checked_add(count).ok_or_else(exhausted)?;
        if self.nodes > self.limits.max_nodes { return Err(exhausted()); }
        (self.check)()
    }
    fn count(&mut self, maximum: usize, minimum_bytes: usize) -> Result<usize> {
        let count = self.u32()? as usize;
        if count > maximum || count.checked_mul(minimum_bytes).is_none_or(|n| n > self.bytes.len()) {
            return Err(exhausted());
        }
        self.claim(count)?;
        Ok(count)
    }
    fn text(&mut self, maximum: usize) -> Result<String> {
        let length = self.u32()? as usize;
        if length > maximum { return Err(exhausted()); }
        let text = std::str::from_utf8(self.raw(length)?).map_err(|_| invalid("archive text is not UTF-8"))?;
        if text.as_bytes().contains(&0) { return Err(invalid("archive text contains NUL")); }
        Ok(text.to_owned())
    }
    fn blob(&mut self) -> Result<Vec<u8>> {
        let length = self.u32()? as usize;
        if length > MAX_TEXT { return Err(exhausted()); }
        Ok(self.raw(length)?.to_vec())
    }
    fn byte(&mut self) -> Result<u8> { Ok(self.array::<1>()?[0]) }
    fn boolean(&mut self) -> Result<bool> {
        match self.byte()? { 0 => Ok(false), 1 => Ok(true), _ => Err(invalid("invalid archive boolean")) }
    }
    fn u32(&mut self) -> Result<u32> { Ok(u32::from_be_bytes(self.array()?)) }
    fn u64(&mut self) -> Result<u64> { Ok(u64::from_be_bytes(self.array()?)) }
    fn u128(&mut self) -> Result<u128> { Ok(u128::from_be_bytes(self.array()?)) }
    fn i32(&mut self) -> Result<i32> { Ok(i32::from_be_bytes(self.array()?)) }
    fn i64(&mut self) -> Result<i64> { Ok(i64::from_be_bytes(self.array()?)) }
}
