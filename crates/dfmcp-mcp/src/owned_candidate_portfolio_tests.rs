//! Selected competing roots receive future stages on the candidate's real
//! shared model; their goals, custody and marginal costs remain independent.

use super::*;
use crate::agent_facade as facade;
use crate::lab_world::ProductionRequest;
use crate::server;
use crate::server::original_forecast::admit_complete_projects;
use dfmcp_core::{ErrorCode, GameTick};

type TestResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

struct Owner(String);
impl Drop for Owner {
    fn drop(&mut self) {
        if let Ok(id) = u128::from_str_radix(&self.0, 16) {
            server::sessions().remove(&SessionId::new(id));
        }
    }
}

fn parsed(raw: &str) -> TestResult<Value> {
    Ok(serde_json::from_str(raw)?)
}

fn open(selector: &str) -> TestResult<Owner> {
    open_kind(selector, false)
}

fn open_kind(selector: &str, durable: bool) -> TestResult<Owner> {
    let value = parsed(&facade::fortress_open_session(
        Some(false),
        Some(selector.to_owned()),
        Some(
            [
                ("observe", "read_only"),
                ("query", "read_only"),
                ("plan", "guarded"),
                ("configure_production", "reversible"),
                ("configure_labor", "reversible"),
                ("designate", "guarded"),
                ("construct", "guarded"),
                ("checkpoint", "guarded"),
                ("control_clock", "reversible"),
            ]
            .into_iter()
            .map(|(capability, risk)| (capability.to_owned(), risk.to_owned()))
            .collect(),
        ),
        Some(60_000),
        Some(8_000),
        Some(32_768),
        Some(16 * 1024 * 1024),
        Some(32_768),
        Some(64),
        Some("starter_fortress_resources".to_owned()),
        Some(false),
        Some(durable),
    ))?;
    assert_eq!(value["ok"], true, "{value}");
    Ok(Owner(
        value["session_id"]
            .as_str()
            .ok_or("session id missing")?
            .to_owned(),
    ))
}

fn view<T>(owner: &Owner, read: impl FnOnce(&LabSession) -> TestResult<T>) -> TestResult<T> {
    let handle = server::resolve_session(Some(owner.0.clone()))?;
    server::with_session_read(&handle, |error| Err(error.into()), read)
}

fn commit_peer(owner: &Owner) -> TestResult<Digest32> {
    commit_request(
        owner,
        json!({
            "template":"production", "quotas":[{"item":"BED","minimum":24}],
            "acquire_resources":{"assign_labor":false},
        }),
    )
}

fn commit_request(owner: &Owner, request: Value) -> TestResult<Digest32> {
    let planned = parsed(&facade::fortress_plan(
        Some(owner.0.clone()),
        None,
        None,
        None,
        Some(request.to_string()),
    ))?;
    assert_eq!(planned["ok"], true, "{planned}");
    let digest = planned["plan_digest"].as_str().ok_or("peer plan missing")?;
    let committed = parsed(&facade::fortress_commit(
        Some(owner.0.clone()),
        digest.to_owned(),
    ))?;
    assert_eq!(committed["ok"], true, "{committed}");
    Digest32::from_hex(digest).ok_or_else(|| "peer digest invalid".into())
}

fn candidate(
    session: &LabSession,
    current: &OperationContext,
) -> TestResult<(ProjectSource, PreparedPlan)> {
    candidate_item(session, current, "BED")
}

fn candidate_item(
    session: &LabSession,
    current: &OperationContext,
    item: &str,
) -> TestResult<(ProjectSource, PreparedPlan)> {
    let source = ProjectSource::Production(ProductionRequest::parse_current(
        &json!({
            "template":"production", "quotas":[{"item":item,"minimum":4}],
            "acquire_resources":{"assign_labor":false},
        })
        .to_string(),
    )?);
    let plan = source.prepare_joint_stage(
        Digest32::of_bytes(b"selected peer portfolio candidate"),
        1,
        session.adapter.snapshot(),
        current,
    )?;
    assert_eq!(source.stage_name(&plan), "production");
    Ok((source, plan))
}

