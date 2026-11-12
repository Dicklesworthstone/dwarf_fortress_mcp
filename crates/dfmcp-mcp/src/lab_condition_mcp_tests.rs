//! Real laboratory preflight, history, evidence and work-discovery consumers.
use super::*;
use dfmcp_world::{FactPresence, FactSource, Value as WorldValue};
use serde_json::Value;

type TestResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

struct Owner(String);
impl Drop for Owner {
    fn drop(&mut self) {
        if let Ok(id) = u128::from_str_radix(&self.0, 16) {
            sessions().remove(&SessionId::new(id));
        }
    }
}

fn parsed(raw: &str) -> TestResult<Value> {
    Ok(serde_json::from_str(raw)?)
}

fn open() -> TestResult<Owner> {
    let result = parsed(&crate::agent_facade::fortress_open_session(
        Some(true),
        None,
        Some(vec![
            ("observe".into(), "read_only".into()),
            ("query".into(), "read_only".into()),
            ("plan".into(), "reversible".into()),
            ("configure_labor".into(), "reversible".into()),
        ]),
        Some(60_000),
        None,
        None,
        None,
        None,
        None,
        Some("starter_fortress".into()),
        Some(false),
        Some(false),
    ))?;
    assert_eq!(result["ok"], true, "{result}");
    Ok(Owner(
        result["session_id"].as_str().ok_or("session")?.into(),
    ))
}

fn snapshot(owner: &Owner) -> TestResult<WorldSnapshot> {
    Ok(resolve_session(Some(owner.0.clone()))?
        .lock()
        .map_err(|_| "session lock")?
        .adapter
        .snapshot()
        .clone())
}

fn query(owner: &Owner, wrapper: Value) -> TestResult<Value> {
    parsed(&crate::agent_facade::fortress_query(
        Some(owner.0.clone()),
        Some(wrapper.to_string()),
    ))
}

fn inspection(condition: Value) -> Value {
    json!({"mode":"structured", "request":{"schema":"dfmcp.query/1",
        "query":{"kind":"condition_evaluation", "condition":condition}}})
}

fn stock() -> Value {
    json!({"op":"field", "entity_id":crate::lab_world::starter::STOCK_LEDGER.to_string(),
        "generation":1, "field":dfmcp_intent::effects::STOCK_DRINK_FIELD,
        "comparison":"ge", "value":{"type":"u64", "value":1}})
}

fn temporal(owner: &Owner, request: Value) -> TestResult<Value> {
    query(
        owner,
        json!({"mode":"temporal", "request":{"schema":"dfmcp.query/1", "query":request}}),
    )
}

fn refuse(value: &Value, code: &str) {
    assert_eq!(value["ok"], false, "{value}");
    assert_eq!(value["error"]["code"], code, "{value}");
    assert!(value.get("evaluation").is_none(), "{value}");
    assert_eq!(
        value["agent_turn"]["coverage"]["absence_proof_scope"],
        json!([])
    );
}

#[test]
fn lab_condition_public_default_budget_preflight_matches_watch_without_consuming_samples()
-> TestResult {
    let _serial = crate::test_serial();
    let owner = open()?;
    let before = snapshot(&owner)?.anchor();
    let result = query(&owner, inspection(stock()))?;
    assert_eq!(result["ok"], true, "{result}");
    assert_eq!(result["evaluation"]["status"], "condition_met");
    assert_eq!(result["evaluation"]["watch_completion_proven"], false);
    assert_eq!(result["evidence_policy"], "laboratory_reference/1");
    assert_eq!(result["watch_registered"], false);
    assert_eq!(result["watch_evaluated"], false);
    assert_eq!(result["native_captures"], 0);
    assert_eq!(snapshot(&owner)?.anchor(), before);
    assert_eq!(
        temporal(&owner, json!({"kind":"watches"}))?["records"],
        json!([])
    );
    let watched = temporal(
        &owner,
        json!({"kind":"watch", "key":"stock",
        "condition":stock(), "deadline_tick":before.tick.0 + 20,
        "stable_observations":2, "poll_interval_ticks":1}),
    )?;
    assert_eq!(watched["ok"], true, "{watched}");
    assert_eq!(watched["record"]["status"], "candidate");
    assert_eq!(
        watched["record"]["evaluation"]["condition"],
        result["evaluation"]["condition_truth"]
    );
    query(&owner, inspection(stock()))?;
    let listed = temporal(&owner, json!({"kind":"watches"}))?;
    assert_eq!(
        listed["records"][0]["evidence_digest"],
        watched["record"]["evidence_digest"]
    );
    assert_eq!(
        listed["records"][0]["stable_observations"],
        watched["record"]["stable_observations"]
    );
    let handoff = {
        let session = resolve_session(Some(owner.0.clone()))?;
        let guard = session.lock().map_err(|_| "session lock")?;
        handoff_json(&guard)
    };
    assert_eq!(handoff["query_work"]["status"], "available");
    assert_eq!(
        handoff["query_work"]["query_watches"][0]["watch"],
        watched["record"]["watch"]
    );
    assert_eq!(
        handoff["query_work"]["condition_watches"][0]["next_step"]["tool"],
        "fortress.query"
    );
    assert_eq!(snapshot(&owner)?.anchor(), before);
    Ok(())
}

