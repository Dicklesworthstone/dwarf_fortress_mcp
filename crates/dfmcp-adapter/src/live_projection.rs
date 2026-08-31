#![forbid(unsafe_code)]

//! Deterministic normalization of one complete live DFHack capsule into the
//! canonical world model.
//!
//! The bridge capsule is transport-independent evidence, but it is not itself
//! the semantic world. This module performs the first admitted normalization:
//! one fortress summary entity and one unit entity per completely covered
//! citizen record. Every fact cites the source capsule digest. Domains absent
//! from bridge V1 are declared omitted rather than silently represented by an
//! empty collection.

use std::collections::{BTreeMap, BTreeSet};

use dfmcp_core::{
    CoverageDomain, CoverageReport, CoverageStatus, DfmcpError, Digest32, EntityId, ErrorCode,
    FortressId, GameTick, MapCoord, ObservationCursor, Result, StateAnchor,
};
use dfmcp_world::{
    EntityKind, EntityRecord, Fact, FactSource, Value, WorldGraph, WorldSnapshot,
};

use crate::{CitizenRecord, LiveObservationCapsule};

pub const LIVE_PROJECTION_SCHEMA: &str = "dfmcp.live_world_projection/1";
pub const TICKS_PER_DAY: u64 = 1_200;
pub const DAYS_PER_MONTH: u64 = 28;
pub const MONTHS_PER_YEAR: u64 = 12;
pub const TICKS_PER_YEAR: u64 = TICKS_PER_DAY * DAYS_PER_MONTH * MONTHS_PER_YEAR;
pub const FORTRESS_ENTITY_ID: EntityId = EntityId::new(u64::MAX);

const COMPLETE_DOMAINS: [&str; 3] = [
    "fortress.summary",
    "fortress.citizens.roster",
    "fortress.citizens.identity_position_status",
];
const OMITTED_DOMAINS: [(&str, &str); 7] = [
    ("fortress.items", "bridge protocol V1 does not observe items"),
    ("fortress.jobs", "bridge protocol V1 does not observe jobs"),
    ("fortress.map", "bridge protocol V1 does not observe map state"),
    ("fortress.economy", "bridge protocol V1 does not observe economy state"),
    ("fortress.welfare", "bridge protocol V1 does not observe detailed welfare state"),
    ("fortress.military", "bridge protocol V1 does not observe military state"),
    ("fortress.history", "bridge protocol V1 does not observe historical state"),
];

