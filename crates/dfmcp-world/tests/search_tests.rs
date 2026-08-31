#![forbid(unsafe_code)]

//! Integration tests for the in-memory lexical search contract prototype.

use std::collections::BTreeMap;

use dfmcp_core::{Digest32, EntityId, FortressId, GameTick, ObservationCursor};
use dfmcp_world::search::FrankenSearchEngine;
use dfmcp_world::{EntityKind, EntityRecord, Fact, FactSource, Value, WorldGraph, WorldSnapshot};

#[test]
fn test_in_memory_lexical_snapshot_indexing() -> Result<(), Box<dyn std::error::Error>> {
    let mut engine = FrankenSearchEngine::new();
    let mut graph = WorldGraph::default();

    let mut fields = BTreeMap::new();
    fields.insert(
        "thought".to_owned(),
        Fact::known(
            Value::Text(
                "felt terrified after encountering a giant spider in the deep caverns".to_owned(),
            ),
            GameTick(100),
            FactSource::Derived("mood_scanner".to_owned()),
            Digest32::ZERO,
        ),
    );

    let dwarf = EntityRecord {
        id: EntityId::new(10),
        kind: EntityKind::Unit,
        generation: 1,
        revision: 1,
        label: "Urist".to_owned(),
        fields,
    };

    graph.entities.insert(EntityId::new(10), dwarf);

    let snapshot = WorldSnapshot::new(
        FortressId::new(1),
        GameTick(100),
        ObservationCursor::ORIGIN,
        true,
        graph,
    );

    engine.index_snapshot(&snapshot)?;

    let results = engine.search("spider caverns", 5)?;
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].entity_id, Some(EntityId::new(10)));
    Ok(())
}
