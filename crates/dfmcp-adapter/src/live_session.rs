#![forbid(unsafe_code)]

//! Bounded drivers for complete canonical live reads.
//!
//! Citizen pagination and announcement pagination have different coherence
//! rules. A citizen capsule requires stable summary fields and permits multiple
//! pages only while the fortress is paused. An announcement read freezes the
//! first page's retained high-water mark, then requires every continuation to
//! reproduce the same retained-window witness. Neither driver publishes a
//! partial candidate.

use std::io::{Read, Write};

use dfmcp_core::{DfmcpError, ErrorCode, Result};

use crate::{
    AnnouncementPage, AnnouncementSourceIdentity, AnnouncementWindowAssembler,
    BridgeManifest, DfHackRpcClient, LiveAnnouncementWindow, LiveObservationCapsule,
    MAX_ANNOUNCEMENTS_PER_PAGE, MAX_ANNOUNCEMENT_WINDOW_RECORDS,
    MAX_CAPSULE_CITIZENS, MAX_CITIZENS_PER_PAGE, ObservationAssembler,
    ObservationPage,
};

fn error(code: ErrorCode, message: impl Into<String>) -> DfmcpError {
    DfmcpError::new(code, message)
}

pub trait LiveObservationSource {
    fn bridge_manifest(&self) -> BridgeManifest;

    fn read_observation_page(
        &mut self,
        offset: u32,
        maximum: u32,
        include_names: bool,
    ) -> Result<ObservationPage>;
}

impl<S: Read + Write> LiveObservationSource for DfHackRpcClient<S> {
    fn bridge_manifest(&self) -> BridgeManifest {
        self.manifest().clone()
    }

    fn read_observation_page(
        &mut self,
        offset: u32,
        maximum: u32,
        include_names: bool,
    ) -> Result<ObservationPage> {
        self.read_observation(offset, maximum, include_names)
    }
}

pub trait LiveAnnouncementSource {
    fn announcement_source_identity(&self) -> Result<AnnouncementSourceIdentity>;

    fn read_announcement_page(
        &mut self,
        after_report_id: i32,
        through_report_id: i32,
        maximum: u32,
    ) -> Result<AnnouncementPage>;
}

impl<S: Read + Write> LiveAnnouncementSource for DfHackRpcClient<S> {
    fn announcement_source_identity(&self) -> Result<AnnouncementSourceIdentity> {
        DfHackRpcClient::announcement_source_identity(self)
    }

    fn read_announcement_page(
        &mut self,
        after_report_id: i32,
        through_report_id: i32,
        maximum: u32,
    ) -> Result<AnnouncementPage> {
        self.read_announcements(after_report_id, through_report_id, maximum)
    }
}

pub fn read_complete_observation<T: LiveObservationSource>(
    source: &mut T,
    page_size: u32,
    include_names: bool,
) -> Result<LiveObservationCapsule> {
    let hard_total = u32::try_from(MAX_CAPSULE_CITIZENS).map_err(|_| {
        error(
            ErrorCode::InternalInvariantViolation,
            "capsule citizen ceiling does not fit u32",
        )
    })?;
    read_complete_observation_bounded(source, page_size, include_names, hard_total)
}

pub fn read_complete_observation_bounded<T: LiveObservationSource>(
    source: &mut T,
    page_size: u32,
    include_names: bool,
    max_citizens: u32,
) -> Result<LiveObservationCapsule> {
    if page_size == 0 || page_size > MAX_CITIZENS_PER_PAGE {
        return Err(error(
            ErrorCode::InvalidRequest,
            format!(
                "live observation page size must be in 1..={MAX_CITIZENS_PER_PAGE}"
            ),
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

    let manifest = source.bridge_manifest();
    manifest.validate()?;
    if !manifest.world_loaded || !manifest.fortress_mode {
        return Err(error(
            ErrorCode::AdapterUnavailable,
            "DFHack handshake does not report a loaded fortress-mode world",
        ));
    }
    let mut assembler = ObservationAssembler::with_names(manifest, include_names);
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
        let page = source.read_observation_page(offset, page_size, include_names)?;
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
                "bridge returned an empty nonterminal citizen page",
            ));
        }
        if !page.complete && !page.paused {
            return Err(error(
                ErrorCode::PreconditionsFailed,
                "bridge V1 cannot assemble a coherent multipage observation while Dwarf Fortress is running; request a larger page or pause the game",
            )
            .retryable(true));
        }
        assembler.push_page(page)?;
        if assembler.is_complete() {
            return assembler.finalize();
        }
    }

    Err(error(
        ErrorCode::BudgetExceeded,
        "live observation exceeded the maximum admitted page count",
    ))
}