fn state(session: &LabSession) -> Value {
    json!({
        "adapter":format!("{:?}", session.adapter),
        "pending":session.pending.as_ref().map(|pending| pending.plan.digest.to_hex()),
        "receipts":session.commit_receipts,
        "actions":format!("{:?}", session.open_actions),
        "carried":format!("{:?}", session.carried),
        "objectives":format!("{:?}", session.objectives),
        "history":session.history.len(),
    })
}

fn predict(
    session: &LabSession,
    selected: Digest32,
    source: &ProjectSource,
    plan: &PreparedPlan,
    current: &OperationContext,
    ticks: u64,
) -> Result<(Value, Value)> {
    let started = Instant::now();
    let owned = OwnedWork::admit(session, current, started)?;
    let peers = admit_complete_projects(session, &[selected], current, started)?;
    let mut budget = ForecastBudget::new_project(current, started, ticks)?;
    let result = owned.forecast_project_with_peers(
        &session.adapter,
        source,
        plan,
        current,
        source.reserve_until_tick(),
        started,
        &mut budget,
        &peers,
    )?;
    Ok((result, budget.work_json()))
}

#[test]
fn selected_peer_harvest_continues_into_production_without_borrowing_candidate_success()
-> TestResult {
    let _serial = crate::test_serial();
    let owner = open("8194501")?;
    let peer = commit_peer(&owner)?;
    view(&owner, |session| {
        let current = server::read_context(session);
        let (source, plan) = candidate(session, &current)?;
        let before = state(session);
        let started = Instant::now();
        let owned = OwnedWork::admit(session, &current, started)?;
        let mut alone_budget = ForecastBudget::new_project(&current, started, 4_000)?;
        let alone = owned.forecast_project(
            &session.adapter,
            &source,
            &plan,
            &current,
            None,
            started,
            &mut alone_budget,
        )?;
        assert_eq!(alone["predicted_complete"], true, "{alone}");
        assert!(alone["peer_projects"].is_null());

        let (portfolio, work) = predict(session, peer, &source, &plan, &current, 4_000)?;
        assert_eq!(portfolio["available"], true, "{portfolio}");
        assert_eq!(portfolio["predicted_complete"], true, "{portfolio}");
        assert_eq!(
            portfolio["predicted_portfolio_complete"], true,
            "{portfolio}"
        );
        assert_eq!(portfolio["portfolio_evidence_complete"], true);
        assert_eq!(portfolio["portfolio_goal_truth"], "true");
        let row = &portfolio["peer_projects"][0];
        assert_eq!(row["actual_plan_digest"], peer.to_hex());
        assert_eq!(row["requested_plan_digest"], peer.to_hex());
        assert_eq!(row["root_plan_digest"], peer.to_hex());
        assert_eq!(row["predicted_project_actions_complete"], true);
        assert_eq!(row["predicted_goal_truth"], "true");
        assert_eq!(row["future_stages"].as_array().map(Vec::len), Some(1));
        assert_eq!(row["future_stages"][0]["stage"], "production");
        assert_eq!(row["future_stages"][0]["executable_review"], false);
        assert!(row["steps"].as_array().is_some_and(|steps| {
            !steps.is_empty() && steps.iter().all(|step| step["action_id"].is_string())
        }));
        assert!(
            row["future_stages"][0]["steps"]
                .as_array()
                .is_some_and(|steps| {
                    !steps.is_empty()
                        && steps.iter().all(|step| {
                            step.get("action_id").is_none() && step["model_action_id"].is_string()
                        })
                })
        );
        let future = row["future_stages"][0]["action_count"]
            .as_u64()
            .ok_or("future peer actions missing")?;
        assert!(future > 0);
        assert_eq!(
            portfolio["portfolio_action_count"].as_u64(),
            portfolio["project_action_count"]
                .as_u64()
                .map(|count| count + future)
        );
        assert_eq!(
            portfolio["portfolio_cost_scope"],
            "candidate_and_hypothetical_peer_stages"
        );
        assert_eq!(work["hypothetical_competing_action_steps"], future);
        assert_eq!(work["hypothetical_peer_stages"], 1);
        assert_eq!(work["project_stages"], 2);
        assert_eq!(work["source_forks"], 2);
        assert_eq!(work["limits"]["source_forks"], 8);
        assert_eq!(
            portfolio["portfolio_completion_tick"],
            portfolio["predicted_frontier_tick"]
        );
        assert!(
            portfolio["portfolio_completion_tick"].as_u64()
                > alone["predicted_completion_tick"].as_u64()
        );
        assert_eq!(
            portfolio["predicted_actions_completion_tick"],
            alone["predicted_actions_completion_tick"]
        );
        assert_eq!(state(session), before);
        Ok(())
    })
}

