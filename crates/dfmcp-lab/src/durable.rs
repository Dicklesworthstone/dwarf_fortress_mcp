//! Crash-durable storage for laboratory fortresses.
//!
//! A laboratory fortress opened as durable survives process loss: its latest
//! world state and every checkpoint live in an operator-chosen directory.
//!
//! Layout (directory `0700`, files `0600`, no symbolic links):
//!
//! ```text
//! <root>/journal      append-only, hash-chained record log (exclusively locked)
//! <root>/objects/<sha256>.snap   canonical snapshot bytes, content addressed
//! ```
//!
//! Every object is written to a temporary file, synced, renamed into place and
//! the directory synced **before** the journal record naming it is appended
//! and synced, so a crash at any point leaves either the previous record set
//! or the new one, never a record that names missing bytes.
//!
//! Each journal line is `<chain> <payload>\n` where
//! `chain = SHA-256("dfmcp-lab-journal/1\0" || previous chain || payload)`.
//! On open, an incomplete final line (a torn append) is discarded and the
//! journal truncated to the last complete record; that is the only repair.
//! A complete record whose chain, syntax, or object fails verification is a
//! corrupt ledger and the store refuses to open rather than guess.
//!
//! Snapshots are stored as [`WorldSnapshot::canonical_bytes`] and decoded with
//! the strict canonical decoder, so a recovered world has exactly the state
//! hash it had when it was written. Recovery never resurrects action handles:
//! temporal work that lives in the world (designations, construction, work
//! orders) continues; agent-side obligations must be re-established from
//! observation.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use dfmcp_core::{
    CheckpointId, DfmcpError, Digest32, ErrorCode, FortressId, GameTick, ObservationCursor, Result,
    SessionId, StateAnchor,
};
use dfmcp_world::WorldSnapshot;

const JOURNAL_DOMAIN: &[u8] = b"dfmcp-lab-journal/1\0";
/// Largest journal accepted on open.
pub const MAX_JOURNAL_BYTES: u64 = 64 * 1024 * 1024;
/// Largest single journal record line.
pub const MAX_RECORD_BYTES: usize = 64 * 1024;
/// A progress record can cover every admitted commit in one atomic publication.
/// Ordinary journal records retain their smaller limit.
pub const MAX_PROGRESS_RECORD_BYTES: usize = 8 * 1024 * 1024;
/// Largest stored snapshot object.
pub const MAX_OBJECT_BYTES: u64 = 64 * 1024 * 1024;
/// Records after which the journal is compacted to live heads + checkpoints.
pub const COMPACT_AFTER_RECORDS: usize = 1_024;
/// Most durable fortresses one store holds.
pub const MAX_FORTRESSES: usize = 256;
/// Most durable checkpoints per fortress.
pub const MAX_CHECKPOINTS_PER_FORTRESS: usize = 256;
const MAX_SCENARIO_BYTES: usize = 64;
const MAX_LABEL_BYTES: usize = 256;
/// Largest stored plan summary.
pub const MAX_PLAN_SUMMARY_BYTES: usize = 4 * 1024;
/// Largest stored plan request (actions, blueprint or production JSON).
pub const MAX_PLAN_REQUEST_BYTES: usize = 16 * 1024;
/// Most unfinished durable commits per fortress.
pub const MAX_COMMITS_PER_FORTRESS: usize = 256;
/// Most retained original objectives per fortress. Unresolved objectives are
/// never evicted to admit new work.
pub const MAX_OBJECTIVES_PER_FORTRESS: usize = 64;
/// Most step records per durable commit.
pub const MAX_STEPS_PER_COMMIT: usize = 256;
/// Largest atomic step frontier, including all unfinished plans of a fortress.
pub const MAX_PROGRESS_UPDATES: usize = MAX_COMMITS_PER_FORTRESS * MAX_STEPS_PER_COMMIT;
/// Step states a durable commit may record. Every state but `dispatched` is
/// final for recovery purposes.
pub const STEP_STATES: [&str; 7] = [
    "dispatched",
    "verified",
    "failed",
    "cancelled",
    "compensated",
    "not_dispatched",
    "abandoned",
];

fn invalid(message: impl Into<String>) -> DfmcpError {
    DfmcpError::new(ErrorCode::InvalidRequest, message)
}

fn corrupt(message: impl Into<String>) -> DfmcpError {
    DfmcpError::new(ErrorCode::CorruptLedger, message).retryable(false)
}

fn io(context: &str, error: &std::io::Error) -> DfmcpError {
    DfmcpError::new(ErrorCode::AdapterUnavailable, format!("{context}: {error}"))
}

/// The latest durable world state of one fortress.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DurableHead {
    pub fortress_id: FortressId,
    pub scenario: String,
    pub anchor: StateAnchor,
}

/// One durable checkpoint.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DurableCheckpoint {
    pub fortress_id: FortressId,
    pub checkpoint_id: CheckpointId,
    pub label: String,
    pub state_hash: Digest32,
}

/// The agent request behind a committed plan, kept so the exact sealed plan
/// can be deterministically recompiled from the world it was sealed against.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DurablePlanSource {
    Pause {
        summary: String,
        paused: bool,
    },
    Actions {
        summary: String,
        raw: String,
    },
    Blueprint {
        summary: String,
        raw: String,
    },
    /// Original quota request; never inferred from a legacy action list.
    Production {
        summary: String,
        raw: String,
    },
    /// A separately reviewed pursuit retaining the original production request
    /// and flat parent/root lineage. Reopening never grants dispatch authority.
    ProductionContinuation {
        summary: String,
        raw: String,
    },
}

impl DurablePlanSource {
    fn fields(&self) -> (&str, &str, &str) {
        match self {
            Self::Pause { summary, paused } => {
                ("pause", summary, if *paused { "true" } else { "false" })
            }
            Self::Actions { summary, raw } => ("actions", summary, raw),
            Self::Blueprint { summary, raw } => ("blueprint", summary, raw),
            Self::Production { summary, raw } => ("production", summary, raw),
            Self::ProductionContinuation { summary, raw } => {
                ("production_continuation", summary, raw)
            }
        }
    }

    fn validate_bound(&self) -> Result<()> {
        let (_, summary, raw) = self.fields();
        if summary.len() > MAX_PLAN_SUMMARY_BYTES || raw.len() > MAX_PLAN_REQUEST_BYTES {
            return Err(invalid("plan request exceeds the durable record bound"));
        }
        Ok(())
    }
}

/// An original committed objective, retained independently of its action work.
///
/// The source and sealed world reconstruct the exact original predicate. The
/// first satisfaction anchor records immutable history, not current truth.
/// Restore abandonment prevents later worlds from newly satisfying this goal.
/// Neither anchor grants dispatch or observation authority to the store.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DurableObjective {
    pub fortress_id: FortressId,
    pub plan_digest: Digest32,
    pub sealed_state_hash: Digest32,
    pub intent_id: u128,
    pub source: DurablePlanSource,
    pub owner_session_id: SessionId,
    pub first_satisfied_anchor: Option<StateAnchor>,
    pub restore_abandoned_anchor: Option<StateAnchor>,
}

impl DurableObjective {
    fn validate(&self) -> Result<()> {
        if self.owner_session_id == SessionId::NIL || self.intent_id == 0 {
            return Err(corrupt(
                "durable objective owner and intent must be nonzero",
            ));
        }
        self.source.validate_bound()?;
        if self
            .first_satisfied_anchor
            .iter()
            .chain(self.restore_abandoned_anchor.iter())
            .any(|anchor| anchor.fortress_id != self.fortress_id)
        {
            return Err(corrupt("objective evidence belongs to another fortress"));
        }
        Ok(())
    }

    fn opening_commit(&self) -> DurableCommit {
        DurableCommit {
            fortress_id: self.fortress_id,
            plan_digest: self.plan_digest,
            sealed_state_hash: self.sealed_state_hash,
            intent_id: self.intent_id,
            source: self.source.clone(),
            steps: BTreeMap::new(),
            step_anchors: BTreeMap::new(),
        }
    }
}

/// A committed plan whose steps are not all final.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DurableCommit {
    pub fortress_id: FortressId,
    pub plan_digest: Digest32,
    /// State hash of the world the plan was sealed against (a stored object).
    pub sealed_state_hash: Digest32,
    pub intent_id: u128,
    pub source: DurablePlanSource,
    /// Last recorded state per step id. An absent state proves no recovered
    /// laboratory effect only after an atomic progress publication.
    pub steps: BTreeMap<u32, String>,
    /// The exact world published with each step transition. Legacy independent
    /// S records lack this binding and cannot certify historical completion.
    pub step_anchors: BTreeMap<u32, StateAnchor>,
}

/// One step transition published with the exact world that supports it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DurableStepUpdate {
    pub plan_digest: Digest32,
    pub step: u32,
    pub state: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Record {
    Head(DurableHead),
    Progress {
        head: DurableHead,
        updates: Vec<DurableStepUpdate>,
        retired: Vec<Digest32>,
    },
    ProgressWithObjectives {
        head: DurableHead,
        updates: Vec<DurableStepUpdate>,
        retired: Vec<Digest32>,
        satisfied: Vec<Digest32>,
        abandoned: Vec<Digest32>,
    },
    Checkpoint(DurableCheckpoint),
    Commit(DurableCommit),
    ObjectiveCommit {
        objective: DurableObjective,
        evicted_history: Vec<Digest32>,
    },
    /// Compaction representation; action commits are retained separately.
    Objective(DurableObjective),
    Step {
        fortress_id: FortressId,
        plan_digest: Digest32,
        step: u32,
        state: String,
    },
    StepAt {
        fortress_id: FortressId,
        plan_digest: Digest32,
        step: u32,
        state: String,
        anchor: StateAnchor,
    },
    Done {
        fortress_id: FortressId,
        plan_digest: Digest32,
    },
}

/// What opening the store found, for doctor reports and recovery packets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DurableStoreReport {
    pub root: PathBuf,
    pub records: usize,
    pub fortresses: usize,
    pub checkpoints: usize,
    pub chain_head: Digest32,
    /// Bytes of an incomplete final record discarded on open.
    pub torn_tail_bytes: u64,
    pub compactions: u64,
}

/// Exclusive handle on a durable laboratory store.
#[derive(Debug)]
pub struct DurableLabStore {
    root: PathBuf,
    journal: File,
    chain: Digest32,
    records: usize,
    index: Index,
    torn_tail_bytes: u64,
    compactions: u64,
    /// Fault injection: appends still allowed before the store behaves as if
    /// the process died (nothing further reaches disk). `None` is unlimited.
    append_budget: Option<usize>,
    /// An uncertain write must be reconciled by reopening and replaying the
    /// journal. Retrying against the old in-memory chain could corrupt it.
    write_fault: Option<String>,
}

fn hex_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len() * 2);
    for byte in text.bytes() {
        out.push_str(&format!("{byte:02x}"));
    }
    if out.is_empty() {
        out.push('-');
    }
    out
}

fn unhex_text(raw: &str, bound: usize) -> Result<String> {
    let text = unhex_payload(raw, bound)?;
    if text.chars().any(char::is_control) {
        return Err(corrupt("journal text field contains control characters"));
    }
    Ok(text)
}

/// Hex-encoded free text (plan requests may contain newlines).
fn unhex_payload(raw: &str, bound: usize) -> Result<String> {
    if raw == "-" {
        return Ok(String::new());
    }
    if !raw.len().is_multiple_of(2) || raw.len() / 2 > bound {
        return Err(corrupt("journal text field has an invalid length"));
    }
    let mut bytes = Vec::with_capacity(raw.len() / 2);
    for index in (0..raw.len()).step_by(2) {
        let pair = raw
            .get(index..index + 2)
            .ok_or_else(|| corrupt("journal text field is not hexadecimal"))?;
        bytes.push(
            u8::from_str_radix(pair, 16)
                .map_err(|_| corrupt("journal text field is not hexadecimal"))?,
        );
    }
    String::from_utf8(bytes).map_err(|_| corrupt("journal text field is not UTF-8"))
}

