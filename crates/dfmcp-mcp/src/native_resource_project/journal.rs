//! Original resource-project custody; child owners remain effect authority.
//!
//! An opening marker is acknowledged before constructing each child. Replay
//! never grants initialization, restores a review, or proves a current goal.
//! The independent ACK refuses valid-prefix rollback even after a lost reply.
use std::io::SeekFrom;
use std::path::Path;
use std::time::{Duration, Instant};

pub(crate) use dfmcp_adapter::build_placement::journal::BuildMode as Mode;
use dfmcp_adapter::build_placement::journal::private_file::{
    PrivateBuildFile, open_private_resource_project_storage,
};
use dfmcp_adapter::control_effect_journal::EffectJournalStorage;
use dfmcp_core::{
    Capability, DfmcpError, Digest32, ErrorCode, GameTick, OperationContext, Result, RiskTier,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub(crate) const MAX_BYTES: usize = 8 * 1024 * 1024;
/// Prepaid complete replay, hashing, both-store verification and publication.
pub(crate) const WORK_BYTES: u64 = 16 * MAX_BYTES as u64;
const MAX_FRAMES: u32 = 4096;
const MAX_ORIGINAL: usize = 64 * 1024;
const MAX_HANDOFF: usize = 64 * 1024;
const MAX_TREE_HANDOFF: usize = 16 * 1024;
const MAX_CHECKPOINT: usize = 32 * 1024;
const MAX_FRAME: usize = 224 * 1024;
const MAX_NODES: usize = 4096;
const ACK_RECORD: usize = 140;
const MAX_ACK_BYTES: usize = MAX_FRAMES as usize * ACK_RECORD;
const MAGIC: &[u8; 8] = b"DFMRPJ01";
const FRAME: &[u8; 8] = b"DFMRPF01";
const ACK: &[u8; 8] = b"DFMRPA01";

fn error(code: ErrorCode, message: &str) -> DfmcpError {
    DfmcpError::new(code, message)
}
fn corrupt() -> DfmcpError {
    error(
        ErrorCode::CorruptLedger,
        "resource-project custody changed, is incomplete or rolled back; preserve both original stores",
    )
}
fn bounded() -> DfmcpError {
    error(
        ErrorCode::BudgetExceeded,
        "resource-project work or retention allowance exhausted",
    )
}
fn conflict() -> DfmcpError {
    error(
        ErrorCode::Conflict,
        "resource-project transition would replace an original child, handoff or terminal stop",
    )
}
fn require(value: bool) -> Result<()> {
    if value { Ok(()) } else { Err(corrupt()) }
}
fn hash(domain: &[u8], bytes: &[u8]) -> Digest32 {
    let mut input = Vec::with_capacity(domain.len() + bytes.len() + 1);
    input.extend_from_slice(domain);
    input.push(0);
    input.extend_from_slice(bytes);
    Digest32::of_bytes(&input)
}
fn parsed_digest(value: &str) -> Result<Digest32> {
    let digest = Digest32::from_hex(value).ok_or_else(corrupt)?;
    require(digest != Digest32::ZERO && digest.to_hex() == value)?;
    Ok(digest)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Stage {
    TreeOpening,
    Tree,
    TreeTransition,
    WorkshopOpening,
    Workshop,
    Transition,
    ProductionOpening,
    Production,
    Cancelled,
}

/// These are closed child recovery descriptors, serialized by their actual
/// owners. The parent never interprets them as reviews or native authority.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Checkpoint {
    pub(crate) workshop: Option<Value>,
    pub(crate) handoff: Option<Value>,
    pub(crate) production: Option<Value>,
    /// A child-minted exact original publication intent, acknowledged before
    /// the child can write that original. Absence keeps v1 canonical bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) production_intent: Option<Value>,
    /// Omitted fields preserve the exact canonical bytes of earlier journals.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) tree: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) tree_handoff: Option<Value>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Snapshot {
    pub(crate) stage: Stage,
    pub(crate) checkpoint: Checkpoint,
    pub(crate) cancelled_from: Option<Stage>,
}
impl Snapshot {
    fn initial(stage: Stage) -> Self {
        Self {
            stage,
            checkpoint: Checkpoint::default(),
            cancelled_from: None,
        }
    }
}

/// Retain this outside both stores, including after failure. A later consumed
/// authority floor is rejection evidence, not a durable observation receipt.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Pin {
    pub(crate) journal_id: String,
    pub(crate) head: String,
    pub(crate) frames: u32,
    pub(crate) authority_tick_floor: u64,
}
impl Pin {
    fn validate(&self) -> Result<()> {
        parsed_digest(&self.journal_id)?;
        parsed_digest(&self.head)?;
        require(self.frames > 0 && self.frames <= MAX_FRAMES)?;
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Header {
    schema: String,
    incarnation: String,
    fortress: String,
    initial_tick: u64,
    original: Value,
    /// Earlier Workshop-first originals have no field here and remain canonical.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    initial_stage: Option<Stage>,
}
impl Header {
    fn initial_stage(&self) -> Result<Stage> {
        match self.initial_stage {
            None => Ok(Stage::WorkshopOpening),
            Some(Stage::TreeOpening) => Ok(Stage::TreeOpening),
            _ => Err(corrupt()),
        }
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Body {
    high_tick: u64,
    snapshot: Snapshot,
}
#[derive(Clone, Copy)]
struct Head {
    digest: Digest32,
    tick: u64,
}
struct Replay {
    header: Header,
    journal_id: Digest32,
    snapshot: Snapshot,
    heads: Vec<Head>,
}

/// Validate shape before serialization; replay is additionally bounded before
/// deserialization. The original and child descriptors must be JSON objects.
fn value_bytes(value: &Value, limit: usize) -> Result<Vec<u8>> {
    if !value.is_object() {
        return Err(conflict());
    }
    let mut stack = vec![(value, 0usize)];
    let mut nodes = 0usize;
    let mut text_bytes = 0usize;
    while let Some((node, depth)) = stack.pop() {
        nodes += 1;
        if nodes > MAX_NODES || depth > 16 {
            return Err(bounded());
        }
        let mut text = |s: &str| -> Result<()> {
            text_bytes = text_bytes.saturating_add(s.len());
            if s.len() > 16 * 1024 || s.as_bytes().contains(&0) || text_bytes > limit {
                return Err(bounded());
            }
            Ok(())
        };
        match node {
            Value::String(s) => text(s)?,
            Value::Array(items) => {
                if items.len() > MAX_NODES.saturating_sub(nodes) {
                    return Err(bounded());
                }
                stack.extend(items.iter().map(|item| (item, depth + 1)));
            }
            Value::Object(items) => {
                if items.len() > MAX_NODES.saturating_sub(nodes) {
                    return Err(bounded());
                }
                for (key, item) in items {
                    text(key)?;
                    stack.push((item, depth + 1));
                }
            }
            Value::Number(n) if !n.is_u64() && !n.is_i64() => return Err(conflict()),
            Value::Null | Value::Bool(_) | Value::Number(_) => {}
        }
    }
    let bytes = serde_json::to_vec(value).map_err(|_| conflict())?;
    if bytes.len() > limit {
        return Err(bounded());
    }
    Ok(bytes)
}

fn validate_snapshot(snapshot: &Snapshot) -> Result<()> {
    let c = &snapshot.checkpoint;
    if let Some(value) = &c.tree {
        value_bytes(value, MAX_CHECKPOINT)?;
    }
    if let Some(value) = &c.tree_handoff {
        value_bytes(value, MAX_TREE_HANDOFF)?;
    }
    if let Some(value) = &c.workshop {
        value_bytes(value, MAX_CHECKPOINT)?;
    }
    if let Some(value) = &c.handoff {
        value_bytes(value, MAX_HANDOFF)?;
    }
    if let Some(value) = &c.production {
        value_bytes(value, MAX_CHECKPOINT)?;
    }
    if let Some(value) = &c.production_intent {
        value_bytes(value, MAX_CHECKPOINT)?;
    }
    let phase = if snapshot.stage == Stage::Cancelled {
        snapshot.cancelled_from.ok_or_else(conflict)?
    } else {
        if snapshot.cancelled_from.is_some() {
            return Err(conflict());
        }
        snapshot.stage
    };
    if c.production_intent.is_some() && phase != Stage::Production {
        return Err(conflict());
    }
    let tree_pair = c.tree.is_some() == c.tree_handoff.is_some();
    let later_children_absent =
        c.workshop.is_none() && c.handoff.is_none() && c.production.is_none();
    let valid = match phase {
        Stage::TreeOpening => c == &Checkpoint::default(),
        Stage::Tree => c.tree.is_some() && c.tree_handoff.is_none() && later_children_absent,
        Stage::TreeTransition => {
            c.tree.is_some() && c.tree_handoff.is_some() && later_children_absent
        }
        Stage::WorkshopOpening => tree_pair && later_children_absent,
        Stage::Workshop => c.workshop.is_some() && c.handoff.is_none() && c.production.is_none(),
        Stage::Transition | Stage::ProductionOpening => {
            c.workshop.is_some() && c.handoff.is_some() && c.production.is_none()
        }
        Stage::Production => c.workshop.is_some() && c.handoff.is_some() && c.production.is_some(),
        Stage::Cancelled => false,
    };
    let tree_phase = matches!(
        phase,
        Stage::TreeOpening | Stage::Tree | Stage::TreeTransition
    );
    if !valid || (!tree_phase && !tree_pair) {
        return Err(conflict());
    }
    if let Some(value) = &c.production_intent {
        let intent = crate::live_production_runtime::ProjectProductionIntent::from_json(value)?;
        intent.validate_checkpoint(c.production.as_ref().ok_or_else(conflict)?)?;
    }
    Ok(())
}

fn transition(old: &Snapshot, new: &Snapshot) -> Result<()> {
    validate_snapshot(new)?;
    let before = &old.checkpoint;
    let after = &new.checkpoint;
    if before.tree_handoff.is_some()
        && (before.tree_handoff != after.tree_handoff || before.tree != after.tree)
    {
        return Err(conflict());
    }
    let tree_active = matches!(old.stage, Stage::TreeOpening | Stage::Tree)
        || (old.stage == Stage::Cancelled && old.cancelled_from == Some(Stage::Tree));
    if !tree_active && (before.tree != after.tree || before.tree_handoff != after.tree_handoff) {
        return Err(conflict());
    }
    if before.handoff.is_some() && before.handoff != after.handoff {
        return Err(conflict());
    }
    if before.handoff.is_some() && before.workshop != after.workshop {
        return Err(conflict());
    }
    if before.production_intent.is_some() && before.production_intent != after.production_intent {
        return Err(conflict());
    }
    if before.production_intent.is_some() {
        // The child owns descriptor semantics. Reuse the parent's ordinary
        // original/floor continuity check so even a checksummed replay cannot
        // erase a published original or newer sticky rejection evidence.
        super::continuity::same_original(
            before.production.as_ref().ok_or_else(conflict)?,
            after.production.as_ref().ok_or_else(conflict)?,
        )?;
    }
    if before.production_intent.is_none() && after.production_intent.is_some() {
        // The intent itself must be independently acknowledged while the
        // existing child still has no original. It cannot be combined with
        // child publication, stage opening, cancellation or a replay update.
        if old.stage != Stage::Production
            || new.stage != Stage::Production
            || before.production != after.production
            || before
                .production
                .as_ref()
                .and_then(|value| value.get("recovery"))
                != Some(&Value::Null)
        {
            return Err(conflict());
        }
    }
    if new.stage == Stage::Cancelled && old.stage != Stage::Cancelled {
        return if new.cancelled_from == Some(old.stage) && before == after {
            Ok(())
        } else {
            Err(conflict())
        };
    }
    if old.stage == Stage::Cancelled {
        if new.stage != Stage::Cancelled || new.cancelled_from != old.cancelled_from {
            return Err(conflict());
        }
        return match old.cancelled_from {
            Some(Stage::Tree)
                if before.workshop == after.workshop
                    && before.handoff == after.handoff
                    && before.production == after.production =>
            {
                Ok(())
            }
            Some(Stage::Workshop) if before.production == after.production => Ok(()),
            Some(Stage::Production) if before.workshop == after.workshop => Ok(()),
            _ if before == after => Ok(()),
            _ => Err(conflict()),
        };
    }
    let allowed = match (old.stage, new.stage) {
        (Stage::TreeOpening, Stage::Tree)
        | (Stage::Tree, Stage::Tree)
        | (Stage::WorkshopOpening, Stage::Workshop)
        | (Stage::Workshop, Stage::Workshop)
        | (Stage::ProductionOpening, Stage::Production)
        | (Stage::Production, Stage::Production) => true,
        (Stage::Tree, Stage::TreeTransition) => before.tree == after.tree,
        (Stage::TreeTransition, Stage::WorkshopOpening) => before == after,
        (Stage::Workshop, Stage::Transition) => before.workshop == after.workshop,
        (Stage::Transition, Stage::ProductionOpening) => before == after,
        (a, b) if a == b => before == after,
        _ => false,
    };
    if allowed { Ok(()) } else { Err(conflict()) }
}

struct Work {
    start: Instant,
    allowance: Duration,
}
impl Work {
    fn new(context: &OperationContext) -> Result<Self> {
        context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
        if context.budget.max_wall_millis > 60_000 || context.budget.max_bytes < WORK_BYTES {
            return Err(bounded());
        }
        Ok(Self {
            start: Instant::now(),
            allowance: Duration::from_millis(context.budget.max_wall_millis),
        })
    }
    fn check(&self) -> Result<()> {
        if self.start.elapsed() < self.allowance {
            Ok(())
        } else {
            Err(bounded())
        }
    }
}
fn read<S: EffectJournalStorage>(storage: &mut S, maximum: usize, work: &Work) -> Result<Vec<u8>> {
    work.check()?;
    storage.validate_identity().map_err(|_| corrupt())?;
    let length = storage.seek(SeekFrom::End(0)).map_err(|_| corrupt())?;
    require(length <= maximum as u64)?;
    storage.seek(SeekFrom::Start(0)).map_err(|_| corrupt())?;
    let mut bytes = vec![0; length as usize];
    storage.read_exact(&mut bytes).map_err(|_| corrupt())?;
    require(storage.seek(SeekFrom::End(0)).map_err(|_| corrupt())? == length)?;
    storage.validate_identity().map_err(|_| corrupt())?;
    work.check()?;
    Ok(bytes)
}

struct Reader<'a>(&'a [u8]);
impl<'a> Reader<'a> {
    fn take(&mut self, length: usize) -> Result<&'a [u8]> {
        if length > self.0.len() {
            return Err(corrupt());
        }
        let (head, tail) = self.0.split_at(length);
        self.0 = tail;
        Ok(head)
    }
    fn number(&mut self) -> Result<u32> {
        Ok(u32::from_be_bytes(
            self.take(4)?.try_into().map_err(|_| corrupt())?,
        ))
    }
    fn digest(&mut self) -> Result<Digest32> {
        Ok(Digest32::from_bytes(
            self.take(32)?.try_into().map_err(|_| corrupt())?,
        ))
    }
}
fn frame(previous: Digest32, number: u32, snapshot: &Snapshot, tick: u64) -> Result<Vec<u8>> {
    validate_snapshot(snapshot)?;
    let body = serde_json::to_vec(&Body {
        high_tick: tick,
        snapshot: snapshot.clone(),
    })
    .map_err(|_| conflict())?;
    if body.len() + 80 > MAX_FRAME {
        return Err(bounded());
    }
    let mut out = FRAME.to_vec();
    out.extend_from_slice(&(body.len() as u32).to_be_bytes());
    out.extend_from_slice(&number.to_be_bytes());
    out.extend_from_slice(previous.as_bytes());
    out.extend_from_slice(&body);
    let digest = hash(b"dfmcp-resource-project-frame/1", &out);
    out.extend_from_slice(digest.as_bytes());
    Ok(out)
}
fn replay(bytes: &[u8], work: &Work) -> Result<Replay> {
    let mut r = Reader(bytes);
    require(r.take(8)? == MAGIC)?;
    let length = r.number()? as usize;
    require(length <= MAX_ORIGINAL + 1024)?;
    let raw = r.take(length)?;
    let header: Header = serde_json::from_slice(raw).map_err(|_| corrupt())?;
    require(serde_json::to_vec(&header).map_err(|_| corrupt())? == raw)?;
    require(header.schema == "dfmcp.native-resource-project/1")?;
    require(!header.incarnation.is_empty() && header.incarnation.len() <= 80)?;
    require(!header.fortress.is_empty() && header.fortress.len() <= 32)?;
    value_bytes(&header.original, MAX_ORIGINAL).map_err(|_| corrupt())?;
    let journal_id = r.digest()?;
    require(journal_id == hash(b"dfmcp-resource-project-original/1", raw))?;
    let mut previous = journal_id;
    let mut high_tick = header.initial_tick;
    let initial_stage = header.initial_stage()?;
    let mut snapshot = Snapshot::initial(initial_stage);
    let mut heads = Vec::new();
    while !r.0.is_empty() {
        work.check()?;
        require(heads.len() < MAX_FRAMES as usize)?;
        let start = r.0;
        require(r.take(8)? == FRAME)?;
        let length = r.number()? as usize;
        require(length + 80 <= MAX_FRAME)?;
        require(r.number()? == heads.len() as u32 + 1)?;
        require(r.digest()? == previous)?;
        let raw = r.take(length)?;
        let body: Body = serde_json::from_slice(raw).map_err(|_| corrupt())?;
        require(serde_json::to_vec(&body).map_err(|_| corrupt())? == raw)?;
        require(body.high_tick >= high_tick)?;
        if heads.is_empty() {
            require(body.snapshot == Snapshot::initial(initial_stage))?;
        } else {
            transition(&snapshot, &body.snapshot).map_err(|_| corrupt())?;
        }
        previous = r.digest()?;
        require(previous == hash(b"dfmcp-resource-project-frame/1", &start[..48 + length]))?;
        high_tick = body.high_tick;
        snapshot = body.snapshot;
        heads.push(Head {
            digest: previous,
            tick: high_tick,
        });
    }
    require(!heads.is_empty())?;
    work.check()?;
    Ok(Replay {
        header,
        journal_id,
        snapshot,
        heads,
    })
}

fn acknowledgement(journal: Digest32, head: Head, frames: u32, previous: Digest32) -> Vec<u8> {
    let mut out = ACK.to_vec();
    out.extend_from_slice(journal.as_bytes());
    out.extend_from_slice(head.digest.as_bytes());
    out.extend_from_slice(&frames.to_be_bytes());
    out.extend_from_slice(previous.as_bytes());
    let digest = hash(b"dfmcp-resource-project-acknowledgement/1", &out);
    out.extend_from_slice(digest.as_bytes());
    out
}
fn replay_ack(bytes: &[u8], replay: &Replay, work: &Work) -> Result<(u32, Digest32)> {
    require(!bytes.is_empty() && bytes.len().is_multiple_of(ACK_RECORD))?;
    let mut chain = Digest32::ZERO;
    let mut frames = 0;
    for raw in bytes.chunks_exact(ACK_RECORD) {
        work.check()?;
        let mut r = Reader(raw);
        require(r.take(8)? == ACK && r.digest()? == replay.journal_id)?;
        let head = r.digest()?;
        let next = r.number()?;
        require(next > frames && next <= replay.heads.len() as u32)?;
        require(replay.heads[next as usize - 1].digest == head)?;
        require(r.digest()? == chain)?;
        chain = r.digest()?;
        require(chain == hash(b"dfmcp-resource-project-acknowledgement/1", &raw[..108]))?;
        frames = next;
    }
    Ok((frames, chain))
}

pub(crate) struct Journal<S, A> {
    storage: S,
    acknowledgements: A,
    raw: Vec<u8>,
    ack_raw: Vec<u8>,
    ack_chain: Digest32,
    original: Value,
    snapshot: Snapshot,
    pin: Pin,
    high_tick: u64,
    recorded_tick: u64,
    opening: OperationContext,
    mode: Mode,
    fenced: bool,
}
pub(crate) type PrivateJournal = Journal<PrivateBuildFile, PrivateBuildFile>;

pub(crate) fn open_private(
    directory: &Path,
    context: &OperationContext,
    mode: Mode,
    initialize: bool,
    original: Value,
    expected_pin: Option<Pin>,
    initial_stage: Stage,
) -> Result<PrivateJournal> {
    Work::new(context)?;
    value_bytes(&original, MAX_ORIGINAL)?;
    require(matches!(
        initial_stage,
        Stage::WorkshopOpening | Stage::TreeOpening
    ))?;
    if initialize {
        require(mode == Mode::Control && expected_pin.is_none())?;
        context.authorize(Capability::Plan, RiskTier::Guarded, &[], None)?;
    } else {
        expected_pin.as_ref().ok_or_else(corrupt)?.validate()?;
    }
    // Both fixed names are operator-selected host custody, never MCP paths.
    let (storage, remaining) = open_private_resource_project_storage(
        &directory.join("resource-project.journal"),
        context,
        mode,
        initialize,
        MAX_BYTES,
    )?;
    let (acknowledgements, remaining) = open_private_resource_project_storage(
        &directory.join("resource-project.ack"),
        &remaining,
        mode,
        initialize,
        MAX_ACK_BYTES,
    )?;
    Journal::open_with_initial(
        storage,
        acknowledgements,
        &remaining,
        mode,
        initialize,
        original,
        expected_pin,
        initial_stage,
    )
}

impl<S: EffectJournalStorage, A: EffectJournalStorage> Journal<S, A> {
    #[cfg(test)]
    pub(crate) fn open(
        storage: S,
        acknowledgements: A,
        context: &OperationContext,
        mode: Mode,
        initialize: bool,
        original: Value,
        expected_pin: Option<Pin>,
    ) -> Result<Self> {
        Self::open_with_initial(
            storage,
            acknowledgements,
            context,
            mode,
            initialize,
            original,
            expected_pin,
            Stage::WorkshopOpening,
        )
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "separate journal/ACK custody and an explicit first child prevent implicit stage adoption"
    )]
    pub(crate) fn open_with_initial(
        mut storage: S,
        mut acknowledgements: A,
        context: &OperationContext,
        mode: Mode,
        initialize: bool,
        original: Value,
        expected_pin: Option<Pin>,
        initial_stage: Stage,
    ) -> Result<Self> {
        let work = Work::new(context)?;
        value_bytes(&original, MAX_ORIGINAL)?;
        require(matches!(
            initial_stage,
            Stage::WorkshopOpening | Stage::TreeOpening
        ))?;
        if initialize {
            require(mode == Mode::Control && expected_pin.is_none())?;
            context.authorize(Capability::Plan, RiskTier::Guarded, &[], None)?;
        } else {
            expected_pin.as_ref().ok_or_else(corrupt)?.validate()?;
        }
        let mut raw = read(&mut storage, MAX_BYTES, &work)?;
        let mut ack_raw = read(&mut acknowledgements, MAX_ACK_BYTES, &work)?;
        if initialize {
            require(raw.is_empty() && ack_raw.is_empty())?;
            let header = serde_json::to_vec(&Header {
                schema: "dfmcp.native-resource-project/1".to_owned(),
                incarnation: format!("{}:{}", context.session_id, context.request_id),
                fortress: context.anchor.fortress_id.to_string(),
                initial_tick: context.anchor.tick.get(),
                original: original.clone(),
                initial_stage: (initial_stage == Stage::TreeOpening).then_some(initial_stage),
            })
            .map_err(|_| conflict())?;
            let id = hash(b"dfmcp-resource-project-original/1", &header);
            raw.extend_from_slice(MAGIC);
            raw.extend_from_slice(&(header.len() as u32).to_be_bytes());
            raw.extend_from_slice(&header);
            raw.extend_from_slice(id.as_bytes());
            raw.extend_from_slice(&frame(
                id,
                1,
                &Snapshot::initial(initial_stage),
                context.anchor.tick.get(),
            )?);
            publish(&mut storage, &[], &raw, MAX_BYTES, &work)?;
            let decoded = replay(&raw, &work)?;
            ack_raw = acknowledgement(id, decoded.heads[0], 1, Digest32::ZERO);
            publish(&mut acknowledgements, &[], &ack_raw, MAX_ACK_BYTES, &work)?;
        }
        let decoded = replay(&raw, &work)?;
        require(decoded.header.initial_stage()? == initial_stage)?;
        require(decoded.header.original == original)?;
        require(decoded.header.fortress == context.anchor.fortress_id.to_string())?;
        let (mut acknowledged_frames, mut ack_chain) = replay_ack(&ack_raw, &decoded, &work)?;
        let mut high_tick = decoded
            .heads
            .last()
            .ok_or_else(corrupt)?
            .tick
            .max(context.anchor.tick.get());
        if let Some(expected) = expected_pin {
            require(expected.journal_id == decoded.journal_id.to_hex())?;
            require(expected.frames <= acknowledged_frames)?;
            let head = decoded
                .heads
                .get(expected.frames as usize - 1)
                .ok_or_else(corrupt)?;
            require(
                expected.head == head.digest.to_hex() && expected.authority_tick_floor >= head.tick,
            )?;
            high_tick = high_tick.max(expected.authority_tick_floor);
        }
        let mut retained = context.clone();
        retained.anchor.tick = GameTick(high_tick);
        retained.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
        if mode == Mode::Offline {
            require(acknowledged_frames == decoded.heads.len() as u32)?;
        } else if !initialize {
            // Synchronize complete surviving bytes before accepting a lost
            // publication. Torn tails, absent ACKs and impossible frames never
            // enter this branch and are never repaired or truncated.
            work.check()?;
            storage.sync().map_err(|_| corrupt())?;
            require(read(&mut storage, MAX_BYTES, &work)? == raw)?;
            acknowledgements.sync().map_err(|_| corrupt())?;
            require(read(&mut acknowledgements, MAX_ACK_BYTES, &work)? == ack_raw)?;
            if acknowledged_frames != decoded.heads.len() as u32 {
                let frames = decoded.heads.len() as u32;
                let record = acknowledgement(
                    decoded.journal_id,
                    decoded.heads[frames as usize - 1],
                    frames,
                    ack_chain,
                );
                let mut next = ack_raw.clone();
                next.extend_from_slice(&record);
                publish(&mut acknowledgements, &ack_raw, &next, MAX_ACK_BYTES, &work)?;
                ack_raw = next;
                (acknowledged_frames, ack_chain) = replay_ack(&ack_raw, &decoded, &work)?;
            }
        }
        let head = decoded.heads[acknowledged_frames as usize - 1];
        let mut out = Self {
            storage,
            acknowledgements,
            raw,
            ack_raw,
            ack_chain,
            original,
            snapshot: decoded.snapshot,
            pin: Pin {
                journal_id: decoded.journal_id.to_hex(),
                head: head.digest.to_hex(),
                frames: acknowledged_frames,
                authority_tick_floor: high_tick,
            },
            high_tick,
            recorded_tick: head.tick,
            opening: context.clone(),
            mode,
            fenced: false,
        };
        out.access(context, false)?;
        out.verify_work(&work)?;
        Ok(out)
    }

    pub(crate) fn original(&self) -> &Value {
        &self.original
    }
    pub(crate) fn snapshot(&self) -> &Snapshot {
        &self.snapshot
    }
    pub(crate) fn is_fenced(&self) -> bool {
        self.fenced
    }
    pub(crate) fn pin(&self) -> Pin {
        let mut pin = self.pin.clone();
        pin.authority_tick_floor = self.high_tick;
        pin
    }
    fn access(&mut self, context: &OperationContext, plan: bool) -> Result<()> {
        if context.session_id != self.opening.session_id
            || context.anchor.fortress_id != self.opening.anchor.fortress_id
        {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "resource-project custody has another session or fortress owner",
            ));
        }
        self.high_tick = self.high_tick.max(context.anchor.tick.get());
        let mut current = context.clone();
        current.anchor.tick = GameTick(self.high_tick);
        let mut original = current.clone();
        original.grants = self.opening.grants.clone();
        current.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
        original.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
        if plan {
            current.authorize(Capability::Plan, RiskTier::Guarded, &[], None)?;
            original.authorize(Capability::Plan, RiskTier::Guarded, &[], None)?;
        }
        if self.fenced { Err(corrupt()) } else { Ok(()) }
    }
    fn verify_work(&mut self, work: &Work) -> Result<()> {
        let checked = (|| {
            require(read(&mut self.storage, MAX_BYTES, work)? == self.raw)?;
            require(read(&mut self.acknowledgements, MAX_ACK_BYTES, work)? == self.ack_raw)?;
            work.check()
        })();
        if checked.is_err() {
            self.fenced = true;
        }
        checked
    }
    pub(crate) fn verify(&mut self, context: &OperationContext) -> Result<()> {
        self.access(context, false)?;
        let work = Work::new(context)?;
        self.verify_work(&work)
    }
    pub(crate) fn preflight_append(&mut self, context: &OperationContext) -> Result<()> {
        self.verify(context)?;
        if self.mode == Mode::Offline {
            return Err(conflict());
        }
        self.capacity(MAX_FRAME, self.snapshot.stage == Stage::Cancelled)
    }
    fn capacity(&self, added: usize, stopping: bool) -> Result<()> {
        let reserve = if stopping { 0 } else { MAX_FRAME };
        let frames = if stopping { 1 } else { 2 };
        if self.raw.len().saturating_add(added).saturating_add(reserve) > MAX_BYTES
            || self.pin.frames.saturating_add(frames) > MAX_FRAMES
            || self
                .ack_raw
                .len()
                .saturating_add(frames as usize * ACK_RECORD)
                > MAX_ACK_BYTES
        {
            Err(bounded())
        } else {
            Ok(())
        }
    }
    pub(crate) fn advance(
        &mut self,
        context: &OperationContext,
        next: Stage,
        checkpoint: Checkpoint,
    ) -> Result<Pin> {
        let snapshot = Snapshot {
            stage: next,
            checkpoint,
            cancelled_from: self.snapshot.cancelled_from,
        };
        self.append(context, snapshot)
    }
    pub(crate) fn cancel(&mut self, context: &OperationContext) -> Result<Pin> {
        let mut snapshot = self.snapshot.clone();
        if snapshot.stage != Stage::Cancelled {
            snapshot.cancelled_from = Some(snapshot.stage);
            snapshot.stage = Stage::Cancelled;
        }
        self.append(context, snapshot)
    }
    fn append(&mut self, context: &OperationContext, snapshot: Snapshot) -> Result<Pin> {
        let starts_stage = self.snapshot.stage != snapshot.stage
            && matches!(
                snapshot.stage,
                Stage::TreeTransition
                    | Stage::WorkshopOpening
                    | Stage::Transition
                    | Stage::ProductionOpening
            );
        let starts_intent = self.snapshot.checkpoint.production_intent.is_none()
            && snapshot.checkpoint.production_intent.is_some();
        self.access(context, starts_stage || starts_intent)?;
        let work = Work::new(context)?;
        if self.mode == Mode::Offline
            || ((starts_stage || starts_intent) && self.mode != Mode::Control)
        {
            return Err(conflict());
        }
        transition(&self.snapshot, &snapshot)?;
        self.verify_work(&work)?;
        if self.snapshot == snapshot && self.recorded_tick == self.high_tick {
            return Ok(self.pin());
        }
        let record = frame(
            parsed_digest(&self.pin.head)?,
            self.pin.frames + 1,
            &snapshot,
            self.high_tick,
        )?;
        self.capacity(record.len(), snapshot.stage == Stage::Cancelled)?;
        let mut next_raw = self.raw.clone();
        next_raw.extend_from_slice(&record);
        let head = Head {
            digest: Digest32::from_bytes(
                record[record.len() - 32..]
                    .try_into()
                    .map_err(|_| corrupt())?,
            ),
            tick: self.high_tick,
        };
        let frames = self.pin.frames + 1;
        let ack = acknowledgement(
            parsed_digest(&self.pin.journal_id)?,
            head,
            frames,
            self.ack_chain,
        );
        let mut next_ack = self.ack_raw.clone();
        next_ack.extend_from_slice(&ack);
        self.fenced = true;
        publish(&mut self.storage, &self.raw, &next_raw, MAX_BYTES, &work)?;
        publish(
            &mut self.acknowledgements,
            &self.ack_raw,
            &next_ack,
            MAX_ACK_BYTES,
            &work,
        )?;
        require(read(&mut self.storage, MAX_BYTES, &work)? == next_raw)?;
        require(read(&mut self.acknowledgements, MAX_ACK_BYTES, &work)? == next_ack)?;
        work.check()?;
        self.raw = next_raw;
        self.ack_raw = next_ack;
        self.ack_chain = Digest32::from_bytes(ack[108..].try_into().map_err(|_| corrupt())?);
        self.snapshot = snapshot;
        self.pin.head = head.digest.to_hex();
        self.pin.frames = frames;
        self.recorded_tick = head.tick;
        self.fenced = false;
        Ok(self.pin())
    }
}

fn publish<S: EffectJournalStorage>(
    storage: &mut S,
    old: &[u8],
    next: &[u8],
    maximum: usize,
    work: &Work,
) -> Result<()> {
    work.check()?;
    require(next.starts_with(old) && next.len() > old.len() && next.len() <= maximum)?;
    storage.validate_identity().map_err(|_| corrupt())?;
    require(storage.seek(SeekFrom::End(0)).map_err(|_| corrupt())? == old.len() as u64)?;
    storage
        .write_all(&next[old.len()..])
        .map_err(|_| corrupt())?;
    work.check()?;
    storage
        .flush()
        .and_then(|_| storage.sync())
        .map_err(|_| corrupt())?;
    require(read(storage, maximum, work)? == next)?;
    Ok(())
}

#[cfg(test)]
#[path = "journal_tests.rs"]
mod tests;
