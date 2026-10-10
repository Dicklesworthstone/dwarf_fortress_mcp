//! Original-goal setup regressions for df-lab-conditional-production-c4p.
//! These import the real compiler, planner and laboratory adapter.

use super::*;
use dfmcp_adapter::GameAdapter;
use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, CommitState, IntentId, OperationContext,
    RequestId, RiskTier, SessionId, WorkBudget,
};
use dfmcp_intent::{Constraint, Intent, StaticPlanner};
use dfmcp_lab::MemoryAdapter;
use dfmcp_world::{FactPresence, PredicateEvidence, PredicateTruth};

fn setup_world() -> Result<WorldSnapshot> {
    let mut snapshot = scenario_snapshot("starter_fortress", FortressId::new(731), false)?;
    snapshot.graph.entities.remove(&starter::STILL);
    snapshot.graph.entities.remove(&starter::KITCHEN);
    for unit in snapshot
        .graph
        .entities
        .values_mut()
        .filter(|unit| unit.kind == EntityKind::Unit)
    {
        unit.fields.insert("labor.BREW".to_owned(), lab_fact(Value::Bool(false)));
        unit.fields.insert("labor.COOK".to_owned(), lab_fact(Value::Bool(false)));
    }
    snapshot.refresh_hash();
    Ok(snapshot)
}

fn setup_json() -> Json {
    json!({
        "template":"production",
        "quotas":[{"item":"DRINK","minimum":50},{"item":"FOOD","minimum":70}],
        "prerequisites":{
            "assign_labor":true,
            "workshops":[
                {"building":"workshop:Still","location":[0,0,10]},
                {"building":"workshop:Kitchen","location":[2,0,10]},
            ],
        },
    })
}

fn set_field(snapshot: &mut WorldSnapshot, unit: u64, field: &str, value: Value) -> Result<()> {
    snapshot
        .graph
        .entities
        .get_mut(&EntityId::new(unit))
        .ok_or_else(|| invalid("fixture entity missing"))?
        .fields
        .insert(field.to_owned(), lab_fact(value));
    snapshot.refresh_hash();
    Ok(())
}

fn setup_row<'a>(
    compiled: &'a ProductionCompilation,
    section: &str,
    job: &str,
) -> Result<&'a Json> {
    compiled.analysis
        .get("prerequisites")
        .and_then(|setup| setup.get(section))
        .and_then(Json::as_array)
        .and_then(|rows| rows.iter().find(|row| row["job_token"] == job))
        .ok_or_else(|| invalid(format!("missing {section} row for {job}")))
}

fn context(snapshot: &WorldSnapshot) -> OperationContext {
    OperationContext {
        session_id: SessionId::new(1),
        request_id: RequestId::new(1),
        anchor: snapshot.anchor(),
        budget: WorkBudget::default(),
        grants: [
            (Capability::Observe, RiskTier::ReadOnly),
            (Capability::Plan, RiskTier::ReadOnly),
            (Capability::Construct, RiskTier::Guarded),
            (Capability::ConfigureLabor, RiskTier::Reversible),
            (Capability::ConfigureProduction, RiskTier::Reversible),
            (Capability::ControlClock, RiskTier::Reversible),
            (Capability::Checkpoint, RiskTier::Guarded),
        ]
        .into_iter()
        .map(|(capability, max_risk)| CapabilityGrant {
            capability,
            scope: CapabilityScope {
                fortress_id: Some(snapshot.fortress_id),
                ..CapabilityScope::default()
            },
            max_risk,
            expires_at_tick: None,
            remaining_uses: None,
        })
        .collect(),
        cancellation_requested: false,
    }
}

fn seal(snapshot: &WorldSnapshot, compiled: &ProductionCompilation) -> Result<PreparedPlan> {
    StaticPlanner::default().prepare_laboratory(
        snapshot,
        &Intent {
            id: IntentId::new(731),
            anchor: snapshot.anchor(),
            summary: "establish the original drink and food reserves".to_owned(),
            terminal_condition: compiled.terminal.clone(),
            constraints: vec![Constraint::MaxRisk(RiskTier::Guarded)],
            requested_actions: parse_steps(&compiled.actions)?,
        },
        &context(snapshot),
    )
}

