//! Joint finite WOOD feasibility and mandatory material proof for laboratory
//! workshop construction. Planning reserves no inventory; dispatch owns the
//! actual allocation. Legacy worlds produce no added guards or predicates.

use dfmcp_core::{DfmcpError, ErrorCode, Result};
use dfmcp_world::{CompareOp, EntityKind, Predicate, Value, WorldSnapshot, laboratory_fact_value};

use crate::{Action, RequestedAction, effects};

const MAX_ACTIONS: usize = 64;
const MAX_ENTITIES: usize = 65_536;

pub(crate) struct ConstructionMaterialPlanning {
    report: effects::ConstructionMaterialReport,
}

fn refused(message: impl Into<String>) -> DfmcpError {
    DfmcpError::new(ErrorCode::PreconditionsFailed, message)
}

fn bounded() -> DfmcpError {
    DfmcpError::new(
        ErrorCode::BudgetExceeded,
        "construction material planning exceeds its bounded resource domain",
    )
}

fn carpentry(job: &str) -> bool {
    matches!(
        effects::work_order_output_item(job),
        Some("BED" | "DOOR" | "TABLE" | "CHAIR")
    )
}

impl ConstructionMaterialPlanning {
    pub(crate) fn capture(
        snapshot: &WorldSnapshot,
        actions: &[RequestedAction],
    ) -> Result<Option<Self>> {
        if !actions
            .iter()
            .any(|step| matches!(step.action, Action::Build { .. }))
        {
            return Ok(None);
        }
        let Some(report) = effects::construction_material_report(snapshot)? else {
            return Ok(None);
        };
        if actions.len() > MAX_ACTIONS || snapshot.graph.entities.len() > MAX_ENTITIES {
            return Err(bounded());
        }
        let mut workshop_logs = 0u64;
        let mut new_furniture_logs = 0u64;
        for requested in actions {
            if let Some(cost) = effects::construction_wood_cost(snapshot, &requested.action)? {
                workshop_logs = workshop_logs.checked_add(cost).ok_or_else(bounded)?;
            }
            if let Action::CreateWorkOrder {
                job_token, amount, ..
            } = &requested.action
                && carpentry(job_token)
            {
                new_furniture_logs = new_furniture_logs
                    .checked_add(u64::from(*amount))
                    .ok_or_else(bounded)?;
            }
        }
        let mut queued_furniture_logs = 0u64;
        for record in snapshot.graph.entities.values() {
            if record.kind != EntityKind::WorkOrder {
                continue;
            }
            let field = |name: &str| {
                record
                    .fields
                    .get(name)
                    .and_then(|fact| laboratory_fact_value(fact, snapshot.tick))
            };
            match field(effects::STATUS_FIELD) {
                Some(Value::Text(status))
                    if status == effects::STATUS_COMPLETE
                        || status == effects::STATUS_CANCELLED =>
                {
                    continue;
                }
                Some(Value::Text(status)) if status == effects::STATUS_ACTIVE => {}
                _ => {
                    return Err(refused(
                        "construction resource planning cannot establish existing work-order lifecycle",
                    ));
                }
            }
            // Only a qualified closed recipe can supply its material cost.
            // The shared model separately refuses unavailable active selectors.
            let Some(Value::Text(job)) = field("job_token") else {
                return Err(refused(
                    "construction resource planning requires qualified existing work-order job identities",
                ));
            };
            if !carpentry(job) {
                continue;
            }
            let Some(Value::U64(remaining)) = field(effects::AMOUNT_REMAINING_FIELD) else {
                return Err(refused(
                    "construction resource planning cannot establish queued carpentry demand",
                ));
            };
            if !matches!(field("amount_total"), Some(Value::U64(total)) if *remaining > 0 && remaining <= total)
            {
                return Err(refused(
                    "construction resource planning requires canonical queued carpentry quantities",
                ));
            }
            if field(effects::WORK_ORDER_PRODUCTION_MODEL_FIELD)
                != Some(&Value::Text(effects::CARPENTRY_PRODUCTION_MODEL.to_owned()))
            {
                return Err(refused(
                    "construction resource planning cannot reinterpret abstract carpentry as material-backed history",
                ));
            }
            queued_furniture_logs = queued_furniture_logs
                .checked_add(*remaining)
                .ok_or_else(bounded)?;
        }
        let total = workshop_logs
            .checked_add(new_furniture_logs)
            .and_then(|count| count.checked_add(queued_furniture_logs))
            .ok_or_else(bounded)?;
        if total > report.available_wood {
            return Err(refused(format!(
                "construction plan requires {total} available WOOD logs ({workshop_logs} for new workshops, {new_furniture_logs} for new furniture, {queued_furniture_logs} for existing carpentry), but only {} qualified unallocated logs are available",
                report.available_wood,
            )));
        }
        Ok(Some(Self { report }))
    }

    pub(crate) fn bind(
        &self,
        snapshot: &WorldSnapshot,
        requested: &mut RequestedAction,
        key: &str,
    ) -> Result<()> {
        let Some(cost) = effects::construction_wood_cost(snapshot, &requested.action)? else {
            return Ok(());
        };
        requested.preconditions.extend([
            Predicate::EntityKind {
                entity_id: self.report.ledger,
                kind: EntityKind::Other(effects::STOCK_LEDGER_KIND.to_owned()),
            },
            Predicate::FieldCompare {
                entity_id: effects::fortress_settings_entity_id(snapshot.fortress_id),
                field: effects::LABORATORY_CONSTRUCTION_MATERIAL_MODEL_FIELD.to_owned(),
                op: CompareOp::Eq,
                value: Value::Text(effects::LABORATORY_CONSTRUCTION_MATERIAL_MODEL.to_owned()),
            },
            Predicate::FieldCompare {
                entity_id: self.report.ledger,
                field: effects::STOCK_WOOD_FIELD.to_owned(),
                op: CompareOp::Ge,
                value: Value::U64(cost),
            },
        ]);
        let required =
            effects::construction_completion_predicates(snapshot, &requested.action, key)?;
        requested.postconditions.extend(required.iter().cloned());
        for predicates in [&mut requested.preconditions, &mut requested.postconditions] {
            *predicates = predicates.iter().map(Predicate::normalized).collect();
            predicates.sort_by_key(Predicate::canonical_bytes);
            predicates.dedup();
            if predicates.len() > MAX_ACTIONS {
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
#[path = "construction_planning_tests.rs"]
mod tests;
