use std::collections::BTreeMap;

use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, Digest32, FortressId, IntentId,
    ObservationCursor, OperationContext, RequestId, RiskTier, SessionId, WorkBudget,
};
use dfmcp_world::{Fact, FactPresence, FactSource, PredicateTruth, WorldGraph};

use super::*;
use crate::{Constraint, Intent, StaticPlanner};

fn fact(value: Value, tick: GameTick) -> Fact {
    Fact::known(
        value,
        tick,
        FactSource::Derived("dfmcp.lab-scenario/1".to_owned()),
        Digest32::ZERO,
    )
}

fn source() -> WorldSnapshot {
    let mut snapshot = WorldSnapshot::new(
        FortressId::new(7_014),
        GameTick(100),
        ObservationCursor::ORIGIN,
        false,
        WorldGraph::default(),
    );
    let id = effects::fortress_settings_entity_id(snapshot.fortress_id);
    snapshot.graph.entities.insert(
        id,
        EntityRecord {
            id,
            generation: 1,
            revision: 1,
            kind: EntityKind::Other(effects::FORTRESS_SETTINGS_KIND.to_owned()),
            label: "project staffing fixture".to_owned(),
            fields: BTreeMap::from([(
                effects::LABORATORY_WORKFORCE_MODEL_FIELD.to_owned(),
                fact(
                    Value::Text(effects::LABORATORY_WORKFORCE_MODEL.to_owned()),
                    snapshot.tick,
                ),
            )]),
        },
    );
    snapshot.refresh_hash();
    snapshot
}

fn worker(
    snapshot: &mut WorldSnapshot,
    id: u64,
    mining: bool,
    construction: bool,
    squad: Option<Value>,
) {
    let id = EntityId::new(id);
    let mut fields = BTreeMap::from([
        ("alive".to_owned(), fact(Value::Bool(true), snapshot.tick)),
        (
            "labor.MINING".to_owned(),
            fact(Value::Bool(mining), snapshot.tick),
        ),
        (
            "labor.CONSTRUCTION".to_owned(),
            fact(Value::Bool(construction), snapshot.tick),
        ),
    ]);
    if let Some(squad) = squad {
        fields.insert(effects::SQUAD_FIELD.to_owned(), fact(squad, snapshot.tick));
    }
    snapshot.graph.entities.insert(
        id,
        EntityRecord {
            id,
            generation: 1,
            revision: 1,
            kind: EntityKind::Unit,
            label: "project worker".to_owned(),
            fields,
        },
    );
    snapshot.refresh_hash();
}

fn context(snapshot: &WorldSnapshot) -> OperationContext {
    OperationContext {
        session_id: SessionId::new(7_014),
        request_id: RequestId::new(7_014),
        anchor: snapshot.anchor(),
        budget: WorkBudget::default(),
        grants: vec![CapabilityGrant {
            capability: Capability::Plan,
            scope: CapabilityScope::default(),
            max_risk: RiskTier::ReadOnly,
            expires_at_tick: None,
            remaining_uses: None,
        }],
        cancellation_requested: false,
    }
}

#[test]
fn one_worker_supplies_sequential_setup_without_extra_mutations_or_source_changes() -> Result<()> {
    let mut snapshot = source();
    worker(&mut snapshot, 10, false, false, Some(Value::Null));
    let original = snapshot.clone();
    let staffing = ProjectSetupStaffing::plan(
        &snapshot,
        &[ProjectSetupLabor::Construction, ProjectSetupLabor::Mining],
        true,
    )?;
    assert_eq!(staffing.anchor, original.anchor());
    assert_eq!(snapshot, original);
    assert_eq!(staffing.actions.len(), 2);
    assert_eq!(staffing.selections.len(), 2);
    for (index, labor) in [ProjectSetupLabor::Mining, ProjectSetupLabor::Construction]
        .into_iter()
        .enumerate()
    {
        let selection = &staffing.selections[index];
        assert_eq!(selection.labor, labor);
        assert_eq!(selection.worker, EntityId::new(10));
        assert_eq!(selection.generation, 1);
        assert!(!selection.previously_enabled);
        assert_eq!(selection.action_index, Some(index as u32));
        let action = &staffing.actions[index];
        assert_eq!(action.preconditions, selection.source_preconditions());
        assert_eq!(
            action.action,
            Action::SetLabor {
                units: vec![EntityId::new(10)],
                labor: labor.token().to_owned(),
                enabled: true,
            }
        );
        assert_eq!(
            action.compensation,
            Some(Action::SetLabor {
                units: vec![EntityId::new(10)],
                labor: labor.token().to_owned(),
                enabled: false,
            })
        );
        assert!(action.obligation.is_none());
        assert!(action.depends_on.is_empty());
    }
    let canonical = ProjectSetupStaffing::plan(
        &snapshot,
        &[ProjectSetupLabor::Mining, ProjectSetupLabor::Construction],
        true,
    )?;
    assert_eq!(staffing, canonical);
    let repeated = ProjectSetupStaffing::plan(
        &snapshot,
        &[ProjectSetupLabor::Mining, ProjectSetupLabor::Mining],
        true,
    )?;
    assert_eq!(repeated.actions, canonical.actions[..1]);
    Ok(())
}

