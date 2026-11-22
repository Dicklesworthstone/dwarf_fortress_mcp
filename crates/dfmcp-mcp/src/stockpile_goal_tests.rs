//! Complete stockpile settings through source custody and the real tool facade.
use super::*;
use crate::agent_facade as facade;
use dfmcp_intent::effects;
use dfmcp_world::Value as WorldValue;
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

fn request(bins: Option<u32>, barrels: Option<u32>, wheelbarrows: Option<u32>) -> Value {
    let mut action = json!({"kind":"configure_stockpile","stockpile":"2001","accepts":["FOOD"]});
    for (field, value) in [
        ("max_bins", bins),
        ("max_barrels", barrels),
        ("max_wheelbarrows", wheelbarrows),
    ] {
        if let Some(value) = value {
            action[field] = json!(value);
        }
    }
    json!([{"action":action}])
}

fn open() -> TestResult<Owner> {
    let opened = parsed(&facade::fortress_open_session(
        Some(true),
        None,
        Some(
            [
                ("observe", "read_only"),
                ("query", "read_only"),
                ("plan", "read_only"),
                ("configure_logistics", "reversible"),
            ]
            .into_iter()
            .map(|(c, r)| (c.to_owned(), r.to_owned()))
            .collect(),
        ),
        Some(30_000),
        None,
        None,
        None,
        Some(16_384),
        Some(64),
        Some("starter_fortress".to_owned()),
        Some(false),
        Some(false),
    ))?;
    assert_eq!(opened["ok"], true, "{opened}");
    Ok(Owner(
        opened["session_id"].as_str().ok_or("session")?.to_owned(),
    ))
}

fn plan(owner: &Owner, actions: &Value) -> TestResult<Value> {
    parsed(&facade::fortress_plan(
        Some(owner.0.clone()),
        None,
        None,
        Some(actions.to_string()),
        None,
    ))
}

fn commit(owner: &Owner, actions: &Value) -> TestResult<Value> {
    let planned = plan(owner, actions)?;
    assert_eq!(planned["ok"], true, "{planned}");
    parsed(&facade::fortress_commit(
        Some(owner.0.clone()),
        planned["plan_digest"].as_str().ok_or("digest")?.to_owned(),
    ))
}

#[test]
fn capacity_only_stockpile_updates_are_plannable_proved_and_idempotent() -> TestResult {
    let _serial = crate::test_serial();
    let owner = open()?;
    assert_eq!(
        commit(&owner, &request(Some(4), Some(5), Some(6)))?["ok"],
        true
    );
    for (field, target, actions) in [
        ("max_bins", 0, request(Some(0), None, None)),
        ("max_barrels", 7, request(None, Some(7), None)),
        ("max_wheelbarrows", 2, request(None, None, Some(2))),
    ] {
        let planned = plan(&owner, &actions)?;
        assert_eq!(planned["ok"], true, "{field}: {planned}");
        let digest = planned["plan_digest"].as_str().ok_or("digest")?.to_owned();
        {
            let session = resolve_session(Some(owner.0.clone()))?;
            let guard = session.lock().map_err(|_| "session")?;
            let pending = guard.pending.as_ref().ok_or("pending")?;
            let PlanSource::Actions { raw, .. } = &pending.source else {
                return Err("source".into());
            };
            assert_eq!(parsed(raw)?[0]["proof"], "stockpile_settings_v1");
            assert_eq!(pending.plan.steps[0].postconditions.len(), 3);
            assert_ne!(
                PredicateEvidence::laboratory(guard.adapter.snapshot())?
                    .evaluate(&pending.plan.terminal_condition)?,
                PredicateTruth::True
            );
        }
        let committed = parsed(&facade::fortress_commit(
            Some(owner.0.clone()),
            digest.clone(),
        ))?;
        assert_eq!(committed["ok"], true, "{committed}");
        assert_eq!(committed["actions"][0]["state"], "Verified");
        let replayed = parsed(&facade::fortress_commit(
            Some(owner.0.clone()),
            digest.clone(),
        ))?;
        assert_eq!(replayed["actions"], committed["actions"]);
        let session = resolve_session(Some(owner.0.clone()))?;
        let guard = session.lock().map_err(|_| "session")?;
        assert_eq!(
            guard.adapter.snapshot().graph.entities[&EntityId::new(2001)].fields[field]
                .known_value(),
            Some(&WorldValue::U64(target))
        );
        assert_eq!(
            original_goal_observation(&guard, &digest)?.0,
            PredicateTruth::True
        );
    }
    let unchanged = plan(&owner, &request(Some(0), Some(7), Some(2)))?;
    assert_eq!(unchanged["ok"], false, "{unchanged}");
    assert_eq!(unchanged["error"]["code"], "invalid_intent");
    Ok(())
}

