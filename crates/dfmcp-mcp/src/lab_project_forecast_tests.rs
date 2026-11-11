//! Complete-project forecasts exercise real registered compilers and physical work.
//! All outputs are predictions; the real adapter and original review stay unchanged.

use super::*;
use std::time::Instant;

use crate::lab_world::{scenario_snapshot, starter};
use dfmcp_core::{CapabilityGrant, CapabilityScope, FortressId, RequestId, SessionId, WorkBudget};
use dfmcp_world::{Fact, FactSource};

fn context(adapter: &MemoryAdapter) -> OperationContext {
    OperationContext {
        session_id: SessionId::new(1),
        request_id: RequestId::new(1),
        anchor: adapter.snapshot().anchor(),
        cancellation_requested: false,
        budget: WorkBudget {
            max_wall_millis: 60_000,
            max_game_ticks: 20_000,
            max_entities: 10_000,
            max_bytes: 4 * 1024 * 1024,
            max_actions: 128,
            max_output_tokens: 32_768,
            ..WorkBudget::default()
        },
        grants: [
            (Capability::Observe, RiskTier::ReadOnly),
            (Capability::Query, RiskTier::ReadOnly),
            (Capability::Plan, RiskTier::ReadOnly),
            (Capability::Designate, RiskTier::Guarded),
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
                fortress_id: Some(adapter.snapshot().fortress_id),
                ..CapabilityScope::default()
            },
            expires_at_tick: None,
            remaining_uses: None,
        })
        .collect(),
    }
}

#[test]
fn missed_absolute_reserve_is_not_rescued_by_later_project_stage() -> Result<()> {
    let mut adapter = fixture(false)?;
    adapter.advance_ticks(1_100)?;
    let source = ProjectSource::Production(ProductionRequest::parse_current(
        &json!({
            "template":"production",
            "quotas":[{"item":"DRINK","minimum":140},{"item":"FOOD","minimum":65}],
            "reserve_until_tick":1102,
            "acquire_resources":{"assign_labor":false},
        })
        .to_string(),
    )?);
    let current = context(&adapter);
    let plan = prepare(&adapter, &source, &current)?;
    let original_plan = plan.clone();
    let original = format!("{adapter:?}");
    let (report, work) = predict(&adapter, &source, &plan, &current, 5_000)?;
    assert_eq!(report["available"], true, "{report}");
    assert_eq!(report["predicted_complete"], false);
    assert_eq!(report["stop_reason"], "original_reserve_expired");
    assert_eq!(report["stages"].as_array().map(Vec::len), Some(1));
    assert_eq!(report["stages"][0]["predicted_stage_complete"], true);
    assert_eq!(report["reserve_forecast"]["available"], true);
    assert_eq!(report["reserve_forecast"]["predicted_tick"], 1102);
    assert_eq!(report["reserve_forecast"]["predicted_goal_truth"], "false");
    assert_eq!(
        report["reserve_forecast"]["changes_action_deadlines"],
        false
    );
    assert_eq!(work["source_forks"], 1);
    assert_eq!(plan, original_plan);
    assert_eq!(format!("{adapter:?}"), original);
    Ok(())
}

fn fixture(paused: bool) -> Result<MemoryAdapter> {
    Ok(MemoryAdapter::new(scenario_snapshot(
        "starter_fortress_resources",
        FortressId::new(103_701),
        paused,
    )?))
}

fn write_stock(snapshot: &mut WorldSnapshot, field: &str, quantity: u64) -> Result<()> {
    snapshot
        .graph
        .entities
        .get_mut(&starter::STOCK_LEDGER)
        .ok_or_else(|| invariant("project fixture ledger missing"))?
        .fields
        .insert(
            field.to_owned(),
            Fact::known(
                WorldValue::U64(quantity),
                snapshot.tick,
                FactSource::Derived("dfmcp.lab-scenario/1".to_owned()),
                Digest32::ZERO,
            ),
        );
    snapshot.refresh_hash();
    Ok(())
}

fn beds(amount: u32, assign_labor: bool) -> Result<ProjectSource> {
    Ok(ProjectSource::Production(ProductionRequest::parse_current(
        &json!({
            "template":"production",
            "quotas":[{"item":"BED","minimum":amount}],
            "acquire_resources":{"assign_labor":assign_labor},
        })
        .to_string(),
    )?))
}

