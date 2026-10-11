//! Consumption-aware generation regressions execute the real compiler,
//! StaticPlanner, MemoryAdapter and finite-capacity reference timeline.

use super::*;
use dfmcp_adapter::GameAdapter;
use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, CommitState, IntentId, OperationContext,
    RequestId, RiskTier, SessionId, WorkBudget,
};
use dfmcp_intent::{Constraint, Intent, StaticPlanner};
use dfmcp_lab::MemoryAdapter;
use dfmcp_world::{FactPresence, PredicateEvidence, PredicateTruth};

fn world_at(tick: u64) -> Result<WorldSnapshot> {
    let mut snapshot = scenario_snapshot("starter_fortress", FortressId::new(953), false)?;
    let elapsed = tick - snapshot.tick.0;
    snapshot.tick = GameTick(tick);
    effects::advance_effects(&mut snapshot, elapsed)?;
    snapshot.refresh_hash();
    Ok(snapshot)
}

fn context(snapshot: &WorldSnapshot) -> OperationContext {
    OperationContext {
        session_id: SessionId::new(1),
        request_id: RequestId::new(1),
        anchor: snapshot.anchor(),
        budget: WorkBudget::default(),
        cancellation_requested: false,
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
            max_risk,
            scope: CapabilityScope {
                fortress_id: Some(snapshot.fortress_id),
                ..CapabilityScope::default()
            },
            expires_at_tick: None,
            remaining_uses: None,
        })
        .collect(),
    }
}

fn intent(snapshot: &WorldSnapshot, compiled: &ProductionCompilation, id: u128) -> Result<Intent> {
    Ok(Intent {
        id: IntentId::new(id),
        anchor: snapshot.anchor(),
        summary: "reach every original reserve after production and consumption".to_owned(),
        terminal_condition: compiled.terminal.clone(),
        constraints: vec![Constraint::MaxRisk(RiskTier::Guarded)],
        requested_actions: parse_steps(&compiled.actions)?,
    })
}

fn seal(
    snapshot: &WorldSnapshot,
    compiled: &ProductionCompilation,
    id: u128,
) -> Result<PreparedPlan> {
    let mut intent = intent(snapshot, compiled, id)?;
    compiled.apply_capacity_horizon(&mut intent)?;
    StaticPlanner::default().prepare_laboratory(snapshot, &intent, &context(snapshot))
}

fn execute(
    source: WorldSnapshot,
    plan: &PreparedPlan,
) -> Result<(MemoryAdapter, Vec<CommitState>)> {
    let mut adapter = MemoryAdapter::new(source);
    let prepared = adapter.prepare(plan, &context(adapter.snapshot()))?;
    let receipt = adapter.commit(plan, &prepared, &context(adapter.snapshot()))?;
    let mut states: Vec<_> = receipt.actions.iter().map(|action| action.state).collect();
    let horizon = plan
        .steps
        .iter()
        .filter_map(|step| step.obligation.as_ref().map(|value| value.deadline_tick.0))
        .max()
        .unwrap_or(adapter.snapshot().tick.0);
    while adapter.snapshot().tick.0 < horizon && states.iter().any(|state| !state.is_terminal()) {
        let ticks = effects::DEFAULT_POLL_INTERVAL_TICKS.min(horizon - adapter.snapshot().tick.0);
        adapter.advance_ticks(ticks)?;
        for (action, state) in receipt.actions.iter().zip(&mut states) {
            if !state.is_terminal() {
                *state = adapter
                    .poll_action(action.action_id, &context(adapter.snapshot()))?
                    .state;
            }
        }
    }
    Ok((adapter, states))
}

fn stock(snapshot: &WorldSnapshot, field: &str) -> Result<u64> {
    match snapshot
        .graph
        .entities
        .get(&starter::STOCK_LEDGER)
        .and_then(|ledger| production_fact(ledger, field, snapshot.tick))
    {
        Some(Value::U64(value)) => Ok(*value),
        _ => Err(invalid("fixture stock missing")),
    }
}

fn write(snapshot: &mut WorldSnapshot, id: EntityId, field: &str, value: Value) -> Result<()> {
    snapshot
        .graph
        .entities
        .get_mut(&id)
        .ok_or_else(|| invalid("fixture entity missing"))?
        .fields
        .insert(field.to_owned(), lab_fact(value));
    snapshot.refresh_hash();
    Ok(())
}

fn one_worker(snapshot: &mut WorldSnapshot) {
    for unit in snapshot
        .graph
        .entities
        .values_mut()
        .filter(|unit| unit.kind == EntityKind::Unit)
    {
        for field in ["labor.BREW", "labor.COOK"] {
            unit.fields.insert(
                field.to_owned(),
                lab_fact(Value::Bool(unit.id == EntityId::new(starter::FIRST_DWARF))),
            );
        }
    }
    snapshot.refresh_hash();
}

