//! Passive whole-set custody, currentness and no-publication regressions.
//! Owner: df-fastmcp-conformance-5pj.3.1.

use super::*;
use crate::agent_facade as facade;
use dfmcp_intent::effects;
use dfmcp_world::{Fact, FactSource, Value as WorldValue};

type TestResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;
const HORIZON: u64 = 1_234;

fn parsed(raw: &str) -> TestResult<Value> {
    Ok(serde_json::from_str(raw)?)
}

fn field(value: &Value, name: &str) -> TestResult<String> {
    Ok(value[name]
        .as_str()
        .ok_or_else(|| format!("missing {name}: {value}"))?
        .to_owned())
}

fn open(selector: &str, durable: bool) -> TestResult<String> {
    let value = parsed(&open_session_in_scenario(
        Some(false),
        Some(selector.to_owned()),
        Some(
            [
                ("observe", "read_only"),
                ("query", "read_only"),
                ("plan", "reversible"),
                ("control_clock", "reversible"),
                ("configure_labor", "reversible"),
                ("configure_production", "reversible"),
            ]
            .into_iter()
            .map(|(capability, risk)| (capability.to_owned(), risk.to_owned()))
            .collect(),
        ),
        Some(60_000),
        Some(4_000),
        Some(32_768),
        Some(4 * 1024 * 1024),
        Some(65_536),
        Some(64),
        Some("starter_fortress".to_owned()),
        Some(false),
        Some(durable),
    ))?;
    assert_eq!(value["ok"], true, "{value}");
    field(&value, "session_id")
}

fn prepare(session: &str, actions: Option<Value>, source: Option<Value>) -> TestResult<String> {
    let value = parsed(&facade::fortress_plan(
        Some(session.to_owned()),
        None,
        None,
        actions.map(|actions| actions.to_string()),
        source.map(|source| source.to_string()),
    ))?;
    assert_eq!(value["ok"], true, "{value}");
    field(&value, "plan_digest")
}

fn commit(session: &str, actions: Option<Value>, source: Option<Value>) -> TestResult<String> {
    let digest = prepare(session, actions, source)?;
    let value = parsed(&facade::fortress_commit(
        Some(session.to_owned()),
        digest.clone(),
    ))?;
    assert_eq!(value["ok"], true, "{value}");
    Ok(digest)
}

fn production(session: &str, item: &str, minimum: u32, horizon: Option<u64>) -> TestResult<String> {
    let mut source = json!({"template":"production","quotas":[{"item":item,"minimum":minimum}]});
    if let Some(horizon) = horizon {
        source["reserve_until_tick"] = json!(horizon);
    }
    commit(session, None, Some(source))
}

fn order(name: &str, amount: u32) -> Value {
    json!({"kind":"create_work_order","name":name,"job_token":"BREW_DRINK","amount":amount})
}

fn wait(session: &str, ticks: u64) -> TestResult<Value> {
    let value = parsed(&facade::fortress_wait(
        Some(session.to_owned()),
        Some(ticks),
    ))?;
    assert_eq!(value["ok"], true, "{value}");
    Ok(value)
}

fn reach(session: &str, tick: u64) -> TestResult {
    let now = with_task_read(session, |session| Ok(session.adapter.snapshot().tick.0))?;
    wait(
        session,
        tick.checked_sub(now)
            .ok_or("fixture cannot move backwards")?,
    )?;
    Ok(())
}

fn capture(session: &str, digests: &[String]) -> TestResult<Value> {
    let value = parsed(&facade::fortress_wait_for_plans(
        Some(session.to_owned()),
        Some(0),
        digests.to_vec(),
        Some("all".to_owned()),
    ))?;
    assert_eq!(value["ok"], true, "{value}");
    Ok(value)
}

fn member<'a>(value: &'a Value, digest: &str) -> TestResult<&'a Value> {
    value["selected_plan_set_wait"]["plans"]
        .as_array()
        .and_then(|plans| plans.iter().find(|plan| plan["plan_digest"] == digest))
        .ok_or_else(|| format!("selected original missing: {digest}").into())
}

