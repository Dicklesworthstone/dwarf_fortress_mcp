//! Bounded planning allowances for the opt-in shared laboratory workforce.
//!
//! Physical work already present in the world competes with every newly
//! requested activity. Count that service even when it is currently blocked;
//! this is a conservative deadline input, never a completion or staffing proof.
//! Marker-free worlds retain the original planner and its exact sealed bytes.

use dfmcp_core::{DfmcpError, ErrorCode, GameTick, Result, StateAnchor};
use dfmcp_world::{CompareOp, Predicate, Value, WorldSnapshot};

use crate::{Action, PreparedPlan, RequestedAction, effects};

const MAX_NEW_ACTIVITIES: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaboratoryWorkforcePlanning {
    pub anchor: StateAnchor,
    pub queued_activities: usize,
    pub queued_service_ticks: u64,
}

impl LaboratoryWorkforcePlanning {
    pub fn capture(snapshot: &WorldSnapshot) -> Result<Option<Self>> {
        Ok(
            effects::laboratory_workforce_report(snapshot)?.map(|report| Self {
                anchor: snapshot.anchor(),
                queued_activities: report.active_activities,
                queued_service_ticks: report.remaining_service_ticks,
            }),
        )
    }

    /// A common absolute floor for synthesized obligations. Serializing all
    /// old and new service is conservative even when several workers can run
    /// concurrently. The allowance also covers ordinary observation boundaries.
    /// Callers still retain longer dependency horizons and explicit deadlines.
    pub fn deadline_floor(&self, actions: &[RequestedAction]) -> Result<GameTick> {
        if actions.len() > MAX_NEW_ACTIVITIES {
            return Err(horizon_error());
        }
        let mut service = self.queued_service_ticks;
        let mut activities = u64::try_from(self.queued_activities).map_err(|_| horizon_error())?;
        for requested in actions {
            let Some(ticks) = service_ticks(&requested.action)? else {
                continue;
            };
            service = service.checked_add(ticks).ok_or_else(horizon_error)?;
            activities = activities.checked_add(1).ok_or_else(horizon_error)?;
        }
        let settlement = activities
            .checked_mul(effects::DEFAULT_POLL_INTERVAL_TICKS)
            .and_then(|ticks| ticks.checked_mul(2))
            .ok_or_else(horizon_error)?;
        let allowance = service
            .checked_mul(2)
            .and_then(|ticks| ticks.checked_add(settlement))
            .and_then(|ticks| ticks.checked_add(100))
            .filter(|ticks| *ticks <= effects::MAX_DEFAULT_OBLIGATION_TICKS)
            .ok_or_else(horizon_error)?;
        self.anchor
            .tick
            .checked_add(allowance)
            .ok_or_else(horizon_error)
    }

    /// Sealing this guard binds temporal work to its explicit model version.
    /// It does not certify the complete workload read used by this allowance.
    #[must_use]
    pub fn model_precondition(&self) -> Predicate {
        Predicate::FieldCompare {
            entity_id: effects::fortress_settings_entity_id(self.anchor.fortress_id),
            field: effects::LABORATORY_WORKFORCE_MODEL_FIELD.to_owned(),
            op: CompareOp::Eq,
            value: Value::Text(effects::LABORATORY_WORKFORCE_MODEL.to_owned()),
        }
    }

    pub fn validate_deadline(&self, deadline: GameTick) -> Result<()> {
        if deadline
            .0
            .checked_sub(self.anchor.tick.0)
            .is_none_or(|ticks| ticks > effects::MAX_DEFAULT_OBLIGATION_TICKS)
        {
            return Err(horizon_error());
        }
        Ok(())
    }
}

/// Current workload inspection includes negative and range reads beyond an
/// action's ordinary witness. The shell must review a freshly sealed replay
/// after any anchor change instead of silently carrying this allowance over.
#[must_use]
pub fn plan_uses_shared_workforce(plan: &PreparedPlan) -> bool {
    let guard = Predicate::FieldCompare {
        entity_id: effects::fortress_settings_entity_id(plan.anchor.fortress_id),
        field: effects::LABORATORY_WORKFORCE_MODEL_FIELD.to_owned(),
        op: CompareOp::Eq,
        value: Value::Text(effects::LABORATORY_WORKFORCE_MODEL.to_owned()),
    };
    plan.steps
        .iter()
        .any(|step| step.preconditions.contains(&guard))
}

fn service_ticks(action: &Action) -> Result<Option<u64>> {
    match action {
        Action::DesignateDig { area, .. } => area
            .tile_count()
            .and_then(|tiles| tiles.checked_mul(effects::DIG_TICKS_PER_TILE))
            .map(Some)
            .ok_or_else(horizon_error),
        Action::Build { .. } | Action::PlaceFurniture { .. } => Ok(Some(effects::BUILD_TICKS)),
        Action::CreateWorkOrder { amount, .. } => u64::from(*amount)
            .checked_mul(effects::WORK_ORDER_TICKS_PER_UNIT)
            .map(Some)
            .ok_or_else(horizon_error),
        _ => Ok(None),
    }
}

fn horizon_error() -> DfmcpError {
    DfmcpError::new(
        ErrorCode::BudgetExceeded,
        "shared laboratory workforce planning exceeds its 64-action or one-game-year service horizon",
    )
}

#[cfg(test)]
#[path = "workforce_planning_tests.rs"]
mod tests;
