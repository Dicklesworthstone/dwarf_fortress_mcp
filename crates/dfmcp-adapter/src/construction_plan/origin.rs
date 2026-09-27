//! Immutable linkage to every original Rust furnishing-batch placement.
//!
//! This source-selected format differs from the Python directory-based batch
//! format. Exact complete native receipts and both original private identities
//! are retained; an optional later parent stop does not change this origin.
use std::path::PathBuf;

use dfmcp_core::{DfmcpError, Digest32, ErrorCode, Result};

use super::Goal;
use crate::build_placement::journal::private_file::PrivateFileIdentity;
use crate::build_placement::journal::{BuildInventory, MAX_FRAMES, MAX_JOURNAL_BYTES};
use crate::build_placement::{BuildBinding, BuildCapture, BuildPhase, BuildRecord};
use crate::furniture_batch::{BatchDefinition, MAX_DEFINITION_BYTES};

pub const MAX_ORIGIN_BYTES: usize = 256 * 1024;
pub const MAX_DEFINITION_SIZE: usize = MAX_ORIGIN_BYTES + super::MAX_GOAL + 16;
const ORIGIN_MAGIC: &[u8; 8] = b"DFMFRO01";
const DEFINITION_MAGIC: &[u8; 8] = b"DFMFRG01";

fn corrupt() -> DfmcpError {
    DfmcpError::new(
        ErrorCode::CorruptLedger,
        "construction monitoring requires the complete unchanged original furnishing batch",
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
fn field(out: &mut Vec<u8>, value: &[u8]) {
    out.extend_from_slice(&(value.len() as u32).to_be_bytes());
    out.extend_from_slice(value);
}
struct Reader<'a>(&'a [u8]);
impl<'a> Reader<'a> {
    fn take(&mut self, length: usize) -> Result<&'a [u8]> {
        require(length <= self.0.len())?;
        let (value, rest) = self.0.split_at(length);
        self.0 = rest;
        Ok(value)
    }
    fn array<const N: usize>(&mut self) -> Result<[u8; N]> {
        self.take(N)?.try_into().map_err(|_| corrupt())
    }
    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_be_bytes(self.array()?))
    }
    fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_be_bytes(self.array()?))
    }
    fn field(&mut self, maximum: usize) -> Result<&'a [u8]> {
        let length = self.u32()? as usize;
        require(length <= maximum)?;
        self.take(length)
    }
}
fn identity_bytes(identity: &PrivateFileIdentity) -> Result<Vec<u8>> {
    identity.validate()?;
    let mut out = Vec::new();
    field(
        &mut out,
        identity.path.to_str().ok_or_else(corrupt)?.as_bytes(),
    );
    out.extend_from_slice(&identity.file_device.to_be_bytes());
    out.extend_from_slice(&identity.file_inode.to_be_bytes());
    out.extend_from_slice(&identity.file_owner.to_be_bytes());
    out.extend_from_slice(&identity.directory_device.to_be_bytes());
    out.extend_from_slice(&identity.directory_inode.to_be_bytes());
    out.extend_from_slice(&identity.directory_owner.to_be_bytes());
    Ok(out)
}
fn identity_decode(bytes: &[u8]) -> Result<PrivateFileIdentity> {
    let mut reader = Reader(bytes);
    let path = std::str::from_utf8(reader.field(4096)?).map_err(|_| corrupt())?;
    let identity = PrivateFileIdentity {
        path: PathBuf::from(path),
        file_device: reader.u64()?,
        file_inode: reader.u64()?,
        file_owner: reader.u32()?,
        directory_device: reader.u64()?,
        directory_inode: reader.u64()?,
        directory_owner: reader.u32()?,
    };
    require(reader.0.is_empty() && identity_bytes(&identity)? == bytes)?;
    Ok(identity)
}

