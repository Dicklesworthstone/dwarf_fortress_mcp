//! Durable associations between semantic plans and one exact native workforce
//! journal. This is coordinator custody, never a decoder for PreparedPlan or a
//! source of authority. The owner must verify this store before consulting the
//! cached association and must refuse adoption of an unassociated native key.
//!
//! Storage is injected by the operator-owned session. Its exclusive private-file
//! custody contract is the same EffectJournalStorage contract as the native
//! coordinator. No client path, journal repair, rebinding, or native call exists
//! here. A new association is published only after append, sync, and byte-exact
//! verification; every ambiguous write fences this handle until verified reopen.

use std::collections::BTreeMap;
use std::io::{self, SeekFrom};
use std::path::Path;
use std::time::{Duration, Instant};

use dfmcp_core::{
    Capability, DfmcpError, Digest32, ErrorCode, FortressId, GameTick, ObservationCursor,
    OperationContext, Result, RiskTier, SessionId, StateAnchor, StepId,
};

use crate::bounded_run::{hash, validate_key};
use crate::build_placement::journal::BuildMode;
use crate::build_placement::journal::private_file::{PrivateBuildFile, open_private_storage};
use crate::control_effect_journal::EffectJournalStorage;

pub const MAX_ASSOCIATIONS: usize = 64;
pub const MAX_STORE_BYTES: usize = 64 * 1024;
const MAGIC: &[u8; 8] = b"DFMSWJ01";
const FRAME: &[u8; 8] = b"DFMSWFR1";
const END: &[u8; 8] = b"DFMSWEN1";
const HEADER_BYTES: usize = 80;
const MAX_BODY_BYTES: usize = 326;
const FRAME_OVERHEAD: usize = 92;

fn fail(code: ErrorCode, message: &str) -> DfmcpError {
    DfmcpError::new(code, message)
}

fn corrupt(message: &str) -> DfmcpError {
    fail(ErrorCode::CorruptLedger, message)
}

fn storage_error(_: io::Error) -> DfmcpError {
    corrupt("semantic workforce association I/O or custody failed; reopen without repair")
}

fn exhausted() -> DfmcpError {
    fail(
        ErrorCode::BudgetExceeded,
        "semantic workforce association work or retention allowance exhausted",
    )
}

fn authorize(context: &OperationContext, fortress: FortressId, write: bool) -> Result<()> {
    if fortress == FortressId::NIL || context.anchor.fortress_id != fortress {
        return Err(fail(
            ErrorCode::CapabilityDenied,
            "semantic workforce association authority belongs to another fortress",
        ));
    }
    context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    if write {
        context.authorize(Capability::Plan, RiskTier::Guarded, &[], None)?;
        context.authorize(Capability::ConfigureLabor, RiskTier::Guarded, &[], None)?;
    }
    Ok(())
}

struct Allowance<'a> {
    context: &'a OperationContext,
    deadline: Instant,
    bytes: u64,
}

impl<'a> Allowance<'a> {
    fn new(context: &'a OperationContext) -> Result<Self> {
        context.budget.validate()?;
        let deadline = Instant::now()
            .checked_add(Duration::from_millis(context.budget.max_wall_millis))
            .ok_or_else(exhausted)?;
        Ok(Self {
            context,
            deadline,
            bytes: context.budget.max_bytes,
        })
    }

    fn check(&self) -> Result<()> {
        self.context
            .authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
        if Instant::now() >= self.deadline {
            return Err(exhausted());
        }
        Ok(())
    }

    fn charge(&mut self, bytes: usize) -> Result<()> {
        self.check()?;
        self.bytes = self
            .bytes
            .checked_sub(u64::try_from(bytes).map_err(|_| exhausted())?)
            .ok_or_else(exhausted)?;
        Ok(())
    }

