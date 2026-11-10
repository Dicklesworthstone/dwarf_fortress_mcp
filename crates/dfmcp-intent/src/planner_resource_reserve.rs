//! Additive admission for a finite acquisition stage whose original production
//! stock minima already hold but whose explicitly requested reserve needs work.
//! The older production-only constraint retains its closed action set.

use super::*;

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
    // The regular harvest planner subsequently verifies source generations,
    // conserved finite supply, worker capacity and exact mandatory task proof.
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
    }
    Ok(Some(until))
}

fn invalid(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::InvalidIntent, message)
}
