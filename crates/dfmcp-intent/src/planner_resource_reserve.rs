//! Additive admission for a finite acquisition stage whose original production
//! stock minima already hold but whose explicitly requested reserve needs work.
//! The older production-only constraint retains its closed action set.

use super::*;
use dfmcp_world::{CompareOp, EntityKind, Value, laboratory_fact_value};
use std::collections::BTreeMap;

pub(super) fn horizon(
    intent: &Intent,
    evidence: &PredicateEvidence<'_>,
    laboratory: bool,
) -> Result<Option<GameTick>> {
    let mut horizon = None;
    for constraint in &intent.constraints {
        if let Constraint::LaboratoryResourceProductionReserveUntil(until) = constraint {
            if !laboratory || horizon.is_some() {
                return Err(invalid(
                    "resource reserve admission requires one explicit laboratory-only horizon",
                ));
            }
            let ticks = until.0.checked_sub(evidence.snapshot().tick.0);
            if ticks.is_none_or(|ticks| {
                ticks == 0 || ticks > crate::effects::MAX_DEFAULT_OBLIGATION_TICKS
            }) {
                return Err(invalid(
                    "laboratory resource reserve horizon must be future and within one game year",
                ));
            }
            horizon = Some(*until);
        }
    }
    let Some(until) = horizon else {
        return Ok(None);
    };
    if intent
        .constraints
        .iter()
        .any(|constraint| matches!(constraint, Constraint::LaboratoryProductionReserveUntil(_)))
    {
        return Err(invalid(
            "production and acquisition reserve admissions cannot be combined",
        ));
    }
    if !production_stock_minima(&intent.terminal_condition, evidence.snapshot())
        || !evidence.establishes(&intent.terminal_condition)?
    {
        return Err(invalid(
            "laboratory resource reserve requires qualified established original production stock minima",
        ));
    }
    if !remaining_reserve_needed(evidence.snapshot(), &intent.terminal_condition, until)? {
        return Err(invalid(
            "qualified output stock already covers the remaining explicit production reserve",
        ));
    }
    // The regular harvest planner subsequently verifies source generations,
    // conserved finite supply, worker capacity and exact mandatory task proof.
    // The production compiler owns recipe and setup input sizing; this gate
    // proves remaining output need, not independent raw-material necessity.
    // Here the exception is limited to that registered effect family, with
    // enabled labor only for an actually requested harvest resource.
    let resources: BTreeSet<_> = intent
        .requested_actions
        .iter()
        .filter_map(|requested| match requested.action {
            Action::HarvestResource { resource, .. } => Some(resource),
            _ => None,
        })
        .collect();
    if resources.is_empty() {
        return Err(invalid(
            "laboratory resource reserve requires finite harvest work",
        ));
    }
    for requested in &intent.requested_actions {
        let allowed = match &requested.action {
            Action::HarvestResource { .. } => true,
            Action::SetLabor {
                labor,
                enabled: true,
                ..
            } => resources.iter().any(|resource| labor == resource.labor()),
            _ => false,
        };
        if !allowed {
            return Err(invalid(
                "laboratory resource reserve permits only finite harvest and its explicit harvesting labor setup",
            ));
        }
        if !compiler_compensation(requested) {
            return Err(invalid(
                "laboratory resource reserve compensation must be absent for harvest or disable exactly its own enabled harvesting labor",
            ));
        }
    }
    Ok(Some(until))
}

/// Preserve the acquisition compiler's exact inverse, including the ordinary
/// planner's equivalent default when an explicit inverse is omitted. A reserve
/// exception cannot carry unrelated effects in its compensation program.
fn compiler_compensation(requested: &RequestedAction) -> bool {
    match (&requested.action, &requested.compensation) {
        (Action::HarvestResource { .. }, None) | (Action::SetLabor { enabled: true, .. }, None) => {
            true
        }
        (
            Action::SetLabor {
                units,
                labor,
                enabled: true,
            },
            Some(Action::SetLabor {
                units: inverse_units,
                labor: inverse_labor,
                enabled: false,
            }),
        ) => units == inverse_units && labor == inverse_labor,
        _ => false,
    }
}

