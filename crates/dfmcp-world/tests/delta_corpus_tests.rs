#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::error::Error;

use dfmcp_core::{
    Digest32, EdgeId, EntityId, ErrorCode, EventId, FortressId, GameTick, ObservationCursor,
};
use dfmcp_world::{
    ChunkCoord, ContinuationToken, EdgeKind, EdgeRecord, EntityKind, EntityRecord, Fact,
    FactSource, MapChunk, TerrainRun, Value, WorldEvent, WorldEventKind, WorldGraph, WorldSnapshot,
    apply_delta, build_delta, diff_snapshots,
};

/// Deterministic xorshift64 PRNG
struct TestPrng {
    state: u64,
}

impl TestPrng {
    fn new(seed: u64) -> Self {
        Self {
            state: if seed == 0 {
                0x1234_5678_9abc_def0
            } else {
                seed
            },
        }
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state = x;
        x
    }

    fn next_range(&mut self, min: u64, max: u64) -> u64 {
        if min >= max {
            return min;
        }
        min + (self.next_u64() % (max - min + 1))
    }
}

fn generate_random_snapshot(
    prng: &mut TestPrng,
    tick: u64,
    cursor: ObservationCursor,
) -> WorldSnapshot {
    let mut graph = WorldGraph::default();

    let entity_count = prng.next_range(1, 10) as usize;
    for i in 1..=entity_count {
        let id = EntityId::new(i as u64);
        let mut fields = BTreeMap::new();
        fields.insert(
            "attr".to_owned(),
            Fact::known(
                Value::I64(prng.next_range(1, 100) as i64),
                GameTick(tick),
                FactSource::Replay,
                Digest32::ZERO,
            ),
        );
        graph.entities.insert(
            id,
            EntityRecord {
                id,
                generation: 1,
                revision: 1,
                kind: EntityKind::Unit,
                label: format!("Entity_{i}"),
                fields,
            },
        );
    }

    if entity_count >= 2 {
        let edge_id = EdgeId::new(100);
        graph.edges.insert(
            edge_id,
            EdgeRecord {
                id: edge_id,
                revision: 1,
                kind: EdgeKind::AssignedTo,
                from: EntityId::new(1),
                to: EntityId::new(2),
                fields: BTreeMap::new(),
            },
        );
    }

    let chunk_coord = ChunkCoord { x: 0, y: 0, z: 0 };
    graph.chunks.insert(
        chunk_coord,
        MapChunk {
            coord: chunk_coord,
            revision: 1,
            width: 4,
            height: 4,
            terrain_runs: vec![TerrainRun {
                tile_code: 1,
                length: 16,
            }],
            sparse_overlays: BTreeMap::new(),
        },
    );

    WorldSnapshot::new(FortressId::new(1), GameTick(tick), cursor, false, graph)
}

/// TEST-004: Equivalence corpus — 1000+ seeded mutations with full→delta exact reconstruction
#[test]
fn test_004_full_to_delta_equivalence_corpus_1000_scenarios() -> Result<(), Box<dyn Error>> {
    let mut prng = TestPrng::new(0x2026_0830_1000);

    for scenario_idx in 0..1000 {
        let base_tick = prng.next_range(100, 500);
        let base_cursor = ObservationCursor {
            epoch: 1,
            sequence: prng.next_range(1, 50),
        };
        let base = generate_random_snapshot(&mut prng, base_tick, base_cursor);

        // Mutate base into target
        let mut target = base.clone();
        target.cursor = base.cursor.next();
        target.tick = GameTick(base_tick + prng.next_range(1, 10));

        let num_mutations = prng.next_range(1, 4);
        for m in 0..num_mutations {
            match m % 3 {
                0 => {
                    // Add or update an entity
                    let new_id = EntityId::new(prng.next_range(1, 20));
                    let mut fields = BTreeMap::new();
                    fields.insert(
                        "val".to_owned(),
                        Fact::known(
                            Value::I64(prng.next_range(1, 100) as i64),
                            target.tick,
                            FactSource::Replay,
                            Digest32::ZERO,
                        ),
                    );
                    target.graph.entities.insert(
                        new_id,
                        EntityRecord {
                            id: new_id,
                            generation: 1,
                            revision: 2,
                            kind: EntityKind::Item,
                            label: format!("Mutated_{scenario_idx}_{m}"),
                            fields,
                        },
                    );
                }
                1 => {
                    // Update map chunk
                    let chunk_coord = ChunkCoord { x: 0, y: 0, z: 0 };
                    let mut overlays = BTreeMap::new();
                    let mut props = BTreeMap::new();
                    props.insert("level".to_owned(), Value::U64(prng.next_range(1, 7)));
                    overlays.insert(0, props);
                    target.graph.chunks.insert(
                        chunk_coord,
                        MapChunk {
                            coord: chunk_coord,
                            revision: 2,
                            width: 4,
                            height: 4,
                            terrain_runs: vec![TerrainRun {
                                tile_code: 2,
                                length: 16,
                            }],
                            sparse_overlays: overlays,
                        },
                    );
                }
                _ => {
                    // Append event
                    let event_id = EventId::new(u128::from(prng.next_range(1, 500)));
                    target.graph.events.insert(
                        event_id,
                        WorldEvent {
                            id: event_id,
                            tick: target.tick,
                            kind: WorldEventKind::Announcement,
                            subject: None,
                            summary: format!("Scenario Event {scenario_idx}"),
                            fields: BTreeMap::new(),
                        },
                    );
                }
            }
        }
        target.refresh_hash();

        // Compute delta and apply
        let delta = diff_snapshots(&base, &target)?;
        let reconstructed = apply_delta(&base, &delta)?;

        assert_eq!(
            reconstructed.state_hash, target.state_hash,
            "Scenario {scenario_idx}: reconstructed state_hash must match target state_hash"
        );
        assert_eq!(
            reconstructed.cursor, target.cursor,
            "Scenario {scenario_idx}: reconstructed cursor must match target"
        );
        assert_eq!(
            reconstructed.tick, target.tick,
            "Scenario {scenario_idx}: reconstructed tick must match target"
        );
    }
    Ok(())
}