fn request(quotas: Json) -> Result<ProductionRequest> {
    ProductionRequest::parse_current(&json!({"template":"production","quotas":quotas}).to_string())
}

fn assert_through_horizon(
    mut adapter: MemoryAdapter,
    source: &WorldSnapshot,
    compiled: &ProductionCompilation,
) -> Result<()> {
    assert_eq!(
        PredicateEvidence::laboratory(adapter.snapshot())?.evaluate(&compiled.terminal)?,
        PredicateTruth::True,
    );
    let horizon = compiled.analysis["consumption"]["planning_horizon_ticks"]
        .as_u64()
        .ok_or_else(|| invalid("fixture horizon missing"))?;
    let end = source.tick.0 + horizon;
    adapter.advance_ticks(end - adapter.snapshot().tick.0)?;
    assert_eq!(
        PredicateEvidence::laboratory(adapter.snapshot())?.evaluate(&compiled.terminal)?,
        PredicateTruth::True,
    );
    Ok(())
}

#[test]
fn consumption_generation_reaches_goal_that_legacy_completed_work_misses() -> Result<()> {
    let source = world_at(1100)?;
    let raw = r#"{"template":"production","quotas":[{"item":"DRINK","minimum":60}]}"#;
    let legacy = ProductionRequest::parse(raw)?.compile(&source)?;
    let modern_request = ProductionRequest::parse_current(raw)?;
    let modern = modern_request.compile(&source)?;
    assert_eq!(legacy.terminal, modern.terminal);
    assert_eq!(parse_steps(&legacy.actions)?.len(), 1);
    assert_eq!(modern.analysis["requirements"][0]["minimum_stock"], 60);
    assert_eq!(
        modern.analysis["requirements"][0]["planning_stock_target"],
        67
    );
    assert_eq!(
        modern.analysis["requirements"][0]["consumption_allowance"],
        7
    );
    let (old, old_states) = execute(source.clone(), &seal(&source, &legacy, 953)?)?;
    assert!(
        old_states
            .iter()
            .all(|state| *state == CommitState::Verified)
    );
    assert_eq!(stock(old.snapshot(), effects::STOCK_DRINK_FIELD)?, 53);
    assert_eq!(
        PredicateEvidence::laboratory(old.snapshot())?.evaluate(&legacy.terminal)?,
        PredicateTruth::False,
    );
    let (new, states) = execute(source.clone(), &seal(&source, &modern, 954)?)?;
    assert!(states.iter().all(|state| *state == CommitState::Verified));
    assert_eq!(stock(new.snapshot(), effects::STOCK_DRINK_FIELD)?, 63);
    assert_through_horizon(new, &source, &modern)?;
    assert_eq!(
        modern_request.planner(),
        Some(ProductionPlanner::ConsumptionAwareV1)
    );
    Ok(())
}

#[test]
fn initially_satisfied_quota_gets_its_own_consumption_reserve() -> Result<()> {
    let source = world_at(1151)?;
    let original = request(json!([
        {"item":"DRINK","minimum":40}, {"item":"FOOD","minimum":65}
    ]))?;
    let compiled = original.compile(&source)?;
    let steps = parse_steps(&compiled.actions)?;
    assert_eq!(steps.len(), 2);
    assert!(steps.iter().all(|step| step.depends_on.is_empty()));
    assert_eq!(compiled.analysis["requirements"][0]["minimum_stock"], 40);
    assert_eq!(
        compiled.analysis["requirements"][0]["planning_stock_target"],
        47
    );
    let plan = seal(&source, &compiled, 955)?;
    let (adapter, states) = execute(source.clone(), &plan)?;
    assert!(states.iter().all(|state| *state == CommitState::Verified));
    assert_through_horizon(adapter, &source, &compiled)?;
    Ok(())
}

