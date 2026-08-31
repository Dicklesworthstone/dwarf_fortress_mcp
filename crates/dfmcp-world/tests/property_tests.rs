#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::error::Error;

use dfmcp_core::{Digest32, EntityId, FortressId, GameTick, ObservationCursor};
use dfmcp_world::{
    EntityKind, EntityRecord, Fact, FactSource, Value, WorldChange, WorldGraph, WorldSnapshot,
    apply_delta, build_delta,
};

/// Deterministic xorshift64 PRNG for reproducible property tests
struct TestPrng {
    state: u64,
}

impl TestPrng {
    fn new(seed: u64) -> Self {
        Self {
            state: if seed == 0 {
                0xcafe_babe_dead_beef
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

/// Property Test: Insertion-order independence for canonical WorldSnapshot state hash
#[test]
fn test_property_graph_canonical_hash_insertion_order_independence() {
    let seed = 0x2026_0830_0020;
    let mut prng = TestPrng::new(seed);

    for _ in 0..50 {
        let entity_count = prng.next_range(5, 20) as usize;
        let mut entities = Vec::with_capacity(entity_count);

        for i in 1..=entity_count {
            let id = i as u64;
            let mut fields = BTreeMap::new();
            fields.insert(
                "stat".to_owned(),
                Fact::known(
                    Value::I64(prng.next_range(1, 100) as i64),
                    GameTick(10),
                    FactSource::Replay,
                    Digest32::ZERO,
                ),
            );
            entities.push(EntityRecord {
                id: EntityId::new(id),
                generation: 1,
                revision: 1,
                kind: EntityKind::Unit,
                label: format!("Unit_{id}"),
                fields,
            });
        }

        // Build snapshot 1: sequential insertion
        let mut graph1 = WorldGraph::default();
        for entity in &entities {
            graph1.entities.insert(entity.id, entity.clone());
        }
        let snapshot1 = WorldSnapshot::new(
            FortressId::new(1),
            GameTick(10),
            ObservationCursor::ORIGIN,
            false,
            graph1,
        );

        // Build snapshot 2: reversed insertion order
        let mut graph2 = WorldGraph::default();
        for entity in entities.iter().rev() {
            graph2.entities.insert(entity.id, entity.clone());
        }
        let snapshot2 = WorldSnapshot::new(
            FortressId::new(1),
            GameTick(10),
            ObservationCursor::ORIGIN,
            false,
            graph2,
        );

        assert_eq!(
            snapshot1.state_hash, snapshot2.state_hash,
            "Snapshots with identical contents in different insertion order must produce identical state hash"
        );
    }
}

/// Property Test: ABA generation monotonicity and anti-reuse guarantees
#[test]
fn test_property_aba_generation_regression_rejection() {
    let seed = 0x2026_0830_0021;
    let mut prng = TestPrng::new(seed);

    for _ in 0..100 {
        let id = EntityId::new(prng.next_range(1, 1000));
        let initial_gen = prng.next_range(2, 50) as u32;
        let initial_rev = prng.next_range(1, 50);

        let mut graph = WorldGraph::default();
        graph.entities.insert(
            id,
            EntityRecord {
                id,
                generation: initial_gen,
                revision: initial_rev,
                kind: EntityKind::Building,
                label: "Workshop".to_owned(),
                fields: BTreeMap::new(),
            },
        );

        let base = WorldSnapshot::new(
            FortressId::new(1),
            GameTick(1),
            ObservationCursor::ORIGIN,
            false,
            graph,
        );

        // Attempt to upsert with a regressed generation (e.g. initial_gen - 1)
        let regressed_gen = initial_gen - 1;
        let regressed_entity = EntityRecord {
            id,
            generation: regressed_gen,
            revision: initial_rev + 1,
            kind: EntityKind::Building,
            label: "Stale Workshop".to_owned(),
            fields: BTreeMap::new(),
        };

        let result = build_delta(
            &base,
            base.cursor.next(),
            GameTick(2),
            vec![WorldChange::UpsertEntity(regressed_entity)],
        );

        assert!(
            result.is_err(),
            "Upserting an entity with regressed generation {regressed_gen} < {initial_gen} must be rejected"
        );
    }
}

/// Property Test: StateDelta composition and round-trip consistency
#[test]
fn test_property_delta_state_hash_composition() -> Result<(), Box<dyn Error>> {
    let seed = 0x2026_0830_0022;
    let mut prng = TestPrng::new(seed);

    let mut current_snapshot = {
        let mut graph = WorldGraph::default();
        graph.entities.insert(
            EntityId::new(1),
            EntityRecord {
                id: EntityId::new(1),
                generation: 1,
                revision: 1,
                kind: EntityKind::Unit,
                label: "Origin Dwarf".to_owned(),
                fields: BTreeMap::new(),
            },
        );
        WorldSnapshot::new(
            FortressId::new(42),
            GameTick(1),
            ObservationCursor::ORIGIN,
            true,
            graph,
        )
    };

    let mut current_tick = 1u64;

    for step in 1..=30 {
        current_tick += prng.next_range(1, 5);
        let next_cursor = current_snapshot.cursor.next();

        let new_entity_id = EntityId::new(step + 10);
        let change = WorldChange::UpsertEntity(EntityRecord {
            id: new_entity_id,
            generation: 1,
            revision: 1,
            kind: EntityKind::Item,
            label: format!("Item_{step}"),
            fields: BTreeMap::new(),
        });

        let delta = build_delta(
            &current_snapshot,
            next_cursor,
            GameTick(current_tick),
            vec![change],
        )?;

        let next_snapshot = apply_delta(&current_snapshot, &delta)?;

        assert!(next_snapshot.hash_is_valid());
        assert_eq!(next_snapshot.tick, GameTick(current_tick));
        assert_eq!(next_snapshot.cursor, next_cursor);

        current_snapshot = next_snapshot;
    }
    Ok(())
}