    fn records(&self, count: usize) -> Result<()> {
        self.check()?;
        if count > self.context.budget.max_entities as usize {
            return Err(exhausted());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Association {
    pub(super) key: String,
    pub(super) semantic: Digest32,
    pub(super) step: StepId,
    pub(super) anchor: StateAnchor,
    pub(super) source: Digest32,
    pub(super) native: Digest32,
    pub(super) witness: Digest32,
}

impl Association {
    fn validate(&self, fortress: FortressId) -> Result<()> {
        validate_key(&self.key)?;
        if self.anchor.fortress_id == FortressId::NIL
            || self.anchor.fortress_id != fortress
            || [
                self.semantic,
                self.anchor.state_hash,
                self.source,
                self.native,
                self.witness,
            ]
            .contains(&Digest32::ZERO)
        {
            return Err(fail(
                ErrorCode::InvalidRequest,
                "semantic workforce association requires exact nonzero identities in one fortress",
            ));
        }
        Ok(())
    }

    fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(MAX_BODY_BYTES);
        out.extend_from_slice(&(self.key.len() as u16).to_be_bytes());
        out.extend_from_slice(self.key.as_bytes());
        out.extend_from_slice(self.semantic.as_bytes());
        out.extend_from_slice(&self.step.get().to_be_bytes());
        out.extend_from_slice(&self.anchor.fortress_id.get().to_be_bytes());
        out.extend_from_slice(&self.anchor.cursor.epoch.to_be_bytes());
        out.extend_from_slice(&self.anchor.cursor.sequence.to_be_bytes());
        out.extend_from_slice(&self.anchor.tick.get().to_be_bytes());
        out.extend_from_slice(self.anchor.state_hash.as_bytes());
        out.extend_from_slice(self.source.as_bytes());
        out.extend_from_slice(self.native.as_bytes());
        out.extend_from_slice(self.witness.as_bytes());
        out
    }

    fn decode(raw: &[u8], fortress: FortressId) -> Result<Self> {
        if raw.len() > MAX_BODY_BYTES {
            return Err(corrupt(
                "semantic workforce association exceeds its frame bound",
            ));
        }
        let mut reader = Reader(raw);
        let key_len = usize::from(u16::from_be_bytes(reader.array()?));
        if key_len == 0 || key_len > 128 {
            return Err(corrupt("invalid semantic workforce association key length"));
        }
        let key = std::str::from_utf8(reader.take(key_len)?)
            .map_err(|_| corrupt("invalid semantic workforce association key encoding"))?
            .to_owned();
        let association = Self {
            key,
            semantic: Digest32::from_bytes(reader.array()?),
            step: StepId::new(u32::from_be_bytes(reader.array()?)),
            anchor: StateAnchor {
                fortress_id: FortressId::new(u64::from_be_bytes(reader.array()?)),
                cursor: ObservationCursor {
                    epoch: u64::from_be_bytes(reader.array()?),
                    sequence: u64::from_be_bytes(reader.array()?),
                },
                tick: GameTick::new(u64::from_be_bytes(reader.array()?)),
                state_hash: Digest32::from_bytes(reader.array()?),
            },
            source: Digest32::from_bytes(reader.array()?),
            native: Digest32::from_bytes(reader.array()?),
            witness: Digest32::from_bytes(reader.array()?),
        };
        reader.finish()?;
        association
            .validate(fortress)
            .map_err(|_| corrupt("invalid semantic workforce association identity"))?;
        Ok(association)
    }

    /// Review identity of the complete immutable association. Only the parent
    /// session constructs associations; it validates the original PreparedPlan
    /// and exact source before returning this seal to its caller.
    pub(super) fn seal(&self, native_journal: Digest32) -> Digest32 {
        let mut out = Vec::with_capacity(32 + MAX_BODY_BYTES);
        out.extend_from_slice(native_journal.as_bytes());
        out.extend_from_slice(&self.encode());
        hash(b"dfmcp-semantic-workforce-review/1", &out)
    }
}

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, count: usize) -> Result<&'a [u8]> {
        let out = self
            .0
            .get(..count)
            .ok_or_else(|| corrupt("truncated semantic workforce association store"))?;
        self.0 = &self.0[count..];
        Ok(out)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N]> {
        let mut out = [0; N];
        out.copy_from_slice(self.take(N)?);
        Ok(out)
    }

    fn finish(self) -> Result<()> {
        if !self.0.is_empty() {
            return Err(corrupt("trailing semantic workforce association bytes"));
        }
        Ok(())
    }
}