#[derive(Clone, Debug)]
pub struct Origin {
    definition: BatchDefinition,
    head: Digest32,
    frames: u32,
    byte_len: usize,
    parent: PrivateFileIdentity,
    child: PrivateFileIdentity,
    receipts: Vec<BuildRecord>,
    bytes: Vec<u8>,
    digest: Digest32,
}
impl Origin {
    /// Import every step only after current held owners verified the original
    /// parent and complete original placement journal. Incomplete batches fail.
    pub fn from_batch(
        definition: BatchDefinition,
        binding: &BuildBinding,
        inventory: &BuildInventory,
        parent: PrivateFileIdentity,
        child: PrivateFileIdentity,
    ) -> Result<Self> {
        require(definition.binding() == binding)?;
        require(definition.audit(inventory, false)?.status == "all_placed")?;
        let receipts = definition
            .plan()
            .ordered_steps()
            .map(|step| {
                inventory
                    .entry(&definition.key(step))
                    .and_then(|entry| entry.native())
                    .cloned()
                    .ok_or_else(corrupt)
            })
            .collect::<Result<Vec<_>>>()?;
        Self::assemble(
            definition,
            inventory.head,
            inventory.frames,
            inventory.byte_len,
            parent,
            child,
            receipts,
        )
    }
    fn assemble(
        definition: BatchDefinition,
        head: Digest32,
        frames: u32,
        byte_len: usize,
        parent: PrivateFileIdentity,
        child: PrivateFileIdentity,
        receipts: Vec<BuildRecord>,
    ) -> Result<Self> {
        parent.validate()?;
        child.validate()?;
        require(
            parent.path != child.path
                && (parent.file_device, parent.file_inode) != (child.file_device, child.file_inode)
                && frames > 0
                && frames <= MAX_FRAMES
                && byte_len > 0
                && byte_len <= MAX_JOURNAL_BYTES
                && head.as_bytes() != &[0; 32]
                && receipts.len() == definition.plan().steps().len(),
        )?;
        let mut previous: Option<&BuildCapture> = None;
        for (step, record) in definition.plan().ordered_steps().zip(&receipts) {
            let before = record.plan().before();
            require(
                record.phase() == BuildPhase::Placed
                    && record.resolved()
                    && record.plan().key() == definition.key(step)
                    && before.selection() == step.selection
                    && definition.binding().capture_matches(before),
            )?;
            if let Some(prior) = previous {
                require(
                    before.tick() >= prior.tick()
                        && before.sequence() >= prior.sequence()
                        && before.next_building_id() >= prior.next_building_id()
                        && before.next_job_id() >= prior.next_job_id(),
                )?;
            }
            previous = record.after();
            require(previous.is_some())?;
        }
        let mut bytes = ORIGIN_MAGIC.to_vec();
        field(&mut bytes, &definition.canonical_bytes());
        bytes.extend_from_slice(head.as_bytes());
        bytes.extend_from_slice(&frames.to_be_bytes());
        bytes.extend_from_slice(&(byte_len as u64).to_be_bytes());
        field(&mut bytes, &identity_bytes(&parent)?);
        field(&mut bytes, &identity_bytes(&child)?);
        bytes.push(receipts.len() as u8);
        for receipt in &receipts {
            field(&mut bytes, receipt.canonical_bytes());
        }
        require(bytes.len() <= MAX_ORIGIN_BYTES)?;
        let digest = hash(b"dfmcp.furniture-completion-rust-origin/1\0", &bytes);
        Ok(Self {
            definition,
            head,
            frames,
            byte_len,
            parent,
            child,
            receipts,
            bytes,
            digest,
        })
    }
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        require(bytes.len() <= MAX_ORIGIN_BYTES)?;
        let mut reader = Reader(bytes);
        require(reader.take(8)? == ORIGIN_MAGIC)?;
        let definition = BatchDefinition::decode(reader.field(MAX_DEFINITION_BYTES)?)?;
        let head = Digest32::from_bytes(reader.array()?);
        let frames = reader.u32()?;
        let byte_len = usize::try_from(reader.u64()?).map_err(|_| corrupt())?;
        let parent = identity_decode(reader.field(4140)?)?;
        let child = identity_decode(reader.field(4140)?)?;
        let count = reader.array::<1>()?[0] as usize;
        require((1..=32).contains(&count))?;
        let mut receipts = Vec::with_capacity(count);
        for _ in 0..count {
            receipts.push(BuildRecord::decode(reader.field(6144)?)?);
        }
        require(reader.0.is_empty())?;
        let out = Self::assemble(definition, head, frames, byte_len, parent, child, receipts)?;
        require(out.canonical_bytes() == bytes)?;
        Ok(out)
    }
    pub fn verify_batch(
        &self,
        definition: &BatchDefinition,
        binding: &BuildBinding,
        inventory: &BuildInventory,
        parent: &PrivateFileIdentity,
        child: &PrivateFileIdentity,
    ) -> Result<()> {
        require(
            self.parent == *parent
                && self.child == *child
                && self.definition.canonical_bytes() == definition.canonical_bytes()
                && definition.binding() == binding
                && inventory.journal_id == self.definition.journal_id()
                && inventory.head == self.head
                && inventory.frames == self.frames
                && inventory.byte_len == self.byte_len,
        )?;
        require(definition.audit(inventory, false)?.status == "all_placed")?;
        for (step, receipt) in definition.plan().ordered_steps().zip(&self.receipts) {
            require(
                inventory
                    .entry(&definition.key(step))
                    .and_then(|entry| entry.native())
                    .is_some_and(|record| record.canonical_bytes() == receipt.canonical_bytes()),
            )?;
        }
        Ok(())
    }
    pub fn definition(&self) -> &BatchDefinition {
        &self.definition
    }
    pub fn receipts(&self) -> &[BuildRecord] {
        &self.receipts
    }
    pub fn digest(&self) -> Digest32 {
        self.digest
    }
    pub fn canonical_bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub fn parent_identity(&self) -> &PrivateFileIdentity {
        &self.parent
    }
    pub fn child_identity(&self) -> &PrivateFileIdentity {
        &self.child
    }
    pub fn journal_head(&self) -> Digest32 {
        self.head
    }
    pub fn journal_frames(&self) -> u32 {
        self.frames
    }
    pub fn journal_bytes(&self) -> usize {
        self.byte_len
    }
}

