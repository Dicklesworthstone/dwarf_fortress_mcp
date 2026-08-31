#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::path::{Component, Path};

use dfmcp_core::{
    CheckpointId, DfmcpError, Digest32, ErrorCode, FortressId, GameTick, Result, StateAnchor,
};

use crate::model::WorldSnapshot;

pub const MAX_CHECKPOINT_MANIFEST_FILES: usize = 4_096;
pub const MAX_CHECKPOINTS: usize = 65_536;
pub const MAX_CHECKPOINT_FILE_BYTES: u64 = 1024 * 1024 * 1024;
pub const MAX_CHECKPOINT_TOTAL_BYTES: u64 = 64 * 1024 * 1024 * 1024;

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
    fn new(
        checkpoint_id: CheckpointId,
        fortress_id: FortressId,
        epoch: u64,
        tick: GameTick,
        state_hash: Digest32,
        files: BTreeMap<String, (u64, Digest32)>,
    ) -> Self {
        let manifest_digest =
            compute_manifest_digest(checkpoint_id, fortress_id, epoch, tick, state_hash, &files);

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

    /// Verify that the manifest metadata still matches its sealed digest.
    #[must_use]
    pub fn integrity_is_valid(&self) -> bool {
        self.manifest_digest
            == compute_manifest_digest(
                self.checkpoint_id,
                self.fortress_id,
                self.epoch,
                self.tick,
                self.state_hash,
                &self.files,
            )
    }

    pub fn verify_files<F>(&self, mut read_file: F) -> Result<()>
    where
        F: FnMut(&str) -> Option<Vec<u8>>,
    {
        validate_manifest_shape(self)?;
        if !self.integrity_is_valid() {
            return Err(DfmcpError::new(
                ErrorCode::CorruptLedger,
                "checkpoint manifest integrity digest mismatch",
            ));
        }
        for (path, (expected_size, expected_hash)) in &self.files {
            if !valid_manifest_path(path) {
                return Err(DfmcpError::new(
                    ErrorCode::CorruptLedger,
                    format!("checkpoint manifest contains unsafe path '{path}'"),
                ));
            }
            let file_bytes = read_file(path).ok_or_else(|| {
                DfmcpError::new(
                    ErrorCode::CorruptLedger,
                    format!("checkpoint verification failed: missing file '{path}'"),
                )
            })?;

            let actual_size = u64::try_from(file_bytes.len()).map_err(|_| {
                DfmcpError::new(
                    ErrorCode::BudgetExceeded,
                    "checkpoint file length cannot be represented in the manifest format",
                )
            })?;
            if actual_size != *expected_size {
                return Err(DfmcpError::new(
                    ErrorCode::CorruptLedger,
                    format!(
                        "checkpoint verification failed: file size mismatch for '{path}' (expected {expected_size}, got {})",
                        file_bytes.len()
                    ),
                ));
            }

            let actual_hash = Digest32::of_bytes(&file_bytes);
            if actual_hash != *expected_hash {
                return Err(DfmcpError::new(
                    ErrorCode::CorruptLedger,
                    format!(
                        "checkpoint bit-rot detected: hash mismatch for '{path}' (expected {expected_hash}, got {actual_hash})"
                    ),
                ));
            }
        }
        Ok(())
    }
}

fn validate_manifest_shape(manifest: &CheckpointManifest) -> Result<()> {
    if manifest.checkpoint_id == CheckpointId::NIL
        || manifest.fortress_id == FortressId::NIL
        || manifest.state_hash == Digest32::ZERO
        || manifest.files.len() > MAX_CHECKPOINT_MANIFEST_FILES
    {
        return Err(DfmcpError::new(
            ErrorCode::CorruptLedger,
            "checkpoint manifest has invalid identity or exceeds its file-count bound",
        ));
    }
    let mut total_bytes = 0u64;
    for (path, (size, _)) in &manifest.files {
        if !valid_manifest_path(path) || *size > MAX_CHECKPOINT_FILE_BYTES {
            return Err(DfmcpError::new(
                ErrorCode::CorruptLedger,
                "checkpoint manifest contains an unsafe path or oversized file",
            ));
        }
        total_bytes = total_bytes.checked_add(*size).ok_or_else(|| {
            DfmcpError::new(
                ErrorCode::BudgetExceeded,
                "checkpoint manifest total byte count overflowed",
            )
        })?;
        if total_bytes > MAX_CHECKPOINT_TOTAL_BYTES {
            return Err(DfmcpError::new(
                ErrorCode::BudgetExceeded,
                "checkpoint manifest exceeds its total byte bound",
            ));
        }
    }
    Ok(())
}