#[test]
fn setup_source_is_closed_bounded_canonical_and_preserves_legacy_bytes() -> Result<()> {
    let legacy = ProductionRequest::parse(
        r#"{"template":"production","quotas":[{"item":"DRINK","minimum":50}]}"#,
    )?;
    assert_eq!(
        legacy.canonical_json(),
        json!({"template":"production","quotas":[{"item":"DRINK","minimum":50}]}).to_string()
    );
    let first = ProductionRequest::parse(&setup_json().to_string())?;
    let mut reordered = setup_json();
    reordered["quotas"] = json!([{"minimum":70,"item":"FOOD"},{"minimum":50,"item":"DRINK"}]);
    reordered["prerequisites"]["workshops"] = json!([
        {"building":"workshop:Kitchen","location":[2,0,10],"min":[2,0,10],"max":[2,0,10]},
        {"building":"workshop:Still","location":[0,0,10],"min":[0,0,10],"max":[0,0,10]},
    ]);
    let second = ProductionRequest::parse(&reordered.to_string())?;
    assert_eq!(first, second);
    assert_eq!(first.canonical_json(), second.canonical_json());
    assert_eq!(ProductionRequest::parse(&first.canonical_json())?, first);

    let still = json!({"building":"workshop:Still","location":[0,0,10]});
    for sites in [
        json!([still.clone(), still.clone()]),
        json!([still.clone(), still.clone(), still.clone()]),
        json!([{"building":"workshop:Forge","location":[0,0,10]}]),
        json!([{"building":"workshop:Still","location":[0,0,10],"min":[0,0,10]}]),
        json!([{"building":"workshop:Still","location":[0,0,10],"max":[0,0,10]}]),
        json!([{"building":"workshop:Still","location":[0,0,10],"min":[0,0,10],"max":[64,0,10]}]),
        json!([{"building":"workshop:Still","location":[0,0,10],"min":[0,0,10],"max":[0,0,11]}]),
        json!([{"building":"workshop:Still","location":[2,0,10],"min":[0,0,10],"max":[1,0,10]}]),
        json!([{"building":"workshop:Still","location":[i32::MAX,0,10]}]),
        json!([{"building":"workshop:Still","location":[i32::MIN,0,10]}]),
        json!([{"building":"workshop:Still","location":[0,0,10],"materials":["WOOD"]}]),
        json!([
            {"building":"workshop:Still","location":[0,0,10]},
            {"building":"workshop:Kitchen","location":[0,0,10]},
        ]),
    ] {
        let mut request = setup_json();
        request["prerequisites"]["workshops"] = sites;
        assert!(ProductionRequest::parse(&request.to_string()).is_err(), "{request}");
    }
    let mut maximum = setup_json();
    maximum["prerequisites"]["workshops"] = json!([
        {"building":"workshop:Still","location":[0,0,10],"min":[0,0,10],"max":[63,0,10]},
    ]);
    assert!(ProductionRequest::parse(&maximum.to_string()).is_ok());
    let mut unrelated = setup_json();
    unrelated["quotas"] = json!([{"item":"DRINK","minimum":50}]);
    assert!(ProductionRequest::parse(&unrelated.to_string()).is_err());
    for options in [
        json!({"assign_labor":"yes"}),
        json!({"assign_labor":true,"unknown":true}),
        json!({"workshops":"automatic"}),
    ] {
        let mut request = setup_json();
        request["prerequisites"] = options;
        assert!(ProductionRequest::parse(&request.to_string()).is_err(), "{request}");
    }
    Ok(())
}

