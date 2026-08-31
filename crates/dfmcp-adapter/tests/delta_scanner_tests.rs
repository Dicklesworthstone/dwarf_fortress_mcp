#![forbid(unsafe_code)]

//! Integration tests for WP-DFH-03 dirty chunk map and entity delta streamer.

use std::collections::BTreeMap;
use std::error::Error;

use dfmcp_adapter::delta_scanner::{
    ContinuousDeltaStreamer, DirtyChunkTracker, EntityDeltaTracker, EventRingBuffer,
};
use dfmcp_core::{Digest32, EntityId, EventId, FortressId, GameTick, ObservationCursor};
use dfmcp_world::{
    ChunkCoord, EntityKind, Fact, FactSource, Value, WorldChange, WorldEvent, WorldGraph,
    WorldSnapshot,
};

#[test]
fn test_dirty_chunk_batch_draining() {
    let mut tracker = DirtyChunkTracker::new();
    for x in 0..5 {
        for y in 0..5 {
            tracker.mark_dirty(ChunkCoord { x, y, z: 100 });
        }
    }
    assert_eq!(tracker.dirty_count(), 25);
    let drained = tracker.drain_dirty();
    assert_eq!(drained.len(), 25);
    assert_eq!(tracker.dirty_count(), 0);
}

#[test]
fn test_entity_generation_change_detection() {
    let mut tracker = EntityDeltaTracker::new();
    let id = EntityId::new(100);

    let e_gen1 = entity_record_helper(id, 1, 1);
    let changes1 = tracker.process_entities(&[e_gen1]);
    assert_eq!(changes1.len(), 1);

    // Generation increment (e.g. entity deleted and re-spawned with same ID)
    let e_gen2 = entity_record_helper(id, 2, 1);
    let changes2 = tracker.process_entities(&[e_gen2]);
    assert_eq!(changes2.len(), 1);
    assert!(matches!(changes2[0], WorldChange::UpsertEntity(_)));
}

#[test]
fn test_event_ring_buffer_fifo_order() {
    let mut buffer = EventRingBuffer::new(3);
    for i in 1..=5 {
        buffer.push_event(WorldEvent {
            id: EventId::new(i),
            tick: GameTick(100 + i as u64),
            kind: dfmcp_world::WorldEventKind::Announcement,
            subject: None,
            summary: format!("combat event {}", i),
            fields: BTreeMap::new(),
        });
    }

    assert_eq!(buffer.len(), 3);
    assert_eq!(buffer.shed_events_count(), 2);

    let drained = buffer.drain_events();
    assert_eq!(drained.len(), 3);
    assert_eq!(drained[0].id, EventId::new(3));
    assert_eq!(drained[1].id, EventId::new(4));
    assert_eq!(drained[2].id, EventId::new(5));
}

#[test]
fn test_continuous_delta_sequence_stream() -> Result<(), Box<dyn Error>> {
    let base = WorldSnapshot::new(
        FortressId::new(42),
        GameTick(500),
        ObservationCursor {
            epoch: 1,
            sequence: 10,
        },
        false,
        WorldGraph::default(),
    );

    let mut streamer = ContinuousDeltaStreamer::new(&base);

    for step in 1..=5 {
        let tick = GameTick(500 + step);
        let hash = Digest32::of_bytes(format!("hash_step_{}", step).as_bytes());
        let delta = streamer.emit_next_delta(tick, &[], &[], hash)?;

        assert_eq!(delta.fortress_id, FortressId::new(42));
        assert_eq!(
            delta.base_cursor,
            ObservationCursor {
                epoch: 1,
                sequence: 10 + step - 1
            }
        );
        assert_eq!(
            delta.target_cursor,
            ObservationCursor {
                epoch: 1,
                sequence: 10 + step
            }
        );
        assert_eq!(delta.target_tick, tick);
        assert_eq!(delta.target_hash, hash);
    }

    assert_eq!(
        streamer.current_cursor(),
        ObservationCursor {
            epoch: 1,
            sequence: 15
        }
    );
    Ok(())
}

fn entity_record_helper(id: EntityId, generation: u32, revision: u64) -> dfmcp_world::EntityRecord {
    let mut fields = BTreeMap::new();
    fields.insert(
        "role".to_owned(),
        Fact::known(
            Value::Text("Craftsdwarf".to_owned()),
            GameTick(100),
            FactSource::Replay,
            Digest32::ZERO,
        ),
    );
    dfmcp_world::EntityRecord {
        id,
        kind: EntityKind::Unit,
        generation,
        revision,
        label: format!("Unit {}", id.get()),
        fields,
    }
}
