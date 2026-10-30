//! Private custody of one immutable mining blueprint and a permanent local stop.
//!
//! Native effects remain in the original dig journal. This parent grants no
//! designation authority, recreates no preparation and never repairs evidence.
//! A recovery pin prevents a previously acknowledged stop from disappearing;
//! a complete newer stop can be adopted after a lost acknowledgement.
use std::io::SeekFrom;
use std::path::Path;
use std::time::{Duration, Instant};

use dfmcp_core::{
    Capability, DfmcpError, Digest32, ErrorCode, GameTick, MapCuboid, OperationContext, Result,
    RiskTier, SessionId,
};

use super::{BatchDefinition, MAX_DEFINITION_BYTES};
use crate::build_placement::journal::BuildMode;
use crate::build_placement::journal::private_file::{
    PrivateBuildFile, PrivateFileIdentity, open_private_storage_scoped,
};
use crate::control_effect_journal::EffectJournalStorage;
use crate::dig_designation::journal::DigMode;

pub const MAX_STORE_BYTES: usize = 64 * 1024;
const MAGIC: &[u8; 8] = b"DFMDBJ01";
const HEADER_END: &[u8; 8] = b"DFMDBHE1";
const STOP: &[u8; 8] = b"DFMDBST1";
const STOP_END: &[u8; 8] = b"DFMDBSE1";
const HEADER_OVERHEAD: usize = 8 + 4 + 32 + 8;
const STOP_BYTES: usize = 8 + 32 + 32 + 32 + 8;

fn error(code: ErrorCode, message: &str) -> DfmcpError {
    DfmcpError::new(code, message)
}
fn corrupt() -> DfmcpError {
    error(
        ErrorCode::CorruptLedger,
        "mining blueprint custody or bytes changed; retain the original evidence without repair",
    )
}
fn bounded() -> DfmcpError {
    error(
        ErrorCode::BudgetExceeded,
        "mining blueprint storage allowance exhausted",
    )
}
fn require(value: bool) -> Result<()> {
    if value { Ok(()) } else { Err(corrupt()) }
}
fn hash(domain: &[u8], bytes: &[u8]) -> Digest32 {
    let mut input = Vec::with_capacity(domain.len() + bytes.len());
    input.extend_from_slice(domain);
    input.extend_from_slice(bytes);
    Digest32::of_bytes(&input)
}
fn head(bytes: &[u8]) -> Digest32 {
    hash(b"dfmcp-mining-blueprint-custody/1\0", bytes)
}
fn authorize(context: &OperationContext, definition: Option<&BatchDefinition>) -> Result<()> {
    if let Some(definition) = definition {
        if definition.binding().fortress_id() != context.anchor.fortress_id {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "mining blueprint belongs to another fortress",
            ));
        }
        let current = retained_context(context, definition);
        current.authorize(
            Capability::Query,
            RiskTier::ReadOnly,
            &[],
            Some(definition.binding().scope()),
        )
    } else {
        // Before decoding private custody, validate one actual permitted scope.
        // The exact decoded binding is checked before publication or disclosure.
        query_scope(context).map(|_| ())
    }
}
fn query_scope(context: &OperationContext) -> Result<Option<MapCuboid>> {
    context
        .grants
        .iter()
        .filter(|grant| matches!(grant.capability, Capability::Query | Capability::Admin))
        .find_map(|grant| {
            context
                .authorize(
                    Capability::Query,
                    RiskTier::ReadOnly,
                    &[],
                    grant.scope.map_area,
                )
                .is_ok()
                .then_some(grant.scope.map_area)
        })
        .ok_or_else(|| {
            error(
                ErrorCode::CapabilityDenied,
                "mining blueprint requires current Query authority",
            )
        })
}
fn retained_context(context: &OperationContext, definition: &BatchDefinition) -> OperationContext {
    let mut current = context.clone();
    current.anchor.tick = current
        .anchor
        .tick
        .max(GameTick(definition.bootstrap().tick()));
    current
}
fn authorize_plan(context: &OperationContext, definition: &BatchDefinition) -> Result<()> {
    retained_context(context, definition).authorize(
        Capability::Plan,
        RiskTier::Guarded,
        &[],
        Some(definition.binding().scope()),
    )
}

