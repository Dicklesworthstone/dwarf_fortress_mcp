//! Shared anchors reconstruct complete current work without rewriting history.

use super::*;

fn anchor() -> Value {
    json!({"fortress_id":"9", "epoch":1, "sequence":3,
        "game_tick":101, "state_hash":"a".repeat(64)})
}

fn views(current: &Value) -> (Value, Value) {
    let historical = json!({"game_tick":1, "sequence":0, "state_hash":"b".repeat(64),
        "epoch":1, "fortress_id":"9"});
    let work = json!({
        "pending_plans":[{"anchor":current, "steps":[{"literal_anchor":current}]}],
        "actions":[{"work_state":{"observed_anchor":current,
            "workforce":{"source_anchor":current},
            "construction_materials":{"source_anchor":current},
            "provisions":{"source_anchor":current},
            "harvesting":{"source_anchor":current}}}],
        "untracked_work":{"observed_anchor":current, "quiescent":false},
        "carried_obligations":[{"observed_anchor":historical, "state":"unknown"}],
    });
    let goals = json!([{
        "sealed_anchor":historical, "observed_anchor":current,
        "first_satisfied_anchor":null, "recorded_first_satisfied_anchor":historical,
        "restore_abandoned_anchor":{"epistemic_state":"unknown"},
        "semantic_room_goal":{"observed_anchor":current, "truth":"false"},
        "semantic_output_goal":{"observed_anchor":current, "predicate_truth":"unknown"},
        "retired_undispatched_steps":[{"proof_anchor":current, "step":2}],
        "original_source":{"request":{"literal_anchor":current, "$ref":"literal source data"}},
        "terminal_condition":{"field":{"value":current}},
        "resource_reserve":{"status":"work_required", "epistemic":"predicted"},
        "needs_replan":true,
    }]);
    (work, goals)
}

#[test]
fn current_anchor_sharing_reconstructs_complete_work_and_objective_rows() -> Result<()> {
    let current = anchor();
    let (original_work, original_goals) = views(&current);
    let (mut work, mut goals) = (original_work.clone(), original_goals.clone());
    let encoding = apply(&mut work, &mut goals, &current)?;
    assert_eq!(encoding["schema"], "dfmcp.current-anchor-references/1");
    assert_eq!(encoding["scope"], "same_response");
    assert_eq!(encoding["json_pointer"], "/current_anchor");
    assert_eq!(encoding["complete"], true);
    assert_eq!(encoding["reference_count"], 11);
    assert!(
        work.to_string().len() + goals.to_string().len()
            < original_work.to_string().len() + original_goals.to_string().len()
    );
    assert_eq!(
        work["pending_plans"][0]["steps"],
        original_work["pending_plans"][0]["steps"]
    );
    for field in [
        "original_source",
        "terminal_condition",
        "resource_reserve",
        "sealed_anchor",
        "first_satisfied_anchor",
        "recorded_first_satisfied_anchor",
        "restore_abandoned_anchor",
    ] {
        assert_eq!(
            goals[0][field], original_goals[0][field],
            "unchanged {field}"
        );
    }
    let mut expanded = 0_u64;
    visit_anchors(&mut work, &mut goals, &mut |value| {
        if value == &json!({"$ref":"/current_anchor"}) {
            *value = current.clone();
            expanded += 1;
        }
        Ok(())
    })?;
    assert_eq!(encoding["reference_count"], expanded);
    assert_eq!(work, original_work);
    assert_eq!(goals, original_goals);
    Ok(())
}

#[test]
fn current_anchor_sharing_preserves_unequal_and_unavailable_evidence() -> Result<()> {
    let current = anchor();
    let mut work = json!({"pending_plans":[], "actions":[
        {"work_state":{"observed_anchor":null}},
        {"work_state":{"observed_anchor":{"epistemic_state":"unknown"}}}
    ]});
    let mut goals = json!([{"observed_anchor":{"fortress_id":"other", "epoch":1,
        "sequence":3, "game_tick":101, "state_hash":"a".repeat(64)},
        "status":"unavailable", "semantic_output_goal":{"predicate_truth":"unknown"}}]);
    let before = (work.clone(), goals.clone());
    let encoding = apply(&mut work, &mut goals, &current)?;
    assert_eq!(encoding["reference_count"], 0);
    assert_eq!((work, goals), before);
    Ok(())
}

#[test]
fn current_anchor_sharing_refuses_malformed_targets_and_preexisting_references_atomically() {
    for case in 0..5 {
        let mut current = anchor();
        let (mut work, mut goals) = views(&current);
        match case {
            0 => current["state_hash"] = json!("not a digest"),
            1 => current["sequence"] = Value::Null,
            2 => work["actions"][0]["work_state"]["observed_anchor"] = json!({"$ref":"/foreign"}),
            3 => {
                goals[0]["semantic_output_goal"]["observed_anchor"] =
                    json!({"$ref":"/current_anchor"})
            }
            _ => goals[0]["retired_undispatched_steps"] = json!("incomplete"),
        }
        let before = (work.clone(), goals.clone());
        assert!(
            apply(&mut work, &mut goals, &current).is_err(),
            "case {case}"
        );
        assert_eq!((work, goals), before, "case {case} changed custody");
    }
}