/// An append-only association journal paired with one exact native journal.
/// The storage owner supplies exclusive custody and decides whether an empty
/// file was exclusively created. Reopening never restores an old session grant.
pub struct AssociationStore<S> {
    storage: S,
    native_journal: Digest32,
    fortress: FortressId,
    session: SessionId,
    raw: Vec<u8>,
    head: Digest32,
    records: BTreeMap<String, Association>,
    read_only: bool,
    fenced: bool,
}

/// The existing descriptor-pinned, exclusively locked Linux private-file shell.
/// The associated bytes use only this semantic codec, never the placement codec.
pub type PrivateAssociationStore = AssociationStore<PrivateBuildFile>;

/// Open an operator-configured association file for one exact native journal.
/// Paths must never come from MCP tool arguments. The existing private-file
/// shell enforces normalized absolute paths, no symlinks, a real 0700 parent,
/// one 0600 regular file, single-link ownership, and file plus directory sync.
///
/// Initialization exclusively creates a missing file under current Query,
/// Plan, and ConfigureLabor authority; an existing file is never initialized.
/// Recovery requires an existing complete file and current Query authority.
/// Read-only recovery never writes, syncs, repairs, or creates storage. Current
/// write grants are checked again by retain, independently of opening grants.
pub fn open_private_association_store(
    path: &Path,
    native_journal: Digest32,
    initialize: bool,
    read_only: bool,
    context: &OperationContext,
) -> Result<PrivateAssociationStore> {
    authorize(context, context.anchor.fortress_id, initialize)?;
    if native_journal == Digest32::ZERO {
        return Err(fail(
            ErrorCode::InvalidRequest,
            "semantic workforce association store requires a native journal identity",
        ));
    }
    if initialize && read_only {
        return Err(fail(
            ErrorCode::CapabilityDenied,
            "read-only semantic workforce association storage cannot initialize",
        ));
    }
    if initialize && context.budget.max_bytes < (2 * HEADER_BYTES) as u64 {
        return Err(exhausted());
    }
    // This reusable raw storage factory requires Query, not placement authority.
    // BuildMode selects filesystem access only; it does not select a codec or
    // grant any game capability. The semantic header is always verified below.
    let mode = if read_only {
        BuildMode::Offline
    } else if initialize {
        BuildMode::Control
    } else {
        BuildMode::Recover
    };
    let (storage, created, current) =
        open_private_storage(path, context, mode, initialize, MAX_STORE_BYTES)?;
    if created != initialize {
        return Err(fail(
            ErrorCode::Conflict,
            "semantic workforce association initialization requires newly created private storage",
        ));
    }
    AssociationStore::open(storage, native_journal, created, read_only, &current)
}

