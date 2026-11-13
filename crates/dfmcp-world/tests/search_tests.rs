#![forbid(unsafe_code)]

//! Integration tests for the in-memory lexical search contract prototype.

use std::collections::BTreeMap;

use dfmcp_core::{Digest32, EntityId, EventId, FortressId, GameTick, ObservationCursor};
use dfmcp_world::search::FrankenSearchEngine;
use dfmcp_world::{
    EntityKind, EntityRecord, Fact, FactPresence, FactSource, Value, WorldEvent, WorldEventKind,
    WorldGraph, WorldSnapshot,
};

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

fn snapshot_with_fields(fields: BTreeMap<String, Fact>) -> WorldSnapshot {
    let entity = EntityRecord {
        id: EntityId::new(10),
        kind: EntityKind::Unit,
        generation: 1,
        revision: 1,
        label: "Urist".to_owned(),
        fields,
    };
    WorldSnapshot::new(
        FortressId::new(1),
        GameTick(100),
        ObservationCursor::ORIGIN,
        true,
        WorldGraph {
            entities: BTreeMap::from([(entity.id, entity)]),
            ..WorldGraph::default()
        },
    )
}

#[test]
fn snapshot_search_excludes_unavailable_values_from_matches_snippets_and_scores()
-> Result<(), Box<dyn std::error::Error>> {
    let baseline = snapshot_with_fields(BTreeMap::new());
    let mut engine = FrankenSearchEngine::new();
    engine.index_snapshot(&baseline)?;
    let baseline_hits = engine.search("Urist", 5)?;
    assert_eq!(baseline_hits.len(), 1);

    for presence in [
        FactPresence::Absent,
        FactPresence::Unknown("withheldmetadata".to_owned()),
        FactPresence::Unsupported("withheldmetadata".to_owned()),
        FactPresence::Omitted("withheldmetadata".to_owned()),
        FactPresence::Redacted("withheldmetadata".to_owned()),
        FactPresence::Stale(baseline.anchor()),
        FactPresence::Known(Value::Text("claimedmarker".to_owned())),
    ] {
        let mut fact = Fact::with_presence(
            presence,
            GameTick(100),
            FactSource::Replay,
            Digest32::ZERO,
        );
        fact.value = Value::Text("retainedmarker".to_owned());
        let snapshot = snapshot_with_fields(BTreeMap::from([("message".to_owned(), fact)]));
        engine.index_snapshot(&snapshot)?;

        for text in ["retainedmarker", "claimedmarker", "withheldmetadata", "message"] {
            assert!(
                engine.search(text, 5)?.is_empty(),
                "unavailable field data became searchable: {text}"
            );
        }
        // Hidden bytes must not influence document length, scores or snippets,
        // including when the visible label independently matches the query.
        assert_eq!(engine.search("Urist", 5)?, baseline_hits);
    }
    Ok(())
}

#[test]
fn snapshot_search_keeps_legacy_and_explicit_known_values_and_retained_events()
-> Result<(), Box<dyn std::error::Error>> {
    let mut snapshot = snapshot_with_fields(BTreeMap::from([
        (
            "legacy".to_owned(),
            Fact::known(
                Value::Text("agatemarker".to_owned()),
                GameTick(100),
                FactSource::Replay,
                Digest32::ZERO,
            ),
        ),
        (
            "explicit".to_owned(),
            Fact::with_presence(
                FactPresence::Known(Value::Text("hematitemarker".to_owned())),
                GameTick(100),
                FactSource::Replay,
                Digest32::ZERO,
            ),
        ),
    ]));
    let event = WorldEvent {
        id: EventId::new(91),
        tick: GameTick(100),
        kind: WorldEventKind::Announcement,
        subject: None,
        summary: "Retained report".to_owned(),
        fields: BTreeMap::from([(
            "message".to_owned(),
            Value::Text("eventmarker".to_owned()),
        )]),
    };
    snapshot.graph.events.insert(event.id, event);
    snapshot.refresh_hash();

    let mut engine = FrankenSearchEngine::new();
    engine.index_snapshot(&snapshot)?;
    for text in ["agatemarker", "hematitemarker"] {
        let hits = engine.search(text, 5)?;
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].entity_id, Some(EntityId::new(10)));
        assert_eq!(hits[0].event_id, None);
        assert!(hits[0].snippet.contains(text));
    }
    let events = engine.search("eventmarker", 5)?;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].entity_id, None);
    assert_eq!(events[0].event_id, Some(EventId::new(91)));
    Ok(())
}

