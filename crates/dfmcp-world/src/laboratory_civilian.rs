//! Complete-domain civilian and threat census for the reference laboratory.
//!
//! The caller must own the complete reference world. Source strings and a valid
//! snapshot hash cannot promote an imported or native snapshot into that model.
//! Only qualified `lab-scenario/1` and `reference-effects/1` inputs are consumed.
//! This establishes current configuration and threat facts, never a historical
//! clear window, physical arrival, or native evacuation/pathfinding evidence.

use std::collections::BTreeSet;

use dfmcp_core::{
    DfmcpError, Digest32, EntityId, ErrorCode, FortressId, GameTick, Result, StateAnchor,
};

use crate::{EntityRecord, PredicateTruth, Value, WorldSnapshot, laboratory_fact_value};

pub const LABORATORY_CIVILIAN_RELEASE_POLICY: &str = "dfmcp.laboratory-civilian-release/1";
pub const MAX_LABORATORY_CIVILIANS: usize = 64;
pub const MAX_LABORATORY_CIVILIAN_ENTITIES: usize = 65_536;
pub const MAX_PREDICATE_LABORATORY_CIVILIAN_WORK: u64 =
    16 * 32 * MAX_LABORATORY_CIVILIAN_ENTITIES as u64;

/// An exact entity incarnation. Reusing an ID cannot inherit release approval.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LaboratoryEntityIdentity {
    pub id: EntityId,
    pub generation: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LaboratoryThreatAssessment {
    Present,
    Clear,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaboratoryCivilianCensus {
    pub anchor: StateAnchor,
    pub burrow: Option<LaboratoryEntityIdentity>,
    /// Canonical IDs, capped at 64. Overflow makes the roster incomplete.
    pub civilians: Vec<LaboratoryEntityIdentity>,
    pub complete_civilian_roster: bool,
    pub threat: LaboratoryThreatAssessment,
    /// True for any known retained protection, including a partial assignment
    /// or an ACTIVE restriction. It does not assert that everyone is protected.
    pub protected: PredicateTruth,
    /// True only for the complete current civilian roster, all qualified true
    /// memberships and an ACTIVE restriction. This is configuration, not arrival.
    pub protection_complete: PredicateTruth,
    /// True only for a complete roster, explicit false memberships and INACTIVE.
    pub released: PredicateTruth,
    /// Stable across ticks and membership/order edits; changes with source,
    /// roster, role, threat-domain or entity-incarnation changes. Not authority.
    pub source_scope: Digest32,
    pub inspected_entities: usize,
}

fn invalid(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::InvalidRequest, message)
}

pub(crate) fn work_bound(entities: usize) -> u64 {
    (entities as u64).saturating_mul(32)
}

pub(crate) fn validate_request(
    burrow: LaboratoryEntityIdentity,
    civilians: &[LaboratoryEntityIdentity],
) -> Result<()> {
    if civilians.is_empty() || civilians.len() > MAX_LABORATORY_CIVILIANS {
        return Err(DfmcpError::new(
            ErrorCode::BudgetExceeded,
            "laboratory civilian release requires 1..=64 civilians",
        ));
    }
    let mut ids = BTreeSet::new();
    if burrow.id == EntityId::NIL || burrow.generation == 0 {
        return Err(invalid(
            "civilian release requires an exact nonzero burrow incarnation",
        ));
    }
    for civilian in civilians {
        if civilian.id == EntityId::NIL
            || civilian.id == burrow.id
            || civilian.generation == 0
            || !ids.insert(civilian.id)
        {
            return Err(invalid(
                "civilian release requires distinct nonzero civilian identities and generations",
            ));
        }
    }
    Ok(())
}

/// The existing reference standing-order identity. Keeping this implementation
/// in the world model lets observations and effects share its exact legacy bytes.
#[must_use]
pub fn laboratory_fortress_settings_entity_id(fortress: FortressId) -> EntityId {
    let mut bytes = b"dfmcp-fortress-settings-v1\0".to_vec();
    bytes.extend_from_slice(&fortress.get().to_be_bytes());
    let digest = Digest32::of_bytes(&bytes);
    let mut low = [0u8; 8];
    low.copy_from_slice(&digest.as_bytes()[..8]);
    EntityId::new(0x7E00_0000_0000_0000 | (u64::from_be_bytes(low) & 0x00FF_FFFF_FFFF_FFFF))
}

fn identity(record: &EntityRecord) -> Option<LaboratoryEntityIdentity> {
    (record.id != EntityId::NIL && record.generation != 0).then_some(LaboratoryEntityIdentity {
        id: record.id,
        generation: record.generation,
    })
}