/// TEST-004: Gap, Fork, Epoch, and Duplicate Rejection
#[test]
fn test_004_delta_rejection_suite() -> Result<(), Box<dyn Error>> {
    let base = generate_random_snapshot(
        &mut TestPrng::new(42),
        100,
        ObservationCursor {
            epoch: 1,
            sequence: 10,
        },
    );

    let mut target = base.clone();
    target.cursor = base.cursor.next();
    target.tick = GameTick(105);
    target.refresh_hash();

    let delta = diff_snapshots(&base, &target)?;

    // 1. Cursor gap rejection: applying delta when base cursor is not matching (e.g. sequence gap)
    let mut gap_snapshot = base.clone();
    gap_snapshot.cursor = ObservationCursor {
        epoch: 1,
        sequence: 15,
    };
    gap_snapshot.refresh_hash();
    let Err(err_gap) = apply_delta(&gap_snapshot, &delta) else {
        return Err("expected CursorGap on sequence gap".into());
    };
    assert_eq!(err_gap.code, ErrorCode::CursorGap);

    // 2. Epoch change rejection: delta across epoch boundary
    let Err(err_epoch) = build_delta(
        &base,
        ObservationCursor {
            epoch: 2,
            sequence: 1,
        },
        GameTick(105),
        vec![],
    ) else {
        return Err("expected CursorGap on epoch change".into());
    };
    assert_eq!(err_epoch.code, ErrorCode::CursorGap);
    assert!(err_epoch.message.contains("epoch changes"));

    // 3. Fork rejection: base cursor matches but state_hash differs (forked history)
    let mut forked_base = base.clone();
    forked_base.paused = !forked_base.paused;
    forked_base.refresh_hash();
    let Err(err_fork) = apply_delta(&forked_base, &delta) else {
        return Err("expected CursorGap on forked state_hash".into());
    };
    assert_eq!(err_fork.code, ErrorCode::CursorGap);

    // 4. Duplicate rejection: applying delta to target that was already advanced
    let advanced_snapshot = apply_delta(&base, &delta)?;
    let Err(err_dup) = apply_delta(&advanced_snapshot, &delta) else {
        return Err("expected CursorGap on duplicate delta".into());
    };
    assert_eq!(err_dup.code, ErrorCode::CursorGap);

    // 5. Stale tick rejection: delta target tick < base tick
    let Err(err_stale_tick) = build_delta(&base, base.cursor.next(), GameTick(50), vec![]) else {
        return Err("expected InvalidRequest on stale tick".into());
    };
    assert_eq!(err_stale_tick.code, ErrorCode::InvalidRequest);

    Ok(())
}

/// Continuation Token Round-Trip & Verification
#[test]
fn test_continuation_token_round_trip() -> Result<(), Box<dyn Error>> {
    let token = ContinuationToken::new(
        FortressId::new(7),
        ObservationCursor {
            epoch: 3,
            sequence: 142,
        },
        500,
    );

    let encoded = token.encode();
    assert_eq!(encoded, "cont:7:3:142:500");

    let decoded = ContinuationToken::decode(&encoded)?;
    assert_eq!(decoded, token);

    // Invalid format rejection
    assert!(ContinuationToken::decode("invalid_token").is_err());
    assert!(ContinuationToken::decode("cont:7:3:142").is_err());
    assert!(ContinuationToken::decode("cont:not_num:3:142:500").is_err());

    Ok(())
}