#[test]
fn candidate_complete_at_horizon_does_not_complete_unfinished_selected_project() -> TestResult {
    let _serial = crate::test_serial();
    let owner = open("8194502")?;
    let peer = commit_peer(&owner)?;
    view(&owner, |session| {
        let current = server::read_context(session);
        let (source, plan) = candidate(session, &current)?;
        let before = state(session);
        let started = Instant::now();
        let owned = OwnedWork::admit(session, &current, started)?;
        let mut budget = ForecastBudget::new_project(&current, started, 4_000)?;
        let alone = owned.forecast_project(
            &session.adapter,
            &source,
            &plan,
            &current,
            None,
            started,
            &mut budget,
        )?;
        let horizon = alone["predicted_completion_tick"]
            .as_u64()
            .ok_or("candidate must finish")?
            - current.anchor.tick.0;
        assert!(horizon > 0);
        let (portfolio, _) = predict(session, peer, &source, &plan, &current, horizon)?;
        assert_eq!(portfolio["available"], true, "{portfolio}");
        assert_eq!(portfolio["predicted_complete"], true, "{portfolio}");
        assert_eq!(
            portfolio["predicted_portfolio_complete"], false,
            "{portfolio}"
        );
        assert_eq!(portfolio["peer_projects"][0]["predicted_complete"], false);
        assert_eq!(portfolio["portfolio_goal_truth"], "false");
        assert_eq!(portfolio["portfolio_evidence_complete"], true);
        assert!(portfolio["portfolio_completion_tick"].is_null());
        assert_eq!(portfolio["simulation_horizon_truncated"], true);
        assert_eq!(state(session), before);
        Ok(())
    })
}

#[test]
fn future_peer_effect_authority_precedes_aggregate_action_admission() -> TestResult {
    let _serial = crate::test_serial();
    let owner = open("8194503")?;
    let peer = commit_peer(&owner)?;
    view(&owner, |session| {
        let mut current = server::read_context(session);
        let (source, plan) = candidate(session, &current)?;
        let admitted = admit_complete_projects(session, &[peer], &current, Instant::now())?;
        current.budget.max_actions =
            u32::try_from(admitted.action_steps + plan.steps.len() as u64)?;
        for grant in &mut current.grants {
            if grant.capability == Capability::ConfigureProduction {
                grant.expires_at_tick = Some(GameTick(current.anchor.tick.0 + 1));
            }
        }
        let before = state(session);
        let (refused, work) = predict(session, peer, &source, &plan, &current, 4_000)?;
        assert_eq!(refused["available"], false, "{refused}");
        assert_eq!(refused["reason"]["code"], "capability_denied");
        assert_eq!(refused["predicted_portfolio_complete"], false);
        assert_eq!(refused["portfolio_evidence_complete"], false);
        assert_eq!(work["project_stages"], 1);
        assert_eq!(work["hypothetical_peer_stages"], 0);
        assert_eq!(work["hypothetical_competing_action_steps"], 0);
        assert_eq!(state(session), before);
        Ok(())
    })
}

