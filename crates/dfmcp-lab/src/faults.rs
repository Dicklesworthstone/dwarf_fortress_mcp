//! Seeded, serializable fault schedules over named durability boundaries.
//!
//! A [`FaultSchedule`] names exactly where a fault lands: the n-th crossing of
//! a [`Boundary`] receives one [`Fault`]. Schedules have one canonical text
//! encoding (so the same schedule always has the same bytes and digest), can
//! be generated from a seed, and are consumed identically by the in-memory
//! observation publisher ([`run_publication_campaign`]) and the crash-durable
//! laboratory journal ([`run_journal_campaign`]).
//!
//! An injected fault never panics. Each injection is appended to the
//! campaign transcript first as `indeterminate` (the effect of a crash is
//! unknown at the instant it happens) and then reconciled with what recovery
//! actually established, so the transcript is the evidence of every injection.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs::OpenOptions;
use std::path::Path;

use dfmcp_core::{
    DfmcpError, Digest32, EntityId, ErrorCode, FortressId, GameTick, ObservationCursor, Result,
    StateAnchor,
};
use dfmcp_world::{
    CapsulePublisher, DurableLedger, EntityKind, EntityRecord, ObservationCapsule, WorldGraph,
    WorldSnapshot, diff_snapshots,
};

use crate::durable::DurableLabStore;

const SCHEDULE_HEADER: &str = "dfmcp-fault-schedule/1";
const TRANSCRIPT_DOMAIN: &[u8] = b"dfmcp-fault-transcript/1\0";
/// Most fault points one schedule may carry.
pub const MAX_FAULT_POINTS: usize = 1_024;
/// Most boundary crossings one campaign may run.
pub const MAX_CAMPAIGN_STEPS: u64 = 4_096;

/// A named durability boundary a fault can land on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Boundary {
    /// Reserving a successor anchor for an observation capsule.
    PublicationReserve,
    /// Validating and staging the capsule (children).
    PublicationMaterialize,
    /// Swapping the published root.
    PublicationPublish,
    /// Appending one record to the durable journal.
    JournalAppend,
}

/// What happens at a boundary crossing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Fault {
    /// The process dies before the crossing completes.
    Crash,
    /// The write reaches storage only partially, then the process dies.
    TornWrite,
    /// The capsule arrives out of order (the next one before this one).
    Reorder,
    /// The capsule is lost in transit and must be re-sent.
    Drop,
}

impl Boundary {
    /// Every boundary, in canonical order.
    pub const ALL: [Self; 4] = [
        Self::PublicationReserve,
        Self::PublicationMaterialize,
        Self::PublicationPublish,
        Self::JournalAppend,
    ];

    /// Stable wire name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::PublicationReserve => "publication.reserve",
            Self::PublicationMaterialize => "publication.materialize",
            Self::PublicationPublish => "publication.publish",
            Self::JournalAppend => "journal.append",
        }
    }

    fn parse(raw: &str) -> Result<Self> {
        Self::ALL
            .into_iter()
            .find(|b| b.name() == raw)
            .ok_or_else(|| invalid(format!("unknown fault boundary {raw:?}")))
    }

    /// Faults that are meaningful at this boundary, in canonical order.
    #[must_use]
    pub const fn admissible(self) -> &'static [Fault] {
        match self {
            Self::PublicationReserve | Self::PublicationPublish => &[Fault::Crash, Fault::Drop],
            Self::PublicationMaterialize => &[Fault::Crash, Fault::Reorder, Fault::Drop],
            Self::JournalAppend => &[Fault::Crash, Fault::TornWrite],
        }
    }
}

impl Fault {
    /// Stable wire name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Crash => "crash",
            Self::TornWrite => "torn_write",
            Self::Reorder => "reorder",
            Self::Drop => "drop",
        }
    }

    fn parse(raw: &str) -> Result<Self> {
        [Self::Crash, Self::TornWrite, Self::Reorder, Self::Drop]
            .into_iter()
            .find(|f| f.name() == raw)
            .ok_or_else(|| invalid(format!("unknown fault {raw:?}")))
    }
}

fn invalid(message: impl Into<String>) -> DfmcpError {
    DfmcpError::new(ErrorCode::InvalidRequest, message)
}

