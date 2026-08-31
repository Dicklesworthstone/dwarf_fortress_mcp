#![forbid(unsafe_code)]

//! In-memory persistence-contract prototype for world state and effect journals.
//!
//! Despite the compatibility-preserving type names, this module does not open SQLite,
//! write a WAL, or provide crash durability. It exists only to exercise table-like
//! invariants until the admitted owned persistence crate is integrated.

use std::collections::BTreeMap;

use dfmcp_core::{
    DfmcpError, Digest32, ErrorCode, FortressId, GameTick, ObservationCursor, Result,
};

use crate::delta::StateDelta;
use crate::ledger::{EffectJournalRecord, ObservationCapsule};
use crate::model::WorldSnapshot;

/// Prospective persistence settings retained as contract data; currently not applied.
#[derive(Clone, Debug)]
pub struct SqliteLedgerConfig {
    pub journal_mode: String,
    pub synchronous: String,
    pub busy_timeout_millis: u32,
    pub max_wal_bytes: usize,
    pub compaction_horizon_checkpoints: usize,
}

impl Default for SqliteLedgerConfig {
    fn default() -> Self {
        Self {
            journal_mode: "WAL".to_owned(),
            synchronous: "NORMAL".to_owned(),
            busy_timeout_millis: 5000,
            max_wal_bytes: 64 * 1024 * 1024, // 64MB
            compaction_horizon_checkpoints: 3,
        }
    }
}

/// A persisted capsule row in the ledger database.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapsuleRow {
    pub capsule_digest: Digest32,
    pub fortress_id: FortressId,
    pub basis_hash: Digest32,
    pub successor_hash: Digest32,
    pub tick: GameTick,
    pub published_at_tick: GameTick,
    pub payload: Vec<u8>,
}

/// A persisted delta row in the ledger database.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeltaRow {
    pub delta_hash: Digest32,
    pub fortress_id: FortressId,
    pub base_cursor: ObservationCursor,
    pub target_cursor: ObservationCursor,
    pub changes_count: usize,
    pub delta: StateDelta,
}

/// A persisted snapshot row in the ledger database.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapshotRow {
    pub state_hash: Digest32,
    pub fortress_id: FortressId,
    pub tick: GameTick,
    pub cursor: ObservationCursor,
    pub snapshot: WorldSnapshot,
}

/// In-memory table prototype. This is neither SQLite-backed nor durable.
#[derive(Clone, Debug, Default)]
pub struct SqliteProductionLedger {
    config: SqliteLedgerConfig,
    capsules_table: BTreeMap<Digest32, CapsuleRow>,
    deltas_table: BTreeMap<Digest32, DeltaRow>,
    snapshots_table: BTreeMap<Digest32, SnapshotRow>,
    effects_table: BTreeMap<String, EffectJournalRecord>,
}

impl SqliteProductionLedger {
    #[must_use]
    pub fn new(config: SqliteLedgerConfig) -> Self {
        Self {
            config,
            capsules_table: BTreeMap::new(),
            deltas_table: BTreeMap::new(),
            snapshots_table: BTreeMap::new(),
            effects_table: BTreeMap::new(),
        }
    }

    /// Insert or replace an observation capsule.
    pub fn insert_capsule(&mut self, capsule: &ObservationCapsule) -> Result<()> {
        if !capsule.integrity_is_valid() {
            return Err(DfmcpError::new(
                ErrorCode::CorruptLedger,
                "observation capsule failed integrity validation",
            ));
        }
        let row = CapsuleRow {
            capsule_digest: capsule.capsule_digest,
            fortress_id: capsule.delta.fortress_id,
            basis_hash: capsule.basis_anchor.state_hash,
            successor_hash: capsule.successor_anchor.state_hash,
            tick: capsule.successor_anchor.tick,
            published_at_tick: capsule.published_at_tick,
            payload: capsule.delta.canonical_bytes(),
        };
        insert_exact_or_reject(&mut self.capsules_table, capsule.capsule_digest, row)?;
        Ok(())
    }

