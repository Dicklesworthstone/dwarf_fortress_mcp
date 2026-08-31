#![forbid(unsafe_code)]

//! Continuous dirty-chunk map and entity delta streaming pipeline.
//!
//! WP-DFH-03: Tracks modified 16x16x1 map blocks and entity state changes in the
//! DFHack bridge, emitting compact incremental deltas to the Rust world ledger.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use dfmcp_core::{
    DfmcpError, Digest32, EntityId, ErrorCode, FortressId, GameTick, ObservationCursor, Result,
};
use dfmcp_world::{
    ChunkCoord, EntityRecord, MapChunk, StateDelta, WorldChange, WorldEvent, WorldGraph,
    WorldSnapshot,
};

/// Maximum capacity of the bounded event ring buffer before shedding oldest non-critical events.
pub const MAX_EVENT_BUFFER_CAPACITY: usize = 10_000;

/// Tracks dirty 16x16x1 map chunks.
#[derive(Clone, Debug, Default)]
pub struct DirtyChunkTracker {
    dirty_chunks: BTreeSet<ChunkCoord>,
}

impl DirtyChunkTracker {
    #[must_use]
    pub fn new() -> Self {
        Self {
            dirty_chunks: BTreeSet::new(),
        }
    }

    /// Mark a specific chunk coordinate dirty.
    pub fn mark_dirty(&mut self, coord: ChunkCoord) {
        self.dirty_chunks.insert(coord);
    }

    /// Mark multiple chunk coordinates dirty.
    pub fn mark_all_dirty(&mut self, coords: impl IntoIterator<Item = ChunkCoord>) {
        for coord in coords {
            self.dirty_chunks.insert(coord);
        }
    }

    /// Check if a chunk is marked dirty.
    #[must_use]
    pub fn is_dirty(&self, coord: &ChunkCoord) -> bool {
        self.dirty_chunks.contains(coord)
    }

    /// Drain all dirty coordinates for delta processing.
    pub fn drain_dirty(&mut self) -> Vec<ChunkCoord> {
        std::mem::take(&mut self.dirty_chunks).into_iter().collect()
    }

    /// Number of currently dirty chunks.
    #[must_use]
    pub fn dirty_count(&self) -> usize {
        self.dirty_chunks.len()
    }

    /// Clear all dirty marks.
    pub fn clear(&mut self) {
        self.dirty_chunks.clear();
    }
}

/// Tracks entity state and detects generation and revision changes.
#[derive(Clone, Debug, Default)]
pub struct EntityDeltaTracker {
    known_entities: BTreeMap<EntityId, (u32, u64)>, // EntityId -> (generation, revision)
}

impl EntityDeltaTracker {
    #[must_use]
    pub fn new() -> Self {
        Self {
            known_entities: BTreeMap::new(),
        }
    }

    /// Seed the tracker from an existing world graph snapshot.
    pub fn seed_from_graph(&mut self, graph: &WorldGraph) {
        self.known_entities.clear();
        for (id, entity) in &graph.entities {
            self.known_entities
                .insert(*id, (entity.generation, entity.revision));
        }
    }

    /// Compare active entities against known state and generate entity changes.
    pub fn process_entities(&mut self, active_entities: &[EntityRecord]) -> Vec<WorldChange> {
        let mut changes = Vec::new();
        let mut seen_ids = BTreeSet::new();

        for entity in active_entities {
            seen_ids.insert(entity.id);
            match self.known_entities.get(&entity.id) {
                Some(&(known_gen, known_rev)) => {
                    if entity.generation != known_gen || entity.revision > known_rev {
                        self.known_entities
                            .insert(entity.id, (entity.generation, entity.revision));
                        changes.push(WorldChange::UpsertEntity(entity.clone()));
                    }
                }
                None => {
                    self.known_entities
                        .insert(entity.id, (entity.generation, entity.revision));
                    changes.push(WorldChange::UpsertEntity(entity.clone()));
                }
            }
        }

        // Detect removed entities
        let removed_ids: Vec<EntityId> = self
            .known_entities
            .keys()
            .copied()
            .filter(|id| !seen_ids.contains(id))
            .collect();

        for id in removed_ids {
            if let Some((entity_generation, rev)) = self.known_entities.remove(&id) {
                changes.push(WorldChange::RemoveEntity {
                    id,
                    expected_generation: entity_generation,
                    expected_revision: rev,
                });
            }
        }

        changes
    }
}

/// Bounded circular ring buffer for game announcements and combat reports.
#[derive(Clone, Debug)]
pub struct EventRingBuffer {
    events: VecDeque<WorldEvent>,
    capacity: usize,
    shed_events_count: u64,
}