#[test]
fn lab_condition_public_real_labor_history_keeps_current_authority_and_exact_source() -> TestResult
{
    let _serial = crate::test_serial();
    let owner = open()?;
    let before = snapshot(&owner)?.anchor();
    let condition = json!({"op":"field", "entity_id":"1001", "generation":1,
        "field":"labor.MINING", "comparison":"eq", "value":{"type":"bool", "value":true}});
    assert_eq!(
        query(&owner, inspection(condition.clone()))?["evaluation"]["status"],
        "blocked_unknown"
    );
    let plan = parsed(&crate::agent_facade::fortress_plan(
        Some(owner.0.clone()), None, None,
        Some(json!([{"action":{"kind":"set_labor", "units":["1001"], "labor":"MINING", "enabled":true}}]).to_string()),
        None,
    ))?;
    assert_eq!(plan["ok"], true, "{plan}");
    let committed = parsed(&crate::agent_facade::fortress_commit(
        Some(owner.0.clone()),
        plan["plan_digest"].as_str().ok_or("digest")?.into(),
    ))?;
    assert_eq!(committed["ok"], true, "{committed}");
    let current = query(&owner, inspection(condition.clone()))?;
    assert_eq!(
        current["evaluation"]["status"], "condition_met",
        "{current}"
    );
    let mut historical = inspection(condition);
    historical["at"] = json!(before.state_hash.to_string());
    historical["request"]["expected_anchor"] = anchor_json(&before);
    let old = query(&owner, historical.clone())?;
    assert_eq!(old["ok"], true, "{old}");
    assert_eq!(old["evaluation"]["status"], "blocked_unknown");
    assert_eq!(old["anchor"], anchor_json(&before));
    assert_eq!(old["current_anchor"], current["anchor"]);
    assert_eq!(old["historical"], true);
    assert_eq!(
        old["observation_coverage"]["eligible_as_mutation_evidence"],
        false
    );
    let mut wrong = historical.clone();
    wrong["request"]["expected_anchor"] = current["anchor"].clone();
    refuse(&query(&owner, wrong)?, "stale_anchor");
    {
        let session = resolve_session(Some(owner.0.clone()))?;
        let mut guard = session.lock().map_err(|_| "session lock")?;
        guard
            .grants
            .retain(|grant| grant.capability != Capability::Query);
    }
    refuse(&query(&owner, historical)?, "capability_denied");
    let observed = parsed(&crate::agent_facade::fortress_observe(Some(
        owner.0.clone(),
    )))?;
    assert_eq!(observed["ok"], true, "{observed}");
    assert_eq!(
        observed["agent_turn"]["active_work"]["query_work_coverage"]["status"],
        "unavailable"
    );
    assert_eq!(
        observed["agent_turn"]["active_work"]["query_work_coverage"]["code"],
        "capability_denied"
    );
    assert!(
        observed["agent_turn"]["active_work"]
            .get("query_watches")
            .is_none()
    );
    Ok(())
}

