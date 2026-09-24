#![forbid(unsafe_code)]

//! One suspended native read of jobs, buildings, items, and their observed links.
//! The embedded jobs encoding is reused as a data format, never fetched from an
//! independent server. Inventory membership does not establish usable supply.

use crate::live_jobs::{JobPublication, LiveJobObservation, LiveJobsState, MAX_JOB_FRAME_BYTES};
use dfmcp_core::{DfmcpError, Digest32, EdgeId, EntityId, ErrorCode, ObservationCursor, Result};
use dfmcp_world::{
    EdgeKind, EdgeRecord, EntityKind, EntityRecord, Fact, FactPresence, FactSource, Value,
    WorldSnapshot,
};
use std::collections::{BTreeMap, BTreeSet};
#[path = "operations_profile.rs"]
mod profile;
pub use profile::OperationsProfile;

pub const MAX_BUILDINGS: usize = 4096;
pub const MAX_ITEMS: usize = 32768;
pub const MAX_ATTACHMENTS: usize = 65536;
pub const MAX_OPERATIONS_BYTES: usize = MAX_JOB_FRAME_BYTES;
const MAX_IDENTITIES: usize = 131072;
const BUILDING_NAMESPACE: u64 = 1 << 40;
const ITEM_NAMESPACE: u64 = 2 << 40;
const ITEM_FLAGS: u32 = 0x1ff;