impl Default for EventRingBuffer {
    fn default() -> Self {
        Self::new(MAX_EVENT_BUFFER_CAPACITY)
    }
}

impl EventRingBuffer {
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            events: VecDeque::with_capacity(capacity.min(MAX_EVENT_BUFFER_CAPACITY)),
            capacity: capacity.max(1).min(MAX_EVENT_BUFFER_CAPACITY),
            shed_events_count: 0,
        }
    }

    /// Push an event into the ring buffer. Sheds oldest events if at capacity.
    pub fn push_event(&mut self, event: WorldEvent) {
        if self.events.len() >= self.capacity {
            self.events.pop_front();
            self.shed_events_count = self.shed_events_count.saturating_add(1);
        }
        self.events.push_back(event);
    }

    /// Drain all pending events for delta stream emission.
    pub fn drain_events(&mut self) -> Vec<WorldEvent> {
        self.events.drain(..).collect()
    }

    /// Number of events currently in buffer.
    #[must_use]
    pub fn len(&self) -> usize {
        self.events.len()
    }

    /// Whether buffer is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Total number of events shed due to capacity limits.
    #[must_use]
    pub fn shed_events_count(&self) -> u64 {
        self.shed_events_count
    }
}

/// Continuous Delta Stream Generator that combines dirty chunks, entity updates,
/// and event stream into continuous `StateDelta` packets.
#[derive(Clone, Debug)]
pub struct ContinuousDeltaStreamer {
    fortress_id: FortressId,
    current_cursor: ObservationCursor,
    current_hash: Digest32,
    chunk_tracker: DirtyChunkTracker,
    entity_tracker: EntityDeltaTracker,
    event_buffer: EventRingBuffer,
}

impl ContinuousDeltaStreamer {
    /// Initialize streamer with base snapshot.
    pub fn new(base_snapshot: &WorldSnapshot) -> Self {
        let mut entity_tracker = EntityDeltaTracker::new();
        entity_tracker.seed_from_graph(&base_snapshot.graph);

        Self {
            fortress_id: base_snapshot.fortress_id,
            current_cursor: base_snapshot.cursor,
            current_hash: base_snapshot.state_hash,
            chunk_tracker: DirtyChunkTracker::new(),
            entity_tracker,
            event_buffer: EventRingBuffer::default(),
        }
    }

    /// Mark a chunk dirty.
    pub fn mark_chunk_dirty(&mut self, coord: ChunkCoord) {
        self.chunk_tracker.mark_dirty(coord);
    }

    /// Record a world event.
    pub fn record_event(&mut self, event: WorldEvent) {
        self.event_buffer.push_event(event);
    }

    /// Produce the next incremental `StateDelta` advancing cursor sequence by 1.
    pub fn emit_next_delta(
        &mut self,
        target_tick: GameTick,
        active_entities: &[EntityRecord],
        modified_chunks: &[MapChunk],
        target_hash: Digest32,
    ) -> Result<StateDelta> {
        let base_cursor = self.current_cursor;
        let target_cursor = ObservationCursor {
            epoch: base_cursor.epoch,
            sequence: base_cursor
                .sequence
                .checked_add(1)
                .ok_or_else(|| DfmcpError::new(ErrorCode::CursorGap, "cursor sequence overflow"))?,
        };

        let mut changes = Vec::new();

        // 1. Entity changes
        let entity_changes = self.entity_tracker.process_entities(active_entities);
        changes.extend(entity_changes);

        // 2. Chunk changes
        let dirty_coords = self.chunk_tracker.drain_dirty();
        let mut chunk_map: BTreeMap<ChunkCoord, MapChunk> = modified_chunks
            .iter()
            .map(|chunk| (chunk.coord, chunk.clone()))
            .collect();

        for coord in dirty_coords {
            if let Some(chunk) = chunk_map.remove(&coord) {
                changes.push(WorldChange::UpsertMapChunk(chunk));
            }
        }
        for (_, chunk) in chunk_map {
            changes.push(WorldChange::UpsertMapChunk(chunk));
        }

        // 3. Event changes
        let events = self.event_buffer.drain_events();
        for event in events {
            changes.push(WorldChange::AppendEvent(event));
        }

        let delta = StateDelta {
            fortress_id: self.fortress_id,
            base_cursor,
            target_cursor,
            base_hash: self.current_hash,
            target_hash,
            target_tick,
            changes,
            truncated: false,
            continuation: None,
        };

        // Advance internal state
        self.current_cursor = target_cursor;
        self.current_hash = target_hash;

        Ok(delta)
    }

    /// Current cursor sequence.
    #[must_use]
    pub fn current_cursor(&self) -> ObservationCursor {
        self.current_cursor
    }