/// An externally retained parent identity and monotonic stop-state pin.
/// These values identify evidence; they do not authorize control or a path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Recovery {
    pub blueprint_id: Digest32,
    pub head: Digest32,
}
impl Recovery {
    fn validate(self) -> Result<()> {
        if self.blueprint_id == Digest32::ZERO || self.head == Digest32::ZERO {
            return Err(error(
                ErrorCode::InvalidRequest,
                "mining blueprint recovery needs nonzero original identity and custody head",
            ));
        }
        Ok(())
    }
}

struct Work {
    context: OperationContext,
    deadline: Instant,
    bytes: u64,
}
impl Work {
    fn new(context: &OperationContext) -> Result<Self> {
        context.budget.validate()?;
        authorize(context, None)?;
        if context.budget.max_wall_millis > 60_000 {
            return Err(bounded());
        }
        let work = Self {
            context: context.clone(),
            deadline: Instant::now()
                .checked_add(Duration::from_millis(context.budget.max_wall_millis))
                .ok_or_else(bounded)?,
            bytes: context.budget.max_bytes,
        };
        work.check()?;
        Ok(work)
    }
    fn check(&self) -> Result<()> {
        if Instant::now() >= self.deadline {
            return Err(bounded());
        }
        authorize(&self.context, None)
    }
    fn charge(&mut self, bytes: usize) -> Result<()> {
        self.check()?;
        self.bytes = self.bytes.checked_sub(bytes as u64).ok_or_else(bounded)?;
        Ok(())
    }
    fn reserve(&self, bytes: usize) -> Result<()> {
        self.check()?;
        if self.bytes < bytes as u64 {
            return Err(bounded());
        }
        Ok(())
    }
}

fn read<S: EffectJournalStorage>(storage: &mut S, work: &mut Work) -> Result<Vec<u8>> {
    work.check()?;
    storage.validate_identity().map_err(|_| corrupt())?;
    let length = storage.seek(SeekFrom::End(0)).map_err(|_| corrupt())?;
    require(length <= MAX_STORE_BYTES as u64)?;
    work.charge(length as usize)?;
    storage.seek(SeekFrom::Start(0)).map_err(|_| corrupt())?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(length as usize)
        .map_err(|_| bounded())?;
    bytes.resize(length as usize, 0);
    let mut offset = 0;
    while offset < bytes.len() {
        work.check()?;
        let count = storage.read(&mut bytes[offset..]).map_err(|_| corrupt())?;
        require(count > 0)?;
        offset += count;
    }
    work.check()?;
    require(storage.seek(SeekFrom::End(0)).map_err(|_| corrupt())? == length)?;
    storage.validate_identity().map_err(|_| corrupt())?;
    work.check()?;
    Ok(bytes)
}

fn header(definition: &BatchDefinition) -> Result<Vec<u8>> {
    let body = definition.canonical_bytes();
    require(
        !body.is_empty()
            && body.len() <= MAX_DEFINITION_BYTES
            && body.len() <= MAX_STORE_BYTES - HEADER_OVERHEAD - STOP_BYTES,
    )?;
    let mut bytes = Vec::with_capacity(body.len() + HEADER_OVERHEAD);
    bytes.extend_from_slice(MAGIC);
    bytes.extend_from_slice(&(body.len() as u32).to_be_bytes());
    bytes.extend_from_slice(&body);
    let digest = hash(b"dfmcp-mining-blueprint-store/1\0", &bytes);
    bytes.extend_from_slice(digest.as_bytes());
    bytes.extend_from_slice(HEADER_END);
    Ok(bytes)
}
/// Expected initial identity, available before publication so an ambiguous
/// create can return its exact recovery reference without claiming success.
pub fn initial_recovery(definition: &BatchDefinition) -> Result<Recovery> {
    Ok(Recovery {
        blueprint_id: definition.id(),
        head: head(&header(definition)?),
    })
}
fn stop_frame(definition: &BatchDefinition, header: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(STOP_BYTES);
    bytes.extend_from_slice(STOP);
    bytes.extend_from_slice(definition.id().as_bytes());
    bytes.extend_from_slice(head(header).as_bytes());
    let digest = hash(b"dfmcp-mining-blueprint-stop/1\0", &bytes);
    bytes.extend_from_slice(digest.as_bytes());
    bytes.extend_from_slice(STOP_END);
    bytes
}
fn decode(bytes: &[u8]) -> Result<(BatchDefinition, bool, usize)> {
    require(bytes.len() >= HEADER_OVERHEAD && bytes.len() <= MAX_STORE_BYTES)?;
    require(bytes.get(..8) == Some(MAGIC.as_slice()))?;
    let length = u32::from_be_bytes(
        bytes
            .get(8..12)
            .ok_or_else(corrupt)?
            .try_into()
            .map_err(|_| corrupt())?,
    ) as usize;
    require(
        length > 0
            && length <= MAX_DEFINITION_BYTES
            && length <= MAX_STORE_BYTES - HEADER_OVERHEAD - STOP_BYTES,
    )?;
    let definition = BatchDefinition::decode(bytes.get(12..12 + length).ok_or_else(corrupt)?)
        .map_err(|_| corrupt())?;
    let expected = header(&definition)?;
    require(bytes.get(..expected.len()) == Some(expected.as_slice()))?;
    let stopped = bytes.len() != expected.len();
    if stopped {
        require(
            bytes.get(expected.len()..) == Some(stop_frame(&definition, &expected).as_slice()),
        )?;
    }
    Ok((definition, stopped, expected.len()))
}

