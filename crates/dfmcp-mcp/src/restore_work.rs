//! Observe physical reference work whose originating action handle is absent.
//!
//! Restoring or reopening a snapshot can retain an active entity after the
//! adapter's action handles have been invalidated. This scan describes that
//! current work and fences its region without inventing a goal proof, an
//! idempotency key, or authority to stop it. The canonical world supplies the
//! record again after restart; no synthetic durable commit is created.

use std::collections::BTreeSet;

use dfmcp_core::{DfmcpError, EntityId, ErrorCode, GameTick, MapCuboid, Result, StateAnchor};
use dfmcp_intent::{EffectWorkState, effects};
use dfmcp_world::{
    EntityKind, EntityRecord, PredicateEvidence, Value, WorldSnapshot, laboratory_fact_value,
};
use serde_json::json;

use super::{anchor_json, effect_work_json};

#[derive(Clone, Debug, PartialEq, Eq)]
enum WorkScope {
    NonSpatial,
    Spatial(MapCuboid),
    UnknownSpatial,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct UntrackedWork {
    pub(super) entity_id: EntityId,
    pub(super) generation: u32,
    pub(super) kind: &'static str,
    pub(super) state: EffectWorkState,
    pub(super) observed_anchor: StateAnchor,
    scope: WorkScope,
}

impl UntrackedWork {
    /// Missing or ineligible geometry cannot open a spatial reservation.
    pub(super) fn conflicts_with(&self, area: &MapCuboid) -> bool {
        match &self.scope {
            WorkScope::NonSpatial => false,
            WorkScope::Spatial(owned) => dfmcp_core::lease::cuboids_intersect(owned, area),
            WorkScope::UnknownSpatial => true,
        }
    }

    pub(super) fn to_json(&self) -> serde_json::Value {
        let (scope, area) = match &self.scope {
            WorkScope::NonSpatial => ("non_spatial", serde_json::Value::Null),
            WorkScope::UnknownSpatial => ("unknown_spatial", serde_json::Value::Null),
            WorkScope::Spatial(area) => (
                "spatial",
                json!({
                    "min": [area.min.x, area.min.y, area.min.z],
                    "max": [area.max.x, area.max.y, area.max.z],
                }),
            ),
        };
        let mut work = effect_work_json(&self.state);
        work["observed_anchor"] = anchor_json(&self.observed_anchor);
        json!({
            "entity_id": self.entity_id.to_string(), "generation": self.generation,
            "kind": self.kind, "origin": "untracked_snapshot_entity",
            "action_id": null, "goal_proof": "unavailable",
            "work_state": work, "scope": scope, "map_area": area,
            "observed_anchor": anchor_json(&self.observed_anchor),
            "observation_only": true,
            "note": "the originating action handle is unavailable; this observation grants no stop or dispatch authority",
        })
    }
}

#[derive(Clone, Copy)]
enum WorkKind {
    Dig,
    Construction,
    Production,
}

impl WorkKind {
    fn of(entity: &EntityRecord) -> Option<Self> {
        match &entity.kind {
            EntityKind::WorkOrder => Some(Self::Production),
            EntityKind::Other(kind) if kind == effects::DIG_DESIGNATION_KIND => Some(Self::Dig),
            // Without a retained originating action, omitted construction
            // metadata cannot prove that a building was never temporal work.
            EntityKind::Building => Some(Self::Construction),
            _ => None,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Dig => "dig_designation",
            Self::Construction => "construction",
            Self::Production => "work_order",
        }
    }

    fn lifecycle(self) -> &'static str {
        match self {
            Self::Construction => effects::CONSTRUCTION_STAGE_FIELD,
            Self::Dig | Self::Production => effects::STATUS_FIELD,
        }
    }

    fn active(self, value: &str) -> bool {
        match self {
            Self::Construction => {
                matches!(
                    value,
                    effects::STAGE_PLANNED | effects::STAGE_UNDER_CONSTRUCTION
                )
            }
            Self::Dig | Self::Production => value == effects::STATUS_ACTIVE,
        }
    }

    fn scope(self, entity: &EntityRecord, tick: GameTick) -> WorkScope {
        let (minimum, maximum) = match self {
            Self::Dig => ("area_min", "area_max"),
            Self::Construction => ("footprint_min", "footprint_max"),
            Self::Production => return WorkScope::NonSpatial,
        };
        match (field(entity, minimum, tick), field(entity, maximum, tick)) {
            (Some(Value::Coord(min)), Some(Value::Coord(max))) => MapCuboid::new(*min, *max)
                .map(WorkScope::Spatial)
                .unwrap_or(WorkScope::UnknownSpatial),
            _ => WorkScope::UnknownSpatial,
        }
    }
}

