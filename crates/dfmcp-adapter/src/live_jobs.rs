#![forbid(unsafe_code)]

//! The jobs/1.2 observation profile is independent of citizen/announcement reads.
//! A complete frame describes the current global job list, not job history,
//! inventory availability, building completion, or the reason a job is suspended.

use dfmcp_core::{
    DfmcpError, Digest32, EntityId, ErrorCode, FortressId, GameTick, MapCoord, ObservationCursor,
    Result,
};
use dfmcp_world::{
    EntityKind, EntityRecord, Fact, FactPresence, FactSource, Value, WorldGraph, WorldSnapshot,
};
use std::collections::BTreeMap;

pub const MAX_JOBS: usize = 4096;
pub const MAX_JOB_FRAME_BYTES: usize = 2 * 1024 * 1024;
const MAX_TRACKED_JOB_IDS: usize = 65_536;
const TICKS_PER_YEAR: u32 = 403_200;
const MAGIC: &[u8; 8] = b"DFMJ1200";

fn invalid(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::AdapterRejected, message)
}
fn exhausted(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::BudgetExceeded, message)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveJob {
    pub native_id: u32,
    pub job_type: i32,
    pub type_key: String,
    pub reaction: String,
    pub suspended: bool,
    pub repeating: bool,
    pub position: MapCoord,
    /// Native identities, deliberately not handles into a separately observed graph.
    pub worker_native_id: Option<u32>,
    pub holder_native_id: Option<u32>,
    pub completion_timer: i32,
    pub attached_item_count: u32,
    pub required_item_filter_count: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveJobObservation {
    pub bridge_generation: u64,
    pub df_version: String,
    pub dfhack_version: String,
    pub year: u32,
    pub year_tick: u32,
    pub paused: bool,
    pub site_id: i32,
    pub world_folder: String,
    pub next_job_id: u32,
    pub jobs: Vec<LiveJob>,
}

impl LiveJobObservation {
    pub fn validate(&self) -> Result<()> {
        if self.bridge_generation == 0
            || self.site_id < 0
            || self.year_tick >= TICKS_PER_YEAR
            || self.next_job_id > i32::MAX as u32
        {
            return Err(invalid(
                "invalid jobs source generation, site, clock, or ID horizon",
            ));
        }
        for (text, max, empty) in [
            (&self.world_folder, 512, false),
            (&self.df_version, 128, false),
            (&self.dfhack_version, 128, false),
        ] {
            validate_text(text, max, empty)?;
        }
        if self.jobs.len() > MAX_JOBS {
            return Err(exhausted("job roster exceeds 4096 entries"));
        }
        let mut previous = None;
        for job in &self.jobs {
            if job.native_id >= self.next_job_id
                || previous.is_some_and(|id| id >= job.native_id)
                || job.job_type < 0
                || job.completion_timer < -1
                || job.worker_native_id.is_some_and(|id| id > i32::MAX as u32)
                || job.holder_native_id.is_some_and(|id| id > i32::MAX as u32)
            {
                return Err(invalid(
                    "job record has invalid identity, order, type, timer, or reference",
                ));
            }
            validate_text(&job.type_key, 128, false)?;
            validate_text(&job.reaction, 128, true)?;
            if job.attached_item_count > 65_536 || job.required_item_filter_count > 4096 {
                return Err(exhausted("job item-reference counts exceed their bounds"));
            }
            previous = Some(job.native_id);
        }
        Ok(())
    }

    /// Native payload, excluding the separately authenticated reply manifest.
    pub fn encode_payload(&self) -> Result<Vec<u8>> {
        self.validate()?;
        let mut out = MAGIC.to_vec();
        put_u32(&mut out, self.year);
        put_u32(&mut out, self.year_tick);
        out.push(u8::from(self.paused));
        put_i32(&mut out, self.site_id);
        put_u32(&mut out, self.next_job_id);
        put_text(&mut out, &self.world_folder)?;
        put_u32(&mut out, self.jobs.len() as u32);
        for job in &self.jobs {
            put_u32(&mut out, job.native_id);
            put_i32(&mut out, job.job_type);
            put_text(&mut out, &job.type_key)?;
            put_text(&mut out, &job.reaction)?;
            out.push(u8::from(job.suspended));
            out.push(u8::from(job.repeating));
            for coordinate in [job.position.x, job.position.y, job.position.z] {
                put_i32(&mut out, coordinate);
            }
            for reference in [job.worker_native_id, job.holder_native_id] {
                out.push(u8::from(reference.is_some()));
                if let Some(id) = reference {
                    put_u32(&mut out, id);
                }
            }
            put_i32(&mut out, job.completion_timer);
            put_u32(&mut out, job.attached_item_count);
            put_u32(&mut out, job.required_item_filter_count);
        }
        if out.len() > MAX_JOB_FRAME_BYTES {
            return Err(exhausted("job payload exceeds byte bound"));
        }
        Ok(out)
    }

    pub fn decode_payload(
        bytes: &[u8],
        generation: u64,
        df_version: String,
        dfhack_version: String,
    ) -> Result<Self> {
        if bytes.len() > MAX_JOB_FRAME_BYTES {
            return Err(exhausted("job payload exceeds byte bound"));
        }
        let mut reader = Reader { bytes, offset: 0 };
        if reader.take(8)? != MAGIC {
            return Err(invalid("unsupported job payload schema"));
        }
        let year = reader.u32()?;
        let year_tick = reader.u32()?;
        let paused = reader.boolean()?;
        let site_id = reader.i32()?;
        let next_job_id = reader.u32()?;
        let world_folder = reader.text(512)?;
        let count = reader.u32()? as usize;
        if count > MAX_JOBS {
            return Err(exhausted("job payload count exceeds bound"));
        }
        let mut jobs = Vec::with_capacity(count);
        for _ in 0..count {
            jobs.push(LiveJob {
                native_id: reader.u32()?,
                job_type: reader.i32()?,
                type_key: reader.text(128)?,
                reaction: reader.text(128)?,
                suspended: reader.boolean()?,
                repeating: reader.boolean()?,
                position: MapCoord::new(reader.i32()?, reader.i32()?, reader.i32()?),
                worker_native_id: reader.reference()?,
                holder_native_id: reader.reference()?,
                completion_timer: reader.i32()?,
                attached_item_count: reader.u32()?,
                required_item_filter_count: reader.u32()?,
            });
        }
        if reader.offset != bytes.len() {
            return Err(invalid("trailing bytes in job payload"));
        }
        let result = Self {
            bridge_generation: generation,
            df_version,
            dfhack_version,
            year,
            year_tick,
            paused,
            site_id,
            world_folder,
            next_job_id,
            jobs,
        };
        result.validate()?;
        Ok(result)
    }

    pub fn source_digest(&self) -> Result<Digest32> {
        let mut bytes = b"dfmcp-jobs-source-1.2\0".to_vec();
        bytes.extend_from_slice(&self.bridge_generation.to_be_bytes());
        put_text(&mut bytes, &self.df_version)?;
        put_text(&mut bytes, &self.dfhack_version)?;
        bytes.extend_from_slice(&self.encode_payload()?);
        Ok(Digest32::of_bytes(&bytes))
    }

    /// Same fortress identity derivation as the existing live citizen profile.
    pub fn fortress_id(&self) -> Result<FortressId> {
        self.validate()?;
        let mut bytes = b"dfmcp-live-fortress-id-v1\0".to_vec();
        bytes.extend_from_slice(self.world_folder.as_bytes());
        bytes.push(0);
        bytes.extend_from_slice(&self.site_id.to_be_bytes());
        let hash = Digest32::of_bytes(&bytes);
        let raw: [u8; 8] = hash.as_bytes()[..8]
            .try_into()
            .map_err(|_| invalid("invalid fortress identity digest"))?;
        Ok(FortressId::new(u64::from_be_bytes(raw) | 1))
    }

    #[must_use]
    pub fn tick(&self) -> GameTick {
        GameTick(u64::from(self.year) * u64::from(TICKS_PER_YEAR) + u64::from(self.year_tick))
    }
}

fn validate_text(text: &str, max: usize, allow_empty: bool) -> Result<()> {
    if (!allow_empty && text.is_empty()) || text.len() > max || text.contains('\0') {
        return Err(invalid("job source text violates its exact byte bounds"));
    }
    Ok(())
}
fn put_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_be_bytes());
}
fn put_i32(out: &mut Vec<u8>, value: i32) {
    out.extend_from_slice(&value.to_be_bytes());
}
fn put_text(out: &mut Vec<u8>, text: &str) -> Result<()> {
    let length = u16::try_from(text.len()).map_err(|_| exhausted("text length exceeds u16"))?;
    out.extend_from_slice(&length.to_be_bytes());
    out.extend_from_slice(text.as_bytes());
    Ok(())
}
struct Reader<'a> {
    bytes: &'a [u8],
    offset: usize,
}
impl<'a> Reader<'a> {
    fn take(&mut self, length: usize) -> Result<&'a [u8]> {
        let end = self
            .offset
            .checked_add(length)
            .ok_or_else(|| invalid("job length overflow"))?;
        let bytes = self
            .bytes
            .get(self.offset..end)
            .ok_or_else(|| invalid("truncated job payload"))?;
        self.offset = end;
        Ok(bytes)
    }
    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_be_bytes(
            self.take(4)?
                .try_into()
                .map_err(|_| invalid("invalid u32"))?,
        ))
    }
    fn i32(&mut self) -> Result<i32> {
        Ok(i32::from_be_bytes(
            self.take(4)?
                .try_into()
                .map_err(|_| invalid("invalid i32"))?,
        ))
    }
    fn boolean(&mut self) -> Result<bool> {
        match self.take(1)?[0] {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(invalid("noncanonical boolean")),
        }
    }
    fn reference(&mut self) -> Result<Option<u32>> {
        if self.boolean()? {
            Ok(Some(self.u32()?))
        } else {
            Ok(None)
        }
    }
    fn text(&mut self, maximum: usize) -> Result<String> {
        let length = u16::from_be_bytes(
            self.take(2)?
                .try_into()
                .map_err(|_| invalid("invalid text length"))?,
        ) as usize;
        if length > maximum {
            return Err(exhausted("job text exceeds byte bound"));
        }
        String::from_utf8(self.take(length)?.to_vec())
            .map_err(|_| invalid("invalid UTF-8 in job payload"))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobPublication {
    Bootstrap,
    Heartbeat,
    Advanced,
    Reset,
}

#[derive(Clone, Debug, Default)]
pub struct LiveJobsState {
    observation: Option<LiveJobObservation>,
    snapshot: Option<WorldSnapshot>,
    generations: BTreeMap<u32, u32>,
}

impl LiveJobsState {
    #[must_use]
    pub fn snapshot(&self) -> Option<&WorldSnapshot> {
        self.snapshot.as_ref()
    }
    #[must_use]
    pub fn observation(&self) -> Option<&LiveJobObservation> {
        self.observation.as_ref()
    }

    /// Materialize and validate everything before replacing the visible root.
    pub fn publish(&mut self, observation: LiveJobObservation) -> Result<JobPublication> {
        observation.validate()?;
        let fortress = observation.fortress_id()?;
        let mut outcome = JobPublication::Bootstrap;
        let mut cursor = ObservationCursor {
            epoch: 0,
            sequence: 0,
        };
        if let (Some(prior), Some(snapshot)) = (&self.observation, &self.snapshot) {
            if fortress != snapshot.fortress_id
                || observation.df_version != prior.df_version
                || observation.dfhack_version != prior.dfhack_version
            {
                return Err(DfmcpError::new(
                    ErrorCode::StaleAnchor,
                    "jobs source changed fortress or software version; reopen the session",
                ));
            }
            if &observation == prior {
                return Ok(JobPublication::Heartbeat);
            }
            let reset = observation.bridge_generation != prior.bridge_generation
                || observation.tick() < prior.tick()
                || observation.next_job_id < prior.next_job_id;
            outcome = if reset {
                JobPublication::Reset
            } else {
                JobPublication::Advanced
            };
            cursor = if reset {
                ObservationCursor {
                    epoch: snapshot
                        .cursor
                        .epoch
                        .checked_add(1)
                        .ok_or_else(|| exhausted("jobs epoch exhausted"))?,
                    sequence: 0,
                }
            } else {
                ObservationCursor {
                    epoch: snapshot.cursor.epoch,
                    sequence: snapshot
                        .cursor
                        .sequence
                        .checked_add(1)
                        .ok_or_else(|| exhausted("jobs sequence exhausted"))?,
                }
            };
        }
        let mut generations = self.generations.clone();
        let source = observation.source_digest()?;
        let tick = observation.tick();
        let mut graph = WorldGraph::default();
        let fact = |field: &str, value| {
            Fact::known(
                value,
                tick,
                FactSource::DfhackField(format!("jobs/1.2.{field}")),
                source,
            )
        };
        let revision = cursor
            .sequence
            .checked_add(1)
            .ok_or_else(|| exhausted("jobs revision exhausted"))?;
        let fortress_generation = u32::try_from(
            cursor
                .epoch
                .checked_add(1)
                .ok_or_else(|| exhausted("jobs epoch exhausted"))?,
        )
        .map_err(|_| exhausted("jobs generation exhausted"))?;
        graph.entities.insert(
            EntityId::new(1),
            EntityRecord {
                id: EntityId::new(1),
                generation: fortress_generation,
                revision,
                kind: EntityKind::Fortress,
                label: "Fortress job roster".to_owned(),
                fields: BTreeMap::from([
                    (
                        "job_count".to_owned(),
                        fact("job_count", Value::U64(observation.jobs.len() as u64)),
                    ),
                    (
                        "paused".to_owned(),
                        fact("paused", Value::Bool(observation.paused)),
                    ),
                    (
                        "next_job_id".to_owned(),
                        fact(
                            "next_job_id",
                            Value::U64(u64::from(observation.next_job_id)),
                        ),
                    ),
                ]),
            },
        );
        for job in &observation.jobs {
            let id = EntityId::new(u64::from(job.native_id) + 2);
            let previously_present = self
                .snapshot
                .as_ref()
                .is_some_and(|snapshot| snapshot.graph.entities.contains_key(&id));
            let generation = match generations.get(&job.native_id).copied() {
                Some(previous) if !previously_present || outcome == JobPublication::Reset => {
                    previous
                        .checked_add(1)
                        .ok_or_else(|| exhausted("job generation exhausted"))?
                }
                Some(previous) => previous,
                None => 1,
            };
            if !generations.contains_key(&job.native_id) && generations.len() >= MAX_TRACKED_JOB_IDS
            {
                return Err(exhausted(
                    "job identity history is full; reopen the session",
                ));
            }
            generations.insert(job.native_id, generation);
            let mut fields = BTreeMap::from([
                (
                    "native_job_id".to_owned(),
                    fact("job.id", Value::U64(u64::from(job.native_id))),
                ),
                (
                    "job_type".to_owned(),
                    fact("job.type", Value::I64(i64::from(job.job_type))),
                ),
                (
                    "type_key".to_owned(),
                    fact("job.type_key", Value::Text(job.type_key.clone())),
                ),
                (
                    "reaction".to_owned(),
                    fact("job.reaction_name", Value::Text(job.reaction.clone())),
                ),
                (
                    "suspended".to_owned(),
                    fact("job.flags.suspend", Value::Bool(job.suspended)),
                ),
                (
                    "repeating".to_owned(),
                    fact("job.flags.repeat", Value::Bool(job.repeating)),
                ),
                (
                    "worker_assigned".to_owned(),
                    fact("Job.getWorker", Value::Bool(job.worker_native_id.is_some())),
                ),
                (
                    "attached_item_count".to_owned(),
                    fact(
                        "job.items.size",
                        Value::U64(u64::from(job.attached_item_count)),
                    ),
                ),
                (
                    "required_item_filter_count".to_owned(),
                    fact(
                        "job.job_items.elements.size",
                        Value::U64(u64::from(job.required_item_filter_count)),
                    ),
                ),
            ]);
            for (name, reference) in [
                ("worker_native_id", job.worker_native_id),
                ("holder_native_id", job.holder_native_id),
            ] {
                fields.insert(
                    name.to_owned(),
                    match reference {
                        Some(value) => fact(name, Value::U64(u64::from(value))),
                        None => Fact::with_presence(
                            FactPresence::Absent,
                            tick,
                            FactSource::DfhackField(format!("jobs/1.2.{name}")),
                            source,
                        ),
                    },
                );
            }
            fields.insert(
                "position".to_owned(),
                if job.position.x >= 0 && job.position.y >= 0 && job.position.z >= 0 {
                    fact("job.pos", Value::Coord(job.position))
                } else {
                    Fact::with_presence(
                        FactPresence::Unknown("job has no valid map position".to_owned()),
                        tick,
                        FactSource::DfhackField("jobs/1.2.job.pos".to_owned()),
                        source,
                    )
                },
            );
            fields.insert(
                "completion_timer_raw".to_owned(),
                fact(
                    "job.completion_timer",
                    Value::I64(i64::from(job.completion_timer)),
                ),
            );
            fields.insert("blocking_reason".to_owned(), Fact::with_presence(
                FactPresence::Unsupported("suspension/assignment flags do not establish material, path, or labor blockers".to_owned()),
                tick, FactSource::DfhackField("jobs/1.2.coverage".to_owned()), source));
            graph.entities.insert(
                id,
                EntityRecord {
                    id,
                    generation,
                    revision,
                    kind: EntityKind::Job,
                    label: format!("{} #{}", job.type_key, job.native_id),
                    fields,
                },
            );
        }
        let snapshot = WorldSnapshot::new(fortress, tick, cursor, observation.paused, graph);
        self.generations = generations;
        self.observation = Some(observation);
        self.snapshot = Some(snapshot);
        Ok(outcome)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn observation() -> LiveJobObservation {
        LiveJobObservation {
            bridge_generation: 7,
            df_version: "test-df".to_owned(),
            dfhack_version: "test-dfhack".to_owned(),
            year: 105,
            year_tick: 3,
            paused: true,
            site_id: 4,
            world_folder: "region1".to_owned(),
            next_job_id: 10,
            jobs: vec![LiveJob {
                native_id: 0,
                job_type: 5,
                type_key: "Dig".to_owned(),
                reaction: String::new(),
                suspended: true,
                repeating: false,
                position: MapCoord::new(1, 2, 3),
                worker_native_id: None,
                holder_native_id: Some(0),
                completion_timer: -1,
                attached_item_count: 0,
                required_item_filter_count: 1,
            }],
        }
    }
    #[test]
    fn payload_roundtrip_and_every_truncation() -> Result<()> {
        let source = observation();
        let bytes = source.encode_payload()?;
        assert_eq!(
            LiveJobObservation::decode_payload(
                &bytes,
                7,
                "test-df".to_owned(),
                "test-dfhack".to_owned()
            )?,
            source
        );
        for end in 0..bytes.len() {
            assert!(
                LiveJobObservation::decode_payload(
                    &bytes[..end],
                    7,
                    "df".to_owned(),
                    "dfhack".to_owned()
                )
                .is_err()
            );
        }
        let mut extra = bytes;
        extra.push(0);
        assert!(
            LiveJobObservation::decode_payload(&extra, 7, "df".to_owned(), "dfhack".to_owned())
                .is_err()
        );
        Ok(())
    }
    #[test]
    fn malformed_values_do_not_publish() -> Result<()> {
        let mut state = LiveJobsState::default();
        state.publish(observation())?;
        let before = state.snapshot().cloned();
        for change in 0..6 {
            let mut invalid = observation();
            match change {
                0 => invalid.jobs.push(invalid.jobs[0].clone()),
                1 => invalid.year_tick = TICKS_PER_YEAR,
                2 => invalid.jobs[0].native_id = invalid.next_job_id,
                3 => invalid.world_folder = "x\0y".to_owned(),
                4 => invalid.jobs[0].completion_timer = -2,
                _ => invalid.jobs[0].worker_native_id = Some(u32::MAX),
            }
            assert!(state.publish(invalid).is_err());
            assert_eq!(state.snapshot(), before.as_ref());
        }
        Ok(())
    }
    #[test]
    fn ordinary_progress_is_not_a_fake_completion() -> Result<()> {
        let mut state = LiveJobsState::default();
        assert_eq!(state.publish(observation())?, JobPublication::Bootstrap);
        assert_eq!(state.publish(observation())?, JobPublication::Heartbeat);
        let mut next = observation();
        next.year_tick += 1;
        next.jobs[0].suspended = false;
        assert_eq!(state.publish(next)?, JobPublication::Advanced);
        let snapshot = state
            .snapshot()
            .ok_or_else(|| invalid("missing snapshot"))?;
        assert_eq!(
            snapshot.graph.entities[&EntityId::new(2)].fields["suspended"].value,
            Value::Bool(false)
        );
        assert!(matches!(
            snapshot.graph.entities[&EntityId::new(2)].fields["blocking_reason"].presence,
            Some(FactPresence::Unsupported(_))
        ));
        assert!(snapshot.hash_is_valid());
        Ok(())
    }
    #[test]
    fn retirement_reappearance_advances_generation() -> Result<()> {
        let mut state = LiveJobsState::default();
        state.publish(observation())?;
        let mut retired = observation();
        retired.jobs.clear();
        retired.year_tick += 1;
        state.publish(retired)?;
        let mut returned = observation();
        returned.year_tick += 2;
        state.publish(returned)?;
        assert_eq!(
            state
                .snapshot()
                .ok_or_else(|| invalid("missing snapshot"))?
                .graph
                .entities[&EntityId::new(2)]
                .generation,
            2
        );
        Ok(())
    }
    #[test]
    fn restart_clock_and_job_horizon_reset_epochs() -> Result<()> {
        for change in 0..3 {
            let mut state = LiveJobsState::default();
            state.publish(observation())?;
            let mut next = observation();
            match change {
                0 => next.bridge_generation += 1,
                1 => next.year_tick -= 1,
                _ => next.next_job_id -= 1,
            }
            assert_eq!(state.publish(next)?, JobPublication::Reset);
            assert_eq!(
                state
                    .snapshot()
                    .ok_or_else(|| invalid("missing snapshot"))?
                    .cursor
                    .epoch,
                1
            );
        }
        Ok(())
    }
    #[test]
    fn source_switch_refuses_without_replacing_root() -> Result<()> {
        let mut state = LiveJobsState::default();
        state.publish(observation())?;
        let before = state.snapshot().cloned();
        let mut switched = observation();
        switched.site_id += 1;
        assert!(state.publish(switched).is_err());
        assert_eq!(state.snapshot(), before.as_ref());
        Ok(())
    }
}
