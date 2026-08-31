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
}

impl SavegameArchive {
    #[must_use]
    pub fn new() -> Self {
        Self {
            blocks: BTreeMap::new(),
            snapshots: BTreeMap::new(),
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
        let mut block_digests = Vec::new();

        for chunk in serialized_bytes.chunks(BLOCK_CHUNK_SIZE) {
            let digest = Digest32::of_bytes(chunk);
            self.blocks.entry(digest).or_insert_with(|| ArchiveBlock {
                digest,
                data: chunk.to_vec(),
            });
            block_digests.push(digest);
        }

        self.snapshots
            .insert(snapshot.state_hash, block_digests.clone());
        Ok(block_digests)
    }

    /// Retrieve raw snapshot payload by assembling its constituent blocks.
    pub fn read_snapshot_payload(&self, snapshot_hash: &Digest32) -> Result<Vec<u8>> {
        let block_digests = self.snapshots.get(snapshot_hash).ok_or_else(|| {
            DfmcpError::new(ErrorCode::InvalidRequest, "snapshot not found in archive")
        })?;

        let mut payload = Vec::new();
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

        Ok(())
    }
}
