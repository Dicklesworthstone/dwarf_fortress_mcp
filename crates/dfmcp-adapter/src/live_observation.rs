#![forbid(unsafe_code)]

//! Canonical assembly of one live read-only DFHack observation.
//!
//! Transport pagination is not state semantics. The same fortress observation
//! returned in one page or many must produce byte-identical canonical bytes
//! and the same capsule digest. The assembler therefore requires contiguous
//! pages, stable summary fields, one bridge generation, strict unit-ID order,
//! and a complete final coverage proof before publication.

use dfmcp_core::{DfmcpError, Digest32, ErrorCode, Result, sha256};

use crate::{BridgeManifest, CitizenRecord, ObservationPage};

const CAPSULE_DOMAIN: &[u8] = b"dfmcp.live-observation-capsule.v1\0";
pub const MAX_CAPSULE_CITIZENS: usize = 100_000;
pub const MAX_CANONICAL_CAPSULE_BYTES: usize = 64 * 1024 * 1024;

fn error(code: ErrorCode, message: impl Into<String>) -> DfmcpError {
    DfmcpError::new(code, message)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CitizenCoverage {
    pub offset: u32,
    pub returned: u32,
    pub total: u32,
    pub complete: bool,
}

impl CitizenCoverage {
    #[must_use]
    pub const fn proves_complete_roster(&self) -> bool {
        self.complete && self.offset == 0 && self.returned == self.total
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveObservationCapsule {
    pub bridge: BridgeManifest,
    pub paused: bool,
    pub current_year: u32,
    pub current_year_tick: u32,
    pub world_name: String,
    pub world_folder: String,
    pub site_id: i32,
    pub citizen_coverage: CitizenCoverage,
    pub citizens: Vec<CitizenRecord>,
    pub canonical_bytes: Vec<u8>,
    pub content_digest: Digest32,
}

impl LiveObservationCapsule {
    pub fn validate(&self) -> Result<()> {
        if !self.bridge.world_loaded || !self.bridge.fortress_mode {
            return Err(error(
                ErrorCode::AdapterRejected,
                "a live observation capsule requires a loaded fortress-mode world",
            ));
        }
        if !self.citizen_coverage.proves_complete_roster() {
            return Err(error(
                ErrorCode::CursorGap,
                "a published live observation capsule requires complete citizen coverage",
            ));
        }
        if usize::try_from(self.citizen_coverage.returned).map_err(|_| {
            error(
                ErrorCode::InternalInvariantViolation,
                "citizen coverage does not fit usize",
            )
        })? != self.citizens.len()
        {
            return Err(error(
                ErrorCode::InternalInvariantViolation,
                "citizen coverage does not match the canonical roster length",
            ));
        }
        if self.canonical_bytes.len() > MAX_CANONICAL_CAPSULE_BYTES {
            return Err(error(
                ErrorCode::BudgetExceeded,
                "canonical live observation exceeds the 64 MiB capsule ceiling",
            ));
        }
        if sha256(&self.canonical_bytes) != self.content_digest {
            return Err(error(
                ErrorCode::ChecksumMismatch,
                "live observation capsule digest does not match its canonical bytes",
            ));
        }
        for pair in self.citizens.windows(2) {
            if pair[0].unit_id >= pair[1].unit_id {
                return Err(error(
                    ErrorCode::InternalInvariantViolation,
                    "canonical citizen roster is not in strict unit-ID order",
                ));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SummaryIdentity {
    paused: bool,
    current_year: u32,
    current_year_tick: u32,
    world_name: String,
    world_folder: String,
    site_id: i32,
    total: u32,
}

impl SummaryIdentity {
    fn from_page(page: &ObservationPage) -> Self {
        Self {
            paused: page.paused,
            current_year: page.current_year,
            current_year_tick: page.current_year_tick,
            world_name: page.world_name.clone(),
            world_folder: page.world_folder.clone(),
            site_id: page.site_id,
            total: page.citizen_count_total,
        }
    }
}

#[derive(Clone, Debug)]
pub struct ObservationAssembler {
    bridge: BridgeManifest,
    summary: Option<SummaryIdentity>,
    citizens: Vec<CitizenRecord>,
    complete: bool,
}

impl ObservationAssembler {
    #[must_use]
    pub fn new(bridge: BridgeManifest) -> Self {
        Self {
            bridge,
            summary: None,
            citizens: Vec::new(),
            complete: false,
        }
    }

    #[must_use]
    pub fn next_offset(&self) -> u32 {
        u32::try_from(self.citizens.len()).unwrap_or(u32::MAX)
    }

    #[must_use]
    pub const fn is_complete(&self) -> bool {
        self.complete
    }

    pub fn push_page(&mut self, page: ObservationPage) -> Result<()> {
        if self.complete {
            return Err(error(
                ErrorCode::InvalidRequest,
                "cannot append a page after complete observation coverage",
            ));
        }
        if page.bridge_generation != self.bridge.bridge_generation {
            return Err(error(
                ErrorCode::StaleAnchor,
                "bridge generation changed while assembling an observation",
            ));
        }
        if !page.world_loaded || !page.fortress_mode {
            return Err(error(
                ErrorCode::AdapterRejected,
                "live observation page is not from a loaded fortress-mode world",
            ));
        }
        if !self.bridge.world_loaded || !self.bridge.fortress_mode {
            return Err(error(
                ErrorCode::StaleAnchor,
                "handshake world posture no longer matches the observation request",
            ));
        }
        let expected_offset = u32::try_from(self.citizens.len()).map_err(|_| {
            error(
                ErrorCode::BudgetExceeded,
                "assembled citizen offset does not fit u32",
            )
        })?;
        if page.citizen_offset != expected_offset {
            return Err(error(
                ErrorCode::CursorGap,
                format!(
                    "citizen page begins at {}, expected contiguous offset {expected_offset}",
                    page.citizen_offset
                ),
            ));
        }

        let candidate_summary = SummaryIdentity::from_page(&page);
        if let Some(summary) = self.summary.as_ref() {
            if summary != &candidate_summary {
                return Err(error(
                    ErrorCode::StaleAnchor,
                    "fortress summary changed between citizen pages",
                ));
            }
        } else {
            self.summary = Some(candidate_summary);
        }

        if self.citizens.len().saturating_add(page.citizens.len()) > MAX_CAPSULE_CITIZENS {
            return Err(error(
                ErrorCode::BudgetExceeded,
                "assembled observation exceeds the 100,000-citizen safety ceiling",
            ));
        }
        if let (Some(previous), Some(first)) = (self.citizens.last(), page.citizens.first()) {
            if previous.unit_id >= first.unit_id {
                return Err(error(
                    ErrorCode::AdapterRejected,
                    "citizen order is not strict across page boundaries",
                ));
            }
        }
        for pair in page.citizens.windows(2) {
            if pair[0].unit_id >= pair[1].unit_id {
                return Err(error(
                    ErrorCode::AdapterRejected,
                    "citizen page is not in strict unit-ID order",
                ));
            }
        }
        self.citizens.extend(page.citizens);

        let assembled = u32::try_from(self.citizens.len()).map_err(|_| {
            error(
                ErrorCode::BudgetExceeded,
                "assembled citizen count does not fit u32",
            )
        })?;
        let total = self
            .summary
            .as_ref()
            .map(|summary| summary.total)
            .ok_or_else(|| {
                error(
                    ErrorCode::InternalInvariantViolation,
                    "observation summary disappeared during assembly",
                )
            })?;
        if assembled > total {
            return Err(error(
                ErrorCode::AdapterRejected,
                "assembled citizen count exceeds the declared total",
            ));
        }
        if page.complete != (assembled == total) {
            return Err(error(
                ErrorCode::AdapterRejected,
                "page completeness disagrees with assembled coverage",
            ));
        }
        self.complete = page.complete;
        Ok(())
    }

    pub fn finalize(self) -> Result<LiveObservationCapsule> {
        if !self.complete {
            return Err(error(
                ErrorCode::CursorGap,
                "cannot publish an incomplete live observation",
            ));
        }
        let summary = self.summary.ok_or_else(|| {
            error(
                ErrorCode::InvalidRequest,
                "cannot publish an observation with no pages",
            )
        })?;
        let returned = u32::try_from(self.citizens.len()).map_err(|_| {
            error(
                ErrorCode::BudgetExceeded,
                "canonical citizen count does not fit u32",
            )
        })?;
        if returned != summary.total {
            return Err(error(
                ErrorCode::CursorGap,
                "complete observation does not contain the declared citizen total",
            ));
        }

        let canonical_bytes = canonical_bytes(&self.bridge, &summary, &self.citizens)?;
        if canonical_bytes.len() > MAX_CANONICAL_CAPSULE_BYTES {
            return Err(error(
                ErrorCode::BudgetExceeded,
                "canonical live observation exceeds the 64 MiB capsule ceiling",
            ));
        }
        let content_digest = sha256(&canonical_bytes);
        let capsule = LiveObservationCapsule {
            bridge: self.bridge,
            paused: summary.paused,
            current_year: summary.current_year,
            current_year_tick: summary.current_year_tick,
            world_name: summary.world_name,
            world_folder: summary.world_folder,
            site_id: summary.site_id,
            citizen_coverage: CitizenCoverage {
                offset: 0,
                returned,
                total: summary.total,
                complete: true,
            },
            citizens: self.citizens,
            canonical_bytes,
            content_digest,
        };
        capsule.validate()?;
        Ok(capsule)
    }
}

fn push_u8(output: &mut Vec<u8>, value: u8) {
    output.push(value);
}

fn push_u32(output: &mut Vec<u8>, value: u32) {
    output.extend_from_slice(&value.to_le_bytes());
}

fn push_i32(output: &mut Vec<u8>, value: i32) {
    output.extend_from_slice(&value.to_le_bytes());
}

fn push_u64(output: &mut Vec<u8>, value: u64) {
    output.extend_from_slice(&value.to_le_bytes());
}

fn push_bool(output: &mut Vec<u8>, value: bool) {
    push_u8(output, u8::from(value));
}

fn push_bytes(output: &mut Vec<u8>, value: &[u8]) -> Result<()> {
    let length = u32::try_from(value.len()).map_err(|_| {
        error(
            ErrorCode::BudgetExceeded,
            "canonical field length does not fit u32",
        )
    })?;
    push_u32(output, length);
    output.extend_from_slice(value);
    Ok(())
}

fn push_string(output: &mut Vec<u8>, value: &str) -> Result<()> {
    push_bytes(output, value.as_bytes())
}

fn canonical_bytes(
    bridge: &BridgeManifest,
    summary: &SummaryIdentity,
    citizens: &[CitizenRecord],
) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    output.extend_from_slice(CAPSULE_DOMAIN);
    push_string(&mut output, &bridge.bridge_version)?;
    push_string(&mut output, &bridge.dfhack_version)?;
    push_string(&mut output, &bridge.df_version)?;
    push_bool(&mut output, bridge.world_loaded);
    push_bool(&mut output, bridge.fortress_mode);
    push_u64(&mut output, bridge.bridge_generation);
    push_u32(
        &mut output,
        u32::try_from(bridge.supported_methods.len()).map_err(|_| {
            error(
                ErrorCode::BudgetExceeded,
                "bridge method count does not fit u32",
            )
        })?,
    );
    for method in &bridge.supported_methods {
        push_string(&mut output, method)?;
    }

    push_bool(&mut output, summary.paused);
    push_u32(&mut output, summary.current_year);
    push_u32(&mut output, summary.current_year_tick);
    push_string(&mut output, &summary.world_name)?;
    push_string(&mut output, &summary.world_folder)?;
    push_i32(&mut output, summary.site_id);
    push_u32(&mut output, summary.total);

    for citizen in citizens {
        push_i32(&mut output, citizen.unit_id);
        push_string(&mut output, &citizen.name)?;
        push_string(&mut output, &citizen.race)?;
        push_i32(&mut output, citizen.profession);
        push_i32(&mut output, citizen.x);
        push_i32(&mut output, citizen.y);
        push_i32(&mut output, citizen.z);
        push_bool(&mut output, citizen.alive);
        push_bool(&mut output, citizen.sane);
        push_bool(&mut output, citizen.active);
        push_bool(&mut output, citizen.visible);
        push_bool(&mut output, citizen.citizen);
        push_bool(&mut output, citizen.resident);
        push_bool(&mut output, citizen.baby);
        push_bool(&mut output, citizen.child);
        push_bool(&mut output, citizen.adult);
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

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
            current_year_tick: 12345,
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
    fn pagination_does_not_change_capsule_identity() -> Result<()> {
        let mut one_page = ObservationAssembler::new(manifest());
        one_page.push_page(page(0, 4, &[1, 2, 3, 4], true))?;
        let one = one_page.finalize()?;

        let mut many_pages = ObservationAssembler::new(manifest());
        many_pages.push_page(page(0, 4, &[1, 2], false))?;
        many_pages.push_page(page(2, 4, &[3], false))?;
        many_pages.push_page(page(3, 4, &[4], true))?;
        let many = many_pages.finalize()?;

        assert_eq!(one.content_digest, many.content_digest);
        assert_eq!(one.canonical_bytes, many.canonical_bytes);
        assert_eq!(one.citizens, many.citizens);
        Ok(())
    }

    #[test]
    fn summary_drift_between_pages_is_rejected() -> Result<()> {
        let mut assembler = ObservationAssembler::new(manifest());
        assembler.push_page(page(0, 2, &[1], false))?;
        let mut changed = page(1, 2, &[2], true);
        changed.current_year_tick = 12346;
        assert!(assembler.push_page(changed).is_err());
        Ok(())
    }

    #[test]
    fn gaps_and_cross_page_reordering_are_rejected() -> Result<()> {
        let mut gap = ObservationAssembler::new(manifest());
        gap.push_page(page(0, 3, &[1], false))?;
        assert!(gap.push_page(page(2, 3, &[3], true)).is_err());

        let mut reordered = ObservationAssembler::new(manifest());
        reordered.push_page(page(0, 2, &[2], false))?;
        assert!(reordered.push_page(page(1, 2, &[1], true)).is_err());
        Ok(())
    }

    #[test]
    fn incomplete_assembly_cannot_publish() -> Result<()> {
        let mut assembler = ObservationAssembler::new(manifest());
        assembler.push_page(page(0, 2, &[1], false))?;
        assert!(assembler.finalize().is_err());
        Ok(())
    }
}