fn field<'a>(entity: &'a EntityRecord, name: &str, tick: GameTick) -> Option<&'a Value> {
    entity
        .fields
        .get(name)
        .and_then(|fact| laboratory_fact_value(fact, tick))
}

/// Return every active or unresolved reference-work entity not already covered
/// by retained dispatch or carried-step custody. Only eligible complete or
/// cancelled lifecycle facts establish that a current record owns no work.
///
/// The caller must derive `known_work_entities` from retained execution records,
/// not from a wish to omit inconvenient work. A failed or over-budget scan is
/// unresolved coverage and must never be replaced by an empty result.
pub(super) fn inspect_untracked_work(
    snapshot: &WorldSnapshot,
    known_work_entities: &BTreeSet<EntityId>,
    max_entities: u32,
) -> Result<Vec<UntrackedWork>> {
    if max_entities == 0 || snapshot.graph.entities.len() > max_entities as usize {
        return Err(DfmcpError::new(
            ErrorCode::BudgetExceeded,
            "untracked physical work requires complete entity coverage within the session bound",
        ));
    }
    let _observation = PredicateEvidence::laboratory(snapshot)?;
    let mut work = Vec::new();
    for (id, entity) in &snapshot.graph.entities {
        if known_work_entities.contains(id) {
            continue;
        }
        let Some(kind) = WorkKind::of(entity) else {
            continue;
        };
        let identity_valid = entity.id == *id && *id != EntityId::NIL && entity.generation > 0;
        let scope = if identity_valid {
            kind.scope(entity, snapshot.tick)
        } else {
            // An embedded identifier can address a different record during
            // progress. Invalid identity cannot justify any narrow footprint.
            WorkScope::UnknownSpatial
        };
        let lifecycle = field(entity, kind.lifecycle(), snapshot.tick);
        if identity_valid
            && matches!(lifecycle, Some(Value::Text(value))
                if value == effects::STATUS_COMPLETE || value == effects::STATUS_CANCELLED)
        {
            continue;
        }
        let state = if !identity_valid {
            EffectWorkState::Unknown {
                entity_id: Some(*id),
                reason: "untracked work has an invalid canonical entity identity".to_owned(),
            }
        } else if scope == WorkScope::UnknownSpatial {
            EffectWorkState::Unknown {
                entity_id: Some(*id),
                reason: "untracked spatial work has no eligible bounded footprint".to_owned(),
            }
        } else if matches!(lifecycle, Some(Value::Text(value)) if kind.active(value)) {
            EffectWorkState::Active { entity_id: *id }
        } else {
            EffectWorkState::Unknown {
                entity_id: Some(*id),
                reason: "untracked work has missing, ineligible or unregistered lifecycle evidence"
                    .to_owned(),
            }
        };
        work.push(UntrackedWork {
            entity_id: *id,
            generation: entity.generation,
            kind: kind.label(),
            state,
            observed_anchor: snapshot.anchor(),
            scope,
        });
    }
    Ok(work)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    use dfmcp_core::{Digest32, FortressId, MapCoord, ObservationCursor};
    use dfmcp_world::{Fact, FactPresence, FactSource, WorldGraph};

    fn fact(value: Value) -> Fact {
        Fact::known(
            value,
            GameTick(100),
            FactSource::Derived("dfmcp.reference-effects/1".to_owned()),
            Digest32::ZERO,
        )
    }

    fn record(id: u64, kind: EntityKind, fields: Vec<(&str, Value)>) -> EntityRecord {
        EntityRecord {
            id: EntityId::new(id),
            generation: 1,
            revision: 1,
            kind,
            label: "restored reference fixture".to_owned(),
            fields: fields
                .into_iter()
                .map(|(name, value)| (name.to_owned(), fact(value)))
                .collect(),
        }
    }

    fn snapshot() -> WorldSnapshot {
        let entities = [
            record(
                10,
                EntityKind::Other(effects::DIG_DESIGNATION_KIND.to_owned()),
                vec![
                    (
                        effects::STATUS_FIELD,
                        Value::Text(effects::STATUS_ACTIVE.to_owned()),
                    ),
                    ("area_min", Value::Coord(MapCoord::new(1, 1, 10))),
                    ("area_max", Value::Coord(MapCoord::new(2, 2, 10))),
                ],
            ),
            record(
                20,
                EntityKind::Building,
                vec![
                    (
                        effects::CONSTRUCTION_STAGE_FIELD,
                        Value::Text(effects::STAGE_PLANNED.to_owned()),
                    ),
                    ("progress_ticks", Value::U64(0)),
                    ("required_ticks", Value::U64(40)),
                    ("footprint_min", Value::Coord(MapCoord::new(4, 4, 10))),
                    ("footprint_max", Value::Coord(MapCoord::new(5, 5, 10))),
                ],
            ),
            record(
                30,
                EntityKind::WorkOrder,
                vec![(
                    effects::STATUS_FIELD,
                    Value::Text(effects::STATUS_ACTIVE.to_owned()),
                )],
            ),
            record(
                40,
                EntityKind::Building,
                vec![(
                    effects::CONSTRUCTION_STAGE_FIELD,
                    Value::Text(effects::STAGE_COMPLETE.to_owned()),
                )],
            ),
        ]
        .into_iter()
        .map(|entity| (entity.id, entity))
        .collect::<BTreeMap<_, _>>();
        WorldSnapshot::new(
            FortressId::new(9),
            GameTick(100),
            ObservationCursor::ORIGIN,
            false,
            WorldGraph {
                entities,
                ..WorldGraph::default()
            },
        )
    }

    fn required_record(snapshot: &mut WorldSnapshot, id: u64) -> Result<&mut EntityRecord> {
        snapshot
            .graph
            .entities
            .get_mut(&EntityId::new(id))
            .ok_or_else(|| {
                DfmcpError::new(
                    ErrorCode::InternalInvariantViolation,
                    "restored work fixture missing",
                )
            })
    }

    #[test]
    fn restored_work_is_visible_without_inventing_action_handles_and_fences_only_its_region()
    -> Result<()> {
        let snapshot = snapshot();
        let before = snapshot.clone();
        let work = inspect_untracked_work(&snapshot, &BTreeSet::new(), 4)?;
        assert_eq!(work.len(), 3);
        assert!(
            work.iter()
                .all(|item| matches!(item.state, EffectWorkState::Active { .. }))
        );
        let near_dig = MapCuboid::new(MapCoord::new(2, 2, 10), MapCoord::new(3, 3, 10))?;
        let far = MapCuboid::new(MapCoord::new(50, 50, 10), MapCoord::new(51, 51, 10))?;
        assert!(work[0].conflicts_with(&near_dig));
        assert!(!work[0].conflicts_with(&far));
        assert!(!work[2].conflicts_with(&near_dig));
        assert!(work[0].to_json()["action_id"].is_null());
        assert_eq!(work[0].to_json()["goal_proof"], "unavailable");
        assert_eq!(work[0].observed_anchor, snapshot.anchor());
        assert_eq!(snapshot, before);
        Ok(())
    }

    #[test]
    fn only_registered_quiet_lifecycle_and_retained_identity_can_remove_untracked_work()
    -> Result<()> {
        let mut snapshot = snapshot();
        let owned = BTreeSet::from([EntityId::new(10), EntityId::new(30)]);
        let work = inspect_untracked_work(&snapshot, &owned, 4)?;
        assert_eq!(work.len(), 1);
        assert_eq!(work[0].entity_id, EntityId::new(20));
        for (id, field, status) in [
            (10, effects::STATUS_FIELD, effects::STATUS_COMPLETE),
            (
                20,
                effects::CONSTRUCTION_STAGE_FIELD,
                effects::STATUS_CANCELLED,
            ),
            (30, effects::STATUS_FIELD, effects::STATUS_COMPLETE),
        ] {
            required_record(&mut snapshot, id)?
                .fields
                .insert(field.to_owned(), fact(Value::Text(status.to_owned())));
        }
        snapshot.refresh_hash();
        assert!(inspect_untracked_work(&snapshot, &BTreeSet::new(), 4)?.is_empty());
        Ok(())
    }

    #[test]
    fn missing_untrusted_future_or_unknown_lifecycle_never_proves_restored_quiescence() -> Result<()>
    {
        for mutation in 0..7 {
            let mut snapshot = snapshot();
            let entity = required_record(&mut snapshot, 30)?;
            if mutation == 0 {
                entity.fields.remove(effects::STATUS_FIELD);
            } else {
                let mut status = fact(Value::Text(effects::STATUS_COMPLETE.to_owned()));
                match mutation {
                    1 => status.source = FactSource::AgentAssertion("claimed complete".to_owned()),
                    2 => status.source = FactSource::Replay,
                    3 => status.observed_at = GameTick(101),
                    4 => status.source_digest = Digest32::of_bytes(b"unknown producer"),
                    5 => status.presence = Some(FactPresence::Unknown("not observed".to_owned())),
                    _ => status.value = Value::Text("not a registered state".to_owned()),
                }
                entity
                    .fields
                    .insert(effects::STATUS_FIELD.to_owned(), status);
            }
            snapshot.refresh_hash();
            let owned = BTreeSet::from([EntityId::new(10), EntityId::new(20)]);
            let work = inspect_untracked_work(&snapshot, &owned, 4)?;
            assert_eq!(work.len(), 1);
            assert!(matches!(work[0].state, EffectWorkState::Unknown { .. }));
            assert!(!work[0].state.is_quiescent());
        }
        Ok(())
    }

    #[test]
    fn missing_untrusted_or_invalid_geometry_fences_every_spatial_reservation() -> Result<()> {
        let far = MapCuboid::new(MapCoord::new(50, 50, 10), MapCoord::new(51, 51, 10))?;
        for mutation in 0..3 {
            let mut snapshot = snapshot();
            let entity = required_record(&mut snapshot, 10)?;
            if mutation == 0 {
                entity.fields.remove("area_min");
            } else {
                let mut minimum = fact(Value::Coord(MapCoord::new(1, 1, 10)));
                if mutation == 1 {
                    minimum.source = FactSource::AgentAssertion("claimed area".to_owned());
                } else {
                    minimum.value = Value::Coord(MapCoord::new(8, 8, 10));
                }
                entity.fields.insert("area_min".to_owned(), minimum);
            }
            snapshot.refresh_hash();
            let owned = BTreeSet::from([EntityId::new(20), EntityId::new(30)]);
            let work = inspect_untracked_work(&snapshot, &owned, 4)?;
            assert_eq!(work.len(), 1);
            assert!(work[0].conflicts_with(&far));
            assert!(matches!(work[0].state, EffectWorkState::Unknown { .. }));
            assert_eq!(work[0].to_json()["scope"], "unknown_spatial");
        }
        Ok(())
    }

    #[test]
    fn incomplete_entity_coverage_and_invalid_observations_are_refused() -> Result<()> {
        let mut snapshot = snapshot();
        let Err(error) = inspect_untracked_work(&snapshot, &BTreeSet::new(), 3) else {
            return Err(DfmcpError::new(
                ErrorCode::InternalInvariantViolation,
                "expected incomplete census refusal",
            ));
        };
        assert_eq!(error.code, ErrorCode::BudgetExceeded);
        snapshot.state_hash = Digest32::ZERO;
        assert!(inspect_untracked_work(&snapshot, &BTreeSet::new(), 4).is_err());
        Ok(())
    }

    #[test]
    fn malformed_record_identity_cannot_inherit_a_narrow_spatial_fence() -> Result<()> {
        let mut snapshot = snapshot();
        required_record(&mut snapshot, 10)?.generation = 0;
        snapshot.refresh_hash();
        let owned = BTreeSet::from([EntityId::new(20), EntityId::new(30)]);
        let work = inspect_untracked_work(&snapshot, &owned, 4)?;
        assert_eq!(work.len(), 1);
        let far = MapCuboid::new(MapCoord::new(50, 50, 10), MapCoord::new(51, 51, 10))?;
        assert!(work[0].conflicts_with(&far));
        assert!(matches!(work[0].state, EffectWorkState::Unknown { .. }));
        assert_eq!(work[0].to_json()["scope"], "unknown_spatial");
        required_record(&mut snapshot, 10)?.id = EntityId::new(99);
        snapshot.refresh_hash();
        assert!(inspect_untracked_work(&snapshot, &owned, 4).is_err());
        Ok(())
    }

    #[test]
    fn losing_all_construction_metadata_does_not_turn_restored_work_into_absence() -> Result<()> {
        let mut snapshot = snapshot();
        let building = required_record(&mut snapshot, 20)?;
        for field in [
            effects::CONSTRUCTION_STAGE_FIELD,
            "progress_ticks",
            "required_ticks",
        ] {
            building.fields.remove(field);
        }
        snapshot.refresh_hash();
        let owned = BTreeSet::from([EntityId::new(10), EntityId::new(30)]);
        let work = inspect_untracked_work(&snapshot, &owned, 4)?;
        assert_eq!(work.len(), 1);
        assert_eq!(work[0].entity_id, EntityId::new(20));
        assert!(matches!(work[0].state, EffectWorkState::Unknown { .. }));
        let area = MapCuboid::new(MapCoord::new(4, 4, 10), MapCoord::new(5, 5, 10))?;
        assert!(work[0].conflicts_with(&area));
        Ok(())
    }
}
