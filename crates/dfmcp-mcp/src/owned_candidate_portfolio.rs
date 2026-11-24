//! Explicit selected complete projects in a discarded candidate portfolio.
//! Preserve each original's proof and exact reserve independently; aggregate
//! marginal future work without turning peer success into candidate evidence.

use super::*;

pub(super) fn validate(
    owned: &OwnedWork,
    peers: &AdmittedProjectSet<'_>,
    snapshot: &WorldSnapshot,
    candidate: &PreparedPlan,
    context: &OperationContext,
    started: Instant,
) -> Result<()> {
    peers.check_source(snapshot, context, started)?;
    peers.inspect_structure(snapshot, context, started)?;
    if peers.originals.is_empty()
        || peers.originals.len() > 7
        || peers.projects.len() != peers.originals.len()
        || peers.owned_actions != owned.actions.as_slice()
        || peers.fixed_records < owned.records
        || peers.fixed_bytes < owned.bytes
        || peers.action_steps < owned.actions.len().saturating_add(owned.carried.len()) as u64
        || peers.carried.len() != owned.carried.len()
    {
        return Err(invalid(
            "portfolio admission differs from complete authenticated owned work",
        ));
    }
    let mut identities = BTreeSet::from([candidate.digest]);
    for (original, project) in peers.originals.iter().zip(&peers.projects) {
        if original.actual != project.original || !identities.insert(original.actual) {
            return Err(invalid(
                "portfolio roots duplicate or change admitted project identities",
            ));
        }
    }
    // Both admissions independently authenticate custody against this same
    // owner. Compare the retained observation and sealed effect definitions as
    // well, so a prior admitted work set cannot replace a newer monitor.
    for (selected, current) in peers.carried.iter().zip(&owned.carried) {
        if selected.to_json() != current.to_json() || selected.work_step != current.work_step {
            return Err(invalid(
                "portfolio recovered custody differs from owned progression",
            ));
        }
    }
    Ok(())
}

fn risk(value: &Value) -> Result<RiskTier> {
    match value.as_str() {
        Some("read_only") => Ok(RiskTier::ReadOnly),
        Some("reversible") => Ok(RiskTier::Reversible),
        Some("guarded") => Ok(RiskTier::Guarded),
        Some("irreversible") => Ok(RiskTier::Irreversible),
        _ => Err(invalid("portfolio stage omitted its exact risk")),
    }
}

/// Existing committed peer history is a sunk action cost. It remains fully
/// present in source/action admission and original proof; only hypothetical
/// peer stages join the proposed candidate's marginal review and effect cost.
fn costs<'a>(value: &mut Value, stages: impl IntoIterator<Item = &'a Value>) -> Result<()> {
    let mut maximum = risk(&value["project_max_risk"])?;
    let mut checkpoint = value["project_requires_checkpoint"]
        .as_bool()
        .ok_or_else(|| invalid("portfolio candidate omitted its checkpoint cost"))?;
    let mut actions = value["project_action_count"]
        .as_u64()
        .ok_or_else(|| invalid("portfolio candidate omitted its action cost"))?;
    let mut capabilities = BTreeSet::new();
    for capability in value["project_required_capabilities"]
        .as_array()
        .ok_or_else(|| invalid("portfolio candidate omitted its capability cost"))?
    {
        capabilities.insert(
            capability
                .as_str()
                .ok_or_else(|| invalid("portfolio candidate has an invalid capability"))?
                .to_owned(),
        );
    }
    for stage in stages {
        maximum = maximum.max(risk(&stage["max_risk"])?);
        checkpoint |= stage["requires_checkpoint"]
            .as_bool()
            .ok_or_else(|| invalid("portfolio peer stage omitted its checkpoint cost"))?;
        actions = actions.saturating_add(
            stage["action_count"]
                .as_u64()
                .ok_or_else(|| invalid("portfolio peer stage omitted its action cost"))?,
        );
        for capability in stage["required_capabilities"]
            .as_array()
            .ok_or_else(|| invalid("portfolio peer stage omitted its capability cost"))?
        {
            capabilities.insert(
                capability
                    .as_str()
                    .ok_or_else(|| invalid("portfolio peer stage has an invalid capability"))?
                    .to_owned(),
            );
        }
    }
    value["portfolio_action_count"] = json!(actions);
    value["portfolio_max_risk"] = json!(maximum.as_str());
    value["portfolio_requires_checkpoint"] = json!(checkpoint);
    value["portfolio_required_capabilities"] = json!(capabilities);
    value["portfolio_cost_scope"] = json!("candidate_and_hypothetical_peer_stages");
    Ok(())
}

fn known_truth(value: &Value) -> bool {
    matches!(value.as_str(), Some("true" | "false"))
}

fn reserve_known(row: &Value) -> Result<bool> {
    if row["reserve_until_tick"].is_null() {
        return Ok(true);
    }
    if row["reserve_until_tick"].as_u64().is_none() {
        return Err(invalid(
            "portfolio original omitted its exact reserve identity",
        ));
    }
    Ok(known_truth(&row["reserve_frontier_truth"]))
}

