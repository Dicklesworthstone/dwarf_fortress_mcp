//! Source-qualified setup labor for complete laboratory projects.
//!
//! This is an opt-in proposal compiler, not a worker reservation or a native
//! job assignment. It supplies the mining and construction capacity on which
//! later physical actions depend. The shell must retain the complete source
//! request, require reviewed replay after anchor changes, authorize the labor
//! mutations, and wire their step dependencies before deriving any effect keys.

use dfmcp_core::{DfmcpError, EntityId, ErrorCode, GameTick, Result, StateAnchor};
use dfmcp_world::{
    CompareOp, EntityKind, EntityRecord, Predicate, PredicateEvidence, Value, WorldSnapshot,
    laboratory_fact_value,
};

use crate::{Action, RequestedAction, effects};

/// Match the bounded source domain used by laboratory production staffing.
pub const MAX_PROJECT_SETUP_ENTITIES: usize = 65_536;
const MAX_SETUP_LABORS: usize = 2;

/// The closed prerequisite families needed by excavation and construction.
/// Recipe-specific production staffing remains owned by its existing compiler.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ProjectSetupLabor {
    Mining,
    Construction,
}

impl ProjectSetupLabor {
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::Mining => "MINING",
            Self::Construction => "CONSTRUCTION",
        }
    }

    #[must_use]
    pub const fn for_action(action: &Action) -> Option<Self> {
        match action {
            Action::DesignateDig { .. } => Some(Self::Mining),
            Action::Build { .. } | Action::PlaceFurniture { .. } => Some(Self::Construction),
            _ => None,
        }
    }

    fn field(self) -> String {
        format!("{}{}", effects::LABOR_FIELD_PREFIX, self.token())
    }
}

/// One current, qualified worker choice. The generation is inspection and
/// source-custody data: the existing predicate language does not encode an
/// entity-generation guard, so callers must preserve the exact anchor and
/// ordinary witness/replay checks rather than treating these guards as an ABA
/// certificate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectSetupSelection {
    pub labor: ProjectSetupLabor,
    pub worker: EntityId,
    pub generation: u32,
    pub previously_enabled: bool,
    /// Index into `ProjectSetupStaffing::actions`, before shell insertion.
    pub action_index: Option<u32>,
    source_preconditions: Vec<Predicate>,
}

impl ProjectSetupSelection {
    /// Guards describing the reviewed source, including the original labor
    /// value. An initially disabled guard belongs on its enabling action, not
    /// on physical work that must start after that enable has been proved.
    #[must_use]
    pub fn source_preconditions(&self) -> &[Predicate] {
        &self.source_preconditions
    }
}

/// A bounded, deterministic proposal for the exact trusted reference snapshot.
/// Empty requirements are valid but still require the explicit workforce model.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectSetupStaffing {
    pub anchor: StateAnchor,
    pub selections: Vec<ProjectSetupSelection>,
    pub actions: Vec<RequestedAction>,
}

struct Candidate<'a> {
    unit: &'a EntityRecord,
    enabled: bool,
    military: u8,
}

impl Candidate<'_> {
    fn preference(&self) -> (bool, u8, EntityId) {
        (!self.enabled, self.military, self.unit.id)
    }
}