fn field<'a>(record: &'a EntityRecord, name: &str, tick: GameTick) -> Option<&'a Value> {
    record
        .fields
        .get(name)
        .and_then(|fact| laboratory_fact_value(fact, tick))
}

fn scope_identity(scope: &mut Vec<u8>, record: &EntityRecord) {
    scope.extend_from_slice(&record.id.get().to_be_bytes());
    scope.extend_from_slice(&record.generation.to_be_bytes());
}

fn scope_source(scope: &mut Vec<u8>, record: &EntityRecord, name: &str, tick: GameTick) {
    scope.push(match record.fields.get(name) {
        None => 0,
        // Both registered derivations are the same reference observation
        // authority. Ordinary source refreshes must not erase a clear window.
        Some(fact) if laboratory_fact_value(fact, tick).is_some() => 1,
        Some(_) => 2,
    });
}

/// Classify one selected burrow against the entire bounded reference entity
/// domain. Missing `squad` is the legacy laboratory civilian default; explicit
/// qualified Null has the same role. A present but unavailable squad field is
/// unknown. Military roles require a current qualified squad identity.
pub fn laboratory_civilian_census(
    snapshot: &WorldSnapshot,
    selected_burrow: EntityId,
) -> Result<LaboratoryCivilianCensus> {
    if selected_burrow == EntityId::NIL || snapshot.fortress_id == FortressId::NIL {
        return Err(invalid(
            "civilian census requires a nonzero fortress and selected burrow",
        ));
    }
    if snapshot.graph.entities.len() > MAX_LABORATORY_CIVILIAN_ENTITIES {
        return Err(DfmcpError::new(
            ErrorCode::BudgetExceeded,
            "laboratory civilian census exceeds its complete entity-domain bound",
        ));
    }
    let burrow = snapshot
        .graph
        .entities
        .get(&selected_burrow)
        .filter(|record| record.kind.as_str() == "burrow")
        .and_then(identity);
    let mut scope = LABORATORY_CIVILIAN_RELEASE_POLICY.as_bytes().to_vec();
    scope.push(0);
    scope.extend_from_slice(&snapshot.fortress_id.get().to_be_bytes());
    scope.extend_from_slice(&selected_burrow.get().to_be_bytes());
    scope.extend_from_slice(&burrow.map_or(0, |value| value.generation).to_be_bytes());
    let mut civilians = Vec::new();
    let mut complete = true;
    let mut any_assignment = false;
    let mut all_unassigned = true;
    let mut all_assigned = true;
    let mut any_unassigned = false;
    let mut present = false;
    let mut unknown_threat = false;
    let membership = format!("burrow.{}", selected_burrow.get());
    for (key, record) in &snapshot.graph.entities {
        if *key != record.id {
            return Err(invalid(
                "civilian census entity lookup keys must match anchored identities",
            ));
        }
        // Canonical kind names include aliases: byte-identical snapshots must
        // classify identically and a reserved alias must not hide a threat.
        match record.kind.as_str() {
            "unit" => {
                scope.push(0);
                scope_identity(&mut scope, record);
                scope_source(&mut scope, record, "alive", snapshot.tick);
                let alive = field(record, "alive", snapshot.tick);
                scope.push(match alive {
                    Some(Value::Bool(false)) => 0,
                    Some(Value::Bool(true)) => 1,
                    _ => 2,
                });
                let Some(unit) = identity(record) else {
                    complete = false;
                    continue;
                };
                match alive {
                    Some(Value::Bool(false)) => continue,
                    Some(Value::Bool(true)) => {}
                    _ => {
                        complete = false;
                        continue;
                    }
                }
                scope_source(&mut scope, record, "squad", snapshot.tick);
                let civilian = match (
                    record.fields.get("squad"),
                    field(record, "squad", snapshot.tick),
                ) {
                    (None, _) | (_, Some(Value::Null)) => {
                        scope.push(0);
                        true
                    }
                    (_, Some(Value::Entity(squad_id))) => {
                        scope.push(1);
                        scope.extend_from_slice(&squad_id.get().to_be_bytes());
                        let squad = snapshot
                            .graph
                            .entities
                            .get(squad_id)
                            .filter(|record| record.kind.as_str() == "squad")
                            .and_then(identity);
                        scope.extend_from_slice(
                            &squad.map_or(0, |value| value.generation).to_be_bytes(),
                        );
                        if squad.is_none() {
                            complete = false;
                        }
                        false
                    }
                    _ => {
                        scope.push(2);
                        complete = false;
                        false
                    }
                };
                if civilian {
                    if civilians.len() < MAX_LABORATORY_CIVILIANS {
                        civilians.push(unit);
                    } else {
                        complete = false;
                    }
                    match field(record, &membership, snapshot.tick) {
                        Some(Value::Bool(true)) => {
                            any_assignment = true;
                            all_unassigned = false;
                        }
                        Some(Value::Bool(false)) => {
                            all_assigned = false;
                            any_unassigned = true;
                        }
                        _ => {
                            all_assigned = false;
                            all_unassigned = false;
                        }
                    }
                }
            }
            "creature" => {
                scope.push(1);
                scope_identity(&mut scope, record);
                scope_source(&mut scope, record, "hostile", snapshot.tick);
                if identity(record).is_none() {
                    unknown_threat = true;
                }
                match field(record, "hostile", snapshot.tick) {
                    Some(Value::Bool(false)) => scope.push(0),
                    Some(Value::Bool(true)) => {
                        scope.push(1);
                        scope_source(&mut scope, record, "threat_status", snapshot.tick);
                        match field(record, "threat_status", snapshot.tick) {
                            Some(Value::Text(status)) if status == "approaching" => {
                                scope.push(0);
                                present = true;
                            }
                            Some(Value::Text(status)) if status == "attacking" => {
                                scope.push(1);
                                present = true;
                            }
                            Some(Value::Text(status)) if status == "slain" => scope.push(2),
                            _ => {
                                scope.push(3);
                                unknown_threat = true;
                            }
                        }
                    }
                    _ => {
                        scope.push(2);
                        unknown_threat = true;
                    }
                }
            }
            _ => {}
        }
    }
    let restriction = snapshot
        .graph
        .entities
        .get(&laboratory_fortress_settings_entity_id(
            snapshot.fortress_id,
        ))
        .filter(|record| record.kind.as_str() == "fortress_settings" && identity(record).is_some())
        .and_then(|record| {
            field(
                record,
                "standing_order.CIVILIAN_BURROW_RESTRICTION",
                snapshot.tick,
            )
        });
    let active = matches!(restriction, Some(Value::Text(value)) if value == "ACTIVE");
    let inactive = matches!(restriction, Some(Value::Text(value)) if value == "INACTIVE");
    let retained = active || any_assignment;
    let released = burrow.is_some() && complete && all_unassigned && inactive;
    Ok(LaboratoryCivilianCensus {
        anchor: snapshot.anchor(),
        burrow,
        civilians,
        complete_civilian_roster: complete,
        threat: if present {
            LaboratoryThreatAssessment::Present
        } else if unknown_threat {
            LaboratoryThreatAssessment::Unknown
        } else {
            LaboratoryThreatAssessment::Clear
        },
        protected: if retained {
            PredicateTruth::True
        } else if released {
            PredicateTruth::False
        } else {
            PredicateTruth::Unknown
        },
        protection_complete: if inactive || any_unassigned {
            PredicateTruth::False
        } else if burrow.is_some() && complete && all_assigned && active {
            PredicateTruth::True
        } else {
            PredicateTruth::Unknown
        },
        released: if retained {
            PredicateTruth::False
        } else if released {
            PredicateTruth::True
        } else {
            PredicateTruth::Unknown
        },
        source_scope: Digest32::of_bytes(&scope),
        inspected_entities: snapshot.graph.entities.len(),
    })
}