impl<S: EffectJournalStorage> AssociationStore<S> {
    /// Initialize only storage exclusively created empty by the trusted owner.
    /// Every existing byte must decode canonically; incomplete tails are never
    /// repaired. An online reopen syncs complete surviving bytes before use. A
    /// read-only reopen performs no write, flush, sync, or native operation.
    pub fn open(
        mut storage: S,
        native_journal: Digest32,
        initialize: bool,
        read_only: bool,
        context: &OperationContext,
    ) -> Result<Self> {
        authorize(context, context.anchor.fortress_id, initialize)?;
        if native_journal == Digest32::ZERO {
            return Err(fail(
                ErrorCode::InvalidRequest,
                "semantic workforce association store requires a native journal identity",
            ));
        }
        if initialize && read_only {
            return Err(fail(
                ErrorCode::CapabilityDenied,
                "read-only semantic workforce association storage cannot initialize",
            ));
        }
        let mut budget = Allowance::new(context)?;
        storage.validate_identity().map_err(storage_error)?;
        let length = storage.seek(SeekFrom::End(0)).map_err(storage_error)?;
        if length > MAX_STORE_BYTES as u64 {
            return Err(exhausted());
        }
        let length = length as usize;
        budget.charge(length)?;
        let mut raw = Vec::new();
        raw.try_reserve_exact(length).map_err(|_| exhausted())?;
        raw.resize(length, 0);
        storage.seek(SeekFrom::Start(0)).map_err(storage_error)?;
        storage.read_exact(&mut raw).map_err(storage_error)?;
        storage.validate_identity().map_err(storage_error)?;

        if initialize {
            if !raw.is_empty() {
                return Err(fail(
                    ErrorCode::Conflict,
                    "semantic workforce association initialization requires empty exclusive storage",
                ));
            }
            // Preflight both initialization bytes and its final verification.
            budget.charge(2 * HEADER_BYTES)?;
            raw.try_reserve_exact(HEADER_BYTES)
                .map_err(|_| exhausted())?;
            raw.extend_from_slice(MAGIC);
            raw.extend_from_slice(native_journal.as_bytes());
            raw.extend_from_slice(&context.anchor.fortress_id.get().to_be_bytes());
            let digest = hash(b"dfmcp-semantic-workforce-store/1", &raw);
            raw.extend_from_slice(digest.as_bytes());
            authorize(context, context.anchor.fortress_id, true)?;
            budget.check()?;
            storage.validate_identity().map_err(storage_error)?;
            if storage.seek(SeekFrom::End(0)).map_err(storage_error)? != 0 {
                return Err(corrupt(
                    "semantic workforce association storage changed before initialization",
                ));
            }
            storage
                .write_all(&raw)
                .and_then(|_| storage.flush())
                .and_then(|_| storage.sync())
                .map_err(storage_error)?;
        }

        let mut reader = Reader(&raw);
        if reader.take(8)? != MAGIC {
            return Err(corrupt(
                "unknown semantic workforce association store format",
            ));
        }
        let stored_native = Digest32::from_bytes(reader.array()?);
        let fortress = FortressId::new(u64::from_be_bytes(reader.array()?));
        let header_end = raw.len() - reader.0.len();
        let mut head = Digest32::from_bytes(reader.array()?);
        if head != hash(b"dfmcp-semantic-workforce-store/1", &raw[..header_end]) {
            return Err(corrupt(
                "semantic workforce association header checksum failed",
            ));
        }
        if stored_native != native_journal || fortress != context.anchor.fortress_id {
            return Err(fail(
                ErrorCode::Conflict,
                "semantic workforce association store belongs to another native journal or fortress",
            ));
        }
        let mut records = BTreeMap::new();
        while !reader.0.is_empty() {
            budget.records(records.len() + 1)?;
            if records.len() >= MAX_ASSOCIATIONS {
                return Err(corrupt(
                    "semantic workforce association history exceeds its key bound",
                ));
            }
            let start = raw.len() - reader.0.len();
            if reader.take(8)? != FRAME {
                return Err(corrupt(
                    "invalid semantic workforce association frame magic",
                ));
            }
            let body_len = u32::from_be_bytes(reader.array()?) as usize;
            let sequence = u64::from_be_bytes(reader.array()?);
            let previous = Digest32::from_bytes(reader.array()?);
            if body_len > MAX_BODY_BYTES || sequence != records.len() as u64 + 1 || previous != head
            {
                return Err(corrupt(
                    "invalid semantic workforce association frame chain",
                ));
            }
            let association = Association::decode(reader.take(body_len)?, fortress)?;
            let end = raw.len() - reader.0.len();
            let digest = Digest32::from_bytes(reader.array()?);
            if digest != hash(b"dfmcp-semantic-workforce-frame/1", &raw[start..end])
                || reader.take(8)? != END
                || records.contains_key(&association.key)
            {
                return Err(corrupt(
                    "semantic workforce association checksum or immutable-key rule failed",
                ));
            }
            records.insert(association.key.clone(), association);
            head = digest;
        }

        authorize(context, fortress, false)?;
        budget.check()?;
        if !initialize {
            budget.charge(raw.len())?;
            if !read_only {
                // A complete frame from a failed prior acknowledgement can be
                // made durable. This does not authorize resubmitting its effect.
                storage.validate_identity().map_err(storage_error)?;
                storage.sync().map_err(storage_error)?;
            }
        }
        Self::verify_bytes(&mut storage, &raw, &budget)?;
        Ok(Self {
            storage,
            native_journal,
            fortress,
            session: context.session_id,
            raw,
            head,
            records,
            read_only,
            fenced: false,
        })
    }