fn parse_u64(raw: &str) -> Result<u64> {
    if raw.is_empty() || raw.len() > 20 || !raw.bytes().all(|b| b.is_ascii_digit()) {
        return Err(corrupt("journal integer field is malformed"));
    }
    raw.parse::<u64>()
        .map_err(|_| corrupt("journal integer field is malformed"))
}

fn parse_digest(raw: &str) -> Result<Digest32> {
    if raw.bytes().any(|b| b.is_ascii_uppercase()) {
        return Err(corrupt("journal digest is not lowercase hexadecimal"));
    }
    Digest32::from_hex(raw).ok_or_else(|| corrupt("journal digest is malformed"))
}

fn parse_identifier(raw: &str) -> Result<u128> {
    if raw.len() != 32 || !raw.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
        return Err(corrupt("journal identifier is malformed"));
    }
    u128::from_str_radix(raw, 16).map_err(|_| corrupt("journal identifier is malformed"))
}

fn plan_payload(
    fortress: FortressId,
    digest: Digest32,
    sealed: Digest32,
    intent: u128,
    source: &DurablePlanSource,
) -> String {
    let (kind, summary, raw) = source.fields();
    format!(
        "{} {} {} {intent:032x} {kind} {} {}",
        fortress.get(),
        digest.to_hex(),
        sealed.to_hex(),
        hex_text(summary),
        hex_text(raw),
    )
}

fn parse_plan(
    fortress: &str,
    digest: &str,
    sealed: &str,
    intent: &str,
    kind: &str,
    summary: &str,
    payload: &str,
) -> Result<DurableCommit> {
    let intent_id = parse_identifier(intent)?;
    let summary = unhex_payload(summary, MAX_PLAN_SUMMARY_BYTES)?;
    let source = match kind {
        "pause" => match unhex_text(payload, 5)?.as_str() {
            "true" => DurablePlanSource::Pause {
                summary,
                paused: true,
            },
            "false" => DurablePlanSource::Pause {
                summary,
                paused: false,
            },
            _ => return Err(corrupt("journal pause target is malformed")),
        },
        "actions" => DurablePlanSource::Actions {
            summary,
            raw: unhex_payload(payload, MAX_PLAN_REQUEST_BYTES)?,
        },
        "blueprint" => DurablePlanSource::Blueprint {
            summary,
            raw: unhex_payload(payload, MAX_PLAN_REQUEST_BYTES)?,
        },
        "production" => DurablePlanSource::Production {
            summary,
            raw: unhex_payload(payload, MAX_PLAN_REQUEST_BYTES)?,
        },
        "production_continuation" => DurablePlanSource::ProductionContinuation {
            summary,
            raw: unhex_payload(payload, MAX_PLAN_REQUEST_BYTES)?,
        },
        _ => return Err(corrupt("journal plan source kind is not recognized")),
    };
    Ok(DurableCommit {
        fortress_id: FortressId::new(parse_u64(fortress)?),
        plan_digest: parse_digest(digest)?,
        sealed_state_hash: parse_digest(sealed)?,
        intent_id,
        source,
        steps: BTreeMap::new(),
        step_anchors: BTreeMap::new(),
    })
}

fn optional_anchor_payload(anchor: Option<StateAnchor>) -> String {
    match anchor {
        None => "- 0 0 0".to_owned(),
        Some(anchor) => format!(
            "{} {} {} {}",
            anchor.state_hash.to_hex(),
            anchor.tick.0,
            anchor.cursor.epoch,
            anchor.cursor.sequence,
        ),
    }
}

fn parse_optional_anchor(
    fortress_id: FortressId,
    hash: &str,
    tick: &str,
    epoch: &str,
    sequence: &str,
) -> Result<Option<StateAnchor>> {
    if hash == "-" {
        if (tick, epoch, sequence) != ("0", "0", "0") {
            return Err(corrupt(
                "absent objective evidence has nonzero anchor fields",
            ));
        }
        return Ok(None);
    }
    Ok(Some(StateAnchor {
        fortress_id,
        state_hash: parse_digest(hash)?,
        tick: GameTick(parse_u64(tick)?),
        cursor: ObservationCursor {
            epoch: parse_u64(epoch)?,
            sequence: parse_u64(sequence)?,
        },
    }))
}

fn progress_payload(
    head: &DurableHead,
    updates: &[DurableStepUpdate],
    retired: &[Digest32],
    objectives: Option<(&[Digest32], &[Digest32])>,
) -> String {
    let tag = if objectives.is_some() { "V" } else { "F" };
    let mut payload = format!(
        "{tag} {} {} {} {} {} {} {} {}",
        head.fortress_id.get(),
        hex_text(&head.scenario),
        head.anchor.state_hash.to_hex(),
        head.anchor.tick.0,
        head.anchor.cursor.epoch,
        head.anchor.cursor.sequence,
        updates.len(),
        retired.len(),
    );
    if let Some((satisfied, abandoned)) = objectives {
        payload.push_str(&format!(" {} {}", satisfied.len(), abandoned.len()));
    }
    for update in updates {
        payload.push_str(&format!(
            " {} {} {}",
            update.plan_digest.to_hex(),
            update.step,
            update.state,
        ));
    }
    let (satisfied, abandoned) = match objectives {
        Some(lists) => lists,
        None => (&[][..], &[][..]),
    };
    for digest in retired.iter().chain(satisfied).chain(abandoned) {
        payload.push(' ');
        payload.push_str(&digest.to_hex());
    }
    payload
}

fn parse_ordered_digests<'a>(
    next: &mut impl FnMut() -> Result<&'a str>,
    count: u64,
) -> Result<Vec<Digest32>> {
    let mut digests = Vec::new();
    for _ in 0..count {
        let digest = parse_digest(next()?)?;
        if digests.last().is_some_and(|previous| *previous >= digest) {
            return Err(corrupt("journal plan digests are not strictly ordered"));
        }
        digests.push(digest);
    }
    Ok(digests)
}

impl Record {
    fn payload(&self) -> String {
        match self {
            Self::Head(head) => format!(
                "H {} {} {} {} {} {}",
                head.fortress_id.get(),
                hex_text(&head.scenario),
                head.anchor.state_hash.to_hex(),
                head.anchor.tick.0,
                head.anchor.cursor.epoch,
                head.anchor.cursor.sequence,
            ),
            Self::Progress {
                head,
                updates,
                retired,
            } => progress_payload(head, updates, retired, None),
            Self::ProgressWithObjectives {
                head,
                updates,
                retired,
                satisfied,
                abandoned,
            } => progress_payload(head, updates, retired, Some((satisfied, abandoned))),
            Self::Checkpoint(checkpoint) => format!(
                "C {} {:032x} {} {}",
                checkpoint.fortress_id.get(),
                checkpoint.checkpoint_id.get(),
                hex_text(&checkpoint.label),
                checkpoint.state_hash.to_hex(),
            ),
            Self::Commit(commit) => format!(
                "P {}",
                plan_payload(
                    commit.fortress_id,
                    commit.plan_digest,
                    commit.sealed_state_hash,
                    commit.intent_id,
                    &commit.source,
                ),
            ),
            Self::ObjectiveCommit {
                objective,
                evicted_history,
            } => {
                let mut payload = format!(
                    "G {} {:032x} {}",
                    plan_payload(
                        objective.fortress_id,
                        objective.plan_digest,
                        objective.sealed_state_hash,
                        objective.intent_id,
                        &objective.source,
                    ),
                    objective.owner_session_id.get(),
                    evicted_history.len(),
                );
                for digest in evicted_history {
                    payload.push(' ');
                    payload.push_str(&digest.to_hex());
                }
                payload
            }
            Self::Objective(objective) => format!(
                "O {} {:032x} {} {}",
                plan_payload(
                    objective.fortress_id,
                    objective.plan_digest,
                    objective.sealed_state_hash,
                    objective.intent_id,
                    &objective.source,
                ),
                objective.owner_session_id.get(),
                optional_anchor_payload(objective.first_satisfied_anchor),
                optional_anchor_payload(objective.restore_abandoned_anchor),
            ),
            Self::Step {
                fortress_id,
                plan_digest,
                step,
                state,
            } => format!(
                "S {} {} {step} {state}",
                fortress_id.get(),
                plan_digest.to_hex()
            ),
            Self::StepAt {
                fortress_id,
                plan_digest,
                step,
                state,
                anchor,
            } => format!(
                "A {} {} {step} {state} {} {} {} {}",
                fortress_id.get(),
                plan_digest.to_hex(),
                anchor.state_hash.to_hex(),
                anchor.tick.0,
                anchor.cursor.epoch,
                anchor.cursor.sequence,
            ),
            Self::Done {
                fortress_id,
                plan_digest,
            } => format!("D {} {}", fortress_id.get(), plan_digest.to_hex()),
        }
    }

