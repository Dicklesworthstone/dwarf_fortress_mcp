use std::collections::BTreeMap;

use dfmcp_core::{
    Digest32, EdgeId, EntityId, EventId, FortressId, GameTick, MapCoord, ObservationCursor,
    StateAnchor,
};

use crate::canonical::{
    put_anchor, put_bool, put_bytes, put_i32, put_i64, put_str, put_u16, put_u32, put_u64,
};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EntityKind {
    Fortress,
    Unit,
    Item,
    Building,
    Job,
    WorkOrder,
    Stockpile,
    Zone,
    Burrow,
    Squad,
    MilitaryOrder,
    TileFeature,
    Plant,
    Creature,
    HistoricalFigure,
    Civilization,
    Announcement,
    Syndrome,
    Other(String),
}

impl EntityKind {
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Fortress => "fortress",
            Self::Unit => "unit",
            Self::Item => "item",
            Self::Building => "building",
            Self::Job => "job",
            Self::WorkOrder => "work_order",
            Self::Stockpile => "stockpile",
            Self::Zone => "zone",
            Self::Burrow => "burrow",
            Self::Squad => "squad",
            Self::MilitaryOrder => "military_order",
            Self::TileFeature => "tile_feature",
            Self::Plant => "plant",
            Self::Creature => "creature",
            Self::HistoricalFigure => "historical_figure",
            Self::Civilization => "civilization",
            Self::Announcement => "announcement",
            Self::Syndrome => "syndrome",
            Self::Other(value) => value,
        }
    }

    pub(crate) fn encode(&self, output: &mut Vec<u8>) {
        put_str(output, self.as_str());
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Value {
    Null,
    Bool(bool),
    I64(i64),
    U64(u64),
    Fixed { units: i64, scale: u32 },
    Text(String),
    Entity(EntityId),
    Coord(MapCoord),
    Bytes(Vec<u8>),
    List(Vec<Value>),
    Object(BTreeMap<String, Value>),
}

impl Value {
    pub(crate) fn encode(&self, output: &mut Vec<u8>) {
        match self {
            Self::Null => output.push(0),
            Self::Bool(value) => {
                output.push(1);
                put_bool(output, *value);
            }
            Self::I64(value) => {
                output.push(2);
                put_i64(output, *value);
            }
            Self::U64(value) => {
                output.push(3);
                put_u64(output, *value);
            }
            Self::Fixed { units, scale } => {
                output.push(4);
                put_i64(output, *units);
                put_u32(output, *scale);
            }
            Self::Text(value) => {
                output.push(5);
                put_str(output, value);
            }
            Self::Entity(value) => {
                output.push(6);
                put_u64(output, value.get());
            }
            Self::Coord(value) => {
                output.push(7);
                put_i32(output, value.x);
                put_i32(output, value.y);
                put_i32(output, value.z);
            }
            Self::Bytes(value) => {
                output.push(8);
                put_bytes(output, value);
            }
            Self::List(values) => {
                output.push(9);
                put_u64(output, values.len() as u64);
                for value in values {
                    value.encode(output);
                }
            }
            Self::Object(values) => {
                output.push(10);
                put_u64(output, values.len() as u64);
                for (key, value) in values {
                    put_str(output, key);
                    value.encode(output);
                }
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FactPresence {
    Known(Value),
    Absent,
    Unknown(String),
    Unsupported(String),
    Omitted(String),
    Redacted(String),
    Stale(StateAnchor),
}

impl FactPresence {
    #[must_use]
    pub fn is_known(&self) -> bool {
        matches!(self, Self::Known(_))
    }

    #[must_use]
    pub fn as_known(&self) -> Option<&Value> {
        match self {
            Self::Known(v) => Some(v),
            _ => None,
        }
    }

    #[must_use]
    pub fn is_absent(&self) -> bool {
        matches!(self, Self::Absent)
    }

    #[must_use]
    pub fn is_stale(&self) -> bool {
        matches!(self, Self::Stale(_))
    }

    pub(crate) fn encode(&self, output: &mut Vec<u8>) {
        match self {
            Self::Known(value) => {
                output.push(0);
                value.encode(output);
            }
            Self::Absent => output.push(1),
            Self::Unknown(reason) => {
                output.push(2);
                put_str(output, reason);
            }
            Self::Unsupported(reason) => {
                output.push(3);
                put_str(output, reason);
            }
            Self::Omitted(reason) => {
                output.push(4);
                put_str(output, reason);
            }
            Self::Redacted(reason) => {
                output.push(5);
                put_str(output, reason);
            }
            Self::Stale(anchor) => {
                output.push(6);
                put_anchor(output, *anchor);
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FactSource {
    DfhackField(String),
    Derived(String),
    AgentAssertion(String),
    Replay,
}

impl FactSource {
    pub(crate) fn encode(&self, output: &mut Vec<u8>) {
        match self {
            Self::DfhackField(value) => {
                output.push(0);
                put_str(output, value);
            }
            Self::Derived(value) => {
                output.push(1);
                put_str(output, value);
            }
            Self::AgentAssertion(value) => {
                output.push(2);
                put_str(output, value);
            }
            Self::Replay => output.push(3),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Fact {
    pub value: Value,
    pub observed_at: GameTick,
    pub source: FactSource,
    pub source_digest: Digest32,
    pub presence: Option<FactPresence>,
}

impl Fact {
    #[must_use]
    pub fn known(
        value: Value,
        observed_at: GameTick,
        source: FactSource,
        source_digest: Digest32,
    ) -> Self {
        Self {
            value,
            observed_at,
            source,
            source_digest,
            presence: None,
        }
    }

    #[must_use]
    pub fn with_presence(
        presence: FactPresence,
        observed_at: GameTick,
        source: FactSource,
        source_digest: Digest32,
    ) -> Self {
        let value = presence.as_known().cloned().unwrap_or(Value::Null);
        Self {
            value,
            observed_at,
            source,
            source_digest,
            presence: Some(presence),
        }
    }

    #[must_use]
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut output = Vec::new();
        self.encode(&mut output);
        output
    }

    pub(crate) fn encode(&self, output: &mut Vec<u8>) {
        self.value.encode(output);
        put_u64(output, self.observed_at.0);
        self.source.encode(output);
        put_bytes(output, self.source_digest.as_bytes());
        match &self.presence {
            Some(presence) => {
                output.push(1);
                presence.encode(output);
            }
            None => output.push(0),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntityRecord {
    pub id: EntityId,
    pub generation: u32,
    pub revision: u64,
    pub kind: EntityKind,
    pub label: String,
    pub fields: BTreeMap<String, Fact>,
}

impl EntityRecord {
    #[must_use]
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut output = Vec::new();
        self.encode(&mut output);
        output
    }

    pub(crate) fn encode(&self, output: &mut Vec<u8>) {
        put_u64(output, self.id.get());
        put_u32(output, self.generation);
        put_u64(output, self.revision);
        self.kind.encode(output);
        put_str(output, &self.label);
        put_u64(output, self.fields.len() as u64);
        for (key, value) in &self.fields {
            put_str(output, key);
            value.encode(output);
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EdgeKind {
    LocatedAt,
    ContainedIn,
    AssignedTo,
    MemberOf,
    Performs,
    Requires,
    Produces,
    Uses,
    Supports,
    Threatens,
    OrderedBy,
    ParentOf,
    Custom(String),
}

impl EdgeKind {
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::LocatedAt => "located_at",
            Self::ContainedIn => "contained_in",
            Self::AssignedTo => "assigned_to",
            Self::MemberOf => "member_of",
            Self::Performs => "performs",
            Self::Requires => "requires",
            Self::Produces => "produces",
            Self::Uses => "uses",
            Self::Supports => "supports",
            Self::Threatens => "threatens",
            Self::OrderedBy => "ordered_by",
            Self::ParentOf => "parent_of",
            Self::Custom(value) => value,
        }
    }

    pub(crate) fn encode(&self, output: &mut Vec<u8>) {
        put_str(output, self.as_str());
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EdgeRecord {
    pub id: EdgeId,
    pub revision: u64,
    pub kind: EdgeKind,
    pub from: EntityId,
    pub to: EntityId,
    pub fields: BTreeMap<String, Fact>,
}

impl EdgeRecord {
    pub(crate) fn encode(&self, output: &mut Vec<u8>) {
        output.extend_from_slice(&self.id.get().to_be_bytes());
        put_u64(output, self.revision);
        self.kind.encode(output);
        put_u64(output, self.from.get());
        put_u64(output, self.to.get());
        put_u64(output, self.fields.len() as u64);
        for (key, value) in &self.fields {
            put_str(output, key);
            value.encode(output);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ChunkCoord {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

impl ChunkCoord {
    pub(crate) fn encode(self, output: &mut Vec<u8>) {
        put_i32(output, self.x);
        put_i32(output, self.y);
        put_i32(output, self.z);
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerrainRun {
    pub tile_code: u32,
    pub length: u32,
}

impl TerrainRun {
    pub(crate) fn encode(&self, output: &mut Vec<u8>) {
        put_u32(output, self.tile_code);
        put_u32(output, self.length);
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MapChunk {
    pub coord: ChunkCoord,
    pub revision: u64,
    pub width: u16,
    pub height: u16,
    pub terrain_runs: Vec<TerrainRun>,
    pub sparse_overlays: BTreeMap<u32, BTreeMap<String, Value>>,
}

impl MapChunk {
    #[must_use]
    pub fn tile_count(&self) -> u32 {
        u32::from(self.width) * u32::from(self.height)
    }

    #[must_use]
    pub fn encoded_tile_count(&self) -> Option<u32> {
        self.terrain_runs
            .iter()
            .try_fold(0u32, |total, run| total.checked_add(run.length))
    }

    #[must_use]
    pub fn compute_hash(&self) -> Digest32 {
        let mut bytes = Vec::new();
        self.encode(&mut bytes);
        Digest32::of_bytes(&bytes)
    }

    pub(crate) fn encode(&self, output: &mut Vec<u8>) {
        self.coord.encode(output);
        put_u64(output, self.revision);
        put_u16(output, self.width);
        put_u16(output, self.height);
        put_u64(output, self.terrain_runs.len() as u64);
        for run in &self.terrain_runs {
            run.encode(output);
        }
        put_u64(output, self.sparse_overlays.len() as u64);
        for (offset, fields) in &self.sparse_overlays {
            put_u32(output, *offset);
            put_u64(output, fields.len() as u64);
            for (key, value) in fields {
                put_str(output, key);
                value.encode(output);
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum WorldEventKind {
    Announcement,
    JobChanged,
    UnitChanged,
    ConstructionChanged,
    ThreatDetected,
    SeasonChanged,
    AdapterNotice,
    Other(String),
}

impl WorldEventKind {
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Announcement => "announcement",
            Self::JobChanged => "job_changed",
            Self::UnitChanged => "unit_changed",
            Self::ConstructionChanged => "construction_changed",
            Self::ThreatDetected => "threat_detected",
            Self::SeasonChanged => "season_changed",
            Self::AdapterNotice => "adapter_notice",
            Self::Other(value) => value,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorldEvent {
    pub id: EventId,
    pub tick: GameTick,
    pub kind: WorldEventKind,
    pub subject: Option<EntityId>,
    pub summary: String,
    pub fields: BTreeMap<String, Value>,
}

impl WorldEvent {
    pub(crate) fn encode(&self, output: &mut Vec<u8>) {
        output.extend_from_slice(&self.id.get().to_be_bytes());
        put_u64(output, self.tick.0);
        put_str(output, self.kind.as_str());
        match self.subject {
            Some(value) => {
                output.push(1);
                put_u64(output, value.get());
            }
            None => output.push(0),
        }
        put_str(output, &self.summary);
        put_u64(output, self.fields.len() as u64);
        for (key, value) in &self.fields {
            put_str(output, key);
            value.encode(output);
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct WorldGraph {
    pub entities: BTreeMap<EntityId, EntityRecord>,
    pub edges: BTreeMap<EdgeId, EdgeRecord>,
    pub chunks: BTreeMap<ChunkCoord, MapChunk>,
    pub events: BTreeMap<EventId, WorldEvent>,
}

impl WorldGraph {
    pub(crate) fn encode(&self, output: &mut Vec<u8>) {
        put_u64(output, self.entities.len() as u64);
        for entity in self.entities.values() {
            entity.encode(output);
        }
        put_u64(output, self.edges.len() as u64);
        for edge in self.edges.values() {
            edge.encode(output);
        }
        put_u64(output, self.chunks.len() as u64);
        for chunk in self.chunks.values() {
            chunk.encode(output);
        }
        put_u64(output, self.events.len() as u64);
        for event in self.events.values() {
            event.encode(output);
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorldSnapshot {
    pub fortress_id: FortressId,
    pub tick: GameTick,
    pub cursor: ObservationCursor,
    pub paused: bool,
    pub graph: WorldGraph,
    pub state_hash: Digest32,
}

impl WorldSnapshot {
    #[must_use]
    pub fn new(
        fortress_id: FortressId,
        tick: GameTick,
        cursor: ObservationCursor,
        paused: bool,
        graph: WorldGraph,
    ) -> Self {
        let mut snapshot = Self {
            fortress_id,
            tick,
            cursor,
            paused,
            graph,
            state_hash: Digest32::ZERO,
        };
        snapshot.refresh_hash();
        snapshot
    }

    #[must_use]
    pub fn anchor(&self) -> StateAnchor {
        StateAnchor {
            fortress_id: self.fortress_id,
            cursor: self.cursor,
            tick: self.tick,
            state_hash: self.state_hash,
        }
    }

    #[must_use]
    pub fn compute_hash(&self) -> Digest32 {
        Digest32::of_bytes(&self.canonical_bytes())
    }

    /// Canonical, length-delimited bytes covered by [`Self::state_hash`].
    #[must_use]
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        put_str(&mut bytes, "dfmcp-world-snapshot-v1");
        put_u64(&mut bytes, self.fortress_id.get());
        put_u64(&mut bytes, self.tick.0);
        put_u64(&mut bytes, self.cursor.epoch);
        put_u64(&mut bytes, self.cursor.sequence);
        put_bool(&mut bytes, self.paused);
        self.graph.encode(&mut bytes);
        bytes
    }

    pub fn refresh_hash(&mut self) {
        self.state_hash = self.compute_hash();
    }

    #[must_use]
    pub fn hash_is_valid(&self) -> bool {
        self.state_hash == self.compute_hash()
    }
}
