//! Shared complete citizen acquisition for the isolated semantic native shells.
//! Transport pages remain bounded independently of the complete canonical roster.

use dfmcp_adapter::semantic_workforce::evidence_owner::{
    CitizenEvidenceLimits, CitizenEvidenceSource,
};
use dfmcp_adapter::{
    BridgeManifest, LiveObservationCapsule, LiveObservationSource, ObservationPage,
    read_complete_observation_bounded,
};
use dfmcp_core::{DfmcpError, ErrorCode, OperationContext, Result};

/// Keep the complete roster within every existing 16 MiB evidence refresh.
/// At 200 citizens the adapter reserves 202 * 64 KiB local work, 1 MiB setup,
/// two 663,552-byte page envelopes and two 256 KiB policy checks: 15.390625 MiB.
/// Raising this ceiling further requires reviewing those nested reservations;
/// the transport page size, frame protocol and source policy do not change here.
pub(crate) const LIMITS: CitizenEvidenceLimits = CitizenEvidenceLimits {
    page_size: 128,
    max_citizens: 200,
};

struct Pages<'a, S, F> {
    source: &'a mut S,
    reserve: &'a mut F,
    manifest: BridgeManifest,
}
impl<S, F> LiveObservationSource for Pages<'_, S, F>
where
    S: CitizenEvidenceSource,
    F: FnMut(u64) -> Result<OperationContext>,
{
    fn bridge_manifest(&self) -> BridgeManifest {
        self.manifest.clone()
    }
    fn read_observation_page(
        &mut self,
        offset: u32,
        maximum: u32,
        names: bool,
    ) -> Result<ObservationPage> {
        // The caller retains the same shrinking operation deadline and grants.
        // This is the native shells' existing per-page wire-work reservation.
        let context = (self.reserve)(1024 * 1024)?;
        let page = self.source.read_page(offset, maximum, names, &context)?;
        if self.source.bridge_manifest() != self.manifest {
            return Err(DfmcpError::new(
                ErrorCode::CapabilityDenied,
                "citizen incarnation changed during complete acquisition",
            ));
        }
        Ok(page)
    }
}

/// Assemble every required page or publish nothing. The source remains owned by
/// the caller, which validates its operator pins and closes it before publication.
pub(crate) fn read_complete<S, F>(source: &mut S, mut reserve: F) -> Result<LiveObservationCapsule>
where
    S: CitizenEvidenceSource,
    F: FnMut(u64) -> Result<OperationContext>,
{
    let manifest = source.bridge_manifest();
    let mut pages = Pages {
        source,
        reserve: &mut reserve,
        manifest,
    };
    read_complete_observation_bounded(&mut pages, LIMITS.page_size, false, LIMITS.max_citizens)
}

#[cfg(test)]
#[path = "semantic_citizens_tests.rs"]
mod tests;