fn prepare(
    adapter: &MemoryAdapter,
    source: &ProjectSource,
    current: &OperationContext,
) -> Result<PreparedPlan> {
    let id = IntentId::new(103_701);
    let (intent, _) = match source {
        ProjectSource::Production(request) => super::super::production_source::compile(
            request,
            id,
            adapter.snapshot(),
            "original project",
        )?,
        ProjectSource::Room(request) => request.compile(id, adapter.snapshot(), "original room")?,
        ProjectSource::ConnectedRoom(request) => {
            request.compile(id, adapter.snapshot(), "original connected room")?
        }
    };
    StaticPlanner::default().prepare_laboratory(adapter.snapshot(), &intent, current)
}

fn predict(
    adapter: &MemoryAdapter,
    source: &ProjectSource,
    plan: &PreparedPlan,
    current: &OperationContext,
    horizon: u64,
) -> Result<(Value, Value)> {
    let mut budget = ForecastBudget::new_project(current, Instant::now(), horizon)?;
    let report = forecast_project_bounded(adapter, source, plan, current, &mut budget)?;
    Ok((report, budget.work_json()))
}

fn stock(report: &Value, item: &str) -> Result<u64> {
    report["predicted_stock"]["stocks"]
        .as_array()
        .and_then(|rows| rows.iter().find(|row| row["item"] == item))
        .and_then(|row| row["quantity"].as_u64())
        .ok_or_else(|| invariant("predicted project stock missing"))
}

#[test]
fn complete_production_stops_acquisition_before_false_quota_then_produces_observed_yields()
-> Result<()> {
    let adapter = fixture(false)?;
    let source = beds(24, false)?;
    let current = context(&adapter);
    let plan = prepare(&adapter, &source, &current)?;
    let original_world = format!("{adapter:?}");
    let original_source = source.canonical_json();
    let original_plan = plan.clone();
    let legacy = super::super::plan_forecast::forecast_plan(&adapter, &plan, &current);
    assert_eq!(legacy["predicted_actions_complete"], true, "{legacy}");
    assert_eq!(legacy["predicted_goal_truth"], "false");
    assert_eq!(legacy["predicted_complete"], false);

    let (report, work) = predict(&adapter, &source, &plan, &current, 5_000)?;
    assert_eq!(report["available"], true, "{report}");
    assert_eq!(report["predicted_complete"], true, "{report}");
    assert_eq!(report["predicted_goal_truth"], "true");
    assert_eq!(report["predicted_physical_quiescent"], true);
    assert_eq!(report["stages"].as_array().map(Vec::len), Some(2));
    assert_eq!(report["stages"][0]["stage"], "acquire_resources");
    assert_eq!(report["stages"][0]["predicted_stage_complete"], true);
    assert_eq!(report["stages"][0]["predicted_goal_truth"], "false");
    assert_eq!(report["stages"][1]["stage"], "production");
    assert_eq!(
        report["stages"][1]["from_tick"],
        report["stages"][0]["predicted_frontier_tick"]
    );
    assert!(
        report["stages"][0]["predicted_frontier_tick"].as_u64()
            < legacy["predicted_frontier_tick"].as_u64()
    );
    assert_eq!(stock(&report, "WOOD")?, 0);
    assert_eq!(stock(&report, "BED")?, 24);
    assert_eq!(work["project_stages"], 2);
    assert_eq!(work["source_forks"], 2);
    assert_eq!(
        work["candidate_action_steps"],
        report["project_action_count"]
    );
    assert_eq!(work["simulation_slices"], report["simulation_slices"]);
    assert_eq!(report["effect_evidence"], false);
    assert_eq!(report["actual_game_ticks_advanced"], 0);
    assert_eq!(report["future_stage_plans_are_executable"], false);
    assert_eq!(format!("{adapter:?}"), original_world);
    assert_eq!(source.canonical_json(), original_source);
    assert_eq!(plan, original_plan);
    assert_eq!(
        super::super::plan_forecast::forecast_plan(&adapter, &plan, &current),
        legacy
    );
    Ok(())
}

