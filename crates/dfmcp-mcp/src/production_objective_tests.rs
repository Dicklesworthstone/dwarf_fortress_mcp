//! Production plans retain original stock goals through effects and replay.
use super::*;
use crate::agent_facade as facade;
use crate::lab_world::{ProductionRequest, starter};
use dfmcp_intent::effects;
use dfmcp_world::{Fact, FactSource, Value as WorldValue};
use serde_json::Value as Json;

type TestResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn parsed(raw: &str) -> TestResult<Json> {
    Ok(serde_json::from_str(raw)?)
}

fn open(selector: &str) -> TestResult<String> {
    let opened = parsed(&facade::fortress_open_session(
        Some(false),
        Some(selector.to_owned()),
        Some(
            [
                ("observe", "read_only"),
                ("query", "read_only"),
                ("plan", "reversible"),
                ("control_clock", "reversible"),
                ("configure_production", "reversible"),
            ]
            .into_iter()
            .map(|(capability, risk)| (capability.to_owned(), risk.to_owned()))
            .collect(),
        ),
        None,
        Some(2_000),
        None,
        None,
        Some(8_192),
        Some(16),
        Some("starter_fortress".to_owned()),
        None,
        None,
    ))?;
    assert_eq!(opened["ok"], true, "{opened}");
    Ok(opened["session_id"]
        .as_str()
        .ok_or("session missing")?
        .to_owned())
}

fn wait(session: &str, ticks: u64) -> TestResult<Json> {
    let result = parsed(&facade::fortress_wait(
        Some(session.to_owned()),
        Some(ticks),
    ))?;
    assert_eq!(result["ok"], true, "{result}");
    Ok(result)
}

fn plan(session: &str, quotas: Json) -> TestResult<Json> {
    let result = parsed(&facade::fortress_plan(
        Some(session.to_owned()),
        None,
        None,
        None,
        Some(json!({"template":"production", "quotas":quotas}).to_string()),
    ))?;
    assert_eq!(result["ok"], true, "{result}");
    Ok(result)
}

fn commit(session: &str, planned: &Json) -> TestResult<Json> {
    parsed(&facade::fortress_commit(
        Some(session.to_owned()),
        planned["plan_digest"]
            .as_str()
            .ok_or("digest missing")?
            .to_owned(),
    ))
}

fn stock(session: &str, field: &str) -> TestResult<u64> {
    let session = resolve_session(Some(session.to_owned()))?;
    let guard = session.lock().map_err(|_| "session poisoned")?;
    let fact = guard
        .adapter
        .snapshot()
        .graph
        .entities
        .get(&starter::STOCK_LEDGER)
        .and_then(|ledger| ledger.fields.get(field))
        .ok_or("stock missing")?;
    match fact.known_value() {
        Some(WorldValue::U64(held)) => Ok(*held),
        _ => Err("stock unavailable".into()),
    }
}

#[test]
fn changed_anchor_requires_reviewed_production_replay_even_when_stock_is_unchanged() -> TestResult {
    let session = open("743021")?;
    let planned = plan(&session, json!([{"item":"DRINK","minimum":45}]))?;
    wait(&session, 1)?;
    let replayed = commit(&session, &planned)?;
    assert_eq!(replayed["ok"], false, "{replayed}");
    assert_eq!(replayed["rebase"]["method"], "intent_replay", "{replayed}");
    assert_ne!(
        replayed["rebased_plan"]["plan_digest"], planned["plan_digest"],
        "the new workload horizon must have its own reviewed seal",
    );
    assert_eq!(stock(&session, effects::STOCK_DRINK_FIELD)?, 40);
    let committed = commit(&session, &replayed["rebased_plan"])?;
    assert_eq!(committed["ok"], true, "{committed}");
    Ok(())
}

