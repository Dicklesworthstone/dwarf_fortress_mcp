#![forbid(unsafe_code)]

//! In-memory content-addressed snapshot archive and bit-rot laboratory.
//!
//! This prototype exercises canonical chunking and verification semantics. It is not
//! durable storage and is not an integration with FrankenFS.

use std::collections::BTreeMap;

use dfmcp_core::{DfmcpError, Digest32, ErrorCode, Result};

use crate::model::WorldSnapshot;

/// Standard block chunk size for content-addressed savegame deduplication (64KB).
pub const BLOCK_CHUNK_SIZE: usize = 64 * 1024;
pub const MAX_ARCHIVE_SNAPSHOTS: usize = 4_096;
pub const MAX_ARCHIVE_STORED_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_ARCHIVE_PAYLOAD_BYTES: usize = 64 * 1024 * 1024;

/// Content-addressed binary storage block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArchiveBlock {
    pub digest: Digest32,
    pub data: Vec<u8>,
}

/// Verification report produced by the bit-rot scrubber.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScrubReport {
    pub total_blocks: usize,
    pub verified_blocks: usize,
    pub corrupt_blocks: Vec<Digest32>,
    pub is_clean: bool,
}

/// In-memory content-addressed world-snapshot archive.
#[derive(Clone, Debug, Default)]
pub struct SavegameArchive {
    blocks: BTreeMap<Digest32, ArchiveBlock>,
    snapshots: BTreeMap<Digest32, Vec<Digest32>>, // snapshot_hash -> [block_digests]
    stored_bytes: usize,
}

impl SavegameArchive {
    #[must_use]
    pub fn new() -> Self {
        Self {
            blocks: BTreeMap::new(),
            snapshots: BTreeMap::new(),
            stored_bytes: 0,
        }
    }

    /// Store a world snapshot into content-addressed deduplicated blocks.
    pub fn store_snapshot(&mut self, snapshot: &WorldSnapshot) -> Result<Vec<Digest32>> {
        if !snapshot.hash_is_valid() {
            return Err(DfmcpError::new(
                ErrorCode::InternalInvariantViolation,
                "cannot archive a snapshot with an invalid canonical state hash",
            ));
        }
        let serialized_bytes = snapshot.canonical_bytes();
        if serialized_bytes.len() > MAX_ARCHIVE_PAYLOAD_BYTES {
            return Err(DfmcpError::new(
                ErrorCode::BudgetExceeded,
                "snapshot exceeds the in-memory archive payload bound",
            ));
        }
        if let Some(existing) = self.snapshots.get(&snapshot.state_hash) {
            if self.read_snapshot_payload(&snapshot.state_hash)? != serialized_bytes {
                return Err(DfmcpError::new(
                    ErrorCode::CorruptLedger,
                    "archived snapshot digest is bound to different canonical bytes",
                ));
            }
            return Ok(existing.clone());
        }
        if self.snapshots.len() >= MAX_ARCHIVE_SNAPSHOTS {
            return Err(DfmcpError::new(
                ErrorCode::BudgetExceeded,
                "in-memory archive reached its explicit snapshot bound",
            ));
        }
        let mut block_digests = Vec::new();
        let mut new_blocks: Vec<(Digest32, Vec<u8>)> = Vec::new();
        let mut additional_bytes = 0usize;

        for chunk in serialized_bytes.chunks(BLOCK_CHUNK_SIZE) {
            let digest = Digest32::of_bytes(chunk);
            if let Some(existing) = self.blocks.get(&digest) {
                if existing.digest != digest || existing.data.as_slice() != chunk {
                    return Err(DfmcpError::new(
                        ErrorCode::CorruptLedger,
                        "content-addressed archive block conflicts with existing bytes",
                    ));
                }
            } else if let Some((_, pending_data)) =
                new_blocks.iter().find(|entry| entry.0 == digest)
            {
                if pending_data.as_slice() != chunk {
                    return Err(DfmcpError::new(
                        ErrorCode::CorruptLedger,
                        "two archive blocks produced the same digest for different bytes",
                    ));
                }
            } else {
                additional_bytes = additional_bytes.checked_add(chunk.len()).ok_or_else(|| {
                    DfmcpError::new(
                        ErrorCode::BudgetExceeded,
                        "archive stored-byte count overflowed",
                    )
                })?;
                new_blocks.push((digest, chunk.to_vec()));
            }
            block_digests.push(digest);
        }

        let next_stored_bytes =
            self.stored_bytes
                .checked_add(additional_bytes)
                .ok_or_else(|| {
                    DfmcpError::new(
                        ErrorCode::BudgetExceeded,
                        "archive stored-byte count overflowed",
                    )
                })?;
        if next_stored_bytes > MAX_ARCHIVE_STORED_BYTES {
            return Err(DfmcpError::new(
                ErrorCode::BudgetExceeded,
                "in-memory archive reached its explicit stored-byte bound",
            ));
        }
        for (digest, data) in new_blocks {
            self.blocks.insert(digest, ArchiveBlock { digest, data });
        }

        self.snapshots
            .insert(snapshot.state_hash, block_digests.clone());
        self.stored_bytes = next_stored_bytes;
        Ok(block_digests)
    }