fn room_value(stairs: bool) -> Value {
    json!({
        "template":"provisioned_room",
        "acquire_resources":{"assign_labor":false},
        "layout":{"template":"bedroom_cluster","origin":[2,6,if stairs {9} else {10}],
            "rooms":2,"room_size":[3,3]},
        "prerequisites":{"assign_labor":true,"workforce":"complete_project_v1",
            "workshops":[{"building":"workshop:Carpenters","location":[0,0,10]}]},
    })
}

#[test]
fn complete_room_and_cross_level_access_include_setup_and_exact_furniture_custody() -> Result<()> {
    for connected in [false, true] {
        let mut snapshot = scenario_snapshot(
            if connected {
                "starter_fortress_multilevel_resources"
            } else {
                "starter_fortress_resources"
            },
            FortressId::new(103_702),
            false,
        )?;
        snapshot.graph.entities.remove(&starter::CARPENTERS);
        write_stock(&mut snapshot, effects::STOCK_WOOD_FIELD, 0)?;
        let source = if connected {
            ProjectSource::ConnectedRoom(ConnectedRoomRequest::parse_current(
                &json!({
                    "template":"connected_room",
                    "planner":"connected_room_stairs_resources_v1",
                    "room":room_value(true),
                    "access":{"entry":[0,2,10],"region":{"min":[0,0,9],"max":[12,12,10]}},
                })
                .to_string(),
            )?)
        } else {
            ProjectSource::Room(ProvisionedRoomRequest::parse_current(
                &room_value(false).to_string(),
            )?)
        };
        let adapter = MemoryAdapter::new(snapshot);
        let original = format!("{adapter:?}");
        let current = context(&adapter);
        let plan = prepare(&adapter, &source, &current)?;
        let (report, work) = predict(&adapter, &source, &plan, &current, 5_000)?;
        assert_eq!(report["available"], true, "{report}");
        assert_eq!(report["predicted_complete"], true, "{report}");
        assert_eq!(report["predicted_goal_truth"], "true");
        assert_eq!(report["stages"].as_array().map(Vec::len), Some(2));
        assert_eq!(report["stages"][0]["stage"], "acquire_resources");
        assert_eq!(report["stages"][1]["stage"], "room");
        assert_eq!(report["project_max_risk"], RiskTier::Guarded.as_str());
        assert_eq!(report["project_requires_checkpoint"], true);
        assert!(
            report["project_action_count"]
                .as_u64()
                .is_some_and(|count| count > plan.steps.len() as u64)
        );
        assert_eq!(stock(&report, "WOOD")?, 0);
        assert_eq!(
            stock(&report, "BED")?,
            0,
            "furniture must be installed, not loose stock"
        );
        assert_eq!(work["source_forks"], 2);
        assert_eq!(format!("{adapter:?}"), original);
    }
    Ok(())
}

#[test]
fn future_reserve_does_not_stop_at_currently_true_quota_or_jump_before_production() -> Result<()> {
    let mut snapshot = fixture(false)?.snapshot().clone();
    write_stock(&mut snapshot, effects::STOCK_PLANT_FIELD, 0)?;
    let adapter = MemoryAdapter::new(snapshot);
    let source = ProjectSource::Production(ProductionRequest::parse_current(
        &json!({
            "template":"production",
            "quotas":[{"item":"DRINK","minimum":40},{"item":"FOOD","minimum":60}],
            "reserve_until_tick":2401,
            "acquire_resources":{"assign_labor":false},
        })
        .to_string(),
    )?);
    let original_source = source.canonical_json();
    let current = context(&adapter);
    let plan = prepare(&adapter, &source, &current)?;
    let original = format!("{adapter:?}");
    let (report, work) = predict(&adapter, &source, &plan, &current, 5_000)?;
    assert_eq!(report["predicted_complete"], true, "{report}");
    assert_eq!(report["stages"].as_array().map(Vec::len), Some(2));
    assert_eq!(report["stages"][0]["predicted_goal_truth"], "true");
    assert_eq!(report["stages"][1]["stage"], "production");
    assert!(
        report["stages"][1]["from_tick"]
            .as_u64()
            .is_some_and(|tick| tick < 2401)
    );
    assert_eq!(report["reserve_forecast"]["available"], true, "{report}");
    assert_eq!(report["reserve_forecast"]["predicted_tick"], 2401);
    assert_eq!(report["reserve_forecast"]["reserve_until_tick"], 2401);
    assert_eq!(report["reserve_forecast"]["predicted_goal_truth"], "true");
    assert_eq!(
        report["reserve_forecast"]["predicted_physical_quiescent"],
        true
    );
    assert_eq!(
        report["reserve_forecast"]["continuous_maintenance_proven"],
        false
    );
    assert_eq!(
        report["reserve_forecast"]["changes_action_deadlines"],
        false
    );
    assert_eq!(work["source_forks"], 3);
    assert_eq!(work["simulated_game_ticks"], 2400);
    assert_eq!(work["simulation_slices"], report["simulation_slices"]);
    assert_eq!(source.canonical_json(), original_source);
    assert_eq!(format!("{adapter:?}"), original);
    Ok(())
}

