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
    StateAnchor,
};
use dfmcp_world::WorldSnapshot;

const JOURNAL_DOMAIN: &[u8] = b"dfmcp-lab-journal/1\0";
/// Largest journal accepted on open.
pub const MAX_JOURNAL_BYTES: u64 = 64 * 1024 * 1024;
/// Largest single journal record line.
pub const MAX_RECORD_BYTES: usize = 4 * 1024;
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

#[derive(Clone, Debug, PartialEq, Eq)]
enum Record {
    Head(DurableHead),
    Checkpoint(DurableCheckpoint),
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
    if raw == "-" {
        return Ok(String::new());
    }
    if raw.len() % 2 != 0 || raw.len() / 2 > bound {
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
    let text = String::from_utf8(bytes).map_err(|_| corrupt("journal text field is not UTF-8"))?;
    if text.chars().any(char::is_control) {
        return Err(corrupt("journal text field contains control characters"));
    }
    Ok(text)
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
            Self::Checkpoint(checkpoint) => format!(
                "C {} {:032x} {} {}",
                checkpoint.fortress_id.get(),
                checkpoint.checkpoint_id.get(),
                hex_text(&checkpoint.label),
                checkpoint.state_hash.to_hex(),
            ),
        }
    }

    fn parse(payload: &str) -> Result<Self> {
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
            _ => Err(corrupt("journal record kind or arity is not recognized")),
        }
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
        for hash in store.referenced_objects() {
            store.load_snapshot(hash)?;
        }
        Ok(store)
    }

    fn replay_line(&mut self, line: &[u8]) -> Result<()> {
        if line.len() > MAX_RECORD_BYTES + 65 {
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

    fn append(&mut self, record: Record) -> Result<()> {
        let payload = record.payload();
        if payload.len() > MAX_RECORD_BYTES {
            return Err(invalid("durable journal record exceeds its bound"));
        }
        let mut next = self.index.clone();
        next.apply(record)?;
        let chain = chain_next(self.chain, &payload);
        let line = format!("{} {payload}\n", chain.to_hex());
        self.journal
            .write_all(line.as_bytes())
            .and_then(|()| self.journal.sync_data())
            .map_err(|e| io("cannot append to the durable laboratory journal", &e))?;
        self.index = next;
        self.chain = chain;
        self.records += 1;
        if self.records >= COMPACT_AFTER_RECORDS {
            self.compact()?;
        }
        Ok(())
    }

    /// Persist `snapshot` as the latest state of its fortress.
    pub fn persist_head(&mut self, scenario: &str, snapshot: &WorldSnapshot) -> Result<()> {
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
        self.write_object(snapshot)?;
        self.append(Record::Head(DurableHead {
            fortress_id: snapshot.fortress_id,
            scenario: scenario.to_owned(),
            anchor: snapshot.anchor(),
        }))
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
        self.write_object(snapshot)?;
        self.append(Record::Checkpoint(DurableCheckpoint {
            fortress_id: snapshot.fortress_id,
            checkpoint_id,
            label: label.to_owned(),
            state_hash: snapshot.state_hash,
        }))
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

    /// Rewrite the journal as the live heads and checkpoints only, then drop
    /// objects nothing references. The replacement is synced and renamed over
    /// the old journal, so a crash leaves one complete journal or the other.
    pub fn compact(&mut self) -> Result<()> {
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
            .collect();
        for record in live {
            let payload = record.payload();
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
        sync_dir(&self.root)?;
        self.journal = file;
        self.chain = chain;
        self.records = records;
        self.compactions += 1;

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
}

impl Index {
    fn apply(&mut self, record: Record) -> Result<()> {
        match record {
            Record::Head(head) => {
                if !self.heads.contains_key(&head.fortress_id) && self.heads.len() >= MAX_FORTRESSES
                {
                    return Err(DfmcpError::new(
                        ErrorCode::BudgetExceeded,
                        "durable laboratory store reached its fortress bound",
                    ));
                }
                self.heads.insert(head.fortress_id, head);
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
        }
        Ok(())
    }
}

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