fn opening(
    context: &OperationContext,
    mode: DigMode,
    expected: Option<&BatchDefinition>,
    create: bool,
    recovery: Option<Recovery>,
) -> Result<()> {
    authorize(context, expected)?;
    context.budget.validate()?;
    if create {
        if mode != DigMode::Control || expected.is_none() || recovery.is_some() {
            return Err(error(
                ErrorCode::InvalidRequest,
                "new mining blueprint custody requires Control mode, its complete definition and no recovery pin",
            ));
        }
        authorize_plan(context, expected.ok_or_else(corrupt)?)?;
    } else {
        let recovery = recovery.ok_or_else(|| {
            error(
                ErrorCode::InvalidRequest,
                "existing mining blueprint custody requires its original recovery pin",
            )
        })?;
        recovery.validate()?;
        if expected.is_some_and(|d| d.id() != recovery.blueprint_id) {
            return Err(error(
                ErrorCode::Conflict,
                "mining blueprint definition and recovery pin differ",
            ));
        }
    }
    Ok(())
}

/// Exclusive parent custody. Verify current bytes before using the cached
/// definition or permanent stop to constrain any native operation.
pub struct BatchStore<S> {
    storage: S,
    definition: BatchDefinition,
    raw: Vec<u8>,
    mode: DigMode,
    owner: SessionId,
    stopped: bool,
    fenced: bool,
}
pub type PrivateBlueprintStore = BatchStore<PrivateBuildFile>;

/// Open fixed operator-selected raw custody for lazy publication after the
/// first native observation. This creates only an empty private file, never a
/// blueprint definition or dispatch permission. The caller must retain it and
/// pass the exact same `create` choice to `BatchStore::open`.
pub fn open_private_blueprint_storage(
    path: &Path,
    context: &OperationContext,
    mode: DigMode,
    create: bool,
    scope: MapCuboid,
) -> Result<(PrivateBuildFile, OperationContext)> {
    MapCuboid::new(scope.min, scope.max)?;
    context.authorize(Capability::Query, RiskTier::ReadOnly, &[], Some(scope))?;
    context.budget.validate()?;
    if create {
        if mode != DigMode::Control {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "new mining blueprint storage requires Control mode",
            ));
        }
        context.authorize(Capability::Plan, RiskTier::Guarded, &[], Some(scope))?;
    }
    let file_mode = match mode {
        DigMode::Control => BuildMode::Control,
        DigMode::Recover => BuildMode::Recover,
        DigMode::Offline => BuildMode::Offline,
    };
    let (storage, created, current) = open_private_storage_scoped(
        path,
        context,
        file_mode,
        create,
        MAX_STORE_BYTES,
        Some(scope),
    )?;
    if created != create {
        return Err(error(
            ErrorCode::Conflict,
            "mining blueprint initialization and existing private custody differ",
        ));
    }
    Ok((storage, current))
}