/// This prediction cannot establish future stock. The already validated,
/// complete laboratory evidence must qualify every current population member,
/// original stock minimum and metabolism boundary before admitting more work.
fn remaining_reserve_needed(
    snapshot: &WorldSnapshot,
    terminal: &Predicate,
    until: GameTick,
) -> Result<bool> {
    const MAX_SOURCE_RECORDS: usize = 65_536;
    if snapshot.graph.entities.len() > MAX_SOURCE_RECORDS
        || snapshot.graph.edges.len() > MAX_SOURCE_RECORDS
        || snapshot.graph.chunks.len() > MAX_SOURCE_RECORDS
        || snapshot.graph.events.len() > MAX_SOURCE_RECORDS
    {
        return Err(bounded());
    }
    terminal.validate_shape()?;
    let horizon = until
        .0
        .checked_sub(snapshot.tick.0)
        .filter(|ticks| *ticks > 0 && *ticks <= crate::effects::MAX_DEFAULT_OBLIGATION_TICKS)
        .ok_or_else(|| invalid("resource reserve requires its unchanged bounded future tick"))?;
    let mut ledgers = snapshot.graph.entities.iter().filter(|(_, entity)| {
        matches!(&entity.kind, EntityKind::Other(kind)
            if kind == crate::effects::STOCK_LEDGER_KIND)
    });
    let (ledger_id, ledger) = ledgers
        .next()
        .ok_or_else(|| refused("resource reserve requires its complete original stock ledger"))?;
    if ledgers.next().is_some()
        || *ledger_id != ledger.id
        || *ledger_id == EntityId::NIL
        || ledger.generation == 0
    {
        return Err(refused(
            "resource reserve stock ledger custody is ambiguous",
        ));
    }
    let mut minima = BTreeMap::new();
    collect_minima(terminal, *ledger_id, &mut minima)?;
    let mut living = 0u64;
    for (key, unit) in &snapshot.graph.entities {
        if unit.kind.as_str() != EntityKind::Unit.as_str() {
            continue;
        }
        // The legacy canonical encoding aliases Other("unit") with Unit.
        // Such a retag must not silently remove a consumer at the same anchor.
        if unit.kind != EntityKind::Unit {
            return Err(refused(
                "resource reserve population contains a noncanonical unit kind alias",
            ));
        }
        if *key != unit.id || unit.id == EntityId::NIL || unit.generation == 0 {
            return Err(refused("resource reserve population identity is ambiguous"));
        }
        match unit
            .fields
            .get("alive")
            .and_then(|fact| laboratory_fact_value(fact, snapshot.tick))
        {
            Some(Value::Bool(true)) => living += 1,
            Some(Value::Bool(false)) => {}
            _ => {
                return Err(refused(
                    "resource reserve population is not source-qualified",
                ));
            }
        }
    }
    let Some(Value::U64(metabolism)) = ledger
        .fields
        .get(crate::effects::METABOLISM_TICKS_FIELD)
        .and_then(|fact| laboratory_fact_value(fact, snapshot.tick))
    else {
        return Err(refused(
            "resource reserve metabolism is not source-qualified",
        ));
    };
    let after = metabolism.checked_add(horizon).ok_or_else(bounded)?;
    let mut needed = false;
    for (field, minimum) in minima {
        let interval = match field.as_str() {
            crate::effects::STOCK_DRINK_FIELD => Some(crate::effects::DRINK_INTERVAL_TICKS),
            crate::effects::STOCK_FOOD_FIELD => Some(crate::effects::FOOD_INTERVAL_TICKS),
            _ => None,
        };
        let allowance = interval
            .map_or(Some(0), |interval| {
                (after / interval - metabolism / interval).checked_mul(living)
            })
            .ok_or_else(bounded)?;
        let target = minimum.checked_add(allowance).ok_or_else(bounded)?;
        let Some(Value::U64(observed)) = ledger
            .fields
            .get(&field)
            .and_then(|fact| laboratory_fact_value(fact, snapshot.tick))
        else {
            return Err(refused(
                "resource reserve output stock is not source-qualified",
            ));
        };
        needed |= *observed < target;
    }
    Ok(needed)
}

/// Zero minima and repeated submitted quotas are valid published v1 requests.
/// Their conjunction has the same requirement as its greatest minimum; this
/// validation must not silently introduce a different historical goal grammar.
fn collect_minima(
    predicate: &Predicate,
    ledger: EntityId,
    minima: &mut BTreeMap<String, u64>,
) -> Result<()> {
    match predicate {
        Predicate::FieldCompare {
            entity_id,
            field,
            op: CompareOp::Ge,
            value: Value::U64(minimum),
        } if *entity_id == ledger
            && matches!(
                field.as_str(),
                crate::effects::STOCK_DRINK_FIELD
                    | crate::effects::STOCK_FOOD_FIELD
                    | crate::effects::STOCK_BED_FIELD
                    | crate::effects::STOCK_DOOR_FIELD
                    | crate::effects::STOCK_TABLE_FIELD
                    | crate::effects::STOCK_CHAIR_FIELD
            ) =>
        {
            let retained = minima.entry(field.clone()).or_default();
            *retained = (*retained).max(*minimum);
            Ok(())
        }
        Predicate::All(terms) if !terms.is_empty() => {
            for term in terms {
                collect_minima(term, ledger, minima)?;
            }
            Ok(())
        }
        _ => Err(invalid(
            "resource reserve requires closed original output minima in its one stock ledger",
        )),
    }
}

fn refused(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::PreconditionsFailed, message)
}

fn bounded() -> DfmcpError {
    DfmcpError::new(
        ErrorCode::BudgetExceeded,
        "resource reserve exceeds its complete source or exact arithmetic bound",
    )
}

fn invalid(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::InvalidIntent, message)
}
