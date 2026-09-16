//! Compare two complete selected projections without allocating a baseline.
//! Called only after the archive owner has verified both record identities.
//! Selection, presence and provenance semantics are shared with query baselines.
use super::*;
use std::time::Instant;

fn remaining(context: &OperationContext, started: Instant) -> Result<OperationContext> {
    context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    let elapsed = started.elapsed().as_millis();
    if elapsed >= u128::from(context.budget.max_wall_millis) {
        return Err(exhausted("endpoint comparison exhausted its shared wall-time allowance"));
    }
    let mut result = context.clone();
    result.budget.max_wall_millis -= elapsed as u64;
    Ok(result)
}

/// `binding` identifies the verified archive, current session and exact record
/// pair. It is an identity checksum, not authorization. Current authority is
/// checked before any historical anchor is substituted and after computation.
/// Neither the baseline store nor the watch store is read or modified.
pub(in super::super) fn compare_endpoints(
    before: &WorldSnapshot,
    after: &WorldSnapshot,
    context: &OperationContext,
    selection: &Value,
    binding: Digest32,
    limit: Option<u32>,
    continuation: Option<&str>,
) -> Result<Value> {
    let started = Instant::now();
    remaining(context, started)?;
    check_input(selection)?;
    validate_selection(selection)?;
    let basis = before.anchor();
    let target = after.anchor();
    if basis.fortress_id != context.anchor.fortress_id || target.fortress_id != basis.fortress_id
        || basis.cursor.epoch != target.cursor.epoch || target.cursor.sequence < basis.cursor.sequence
        || target.tick < basis.tick || (target.cursor == basis.cursor && target != basis)
    {
        return Err(failure(ErrorCode::StaleAnchor,
            "endpoint comparison requires forward, unforked observations in one fortress and epoch"));
    }
    let hard_limit = context.budget.max_entities.min(128);
    let limit = limit.unwrap_or(hard_limit.min(16));
    if limit == 0 || limit > hard_limit {
        return Err(invalid("historical change limit exceeds the negotiated row allowance"));
    }
    for snapshot in [before, after] {
        if snapshot.graph.entities.len() > context.budget.max_entities as usize {
            return Err(exhausted("historical endpoint exceeds the entity scan budget"));
        }
        if !snapshot.hash_is_valid() { return Err(invariant("historical endpoint hash is invalid")); }
    }
    let mut old_context = remaining(context, started)?;
    old_context.anchor = basis;
    let (old_rows, _) = materialize(before, &old_context, selection)?;
    let mut new_context = remaining(context, started)?;
    new_context.anchor = target;
    let (new_rows, _) = materialize(after, &new_context, selection)?;
    let before_digest = rows_digest(&old_rows)?;
    let after_digest = rows_digest(&new_rows)?;
    let identity = Digest32::of_bytes(&encode(&json!({
        "domain":"dfmcp-archived-selection-changes/1", "binding":binding.to_string(),
        "session":context.session_id.to_string(), "authority_anchor":anchor_json(context.anchor),
        "basis":anchor_json(basis), "target":anchor_json(target), "selection":selection,
        "before_result":before_digest.to_string(), "after_result":after_digest.to_string()
    }))?);
    let (changes, refreshed) = compare_rows(&old_rows, &new_rows);
    let mut counts = BTreeMap::from([
        ("entered_result", 0usize), ("left_result", 0usize), ("changed_in_result", 0usize),
    ]);
    for change in &changes {
        let name = change["kind"].as_str().ok_or_else(|| invariant("change kind absent"))?;
        let count = counts.get_mut(name).ok_or_else(|| invariant("unknown endpoint change kind"))?;
        *count += 1;
    }
    // The established qh1 codec binds the new domain-separated identity. Baseline
    // tokens cannot cross into this operation; page width is not part of identity.
    let start = offset(continuation, identity, changes.len())?;
    let mut payload = json!({
        "schema":"dfmcp.query.result/1", "kind":"historical_changes", "anchor":anchor_json(target),
        "basis":anchor_json(basis), "selection":selection, "matched_before":old_rows.len(),
        "matched_after":new_rows.len(), "basis_result_digest":before_digest.to_string(),
        "target_result_digest":after_digest.to_string(), "comparison_digest":identity.to_string(),
        "change_count":changes.len(), "change_counts":counts, "provenance_only_refreshes":refreshed,
        "anchor_advanced":basis != target, "baseline_created":false, "comparison_persisted":false,
        "mutation_authority":false, "changes":[], "returned":0, "truncated":false, "continuation":null,
        "ordering":"entity_id_ascending_then_generation_ascending",
        "coverage":{"domain":"complete_selected_endpoint_projections", "absence_proven":false,
            "temporal_coverage":"endpoint_comparison_only", "intermediate_observations_evaluated":false},
        "note":"Entered/left means selected at one endpoint only, not born, dead, created or deleted. Generation reuse is a separate departure and arrival; changes between endpoints are not inferred."
    });
    let maximum = usize::try_from(context.budget.max_bytes.min(u64::from(context.budget.max_output_tokens) * 4))
        .map_err(|_| exhausted("historical comparison byte allowance overflow"))?;
    let mut end = start;
    for change in changes.iter().skip(start).take(limit as usize) {
        let mut candidate = payload.clone();
        candidate["changes"].as_array_mut().ok_or_else(|| invariant("change array absent"))?.push(change.clone());
        candidate["returned"] = json!(end + 1 - start);
        candidate["truncated"] = json!(end + 1 < changes.len());
        candidate["continuation"] = if end + 1 < changes.len() { json!(cursor(end + 1, identity)?) } else { Value::Null };
        if encode(&candidate)?.len() > maximum { break; }
        payload = candidate;
        end += 1;
    }
    if start < changes.len() && end == start {
        return Err(exhausted("one complete before/after change cannot fit; narrow selected fields or increase the response budget"));
    }
    if encode(&payload)?.len() > maximum { return Err(exhausted("historical comparison metadata exceeds the response budget")); }
    remaining(context, started)?;
    Ok(payload)
}

#[cfg(test)]
#[path = "query_history_endpoints_tests.rs"]
mod tests;