#[test]
fn existing_capacity_wins_before_nonmilitary_preference_and_canonical_identity() -> Result<()> {
    let mut snapshot = source();
    worker(&mut snapshot, 1, false, false, Some(Value::Null));
    worker(&mut snapshot, 2, true, false, Some(Value::U64(500)));
    worker(&mut snapshot, 3, false, true, None);
    worker(&mut snapshot, 4, false, true, Some(Value::Null));
    worker(&mut snapshot, 5, false, true, Some(Value::Null));
    let staffing = ProjectSetupStaffing::plan(
        &snapshot,
        &[ProjectSetupLabor::Mining, ProjectSetupLabor::Construction],
        true,
    )?;
    assert!(staffing.actions.is_empty());
    assert_eq!(staffing.selections[0].worker, EntityId::new(2));
    assert_eq!(staffing.selections[1].worker, EntityId::new(4));
    assert!(staffing.selections.iter().all(|choice| {
        choice.previously_enabled && choice.action_index.is_none()
    }));
    assert_eq!(
        staffing,
        ProjectSetupStaffing::plan(
            &snapshot,
            &[ProjectSetupLabor::Mining, ProjectSetupLabor::Construction],
            false,
        )?
    );
    Ok(())
}

#[test]
fn enabling_prefers_explicit_nonmembership_then_unknown_then_membership() -> Result<()> {
    let mut snapshot = source();
    worker(&mut snapshot, 1, false, false, Some(Value::U64(500)));
    worker(&mut snapshot, 2, false, false, None);
    worker(&mut snapshot, 3, false, false, Some(Value::Null));
    let required = [ProjectSetupLabor::Construction];
    let staffing = ProjectSetupStaffing::plan(&snapshot, &required, true)?;
    assert_eq!(staffing.selections[0].worker, EntityId::new(3));
    let unit = snapshot
        .graph
        .entities
        .get_mut(&EntityId::new(3))
        .ok_or_else(|| refused("fixture worker missing"))?;
    unit.fields.insert(
        "alive".to_owned(),
        fact(Value::Bool(false), snapshot.tick),
    );
    snapshot.refresh_hash();
    let staffing = ProjectSetupStaffing::plan(&snapshot, &required, true)?;
    assert_eq!(staffing.selections[0].worker, EntityId::new(2));
    // An asserted nonmembership cannot improve the preference over a genuine
    // unknown; this is a choice policy, never proof of absence from a squad.
    let unit = snapshot
        .graph
        .entities
        .get_mut(&EntityId::new(1))
        .ok_or_else(|| refused("fixture worker missing"))?;
    let squad = unit
        .fields
        .get_mut(effects::SQUAD_FIELD)
        .ok_or_else(|| refused("fixture squad missing"))?;
    squad.value = Value::Null;
    squad.presence = None;
    squad.source = FactSource::AgentAssertion("not observed".to_owned());
    snapshot.refresh_hash();
    let staffing = ProjectSetupStaffing::plan(&snapshot, &required, true)?;
    assert_eq!(staffing.selections[0].worker, EntityId::new(1));
    Ok(())
}

#[test]
fn disabled_or_unavailable_labor_never_creates_an_unrequested_assignment() -> Result<()> {
    let mut snapshot = source();
    worker(&mut snapshot, 10, true, false, Some(Value::Null));
    let before = snapshot.clone();
    assert!(matches!(
        ProjectSetupStaffing::plan(&snapshot, &[ProjectSetupLabor::Construction], false),
        Err(error) if error.code == ErrorCode::PreconditionsFailed
    ));
    let reused = ProjectSetupStaffing::plan(&snapshot, &[ProjectSetupLabor::Mining], false)?;
    assert!(reused.actions.is_empty());
    assert_eq!(snapshot, before);
    Ok(())
}