impl BatchStore<PrivateBuildFile> {
    pub fn private_identity(&self, context: &OperationContext) -> Result<PrivateFileIdentity> {
        self.access(context)?;
        self.storage.private_identity_scoped(
            &retained_context(context, &self.definition),
            self.definition.binding().scope(),
        )
    }
}
impl<S: EffectJournalStorage> BatchStore<S> {
    /// `create` describes exclusively created empty storage, never an
    /// instruction to initialize whatever bytes happen to be at a path.
    /// Reopen retains exact identity and accepts only a monotonic appended stop.
    pub fn open(
        mut storage: S,
        context: &OperationContext,
        mode: DigMode,
        expected: Option<BatchDefinition>,
        create: bool,
        recovery: Option<Recovery>,
    ) -> Result<Self> {
        opening(context, mode, expected.as_ref(), create, recovery)?;
        let current = expected.as_ref().map_or_else(
            || context.clone(),
            |definition| retained_context(context, definition),
        );
        let mut work = Work::new(&current)?;
        let mut raw = read(&mut storage, &mut work)?;
        if create {
            require(raw.is_empty())?;
            let definition = expected.as_ref().ok_or_else(corrupt)?;
            raw = header(definition)?;
            // Prepay the entire publication and final verification. Nothing
            // written below can authorize native work by itself.
            work.reserve(raw.len().saturating_mul(9))?;
            work.charge(raw.len().saturating_mul(4))?;
            let (decoded, stopped, _) = decode(&raw)?;
            require(!stopped && decoded.canonical_bytes() == definition.canonical_bytes())?;
            work.check()?;
            authorize_plan(&work.context, definition)?;
            storage.validate_identity().map_err(|_| corrupt())?;
            require(storage.seek(SeekFrom::End(0)).map_err(|_| corrupt())? == 0)?;
            storage.write_all(&raw).map_err(|_| corrupt())?;
            work.check()?;
            storage
                .flush()
                .and_then(|_| storage.sync())
                .map_err(|_| corrupt())?;
            require(read(&mut storage, &mut work)? == raw)?;
        }
        work.charge(raw.len().saturating_mul(3))?;
        let (definition, stopped, header_len) = decode(&raw)?;
        work.context = retained_context(&work.context, &definition);
        if let Some(expected) = expected {
            require(expected.canonical_bytes() == definition.canonical_bytes())?;
        }
        if let Some(recovery) = recovery {
            require(recovery.blueprint_id == definition.id())?;
            require(
                recovery.head == head(&raw)
                    || (stopped && recovery.head == head(&raw[..header_len])),
            )?;
        }
        authorize(&work.context, Some(&definition))?;
        // A complete surviving stop whose acknowledgement was lost becomes
        // durable before writable recovery can expose it. Offline replay never
        // writes, flushes, syncs or repairs the original file.
        if !create && mode != DigMode::Offline {
            work.reserve(raw.len())?;
            work.check()?;
            storage.validate_identity().map_err(|_| corrupt())?;
            storage.sync().map_err(|_| corrupt())?;
            require(read(&mut storage, &mut work)? == raw)?;
        }
        work.check()?;
        Ok(Self {
            storage,
            definition,
            raw,
            mode,
            owner: work.context.session_id,
            stopped,
            fenced: false,
        })
    }
    pub fn definition(&self) -> &BatchDefinition {
        &self.definition
    }
    /// Identity of the last verified publication, not a custody verification.
    /// A failed stop can leave a complete newer stop; this pin accepts it on
    /// exact recovery while refusing any rollback from an acknowledged stop.
    pub fn recovery(&self) -> Recovery {
        Recovery {
            blueprint_id: self.definition.id(),
            head: self.head(),
        }
    }
    pub fn head(&self) -> Digest32 {
        head(&self.raw)
    }
    pub fn byte_len(&self) -> usize {
        self.raw.len()
    }
    pub fn stopped(&self) -> bool {
        self.stopped || self.fenced
    }
    pub fn durable_stopped(&self) -> bool {
        self.stopped
    }
    pub fn is_fenced(&self) -> bool {
        self.fenced
    }
    fn access(&self, context: &OperationContext) -> Result<()> {
        if context.session_id != self.owner {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "mining blueprint has another session owner",
            ));
        }
        authorize(context, Some(&self.definition))?;
        if self.fenced {
            return Err(corrupt());
        }
        Ok(())
    }
    fn verify_work(&mut self, work: &mut Work) -> Result<()> {
        self.access(&work.context)?;
        match read(&mut self.storage, work) {
            Ok(bytes) if bytes == self.raw => Ok(()),
            Err(cause)
                if matches!(
                    cause.code,
                    ErrorCode::BudgetExceeded | ErrorCode::CancellationRequested
                ) =>
            {
                Err(cause)
            }
            _ => {
                self.fenced = true;
                Err(corrupt())
            }
        }
    }
    pub fn verify(&mut self, context: &OperationContext) -> Result<()> {
        self.access(context)?;
        self.verify_work(&mut Work::new(&retained_context(
            context,
            &self.definition,
        ))?)
    }
    /// Permanently retire future blueprint advancement. Query authority is
    /// sufficient in Control/Recover mode; no native cancellation is dispatched.
    pub fn stop(&mut self, context: &OperationContext) -> Result<()> {
        self.access(context)?;
        if self.mode == DigMode::Offline {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "offline mining blueprint cannot publish a stop",
            ));
        }
        let result = self.publish_stop(context);
        if result.is_err() {
            self.fenced = true;
        }
        result
    }
    fn publish_stop(&mut self, context: &OperationContext) -> Result<()> {
        let mut work = Work::new(&retained_context(context, &self.definition))?;
        self.verify_work(&mut work)?;
        if self.stopped {
            return Ok(());
        }
        let frame = stop_frame(&self.definition, &self.raw);
        let next_len = self
            .raw
            .len()
            .checked_add(frame.len())
            .ok_or_else(bounded)?;
        require(next_len <= MAX_STORE_BYTES)?;
        work.reserve(next_len.saturating_mul(4) + frame.len())?;
        work.charge(next_len.saturating_mul(3) + frame.len())?;
        let mut proposed = Vec::new();
        proposed
            .try_reserve_exact(next_len)
            .map_err(|_| bounded())?;
        proposed.extend_from_slice(&self.raw);
        proposed.extend_from_slice(&frame);
        let (definition, stopped, _) = decode(&proposed)?;
        require(stopped && definition.canonical_bytes() == self.definition.canonical_bytes())?;
        work.check()?;
        self.access(&work.context)?;
        self.fenced = true;
        self.storage.validate_identity().map_err(|_| corrupt())?;
        require(
            self.storage.seek(SeekFrom::End(0)).map_err(|_| corrupt())? == self.raw.len() as u64,
        )?;
        self.storage.write_all(&frame).map_err(|_| corrupt())?;
        work.check()?;
        self.storage
            .flush()
            .and_then(|_| self.storage.sync())
            .map_err(|_| corrupt())?;
        require(read(&mut self.storage, &mut work)? == proposed)?;
        authorize(&work.context, Some(&self.definition))?;
        work.check()?;
        self.raw = proposed;
        self.stopped = true;
        self.fenced = false;
        Ok(())
    }
}

