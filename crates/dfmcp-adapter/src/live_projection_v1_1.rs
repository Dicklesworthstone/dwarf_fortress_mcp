#![forbid(unsafe_code)]

//! Deterministic protocol-1.1 projection of citizens and retained announcements.
//!
//! The protocol-1.0 citizen projection remains the semantic foundation. This
//! layer merges source-bound announcement entities into the same canonical
//! snapshot and adds coverage that distinguishes a complete retained suffix
//! from complete historical coverage. No announcement observation grants or
//! implies mutation authority.

use std::collections::BTreeSet;

use dfmcp_core::{
    CoverageDomain, CoverageReport, CoverageStatus, DfmcpError, Digest32, ErrorCode,
    FortressId, ObservationCursor, Result, StateAnchor,
};
use dfmcp_world::{WorldGraph, WorldSnapshot};

use crate::{
    LiveAnnouncementProjection, LiveObservationCapsuleV1_1, LiveWorldProjection,
    project_live_announcement_batch, project_live_capsule,
};

pub const LIVE_PROJECTION_V1_1_SCHEMA: &str = "dfmcp.live_world_projection/3";
const ANNOUNCEMENT_SUFFIX_DOMAIN: &str = "fortress.announcements.retained_suffix";
const ANNOUNCEMENT_HISTORY_DOMAIN: &str = "fortress.announcements.history";

