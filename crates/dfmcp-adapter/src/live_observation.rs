#![forbid(unsafe_code)]

//! Canonical assembly of one live read-only DFHack observation.
//!
//! Transport pagination is not state semantics. The same fortress observation
//! returned in one page or many must produce byte-identical canonical bytes
//! and the same capsule digest. The assembler therefore requires contiguous
//! pages, stable summary fields, one bridge generation, strict unit-ID order,
//! and a complete final coverage proof before publication.

use dfmcp_core::{DfmcpError, Digest32, ErrorCode, Result, sha256};

use crate::dfhack_rpc::{
    MAX_CITIZENS_PER_PAGE, MAX_RACE_NAME_BYTES, MAX_UNIT_NAME_BYTES,
    MAX_WORLD_FOLDER_BYTES, MAX_WORLD_NAME_BYTES,
};
use crate::{BridgeManifest, CitizenRecord, ObservationPage};

const CAPSULE_DOMAIN: &[u8] = b"dfmcp.live-observation-capsule.v2\0";
const TICKS_PER_YEAR: u32 = 403_200;
pub const MAX_CAPSULE_CITIZENS: usize = 100_000;
pub const MAX_CANONICAL_CAPSULE_BYTES: usize = 64 * 1024 * 1024;

fn error(code: ErrorCode, message: impl Into<String>) -> DfmcpError {
    DfmcpError::new(code, message)
}

fn validate_string_bound(value: &str, field: &str, maximum: usize) -> Result<()> {
    if value.len() > maximum {
        return Err(error(
            ErrorCode::BudgetExceeded,
            format!("{field} exceeds its {maximum}-byte bound"),
        ));
    }
    Ok(())
}

fn validate_nonempty_string_bound(value: &str, field: &str, maximum: usize) -> Result<()> {
    if value.is_empty() {
        return Err(error(
            ErrorCode::AdapterRejected,
            format!("{field} must not be empty"),
        ));
    }
    validate_string_bound(value, field, maximum)
}

fn validate_summary(summary: &SummaryIdentity) -> Result<()> {
    validate_nonempty_string_bound(&summary.world_name, "world name", MAX_WORLD_NAME_BYTES)?;
    validate_nonempty_string_bound(
        &summary.world_folder,
        "world folder",
        MAX_WORLD_FOLDER_BYTES,
    )?;
    if summary.site_id < 0 {
        return Err(error(
            ErrorCode::AdapterRejected,
            "live observation site ID must not be negative",
        ));
    }
    if summary.current_year_tick >= TICKS_PER_YEAR {
        return Err(error(
            ErrorCode::AdapterRejected,
            format!(
                "live observation year tick {} is outside 0..{}",
                summary.current_year_tick,
                TICKS_PER_YEAR - 1
            ),
        ));
    }
    let total = usize::try_from(summary.total).map_err(|_| {
        error(
            ErrorCode::BudgetExceeded,
            "declared citizen total does not fit usize",
        )
    })?;
    if total > MAX_CAPSULE_CITIZENS {
        return Err(error(
            ErrorCode::BudgetExceeded,
            "declared citizen total exceeds the capsule safety ceiling",
        ));
    }
    Ok(())
}

fn validate_citizen(citizen: &CitizenRecord, names_included: bool) -> Result<()> {
    if citizen.unit_id < 0 {
        return Err(error(
            ErrorCode::AdapterRejected,
            "citizen unit ID must not be negative",
        ));
    }
    validate_string_bound(&citizen.name, "citizen name", MAX_UNIT_NAME_BYTES)?;
    validate_nonempty_string_bound(&citizen.race, "citizen race", MAX_RACE_NAME_BYTES)?;
    if !names_included && !citizen.name.is_empty() {
        return Err(error(
            ErrorCode::CorruptLedger,
            "name-omitted live observation contains an observed citizen name",
        ));
    }
    if !citizen.citizen || citizen.resident {
        return Err(error(
            ErrorCode::AdapterRejected,
            "strict citizen observation contains a non-citizen or resident record",
        ));
    }
    Ok(())
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
    /// Whether citizen names were requested and therefore observed. An empty
    /// string with this flag set is an observed empty/unnamed value; an empty
    /// string with this flag clear is an explicit omission placeholder.
    pub names_included: bool,
    pub citizen_coverage: CitizenCoverage,
    pub citizens: Vec<CitizenRecord>,
    pub canonical_bytes: Vec<u8>,
    pub content_digest: Digest32,
}