#[test]
fn expanded_goal_seals_real_setup_dependencies_and_completes_through_lab_adapter() -> Result<()> {
    let mut source = setup_world()?;
    set_field(&mut source, 1001, "labor.MINE", Value::Bool(true))?;
    let before = source.anchor();
    let compiled = ProductionRequest::parse(&setup_json().to_string())?.compile(&source)?;
    assert_eq!(source.anchor(), before, "planning cannot change the source");
    assert_eq!(compiled.analysis["prerequisites"]["steps_added"], 4);
    let steps = parse_steps(&compiled.actions)?;
    assert_eq!(steps.len(), 6);
    assert_eq!(steps[4].depends_on, vec![0, 1]);
    assert_eq!(steps[5].depends_on, vec![2, 3]);
    assert!(matches!(&steps[0].action, Action::SetLabor { units, labor, enabled: true }
        if units == &vec![EntityId::new(1001)] && labor == "COOK"));
    assert!(matches!(&steps[2].action, Action::SetLabor { units, labor, enabled: true }
        if units == &vec![EntityId::new(1002)] && labor == "BREW"));
    let plan = seal(&source, &compiled)?;
    assert!(plan.digest_is_valid());
    assert_eq!(plan.terminal_condition, compiled.terminal);
    assert_eq!(
        plan.required_capabilities,
        BTreeSet::from([
            Capability::ConfigureLabor,
            Capability::Construct,
            Capability::ConfigureProduction,
            Capability::Checkpoint,
        ])
    );
    for index in [0, 2] {
        assert!(matches!(plan.steps[index].compensation,
            Some(Action::SetLabor { enabled: false, .. })));
        assert_eq!(
            plan.steps[index].postconditions,
            effects::default_postconditions(
                &plan.steps[index].action,
                &plan.steps[index].idempotency_key,
                source.fortress_id,
            )
        );
    }
    for (parent, child) in [(1, 4), (3, 5)] {
        let parent_deadline = plan.steps[parent].obligation.as_ref()
            .ok_or_else(|| invalid("missing construction obligation"))?.deadline_tick;
        let child_deadline = plan.steps[child].obligation.as_ref()
            .ok_or_else(|| invalid("missing production obligation"))?.deadline_tick;
        assert!(child_deadline > parent_deadline);
    }
    let mut adapter = MemoryAdapter::new(source);
    let prepared = adapter.prepare(&plan, &context(adapter.snapshot()))?;
    let committed = adapter.commit(&plan, &prepared, &context(adapter.snapshot()))?;
    for index in [4, 5] {
        assert_eq!(committed.actions[index].state, CommitState::Prepared);
        let id = effects::created_entity_id(&plan.steps[index].idempotency_key, 0);
        assert!(!adapter.snapshot().graph.entities.contains_key(&id));
    }
    adapter.advance_ticks(effects::BUILD_TICKS)?;
    for action in &committed.actions {
        adapter.poll_action(action.action_id, &context(adapter.snapshot()))?;
    }
    assert_eq!(
        PredicateEvidence::laboratory(adapter.snapshot())?.evaluate(&compiled.terminal)?,
        PredicateTruth::False
    );
    adapter.advance_ticks(2 * effects::WORK_ORDER_TICKS_PER_UNIT)?;
    for action in &committed.actions {
        assert_eq!(
            adapter.poll_action(action.action_id, &context(adapter.snapshot()))?.state,
            CommitState::Verified
        );
    }
    assert_eq!(
        PredicateEvidence::laboratory(adapter.snapshot())?.evaluate(&compiled.terminal)?,
        PredicateTruth::True
    );
    assert_eq!(
        production_fact(&adapter.snapshot().graph.entities[&EntityId::new(1001)],
            "labor.MINE", adapter.snapshot().tick),
        Some(&Value::Bool(true))
    );
    Ok(())
}

#[test]
fn generated_setup_requires_each_original_action_capability_before_effects() -> Result<()> {
    let source = setup_world()?;
    let compiled = ProductionRequest::parse(&setup_json().to_string())?.compile(&source)?;
    let plan = seal(&source, &compiled)?;
    for missing in [
        Capability::ConfigureLabor,
        Capability::Construct,
        Capability::ConfigureProduction,
        Capability::Checkpoint,
    ] {
        let mut adapter = MemoryAdapter::new(source.clone());
        let mut ctx = context(adapter.snapshot());
        ctx.grants.retain(|grant| grant.capability != missing);
        assert!(adapter.prepare(&plan, &ctx).is_err(), "{missing:?}");
        assert_eq!(adapter.snapshot().anchor(), source.anchor());
    }
    Ok(())
}