/// One scheduled fault: the `occurrence`-th (1-based) crossing of `boundary`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct FaultPoint {
    /// Where.
    pub boundary: Boundary,
    /// Which crossing, counting from 1.
    pub occurrence: u64,
    /// What.
    pub fault: Fault,
}

/// A canonical, seed-pinned fault schedule.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FaultSchedule {
    seed: u64,
    points: BTreeMap<(Boundary, u64), Fault>,
}

/// XorShift64: tiny, deterministic, and only ever used to pick fault points.
fn next(state: &mut u64) -> u64 {
    let mut x = *state;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    *state = x;
    x
}

impl FaultSchedule {
    /// Builds a schedule from explicit points; a boundary crossing carries at
    /// most one fault and every fault must be admissible at its boundary.
    pub fn new(seed: u64, points: impl IntoIterator<Item = FaultPoint>) -> Result<Self> {
        let mut map = BTreeMap::new();
        for point in points {
            if point.occurrence == 0 || point.occurrence > MAX_CAMPAIGN_STEPS {
                return Err(invalid("fault occurrences are 1-based and bounded"));
            }
            if !point.boundary.admissible().contains(&point.fault) {
                return Err(invalid(format!(
                    "{} cannot be injected at {}",
                    point.fault.name(),
                    point.boundary.name()
                )));
            }
            if map
                .insert((point.boundary, point.occurrence), point.fault)
                .is_some()
            {
                return Err(invalid("a boundary crossing carries at most one fault"));
            }
            if map.len() > MAX_FAULT_POINTS {
                return Err(invalid("fault schedule exceeds its bound"));
            }
        }
        Ok(Self { seed, points: map })
    }

    /// Generates `count` distinct admissible faults over the first
    /// `max_occurrence` crossings of `boundaries`, purely from `seed`.
    pub fn seeded(
        seed: u64,
        boundaries: &[Boundary],
        max_occurrence: u64,
        count: usize,
    ) -> Result<Self> {
        if boundaries.is_empty() || max_occurrence == 0 || max_occurrence > MAX_CAMPAIGN_STEPS {
            return Err(invalid(
                "seeded schedules need boundaries and a bounded horizon",
            ));
        }
        let capacity = (boundaries.len() as u64).saturating_mul(max_occurrence);
        if count as u64 > capacity || count > MAX_FAULT_POINTS {
            return Err(invalid("more faults requested than boundary crossings"));
        }
        let mut state = seed | 1;
        let mut points = BTreeMap::new();
        while points.len() < count {
            let boundary = boundaries[(next(&mut state) % boundaries.len() as u64) as usize];
            let occurrence = 1 + next(&mut state) % max_occurrence;
            let faults = boundary.admissible();
            let fault = faults[(next(&mut state) % faults.len() as u64) as usize];
            points.entry((boundary, occurrence)).or_insert(fault);
        }
        Ok(Self { seed, points })
    }

    /// The seed this schedule is pinned to.
    #[must_use]
    pub const fn seed(&self) -> u64 {
        self.seed
    }