impl LiveObservationCapsule {
    pub fn validate(&self) -> Result<()> {
        self.bridge.validate()?;
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
        if self.citizens.len() > MAX_CAPSULE_CITIZENS {
            return Err(error(
                ErrorCode::BudgetExceeded,
                "live observation capsule exceeds the citizen safety ceiling",
            ));
        }
        if self.canonical_bytes.len() > MAX_CANONICAL_CAPSULE_BYTES {
            return Err(error(
                ErrorCode::BudgetExceeded,
                "canonical live observation exceeds the 64 MiB capsule ceiling",
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
        let summary = SummaryIdentity {
            paused: self.paused,
            current_year: self.current_year,
            current_year_tick: self.current_year_tick,
            world_name: self.world_name.clone(),
            world_folder: self.world_folder.clone(),
            site_id: self.site_id,
            total: self.citizen_coverage.total,
        };
        validate_summary(&summary)?;
        for citizen in &self.citizens {
            validate_citizen(citizen, self.names_included)?;
        }
        for pair in self.citizens.windows(2) {
            if pair[0].unit_id >= pair[1].unit_id {
                return Err(error(
                    ErrorCode::InternalInvariantViolation,
                    "canonical citizen roster is not in strict unit-ID order",
                ));
            }
        }

        let recomputed = canonical_bytes(
            &self.bridge,
            &summary,
            self.names_included,
            &self.citizens,
        )?;
        if recomputed != self.canonical_bytes {
            return Err(error(
                ErrorCode::CorruptLedger,
                "live observation fields do not reproduce the stored canonical bytes",
            ));
        }
        if sha256(&self.canonical_bytes) != self.content_digest {
            return Err(error(
                ErrorCode::CorruptLedger,
                "live observation capsule digest does not match its canonical bytes",
            ));
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
    names_included: bool,
    summary: Option<SummaryIdentity>,
    citizens: Vec<CitizenRecord>,
    complete: bool,
}

impl ObservationAssembler {
    /// Construct an assembler for the full-name projection retained by the
    /// original API. Call [`Self::with_names`] when the projection is explicit.
    #[must_use]
    pub fn new(bridge: BridgeManifest) -> Self {
        Self::with_names(bridge, true)
    }

    #[must_use]
    pub fn with_names(bridge: BridgeManifest, names_included: bool) -> Self {
        Self {
            bridge,
            names_included,
            summary: None,
            citizens: Vec::new(),
            complete: false,
        }
    }

    pub fn next_offset(&self) -> Result<u32> {
        u32::try_from(self.citizens.len()).map_err(|_| {
            error(
                ErrorCode::BudgetExceeded,
                "assembled citizen offset does not fit u32",
            )
        })
    }

    #[must_use]
    pub const fn is_complete(&self) -> bool {
        self.complete
    }

    pub fn push_page(&mut self, page: ObservationPage) -> Result<()> {
        self.bridge.validate()?;
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
        let expected_offset = self.next_offset()?;
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
        validate_summary(&candidate_summary)?;
        if let Some(summary) = self.summary.as_ref()
            && summary != &candidate_summary
        {
            return Err(error(
                ErrorCode::StaleAnchor,
                "fortress summary changed between citizen pages",
            ));
        }
        let page_limit = usize::try_from(MAX_CITIZENS_PER_PAGE).map_err(|_| {
            error(
                ErrorCode::InternalInvariantViolation,
                "citizen page limit does not fit usize",
            )
        })?;
        if page.citizens.len() > page_limit {
            return Err(error(
                ErrorCode::BudgetExceeded,
                "observation page exceeds the bridge citizen-page ceiling",
            ));
        }
        for citizen in &page.citizens {
            validate_citizen(citizen, self.names_included)?;
        }
        if let (Some(previous), Some(first)) = (self.citizens.last(), page.citizens.first())
            && previous.unit_id >= first.unit_id
        {
            return Err(error(
                ErrorCode::AdapterRejected,
                "citizen order is not strict across page boundaries",
            ));
        }
        for pair in page.citizens.windows(2) {
            if pair[0].unit_id >= pair[1].unit_id {
                return Err(error(
                    ErrorCode::AdapterRejected,
                    "citizen page is not in strict unit-ID order",
                ));
            }
        }

        let candidate_len = self
            .citizens
            .len()
            .checked_add(page.citizens.len())
            .ok_or_else(|| {
                error(
                    ErrorCode::BudgetExceeded,
                    "assembled observation citizen count overflowed",
                )
            })?;
        if candidate_len > MAX_CAPSULE_CITIZENS {
            return Err(error(
                ErrorCode::BudgetExceeded,
                "assembled observation exceeds the 100,000-citizen safety ceiling",
            ));
        }
        let assembled = u32::try_from(candidate_len).map_err(|_| {
            error(
                ErrorCode::BudgetExceeded,
                "assembled citizen count does not fit u32",
            )
        })?;
        if assembled > candidate_summary.total {
            return Err(error(
                ErrorCode::AdapterRejected,
                "assembled citizen count exceeds the declared total",
            ));
        }
        if page.complete != (assembled == candidate_summary.total) {
            return Err(error(
                ErrorCode::AdapterRejected,
                "page completeness disagrees with assembled coverage",
            ));
        }

        if self.summary.is_none() {
            self.summary = Some(candidate_summary);
        }
        self.citizens.extend(page.citizens);
        self.complete = page.complete;
        Ok(())
    }

    pub fn finalize(self) -> Result<LiveObservationCapsule> {
        self.bridge.validate()?;
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
        validate_summary(&summary)?;
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

        let canonical_bytes = canonical_bytes(
            &self.bridge,
            &summary,
            self.names_included,
            &self.citizens,
        )?;
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
            names_included: self.names_included,
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
    names_included: bool,
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

    push_bool(&mut output, names_included);
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
    fn name_projection_is_part_of_capsule_identity() -> Result<()> {
        let mut included = ObservationAssembler::with_names(manifest(), true);
        included.push_page(page_without_names(0, 1, &[1], true))?;
        let included = included.finalize()?;

        let mut omitted = ObservationAssembler::with_names(manifest(), false);
        omitted.push_page(page_without_names(0, 1, &[1], true))?;
        let omitted = omitted.finalize()?;

        assert!(included.names_included);
        assert!(!omitted.names_included);
        assert_ne!(included.content_digest, omitted.content_digest);
        assert_ne!(included.canonical_bytes, omitted.canonical_bytes);
        Ok(())
    }

    #[test]
    fn name_omitted_assembler_rejects_observed_names() {
        let mut assembler = ObservationAssembler::with_names(manifest(), false);
        assert!(assembler.push_page(page(0, 1, &[1], true)).is_err());
    }

    #[test]
    fn structured_field_tampering_invalidates_the_capsule() -> Result<()> {
        let mut assembler = ObservationAssembler::new(manifest());
        assembler.push_page(page(0, 1, &[1], true))?;
        let mut capsule = assembler.finalize()?;
        capsule.citizens[0].x = 999;
        assert!(capsule.validate().is_err());
        Ok(())
    }

    #[test]
    fn canonical_byte_tampering_invalidates_the_capsule() -> Result<()> {
        let mut assembler = ObservationAssembler::new(manifest());
        assembler.push_page(page(0, 1, &[1], true))?;
        let mut capsule = assembler.finalize()?;
        capsule.canonical_bytes.push(0);
        assert!(capsule.validate().is_err());
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

    #[test]
    fn rejected_page_does_not_partially_mutate_assembler() -> Result<()> {
        let mut assembler = ObservationAssembler::new(manifest());
        assembler.push_page(page(0, 2, &[1], false))?;
        let offset_before = assembler.next_offset()?;

        let invalid = page(1, 2, &[2], false);
        assert!(assembler.push_page(invalid).is_err());
        assert_eq!(assembler.next_offset()?, offset_before);
        assert!(!assembler.is_complete());

        assembler.push_page(page(1, 2, &[2], true))?;
        assert!(assembler.is_complete());
        assert_eq!(assembler.finalize()?.citizens.len(), 2);
        Ok(())
    }

    #[test]
    fn rejected_first_page_does_not_capture_summary() -> Result<()> {
        let mut assembler = ObservationAssembler::new(manifest());
        let invalid = page(0, 1, &[1], false);
        assert!(assembler.push_page(invalid).is_err());
        assert_eq!(assembler.next_offset()?, 0);

        let mut corrected = page(0, 1, &[1], true);
        corrected.world_name = "A Different Valid Realm".to_owned();
        assembler.push_page(corrected)?;
        assert_eq!(assembler.finalize()?.world_name, "A Different Valid Realm");
        Ok(())
    }

    #[test]
    fn invalid_manifest_summary_and_citizen_semantics_fail_closed() {
        let mut invalid_manifest = manifest();
        invalid_manifest.bridge_generation = 0;
        let mut assembler = ObservationAssembler::new(invalid_manifest);
        assert!(assembler.push_page(page(0, 1, &[1], true)).is_err());
        assert_eq!(assembler.next_offset().ok(), Some(0));

        let mut assembler = ObservationAssembler::new(manifest());
        let mut invalid_summary = page(0, 1, &[1], true);
        invalid_summary.site_id = -1;
        assert!(assembler.push_page(invalid_summary).is_err());
        assert_eq!(assembler.next_offset().ok(), Some(0));

        let mut invalid_citizen = page(0, 1, &[1], true);
        invalid_citizen.citizens[0].resident = true;
        assert!(assembler.push_page(invalid_citizen).is_err());
        assert_eq!(assembler.next_offset().ok(), Some(0));
    }
}