    pub fn native_journal(&self) -> Digest32 {
        self.native_journal
    }

    pub fn byte_len(&self) -> usize {
        self.raw.len()
    }

    pub fn is_fenced(&self) -> bool {
        self.fenced
    }

    /// Offline association replay does not promote surviving bytes to durable
    /// custody. The semantic owner must refuse effectful control in this mode.
    pub fn is_read_only(&self) -> bool {
        self.read_only
    }

    /// The owner must complete verify(context) in the same bounded operation
    /// before consulting this cache. No file I/O or authority restoration occurs.
    pub(super) fn get(&self, key: &str) -> Option<&Association> {
        if self.fenced {
            return None;
        }
        self.records.get(key)
    }

    fn access(&self, context: &OperationContext, write: bool) -> Result<()> {
        if self.fenced {
            return Err(corrupt(
                "semantic workforce association store is fenced; reopen for verified recovery",
            ));
        }
        if context.session_id != self.session || (write && self.read_only) {
            return Err(fail(
                ErrorCode::CapabilityDenied,
                "semantic workforce association session or read-only mode denies this operation",
            ));
        }
        authorize(context, self.fortress, write)
    }

    /// Rechecks identity, exact extent, and every cached byte. Its byte cost is
    /// exactly byte_len(); the caller also retains its aggregate wall deadline.
    pub fn verify(&mut self, context: &OperationContext) -> Result<()> {
        self.access(context, false)?;
        let mut budget = Allowance::new(context)?;
        budget.records(self.records.len())?;
        budget.charge(self.raw.len())?;
        let result = Self::verify_bytes(&mut self.storage, &self.raw, &budget);
        if result
            .as_ref()
            .is_err_and(|error| error.code == ErrorCode::CorruptLedger)
        {
            self.fenced = true;
        }
        result
    }

    fn verify_bytes(storage: &mut S, expected: &[u8], budget: &Allowance<'_>) -> Result<()> {
        budget.check()?;
        storage.validate_identity().map_err(storage_error)?;
        if storage.seek(SeekFrom::End(0)).map_err(storage_error)? != expected.len() as u64 {
            return Err(corrupt("semantic workforce association extent changed"));
        }
        storage.seek(SeekFrom::Start(0)).map_err(storage_error)?;
        let mut buffer = [0u8; 4096];
        for chunk in expected.chunks(buffer.len()) {
            budget.check()?;
            storage
                .read_exact(&mut buffer[..chunk.len()])
                .map_err(storage_error)?;
            if &buffer[..chunk.len()] != chunk {
                return Err(corrupt("semantic workforce association bytes changed"));
            }
        }
        storage.validate_identity().map_err(storage_error)?;
        if storage.seek(SeekFrom::End(0)).map_err(storage_error)? != expected.len() as u64 {
            return Err(corrupt(
                "semantic workforce association extent changed during verification",
            ));
        }
        budget.check()
    }