#[test]
fn snapshot_search_finds_builtin_and_custom_kinds_with_stable_identity_ties()
-> Result<(), Box<dyn std::error::Error>> {
    let record = |id, kind| EntityRecord {
        id: EntityId::new(id),
        generation: 1,
        revision: 1,
        kind,
        label: "Record".to_owned(),
        fields: BTreeMap::new(),
    };
    let mut snapshot = snapshot_with_fields(BTreeMap::new());
    snapshot.graph.entities.clear();
    // Reverse insertion order must not affect equal-score canonical ties.
    for entity in [
        record(42, EntityKind::Building),
        record(17, EntityKind::Building),
        record(90, EntityKind::Other("special_archive".to_owned())),
    ] {
        snapshot.graph.entities.insert(entity.id, entity);
    }
    snapshot.refresh_hash();

    let mut engine = FrankenSearchEngine::new();
    engine.index_snapshot(&snapshot)?;
    let hits = engine.search("building", 5)?;
    assert_eq!(
        hits.iter().map(|hit| hit.entity_id).collect::<Vec<_>>(),
        vec![Some(EntityId::new(17)), Some(EntityId::new(42))]
    );
    assert_eq!(hits[0].score_micros, hits[1].score_micros);
    let custom = engine.search("special archive", 5)?;
    assert_eq!(custom.len(), 1);
    assert_eq!(custom[0].entity_id, Some(EntityId::new(90)));
    engine.index_snapshot(&snapshot)?;
    assert_eq!(engine.search("building", 5)?, hits);
    Ok(())
}

#[test]
fn rebuilding_search_withdraws_newly_unavailable_values()
-> Result<(), Box<dyn std::error::Error>> {
    let mut snapshot = snapshot_with_fields(BTreeMap::from([(
        "message".to_owned(),
        Fact::known(
            Value::Text("withdrawnmarker".to_owned()),
            GameTick(100),
            FactSource::Replay,
            Digest32::ZERO,
        ),
    )]));
    let mut engine = FrankenSearchEngine::new();
    engine.index_snapshot(&snapshot)?;
    assert_eq!(engine.search("withdrawnmarker", 5)?.len(), 1);

    let entity = snapshot
        .graph
        .entities
        .get_mut(&EntityId::new(10))
        .ok_or("fixture entity is missing")?;
    entity
        .fields
        .get_mut("message")
        .ok_or("fixture field is missing")?
        .presence = Some(FactPresence::Redacted("policy/2".to_owned()));
    entity.revision += 1;
    snapshot.cursor = snapshot.cursor.checked_next().ok_or("cursor exhausted")?;
    snapshot.refresh_hash();

    engine.index_snapshot(&snapshot)?;
    assert!(engine.search("withdrawnmarker", 5)?.is_empty());
    let visible = engine.search("Urist", 5)?;
    assert_eq!(visible.len(), 1);
    assert!(!visible[0].snippet.contains("withdrawnmarker"));
    Ok(())
}