#[test]
fn stockpile_original_goal_does_not_hide_a_later_capacity_change() -> TestResult {
    let _serial = crate::test_serial();
    let owner = open()?;
    let planned = plan(&owner, &request(Some(2), Some(3), None))?;
    assert_eq!(planned["ok"], true, "{planned}");
    let digest = planned["plan_digest"].as_str().ok_or("digest")?.to_owned();
    assert_eq!(
        parsed(&facade::fortress_commit(
            Some(owner.0.clone()),
            digest.clone()
        ))?["ok"],
        true
    );
    assert_eq!(commit(&owner, &request(Some(9), None, None))?["ok"], true);
    let session = resolve_session(Some(owner.0.clone()))?;
    let guard = session.lock().map_err(|_| "session")?;
    let (truth, details) = original_goal_observation(&guard, &digest)?;
    assert_eq!(truth, PredicateTruth::False, "{details}");
    assert!(details["achieved_tick"].is_u64(), "{details}");
    assert_eq!(
        guard.adapter.snapshot().graph.entities[&EntityId::new(2001)].fields
            [effects::STOCKPILE_MAX_BARRELS_FIELD]
            .known_value(),
        Some(&WorldValue::U64(3))
    );
    Ok(())
}

#[test]
fn stockpile_durable_sources_reconstruct_both_exact_proof_generations() -> TestResult {
    let snapshot =
        crate::lab_world::scenario_snapshot("starter_fortress", FortressId::new(459_104), true)?;
    let raw = request(Some(0), Some(7), Some(2)).to_string();
    let legacy = PlanSource::Actions {
        summary: "configure storage".to_owned(),
        raw: raw.clone(),
    };
    let current = PlanSource::Actions {
        summary: "configure storage".to_owned(),
        raw: crate::lab_world::canonical_current_actions(&raw)?,
    };
    let context = OperationContext {
        session_id: SessionId::new(1),
        request_id: RequestId::new(1),
        anchor: snapshot.anchor(),
        budget: WorkBudget::default(),
        cancellation_requested: false,
        grants: vec![dfmcp_core::CapabilityGrant {
            capability: Capability::Plan,
            scope: dfmcp_core::CapabilityScope::default(),
            max_risk: RiskTier::ReadOnly,
            expires_at_tick: None,
            remaining_uses: None,
        }],
    };
    let old_intent = legacy.intent(IntentId::new(501), &snapshot)?;
    let new_intent = current.intent(IntentId::new(501), &snapshot)?;
    let planner = StaticPlanner::default();
    let old = planner.prepare_laboratory(&snapshot, &old_intent, &context)?;
    let new = planner.prepare_laboratory(&snapshot, &new_intent, &context)?;
    assert_ne!(old.digest, new.digest);
    assert_eq!(old.steps[0].postconditions.len(), 1);
    assert_eq!(new.steps[0].postconditions.len(), 5);
    for (source, expected) in [(&legacy, &old), (&current, &new)] {
        let durable = source.durable();
        let reopened = PlanSource::from_durable(&durable);
        assert_eq!(reopened.durable(), durable);
        let intent = reopened.intent(IntentId::new(501), &snapshot)?;
        assert_eq!(
            recovery_compiler::prepare_exact(&snapshot, &intent, &context, expected.digest)?,
            *expected
        );
    }
    assert_eq!(
        recovery_compiler::prepare_exact(&snapshot, &old_intent, &context, new.digest)
            .err()
            .map(|e| e.code),
        Some(ErrorCode::CorruptLedger)
    );
    assert_eq!(
        recovery_compiler::prepare_exact(&snapshot, &new_intent, &context, old.digest)
            .err()
            .map(|e| e.code),
        Some(ErrorCode::CorruptLedger)
    );
    Ok(())
}

#[test]
fn stockpile_candidate_comparison_and_selection_cannot_restore_filter_only_goals() -> TestResult {
    let _serial = crate::test_serial();
    let owner = open()?;
    assert_eq!(commit(&owner, &request(Some(4), None, None))?["ok"], true);
    let actions = request(Some(0), None, None);
    let wrapper = json!({"template":"compare_candidates","version":1,"horizon_ticks":100,
        "candidates":[{"key":"a","actions":actions},{"key":"b","actions":actions}]});
    let report = parsed(&facade::fortress_plan(
        Some(owner.0.clone()),
        None,
        None,
        None,
        Some(wrapper.to_string()),
    ))?;
    assert_eq!(report["ok"], true, "{report}");
    assert_eq!(report["candidates"].as_array().map(Vec::len), Some(2));
    for row in report["candidates"].as_array().ok_or("candidates")? {
        assert_eq!(row["status"], "evaluated", "{row}");
        assert_eq!(row["prediction"]["predicted_complete"], true, "{row}");
    }
    let selection = report["candidates"][0]["next_step"]["arguments"]["blueprint"]
        .as_str()
        .ok_or("selection")?;
    let selected = parsed(&facade::fortress_plan(
        Some(owner.0.clone()),
        None,
        None,
        None,
        Some(selection.to_owned()),
    ))?;
    assert_eq!(selected["ok"], true, "{selected}");
    let committed = parsed(&facade::fortress_commit(
        Some(owner.0.clone()),
        selected["plan_digest"].as_str().ok_or("digest")?.to_owned(),
    ))?;
    assert_eq!(committed["ok"], true, "{committed}");
    let session = resolve_session(Some(owner.0.clone()))?;
    let guard = session.lock().map_err(|_| "session")?;
    assert_eq!(
        guard.adapter.snapshot().graph.entities[&EntityId::new(2001)].fields
            [effects::STOCKPILE_MAX_BINS_FIELD]
            .known_value(),
        Some(&WorldValue::U64(0))
    );
    Ok(())
}