fn state(session: &str) -> TestResult<Value> {
    Ok(with_task_read(session, |session| {
        Ok(json!({
            "anchor":anchor_json(&session.adapter.snapshot().anchor()),
            "next_request":session.next_request_id,
            "receipts":session.adapter.retained_action_ids().map(|id| {
                (id.to_string(), format!("{:?}", session.adapter.action_receipt(id)))
            }).collect::<Vec<_>>(),
            "commits":session.commit_receipts,
            "open":session.open_actions.iter().map(ToString::to_string).collect::<Vec<_>>(),
            "last":session.last_action.map(|id| id.to_string()),
            "last_plan":session.last_plan_actions.iter().map(ToString::to_string).collect::<Vec<_>>(),
            "pending":session.pending.as_ref().map(|pending| &pending.digest),
            "carried":session.carried.iter().map(CarriedStep::to_json).collect::<Vec<_>>(),
            "history":session.history.keys().map(|digest| digest.to_hex()).collect::<Vec<_>>(),
            "endpoints":session.objectives.iter().map(|original| json!({
                "plan_digest":original.plan_digest.to_hex(),
                "endpoint":original.selected_wait_reserve_observation.map(|(anchor, truth)|
                    json!({"anchor":anchor_json(&anchor),"truth":format!("{truth:?}")})),
                "durable":original.selected_wait_reserve_durable,
            })).collect::<Vec<_>>(),
        }))
    })?)
}

fn set_drink(session: &str, amount: Option<u64>) -> TestResult {
    with_task_session(session, |session| {
        let mut current = session.adapter.snapshot().clone();
        let ledger = effects::stock_ledger(&current).ok_or_else(session_error)?;
        let stock = current
            .graph
            .entities
            .get_mut(&ledger)
            .ok_or_else(session_error)?;
        if let Some(amount) = amount {
            stock.fields.insert(
                effects::STOCK_DRINK_FIELD.to_owned(),
                Fact::known(
                    WorldValue::U64(amount),
                    current.tick,
                    FactSource::Replay,
                    Digest32::of_bytes(b"passive-set-observer-stock-fixture"),
                ),
            );
        } else {
            stock.fields.remove(effects::STOCK_DRINK_FIELD);
        }
        stock.revision += 1;
        current.cursor = current.cursor.checked_next().ok_or_else(session_error)?;
        current.refresh_hash();
        session.adapter.inject_snapshot(current)
    })?;
    Ok(())
}