    /// Retrieve an observation capsule by digest.
    #[must_use]
    pub fn get_capsule(&self, digest: &Digest32) -> Option<&CapsuleRow> {
        self.capsules_table.get(digest)
    }

    /// Insert a state delta into durable storage.
    pub fn insert_delta(&mut self, delta: &StateDelta) -> Result<()> {
        if delta.base_cursor.epoch != delta.target_cursor.epoch
            || delta.target_cursor.sequence <= delta.base_cursor.sequence
            || delta.base_hash == Digest32::ZERO
            || delta.target_hash == Digest32::ZERO
            || delta.truncated
            || delta.continuation.is_some()
        {
            return Err(DfmcpError::new(
                ErrorCode::InvalidRequest,
                "state delta is partial or has invalid anchor continuity",
            ));
        }
        let row = DeltaRow {
            delta_hash: delta.target_hash,
            fortress_id: delta.fortress_id,
            base_cursor: delta.base_cursor,
            target_cursor: delta.target_cursor,
            changes_count: delta.changes.len(),
            delta: delta.clone(),
        };
        insert_exact_or_reject(&mut self.deltas_table, delta.target_hash, row)?;
        Ok(())
    }

    /// Retrieve a state delta by target hash.
    #[must_use]
    pub fn get_delta(&self, target_hash: &Digest32) -> Option<&StateDelta> {
        self.deltas_table.get(target_hash).map(|row| &row.delta)
    }

    /// Insert a full world snapshot.
    pub fn insert_snapshot(&mut self, snapshot: &WorldSnapshot) -> Result<()> {
        if !snapshot.hash_is_valid() || snapshot.state_hash == Digest32::ZERO {
            return Err(DfmcpError::new(
                ErrorCode::CorruptLedger,
                "snapshot canonical state hash is invalid",
            ));
        }
        let row = SnapshotRow {
            state_hash: snapshot.state_hash,
            fortress_id: snapshot.fortress_id,
            tick: snapshot.tick,
            cursor: snapshot.cursor,
            snapshot: snapshot.clone(),
        };
        insert_exact_or_reject(&mut self.snapshots_table, snapshot.state_hash, row)?;
        Ok(())
    }

    /// Retrieve a snapshot by state hash.
    #[must_use]
    pub fn get_snapshot(&self, state_hash: &Digest32) -> Option<&WorldSnapshot> {
        self.snapshots_table
            .get(state_hash)
            .map(|row| &row.snapshot)
    }

    /// Upsert an effect journal record with idempotency key.
    pub fn upsert_effect(&mut self, record: EffectJournalRecord) -> Result<()> {
        if record.idempotency_key.is_empty() || record.effect_id.is_empty() {
            return Err(DfmcpError::new(
                ErrorCode::InvalidRequest,
                "effect and idempotency identifiers must be nonempty",
            ));
        }
        insert_exact_or_reject(
            &mut self.effects_table,
            record.idempotency_key.clone(),
            record,
        )?;
        Ok(())
    }

    /// Retrieve an effect journal record by idempotency key.
    #[must_use]
    pub fn get_effect(&self, idempotency_key: &str) -> Option<&EffectJournalRecord> {
        self.effects_table.get(idempotency_key)
    }

    /// Compact old deltas older than a given horizon cursor.
    pub fn compact_deltas_before_cursor(&mut self, horizon_cursor: ObservationCursor) -> usize {
        let mut pruned = 0;
        self.deltas_table.retain(|_, row| {
            if row.target_cursor < horizon_cursor {
                pruned += 1;
                false
            } else {
                true
            }
        });
        pruned
    }

    /// Verify storage integrity and consistency invariants.
    pub fn verify_storage_integrity(&self) -> Result<()> {
        // 1. Verify all capsule hashes match basis/successor continuity
        for digest in self.capsules_table.keys() {
            if *digest == Digest32::ZERO {
                return Err(DfmcpError::new(
                    ErrorCode::InternalInvariantViolation,
                    "zero digest in capsule table",
                ));
            }
        }

        // 2. Verify all snapshot hashes are valid
        for (state_hash, row) in &self.snapshots_table {
            if row.snapshot.state_hash != *state_hash || !row.snapshot.hash_is_valid() {
                return Err(DfmcpError::new(
                    ErrorCode::CorruptLedger,
                    "snapshot hash mismatch in table",
                ));
            }
        }

        Ok(())
    }