    /// Retrieve raw snapshot payload by assembling its constituent blocks.
    pub fn read_snapshot_payload(&self, snapshot_hash: &Digest32) -> Result<Vec<u8>> {
        let block_digests = self.snapshots.get(snapshot_hash).ok_or_else(|| {
            DfmcpError::new(ErrorCode::InvalidRequest, "snapshot not found in archive")
        })?;

        let mut payload_bytes = 0usize;
        for digest in block_digests {
            let block = self.blocks.get(digest).ok_or_else(|| {
                DfmcpError::new(
                    ErrorCode::InternalInvariantViolation,
                    format!("missing block in archive with digest {:?}", digest),
                )
            })?;
            if Digest32::of_bytes(&block.data) != *digest || block.digest != *digest {
                return Err(DfmcpError::new(
                    ErrorCode::CorruptLedger,
                    format!("corrupt archive block detected while reading {digest}"),
                ));
            }
            payload_bytes = payload_bytes.checked_add(block.data.len()).ok_or_else(|| {
                DfmcpError::new(
                    ErrorCode::BudgetExceeded,
                    "archive payload byte count overflowed",
                )
            })?;
            if payload_bytes > MAX_ARCHIVE_PAYLOAD_BYTES {
                return Err(DfmcpError::new(
                    ErrorCode::BudgetExceeded,
                    "archive payload exceeds its explicit read bound",
                ));
            }
        }

        let mut payload = Vec::with_capacity(payload_bytes);
        for digest in block_digests {
            let block = self.blocks.get(digest).ok_or_else(|| {
                DfmcpError::new(
                    ErrorCode::InternalInvariantViolation,
                    format!("missing block in archive with digest {digest}"),
                )
            })?;
            payload.extend_from_slice(&block.data);
        }

        Ok(payload)
    }

    /// Corrupt a block for testing and chaos injection.
    pub fn inject_corruption(&mut self, digest: &Digest32) {
        if let Some(block) = self.blocks.get_mut(digest)
            && let Some(first_byte) = block.data.first_mut()
        {
            *first_byte ^= 0xFF;
        }
    }

    /// Total unique blocks stored in archive.
    #[must_use]
    pub fn block_count(&self) -> usize {
        self.blocks.len()
    }

    /// Total snapshots archived.
    #[must_use]
    pub fn snapshot_count(&self) -> usize {
        self.snapshots.len()
    }

    #[must_use]
    pub const fn stored_bytes(&self) -> usize {
        self.stored_bytes
    }
}

/// Bit-Rot Scrubber verifying cryptographic block integrity.
#[derive(Clone, Debug, Default)]
pub struct SavegameScrubber;

impl SavegameScrubber {
    /// Perform a full cryptographic scrub of all archive blocks.
    #[must_use]
    pub fn scrub_archive(&self, archive: &SavegameArchive) -> ScrubReport {
        let mut verified_count = 0;
        let mut corrupt_blocks = Vec::new();

        for (digest, block) in &archive.blocks {
            let actual_digest = Digest32::of_bytes(&block.data);
            if actual_digest == *digest {
                verified_count += 1;
            } else {
                corrupt_blocks.push(*digest);
            }
        }

        let is_clean = corrupt_blocks.is_empty();
        ScrubReport {
            total_blocks: archive.blocks.len(),
            verified_blocks: verified_count,
            corrupt_blocks,
            is_clean,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::WorldGraph;
    use dfmcp_core::{FortressId, GameTick, ObservationCursor};

    fn sample_snapshot(tick: u64) -> WorldSnapshot {
        WorldSnapshot::new(
            FortressId::new(1),
            GameTick(tick),
            ObservationCursor {
                epoch: 0,
                sequence: tick,
            },
            true,
            WorldGraph::default(),
        )
    }

    #[test]
    fn test_savegame_archive_and_scrubber_pass() -> Result<()> {
        let mut archive = SavegameArchive::new();
        let snap1 = sample_snapshot(100);
        let snap2 = sample_snapshot(101);

        let blocks1 = archive.store_snapshot(&snap1)?;
        let blocks2 = archive.store_snapshot(&snap2)?;

        assert!(!blocks1.is_empty());
        assert!(!blocks2.is_empty());
        assert_eq!(archive.snapshot_count(), 2);

        let scrubber = SavegameScrubber;
        let report = scrubber.scrub_archive(&archive);

        assert!(report.is_clean);
        assert_eq!(report.corrupt_blocks.len(), 0);
        assert_eq!(report.verified_blocks, report.total_blocks);

        // Inject bit-rot into a block
        let corrupt_target = blocks1[0];
        archive.inject_corruption(&corrupt_target);

        let report_after_rot = scrubber.scrub_archive(&archive);
        assert!(!report_after_rot.is_clean);
        assert_eq!(report_after_rot.corrupt_blocks, vec![corrupt_target]);
        assert!(archive.store_snapshot(&snap1).is_err());

        Ok(())
    }
}