pub fn read_complete_announcement_window<T: LiveAnnouncementSource>(
    source: &mut T,
    after_report_id: i32,
    page_size: u32,
    max_records: u32,
) -> Result<LiveAnnouncementWindow> {
    if after_report_id < -1 {
        return Err(error(
            ErrorCode::InvalidRequest,
            "announcement cursor must be -1 or a nonnegative report ID",
        ));
    }
    if page_size == 0 || page_size > MAX_ANNOUNCEMENTS_PER_PAGE {
        return Err(error(
            ErrorCode::InvalidRequest,
            format!(
                "announcement page size must be in 1..={MAX_ANNOUNCEMENTS_PER_PAGE}"
            ),
        ));
    }
    let hard_records = u32::try_from(MAX_ANNOUNCEMENT_WINDOW_RECORDS).map_err(|_| {
        error(
            ErrorCode::InternalInvariantViolation,
            "announcement record ceiling does not fit u32",
        )
    })?;
    if max_records == 0 || max_records > hard_records {
        return Err(error(
            ErrorCode::InvalidRequest,
            format!(
                "announcement record ceiling must be in 1..={hard_records}"
            ),
        ));
    }

    let identity = source.announcement_source_identity()?;
    identity.validate()?;
    let mut assembler = AnnouncementWindowAssembler::new(identity, after_report_id)?;
    let maximum_pages = max_records
        .saturating_add(page_size.saturating_sub(1))
        .checked_div(page_size)
        .unwrap_or(0)
        .saturating_add(1);
    let mut retained = 0u32;

    for _ in 0..maximum_pages {
        let after = assembler.next_after_report_id();
        let through = assembler.frozen_high_water_mark().unwrap_or(-1);
        let remaining = max_records.saturating_sub(retained);
        if remaining == 0 {
            return Err(error(
                ErrorCode::BudgetExceeded,
                "announcement window exceeds the caller record ceiling",
            ));
        }
        let requested = page_size.min(remaining);
        let page = source.read_announcement_page(after, through, requested)?;
        let page_records = u32::try_from(page.announcements.len()).map_err(|_| {
            error(
                ErrorCode::BudgetExceeded,
                "announcement page length does not fit u32",
            )
        })?;
        let candidate_total = retained.checked_add(page_records).ok_or_else(|| {
            error(
                ErrorCode::BudgetExceeded,
                "announcement window record count overflowed",
            )
        })?;
        if candidate_total > max_records {
            return Err(error(
                ErrorCode::BudgetExceeded,
                "announcement window exceeds the caller record ceiling",
            ));
        }
        assembler.push_page(page)?;
        retained = candidate_total;
        if assembler.is_complete() {
            return assembler.finalize();
        }
    }

    Err(error(
        ErrorCode::BudgetExceeded,
        "announcement window exceeded the maximum admitted page count",
    ))
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeSet, VecDeque};

    use super::*;
    use crate::{AnnouncementRecord, CitizenRecord};

    #[derive(Clone)]
    struct FakeSource {
        manifest: BridgeManifest,
        pages: Vec<ObservationPage>,
        index: usize,
        calls: usize,
    }

    impl LiveObservationSource for FakeSource {
        fn bridge_manifest(&self) -> BridgeManifest {
            self.manifest.clone()
        }

        fn read_observation_page(
            &mut self,
            _offset: u32,
            _maximum: u32,
            _include_names: bool,
        ) -> Result<ObservationPage> {
            self.calls = self.calls.saturating_add(1);
            let page = self.pages.get(self.index).cloned().ok_or_else(|| {
                error(
                    ErrorCode::AdapterFailure,
                    "fake source exhausted its scripted pages",
                )
            })?;
            self.index = self.index.saturating_add(1);
            Ok(page)
        }
    }

    #[derive(Clone)]
    struct FakeAnnouncementSource {
        identity: AnnouncementSourceIdentity,
        pages: VecDeque<AnnouncementPage>,
        calls: Vec<(i32, i32, u32)>,
    }

    impl LiveAnnouncementSource for FakeAnnouncementSource {
        fn announcement_source_identity(&self) -> Result<AnnouncementSourceIdentity> {
            Ok(self.identity.clone())
        }

        fn read_announcement_page(
            &mut self,
            after_report_id: i32,
            through_report_id: i32,
            maximum: u32,
        ) -> Result<AnnouncementPage> {
            self.calls
                .push((after_report_id, through_report_id, maximum));
            self.pages.pop_front().ok_or_else(|| {
                error(
                    ErrorCode::AdapterFailure,
                    "fake announcement source exhausted its pages",
                )
            })
        }
    }

    fn source(pages: Vec<ObservationPage>) -> FakeSource {
        FakeSource {
            manifest: manifest(),
            pages,
            index: 0,
            calls: 0,
        }
    }

    fn manifest() -> BridgeManifest {
        BridgeManifest {
            bridge_version: "0.1.0".to_owned(),
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

    fn announcement_identity() -> AnnouncementSourceIdentity {
        AnnouncementSourceIdentity {
            bridge_version: "0.2.0".to_owned(),
            dfhack_version: "0.51.11-r1".to_owned(),
            dwarf_fortress_version: "0.51.11".to_owned(),
            protocol_major: 1,
            protocol_minor: 1,
            bridge_generation: 42,
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

    fn page(offset: u32, total: u32, ids: &[i32], complete: bool) -> ObservationPage {
        ObservationPage {
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
        }
    }

    fn page_without_names(
        offset: u32,
        total: u32,
        ids: &[i32],
        complete: bool,
    ) -> ObservationPage {
        let mut page = page(offset, total, ids, complete);
        for citizen in &mut page.citizens {
            citizen.name.clear();
        }
        page
    }

    fn announcement(report_id: i32) -> AnnouncementRecord {
        AnnouncementRecord {
            report_id,
            announcement_type: 7,
            text: format!("announcement {report_id}"),
            year: 105,
            year_tick: 12_345,
            has_position: false,
            x: 0,
            y: 0,
            z: 0,
            repeat_count: 0,
            continuation: false,
            unconscious: false,
            announcement: true,
        }
    }

    fn announcement_page(
        after: i32,
        maximum: u32,
        ids: &[i32],
        complete: bool,
    ) -> AnnouncementPage {
        AnnouncementPage {
            bridge_generation: 42,
            requested_after_report_id: after,
            requested_maximum: maximum,
            oldest_retained_report_id: 1,
            latest_retained_report_id: 4,
            window_latest_report_id: 4,
            next_after_report_id: ids.last().copied().unwrap_or(after),
            history_truncated: false,
            complete,
            announcements: ids.iter().copied().map(announcement).collect(),
        }
    }

    #[test]
    fn drives_multiple_paused_pages_to_one_complete_capsule() -> Result<()> {
        let mut source = source(vec![page(0, 3, &[1, 2], false), page(2, 3, &[3], true)]);
        let capsule = read_complete_observation(&mut source, 2, true)?;
        assert!(capsule.citizen_coverage.proves_complete_roster());
        assert!(capsule.names_included);
        assert_eq!(capsule.citizens.len(), 3);
        assert_eq!(source.calls, 2);
        Ok(())
    }

    #[test]
    fn moving_multipage_observation_is_rejected_before_assembly() {
        let mut first = page(0, 3, &[1, 2], false);
        first.paused = false;
        let mut source = source(vec![first, page(2, 3, &[3], true)]);
        let result = read_complete_observation(&mut source, 2, true);
        assert!(result.is_err());
        assert_eq!(source.calls, 1);
    }

    #[test]
    fn moving_single_page_observation_is_admitted() -> Result<()> {
        let mut only = page(0, 2, &[1, 2], true);
        only.paused = false;
        let mut source = source(vec![only]);
        let capsule = read_complete_observation(&mut source, 2, true)?;
        assert!(!capsule.paused);
        assert_eq!(source.calls, 1);
        Ok(())
    }

    #[test]
    fn requested_name_omission_is_preserved_in_the_capsule() -> Result<()> {
        let mut source = source(vec![page_without_names(0, 1, &[1], true)]);
        let capsule = read_complete_observation(&mut source, 1, false)?;
        assert!(!capsule.names_included);
        assert!(capsule.citizens[0].name.is_empty());
        Ok(())
    }

    #[test]
    fn caller_ceiling_aborts_after_the_first_page() {
        let mut source = source(vec![page(0, 3, &[1, 2], false), page(2, 3, &[3], true)]);
        assert!(read_complete_observation_bounded(&mut source, 2, true, 2).is_err());
        assert_eq!(source.calls, 1);
    }

    #[test]
    fn empty_nonterminal_page_is_rejected() {
        let mut source = source(vec![page(0, 1, &[], false)]);
        assert!(read_complete_observation(&mut source, 1, true).is_err());
    }

    #[test]
    fn invalid_page_size_is_rejected_before_source_io() {
        let mut source = source(Vec::new());
        assert!(read_complete_observation(&mut source, 0, true).is_err());
        assert_eq!(source.calls, 0);
    }

    #[test]
    fn zero_citizen_fortress_finishes_in_one_empty_page() -> Result<()> {
        let mut source = source(vec![page(0, 0, &[], true)]);
        let capsule = read_complete_observation_bounded(&mut source, 64, true, 0)?;
        assert!(capsule.citizen_coverage.proves_complete_roster());
        assert!(capsule.citizens.is_empty());
        Ok(())
    }

    #[test]
    fn announcement_driver_freezes_first_page_high_water() -> Result<()> {
        let mut source = FakeAnnouncementSource {
            identity: announcement_identity(),
            pages: VecDeque::from([
                announcement_page(-1, 2, &[1, 2], false),
                announcement_page(2, 2, &[3, 4], true),
            ]),
            calls: Vec::new(),
        };
        let window = read_complete_announcement_window(&mut source, -1, 2, 8)?;
        assert_eq!(window.announcements.len(), 4);
        assert_eq!(source.calls, vec![(-1, -1, 2), (2, 4, 2)]);
        Ok(())
    }

    #[test]
    fn announcement_driver_rejects_record_budget_before_partial_publication() {
        let mut source = FakeAnnouncementSource {
            identity: announcement_identity(),
            pages: VecDeque::from([announcement_page(-1, 2, &[1, 2], false)]),
            calls: Vec::new(),
        };
        assert!(read_complete_announcement_window(&mut source, -1, 2, 1).is_err());
        assert_eq!(source.calls.len(), 1);
    }

    #[test]
    fn announcement_driver_preserves_retained_history_loss() -> Result<()> {
        let mut page = announcement_page(0, 4, &[3, 4], true);
        page.oldest_retained_report_id = 3;
        page.history_truncated = true;
        let mut source = FakeAnnouncementSource {
            identity: announcement_identity(),
            pages: VecDeque::from([page]),
            calls: Vec::new(),
        };
        let window = read_complete_announcement_window(&mut source, 0, 4, 8)?;
        assert!(window.history_truncated);
        assert!(!window.can_prove_absence_in_frozen_interval());
        Ok(())
    }

    #[test]
    fn announcement_driver_rejects_invalid_inputs_without_io() {
        let mut source = FakeAnnouncementSource {
            identity: announcement_identity(),
            pages: VecDeque::new(),
            calls: Vec::new(),
        };
        assert!(read_complete_announcement_window(&mut source, -2, 1, 1).is_err());
        assert!(read_complete_announcement_window(&mut source, -1, 0, 1).is_err());
        assert!(read_complete_announcement_window(&mut source, -1, 1, 0).is_err());
        assert!(source.calls.is_empty());
    }
}
