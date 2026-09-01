#![forbid(unsafe_code)]

//! Atomic protocol-1.1 observation assembly.
//!
//! Citizen pagination is transport detail. Every page in one logical
//! observation must reproduce the exact same announcement batch, fortress
//! summary, bridge generation, and name projection. A moving-world multipage
//! read is rejected before publication, matching the protocol-1.0 coherence
//! rule. The final capsule binds the already-audited citizen capsule and the
//! canonical announcement batch into one identity.

use dfmcp_core::{DfmcpError, Digest32, ErrorCode, Result};

use crate::{
    BridgeManifest, LiveAnnouncementBatch, LiveObservationCapsule, ObservationAssembler,
    ObservationPage, ObservationPageV1_1,
};

const CAPSULE_V1_1_DOMAIN: &[u8] = b"dfmcp.live-observation-capsule.v3\0";
pub const MAX_CANONICAL_CAPSULE_V1_1_BYTES: usize = 66 * 1024 * 1024;

fn error(code: ErrorCode, message: impl Into<String>) -> DfmcpError {
    DfmcpError::new(code, message)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveObservationCapsuleV1_1 {
    pub base: LiveObservationCapsule,
    pub announcement_batch: LiveAnnouncementBatch,
    pub canonical_bytes: Vec<u8>,
    pub content_digest: Digest32,
}

impl LiveObservationCapsuleV1_1 {
    pub fn validate(&self) -> Result<()> {
        self.base.validate()?;
        self.announcement_batch.validate()?;
        if self.base.bridge.bridge_generation != self.announcement_batch.bridge_generation
            || self.base.paused != self.announcement_batch.paused
            || self.base.current_year != self.announcement_batch.current_year
            || self.base.current_year_tick != self.announcement_batch.current_year_tick
            || self.base.site_id != self.announcement_batch.site_id
        {
            return Err(error(
                ErrorCode::CorruptLedger,
                "protocol-1.1 citizen and announcement evidence describe different observation instants",
            ));
        }
        let reproduced = canonical_bytes(&self.base, &self.announcement_batch)?;
        if reproduced != self.canonical_bytes
            || Digest32::of_bytes(&self.canonical_bytes) != self.content_digest
        {
            return Err(error(
                ErrorCode::CorruptLedger,
                "protocol-1.1 capsule fields do not reproduce their canonical identity",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct ObservationAssemblerV1_1 {
    base: ObservationAssembler,
    announcement_batch: Option<LiveAnnouncementBatch>,
    complete: bool,
}

impl ObservationAssemblerV1_1 {
    #[must_use]
    pub fn new(bridge: BridgeManifest) -> Self {
        Self::with_names(bridge, true)
    }

    #[must_use]
    pub fn with_names(bridge: BridgeManifest, names_included: bool) -> Self {
        Self {
            base: ObservationAssembler::with_names(bridge, names_included),
            announcement_batch: None,
            complete: false,
        }
    }

    pub fn next_offset(&self) -> Result<u32> {
        self.base.next_offset()
    }

    #[must_use]
    pub const fn is_complete(&self) -> bool {
        self.complete
    }

    pub fn push_page(&mut self, page: ObservationPageV1_1) -> Result<()> {
        if self.complete {
            return Err(error(
                ErrorCode::InvalidRequest,
                "cannot append a page after complete protocol-1.1 coverage",
            ));
        }
        page.announcement_batch.validate()?;
        if page.bridge_generation != page.announcement_batch.bridge_generation
            || page.paused != page.announcement_batch.paused
            || page.current_year != page.announcement_batch.current_year
            || page.current_year_tick != page.announcement_batch.current_year_tick
            || page.site_id != page.announcement_batch.site_id
        {
            return Err(error(
                ErrorCode::AdapterRejected,
                "protocol-1.1 page and announcement batch have different summary identity",
            ));
        }
        if !page.complete && !page.paused {
            return Err(error(
                ErrorCode::PreconditionsFailed,
                "protocol 1.1 cannot assemble a coherent multipage observation while Dwarf Fortress is running",
            )
            .retryable(true));
        }
        if let Some(expected) = self.announcement_batch.as_ref()
            && expected != &page.announcement_batch
        {
            return Err(error(
                ErrorCode::StaleAnchor,
                "announcement evidence changed between citizen pages",
            ));
        }

        let complete = page.complete;
        let announcement_batch = page.announcement_batch;
        let base_page = ObservationPage {
            bridge_generation: page.bridge_generation,
            world_loaded: page.world_loaded,
            fortress_mode: page.fortress_mode,
            paused: page.paused,
            current_year: page.current_year,
            current_year_tick: page.current_year_tick,
            world_name: page.world_name,
            world_folder: page.world_folder,
            site_id: page.site_id,
            citizen_count_total: page.citizen_count_total,
            citizen_offset: page.citizen_offset,
            complete,
            citizens: page.citizens,
        };
        self.base.push_page(base_page)?;
        if self.announcement_batch.is_none() {
            self.announcement_batch = Some(announcement_batch);
        }
        self.complete = complete;
        Ok(())
    }

    pub fn finalize(self) -> Result<LiveObservationCapsuleV1_1> {
        if !self.complete {
            return Err(error(
                ErrorCode::CursorGap,
                "cannot publish an incomplete protocol-1.1 observation",
            ));
        }
        let base = self.base.finalize()?;
        let announcement_batch = self.announcement_batch.ok_or_else(|| {
            error(
                ErrorCode::InternalInvariantViolation,
                "complete protocol-1.1 assembly lost its announcement batch",
            )
        })?;
        let canonical_bytes = canonical_bytes(&base, &announcement_batch)?;
        let content_digest = Digest32::of_bytes(&canonical_bytes);
        let capsule = LiveObservationCapsuleV1_1 {
            base,
            announcement_batch,
            canonical_bytes,
            content_digest,
        };
        capsule.validate()?;
        Ok(capsule)
    }
}

fn push_bytes(output: &mut Vec<u8>, value: &[u8]) -> Result<()> {
    let length = u64::try_from(value.len()).map_err(|_| {
        error(
            ErrorCode::BudgetExceeded,
            "protocol-1.1 canonical component length does not fit u64",
        )
    })?;
    output.extend_from_slice(&length.to_le_bytes());
    output.extend_from_slice(value);
    Ok(())
}

fn canonical_bytes(
    base: &LiveObservationCapsule,
    announcements: &LiveAnnouncementBatch,
) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    output.extend_from_slice(CAPSULE_V1_1_DOMAIN);
    push_bytes(&mut output, &base.canonical_bytes)?;
    push_bytes(&mut output, &announcements.canonical_bytes)?;
    if output.len() > MAX_CANONICAL_CAPSULE_V1_1_BYTES {
        return Err(error(
            ErrorCode::BudgetExceeded,
            "canonical protocol-1.1 observation exceeds its 66 MiB ceiling",
        ));
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::{
        AnnouncementContinuity, AnnouncementCoverage, AnnouncementRecord, CitizenRecord,
    };

    fn manifest() -> BridgeManifest {
        BridgeManifest {
            bridge_version: "0.2.0".to_owned(),
            dfhack_version: "0.51.11-r1".to_owned(),
            df_version: "0.51.11".to_owned(),
            world_loaded: true,
            fortress_mode: true,
            bridge_generation: 42,
            supported_methods: BTreeSet::from([
                "Handshake".to_owned(),
                "ReadObservation".to_owned(),
            ]),
        }
    }

    fn citizen(unit_id: i32) -> CitizenRecord {
        CitizenRecord {
            unit_id,
            name: format!("Urist {unit_id}"),
            race: "dwarf".to_owned(),
            profession: 4,
            x: unit_id,
            y: 2,
            z: 3,
            alive: true,
            sane: true,
            active: true,
            visible: true,
            citizen: true,
            resident: false,
            baby: false,
            child: false,
            adult: true,
        }
    }

    fn announcement_batch(after: i32, text: &str) -> Result<LiveAnnouncementBatch> {
        let record = AnnouncementRecord {
            report_id: 10,
            report_type: 7,
            text: text.to_owned(),
            year: 105,
            year_tick: 12_340,
            repeat_count: 0,
            continuation: false,
            unconscious: false,
            announcement: true,
        };
        LiveAnnouncementBatch::new(
            42,
            true,
            105,
            12_345,
            7,
            AnnouncementCoverage {
                requested_after_id: after,
                oldest_available_id: 10,
                latest_available_id: 10,
                returned: 1,
                complete_through_latest: true,
                continuity: if after < 9 {
                    AnnouncementContinuity::GapBeforeRetainedWindow
                } else {
                    AnnouncementContinuity::CompleteSuffix
                },
                next_after_id: 10,
            },
            vec![record],
        )
    }

    fn page(
        offset: u32,
        total: u32,
        ids: &[i32],
        complete: bool,
        announcements: LiveAnnouncementBatch,
    ) -> ObservationPageV1_1 {
        ObservationPageV1_1 {
            bridge_generation: 42,
            world_loaded: true,
            fortress_mode: true,
            paused: true,
            current_year: 105,
            current_year_tick: 12_345,
            world_name: "The Balanced Realm".to_owned(),
            world_folder: "region1".to_owned(),
            site_id: 7,
            citizen_count_total: total,
            citizen_offset: offset,
            complete,
            citizens: ids.iter().copied().map(citizen).collect(),
            announcement_batch: announcements,
        }
    }

    #[test]
    fn pagination_does_not_change_combined_capsule_identity() -> Result<()> {
        let announcements = announcement_batch(9, "A caravan has arrived")?;
        let mut one = ObservationAssemblerV1_1::new(manifest());
        one.push_page(page(0, 3, &[1, 2, 3], true, announcements.clone()))?;
        let one = one.finalize()?;

        let mut many = ObservationAssemblerV1_1::new(manifest());
        many.push_page(page(0, 3, &[1, 2], false, announcements.clone()))?;
        many.push_page(page(2, 3, &[3], true, announcements))?;
        let many = many.finalize()?;
        assert_eq!(one.canonical_bytes, many.canonical_bytes);
        assert_eq!(one.content_digest, many.content_digest);
        Ok(())
    }

    #[test]
    fn announcement_drift_between_pages_is_transactionally_rejected() -> Result<()> {
        let mut assembler = ObservationAssemblerV1_1::new(manifest());
        assembler.push_page(page(
            0,
            2,
            &[1],
            false,
            announcement_batch(9, "first")?,
        ))?;
        let offset = assembler.next_offset()?;
        assert!(
            assembler
                .push_page(page(
                    1,
                    2,
                    &[2],
                    true,
                    announcement_batch(9, "changed")?,
                ))
                .is_err()
        );
        assert_eq!(assembler.next_offset()?, offset);
        assert!(!assembler.is_complete());
        Ok(())
    }

    #[test]
    fn moving_multipage_observation_is_rejected_before_mutation() -> Result<()> {
        let announcements = announcement_batch(9, "stable")?;
        let mut first = page(0, 2, &[1], false, announcements);
        first.paused = false;
        first.announcement_batch.paused = false;
        let mut assembler = ObservationAssemblerV1_1::new(manifest());
        assert!(assembler.push_page(first).is_err());
        assert_eq!(assembler.next_offset()?, 0);
        Ok(())
    }

    #[test]
    fn combined_capsule_tampering_fails_closed() -> Result<()> {
        let mut assembler = ObservationAssemblerV1_1::new(manifest());
        assembler.push_page(page(
            0,
            1,
            &[1],
            true,
            announcement_batch(9, "stable")?,
        ))?;
        let mut capsule = assembler.finalize()?;
        capsule.announcement_batch.announcements[0]
            .text
            .push('!');
        assert!(capsule.validate().is_err());
        Ok(())
    }

    #[test]
    fn retained_window_gap_survives_combined_capsule() -> Result<()> {
        let mut assembler = ObservationAssemblerV1_1::new(manifest());
        assembler.push_page(page(
            0,
            1,
            &[1],
            true,
            announcement_batch(1, "retained suffix")?,
        ))?;
        let capsule = assembler.finalize()?;
        assert!(capsule.announcement_batch.coverage.has_gap());
        capsule.validate()
    }

    #[test]
    fn incomplete_assembly_cannot_publish() -> Result<()> {
        let mut assembler = ObservationAssemblerV1_1::new(manifest());
        assembler.push_page(page(
            0,
            2,
            &[1],
            false,
            announcement_batch(9, "stable")?,
        ))?;
        assert!(assembler.finalize().is_err());
        Ok(())
    }
}