#[test]
fn ready_capacity_is_reused_without_new_setup_or_source_options_loss() -> Result<()> {
    let source = scenario_snapshot("starter_fortress", FortressId::new(732), false)?;
    let request = ProductionRequest::parse(&setup_json().to_string())?;
    let compiled = request.compile(&source)?;
    let mut legacy = setup_json();
    legacy.as_object_mut().ok_or_else(|| invalid("fixture object missing"))?
        .remove("prerequisites");
    let unchanged = ProductionRequest::parse(&legacy.to_string())?.compile(&source)?;
    assert_eq!(compiled.actions, unchanged.actions);
    assert_eq!(compiled.terminal, unchanged.terminal);
    assert_eq!(compiled.analysis["prerequisites"]["steps_added"], 0);
    assert_eq!(setup_row(&compiled, "workshops", "BREW_DRINK")?["entity_id"], "7001");
    assert_eq!(setup_row(&compiled, "staffing", "BREW_DRINK")?["unit"], "1003");
    assert_eq!(setup_row(&compiled, "staffing", "PREPARE_MEAL")?["unit"], "1006");
    assert!(request.canonical_json().contains("workshops"));
    Ok(())
}

#[test]
fn existing_specialist_matching_avoids_unnecessary_labor_changes() -> Result<()> {
    let mut source = scenario_snapshot("starter_fortress", FortressId::new(733), false)?;
    for id in 1001..=1007 {
        set_field(&mut source, id, "labor.BREW", Value::Bool(false))?;
        set_field(&mut source, id, "labor.COOK", Value::Bool(false))?;
    }
    set_field(&mut source, 1001, "labor.BREW", Value::Bool(true))?;
    set_field(&mut source, 1001, "labor.COOK", Value::Bool(true))?;
    set_field(&mut source, 1002, "labor.BREW", Value::Bool(true))?;
    let compiled = ProductionRequest::parse(&setup_json().to_string())?.compile(&source)?;
    assert_eq!(compiled.analysis["prerequisites"]["steps_added"], 0);
    assert_eq!(setup_row(&compiled, "staffing", "BREW_DRINK")?["unit"], "1002");
    assert_eq!(setup_row(&compiled, "staffing", "PREPARE_MEAL")?["unit"], "1001");
    Ok(())
}

#[test]
fn optional_staffing_can_add_distinct_capacity_without_disabling_existing_labor() -> Result<()> {
    let mut source = scenario_snapshot("starter_fortress", FortressId::new(734), false)?;
    for id in 1001..=1007 {
        set_field(&mut source, id, "labor.BREW", Value::Bool(false))?;
        set_field(&mut source, id, "labor.COOK", Value::Bool(false))?;
    }
    set_field(&mut source, 1001, "labor.BREW", Value::Bool(true))?;
    set_field(&mut source, 1001, "labor.COOK", Value::Bool(true))?;
    let compiled = ProductionRequest::parse(&setup_json().to_string())?.compile(&source)?;
    let steps = parse_steps(&compiled.actions)?;
    assert_eq!(steps.len(), 3);
    assert!(matches!(&steps[0].action, Action::SetLabor { units, labor, enabled: true }
        if units == &vec![EntityId::new(1002)] && labor == "BREW"));
    assert!(steps[1].depends_on.is_empty());
    assert_eq!(steps[2].depends_on, vec![0]);
    let mut without_staffing = setup_json();
    without_staffing["prerequisites"]["assign_labor"] = json!(false);
    let serial = ProductionRequest::parse(&without_staffing.to_string())?.compile(&source)?;
    assert_eq!(parse_steps(&serial.actions)?.len(), 2);
    assert_eq!(serial.analysis["prerequisites"]["staffing"][1]["reused_across_jobs"], true);
    assert_eq!(parse_steps(&serial.actions)?[1].depends_on, vec![0]);
    Ok(())
}


