#![forbid(unsafe_code)]

//! Assemble immutable native bytes, never independently timed world fragments.

use dfmcp_core::{DfmcpError, Digest32, ErrorCode, Result};

pub const MIN_PAGE_BYTES: usize = 16 * 1024;
pub const MAX_PAGE_BYTES: usize = 256 * 1024;
pub const MAX_SNAPSHOT_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapshotManifest {
    pub token: [u8; 16],
    pub generation: u64,
    pub df_version: String,
    pub dfhack_version: String,
    pub total_bytes: usize,
    pub payload_digest: Digest32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapshotPage {
    pub manifest: SnapshotManifest,
    pub offset: usize,
    pub bytes: Vec<u8>,
    pub complete: bool,
}

pub struct SnapshotAssembler {
    maximum: usize,
    page_bytes: usize,
    manifest: Option<SnapshotManifest>,
    bytes: Vec<u8>,
    complete: bool,
    failed: bool,
}

fn invalid(text: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::AdapterRejected, text)
}

impl SnapshotAssembler {
    pub fn new(maximum: usize, page_bytes: usize) -> Result<Self> {
        if !(1..=MAX_SNAPSHOT_BYTES).contains(&maximum)
            || !(MIN_PAGE_BYTES..=MAX_PAGE_BYTES).contains(&page_bytes)
        {
            return Err(DfmcpError::new(ErrorCode::BudgetExceeded, "invalid native snapshot or page bound"));
        }
        Ok(Self { maximum, page_bytes, manifest: None, bytes: Vec::new(), complete: false, failed: false })
    }

    pub fn offset(&self) -> usize { self.bytes.len() }
    pub fn manifest(&self) -> Option<&SnapshotManifest> { self.manifest.as_ref() }
    pub fn complete(&self) -> bool { self.complete && !self.failed }

    /// A failed page permanently poisons this assembly. No partial result is exposed.
    pub fn push(&mut self, page: SnapshotPage) -> Result<()> {
        let result = self.accept(page);
        if result.is_err() {
            self.failed = true;
            self.complete = false;
            self.bytes.clear();
        }
        result
    }

    fn accept(&mut self, page: SnapshotPage) -> Result<()> {
        if self.failed || self.complete {
            return Err(invalid("native snapshot assembly is already terminal"));
        }
        let manifest = &page.manifest;
        if manifest.generation == 0 || manifest.token == [0; 16]
            || manifest.total_bytes == 0 || manifest.total_bytes > self.maximum
            || manifest.payload_digest == Digest32::ZERO
        {
            return Err(invalid("invalid native snapshot manifest"));
        }
        for text in [&manifest.df_version, &manifest.dfhack_version] {
            if text.is_empty() || text.len() > 128 || text.contains('\0') {
                return Err(invalid("invalid native snapshot software identity"));
            }
        }
        if self.manifest.as_ref().is_some_and(|prior| prior != manifest) {
            return Err(DfmcpError::new(ErrorCode::StaleAnchor, "native snapshot identity changed between pages"));
        }
        if page.offset != self.bytes.len() || page.bytes.is_empty() || page.bytes.len() > self.page_bytes {
            return Err(invalid("native snapshot page offset, progress, or width is invalid"));
        }
        let end = page.offset.checked_add(page.bytes.len()).ok_or_else(|| invalid("native page length overflow"))?;
        if end > manifest.total_bytes || page.complete != (end == manifest.total_bytes)
            || (!page.complete && page.bytes.len() != self.page_bytes)
        {
            return Err(invalid("native snapshot page completeness does not match its byte range"));
        }
        if self.manifest.is_none() {
            self.bytes.try_reserve_exact(manifest.total_bytes).map_err(|_| {
                DfmcpError::new(ErrorCode::BudgetExceeded, "cannot reserve bounded native snapshot storage")
            })?;
            self.manifest = Some(page.manifest);
        }
        self.bytes.extend_from_slice(&page.bytes);
        self.complete = page.complete;
        Ok(())
    }