impl ProjectSetupStaffing {
    /// Select one qualified living worker per requested setup family. Prefer
    /// already enabled labor, then explicit nonmilitary status, then unknown
    /// military status, then known military membership, then canonical ID.
    /// Different setup stages may reuse one worker; the finite workforce
    /// runtime still grants each worker only one activity per interval.
    ///
    /// Missing, unqualified, future or nonboolean liveness/labor facts supply
    /// no candidate. A false labor can be enabled only when `allow_assign` is
    /// explicit. No assignment occurs here, and this API grants no capability.
    /// The caller must own a complete laboratory world; imported or live data
    /// cannot become laboratory evidence by supplying its producer labels.
    pub fn plan(
        snapshot: &WorldSnapshot,
        required: &[ProjectSetupLabor],
        allow_assign: bool,
    ) -> Result<Self> {
        if required.len() > MAX_SETUP_LABORS
            || snapshot.graph.entities.len() > MAX_PROJECT_SETUP_ENTITIES
            || snapshot.graph.edges.len() > MAX_PROJECT_SETUP_ENTITIES
        {
            return Err(DfmcpError::new(
                ErrorCode::BudgetExceeded,
                "project setup staffing exceeds its two-labor or bounded source census allowance",
            ));
        }
        let evidence = PredicateEvidence::laboratory(snapshot)?;
        let snapshot = evidence.snapshot();
        if effects::laboratory_workforce_model(snapshot)?
            != Some(effects::LABORATORY_WORKFORCE_MODEL)
        {
            return Err(refused(
                "complete project staffing requires the explicit shared-workforce/1 laboratory model",
            ));
        }

        // Canonicalize the closed two-element set without allocating a roster
        // or accepting an unbounded duplicate input. Only these two scans and
        // at most two proposed actions depend on the caller's requirements.
        let mut staffing = Self {
            anchor: snapshot.anchor(),
            selections: Vec::with_capacity(MAX_SETUP_LABORS),
            actions: Vec::with_capacity(MAX_SETUP_LABORS),
        };
        for labor in [ProjectSetupLabor::Mining, ProjectSetupLabor::Construction] {
            if !required.contains(&labor) {
                continue;
            }
            let field = labor.field();
            let mut selected: Option<Candidate<'_>> = None;
            for unit in snapshot.graph.entities.values() {
                if unit.kind != EntityKind::Unit
                    || unit.id == EntityId::NIL
                    || unit.generation == 0
                    || value(unit, "alive", snapshot.tick) != Some(&Value::Bool(true))
                {
                    continue;
                }
                let enabled = match value(unit, &field, snapshot.tick) {
                    Some(Value::Bool(true)) => true,
                    Some(Value::Bool(false)) if allow_assign => false,
                    _ => continue,
                };
                let candidate = Candidate {
                    unit,
                    enabled,
                    military: military_rank(unit, snapshot.tick),
                };
                if selected
                    .as_ref()
                    .is_none_or(|current| candidate.preference() < current.preference())
                {
                    selected = Some(candidate);
                }
            }
            let selected = selected.ok_or_else(|| {
                refused(format!(
                    "project setup requires an established living worker with {field}=true, or a qualified false labor and prerequisites.assign_labor=true; unavailable facts cannot establish staffing or compensation",
                ))
            })?;
            let Candidate { unit, enabled, .. } = selected;
            let source_preconditions = guards(snapshot, unit.id, labor, enabled);
            let action_index = if enabled {
                None
            } else {
                let index = u32::try_from(staffing.actions.len()).map_err(|_| {
                    DfmcpError::new(
                        ErrorCode::BudgetExceeded,
                        "project setup staffing action index exceeds its bounded representation",
                    )
                })?;
                staffing.actions.push(RequestedAction {
                    action: Action::SetLabor {
                        units: vec![unit.id],
                        labor: labor.token().to_owned(),
                        enabled: true,
                    },
                    preconditions: source_preconditions.clone(),
                    postconditions: guards(snapshot, unit.id, labor, true),
                    compensation: Some(Action::SetLabor {
                        units: vec![unit.id],
                        labor: labor.token().to_owned(),
                        enabled: false,
                    }),
                    obligation: None,
                    depends_on: Vec::new(),
                });
                Some(index)
            };
            staffing.selections.push(ProjectSetupSelection {
                labor,
                worker: unit.id,
                generation: unit.generation,
                previously_enabled: enabled,
                action_index,
                source_preconditions,
            });
        }
        Ok(staffing)
    }
}

fn value<'a>(unit: &'a EntityRecord, field: &str, tick: GameTick) -> Option<&'a Value> {
    unit.fields
        .get(field)
        .and_then(|fact| laboratory_fact_value(fact, tick))
}

fn military_rank(unit: &EntityRecord, tick: GameTick) -> u8 {
    match value(unit, effects::SQUAD_FIELD, tick) {
        Some(Value::Null) => 0,
        Some(Value::Entity(_) | Value::U64(_)) => 2,
        _ => 1,
    }
}

fn guards(
    snapshot: &WorldSnapshot,
    worker: EntityId,
    labor: ProjectSetupLabor,
    enabled: bool,
) -> Vec<Predicate> {
    vec![
        Predicate::EntityKind {
            entity_id: worker,
            kind: EntityKind::Unit,
        },
        Predicate::FieldCompare {
            entity_id: worker,
            field: "alive".to_owned(),
            op: CompareOp::Eq,
            value: Value::Bool(true),
        },
        Predicate::FieldCompare {
            entity_id: worker,
            field: labor.field(),
            op: CompareOp::Eq,
            value: Value::Bool(enabled),
        },
        Predicate::FieldCompare {
            entity_id: effects::fortress_settings_entity_id(snapshot.fortress_id),
            field: effects::LABORATORY_WORKFORCE_MODEL_FIELD.to_owned(),
            op: CompareOp::Eq,
            value: Value::Text(effects::LABORATORY_WORKFORCE_MODEL.to_owned()),
        },
    ]
}

fn refused(message: impl Into<String>) -> DfmcpError {
    DfmcpError::new(ErrorCode::PreconditionsFailed, message)
}

#[cfg(test)]
#[path = "project_setup_staffing_tests.rs"]
mod tests;
