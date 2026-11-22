use super::*;

type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;
const RAW: &str = r#"[{"action":{"kind":"configure_stockpile","stockpile":"2001","accepts":["FOOD"],"max_bins":0,"max_barrels":8,"max_wheelbarrows":2}}]"#;

#[test]
fn fresh_stockpile_sources_always_retain_complete_proof_and_parse_idempotently() -> TestResult {
    let current = canonical_current_actions(RAW)?;
    assert_eq!(canonical_current_actions(&current)?, current);
    let value: Value = serde_json::from_str(&current)?;
    assert_eq!(value[0]["proof"], "stockpile_settings_v1");
    let steps = super::super::parse_steps(&current)?;
    assert_eq!(steps[0].postconditions.len(), 5);
    // Historical reconstruction must not silently acquire a stronger goal.
    assert!(super::super::parse_steps(RAW)?[0].postconditions.is_empty());
    Ok(())
}

#[test]
fn source_marker_cannot_be_downgraded_ambiguous_or_attached_to_another_action() -> TestResult {
    let value: Value = serde_json::from_str(RAW)?;
    for marker in [Value::Null, json!("filter_only"), json!("future"), json!(1)] {
        let mut changed = value.clone();
        changed[0]["proof"] = marker;
        assert!(canonical_current_actions(&changed.to_string()).is_err());
    }
    for raw in [
        r#"[{"action":{"kind":"configure_stockpile","stockpile":"2001","accepts":[],"max_bins":0,"max_bins":1}}]"#,
        r#"[{"action":{"kind":"configure_stockpile","stockpile":"2001","accepts":[]},"proof":"stockpile_settings_v1","proof":"stockpile_settings_v1"}]"#,
        r#"[{"action":{"kind":"pause","paused":true},"proof":"stockpile_settings_v1"}]"#,
    ] {
        assert!(canonical_current_actions(raw).is_err(), "{raw}");
    }
    Ok(())
}

#[test]
fn unrelated_sources_keep_exact_bytes_and_complete_source_growth_is_bounded() -> TestResult {
    let unrelated = "[ { \"action\": {\"kind\":\"pause\",\"paused\":true} } ]";
    assert_eq!(canonical_current_actions(unrelated)?, unrelated);
    let oversized = format!(
        "{}{}",
        " ".repeat(super::super::MAX_ACTIONS_JSON_BYTES),
        RAW
    );
    assert_eq!(
        canonical_current_actions(&oversized).err().map(|e| e.code),
        Some(ErrorCode::BudgetExceeded)
    );
    Ok(())
}
