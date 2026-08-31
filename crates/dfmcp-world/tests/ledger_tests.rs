#![forbid(unsafe_code)]

use std::collections::BTreeMap;

use dfmcp_core::{
    CommitState, Digest32, EntityId, ErrorCode, FortressId, GameTick, ObservationCursor,
};
use dfmcp_world::{
    DurableLedger, EntityKind, EntityRecord, ObservationCapsule, WitnessSet, WorldGraph,
    WorldSnapshot, diff_snapshots,
};

fn make_sample_snapshot(tick: u64, cursor: ObservationCursor) -> WorldSnapshot {
    WorldSnapshot::new(
        FortressId::new(10),
        GameTick(tick),
        cursor,
        false,
        WorldGraph::default(),
    )
}

/// TEST-016: Crash-Point Matrix and Recovery Semantics
#[test]
fn test_016_crash_point_matrix_and_recovery() -> Result<(), Box<dyn std::error::Error>> {
    let base_cursor = ObservationCursor {
        epoch: 1,
        sequence: 1,
    };
    let base_snapshot = make_sample_snapshot(100, base_cursor);
    let mut ledger = DurableLedger::new(base_snapshot.clone());

    let target_cursor = base_cursor.next();
    let mut target_snapshot = make_sample_snapshot(105, target_cursor);
    target_snapshot.paused = true;
    target_snapshot.refresh_hash();

    let delta = diff_snapshots(&base_snapshot, &target_snapshot)?;
    let capsule = ObservationCapsule::new(
        base_snapshot.anchor(),
        target_snapshot.anchor(),
        delta.clone(),
        GameTick(105),
    )?;

    ledger.stage_capsule(capsule)?;
    assert!(ledger.unpublished_capsule.is_some());

    ledger.recover_from_crash();
    assert!(ledger.unpublished_capsule.is_none());
    assert_eq!(ledger.head_anchor(), base_snapshot.anchor());

    let capsule2 = ObservationCapsule::new(
        base_snapshot.anchor(),
        target_snapshot.anchor(),
        delta,
        GameTick(105),
    )?;
    ledger.stage_capsule(capsule2)?;
    let published_anchor = ledger.publish_staged()?;
    assert_eq!(published_anchor, target_snapshot.anchor());
    assert_eq!(ledger.head_anchor(), target_snapshot.anchor());

    let eff_key = "effect_001";
    ledger.record_dispatch_attempt(
        "eff_1",
        eff_key,
        Digest32::of_bytes(b"plan_1"),
        GameTick(106),
    );

    ledger.recover_from_crash();

    let recovered_effect = ledger.effects.get(eff_key).ok_or("effect missing")?;
    assert_eq!(recovered_effect.state, CommitState::Indeterminate);
    assert!(
        recovered_effect
            .error_message
            .as_deref()
            .unwrap_or("")
            .contains("reconciliation required")
    );

    let eff_key2 = "effect_002";
    ledger.record_dispatch_attempt(
        "eff_2",
        eff_key2,
        Digest32::of_bytes(b"plan_2"),
        GameTick(107),
    );
    ledger.record_commit_receipt(eff_key2, Digest32::of_bytes(b"receipt_2"))?;

    ledger.recover_from_crash();

    let recovered_committed = ledger.effects.get(eff_key2).ok_or("effect missing")?;
    assert_eq!(recovered_committed.state, CommitState::Verified);

    Ok(())
}

/// Witness Validation & Negative-Read Phantom Insertion Protection
#[test]
fn test_witness_negative_phantom_and_aba_protection() -> Result<(), Box<dyn std::error::Error>> {
    let mut witnesses = WitnessSet::new();

    witnesses.add_positive_entity(EntityId::new(1), 1, 5);
    witnesses.add_negative_entity(EntityId::new(99));

    let base = make_sample_snapshot(100, ObservationCursor::ORIGIN);

    let phantom_entity = EntityRecord {
        id: EntityId::new(99),
        generation: 1,
        revision: 1,
        kind: EntityKind::Unit,
        label: "Phantom".to_owned(),
        fields: BTreeMap::new(),
    };
    let phantom_delta = dfmcp_world::build_delta(
        &base,
        base.cursor.next(),
        GameTick(101),
        vec![dfmcp_world::WorldChange::UpsertEntity(phantom_entity)],
    )?;

    let Err(phantom_err) = witnesses.validate_against_delta(&phantom_delta) else {
        return Err("expected phantom conflict".into());
    };
    assert_eq!(phantom_err.code, ErrorCode::Conflict);
    assert!(phantom_err.message.contains("phantom"));

    let aba_entity = EntityRecord {
        id: EntityId::new(1),
        generation: 2,
        revision: 5,
        kind: EntityKind::Unit,
        label: "ABA".to_owned(),
        fields: BTreeMap::new(),
    };
    let aba_delta = dfmcp_world::build_delta(
        &base,
        base.cursor.next(),
        GameTick(101),
        vec![dfmcp_world::WorldChange::UpsertEntity(aba_entity)],
    )?;

    let Err(aba_err) = witnesses.validate_against_delta(&aba_delta) else {
        return Err("expected ABA generation conflict".into());
    };
    assert_eq!(aba_err.code, ErrorCode::Conflict);
    assert!(aba_err.message.contains("generation"));

    Ok(())
}