#[test]
fn selected_peer_successors_share_action_and_generated_custody_limits_across_alternatives()
-> TestResult {
    let _serial = crate::test_serial();
    let owner = open("8194504")?;
    let peer = commit_peer(&owner)?;
    view(&owner, |session| {
        let wide = server::read_context(session);
        let (source, plan) = candidate(session, &wide)?;
        let before = state(session);
        let (complete, full) = predict(session, peer, &source, &plan, &wide, 4_000)?;
        assert_eq!(complete["predicted_portfolio_complete"], true, "{complete}");
        let total_actions = full["candidate_action_steps"]
            .as_u64()
            .ok_or("candidate actions")?
            + full["competing_action_steps"]
                .as_u64()
                .ok_or("competing actions")?;
        let total_records = full["forked_source_records"]
            .as_u64()
            .ok_or("world records")?
            + full["forked_retained_custody_records"]
                .as_u64()
                .ok_or("custody records")?
            + full["generated_project_custody_records"]
                .as_u64()
                .ok_or("future records")?;
        let future = full["hypothetical_competing_action_steps"]
            .as_u64()
            .ok_or("future actions")?;
        assert!(future > 0);
        assert!(2 * (total_actions - future) <= 2 * total_actions - 1);
        for limit in ["actions", "generated_custody"] {
            let mut current = wide.clone();
            if limit == "actions" {
                current.budget.max_actions = u32::try_from(2 * total_actions - 1)?;
            } else {
                current.budget.max_entities = u32::try_from(2 * total_records - 1)?;
            }
            let started = Instant::now();
            let owned = OwnedWork::admit(session, &current, started)?;
            let peers = admit_complete_projects(session, &[peer], &current, started)?;
            let mut budget = ForecastBudget::new_project(&current, started, 4_000)?;
            let first = owned.forecast_project_with_peers(
                &session.adapter,
                &source,
                &plan,
                &current,
                None,
                started,
                &mut budget,
                &peers,
            )?;
            assert_eq!(
                first["predicted_portfolio_complete"], true,
                "{limit}: {first}"
            );
            let failed = owned.forecast_project_with_peers(
                &session.adapter,
                &source,
                &plan,
                &current,
                None,
                started,
                &mut budget,
                &peers,
            );
            assert!(
                matches!(failed, Err(ref error) if error.code == ErrorCode::BudgetExceeded),
                "{limit}: {failed:?}"
            );
            let used = budget.work_json();
            assert_eq!(used["candidates"], 2);
            assert_eq!(used["source_forks"], 4);
            assert_eq!(used["hypothetical_peer_stages"], 1);
            assert_eq!(used["project_stages"], 3);
            assert_eq!(
                used["generated_project_custody_records"],
                full["generated_project_custody_records"]
            );
            assert_eq!(state(session), before);
        }
        Ok(())
    })
}

#[test]
fn unknown_exact_reserve_is_not_hidden_by_another_false_goal() -> Result<()> {
    let candidate = json!({
        "predicted_goal_truth":"false", "predicted_physical_quiescent":true,
        "reserve_until_tick":null, "reserve_frontier_truth":null, "retained_stages":[],
    });
    let peer = json!({
        "predicted_goal_truth":"true", "predicted_physical_quiescent":true,
        "reserve_until_tick":400, "reserve_frontier_truth":"true",
        "retained_stages":[{
            "predicted_goal_truth":"true", "predicted_physical_quiescent":true,
            "reserve_until_tick":400, "reserve_frontier_truth":"unknown",
        }],
    });
    assert!(portfolio::original_evidence_complete(&candidate)?);
    assert!(!portfolio::original_evidence_complete(&peer)?);
    Ok(())
}

