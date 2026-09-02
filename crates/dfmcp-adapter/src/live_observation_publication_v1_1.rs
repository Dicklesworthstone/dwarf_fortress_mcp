#![forbid(unsafe_code)]

//! Transactional publication of one protocol-1.1 observation.
//!
//! `read_complete_observation_v1_1_bounded` proves one complete citizen roster
//! plus one bounded announcement page. This layer repeats that read only when
//! the announcement suffix needs continuation, requires an unchanged paused
//! citizen observation and stable retained-window bounds across every page, and
//! publishes one combined capsule only after the suffix reaches its observed
//! high-water mark. Failure leaves no partially assembled canonical capsule.

use dfmcp_core::{DfmcpError, Digest32, ErrorCode, Result};

use crate::{
    AnnouncementContinuity, AnnouncementCoverage, LiveAnnouncementBatch,
    LiveObservationCapsule, LiveObservationCapsuleV1_1,
    LiveObservationSourceV1_1, MAX_ANNOUNCEMENTS_PER_BATCH,
    MAX_CANONICAL_CAPSULE_V1_1_BYTES, MAX_CAPSULE_CITIZENS,
    MAX_V1_1_CITIZENS_PER_PAGE, read_complete_observation_v1_1_bounded,
};

const CAPSULE_V1_1_DOMAIN: &[u8] = b"dfmcp.live-observation-capsule.v3\0";