    /// Points in canonical (boundary, occurrence) order.
    pub fn points(&self) -> impl Iterator<Item = FaultPoint> + '_ {
        self.points
            .iter()
            .map(|(&(boundary, occurrence), &fault)| FaultPoint {
                boundary,
                occurrence,
                fault,
            })
    }

    /// The fault scheduled for this crossing, if any.
    #[must_use]
    pub fn at(&self, boundary: Boundary, occurrence: u64) -> Option<Fault> {
        self.points.get(&(boundary, occurrence)).copied()
    }

    /// Canonical encoding: a header line, then one `boundary occurrence fault`
    /// line per point in canonical order.
    #[must_use]
    pub fn encode(&self) -> String {
        let mut out = format!("{SCHEDULE_HEADER} seed={}\n", self.seed);
        for point in self.points() {
            let _ = writeln!(
                out,
                "{} {} {}",
                point.boundary.name(),
                point.occurrence,
                point.fault.name()
            );
        }
        out
    }

    /// Strict inverse of [`Self::encode`]; non-canonical text is refused.
    pub fn decode(text: &str) -> Result<Self> {
        let mut lines = text
            .strip_suffix('\n')
            .ok_or_else(|| invalid("fault schedule must end with a newline"))?
            .split('\n');
        let seed = lines
            .next()
            .and_then(|h| h.strip_prefix(SCHEDULE_HEADER))
            .and_then(|h| h.strip_prefix(" seed="))
            .and_then(|s| s.parse::<u64>().ok())
            .ok_or_else(|| invalid("missing or malformed fault schedule header"))?;
        let mut points = Vec::new();
        for line in lines {
            let fields: Vec<&str> = line.split(' ').collect();
            let [boundary, occurrence, fault] = fields.as_slice() else {
                return Err(invalid("fault schedule lines have three fields"));
            };
            points.push(FaultPoint {
                boundary: Boundary::parse(boundary)?,
                occurrence: occurrence
                    .parse()
                    .map_err(|_| invalid("malformed fault occurrence"))?,
                fault: Fault::parse(fault)?,
            });
            if points.len() > MAX_FAULT_POINTS {
                return Err(invalid("fault schedule exceeds its bound"));
            }
        }
        let schedule = Self::new(seed, points)?;
        if schedule.encode() != text {
            return Err(invalid("fault schedule text is not canonical"));
        }
        Ok(schedule)
    }

    /// Digest of the canonical encoding.
    #[must_use]
    pub fn digest(&self) -> Digest32 {
        Digest32::of_bytes(self.encode().as_bytes())
    }
}

/// One injected fault and what recovery established about it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InjectionRecord {
    /// Where and what.
    pub point: FaultPoint,
    /// Campaign step at which it landed.
    pub step: u64,
    /// `indeterminate` until reconciled; then what recovery proved.
    pub outcome: String,
    /// Anchor the system stood on once the fault was reconciled.
    pub anchor_after: Option<StateAnchor>,
}

/// Result of running one schedule.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CampaignReport {
    /// Digest of the schedule that ran.
    pub schedule_digest: Digest32,
    /// Every injection, in order.
    pub transcript: Vec<InjectionRecord>,
    /// Hash chain over the transcript.
    pub transcript_digest: Digest32,
    /// Anchor at the end of the campaign.
    pub final_anchor: Option<StateAnchor>,
    /// Successful operations (publications or journal records).
    pub completed: u64,
}

struct Transcript {
    records: Vec<InjectionRecord>,
}

impl Transcript {
    /// Records the fault as indeterminate before anything else happens.
    fn inject(&mut self, point: FaultPoint, step: u64) -> usize {
        self.records.push(InjectionRecord {
            point,
            step,
            outcome: "indeterminate".to_owned(),
            anchor_after: None,
        });
        self.records.len() - 1
    }

    fn reconcile(&mut self, index: usize, outcome: &str, anchor: Option<StateAnchor>) {
        if let Some(record) = self.records.get_mut(index) {
            outcome.clone_into(&mut record.outcome);
            record.anchor_after = anchor;
        }
    }

    fn finish(
        self,
        schedule: &FaultSchedule,
        final_anchor: Option<StateAnchor>,
        completed: u64,
    ) -> CampaignReport {
        let mut chain = Digest32::of_bytes(TRANSCRIPT_DOMAIN);
        for record in &self.records {
            let line = format!(
                "{} {} {} {} {} {}",
                record.point.boundary.name(),
                record.point.occurrence,
                record.point.fault.name(),
                record.step,
                record.outcome,
                record
                    .anchor_after
                    .map_or_else(|| "-".to_owned(), |a| a.state_hash.to_string())
            );
            let mut bytes = chain.as_bytes().to_vec();
            bytes.extend_from_slice(line.as_bytes());
            chain = Digest32::of_bytes(&bytes);
        }
        CampaignReport {
            schedule_digest: schedule.digest(),
            transcript: self.records,
            transcript_digest: chain,
            final_anchor,
            completed,
        }
    }
}