#[test]
fn shared_worker_is_serialized_without_setup_permission_in_both_directions() -> Result<()> {
    for (drink, food) in [(140, 65), (45, 160)] {
        let mut source = world_at(1)?;
        one_worker(&mut source);
        let raw = json!({"template":"production","quotas":[
            {"item":"DRINK","minimum":drink}, {"item":"FOOD","minimum":food}
        ]})
        .to_string();
        let legacy = ProductionRequest::parse(&raw)?.compile(&source)?;
        let old_steps = parse_steps(&legacy.actions)?;
        assert!(old_steps.iter().all(|step| step.depends_on.is_empty()));
        // Choose an actual sealed identity whose native reference order puts
        // the long job first; the old small deadline then expires behind it.
        let mut old_plan = None;
        for id in 1..=100 {
            let plan = seal(&source, &legacy, id)?;
            let mut order = plan.steps.iter().collect::<Vec<_>>();
            order.sort_by_key(|step| effects::created_entity_id(&step.idempotency_key, 0));
            if matches!(&order[0].action, Action::CreateWorkOrder { amount, .. } if *amount == 20) {
                old_plan = Some(plan);
                break;
            }
        }
        let old_plan =
            old_plan.ok_or_else(|| invalid("no adversarial bounded fixture identity"))?;
        let (_, states) = execute(source.clone(), &old_plan)?;
        assert!(states.contains(&CommitState::Failed));

        let compiled = ProductionRequest::parse_current(&raw)?.compile(&source)?;
        let steps = parse_steps(&compiled.actions)?;
        assert_eq!(steps.len(), 2);
        assert_eq!(steps[1].depends_on, vec![0]);
        assert!(
            steps
                .iter()
                .all(|step| matches!(&step.action, Action::CreateWorkOrder { .. }))
        );
        assert_eq!(
            compiled.analysis["staffing"]["setup_actions_authorized"],
            false
        );
        let mut narrowed = context(&source);
        narrowed.grants.retain(|grant| {
            !matches!(
                grant.capability,
                Capability::ConfigureLabor | Capability::Construct
            )
        });
        let mut proposed = intent(&source, &compiled, 956)?;
        compiled.apply_capacity_horizon(&mut proposed)?;
        let plan = StaticPlanner::default().prepare_laboratory(&source, &proposed, &narrowed)?;
        let (adapter, states) = execute(source.clone(), &plan)?;
        assert!(states.iter().all(|state| *state == CommitState::Verified));
        assert_through_horizon(adapter, &source, &compiled)?;
    }
    Ok(())
}

#[test]
fn distinct_specialist_and_flexible_workers_keep_real_parallel_capacity() -> Result<()> {
    let mut source = world_at(1)?;
    one_worker(&mut source);
    write(
        &mut source,
        EntityId::new(starter::FIRST_DWARF + 1),
        "labor.BREW",
        Value::Bool(true),
    )?;
    let compiled = request(json!([
        {"item":"DRINK","minimum":50}, {"item":"FOOD","minimum":70}
    ]))?
    .compile(&source)?;
    let steps = parse_steps(&compiled.actions)?;
    assert!(steps.iter().all(|step| step.depends_on.is_empty()));
    let (adapter, states) = execute(source.clone(), &seal(&source, &compiled, 957)?)?;
    assert!(states.iter().all(|state| *state == CommitState::Verified));
    assert_eq!(adapter.snapshot().tick, GameTick(101));
    assert_through_horizon(adapter, &source, &compiled)?;
    Ok(())
}

#[test]
fn setup_time_reserves_both_resources_and_preserves_explicit_sites() -> Result<()> {
    let mut source = world_at(2151)?;
    source.graph.entities.remove(&starter::KITCHEN);
    source.refresh_hash();
    let raw = json!({"template":"production","quotas":[
        {"item":"DRINK","minimum":33},{"item":"FOOD","minimum":65}
    ],"prerequisites":{"assign_labor":false,"workshops":[
        {"building":"workshop:Kitchen","location":[2,0,10]},
        {"building":"workshop:Still","location":[0,0,10]}
    ]}})
    .to_string();
    let original = ProductionRequest::parse_current(&raw)?;
    let canonical = original.canonical_json();
    let compiled = original.compile(&source)?;
    assert_eq!(original.canonical_json(), canonical);
    assert_eq!(compiled.analysis["prerequisites"]["steps_added"], 1);
    // Construction's registered 500 service ticks receive a 4x default
    // obligation horizon. The whole plan therefore crosses two drink rounds
    // but only one meal round at this observed metabolism phase.
    assert_eq!(
        compiled.analysis["requirements"][0]["consumption_allowance"],
        14
    );
    assert_eq!(
        compiled.analysis["requirements"][1]["consumption_allowance"],
        7
    );
    let plan = seal(&source, &compiled, 958)?;
    assert!(
        plan.steps
            .iter()
            .any(|step| matches!(&step.action, Action::Build { .. }))
    );
    let (adapter, states) = execute(source.clone(), &plan)?;
    assert!(states.iter().all(|state| *state == CommitState::Verified));
    assert_through_horizon(adapter, &source, &compiled)?;
    let reopened = ProductionRequest::parse(&canonical)?.compile(&source)?;
    assert_eq!(seal(&source, &reopened, 958)?, plan);
    Ok(())
}

