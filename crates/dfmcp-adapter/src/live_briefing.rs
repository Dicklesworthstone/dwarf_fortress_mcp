#![forbid(unsafe_code)]

//! Deterministic agent briefing and semantic change summary for the first live
//! DFHack read slice.
//!
//! This module never invents game facts. It summarizes only the complete
//! fortress/citizen capsule and explicitly lists every major domain that V1
//! does not observe. Attention candidates are mechanical findings with score
//! components, not authority or autonomous action recommendations.

use std::collections::{BTreeMap, BTreeSet};

use dfmcp_core::{DfmcpError, Digest32, ErrorCode, Result};

use crate::{CitizenRecord, LiveObservationCapsule};

pub const MAX_BRIEFING_ATTENTION_ITEMS: usize = 64;
pub const MAX_BRIEFING_CHANGE_IDS: usize = 1_024;

fn error(code: ErrorCode, message: impl Into<String>) -> DfmcpError {
    DfmcpError::new(code, message)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LiveCoverageDomain {
    FortressIdentity,
    Calendar,
    PauseState,
    CitizenRoster,
    CitizenBasicStatus,
    CitizenPosition,
    Map,
    Items,
    Buildings,
    Jobs,
    WorkOrders,
    Economy,
    FoodAndDrink,
    WelfareAndThoughts,
    Health,
    Military,
    Threats,
    Announcements,
    HistoricalContext,
}

impl LiveCoverageDomain {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::FortressIdentity => "fortress_identity",
            Self::Calendar => "calendar",
            Self::PauseState => "pause_state",
            Self::CitizenRoster => "citizen_roster",
            Self::CitizenBasicStatus => "citizen_basic_status",
            Self::CitizenPosition => "citizen_position",
            Self::Map => "map",
            Self::Items => "items",
            Self::Buildings => "buildings",
            Self::Jobs => "jobs",
            Self::WorkOrders => "work_orders",
            Self::Economy => "economy",
            Self::FoodAndDrink => "food_and_drink",
            Self::WelfareAndThoughts => "welfare_and_thoughts",
            Self::Health => "health",
            Self::Military => "military",
            Self::Threats => "threats",
            Self::Announcements => "announcements",
            Self::HistoricalContext => "historical_context",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LiveCoverageStatus {
    Complete,
    Omitted,
}

impl LiveCoverageStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Complete => "complete",
            Self::Omitted => "omitted",
        }
    }

    #[must_use]
    pub const fn can_prove_absence(self) -> bool {
        matches!(self, Self::Complete)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveCoverageEntry {
    pub domain: LiveCoverageDomain,
    pub status: LiveCoverageStatus,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CitizenStatusCounts {
    pub total: u32,
    pub alive: u32,
    pub sane: u32,
    pub active: u32,
    pub visible: u32,
    pub citizens: u32,
    pub residents: u32,
    pub babies: u32,
    pub children: u32,
    pub adults: u32,
}

impl CitizenStatusCounts {
    fn from_citizens(citizens: &[CitizenRecord]) -> Result<Self> {
        let total = u32::try_from(citizens.len()).map_err(|_| {
            error(
                ErrorCode::BudgetExceeded,
                "citizen status count does not fit u32",
            )
        })?;
        let count = |predicate: fn(&CitizenRecord) -> bool| -> Result<u32> {
            u32::try_from(citizens.iter().filter(|unit| predicate(unit)).count()).map_err(|_| {
                error(
                    ErrorCode::BudgetExceeded,
                    "citizen status subtotal does not fit u32",
                )
            })
        };
        Ok(Self {
            total,
            alive: count(|unit| unit.alive)?,
            sane: count(|unit| unit.sane)?,
            active: count(|unit| unit.active)?,
            visible: count(|unit| unit.visible)?,
            citizens: count(|unit| unit.citizen)?,
            residents: count(|unit| unit.resident)?,
            babies: count(|unit| unit.baby)?,
            children: count(|unit| unit.child)?,
            adults: count(|unit| unit.adult)?,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LiveAttentionSeverity {
    Critical,
    High,
    Medium,
    Low,
}

impl LiveAttentionSeverity {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Critical => "critical",
            Self::High => "high",
            Self::Medium => "medium",
            Self::Low => "low",
        }
    }

    #[must_use]
    const fn rank(self) -> u8 {
        match self {
            Self::Critical => 0,
            Self::High => 1,
            Self::Medium => 2,
            Self::Low => 3,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveAttentionItem {
    pub attention_id: String,
    pub severity: LiveAttentionSeverity,
    pub category: String,
    pub finding: String,
    pub affected_unit_ids: Vec<i32>,
    pub score_components: BTreeMap<String, i64>,
    pub source_digest: Digest32,
}

impl LiveAttentionItem {
    fn sort_key(&self) -> (u8, &str, &str) {
        (
            self.severity.rank(),
            self.category.as_str(),
            self.attention_id.as_str(),
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveFortressBriefing {
    pub source_digest: Digest32,
    pub bridge_generation: u64,
    pub dwarf_fortress_version: String,
    pub dfhack_version: String,
    pub bridge_version: String,
    pub site_id: i32,
    pub world_name: String,
    pub world_folder: String,
    pub paused: bool,
    pub current_year: u32,
    pub current_year_tick: u32,
    pub citizen_status: CitizenStatusCounts,
    pub coverage: Vec<LiveCoverageEntry>,
    pub attention: Vec<LiveAttentionItem>,
}

impl LiveFortressBriefing {
    #[must_use]
    pub fn coverage_status(&self, domain: LiveCoverageDomain) -> Option<LiveCoverageStatus> {
        self.coverage
            .iter()
            .find(|entry| entry.domain == domain)
            .map(|entry| entry.status)
    }

    #[must_use]
    pub fn can_prove_absence_in(&self, domain: LiveCoverageDomain) -> bool {
        self.coverage_status(domain)
            .is_some_and(LiveCoverageStatus::can_prove_absence)
    }
}

fn complete(domain: LiveCoverageDomain) -> LiveCoverageEntry {
    LiveCoverageEntry {
        domain,
        status: LiveCoverageStatus::Complete,
        reason: None,
    }
}

fn omitted(domain: LiveCoverageDomain, reason: &str) -> LiveCoverageEntry {
    LiveCoverageEntry {
        domain,
        status: LiveCoverageStatus::Omitted,
        reason: Some(reason.to_owned()),
    }
}

fn coverage() -> Vec<LiveCoverageEntry> {
    vec![
        complete(LiveCoverageDomain::FortressIdentity),
        complete(LiveCoverageDomain::Calendar),
        complete(LiveCoverageDomain::PauseState),
        complete(LiveCoverageDomain::CitizenRoster),
        complete(LiveCoverageDomain::CitizenBasicStatus),
        complete(LiveCoverageDomain::CitizenPosition),
        omitted(LiveCoverageDomain::Map, "DFHack read bridge V1 does not observe map tiles"),
        omitted(LiveCoverageDomain::Items, "DFHack read bridge V1 does not observe items"),
        omitted(LiveCoverageDomain::Buildings, "DFHack read bridge V1 does not observe buildings"),
        omitted(LiveCoverageDomain::Jobs, "DFHack read bridge V1 does not observe jobs"),
        omitted(LiveCoverageDomain::WorkOrders, "DFHack read bridge V1 does not observe work orders"),
        omitted(LiveCoverageDomain::Economy, "DFHack read bridge V1 has no economic aggregates"),
        omitted(LiveCoverageDomain::FoodAndDrink, "DFHack read bridge V1 has no food or drink inventory"),
        omitted(LiveCoverageDomain::WelfareAndThoughts, "DFHack read bridge V1 has no thoughts, needs, or stress"),
        omitted(LiveCoverageDomain::Health, "DFHack read bridge V1 has only basic alive/sane status"),
        omitted(LiveCoverageDomain::Military, "DFHack read bridge V1 intentionally omits military state"),
        omitted(LiveCoverageDomain::Threats, "DFHack read bridge V1 has no hostile-unit or siege projection"),
        omitted(LiveCoverageDomain::Announcements, "DFHack read bridge V1 has no announcement stream"),
        omitted(LiveCoverageDomain::HistoricalContext, "DFHack read bridge V1 has no historical graph"),
    ]
}

fn bounded_ids<'a>(
    units: impl Iterator<Item = &'a CitizenRecord>,
) -> (Vec<i32>, bool) {
    let mut ids = Vec::new();
    let mut truncated = false;
    for unit in units {
        if ids.len() >= MAX_BRIEFING_CHANGE_IDS {
            truncated = true;
            break;
        }
        ids.push(unit.unit_id);
    }
    (ids, truncated)
}

fn bounded_i64(value: usize) -> i64 {
    i64::try_from(value).map_or(i64::MAX, |converted| converted)
}

fn attention(capsule: &LiveObservationCapsule) -> Vec<LiveAttentionItem> {
    let mut items = Vec::new();
    let source = capsule.content_digest;

    let (not_alive, not_alive_truncated) = bounded_ids(capsule.citizens.iter().filter(|unit| !unit.alive));
    if !not_alive.is_empty() || not_alive_truncated {
        let mut score = BTreeMap::new();
        score.insert("affected_units".to_owned(), bounded_i64(not_alive.len()));
        score.insert("ids_truncated".to_owned(), i64::from(not_alive_truncated));
        items.push(LiveAttentionItem {
            attention_id: "live.basic_status.not_alive".to_owned(),
            severity: LiveAttentionSeverity::Critical,
            category: "citizen_basic_status".to_owned(),
            finding: "one or more records in the complete citizen roster are not marked alive".to_owned(),
            affected_unit_ids: not_alive,
            score_components: score,
            source_digest: source,
        });
    }

    let (not_sane, not_sane_truncated) = bounded_ids(capsule.citizens.iter().filter(|unit| !unit.sane));
    if !not_sane.is_empty() || not_sane_truncated {
        let mut score = BTreeMap::new();
        score.insert("affected_units".to_owned(), bounded_i64(not_sane.len()));
        score.insert("ids_truncated".to_owned(), i64::from(not_sane_truncated));
        items.push(LiveAttentionItem {
            attention_id: "live.basic_status.not_sane".to_owned(),
            severity: LiveAttentionSeverity::High,
            category: "citizen_basic_status".to_owned(),
            finding: "one or more citizens are not marked sane by DFHack".to_owned(),
            affected_unit_ids: not_sane,
            score_components: score,
            source_digest: source,
        });
    }

    let (inactive, inactive_truncated) = bounded_ids(capsule.citizens.iter().filter(|unit| !unit.active));
    if !inactive.is_empty() || inactive_truncated {
        let mut score = BTreeMap::new();
        score.insert("affected_units".to_owned(), bounded_i64(inactive.len()));
        score.insert("ids_truncated".to_owned(), i64::from(inactive_truncated));
        items.push(LiveAttentionItem {
            attention_id: "live.basic_status.inactive".to_owned(),
            severity: LiveAttentionSeverity::Medium,
            category: "citizen_basic_status".to_owned(),
            finding: "one or more citizens are not marked active by DFHack".to_owned(),
            affected_unit_ids: inactive,
            score_components: score,
            source_digest: source,
        });
    }

    items.sort_by(|left, right| left.sort_key().cmp(&right.sort_key()));
    items.truncate(MAX_BRIEFING_ATTENTION_ITEMS);
    items
}

pub fn build_live_briefing(capsule: &LiveObservationCapsule) -> Result<LiveFortressBriefing> {
    capsule.validate()?;
    let citizen_status = CitizenStatusCounts::from_citizens(&capsule.citizens)?;
    if citizen_status.total != capsule.citizen_coverage.total {
        return Err(error(
            ErrorCode::InternalInvariantViolation,
            "briefing citizen count does not match complete capsule coverage",
        ));
    }
    Ok(LiveFortressBriefing {
        source_digest: capsule.content_digest,
        bridge_generation: capsule.bridge.bridge_generation,
        dwarf_fortress_version: capsule.bridge.df_version.clone(),
        dfhack_version: capsule.bridge.dfhack_version.clone(),
        bridge_version: capsule.bridge.bridge_version.clone(),
        site_id: capsule.site_id,
        world_name: capsule.world_name.clone(),
        world_folder: capsule.world_folder.clone(),
        paused: capsule.paused,
        current_year: capsule.current_year,
        current_year_tick: capsule.current_year_tick,
        citizen_status,
        coverage: coverage(),
        attention: attention(capsule),
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveChangeSummary {
    pub basis_digest: Digest32,
    pub target_digest: Digest32,
    pub heartbeat: bool,
    pub pause_changed: Option<(bool, bool)>,
    pub calendar_changed: bool,
    pub citizens_added: Vec<i32>,
    pub citizens_removed: Vec<i32>,
    pub citizens_changed: Vec<i32>,
    pub ids_truncated: bool,
}

fn citizen_digest(unit: &CitizenRecord) -> Result<Digest32> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"dfmcp-live-citizen-change-v1\0");
    bytes.extend_from_slice(&unit.unit_id.to_be_bytes());
    for text in [&unit.name, &unit.race] {
        let length = u32::try_from(text.len()).map_err(|_| {
            error(
                ErrorCode::BudgetExceeded,
                "citizen change text length does not fit u32",
            )
        })?;
        bytes.extend_from_slice(&length.to_be_bytes());
        bytes.extend_from_slice(text.as_bytes());
    }
    for value in [unit.profession, unit.x, unit.y, unit.z] {
        bytes.extend_from_slice(&value.to_be_bytes());
    }
    for value in [
        unit.alive,
        unit.sane,
        unit.active,
        unit.visible,
        unit.citizen,
        unit.resident,
        unit.baby,
        unit.child,
        unit.adult,
    ] {
        bytes.push(u8::from(value));
    }
    Ok(Digest32::of_bytes(&bytes))
}

pub fn summarize_live_change(
    basis: &LiveObservationCapsule,
    target: &LiveObservationCapsule,
) -> Result<LiveChangeSummary> {
    basis.validate()?;
    target.validate()?;
    if basis.site_id != target.site_id || basis.world_folder != target.world_folder {
        return Err(error(
            ErrorCode::RestoreRequired,
            "cannot summarize change across different fortress identities",
        ));
    }
    if basis.content_digest == target.content_digest {
        return Ok(LiveChangeSummary {
            basis_digest: basis.content_digest,
            target_digest: target.content_digest,
            heartbeat: true,
            pause_changed: None,
            calendar_changed: false,
            citizens_added: Vec::new(),
            citizens_removed: Vec::new(),
            citizens_changed: Vec::new(),
            ids_truncated: false,
        });
    }

    let mut basis_units = BTreeMap::new();
    for unit in &basis.citizens {
        basis_units.insert(unit.unit_id, citizen_digest(unit)?);
    }
    let mut target_units = BTreeMap::new();
    for unit in &target.citizens {
        target_units.insert(unit.unit_id, citizen_digest(unit)?);
    }

    let basis_ids: BTreeSet<i32> = basis_units.keys().copied().collect();
    let target_ids: BTreeSet<i32> = target_units.keys().copied().collect();
    let mut added: Vec<i32> = target_ids.difference(&basis_ids).copied().collect();
    let mut removed: Vec<i32> = basis_ids.difference(&target_ids).copied().collect();
    let mut changed: Vec<i32> = basis_ids
        .intersection(&target_ids)
        .copied()
        .filter(|id| basis_units.get(id) != target_units.get(id))
        .collect();

    let mut ids_truncated = false;
    for values in [&mut added, &mut removed, &mut changed] {
        if values.len() > MAX_BRIEFING_CHANGE_IDS {
            values.truncate(MAX_BRIEFING_CHANGE_IDS);
            ids_truncated = true;
        }
    }

    Ok(LiveChangeSummary {
        basis_digest: basis.content_digest,
        target_digest: target.content_digest,
        heartbeat: false,
        pause_changed: (basis.paused != target.paused).then_some((basis.paused, target.paused)),
        calendar_changed: basis.current_year != target.current_year
            || basis.current_year_tick != target.current_year_tick,
        citizens_added: added,
        citizens_removed: removed,
        citizens_changed: changed,
        ids_truncated,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::{BridgeManifest, ObservationAssembler, ObservationPage};

    fn citizen(id: i32, sane: bool, x: i32) -> CitizenRecord {
        CitizenRecord {
            unit_id: id,
            name: format!("Urist {id}"),
            race: "dwarf".to_owned(),
            profession: 4,
            x,
            y: 2,
            z: 3,
            alive: true,
            sane,
            active: true,
            visible: true,
            citizen: true,
            resident: false,
            baby: false,
            child: false,
            adult: true,
        }
    }

    fn capsule(discriminator: u8, citizens: Vec<CitizenRecord>) -> Result<LiveObservationCapsule> {
        let bridge = BridgeManifest {
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
        };
        let total = u32::try_from(citizens.len()).map_err(|_| {
            error(ErrorCode::BudgetExceeded, "test citizen count does not fit u32")
        })?;
        let mut assembler = ObservationAssembler::new(bridge);
        assembler.push_page(ObservationPage {
            bridge_generation: 42,
            world_loaded: true,
            fortress_mode: true,
            paused: true,
            current_year: 105,
            current_year_tick: 12_345u32.saturating_add(u32::from(discriminator)),
            world_name: "The Balanced Realm".to_owned(),
            world_folder: "region1".to_owned(),
            site_id: 7,
            citizen_count_total: total,
            citizen_offset: 0,
            complete: true,
            citizens,
        })?;
        assembler.finalize()
    }

    #[test]
    fn briefing_is_explicit_about_complete_and_omitted_domains() -> Result<()> {
        let briefing = build_live_briefing(&capsule(1, vec![citizen(1, true, 10)])?)?;
        assert!(briefing.can_prove_absence_in(LiveCoverageDomain::CitizenRoster));
        assert!(!briefing.can_prove_absence_in(LiveCoverageDomain::Threats));
        assert_eq!(
            briefing.coverage_status(LiveCoverageDomain::Threats),
            Some(LiveCoverageStatus::Omitted)
        );
        Ok(())
    }

    #[test]
    fn attention_is_mechanical_and_evidence_linked() -> Result<()> {
        let source = capsule(1, vec![citizen(1, false, 10)])?;
        let briefing = build_live_briefing(&source)?;
        assert_eq!(briefing.attention.len(), 1);
        assert_eq!(briefing.attention[0].attention_id, "live.basic_status.not_sane");
        assert_eq!(briefing.attention[0].source_digest, source.content_digest);
        Ok(())
    }

    #[test]
    fn change_summary_is_stable_and_id_ordered() -> Result<()> {
        let basis = capsule(1, vec![citizen(1, true, 10), citizen(3, true, 10)])?;
        let target = capsule(2, vec![citizen(1, true, 11), citizen(2, true, 10)])?;
        let change = summarize_live_change(&basis, &target)?;
        assert!(!change.heartbeat);
        assert_eq!(change.citizens_added, vec![2]);
        assert_eq!(change.citizens_removed, vec![3]);
        assert_eq!(change.citizens_changed, vec![1]);
        Ok(())
    }

    #[test]
    fn identical_capsules_summarize_as_heartbeat() -> Result<()> {
        let source = capsule(1, vec![citizen(1, true, 10)])?;
        let change = summarize_live_change(&source, &source)?;
        assert!(change.heartbeat);
        assert!(change.citizens_added.is_empty());
        assert!(change.citizens_removed.is_empty());
        assert!(change.citizens_changed.is_empty());
        Ok(())
    }
}