/// Current safety for one sealed release roster. Historical clearance is a
/// separate process-owned monitor decision and cannot be asserted by this
/// predicate. Each ordinary release effect must recheck this exact condition.
pub fn laboratory_civilian_release_safety_truth(
    snapshot: &WorldSnapshot,
    burrow: LaboratoryEntityIdentity,
    civilians: &[LaboratoryEntityIdentity],
) -> Result<PredicateTruth> {
    validate_request(burrow, civilians)?;
    let census = laboratory_civilian_census(snapshot, burrow.id)?;
    if census.threat == LaboratoryThreatAssessment::Present {
        return Ok(PredicateTruth::False);
    }
    if census.burrow != Some(burrow) {
        return Ok(match snapshot.graph.entities.get(&burrow.id) {
            Some(record) if record.generation == 0 => PredicateTruth::Unknown,
            _ => PredicateTruth::False,
        });
    }
    if !census.complete_civilian_roster || census.threat == LaboratoryThreatAssessment::Unknown {
        return Ok(PredicateTruth::Unknown);
    }
    let mut expected = civilians.to_vec();
    expected.sort_unstable();
    Ok(if census.civilians == expected {
        PredicateTruth::True
    } else {
        PredicateTruth::False
    })
}

#[cfg(test)]
#[path = "laboratory_civilian_tests.rs"]
mod tests;
