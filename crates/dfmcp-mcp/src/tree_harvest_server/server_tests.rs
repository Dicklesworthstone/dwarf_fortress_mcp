//! Packet production, including the actual output-overflow branch.
use super::*;
fn spine(value: &Value) {
    let contract: Value = serde_json::from_str(include_str!(
        "../../../../architecture/agent_turn_contract.json"
    ))
    .expect("checked contract");
    for key in contract["field_order"].as_array().expect("field order") {
        assert!(
            value["agent_turn"]
                .get(key.as_str().expect("field name"))
                .is_some(),
            "missing {key}"
        );
    }
    assert_eq!(value["agent_turn"]["schema"], "dfmcp.agent_turn/1");
    assert!(value["agent_turn"]["anchor"].is_null());
    assert!(
        value["agent_turn"]["references"]
            .as_array()
            .expect("references")
            .iter()
            .all(Value::is_object)
    );
}
#[test]
fn tree_packet_pre_effect_errors_have_no_invented_work_or_null_references() {
    for operation in ["fortress.open_session", "fortress.plan"] {
        let raw = failed(
            operation,
            None,
            None,
            error(ErrorCode::InvalidRequest, "invalid mode or handle"),
            Value::Null,
        );
        let value: Value = serde_json::from_str(&raw).expect("packet");
        spine(&value);
        for key in [
            "pending_plans",
            "obligations",
            "indeterminate_effects",
            "confirmations",
        ] {
            assert_eq!(value["agent_turn"]["active_work"][key], json!([]));
        }
        assert_eq!(value["agent_turn"]["references"], json!([]));
        assert_eq!(value["agent_turn"]["recommendations"], json!([]));
    }
}
#[test]
fn tree_packet_output_overflow_keeps_full_spine_and_original_uncertainty() {
    let session = SessionId::new((1u128 << 127) | FAMILY | 1);
    let request = RequestId::new(17);
    let reference = json!({"original_key":"tree-original","original_review_seal":"exact-seal","uncertain_effect":true,"automatic_redispatch":false});
    let result = json!({"ok":true,"retained_state_currently_verified":true,"original":{"key":"tree-original","review_seal":"exact-seal","evidence":"x".repeat(RESPONSE_BYTES as usize+1)}});
    let raw = packet(
        "fortress.commit",
        Some(session),
        Some(request),
        result,
        reference.clone(),
    );
    assert!(raw.len() as u64 <= RESPONSE_BYTES);
    let value: Value = serde_json::from_str(&raw).expect("overflow packet");
    spine(&value);
    assert_eq!(value["result"]["error"]["code"], "budget_exceeded");
    assert_eq!(value["agent_turn"]["session_id"], session.to_string());
    assert_eq!(value["agent_turn"]["request_id"], request.to_string());
    assert_eq!(value["agent_turn"]["phase"], "commit");
    assert_eq!(
        value["agent_turn"]["active_work"]["indeterminate_effects"],
        json!([reference.clone()])
    );
    assert_eq!(
        value["agent_turn"]["references"],
        json!([reference.clone()])
    );
    assert_eq!(value["retained_reference"], reference);
    assert_eq!(
        value["agent_turn"]["coverage"]["omitted_domains"],
        json!(["full_native_evidence"])
    );
}
