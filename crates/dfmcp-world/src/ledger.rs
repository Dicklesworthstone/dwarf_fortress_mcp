#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};

use dfmcp_core::{
    CommitState, DfmcpError, Digest32, EntityId, ErrorCode, FortressId, GameTick, Result,
    StateAnchor,
};

use crate::delta::StateDelta;
use crate::model::{ChunkCoord, WorldSnapshot};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObservationCapsule {
    pub basis_anchor: StateAnchor,
    pub successor_anchor: StateAnchor,
    pub delta: StateDelta,
    pub capsule_digest: Digest32,
    pub published_at_tick: GameTick,
}

impl ObservationCapsule {
    pub fn new(
        basis: StateAnchor,
        successor: StateAnchor,
        delta: StateDelta,
        published_at_tick: GameTick,
    ) -> Result<Self> {
        if basis.fortress_id != successor.fortress_id {
            return Err(DfmcpError::new(
                ErrorCode::StaleAnchor,
                "observation capsule basis and successor must belong to the same fortress",
            ));
        }
        if delta.fortress_id != basis.fortress_id {
            return Err(DfmcpError::new(
                ErrorCode::StaleAnchor,
                "delta belongs to a different fortress than the basis anchor",
            ));
        }
        if delta.base_cursor != basis.cursor {
            return Err(DfmcpError::new(
                ErrorCode::CursorGap,
                "delta base_cursor does not match basis anchor cursor",
            ));
        }
        if delta.target_cursor != successor.cursor {
            return Err(DfmcpError::new(
                ErrorCode::CursorGap,
                "delta target_cursor does not match successor anchor cursor",
            ));
        }
        if delta.base_hash != basis.state_hash {
            return Err(DfmcpError::new(
                ErrorCode::StaleAnchor,
                "delta base_hash does not match basis anchor state_hash; refusing to capsule a tampered chain",
            ));
        }
        if delta.target_hash != successor.state_hash {
            return Err(DfmcpError::new(
                ErrorCode::StaleAnchor,
                "delta target_hash does not match successor anchor state_hash; refusing to capsule an unverified transition",
            ));
        }

        let mut hasher_bytes = Vec::new();
        hasher_bytes.extend_from_slice(basis.state_hash.as_bytes());
        hasher_bytes.extend_from_slice(successor.state_hash.as_bytes());
        hasher_bytes.extend_from_slice(delta.target_hash.as_bytes());
        hasher_bytes.extend_from_slice(&published_at_tick.0.to_be_bytes());
        let capsule_digest = Digest32::of_bytes(&hasher_bytes);

        Ok(Self {
            basis_anchor: basis,
            successor_anchor: successor,
            delta,
            capsule_digest,
            published_at_tick,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct WitnessSet {
    pub positive_entities: BTreeSet<(EntityId, u32, u64)>,
    pub negative_entities: BTreeSet<EntityId>,
    pub witnessed_chunks: BTreeMap<ChunkCoord, u64>,
}

impl WitnessSet {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_positive_entity(&mut self, id: EntityId, generation: u32, revision: u64) {
        self.positive_entities.insert((id, generation, revision));
    }

    pub fn add_negative_entity(&mut self, id: EntityId) {
        self.negative_entities.insert(id);
    }

    pub fn add_chunk(&mut self, coord: ChunkCoord, revision: u64) {
        self.witnessed_chunks.insert(coord, revision);
    }

    pub fn validate_against_delta(&self, delta: &StateDelta) -> Result<()> {
        for change in &delta.changes {
            match change {
                crate::WorldChange::UpsertEntity(entity_record) => {
                    if self.negative_entities.contains(&entity_record.id) {
                        return Err(DfmcpError::new(
                            ErrorCode::Conflict,
                            format!(
                                "negative-read phantom conflict: entity {} was asserted absent but was inserted",
                                entity_record.id
                            ),
                        ));
                    }

                    for (w_id, w_gen, w_rev) in &self.positive_entities {
                        if entity_record.id == *w_id {
                            if entity_record.generation != *w_gen {
                                return Err(DfmcpError::new(
                                    ErrorCode::Conflict,
                                    format!(
                                        "ABA generation conflict: entity {} generation changed from {} to {}",
                                        w_id, w_gen, entity_record.generation
                                    ),
                                ));
                            }
                            if entity_record.revision != *w_rev {
                                return Err(DfmcpError::new(
                                    ErrorCode::Conflict,
                                    format!(
                                        "revision conflict: entity {} revision changed from {} to {}",
                                        w_id, w_rev, entity_record.revision
                                    ),
                                ));
                            }
                        }
                    }
                }
                crate::WorldChange::RemoveEntity { id: deleted_id, .. }
                    if self
                        .positive_entities
                        .iter()
                        .any(|(id, _, _)| id == deleted_id) =>
                {
                    return Err(DfmcpError::new(
                        ErrorCode::Conflict,
                        format!("conflict: witnessed positive entity {deleted_id} was deleted"),
                    ));
                }
                _ => {}
            }
        }

        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EffectJournalRecord {
    pub effect_id: String,
    pub idempotency_key: String,
    pub plan_digest: Digest32,
    pub state: CommitState,
    pub dispatch_attempted_tick: Option<GameTick>,
    pub receipt_digest: Option<Digest32>,
    pub observed_state_hash: Option<Digest32>,
    pub error_message: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DurableLedger {
    pub fortress_id: FortressId,
    pub epoch: u64,
    pub root_snapshot: WorldSnapshot,
    pub capsules: Vec<ObservationCapsule>,
    pub effects: BTreeMap<String, EffectJournalRecord>,
    pub unpublished_capsule: Option<ObservationCapsule>,
}

impl DurableLedger {
    pub fn new(root_snapshot: WorldSnapshot) -> Self {
        let fortress_id = root_snapshot.fortress_id;
        let epoch = root_snapshot.cursor.epoch;
        Self {
            fortress_id,
            epoch,
            root_snapshot,
            capsules: Vec::new(),
            effects: BTreeMap::new(),
            unpublished_capsule: None,
        }
    }

    #[must_use]
    pub fn head_anchor(&self) -> StateAnchor {
        if let Some(last) = self.capsules.last() {
            last.successor_anchor
        } else {
            self.root_snapshot.anchor()
        }
    }

    pub fn stage_capsule(&mut self, capsule: ObservationCapsule) -> Result<()> {
        let current_head = self.head_anchor();
        if capsule.basis_anchor.fortress_id != self.fortress_id {
            return Err(DfmcpError::new(
                ErrorCode::StaleAnchor,
                "staged capsule belongs to a different fortress than the ledger root",
            ));
        }
        if capsule.basis_anchor.cursor != current_head.cursor {
            return Err(DfmcpError::new(
                ErrorCode::CursorGap,
                "staged capsule basis does not match ledger head anchor cursor",
            ));
        }
        if capsule.basis_anchor.state_hash != current_head.state_hash {
            return Err(DfmcpError::new(
                ErrorCode::CursorGap,
                "staged capsule basis state_hash does not match ledger head state_hash",
            ));
        }

        self.unpublished_capsule = Some(capsule);
        Ok(())
    }

    pub fn publish_staged(&mut self) -> Result<StateAnchor> {
        let capsule = self.unpublished_capsule.take().ok_or_else(|| {
            DfmcpError::new(
                ErrorCode::InvalidRequest,
                "no staged capsule available to publish",
            )
        })?;

        // Re-validate the staged capsule still matches the current head. If
        // someone moved the head between stage_capsule and publish_staged,
        // we must NOT publish a stale capsule on top of the new head — that
        // would silently fork the ledger.
        let current_head = self.head_anchor();
        if capsule.basis_anchor.cursor != current_head.cursor
            || capsule.basis_anchor.state_hash != current_head.state_hash
        {
            // Restore the staged capsule so the caller can reconcile it.
            self.unpublished_capsule = Some(capsule);
            return Err(DfmcpError::new(
                ErrorCode::CursorGap,
                "ledger head moved between stage_capsule and publish_staged; refusing to publish a stale capsule",
            ));
        }

        let successor = capsule.successor_anchor;
        self.capsules.push(capsule);
        Ok(successor)
    }

    pub fn abort_staged(&mut self) {
        self.unpublished_capsule = None;
    }

    pub fn record_dispatch_attempt(
        &mut self,
        effect_id: impl Into<String>,
        idempotency_key: impl Into<String>,
        plan_digest: Digest32,
        tick: GameTick,
    ) {
        let eff_id = effect_id.into();
        let key = idempotency_key.into();
        self.effects.insert(
            key.clone(),
            EffectJournalRecord {
                effect_id: eff_id,
                idempotency_key: key,
                plan_digest,
                state: CommitState::Prepared,
                dispatch_attempted_tick: Some(tick),
                receipt_digest: None,
                observed_state_hash: None,
                error_message: None,
            },
        );
    }

    pub fn record_commit_receipt(
        &mut self,
        idempotency_key: &str,
        receipt_digest: Digest32,
    ) -> Result<()> {
        let effect = self.effects.get_mut(idempotency_key).ok_or_else(|| {
            DfmcpError::new(
                ErrorCode::InvalidRequest,
                format!("unknown idempotency key '{idempotency_key}'"),
            )
        })?;
        effect.state = CommitState::Verified;
        effect.receipt_digest = Some(receipt_digest);
        Ok(())
    }

    pub fn record_indeterminate(
        &mut self,
        idempotency_key: &str,
        message: impl Into<String>,
    ) -> Result<()> {
        let effect = self.effects.get_mut(idempotency_key).ok_or_else(|| {
            DfmcpError::new(
                ErrorCode::InvalidRequest,
                format!("unknown idempotency key '{idempotency_key}'"),
            )
        })?;
        effect.state = CommitState::Indeterminate;
        effect.error_message = Some(message.into());
        Ok(())
    }

    pub fn recover_from_crash(&mut self) {
        self.unpublished_capsule = None;

        for effect in self.effects.values_mut() {
            if effect.state == CommitState::Prepared
                && effect.dispatch_attempted_tick.is_some()
                && effect.receipt_digest.is_none()
            {
                effect.state = CommitState::Indeterminate;
                effect.error_message = Some(
                    "Recovery: dispatch outcome unknown prior to crash; reconciliation required"
                        .to_owned(),
                );
            }
        }
    }
}