/// Fixed original plan plus fixed receipt condition and timing. The core goal's
/// identity remains the Python-compatible receipt-goal digest; id additionally
/// binds full original-plan custody and cannot be substituted on reopen.
#[derive(Clone, Debug)]
pub struct MonitorDefinition {
    origin: Origin,
    goal: Goal,
    bytes: Vec<u8>,
    id: Digest32,
}
impl MonitorDefinition {
    pub fn new(origin: Origin, goal: Goal) -> Result<Self> {
        require(origin.receipts().len() == goal.records().len())?;
        for receipt in origin.receipts() {
            require(
                goal.records()
                    .iter()
                    .any(|member| member.canonical_bytes() == receipt.canonical_bytes()),
            )?;
        }
        let mut bytes = DEFINITION_MAGIC.to_vec();
        field(&mut bytes, origin.canonical_bytes());
        field(&mut bytes, goal.canonical_bytes());
        require(bytes.len() <= MAX_DEFINITION_SIZE)?;
        let id = hash(b"dfmcp.furniture-completion-rust-goal/1\0", &bytes);
        Ok(Self {
            origin,
            goal,
            bytes,
            id,
        })
    }
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        require(bytes.len() <= MAX_DEFINITION_SIZE)?;
        let mut reader = Reader(bytes);
        require(reader.take(8)? == DEFINITION_MAGIC)?;
        let origin = Origin::decode(reader.field(MAX_ORIGIN_BYTES)?)?;
        let goal = Goal::decode(reader.field(super::MAX_GOAL)?)?;
        require(reader.0.is_empty())?;
        let out = Self::new(origin, goal)?;
        require(out.canonical_bytes() == bytes)?;
        Ok(out)
    }
    pub fn origin(&self) -> &Origin {
        &self.origin
    }
    pub fn goal(&self) -> &Goal {
        &self.goal
    }
    pub fn id(&self) -> Digest32 {
        self.id
    }
    pub fn canonical_bytes(&self) -> &[u8] {
        &self.bytes
    }
}