fn error(code: ErrorCode, message: impl Into<String>) -> DfmcpError {
    DfmcpError::new(code, message)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveProjectionReceiptV1_1 {
    schema: &'static str,
    source_capsule_digest: Digest32,
    source_citizen_capsule_digest: Digest32,
    source_announcement_batch_digest: Digest32,
    source_bridge_generation: u64,
    snapshot_anchor: StateAnchor,
    coverage: CoverageReport,
}

impl LiveProjectionReceiptV1_1 {
    #[must_use]
    pub const fn schema(&self) -> &'static str {
        self.schema
    }

    #[must_use]
    pub const fn source_capsule_digest(&self) -> Digest32 {
        self.source_capsule_digest
    }

    #[must_use]
    pub const fn source_citizen_capsule_digest(&self) -> Digest32 {
        self.source_citizen_capsule_digest
    }

    #[must_use]
    pub const fn source_announcement_batch_digest(&self) -> Digest32 {
        self.source_announcement_batch_digest
    }

    #[must_use]
    pub const fn source_bridge_generation(&self) -> u64 {
        self.source_bridge_generation
    }

    #[must_use]
    pub const fn snapshot_anchor(&self) -> StateAnchor {
        self.snapshot_anchor
    }

    #[must_use]
    pub const fn coverage(&self) -> &CoverageReport {
        &self.coverage
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveWorldProjectionV1_1 {
    pub snapshot: WorldSnapshot,
    pub receipt: LiveProjectionReceiptV1_1,
}

impl LiveWorldProjectionV1_1 {
    pub fn validate_against(&self, capsule: &LiveObservationCapsuleV1_1) -> Result<()> {
        capsule.validate()?;
        if !self.snapshot.hash_is_valid() {
            return Err(error(
                ErrorCode::CorruptLedger,
                "protocol-1.1 live snapshot hash is invalid",
            ));
        }
        if self.receipt.schema != LIVE_PROJECTION_V1_1_SCHEMA
            || self.receipt.source_capsule_digest != capsule.content_digest
            || self.receipt.source_citizen_capsule_digest != capsule.base.content_digest
            || self.receipt.source_announcement_batch_digest
                != capsule.announcement_batch.content_digest
            || self.receipt.source_bridge_generation
                != capsule.base.bridge.bridge_generation
            || self.receipt.snapshot_anchor != self.snapshot.anchor()
        {
            return Err(error(
                ErrorCode::CorruptLedger,
                "protocol-1.1 projection receipt does not bind its sources and snapshot",
            ));
        }
        self.receipt.coverage.validate()?;
        if self.receipt.coverage.anchor != Some(self.snapshot.anchor()) {
            return Err(error(
                ErrorCode::CorruptLedger,
                "protocol-1.1 coverage is not bound to the snapshot anchor",
            ));
        }

        let base = project_live_capsule(
            &capsule.base,
            self.snapshot.fortress_id,
            self.snapshot.cursor,
        )?;
        let announcements = project_live_announcement_batch(
            &capsule.announcement_batch,
            self.snapshot.tick,
            entity_generation(self.snapshot.cursor)?,
            self.snapshot.cursor.sequence,
        )?;
        let expected_graph = merged_graph(&base, &announcements)?;
        if expected_graph != self.snapshot.graph {
            return Err(error(
                ErrorCode::CorruptLedger,
                "protocol-1.1 snapshot graph does not reproduce its citizen and announcement projections",
            ));
        }
        let expected_coverage = combined_coverage(
            self.snapshot.anchor(),
            base.receipt.coverage(),
            &capsule.announcement_batch,
        )?;
        if expected_coverage != self.receipt.coverage {
            return Err(error(
                ErrorCode::CorruptLedger,
                "protocol-1.1 coverage does not reproduce its source coverage",
            ));
        }
        Ok(())
    }
}

pub fn project_live_capsule_v1_1(
    capsule: &LiveObservationCapsuleV1_1,
    fortress_id: FortressId,
    cursor: ObservationCursor,
) -> Result<LiveWorldProjectionV1_1> {
    capsule.validate()?;
    let base = project_live_capsule(&capsule.base, fortress_id, cursor)?;
    let announcements = project_live_announcement_batch(
        &capsule.announcement_batch,
        base.snapshot.tick,
        entity_generation(cursor)?,
        cursor.sequence,
    )?;
    let graph = merged_graph(&base, &announcements)?;
    let snapshot = WorldSnapshot::new(
        fortress_id,
        base.snapshot.tick,
        cursor,
        base.snapshot.paused,
        graph,
    );
    let coverage = combined_coverage(
        snapshot.anchor(),
        base.receipt.coverage(),
        &capsule.announcement_batch,
    )?;
    let receipt = LiveProjectionReceiptV1_1 {
        schema: LIVE_PROJECTION_V1_1_SCHEMA,
        source_capsule_digest: capsule.content_digest,
        source_citizen_capsule_digest: capsule.base.content_digest,
        source_announcement_batch_digest: capsule.announcement_batch.content_digest,
        source_bridge_generation: capsule.base.bridge.bridge_generation,
        snapshot_anchor: snapshot.anchor(),
        coverage,
    };
    let projection = LiveWorldProjectionV1_1 { snapshot, receipt };
    projection.validate_against(capsule)?;
    Ok(projection)
}

fn entity_generation(cursor: ObservationCursor) -> Result<u32> {
    u32::try_from(cursor.epoch)
        .ok()
        .and_then(|epoch| epoch.checked_add(1))
        .ok_or_else(|| {
            error(
                ErrorCode::BudgetExceeded,
                "observation epoch cannot be represented as an entity generation",
            )
        })
}

fn merged_graph(
    base: &LiveWorldProjection,
    announcements: &LiveAnnouncementProjection,
) -> Result<WorldGraph> {
    let mut graph = base.snapshot.graph.clone();
    for (entity_id, entity) in &announcements.entities {
        if graph.entities.insert(*entity_id, entity.clone()).is_some() {
            return Err(error(
                ErrorCode::InternalInvariantViolation,
                "announcement entity namespace collided with an existing canonical entity",
            ));
        }
    }
    Ok(graph)
}

fn combined_coverage(
    anchor: StateAnchor,
    base: &CoverageReport,
    batch: &crate::LiveAnnouncementBatch,
) -> Result<CoverageReport> {
    base.validate()?;
    batch.validate()?;
    let mut coverage = base.clone();
    coverage.anchor = Some(anchor);
    coverage.domains.insert(
        ANNOUNCEMENT_SUFFIX_DOMAIN.to_owned(),
        CoverageDomain {
            domain: ANNOUNCEMENT_SUFFIX_DOMAIN.to_owned(),
            status: if batch.coverage.complete_through_latest {
                CoverageStatus::Complete
            } else {
                CoverageStatus::Partial
            },
            reason: if batch.coverage.complete_through_latest {
                None
            } else {
                Some(format!(
                    "more retained announcements remain after report ID {}",
                    batch.coverage.next_after_id
                ))
            },
            evidence: BTreeSet::new(),
        },
    );
    coverage.domains.insert(
        ANNOUNCEMENT_HISTORY_DOMAIN.to_owned(),
        CoverageDomain {
            domain: ANNOUNCEMENT_HISTORY_DOMAIN.to_owned(),
            status: CoverageStatus::Partial,
            reason: Some(if batch.coverage.has_gap() {
                format!(
                    "the requested cursor predates the oldest retained report ID {}; older history is unavailable",
                    batch.coverage.oldest_available_id
                )
            } else {
                "Dwarf Fortress retains a bounded in-memory report window; a complete retained suffix is not complete historical coverage"
                    .to_owned()
            }),
            evidence: BTreeSet::new(),
        },
    );
    coverage.continuation = if batch.coverage.complete_through_latest {
        None
    } else {
        Some(format!(
            "announcement_after_id={}",
            batch.coverage.next_after_id
        ))
    };
    coverage.validate()?;
    Ok(coverage)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::{
        AnnouncementBatchRecord, AnnouncementContinuity, AnnouncementCoverage,
        BridgeManifest, CitizenRecord, LiveAnnouncementBatch,
        ObservationAssemblerV1_1, ObservationPageV1_1,
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

    fn announcement_batch(
        complete: bool,
        continuity: AnnouncementContinuity,
    ) -> Result<LiveAnnouncementBatch> {
        let records = vec![AnnouncementBatchRecord {
            report_id: 10,
            report_type: 7,
            text: "A caravan has arrived".to_owned(),
            year: 105,
            year_tick: 12_340,
            repeat_count: 0,
            continuation: false,
            unconscious: false,
            announcement: true,
        }];
        LiveAnnouncementBatch::new(
            42,
            true,
            105,
            12_345,
            7,
            AnnouncementCoverage {
                requested_after_id: if matches!(
                    continuity,
                    AnnouncementContinuity::GapBeforeRetainedWindow
                ) {
                    1
                } else {
                    9
                },
                oldest_available_id: 10,
                latest_available_id: if complete { 10 } else { 11 },
                returned: 1,
                complete_through_latest: complete,
                continuity,
                next_after_id: 10,
            },
            records,
        )
    }

    fn capsule(
        complete: bool,
        continuity: AnnouncementContinuity,
    ) -> Result<LiveObservationCapsuleV1_1> {
        let announcements = announcement_batch(complete, continuity)?;
        let mut assembler = ObservationAssemblerV1_1::new(manifest());
        assembler.push_page(ObservationPageV1_1 {
            bridge_generation: 42,
            world_loaded: true,
            fortress_mode: true,
            paused: true,
            current_year: 105,
            current_year_tick: 12_345,
            world_name: "The Balanced Realm".to_owned(),
            world_folder: "region1".to_owned(),
            site_id: 7,
            citizen_count_total: 1,
            citizen_offset: 0,
            complete: true,
            citizens: vec![citizen(1)],
            announcement_batch: announcements,
        })?;
        assembler.finalize()
    }

    #[test]
    fn combined_projection_contains_citizens_and_announcements() -> Result<()> {
        let capsule = capsule(true, AnnouncementContinuity::CompleteSuffix)?;
        let projection = project_live_capsule_v1_1(
            &capsule,
            FortressId::new(7),
            ObservationCursor::ORIGIN,
        )?;
        assert_eq!(projection.snapshot.graph.entities.len(), 3);
        assert_eq!(projection.snapshot.graph.edges.len(), 1);
        assert_eq!(
            projection
                .receipt
                .coverage()
                .domains
                .get(ANNOUNCEMENT_SUFFIX_DOMAIN)
                .map(|domain| domain.status),
            Some(CoverageStatus::Complete)
        );
        projection.validate_against(&capsule)
    }

    #[test]
    fn partial_suffix_has_bounded_continuation() -> Result<()> {
        let capsule = capsule(false, AnnouncementContinuity::CompleteSuffix)?;
        let projection = project_live_capsule_v1_1(
            &capsule,
            FortressId::new(7),
            ObservationCursor::ORIGIN,
        )?;
        assert_eq!(
            projection.receipt.coverage().continuation.as_deref(),
            Some("announcement_after_id=10")
        );
        assert_eq!(
            projection
                .receipt
                .coverage()
                .domains
                .get(ANNOUNCEMENT_SUFFIX_DOMAIN)
                .map(|domain| domain.status),
            Some(CoverageStatus::Partial)
        );
        Ok(())
    }

    #[test]
    fn retained_window_gap_never_becomes_complete_history() -> Result<()> {
        let capsule = capsule(
            true,
            AnnouncementContinuity::GapBeforeRetainedWindow,
        )?;
        let projection = project_live_capsule_v1_1(
            &capsule,
            FortressId::new(7),
            ObservationCursor::ORIGIN,
        )?;
        let history = projection
            .receipt
            .coverage()
            .domains
            .get(ANNOUNCEMENT_HISTORY_DOMAIN)
            .ok_or_else(|| error(ErrorCode::InternalInvariantViolation, "history domain missing"))?;
        assert_eq!(history.status, CoverageStatus::Partial);
        assert!(
            history
                .reason
                .as_deref()
                .is_some_and(|reason| reason.contains("older history is unavailable"))
        );
        Ok(())
    }

    #[test]
    fn graph_tampering_invalidates_projection() -> Result<()> {
        let capsule = capsule(true, AnnouncementContinuity::CompleteSuffix)?;
        let mut projection = project_live_capsule_v1_1(
            &capsule,
            FortressId::new(7),
            ObservationCursor::ORIGIN,
        )?;
        let announcement_id = crate::report_id_to_announcement_entity_id(10)?;
        projection.snapshot.graph.entities.remove(&announcement_id);
        assert!(projection.validate_against(&capsule).is_err());
        Ok(())
    }

    #[test]
    fn receipt_tampering_invalidates_projection() -> Result<()> {
        let capsule = capsule(true, AnnouncementContinuity::CompleteSuffix)?;
        let mut projection = project_live_capsule_v1_1(
            &capsule,
            FortressId::new(7),
            ObservationCursor::ORIGIN,
        )?;
        projection.receipt.source_capsule_digest = Digest32::ZERO;
        assert!(projection.validate_against(&capsule).is_err());
        Ok(())
    }
}