#[test]
fn lab_condition_public_unknown_failure_and_unqualified_sources_never_prove_success() -> TestResult
{
    let _serial = crate::test_serial();
    let owner = open()?;
    let mut guarded = inspection(stock());
    guarded["request"]["query"]["failure_condition"] = json!({"op":"field",
        "entity_id":"1001", "generation":1, "field":"not_observed",
        "comparison":"eq", "value":{"type":"bool", "value":true}});
    let result = query(&owner, guarded.clone())?;
    assert_eq!(result["evaluation"]["condition_truth"], "true");
    assert_eq!(result["evaluation"]["failure_condition_truth"], "unknown");
    assert_eq!(result["evaluation"]["status"], "blocked_unknown");
    guarded["request"]["query"]["failure_condition"] = json!({"op":"paused", "value":true});
    assert_eq!(
        query(&owner, guarded)?["evaluation"]["status"],
        "failure_condition_met"
    );
    let original = snapshot(&owner)?;
    for redacted in [false, true] {
        let mut source = original.clone();
        let fact = source
            .graph
            .entities
            .get_mut(&crate::lab_world::starter::STOCK_LEDGER)
            .and_then(|record| {
                record
                    .fields
                    .get_mut(dfmcp_intent::effects::STOCK_DRINK_FIELD)
            })
            .ok_or("stock fact")?;
        if redacted {
            fact.value = WorldValue::Text("hidden-stock-marker".into());
            fact.presence = Some(FactPresence::Redacted("private".into()));
        } else {
            fact.source = FactSource::AgentAssertion("untrusted".into());
        }
        source.cursor = snapshot(&owner)?.cursor.checked_next().ok_or("cursor")?;
        source.refresh_hash();
        resolve_session(Some(owner.0.clone()))?
            .lock()
            .map_err(|_| "session lock")?
            .adapter
            .inject_snapshot(source)?;
        let value = query(&owner, inspection(json!({"op":"not", "arg":stock()})))?;
        assert_eq!(value["evaluation"]["status"], "blocked_unknown", "{value}");
        assert!(!value.to_string().contains("hidden-stock-marker"));
    }
    Ok(())
}

#[test]
fn lab_condition_public_rejects_ambiguous_unsupported_and_stateful_preflight() -> TestResult {
    let _serial = crate::test_serial();
    let owner = open()?;
    for raw in [
        r#"{"mode":"structured","request":{"schema":"dfmcp.query/1","query":{"kind":"condition_evaluation","condition":{"op":"paused","value":true,"value":false}}}}"#,
        r#"{"mode":"structured","request":{"schema":"dfmcp.query/1","query":{"kind":"condition_evaluation","condition":{"op":"paused","value":true},"condition":{"op":"paused","value":false}}}}"#,
        r#"{"mode":"structured","request":{"schema":"dfmcp.query/1","query":{"kind":"condition_evaluation","condition":{"op":"item_quantity","comparison":"ge","value":1}}}}"#,
        r#"{"mode":"structured","request":{"schema":"dfmcp.query/1","query":{"kind":"condition_evaluation","condition":{"op":"paused","value":true},"stable_observations":1}}}"#,
        r#"{"mode":"structured","request":{"schema":"dfmcp.query/1","query":{"kind":"condition_evaluation","condition":{"op":"paused","value":true},"evidence_policy":"native"}}}"#,
    ] {
        let result = parsed(&crate::agent_facade::fortress_query(
            Some(owner.0.clone()),
            Some(raw.into()),
        ))?;
        refuse(&result, "invalid_request");
    }
    assert_eq!(
        temporal(&owner, json!({"kind":"watches"}))?["records"],
        json!([])
    );
    Ok(())
}


