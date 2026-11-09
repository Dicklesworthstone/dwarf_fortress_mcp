//! Original finite ecological supply and mandatory harvest proof.
//! Planning counts all current unfinished demand without reserving resources
//! or crediting future harvest as observed production input.

use std::collections::BTreeMap;

use dfmcp_core::{DfmcpError, EntityId, ErrorCode, Result};
use dfmcp_world::{CompareOp, EntityKind, Predicate, Value, WorldSnapshot};

use crate::{Action, RequestedAction, effects};

pub(crate) struct HarvestPlanning {
    report: effects::HarvestReport,
}

fn bounded() -> DfmcpError {
    DfmcpError::new(
        ErrorCode::BudgetExceeded,
        "harvest planning exceeds its action, quantity or predicate bound",
    )
}

fn condition(id: EntityId, field: &str, op: CompareOp, value: Value) -> Predicate {
    Predicate::FieldCompare {
        entity_id: id,
        field: field.to_owned(),
        op,
        value,
    }
}

impl HarvestPlanning {
    pub(crate) fn capture(
        snapshot: &WorldSnapshot,
        actions: &[RequestedAction],
    ) -> Result<Option<Self>> {
        if !actions
            .iter()
            .any(|requested| matches!(requested.action, Action::HarvestResource { .. }))
        {
            return Ok(None);
        }
        if actions.len() > 64 {
            return Err(bounded());
        }
        let report = effects::harvest_report(snapshot)?.ok_or_else(|| {
            DfmcpError::new(
                ErrorCode::PreconditionsFailed,
                "harvest planning requires explicit finite-harvest/1 semantics",
            )
        })?;
        let sources: BTreeMap<_, _> = report
            .sources
            .iter()
            .map(|source| (source.source, source))
            .collect();
        let mut demand = BTreeMap::<EntityId, u64>::new();
        for requested in actions {
            let Action::HarvestResource {
                source,
                source_generation,
                resource,
                amount,
            } = &requested.action
            else {
                continue;
            };
            if *source == EntityId::NIL
                || *source_generation == 0
                || u32::try_from(*source_generation).is_err()
                || !(1..=effects::MAX_HARVEST_AMOUNT).contains(amount)
            {
                return Err(DfmcpError::new(
                    ErrorCode::InvalidIntent,
                    "harvest action requires a positive source/generation and amount in 1..=100",
                ));
            }
            let observed = sources.get(source).ok_or_else(|| {
                DfmcpError::new(
                    ErrorCode::PreconditionsFailed,
                    "selected harvest source is not qualified",
                )
            })?;
            if observed.generation != *source_generation || observed.resource != *resource {
                return Err(DfmcpError::new(
                    ErrorCode::PreconditionsFailed,
                    "selected harvest source identity, generation or resource changed",
                ));
            }
            let total = demand.entry(*source).or_insert(observed.active_demand);
            *total = total.checked_add(u64::from(*amount)).ok_or_else(bounded)?;
            if *total > observed.remaining {
                return Err(DfmcpError::new(
                    ErrorCode::PreconditionsFailed,
                    "harvest plan exceeds finite source supply after complete active demand",
                ));
            }
        }
        Ok(Some(Self { report }))
    }

    pub(crate) fn bind(
        &self,
        snapshot: &WorldSnapshot,
        requested: &mut RequestedAction,
        key: &str,
    ) -> Result<()> {
        let Action::HarvestResource {
            source,
            resource,
            amount,
            ..
        } = &requested.action
        else {
            return Ok(());
        };
        requested.preconditions.extend([
            condition(
                effects::fortress_settings_entity_id(snapshot.fortress_id),
                effects::LABORATORY_HARVEST_MODEL_FIELD,
                CompareOp::Eq,
                Value::Text(effects::LABORATORY_HARVEST_MODEL.to_owned()),
            ),
            Predicate::EntityKind {
                entity_id: *source,
                kind: EntityKind::Plant,
            },
            condition(
                *source,
                effects::HARVEST_SOURCE_MODEL_FIELD,
                CompareOp::Eq,
                Value::Text(effects::LABORATORY_HARVEST_MODEL.to_owned()),
            ),
            condition(
                *source,
                effects::HARVEST_RESOURCE_FIELD,
                CompareOp::Eq,
                Value::Text(resource.as_str().to_owned()),
            ),
            condition(
                *source,
                effects::HARVEST_REMAINING_FIELD,
                CompareOp::Ge,
                Value::U64(u64::from(*amount)),
            ),
            Predicate::EntityKind {
                entity_id: self.report.ledger,
                kind: EntityKind::Other(effects::STOCK_LEDGER_KIND.to_owned()),
            },
        ]);
        let id = effects::created_entity_id(key, 0);
        let mut required = effects::harvest_completion_predicates(&requested.action, key);
        required.extend([
            condition(
                id,
                effects::HARVEST_LEDGER_FIELD,
                CompareOp::Eq,
                Value::Entity(self.report.ledger),
            ),
            condition(
                id,
                effects::HARVEST_LEDGER_GENERATION_FIELD,
                CompareOp::Eq,
                Value::U64(self.report.ledger_generation),
            ),
        ]);
        requested.postconditions.extend(required.iter().cloned());
        for predicates in [&mut requested.preconditions, &mut requested.postconditions] {
            *predicates = predicates.iter().map(Predicate::normalized).collect();
            predicates.sort_by_key(Predicate::canonical_bytes);
            predicates.dedup();
            if predicates.len() > 64 {
                return Err(bounded());
            }
        }
        if let Some(obligation) = &mut requested.obligation {
            required.push(obligation.terminal.clone());
            obligation.terminal = Predicate::All(required).normalized();
            obligation.terminal.validate_shape()?;
        }
        Ok(())
    }
}
