#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};

use dfmcp_core::{
    CommitState, DfmcpError, Digest32, EntityId, ErrorCode, FortressId, GameTick, Result,
    StateAnchor,
};

use crate::delta::{MAX_STATE_DELTA_CHANGES, StateDelta, apply_delta};
use crate::model::{ChunkCoord, WorldSnapshot};

const MAX_LEDGER_CAPSULES: usize = 65_536;
const MAX_LEDGER_EFFECTS: usize = 65_536;
const MAX_EFFECT_ID_BYTES: usize = 256;
const MAX_EFFECT_ERROR_BYTES: usize = 4_096;

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
        if delta.target_tick != successor.tick || successor.tick < basis.tick {
            return Err(DfmcpError::new(
                ErrorCode::StaleAnchor,
                "delta target tick does not match a monotonic successor anchor",
            ));
        }
        if delta.changes.len() > MAX_STATE_DELTA_CHANGES
            || delta.truncated
            || delta.continuation.is_some()
            || delta.target_cursor.epoch != delta.base_cursor.epoch
            || delta.target_cursor.sequence <= delta.base_cursor.sequence
        {
            return Err(DfmcpError::new(
                ErrorCode::CursorGap,
                "partial, unbounded, or non-advancing deltas cannot be sealed as complete observation capsules",
            ));
        }
        if published_at_tick < successor.tick {
            return Err(DfmcpError::new(
                ErrorCode::InvalidRequest,
                "observation capsule publication tick precedes its successor state",
            ));
        }

        let capsule_digest = compute_capsule_digest(basis, successor, &delta, published_at_tick);

        Ok(Self {
            basis_anchor: basis,
            successor_anchor: successor,
            delta,
            capsule_digest,
            published_at_tick,
        })
    }

    /// Verify the capsule's digest and internal anchor continuity.
    #[must_use]
    pub fn integrity_is_valid(&self) -> bool {
        self.basis_anchor.fortress_id == self.successor_anchor.fortress_id
            && self.delta.fortress_id == self.basis_anchor.fortress_id
            && self.delta.base_cursor == self.basis_anchor.cursor
            && self.delta.target_cursor == self.successor_anchor.cursor
            && self.delta.base_hash == self.basis_anchor.state_hash
            && self.delta.target_hash == self.successor_anchor.state_hash
            && self.delta.target_tick == self.successor_anchor.tick
            && self.successor_anchor.tick >= self.basis_anchor.tick
            && self.delta.changes.len() <= MAX_STATE_DELTA_CHANGES
            && self.delta.target_cursor.epoch == self.delta.base_cursor.epoch
            && self.delta.target_cursor.sequence > self.delta.base_cursor.sequence
            && !self.delta.truncated
            && self.delta.continuation.is_none()
            && self.published_at_tick >= self.successor_anchor.tick
            && self.capsule_digest
                == compute_capsule_digest(
                    self.basis_anchor,
                    self.successor_anchor,
                    &self.delta,
                    self.published_at_tick,
                )
    }
}

fn compute_capsule_digest(
    basis: StateAnchor,
    successor: StateAnchor,
    delta: &StateDelta,
    published_at_tick: GameTick,
) -> Digest32 {
    let mut bytes = Vec::new();
    crate::canonical::put_str(&mut bytes, "dfmcp-observation-capsule-v1");
    crate::canonical::put_anchor(&mut bytes, basis);
    crate::canonical::put_anchor(&mut bytes, successor);
    crate::canonical::put_bytes(&mut bytes, &delta.canonical_bytes());
    crate::canonical::put_u64(&mut bytes, published_at_tick.0);
    Digest32::of_bytes(&bytes)
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
    fortress_id: FortressId,
    epoch: u64,
    root_snapshot: WorldSnapshot,
    head_snapshot: WorldSnapshot,
    capsules: Vec<ObservationCapsule>,
    effects: BTreeMap<String, EffectJournalRecord>,
    unpublished_capsule: Option<ObservationCapsule>,
}

impl DurableLedger {
    pub fn new(root_snapshot: WorldSnapshot) -> Self {
        let fortress_id = root_snapshot.fortress_id;
        let epoch = root_snapshot.cursor.epoch;
        let head_snapshot = root_snapshot.clone();
        Self {
            fortress_id,
            epoch,
            root_snapshot,
            head_snapshot,
            capsules: Vec::new(),
            effects: BTreeMap::new(),
            unpublished_capsule: None,
        }
    }