#[test]
fn paused_and_short_horizon_projects_remain_incomplete_without_second_stage() -> Result<()> {
    for paused in [true, false] {
        let adapter = fixture(paused)?;
        let source = beds(24, false)?;
        let current = context(&adapter);
        let plan = prepare(&adapter, &source, &current)?;
        let original = format!("{adapter:?}");
        let (report, work) = predict(
            &adapter,
            &source,
            &plan,
            &current,
            if paused { 5_000 } else { 1 },
        )?;
        assert_eq!(report["available"], true, "{report}");
        assert_eq!(report["predicted_complete"], false);
        assert_eq!(report["stages"].as_array().map(Vec::len), Some(1));
        assert_eq!(
            report["stop_reason"],
            if paused { "paused" } else { "common_horizon" }
        );
        assert_eq!(work["project_stages"], 1);
        assert_eq!(work["source_forks"], 1);
        assert_eq!(format!("{adapter:?}"), original);
    }
    Ok(())
}

#[test]
fn future_stage_authority_and_aggregate_actions_are_checked_before_simulation() -> Result<()> {
    let adapter = fixture(false)?;
    let source = beds(24, false)?;
    let current = context(&adapter);
    let plan = prepare(&adapter, &source, &current)?;
    let original = format!("{adapter:?}");

    let mut limited = current.clone();
    limited.budget.max_actions = plan.steps.len() as u32;
    let mut budget = ForecastBudget::new_project(&limited, Instant::now(), 5_000)?;
    assert!(matches!(
        forecast_project_bounded(&adapter, &source, &plan, &limited, &mut budget),
        Err(error) if error.code == ErrorCode::BudgetExceeded
    ));
    assert_eq!(budget.work_json()["source_forks"], 1);
    assert_eq!(budget.work_json()["project_stages"], 1);

    let mut denied = current.clone();
    denied
        .grants
        .retain(|grant| grant.capability != Capability::ConfigureProduction);
    let mut budget = ForecastBudget::new_project(&denied, Instant::now(), 5_000)?;
    let report = forecast_project_bounded(&adapter, &source, &plan, &denied, &mut budget)?;
    assert_eq!(report["available"], false, "{report}");
    assert_eq!(
        report["reason"]["code"],
        ErrorCode::CapabilityDenied.as_str()
    );
    assert_eq!(report["stages"].as_array().map(Vec::len), Some(1));
    assert_eq!(budget.work_json()["source_forks"], 1);
    assert_eq!(format!("{adapter:?}"), original);
    Ok(())
}

