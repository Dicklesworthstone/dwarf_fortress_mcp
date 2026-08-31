#![forbid(unsafe_code)]

use std::collections::BTreeMap;

use dfmcp_core::{
    CheckpointId, DfmcpError, Digest32, ErrorCode, FortressId, GameTick, Result, StateAnchor,
};

use crate::model::WorldSnapshot;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckpointManifest {
    pub checkpoint_id: CheckpointId,
    pub fortress_id: FortressId,
    pub epoch: u64,
    pub tick: GameTick,
    pub state_hash: Digest32,
    pub files: BTreeMap<String, (u64, Digest32)>,
    pub manifest_digest: Digest32,
}

impl CheckpointManifest {
    pub fn new(
        checkpoint_id: CheckpointId,
        fortress_id: FortressId,
        epoch: u64,
        tick: GameTick,
        state_hash: Digest32,
        files: BTreeMap<String, (u64, Digest32)>,
    ) -> Self {
        let mut hasher_bytes = Vec::new();
        hasher_bytes.extend_from_slice(&checkpoint_id.get().to_be_bytes());
        hasher_bytes.extend_from_slice(&fortress_id.get().to_be_bytes());
        hasher_bytes.extend_from_slice(&epoch.to_be_bytes());
        hasher_bytes.extend_from_slice(&tick.0.to_be_bytes());
        hasher_bytes.extend_from_slice(state_hash.as_bytes());

        for (path, (size, fhash)) in &files {
            hasher_bytes.extend_from_slice(path.as_bytes());
            hasher_bytes.extend_from_slice(&size.to_be_bytes());
            hasher_bytes.extend_from_slice(fhash.as_bytes());
        }

        let manifest_digest = Digest32::of_bytes(&hasher_bytes);

        Self {
            checkpoint_id,
            fortress_id,
            epoch,
            tick,
            state_hash,
            files,
            manifest_digest,
        }
    }

    pub fn verify_files<F>(&self, mut read_file: F) -> Result<()>
    where
        F: FnMut(&str) -> Option<Vec<u8>>,
    {
        for (path, (expected_size, expected_hash)) in &self.files {
            let file_bytes = read_file(path).ok_or_else(|| {
                DfmcpError::new(
                    ErrorCode::InternalInvariantViolation,
                    format!("checkpoint verification failed: missing file '{path}'"),
                )
            })?;

            if file_bytes.len() as u64 != *expected_size {
                return Err(DfmcpError::new(
                    ErrorCode::InternalInvariantViolation,
                    format!(
                        "checkpoint verification failed: file size mismatch for '{path}' (expected {expected_size}, got {})",
                        file_bytes.len()
                    ),
                ));
            }

            let actual_hash = Digest32::of_bytes(&file_bytes);
            if actual_hash != *expected_hash {
                return Err(DfmcpError::new(
                    ErrorCode::InternalInvariantViolation,
                    format!(
                        "checkpoint bit-rot detected: hash mismatch for '{path}' (expected {expected_hash}, got {actual_hash})"
                    ),
                ));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RestoreCertificate {
    pub fortress_id: FortressId,
    pub prior_anchor: StateAnchor,
    pub restored_anchor: StateAnchor,
    pub manifest_digest: Digest32,
    pub certificate_digest: Digest32,
}

impl RestoreCertificate {
    #[must_use]
    pub fn new(
        fortress_id: FortressId,
        prior_anchor: StateAnchor,
        restored_anchor: StateAnchor,
        manifest_digest: Digest32,
    ) -> Self {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&fortress_id.get().to_be_bytes());
        bytes.extend_from_slice(prior_anchor.state_hash.as_bytes());
        bytes.extend_from_slice(restored_anchor.state_hash.as_bytes());
        bytes.extend_from_slice(manifest_digest.as_bytes());
        let certificate_digest = Digest32::of_bytes(&bytes);

        Self {
            fortress_id,
            prior_anchor,
            restored_anchor,
            manifest_digest,
            certificate_digest,
        }
    }
}

pub struct CheckpointStore {
    pub manifests: BTreeMap<CheckpointId, CheckpointManifest>,
    next_checkpoint_id: u64,
}

impl Default for CheckpointStore {
    fn default() -> Self {
        Self::new()
    }
}

impl CheckpointStore {
    #[must_use]
    pub fn new() -> Self {
        Self {
            manifests: BTreeMap::new(),
            next_checkpoint_id: 1,
        }
    }

    #[must_use]
    pub fn get(&self, id: CheckpointId) -> Option<&CheckpointManifest> {
        self.manifests.get(&id)
    }

    pub fn create_checkpoint(
        &mut self,
        snapshot: &WorldSnapshot,
        files: BTreeMap<String, (u64, Digest32)>,
    ) -> CheckpointManifest {
        let cid = CheckpointId::new(u128::from(self.next_checkpoint_id));
        self.next_checkpoint_id = self.next_checkpoint_id.wrapping_add(1);

        let manifest = CheckpointManifest::new(
            cid,
            snapshot.fortress_id,
            snapshot.cursor.epoch,
            snapshot.tick,
            snapshot.state_hash,
            files,
        );

        self.manifests.insert(cid, manifest.clone());
        manifest
    }

    pub fn restore_checkpoint(
        &self,
        checkpoint_id: CheckpointId,
        prior_anchor: StateAnchor,
        restored_anchor: StateAnchor,
    ) -> Result<RestoreCertificate> {
        let manifest = self.manifests.get(&checkpoint_id).ok_or_else(|| {
            DfmcpError::new(
                ErrorCode::InvalidRequest,
                format!("checkpoint {checkpoint_id} not found"),
            )
        })?;

        Ok(RestoreCertificate::new(
            manifest.fortress_id,
            prior_anchor,
            restored_anchor,
            manifest.manifest_digest,
        ))
    }

    pub fn validate_anchor_epoch(anchor: &StateAnchor, current_epoch: u64) -> Result<()> {
        if anchor.cursor.epoch != current_epoch {
            return Err(DfmcpError::new(
                ErrorCode::CursorGap,
                format!(
                    "stale epoch rejection: anchor epoch {} does not match current epoch {current_epoch} (restore occurred)",
                    anchor.cursor.epoch
                ),
            ));
        }
        Ok(())
    }
}
