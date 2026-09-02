#![forbid(unsafe_code)]

//! Bounded driver for one complete protocol-1.1 citizen observation plus one
//! canonical announcement suffix.
//!
//! The same announcement cursor and limit are sent with every citizen page.
//! The driver independently revalidates the echoed cursor and actual returned
//! record count before the page reaches assembly. The assembler then requires
//! byte-identical announcement evidence across pages, so drift aborts without
//! publishing a capsule. Partial announcement suffixes are allowed only with
//! an explicit continuation cursor in their coverage.

use std::io::{Read, Write};

use dfmcp_core::{DfmcpError, ErrorCode, Result};

use crate::{
    BridgeManifest, DfHackRpcClientV1_1, LiveObservationCapsuleV1_1,
    MAX_ANNOUNCEMENTS_PER_BATCH, MAX_CAPSULE_CITIZENS, MAX_V1_1_CITIZENS_PER_PAGE,
    ObservationAssemblerV1_1, ObservationPageV1_1,
};

fn error(code: ErrorCode, message: impl Into<String>) -> DfmcpError {
    DfmcpError::new(code, message)
}

pub trait LiveObservationSourceV1_1 {
    fn bridge_manifest_v1_1(&self) -> BridgeManifest;

    fn read_observation_page_v1_1(
        &mut self,
        offset: u32,
        maximum: u32,
        include_names: bool,
        announcement_after_id: i32,
        max_announcements: u32,
    ) -> Result<ObservationPageV1_1>;
}

impl<S: Read + Write> LiveObservationSourceV1_1 for DfHackRpcClientV1_1<S> {
    fn bridge_manifest_v1_1(&self) -> BridgeManifest {
        self.manifest().clone()
    }

    fn read_observation_page_v1_1(
        &mut self,
        offset: u32,
        maximum: u32,
        include_names: bool,
        announcement_after_id: i32,
        max_announcements: u32,
    ) -> Result<ObservationPageV1_1> {
        self.read_observation(
            offset,
            maximum,
            include_names,
            announcement_after_id,
            max_announcements,
        )
    }
}

pub fn read_complete_observation_v1_1<T: LiveObservationSourceV1_1>(
    source: &mut T,
    page_size: u32,
    include_names: bool,
    announcement_after_id: i32,
    max_announcements: u32,
) -> Result<LiveObservationCapsuleV1_1> {
    let hard_total = u32::try_from(MAX_CAPSULE_CITIZENS).map_err(|_| {
        error(
            ErrorCode::InternalInvariantViolation,
            "capsule citizen ceiling does not fit u32",
        )
    })?;
    read_complete_observation_v1_1_bounded(
        source,
        page_size,
        include_names,
        hard_total,
        announcement_after_id,
        max_announcements,
    )
}