#[test]
fn production_reserves_consumption_until_the_original_goal_is_proved() -> TestResult {
    let session = open("743001")?;
    wait(&session, 1099)?; // Source tick 1100; metabolism starts at tick 1.
    let planned = plan(&session, json!([{"item":"DRINK","minimum":60}]))?;
    assert_eq!(
        planned["production"]["consumption"]["planner"],
        "consumption_aware_v1"
    );
    let requirement = &planned["production"]["requirements"][0];
    assert_eq!(requirement["minimum_stock"], 60);
    assert_eq!(requirement["planning_stock_target"], 67);
    assert_eq!(requirement["consumption_allowance"], 7);
    {
        let owner = resolve_session(Some(session.clone()))?;
        let guard = owner.lock().map_err(|_| "session poisoned")?;
        let pending = guard.pending.as_ref().ok_or("plan missing")?;
        let PlanSource::Production { raw, .. } = &pending.source else {
            return Err("original production source missing".into());
        };
        assert_eq!(parsed(raw)?["planner"], "consumption_aware_v1");
        assert!(matches!(
            &pending.plan.steps[0].action,
            Action::CreateWorkOrder { amount: 6, .. }
        ));
    }
    let committed = commit(&session, &planned)?;
    assert_eq!(committed["ok"], true, "{committed}");
    let settled = wait(&session, 300)?;
    assert_eq!(stock(&session, effects::STOCK_DRINK_FIELD)?, 63);
    assert_eq!(
        settled["polled_actions"][0]["state"], "verified",
        "{settled}"
    );
    assert_eq!(
        settled["objectives"][0]["predicate_truth"], "true",
        "{settled}"
    );
    assert_eq!(settled["objectives"][0]["status"], "achieved", "{settled}");
    assert!(settled["objectives"][0]["achieved_tick"].is_u64());
    assert_eq!(
        settled["agent_turn"]["briefing"]["objective_status"][0]["predicate_truth"],
        "true"
    );
    Ok(())
}

#[test]
fn joint_production_reserves_a_quota_already_satisfied_at_the_source() -> TestResult {
    let session = open("743002")?;
    wait(&session, 1150)?; // The first drink meal occurs at game tick 1201.
    let planned = plan(
        &session,
        json!([
            {"item":"DRINK","minimum":40}, {"item":"FOOD","minimum":65},
        ]),
    )?;
    assert_eq!(planned["steps"].as_array().map(Vec::len), Some(2));
    let drink = planned["production"]["requirements"]
        .as_array()
        .and_then(|rows| rows.iter().find(|row| row["item"] == "DRINK"))
        .ok_or("drink requirement missing")?;
    assert_eq!(drink["minimum_stock"], 40);
    assert_eq!(drink["planning_stock_target"], 47);
    let committed = commit(&session, &planned)?;
    assert_eq!(committed["ok"], true, "{committed}");
    let settled = wait(&session, 100)?;
    assert_eq!(stock(&session, effects::STOCK_FOOD_FIELD)?, 65);
    assert_eq!(stock(&session, effects::STOCK_DRINK_FIELD)?, 43);
    assert!(
        settled["polled_actions"]
            .as_array()
            .is_some_and(|actions| actions.iter().all(|action| action["state"] == "verified")),
        "{settled}"
    );
    assert_eq!(
        settled["objectives"][0]["predicate_truth"], "true",
        "{settled}"
    );
    assert!(settled["objectives"][0]["achieved_tick"].is_u64());
    Ok(())
}

#[test]
fn stale_production_plan_replays_the_original_quota_at_changed_stock() -> TestResult {
    let session = open("743003")?;
    wait(&session, 1099)?;
    let original = plan(&session, json!([{"item":"DRINK","minimum":60}]))?;
    wait(&session, 101)?;
    assert_eq!(stock(&session, effects::STOCK_DRINK_FIELD)?, 33);
    let stale = commit(&session, &original)?;
    assert_eq!(stale["ok"], false, "{stale}");
    assert_eq!(stale["error"]["code"], "stale_anchor", "{stale}");
    assert_eq!(stale["rebase"]["method"], "intent_replay", "{stale}");
    assert_eq!(
        stale["rebased_plan"]["production"]["requirements"][0]["stock"], 33,
        "{stale}"
    );
    assert_eq!(
        stale["rebased_plan"]["production"]["requirements"][0]["planned"], 30,
        "{stale}"
    );
    {
        let owner = resolve_session(Some(session.clone()))?;
        let guard = owner.lock().map_err(|_| "session poisoned")?;
        let pending = guard.pending.as_ref().ok_or("replayed plan missing")?;
        assert!(matches!(&pending.source, PlanSource::Production { .. }));
        assert!(matches!(
            &pending.plan.steps[0].action,
            Action::CreateWorkOrder { amount: 6, .. }
        ));
        assert!(
            crate::witness::ReadWitness::of(&pending.plan).to_json()["entities"]
                .as_array()
                .is_some_and(|ids| ids.contains(&json!(starter::STOCK_LEDGER.get().to_string())))
        );
    }
    let committed = commit(&session, &stale["rebased_plan"])?;
    assert_eq!(committed["ok"], true, "{committed}");
    let settled = wait(&session, 300)?;
    assert_eq!(stock(&session, effects::STOCK_DRINK_FIELD)?, 63);
    assert_eq!(
        settled["objectives"][0]["predicate_truth"], "true",
        "{settled}"
    );
    Ok(())
}