#[test]
fn queued_service_participates_in_consumption_fixed_point() -> Result<()> {
    let mut source = world_at(1)?;
    one_worker(&mut source);
    let id = EntityId::new(91);
    source.graph.entities.insert(
        id,
        record(
            id,
            EntityKind::WorkOrder,
            "old meal order",
            vec![
                (
                    effects::STATUS_FIELD,
                    Value::Text(effects::STATUS_ACTIVE.to_owned()),
                ),
                ("job_token", Value::Text("PREPARE_MEAL".to_owned())),
                (
                    effects::WORK_ORDER_NAME_FIELD,
                    Value::Text("old meal order".to_owned()),
                ),
                (
                    effects::WORK_ORDER_CONDITIONS_FIELD,
                    effects::work_order_conditions_value(&[])?,
                ),
                ("amount_total", Value::U64(20)),
                (effects::AMOUNT_REMAINING_FIELD, Value::U64(20)),
                ("work_ticks", Value::U64(0)),
            ],
        ),
    );
    source.refresh_hash();
    let compiled = request(json!([{"item":"DRINK","minimum":45}]))?.compile(&source)?;
    assert_eq!(
        compiled.analysis["capacity"]["queued_service_allowance_ticks"],
        2000
    );
    assert_eq!(
        compiled.analysis["requirements"][0]["consumption_allowance"],
        14
    );
    assert_eq!(
        compiled.analysis["consumption"]["planning_horizon_ticks"],
        2500
    );
    assert_eq!(compiled.analysis["consumption"]["fixed_point_rounds"], 3);
    let (adapter, states) = execute(source.clone(), &seal(&source, &compiled, 959)?)?;
    assert!(states.iter().all(|state| *state == CommitState::Verified));
    assert_through_horizon(adapter, &source, &compiled)?;
    Ok(())
}

#[test]
fn metabolism_phase_comes_from_its_observed_clock_and_counts_exact_boundaries() -> Result<()> {
    let mut source = world_at(1000)?;
    let original = request(json!([{"item":"DRINK","minimum":45}]))?;
    for (metabolism, expected) in [(0, 0), (999, 0), (1000, 7), (1199, 7)] {
        write(
            &mut source,
            starter::STOCK_LEDGER,
            effects::METABOLISM_TICKS_FIELD,
            Value::U64(metabolism),
        )?;
        let compiled = original.compile(&source)?;
        assert_eq!(
            compiled.analysis["requirements"][0]["consumption_allowance"],
            expected
        );
    }
    Ok(())
}

#[test]
fn unknown_asserted_future_or_malformed_population_and_metabolism_refuse_without_effects()
-> Result<()> {
    let original = world_at(1100)?;
    let request = request(json!([{"item":"DRINK","minimum":60}]))?;
    for (id, key) in [
        (EntityId::new(starter::FIRST_DWARF + 6), "alive"),
        (starter::STOCK_LEDGER, effects::METABOLISM_TICKS_FIELD),
    ] {
        for variant in 0..5 {
            let mut source = original.clone();
            let record = source
                .graph
                .entities
                .get_mut(&id)
                .ok_or_else(|| invalid("fixture entity missing"))?;
            if variant == 0 {
                record.fields.remove(key);
            } else {
                let fact = record
                    .fields
                    .get_mut(key)
                    .ok_or_else(|| invalid("fixture field missing"))?;
                match variant {
                    1 => fact.presence = Some(FactPresence::Unknown("not observed".to_owned())),
                    2 => fact.source = FactSource::AgentAssertion("guessed".to_owned()),
                    3 => fact.observed_at = GameTick(source.tick.0 + 1),
                    _ => {
                        fact.value = Value::Text("wrong type".to_owned());
                        fact.presence = None;
                    }
                }
            }
            source.refresh_hash();
            let before = source.clone();
            assert!(
                request
                    .compile(&source)
                    .is_err_and(|error| error.code == ErrorCode::PreconditionsFailed)
            );
            assert_eq!(source, before);
        }
    }
    Ok(())
}