#[test]
fn joint_staffing_preserves_scarce_current_and_enableable_roles_before_military_preferences()
-> Result<()> {
    // In either orientation, only A can serve the scarce role. B's other
    // field is unknown, so it must never be read as disabled. Even when A
    // already serves the common role, distinct capacity requires moving that
    // role's planned allocation to B without disabling A's existing labor.
    for scarce in ["BREW", "COOK"] {
        let common = if scarce == "BREW" { "COOK" } else { "BREW" };
        for (common_enabled, scarce_enabled) in [
            (false, true),
            (true, true),
            (false, false),
            (true, false),
        ] {
            let mut source =
                scenario_snapshot("starter_fortress", FortressId::new(738), false)?;
            for id in 1003..=1007 {
                set_field(&mut source, id, "alive", Value::Bool(false))?;
            }
            set_field(&mut source, 1001, &format!("labor.{common}"), Value::Bool(common_enabled))?;
            set_field(&mut source, 1001, &format!("labor.{scarce}"), Value::Bool(scarce_enabled))?;
            set_field(&mut source, 1001, effects::SQUAD_FIELD, Value::Null)?;
            set_field(&mut source, 1002, &format!("labor.{common}"), Value::Bool(false))?;
            set_field(&mut source, 1002, effects::SQUAD_FIELD,
                Value::Entity(starter::MILITIA_SQUAD))?;
            source.graph.entities.get_mut(&EntityId::new(1002))
                .ok_or_else(|| invalid("fixture worker missing"))?
                .fields.remove(&format!("labor.{scarce}"));
            source.refresh_hash();
            let compiled = ProductionRequest::parse(&setup_json().to_string())?.compile(&source)?;
            let job = |labor| if labor == "BREW" { "BREW_DRINK" } else { "PREPARE_MEAL" };
            assert_eq!(setup_row(&compiled, "staffing", job(scarce))?["unit"], "1001");
            assert_eq!(setup_row(&compiled, "staffing", job(common))?["unit"], "1002");
            assert_eq!(
                setup_row(&compiled, "staffing", job(common))?["reused_across_jobs"],
                false
            );
            let expected_changes = if scarce_enabled { 1 } else { 2 };
            assert_eq!(
                parse_steps(&compiled.actions)?.iter()
                    .filter(|step| matches!(step.action, Action::SetLabor { .. }))
                    .count(),
                expected_changes
            );
            assert_eq!(
                production_fact(&source.graph.entities[&EntityId::new(1001)],
                    &format!("labor.{common}"), source.tick),
                Some(&Value::Bool(common_enabled))
            );
        }
    }
    Ok(())
}

#[test]
fn one_known_worker_can_receive_two_independent_labor_prerequisites() -> Result<()> {
    let mut source = setup_world()?;
    for id in 1002..=1007 {
        set_field(&mut source, id, "alive", Value::Bool(false))?;
    }
    let compiled = ProductionRequest::parse(&setup_json().to_string())?.compile(&source)?;
    assert_eq!(compiled.analysis["prerequisites"]["staffing"][0]["unit"], "1001");
    assert_eq!(compiled.analysis["prerequisites"]["staffing"][1]["unit"], "1001");
    assert_eq!(compiled.analysis["prerequisites"]["staffing"][1]["reused_across_jobs"], true);
    assert_eq!(parse_steps(&compiled.actions)?.len(), 6);
    Ok(())
}