#[test]
fn semantic_comparison_keeps_original_goal_generation_and_excludes_setup_permissions() -> Result<()>
{
    let adapter = fixture(false)?;
    let source = beds(24, false)?;
    let alternative = beds(24, true)?;
    assert_eq!(
        source.semantic_key(adapter.snapshot())?,
        alternative.semantic_key(adapter.snapshot())?
    );
    assert_ne!(source.source_digest(), alternative.source_digest());
    assert_ne!(
        source.semantic_key(adapter.snapshot())?,
        beds(25, false)?.semantic_key(adapter.snapshot())?
    );

    let mut room = room_value(false);
    let original = ProjectSource::Room(ProvisionedRoomRequest::parse_current(&room.to_string())?);
    room["acquire_resources"]["assign_labor"] = json!(true);
    room["prerequisites"]["assign_labor"] = json!(false);
    let alternative =
        ProjectSource::Room(ProvisionedRoomRequest::parse_current(&room.to_string())?);
    assert_eq!(
        original.semantic_key(adapter.snapshot())?,
        alternative.semantic_key(adapter.snapshot())?
    );
    room["layout"]["origin"] = json!([3, 6, 10]);
    let shifted = ProjectSource::Room(ProvisionedRoomRequest::parse_current(&room.to_string())?);
    assert_ne!(
        original.semantic_key(adapter.snapshot())?,
        shifted.semantic_key(adapter.snapshot())?
    );

    let ledger = ProductionGoalLedger::capture(adapter.snapshot())?;
    let mut replaced = adapter.snapshot().clone();
    replaced
        .graph
        .entities
        .get_mut(&starter::STOCK_LEDGER)
        .ok_or_else(|| invariant("fixture ledger missing"))?
        .generation += 1;
    replaced.refresh_hash();
    assert_ne!(
        source.semantic_key(adapter.snapshot())?,
        source.semantic_key(&replaced)?
    );
    assert!(source.goal_truth(&replaced, ledger).is_err());
    assert_eq!(
        ProjectSource::parse_production(&source.canonical_json())?,
        source
    );
    assert_eq!(
        ProjectSource::parse_room(&original.canonical_json())?,
        original
    );
    Ok(())
}

#[test]
fn project_budgets_preserve_owner_and_refuse_more_than_four_stage_reviews() -> Result<()> {
    let adapter = fixture(false)?;
    let source = beds(24, false)?;
    let current = context(&adapter);
    let plan = prepare(&adapter, &source, &current)?;
    let mut budget = ForecastBudget::new_project(&current, Instant::now(), 5_000)?;
    budget.begin_project(adapter.snapshot(), &plan, &current)?;
    for _ in 1..MAX_PROJECT_STAGES {
        budget.admit_project_stage(adapter.snapshot(), &plan, &current)?;
    }
    assert_eq!(budget.work_json()["project_stages"], 4);
    assert!(matches!(
        budget.admit_project_stage(adapter.snapshot(), &plan, &current),
        Err(error) if error.code == ErrorCode::BudgetExceeded
    ));
    let mut foreign = current.clone();
    foreign.request_id = RequestId::new(2);
    assert!(matches!(budget.check(&foreign), Err(error) if error.code == ErrorCode::StaleAnchor));
    let mut denied = current.clone();
    denied
        .grants
        .retain(|grant| grant.capability != Capability::Query);
    assert!(matches!(
        ForecastBudget::new_project(&denied, Instant::now(), 5_000),
        Err(error) if error.code == ErrorCode::CapabilityDenied
    ));
    Ok(())
}

#[test]
fn later_verified_production_cannot_turn_a_false_original_reserve_into_project_completion()
-> Result<()> {
    let adapter = fixture(false)?;
    let source = ProjectSource::Production(ProductionRequest::parse_current(
        &json!({
            "template":"production",
            "quotas":[{"item":"DRINK","minimum":45}],
            "reserve_until_tick":2,
        })
        .to_string(),
    )?);
    let current = context(&adapter);
    let plan = prepare(&adapter, &source, &current)?;
    let original = format!("{adapter:?}");
    let (report, work) = predict(&adapter, &source, &plan, &current, 5_000)?;
    assert_eq!(report["available"], true, "{report}");
    assert_eq!(report["predicted_actions_complete"], true);
    assert_eq!(report["predicted_physical_quiescent"], true);
    assert_eq!(report["predicted_goal_truth"], "true");
    assert_eq!(report["predicted_complete"], false, "{report}");
    assert_eq!(report["predicted_completion_tick"], Value::Null);
    assert_eq!(report["stop_reason"], "original_reserve_expired");
    assert_eq!(report["reserve_forecast"]["available"], true);
    assert_eq!(report["reserve_forecast"]["predicted_tick"], 2);
    assert_eq!(report["reserve_forecast"]["predicted_goal_truth"], "false");
    assert_eq!(
        report["reserve_forecast"]["changes_action_deadlines"],
        false
    );
    assert_eq!(report["stages"].as_array().map(Vec::len), Some(1));
    assert_eq!(work["source_forks"], 1);
    assert_eq!(format!("{adapter:?}"), original);
    Ok(())
}