#[test]
fn unavailable_or_unqualified_source_fields_never_supply_staffing_or_compensation() -> Result<()> {
    for field in ["alive", "labor.MINING"] {
        for variant in 0..12 {
            let mut snapshot = source();
            worker(&mut snapshot, 10, false, false, Some(Value::Null));
            let anchor = snapshot.anchor();
            let unit = snapshot
                .graph
                .entities
                .get_mut(&EntityId::new(10))
                .ok_or_else(|| refused("fixture worker missing"))?;
            if variant == 0 {
                unit.fields.remove(field);
            } else {
                let input = unit
                    .fields
                    .get_mut(field)
                    .ok_or_else(|| refused("fixture input missing"))?;
                match variant {
                    1 => input.presence = Some(FactPresence::Unknown("unread".to_owned())),
                    2 => input.presence = Some(FactPresence::Absent),
                    3 => input.presence = Some(FactPresence::Omitted("bounded".to_owned())),
                    4 => input.presence = Some(FactPresence::Stale(anchor)),
                    5 => input.source = FactSource::AgentAssertion("unobserved".to_owned()),
                    6 => input.source = FactSource::Replay,
                    7 => input.source = FactSource::DfhackField("unit.labor".to_owned()),
                    8 => input.source = FactSource::Derived("unregistered".to_owned()),
                    9 => input.source_digest = Digest32::of_bytes(b"changed producer"),
                    10 => input.observed_at = GameTick(snapshot.tick.0 + 1),
                    _ => {
                        input.value = Value::Text("false".to_owned());
                        input.presence = None;
                    }
                }
            }
            snapshot.refresh_hash();
            let before = snapshot.clone();
            assert!(
                matches!(
                    ProjectSetupStaffing::plan(&snapshot, &[ProjectSetupLabor::Mining], true),
                    Err(error) if error.code == ErrorCode::PreconditionsFailed
                ),
                "unavailable {field} variant {variant} supplied staffing"
            );
            assert_eq!(snapshot, before);
            // An unknown citizen is never needed to remove a qualified
            // choice from the complete census. It simply supplies no labor.
            worker(&mut snapshot, 20, false, false, Some(Value::Null));
            let selected =
                ProjectSetupStaffing::plan(&snapshot, &[ProjectSetupLabor::Mining], true)?;
            assert_eq!(selected.selections[0].worker, EntityId::new(20));
        }
    }
    Ok(())
}

#[test]
fn guarded_setup_prepares_and_executes_with_real_source_evidence() -> Result<()> {
    let mut snapshot = source();
    worker(&mut snapshot, 10, false, false, Some(Value::Null));
    let staffing = ProjectSetupStaffing::plan(
        &snapshot,
        &[ProjectSetupLabor::Mining, ProjectSetupLabor::Construction],
        true,
    )?;
    let goal = Predicate::All(
        staffing
            .actions
            .iter()
            .flat_map(|action| action.postconditions.clone())
            .collect(),
    );
    let intent = Intent {
        id: IntentId::new(7_014),
        anchor: snapshot.anchor(),
        summary: "provide missing project setup labor".to_owned(),
        terminal_condition: goal.clone(),
        constraints: vec![Constraint::MaxRisk(RiskTier::Reversible)],
        requested_actions: staffing.actions.clone(),
    };
    let prepared =
        StaticPlanner::default().prepare_laboratory(&snapshot, &intent, &context(&snapshot))?;
    assert!(prepared.digest_is_valid());
    assert_eq!(prepared.steps.len(), 2);
    assert_eq!(
        prepared.required_capabilities,
        [Capability::ConfigureLabor].into_iter().collect()
    );
    assert!(matches!(
        StaticPlanner::default().prepare(&snapshot, &intent, &context(&snapshot)),
        Err(error) if error.code == ErrorCode::PreconditionsFailed
    ));
    let original = snapshot.clone();
    for step in &prepared.steps {
        for predicate in &step.preconditions {
            assert!(PredicateEvidence::laboratory(&snapshot)?.establishes(predicate)?);
        }
        assert!(effects::apply_effect(
            &mut snapshot,
            &step.action,
            &step.idempotency_key,
        )?);
        snapshot.refresh_hash();
        for predicate in &step.postconditions {
            assert!(PredicateEvidence::laboratory(&snapshot)?.establishes(predicate)?);
        }
    }
    assert_eq!(
        PredicateEvidence::laboratory(&snapshot)?.evaluate(&goal)?,
        PredicateTruth::True
    );
    let no_changes = ProjectSetupStaffing::plan(
        &snapshot,
        &[ProjectSetupLabor::Mining, ProjectSetupLabor::Construction],
        false,
    )?;
    assert!(no_changes.actions.is_empty());
    // Reframing the original actions at the new anchor does not bypass their
    // required false prior values or invent compensable no-op enables.
    let mut reused = intent;
    reused.anchor = snapshot.anchor();
    reused.terminal_condition = Predicate::EntityExists(EntityId::new(20));
    assert!(matches!(
        StaticPlanner::default().prepare_laboratory(&snapshot, &reused, &context(&snapshot)),
        Err(error) if error.code == ErrorCode::PreconditionsFailed
    ));
    assert_ne!(snapshot.anchor(), original.anchor());
    Ok(())
}