#[test]
fn one_worker_serializes_unequal_orders_and_retains_the_short_orders_full_deadline()
-> Result<()> {
    let mut source =
        scenario_snapshot("starter_fortress", FortressId::new(739), false)?;
    for id in 1002..=1007 {
        set_field(&mut source, id, "alive", Value::Bool(false))?;
    }
    set_field(&mut source, 1001, "labor.COOK", Value::Bool(true))?;
    set_field(&mut source, 1001, "labor.BREW", Value::Bool(true))?;
    let request = ProductionRequest::parse(&json!({
        "template":"production",
        "quotas":[{"item":"DRINK","minimum":45},{"item":"FOOD","minimum":110}],
        "prerequisites":{"assign_labor":false},
    }).to_string())?;
    let compiled = request.compile(&source)?;
    let steps = parse_steps(&compiled.actions)?;
    assert_eq!(steps.len(), 2);
    assert!(matches!(&steps[0].action, Action::CreateWorkOrder {
        job_token, amount: 10, ..
    } if job_token == "PREPARE_MEAL"));
    assert!(matches!(&steps[1].action, Action::CreateWorkOrder {
        job_token, amount: 1, ..
    } if job_token == "BREW_DRINK"));
    assert!(steps[0].depends_on.is_empty());
    assert_eq!(steps[1].depends_on, vec![0]);
    assert_eq!(
        setup_row(&compiled, "staffing", "BREW_DRINK")?["serial_after_job"],
        "PREPARE_MEAL"
    );
    let plan = seal(&source, &compiled)?;
    let first_deadline = plan.steps[0].obligation.as_ref()
        .ok_or_else(|| invalid("missing first order obligation"))?.deadline_tick;
    let short_deadline = plan.steps[1].obligation.as_ref()
        .ok_or_else(|| invalid("missing short order obligation"))?.deadline_tick;
    assert!(short_deadline > first_deadline);
    let isolated = effects::default_obligation(
        &plan.steps[1].action,
        &plan.steps[1].idempotency_key,
        source.fortress_id,
        source.tick,
    )?.ok_or_else(|| invalid("missing isolated short obligation"))?;
    assert!(isolated.deadline_tick.0 < source.tick.0 + 500);
    let original = source.clone();
    let mut adapter = MemoryAdapter::new(source);
    let prepared = adapter.prepare(&plan, &context(adapter.snapshot()))?;
    let committed = adapter.commit(&plan, &prepared, &context(adapter.snapshot()))?;
    let first = committed.actions[0].action_id;
    let short = committed.actions[1].action_id;
    let short_entity = effects::created_entity_id(&plan.steps[1].idempotency_key, 0);
    assert_eq!(committed.actions[1].state, CommitState::Prepared);
    assert!(!adapter.snapshot().graph.entities.contains_key(&short_entity));
    adapter.advance_ticks(500)?;
    // Physical completion alone cannot dispatch a dependent. Its own complete
    // horizon survives waiting beyond what its isolated deadline would allow.
    assert_eq!(
        adapter.poll_action(short, &context(adapter.snapshot()))?.state,
        CommitState::Prepared
    );
    assert!(!adapter.snapshot().graph.entities.contains_key(&short_entity));
    assert_eq!(
        adapter.poll_action(first, &context(adapter.snapshot()))?.state,
        CommitState::Verified
    );
    assert_eq!(
        adapter.poll_action(short, &context(adapter.snapshot()))?.state,
        CommitState::AppliedAwaitingVerification
    );
    adapter.advance_ticks(50)?;
    assert_eq!(
        adapter.poll_action(short, &context(adapter.snapshot()))?.state,
        CommitState::Verified
    );
    assert_eq!(
        PredicateEvidence::laboratory(adapter.snapshot())?.evaluate(&compiled.terminal)?,
        PredicateTruth::True
    );

    // The opt-in source preserves the original behavior and identity of older
    // requests that did not authorize prerequisite lowering.
    let mut raw: Json = serde_json::from_str(&request.canonical_json())
        .map_err(|error| invalid(error.to_string()))?;
    raw.as_object_mut().ok_or_else(|| invalid("fixture object missing"))?
        .remove("prerequisites");
    let old = ProductionRequest::parse(&raw.to_string())?.compile(&original)?;
    assert!(parse_steps(&old.actions)?.iter().all(|step| step.depends_on.is_empty()));
    Ok(())
}