#[test]
fn lab_field_sum_public_production_wait_and_history_share_qualified_quantity_truth() -> TestResult {
    let _serial = crate::test_serial();
    let opened = parsed(&crate::agent_facade::fortress_open_session(
        Some(false),
        None,
        Some(vec![
            ("observe".into(), "read_only".into()),
            ("query".into(), "read_only".into()),
            ("plan".into(), "reversible".into()),
            ("configure_production".into(), "reversible".into()),
            ("control_clock".into(), "reversible".into()),
        ]),
        Some(60_000),
        Some(2_000),
        Some(8_192),
        Some(4 * 1024 * 1024),
        Some(16_384),
        Some(64),
        Some("starter_fortress".into()),
        Some(false),
        Some(false),
    ))?;
    assert_eq!(opened["ok"], true, "{opened}");
    let owner = Owner(opened["session_id"].as_str().ok_or("session")?.into());
    let first = snapshot(&owner)?;
    let amount = match first
        .graph
        .entities
        .get(&crate::lab_world::starter::STOCK_LEDGER)
        .and_then(|entity| entity.fields.get(dfmcp_intent::effects::STOCK_DRINK_FIELD))
        .and_then(|fact| dfmcp_world::laboratory_fact_value(fact, first.tick))
    {
        Some(WorldValue::U64(amount)) => *amount,
        _ => return Err("starter drink quantity is not qualified".into()),
    };
    let threshold = amount.checked_add(1).ok_or("stock threshold overflow")?;
    let condition = json!({
        "op":"field_sum",
        "scope":"observed_projection",
        "kind":"stock_ledger",
        "field":dfmcp_intent::effects::STOCK_DRINK_FIELD,
        "predicate":{"op":"always"},
        "comparison":"ge",
        "value":threshold
    });
    let preflight = query(&owner, inspection(condition.clone()))?;
    assert_eq!(preflight["ok"], true, "{preflight}");
    assert_eq!(preflight["evaluation"]["condition_truth"], "false");
    assert_eq!(snapshot(&owner)?.anchor(), first.anchor());
    let watched = temporal(
        &owner,
        json!({
            "kind":"watch", "key":"total-drink",
            "condition":condition, "deadline_tick":first.tick.0 + 1_000,
            "stable_observations":2, "poll_interval_ticks":10
        }),
    )?;
    assert_eq!(watched["ok"], true, "{watched}");
    let handle = watched["record"]["watch"].as_str().ok_or("watch handle")?.to_owned();
    let plan = parsed(&crate::agent_facade::fortress_plan(
        Some(owner.0.clone()),
        Some("produce a qualified total drink quantity".into()),
        None,
        Some(json!([{
            "action":{"kind":"create_work_order", "name":"quantity-watch-brew",
                "job_token":"BREW_DRINK", "amount":20}
        }]).to_string()),
        None,
    ))?;
    assert_eq!(plan["ok"], true, "{plan}");
    let committed = parsed(&crate::agent_facade::fortress_commit(
        Some(owner.0.clone()),
        plan["plan_digest"].as_str().ok_or("plan digest")?.into(),
    ))?;
    assert_eq!(committed["ok"], true, "{committed}");
    let waited = parsed(&crate::agent_facade::fortress_wait_for_watch(
        Some(owner.0.clone()),
        Some(1_000),
        handle.clone(),
    ))?;
    assert_eq!(waited["ok"], true, "{waited}");
    assert_eq!(waited["selected_watch_wait"]["stop_reason"], "satisfied", "{waited}");
    assert_eq!(waited["selected_watch_wait"]["record"]["definition"],
        watched["record"]["definition"]);
    assert!(waited["advanced_game_ticks"].as_u64().is_some_and(|ticks| ticks > 0 && ticks < 1_000));
    assert!(waited["agent_turn"]["active_work"]["query_watches"]
        .as_array().ok_or("watch catalog")?
        .iter().any(|record| record["watch"] == handle));

    let current = snapshot(&owner)?;
    let current_truth = query(&owner, inspection(condition.clone()))?;
    assert_eq!(current_truth["evaluation"]["condition_truth"], "true", "{current_truth}");
    let mut historical = inspection(condition.clone());
    historical["at"] = json!(first.state_hash.to_string());
    let original_truth = query(&owner, historical)?;
    assert_eq!(original_truth["ok"], true, "{original_truth}");
    assert_eq!(original_truth["evaluation"]["condition_truth"], "false");
    assert_eq!(original_truth["anchor"], anchor_json(&first.anchor()));
    assert_eq!(original_truth["current_anchor"], anchor_json(&current.anchor()));

    let evidence_before = temporal(&owner, json!({"kind":"watches"}))?["records"].clone();
    let replayed = query(
        &owner,
        json!({
            "mode":"condition_replay",
            "from":current.state_hash.to_string(), "to":current.state_hash.to_string(),
            "definition":{"condition":condition, "deadline_tick":current.tick.0 + 20,
                "poll_interval_ticks":1, "stable_observations":1},
            "detail":"evidence"
        }),
    )?;
    assert_eq!(replayed["ok"], true, "{replayed}");
    assert_eq!(replayed["status"], "satisfied", "{replayed}");
    assert_eq!(replayed["range"]["records"], 1);
    assert_eq!(replayed["eligible_as_mutation_evidence"], false);
    assert_eq!(temporal(&owner, json!({"kind":"watches"}))?["records"], evidence_before);
    assert_eq!(snapshot(&owner)?, current);
    Ok(())
}
