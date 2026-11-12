//! History metadata presentation must not acknowledge a world observation.
//! Owning bead: df-lab-structured-query-f7q.4.

use super::*;

type TestResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

struct Owner(String);
impl Drop for Owner {
    fn drop(&mut self) {
        orientation_sessions().remove(&self.0);
    }
}

fn parsed(raw: &str) -> TestResult<Value> {
    Ok(serde_json::from_str(raw)?)
}

#[test]
fn history_metadata_does_not_scan_world_changes_or_publish_a_new_observation_basis()
-> TestResult {
    let _serial = crate::test_serial();
    let opened = parsed(&fortress_open_session(
        Some(false), None,
        Some(vec![("observe".into(),"read_only".into()),
            ("query".into(),"read_only".into()),("control_clock".into(),"reversible".into())]),
        Some(60_000), Some(5_000), Some(16_384), Some(4 * 1024 * 1024),
        Some(16_384), Some(64), Some("starter_fortress".into()), Some(false), Some(false),
    ))?;
    assert_eq!(opened["ok"], true, "{opened}");
    let owner = Owner(opened["session_id"].as_str().ok_or("session")?.into());
    let prior = orientation_sessions().get(&owner.0).ok_or("orientation")?.anchor.clone();

    // Advance the real server world without publishing a facade observation.
    let advanced = parsed(&crate::server::wait_with_ticks(Some(owner.0.clone()), Some(1)))?;
    assert_eq!(advanced["ok"], true, "{advanced}");
    let current = advanced["observed_anchor"].clone();
    assert!(current.is_object(), "{advanced}");
    assert_ne!(Some(current.clone()), prior);
    assert_eq!(orientation_sessions().get(&owner.0).ok_or("orientation")?.anchor, prior);

    let history = parsed(&fortress_query(
        Some(owner.0.clone()), Some(json!({"mode":"history","limit":1}).to_string()),
    ))?;
    assert_eq!(history["ok"], true, "{history}");
    assert_eq!(history["agent_turn"]["anchor"], current);
    assert_eq!(history["agent_turn"]["continuity"]["status"], "partial");
    assert_eq!(history["agent_turn"]["changes"], json!([]));
    assert_eq!(history["source_work"]["world_contents_scanned"], false);
    assert_eq!(orientation_sessions().get(&owner.0).ok_or("orientation")?.anchor, prior);

    // A prospective metadata result also leaves the observation basis intact
    // when only the complete Agent Turn pushes it over the final allowance.
    let mut result = history.clone();
    result.as_object_mut().ok_or("result")?.remove("agent_turn");
    result.as_object_mut().ok_or("result")?.remove("output_budget");
    result["query_budget"] = json!({
        "maximum_response_bytes":1024,"max_output_tokens":256,"max_wall_millis":60_000,
    });
    let (success, failure) = ResponsePresentation::capture_metadata_query(&result, Some(&owner.0));
    assert!(success.world_delta.is_empty());
    assert!(failure.world_delta.is_empty());
    let response = success.finish_query(&failure, result, std::time::Instant::now());
    assert!(response.len() <= 1024, "{response}");
    let refused = parsed(&response)?;
    assert_eq!(refused["ok"], false, "{refused}");
    assert_eq!(refused["error"]["code"], "budget_exceeded", "{refused}");
    assert!(refused.get("records").is_none());
    assert!(refused.get("continuation").is_none_or(Value::is_null));
    assert_eq!(orientation_sessions().get(&owner.0).ok_or("orientation")?.anchor, prior);
    Ok(())
}