#[test]
fn staffing_prefers_known_nonmilitary_without_changing_assignments() -> Result<()> {
    let mut source = setup_world()?;
    set_field(&mut source, 1001, effects::SQUAD_FIELD, Value::Entity(starter::MILITIA_SQUAD))?;
    set_field(&mut source, 1002, effects::SQUAD_FIELD, Value::Null)?;
    let before_squad = source.graph.entities[&EntityId::new(1001)].fields[effects::SQUAD_FIELD].clone();
    let compiled = ProductionRequest::parse(&setup_json().to_string())?.compile(&source)?;
    assert_eq!(compiled.analysis["prerequisites"]["staffing"][0]["unit"], "1002");
    assert_eq!(source.graph.entities[&EntityId::new(1001)].fields[effects::SQUAD_FIELD], before_squad);
    assert!(parse_steps(&compiled.actions)?.iter().all(|step|
        !matches!(step.action, Action::AssignSquad { .. })));
    Ok(())
}

#[test]
fn missing_or_unavailable_labor_never_becomes_false_for_setup_or_compensation() -> Result<()> {
    let mut base = setup_world()?;
    for id in 1002..=1007 {
        set_field(&mut base, id, "alive", Value::Bool(false))?;
    }
    let mut variants = vec![None];
    let original = base.graph.entities[&EntityId::new(1001)].fields["labor.BREW"].clone();
    for source in [
        FactSource::Replay,
        FactSource::AgentAssertion("not established".to_owned()),
        FactSource::Derived("unregistered forecast".to_owned()),
    ] {
        let mut fact = original.clone();
        fact.source = source;
        variants.push(Some(fact));
    }
    for presence in [
        FactPresence::Absent,
        FactPresence::Unknown("not captured".to_owned()),
        FactPresence::Omitted("not requested".to_owned()),
        FactPresence::Stale(base.anchor()),
    ] {
        let mut fact = original.clone();
        fact.presence = Some(presence);
        variants.push(Some(fact));
    }
    let mut future = original;
    future.observed_at = GameTick(u64::MAX);
    variants.push(Some(future));
    for replacement in variants {
        let mut source = base.clone();
        let unit = source.graph.entities.get_mut(&EntityId::new(1001))
            .ok_or_else(|| invalid("fixture worker missing"))?;
        match replacement {
            Some(fact) => { unit.fields.insert("labor.BREW".to_owned(), fact); }
            None => { unit.fields.remove("labor.BREW"); }
        }
        source.refresh_hash();
        assert!(ProductionRequest::parse(&setup_json().to_string())?
            .compile(&source)
            .is_err_and(|error| error.code == ErrorCode::PreconditionsFailed));
    }
    Ok(())
}

#[test]
fn setup_requires_explicit_known_life_and_ready_workshop_evidence() -> Result<()> {
    for life in [None, Some(lab_fact(Value::Bool(false))), {
        let mut fact = lab_fact(Value::Bool(true));
        fact.source = FactSource::AgentAssertion("alive?".to_owned());
        Some(fact)
    }] {
        let mut source = setup_world()?;
        for unit in source.graph.entities.values_mut().filter(|unit| unit.kind == EntityKind::Unit) {
            match &life {
                Some(fact) => { unit.fields.insert("alive".to_owned(), fact.clone()); }
                None => { unit.fields.remove("alive"); }
            }
        }
        source.refresh_hash();
        assert!(ProductionRequest::parse(&setup_json().to_string())?.compile(&source).is_err());
    }
    let mut source = scenario_snapshot("starter_fortress", FortressId::new(736), false)?;
    let building = source.graph.entities.get_mut(&starter::STILL)
        .ok_or_else(|| invalid("fixture still missing"))?;
    let fact = building.fields.get_mut(effects::CONSTRUCTION_STAGE_FIELD)
        .ok_or_else(|| invalid("fixture construction stage missing"))?;
    fact.source = FactSource::AgentAssertion("complete?".to_owned());
    source.refresh_hash();
    let compiled = ProductionRequest::parse(&setup_json().to_string())?.compile(&source)?;
    assert_eq!(setup_row(&compiled, "workshops", "BREW_DRINK")?["evidence"], "planned");
    assert_eq!(compiled.analysis["prerequisites"]["steps_added"], 1);
    Ok(())
}