    fn parse(payload: &str) -> Result<Self> {
        if payload.starts_with("F ") || payload.starts_with("V ") {
            return Self::parse_progress(payload);
        }
        if payload.len() > MAX_RECORD_BYTES {
            return Err(corrupt("journal record exceeds its bound"));
        }
        if payload.starts_with("G ") {
            return Self::parse_objective_commit(payload);
        }
        let fields: Vec<&str> = payload.split(' ').collect();
        match fields.as_slice() {
            ["H", fortress, scenario, hash, tick, epoch, sequence] => {
                let fortress_id = FortressId::new(parse_u64(fortress)?);
                Ok(Self::Head(DurableHead {
                    fortress_id,
                    scenario: unhex_text(scenario, MAX_SCENARIO_BYTES)?,
                    anchor: StateAnchor {
                        fortress_id,
                        cursor: ObservationCursor {
                            epoch: parse_u64(epoch)?,
                            sequence: parse_u64(sequence)?,
                        },
                        tick: GameTick(parse_u64(tick)?),
                        state_hash: parse_digest(hash)?,
                    },
                }))
            }
            ["C", fortress, id, label, hash] => {
                if id.len() != 32 || !id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
                    return Err(corrupt("journal checkpoint id is malformed"));
                }
                let raw_id = u128::from_str_radix(id, 16)
                    .map_err(|_| corrupt("journal checkpoint id is malformed"))?;
                if raw_id == 0 {
                    return Err(corrupt("journal checkpoint id zero is reserved"));
                }
                Ok(Self::Checkpoint(DurableCheckpoint {
                    fortress_id: FortressId::new(parse_u64(fortress)?),
                    checkpoint_id: CheckpointId::new(raw_id),
                    label: unhex_text(label, MAX_LABEL_BYTES)?,
                    state_hash: parse_digest(hash)?,
                }))
            }
            [
                "P",
                fortress,
                digest,
                sealed,
                intent,
                kind,
                summary,
                payload,
            ] => Ok(Self::Commit(parse_plan(
                fortress, digest, sealed, intent, kind, summary, payload,
            )?)),
            [
                "O",
                fortress,
                digest,
                sealed,
                intent,
                kind,
                summary,
                payload,
                owner,
                proof_hash,
                proof_tick,
                proof_epoch,
                proof_sequence,
                abandoned_hash,
                abandoned_tick,
                abandoned_epoch,
                abandoned_sequence,
            ] => {
                let commit = parse_plan(fortress, digest, sealed, intent, kind, summary, payload)?;
                let objective = DurableObjective {
                    fortress_id: commit.fortress_id,
                    plan_digest: commit.plan_digest,
                    sealed_state_hash: commit.sealed_state_hash,
                    intent_id: commit.intent_id,
                    source: commit.source,
                    owner_session_id: SessionId::new(parse_identifier(owner)?),
                    first_satisfied_anchor: parse_optional_anchor(
                        commit.fortress_id,
                        proof_hash,
                        proof_tick,
                        proof_epoch,
                        proof_sequence,
                    )?,
                    restore_abandoned_anchor: parse_optional_anchor(
                        commit.fortress_id,
                        abandoned_hash,
                        abandoned_tick,
                        abandoned_epoch,
                        abandoned_sequence,
                    )?,
                };
                objective.validate()?;
                Ok(Self::Objective(objective))
            }
            ["S", fortress, digest, step, state] => {
                if !STEP_STATES.contains(state) {
                    return Err(corrupt("journal step state is not recognized"));
                }
                let step = u32::try_from(parse_u64(step)?)
                    .map_err(|_| corrupt("journal step id is malformed"))?;
                Ok(Self::Step {
                    fortress_id: FortressId::new(parse_u64(fortress)?),
                    plan_digest: parse_digest(digest)?,
                    step,
                    state: (*state).to_owned(),
                })
            }
            [
                "A",
                fortress,
                digest,
                step,
                state,
                hash,
                tick,
                epoch,
                sequence,
            ] => {
                if !STEP_STATES.contains(state) {
                    return Err(corrupt("journal anchored step state is not recognized"));
                }
                let fortress_id = FortressId::new(parse_u64(fortress)?);
                Ok(Self::StepAt {
                    fortress_id,
                    plan_digest: parse_digest(digest)?,
                    step: u32::try_from(parse_u64(step)?)
                        .map_err(|_| corrupt("journal anchored step id is malformed"))?,
                    state: (*state).to_owned(),
                    anchor: StateAnchor {
                        fortress_id,
                        cursor: ObservationCursor {
                            epoch: parse_u64(epoch)?,
                            sequence: parse_u64(sequence)?,
                        },
                        tick: GameTick(parse_u64(tick)?),
                        state_hash: parse_digest(hash)?,
                    },
                })
            }
            ["D", fortress, digest] => Ok(Self::Done {
                fortress_id: FortressId::new(parse_u64(fortress)?),
                plan_digest: parse_digest(digest)?,
            }),
            _ => Err(corrupt("journal record kind or arity is not recognized")),
        }
    }

    fn parse_objective_commit(payload: &str) -> Result<Self> {
        if payload.len() > MAX_RECORD_BYTES {
            return Err(corrupt("journal objective admission exceeds its bound"));
        }
        let mut fields = payload.split(' ');
        let mut next = || {
            fields
                .next()
                .ok_or_else(|| corrupt("journal objective admission is truncated"))
        };
        if next()? != "G" {
            return Err(corrupt("journal objective admission kind is invalid"));
        }
        let commit = parse_plan(
            next()?,
            next()?,
            next()?,
            next()?,
            next()?,
            next()?,
            next()?,
        )?;
        let owner_session_id = SessionId::new(parse_identifier(next()?)?);
        let evicted_count = parse_u64(next()?)?;
        if evicted_count > MAX_OBJECTIVES_PER_FORTRESS as u64 {
            return Err(corrupt(
                "journal objective history eviction exceeds its bound",
            ));
        }
        let evicted_history = parse_ordered_digests(&mut next, evicted_count)?;
        if fields.next().is_some() {
            return Err(corrupt("journal objective admission has trailing fields"));
        }
        let objective = DurableObjective {
            fortress_id: commit.fortress_id,
            plan_digest: commit.plan_digest,
            sealed_state_hash: commit.sealed_state_hash,
            intent_id: commit.intent_id,
            source: commit.source,
            owner_session_id,
            first_satisfied_anchor: None,
            restore_abandoned_anchor: None,
        };
        objective.validate()?;
        Ok(Self::ObjectiveCommit {
            objective,
            evicted_history,
        })
    }

    fn parse_progress(payload: &str) -> Result<Self> {
        if payload.len() > MAX_PROGRESS_RECORD_BYTES {
            return Err(corrupt("journal progress record exceeds its bound"));
        }
        // Iterate instead of allocating a field array from an untrusted record.
        let mut fields = payload.split(' ');
        let mut next = || {
            fields
                .next()
                .ok_or_else(|| corrupt("journal progress record is truncated"))
        };
        let tag = next()?;
        if !matches!(tag, "F" | "V") {
            return Err(corrupt("journal progress record kind is invalid"));
        }
        let fortress_id = FortressId::new(parse_u64(next()?)?);
        let scenario = unhex_text(next()?, MAX_SCENARIO_BYTES)?;
        let state_hash = parse_digest(next()?)?;
        let tick = GameTick(parse_u64(next()?)?);
        let epoch = parse_u64(next()?)?;
        let sequence = parse_u64(next()?)?;
        let update_count = parse_u64(next()?)?;
        let retired_count = parse_u64(next()?)?;
        let (satisfied_count, abandoned_count) = if tag == "V" {
            (parse_u64(next()?)?, parse_u64(next()?)?)
        } else {
            (0, 0)
        };
        if update_count > MAX_PROGRESS_UPDATES as u64
            || retired_count > MAX_COMMITS_PER_FORTRESS as u64
            || satisfied_count > MAX_OBJECTIVES_PER_FORTRESS as u64
            || abandoned_count > MAX_OBJECTIVES_PER_FORTRESS as u64
        {
            return Err(corrupt("journal progress frontier exceeds its bound"));
        }
        let mut updates: Vec<DurableStepUpdate> = Vec::new();
        for _ in 0..update_count {
            let plan_digest = parse_digest(next()?)?;
            let step = u32::try_from(parse_u64(next()?)?)
                .map_err(|_| corrupt("journal progress step id is malformed"))?;
            let state = next()?;
            if !STEP_STATES.contains(&state) {
                return Err(corrupt("journal progress step state is not recognized"));
            }
            if updates.last().is_some_and(|previous| {
                (previous.plan_digest, previous.step) >= (plan_digest, step)
            }) {
                return Err(corrupt("journal progress steps are not strictly ordered"));
            }
            updates.push(DurableStepUpdate {
                plan_digest,
                step,
                state: state.to_owned(),
            });
        }
        let retired = parse_ordered_digests(&mut next, retired_count)?;
        let satisfied = parse_ordered_digests(&mut next, satisfied_count)?;
        let abandoned = parse_ordered_digests(&mut next, abandoned_count)?;
        if satisfied
            .iter()
            .any(|digest| abandoned.binary_search(digest).is_ok())
        {
            return Err(corrupt("journal objective is both satisfied and abandoned"));
        }
        if fields.next().is_some() {
            return Err(corrupt("journal progress record has trailing fields"));
        }
        let head = DurableHead {
            fortress_id,
            scenario,
            anchor: StateAnchor {
                fortress_id,
                cursor: ObservationCursor { epoch, sequence },
                tick,
                state_hash,
            },
        };
        Ok(if tag == "V" {
            Self::ProgressWithObjectives {
                head,
                updates,
                retired,
                satisfied,
                abandoned,
            }
        } else {
            Self::Progress {
                head,
                updates,
                retired,
            }
        })
    }
}

fn chain_next(previous: Digest32, payload: &str) -> Digest32 {
    let mut bytes = Vec::with_capacity(JOURNAL_DOMAIN.len() + 32 + payload.len());
    bytes.extend_from_slice(JOURNAL_DOMAIN);
    bytes.extend_from_slice(previous.as_bytes());
    bytes.extend_from_slice(payload.as_bytes());
    Digest32::of_bytes(&bytes)
}

#[cfg(unix)]
fn private_dir(path: &Path) -> Result<()> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    match fs::symlink_metadata(path) {
        Ok(meta) => {
            if !meta.file_type().is_dir() {
                return Err(invalid(format!(
                    "durable laboratory path {} is not a real directory",
                    path.display()
                )));
            }
            if meta.permissions().mode() & 0o077 != 0 {
                return Err(invalid(format!(
                    "durable laboratory directory {} must not be accessible to group or others",
                    path.display()
                )));
            }
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => fs::DirBuilder::new()
            .mode(0o700)
            .create(path)
            .map_err(|e| io("cannot create durable laboratory directory", &e)),
        Err(error) => Err(io("cannot inspect durable laboratory directory", &error)),
    }
}

#[cfg(not(unix))]
fn private_dir(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_dir() => Ok(()),
        Ok(_) => Err(invalid("durable laboratory path is not a real directory")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir(path).map_err(|e| io("cannot create durable laboratory directory", &e))
        }
        Err(error) => Err(io("cannot inspect durable laboratory directory", &error)),
    }
}

fn private_options() -> OpenOptions {
    let mut options = OpenOptions::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
}

fn sync_dir(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        File::open(path)
            .and_then(|dir| dir.sync_all())
            .map_err(|e| io("cannot sync durable laboratory directory", &e))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

fn regular_file(path: &Path) -> Result<Option<u64>> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_file() => Ok(Some(meta.len())),
        Ok(_) => Err(corrupt(format!(
            "{} is not a regular file (symbolic links and special files are refused)",
            path.display()
        ))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(io("cannot inspect durable laboratory file", &error)),
    }
}

impl DurableLabStore {
    /// Open (creating when absent) the store at an absolute path, take an
    /// exclusive lock, verify the whole journal and every object it names,
    /// and discard only an incomplete final record.
    pub fn open(root: &Path) -> Result<Self> {
        if !root.is_absolute() {
            return Err(invalid(
                "durable laboratory directory must be an absolute path",
            ));
        }
        private_dir(root)?;
        let objects = root.join("objects");
        private_dir(&objects)?;
        let journal_path = root.join("journal");
        if let Some(len) = regular_file(&journal_path)?
            && len > MAX_JOURNAL_BYTES
        {
            return Err(corrupt("durable laboratory journal exceeds its bound"));
        }
        let mut journal = private_options()
            .read(true)
            .append(true)
            .create(true)
            .open(&journal_path)
            .map_err(|e| io("cannot open durable laboratory journal", &e))?;
        journal.try_lock().map_err(|_| {
            DfmcpError::new(
                ErrorCode::Conflict,
                "another process holds the durable laboratory store",
            )
        })?;
        let mut bytes = Vec::new();
        journal
            .seek(SeekFrom::Start(0))
            .and_then(|_| {
                (&journal)
                    .take(MAX_JOURNAL_BYTES + 1)
                    .read_to_end(&mut bytes)
            })
            .map_err(|e| io("cannot read durable laboratory journal", &e))?;
        if bytes.len() as u64 > MAX_JOURNAL_BYTES {
            return Err(corrupt("durable laboratory journal exceeds its bound"));
        }

        let mut store = Self {
            root: root.to_path_buf(),
            journal,
            chain: Digest32::ZERO,
            records: 0,
            index: Index::default(),
            torn_tail_bytes: 0,
            compactions: 0,
            append_budget: None,
            write_fault: None,
        };
        let complete = bytes
            .iter()
            .rposition(|byte| *byte == b'\n')
            .map_or(0, |index| index + 1);
        for line in bytes[..complete].split(|byte| *byte == b'\n') {
            if line.is_empty() {
                continue;
            }
            store.replay_line(line)?;
        }
        let torn = (bytes.len() - complete) as u64;
        if torn > 0 {
            store
                .journal
                .set_len(complete as u64)
                .and_then(|()| store.journal.sync_all())
                .map_err(|e| io("cannot discard a torn journal tail", &e))?;
            store.torn_tail_bytes = torn;
        }
        let mut verified_anchors = BTreeMap::new();
        for hash in store.referenced_objects() {
            verified_anchors.insert(hash, store.load_snapshot(hash)?.anchor());
        }
        for head in store.index.heads.values() {
            if verified_anchors.get(&head.anchor.state_hash) != Some(&head.anchor) {
                return Err(corrupt("durable head anchor does not match its snapshot"));
            }
        }
        for commit in store.index.commits.values().flat_map(BTreeMap::values) {
            if verified_anchors
                .get(&commit.sealed_state_hash)
                .is_none_or(|anchor| anchor.fortress_id != commit.fortress_id)
            {
                return Err(corrupt("durable plan names a different fortress snapshot"));
            }
            for anchor in commit.step_anchors.values() {
                if verified_anchors.get(&anchor.state_hash) != Some(anchor) {
                    return Err(corrupt("durable step anchor does not match its snapshot"));
                }
            }
        }
        for objective in store.index.objectives.values().flat_map(BTreeMap::values) {
            if verified_anchors
                .get(&objective.sealed_state_hash)
                .is_none_or(|anchor| anchor.fortress_id != objective.fortress_id)
            {
                return Err(corrupt(
                    "durable objective names a different fortress snapshot",
                ));
            }
            for anchor in objective
                .first_satisfied_anchor
                .iter()
                .chain(objective.restore_abandoned_anchor.iter())
            {
                if verified_anchors.get(&anchor.state_hash) != Some(anchor) {
                    return Err(corrupt(
                        "durable objective anchor does not match its snapshot",
                    ));
                }
            }
        }
        store.remove_crash_leftovers();
        Ok(store)
    }