#[test]
fn long_announcement_titles_preserve_suffix_matches_and_identity_ties()
-> Result<(), Box<dyn std::error::Error>> {
    let mut snapshot = snapshot_with_fields(BTreeMap::new());
    let summary = format!("{}buriedmarker", "routine report ".repeat(30));
    assert!(summary.len() > 256);
    for id in [92, 91] {
        let event = WorldEvent {
            id: EventId::new(id),
            tick: snapshot.tick,
            kind: WorldEventKind::Announcement,
            subject: None,
            summary: summary.clone(),
            fields: BTreeMap::new(),
        };
        snapshot.graph.events.insert(event.id, event);
    }
    snapshot.refresh_hash();

    let mut engine = FrankenSearchEngine::new();
    engine.index_snapshot(&snapshot)?;
    let hits = engine.search("buriedmarker", 5)?;
    assert_eq!(
        hits.iter().map(|hit| hit.event_id).collect::<Vec<_>>(),
        vec![Some(EventId::new(91)), Some(EventId::new(92))]
    );
    assert_eq!(hits[0].score_micros, hits[1].score_micros);
    for hit in &hits {
        assert_eq!(hit.entity_id, None);
        assert!(hit.title.len() <= 256);
        assert!(hit.title.ends_with("..."));
        assert!(!hit.title.contains("buriedmarker"));
        assert!(hit.matched_terms.contains(&"buriedmarker".to_owned()));
    }
    let unrelated = engine.search("Urist", 5)?;
    assert_eq!(unrelated.len(), 1);
    assert_eq!(unrelated[0].entity_id, Some(EntityId::new(10)));
    engine.index_snapshot(&snapshot)?;
    assert_eq!(engine.search("buriedmarker", 5)?, hits);
    Ok(())
}

#[test]
fn announcement_display_titles_preserve_exact_bounds_and_utf8()
-> Result<(), Box<dyn std::error::Error>> {
    for (summary, expected_title, query) in [
        ("a".repeat(256), "a".repeat(256), "a".repeat(256)),
        (
            format!("{}🪨 suffixmarker", "岩".repeat(85)),
            format!("{}...", "岩".repeat(84)),
            "suffixmarker".to_owned(),
        ),
    ] {
        let mut snapshot = snapshot_with_fields(BTreeMap::new());
        let event = WorldEvent {
            id: EventId::new(91),
            tick: snapshot.tick,
            kind: WorldEventKind::Announcement,
            subject: None,
            summary,
            fields: BTreeMap::new(),
        };
        snapshot.graph.events.insert(event.id, event);
        snapshot.refresh_hash();
        let mut engine = FrankenSearchEngine::new();
        engine.index_snapshot(&snapshot)?;
        let hits = engine.search(&query, 5)?;
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].event_id, Some(EventId::new(91)));
        assert_eq!(hits[0].title, expected_title);
        assert!(hits[0].title.len() <= 256);
    }
    Ok(())
}

#[test]
fn direct_document_title_and_body_limits_remain_atomic() -> Result<(), Box<dyn std::error::Error>> {
    let mut engine = FrankenSearchEngine::new();
    engine.index_document(
        Some(EntityId::new(10)),
        None,
        "retained".to_owned(),
        "beforemarker".to_owned(),
    )?;
    let before = engine.search("beforemarker", 5)?;
    for (title, body) in [
        ("a".repeat(257), "latermarker".to_owned()),
        ("valid".to_owned(), "x".repeat(65_537)),
    ] {
        let refused = engine.index_document(None, Some(EventId::new(91)), title, body);
        assert_eq!(
            refused.err().ok_or("oversized document was admitted")?.code,
            dfmcp_core::ErrorCode::BudgetExceeded
        );
        assert_eq!(engine.search("beforemarker", 5)?, before);
        assert!(engine.search("latermarker", 5)?.is_empty());
    }
    let mut oversized = snapshot_with_fields(BTreeMap::new());
    let event = WorldEvent {
        id: EventId::new(91),
        tick: oversized.tick,
        kind: WorldEventKind::Announcement,
        subject: None,
        summary: "x".repeat(65_537),
        fields: BTreeMap::new(),
    };
    oversized.graph.events.insert(event.id, event);
    oversized.refresh_hash();
    let refused = engine.index_snapshot(&oversized);
    assert_eq!(
        refused.err().ok_or("oversized snapshot body was admitted")?.code,
        dfmcp_core::ErrorCode::BudgetExceeded
    );
    assert_eq!(engine.search("beforemarker", 5)?, before);
    Ok(())
}
