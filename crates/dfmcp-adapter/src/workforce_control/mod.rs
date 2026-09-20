#![forbid(unsafe_code)]
//! Sealed workforce/1.17 evidence. Membership and labor readback are not job completion.
//! The wire is unchanged; this module cannot create authority from a receipt.

pub mod rpc;
pub mod journal;
pub mod private_file;

use std::collections::BTreeSet;
use dfmcp_core::{Digest32, ErrorCode, FortressId, Result};
use crate::bounded_run::{Reader, error, hash, require, validate_key};

pub const MAX_CAPTURE: usize = 65_536;
pub const MAX_EFFECT: usize = 8_192;
pub const MAX_PLAN: usize = 65_675;
pub const MAX_UNITS: usize = 32;
pub const MAX_NATIVE_TICK: u64 = u32::MAX as u64 * 403_200 + 403_199;

fn u16(r: &mut Reader<'_>) -> Result<usize> { Ok(usize::from(u16::from_be_bytes(r.array()?))) }
fn text(r: &mut Reader<'_>, maximum: usize, empty: bool) -> Result<String> {
    let n = u16(r)?;
    require(n <= maximum && (empty || n != 0), "invalid workforce text length")?;
    let value = std::str::from_utf8(r.take(n)?)
        .map_err(|_| error(ErrorCode::AdapterRejected, "invalid workforce UTF-8"))?;
    require(!value.contains('\0'), "NUL in workforce text")?;
    Ok(value.to_owned())
}
fn put_text(out: &mut Vec<u8>, value: &str) {
    out.extend_from_slice(&(value.len() as u16).to_be_bytes());
    out.extend_from_slice(value.as_bytes());
}
fn bits(r: &mut Reader<'_>, n: usize) -> Result<Vec<u8>> {
    let value = r.take(n)?;
    require(value.iter().all(|b| *b <= 1), "noncanonical labor mask")?;
    Ok(value.to_vec())
}
pub fn validate_ids(ids: &[u32]) -> Result<()> { check_ids(ids, MAX_UNITS, false) }
fn check_ids(ids: &[u32], max: usize, empty: bool) -> Result<()> {
    require((empty || !ids.is_empty()) && ids.len() <= max
        && ids.iter().all(|id| *id <= i32::MAX as u32)
        && ids.windows(2).all(|p| p[0] < p[1]), "workforce IDs must be bounded, sorted and unique")
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Detail {
    name: String, flags: u32, selected_only: bool, labors: Vec<u8>, members: Vec<u32>,
}
impl Detail {
    pub fn name(&self) -> &str { &self.name }
    pub fn selected_only(&self) -> bool { self.selected_only }
    pub fn labors(&self) -> &[u8] { &self.labors }
    pub fn members(&self) -> &[u32] { &self.members }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Citizen { id: u32, historical_id: u32, eligible: bool, labors: Vec<u8> }
impl Citizen {
    fn decode(r: &mut Reader<'_>, n: usize) -> Result<Self> {
        let id = r.u32()?; let historical_id = r.u32()?; let eligible = r.boolean()?;
        require(id <= i32::MAX as u32 && historical_id <= i32::MAX as u32, "invalid citizen identity")?;
        Ok(Self { id, historical_id, eligible, labors: bits(r, n)? })
    }
    fn append(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.id.to_be_bytes());
        out.extend_from_slice(&self.historical_id.to_be_bytes());
        out.push(u8::from(self.eligible)); out.extend_from_slice(&self.labors);
    }
    pub fn id(&self) -> u32 { self.id }
    pub fn historical_id(&self) -> u32 { self.historical_id }
    pub fn eligible(&self) -> bool { self.eligible }
    pub fn labors(&self) -> &[u8] { &self.labors }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkforceCapture {
    bytes: Vec<u8>, generation: u64, sequence: u64, tick: u64, site: u32,
    folder: String, paused: bool, automatic: bool, labor_keys: Vec<String>,
    details: Vec<Detail>, citizens: Vec<Citizen>,
}
impl WorkforceCapture {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        require(bytes.len() <= MAX_CAPTURE, "workforce capture exceeds 64 KiB")?;
        let mut r = Reader(bytes);
        require(r.take(8)? == b"DFMWF017", "not a workforce/1.17 capture")?;
        let generation = r.u64()?; let sequence = r.u64()?; let tick = r.u64()?; let site = r.u32()?;
        require(generation > 0 && generation < u64::MAX && tick <= MAX_NATIVE_TICK
            && site <= i32::MAX as u32, "invalid workforce source or clock")?;
        let folder = text(&mut r, 512, false)?;
        let paused = r.boolean()?; let automatic = r.boolean()?;
        let columns = u16(&mut r)?;
        require((1..=128).contains(&columns), "invalid labor column count")?;
        let mut labor_keys = Vec::with_capacity(columns); let mut seen = BTreeSet::new();
        for _ in 0..columns {
            let key = text(&mut r, 64, false)?;
            require(seen.insert(key.clone()), "duplicate labor column")?; labor_keys.push(key);
        }
        let count = u16(&mut r)?; require(count <= 64, "too many work details")?;
        let mut details = Vec::with_capacity(count); let mut total = 0usize;
        for _ in 0..count {
            let name = text(&mut r, 256, true)?; let flags = r.u32()?; let selected_only = r.boolean()?;
            let labors = bits(&mut r, columns)?; let n = u16(&mut r)?;
            total += n; require(total <= 4096 && n <= r.0.len() / 4, "workforce membership bound")?;
            let mut members = Vec::with_capacity(n);
            for _ in 0..n { members.push(r.u32()?); }
            check_ids(&members, 4096, true)?;
            details.push(Detail { name, flags, selected_only, labors, members });
        }
        let count = u16(&mut r)?; require((1..=MAX_UNITS).contains(&count), "invalid selected citizen count")?;
        let mut citizens = Vec::with_capacity(count);
        for _ in 0..count { citizens.push(Citizen::decode(&mut r, columns)?); }
        validate_ids(&citizens.iter().map(Citizen::id).collect::<Vec<_>>())?; r.finish()?;
        Ok(Self { bytes: bytes.to_vec(), generation, sequence, tick, site, folder, paused, automatic,
            labor_keys, details, citizens })
    }
    fn encode_values(&self) -> Result<Vec<u8>> {
        // Only decoded, private fields reach this serializer. Re-decode after
        // membership changes to check aggregate capacity and exact canonicality.
        let mut out = b"DFMWF017".to_vec();
        for n in [self.generation, self.sequence, self.tick] { out.extend_from_slice(&n.to_be_bytes()); }
        out.extend_from_slice(&self.site.to_be_bytes()); put_text(&mut out, &self.folder);
        out.extend_from_slice(&[u8::from(self.paused), u8::from(self.automatic)]);
        out.extend_from_slice(&(self.labor_keys.len() as u16).to_be_bytes());
        for k in &self.labor_keys { put_text(&mut out, k); }
        out.extend_from_slice(&(self.details.len() as u16).to_be_bytes());
        for d in &self.details {
            put_text(&mut out, &d.name); out.extend_from_slice(&d.flags.to_be_bytes());
            out.push(u8::from(d.selected_only)); out.extend_from_slice(&d.labors);
            out.extend_from_slice(&(d.members.len() as u16).to_be_bytes());
            for id in &d.members { out.extend_from_slice(&id.to_be_bytes()); }
        }
        out.extend_from_slice(&(self.citizens.len() as u16).to_be_bytes());
        for u in &self.citizens { u.append(&mut out); }
        Self::decode(&out)?; Ok(out)
    }
    fn expected(&self, spec: AssignmentSpec) -> Result<(Self, Vec<u32>)> {
        let d = self.details.get(spec.detail as usize)
            .ok_or_else(|| error(ErrorCode::InvalidRequest, "work detail is outside this capture"))?;
        require(self.paused && self.automatic && self.sequence < u64::MAX && d.selected_only
            && d.labors.contains(&1) && self.citizens.iter().all(Citizen::eligible),
            "assignment requires paused automatic professions and eligible citizens in a selected-only detail")?;
        let changed: Vec<u32> = self.citizens.iter().map(Citizen::id)
            .filter(|id| d.members.binary_search(id).is_ok() != spec.assigned).collect();
        require(!changed.is_empty(), "assignment is already in the requested state")?;
        let mut out = self.clone(); out.sequence += 1;
        let members = &mut out.details[spec.detail as usize].members;
        for id in self.citizens.iter().map(Citizen::id) {
            match (members.binary_search(&id), spec.assigned) {
                (Err(at), true) => members.insert(at, id),
                (Ok(at), false) => { members.remove(at); },
                _ => {},
            }
        }
        out.bytes = out.encode_values()?;
        Ok((out, changed))
    }
    pub fn canonical_bytes(&self) -> &[u8] { &self.bytes }
    pub fn witness(&self) -> Digest32 { Digest32::of_bytes(&self.bytes) }
    pub fn generation(&self) -> u64 { self.generation }
    pub fn sequence(&self) -> u64 { self.sequence }
    pub fn tick(&self) -> u64 { self.tick }
    pub fn site(&self) -> u32 { self.site }
    pub fn folder(&self) -> &str { &self.folder }
    pub fn paused(&self) -> bool { self.paused }
    pub fn automatic(&self) -> bool { self.automatic }
    pub fn labor_keys(&self) -> &[String] { &self.labor_keys }
    pub fn details(&self) -> &[Detail] { &self.details }
    pub fn citizens(&self) -> &[Citizen] { &self.citizens }
    pub fn ids(&self) -> Vec<u32> { self.citizens.iter().map(Citizen::id).collect() }
    pub fn entity_cost(&self) -> usize { self.citizens.len() + self.details.len() + self.details.iter().map(|d| d.members.len()).sum::<usize>() }
    pub fn fortress_id(&self) -> FortressId { fortress_id(&self.folder, self.site) }
}
pub fn fortress_id(folder: &str, site: u32) -> FortressId {
    let mut data = b"dfmcp-live-fortress-id-v1\0".to_vec(); data.extend_from_slice(folder.as_bytes());
    data.push(0); data.extend_from_slice(&site.to_be_bytes());
    let digest = Digest32::of_bytes(&data); let mut id = [0; 8]; id.copy_from_slice(&digest.as_bytes()[..8]);
    FortressId::new(u64::from_be_bytes(id) | 1)
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AssignmentSpec { detail: u32, assigned: bool }
impl AssignmentSpec {
    pub fn new(detail: u32, assigned: bool) -> Result<Self> {
        require(detail < 64, "detail index must be 0..63")?; Ok(Self { detail, assigned })
    }
    pub fn detail(self) -> u32 { self.detail }
    pub fn assigned(self) -> bool { self.assigned }
    fn append(self, out: &mut Vec<u8>) { out.extend_from_slice(&self.detail.to_be_bytes()); out.push(u8::from(self.assigned)); }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssignmentPlan {
    key: String, spec: AssignmentSpec, before: WorkforceCapture, digest: Digest32, token: [u8; 16],
}
impl AssignmentPlan {
    pub fn new(key: &str, spec: AssignmentSpec, before: WorkforceCapture) -> Result<Self> {
        validate_key(key)?; before.expected(spec)?;
        let mut data = Vec::new(); spec.append(&mut data); data.extend_from_slice(before.witness().as_bytes());
        let digest = hash(b"dfmcp-workforce-plan/1", &data);
        let mut data = Vec::new(); put_text(&mut data, key); data.extend_from_slice(digest.as_bytes());
        let full = hash(b"dfmcp-workforce-token/1", &data); let mut token = [0; 16]; token.copy_from_slice(&full.as_bytes()[..16]);
        Ok(Self { key: key.to_owned(), spec, before, digest, token })
    }
    pub fn key(&self) -> &str { &self.key }
    pub fn spec(&self) -> AssignmentSpec { self.spec }
    pub fn before(&self) -> &WorkforceCapture { &self.before }
    pub fn digest(&self) -> Digest32 { self.digest }
    pub fn token(&self) -> &[u8; 16] { &self.token }
    pub fn changed_ids(&self) -> Result<Vec<u32>> { Ok(self.before.expected(self.spec)?.1) }
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new(); put_text(&mut out, &self.key); self.spec.append(&mut out);
        out.extend_from_slice(&(self.before.bytes.len() as u32).to_be_bytes()); out.extend_from_slice(&self.before.bytes); out
    }
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        require(bytes.len() <= MAX_PLAN, "workforce plan exceeds bound")?;
        let mut r = Reader(bytes); let key = text(&mut r, 128, false)?;
        let spec = AssignmentSpec::new(r.u32()?, r.boolean()?)?;
        let n = r.u32()? as usize; require(n <= MAX_CAPTURE, "capture length exceeds bound")?;
        let before = WorkforceCapture::decode(r.take(n)?)?; r.finish()?;
        Self::new(&key, spec, before)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum AssignmentPhase { Prepared = 0, Unknown = 1, Applied = 2, Refused = 3, Cancelled = 4 }
impl AssignmentPhase {
    pub fn settled(self) -> bool { matches!(self, Self::Applied | Self::Refused | Self::Cancelled) }
    pub fn as_str(self) -> &'static str { match self {
        Self::Prepared => "prepared", Self::Unknown => "unknown", Self::Applied => "applied",
        Self::Refused => "refused", Self::Cancelled => "cancelled",
    } }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssignmentEffect {
    bytes: Vec<u8>, key: String, plan: Digest32, phase: AssignmentPhase, receipt: Digest32,
    after: Option<Digest32>, post: Vec<Citizen>,
}
impl AssignmentEffect {
    pub fn decode(bytes: &[u8], plan: &AssignmentPlan) -> Result<Self> {
        require(bytes.len() <= MAX_EFFECT, "workforce effect exceeds 8 KiB")?;
        let mut r = Reader(bytes); require(r.take(8)? == b"DFMWE017", "not a workforce/1.17 effect")?;
        require(text(&mut r, 128, false)? == plan.key && r.take(32)? == plan.digest.as_bytes()
            && r.take(16)? == plan.token.as_slice(), "workforce effect identity mismatch")?;
        require(r.u32()? == plan.spec.detail && r.boolean()? == plan.spec.assigned
            && r.take(32)? == plan.before.witness().as_bytes(), "workforce effect plan mismatch")?;
        require(r.u64()? == plan.before.generation && r.u64()? == plan.before.sequence
            && r.u64()? == plan.before.tick, "workforce effect source mismatch")?;
        let phase = match r.byte()? {
            0 => AssignmentPhase::Prepared, 1 => AssignmentPhase::Unknown, 2 => AssignmentPhase::Applied,
            3 => AssignmentPhase::Refused, 4 => AssignmentPhase::Cancelled,
            _ => return Err(error(ErrorCode::AdapterRejected, "unknown assignment phase")),
        };
        let after = Digest32::from_bytes(r.array()?); let columns = u16(&mut r)?; let count = u16(&mut r)?;
        require(columns == plan.before.labor_keys.len() && count <= MAX_UNITS, "workforce readback dimensions changed")?;
        let mut post = Vec::with_capacity(count);
        for _ in 0..count { post.push(Citizen::decode(&mut r, columns)?); }
        let receipt = Digest32::from_bytes(r.array()?); r.finish()?;
        require(receipt == hash(b"dfmcp-workforce-receipt/1", &bytes[..bytes.len()-32]), "workforce receipt mismatch")?;
        if phase == AssignmentPhase::Applied {
            let (mut expected, changed) = plan.before.expected(plan.spec)?;
            require(post.len() == expected.citizens.len(), "missing citizen readback")?;
            for (old, new) in expected.citizens.iter_mut().zip(&post) {
                if changed.binary_search(&old.id).is_ok() {
                    if plan.spec.assigned {
                        require(plan.before.details[plan.spec.detail as usize].labors.iter().zip(&new.labors)
                            .all(|(allowed, enabled)| *allowed == 0 || *enabled == 1), "recomputed labor permissions missing")?;
                    }
                    old.labors.clone_from(&new.labors);
                }
                require(&*old == new, "unexpected citizen identity, eligibility or unchanged-mask write")?;
            }
            require(after == Digest32::of_bytes(&expected.encode_values()?), "full post-configuration witness mismatch")?;
        } else { require(post.is_empty() && after == Digest32::ZERO, "non-applied effect invents readback")?; }
        Ok(Self { bytes: bytes.to_vec(), key: plan.key.clone(), plan: plan.digest, phase, receipt,
            after: (phase == AssignmentPhase::Applied).then_some(after), post })
    }
    pub fn canonical_bytes(&self) -> &[u8] { &self.bytes }
    pub fn phase(&self) -> AssignmentPhase { self.phase }
    pub fn receipt(&self) -> Digest32 { self.receipt }
    pub fn after_witness(&self) -> Option<Digest32> { self.after }
    pub fn post_citizens(&self) -> &[Citizen] { &self.post }
    /// Unknown is permanent in this native generation, not a later promotion slot.
    pub fn follows(&self, old: &Self) -> Result<()> {
        require(self.key == old.key && self.plan == old.plan, "assignment receipt changed plans")?;
        if old.phase != AssignmentPhase::Prepared {
            require(self.bytes == old.bytes, "immutable assignment evidence changed")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