fn valid_manifest_path(path: &str) -> bool {
    if path.is_empty()
        || path.len() > 4_096
        || path
            .chars()
            .any(|character| character == '\0' || character == '\\')
        || path.contains("//")
    {
        return false;
    }
    Path::new(path)
        .components()
        .all(|component| matches!(component, Component::Normal(_)))
}

fn compute_manifest_digest(
    checkpoint_id: CheckpointId,
    fortress_id: FortressId,
    epoch: u64,
    tick: GameTick,
    state_hash: Digest32,
    files: &BTreeMap<String, (u64, Digest32)>,
) -> Digest32 {
    let mut bytes = Vec::new();
    crate::canonical::put_str(&mut bytes, "dfmcp-checkpoint-manifest-v1");
    bytes.extend_from_slice(&checkpoint_id.get().to_be_bytes());
    crate::canonical::put_u64(&mut bytes, fortress_id.get());
    crate::canonical::put_u64(&mut bytes, epoch);
    crate::canonical::put_u64(&mut bytes, tick.0);
    crate::canonical::put_bytes(&mut bytes, state_hash.as_bytes());
    crate::canonical::put_u64(&mut bytes, files.len() as u64);
    for (path, (size, file_hash)) in files {
        crate::canonical::put_str(&mut bytes, path);
        crate::canonical::put_u64(&mut bytes, *size);
        crate::canonical::put_bytes(&mut bytes, file_hash.as_bytes());
    }
    Digest32::of_bytes(&bytes)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RestoreCertificate {
    pub fortress_id: FortressId,
    pub prior_anchor: StateAnchor,
    pub restored_anchor: StateAnchor,
    pub manifest_digest: Digest32,
    pub restored_content_digest: Digest32,
    pub certificate_digest: Digest32,
}

impl RestoreCertificate {
    #[must_use]
    pub fn new(
        fortress_id: FortressId,
        prior_anchor: StateAnchor,
        restored_anchor: StateAnchor,
        manifest_digest: Digest32,
        restored_content_digest: Digest32,
    ) -> Self {
        let certificate_digest = restore_certificate_digest(
            fortress_id,
            prior_anchor,
            restored_anchor,
            manifest_digest,
            restored_content_digest,
        );

        Self {
            fortress_id,
            prior_anchor,
            restored_anchor,
            manifest_digest,
            restored_content_digest,
            certificate_digest,
        }
    }

    #[must_use]
    pub fn integrity_is_valid(&self) -> bool {
        self.certificate_digest
            == restore_certificate_digest(
                self.fortress_id,
                self.prior_anchor,
                self.restored_anchor,
                self.manifest_digest,
                self.restored_content_digest,
            )
    }
}

fn restore_certificate_digest(
    fortress_id: FortressId,
    prior_anchor: StateAnchor,
    restored_anchor: StateAnchor,
    manifest_digest: Digest32,
    restored_content_digest: Digest32,
) -> Digest32 {
    let mut bytes = Vec::new();
    crate::canonical::put_str(&mut bytes, "dfmcp-restore-certificate-v1");
    crate::canonical::put_u64(&mut bytes, fortress_id.get());
    crate::canonical::put_anchor(&mut bytes, prior_anchor);
    crate::canonical::put_anchor(&mut bytes, restored_anchor);
    crate::canonical::put_bytes(&mut bytes, manifest_digest.as_bytes());
    crate::canonical::put_bytes(&mut bytes, restored_content_digest.as_bytes());
    Digest32::of_bytes(&bytes)
}

pub struct CheckpointStore {
    manifests: BTreeMap<CheckpointId, CheckpointManifest>,
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

    #[must_use]
    pub fn len(&self) -> usize {
        self.manifests.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.manifests.is_empty()
    }

    pub fn create_checkpoint(
        &mut self,
        snapshot: &WorldSnapshot,
        files: BTreeMap<String, (u64, Digest32)>,
    ) -> Result<CheckpointManifest> {
        if !snapshot.hash_is_valid() {
            return Err(DfmcpError::new(
                ErrorCode::InternalInvariantViolation,
                "cannot checkpoint a snapshot with an invalid state hash",
            ));
        }
        if files.len() > MAX_CHECKPOINT_MANIFEST_FILES {
            return Err(DfmcpError::new(
                ErrorCode::BudgetExceeded,
                "checkpoint manifest exceeds its explicit file-count bound",
            ));
        }
        let mut total_bytes = 0u64;
        if files.iter().any(|(path, (size, _))| {
            total_bytes = total_bytes.saturating_add(*size);
            !valid_manifest_path(path)
                || *size > MAX_CHECKPOINT_FILE_BYTES
                || total_bytes > MAX_CHECKPOINT_TOTAL_BYTES
        }) {
            return Err(DfmcpError::new(
                ErrorCode::InvalidRequest,
                "checkpoint manifest paths and declared file sizes must be safely bounded",
            ));
        }
        if self.manifests.len() >= MAX_CHECKPOINTS {
            return Err(DfmcpError::new(
                ErrorCode::BudgetExceeded,
                "checkpoint store reached its explicit manifest bound",
            ));
        }
        let cid = CheckpointId::new(u128::from(self.next_checkpoint_id));
        if self.manifests.contains_key(&cid) {
            return Err(DfmcpError::new(
                ErrorCode::InternalInvariantViolation,
                "checkpoint identifier collision",
            ));
        }
        let next_checkpoint_id = self.next_checkpoint_id.checked_add(1).ok_or_else(|| {
            DfmcpError::new(
                ErrorCode::InternalInvariantViolation,
                "checkpoint identifier space exhausted",
            )
        })?;

        let manifest = CheckpointManifest::new(
            cid,
            snapshot.fortress_id,
            snapshot.cursor.epoch,
            snapshot.tick,
            snapshot.state_hash,
            files,
        );

        self.manifests.insert(cid, manifest.clone());
        self.next_checkpoint_id = next_checkpoint_id;
        Ok(manifest)
    }

    pub fn restore_checkpoint(
        &self,
        checkpoint_id: CheckpointId,
        prior_anchor: StateAnchor,
        restored_anchor: StateAnchor,
        restored_content_digest: Digest32,
    ) -> Result<RestoreCertificate> {
        let manifest = self.manifests.get(&checkpoint_id).ok_or_else(|| {
            DfmcpError::new(
                ErrorCode::InvalidRequest,
                format!("checkpoint {checkpoint_id} not found"),
            )
        })?;

        validate_manifest_shape(manifest)?;
        if !manifest.integrity_is_valid() {
            return Err(DfmcpError::new(
                ErrorCode::CorruptLedger,
                "checkpoint manifest integrity digest mismatch",
            ));
        }
        if prior_anchor.fortress_id != manifest.fortress_id
            || restored_anchor.fortress_id != manifest.fortress_id
        {
            return Err(DfmcpError::new(
                ErrorCode::StaleAnchor,
                "checkpoint restore anchors belong to a different fortress",
            ));
        }
        let expected_epoch = prior_anchor.cursor.epoch.checked_add(1).ok_or_else(|| {
            DfmcpError::new(
                ErrorCode::CursorGap,
                "cannot restore because the observation epoch is exhausted",
            )
        })?;
        if restored_anchor.cursor.epoch != expected_epoch {
            return Err(DfmcpError::new(
                ErrorCode::CursorGap,
                "checkpoint restore must advance the observation epoch exactly once",
            ));
        }
        if restored_anchor.cursor.sequence != 0 {
            return Err(DfmcpError::new(
                ErrorCode::CursorGap,
                "checkpoint restore must reset the observation sequence",
            ));
        }
        if restored_anchor.tick != manifest.tick {
            return Err(DfmcpError::new(
                ErrorCode::StaleAnchor,
                "restored anchor tick does not match checkpoint manifest",
            ));
        }
        if restored_content_digest != manifest.state_hash {
            return Err(DfmcpError::new(
                ErrorCode::CorruptLedger,
                "restored content digest does not match checkpoint manifest",
            ));
        }

        Ok(RestoreCertificate::new(
            manifest.fortress_id,
            prior_anchor,
            restored_anchor,
            manifest.manifest_digest,
            restored_content_digest,
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