fn invalid(text: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::AdapterRejected, text)
}
fn exhausted(text: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::BudgetExceeded, text)
}
fn text_valid(text: &str) -> bool {
    !text.is_empty() && text.len() <= 128 && !text.contains('\0')
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveBuilding {
    pub native_id: u32,
    pub building_type: i32,
    pub type_key: String,
    pub x1: i32,
    pub y1: i32,
    pub x2: i32,
    pub y2: i32,
    pub z: i32,
    pub build_stage: i32,
    pub max_build_stage: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveItem {
    pub native_id: u32,
    pub item_type: i32,
    pub type_key: String,
    pub subtype: i32,
    pub material_type: i32,
    pub material_index: i32,
    pub stack_size: u32,
    /// Stored item.pos, not a recursively resolved world or accessible position.
    pub raw_position: dfmcp_core::MapCoord,
    /// forbid, in_job, dump, removed, rotten, trader, on_ground, in_inventory, in_building.
    pub flags: u32,
    pub container_native_id: Option<u32>,
    pub holder_building_native_id: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct JobItemAttachment {
    pub job_native_id: u32,
    pub item_native_id: u32,
    pub role: i32,
    pub filter_index: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveOperationsObservation {
    pub jobs: LiveJobObservation,
    pub next_building_id: u32,
    pub next_item_id: u32,
    pub buildings: Vec<LiveBuilding>,
    pub items: Vec<LiveItem>,
    pub attachments: Vec<JobItemAttachment>,
}

impl LiveOperationsObservation {
    pub fn validate(&self) -> Result<()> {
        self.validate_profile(OperationsProfile::V1_3)
    }

    pub fn validate_profile(&self, profile: OperationsProfile) -> Result<()> {
        self.jobs.validate()?;
        if self.buildings.len() > MAX_BUILDINGS
            || self.items.len() > profile.maximum_items()
            || self.attachments.len() > MAX_ATTACHMENTS
        {
            return Err(exhausted("operations roster exceeds its count bounds"));
        }
        let building_ids = validate_ids(
            self.buildings.iter().map(|v| v.native_id),
            self.next_building_id,
        )?;
        let item_ids = validate_ids(self.items.iter().map(|v| v.native_id), self.next_item_id)?;
        for building in &self.buildings {
            if building.building_type < 0
                || !text_valid(&building.type_key)
                || building.x1 > building.x2
                || building.y1 > building.y2
                || building.build_stage < 0
                || building.max_build_stage < building.build_stage
            {
                return Err(invalid(
                    "invalid building kind, bounds, or construction stage",
                ));
            }
        }
        let mut containers = BTreeMap::new();
        for item in &self.items {
            if item.item_type < 0
                || !text_valid(&item.type_key)
                || item.subtype < -1
                || item.material_type < -1
                || item.material_index < -1
                || item.stack_size > i32::MAX as u32
                || item.flags & !ITEM_FLAGS != 0
            {
                return Err(invalid(
                    "invalid item type, material, stack count, or flags",
                ));
            }
            if let Some(parent) = item.container_native_id {
                if !item_ids.contains(&parent) {
                    return Err(invalid(
                        "item container is outside the same complete item roster",
                    ));
                }
                containers.insert(item.native_id, parent);
            }
            if item
                .holder_building_native_id
                .is_some_and(|id| !building_ids.contains(&id))
            {
                return Err(invalid(
                    "item building holder is outside the same complete building roster",
                ));
            }
        }
        // Iterative functional-graph walk, linear vertex visits and bounded stack.
        let mut colors = BTreeMap::new();
        for id in &item_ids {
            let mut current = Some(*id);
            let mut chain = Vec::new();
            while let Some(node) = current {
                match colors.get(&node) {
                    Some(1) => return Err(invalid("cyclic item containment")),
                    Some(2) => break,
                    _ => {}
                }
                colors.insert(node, 1u8);
                chain.push(node);
                current = containers.get(&node).copied();
            }
            for node in chain {
                colors.insert(node, 2u8);
            }
        }
        let jobs: BTreeMap<_, _> = self
            .jobs
            .jobs
            .iter()
            .map(|job| (job.native_id, job))
            .collect();
        let mut counts = BTreeMap::<u32, u32>::new();
        let mut previous = None;
        for attachment in &self.attachments {
            let job = jobs
                .get(&attachment.job_native_id)
                .ok_or_else(|| invalid("attachment has no observed job"))?;
            if !item_ids.contains(&attachment.item_native_id)
                || attachment.role < 0
                || attachment.filter_index < -1
                || (attachment.filter_index >= 0
                    && attachment.filter_index as u32 >= job.required_item_filter_count)
                || previous.is_some_and(|prior| prior >= attachment)
            {
                return Err(invalid(
                    "invalid, dangling, duplicated, or unordered job-item attachment",
                ));
            }
            *counts.entry(attachment.job_native_id).or_default() += 1;
            previous = Some(attachment);
        }
        for job in &self.jobs.jobs {
            if job
                .holder_native_id
                .is_some_and(|id| !building_ids.contains(&id))
            {
                return Err(invalid(
                    "job holder is outside the same complete building roster",
                ));
            }
            if counts.get(&job.native_id).copied().unwrap_or(0) != job.attached_item_count {
                return Err(invalid(
                    "job attachment count disagrees with the complete attachment roster",
                ));
            }
        }
        Ok(())
    }

    pub fn encode_payload(&self) -> Result<Vec<u8>> {
        self.encode_profile(OperationsProfile::V1_3)
    }

    pub fn encode_profile(&self, profile: OperationsProfile) -> Result<Vec<u8>> {
        self.validate_profile(profile)?;
        let jobs = self.jobs.encode_payload()?;
        let mut out = profile.magic().to_vec();
        put(&mut out, jobs.len() as u32);
        out.extend_from_slice(&jobs);
        put(&mut out, self.next_building_id);
        put(&mut out, self.next_item_id);
        put(&mut out, self.buildings.len() as u32);
        for v in &self.buildings {
            put(&mut out, v.native_id);
            signed(&mut out, v.building_type);
            text(&mut out, &v.type_key);
            for n in [
                v.x1,
                v.y1,
                v.x2,
                v.y2,
                v.z,
                v.build_stage,
                v.max_build_stage,
            ] {
                signed(&mut out, n);
            }
        }
        put(&mut out, self.items.len() as u32);
        for v in &self.items {
            put(&mut out, v.native_id);
            signed(&mut out, v.item_type);
            text(&mut out, &v.type_key);
            for n in [v.subtype, v.material_type, v.material_index] {
                signed(&mut out, n);
            }
            put(&mut out, v.stack_size);
            for n in [v.raw_position.x, v.raw_position.y, v.raw_position.z] {
                signed(&mut out, n);
            }
            put(&mut out, v.flags);
            reference(&mut out, v.container_native_id);
            reference(&mut out, v.holder_building_native_id);
        }
        put(&mut out, self.attachments.len() as u32);
        for v in &self.attachments {
            put(&mut out, v.job_native_id);
            put(&mut out, v.item_native_id);
            signed(&mut out, v.role);
            signed(&mut out, v.filter_index);
        }
        if out.len() > profile.maximum_bytes() {
            return Err(exhausted("operations payload exceeds its byte bound"));
        }
        Ok(out)
    }

    pub fn decode_payload(
        bytes: &[u8],
        generation: u64,
        df: String,
        dfhack: String,
    ) -> Result<Self> {
        Self::decode_profile(bytes, generation, df, dfhack, OperationsProfile::V1_3)
    }

    pub fn decode_profile(
        bytes: &[u8],
        generation: u64,
        df: String,
        dfhack: String,
        profile: OperationsProfile,
    ) -> Result<Self> {
        if bytes.len() > profile.maximum_bytes() {
            return Err(exhausted("operations payload exceeds its byte bound"));
        }
        let mut r = Reader { bytes, offset: 0 };
        if r.take(8)? != profile.magic() {
            return Err(invalid("unsupported operations payload schema"));
        }
        let length = r.number()? as usize;
        let jobs = LiveJobObservation::decode_payload(r.take(length)?, generation, df, dfhack)?;
        let next_building_id = r.number()?;
        let next_item_id = r.number()?;
        let count = r.count(MAX_BUILDINGS)?;
        let mut buildings = Vec::with_capacity(count);
        for _ in 0..count {
            buildings.push(LiveBuilding {
                native_id: r.number()?,
                building_type: r.signed()?,
                type_key: r.text()?,
                x1: r.signed()?,
                y1: r.signed()?,
                x2: r.signed()?,
                y2: r.signed()?,
                z: r.signed()?,
                build_stage: r.signed()?,
                max_build_stage: r.signed()?,
            });
        }
        let count = r.count(profile.maximum_items())?;
        let mut items = Vec::with_capacity(count);
        for _ in 0..count {
            items.push(LiveItem {
                native_id: r.number()?,
                item_type: r.signed()?,
                type_key: r.text()?,
                subtype: r.signed()?,
                material_type: r.signed()?,
                material_index: r.signed()?,
                stack_size: r.number()?,
                raw_position: dfmcp_core::MapCoord::new(r.signed()?, r.signed()?, r.signed()?),
                flags: r.number()?,
                container_native_id: r.reference()?,
                holder_building_native_id: r.reference()?,
            });
        }
        let count = r.count(MAX_ATTACHMENTS)?;
        let mut attachments = Vec::with_capacity(count);
        for _ in 0..count {
            attachments.push(JobItemAttachment {
                job_native_id: r.number()?,
                item_native_id: r.number()?,
                role: r.signed()?,
                filter_index: r.signed()?,
            });
        }
        if r.offset != bytes.len() {
            return Err(invalid("trailing operations payload bytes"));
        }
        let value = Self {
            jobs,
            next_building_id,
            next_item_id,
            buildings,
            items,
            attachments,
        };
        value.validate_profile(profile)?;
        Ok(value)
    }

    pub fn source_digest(&self) -> Result<Digest32> {
        self.source_digest_profile(OperationsProfile::V1_3)
    }

    pub fn source_digest_profile(&self, profile: OperationsProfile) -> Result<Digest32> {
        let mut bytes = profile.source_domain().to_vec();
        bytes.extend_from_slice(self.jobs.source_digest()?.as_bytes());
        bytes.extend_from_slice(&self.encode_profile(profile)?);
        Ok(Digest32::of_bytes(&bytes))
    }
}

fn validate_ids(ids: impl Iterator<Item = u32>, horizon: u32) -> Result<BTreeSet<u32>> {
    if horizon > i32::MAX as u32 {
        return Err(invalid("native identity horizon exceeds i32"));
    }
    let mut previous = None;
    let mut result = BTreeSet::new();
    for id in ids {
        if id >= horizon || previous.is_some_and(|prior| prior >= id) {
            return Err(invalid(
                "native roster is not strictly ordered below its identity horizon",
            ));
        }
        previous = Some(id);
        result.insert(id);
    }
    Ok(result)
}
fn put(out: &mut Vec<u8>, n: u32) {
    out.extend_from_slice(&n.to_be_bytes());
}
fn signed(out: &mut Vec<u8>, n: i32) {
    out.extend_from_slice(&n.to_be_bytes());
}
fn text(out: &mut Vec<u8>, value: &str) {
    out.extend_from_slice(&(value.len() as u16).to_be_bytes());
    out.extend_from_slice(value.as_bytes());
}
fn reference(out: &mut Vec<u8>, value: Option<u32>) {
    out.push(u8::from(value.is_some()));
    if let Some(id) = value {
        put(out, id);
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
            .ok_or_else(|| invalid("operations length overflow"))?;
        let result = self
            .bytes
            .get(self.offset..end)
            .ok_or_else(|| invalid("truncated operations payload"))?;
        self.offset = end;
        Ok(result)
    }
    fn number(&mut self) -> Result<u32> {
        Ok(u32::from_be_bytes(
            self.take(4)?
                .try_into()
                .map_err(|_| invalid("invalid u32"))?,
        ))
    }
    fn signed(&mut self) -> Result<i32> {
        Ok(self.number()? as i32)
    }
    fn count(&mut self, max: usize) -> Result<usize> {
        let n = self.number()? as usize;
        if n > max {
            return Err(exhausted("operations array count exceeds bound"));
        }
        Ok(n)
    }
    fn text(&mut self) -> Result<String> {
        let n = u16::from_be_bytes(
            self.take(2)?
                .try_into()
                .map_err(|_| invalid("invalid text size"))?,
        ) as usize;
        if n > 128 {
            return Err(exhausted("operations text exceeds bound"));
        }
        String::from_utf8(self.take(n)?.to_vec()).map_err(|_| invalid("invalid operations UTF-8"))
    }
    fn reference(&mut self) -> Result<Option<u32>> {
        match self.take(1)?[0] {
            0 => Ok(None),
            1 => Ok(Some(self.number()?)),
            _ => Err(invalid("noncanonical reference tag")),
        }
    }
}

#[must_use]
pub fn building_entity_id(id: u32) -> EntityId {
    EntityId::new(BUILDING_NAMESPACE + u64::from(id))
}
#[must_use]
pub fn item_entity_id(id: u32) -> EntityId {
    EntityId::new(ITEM_NAMESPACE + u64::from(id))
}

#[derive(Clone, Debug, Default)]
pub struct LiveOperationsState {
    profile: OperationsProfile,
    observation: Option<LiveOperationsObservation>,
    snapshot: Option<WorldSnapshot>,
    generations: BTreeMap<EntityId, u32>,
}
impl LiveOperationsState {
    pub fn with_profile(profile: OperationsProfile) -> Self {
        Self {
            profile,
            ..Self::default()
        }
    }
    pub fn profile(&self) -> OperationsProfile {
        self.profile
    }
    pub fn source_digest(&self) -> Result<Digest32> {
        self.observation
            .as_ref()
            .ok_or_else(|| invalid("operations source absent"))?
            .source_digest_profile(self.profile)
    }
    #[must_use]
    pub fn snapshot(&self) -> Option<&WorldSnapshot> {
        self.snapshot.as_ref()
    }
    #[must_use]
    pub fn observation(&self) -> Option<&LiveOperationsObservation> {
        self.observation.as_ref()
    }

    pub fn publish(&mut self, observation: LiveOperationsObservation) -> Result<JobPublication> {
        observation.validate_profile(self.profile)?;
        let source = observation.source_digest_profile(self.profile)?;
        let fortress = observation.jobs.fortress_id()?;
        let tick = observation.jobs.tick();
        let mut cursor = ObservationCursor::ORIGIN;
        let mut outcome = JobPublication::Bootstrap;
        if let (Some(prior), Some(snapshot)) = (&self.observation, &self.snapshot) {
            if observation.jobs.world_folder != prior.jobs.world_folder
                || observation.jobs.site_id != prior.jobs.site_id
                || observation.jobs.df_version != prior.jobs.df_version
                || observation.jobs.dfhack_version != prior.jobs.dfhack_version
            {
                return Err(DfmcpError::new(
                    ErrorCode::StaleAnchor,
                    "operations source changed world, site, or software; reopen session",
                ));
            }
            if &observation == prior {
                return Ok(JobPublication::Heartbeat);
            }
            let reset = observation.jobs.bridge_generation != prior.jobs.bridge_generation
                || tick < prior.jobs.tick()
                || observation.jobs.next_job_id < prior.jobs.next_job_id
                || observation.next_building_id < prior.next_building_id
                || observation.next_item_id < prior.next_item_id;
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
                        .ok_or_else(|| exhausted("operations epoch exhausted"))?,
                    sequence: 0,
                }
            } else {
                ObservationCursor {
                    epoch: snapshot.cursor.epoch,
                    sequence: snapshot
                        .cursor
                        .sequence
                        .checked_add(1)
                        .ok_or_else(|| exhausted("operations sequence exhausted"))?,
                }
            };
        }
        // Reuse the jobs projection, not an independently observed job snapshot.
        let mut jobs = LiveJobsState::default();
        jobs.publish(observation.jobs.clone())?;
        let mut graph = jobs
            .snapshot()
            .ok_or_else(|| invalid("jobs projection absent"))?
            .graph
            .clone();
        let prefix = self.profile.fact_prefix();
        let fact = |field: &str, value| {
            Fact::known(
                value,
                tick,
                FactSource::DfhackField(format!("{prefix}{field}")),
                source,
            )
        };
        let absent = |field: &str| {
            Fact::with_presence(
                FactPresence::Absent,
                tick,
                FactSource::DfhackField(format!("{prefix}{field}")),
                source,
            )
        };
        if let Some(root) = graph.entities.get_mut(&EntityId::new(1)) {
            root.label = "Fortress operations".to_owned();
            root.fields.insert(
                "building_count".to_owned(),
                fact(
                    "buildings.all.size",
                    Value::U64(observation.buildings.len() as u64),
                ),
            );
            root.fields.insert(
                "item_count".to_owned(),
                fact("items.all.size", Value::U64(observation.items.len() as u64)),
            );
        }
        for v in &observation.buildings {
            let id = building_entity_id(v.native_id);
            graph.entities.insert(
                id,
                EntityRecord {
                    id,
                    generation: 1,
                    revision: 1,
                    kind: EntityKind::Building,
                    label: format!("{} #{}", v.type_key, v.native_id),
                    fields: BTreeMap::from([
                        (
                            "native_building_id".to_owned(),
                            fact("building.id", Value::U64(u64::from(v.native_id))),
                        ),
                        (
                            "building_type".to_owned(),
                            fact("building.getType", Value::I64(i64::from(v.building_type))),
                        ),
                        (
                            "type_key".to_owned(),
                            fact("building.type_key", Value::Text(v.type_key.clone())),
                        ),
                        (
                            "min_corner".to_owned(),
                            fact(
                                "building.bounds",
                                Value::Coord(dfmcp_core::MapCoord::new(v.x1, v.y1, v.z)),
                            ),
                        ),
                        (
                            "max_corner".to_owned(),
                            fact(
                                "building.bounds",
                                Value::Coord(dfmcp_core::MapCoord::new(v.x2, v.y2, v.z)),
                            ),
                        ),
                        (
                            "build_stage".to_owned(),
                            fact(
                                "building.getBuildStage",
                                Value::I64(i64::from(v.build_stage)),
                            ),
                        ),
                        (
                            "max_build_stage".to_owned(),
                            fact(
                                "building.getMaxBuildStage",
                                Value::I64(i64::from(v.max_build_stage)),
                            ),
                        ),
                    ]),
                },
            );
        }
        for v in &observation.items {
            let id = item_entity_id(v.native_id);
            let mut fields = BTreeMap::from([
                (
                    "native_item_id".to_owned(),
                    fact("item.id", Value::U64(u64::from(v.native_id))),
                ),
                (
                    "item_type".to_owned(),
                    fact("item.getType", Value::I64(i64::from(v.item_type))),
                ),
                (
                    "type_key".to_owned(),
                    fact("item.type_key", Value::Text(v.type_key.clone())),
                ),
                (
                    "subtype".to_owned(),
                    fact("item.getSubtype", Value::I64(i64::from(v.subtype))),
                ),
                (
                    "material_type".to_owned(),
                    fact("item.getMaterial", Value::I64(i64::from(v.material_type))),
                ),
                (
                    "material_index".to_owned(),
                    fact(
                        "item.getMaterialIndex",
                        Value::I64(i64::from(v.material_index)),
                    ),
                ),
                (
                    "stack_size".to_owned(),
                    fact("item.getStackSize", Value::U64(u64::from(v.stack_size))),
                ),
                (
                    "raw_position".to_owned(),
                    fact("item.pos", Value::Coord(v.raw_position)),
                ),
            ]);
            for (bit, name) in [
                "forbidden",
                "in_job",
                "dump",
                "removed",
                "rotten",
                "trader",
                "on_ground",
                "in_inventory",
                "in_building",
            ]
            .iter()
            .enumerate()
            {
                fields.insert(
                    (*name).to_owned(),
                    fact(name, Value::Bool(v.flags & (1 << bit) != 0)),
                );
            }
            fields.insert(
                "container".to_owned(),
                v.container_native_id.map_or_else(
                    || absent("Items.getContainer"),
                    |parent| fact("Items.getContainer", Value::Entity(item_entity_id(parent))),
                ),
            );
            fields.insert(
                "holder_building".to_owned(),
                v.holder_building_native_id.map_or_else(
                    || absent("Items.getHolderBuilding"),
                    |parent| {
                        fact(
                            "Items.getHolderBuilding",
                            Value::Entity(building_entity_id(parent)),
                        )
                    },
                ),
            );
            graph.entities.insert(
                id,
                EntityRecord {
                    id,
                    generation: 1,
                    revision: 1,
                    kind: EntityKind::Item,
                    label: format!("{} #{}", v.type_key, v.native_id),
                    fields,
                },
            );
        }
        let mut add_edge = |kind: EdgeKind,
                            from: EntityId,
                            to: EntityId,
                            extra: Option<(i32, i32)>,
                            fields: BTreeMap<String, Fact>|
         -> Result<()> {
            // Stable relationship keys are shared across profiles; source facts
            // and the full snapshot anchor still bind the exact profile.
            let mut identity = b"dfmcp-operations-edge-1.3\0".to_vec();
            text(&mut identity, kind.as_str());
            identity.extend_from_slice(&from.get().to_be_bytes());
            identity.extend_from_slice(&to.get().to_be_bytes());
            identity.push(u8::from(extra.is_some()));
            if let Some((role, index)) = extra {
                signed(&mut identity, role);
                signed(&mut identity, index);
            }
            let digest = Digest32::of_bytes(&identity);
            let raw: [u8; 16] = digest.as_bytes()[..16]
                .try_into()
                .map_err(|_| invalid("invalid edge digest"))?;
            let id = EdgeId::new(u128::from_be_bytes(raw) | 1);
            if graph
                .edges
                .insert(
                    id,
                    EdgeRecord {
                        id,
                        revision: 1,
                        kind,
                        from,
                        to,
                        fields,
                    },
                )
                .is_some()
            {
                return Err(invalid("operations edge identity collision"));
            }
            Ok(())
        };
        for job in &observation.jobs.jobs {
            if let Some(holder) = job.holder_native_id {
                add_edge(
                    EdgeKind::ContainedIn,
                    EntityId::new(u64::from(job.native_id) + 2),
                    building_entity_id(holder),
                    None,
                    BTreeMap::from([(
                        "relation".to_owned(),
                        fact("Job.getHolder", Value::Text("job_holder".to_owned())),
                    )]),
                )?;
            }
        }
        for item in &observation.items {
            for (parent, relation) in [
                (
                    item.container_native_id.map(item_entity_id),
                    "Items.getContainer",
                ),
                (
                    item.holder_building_native_id.map(building_entity_id),
                    "Items.getHolderBuilding",
                ),
            ] {
                if let Some(parent) = parent {
                    add_edge(
                        EdgeKind::ContainedIn,
                        item_entity_id(item.native_id),
                        parent,
                        None,
                        BTreeMap::from([(
                            "relation".to_owned(),
                            fact(relation, Value::Text(relation.to_owned())),
                        )]),
                    )?;
                }
            }
        }
        for a in &observation.attachments {
            add_edge(
                EdgeKind::Uses,
                EntityId::new(u64::from(a.job_native_id) + 2),
                item_entity_id(a.item_native_id),
                Some((a.role, a.filter_index)),
                BTreeMap::from([
                    (
                        "role".to_owned(),
                        fact("job.items.role", Value::I64(i64::from(a.role))),
                    ),
                    (
                        "filter_index".to_owned(),
                        fact(
                            "job.items.job_item_idx",
                            Value::I64(i64::from(a.filter_index)),
                        ),
                    ),
                ]),
            )?;
        }
        let mut generations = self.generations.clone();
        let revision = cursor
            .sequence
            .checked_add(1)
            .ok_or_else(|| exhausted("operations revision exhausted"))?;
        for (id, entity) in &mut graph.entities {
            let present = self
                .snapshot
                .as_ref()
                .is_some_and(|snapshot| snapshot.graph.entities.contains_key(id));
            let generation = match generations.get(id).copied() {
                Some(n) if !present || outcome == JobPublication::Reset => n
                    .checked_add(1)
                    .ok_or_else(|| exhausted("operations generation exhausted"))?,
                Some(n) => n,
                None => 1,
            };
            if !generations.contains_key(id) && generations.len() >= MAX_IDENTITIES {
                return Err(exhausted(
                    "operations identity history full; reopen session",
                ));
            }
            generations.insert(*id, generation);
            entity.generation = generation;
            entity.revision = revision;
            for value in entity.fields.values_mut() {
                value.source_digest = source;
                if let FactSource::DfhackField(name) = &mut value.source {
                    if name.starts_with("jobs/1.2.") {
                        *name = name.replacen("jobs/1.2.", prefix, 1);
                    }
                }
            }
        }
        for edge in graph.edges.values_mut() {
            edge.revision = revision;
        }
        let snapshot = WorldSnapshot::new(fortress, tick, cursor, observation.jobs.paused, graph);
        self.observation = Some(observation);
        self.snapshot = Some(snapshot);
        self.generations = generations;
        Ok(outcome)
    }
}