/// Operator-configured path only. Reuses the existing descriptor-pinned raw
/// file shell; BuildMode selects file access and grants no construction action.
/// Missing recovery files and existing initialization files are both refused.
pub fn open_private_blueprint_store(
    path: &Path,
    context: &OperationContext,
    mode: DigMode,
    expected: Option<BatchDefinition>,
    create: bool,
    recovery: Option<Recovery>,
) -> Result<PrivateBlueprintStore> {
    opening(context, mode, expected.as_ref(), create, recovery)?;
    // Known initialization work is checked before a file can be created.
    if let Some(definition) = expected.as_ref().filter(|_| create) {
        let work = Work::new(context)?;
        work.reserve(header(definition)?.len().saturating_mul(9))?;
    }
    let scope = expected
        .as_ref()
        .map(|d| Some(d.binding().scope()))
        .map_or_else(|| query_scope(context), Ok)?;
    let file_mode = match mode {
        DigMode::Control => BuildMode::Control,
        DigMode::Recover => BuildMode::Recover,
        DigMode::Offline => BuildMode::Offline,
    };
    let (storage, created, current) =
        open_private_storage_scoped(path, context, file_mode, create, MAX_STORE_BYTES, scope)?;
    if created != create {
        return Err(error(
            ErrorCode::Conflict,
            "mining blueprint initialization and existing private custody differ",
        ));
    }
    BatchStore::open(storage, &current, mode, expected, create, recovery)
}

#[cfg(test)]
mod tests;