    /// Whole-payload SHA-256 is checked before the semantic decoder can run.
    pub fn finish(self) -> Result<(SnapshotManifest, Vec<u8>)> {
        if !self.complete || self.failed {
            return Err(invalid("native snapshot is incomplete or poisoned"));
        }
        let manifest = self.manifest.ok_or_else(|| invalid("native snapshot has no manifest"))?;
        if self.bytes.len() != manifest.total_bytes || Digest32::of_bytes(&self.bytes) != manifest.payload_digest {
            return Err(invalid("native snapshot whole-payload digest mismatch"));
        }
        Ok((manifest, self.bytes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(data: &[u8], offset: usize, width: usize) -> SnapshotPage {
        let end = offset.saturating_add(width).min(data.len());
        SnapshotPage {
            manifest: SnapshotManifest { token: [1; 16], generation: 7,
                df_version: "df".to_owned(), dfhack_version: "dfhack".to_owned(),
                total_bytes: data.len(), payload_digest: Digest32::of_bytes(data) },
            offset, bytes: data[offset..end].to_vec(), complete: end == data.len(),
        }
    }

    #[test]
    fn pages_reconstruct_exactly_across_every_final_page_width() -> Result<()> {
        for tail in [1, 55, 56, 63, 64, MIN_PAGE_BYTES - 1, MIN_PAGE_BYTES] {
            let data = vec![42; 2 * MIN_PAGE_BYTES + tail];
            let mut assembly = SnapshotAssembler::new(data.len(), MIN_PAGE_BYTES)?;
            while !assembly.complete() {
                assembly.push(page(&data, assembly.offset(), MIN_PAGE_BYTES))?;
            }
            assert_eq!(assembly.finish()?.1, data);
        }
        Ok(())
    }

    #[test]
    fn all_manifest_components_are_fixed_before_publication() -> Result<()> {
        let data = vec![42; MIN_PAGE_BYTES * 2];
        for change in 0..6 {
            let mut assembly = SnapshotAssembler::new(data.len(), MIN_PAGE_BYTES)?;
            assembly.push(page(&data, 0, MIN_PAGE_BYTES))?;
            let mut next = page(&data, MIN_PAGE_BYTES, MIN_PAGE_BYTES);
            match change {
                0 => next.manifest.token[0] = 2,
                1 => next.manifest.generation += 1,
                2 => next.manifest.df_version.push('x'),
                3 => next.manifest.dfhack_version.push('x'),
                4 => next.manifest.total_bytes -= 1,
                _ => next.manifest.payload_digest = Digest32::of_bytes(b"different"),
            }
            assert!(assembly.push(next).is_err());
            assert!(assembly.push(page(&data, MIN_PAGE_BYTES, MIN_PAGE_BYTES)).is_err());
            assert!(assembly.finish().is_err());
        }
        Ok(())
    }

    #[test]
    fn gaps_overlaps_nonprogress_and_false_completeness_are_rejected() -> Result<()> {
        let data = vec![42; MIN_PAGE_BYTES * 3];
        for change in 0..6 {
            let mut assembly = SnapshotAssembler::new(data.len(), MIN_PAGE_BYTES)?;
            assembly.push(page(&data, 0, MIN_PAGE_BYTES))?;
            let mut next = page(&data, MIN_PAGE_BYTES, MIN_PAGE_BYTES);
            match change {
                0 => next.offset = 0,
                1 => next.offset += 1,
                2 => next.bytes.clear(),
                3 => { next.bytes.pop(); }
                4 => next.bytes.push(42),
                _ => next.complete = true,
            }
            assert!(assembly.push(next).is_err());
            assert!(assembly.finish().is_err());
        }
        Ok(())
    }

    #[test]
    fn complete_bytes_still_require_the_native_digest() -> Result<()> {
        let data = vec![42; MIN_PAGE_BYTES];
        let mut assembly = SnapshotAssembler::new(data.len(), MIN_PAGE_BYTES)?;
        let mut changed = page(&data, 0, MIN_PAGE_BYTES);
        changed.bytes[7] ^= 1;
        assembly.push(changed)?;
        assert!(assembly.finish().is_err());
        Ok(())
    }

    #[test]
    fn oversized_manifests_and_partial_finish_fail_without_success() -> Result<()> {
        let data = vec![42; MIN_PAGE_BYTES * 2];
        let mut assembly = SnapshotAssembler::new(MIN_PAGE_BYTES, MIN_PAGE_BYTES)?;
        assert!(assembly.push(page(&data, 0, MIN_PAGE_BYTES)).is_err());
        assert_eq!(assembly.offset(), 0);
        let mut assembly = SnapshotAssembler::new(data.len(), MIN_PAGE_BYTES)?;
        assembly.push(page(&data, 0, MIN_PAGE_BYTES))?;
        assert!(assembly.finish().is_err());
        assert!(SnapshotAssembler::new(MAX_SNAPSHOT_BYTES + 1, MIN_PAGE_BYTES).is_err());
        assert!(SnapshotAssembler::new(data.len(), 1).is_err());
        Ok(())
    }
}