/// The deterministic world sequence campaigns publish: each step adds one unit.
fn campaign_world(fortress: u64, step: u64) -> WorldSnapshot {
    let mut graph = WorldGraph::default();
    for unit in 1..=step {
        graph.entities.insert(
            EntityId::new(unit),
            EntityRecord {
                id: EntityId::new(unit),
                generation: 1,
                revision: 1,
                kind: EntityKind::Unit,
                label: format!("Urist {unit}"),
                fields: BTreeMap::new(),
            },
        );
    }
    WorldSnapshot::new(
        FortressId::new(fortress),
        GameTick(step * 10),
        ObservationCursor {
            epoch: 1,
            sequence: step + 1,
        },
        false,
        graph,
    )
}

fn capsule(fortress: u64, step: u64) -> Result<ObservationCapsule> {
    let basis = campaign_world(fortress, step - 1);
    let successor = campaign_world(fortress, step);
    let delta = diff_snapshots(&basis, &successor)?;
    ObservationCapsule::new(basis.anchor(), successor.anchor(), delta, successor.tick)
}

/// Publishes `publications` capsules through a [`CapsulePublisher`] while the
/// schedule injects faults at the publication boundaries. After every fault
/// the published root must be exactly the last root published before it.
pub fn run_publication_campaign(
    schedule: &FaultSchedule,
    publications: u64,
) -> Result<CampaignReport> {
    if publications > MAX_CAMPAIGN_STEPS {
        return Err(invalid("campaign exceeds its bound"));
    }
    const FORTRESS: u64 = 31;
    let mut publisher = CapsulePublisher::new(DurableLedger::new(campaign_world(FORTRESS, 0)));
    let reader = publisher.reader();
    let mut transcript = Transcript {
        records: Vec::new(),
    };
    let mut crossings: BTreeMap<Boundary, u64> = BTreeMap::new();
    let mut cross = |boundary: Boundary| {
        let count = crossings.entry(boundary).or_insert(0);
        *count += 1;
        (*count, schedule.at(boundary, *count))
    };
    let mut step = 1;
    let mut attempts = 0_u64;
    while step <= publications {
        attempts += 1;
        if attempts > MAX_CAMPAIGN_STEPS {
            return Err(invalid("campaign did not converge within its bound"));
        }
        let stable = reader.current();
        let next = capsule(FORTRESS, step)?;

        let (n, fault) = cross(Boundary::PublicationReserve);
        let reservation = publisher.reserve(next.successor_anchor)?;
        if let Some(fault) = fault {
            let index = transcript.inject(point(Boundary::PublicationReserve, n, fault), step);
            let outcome = match fault {
                Fault::Crash => crash_recover(&mut publisher, &stable)?,
                _ => {
                    publisher.abort(reservation);
                    "reservation tombstoned; retried"
                }
            };
            transcript.reconcile(index, outcome, Some(reader.current().anchor));
            continue;
        }

        let (n, fault) = cross(Boundary::PublicationMaterialize);
        if let Some(fault) = fault {
            let index = transcript.inject(point(Boundary::PublicationMaterialize, n, fault), step);
            let outcome = match fault {
                Fault::Crash => {
                    let _lost = publisher.materialize(reservation, next)?;
                    crash_recover(&mut publisher, &stable)?
                }
                Fault::Reorder if step < publications => {
                    match publisher.materialize(reservation, capsule(FORTRESS, step + 1)?) {
                        Ok(_) => {
                            return Err(DfmcpError::new(
                                ErrorCode::CorruptLedger,
                                "an out-of-order capsule was accepted",
                            ));
                        }
                        Err(_) => "out-of-order capsule rejected; reservation tombstoned",
                    }
                }
                _ => {
                    publisher.abort(reservation);
                    "capsule lost; reservation tombstoned and capsule re-sent"
                }
            };
            unchanged(&reader.current(), &stable)?;
            transcript.reconcile(index, outcome, Some(reader.current().anchor));
            continue;
        }
        let materialized = publisher.materialize(reservation, next)?;

        let (n, fault) = cross(Boundary::PublicationPublish);
        if let Some(fault) = fault {
            let index = transcript.inject(point(Boundary::PublicationPublish, n, fault), step);
            let outcome = match fault {
                Fault::Crash => {
                    let _lost = materialized;
                    crash_recover(&mut publisher, &stable)?
                }
                _ => {
                    publisher.abort_materialized(materialized);
                    "publication dropped before the root swap; staged children discarded"
                }
            };
            unchanged(&reader.current(), &stable)?;
            transcript.reconcile(index, outcome, Some(reader.current().anchor));
            continue;
        }
        publisher.publish(materialized)?;
        step += 1;
    }
    let final_anchor = Some(reader.current().anchor);
    Ok(transcript.finish(schedule, final_anchor, publications))
}

