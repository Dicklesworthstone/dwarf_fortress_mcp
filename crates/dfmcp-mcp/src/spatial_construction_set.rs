//! One complete proposal for the WHOLE analyzed selection, independent of the
//! page currently rendered. Missing identities disable it, never shrink it.
use super::*;

const POLICY: &str = "dfmcp.furniture-set-condition/1";

pub(super) fn proposal(report: &Report, c: &OperationContext, options: &Monitoring) -> Result<Value> {
    let mut targets = Vec::with_capacity(report.rows.len());
    let mut unavailable = Vec::new();
    for row in &report.rows {
        if matches!(row.status, Status::Missing | Status::IdentityMismatch | Status::Unsupported)
            || row.item.as_ref().is_some_and(|item| item.handle.is_none())
        {
            unavailable.push(row.target.building_native_id);
            continue;
        }
        let building = row.handle.ok_or_else(|| invalid("construction set building absent"))?;
        let kind = match row.type_key.as_deref() {
            Some("Bed") => "bed", Some("Chair") => "chair", Some("Table") => "table",
            _ => return Err(invalid("construction set kind unestablished")),
        };
        let maximum = row.maximum_stage.ok_or_else(|| invalid("construction set stage absent"))?;
        let mut target = json!({"building_native_id":row.target.building_native_id,
            "building_generation":building.generation,"kind":kind,"max_stage":maximum});
        if let Some(item) = &row.item {
            let handle = item.handle.ok_or_else(|| invalid("construction set item absent"))?;
            target["item_native_id"] = json!(item.native_id);
            target["item_generation"] = json!(handle.generation);
        }
        targets.push(target);
    }
    if !unavailable.is_empty() {
        return Ok(json!({"available":false,"mode":"all_targets","policy":POLICY,
            "reason":"every_selected_building_and_item_identity_must_be_established",
            "unavailable_building_native_ids":unavailable,"requested_targets":report.rows.len(),
            "targets_omitted":false,"watch_registered":false}));
    }
    if targets.is_empty() || targets.len() > 32 {
        return Err(invalid("construction set requires 1..32 targets"));
    }
    let request = json!({"schema":"dfmcp.query/1","expected_anchor":anchor_json(c.anchor),
        "query":{"kind":"watch","key":format!("{}.all",options.key_prefix),
            "label":format!("All {} furniture construction conditions",targets.len()),
            "condition":{"op":"furniture_set","targets":targets,"test":"all_complete"},
            "failure_condition":{"op":"furniture_set","targets":targets,"test":"any_removal"},
            "deadline_tick":options.deadline_tick,"poll_interval_ticks":options.poll_interval_ticks,
            "stable_observations":options.stable_observations}});
    let bytes = serde_json::to_vec(&json!({"policy":POLICY,
        "session_id":c.session_id.to_string(),"request":request}))
        .map_err(|_| invalid("construction set proposal cannot be encoded"))?;
    Ok(json!({"available":true,"mode":"all_targets","policy":POLICY,
        "proposal_digest":Digest32::of_bytes(&bytes).to_string(),
        "tool":"fortress.query","session_id":c.session_id.to_string(),"watch_request":request,
        "requested_targets":targets.len(),"watch_slots_required":1,"watch_registered":false,
        "placement_receipt_verified":false,"native_effect_completed_proven":false,
        "interpretation":"Register this complete request explicitly. Every target must match at the same samples; removal of any selected building fails the goal. No original placement receipt, footprint, usability, causality or game-effect completion is proved."}))
}