    /// Temporary objects and an unpublished compaction are never named by
    /// the journal; a crash can leave them behind, so drop them on open.
    fn remove_crash_leftovers(&self) {
        let _ = fs::remove_file(self.root.join("journal.compact"));
        if let Ok(entries) = fs::read_dir(self.root.join("objects")) {
            for entry in entries.flatten() {
                if entry
                    .file_name()
                    .to_str()
                    .is_some_and(|name| name.starts_with(".tmp-"))
                {
                    let _ = fs::remove_file(entry.path());
                }
            }
        }
    }

    fn replay_line(&mut self, line: &[u8]) -> Result<()> {
        if line.len() > MAX_PROGRESS_RECORD_BYTES + 65 {
            return Err(corrupt("journal record exceeds its bound"));
        }
        let text = std::str::from_utf8(line).map_err(|_| corrupt("journal record is not UTF-8"))?;
        let (chain, payload) = text
            .split_once(' ')
            .ok_or_else(|| corrupt("journal record lacks its chain digest"))?;
        let expected = chain_next(self.chain, payload);
        if parse_digest(chain)? != expected {
            return Err(corrupt(format!(
                "journal chain breaks at record {}",
                self.records + 1
            )));
        }
        let record = Record::parse(payload)?;
        self.index.apply(record)?;
        self.chain = expected;
        self.records += 1;
        Ok(())
    }

    fn referenced_objects(&self) -> BTreeSet<Digest32> {
        self.index
            .heads
            .values()
            .map(|head| head.anchor.state_hash)
            .chain(
                self.index
                    .checkpoints
                    .values()
                    .flat_map(BTreeMap::values)
                    .map(|checkpoint| checkpoint.state_hash),
            )
            .chain(
                self.index
                    .commits
                    .values()
                    .flat_map(BTreeMap::values)
                    .map(|commit| commit.sealed_state_hash),
            )
            .chain(
                self.index
                    .commits
                    .values()
                    .flat_map(BTreeMap::values)
                    .flat_map(|commit| commit.step_anchors.values())
                    .map(|anchor| anchor.state_hash),
            )
            .chain(
                self.index
                    .objectives
                    .values()
                    .flat_map(BTreeMap::values)
                    .flat_map(|objective| {
                        std::iter::once(objective.sealed_state_hash).chain(
                            objective
                                .first_satisfied_anchor
                                .iter()
                                .chain(objective.restore_abandoned_anchor.iter())
                                .map(|anchor| anchor.state_hash),
                        )
                    }),
            )
            .collect()
    }

    fn object_path(&self, hash: Digest32) -> PathBuf {
        self.root
            .join("objects")
            .join(format!("{}.snap", hash.to_hex()))
    }

    /// Read and strictly decode one stored snapshot.
    pub fn load_snapshot(&self, hash: Digest32) -> Result<WorldSnapshot> {
        let path = self.object_path(hash);
        let len = regular_file(&path)?.ok_or_else(|| {
            corrupt(format!(
                "durable snapshot object {hash} named by the journal is missing"
            ))
        })?;
        if len > MAX_OBJECT_BYTES {
            return Err(corrupt("durable snapshot object exceeds its bound"));
        }
        let mut bytes = Vec::new();
        File::open(&path)
            .and_then(|file| file.take(MAX_OBJECT_BYTES + 1).read_to_end(&mut bytes))
            .map_err(|e| io("cannot read durable snapshot object", &e))?;
        if Digest32::of_bytes(&bytes) != hash {
            return Err(corrupt(format!(
                "durable snapshot object {hash} does not match its content address"
            )));
        }
        WorldSnapshot::from_canonical_bytes(&bytes)
    }