    /// Current anchor hash.
    #[must_use]
    pub fn current_hash(&self) -> Digest32 {
        self.current_hash
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dfmcp_core::EventId;
    use dfmcp_world::{EntityKind, Fact, FactSource, TerrainRun, Value};

    fn sample_snapshot() -> WorldSnapshot {
        WorldSnapshot::new(
            FortressId::new(1),
            GameTick(100),
            ObservationCursor::ORIGIN,
            true,
            WorldGraph::default(),
        )
    }

    #[test]
    fn test_dirty_chunk_tracker() {
        let mut tracker = DirtyChunkTracker::new();
        let c1 = ChunkCoord { x: 0, y: 0, z: 100 };
        let c2 = ChunkCoord { x: 1, y: 0, z: 100 };

        tracker.mark_dirty(c1);
        tracker.mark_dirty(c2);
        assert_eq!(tracker.dirty_count(), 2);
        assert!(tracker.is_dirty(&c1));

        let drained = tracker.drain_dirty();
        assert_eq!(drained.len(), 2);
        assert_eq!(tracker.dirty_count(), 0);
    }

    #[test]
    fn test_entity_delta_tracker_upsert_and_removal() {
        let mut tracker = EntityDeltaTracker::new();
        let mut fields = BTreeMap::new();
        fields.insert(
            "profession".to_owned(),
            Fact::known(
                Value::Text("Miner".to_owned()),
                GameTick(100),
                FactSource::Replay,
                Digest32::ZERO,
            ),
        );
        let e1 = EntityRecord {
            id: EntityId::new(10),
            kind: EntityKind::Unit,
            generation: 1,
            revision: 1,
            label: "Miner 10".to_owned(),
            fields,
        };

        // First pass: add e1
        let changes1 = tracker.process_entities(&[e1.clone()]);
        assert_eq!(changes1.len(), 1);
        assert!(matches!(changes1[0], WorldChange::UpsertEntity(_)));

        // Second pass: unchanged e1 produces no changes
        let changes2 = tracker.process_entities(&[e1.clone()]);
        assert_eq!(changes2.len(), 0);

        // Third pass: modified revision produces upsert
        let mut e1_mod = e1.clone();
        e1_mod.revision = 2;
        let changes3 = tracker.process_entities(&[e1_mod]);
        assert_eq!(changes3.len(), 1);

        // Fourth pass: empty list produces removal
        let changes4 = tracker.process_entities(&[]);
        assert_eq!(changes4.len(), 1);
        assert!(matches!(changes4[0], WorldChange::RemoveEntity { .. }));
    }

    #[test]
    fn test_event_ring_buffer_capacity_shedding() {
        let mut buffer = EventRingBuffer::new(5);
        for i in 0..10 {
            buffer.push_event(WorldEvent {
                id: EventId::new(i + 1),
                tick: GameTick(100 + i as u64),
                kind: dfmcp_world::WorldEventKind::Announcement,
                subject: None,
                summary: format!("event {}", i),
                fields: BTreeMap::new(),
            });
        }

        assert_eq!(buffer.len(), 5);
        assert_eq!(buffer.shed_events_count(), 5);

        let drained = buffer.drain_events();
        assert_eq!(drained.len(), 5);
        assert_eq!(drained[0].id, EventId::new(6)); // Oldest kept event
    }

    #[test]
    fn test_continuous_delta_streamer_cursor_monotonicity() -> Result<()> {
        let base = sample_snapshot();
        let mut streamer = ContinuousDeltaStreamer::new(&base);

        let c1 = ChunkCoord { x: 0, y: 0, z: 100 };
        let chunk1 = MapChunk {
            coord: c1,
            revision: 1,
            width: 16,
            height: 16,
            terrain_runs: vec![TerrainRun {
                tile_code: 0,
                length: 256,
            }],
            sparse_overlays: BTreeMap::new(),
        };

        streamer.mark_chunk_dirty(c1);

        let target_hash = Digest32::of_bytes(b"target_hash_step_1");
        let delta = streamer.emit_next_delta(GameTick(101), &[], &[chunk1], target_hash)?;

        assert_eq!(delta.base_cursor, ObservationCursor::ORIGIN);
        assert_eq!(
            delta.target_cursor,
            ObservationCursor {
                epoch: 0,
                sequence: 1
            }
        );
        assert_eq!(delta.target_tick, GameTick(101));
        assert_eq!(delta.target_hash, target_hash);
        assert_eq!(
            streamer.current_cursor(),
            ObservationCursor {
                epoch: 0,
                sequence: 1
            }
        );

        Ok(())
    }
}