#[test]
fn unobserved_future_reserve_stays_incomplete_and_exact_verified_endpoint_can_complete()
-> Result<()> {
    let adapter = fixture(false)?;
    let ordinary = ProjectSource::Production(ProductionRequest::parse_current(
        r#"{"template":"production","quotas":[{"item":"DRINK","minimum":45}]}"#,
    )?);
    let current = context(&adapter);
    let ordinary_plan = prepare(&adapter, &ordinary, &current)?;
    let (ordinary_report, _) = predict(&adapter, &ordinary, &ordinary_plan, &current, 5_000)?;
    let completion_tick = ordinary_report["predicted_completion_tick"]
        .as_u64()
        .ok_or_else(|| invariant("ordinary fixture did not finish"))?;
    let exact = ProjectSource::Production(ProductionRequest::parse_current(
        &json!({
            "template":"production",
            "quotas":[{"item":"DRINK","minimum":45}],
            "reserve_until_tick":completion_tick,
        })
        .to_string(),
    )?);
    let exact_plan = prepare(&adapter, &exact, &current)?;
    let (exact_report, _) = predict(&adapter, &exact, &exact_plan, &current, 5_000)?;
    assert_eq!(exact_report["predicted_complete"], true, "{exact_report}");
    assert_eq!(
        exact_report["reserve_forecast"]["predicted_tick"],
        completion_tick
    );
    assert_eq!(
        exact_report["reserve_forecast"]["predicted_goal_truth"],
        "true"
    );

    let distant = ProjectSource::Production(ProductionRequest::parse_current(
        r#"{"template":"production","quotas":[{"item":"DRINK","minimum":45}],"reserve_until_tick":2401}"#,
    )?);
    let distant_plan = prepare(&adapter, &distant, &current)?;
    let (distant_report, _) = predict(&adapter, &distant, &distant_plan, &current, 1_000)?;
    assert_eq!(
        distant_report["predicted_actions_complete"], true,
        "{distant_report}"
    );
    assert_eq!(distant_report["predicted_goal_truth"], "true");
    assert_eq!(distant_report["predicted_physical_quiescent"], true);
    assert_eq!(distant_report["reserve_forecast"]["available"], false);
    assert_eq!(distant_report["predicted_complete"], false);
    assert_eq!(distant_report["predicted_completion_tick"], Value::Null);
    assert_eq!(
        distant_report["stop_reason"],
        "original_reserve_unavailable"
    );
    Ok(())
}

#[test]
fn true_exact_reserve_can_precede_later_stable_verification() -> Result<()> {
    let adapter = fixture(false)?;
    let until = adapter.snapshot().tick.0 + effects::WORK_ORDER_TICKS_PER_UNIT;
    let request = ProductionRequest::parse_current(
        &json!({
            "template":"production",
            "quotas":[{"item":"DRINK","minimum":45}],
            "reserve_until_tick":until,
        })
        .to_string(),
    )?;
    let source = ProjectSource::Production(request.clone());
    let current = context(&adapter);
    let (mut intent, _) = super::super::production_source::compile(
        &request,
        IntentId::new(103_703),
        adapter.snapshot(),
        "strict stable observation",
    )?;
    // Keep the real compiler actions and semantic goal. A stricter reviewed
    // proof must wait for a second observation after the material already exists.
    let obligation = intent
        .requested_actions
        .iter_mut()
        .find_map(|step| step.obligation.as_mut())
        .ok_or_else(|| invariant("production fixture obligation missing"))?;
    obligation.stable_for_observations = 2;
    let plan = StaticPlanner::default().prepare_laboratory(adapter.snapshot(), &intent, &current)?;
    let original = format!("{adapter:?}");
    let (report, _) = predict(&adapter, &source, &plan, &current, 5_000)?;
    assert_eq!(report["available"], true, "{report}");
    assert_eq!(report["predicted_complete"], true, "{report}");
    assert!(
        report["predicted_completion_tick"]
            .as_u64()
            .is_some_and(|tick| tick > until)
    );
    assert_eq!(report["predicted_actions_complete"], true);
    assert_eq!(report["predicted_physical_quiescent"], true);
    assert_eq!(report["reserve_forecast"]["predicted_tick"], until);
    assert_eq!(report["reserve_forecast"]["predicted_goal_truth"], "true");
    assert_eq!(report["reserve_forecast"]["changes_action_deadlines"], false);
    assert_eq!(format!("{adapter:?}"), original);
    Ok(())
}
