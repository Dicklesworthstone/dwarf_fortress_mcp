//! Finite shared ingredient demand and required production proof for the
//! explicit laboratory provisions model. Planning does not reserve inputs.

use dfmcp_core::{DfmcpError, ErrorCode, Result};
use dfmcp_world::{CompareOp, EntityKind, Predicate, Value, WorldSnapshot};

use crate::{Action, RequestedAction, effects};

const MAX_ACTIONS: usize = 64;
const MAX_PREDICATES: usize = 64;

pub(crate) struct ProvisionsPlanning {
    report: effects::ProvisionsReport,
}

fn bounded() -> DfmcpError {
    DfmcpError::new(
        ErrorCode::BudgetExceeded,
        "provisions planning exceeds its bounded resource or predicate domain",
    )
}

impl ProvisionsPlanning {
    pub(crate) fn capture(
        snapshot: &WorldSnapshot,
        actions: &[RequestedAction],
    ) -> Result<Option<Self>> {
        if !actions.iter().any(|step| {
            matches!(&step.action, Action::CreateWorkOrder { job_token, .. }
                if matches!(effects::work_order_output_item(job_token), Some("FOOD" | "DRINK")))
        }) {
            return Ok(None);
        }
        let Some(report) = effects::provisions_report(snapshot)? else {
            return Ok(None);
        };
        if actions.len() > MAX_ACTIONS {
            return Err(bounded());
        }
        let mut new_demand = 0u64;
        for requested in actions {
            if let Some(cost) = effects::provisions_input_cost(snapshot, &requested.action)? {
                new_demand = new_demand.checked_add(cost).ok_or_else(bounded)?;
            }
        }
        let total = new_demand
            .checked_add(report.remaining_demand_plants)
            .ok_or_else(bounded)?;
        if total > report.available_plants {
            return Err(DfmcpError::new(
                ErrorCode::PreconditionsFailed,
                format!(
                    "provisions plan requires {total} available PLANT units ({new_demand} for new food/drink batches and {} for existing unfinished batches), but only {} qualified units are available",
                    report.remaining_demand_plants, report.available_plants,
                ),
            ));
        }
        Ok(Some(Self { report }))
    }

    pub(crate) fn bind(
        &self,
        snapshot: &WorldSnapshot,
        requested: &mut RequestedAction,
        key: &str,
    ) -> Result<()> {
        let Some(cost) = effects::provisions_input_cost(snapshot, &requested.action)? else {
            return Ok(());
        };
        requested.preconditions.extend([
            Predicate::EntityKind {
                entity_id: self.report.ledger,
                kind: EntityKind::Other(effects::STOCK_LEDGER_KIND.to_owned()),
            },
            Predicate::FieldCompare {
                entity_id: effects::fortress_settings_entity_id(snapshot.fortress_id),
                field: effects::LABORATORY_PROVISIONS_MODEL_FIELD.to_owned(),
                op: CompareOp::Eq,
                value: Value::Text(effects::LABORATORY_PROVISIONS_MODEL.to_owned()),
            },
            Predicate::FieldCompare {
                entity_id: self.report.ledger,
                field: effects::STOCK_PLANT_FIELD.to_owned(),
                op: CompareOp::Ge,
                value: Value::U64(cost),
            },
        ]);
        let required = effects::provisions_completion_predicates(snapshot, &requested.action, key)?;
        requested.postconditions.extend(required.iter().cloned());
        for predicates in [&mut requested.preconditions, &mut requested.postconditions] {
            *predicates = predicates.iter().map(Predicate::normalized).collect();
            predicates.sort_by_key(Predicate::canonical_bytes);
            predicates.dedup();
            if predicates.len() > MAX_PREDICATES {
                return Err(bounded());
            }
        }
        if let Some(obligation) = &mut requested.obligation {
            let mut terminal = required;
            terminal.push(obligation.terminal.clone());
            obligation.terminal = Predicate::All(terminal).normalized();
            obligation.terminal.validate_shape()?;
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "provisions_planning_tests.rs"]
mod tests;
