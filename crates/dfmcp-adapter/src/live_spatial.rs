#![forbid(unsafe_code)]

//! One native spatial/1.6 frame, not a join of independently fetched snapshots.
//! Embedded 1.4 operations and 1.5 terrain are codecs, not source authorities.

#[path = "live_spatial_citizens.rs"]
pub mod citizens;

use crate::live_jobs::JobPublication;
use crate::live_map::{LiveMapObservation, LiveMapState, MAX_MAP_BYTES};
use crate::live_operations::{LiveOperationsObservation, LiveOperationsState, OperationsProfile};
use dfmcp_core::{DfmcpError, Digest32, EntityId, ErrorCode, ObservationCursor, Result};
use dfmcp_world::{Fact, FactSource, Value, WorldSnapshot};
use std::collections::BTreeMap;

pub const MAX_SPATIAL_BYTES: usize = 16 * 1024 * 1024;
const MAGIC: &[u8; 8] = b"DFMS1600";
const MAX_IDENTITIES: usize = 262_144;
fn invalid(text: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::AdapterRejected, text)
}
fn exhausted(text: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::BudgetExceeded, text)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveSpatialObservation {
    operations: LiveOperationsObservation,
    terrain: LiveMapObservation,
}
impl LiveSpatialObservation {
    pub fn operations(&self) -> &LiveOperationsObservation {
        &self.operations
    }
    pub fn terrain(&self) -> &LiveMapObservation {
        &self.terrain
    }
    pub fn validate(&self) -> Result<()> {
        self.operations
            .validate_profile(OperationsProfile::PagedV1_4)?;
        self.terrain.validate()?;
        let a = &self.operations.jobs;
        let b = &self.terrain;
        if a.bridge_generation != b.bridge_generation
            || a.df_version != b.df_version
            || a.dfhack_version != b.dfhack_version
            || a.year != b.year
            || a.year_tick != b.year_tick
            || a.paused != b.paused
            || a.site_id < 0
            || a.site_id as u32 != b.site_id
            || a.world_folder != b.world_folder
        {
            return Err(invalid(
                "spatial components disagree on native capture identity",
            ));
        }
        Ok(())
    }
    pub fn decode_payload(
        bytes: &[u8],
        generation: u64,
        df: String,
        dfhack: String,
    ) -> Result<Self> {
        if bytes.len() > MAX_SPATIAL_BYTES {
            return Err(exhausted("spatial capture exceeds its byte bound"));
        }
        let mut r = Reader { bytes, offset: 0 };
        if r.take(8)? != MAGIC {
            return Err(invalid("wrong spatial capture profile"));
        }
        let n = r.length()?;
        let operations = LiveOperationsObservation::decode_profile(
            r.take(n)?,
            generation,
            df.clone(),
            dfhack.clone(),
            OperationsProfile::PagedV1_4,
        )?;
        let n = r.length()?;
        if n > MAX_MAP_BYTES {
            return Err(exhausted("spatial terrain exceeds its byte bound"));
        }
        let terrain = LiveMapObservation::decode_payload(r.take(n)?, generation, df, dfhack)?;
        if r.offset != bytes.len() {
            return Err(invalid("trailing spatial capture bytes"));
        }
        let value = Self {
            operations,
            terrain,
        };
        value.validate()?;
        Ok(value)
    }
    pub fn encode_payload(&self) -> Result<Vec<u8>> {
        self.validate()?;
        let operations = self
            .operations
            .encode_profile(OperationsProfile::PagedV1_4)?;
        let terrain = self.terrain.encode_payload()?;
        let size = operations
            .len()
            .checked_add(terrain.len())
            .and_then(|n| n.checked_add(16))
            .ok_or_else(|| exhausted("spatial capture length overflow"))?;
        if size > MAX_SPATIAL_BYTES {
            return Err(exhausted("spatial capture exceeds its byte bound"));
        }
        let mut bytes = Vec::with_capacity(size);
        bytes.extend_from_slice(MAGIC);
        for part in [&operations, &terrain] {
            bytes.extend_from_slice(&(part.len() as u32).to_be_bytes());
            bytes.extend_from_slice(part);
        }
        Ok(bytes)
    }
    pub fn source_digest(&self) -> Result<Digest32> {
        let mut bytes = b"dfmcp-spatial-source-1.6\0".to_vec();
        bytes.extend_from_slice(&self.terrain.bridge_generation.to_be_bytes());
        for text in [&self.terrain.df_version, &self.terrain.dfhack_version] {
            bytes.extend_from_slice(&(text.len() as u32).to_be_bytes());
            bytes.extend_from_slice(text.as_bytes());
        }
        bytes.extend_from_slice(&self.encode_payload()?);
        Ok(Digest32::of_bytes(&bytes))
    }
}
struct Reader<'a> {
    bytes: &'a [u8],
    offset: usize,
}
impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self
            .offset
            .checked_add(n)
            .ok_or_else(|| invalid("spatial length overflow"))?;
        let part = self
            .bytes
            .get(self.offset..end)
            .ok_or_else(|| invalid("truncated spatial capture"))?;
        self.offset = end;
        Ok(part)
    }
    fn length(&mut self) -> Result<usize> {
        Ok(u32::from_be_bytes(
            self.take(4)?
                .try_into()
                .map_err(|_| invalid("invalid spatial length"))?,
        ) as usize)
    }
}

