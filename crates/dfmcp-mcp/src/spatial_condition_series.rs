//! Presentation of condition samples, not a second condition evaluator or a
//! historical watch state machine. Classification equality never proves that
//! the world, its leaf facts, or the unobserved interval remained unchanged.
use super::*;

#[derive(Debug, PartialEq, Eq)]
struct Classification<'a> {
    condition: &'a str,
    failure: &'a str,
    status: &'a str,
    generation_mismatch: bool,
}
fn malformed() -> DfmcpError {
    error(ErrorCode::InternalInvariantViolation, "condition evaluator returned an invalid timeline sample")
}
fn truth(value: Option<&Value>) -> Result<&str> {
    value.and_then(Value::as_str).filter(|v| matches!(*v, "true" | "false" | "unknown"))
        .ok_or_else(malformed)
}
fn classification(value: &Value) -> Result<Classification<'_>> {
    if value.get("kind").and_then(Value::as_str) != Some("condition_evaluation") { return Err(malformed()); }
    let evaluation = value.get("evaluation").ok_or_else(malformed)?;
    let condition = truth(evaluation.get("condition_truth"))?;
    let failure = truth(evaluation.get("failure_condition_truth"))?;
    let generation_mismatch = evaluation.get("generation_mismatch").and_then(Value::as_bool).ok_or_else(malformed)?;
    let status = evaluation.get("status").and_then(Value::as_str).filter(|v| matches!(*v,
        "invalidated_reference" | "failure_condition_met" | "blocked_unknown" | "condition_met" | "condition_not_met"))
        .ok_or_else(malformed)?;
    if evaluation.get("watch_completion_proven").and_then(Value::as_bool) != Some(false)
        || evaluation.get("eligible_success_sample").and_then(Value::as_bool) != Some(status == "condition_met") {
        return Err(malformed());
    }
    Ok(Classification { condition, failure, status, generation_mismatch })
}

pub(super) fn validate(value: &Value) -> Result<()> { classification(value).map(|_| ()) }

pub(super) fn transition(before: &Value, after: &Value, mut result: Value, ticks: u64) -> Result<Value> {
    let a = classification(before)?; let b = classification(after)?;
    if before.get("predicate_digest") != after.get("predicate_digest")
        || before.get("predicate_digest").and_then(Value::as_str).is_none() {
        return Err(error(ErrorCode::InternalInvariantViolation, "timeline condition identity changed between samples"));
    }
    result["status"] = json!(if a == b {"same_evaluation_classification"} else {"evaluation_classification_changed"});
    result["from_status"] = json!(a.status); result["to_status"] = json!(b.status);
    result["condition_truth"] = json!({"from":a.condition,"to":b.condition});
    result["failure_condition_truth"] = json!({"from":a.failure,"to":b.failure});
    result["generation_mismatch"] = json!({"from":a.generation_mismatch,"to":b.generation_mismatch});
    result["elapsed_game_ticks"] = json!(ticks);
    result["stability_inferred"] = json!(false);
    result["watch_completion_proven"] = json!(false);
    result["unchanged_world_proven"] = json!(false);
    result["interpretation"] = json!("Compare classifications at adjacent retained observations only; equal classifications do not establish unchanged facts or continuous satisfaction, and no watch receives these samples.");
    Ok(result)
}

pub(super) fn row(entry: &JournalEntry, value: &Value, change: Value) -> Result<Value> {
    validate(value)?;
    Ok(json!({"record":archive::entry_json(entry),"evaluation":value["evaluation"],
        "predicate_digest":value["predicate_digest"],"evidence_digest":value["evidence_digest"],
        "change_from_previous":change}))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sample(condition: &str, guard: &str, mismatch: bool) -> Value {
        let status = if mismatch {"invalidated_reference"} else if guard == "true" {"failure_condition_met"}
            else if condition == "unknown" || guard == "unknown" {"blocked_unknown"}
            else if condition == "true" {"condition_met"} else {"condition_not_met"};
        json!({"kind":"condition_evaluation","predicate_digest":"p","evaluation":{
            "condition_truth":condition,"failure_condition_truth":guard,"generation_mismatch":mismatch,
            "status":status,"eligible_success_sample":status=="condition_met","watch_completion_proven":false}})
    }
    #[test]
    fn every_truth_guard_identity_pair_retains_its_exact_classification() -> Result<()> {
        for a in ["true", "false", "unknown"] {
            for af in ["true", "false", "unknown"] {
                for ag in [false, true] {
                    for b in ["true", "false", "unknown"] {
                        for bf in ["true", "false", "unknown"] {
                            for bg in [false, true] {
                                let out = transition(&sample(a,af,ag), &sample(b,bf,bg), json!({}), 1)?;
                                assert_eq!(out["status"], if (a,af,ag)==(b,bf,bg) {
                                    "same_evaluation_classification" } else { "evaluation_classification_changed" });
                                assert_eq!(out["condition_truth"]["from"], a);
                                assert_eq!(out["failure_condition_truth"]["to"], bf);
                                assert_eq!(out["watch_completion_proven"], false);
                                assert_eq!(out["stability_inferred"], false);
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }
    #[test]
    fn equal_unknowns_and_same_tick_samples_do_not_invent_stability_or_rates() -> Result<()> {
        let value = sample("unknown", "false", false);
        let out = transition(&value, &value, json!({}), 0)?;
        assert_eq!(out["status"], "same_evaluation_classification");
        assert_eq!(out["elapsed_game_ticks"], 0);
        assert_eq!(out["unchanged_world_proven"], false);
        assert!(out.get("net_rate").is_none());
        Ok(())
    }
    #[test]
    fn malformed_measurements_and_changed_predicates_fail_explicitly() {
        let a = sample("true", "false", false);
        for mode in 0..5 {
            let mut b = a.clone();
            match mode {
                0 => b["evaluation"]["condition_truth"] = json!(true),
                1 => b["evaluation"]["status"] = json!("satisfied"),
                2 => b["evaluation"]["watch_completion_proven"] = json!(true),
                3 => b["evaluation"]["generation_mismatch"] = Value::Null,
                _ => b["predicate_digest"] = json!("different"),
            }
            assert!(transition(&a, &b, json!({}), 1).is_err());
        }
    }
}
