//! Pure evaluation of the SAME conditions used by foreground watches. This
//! module never takes the watch-store lock, allocates a handle, or publishes a
//! checkpoint. A true predicate at one capture is not stable goal completion.
use super::*;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    schema: String,
    expected_anchor: Option<Value>,
    query: Inspection,
}
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Inspection {
    ConditionEvaluation {
        condition: Condition,
        failure_condition: Option<Condition>,
    },
}

pub(super) fn query(
    snapshot: &WorldSnapshot,
    context: &OperationContext,
    input: &Value,
) -> Result<Value> {
    let mut budget = counts::EvaluationBudget::new(context.budget.max_wall_millis);
    authorize(snapshot, context)?;
    validate_input(input)?;
    let parsed: Input = serde_json::from_value(input.clone())
        .map_err(|_| invalid("invalid condition_evaluation request"))?;
    if parsed.schema != "dfmcp.query/1" {
        return Err(invalid("condition_evaluation requires dfmcp.query/1"));
    }
    if parsed
        .expected_anchor
        .as_ref()
        .is_some_and(|a| a != &anchor(context.anchor))
    {
        return Err(failure(
            ErrorCode::StaleAnchor,
            "condition inspection names another observation",
        ));
    }
    let Inspection::ConditionEvaluation {
        condition,
        failure_condition,
    } = parsed.query;
    // Reuse the authoritative joint success/failure validator. This temporary
    // definition is validation data ONLY: it has no Watch, handle or Store entry.
    let definition = Definition {
        key: "inspection".into(),
        label: "inspection".into(),
        condition,
        failure_condition,
        deadline_tick: 0,
        poll_interval_ticks: 1,
        stable_observations: 1,
    };
    validate_definition(&definition)?;
    let predicate_digest = digest(&json!({"domain":"dfmcp-condition-inspection-predicate/1",
        "condition":definition.condition,"failure_condition":definition.failure_condition}))?;
    let mut probe = Probe::default();
    let condition = probe.evaluate_bounded(&definition.condition, snapshot, &mut budget)?;
    let failure_condition = match &definition.failure_condition {
        Some(condition) => probe.evaluate_bounded(condition, snapshot, &mut budget)?,
        None => Truth::False,
    };
    // Match watch precedence, including non-short-circuited generation checks.
    // Do not invent cadence, stability, deadlines or terminal watch outcomes.
    let status = if probe.invalid_generation {
        "invalidated_reference"
    } else if failure_condition == Truth::True {
        "failure_condition_met"
    } else if condition == Truth::Unknown || failure_condition == Truth::Unknown {
        "blocked_unknown"
    } else if condition == Truth::True {
        "condition_met"
    } else {
        "condition_not_met"
    };
    let evaluation = json!({"status":status,"condition_truth":condition.text(),
        "failure_condition_truth":failure_condition.text(),
        "generation_mismatch":probe.invalid_generation,
        "eligible_success_sample":status=="condition_met",
        "facts":probe.facts,"stability_evaluated":false,"deadline_evaluated":false,
        "watch_completion_proven":false,"continuous_between_observations":false});
    let evidence = digest(&json!({"domain":"dfmcp-condition-inspection-evidence/1",
        "anchor":anchor(context.anchor),"predicate_digest":predicate_digest.to_string(),"evaluation":evaluation}))?;
    let result = json!({"schema":"dfmcp.query.result/1","kind":"condition_evaluation",
        "anchor":anchor(context.anchor),"predicate_digest":predicate_digest.to_string(),
        "evaluation":evaluation,"evidence_digest":evidence.to_string(),
        "native_captures":0,"watch_registered":false,"watch_evaluated":false,
        "mutation_dispatched":false,"truncated":false,"continuation":null,
        "coverage":{"domain":"explicit_predicates_at_selected_observation","absence_proven":false,
            "continuous_between_observations":false,"mutation_success_proven":false},
        "interpretation":"Predicate evidence at one selected capture, not a retained watch sample, stable goal completion, native job readiness or permission to act."});
    let maximum = context
        .budget
        .max_bytes
        .min(u64::from(context.budget.max_output_tokens) * 4);
    if serde_json::to_vec(&result)
        .map_err(|_| invalid("condition evidence cannot be encoded"))?
        .len() as u64
        > maximum
    {
        return Err(bounded(
            "complete condition evidence exceeds the result budget",
        ));
    }
    context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    budget.check()?;
    Ok(result)
}

pub(super) fn extend_schema(mut schema: Value) -> Result<Value> {
    let extension: Value = serde_json::from_str(include_str!(
        "../../../schemas/mcp_condition_evaluation_v1.json"
    ))
    .map_err(|_| invalid("embedded condition inspection schema is invalid"))?;
    schema["$defs"]["query"]["oneOf"]
        .as_array_mut()
        .ok_or_else(|| invalid("query schema variants absent"))?
        .push(extension);
    Ok(schema)
}

#[cfg(test)]
#[path = "query_condition_evaluation_tests.rs"]
mod tests;