pub fn read_complete_observation_v1_1_bounded<T: LiveObservationSourceV1_1>(
    source: &mut T,
    page_size: u32,
    include_names: bool,
    max_citizens: u32,
    announcement_after_id: i32,
    max_announcements: u32,
) -> Result<LiveObservationCapsuleV1_1> {
    if page_size == 0 || page_size > MAX_V1_1_CITIZENS_PER_PAGE {
        return Err(error(
            ErrorCode::InvalidRequest,
            format!(
                "protocol-1.1 citizen page size must be in 1..={MAX_V1_1_CITIZENS_PER_PAGE}"
            ),
        ));
    }
    if announcement_after_id < -1 {
        return Err(error(
            ErrorCode::InvalidRequest,
            "announcement cursor must be -1 or nonnegative",
        ));
    }
    let hard_announcements = u32::try_from(MAX_ANNOUNCEMENTS_PER_BATCH).map_err(|_| {
        error(
            ErrorCode::InternalInvariantViolation,
            "announcement hard limit does not fit u32",
        )
    })?;
    if max_announcements == 0 || max_announcements > hard_announcements {
        return Err(error(
            ErrorCode::InvalidRequest,
            format!("announcement page size must be in 1..={hard_announcements}"),
        ));
    }

    let hard_total = u32::try_from(MAX_CAPSULE_CITIZENS).map_err(|_| {
        error(
            ErrorCode::InternalInvariantViolation,
            "capsule citizen ceiling does not fit u32",
        )
    })?;
    if max_citizens > hard_total {
        return Err(error(
            ErrorCode::InvalidRequest,
            format!(
                "caller citizen ceiling {max_citizens} exceeds the capsule ceiling of {hard_total}"
            ),
        ));
    }

    let manifest = source.bridge_manifest_v1_1();
    manifest.validate()?;
    if manifest.bridge_version != "0.2.0" {
        return Err(error(
            ErrorCode::VersionMismatch,
            "protocol-1.1 observation source must report bridge version 0.2.0",
        ));
    }
    if !manifest.world_loaded || !manifest.fortress_mode {
        return Err(error(
            ErrorCode::AdapterUnavailable,
            "DFHack protocol-1.1 handshake does not report a loaded fortress-mode world",
        ));
    }

    let mut assembler = ObservationAssemblerV1_1::with_names(manifest, include_names);
    let rounded_pages = if max_citizens == 0 {
        0
    } else {
        max_citizens
            .saturating_add(page_size.saturating_sub(1))
            / page_size
    };
    let maximum_pages = rounded_pages.saturating_add(1);

    for _ in 0..maximum_pages {
        let offset = assembler.next_offset()?;
        let page = source.read_observation_page_v1_1(
            offset,
            page_size,
            include_names,
            announcement_after_id,
            max_announcements,
        )?;
        if page.citizen_count_total > hard_total {
            return Err(error(
                ErrorCode::BudgetExceeded,
                format!(
                    "bridge reports {} citizens, exceeding the capsule ceiling of {hard_total}",
                    page.citizen_count_total
                ),
            ));
        }
        if page.citizen_count_total > max_citizens {
            return Err(error(
                ErrorCode::BudgetExceeded,
                format!(
                    "bridge reports {} citizens, exceeding the caller ceiling of {max_citizens}",
                    page.citizen_count_total
                ),
            ));
        }
        if page.citizens.is_empty() && !page.complete {
            return Err(error(
                ErrorCode::AdapterRejected,
                "bridge returned an empty nonterminal protocol-1.1 citizen page",
            ));
        }
        if page.announcement_batch.coverage.requested_after_id != announcement_after_id {
            return Err(error(
                ErrorCode::AdapterRejected,
                format!(
                    "bridge echoed announcement cursor {}, expected {announcement_after_id}",
                    page.announcement_batch.coverage.requested_after_id
                ),
            ));
        }
        let returned_announcements =
            u32::try_from(page.announcement_batch.announcements.len()).map_err(|_| {
                error(
                    ErrorCode::BudgetExceeded,
                    "returned announcement count does not fit u32",
                )
            })?;
        if returned_announcements > max_announcements {
            return Err(error(
                ErrorCode::BudgetExceeded,
                format!(
                    "bridge returned {returned_announcements} announcements, exceeding the caller ceiling of {max_announcements}"
                ),
            ));
        }
        assembler.push_page(page)?;
        if assembler.is_complete() {
            return assembler.finalize();
        }
    }

    Err(error(
        ErrorCode::BudgetExceeded,
        "protocol-1.1 observation exceeded the maximum admitted page count",
    ))
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeSet, VecDeque};

    use super::*;
    use crate::{
        AnnouncementBatchRecord, AnnouncementContinuity, AnnouncementCoverage,
        CitizenRecord, LiveAnnouncementBatch,
    };

    #[derive(Clone)]
    struct FakeSource {
        manifest: BridgeManifest,
        pages: VecDeque<ObservationPageV1_1>,
        calls: usize,
    }

    impl LiveObservationSourceV1_1 for FakeSource {
        fn bridge_manifest_v1_1(&self) -> BridgeManifest {
            self.manifest.clone()
        }

        fn read_observation_page_v1_1(
            &mut self,
            _offset: u32,
            _maximum: u32,
            _include_names: bool,
            _announcement_after_id: i32,
            _max_announcements: u32,
        ) -> Result<ObservationPageV1_1> {
            self.calls = self.calls.saturating_add(1);
            self.pages.pop_front().ok_or_else(|| {
                error(
                    ErrorCode::AdapterFailure,
                    "fake protocol-1.1 source exhausted its pages",
                )
            })
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

    fn announcement(report_id: i32, text: &str) -> AnnouncementBatchRecord {
        AnnouncementBatchRecord {
            report_id,
            report_type: 7,
            text: text.to_owned(),
            year: 105,
            year_tick: 12_340,
            repeat_count: 0,
            continuation: false,
            unconscious: false,
            announcement: true,
        }
    }

    fn announcements(text: &str) -> Result<LiveAnnouncementBatch> {
        announcement_batch(9, vec![announcement(10, text)], true)
    }

    fn announcement_batch(
        requested_after_id: i32,
        records: Vec<AnnouncementBatchRecord>,
        complete_through_latest: bool,
    ) -> Result<LiveAnnouncementBatch> {
        let latest_available_id = records.last().map_or(requested_after_id, |record| record.report_id);
        LiveAnnouncementBatch::new(
            42,
            true,
            105,
            12_345,
            7,
            AnnouncementCoverage {
                requested_after_id,
                oldest_available_id: records.first().map_or(-1, |record| record.report_id),
                latest_available_id,
                returned: u32::try_from(records.len()).map_err(|_| {
                    error(
                        ErrorCode::BudgetExceeded,
                        "test announcement count does not fit u32",
                    )
                })?,
                complete_through_latest,
                continuity: AnnouncementContinuity::CompleteSuffix,
                next_after_id: latest_available_id,
            },
            records,
        )
    }

    fn page(
        offset: u32,
        total: u32,
        ids: &[i32],
        complete: bool,
        announcement_batch: LiveAnnouncementBatch,
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
            announcement_batch,
        }
    }

    fn source(pages: Vec<ObservationPageV1_1>) -> FakeSource {
        FakeSource {
            manifest: manifest(),
            pages: pages.into(),
            calls: 0,
        }
    }

    #[test]
    fn drives_paused_pages_to_one_atomic_capsule() -> Result<()> {
        let batch = announcements("stable")?;
        let mut source = source(vec![
            page(0, 3, &[1, 2], false, batch.clone()),
            page(2, 3, &[3], true, batch),
        ]);
        let capsule = read_complete_observation_v1_1_bounded(
            &mut source,
            2,
            true,
            3,
            9,
            128,
        )?;
        assert_eq!(source.calls, 2);
        assert_eq!(capsule.base.citizens.len(), 3);
        assert_eq!(capsule.announcement_batch.announcements.len(), 1);
        Ok(())
    }

    #[test]
    fn announcement_drift_aborts_after_second_page() -> Result<()> {
        let mut source = source(vec![
            page(0, 2, &[1], false, announcements("first")?),
            page(1, 2, &[2], true, announcements("changed")?),
        ]);
        assert!(
            read_complete_observation_v1_1_bounded(
                &mut source,
                1,
                true,
                2,
                9,
                128,
            )
            .is_err()
        );
        assert_eq!(source.calls, 2);
        Ok(())
    }

    #[test]
    fn invalid_bounds_are_rejected_before_source_io() {
        let mut source = source(Vec::new());
        assert!(
            read_complete_observation_v1_1_bounded(
                &mut source,
                0,
                true,
                10,
                -1,
                128,
            )
            .is_err()
        );
        assert!(
            read_complete_observation_v1_1_bounded(
                &mut source,
                1,
                true,
                10,
                -2,
                128,
            )
            .is_err()
        );
        assert!(
            read_complete_observation_v1_1_bounded(
                &mut source,
                1,
                true,
                10,
                -1,
                0,
            )
            .is_err()
        );
        assert_eq!(source.calls, 0);
    }

    #[test]
    fn caller_citizen_ceiling_aborts_after_first_page() -> Result<()> {
        let mut source = source(vec![page(
            0,
            3,
            &[1, 2],
            false,
            announcements("stable")?,
        )]);
        assert!(
            read_complete_observation_v1_1_bounded(
                &mut source,
                2,
                true,
                2,
                9,
                128,
            )
            .is_err()
        );
        assert_eq!(source.calls, 1);
        Ok(())
    }

    #[test]
    fn caller_announcement_ceiling_aborts_after_first_page() -> Result<()> {
        let batch = announcement_batch(
            9,
            vec![announcement(10, "first"), announcement(11, "second")],
            true,
        )?;
        let mut source = source(vec![page(0, 1, &[1], true, batch)]);
        let failure = read_complete_observation_v1_1_bounded(
            &mut source,
            1,
            true,
            1,
            9,
            1,
        )
        .expect_err("the returned vector exceeds the caller announcement ceiling");
        assert_eq!(failure.code, ErrorCode::BudgetExceeded);
        assert_eq!(source.calls, 1);
        Ok(())
    }

    #[test]
    fn echoed_announcement_cursor_must_match_the_request() -> Result<()> {
        let batch = announcement_batch(8, vec![announcement(10, "wrong cursor")], true)?;
        let mut source = source(vec![page(0, 1, &[1], true, batch)]);
        let failure = read_complete_observation_v1_1_bounded(
            &mut source,
            1,
            true,
            1,
            9,
            128,
        )
        .expect_err("the source echoed a different announcement cursor");
        assert_eq!(failure.code, ErrorCode::AdapterRejected);
        assert_eq!(source.calls, 1);
        Ok(())
    }

    #[test]
    fn zero_citizen_fortress_finishes_with_announcement_evidence() -> Result<()> {
        let mut source = source(vec![page(
            0,
            0,
            &[],
            true,
            announcements("empty fortress report")?,
        )]);
        let capsule = read_complete_observation_v1_1_bounded(
            &mut source,
            64,
            true,
            0,
            9,
            128,
        )?;
        assert!(capsule.base.citizens.is_empty());
        assert_eq!(capsule.announcement_batch.coverage.next_after_id, 10);
        Ok(())
    }
}