const fn point(boundary: Boundary, occurrence: u64, fault: Fault) -> FaultPoint {
    FaultPoint {
        boundary,
        occurrence,
        fault,
    }
}

fn crash_recover(
    publisher: &mut CapsulePublisher,
    stable: &dfmcp_world::PublishedRoot,
) -> Result<&'static str> {
    let report = publisher.recover()?;
    if report.rederived_head != stable.anchor {
        return Err(DfmcpError::new(
            ErrorCode::CorruptLedger,
            "recovery re-derived a different root than the last published one",
        ));
    }
    Ok("crash recovered: staged children discarded, root re-derived identically")
}

fn unchanged(
    current: &dfmcp_world::PublishedRoot,
    stable: &dfmcp_world::PublishedRoot,
) -> Result<()> {
    if current == stable {
        Ok(())
    } else {
        Err(DfmcpError::new(
            ErrorCode::CorruptLedger,
            "a faulted publication changed the visible root",
        ))
    }
}

/// Persists `records` heads into a durable laboratory store at `root` while the
/// schedule crashes or tears journal appends. After every fault the store is
/// reopened and must stand exactly on the last completely written head.
pub fn run_journal_campaign(
    root: &Path,
    schedule: &FaultSchedule,
    records: u64,
) -> Result<CampaignReport> {
    if records > MAX_CAMPAIGN_STEPS {
        return Err(invalid("campaign exceeds its bound"));
    }
    const FORTRESS: u64 = 32;
    let fortress = FortressId::new(FORTRESS);
    let journal = root.join("journal");
    let mut store = DurableLabStore::open(root)?;
    let mut transcript = Transcript {
        records: Vec::new(),
    };
    let mut crossings = 0_u64;
    let mut step = 1;
    while step <= records {
        crossings += 1;
        if crossings > MAX_CAMPAIGN_STEPS {
            return Err(invalid("campaign did not converge within its bound"));
        }
        let stable = store.head(fortress).map(|head| head.anchor);
        let snapshot = campaign_world(FORTRESS, step);
        let Some(fault) = schedule.at(Boundary::JournalAppend, crossings) else {
            store.persist_head("fault_campaign", &snapshot)?;
            step += 1;
            continue;
        };
        let index = transcript.inject(point(Boundary::JournalAppend, crossings, fault), step);
        let outcome = if fault == Fault::TornWrite {
            let before = journal_len(&journal)?;
            store.persist_head("fault_campaign", &snapshot)?;
            let after = journal_len(&journal)?;
            if after <= before + 1 {
                return Err(invalid("journal append wrote no record"));
            }
            // Keep a seed-determined, strictly partial prefix of the record.
            let mut state = schedule.seed() ^ crossings.rotate_left(17) | 1;
            let kept = 1 + next(&mut state) % (after - before - 1);
            drop(store);
            OpenOptions::new()
                .write(true)
                .open(&journal)
                .and_then(|file| file.set_len(before + kept).and_then(|()| file.sync_all()))
                .map_err(|error| invalid(format!("cannot tear the journal: {error}")))?;
            "torn record discarded on reopen; previous head stands"
        } else {
            store.set_append_budget(Some(0));
            if store.persist_head("fault_campaign", &snapshot).is_ok() {
                return Err(invalid("a crashed store accepted a write"));
            }
            drop(store);
            "crash before append; previous head stands"
        };
        store = DurableLabStore::open(root)?;
        let recovered = store.head(fortress).map(|head| head.anchor);
        if recovered != stable {
            return Err(DfmcpError::new(
                ErrorCode::CorruptLedger,
                "reopened journal does not stand on the last complete record",
            ));
        }
        if let Some(anchor) = recovered {
            store.load_snapshot(anchor.state_hash)?;
        }
        transcript.reconcile(index, outcome, recovered);
    }
    let final_anchor = store.head(fortress).map(|head| head.anchor);
    Ok(transcript.finish(schedule, final_anchor, records))
}