    fn write_object(&self, snapshot: &WorldSnapshot) -> Result<()> {
        if !snapshot.hash_is_valid() {
            return Err(DfmcpError::new(
                ErrorCode::InternalInvariantViolation,
                "refusing to persist a snapshot whose state hash is invalid",
            ));
        }
        let path = self.object_path(snapshot.state_hash);
        if regular_file(&path)?.is_some() {
            if self.load_snapshot(snapshot.state_hash)? != *snapshot {
                return Err(corrupt(
                    "stored snapshot differs from the requested publication",
                ));
            }
            return Ok(());
        }
        let bytes = snapshot.canonical_bytes();
        if bytes.len() as u64 > MAX_OBJECT_BYTES {
            return Err(DfmcpError::new(
                ErrorCode::BudgetExceeded,
                "snapshot exceeds the durable object bound",
            ));
        }
        let objects = self.root.join("objects");
        let temporary = objects.join(format!(".tmp-{}", snapshot.state_hash.to_hex()));
        let _ = fs::remove_file(&temporary);
        let mut file = private_options()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|e| io("cannot create durable snapshot object", &e))?;
        file.write_all(&bytes)
            .and_then(|()| file.sync_all())
            .map_err(|e| io("cannot write durable snapshot object", &e))?;
        fs::rename(&temporary, &path)
            .map_err(|e| io("cannot publish durable snapshot object", &e))?;
        sync_dir(&objects)
    }

    /// Fault injection for crash campaigns: allow `budget` more journal
    /// appends, after which every write fails as if the process had died at
    /// that boundary (objects already written stay, as after a real crash).
    pub fn set_append_budget(&mut self, budget: Option<usize>) {
        self.append_budget = budget;
    }

    fn append(&mut self, record: Record, snapshot: Option<&WorldSnapshot>) -> Result<()> {
        self.append_with_limit(record, snapshot, MAX_JOURNAL_BYTES)
    }

    fn append_with_limit(
        &mut self,
        record: Record,
        snapshot: Option<&WorldSnapshot>,
        journal_limit: u64,
    ) -> Result<()> {
        self.ensure_writable()?;
        match self.append_budget.as_mut() {
            Some(0) => {
                return Err(DfmcpError::new(
                    ErrorCode::AdapterUnavailable,
                    "injected crash: the durable store accepts no further writes",
                ));
            }
            Some(remaining) => *remaining -= 1,
            None => {}
        }
        let payload = record.payload();
        let record_bound = if matches!(
            &record,
            Record::Progress { .. } | Record::ProgressWithObjectives { .. }
        ) {
            MAX_PROGRESS_RECORD_BYTES
        } else {
            MAX_RECORD_BYTES
        };
        if payload.len() > record_bound {
            return Err(invalid("durable journal record exceeds its bound"));
        }
        let mut next = self.index.clone();
        next.apply(record)?;
        let line_bytes = payload.len() as u64 + 66;
        let journal_bytes = self
            .journal
            .metadata()
            .map_err(|error| io("cannot inspect durable journal length", &error))?
            .len();
        if journal_bytes.saturating_add(line_bytes) > journal_limit {
            self.compact()?;
            let compacted_bytes = self
                .journal
                .metadata()
                .map_err(|error| io("cannot inspect compacted journal length", &error))?
                .len();
            if compacted_bytes.saturating_add(line_bytes) > journal_limit {
                return Err(DfmcpError::new(
                    ErrorCode::BudgetExceeded,
                    "durable journal cannot fit another atomic record within its reopen bound",
                ));
            }
        }
        // Capacity compaction precedes object materialization. Otherwise its
        // old-root garbage collection could delete this record's new object
        // before the record naming it becomes visible.
        if let Some(snapshot) = snapshot {
            self.write_object(snapshot)?;
        }
        let chain = chain_next(self.chain, &payload);
        let line = format!("{} {payload}\n", chain.to_hex());
        self.write_record_with(line.as_bytes(), |journal, bytes| {
            journal.write_all(bytes).and_then(|()| journal.sync_data())
        })?;
        self.index = next;
        self.chain = chain;
        self.records += 1;
        if self.records >= COMPACT_AFTER_RECORDS
            && let Err(error) = self.compact()
        {
            // This append is already published, even when compaction fails
            // before its own rename. The caller received an error and may not
            // have installed the admitted goal or effects in memory. Fence
            // even apparent no-ops until replay reconciles the published root.
            self.write_fault = Some(error.message.clone());
            return Err(error.retryable(false));
        }
        Ok(())
    }

    fn ensure_writable(&self) -> Result<()> {
        match &self.write_fault {
            None => Ok(()),
            Some(fault) => Err(DfmcpError::new(
                ErrorCode::AdapterUnavailable,
                format!(
                    "durable journal requires reopen and reconciliation after an uncertain write: {fault}"
                ),
            )
            .retryable(false)),
        }
    }

    fn write_record_with(
        &mut self,
        bytes: &[u8],
        write: impl FnOnce(&mut File, &[u8]) -> std::io::Result<()>,
    ) -> Result<()> {
        self.ensure_writable()?;
        if let Err(error) = write(&mut self.journal, bytes) {
            self.write_fault = Some(error.to_string());
            return Err(
                io("cannot append to the durable laboratory journal", &error).retryable(false),
            );
        }
        Ok(())
    }

    /// Persist `snapshot` as the latest state of its fortress.
    pub fn persist_head(&mut self, scenario: &str, snapshot: &WorldSnapshot) -> Result<()> {
        self.ensure_writable()?;
        if scenario.len() > MAX_SCENARIO_BYTES || scenario.chars().any(char::is_control) {
            return Err(invalid("scenario name is not storable"));
        }
        if self
            .index
            .heads
            .get(&snapshot.fortress_id)
            .is_some_and(|head| head.anchor == snapshot.anchor() && head.scenario == scenario)
        {
            return Ok(());
        }
        self.append(
            Record::Head(DurableHead {
                fortress_id: snapshot.fortress_id,
                scenario: scenario.to_owned(),
                anchor: snapshot.anchor(),
            }),
            Some(snapshot),
        )
    }

    /// Publish the world, its step frontier, and completed-plan retirement in
    /// one hash-chained record. A crash exposes either the previous generation
    /// or all of this one; verified steps cannot get ahead of their world.
    ///
    /// The snapshot object is validated and synced before publication. All
    /// updates are checked before any journal write, sorted canonically, and
    /// repetitions of the existing frontier are a no-op. Legacy H/S records
    /// remain readable, but do not establish this atomic-publication guarantee.
    pub fn persist_progress(
        &mut self,
        scenario: &str,
        snapshot: &WorldSnapshot,
        updates: &[DurableStepUpdate],
        retired: &[Digest32],
    ) -> Result<()> {
        self.persist_progress_with_objectives(scenario, snapshot, updates, retired, &[], &[])
    }

    /// Atomically publish action progress and original-goal history with one
    /// exact world. The caller must establish Observe authority and evaluate
    /// the original predicate before naming a satisfied objective.
    ///
    /// The first satisfaction and restore-abandonment anchors never move.
    /// Unknown, duplicate, or overlapping objective lists are refused before
    /// publication. An abandoned objective cannot acquire its first proof.
    pub fn persist_progress_with_objectives(
        &mut self,
        scenario: &str,
        snapshot: &WorldSnapshot,
        updates: &[DurableStepUpdate],
        retired: &[Digest32],
        satisfied: &[Digest32],
        abandoned: &[Digest32],
    ) -> Result<()> {
        self.ensure_writable()?;
        if !snapshot.hash_is_valid() {
            return Err(DfmcpError::new(
                ErrorCode::InternalInvariantViolation,
                "refusing to publish progress against an invalid world snapshot",
            ));
        }
        if scenario.len() > MAX_SCENARIO_BYTES || scenario.chars().any(char::is_control) {
            return Err(invalid("scenario name is not storable"));
        }
        if updates.len() > MAX_PROGRESS_UPDATES
            || retired.len() > MAX_COMMITS_PER_FORTRESS
            || satisfied.len() > MAX_OBJECTIVES_PER_FORTRESS
            || abandoned.len() > MAX_OBJECTIVES_PER_FORTRESS
        {
            return Err(DfmcpError::new(
                ErrorCode::BudgetExceeded,
                "durable progress frontier exceeds its explicit bound",
            ));
        }
        let fortress = snapshot.fortress_id;
        let mut ordered = BTreeMap::new();
        for update in updates {
            if !STEP_STATES.contains(&update.state.as_str()) {
                return Err(invalid("unknown durable progress step state"));
            }
            if ordered
                .insert((update.plan_digest, update.step), update.clone())
                .is_some()
            {
                return Err(invalid("duplicate step in durable progress frontier"));
            }
        }
        let retired_set: BTreeSet<_> = retired.iter().copied().collect();
        if retired_set.len() != retired.len() {
            return Err(invalid(
                "duplicate retired plan in durable progress frontier",
            ));
        }
        let satisfied_set: BTreeSet<_> = satisfied.iter().copied().collect();
        let abandoned_set: BTreeSet<_> = abandoned.iter().copied().collect();
        if satisfied_set.len() != satisfied.len() || abandoned_set.len() != abandoned.len() {
            return Err(invalid("duplicate objective in durable progress frontier"));
        }
        if !satisfied_set.is_disjoint(&abandoned_set) {
            return Err(invalid("objective cannot be both satisfied and abandoned"));
        }
        let mut satisfied = Vec::new();
        for digest in satisfied_set {
            let objective = self
                .objective(fortress, digest)
                .ok_or_else(|| invalid("satisfaction names an unknown durable objective"))?;
            if objective.first_satisfied_anchor.is_none() {
                if objective.restore_abandoned_anchor.is_some() {
                    return Err(invalid(
                        "abandoned objective cannot acquire satisfaction proof",
                    ));
                }
                satisfied.push(digest);
            }
        }
        let mut abandoned = Vec::new();
        for digest in abandoned_set {
            let objective = self
                .objective(fortress, digest)
                .ok_or_else(|| invalid("abandonment names an unknown durable objective"))?;
            if objective.restore_abandoned_anchor.is_none() {
                abandoned.push(digest);
            }
        }
        let updates: Vec<_> = ordered
            .into_values()
            .filter(|update| {
                self.commit(fortress, update.plan_digest)
                    .and_then(|commit| commit.steps.get(&update.step))
                    != Some(&update.state)
            })
            .collect();
        let retired: Vec<_> = retired_set
            .into_iter()
            .filter(|digest| self.commit(fortress, *digest).is_some())
            .collect();
        let head = DurableHead {
            fortress_id: fortress,
            scenario: scenario.to_owned(),
            anchor: snapshot.anchor(),
        };
        if self.index.heads.get(&fortress) == Some(&head)
            && updates.is_empty()
            && retired.is_empty()
            && satisfied.is_empty()
            && abandoned.is_empty()
        {
            return Ok(());
        }
        let record = if satisfied.is_empty() && abandoned.is_empty() {
            Record::Progress {
                head,
                updates,
                retired,
            }
        } else {
            Record::ProgressWithObjectives {
                head,
                updates,
                retired,
                satisfied,
                abandoned,
            }
        };
        let mut candidate = self.index.clone();
        candidate.apply(record.clone())?;
        self.append(record, Some(snapshot))
    }

    /// Persist a checkpoint of `snapshot`.
    pub fn persist_checkpoint(
        &mut self,
        checkpoint_id: CheckpointId,
        label: &str,
        snapshot: &WorldSnapshot,
    ) -> Result<()> {
        if label.len() > MAX_LABEL_BYTES || label.chars().any(char::is_control) {
            return Err(invalid("checkpoint label is not storable"));
        }
        if checkpoint_id == CheckpointId::NIL {
            return Err(invalid("checkpoint id zero is reserved"));
        }
        self.append(
            Record::Checkpoint(DurableCheckpoint {
                fortress_id: snapshot.fortress_id,
                checkpoint_id,
                label: label.to_owned(),
                state_hash: snapshot.state_hash,
            }),
            Some(snapshot),
        )
    }

    /// Record a committed plan, persisting the world it was sealed against.
    pub fn persist_commit(
        &mut self,
        sealed: &WorldSnapshot,
        plan_digest: Digest32,
        intent_id: u128,
        source: DurablePlanSource,
    ) -> Result<()> {
        source.validate_bound()?;
        self.append(
            Record::Commit(DurableCommit {
                fortress_id: sealed.fortress_id,
                plan_digest,
                sealed_state_hash: sealed.state_hash,
                intent_id,
                source,
                steps: BTreeMap::new(),
                step_anchors: BTreeMap::new(),
            }),
            Some(sealed),
        )
    }

    /// Admit original intent and its action commit together, before effects.
    /// The sealed snapshot is durably published before the single admission
    /// record. Legacy action commits are never upgraded into inferred goals.
    ///
    /// History may be evicted only when it already has a satisfaction anchor
    /// and no unfinished durable commit. The caller must additionally inspect
    /// exact effect identities and prove current physical quiescence.
    ///
    /// An identical request is a no-op even after action retirement. It cannot
    /// resurrect work, move proof anchors, or apply additional history evictions.
    pub fn persist_objective_commit(
        &mut self,
        sealed: &WorldSnapshot,
        plan_digest: Digest32,
        intent_id: u128,
        source: DurablePlanSource,
        owner_session_id: SessionId,
        evicted_history: &[Digest32],
    ) -> Result<()> {
        self.ensure_writable()?;
        source.validate_bound()?;
        if !sealed.hash_is_valid() {
            return Err(DfmcpError::new(
                ErrorCode::InternalInvariantViolation,
                "refusing objective admission against an invalid sealed snapshot",
            ));
        }
        if owner_session_id == SessionId::NIL || intent_id == 0 {
            return Err(invalid("objective owner and intent must be nonzero"));
        }
        if evicted_history.len() > MAX_OBJECTIVES_PER_FORTRESS {
            return Err(DfmcpError::new(
                ErrorCode::BudgetExceeded,
                "objective history eviction exceeds its explicit bound",
            ));
        }
        let evicted: BTreeSet<_> = evicted_history.iter().copied().collect();
        if evicted.len() != evicted_history.len() || evicted.contains(&plan_digest) {
            return Err(invalid(
                "objective admission has duplicate or self history eviction",
            ));
        }
        if let Some(previous) = self.objective(sealed.fortress_id, plan_digest) {
            if previous.sealed_state_hash != sealed.state_hash
                || previous.intent_id != intent_id
                || previous.source != source
                || previous.owner_session_id != owner_session_id
            {
                return Err(DfmcpError::new(
                    ErrorCode::Conflict,
                    "objective admission conflicts with the retained original request",
                ));
            }
            return Ok(());
        }
        let objective = DurableObjective {
            fortress_id: sealed.fortress_id,
            plan_digest,
            sealed_state_hash: sealed.state_hash,
            intent_id,
            source,
            owner_session_id,
            first_satisfied_anchor: None,
            restore_abandoned_anchor: None,
        };
        let evicted_history: Vec<_> = evicted.into_iter().collect();
        self.index
            .validate_objective_admission(&objective, &evicted_history)?;
        self.append(
            Record::ObjectiveCommit {
                objective,
                evicted_history,
            },
            Some(sealed),
        )
    }

    /// Record a step's state; repeating the current state is a no-op.
    pub fn persist_step(
        &mut self,
        fortress_id: FortressId,
        plan_digest: Digest32,
        step: u32,
        state: &str,
    ) -> Result<()> {
        self.ensure_writable()?;
        if !STEP_STATES.contains(&state) {
            return Err(invalid("unknown durable step state"));
        }
        if self
            .commit(fortress_id, plan_digest)
            .is_some_and(|commit| commit.steps.get(&step).is_some_and(|s| s == state))
        {
            return Ok(());
        }
        self.append(
            Record::Step {
                fortress_id,
                plan_digest,
                step,
                state: state.to_owned(),
            },
            None,
        )
    }

    /// Retire a commit whose steps are all final.
    pub fn retire_commit(&mut self, fortress_id: FortressId, plan_digest: Digest32) -> Result<()> {
        self.ensure_writable()?;
        if self.commit(fortress_id, plan_digest).is_none() {
            return Ok(());
        }
        self.append(
            Record::Done {
                fortress_id,
                plan_digest,
            },
            None,
        )
    }

    /// One unfinished durable commit.
    #[must_use]
    pub fn commit(&self, fortress_id: FortressId, plan_digest: Digest32) -> Option<&DurableCommit> {
        self.index
            .commits
            .get(&fortress_id)
            .and_then(|book| book.get(&plan_digest))
    }

    /// Every unfinished durable commit of a fortress, in digest order.
    pub fn commits(&self, fortress_id: FortressId) -> impl Iterator<Item = &DurableCommit> {
        self.index
            .commits
            .get(&fortress_id)
            .into_iter()
            .flat_map(BTreeMap::values)
    }

    /// One retained original objective, including achieved or abandoned history.
    #[must_use]
    pub fn objective(
        &self,
        fortress_id: FortressId,
        plan_digest: Digest32,
    ) -> Option<&DurableObjective> {
        self.index
            .objectives
            .get(&fortress_id)
            .and_then(|book| book.get(&plan_digest))
    }

    /// Every retained original objective of a fortress, in digest order.
    pub fn objectives(&self, fortress_id: FortressId) -> impl Iterator<Item = &DurableObjective> {
        self.index
            .objectives
            .get(&fortress_id)
            .into_iter()
            .flat_map(BTreeMap::values)
    }

    /// The latest durable state of a fortress, if it has one.
    #[must_use]
    pub fn head(&self, fortress_id: FortressId) -> Option<&DurableHead> {
        self.index.heads.get(&fortress_id)
    }

    /// Every durable checkpoint of a fortress, in identifier order.
    pub fn checkpoints(&self, fortress_id: FortressId) -> impl Iterator<Item = &DurableCheckpoint> {
        self.index
            .checkpoints
            .get(&fortress_id)
            .into_iter()
            .flat_map(BTreeMap::values)
    }

    #[must_use]
    pub fn report(&self) -> DurableStoreReport {
        DurableStoreReport {
            root: self.root.clone(),
            records: self.records,
            fortresses: self.index.heads.len(),
            checkpoints: self.index.checkpoints.values().map(BTreeMap::len).sum(),
            chain_head: self.chain,
            torn_tail_bytes: self.torn_tail_bytes,
            compactions: self.compactions,
        }
    }

    /// Rewrite the journal as live heads, checkpoints, unfinished commits, and
    /// retained original objectives, then drop objects nothing references.
    /// The replacement is synced and renamed over the old journal, so a crash
    /// leaves one complete journal or the other.
    pub fn compact(&mut self) -> Result<()> {
        self.compact_with_directory_sync(sync_dir)
    }

    fn compact_with_directory_sync(
        &mut self,
        sync_parent: impl FnOnce(&Path) -> Result<()>,
    ) -> Result<()> {
        self.ensure_writable()?;
        let mut chain = Digest32::ZERO;
        let mut text = String::new();
        let mut records = 0usize;
        let live: Vec<Record> = self
            .index
            .heads
            .values()
            .cloned()
            .map(Record::Head)
            .chain(
                self.index
                    .checkpoints
                    .values()
                    .flat_map(BTreeMap::values)
                    .cloned()
                    .map(Record::Checkpoint),
            )
            .chain(
                self.index
                    .commits
                    .values()
                    .flat_map(BTreeMap::values)
                    .flat_map(|commit| {
                        let mut opening = commit.clone();
                        opening.steps.clear();
                        opening.step_anchors.clear();
                        std::iter::once(Record::Commit(opening)).chain(commit.steps.iter().map(
                            |(step, state)| match commit.step_anchors.get(step) {
                                Some(anchor) => Record::StepAt {
                                    fortress_id: commit.fortress_id,
                                    plan_digest: commit.plan_digest,
                                    step: *step,
                                    state: state.clone(),
                                    anchor: *anchor,
                                },
                                None => Record::Step {
                                    fortress_id: commit.fortress_id,
                                    plan_digest: commit.plan_digest,
                                    step: *step,
                                    state: state.clone(),
                                },
                            },
                        ))
                    }),
            )
            .chain(
                self.index
                    .objectives
                    .values()
                    .flat_map(BTreeMap::values)
                    .cloned()
                    .map(Record::Objective),
            )
            .collect();
        for record in live {
            let payload = record.payload();
            if text.len() as u64 + payload.len() as u64 + 66 > MAX_JOURNAL_BYTES {
                return Err(DfmcpError::new(
                    ErrorCode::BudgetExceeded,
                    "compacted durable journal exceeds its explicit bound",
                ));
            }
            chain = chain_next(chain, &payload);
            text.push_str(&chain.to_hex());
            text.push(' ');
            text.push_str(&payload);
            text.push('\n');
            records += 1;
        }
        let temporary = self.root.join("journal.compact");
        let _ = fs::remove_file(&temporary);
        let mut file = private_options()
            .read(true)
            .append(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|e| io("cannot create compacted journal", &e))?;
        file.write_all(text.as_bytes())
            .and_then(|()| file.sync_all())
            .map_err(|e| io("cannot write compacted journal", &e))?;
        file.try_lock().map_err(|_| {
            DfmcpError::new(ErrorCode::Conflict, "cannot lock the compacted journal")
        })?;
        fs::rename(&temporary, self.root.join("journal"))
            .map_err(|e| io("cannot publish compacted journal", &e))?;
        // Rename has already changed the public inode. Install its matching
        // handle and chain even if the directory barrier subsequently fails;
        // never keep appending through the old now-unlinked journal handle.
        self.journal = file;
        self.chain = chain;
        self.records = records;
        self.compactions += 1;
        if let Err(error) = sync_parent(&self.root) {
            self.write_fault = Some(error.message.clone());
            return Err(error.retryable(false));
        }

        let live = self.referenced_objects();
        if let Ok(entries) = fs::read_dir(self.root.join("objects")) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let Some(name) = name.to_str() else { continue };
                let keep = name
                    .strip_suffix(".snap")
                    .and_then(Digest32::from_hex)
                    .is_some_and(|hash| live.contains(&hash));
                if !keep {
                    let _ = fs::remove_file(entry.path());
                }
            }
        }
        sync_dir(&self.root.join("objects"))
    }
}

