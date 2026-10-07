//! Physical reference work is distinct from the outcome of a goal's proof.
//!
//! A deadline or an early terminal predicate does not stop a designation,
//! construction or work order. This inspection uses the action's exact derived
//! entity identity and eligible laboratory observations. It performs no effect
//! and grants no authority to cancel or otherwise change the world.

use dfmcp_core::{DfmcpError, EntityId, ErrorCode, GameTick, Result};
use dfmcp_world::{EntityRecord, PredicateEvidence, Value, WorldSnapshot, laboratory_fact_value};

use crate::{Action, effects};

/// Whether one action still owns work that the reference simulator can advance.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EffectWorkState {
    /// Retained dispatch bookkeeping proves this step never started.
    NeverDispatched,
    /// Its exact, source-qualified reference work entity remains active.
    Active { entity_id: EntityId },
    /// It has no ongoing effect, or its reference work is complete/cancelled.
    Quiescent { entity_id: Option<EntityId> },
    /// Dispatch is known, but the observation cannot establish its work state.
    Unknown {
        entity_id: Option<EntityId>,
        reason: String,
    },
}

impl EffectWorkState {
    /// This proves no physical work remains, not that future dispatch is fenced.
    /// A coordinator must also terminalize an undispatched action before closing.
    #[must_use]
    pub const fn is_quiescent(&self) -> bool {
        matches!(self, Self::NeverDispatched | Self::Quiescent { .. })
    }
}

fn field<'a>(entity: &'a EntityRecord, name: &str, tick: GameTick) -> Option<&'a Value> {
    entity
        .fields
        .get(name)
        .and_then(|fact| laboratory_fact_value(fact, tick))
}

fn unknown(entity_id: EntityId, reason: &str) -> EffectWorkState {
    EffectWorkState::Unknown {
        entity_id: Some(entity_id),
        reason: reason.to_owned(),
    }
}

/// Inspect physical work without changing an immutable action proof receipt.
///
/// `dispatched` must come from retained execution bookkeeping, never from a
/// missing entity. Missing or incompatible state after dispatch is unresolved.
/// The reference model creates generation one; a replacement generation cannot
/// inherit the original action's ownership just by reusing its numeric key.
pub fn inspect_effect_work(
    snapshot: &WorldSnapshot,
    action: &Action,
    idempotency_key: &str,
    dispatched: bool,
) -> Result<EffectWorkState> {
    let _observation = PredicateEvidence::laboratory(snapshot)?;
    if !dispatched {
        return Ok(EffectWorkState::NeverDispatched);
    }
    let (kind, lifecycle) = match action {
        Action::DesignateDig { .. } => (effects::DIG_DESIGNATION_KIND, effects::STATUS_FIELD),
        Action::Build { .. } => ("building", effects::CONSTRUCTION_STAGE_FIELD),
        Action::CreateWorkOrder { .. } => ("work_order", effects::STATUS_FIELD),
        Action::Pause { .. }
        | Action::SetLabor { .. }
        | Action::ConfigureStockpile { .. }
        | Action::AssignSquad { .. }
        | Action::SetBurrowMembership { .. }
        | Action::SetStandingOrder { .. } => {
            return Ok(EffectWorkState::Quiescent { entity_id: None });
        }
        Action::Extension { .. } => {
            return Ok(EffectWorkState::Unknown {
                entity_id: None,
                reason: "extension has no registered reference work lifecycle".to_owned(),
            });
        }
    };
    if idempotency_key.is_empty() || idempotency_key.len() > 512 {
        return Err(DfmcpError::new(
            ErrorCode::InvalidPlan,
            "physical work inspection requires a bounded, nonempty effect identity",
        ));
    }
    let entity_id = effects::created_entity_id(idempotency_key, 0);
    let Some(entity) = snapshot.graph.entities.get(&entity_id) else {
        return Ok(unknown(entity_id, "dispatched work entity is not observed"));
    };
    if entity.kind.as_str() != kind || entity.generation != 1 {
        return Ok(unknown(
            entity_id,
            "work entity kind or generation no longer matches its dispatch",
        ));
    }
    // Verify immutable effect parameters before interpreting or authorizing a
    // stop of this record. A matching lifecycle word alone is not ownership.
    let identity_matches = match action {
        Action::DesignateDig { area, mode } => {
            field(entity, "area_min", snapshot.tick) == Some(&Value::Coord(area.min))
                && field(entity, "area_max", snapshot.tick) == Some(&Value::Coord(area.max))
                && field(entity, "target_tile_code", snapshot.tick)
                    == Some(&Value::U64(u64::from(effects::dig_target_tile_code(*mode))))
        }
        Action::Build {
            kind,
            location,
            footprint,
            ..
        } => {
            field(entity, "position", snapshot.tick) == Some(&Value::Coord(*location))
                && field(entity, "footprint_min", snapshot.tick)
                    == Some(&Value::Coord(footprint.min))
                && field(entity, "footprint_max", snapshot.tick)
                    == Some(&Value::Coord(footprint.max))
                && field(entity, "building_kind", snapshot.tick)
                    == Some(&Value::Text(effects::building_kind_label(kind)))
        }
        Action::CreateWorkOrder {
            job_token, amount, ..
        } => {
            field(entity, "job_token", snapshot.tick) == Some(&Value::Text(job_token.clone()))
                && field(entity, "amount_total", snapshot.tick)
                    == Some(&Value::U64(u64::from(*amount)))
        }
        _ => false,
    };
    if !identity_matches {
        return Ok(unknown(
            entity_id,
            "work entity lacks eligible parameters matching the dispatched action",
        ));
    }
    let result = match field(entity, lifecycle, snapshot.tick) {
        Some(Value::Text(status))
            if status == effects::STATUS_CANCELLED || status == effects::STATUS_COMPLETE =>
        {
            EffectWorkState::Quiescent {
                entity_id: Some(entity_id),
            }
        }
        Some(Value::Text(status))
            if (matches!(action, Action::Build { .. })
                && (status == effects::STAGE_PLANNED
                    || status == effects::STAGE_UNDER_CONSTRUCTION))
                || (!matches!(action, Action::Build { .. })
                    && status == effects::STATUS_ACTIVE) =>
        {
            EffectWorkState::Active { entity_id }
        }
        _ => unknown(
            entity_id,
            "work lifecycle is missing, ineligible or not a registered state",
        ),
    };
    Ok(result)
}
