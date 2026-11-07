//! Public shared-session coverage; helpers are the ordinary eleven-tool facade.
use super::*;

const MINE: &str =
    r#"[{"action":{"kind":"set_labor","units":["1001"],"labor":"MINE","enabled":true}}]"#;
const BREW: &str =
    r#"[{"action":{"kind":"set_labor","units":["1001"],"labor":"BREW","enabled":true}}]"#;
const BURROW: &str = r#"[{"action":{"kind":"set_burrow_membership","units":["1001"],"burrow":"3001","assigned":true}}]"#;

fn pair(selector: &str) -> std::result::Result<(String, String), Box<dyn std::error::Error>> {
    let a = open_shared(selector, Some("starter_fortress"), &ALL_EFFECTS)?;
    let b = open_shared(selector, None, &ALL_EFFECTS)?;
    Ok((
        a["session_id"].as_str().ok_or("a")?.to_owned(),
        b["session_id"].as_str().ok_or("b")?.to_owned(),
    ))
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

fn unit_fields(session: &str) -> std::result::Result<Value, Box<dyn std::error::Error>> {
    let queried = parsed(&fortress_query(
        Some(session.to_owned()),
        Some(r#"{"mode":"entities","kind":"unit"}"#.to_owned()),
    ))?;
    assert_eq!(queried["ok"], true, "{queried}");
    let unit = queried["rows"]
        .as_array()
        .ok_or("rows")?
        .iter()
        .find(|row| row["entity_id"] == "1001")
        .ok_or("unit")?;
    Ok(unit["fields"].clone())
}

#[test]
fn independent_same_unit_setters_commit_with_source_bound_field_certificate() -> TestResult {
    for (selector, actions, field) in [
        ("73811", MINE, "labor.MINE"),
        ("73812", BURROW, "burrow.3001"),
    ] {
        let (a, b) = pair(selector)?;
        let sealed = seal(&a, actions)?;
        let other = plan_and_commit(&b, BREW)?;
        assert_eq!(other["ok"], true, "{other}");
        let committed = parsed(&fortress_commit(Some(a.clone()), sealed.clone()))?;
        assert_eq!(committed["ok"], true, "{committed}");
        assert_eq!(committed["actions"][0]["state"], "Verified");
        let rebase = &committed["witness_rebase"];
        let certificate = &rebase["certificate"];
        assert_eq!(certificate["schema"], "dfmcp.field-witness-rebase/1");
        assert_eq!(certificate["from_digest"], sealed);
        assert_ne!(certificate["to_digest"], sealed);
        assert_ne!(
            certificate["base_state_hash"],
            certificate["current_state_hash"]
        );
        assert_eq!(
            certificate["coarse_conflict"],
            json!({"read":"entity","entity_id":"1001"})
        );
        assert_eq!(certificate["witness"]["fields"][0]["field"], field);
        assert_eq!(certificate["witness"]["fields"][0]["presence"], "missing");
        assert_eq!(
            rebase["certificate_digest"],
            dfmcp_core::Digest32::of_bytes(certificate.to_string().as_bytes()).to_hex()
        );
        let observed = unit_fields(&a)?;
        assert_eq!(observed[field], true);
        assert_eq!(observed["labor.BREW"], true);
        assert!(committed["agent_turn"]["anchor"].is_object());
        let retry = parsed(&fortress_commit(Some(a), sealed))?;
        assert_eq!(retry["ok"], true, "{retry}");
        assert_eq!(retry["actions"], committed["actions"]);
    }
    Ok(())
}

#[test]
fn field_refinement_does_not_recover_revoked_effect_authority() -> TestResult {
    let (a, b) = pair("73813")?;
    let sealed = seal(&a, MINE)?;
    assert_eq!(plan_and_commit(&b, BREW)?["ok"], true);
    let session = crate::server::resolve_session(Some(a.clone()))?;
    {
        let mut guard = session.lock().map_err(|_| "session poisoned")?;
        guard
            .grants
            .retain(|grant| grant.capability != dfmcp_core::Capability::ConfigureLabor);
    }
    let refused = parsed(&fortress_commit(Some(a.clone()), sealed))?;
    assert_eq!(refused["ok"], false, "{refused}");
    assert_eq!(refused["error"]["code"], "capability_denied");
    let observed = unit_fields(&a)?;
    assert!(observed["labor.MINE"].is_null());
    assert_eq!(observed["labor.BREW"], true);
    Ok(())
}

#[test]
fn incomplete_field_census_returns_a_reviewed_replay_without_dispatch() -> TestResult {
    let (a, b) = pair("73814")?;
    let sealed = seal(&a, MINE)?;
    assert_eq!(plan_and_commit(&b, BREW)?["ok"], true);
    let session = crate::server::resolve_session(Some(a.clone()))?;
    {
        let mut guard = session.lock().map_err(|_| "session poisoned")?;
        guard.budget.max_entities = 1;
    }
    let refused = parsed(&fortress_commit(Some(a.clone()), sealed))?;
    assert_eq!(refused["ok"], false, "{refused}");
    assert_eq!(refused["error"]["code"], "stale_anchor");
    assert_eq!(refused["witness_check"]["accepted"], false);
    assert!(
        refused["witness_check"]["refinement_refusal"]
            .as_str()
            .is_some_and(|reason| reason.contains("census"))
    );
    assert!(refused["rebased_plan"]["plan_digest"].is_string());
    {
        let mut guard = session.lock().map_err(|_| "session poisoned")?;
        guard.budget.max_entities = 2_000;
    }
    let observed = unit_fields(&a)?;
    assert!(observed["labor.MINE"].is_null());
    assert_eq!(observed["labor.BREW"], true);
    Ok(())
}