fn error(code: ErrorCode, message: impl Into<String>) -> DfmcpError {
    DfmcpError::new(code, message)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DwarfFortressClock {
    pub year: u32,
    pub year_tick: u32,
}

impl DwarfFortressClock {
    pub fn absolute_tick(self) -> Result<GameTick> {
        if u64::from(self.year_tick) >= TICKS_PER_YEAR {
            return Err(error(
                ErrorCode::AdapterRejected,
                format!(
                    "Dwarf Fortress year_tick {} is outside 0..{}",
                    self.year_tick,
                    TICKS_PER_YEAR - 1
                ),
            ));
        }
        let year_base = u64::from(self.year)
            .checked_mul(TICKS_PER_YEAR)
            .ok_or_else(|| {
                error(
                    ErrorCode::BudgetExceeded,
                    "Dwarf Fortress absolute game tick overflowed u64",
                )
            })?;
        let absolute = year_base
            .checked_add(u64::from(self.year_tick))
            .ok_or_else(|| {
                error(
                    ErrorCode::BudgetExceeded,
                    "Dwarf Fortress absolute game tick overflowed u64",
                )
            })?;
        Ok(GameTick::new(absolute))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveProjectionReceipt {
    schema: &'static str,
    source_capsule_digest: Digest32,
    source_bridge_generation: u64,
    snapshot_anchor: StateAnchor,
    coverage: CoverageReport,
}

impl LiveProjectionReceipt {
    #[must_use]
    pub const fn schema(&self) -> &'static str {
        self.schema
    }

    #[must_use]
    pub const fn source_capsule_digest(&self) -> Digest32 {
        self.source_capsule_digest
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
pub struct LiveWorldProjection {
    pub snapshot: WorldSnapshot,
    pub receipt: LiveProjectionReceipt,
}

impl LiveWorldProjection {
    pub fn validate_against(&self, capsule: &LiveObservationCapsule) -> Result<()> {
        capsule.validate()?;
        if !self.snapshot.hash_is_valid() {
            return Err(error(
                ErrorCode::CorruptLedger,
                "live world projection snapshot hash is invalid",
            ));
        }
        if self.receipt.schema != LIVE_PROJECTION_SCHEMA
            || self.receipt.source_capsule_digest != capsule.content_digest
            || self.receipt.source_bridge_generation != capsule.bridge.bridge_generation
            || self.receipt.snapshot_anchor != self.snapshot.anchor()
        {
            return Err(error(
                ErrorCode::CorruptLedger,
                "live projection receipt does not bind its source capsule and snapshot",
            ));
        }
        self.receipt.coverage.validate()?;
        if self.receipt.coverage.anchor != Some(self.snapshot.anchor()) {
            return Err(error(
                ErrorCode::CorruptLedger,
                "live projection coverage is not bound to the snapshot anchor",
            ));
        }
        for domain in COMPLETE_DOMAINS {
            if !self.receipt.coverage.proves_absence_in(domain) {
                return Err(error(
                    ErrorCode::CorruptLedger,
                    format!("live projection lost complete-domain witness for {domain}"),
                ));
            }
        }
        for entity in self.snapshot.graph.entities.values() {
            for fact in entity.fields.values() {
                if fact.source_digest != capsule.content_digest {
                    return Err(error(
                        ErrorCode::CorruptLedger,
                        "a live projected fact does not cite the source capsule digest",
                    ));
                }
            }
        }
        Ok(())
    }
}

#[must_use]
pub fn raw_unit_id_to_entity_id(raw_unit_id: i32) -> Result<EntityId> {
    if raw_unit_id < 0 {
        return Err(error(
            ErrorCode::AdapterRejected,
            "raw Dwarf Fortress unit ID must not be negative",
        ));
    }
    let raw = u64::from(raw_unit_id as u32);
    let encoded = raw.checked_add(1).ok_or_else(|| {
        error(
            ErrorCode::InternalInvariantViolation,
            "unit entity-ID encoding overflowed",
        )
    })?;
    if encoded == FORTRESS_ENTITY_ID.get() {
        return Err(error(
            ErrorCode::InternalInvariantViolation,
            "unit entity-ID encoding collided with the fortress entity",
        ));
    }
    Ok(EntityId::new(encoded))
}

#[must_use]
pub fn entity_id_to_raw_unit_id(entity_id: EntityId) -> Option<i32> {
    let encoded = entity_id.get();
    if encoded == 0 || encoded == FORTRESS_ENTITY_ID.get() {
        return None;
    }
    let raw = encoded.checked_sub(1)?;
    i32::try_from(raw).ok()
}

pub fn project_live_capsule(
    capsule: &LiveObservationCapsule,
    fortress_id: FortressId,
    cursor: ObservationCursor,
) -> Result<LiveWorldProjection> {
    capsule.validate()?;
    if fortress_id == FortressId::NIL {
        return Err(error(
            ErrorCode::InvalidRequest,
            "fortress identity zero is reserved",
        ));
    }
    if capsule.site_id < 0 {
        return Err(error(
            ErrorCode::FortressNotLoaded,
            "live fortress observation does not contain a valid site ID",
        ));
    }
    let observed_at = DwarfFortressClock {
        year: capsule.current_year,
        year_tick: capsule.current_year_tick,
    }
    .absolute_tick()?;
    let generation = u32::try_from(cursor.epoch)
        .ok()
        .and_then(|epoch| epoch.checked_add(1))
        .ok_or_else(|| {
            error(
                ErrorCode::BudgetExceeded,
                "observation epoch cannot be represented as an entity generation",
            )
        })?;

    let mut entities = BTreeMap::new();
    let fortress = fortress_entity(capsule, observed_at, generation, cursor.sequence);
    entities.insert(fortress.id, fortress);
    for citizen in &capsule.citizens {
        let entity = citizen_entity(
            citizen,
            capsule.content_digest,
            observed_at,
            generation,
            cursor.sequence,
        )?;
        if entities.insert(entity.id, entity).is_some() {
            return Err(error(
                ErrorCode::AdapterRejected,
                "live citizen projection produced a duplicate canonical entity ID",
            ));
        }
    }

    let snapshot = WorldSnapshot::new(
        fortress_id,
        observed_at,
        cursor,
        capsule.paused,
        WorldGraph {
            entities,
            ..WorldGraph::default()
        },
    );
    let coverage = projection_coverage(snapshot.anchor());
    coverage.validate()?;
    let receipt = LiveProjectionReceipt {
        schema: LIVE_PROJECTION_SCHEMA,
        source_capsule_digest: capsule.content_digest,
        source_bridge_generation: capsule.bridge.bridge_generation,
        snapshot_anchor: snapshot.anchor(),
        coverage,
    };
    let projection = LiveWorldProjection { snapshot, receipt };
    projection.validate_against(capsule)?;
    Ok(projection)
}

fn fortress_entity(
    capsule: &LiveObservationCapsule,
    observed_at: GameTick,
    generation: u32,
    revision: u64,
) -> EntityRecord {
    let digest = capsule.content_digest;
    let mut fields = BTreeMap::new();
    insert_fact(
        &mut fields,
        "world_name",
        Value::Text(capsule.world_name.clone()),
        observed_at,
        digest,
        "summary.world_name",
    );
    insert_fact(
        &mut fields,
        "world_folder",
        Value::Text(capsule.world_folder.clone()),
        observed_at,
        digest,
        "summary.world_folder",
    );
    insert_fact(
        &mut fields,
        "site_id",
        Value::I64(i64::from(capsule.site_id)),
        observed_at,
        digest,
        "summary.site_id",
    );
    insert_fact(
        &mut fields,
        "calendar_year",
        Value::U64(u64::from(capsule.current_year)),
        observed_at,
        digest,
        "summary.current_year",
    );
    insert_fact(
        &mut fields,
        "year_tick",
        Value::U64(u64::from(capsule.current_year_tick)),
        observed_at,
        digest,
        "summary.current_year_tick",
    );
    insert_fact(
        &mut fields,
        "citizen_count_total",
        Value::U64(u64::from(capsule.citizen_coverage.total)),
        observed_at,
        digest,
        "summary.citizen_count_total",
    );
    insert_fact(
        &mut fields,
        "citizen_roster_complete",
        Value::Bool(capsule.citizen_coverage.proves_complete_roster()),
        observed_at,
        digest,
        "coverage.citizens",
    );
    insert_fact(
        &mut fields,
        "bridge_generation",
        Value::U64(capsule.bridge.bridge_generation),
        observed_at,
        digest,
        "manifest.bridge_generation",
    );
    for (field, value, source) in [
        (
            "bridge_version",
            capsule.bridge.bridge_version.clone(),
            "manifest.bridge_version",
        ),
        (
            "dfhack_version",
            capsule.bridge.dfhack_version.clone(),
            "manifest.dfhack_version",
        ),
        (
            "dwarf_fortress_version",
            capsule.bridge.df_version.clone(),
            "manifest.df_version",
        ),
    ] {
        insert_fact(
            &mut fields,
            field,
            Value::Text(value),
            observed_at,
            digest,
            source,
        );
    }
    let label = if capsule.world_name.is_empty() {
        format!("fortress-site-{}", capsule.site_id)
    } else {
        capsule.world_name.clone()
    };
    EntityRecord {
        id: FORTRESS_ENTITY_ID,
        generation,
        revision,
        kind: EntityKind::Fortress,
        label,
        fields,
    }
}

fn citizen_entity(
    citizen: &CitizenRecord,
    source_digest: Digest32,
    observed_at: GameTick,
    generation: u32,
    revision: u64,
) -> Result<EntityRecord> {
    let id = raw_unit_id_to_entity_id(citizen.unit_id)?;
    let mut fields = BTreeMap::new();
    insert_fact(
        &mut fields,
        "raw_unit_id",
        Value::I64(i64::from(citizen.unit_id)),
        observed_at,
        source_digest,
        "citizen.unit_id",
    );
    insert_fact(
        &mut fields,
        "name",
        Value::Text(citizen.name.clone()),
        observed_at,
        source_digest,
        "citizen.name",
    );
    insert_fact(
        &mut fields,
        "race",
        Value::Text(citizen.race.clone()),
        observed_at,
        source_digest,
        "citizen.race",
    );
    insert_fact(
        &mut fields,
        "profession",
        Value::I64(i64::from(citizen.profession)),
        observed_at,
        source_digest,
        "citizen.profession",
    );
    insert_fact(
        &mut fields,
        "position",
        Value::Coord(MapCoord::new(citizen.x, citizen.y, citizen.z)),
        observed_at,
        source_digest,
        "citizen.position",
    );
    for (field, value, source) in [
        ("alive", citizen.alive, "citizen.alive"),
        ("sane", citizen.sane, "citizen.sane"),
        ("active", citizen.active, "citizen.active"),
        ("visible", citizen.visible, "citizen.visible"),
        ("citizen", citizen.citizen, "citizen.citizen"),
        ("resident", citizen.resident, "citizen.resident"),
        ("baby", citizen.baby, "citizen.baby"),
        ("child", citizen.child, "citizen.child"),
        ("adult", citizen.adult, "citizen.adult"),
    ] {
        insert_fact(
            &mut fields,
            field,
            Value::Bool(value),
            observed_at,
            source_digest,
            source,
        );
    }
    let label = if citizen.name.is_empty() {
        format!("unit-{}", citizen.unit_id)
    } else {
        citizen.name.clone()
    };
    Ok(EntityRecord {
        id,
        generation,
        revision,
        kind: EntityKind::Unit,
        label,
        fields,
    })
}

fn insert_fact(
    fields: &mut BTreeMap<String, Fact>,
    field: &str,
    value: Value,
    observed_at: GameTick,
    source_digest: Digest32,
    source_field: &str,
) {
    fields.insert(
        field.to_owned(),
        Fact::known(
            value,
            observed_at,
            FactSource::DfhackField(format!(
                "dfmcp_bridge.ReadObservation.{source_field}"
            )),
            source_digest,
        ),
    );
}

fn projection_coverage(anchor: StateAnchor) -> CoverageReport {
    let mut domains = BTreeMap::new();
    for domain in COMPLETE_DOMAINS {
        domains.insert(
            domain.to_owned(),
            CoverageDomain {
                domain: domain.to_owned(),
                status: CoverageStatus::Complete,
                reason: None,
                evidence: BTreeSet::new(),
            },
        );
    }
    for (domain, reason) in OMITTED_DOMAINS {
        domains.insert(
            domain.to_owned(),
            CoverageDomain {
                domain: domain.to_owned(),
                status: CoverageStatus::Omitted,
                reason: Some(reason.to_owned()),
                evidence: BTreeSet::new(),
            },
        );
    }
    CoverageReport {
        anchor: Some(anchor),
        domains,
        continuation: None,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::{BridgeManifest, ObservationAssembler, ObservationPage};

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

    fn capsule(page_sizes: &[&[i32]]) -> Result<LiveObservationCapsule> {
        let total = page_sizes.iter().try_fold(0u32, |total, page| {
            let len = u32::try_from(page.len()).map_err(|_| {
                error(ErrorCode::BudgetExceeded, "test page length does not fit u32")
            })?;
            total.checked_add(len).ok_or_else(|| {
                error(ErrorCode::BudgetExceeded, "test citizen count overflowed")
            })
        })?;
        let mut assembler = ObservationAssembler::new(manifest());
        let mut offset = 0u32;
        for (index, ids) in page_sizes.iter().enumerate() {
            let length = u32::try_from(ids.len()).map_err(|_| {
                error(ErrorCode::BudgetExceeded, "test page length does not fit u32")
            })?;
            let complete = index + 1 == page_sizes.len();
            assembler.push_page(page(offset, total, ids, complete))?;
            offset = offset.checked_add(length).ok_or_else(|| {
                error(ErrorCode::BudgetExceeded, "test page offset overflowed")
            })?;
        }
        assembler.finalize()
    }

    #[test]
    fn calendar_conversion_is_exact_and_bounded() -> Result<()> {
        let clock = DwarfFortressClock {
            year: 105,
            year_tick: 12_345,
        };
        assert_eq!(
            clock.absolute_tick()?.get(),
            105 * TICKS_PER_YEAR + 12_345
        );
        assert!(
            DwarfFortressClock {
                year: 1,
                year_tick: u32::try_from(TICKS_PER_YEAR).map_err(|_| {
                    error(ErrorCode::InternalInvariantViolation, "tick bound does not fit u32")
                })?,
            }
            .absolute_tick()
            .is_err()
        );
        Ok(())
    }

    #[test]
    fn unit_identity_encoding_is_reversible_and_avoids_nil() -> Result<()> {
        for raw in [0, 1, i32::MAX] {
            let entity = raw_unit_id_to_entity_id(raw)?;
            assert_ne!(entity, EntityId::NIL);
            assert_ne!(entity, FORTRESS_ENTITY_ID);
            assert_eq!(entity_id_to_raw_unit_id(entity), Some(raw));
        }
        assert!(raw_unit_id_to_entity_id(-1).is_err());
        Ok(())
    }

    #[test]
    fn projection_is_identical_across_transport_pagination() -> Result<()> {
        let one = capsule(&[&[0, 1, 2, 3]])?;
        let many = capsule(&[&[0, 1], &[2], &[3]])?;
        assert_eq!(one.content_digest, many.content_digest);
        let cursor = ObservationCursor {
            epoch: 7,
            sequence: 11,
        };
        let one_projection = project_live_capsule(&one, FortressId::new(9), cursor)?;
        let many_projection = project_live_capsule(&many, FortressId::new(9), cursor)?;
        assert_eq!(one_projection, many_projection);
        assert!(one_projection.snapshot.hash_is_valid());
        assert_eq!(one_projection.snapshot.graph.entities.len(), 5);
        one_projection.validate_against(&one)?;
        Ok(())
    }

    #[test]
    fn every_projected_fact_cites_the_capsule_digest() -> Result<()> {
        let capsule = capsule(&[&[0, 1]])?;
        let projection = project_live_capsule(
            &capsule,
            FortressId::new(9),
            ObservationCursor::ORIGIN,
        )?;
        for entity in projection.snapshot.graph.entities.values() {
            for fact in entity.fields.values() {
                assert_eq!(fact.source_digest, capsule.content_digest);
            }
        }
        assert!(
            projection
                .receipt
                .coverage()
                .proves_absence_in("fortress.citizens.roster")
        );
        assert!(
            !projection
                .receipt
                .coverage()
                .proves_absence_in("fortress.items")
        );
        Ok(())
    }

    #[test]
    fn projection_rejects_invalid_identity_clock_and_epoch() -> Result<()> {
        let mut capsule = capsule(&[&[1]])?;
        assert!(
            project_live_capsule(
                &capsule,
                FortressId::NIL,
                ObservationCursor::ORIGIN
            )
            .is_err()
        );
        capsule.site_id = -1;
        assert!(
            project_live_capsule(
                &capsule,
                FortressId::new(9),
                ObservationCursor::ORIGIN
            )
            .is_err()
        );
        let valid = capsule(&[&[1]])?;
        assert!(
            project_live_capsule(
                &valid,
                FortressId::new(9),
                ObservationCursor {
                    epoch: u64::MAX,
                    sequence: 0,
                },
            )
            .is_err()
        );
        Ok(())
    }
}
