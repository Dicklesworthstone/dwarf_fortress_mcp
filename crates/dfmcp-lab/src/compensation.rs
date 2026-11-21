//! Live dispatch custody for inverse setters. This is deliberately not restored
//! from durable history: recovered originals have no compensation authority.

use std::collections::{BTreeMap, BTreeSet};

use dfmcp_core::{ActionId, DfmcpError, EntityId, ErrorCode, Result};
use dfmcp_intent::{Action, PlanStep, effects};
use dfmcp_world::{EntityKind, Fact, Predicate, PredicateEvidence, WorldSnapshot};

#[derive(Clone, Debug)]
struct EntityStamp {
    id: EntityId,
    generation: u32,
    revision: u64,
    kind: EntityKind,
    field: Option<(String, Fact)>,
}

#[derive(Clone, Debug)]
pub(super) struct CompensationGuard {
    source_matches: bool,
    entities: Vec<EntityStamp>,
    pause_generation: Option<u64>,
    superseded: bool,
}

fn refused(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::CancellationIncomplete, message)
}

impl CompensationGuard {
    pub(super) fn capture_source(
        step: &PlanStep,
        snapshot: &WorldSnapshot,
    ) -> Result<Option<Self>> {
        let Some(compensation) = &step.compensation else {
            return Ok(None);
        };
        let inverse_setter = match (&step.action, compensation) {
            (Action::Pause { paused: before }, Action::Pause { paused: after }) => before != after,
            (
                Action::SetLabor {
                    labor: before,
                    enabled,
                    ..
                },
                Action::SetLabor {
                    labor: after,
                    enabled: restored,
                    ..
                },
            ) => before == after && enabled != restored,
            (
                Action::SetBurrowMembership {
                    burrow: before,
                    assigned,
                    ..
                },
                Action::SetBurrowMembership {
                    burrow: after,
                    assigned: restored,
                    ..
                },
            ) => before == after && assigned != restored,
            _ => false,
        };
        if !inverse_setter || protective_release(step) {
            // Explicit reassertions and separately authored compensating
            // effects retain their policy semantics; neither claims an inverse.
            return Ok(None);
        }
        let expected = effects::default_compensation_with_evidence(
            &step.action,
            &PredicateEvidence::laboratory(snapshot)?,
        )?;
        Ok(Some(Self {
            source_matches: expected.as_ref() == Some(compensation),
            entities: Vec::new(),
            pause_generation: None,
            superseded: false,
        }))
    }

    pub(super) fn bind_effect(
        &mut self,
        action: &Action,
        snapshot: &WorldSnapshot,
        pause_generation: u64,
    ) -> Result<()> {
        let (units, field, burrow) = match action {
            Action::Pause { .. } => {
                self.pause_generation = Some(pause_generation);
                return Ok(());
            }
            Action::SetLabor { units, labor, .. } => (
                units,
                format!("{}{labor}", effects::LABOR_FIELD_PREFIX),
                None,
            ),
            Action::SetBurrowMembership { units, burrow, .. } => (
                units,
                format!("{}{}", effects::BURROW_FIELD_PREFIX, burrow.get()),
                Some(*burrow),
            ),
            _ => return Err(refused("inverse setter lost its original action family")),
        };
        for (id, field) in units
            .iter()
            .map(|id| (*id, Some(field.as_str())))
            .chain(burrow.map(|id| (id, None)))
        {
            let record = snapshot
                .graph
                .entities
                .get(&id)
                .ok_or_else(|| refused("inverse setter lost its dispatched entity"))?;
            let field = field
                .map(|name| {
                    record
                        .fields
                        .get(name)
                        .cloned()
                        .map(|fact| (name.to_owned(), fact))
                        .ok_or_else(|| refused("inverse setter has no observed effect field"))
                })
                .transpose()?;
            self.entities.push(EntityStamp {
                id,
                generation: record.generation,
                revision: record.revision,
                kind: record.kind.clone(),
                field,
            });
        }
        Ok(())
    }