#[test]
fn production_alias_conflict_is_refused_before_session_resolution() -> TestResult {
    let raw = r#"{"template":"production","quotas":[{"item":"DRINK","minimum":60}]}"#;
    let result = parsed(&plan_request(
        None,
        None,
        None,
        None,
        Some(raw.to_owned()),
        Some(raw.to_owned()),
    ))?;
    assert_eq!(result["ok"], false);
    assert_eq!(result["error"]["code"], "invalid_request");
    assert!(
        result["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("both"))
    );
    Ok(())
}

#[test]
fn archived_production_source_reconstructs_exact_plan_and_retains_replay_semantics() -> TestResult {
    let original =
        crate::lab_world::scenario_snapshot("starter_fortress", FortressId::new(743004), false)?;
    let request = ProductionRequest::parse(
        r#"{"template":"production","quotas":[{"item":"FOOD","minimum":50},{"item":"DRINK","minimum":60}]}"#,
    )?;
    let source = PlanSource::Production {
        summary: "original minima".to_owned(),
        raw: request.canonical_json(),
    };
    let context = OperationContext {
        session_id: SessionId::new(1),
        request_id: RequestId::new(77),
        anchor: original.anchor(),
        budget: MAX_LAB_BUDGET,
        grants: vec![CapabilityGrant {
            capability: Capability::Plan,
            scope: CapabilityScope::default(),
            max_risk: RiskTier::ReadOnly,
            expires_at_tick: None,
            remaining_uses: None,
        }],
        cancellation_requested: false,
    };
    let intent = source.intent(IntentId::new(77), &original)?;
    let planned = StaticPlanner::default().prepare_laboratory(&original, &intent, &context)?;
    struct Directory(std::path::PathBuf);
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let directory = Directory(std::env::temp_dir().join(format!(
        "dfmcp-production-original-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    )));
    let _ = std::fs::remove_dir_all(&directory.0);
    {
        let mut store = dfmcp_lab::durable::DurableLabStore::open(&directory.0)?;
        store.persist_commit(&original, planned.digest, 77, source.durable())?;
        store.compact()?;
    }
    let store = dfmcp_lab::durable::DurableLabStore::open(&directory.0)?;
    let commit = store
        .commit(original.fortress_id, planned.digest)
        .ok_or("durable commit missing")?;
    let sealed = store.load_snapshot(commit.sealed_state_hash)?;
    let recovered = PlanSource::from_durable(&commit.source);
    let rebuilt = recovered.intent(IntentId::new(commit.intent_id), &sealed)?;
    let rebuilt = StaticPlanner::default().prepare_laboratory(&sealed, &rebuilt, &context)?;
    assert_eq!(rebuilt.digest, planned.digest);
    assert_eq!(rebuilt.terminal_condition, planned.terminal_condition);
    let mut changed = sealed;
    changed
        .graph
        .entities
        .get_mut(&starter::STOCK_LEDGER)
        .ok_or("ledger missing")?
        .fields
        .insert(
            effects::STOCK_DRINK_FIELD.to_owned(),
            Fact::known(
                WorldValue::U64(33),
                changed.tick,
                FactSource::Derived("dfmcp.lab-scenario/1".to_owned()),
                Digest32::ZERO,
            ),
        );
    changed.refresh_hash();
    let replayed = recovered.intent(IntentId::new(78), &changed)?;
    assert!(matches!(
        &replayed.requested_actions[0].action,
        Action::CreateWorkOrder { amount: 6, .. }
    ));
    assert_eq!(replayed.terminal_condition, planned.terminal_condition);
    let legacy = PlanSource::from_durable(&dfmcp_lab::durable::DurablePlanSource::Actions {
        summary: "legacy lowered work".to_owned(),
        raw: request.compile(&original)?.actions,
    });
    assert!(matches!(&legacy, PlanSource::Actions { .. }));
    assert_ne!(
        legacy
            .intent(IntentId::new(77), &original)?
            .terminal_condition,
        planned.terminal_condition
    );
    Ok(())
}
