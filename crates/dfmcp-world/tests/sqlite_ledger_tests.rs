#![forbid(unsafe_code)]

//! Integration tests for the in-memory table contract that precedes a real
//! FrankenSQLite ledger integration.

use dfmcp_core::{
    CommitState, DfmcpError, Digest32, ErrorCode, FortressId, GameTick, ObservationCursor, Result,
};
use dfmcp_world::sqlite_ledger::{SqliteLedgerConfig, SqliteProductionLedger};
use dfmcp_world::{EffectJournalRecord, ObservationCapsule, StateDelta, WorldGraph, WorldSnapshot};

fn sample_snapshot(tick: u64, cursor: ObservationCursor) -> WorldSnapshot {
    WorldSnapshot::new(
        FortressId::new(100),
        GameTick(tick),
        cursor,
        true,
        WorldGraph::default(),
    )
}

#[test]
fn test_table_prototype_capsule_and_delta_roundtrip() -> Result<()> {
    let mut ledger = SqliteProductionLedger::new(SqliteLedgerConfig::default());

    let snap_base = sample_snapshot(100, ObservationCursor::ORIGIN);
    let snap_target = sample_snapshot(
        101,
        ObservationCursor {
            epoch: 0,
            sequence: 1,
        },
    );

    let delta = StateDelta {
        fortress_id: FortressId::new(100),
        base_cursor: ObservationCursor::ORIGIN,
        target_cursor: ObservationCursor {
            epoch: 0,
            sequence: 1,
        },
        base_hash: snap_base.state_hash,
        target_hash: snap_target.state_hash,
        target_tick: GameTick(101),
        changes: Vec::new(),
        truncated: false,
        continuation: None,
    };

    let capsule = ObservationCapsule::new(
        snap_base.anchor(),
        snap_target.anchor(),
        delta.clone(),
        GameTick(101),
    )?;

    ledger.insert_snapshot(&snap_base)?;
    ledger.insert_snapshot(&snap_target)?;
    ledger.insert_delta(&delta)?;
    ledger.insert_capsule(&capsule)?;

    assert_eq!(ledger.snapshot_count(), 2);
    assert_eq!(ledger.delta_count(), 1);
    assert_eq!(ledger.capsule_count(), 1);

    let retrieved = ledger.get_capsule(&capsule.capsule_digest);
    let row = retrieved
        .ok_or_else(|| DfmcpError::new(ErrorCode::CorruptLedger, "retrieved capsule is missing"))?;
    assert_eq!(row.basis_hash, snap_base.state_hash);
    assert_eq!(row.successor_hash, snap_target.state_hash);

    assert!(ledger.verify_storage_integrity().is_ok());

    Ok(())
}

#[test]
fn test_table_prototype_effect_records() -> Result<()> {
    let mut ledger = SqliteProductionLedger::new(SqliteLedgerConfig::default());

    let effect = EffectJournalRecord {
        effect_id: "eff_12345".to_owned(),
        idempotency_key: "tx_12345".to_owned(),
        plan_digest: Digest32::of_bytes(b"plan_12345"),
        state: CommitState::Verified,
        dispatch_attempted_tick: Some(GameTick(500)),
        receipt_digest: Some(Digest32::of_bytes(b"receipt_12345")),
        observed_state_hash: Some(Digest32::ZERO),
        error_message: None,
    };

    ledger.upsert_effect(effect.clone())?;
    assert_eq!(ledger.effect_count(), 1);
    assert_eq!(ledger.get_effect("tx_12345"), Some(&effect));

    Ok(())
}
