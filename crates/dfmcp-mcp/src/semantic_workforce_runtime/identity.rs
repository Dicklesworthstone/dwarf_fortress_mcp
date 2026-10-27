//! Immutable original citizen capsule. Stored identity is historical data;
//! current independently issued policy is still required for every use.
use super::{denied, exhausted};
use dfmcp_adapter::build_placement::journal::private_file::PrivateBuildFile;
use dfmcp_adapter::control_effect_journal::EffectJournalStorage;
use dfmcp_adapter::live_observation::CitizenCoverage;
use dfmcp_adapter::semantic_workforce::evidence_owner::open_private_citizen_identity_storage;
use dfmcp_adapter::{BridgeManifest, CitizenRecord, LiveObservationCapsule};
use dfmcp_core::{
    Capability, DfmcpError, Digest32, ErrorCode, OperationContext, Result, RiskTier, SessionId,
};
use std::collections::BTreeSet;
use std::io::{self, SeekFrom};
use std::path::Path;
use std::time::{Duration, Instant};

const DOMAIN: &[u8] = b"dfmcp.live-observation-capsule.v2\0";
pub const MAX_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_CITIZENS: usize = 4096;
fn corrupt() -> DfmcpError {
    DfmcpError::new(
        ErrorCode::CorruptLedger,
        "original citizen identity bytes or custody changed; reopen exact history without repair",
    )
}
fn io_error(_: io::Error) -> DfmcpError {
    corrupt()
}
struct Read<'a> {
    bytes: &'a [u8],
    offset: usize,
}
impl<'a> Read<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.offset.checked_add(n).ok_or_else(corrupt)?;
        let bytes = self.bytes.get(self.offset..end).ok_or_else(corrupt)?;
        self.offset = end;
        Ok(bytes)
    }
    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(
            self.take(4)?.try_into().map_err(|_| corrupt())?,
        ))
    }
    fn i32(&mut self) -> Result<i32> {
        Ok(i32::from_le_bytes(
            self.take(4)?.try_into().map_err(|_| corrupt())?,
        ))
    }
    fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(
            self.take(8)?.try_into().map_err(|_| corrupt())?,
        ))
    }
    fn boolean(&mut self) -> Result<bool> {
        match self.take(1)? {
            [0] => Ok(false),
            [1] => Ok(true),
            _ => Err(corrupt()),
        }
    }
    fn string(&mut self, max: usize) -> Result<String> {
        let n = self.u32()? as usize;
        if n > max {
            return Err(corrupt());
        }
        std::str::from_utf8(self.take(n)?)
            .map(str::to_owned)
            .map_err(|_| corrupt())
    }
}
/// Decode only the already versioned canonical V1 capsule. Re-encoding by the
/// ordinary capsule validator must reproduce every byte, including method order.
pub fn decode(bytes: &[u8], seal: Digest32) -> Result<LiveObservationCapsule> {
    if bytes.is_empty()
        || bytes.len() > MAX_BYTES
        || seal == Digest32::ZERO
        || Digest32::of_bytes(bytes) != seal
    {
        return Err(corrupt());
    }
    let mut r = Read { bytes, offset: 0 };
    if r.take(DOMAIN.len())? != DOMAIN {
        return Err(corrupt());
    }
    let bridge_version = r.string(64)?;
    let dfhack_version = r.string(128)?;
    let df_version = r.string(128)?;
    let world_loaded = r.boolean()?;
    let fortress_mode = r.boolean()?;
    let bridge_generation = r.u64()?;
    if r.u32()? != 2 {
        return Err(corrupt());
    }
    let mut supported_methods = BTreeSet::new();
    for _ in 0..2 {
        if !supported_methods.insert(r.string(64)?) {
            return Err(corrupt());
        }
    }
    let names_included = r.boolean()?;
    let paused = r.boolean()?;
    let current_year = r.u32()?;
    let current_year_tick = r.u32()?;
    let world_name = r.string(1024)?;
    let world_folder = r.string(512)?;
    let site_id = r.i32()?;
    let count = r.u32()?;
    if count as usize > MAX_CITIZENS {
        return Err(corrupt());
    }
    let mut citizens = Vec::with_capacity(count as usize);
    for _ in 0..count {
        citizens.push(CitizenRecord {
            unit_id: r.i32()?,
            name: r.string(1024)?,
            race: r.string(128)?,
            profession: r.i32()?,
            x: r.i32()?,
            y: r.i32()?,
            z: r.i32()?,
            alive: r.boolean()?,
            sane: r.boolean()?,
            active: r.boolean()?,
            visible: r.boolean()?,
            citizen: r.boolean()?,
            resident: r.boolean()?,
            baby: r.boolean()?,
            child: r.boolean()?,
            adult: r.boolean()?,
        });
    }
    if r.offset != bytes.len() {
        return Err(corrupt());
    }
    let capsule = LiveObservationCapsule {
        bridge: BridgeManifest {
            bridge_version,
            dfhack_version,
            df_version,
            world_loaded,
            fortress_mode,
            bridge_generation,
            supported_methods,
        },
        paused,
        current_year,
        current_year_tick,
        world_name,
        world_folder,
        site_id,
        names_included,
        citizen_coverage: CitizenCoverage {
            offset: 0,
            returned: count,
            total: count,
            complete: true,
        },
        citizens,
        canonical_bytes: bytes.to_vec(),
        content_digest: seal,
    };
    capsule.validate()?;
    Ok(capsule)
}
struct Call {
    context: OperationContext,
    until: Instant,
    bytes: u64,
}
impl Call {
    fn new(c: &OperationContext) -> Result<Self> {
        c.budget.validate()?;
        if c.session_id == SessionId::NIL {
            return Err(denied("identity archive needs an owning session"));
        }
        let until = Instant::now()
            .checked_add(Duration::from_millis(c.budget.max_wall_millis))
            .ok_or_else(|| exhausted("identity deadline overflow"))?;
        let call = Self {
            context: c.clone(),
            until,
            bytes: c.budget.max_bytes,
        };
        call.check(false)?;
        Ok(call)
    }
    fn check(&self, write: bool) -> Result<()> {
        self.context
            .authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
        if write {
            self.context
                .authorize(Capability::Plan, RiskTier::ReadOnly, &[], None)?;
        }
        if Instant::now() >= self.until {
            return Err(exhausted("identity archive deadline expired"));
        }
        Ok(())
    }
    fn charge(&mut self, n: u64) -> Result<()> {
        self.check(false)?;
        self.bytes = self
            .bytes
            .checked_sub(n)
            .filter(|n| *n > 0)
            .ok_or_else(|| exhausted("identity archive byte-work exhausted"))?;
        Ok(())
    }
    fn current(&self) -> Result<OperationContext> {
        self.check(false)?;
        let millis = self
            .until
            .checked_duration_since(Instant::now())
            .ok_or_else(|| exhausted("identity deadline expired"))?
            .as_millis();
        if millis == 0 {
            return Err(exhausted("identity deadline expired"));
        }
        let mut c = self.context.clone();
        c.budget.max_bytes = self.bytes;
        c.budget.max_wall_millis =
            u64::try_from(millis).map_err(|_| exhausted("identity deadline overflow"))?;
        Ok(c)
    }
    fn bound(&self, capsule: &LiveObservationCapsule) -> Result<()> {
        self.check(false)?;
        let site = u32::try_from(capsule.site_id).map_err(|_| corrupt())?;
        if dfmcp_adapter::workforce_control::fortress_id(&capsule.world_folder, site)
            != self.context.anchor.fortress_id
        {
            return Err(denied(
                "original citizen identity belongs to another fortress",
            ));
        }
        Ok(())
    }
}
pub struct CapsuleStore<S> {
    storage: S,
    raw: Vec<u8>,
    seal: Digest32,
    session: SessionId,
    fenced: bool,
}
impl<S: EffectJournalStorage> CapsuleStore<S> {
    pub fn create(
        storage: S,
        capsule: &LiveObservationCapsule,
        c: &OperationContext,
    ) -> Result<Self> {
        let mut call = Call::new(c)?;
        call.check(true)?;
        call.bound(capsule)?;
        call.charge(6 * capsule.canonical_bytes.len() as u64 + 4096)?;
        let decoded = decode(&capsule.canonical_bytes, capsule.content_digest)?;
        if &decoded != capsule {
            return Err(corrupt());
        }
        let mut out = Self {
            storage,
            raw: capsule.canonical_bytes.clone(),
            seal: capsule.content_digest,
            session: c.session_id,
            fenced: false,
        };
        out.storage.validate_identity().map_err(io_error)?;
        if out.storage.seek(SeekFrom::End(0)).map_err(io_error)? != 0 {
            return Err(DfmcpError::new(
                ErrorCode::Conflict,
                "identity publication requires empty exclusive storage",
            ));
        }
        out.storage.seek(SeekFrom::Start(0)).map_err(io_error)?;
        for chunk in out.raw.chunks(4096) {
            call.check(true)?;
            out.storage.write_all(chunk).map_err(io_error)?;
        }
        call.check(true)?;
        out.storage
            .flush()
            .and_then(|_| out.storage.sync())
            .map_err(io_error)?;
        out.verify(&call)?;
        call.bound(capsule)?;
        Ok(out)
    }
    pub fn open(mut storage: S, seal: Digest32, c: &OperationContext) -> Result<Self> {
        let mut call = Call::new(c)?;
        storage.validate_identity().map_err(io_error)?;
        let len = storage.seek(SeekFrom::End(0)).map_err(io_error)?;
        if len == 0 || len > MAX_BYTES as u64 {
            return Err(corrupt());
        }
        call.charge(6 * len + 4096)?;
        let mut raw = vec![0; len as usize];
        storage.seek(SeekFrom::Start(0)).map_err(io_error)?;
        for chunk in raw.chunks_mut(4096) {
            call.check(false)?;
            storage.read_exact(chunk).map_err(io_error)?;
        }
        let capsule = decode(&raw, seal)?;
        call.bound(&capsule)?;
        let mut out = Self {
            storage,
            raw,
            seal,
            session: c.session_id,
            fenced: false,
        };
        out.verify(&call)?;
        Ok(out)
    }
    fn verify(&mut self, call: &Call) -> Result<()> {
        call.check(false)?;
        self.storage.validate_identity().map_err(io_error)?;
        if self.storage.seek(SeekFrom::End(0)).map_err(io_error)? != self.raw.len() as u64 {
            return Err(corrupt());
        }
        self.storage.seek(SeekFrom::Start(0)).map_err(io_error)?;
        let mut buffer = [0; 4096];
        for chunk in self.raw.chunks(4096) {
            call.check(false)?;
            self.storage
                .read_exact(&mut buffer[..chunk.len()])
                .map_err(io_error)?;
            if &buffer[..chunk.len()] != chunk {
                return Err(corrupt());
            }
        }
        self.storage.validate_identity().map_err(io_error)?;
        if self.storage.seek(SeekFrom::End(0)).map_err(io_error)? != self.raw.len() as u64 {
            return Err(corrupt());
        }
        call.check(false)
    }
    pub fn recover(&mut self, c: &OperationContext) -> Result<LiveObservationCapsule> {
        if self.fenced {
            return Err(corrupt());
        }
        if self.session != c.session_id {
            return Err(denied(
                "identity archive belongs to another session opening",
            ));
        }
        let mut call = Call::new(c)?;
        call.charge(6 * self.raw.len() as u64 + 4096)?;
        let result = (|| {
            self.verify(&call)?;
            let capsule = decode(&self.raw, self.seal)?;
            call.bound(&capsule)?;
            self.verify(&call)?;
            Ok(capsule)
        })();
        if result
            .as_ref()
            .is_err_and(|e: &DfmcpError| e.code == ErrorCode::CorruptLedger)
        {
            self.fenced = true;
        }
        result
    }
    pub fn seal(&self) -> Digest32 {
        self.seal
    }
    pub fn is_fenced(&self) -> bool {
        self.fenced
    }
}
pub fn create_private_capsule(
    path: &Path,
    capsule: &LiveObservationCapsule,
    c: &OperationContext,
) -> Result<CapsuleStore<PrivateBuildFile>> {
    let mut call = Call::new(c)?;
    call.check(true)?;
    call.bound(capsule)?;
    call.charge(2 * capsule.canonical_bytes.len() as u64 + 4096)?;
    decode(&capsule.canonical_bytes, capsule.content_digest)?;
    let (storage, remaining) = open_private_citizen_identity_storage(path, &call.current()?, true)?;
    CapsuleStore::create(storage, capsule, &remaining)
}
pub fn open_private_capsule(
    path: &Path,
    seal: Digest32,
    c: &OperationContext,
) -> Result<CapsuleStore<PrivateBuildFile>> {
    let call = Call::new(c)?;
    let (storage, remaining) =
        open_private_citizen_identity_storage(path, &call.current()?, false)?;
    CapsuleStore::open(storage, seal, &remaining)
}