    pub(super) fn validate(
        &self,
        step: &PlanStep,
        snapshot: &WorldSnapshot,
        pause_generation: u64,
    ) -> Result<()> {
        if !self.source_matches {
            return Err(refused(
                "sealed inverse does not restore the qualified dispatch source; stop without compensation or review a fresh action",
            ));
        }
        if self.superseded {
            return Err(refused(
                "another setter superseded this effect, including a same-value reassertion; review the current configuration",
            ));
        }
        if self
            .pause_generation
            .is_some_and(|generation| generation != pause_generation)
        {
            return Err(refused(
                "another clock action superseded this pause effect; review the current clock state",
            ));
        }
        for stamp in &self.entities {
            if !snapshot
                .graph
                .entities
                .get(&stamp.id)
                .is_some_and(|record| {
                    record.id == stamp.id
                        && record.generation == stamp.generation
                        && record.revision == stamp.revision
                        && record.kind == stamp.kind
                        && stamp
                            .field
                            .as_ref()
                            .is_none_or(|(name, fact)| record.fields.get(name) == Some(fact))
                })
            {
                return Err(refused(
                    "inverse setter source changed after dispatch; stop without compensation or review the current configuration",
                ));
            }
        }
        let evidence = PredicateEvidence::laboratory(snapshot)?;
        for predicate in effects::default_postconditions(
            &step.action,
            &step.idempotency_key,
            snapshot.fortress_id,
        ) {
            if !evidence.establishes(&predicate)? {
                return Err(refused(
                    "inverse setter's original effect is no longer established",
                ));
            }
        }
        Ok(())
    }
}

/// A qualified same-value setter does not change canonical facts or revisions,
/// but it is still another actor's explicit configuration decision. Permanently
/// supersede older live inverse custody for those exact fields. Storage stays
/// bounded by the existing retained-action limit; no historical writer map grows.
pub(super) fn supersede_written_guards(
    actions: &mut BTreeMap<ActionId, super::LabAction>,
    effect: &Action,
) {
    let (units, field) = match effect {
        Action::SetLabor { units, labor, .. } => {
            (units, format!("{}{labor}", effects::LABOR_FIELD_PREFIX))
        }
        Action::SetBurrowMembership { units, burrow, .. } => (
            units,
            format!("{}{}", effects::BURROW_FIELD_PREFIX, burrow.get()),
        ),
        _ => return,
    };
    let units: BTreeSet<_> = units.iter().copied().collect();
    for action in actions.values_mut() {
        if action.dispatched
            && !action.receipt.state.is_terminal()
            && let Some(guard) = &mut action.compensation_guard
            && !guard.superseded
            && guard.entities.iter().any(|stamp| {
                units.contains(&stamp.id)
                    && stamp.field.as_ref().is_some_and(|(name, _)| name == &field)
            })
        {
            guard.superseded = true;
        }
    }
}

/// The registered release compiler explicitly reasserts protection on cancel,
/// even for a previously unassigned civilian. It seals exact roster identities
/// and a positive release-safety guard; its existing incarnation check remains
/// mandatory at finalization. An arbitrary boolean branch is not this policy.
fn protective_release(step: &PlanStep) -> bool {
    let (
        Action::SetBurrowMembership {
            units,
            burrow,
            assigned: false,
        },
        Some(Action::SetBurrowMembership {
            units: restored,
            burrow: restored_burrow,
            assigned: true,
        }),
    ) = (&step.action, &step.compensation)
    else {
        return false;
    };
    if units != restored || burrow != restored_burrow {
        return false;
    }
    let mut pending: Vec<_> = step.preconditions.iter().collect();
    while let Some(predicate) = pending.pop() {
        match predicate {
            Predicate::All(children) => pending.extend(children),
            Predicate::LaboratoryCivilianReleaseSafety {
                burrow: exact,
                civilians,
            } if exact.id == *burrow
                && civilians.len() == units.len()
                && civilians
                    .iter()
                    .all(|civilian| units.contains(&civilian.id)) =>
            {
                return true;
            }
            _ => {}
        }
    }
    false
}