pub trait SpatialStateView {
    fn snapshot(&self) -> Option<&WorldSnapshot>;
    fn spatial_observation(&self) -> Option<&LiveSpatialObservation>;
    fn source_digest(&self) -> Result<Digest32>;
}

#[derive(Clone, Debug, Default)]
pub struct LiveSpatialState {
    observation: Option<LiveSpatialObservation>,
    snapshot: Option<WorldSnapshot>,
    generations: BTreeMap<EntityId, u32>,
}
impl LiveSpatialState {
    pub fn observation(&self) -> Option<&LiveSpatialObservation> {
        self.observation.as_ref()
    }
    pub fn snapshot(&self) -> Option<&WorldSnapshot> {
        self.snapshot.as_ref()
    }
    pub fn source_digest(&self) -> Result<Digest32> {
        self.observation
            .as_ref()
            .ok_or_else(|| invalid("spatial observation absent"))?
            .source_digest()
    }
    pub fn publish(&mut self, value: LiveSpatialObservation) -> Result<JobPublication> {
        value.validate()?;
        let source = value.source_digest()?;
        let tick = value.terrain.tick();
        let mut cursor = ObservationCursor::ORIGIN;
        let mut outcome = JobPublication::Bootstrap;
        if let (Some(prior), Some(snapshot)) = (&self.observation, &self.snapshot) {
            let a = &prior.terrain;
            let b = &value.terrain;
            if a.world_folder != b.world_folder
                || a.site_id != b.site_id
                || a.df_version != b.df_version
                || a.dfhack_version != b.dfhack_version
                || a.map.region != b.map.region
            {
                return Err(DfmcpError::new(
                    ErrorCode::StaleAnchor,
                    "spatial world, region or software changed; reopen session",
                ));
            }
            if prior == &value {
                return Ok(JobPublication::Heartbeat);
            }
            let reset = a.bridge_generation != b.bridge_generation
                || b.tick() < a.tick()
                || a.map_dimensions != b.map_dimensions
                || value.operations.jobs.next_job_id < prior.operations.jobs.next_job_id
                || value.operations.next_item_id < prior.operations.next_item_id
                || value.operations.next_building_id < prior.operations.next_building_id;
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
                        .ok_or_else(|| exhausted("spatial epoch exhausted"))?,
                    sequence: 0,
                }
            } else {
                ObservationCursor {
                    epoch: snapshot.cursor.epoch,
                    sequence: snapshot
                        .cursor
                        .sequence
                        .checked_add(1)
                        .ok_or_else(|| exhausted("spatial sequence exhausted"))?,
                }
            };
        }
        // Pure component projection only: neither projector performs a bridge read.
        let mut operations = LiveOperationsState::with_profile(OperationsProfile::PagedV1_4);
        operations.publish(value.operations.clone())?;
        let mut terrain = LiveMapState::default();
        terrain.publish(value.terrain.clone())?;
        let mut graph = operations
            .snapshot()
            .ok_or_else(|| invalid("operations projection absent"))?
            .graph
            .clone();
        let map = terrain
            .snapshot()
            .ok_or_else(|| invalid("terrain projection absent"))?;
        for (id, entity) in &map.graph.entities {
            if *id == EntityId::new(1) {
                let root = graph
                    .entities
                    .get_mut(id)
                    .ok_or_else(|| invalid("spatial root absent"))?;
                for (name, fact) in &entity.fields {
                    if let Some(existing) = root.fields.get(name) {
                        if existing.value != fact.value {
                            return Err(invalid("inconsistent spatial root field"));
                        }
                    } else {
                        root.fields.insert(name.clone(), fact.clone());
                    }
                }
                root.label = "Coherent fortress operations and terrain".to_owned();
                root.fields.insert(
                    "observation_profile".to_owned(),
                    Fact::known(
                        Value::Text("spatial/1.6".to_owned()),
                        tick,
                        FactSource::DfhackField("spatial/1.6.capture".to_owned()),
                        source,
                    ),
                );
            } else if graph.entities.insert(*id, entity.clone()).is_some() {
                return Err(invalid("spatial entity namespace collision"));
            }
        }
        let mut generations = self.generations.clone();
        let revision = cursor
            .sequence
            .checked_add(1)
            .ok_or_else(|| exhausted("spatial revision exhausted"))?;
        for (id, entity) in &mut graph.entities {
            let present = self
                .snapshot
                .as_ref()
                .is_some_and(|s| s.graph.entities.contains_key(id));
            let generation = match generations.get(id).copied() {
                Some(n) if !present || outcome == JobPublication::Reset => n
                    .checked_add(1)
                    .ok_or_else(|| exhausted("spatial entity generation exhausted"))?,
                Some(n) => n,
                None => 1,
            };
            if !generations.contains_key(id) && generations.len() >= MAX_IDENTITIES {
                return Err(exhausted("spatial identity history full; reopen session"));
            }
            generations.insert(*id, generation);
            entity.generation = generation;
            entity.revision = revision;
            for fact in entity.fields.values_mut() {
                rebind(fact, source);
            }
        }
        for edge in graph.edges.values_mut() {
            edge.revision = revision;
            for fact in edge.fields.values_mut() {
                rebind(fact, source);
            }
        }
        let snapshot = WorldSnapshot::new(
            value.terrain.fortress_id()?,
            tick,
            cursor,
            value.terrain.paused,
            graph,
        );
        self.generations = generations;
        self.observation = Some(value);
        self.snapshot = Some(snapshot);
        Ok(outcome)
    }
}
impl SpatialStateView for LiveSpatialState {
    fn snapshot(&self) -> Option<&WorldSnapshot> {
        LiveSpatialState::snapshot(self)
    }
    fn spatial_observation(&self) -> Option<&LiveSpatialObservation> {
        LiveSpatialState::observation(self)
    }
    fn source_digest(&self) -> Result<Digest32> {
        LiveSpatialState::source_digest(self)
    }
}
fn rebind(fact: &mut Fact, source: Digest32) {
    fact.source_digest = source;
    if let FactSource::DfhackField(name) = &mut fact.source {
        if let Some(suffix) = name.strip_prefix("operations/1.4.") {
            *name = format!("spatial/1.6.operations.{suffix}");
        } else if let Some(suffix) = name.strip_prefix("map/1.5.") {
            *name = format!("spatial/1.6.map.{suffix}");
        }
    }
}

#[cfg(test)]
#[path = "live_spatial_tests.rs"]
mod tests;