struct StateDir(std::path::PathBuf);
impl StateDir {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "dfmcp-plan-set-observer-{label}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        simulate_durable_restart(Some(path.clone()));
        Self(path)
    }
    fn restart(&self) {
        simulate_durable_restart(Some(self.0.clone()));
    }
}
impl Drop for StateDir {
    fn drop(&mut self) {
        simulate_durable_restart(None);
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn exact_horizon_stays_pending_without_capture_polling_or_dispatching() -> TestResult {
    let _serial = crate::test_serial();
    let session = open("7872201", false)?;
    let first = production(&session, "DRINK", 40, Some(HORIZON))?;
    let second = production(&session, "FOOD", 65, Some(HORIZON))?;
    reach(&session, HORIZON)?;
    commit(
        &session,
        Some(json!([
            {"action":{"kind":"pause","paused":false}},
            {"action":order("passive set deferred peer",20),"depends_on":[0]},
        ])),
        None,
    )?;
    let pending = prepare(
        &session,
        Some(json!([{"action":{"kind":"pause","paused":true}}])),
        None,
    )?;
    with_task_session(&session, |session| {
        assert!(session.last_plan_actions.iter().any(|id| {
            session
                .adapter
                .action_receipt(*id)
                .is_some_and(|receipt| receipt.state == CommitState::Prepared)
        }));
        session
            .grants
            .retain(|grant| matches!(grant.capability, Capability::Observe | Capability::Query));
        Ok(())
    })?;
    let requested = vec![second, first];
    let before = state(&session)?;
    validate_observation_set(&session, &requested, "all")?;
    let observed = observation_set_view(&session, &requested, "all")?;
    assert_eq!(
        observed.status,
        McpTaskStatus::Working,
        "{}",
        observed.payload
    );
    assert_eq!(observed.payload["plan_digests"], json!(requested));
    assert_eq!(
        observed.payload["selected_plan_set_wait"]["reserve_capture_required"],
        true
    );
    assert_eq!(
        observed.payload["next_step"]["arguments"]["max_game_ticks"],
        0
    );
    assert_eq!(
        observed.payload["selected_plan_set_wait"]["endpoint_publication"]["mode"],
        "retained_evidence_only"
    );
    assert_eq!(
        observed.payload["selected_plan_set_wait"]["endpoint_publication"]["endpoints_captured"],
        false
    );
    assert_eq!(state(&session)?, before);
    let captured = capture(&session, &requested)?;
    assert_eq!(captured["wait_progress"]["deferred_actions_dispatched"], 0);
    assert_eq!(
        observation_set_view(&session, &requested, "all")?.status,
        McpTaskStatus::Completed
    );
    let after = state(&session)?;
    assert_eq!(after["anchor"], before["anchor"]);
    assert_eq!(after["receipts"], before["receipts"]);
    assert_eq!(after["pending"], pending);
    assert_eq!(after["last_plan"], before["last_plan"]);
    Ok(())
}

#[test]
fn any_completion_keeps_active_peer_work_and_rechecks_current_goal_truth() -> TestResult {
    let _serial = crate::test_serial();
    let session = open("7872202", false)?;
    let complete = production(&session, "DRINK", 40, None)?;
    let finished = parsed(&facade::fortress_wait_for_plan(
        Some(session.clone()),
        Some(1_000),
        Some(complete.clone()),
    ))?;
    assert_eq!(
        finished["selected_plan_wait"]["complete"], true,
        "{finished}"
    );
    let active = commit(
        &session,
        Some(json!([{"action":order("active any-policy peer",500)}])),
        None,
    )?;
    let requested = vec![active.clone(), complete.clone()];
    let before = state(&session)?;
    let any = observation_set_view(&session, &requested, "any")?;
    assert_eq!(any.status, McpTaskStatus::Completed, "{}", any.payload);
    assert_eq!(any.payload["all_originals_complete"], false);
    assert_eq!(any.payload["physical_quiescent"], false);
    assert_eq!(any.payload["physical_work"]["state"], "active");
    assert_eq!(any.payload["cleanup_required"], false);
    assert!(
        any.payload["remaining_work"]
            .as_u64()
            .is_some_and(|count| count > 0)
    );
    assert!(
        any.payload["selected_plan_set_wait"]["unfinished_plan_digests"]
            .as_array()
            .is_some_and(|members| members.contains(&json!(active)))
    );
    assert_eq!(
        observation_set_view(&session, &requested, "all")?.status,
        McpTaskStatus::Working
    );
    assert_eq!(state(&session)?, before);
    set_drink(&session, Some(0))?;
    let current = observation_set_view(&session, &requested, "any")?;
    assert_eq!(current.status, McpTaskStatus::Failed, "{}", current.payload);
    assert_eq!(member(&current.payload, &complete)?["goal_truth"], "false");
    assert_eq!(current.payload["group_complete"], false);
    assert_eq!(
        current.payload["selected_plan_set_wait"]["earlier_completion_is_latched"],
        false
    );
    Ok(())
}

#[test]
fn one_passive_set_accepts_live_and_verified_recovered_originals() -> TestResult {
    let _serial = serialized_durable_tests();
    let directory = StateDir::new("mixed");
    let original_owner = open("7872203", true)?;
    let recovered = production(&original_owner, "DRINK", 40, Some(HORIZON))?;
    wait(&original_owner, 20)?;
    directory.restart();
    let session = open("7872203", true)?;
    let live = production(&session, "FOOD", 65, Some(HORIZON))?;
    let requested = vec![live.clone(), recovered.clone()];
    let before = state(&session)?;
    let observed = observation_set_view(&session, &requested, "all")?;
    assert_eq!(
        observed.status,
        McpTaskStatus::Working,
        "{}",
        observed.payload
    );
    assert_eq!(
        member(&observed.payload, &recovered)?["selection_origin"],
        "durable_recovery"
    );
    assert_eq!(
        member(&observed.payload, &recovered)?["original_owner_session_id"],
        original_owner
    );
    assert_eq!(
        member(&observed.payload, &recovered)?["dispatch_handles_restored"],
        false
    );
    assert_eq!(
        member(&observed.payload, &live)?["selection_origin"],
        "current_session_commit"
    );
    assert_eq!(observed.payload["owns_original_dispatch"], false);
    assert_eq!(state(&session)?, before);
    let finished = parsed(&facade::fortress_wait_for_plans(
        Some(session.clone()),
        Some(2_000),
        requested.clone(),
        None,
    ))?;
    assert_eq!(
        finished["selected_plan_set_wait"]["group_complete"], true,
        "{finished}"
    );
    assert_eq!(
        observation_set_view(&session, &requested, "all")?.status,
        McpTaskStatus::Completed
    );
    Ok(())
}

#[test]
fn passive_set_cannot_hydrate_a_missing_archived_endpoint_snapshot() -> TestResult {
    let _serial = serialized_durable_tests();
    let _directory = StateDir::new("no-hydration");
    let session = open("7872204", true)?;
    let requested = vec![
        production(&session, "DRINK", 40, Some(HORIZON))?,
        production(&session, "FOOD", 65, Some(HORIZON))?,
    ];
    reach(&session, HORIZON)?;
    let captured = capture(&session, &requested)?;
    let endpoint =
        member(&captured, &requested[0])?["reserve_frontier"]["observed_anchor"]["state_hash"]
            .as_str()
            .and_then(Digest32::from_hex)
            .ok_or("endpoint missing")?;
    wait(&session, 1)?;
    {
        let handle = lookup_session_str(&session)?;
        let mut guard = handle.lock().map_err(|_| "fixture session poisoned")?;
        assert!(guard.history.remove(&endpoint).is_some());
    }
    let before = state(&session)?;
    let error = match observation_set_view(&session, &requested, "all") {
        Err(error) => error,
        Ok(_) => return Err("passive set hydrated archived proof".into()),
    };
    assert_eq!(error.code, ErrorCode::Conflict);
    assert!(error.message.contains("reserve snapshot is unavailable"));
    assert_eq!(state(&session)?, before);
    let loaded = capture(&session, &requested)?;
    assert_eq!(
        loaded["selected_plan_set_wait"]["group_complete"], true,
        "{loaded}"
    );
    assert_eq!(
        observation_set_view(&session, &requested, "all")?.status,
        McpTaskStatus::Completed
    );
    Ok(())
}

#[test]
fn complete_set_admission_refuses_missing_members_authority_and_shared_bounds() -> TestResult {
    let _serial = crate::test_serial();
    let session = open("7872205", false)?;
    let requested = vec![
        production(&session, "DRINK", 40, Some(HORIZON))?,
        production(&session, "FOOD", 65, Some(HORIZON))?,
    ];
    reach(&session, HORIZON)?;
    let (grants, budget) = with_task_read(&session, |session| {
        Ok((session.grants.clone(), session.budget))
    })?;
    for defect in 0..5 {
        with_task_session(&session, |session| {
            session.grants = grants.clone();
            session.budget = budget;
            match defect {
                0 => session
                    .grants
                    .retain(|grant| grant.capability != Capability::Observe),
                1 => session
                    .grants
                    .retain(|grant| grant.capability != Capability::Query),
                2 => session.budget.max_entities = 1,
                3 => session.budget.max_actions = 1,
                _ => session.budget.max_output_tokens = 1_500,
            }
            Ok(())
        })?;
        let before = state(&session)?;
        assert!(validate_observation_set(&session, &requested, "all").is_err());
        assert_eq!(state(&session)?, before);
    }
    with_task_session(&session, |session| {
        session.grants = grants;
        session.budget = budget;
        Ok(())
    })?;
    let before = state(&session)?;
    for invalid in [
        vec![],
        vec![requested[0].clone(), requested[0].clone()],
        vec![requested[0].clone(), "ff".repeat(32)],
    ] {
        assert!(validate_observation_set(&session, &invalid, "all").is_err());
        assert_eq!(state(&session)?, before);
    }
    let original = with_task_session(&session, |session| {
        session
            .commit_receipts
            .insert(requested[1].clone(), "{}".to_owned())
            .ok_or_else(session_error)
    })?;
    let before = state(&session)?;
    assert!(validate_observation_set(&session, &requested, "all").is_err());
    assert_eq!(state(&session)?, before);
    with_task_session(&session, |session| {
        session
            .commit_receipts
            .insert(requested[1].clone(), original);
        Ok(())
    })?;
    assert_eq!(
        observation_set_view(&session, &requested, "all")?.status,
        McpTaskStatus::Working
    );
    Ok(())
}

#[test]
fn passive_set_keeps_first_false_and_unknown_endpoints_immutable() -> TestResult {
    let _serial = crate::test_serial();
    for (selector, amount, expected) in [
        ("7872206", Some(0), "reserve_unmet"),
        ("7872207", None, "reserve_unknown"),
    ] {
        let session = open(selector, false)?;
        let requested = vec![
            production(&session, "DRINK", 40, Some(HORIZON))?,
            production(&session, "FOOD", 65, Some(HORIZON))?,
        ];
        reach(&session, HORIZON)?;
        set_drink(&session, amount)?;
        assert_eq!(
            observation_set_view(&session, &requested, "all")?.status,
            McpTaskStatus::Working
        );
        capture(&session, &requested)?;
        let first = observation_set_view(&session, &requested, "any")?;
        assert_eq!(first.status, McpTaskStatus::Failed, "{}", first.payload);
        assert_eq!(
            first.payload["goal_unconfirmed_after_work"],
            amount.is_none()
        );
        assert_eq!(
            member(&first.payload, &requested[0])?["stop_reason"],
            expected
        );
        let retained =
            member(&first.payload, &requested[0])?["reserve_frontier"]["observed_anchor"].clone();
        set_drink(&session, Some(200))?;
        let before = state(&session)?;
        let later = observation_set_view(&session, &requested, "any")?;
        assert_eq!(later.status, McpTaskStatus::Failed, "{}", later.payload);
        assert_eq!(member(&later.payload, &requested[0])?["goal_truth"], "true");
        assert_eq!(
            member(&later.payload, &requested[0])?["stop_reason"],
            expected
        );
        assert_eq!(
            member(&later.payload, &requested[0])?["reserve_frontier"]["observed_anchor"],
            retained
        );
        assert_eq!(state(&session)?, before);
    }
    Ok(())
}

#[test]
fn passive_set_guidance_requires_clock_scope_and_caps_the_proposed_span() -> TestResult {
    let _serial = crate::test_serial();
    let session = open("7872208", false)?;
    let requested = vec![
        production(&session, "DRINK", 40, Some(HORIZON))?,
        production(&session, "FOOD", 65, Some(HORIZON))?,
    ];
    with_task_session(&session, |session| {
        session.budget.max_game_ticks = 7;
        Ok(())
    })?;
    let view = observation_set_view(&session, &requested, "all")?;
    assert_eq!(view.payload["next_step"]["arguments"]["max_game_ticks"], 7);
    with_task_session(&session, |session| {
        session
            .grants
            .retain(|grant| grant.capability != Capability::ControlClock);
        Ok(())
    })?;
    let before = state(&session)?;
    let view = observation_set_view(&session, &requested, "all")?;
    assert_eq!(view.payload["next_step"]["arguments"]["max_game_ticks"], 0);
    assert_eq!(state(&session)?, before);
    Ok(())
}

#[test]
fn known_failed_original_keeps_action_proof_precedence_over_an_active_peer() -> TestResult {
    let _serial = crate::test_serial();
    let session = open("7872209", false)?;
    commit(
        &session,
        Some(json!([{"action":{
            "kind":"set_labor","units":["1003"],"labor":"BREW","enabled":false,
        }}])),
        None,
    )?;
    let failed = commit(
        &session,
        Some(json!([{"action":order("failed observed original",1)}])),
        None,
    )?;
    let deadline = with_task_read(&session, |session| {
        let action = session
            .last_plan_actions
            .first()
            .ok_or_else(session_error)?;
        session
            .adapter
            .action_step(*action)
            .and_then(|step| step.obligation.as_ref())
            .map(|obligation| obligation.deadline_tick.0)
            .ok_or_else(session_error)
    })?;
    reach(
        &session,
        deadline.checked_add(1).ok_or("fixture deadline overflow")?,
    )?;
    let active = commit(
        &session,
        Some(json!([{"action":order("nonterminal observed peer",1)}])),
        None,
    )?;
    let before = state(&session)?;
    let observed = observation_set_view(&session, &[active.clone(), failed.clone()], "any")?;
    assert_eq!(
        member(&observed.payload, &failed)?["stop_reason"],
        "failed",
        "{}",
        observed.payload
    );
    assert_eq!(
        member(&observed.payload, &active)?["actions_terminal"],
        false
    );
    assert_eq!(observed.status, McpTaskStatus::Failed);
    assert_eq!(observed.payload["action_proof_status"], "failed");
    assert_eq!(observed.payload["physical_quiescent"], false);
    assert_eq!(state(&session)?, before);
    Ok(())
}