struct StateDir(std::path::PathBuf);
impl StateDir {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "dfmcp-owned-portfolio-recovered-commit-{}",
            std::process::id(),
        ));
        let _ = std::fs::remove_dir_all(&path);
        server::simulate_durable_restart(Some(path.clone()));
        Self(path)
    }
    fn restart(&self) {
        server::simulate_durable_restart(Some(self.0.clone()));
    }
}
impl Drop for StateDir {
    fn drop(&mut self) {
        server::simulate_durable_restart(None);
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn selected_recovered_proof_verified_at_candidate_commit_survives_initial_admission() -> TestResult
{
    let _serial = crate::test_serial();
    let _durable = server::serialized_durable_tests();
    let directory = StateDir::new();
    let first = open_kind("8194505", true)?;
    let peer = commit_request(
        &first,
        json!({
            "template":"production", "quotas":[{"item":"BED","minimum":4}],
        }),
    )?;
    let advanced = parsed(&facade::fortress_wait(Some(first.0.clone()), Some(1)))?;
    assert_eq!(advanced["ok"], true, "{advanced}");
    directory.restart();
    drop(first);
    let owner = open_kind("8194505", true)?;
    // Set up an authentic quiet-but-unverified recovered tip. Advance its real
    // private fixture adapter and apply only the ordinary pure monitor
    // transition. Do not reconstruct monitors or edit streak/deadline fields.
    {
        let handle = server::resolve_session(Some(owner.0.clone()))?;
        let mut session = handle.lock().map_err(|_| "fixture session poisoned")?;
        assert!(session.shared.is_none());
        let index = session
            .carried
            .iter()
            .position(|row| {
                row.plan_digest == peer && row.state == "dispatched" && row.monitor.is_some()
            })
            .ok_or("active recovered production monitor missing")?;
        let step = session.carried[index]
            .work_step
            .clone()
            .ok_or("sealed step missing")?;
        let spec = step.obligation.as_ref().ok_or("obligation missing")?;
        let cadence = spec.poll_interval_ticks;
        assert!(cadence > 0 && spec.stable_for_observations > 0);
        for _ in 0..100 {
            if dfmcp_intent::inspect_effect_work(
                session.adapter.snapshot(),
                &step.action,
                &step.idempotency_key,
                true,
            )?
            .is_quiescent()
            {
                break;
            }
            session.adapter.advance_ticks(cadence)?;
        }
        assert!(
            dfmcp_intent::inspect_effect_work(
                session.adapter.snapshot(),
                &step.action,
                &step.idempotency_key,
                true,
            )?
            .is_quiescent()
        );
        let target = spec.stable_for_observations - 1;
        let streak = |row: &CarriedStep| -> TestResult<u32> {
            match row.monitor.as_ref().and_then(|monitor| monitor.status()) {
                Some(dfmcp_intent::ObligationStatus::Active {
                    consecutive_stable_observations,
                    ..
                }) => Ok(*consecutive_stable_observations),
                _ => Err("recovered fixture must retain an active original monitor".into()),
            }
        };
        for _ in 0..spec.stable_for_observations.saturating_add(2) {
            if streak(&session.carried[index])? == target {
                break;
            }
            let next =
                recovered_step_progress::observe(session.adapter.snapshot(), &session.carried)?;
            session.carried = next;
            assert_eq!(session.carried[index].state, "dispatched");
            if streak(&session.carried[index])? < target {
                session.adapter.advance_ticks(cadence)?;
            }
        }
        assert_eq!(streak(&session.carried[index])?, target);
        session.adapter.advance_ticks(cadence)?;
        assert_eq!(session.carried[index].state, "dispatched");
    }
    view(&owner, |session| {
        let before = state(session);
        let current = server::read_context(session);
        let (source, plan) = candidate_item(session, &current, "CHAIR")?;
        let (portfolio, work) = predict(session, peer, &source, &plan, &current, 0)?;
        assert_eq!(portfolio["available"], true, "{portfolio}");
        assert_eq!(portfolio["predicted_portfolio_complete"], false);
        assert_eq!(portfolio["recovered_peers_finished_at_candidate_commit"], 1);
        let original = &portfolio["peer_projects"][0];
        assert_eq!(original["selection_origin"], "durable_recovery");
        assert_eq!(original["predicted_complete"], true, "{original}");
        assert_eq!(original["dispatch_handles_restored"], false);
        let row = &original["steps"][0];
        assert_eq!(row["recorded_state"], "dispatched");
        assert_eq!(row["predicted_state"], "Verified");
        assert!(row["action_id"].is_null());
        assert_eq!(
            row["predicted_proof_anchor"]["game_tick"],
            current.anchor.tick.0
        );
        assert_eq!(row["predicted_proof_is_canonical"], false);
        assert_eq!(work["source_forks"], 2);
        assert_eq!(work["simulated_game_ticks"], 0);
        assert!(
            session
                .carried
                .iter()
                .any(|row| { row.plan_digest == peer && row.state == "dispatched" })
        );
        assert_eq!(state(session), before);
        Ok(())
    })
}