fn error(code: ErrorCode, message: impl Into<String>) -> DfmcpError {
    DfmcpError::new(code, message)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveObservationPublicationConfigV1_1 {
    pub citizen_page_size: u32,
    pub max_citizens: u32,
    pub include_names: bool,
    pub announcement_after_id: i32,
    pub announcement_page_size: u32,
    pub max_total_announcements: u32,
}

impl LiveObservationPublicationConfigV1_1 {
    pub fn validate(&self) -> Result<()> {
        if self.citizen_page_size == 0
            || self.citizen_page_size > MAX_V1_1_CITIZENS_PER_PAGE
        {
            return Err(error(
                ErrorCode::InvalidRequest,
                format!(
                    "protocol-1.1 citizen page size must be in 1..={MAX_V1_1_CITIZENS_PER_PAGE}"
                ),
            ));
        }
        let hard_citizens = u32::try_from(MAX_CAPSULE_CITIZENS).map_err(|_| {
            error(
                ErrorCode::InternalInvariantViolation,
                "capsule citizen ceiling does not fit u32",
            )
        })?;
        if self.max_citizens > hard_citizens {
            return Err(error(
                ErrorCode::InvalidRequest,
                format!(
                    "protocol-1.1 citizen ceiling {} exceeds {hard_citizens}",
                    self.max_citizens
                ),
            ));
        }
        if self.announcement_after_id < -1 {
            return Err(error(
                ErrorCode::InvalidRequest,
                "announcement cursor must be -1 or nonnegative",
            ));
        }
        let hard_announcements = u32::try_from(MAX_ANNOUNCEMENTS_PER_BATCH).map_err(|_| {
            error(
                ErrorCode::InternalInvariantViolation,
                "announcement capsule ceiling does not fit u32",
            )
        })?;
        if self.announcement_page_size == 0
            || self.announcement_page_size > hard_announcements
        {
            return Err(error(
                ErrorCode::InvalidRequest,
                format!(
                    "announcement page size must be in 1..={hard_announcements}"
                ),
            ));
        }
        if self.max_total_announcements == 0
            || self.max_total_announcements > hard_announcements
            || self.announcement_page_size > self.max_total_announcements
        {
            return Err(error(
                ErrorCode::InvalidRequest,
                format!(
                    "total announcement ceiling must be in {}..={hard_announcements}",
                    self.announcement_page_size
                ),
            ));
        }
        Ok(())
    }
}

pub fn read_publishable_observation_v1_1<T: LiveObservationSourceV1_1>(
    source: &mut T,
    config: &LiveObservationPublicationConfigV1_1,
) -> Result<LiveObservationCapsuleV1_1> {
    config.validate()?;
    let maximum_batches = config
        .max_total_announcements
        .saturating_add(config.announcement_page_size.saturating_sub(1))
        / config.announcement_page_size;
    let initial_cursor = config.announcement_after_id;
    let mut cursor = initial_cursor;
    let mut first_base: Option<LiveObservationCapsule> = None;
    let mut first_coverage: Option<AnnouncementCoverage> = None;
    let mut announcements = Vec::new();

    for batch_index in 0..maximum_batches {
        let capsule = read_complete_observation_v1_1_bounded(
            source,
            config.citizen_page_size,
            config.include_names,
            config.max_citizens,
            cursor,
            config.announcement_page_size,
        )?;
        capsule.validate()?;
        let base = capsule.base;
        let batch = capsule.announcement_batch;

        if let Some(expected_base) = first_base.as_ref() {
            if expected_base != &base {
                return Err(error(
                    ErrorCode::StaleAnchor,
                    "citizen observation changed while completing the retained announcement suffix",
                ));
            }
            let expected_coverage = first_coverage.ok_or_else(|| {
                error(
                    ErrorCode::InternalInvariantViolation,
                    "announcement publication lost its initial coverage",
                )
            })?;
            if batch.coverage.oldest_available_id
                != expected_coverage.oldest_available_id
                || batch.coverage.latest_available_id
                    != expected_coverage.latest_available_id
            {
                return Err(error(
                    ErrorCode::StaleAnchor,
                    "retained announcement window changed during transactional publication",
                ));
            }
            if batch.coverage.continuity != AnnouncementContinuity::CompleteSuffix {
                return Err(error(
                    ErrorCode::CursorGap,
                    "announcement continuation developed a retained-window gap",
                ));
            }
        } else {
            first_coverage = Some(batch.coverage);
            first_base = Some(base.clone());
        }

        if batch.coverage.requested_after_id != cursor {
            return Err(error(
                ErrorCode::AdapterRejected,
                "announcement page echoed a different continuation cursor",
            ));
        }
        let page_returned = u32::try_from(batch.announcements.len()).map_err(|_| {
            error(
                ErrorCode::BudgetExceeded,
                "announcement page length does not fit u32",
            )
        })?;
        if !batch.coverage.complete_through_latest
            && page_returned != config.announcement_page_size
        {
            return Err(error(
                ErrorCode::AdapterRejected,
                "partial announcement page did not fill the requested page size",
            ));
        }
        let combined_length = announcements
            .len()
            .checked_add(batch.announcements.len())
            .ok_or_else(|| {
                error(
                    ErrorCode::BudgetExceeded,
                    "combined announcement count overflowed usize",
                )
            })?;
        let combined_u32 = u32::try_from(combined_length).map_err(|_| {
            error(
                ErrorCode::BudgetExceeded,
                "combined announcement count does not fit u32",
            )
        })?;
        if combined_u32 > config.max_total_announcements {
            return Err(error(
                ErrorCode::BudgetExceeded,
                "retained announcement suffix exceeds the publication ceiling",
            )
            .with_detail("next_announcement_after_id", cursor.to_string()));
        }
        announcements.extend(batch.announcements);

        if batch.coverage.complete_through_latest {
            let base = first_base.ok_or_else(|| {
                error(
                    ErrorCode::InternalInvariantViolation,
                    "completed announcement publication lost its citizen capsule",
                )
            })?;
            let initial = first_coverage.ok_or_else(|| {
                error(
                    ErrorCode::InternalInvariantViolation,
                    "completed announcement publication lost its coverage basis",
                )
            })?;
            let returned = u32::try_from(announcements.len()).map_err(|_| {
                error(
                    ErrorCode::BudgetExceeded,
                    "published announcement count does not fit u32",
                )
            })?;
            let next_after_id = announcements
                .last()
                .map_or(initial_cursor, |record| record.report_id);
            let combined = LiveAnnouncementBatch::new(
                base.bridge.bridge_generation,
                base.paused,
                base.current_year,
                base.current_year_tick,
                base.site_id,
                AnnouncementCoverage {
                    requested_after_id: initial_cursor,
                    oldest_available_id: initial.oldest_available_id,
                    latest_available_id: initial.latest_available_id,
                    returned,
                    complete_through_latest: true,
                    continuity: initial.continuity,
                    next_after_id,
                },
                announcements,
            )?;
            return combine_capsule(base, combined);
        }

        if !base.paused {
            return Err(error(
                ErrorCode::PreconditionsFailed,
                "completing a multi-page announcement suffix requires a paused fortress",
            )
            .retryable(true));
        }
        if combined_u32 == config.max_total_announcements {
            return Err(error(
                ErrorCode::BudgetExceeded,
                "retained announcement suffix exceeds the publication ceiling",
            )
            .with_detail(
                "next_announcement_after_id",
                batch.coverage.next_after_id.to_string(),
            ));
        }
        cursor = batch.coverage.next_after_id;
        if batch_index.saturating_add(1) == maximum_batches {
            break;
        }
    }

    Err(error(
        ErrorCode::BudgetExceeded,
        "retained announcement suffix exceeded the maximum publication page count",
    )
    .with_detail("next_announcement_after_id", cursor.to_string()))
}

fn combine_capsule(
    base: LiveObservationCapsule,
    announcement_batch: LiveAnnouncementBatch,
) -> Result<LiveObservationCapsuleV1_1> {
    base.validate()?;
    announcement_batch.validate()?;
    if base.bridge.bridge_generation != announcement_batch.bridge_generation
        || base.paused != announcement_batch.paused
        || base.current_year != announcement_batch.current_year
        || base.current_year_tick != announcement_batch.current_year_tick
        || base.site_id != announcement_batch.site_id
    {
        return Err(error(
            ErrorCode::InternalInvariantViolation,
            "combined protocol-1.1 capsule sources describe different observation instants",
        ));
    }
    let mut canonical_bytes = Vec::new();
    canonical_bytes.extend_from_slice(CAPSULE_V1_1_DOMAIN);
    push_bytes(&mut canonical_bytes, &base.canonical_bytes)?;
    push_bytes(
        &mut canonical_bytes,
        &announcement_batch.canonical_bytes,
    )?;
    if canonical_bytes.len() > MAX_CANONICAL_CAPSULE_V1_1_BYTES {
        return Err(error(
            ErrorCode::BudgetExceeded,
            "canonical protocol-1.1 observation exceeds its 66 MiB ceiling",
        ));
    }
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

#[cfg(test)]
mod tests {
    use std::collections::{BTreeSet, VecDeque};

    use super::*;
    use crate::{
        AnnouncementBatchRecord, BridgeManifest, CitizenRecord,
        ObservationPageV1_1,
    };

    #[derive(Clone)]
    struct ExpectedPage {
        after: i32,
        maximum: u32,
        page: ObservationPageV1_1,
    }

    #[derive(Clone)]
    struct ScriptedSource {
        manifest: BridgeManifest,
        pages: VecDeque<ExpectedPage>,
        calls: usize,
    }

    impl LiveObservationSourceV1_1 for ScriptedSource {
        fn bridge_manifest_v1_1(&self) -> BridgeManifest {
            self.manifest.clone()
        }

        fn read_observation_page_v1_1(
            &mut self,
            _offset: u32,
            _maximum: u32,
            _include_names: bool,
            announcement_after_id: i32,
            max_announcements: u32,
        ) -> Result<ObservationPageV1_1> {
            self.calls = self.calls.saturating_add(1);
            let expected = self.pages.pop_front().ok_or_else(|| {
                error(
                    ErrorCode::AdapterFailure,
                    "scripted protocol-1.1 source exhausted its pages",
                )
            })?;
            if expected.after != announcement_after_id
                || expected.maximum != max_announcements
            {
                return Err(error(
                    ErrorCode::InternalInvariantViolation,
                    "publication driver requested the wrong announcement cursor or page size",
                ));
            }
            Ok(expected.page)
        }
    }

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

    fn citizen() -> CitizenRecord {
        CitizenRecord {
            unit_id: 1,
            name: "Urist".to_owned(),
            race: "dwarf".to_owned(),
            profession: 4,
            x: 1,
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

    fn record(report_id: i32) -> AnnouncementBatchRecord {
        AnnouncementBatchRecord {
            report_id,
            report_type: 7,
            text: format!("report-{report_id}"),
            year: 105,
            year_tick: 12_000 + report_id,
            repeat_count: 0,
            continuation: false,
            unconscious: false,
            announcement: true,
        }
    }

    fn batch(
        requested_after_id: i32,
        oldest_available_id: i32,
        latest_available_id: i32,
        records: Vec<AnnouncementBatchRecord>,
        complete: bool,
        continuity: AnnouncementContinuity,
        paused: bool,
        current_year_tick: u32,
    ) -> Result<LiveAnnouncementBatch> {
        let returned = u32::try_from(records.len()).map_err(|_| {
            error(ErrorCode::BudgetExceeded, "test batch length does not fit u32")
        })?;
        let next_after_id = records
            .last()
            .map_or(requested_after_id, |value| value.report_id);
        LiveAnnouncementBatch::new(
            42,
            paused,
            105,
            current_year_tick,
            7,
            AnnouncementCoverage {
                requested_after_id,
                oldest_available_id,
                latest_available_id,
                returned,
                complete_through_latest: complete,
                continuity,
                next_after_id,
            },
            records,
        )
    }

    fn page(
        announcement_batch: LiveAnnouncementBatch,
        paused: bool,
        current_year_tick: u32,
    ) -> ObservationPageV1_1 {
        ObservationPageV1_1 {
            bridge_generation: 42,
            world_loaded: true,
            fortress_mode: true,
            paused,
            current_year: 105,
            current_year_tick,
            world_name: "The Balanced Realm".to_owned(),
            world_folder: "region1".to_owned(),
            site_id: 7,
            citizen_count_total: 1,
            citizen_offset: 0,
            complete: true,
            citizens: vec![citizen()],
            announcement_batch,
        }
    }

    fn config(page_size: u32, maximum: u32) -> LiveObservationPublicationConfigV1_1 {
        LiveObservationPublicationConfigV1_1 {
            citizen_page_size: 64,
            max_citizens: 64,
            include_names: true,
            announcement_after_id: -1,
            announcement_page_size: page_size,
            max_total_announcements: maximum,
        }
    }

    #[test]
    fn announcement_transport_pagination_does_not_change_capsule_identity() -> Result<()> {
        let all = batch(
            -1,
            1,
            3,
            vec![record(1), record(2), record(3)],
            true,
            AnnouncementContinuity::CompleteSuffix,
            true,
            12_345,
        )?;
        let mut one = ScriptedSource {
            manifest: manifest(),
            pages: VecDeque::from([ExpectedPage {
                after: -1,
                maximum: 4,
                page: page(all, true, 12_345),
            }]),
            calls: 0,
        };
        let one = read_publishable_observation_v1_1(&mut one, &config(4, 4))?;

        let first = batch(
            -1,
            1,
            3,
            vec![record(1), record(2)],
            false,
            AnnouncementContinuity::CompleteSuffix,
            true,
            12_345,
        )?;
        let second = batch(
            2,
            1,
            3,
            vec![record(3)],
            true,
            AnnouncementContinuity::CompleteSuffix,
            true,
            12_345,
        )?;
        let mut many = ScriptedSource {
            manifest: manifest(),
            pages: VecDeque::from([
                ExpectedPage {
                    after: -1,
                    maximum: 2,
                    page: page(first, true, 12_345),
                },
                ExpectedPage {
                    after: 2,
                    maximum: 2,
                    page: page(second, true, 12_345),
                },
            ]),
            calls: 0,
        };
        let many = read_publishable_observation_v1_1(&mut many, &config(2, 4))?;
        assert_eq!(one.canonical_bytes, many.canonical_bytes);
        assert_eq!(one.content_digest, many.content_digest);
        assert_eq!(many.announcement_batch.announcements.len(), 3);
        assert!(many.announcement_batch.coverage.complete_through_latest);
        Ok(())
    }

    #[test]
    fn partial_suffix_requires_a_paused_fortress_before_followup() -> Result<()> {
        let first = batch(
            -1,
            1,
            3,
            vec![record(1), record(2)],
            false,
            AnnouncementContinuity::CompleteSuffix,
            false,
            12_345,
        )?;
        let mut source = ScriptedSource {
            manifest: manifest(),
            pages: VecDeque::from([ExpectedPage {
                after: -1,
                maximum: 2,
                page: page(first, false, 12_345),
            }]),
            calls: 0,
        };
        let failure = read_publishable_observation_v1_1(&mut source, &config(2, 4))
            .err()
            .ok_or_else(|| error(ErrorCode::InternalInvariantViolation, "moving suffix published"))?;
        assert_eq!(failure.code, ErrorCode::PreconditionsFailed);
        assert_eq!(source.calls, 1);
        Ok(())
    }

    #[test]
    fn citizen_or_clock_drift_aborts_without_a_capsule() -> Result<()> {
        let first = batch(
            -1,
            1,
            3,
            vec![record(1), record(2)],
            false,
            AnnouncementContinuity::CompleteSuffix,
            true,
            12_345,
        )?;
        let second = batch(
            2,
            1,
            3,
            vec![record(3)],
            true,
            AnnouncementContinuity::CompleteSuffix,
            true,
            12_346,
        )?;
        let mut source = ScriptedSource {
            manifest: manifest(),
            pages: VecDeque::from([
                ExpectedPage {
                    after: -1,
                    maximum: 2,
                    page: page(first, true, 12_345),
                },
                ExpectedPage {
                    after: 2,
                    maximum: 2,
                    page: page(second, true, 12_346),
                },
            ]),
            calls: 0,
        };
        let failure = read_publishable_observation_v1_1(&mut source, &config(2, 4))
            .err()
            .ok_or_else(|| error(ErrorCode::InternalInvariantViolation, "drifted suffix published"))?;
        assert_eq!(failure.code, ErrorCode::StaleAnchor);
        Ok(())
    }

    #[test]
    fn retained_window_drift_aborts_without_a_capsule() -> Result<()> {
        let first = batch(
            -1,
            1,
            3,
            vec![record(1), record(2)],
            false,
            AnnouncementContinuity::CompleteSuffix,
            true,
            12_345,
        )?;
        let second = batch(
            2,
            1,
            4,
            vec![record(3), record(4)],
            true,
            AnnouncementContinuity::CompleteSuffix,
            true,
            12_345,
        )?;
        let mut source = ScriptedSource {
            manifest: manifest(),
            pages: VecDeque::from([
                ExpectedPage {
                    after: -1,
                    maximum: 2,
                    page: page(first, true, 12_345),
                },
                ExpectedPage {
                    after: 2,
                    maximum: 2,
                    page: page(second, true, 12_345),
                },
            ]),
            calls: 0,
        };
        let failure = read_publishable_observation_v1_1(&mut source, &config(2, 4))
            .err()
            .ok_or_else(|| error(ErrorCode::InternalInvariantViolation, "drifted window published"))?;
        assert_eq!(failure.code, ErrorCode::StaleAnchor);
        Ok(())
    }

    #[test]
    fn incomplete_page_must_fill_the_requested_announcement_size() -> Result<()> {
        let partial = batch(
            -1,
            1,
            3,
            vec![record(1)],
            false,
            AnnouncementContinuity::CompleteSuffix,
            true,
            12_345,
        )?;
        let mut source = ScriptedSource {
            manifest: manifest(),
            pages: VecDeque::from([ExpectedPage {
                after: -1,
                maximum: 2,
                page: page(partial, true, 12_345),
            }]),
            calls: 0,
        };
        let failure = read_publishable_observation_v1_1(&mut source, &config(2, 4))
            .err()
            .ok_or_else(|| error(ErrorCode::InternalInvariantViolation, "underfilled page published"))?;
        assert_eq!(failure.code, ErrorCode::AdapterRejected);
        Ok(())
    }

    #[test]
    fn publication_ceiling_fails_with_a_resume_cursor() -> Result<()> {
        let partial = batch(
            -1,
            1,
            3,
            vec![record(1), record(2)],
            false,
            AnnouncementContinuity::CompleteSuffix,
            true,
            12_345,
        )?;
        let mut source = ScriptedSource {
            manifest: manifest(),
            pages: VecDeque::from([ExpectedPage {
                after: -1,
                maximum: 2,
                page: page(partial, true, 12_345),
            }]),
            calls: 0,
        };
        let failure = read_publishable_observation_v1_1(&mut source, &config(2, 2))
            .err()
            .ok_or_else(|| error(ErrorCode::InternalInvariantViolation, "over-ceiling suffix published"))?;
        assert_eq!(failure.code, ErrorCode::BudgetExceeded);
        assert!(
            failure
                .details
                .iter()
                .any(|(key, value)| key == "next_announcement_after_id" && value == "2")
        );
        Ok(())
    }

    #[test]
    fn initial_retained_window_gap_survives_complete_publication() -> Result<()> {
        let retained = batch(
            1,
            10,
            11,
            vec![record(10), record(11)],
            true,
            AnnouncementContinuity::GapBeforeRetainedWindow,
            true,
            12_345,
        )?;
        let mut source = ScriptedSource {
            manifest: manifest(),
            pages: VecDeque::from([ExpectedPage {
                after: 1,
                maximum: 2,
                page: page(retained, true, 12_345),
            }]),
            calls: 0,
        };
        let mut publication = config(2, 4);
        publication.announcement_after_id = 1;
        let capsule = read_publishable_observation_v1_1(&mut source, &publication)?;
        assert!(capsule.announcement_batch.coverage.has_gap());
        assert!(capsule.announcement_batch.coverage.complete_through_latest);
        Ok(())
    }
}
