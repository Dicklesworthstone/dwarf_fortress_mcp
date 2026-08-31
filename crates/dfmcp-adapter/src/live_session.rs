#![forbid(unsafe_code)]

//! Bounded driver for obtaining one complete canonical live observation.
//!
//! This layer owns pagination policy but no transport implementation. A source
//! may be the real [`DfHackRpcClient`](crate::DfHackRpcClient) or a deterministic
//! laboratory double. It refuses empty nonterminal pages, total counts above
//! the caller or capsule ceiling, page-count overruns, and assembler drift.

use std::io::{Read, Write};

use dfmcp_core::{DfmcpError, ErrorCode, Result};

use crate::{
    BridgeManifest, DfHackRpcClient, LiveObservationCapsule, MAX_CAPSULE_CITIZENS,
    MAX_CITIZENS_PER_PAGE, ObservationAssembler, ObservationPage,
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
    let mut assembler = ObservationAssembler::new(manifest);
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::CitizenRecord;

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

    #[test]
    fn drives_multiple_pages_to_one_complete_capsule() -> Result<()> {
        let mut source = source(vec![page(0, 3, &[1, 2], false), page(2, 3, &[3], true)]);
        let capsule = read_complete_observation(&mut source, 2, true)?;
        assert!(capsule.citizen_coverage.proves_complete_roster());
        assert_eq!(capsule.citizens.len(), 3);
        assert_eq!(source.calls, 2);
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
}