pub(super) fn original_evidence_complete(row: &Value) -> Result<bool> {
    let mut complete = known_truth(&row["predicted_goal_truth"])
        && row["predicted_physical_quiescent"].is_boolean()
        && reserve_known(row)?;
    for stage in row["retained_stages"]
        .as_array()
        .ok_or_else(|| invalid("portfolio original omitted its retained-stage evidence"))?
    {
        complete &= known_truth(&stage["predicted_goal_truth"])
            && stage["predicted_physical_quiescent"].is_boolean()
            && reserve_known(stage)?;
    }
    Ok(complete)
}

fn selected(value: &mut Value, peers: &AdmittedProjectSet<'_>) {
    value["method"] =
        json!("complete_candidate_and_selected_peer_projects_with_owned_foreground_progression");
    value["selected_peer_projects"] = peers.describe();
    value["competing_work"]["peer_original_goals_selected"] = json!(true);
    value["competing_work"]["selected_peer_originals"] = json!(
        peers
            .originals
            .iter()
            .map(|peer| peer.actual.to_hex())
            .collect::<Vec<_>>()
    );
    value["sampling_policy"] = json!(
        "one real foreground cadence with every selected original and retained-stage exact reserve endpoint; private candidate and peer successors share the same world"
    );
    value["unselected_peer_future_stages_included"] = json!(false);
    value["future_stage_plans_are_executable"] = json!(false);
    value["requires_review_between_stages"] = json!(true);
}

pub(super) fn decorate(
    value: &mut Value,
    prediction: &Value,
    peers: &AdmittedProjectSet<'_>,
) -> Result<()> {
    let rows = prediction["originals"]
        .as_array()
        .filter(|rows| rows.len() == peers.originals.len() + 1)
        .ok_or_else(|| invalid("portfolio model omitted complete original outcomes"))?;
    let mut evidence = true;
    let mut any_false = false;
    let mut any_unknown = false;
    for row in rows {
        evidence &= original_evidence_complete(row)?;
        match row["predicted_goal_truth"].as_str() {
            Some("true") => {}
            Some("false") => any_false = true,
            Some("unknown") => any_unknown = true,
            _ => return Err(invalid("portfolio model omitted an original goal truth")),
        }
    }
    let mut outcomes = Vec::with_capacity(peers.originals.len());
    let mut future_stages = Vec::new();
    for (row, original) in rows.iter().skip(1).zip(&peers.originals) {
        if row["requested_plan_digest"] != original.requested.to_hex()
            || row["actual_plan_digest"] != original.actual.to_hex()
        {
            return Err(invalid(
                "portfolio model changed a selected original identity",
            ));
        }
        let mut row = row.clone();
        row["root_plan_digest"] = json!(original.actual.to_hex());
        for stage in row["future_stages"]
            .as_array()
            .ok_or_else(|| invalid("portfolio peer omitted its future-stage evidence"))?
        {
            future_stages.push(stage.clone());
        }
        outcomes.push(row);
    }
    costs(value, &future_stages)?;
    selected(value, peers);
    let complete = prediction["all_complete"]
        .as_bool()
        .ok_or_else(|| invalid("portfolio model omitted its common completion verdict"))?;
    let tick = prediction["predicted_frontier_tick"]
        .as_u64()
        .ok_or_else(|| invalid("portfolio model omitted its common frontier"))?;
    value["peer_projects"] = json!(outcomes);
    value["portfolio_evidence_complete"] = json!(evidence);
    value["predicted_portfolio_complete"] = json!(complete);
    value["portfolio_completion_tick"] = json!(complete.then_some(tick));
    value["portfolio_goal_truth"] = json!(if any_false {
        "false"
    } else if any_unknown {
        "unknown"
    } else {
        "true"
    });
    value["portfolio_completion_rule"] = prediction["completion_rule"].clone();
    Ok(())
}

pub(super) fn unavailable(
    value: &mut Value,
    peers: &AdmittedProjectSet<'_>,
    stages: &BTreeMap<Digest32, Vec<Value>>,
) -> Result<()> {
    costs(value, stages.values().flatten())?;
    selected(value, peers);
    value["predicted_portfolio_complete"] = json!(false);
    value["portfolio_evidence_complete"] = json!(false);
    value["portfolio_goal_truth"] = json!("unknown");
    value["portfolio_completion_tick"] = Value::Null;
    value["peer_projects"] = json!(
        peers
            .originals
            .iter()
            .map(|original| json!({
                "requested_plan_digest":original.requested.to_hex(),
                "actual_plan_digest":original.actual.to_hex(),
                "root_plan_digest":original.actual.to_hex(),
                "reserve_until_tick":original.reserve_until,
                "available":false, "stage_outcomes_available":false,
                "future_stages":stages.get(&original.actual).cloned().unwrap_or_default(),
                "future_stage_plans_are_executable":false,
            }))
            .collect::<Vec<_>>()
    );
    Ok(())
}
