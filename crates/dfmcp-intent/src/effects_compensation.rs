//! Source-qualified inverse setters. Missing and unavailable configuration is
//! not a known false value, and a setter's no-op members must never be inverted.

use dfmcp_core::{DfmcpError, EntityId, ErrorCode, Result};
use dfmcp_world::{CompareOp, EntityKind, Predicate, PredicateEvidence, Value};

use super::{Action, BURROW_FIELD_PREFIX, LABOR_FIELD_PREFIX};

const MAX_COMPENSATION_UNITS: usize = 512;

/// Restore only the configuration this setter changes in the supplied source.
/// Every target must have an independently qualified prior boolean value.
/// Unknown input, a complete no-op, or another action family has no automatic
/// inverse. This is a planning recipe, not authority to apply an undo later:
/// dispatch must preserve the source guard and compensation must check drift.
pub fn default_compensation_with_evidence(
    action: &Action,
    evidence: &PredicateEvidence<'_>,
) -> Result<Option<Action>> {
    Ok(source_bound_compensation(action, evidence)?.map(|(inverse, _)| inverse))
}

pub(crate) fn source_bound_compensation(
    action: &Action,
    evidence: &PredicateEvidence<'_>,
) -> Result<Option<(Action, Predicate)>> {
    let (units, field, target, burrow) = match action {
        Action::Pause { paused } => {
            let prior = Predicate::Paused(!*paused);
            return Ok(evidence
                .establishes(&prior)?
                .then_some((Action::Pause { paused: !*paused }, prior)));
        }
        Action::SetLabor {
            units,
            labor,
            enabled,
        } => (
            units,
            format!("{LABOR_FIELD_PREFIX}{labor}"),
            *enabled,
            None,
        ),
        Action::SetBurrowMembership {
            units,
            burrow,
            assigned,
        } => (
            units,
            format!("{BURROW_FIELD_PREFIX}{}", burrow.get()),
            *assigned,
            Some(*burrow),
        ),
        _ => return Ok(None),
    };
    if units.len() > MAX_COMPENSATION_UNITS {
        return Err(DfmcpError::new(
            ErrorCode::BudgetExceeded,
            "source-qualified compensation exceeds its unit bound",
        ));
    }
    let mut guards = Vec::with_capacity(units.len() * 2 + usize::from(burrow.is_some()));
    if let Some(burrow) = burrow {
        let kind = Predicate::EntityKind {
            entity_id: burrow,
            kind: EntityKind::Burrow,
        };
        if !evidence.establishes(&kind)? {
            return Ok(None);
        }
        guards.push(kind);
    }
    let mut changed = Vec::new();
    for unit in units {
        let kind = Predicate::EntityKind {
            entity_id: *unit,
            kind: EntityKind::Unit,
        };
        if !evidence.establishes(&kind)? {
            return Ok(None);
        }
        guards.push(kind);
        let changed_prior = field_is(*unit, &field, !target);
        if evidence.establishes(&changed_prior)? {
            changed.push(*unit);
            guards.push(changed_prior);
        } else {
            let unchanged_prior = field_is(*unit, &field, target);
            if !evidence.establishes(&unchanged_prior)? {
                // Do not turn an unavailable member into a guessed inverse,
                // or advertise a partial inverse for an unknown full effect.
                return Ok(None);
            }
            guards.push(unchanged_prior);
        }
    }
    if changed.is_empty() {
        return Ok(None);
    }
    changed.sort_unstable();
    changed.dedup();
    let inverse = match action {
        Action::SetLabor { labor, .. } => Action::SetLabor {
            units: changed,
            labor: labor.clone(),
            enabled: !target,
        },
        Action::SetBurrowMembership { burrow, .. } => Action::SetBurrowMembership {
            units: changed,
            burrow: *burrow,
            assigned: !target,
        },
        _ => return Ok(None),
    };
    Ok(Some((inverse, Predicate::All(guards).normalized())))
}

fn field_is(entity_id: EntityId, field: &str, value: bool) -> Predicate {
    Predicate::FieldCompare {
        entity_id,
        field: field.to_owned(),
        op: CompareOp::Eq,
        value: Value::Bool(value),
    }
}

/// The original source is not the dispatch preimage of a later setter when an
/// earlier step writes the same configuration. Keep such forward workflows
/// usable, but do not invent their automatic inverse from the initial source.
/// Both actions have already been normalized, including sorted unit identities.
pub(crate) fn setters_overlap(left: &Action, right: &Action) -> bool {
    match (left, right) {
        (Action::Pause { .. }, Action::Pause { .. }) => true,
        (
            Action::SetLabor {
                units: left,
                labor: left_labor,
                ..
            },
            Action::SetLabor {
                units: right,
                labor: right_labor,
                ..
            },
        ) if left_labor == right_labor => left.iter().any(|unit| right.binary_search(unit).is_ok()),
        (
            Action::SetBurrowMembership {
                units: left,
                burrow: left_burrow,
                ..
            },
            Action::SetBurrowMembership {
                units: right,
                burrow: right_burrow,
                ..
            },
        ) if left_burrow == right_burrow => {
            left.iter().any(|unit| right.binary_search(unit).is_ok())
        }
        _ => false,
    }
}