#[derive(Clone, Debug, Default)]
struct Index {
    heads: BTreeMap<FortressId, DurableHead>,
    checkpoints: BTreeMap<FortressId, BTreeMap<CheckpointId, DurableCheckpoint>>,
    commits: BTreeMap<FortressId, BTreeMap<Digest32, DurableCommit>>,
    objectives: BTreeMap<FortressId, BTreeMap<Digest32, DurableObjective>>,
}

impl Index {
    fn validate_objective_admission(
        &self,
        objective: &DurableObjective,
        evicted_history: &[Digest32],
    ) -> Result<()> {
        objective.validate()?;
        if objective.first_satisfied_anchor.is_some()
            || objective.restore_abandoned_anchor.is_some()
        {
            return Err(corrupt(
                "objective admission cannot invent historical evidence",
            ));
        }
        let book = self.objectives.get(&objective.fortress_id);
        if book.is_some_and(|book| book.contains_key(&objective.plan_digest)) {
            return Err(corrupt("durable objective recorded twice"));
        }
        if evicted_history.len() > MAX_OBJECTIVES_PER_FORTRESS
            || evicted_history.windows(2).any(|pair| pair[0] >= pair[1])
            || evicted_history.contains(&objective.plan_digest)
        {
            return Err(corrupt(
                "objective history eviction is not a bounded ordered set",
            ));
        }
        let commits = self.commits.get(&objective.fortress_id);
        for digest in evicted_history {
            let previous = book
                .and_then(|book| book.get(digest))
                .ok_or_else(|| corrupt("history eviction names an unknown durable objective"))?;
            if previous.first_satisfied_anchor.is_none()
                || commits.is_some_and(|book| book.contains_key(digest))
            {
                return Err(corrupt(
                    "history eviction would discard an unfinished objective",
                ));
            }
        }
        let remaining = book
            .map_or(0, BTreeMap::len)
            .saturating_sub(evicted_history.len());
        if remaining >= MAX_OBJECTIVES_PER_FORTRESS
            || (book.is_none() && self.objectives.len() >= MAX_FORTRESSES)
        {
            return Err(DfmcpError::new(
                ErrorCode::BudgetExceeded,
                "durable objective capacity is full; unresolved goals must be retained",
            ));
        }
        if commits.is_some_and(|book| book.contains_key(&objective.plan_digest)) {
            return Err(corrupt(
                "objective admission conflicts with an existing action commit",
            ));
        }
        if commits.is_some_and(|book| book.len() >= MAX_COMMITS_PER_FORTRESS) {
            return Err(DfmcpError::new(
                ErrorCode::BudgetExceeded,
                "durable laboratory fortress reached its unfinished-commit bound",
            ));
        }
        Ok(())
    }

