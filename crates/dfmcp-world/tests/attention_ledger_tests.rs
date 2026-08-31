#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::error::Error;

use dfmcp_core::{CommitState, Digest32, EntityId, FortressId, GameTick, ObservationCursor};
use dfmcp_world::{
    AttentionEngine, AttentionSignalKind, CompletenessStatus, DurableLedger, EntityKind,
    EntityRecord, Fact, FactSource, Value, WitnessSet, WorldGraph, WorldSnapshot, diff_snapshots,
};

fn make_test_snapshot() -> WorldSnapshot {
    let mut graph = WorldGraph::default();
    let mut fields = BTreeMap::new();
    fields.insert(
        "stress".to_owned(),
        Fact::known(
            Value::I64(95),
            GameTick(100),
            FactSource::Replay,
            Digest32::of_bytes(b"stress_fact_95"),
        ),
    );

    let dwarf = EntityRecord {
        id: EntityId::new(1),
        generation: 1,
        revision: 1,
        kind: EntityKind::Unit,
        label: "Urist McMiner".to_owned(),
        fields,
    };
    graph.entities.insert(EntityId::new(1), dwarf);

    WorldSnapshot::new(
        FortressId::new(1),
        GameTick(100),
        ObservationCursor::ORIGIN,
        false,
        graph,
    )
}

/// TEST-016: Attention Signal Ranking & Deterministic Relevance Ledger (WP-16)
#[test]
fn test_attention_engine_signal_ranking_and_ledger() {
    let snapshot = make_test_snapshot();

    let ledger = AttentionEngine::rank_attention(&snapshot, 1, 10);
    assert_eq!(ledger.generation, 1);
    assert_eq!(ledger.completeness, CompletenessStatus::Complete);
    assert_eq!(ledger.signals.len(), 1);

    let signal = &ledger.signals[0];
    assert_eq!(signal.kind, AttentionSignalKind::StressAnomaly);
    assert_eq!(signal.subject, Some(EntityId::new(1)));
    assert_eq!(signal.severity_score, 95);
    assert!(signal.summary.contains("Urist McMiner"));
    assert_eq!(
        signal.evidence_digest,
        Digest32::of_bytes(b"stress_fact_95")
    );

    // Verify deterministic digest across replays
    let ledger2 = AttentionEngine::rank_attention(&snapshot, 1, 10);
    assert_eq!(ledger.ledger_digest, ledger2.ledger_digest);
}

/// Attention Engine Budget Truncation
#[test]
fn test_attention_engine_budget_truncation() {
    let snapshot = make_test_snapshot();
    let ledger = AttentionEngine::rank_attention(&snapshot, 1, 0); // Limit 0 signals
    assert_eq!(ledger.completeness, CompletenessStatus::BudgetTruncated);
    assert_eq!(ledger.signals.len(), 0);
}

/// Durable Ledger: Crash Recovery & Indeterminate Reconciliation (WP-11)
#[test]
fn test_durable_ledger_crash_recovery_and_witness_validation() -> Result<(), Box<dyn Error>> {
    let snapshot = make_test_snapshot();
    let mut ledger = DurableLedger::new(snapshot.clone());

    // 1. Record dispatch attempt
    ledger.record_dispatch_attempt(
        "eff_01",
        "idemp_key_01",
        Digest32::of_bytes(b"plan_dig"),
        GameTick(105),
    );
    let initial_rec = ledger.effects.get("idemp_key_01").ok_or("effect missing")?;
    assert_eq!(initial_rec.state, CommitState::Prepared);

    // 2. Simulate crash before receipt arrived: must recover as Indeterminate
    ledger.recover_from_crash();
    let rec = ledger.effects.get("idemp_key_01").ok_or("effect missing")?;
    assert_eq!(rec.state, CommitState::Indeterminate);
    let err_msg = rec.error_message.as_ref().ok_or("error message missing")?;
    assert!(err_msg.contains("reconciliation required"));

    // 3. Witness set: Phantom insertion detection (negative-read protection)
    let mut witnesses = WitnessSet::new();
    witnesses.add_negative_entity(EntityId::new(99)); // Entity 99 asserted absent

    let mut new_snapshot = snapshot.clone();
    new_snapshot.cursor = new_snapshot.cursor.next();
    new_snapshot.graph.entities.insert(
        EntityId::new(99),
        EntityRecord {
            id: EntityId::new(99),
            generation: 1,
            revision: 1,
            kind: EntityKind::Unit,
            label: "Phantom Dwarf".to_owned(),
            fields: BTreeMap::new(),
        },
    );
    new_snapshot.refresh_hash();

    let delta = diff_snapshots(&snapshot, &new_snapshot)?;

    let Err(witness_err) = witnesses.validate_against_delta(&delta) else {
        return Err("expected witness conflict error".into());
    };
    assert_eq!(witness_err.code, dfmcp_core::ErrorCode::Conflict);

    Ok(())
}
