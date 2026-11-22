//! Public source/plan/commit integration for concurrent configuration.
use super::*;

const LOGISTICS: [(&str, &str); 1] = [("configure_logistics", "guarded")];

fn pair(selector: &str) -> std::result::Result<(String, String), Box<dyn std::error::Error>> {
    let a = open_shared(selector, Some("starter_fortress"), &LOGISTICS)?;
    let b = open_shared(selector, None, &LOGISTICS)?;
    assert_eq!(a["ok"], true, "{a}");
    assert_eq!(b["ok"], true, "{b}");
    Ok((
        a["session_id"].as_str().ok_or("a")?.to_owned(),
        b["session_id"].as_str().ok_or("b")?.to_owned(),
    ))
}

fn stockpile(field: &str, value: u32) -> String {
    let mut action = json!({"kind":"configure_stockpile","stockpile":"2001","accepts":["FOOD"]});
    action[field] = json!(value);
    json!([{"action":action}]).to_string()
}

fn standing_order(key: &str, value: &str) -> String {
    json!([{"action":{"kind":"set_standing_order","key":key,"value":value}}]).to_string()
}

fn seal(session: &str, actions: &str) -> std::result::Result<String, Box<dyn std::error::Error>> {
    let planned = parsed(&fortress_plan(
        Some(session.to_owned()),
        None,
        None,
        Some(actions.to_owned()),
        None,
    ))?;
    assert_eq!(planned["ok"], true, "{planned}");
    Ok(planned["plan_digest"].as_str().ok_or("digest")?.to_owned())
}

fn fields(session: &str, kind: &str) -> std::result::Result<Value, Box<dyn std::error::Error>> {
    let observed = parsed(&fortress_query(
        Some(session.to_owned()),
        Some(json!({"mode":"entities","kind":kind}).to_string()),
    ))?;
    assert_eq!(observed["ok"], true, "{observed}");
    let rows = observed["rows"].as_array().ok_or("rows")?;
    assert_eq!(rows.len(), 1, "{observed}");
    Ok(rows[0]["fields"].clone())
}

#[test]
fn shared_stockpile_capacity_requests_compose_and_retry_the_original_digest() -> TestResult {
    let (a, b) = pair("73840")?;
    let digest = seal(&a, &stockpile("max_barrels", 8))?;
    let other = plan_and_commit(&b, &stockpile("max_bins", 7))?;
    assert_eq!(other["ok"], true, "{other}");
    let committed = parsed(&fortress_commit(Some(a.clone()), digest.clone()))?;
    assert_eq!(committed["ok"], true, "{committed}");
    assert_eq!(committed["actions"][0]["state"], "Verified");
    assert_eq!(
        committed["witness_rebase"]["certificate"]["schema"],
        "dfmcp.configuration-merge/1"
    );
    assert_eq!(
        committed["witness_rebase"]["certificate"]["from_digest"],
        digest
    );
    assert_eq!(
        committed["witness_rebase"]["certificate"]["witness"]["writes"][0]["disposition"],
        "converged_to_requested_value"
    );
    let observed = fields(&a, "stockpile")?;
    assert_eq!(observed["max_bins"], 7);
    assert_eq!(observed["max_barrels"], 8);
    assert_eq!(observed["accepts"], json!(["FOOD"]));
    let retry = parsed(&fortress_commit(Some(a), digest))?;
    assert_eq!(retry["ok"], true, "{retry}");
    assert_eq!(retry["actions"], committed["actions"]);
    Ok(())
}

#[test]
fn shared_distinct_standing_orders_merge_the_new_canonical_settings_record() -> TestResult {
    let (a, b) = pair("73841")?;
    let digest = seal(&a, &standing_order("gather_refuse", "outside"))?;
    let other = plan_and_commit(&b, &standing_order("gather_bodies", "yes"))?;
    assert_eq!(other["ok"], true, "{other}");
    let committed = parsed(&fortress_commit(Some(a.clone()), digest.clone()))?;
    assert_eq!(committed["ok"], true, "{committed}");
    assert_eq!(committed["actions"][0]["state"], "Verified");
    assert_eq!(
        committed["witness_rebase"]["certificate"]["witness"]["identities"][0]["disposition"],
        "reuse_concurrently_created_canonical_settings"
    );
    let observed = fields(&a, "fortress_settings")?;
    assert_eq!(observed["standing_order.gather_refuse"], "outside");
    assert_eq!(observed["standing_order.gather_bodies"], "yes");
    let retry = parsed(&fortress_commit(Some(a), digest))?;
    assert_eq!(retry["ok"], true, "{retry}");
    assert_eq!(retry["actions"], committed["actions"]);
    Ok(())
}

#[test]
fn shared_divergent_setting_requires_review_and_does_not_dispatch() -> TestResult {
    let (a, b) = pair("73842")?;
    let digest = seal(&a, &stockpile("max_bins", 5))?;
    assert_eq!(plan_and_commit(&b, &stockpile("max_bins", 8))?["ok"], true);
    let refused = parsed(&fortress_commit(Some(a.clone()), digest))?;
    assert_eq!(refused["ok"], false, "{refused}");
    assert_eq!(refused["error"]["code"], "stale_anchor");
    assert!(
        refused["witness_check"]["refinement_refusal"]
            .as_str()
            .is_some_and(|reason| reason.contains("diverged")),
        "{refused}"
    );
    assert!(
        refused["rebased_plan"]["plan_digest"].is_string(),
        "{refused}"
    );
    assert_eq!(fields(&a, "stockpile")?["max_bins"], 8);
    Ok(())
}

#[test]
fn configuration_merge_does_not_restore_revoked_current_effect_authority() -> TestResult {
    let (a, b) = pair("73843")?;
    let digest = seal(&a, &stockpile("max_barrels", 8))?;
    assert_eq!(plan_and_commit(&b, &stockpile("max_bins", 7))?["ok"], true);
    let session = crate::server::resolve_session(Some(a.clone()))?;
    {
        let mut guard = session.lock().map_err(|_| "session poisoned")?;
        guard
            .grants
            .retain(|grant| grant.capability != dfmcp_core::Capability::ConfigureLogistics);
    }
    let refused = parsed(&fortress_commit(Some(a.clone()), digest))?;
    assert_eq!(refused["ok"], false, "{refused}");
    assert_eq!(refused["error"]["code"], "capability_denied");
    let observed = fields(&a, "stockpile")?;
    assert_eq!(observed["max_bins"], 7);
    assert!(observed["max_barrels"].is_null());
    Ok(())
}
