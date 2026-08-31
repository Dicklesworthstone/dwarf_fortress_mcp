#![forbid(unsafe_code)]

use std::collections::BTreeMap;

use dfmcp_core::{Digest32, EntityId, FortressId, GameTick, ObservationCursor};
use dfmcp_world::{
    AttentionEngine, AttentionSignalKind, CompletenessStatus, EntityKind, EntityRecord, Fact,
    FactSource, Value, WorldGraph, WorldSnapshot,
};

fn make_snapshot_with_stressed_dwarves() -> WorldSnapshot {
    let mut graph = WorldGraph::default();

    let mut f1 = BTreeMap::new();
    f1.insert(
        "stress".to_owned(),
        Fact::known(
            Value::I64(90),
            GameTick(10),
            FactSource::Replay,
            Digest32::of_bytes(b"fact_stress_90"),
        ),
    );
    graph.entities.insert(
        EntityId::new(1),
        EntityRecord {
            id: EntityId::new(1),
            generation: 1,
            revision: 2,
            kind: EntityKind::Unit,
            label: "Urist stressed".to_owned(),
            fields: f1,
        },
    );

    let mut f2 = BTreeMap::new();
    f2.insert(
        "stress".to_owned(),
        Fact::known(
            Value::I64(350),
            GameTick(10),
            FactSource::Replay,
            Digest32::of_bytes(b"fact_stress_350"),
        ),
    );
    graph.entities.insert(
        EntityId::new(2),
        EntityRecord {
            id: EntityId::new(2),
            generation: 1,
            revision: 3,
            kind: EntityKind::Unit,
            label: "Domas stressed".to_owned(),
            fields: f2,
        },
    );

    let mut f3 = BTreeMap::new();
    f3.insert(
        "stress".to_owned(),
        Fact::known(
            Value::I64(10),
            GameTick(10),
            FactSource::Replay,
            Digest32::of_bytes(b"fact_stress_10"),
        ),
    );
    graph.entities.insert(
        EntityId::new(3),
        EntityRecord {
            id: EntityId::new(3),
            generation: 1,
            revision: 1,
            kind: EntityKind::Unit,
            label: "Zulgar happy".to_owned(),
            fields: f3,
        },
    );

    WorldSnapshot::new(
        FortressId::new(1),
        GameTick(100),
        ObservationCursor::ORIGIN,
        false,
        graph,
    )
}

#[test]
fn test_attention_deterministic_ranking_and_evidence_ledger()
-> Result<(), Box<dyn std::error::Error>> {
    let snapshot = make_snapshot_with_stressed_dwarves();

    let ledger1 = AttentionEngine::rank_attention(&snapshot, 1, 10);
    let ledger2 = AttentionEngine::rank_attention(&snapshot, 1, 10);

    assert_eq!(ledger1, ledger2);
    assert_eq!(ledger1.ledger_digest, ledger2.ledger_digest);
    assert_eq!(ledger1.completeness, CompletenessStatus::Complete);
    assert_eq!(ledger1.signals.len(), 2);

    assert_eq!(ledger1.signals[0].subject, Some(EntityId::new(2)));
    assert_eq!(ledger1.signals[0].severity_score, 350);
    assert_eq!(ledger1.signals[0].kind, AttentionSignalKind::StressAnomaly);
    assert_eq!(
        ledger1.signals[0].evidence_digest,
        Digest32::of_bytes(b"fact_stress_350")
    );

    assert_eq!(ledger1.signals[1].subject, Some(EntityId::new(1)));
    assert_eq!(ledger1.signals[1].severity_score, 90);

    let truncated_ledger = AttentionEngine::rank_attention(&snapshot, 1, 1);
    assert_eq!(truncated_ledger.signals.len(), 1);
    assert_eq!(
        truncated_ledger.completeness,
        CompletenessStatus::BudgetTruncated
    );
    assert_eq!(truncated_ledger.signals[0].subject, Some(EntityId::new(2)));

    Ok(())
}