    fn apply(&mut self, record: Record) -> Result<()> {
        match record {
            Record::Head(head) => {
                if head.anchor.fortress_id != head.fortress_id {
                    return Err(corrupt("durable head evidence belongs to another fortress"));
                }
                if !self.heads.contains_key(&head.fortress_id) && self.heads.len() >= MAX_FORTRESSES
                {
                    return Err(DfmcpError::new(
                        ErrorCode::BudgetExceeded,
                        "durable laboratory store reached its fortress bound",
                    ));
                }
                self.heads.insert(head.fortress_id, head);
            }
            Record::Progress {
                head,
                updates,
                retired,
            } => {
                let fortress_id = head.fortress_id;
                let anchor = head.anchor;
                self.apply(Record::Head(head))?;
                for update in updates {
                    self.apply(Record::StepAt {
                        fortress_id,
                        plan_digest: update.plan_digest,
                        step: update.step,
                        state: update.state,
                        anchor,
                    })?;
                }
                for plan_digest in retired {
                    if self
                        .commits
                        .get(&fortress_id)
                        .and_then(|book| book.get(&plan_digest))
                        .is_some_and(|commit| {
                            commit.steps.values().any(|state| state == "dispatched")
                        })
                    {
                        return Err(corrupt("progress retired a plan with dispatched work"));
                    }
                    self.apply(Record::Done {
                        fortress_id,
                        plan_digest,
                    })?;
                }
            }
            Record::ProgressWithObjectives {
                head,
                updates,
                retired,
                satisfied,
                abandoned,
            } => {
                if satisfied.len() > MAX_OBJECTIVES_PER_FORTRESS
                    || abandoned.len() > MAX_OBJECTIVES_PER_FORTRESS
                    || satisfied.windows(2).any(|pair| pair[0] >= pair[1])
                    || abandoned.windows(2).any(|pair| pair[0] >= pair[1])
                    || satisfied
                        .iter()
                        .any(|digest| abandoned.binary_search(digest).is_ok())
                {
                    return Err(corrupt(
                        "objective progress is not a pair of disjoint bounded sets",
                    ));
                }
                let fortress_id = head.fortress_id;
                let anchor = head.anchor;
                for digest in satisfied.iter().chain(&abandoned) {
                    let objective = self
                        .objectives
                        .get(&fortress_id)
                        .and_then(|book| book.get(digest))
                        .ok_or_else(|| corrupt("progress names an unknown durable objective"))?;
                    if satisfied.binary_search(digest).is_ok()
                        && objective.first_satisfied_anchor.is_none()
                        && objective.restore_abandoned_anchor.is_some()
                    {
                        return Err(corrupt(
                            "abandoned objective acquired new satisfaction proof",
                        ));
                    }
                }
                self.apply(Record::Progress {
                    head,
                    updates,
                    retired,
                })?;
                for digest in satisfied {
                    let objective = self
                        .objectives
                        .get_mut(&fortress_id)
                        .and_then(|book| book.get_mut(&digest))
                        .ok_or_else(|| {
                            corrupt("satisfaction names an unknown durable objective")
                        })?;
                    objective.first_satisfied_anchor.get_or_insert(anchor);
                }
                for digest in abandoned {
                    let objective = self
                        .objectives
                        .get_mut(&fortress_id)
                        .and_then(|book| book.get_mut(&digest))
                        .ok_or_else(|| corrupt("abandonment names an unknown durable objective"))?;
                    objective.restore_abandoned_anchor.get_or_insert(anchor);
                }
            }
            Record::Checkpoint(checkpoint) => {
                let book = self.checkpoints.entry(checkpoint.fortress_id).or_default();
                if !book.contains_key(&checkpoint.checkpoint_id)
                    && book.len() >= MAX_CHECKPOINTS_PER_FORTRESS
                {
                    return Err(DfmcpError::new(
                        ErrorCode::BudgetExceeded,
                        "durable laboratory fortress reached its checkpoint bound",
                    ));
                }
                book.insert(checkpoint.checkpoint_id, checkpoint);
            }
            Record::Commit(commit) => {
                if self
                    .objectives
                    .get(&commit.fortress_id)
                    .is_some_and(|book| book.contains_key(&commit.plan_digest))
                {
                    return Err(corrupt(
                        "retained objective cannot resurrect an action commit",
                    ));
                }
                let book = self.commits.entry(commit.fortress_id).or_default();
                if book.contains_key(&commit.plan_digest) {
                    return Err(corrupt("durable commit recorded twice"));
                }
                if book.len() >= MAX_COMMITS_PER_FORTRESS {
                    return Err(DfmcpError::new(
                        ErrorCode::BudgetExceeded,
                        "durable laboratory fortress reached its unfinished-commit bound",
                    ));
                }
                book.insert(commit.plan_digest, commit);
            }
            Record::ObjectiveCommit {
                objective,
                evicted_history,
            } => {
                self.validate_objective_admission(&objective, &evicted_history)?;
                self.apply(Record::Commit(objective.opening_commit()))?;
                if let Some(book) = self.objectives.get_mut(&objective.fortress_id) {
                    for digest in evicted_history {
                        book.remove(&digest);
                    }
                }
                self.apply(Record::Objective(objective))?;
            }
            Record::Objective(objective) => {
                objective.validate()?;
                if let Some(commit) = self
                    .commits
                    .get(&objective.fortress_id)
                    .and_then(|book| book.get(&objective.plan_digest))
                    && (commit.sealed_state_hash != objective.sealed_state_hash
                        || commit.intent_id != objective.intent_id
                        || commit.source != objective.source)
                {
                    return Err(corrupt(
                        "durable objective conflicts with its action commit",
                    ));
                }
                if !self.objectives.contains_key(&objective.fortress_id)
                    && self.objectives.len() >= MAX_FORTRESSES
                {
                    return Err(DfmcpError::new(
                        ErrorCode::BudgetExceeded,
                        "durable objective book reached its fortress bound",
                    ));
                }
                let book = self.objectives.entry(objective.fortress_id).or_default();
                if book.contains_key(&objective.plan_digest) {
                    return Err(corrupt("durable objective recorded twice"));
                }
                if book.len() >= MAX_OBJECTIVES_PER_FORTRESS {
                    return Err(DfmcpError::new(
                        ErrorCode::BudgetExceeded,
                        "durable objective book reached its retained-goal bound",
                    ));
                }
                book.insert(objective.plan_digest, objective);
            }
            Record::Step {
                fortress_id,
                plan_digest,
                step,
                state,
            } => {
                let commit = self
                    .commits
                    .get_mut(&fortress_id)
                    .and_then(|book| book.get_mut(&plan_digest))
                    .ok_or_else(|| corrupt("step record names an unknown durable commit"))?;
                if !commit.steps.contains_key(&step) && commit.steps.len() >= MAX_STEPS_PER_COMMIT {
                    return Err(corrupt("durable commit exceeds its step bound"));
                }
                commit.steps.insert(step, state);
                commit.step_anchors.remove(&step);
            }
            Record::StepAt {
                fortress_id,
                plan_digest,
                step,
                state,
                anchor,
            } => {
                if anchor.fortress_id != fortress_id {
                    return Err(corrupt("step evidence belongs to another fortress"));
                }
                self.apply(Record::Step {
                    fortress_id,
                    plan_digest,
                    step,
                    state,
                })?;
                let commit = self
                    .commits
                    .get_mut(&fortress_id)
                    .and_then(|book| book.get_mut(&plan_digest))
                    .ok_or_else(|| corrupt("anchored step names an unknown durable commit"))?;
                commit.step_anchors.insert(step, anchor);
            }
            Record::Done {
                fortress_id,
                plan_digest,
            } => {
                let book = self
                    .commits
                    .get_mut(&fortress_id)
                    .ok_or_else(|| corrupt("done record names an unknown durable commit"))?;
                if book.remove(&plan_digest).is_none() {
                    return Err(corrupt("done record names an unknown durable commit"));
                }
                if book.is_empty() {
                    self.commits.remove(&fortress_id);
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "durable_objective_tests.rs"]
mod objective_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use dfmcp_world::WorldGraph;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "dfmcp-durable-{name}-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = fs::remove_dir_all(&path);
            Self(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn snapshot(fortress: u64, tick: u64) -> WorldSnapshot {
        WorldSnapshot::new(
            FortressId::new(fortress),
            GameTick(tick),
            ObservationCursor {
                epoch: 1,
                sequence: tick,
            },
            false,
            WorldGraph::default(),
        )
    }

    #[test]
    fn heads_and_checkpoints_survive_reopen() -> Result<()> {
        let dir = TempDir::new("reopen");
        let head = snapshot(7, 10);
        let checkpoint = snapshot(7, 5);
        {
            let mut store = DurableLabStore::open(&dir.0)?;
            store.persist_checkpoint(CheckpointId::new(42), "before dig", &checkpoint)?;
            store.persist_head("starter_fortress", &head)?;
        }
        let store = DurableLabStore::open(&dir.0)?;
        let recovered = store.head(FortressId::new(7)).cloned();
        assert_eq!(
            recovered.as_ref().map(|h| (h.scenario.as_str(), h.anchor)),
            Some(("starter_fortress", head.anchor()))
        );
        assert_eq!(store.load_snapshot(head.state_hash)?, head);
        let checkpoints: Vec<_> = store.checkpoints(FortressId::new(7)).cloned().collect();
        assert_eq!(checkpoints.len(), 1);
        assert_eq!(checkpoints[0].label, "before dig");
        assert_eq!(store.load_snapshot(checkpoints[0].state_hash)?, checkpoint);
        assert_eq!(store.report().records, 2);
        Ok(())
    }

    #[test]
    fn capacity_compaction_cannot_collect_the_next_publications_snapshot() -> Result<()> {
        let dir = TempDir::new("capacity-publication");
        let incoming = snapshot(7, 99);
        {
            let mut store = DurableLabStore::open(&dir.0)?;
            for tick in 1..=10 {
                store.persist_head("empty", &snapshot(7, tick))?;
            }
            let record = Record::Head(DurableHead {
                fortress_id: incoming.fortress_id,
                scenario: "empty".to_owned(),
                anchor: incoming.anchor(),
            });
            // Exercise the real append path with a small byte budget, avoiding
            // a 64-MiB fixture. A compacted head plus the new record fits.
            let capacity = 2 * (record.payload().len() as u64 + 66);
            store.append_with_limit(record, Some(&incoming), capacity)?;
            assert_eq!(store.report().compactions, 1);
            assert_eq!(store.load_snapshot(incoming.state_hash)?, incoming);
        }
        let store = DurableLabStore::open(&dir.0)?;
        assert_eq!(
            store.head(incoming.fortress_id).map(|head| head.anchor),
            Some(incoming.anchor())
        );
        assert_eq!(store.load_snapshot(incoming.state_hash)?, incoming);
        Ok(())
    }

    #[test]
    fn unfit_publication_refuses_before_materializing_its_snapshot() -> Result<()> {
        let dir = TempDir::new("capacity-refusal");
        let before = snapshot(7, 10);
        let incoming = snapshot(7, 99);
        let mut store = DurableLabStore::open(&dir.0)?;
        store.persist_head("empty", &before)?;
        let result = store.append_with_limit(
            Record::Head(DurableHead {
                fortress_id: incoming.fortress_id,
                scenario: "empty".to_owned(),
                anchor: incoming.anchor(),
            }),
            Some(&incoming),
            1,
        );
        assert!(matches!(result, Err(error) if error.code == ErrorCode::BudgetExceeded));
        assert_eq!(
            store.head(before.fortress_id).map(|head| head.anchor),
            Some(before.anchor())
        );
        assert!(!store.object_path(incoming.state_hash).exists());
        drop(store);
        let store = DurableLabStore::open(&dir.0)?;
        assert_eq!(store.load_snapshot(before.state_hash)?, before);
        Ok(())
    }

    #[test]
    fn uncertain_partial_or_complete_append_fences_retries_until_reopen() -> Result<()> {
        for complete in [false, true] {
            let dir = TempDir::new(if complete {
                "lost-ack"
            } else {
                "partial-write"
            });
            let before = snapshot(7, 10);
            let incoming = snapshot(7, 11);
            let mut store = DurableLabStore::open(&dir.0)?;
            store.persist_head("empty", &before)?;
            store.write_object(&incoming)?;
            let payload = Record::Head(DurableHead {
                fortress_id: incoming.fortress_id,
                scenario: "empty".to_owned(),
                anchor: incoming.anchor(),
            })
            .payload();
            let line = format!("{} {payload}\n", chain_next(store.chain, &payload).to_hex());
            let result = store.write_record_with(line.as_bytes(), |journal, bytes| {
                let end = if complete {
                    bytes.len()
                } else {
                    bytes.len() / 2
                };
                journal.write_all(&bytes[..end])?;
                Err(std::io::Error::other("injected uncertain journal write"))
            });
            assert!(result.is_err());
            // Even an apparent no-op cannot clear an unresolved durable fault.
            assert!(store.persist_head("empty", &before).is_err());
            assert!(store.persist_progress("empty", &before, &[], &[]).is_err());
            assert!(store.persist_head("empty", &incoming).is_err());
            assert!(store.compact().is_err());
            drop(store);
            let mut store = DurableLabStore::open(&dir.0)?;
            let expected = if complete { &incoming } else { &before };
            assert_eq!(
                store.head(expected.fortress_id).map(|head| head.anchor),
                Some(expected.anchor())
            );
            store.persist_head("empty", &incoming)?;
            drop(store);
            let store = DurableLabStore::open(&dir.0)?;
            assert_eq!(store.load_snapshot(incoming.state_hash)?, incoming);
        }
        Ok(())
    }

    #[test]
    fn failed_directory_sync_after_compaction_rename_fences_the_new_journal() -> Result<()> {
        let dir = TempDir::new("compaction-sync-fault");
        let current = snapshot(7, 2);
        let mut store = DurableLabStore::open(&dir.0)?;
        store.persist_head("empty", &snapshot(7, 1))?;
        store.persist_head("empty", &current)?;
        let result = store.compact_with_directory_sync(|_| {
            Err(DfmcpError::new(
                ErrorCode::AdapterUnavailable,
                "injected compaction directory sync failure",
            ))
        });
        assert!(result.is_err());
        assert_eq!(store.report().records, 1);
        assert_eq!(store.report().compactions, 1);
        assert!(store.persist_head("empty", &snapshot(7, 3)).is_err());
        drop(store);
        let mut store = DurableLabStore::open(&dir.0)?;
        assert_eq!(
            store.head(current.fortress_id).map(|head| head.anchor),
            Some(current.anchor())
        );
        store.persist_head("empty", &snapshot(7, 3))?;
        drop(store);
        let store = DurableLabStore::open(&dir.0)?;
        assert_eq!(
            store.head(current.fortress_id).map(|head| head.anchor.tick),
            Some(GameTick(3))
        );
        Ok(())
    }

    #[test]
    fn commits_and_step_states_survive_reopen_and_compaction() -> Result<()> {
        let dir = TempDir::new("commits");
        let sealed = snapshot(5, 3);
        let digest = Digest32::of_bytes(b"plan");
        let source = DurablePlanSource::Actions {
            summary: "dig".to_owned(),
            raw: r#"[{"action":{"kind":"pause","paused":true}}]"#.to_owned(),
        };
        {
            let mut store = DurableLabStore::open(&dir.0)?;
            store.persist_commit(&sealed, digest, 77, source.clone())?;
            store.persist_step(FortressId::new(5), digest, 1, "dispatched")?;
            store.persist_step(FortressId::new(5), digest, 1, "dispatched")?;
            store.persist_step(FortressId::new(5), digest, 2, "verified")?;
            assert_eq!(store.report().records, 3);
            assert!(
                store
                    .persist_step(FortressId::new(5), digest, 1, "bogus")
                    .is_err()
            );
            store.compact()?;
        }
        let mut store = DurableLabStore::open(&dir.0)?;
        let commit = store.commit(FortressId::new(5), digest).cloned();
        let commit = commit.ok_or_else(|| corrupt("commit lost"))?;
        assert_eq!(commit.source, source);
        assert_eq!(commit.intent_id, 77);
        assert_eq!(store.load_snapshot(commit.sealed_state_hash)?, sealed);
        assert_eq!(commit.steps.get(&1).map(String::as_str), Some("dispatched"));
        assert_eq!(commit.steps.get(&2).map(String::as_str), Some("verified"));
        store.retire_commit(FortressId::new(5), digest)?;
        drop(store);
        let store = DurableLabStore::open(&dir.0)?;
        assert_eq!(store.commits(FortressId::new(5)).count(), 0);
        Ok(())
    }

    #[test]
    fn original_production_sources_survive_reopen_without_upgrading_legacy_actions() -> Result<()> {
        let dir = TempDir::new("production-source");
        let sealed = snapshot(51, 1100);
        let original = r#"{"quotas":[{"item":"DRINK","minimum":60},{"item":"FOOD","minimum":50}],"template":"production"}"#;
        let sources = [
            DurablePlanSource::Production {
                summary: "original quotas".to_owned(),
                raw: original.to_owned(),
            },
            DurablePlanSource::Actions {
                summary: "legacy action record".to_owned(),
                raw: original.to_owned(),
            },
            DurablePlanSource::Blueprint {
                summary: "legacy blueprint record".to_owned(),
                raw: original.to_owned(),
            },
            DurablePlanSource::ProductionContinuation {
                summary: "continue original quotas".to_owned(),
                raw: format!(
                    "{{\"schema\":\"dfmcp.production-continuation/1\",\"parent_plan_digest\":\"{}\",\"root_plan_digest\":\"{}\",\"production\":{original}}}",
                    Digest32::of_bytes(b"parent").to_hex(),
                    Digest32::of_bytes(b"root").to_hex()
                ),
            },
        ];
        {
            let mut store = DurableLabStore::open(&dir.0)?;
            for (index, source) in sources.iter().enumerate() {
                store.persist_commit(
                    &sealed,
                    Digest32::of_bytes(&[index as u8]),
                    77,
                    source.clone(),
                )?;
            }
            store.compact()?;
        }
        let store = DurableLabStore::open(&dir.0)?;
        for (index, expected) in sources.iter().enumerate() {
            let commit = store
                .commit(sealed.fortress_id, Digest32::of_bytes(&[index as u8]))
                .ok_or_else(|| corrupt("source lost during compaction"))?;
            assert_eq!(&commit.source, expected);
            assert_eq!(store.load_snapshot(commit.sealed_state_hash)?, sealed);
        }
        Ok(())
    }

    #[test]
    fn production_source_codec_refuses_unknown_tags_malformed_text_and_excess_bytes() -> Result<()>
    {
        let dir = TempDir::new("production-source-bounds");
        let sealed = snapshot(52, 1);
        let digest = Digest32::of_bytes(b"production-source-bounds");
        let mut store = DurableLabStore::open(&dir.0)?;
        for (summary, raw) in [
            ("s".repeat(MAX_PLAN_SUMMARY_BYTES + 1), "{}".to_owned()),
            ("quota".to_owned(), "x".repeat(MAX_PLAN_REQUEST_BYTES + 1)),
        ] {
            assert!(
                store
                    .persist_commit(
                        &sealed,
                        digest,
                        1,
                        DurablePlanSource::Production { summary, raw }
                    )
                    .is_err()
            );
            assert_eq!(store.report().records, 0);
        }
        let prefix = format!(
            "P 52 {} {} {:032x}",
            digest.to_hex(),
            sealed.state_hash.to_hex(),
            1
        );
        for bad in [
            format!("{prefix} production_v2 71 7b7d"),
            format!("{prefix} production 71 f"),
            format!("{prefix} production 71 ff"),
            format!(
                "{prefix} production 71 {}",
                "78".repeat(MAX_PLAN_REQUEST_BYTES + 1)
            ),
            format!("{prefix} production 71 7b7d trailing"),
        ] {
            assert!(Record::parse(&bad).is_err_and(|error| error.code == ErrorCode::CorruptLedger));
        }
        Ok(())
    }

    #[test]
    fn continuation_source_keeps_lineage_after_objective_retirement_and_compaction() -> Result<()> {
        let dir = TempDir::new("continuation-objective");
        let sealed = snapshot(53, 1);
        let plan = Digest32::of_bytes(b"continuation plan");
        let source = DurablePlanSource::ProductionContinuation {
            summary: "new pursuit".to_owned(),
            raw: format!(
                "{{\"schema\":\"dfmcp.production-continuation/1\",\"parent_plan_digest\":\"{}\",\"root_plan_digest\":\"{}\",\"production\":{{\"template\":\"production\",\"quotas\":[{{\"item\":\"DRINK\",\"minimum\":60}}]}}}}",
                Digest32::of_bytes(b"parent").to_hex(),
                Digest32::of_bytes(b"root").to_hex()
            ),
        };
        {
            let mut store = DurableLabStore::open(&dir.0)?;
            store.persist_objective_commit(
                &sealed,
                plan,
                7,
                source.clone(),
                SessionId::new(8),
                &[],
            )?;
            store.retire_commit(sealed.fortress_id, plan)?;
            store.compact()?;
        }
        let store = DurableLabStore::open(&dir.0)?;
        let objective = store
            .objectives(sealed.fortress_id)
            .find(|objective| objective.plan_digest == plan)
            .ok_or_else(|| corrupt("continuation objective was retired with its actions"))?;
        assert_eq!(objective.source, source);
        assert!(store.commit(sealed.fortress_id, plan).is_none());
        assert_eq!(store.load_snapshot(objective.sealed_state_hash)?, sealed);
        Ok(())
    }

    /// Every crash state of an append-only journal is a byte prefix of it
    /// (objects are always synced before the record naming them). Reopening
    /// any prefix must yield exactly the state after its last complete record.
    #[test]
    fn every_crash_point_recovers_the_last_complete_record() -> Result<()> {
        let dir = TempDir::new("campaign");
        let digest = Digest32::of_bytes(b"campaign-plan");
        let mut states: Vec<(usize, Option<StateAnchor>, usize, Option<String>)> = Vec::new();
        let journal = dir.0.join("journal");
        let record_state = |store: &DurableLabStore| {
            (
                store.head(FortressId::new(4)).map(|h| h.anchor),
                store.checkpoints(FortressId::new(4)).count(),
                store
                    .commit(FortressId::new(4), digest)
                    .and_then(|c| c.steps.get(&1).cloned()),
            )
        };
        {
            let mut store = DurableLabStore::open(&dir.0)?;
            let snap = |len: &mut Vec<_>, store: &DurableLabStore| -> Result<()> {
                let bytes = fs::metadata(&journal).map_err(|e| io("meta", &e))?.len();
                let (head, checkpoints, step) = record_state(store);
                len.push((bytes as usize, head, checkpoints, step));
                Ok(())
            };
            snap(&mut states, &store)?;
            store.persist_head("starter_fortress", &snapshot(4, 1))?;
            snap(&mut states, &store)?;
            store.persist_commit(
                &snapshot(4, 1),
                digest,
                9,
                DurablePlanSource::Pause {
                    summary: "pause".to_owned(),
                    paused: true,
                },
            )?;
            snap(&mut states, &store)?;
            store.persist_step(FortressId::new(4), digest, 1, "dispatched")?;
            snap(&mut states, &store)?;
            store.persist_head("starter_fortress", &snapshot(4, 2))?;
            snap(&mut states, &store)?;
            store.persist_checkpoint(CheckpointId::new(5), "cp", &snapshot(4, 2))?;
            snap(&mut states, &store)?;
            store.persist_step(FortressId::new(4), digest, 1, "verified")?;
            snap(&mut states, &store)?;
            store.retire_commit(FortressId::new(4), digest)?;
            snap(&mut states, &store)?;
            store.persist_head("starter_fortress", &snapshot(4, 3))?;
            snap(&mut states, &store)?;
        }
        let full = fs::read(&journal).map_err(|e| io("read", &e))?;
        let mut checked = 0usize;
        for len in 0..=full.len() {
            fs::write(&journal, &full[..len]).map_err(|e| io("write", &e))?;
            // A crash mid-write can also leave temporary files behind.
            fs::write(dir.0.join("objects").join(".tmp-stale"), b"partial")
                .map_err(|e| io("tmp", &e))?;
            let store = DurableLabStore::open(&dir.0)?;
            let expected = states
                .iter()
                .rev()
                .find(|(bytes, ..)| *bytes <= len)
                .ok_or_else(|| corrupt("no expected state"))?;
            let (head, checkpoints, step) = record_state(&store);
            assert_eq!(
                (head, checkpoints, step),
                (expected.1, expected.2, expected.3.clone()),
                "crash after {len} journal bytes"
            );
            if let Some(head) = head {
                assert_eq!(store.load_snapshot(head.state_hash)?.anchor(), head);
            }
            assert_eq!(store.report().torn_tail_bytes as usize, len - expected.0);
            assert!(!dir.0.join("objects").join(".tmp-stale").exists());
            checked += 1;
        }
        assert_eq!(checked, full.len() + 1);
        Ok(())
    }

    #[test]
    fn a_second_process_cannot_open_a_held_store() -> Result<()> {
        let dir = TempDir::new("lock");
        let _held = DurableLabStore::open(&dir.0)?;
        let second = DurableLabStore::open(&dir.0);
        assert!(matches!(second, Err(ref e) if e.code == ErrorCode::Conflict));
        Ok(())
    }

    #[test]
    fn a_torn_final_record_is_discarded_and_nothing_else() -> Result<()> {
        let dir = TempDir::new("torn");
        {
            let mut store = DurableLabStore::open(&dir.0)?;
            store.persist_head("empty", &snapshot(1, 1))?;
            store.persist_head("empty", &snapshot(1, 2))?;
        }
        let journal = dir.0.join("journal");
        let mut bytes = fs::read(&journal).map_err(|e| io("read", &e))?;
        let full = bytes.len();
        bytes.truncate(full - 9);
        fs::write(&journal, &bytes).map_err(|e| io("write", &e))?;
        let store = DurableLabStore::open(&dir.0)?;
        assert!(store.report().torn_tail_bytes > 0);
        assert_eq!(store.report().records, 1);
        assert_eq!(
            store.head(FortressId::new(1)).map(|h| h.anchor.tick),
            Some(GameTick(1))
        );
        drop(store);
        // The truncation itself was made durable.
        let store = DurableLabStore::open(&dir.0)?;
        assert_eq!(store.report().torn_tail_bytes, 0);
        Ok(())
    }

    #[test]
    fn a_tampered_complete_record_refuses_to_open() -> Result<()> {
        let dir = TempDir::new("tamper");
        {
            let mut store = DurableLabStore::open(&dir.0)?;
            store.persist_head("empty", &snapshot(1, 1))?;
            store.persist_head("empty", &snapshot(1, 2))?;
        }
        let journal = dir.0.join("journal");
        let text = fs::read_to_string(&journal).map_err(|e| io("read", &e))?;
        let tampered = text
            .replacen("empty", "emptz", 1)
            .replacen(" 1 1 ", " 1 9 ", 1);
        fs::write(&journal, tampered).map_err(|e| io("write", &e))?;
        let result = DurableLabStore::open(&dir.0);
        assert!(matches!(result, Err(ref e) if e.code == ErrorCode::CorruptLedger));
        Ok(())
    }

    #[test]
    fn a_corrupt_object_refuses_to_open() -> Result<()> {
        let dir = TempDir::new("object");
        let head = snapshot(3, 4);
        {
            let mut store = DurableLabStore::open(&dir.0)?;
            store.persist_head("empty", &head)?;
        }
        let object = dir
            .0
            .join("objects")
            .join(format!("{}.snap", head.state_hash.to_hex()));
        let mut bytes = fs::read(&object).map_err(|e| io("read", &e))?;
        if let Some(last) = bytes.last_mut() {
            *last ^= 1;
        }
        fs::write(&object, bytes).map_err(|e| io("write", &e))?;
        let result = DurableLabStore::open(&dir.0);
        assert!(matches!(result, Err(ref e) if e.code == ErrorCode::CorruptLedger));
        Ok(())
    }

    #[test]
    fn compaction_keeps_live_state_and_drops_dead_objects() -> Result<()> {
        let dir = TempDir::new("compact");
        let last = snapshot(2, COMPACT_AFTER_RECORDS as u64 + 5);
        {
            let mut store = DurableLabStore::open(&dir.0)?;
            store.persist_checkpoint(CheckpointId::new(9), "keep", &snapshot(2, 0))?;
            for tick in 1..=COMPACT_AFTER_RECORDS as u64 + 5 {
                store.persist_head("empty", &snapshot(2, tick))?;
            }
            assert!(store.report().compactions >= 1);
            assert!(store.report().records < COMPACT_AFTER_RECORDS);
        }
        let store = DurableLabStore::open(&dir.0)?;
        assert_eq!(
            store.head(FortressId::new(2)).map(|h| h.anchor),
            Some(last.anchor())
        );
        assert_eq!(store.checkpoints(FortressId::new(2)).count(), 1);
        let objects = fs::read_dir(dir.0.join("objects"))
            .map_err(|e| io("list", &e))?
            .count();
        assert!(objects <= 1 + 1 + (COMPACT_AFTER_RECORDS / 2));
        Ok(())
    }

    #[test]
    fn relative_roots_and_symlinked_journals_are_refused() -> Result<()> {
        assert!(DurableLabStore::open(Path::new("relative/dir")).is_err());
        #[cfg(unix)]
        {
            let dir = TempDir::new("symlink");
            private_dir(&dir.0)?;
            std::os::unix::fs::symlink("/etc/hostname", dir.0.join("journal"))
                .map_err(|e| io("symlink", &e))?;
            assert!(DurableLabStore::open(&dir.0).is_err());
        }
        Ok(())
    }
}