fn journal_len(path: &Path) -> Result<u64> {
    std::fs::metadata(path)
        .map(|m| m.len())
        .map_err(|error| invalid(format!("cannot stat the journal: {error}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PUBLICATION: [Boundary; 3] = [
        Boundary::PublicationReserve,
        Boundary::PublicationMaterialize,
        Boundary::PublicationPublish,
    ];

    #[test]
    fn schedules_are_seed_pinned_and_round_trip_byte_identically() -> Result<()> {
        let a = FaultSchedule::seeded(0xDF, &Boundary::ALL, 40, 24)?;
        let b = FaultSchedule::seeded(0xDF, &Boundary::ALL, 40, 24)?;
        assert_eq!(a.encode(), b.encode());
        assert_eq!(a.digest(), b.digest());
        assert_ne!(
            a.digest(),
            FaultSchedule::seeded(0xE0, &Boundary::ALL, 40, 24)?.digest()
        );
        assert_eq!(FaultSchedule::decode(&a.encode())?, a);
        assert!(
            a.points()
                .all(|p| p.boundary.admissible().contains(&p.fault))
        );
        // Non-canonical or inadmissible text is refused, not normalised.
        let swapped: String = {
            let text = a.encode();
            let mut lines: Vec<&str> = text.lines().collect();
            lines.swap(1, 2);
            lines.join("\n") + "\n"
        };
        assert!(FaultSchedule::decode(&swapped).is_err());
        assert!(
            FaultSchedule::decode(&format!(
                "{SCHEDULE_HEADER} seed=1\njournal.append 1 reorder\n"
            ))
            .is_err()
        );
        assert!(
            FaultSchedule::decode(&format!("{SCHEDULE_HEADER} seed=1\n"))?
                .points()
                .next()
                .is_none()
        );
        Ok(())
    }

    #[test]
    fn publication_faults_never_expose_a_partial_root_and_replay_identically() -> Result<()> {
        let clean = run_publication_campaign(&FaultSchedule::new(1, [])?, 30)?;
        for seed in 1..=8 {
            let schedule = FaultSchedule::seeded(seed, &PUBLICATION, 30, 12)?;
            let first = run_publication_campaign(&schedule, 30)?;
            let second = run_publication_campaign(&schedule, 30)?;
            assert_eq!(first, second, "seed {seed} replays byte-identically");
            assert_eq!(first.transcript.len(), 12, "every injection is recorded");
            assert!(
                first
                    .transcript
                    .iter()
                    .all(|r| r.outcome != "indeterminate" && r.anchor_after.is_some())
            );
            assert_eq!(
                first.final_anchor, clean.final_anchor,
                "faults never change the result"
            );
        }
        Ok(())
    }

    #[test]
    fn a_crash_at_publish_recovers_by_rederiving_the_root() -> Result<()> {
        let schedule =
            FaultSchedule::new(7, [point(Boundary::PublicationPublish, 3, Fault::Crash)])?;
        let report = run_publication_campaign(&schedule, 5)?;
        let [record] = report.transcript.as_slice() else {
            return Err(invalid("one injection expected"));
        };
        assert_eq!(record.step, 3);
        assert!(record.outcome.starts_with("crash recovered"));
        assert_eq!(record.anchor_after, Some(campaign_world(31, 2).anchor()));
        Ok(())
    }

    #[test]
    fn journal_crashes_and_torn_writes_reopen_on_the_last_complete_record() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("dfmcp-fault-journal-{}", std::process::id()));
        let schedule = FaultSchedule::seeded(0x5EED, &[Boundary::JournalAppend], 24, 10)?;
        let mut digests = Vec::new();
        for _ in 0..2 {
            let _ = std::fs::remove_dir_all(&dir);
            let report = run_journal_campaign(&dir, &schedule, 14)?;
            assert_eq!(report.transcript.len(), 10);
            assert_eq!(report.final_anchor, Some(campaign_world(32, 14).anchor()));
            assert!(
                report
                    .transcript
                    .iter()
                    .any(|r| r.point.fault == Fault::TornWrite)
            );
            digests.push(report.transcript_digest);
        }
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(digests[0], digests[1]);
        Ok(())
    }
}