    /// Record exactly once before native prepare. Exact retries do not append;
    /// changed bindings conflict even when the old native operation is terminal.
    /// The caller must separately reject an existing unassociated native key.
    ///
    /// A new record charges old bytes twice (verification and staged copy),
    /// encoded retained records, body and frame construction, frame write, and
    /// the full new-byte verification. All roots and collection allocations are
    /// prepared before the first write; ambiguous persistence fences this handle.
    pub(super) fn retain(&mut self, value: Association, context: &OperationContext) -> Result<()> {
        self.access(context, true)?;
        value.validate(self.fortress)?;
        let mut budget = Allowance::new(context)?;
        budget.records(self.records.len())?;
        budget.charge(self.raw.len())?;
        let verified = Self::verify_bytes(&mut self.storage, &self.raw, &budget);
        if verified
            .as_ref()
            .is_err_and(|error| error.code == ErrorCode::CorruptLedger)
        {
            self.fenced = true;
        }
        verified?;
        if let Some(old) = self.records.get(&value.key) {
            if *old == value {
                return Ok(());
            }
            return Err(fail(
                ErrorCode::Conflict,
                "semantic workforce native key already binds another immutable association",
            ));
        }
        if self.records.len() >= MAX_ASSOCIATIONS {
            return Err(exhausted());
        }
        budget.records(self.records.len() + 1)?;

        budget.charge(MAX_BODY_BYTES)?;
        let body = value.encode();
        let frame_len = body.len() + FRAME_OVERHEAD;
        let next_len = self
            .raw
            .len()
            .checked_add(frame_len)
            .ok_or_else(exhausted)?;
        if next_len > MAX_STORE_BYTES {
            return Err(exhausted());
        }
        let copy_bytes = self.records.len() * MAX_BODY_BYTES;
        // Reserve complete bounded work before any persistence. The fixed
        // per-record bound also charges the staged map without allocating it.
        budget.charge(self.raw.len() + copy_bytes + 3 * frame_len + next_len)?;
        let mut frame = Vec::new();
        frame
            .try_reserve_exact(frame_len)
            .map_err(|_| exhausted())?;
        frame.extend_from_slice(FRAME);
        frame.extend_from_slice(&(body.len() as u32).to_be_bytes());
        frame.extend_from_slice(&(self.records.len() as u64 + 1).to_be_bytes());
        frame.extend_from_slice(self.head.as_bytes());
        frame.extend_from_slice(&body);
        let head = hash(b"dfmcp-semantic-workforce-frame/1", &frame);
        frame.extend_from_slice(head.as_bytes());
        frame.extend_from_slice(END);

        let mut raw = Vec::new();
        raw.try_reserve_exact(next_len).map_err(|_| exhausted())?;
        raw.extend_from_slice(&self.raw);
        raw.extend_from_slice(&frame);
        let mut records = self.records.clone();
        records.insert(value.key.clone(), value);
        self.access(context, true)?;
        budget.check()?;

        let result = (|| {
            self.storage.validate_identity().map_err(storage_error)?;
            if self.storage.seek(SeekFrom::End(0)).map_err(storage_error)? != self.raw.len() as u64
            {
                return Err(corrupt(
                    "semantic workforce association extent changed before append",
                ));
            }
            self.storage
                .write_all(&frame)
                .and_then(|_| self.storage.flush())
                .and_then(|_| self.storage.sync())
                .map_err(storage_error)?;
            Self::verify_bytes(&mut self.storage, &raw, &budget)
        })();
        if result.is_err() {
            self.fenced = true;
        }
        result?;
        // No fallible publication work remains after durable verification.
        self.raw = raw;
        self.records = records;
        self.head = head;
        Ok(())
    }
}