    #[must_use]
    pub fn head_anchor(&self) -> StateAnchor {
        self.head_snapshot.anchor()
    }

    #[must_use]
    pub const fn fortress_id(&self) -> FortressId {
        self.fortress_id
    }

    #[must_use]
    pub const fn epoch(&self) -> u64 {
        self.epoch
    }

    #[must_use]
    pub const fn root_snapshot(&self) -> &WorldSnapshot {
        &self.root_snapshot
    }

    #[must_use]
    pub const fn head_snapshot(&self) -> &WorldSnapshot {
        &self.head_snapshot
    }

    #[must_use]
    pub fn capsule_count(&self) -> usize {
        self.capsules.len()
    }

    #[must_use]
    pub const fn has_staged_capsule(&self) -> bool {
        self.unpublished_capsule.is_some()
    }

    #[must_use]
    pub fn effect(&self, idempotency_key: &str) -> Option<&EffectJournalRecord> {
        self.effects.get(idempotency_key)
    }

    pub fn stage_capsule(&mut self, capsule: ObservationCapsule) -> Result<()> {
        if !capsule.integrity_is_valid() {
            return Err(DfmcpError::new(
                ErrorCode::CorruptLedger,
                "staged observation capsule failed integrity validation",
            ));
        }
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
        if capsule.basis_anchor.tick != current_head.tick {
            return Err(DfmcpError::new(
                ErrorCode::StaleAnchor,
                "staged capsule basis tick does not match ledger head tick",
            ));
        }
        let reconstructed = apply_delta(&self.head_snapshot, &capsule.delta).map_err(|error| {
            DfmcpError::new(
                ErrorCode::CorruptLedger,
                format!("staged capsule delta does not reconstruct its successor: {error}"),
            )
        })?;
        if reconstructed.anchor() != capsule.successor_anchor {
            return Err(DfmcpError::new(
                ErrorCode::CorruptLedger,
                "staged capsule successor anchor does not match its reconstructed state",
            ));
        }

        if let Some(existing) = &self.unpublished_capsule {
            if existing == &capsule {
                return Ok(());
            }
            return Err(DfmcpError::new(
                ErrorCode::Conflict,
                "a different observation capsule is already staged",
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

        if !capsule.integrity_is_valid() {
            self.unpublished_capsule = Some(capsule);
            return Err(DfmcpError::new(
                ErrorCode::CorruptLedger,
                "staged observation capsule failed publication-time integrity validation",
            ));
        }
        if self.capsules.len() >= MAX_LEDGER_CAPSULES {
            self.unpublished_capsule = Some(capsule);
            return Err(DfmcpError::new(
                ErrorCode::BudgetExceeded,
                "observation capsule ledger reached its explicit bound",
            ));
        }

        // Re-validate the staged capsule still matches the current head. If
        // someone moved the head between stage_capsule and publish_staged,
        // we must NOT publish a stale capsule on top of the new head — that
        // would silently fork the ledger.
        let current_head = self.head_anchor();
        if capsule.basis_anchor != current_head {
            // Restore the staged capsule so the caller can reconcile it.
            self.unpublished_capsule = Some(capsule);
            return Err(DfmcpError::new(
                ErrorCode::CursorGap,
                "ledger head moved between stage_capsule and publish_staged; refusing to publish a stale capsule",
            ));
        }

        let successor_snapshot = match apply_delta(&self.head_snapshot, &capsule.delta) {
            Ok(snapshot) if snapshot.anchor() == capsule.successor_anchor => snapshot,
            Ok(_) => {
                self.unpublished_capsule = Some(capsule);
                return Err(DfmcpError::new(
                    ErrorCode::CorruptLedger,
                    "staged capsule successor changed before publication",
                ));
            }
            Err(error) => {
                self.unpublished_capsule = Some(capsule);
                return Err(DfmcpError::new(
                    ErrorCode::CorruptLedger,
                    format!("staged capsule failed publication-time reconstruction: {error}"),
                ));
            }
        };
        let successor = capsule.successor_anchor;
        self.capsules.push(capsule);
        self.head_snapshot = successor_snapshot;
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
    ) -> Result<()> {
        let eff_id = effect_id.into();
        let key = idempotency_key.into();
        if eff_id.is_empty()
            || key.is_empty()
            || eff_id.len() > MAX_EFFECT_ID_BYTES
            || key.len() > MAX_EFFECT_ID_BYTES
        {
            return Err(DfmcpError::new(
                ErrorCode::InvalidRequest,
                "effect and idempotency identifiers must be nonempty and bounded",
            ));
        }
        if let Some(existing) = self.effects.get(&key) {
            if existing.effect_id == eff_id && existing.plan_digest == plan_digest {
                return Ok(());
            }
            return Err(DfmcpError::new(
                ErrorCode::Conflict,
                "idempotency key is already bound to a different effect or plan",
            ));
        }
        if self.effects.len() >= MAX_LEDGER_EFFECTS {
            return Err(DfmcpError::new(
                ErrorCode::BudgetExceeded,
                "effect journal reached its explicit record bound",
            ));
        }
        self.effects.insert(
            key.clone(),
            EffectJournalRecord {
                effect_id: eff_id,
                idempotency_key: key,
                plan_digest,
                state: CommitState::Committing,
                dispatch_attempted_tick: Some(tick),
                receipt_digest: None,
                observed_state_hash: None,
                error_message: None,
            },
        );
        Ok(())
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
        if matches!(
            effect.state,
            CommitState::AppliedAwaitingVerification | CommitState::Verified
        ) && effect.receipt_digest == Some(receipt_digest)
        {
            return Ok(());
        }
        if effect.state != CommitState::Committing {
            return Err(DfmcpError::new(
                ErrorCode::Conflict,
                "commit receipt is invalid for the effect's current state",
            ));
        }
        effect.state = CommitState::AppliedAwaitingVerification;
        effect.receipt_digest = Some(receipt_digest);
        Ok(())
    }

    /// Record authoritative post-state observation after a commit receipt.
    pub fn record_verified(
        &mut self,
        idempotency_key: &str,
        observed_state_hash: Digest32,
    ) -> Result<()> {
        if observed_state_hash == Digest32::ZERO {
            return Err(DfmcpError::new(
                ErrorCode::InvalidRequest,
                "verified effect requires a nonzero observed state hash",
            ));
        }
        let effect = self.effects.get_mut(idempotency_key).ok_or_else(|| {
            DfmcpError::new(
                ErrorCode::InvalidRequest,
                format!("unknown idempotency key '{idempotency_key}'"),
            )
        })?;
        if effect.state == CommitState::Verified
            && effect.observed_state_hash == Some(observed_state_hash)
        {
            return Ok(());
        }
        if effect.state != CommitState::AppliedAwaitingVerification
            || effect.receipt_digest.is_none()
        {
            return Err(DfmcpError::new(
                ErrorCode::Conflict,
                "effect cannot be verified before its commit receipt",
            ));
        }
        effect.observed_state_hash = Some(observed_state_hash);
        effect.state = CommitState::Verified;
        Ok(())
    }

    pub fn record_indeterminate(
        &mut self,
        idempotency_key: &str,
        message: impl Into<String>,
    ) -> Result<()> {
        let message = message.into();
        if message.len() > MAX_EFFECT_ERROR_BYTES {
            return Err(DfmcpError::new(
                ErrorCode::BudgetExceeded,
                "effect error message exceeds its explicit byte bound",
            ));
        }
        let effect = self.effects.get_mut(idempotency_key).ok_or_else(|| {
            DfmcpError::new(
                ErrorCode::InvalidRequest,
                format!("unknown idempotency key '{idempotency_key}'"),
            )
        })?;
        if effect.state == CommitState::Indeterminate
            && effect.error_message.as_deref() == Some(message.as_str())
        {
            return Ok(());
        }
        if !matches!(
            effect.state,
            CommitState::Committing | CommitState::AppliedAwaitingVerification
        ) {
            return Err(DfmcpError::new(
                ErrorCode::Conflict,
                "only an unresolved in-flight effect can become indeterminate",
            ));
        }
        effect.state = CommitState::Indeterminate;
        effect.error_message = Some(message);
        Ok(())
    }

    pub fn recover_from_crash(&mut self) {
        self.unpublished_capsule = None;

        for effect in self.effects.values_mut() {
            if effect.state == CommitState::Committing
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