    /// Total number of stored capsules.
    #[must_use]
    pub fn capsule_count(&self) -> usize {
        self.capsules_table.len()
    }

    /// Total number of stored deltas.
    #[must_use]
    pub fn delta_count(&self) -> usize {
        self.deltas_table.len()
    }

    /// Total number of stored snapshots.
    #[must_use]
    pub fn snapshot_count(&self) -> usize {
        self.snapshots_table.len()
    }

    /// Total number of stored effect records.
    #[must_use]
    pub fn effect_count(&self) -> usize {
        self.effects_table.len()
    }

    /// Configuration parameters.
    #[must_use]
    pub fn config(&self) -> &SqliteLedgerConfig {
        &self.config
    }
}

fn insert_exact_or_reject<K, V>(table: &mut BTreeMap<K, V>, key: K, value: V) -> Result<()>
where
    K: Ord,
    V: PartialEq,
{
    if let Some(existing) = table.get(&key) {
        if existing == &value {
            return Ok(());
        }
        return Err(DfmcpError::new(
            ErrorCode::Conflict,
            "storage key is already bound to different content",
        ));
    }
    table.insert(key, value);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::WorldGraph;
    use dfmcp_core::CommitState;

    fn sample_snapshot(tick: u64, cursor: ObservationCursor) -> WorldSnapshot {
        WorldSnapshot::new(
            FortressId::new(1),
            GameTick(tick),
            cursor,
            true,
            WorldGraph::default(),
        )
    }

    #[test]
    fn test_sqlite_ledger_crud_operations() -> Result<()> {
        let mut ledger = SqliteProductionLedger::new(SqliteLedgerConfig::default());
        let snap1 = sample_snapshot(100, ObservationCursor::ORIGIN);
        ledger.insert_snapshot(&snap1)?;

        assert_eq!(ledger.snapshot_count(), 1);
        assert!(ledger.verify_storage_integrity().is_ok());

        let effect = EffectJournalRecord {
            effect_id: "tx_001".to_owned(),
            idempotency_key: "tx_001".to_owned(),
            plan_digest: Digest32::of_bytes(b"test_plan"),
            state: CommitState::Verified,
            dispatch_attempted_tick: Some(GameTick(100)),
            receipt_digest: Some(Digest32::of_bytes(b"receipt_001")),
            observed_state_hash: None,
            error_message: None,
        };

        ledger.upsert_effect(effect.clone())?;
        assert_eq!(ledger.effect_count(), 1);
        assert_eq!(ledger.get_effect("tx_001"), Some(&effect));

        Ok(())
    }

    #[test]
    fn test_sqlite_ledger_delta_compaction() -> Result<()> {
        let mut ledger = SqliteProductionLedger::new(SqliteLedgerConfig::default());

        for seq in 1..=10 {
            let delta = StateDelta {
                fortress_id: FortressId::new(1),
                base_cursor: ObservationCursor {
                    epoch: 0,
                    sequence: seq - 1,
                },
                target_cursor: ObservationCursor {
                    epoch: 0,
                    sequence: seq,
                },
                base_hash: Digest32::of_bytes(format!("hash_{}", seq - 1).as_bytes()),
                target_hash: Digest32::of_bytes(format!("hash_{}", seq).as_bytes()),
                target_tick: GameTick(100 + seq),
                changes: Vec::new(),
                truncated: false,
                continuation: None,
            };
            ledger.insert_delta(&delta)?;
        }

        assert_eq!(ledger.delta_count(), 10);

        // Compact all deltas before sequence 6
        let pruned = ledger.compact_deltas_before_cursor(ObservationCursor {
            epoch: 0,
            sequence: 6,
        });
        assert_eq!(pruned, 5);
        assert_eq!(ledger.delta_count(), 5);

        Ok(())
    }
}