#[test]
fn model_identity_source_integrity_and_worker_generation_remain_explicit() -> Result<()> {
    let mut snapshot = source();
    worker(&mut snapshot, 10, false, false, Some(Value::Null));
    let empty = ProjectSetupStaffing::plan(&snapshot, &[], false)?;
    assert!(empty.selections.is_empty() && empty.actions.is_empty());
    let settings = effects::fortress_settings_entity_id(snapshot.fortress_id);
    let mut absent = snapshot.clone();
    absent.graph.entities.remove(&settings);
    absent.refresh_hash();
    assert!(matches!(
        ProjectSetupStaffing::plan(&absent, &[], true),
        Err(error) if error.code == ErrorCode::PreconditionsFailed
    ));
    let mut claimed = snapshot.clone();
    claimed
        .graph
        .entities
        .get_mut(&settings)
        .and_then(|settings| {
            settings
                .fields
                .get_mut(effects::LABORATORY_WORKFORCE_MODEL_FIELD)
        })
        .ok_or_else(|| refused("fixture model missing"))?
        .source = FactSource::AgentAssertion("claimed shared model".to_owned());
    claimed.refresh_hash();
    assert!(matches!(
        ProjectSetupStaffing::plan(&claimed, &[], true),
        Err(error) if error.code == ErrorCode::PreconditionsFailed
    ));
    let original = snapshot.anchor();
    snapshot
        .graph
        .entities
        .get_mut(&EntityId::new(10))
        .ok_or_else(|| refused("fixture worker missing"))?
        .generation = 2;
    assert!(matches!(
        ProjectSetupStaffing::plan(&snapshot, &[ProjectSetupLabor::Mining], true),
        Err(error) if error.code == ErrorCode::ChecksumMismatch
    ));
    snapshot.refresh_hash();
    let selected = ProjectSetupStaffing::plan(&snapshot, &[ProjectSetupLabor::Mining], true)?;
    assert_eq!(selected.selections[0].generation, 2);
    assert_ne!(selected.anchor, original);
    let mut aliased = snapshot.clone();
    aliased
        .graph
        .entities
        .get_mut(&EntityId::new(10))
        .ok_or_else(|| refused("fixture worker missing"))?
        .id = EntityId::new(11);
    aliased.refresh_hash();
    assert!(matches!(
        ProjectSetupStaffing::plan(&aliased, &[ProjectSetupLabor::Mining], true),
        Err(error) if error.code == ErrorCode::InternalInvariantViolation
    ));
    Ok(())
}

#[test]
fn requirement_and_census_bounds_refuse_without_partial_staffing() {
    let snapshot = source();
    assert!(matches!(
        ProjectSetupStaffing::plan(&snapshot, &[ProjectSetupLabor::Mining; 3], true),
        Err(error) if error.code == ErrorCode::BudgetExceeded
    ));
    let mut oversized = snapshot;
    for id in 1..=MAX_PROJECT_SETUP_ENTITIES as u64 {
        let id = EntityId::new(id);
        oversized.graph.entities.insert(
            id,
            EntityRecord {
                id,
                generation: 1,
                revision: 1,
                kind: EntityKind::Item,
                label: String::new(),
                fields: BTreeMap::new(),
            },
        );
    }
    // Bounds are checked before hashing or scanning an oversized source.
    assert!(matches!(
        ProjectSetupStaffing::plan(&oversized, &[ProjectSetupLabor::Mining], true),
        Err(error) if error.code == ErrorCode::BudgetExceeded
    ));
}