#[test]
fn generation_replay_is_exact_and_legacy_source_is_never_upgraded_implicitly() -> Result<()> {
    let source = world_at(1100)?;
    let raw = r#"{"template":"production","quotas":[{"item":"DRINK","minimum":60}]}"#;
    let legacy = ProductionRequest::parse(raw)?;
    let canonical_legacy = legacy.canonical_json();
    assert_eq!(legacy.planner(), None);
    assert!(!canonical_legacy.contains("planner"));
    let old = seal(&source, &legacy.compile(&source)?, 960)?;
    let current = legacy
        .clone()
        .with_planner(ProductionPlanner::ConsumptionAwareV1);
    assert_eq!(current, ProductionRequest::parse_current(raw)?);
    let current_canonical = current.canonical_json();
    let modern = seal(&source, &current.compile(&source)?, 960)?;
    assert_ne!(old.digest, modern.digest);
    assert_eq!(
        seal(
            &source,
            &ProductionRequest::parse(&canonical_legacy)?.compile(&source)?,
            960
        )?,
        old,
    );
    assert_eq!(
        seal(
            &source,
            &ProductionRequest::parse(&current_canonical)?.compile(&source)?,
            960
        )?,
        modern,
    );
    assert_eq!(legacy.canonical_json(), canonical_legacy);
    for version in [
        json!("current"),
        json!("consumption_aware_v2"),
        json!({}),
        json!(1),
    ] {
        let mut input: Json =
            serde_json::from_str(raw).map_err(|error| invalid(error.to_string()))?;
        input["planner"] = version;
        assert!(ProductionRequest::parse(&input.to_string()).is_err());
    }
    Ok(())
}

#[test]
fn sealed_horizon_matches_reserve_and_refused_binding_leaves_intent_unchanged() -> Result<()> {
    let source = world_at(1100)?;
    let compiled = request(json!([{"item":"DRINK","minimum":60}]))?.compile(&source)?;
    let mut proposed = intent(&source, &compiled, 961)?;
    compiled.apply_capacity_horizon(&mut proposed)?;
    let plan =
        StaticPlanner::default().prepare_laboratory(&source, &proposed, &context(&source))?;
    let deadline = plan
        .steps
        .iter()
        .filter_map(|step| step.obligation.as_ref().map(|value| value.deadline_tick.0))
        .max();
    assert_eq!(
        deadline,
        compiled.analysis["consumption"]["planning_horizon_tick"].as_u64()
    );
    let mut unrelated = intent(&source, &compiled, 962)?;
    unrelated.requested_actions[0].action = Action::Pause { paused: true };
    let before = unrelated.clone();
    assert!(compiled.apply_capacity_horizon(&mut unrelated).is_err());
    assert_eq!(unrelated, before);
    Ok(())
}

#[test]
fn unsupported_setup_and_bounded_horizon_refuse_without_weakening_the_original_goal() -> Result<()>
{
    let source = world_at(1)?;
    for minimum in [100_000, u32::MAX] {
        let original = request(json!([{"item":"DRINK","minimum":minimum}]))?;
        let raw = original.canonical_json();
        assert!(
            original
                .compile(&source)
                .is_err_and(|error| error.code == ErrorCode::BudgetExceeded)
        );
        assert_eq!(original.canonical_json(), raw);
    }
    let mut clock_overflow = source.clone();
    write(
        &mut clock_overflow,
        starter::STOCK_LEDGER,
        effects::METABOLISM_TICKS_FIELD,
        Value::U64(u64::MAX),
    )?;
    assert!(
        request(json!([{"item":"DRINK","minimum":45}]))?
            .compile(&clock_overflow)
            .is_err_and(|error| error.code == ErrorCode::BudgetExceeded)
    );
    let mut slow_fixed_point = source.clone();
    for number in 0..52 {
        let id = EntityId::new(95_300 + number);
        slow_fixed_point.graph.entities.insert(
            id,
            record(
                id,
                EntityKind::Unit,
                "additional consumer",
                vec![("alive", Value::Bool(true))],
            ),
        );
    }
    slow_fixed_point.refresh_hash();
    let refusal = request(json!([{"item":"DRINK","minimum":200}]))?
        .compile(&slow_fixed_point)
        .err()
        .ok_or_else(|| invalid("fixed-point budget unexpectedly accepted"))?;
    assert_eq!(refusal.code, ErrorCode::BudgetExceeded);
    assert!(refusal.message.contains("64 bounded rounds"));
    assert!(refusal.message.contains("not that every possible"));
    let mut no_still = source.clone();
    no_still.graph.entities.remove(&starter::STILL);
    no_still.refresh_hash();
    assert!(
        request(json!([{"item":"DRINK","minimum":45}]))?
            .compile(&no_still)
            .is_err_and(|error| error.code == ErrorCode::PreconditionsFailed)
    );
    // A stock-only request does not acquire a perpetual maintenance horizon.
    assert!(
        request(json!([{"item":"DRINK","minimum":40},{"item":"FOOD","minimum":60}]))?
            .compile(&source)
            .is_err_and(|error| error.code == ErrorCode::InvalidIntent)
    );
    Ok(())
}