#[test]
fn construction_refuses_unobserved_hazardous_occupied_and_unknown_geometry_sites() -> Result<()> {
    let base = setup_world()?;
    for at in [
        MapCoord::new(0, 0, 10),  // footprint must be floor
        MapCoord::new(-1, 0, 10), // known halo hazard
        MapCoord::new(0, 0, 9),  // support must be established
    ] {
        let mut source = base.clone();
        source.set_tile_code(at, tile_codes::MAGMA_WALL)?;
        source.refresh_hash();
        assert!(ProductionRequest::parse(&setup_json().to_string())?.compile(&source).is_err());
    }
    let mut missing = base.clone();
    missing.graph.chunks.remove(&ChunkCoord { x: -1, y: 0, z: 10 });
    missing.refresh_hash();
    assert!(ProductionRequest::parse(&setup_json().to_string())?.compile(&missing).is_err());
    for geometry in [true, false] {
        let mut source = base.clone();
        let fields = if geometry {
            vec![("footprint_min", Value::Coord(MapCoord::new(0,0,10))),
                ("footprint_max", Value::Coord(MapCoord::new(0,0,10)))]
        } else {
            Vec::new()
        };
        source.graph.entities.insert(EntityId::new(8801),
            record(EntityId::new(8801), EntityKind::Building, "existing", fields));
        source.refresh_hash();
        assert!(ProductionRequest::parse(&setup_json().to_string())?.compile(&source).is_err());
    }
    Ok(())
}

#[test]
fn replay_reuses_newly_ready_setup_and_recomputes_batches_from_original_goal() -> Result<()> {
    let mut source = setup_world()?;
    let request = ProductionRequest::parse(&setup_json().to_string())?;
    let first = request.compile(&source)?;
    let steps = parse_steps(&first.actions)?;
    for (index, step) in steps.iter().take(4).enumerate() {
        effects::apply_effect(&mut source, &step.action, &format!("setup-replay-{index}"))?;
    }
    source.tick = GameTick(source.tick.0 + effects::BUILD_TICKS);
    effects::advance_effects(&mut source, effects::BUILD_TICKS)?;
    set_field(&mut source, starter::STOCK_LEDGER.get(), effects::STOCK_DRINK_FIELD, Value::U64(45))?;
    source.refresh_hash();
    let replayed = ProductionRequest::parse(&request.canonical_json())?.compile(&source)?;
    assert_eq!(replayed.terminal, first.terminal);
    assert_eq!(replayed.analysis["prerequisites"]["steps_added"], 0);
    let replay_steps = parse_steps(&replayed.actions)?;
    assert_eq!(replay_steps.len(), 2);
    assert!(replay_steps.iter().any(|step| matches!(&step.action,
        Action::CreateWorkOrder { job_token, amount: 1, .. } if job_token == "BREW_DRINK")));
    assert!(replay_steps.iter().any(|step| matches!(&step.action,
        Action::CreateWorkOrder { job_token, amount: 2, .. } if job_token == "PREPARE_MEAL")));
    assert!(request.canonical_json().contains("workshop:Still"));
    assert!(request.canonical_json().contains("workshop:Kitchen"));
    Ok(())
}

#[test]
fn absent_setup_and_absent_sites_keep_refusals_explicit() -> Result<()> {
    let source = setup_world()?;
    for options in [None, Some(json!({"assign_labor":true})), Some(json!({"workshops":[]}))] {
        let mut raw = setup_json();
        match options {
            Some(options) => raw["prerequisites"] = options,
            None => {
                raw.as_object_mut().ok_or_else(|| invalid("fixture object missing"))?
                    .remove("prerequisites");
            }
        }
        assert!(ProductionRequest::parse(&raw.to_string())?.compile(&source).is_err());
    }
    let ready = scenario_snapshot("starter_fortress", FortressId::new(737), false)?;
    let mut request = setup_json();
    request["prerequisites"]["workshops"][0]["location"] = json!([1000,1000,10]);
    let request = ProductionRequest::parse(&request.to_string())?;
    let compiled = request.compile(&ready)?;
    assert_eq!(compiled.analysis["prerequisites"]["steps_added"], 0);
    assert!(request.canonical_json().contains("1000"));
    Ok(())
}
