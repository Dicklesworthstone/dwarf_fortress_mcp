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

#[test]
fn tree_packet_chunked_native_evidence_roundtrips_and_respects_modern_bounds() {
    use dfmcp_core::Digest32;
    use fastmcp_rust::{
        ContentBlock, CoreResult, FinalCoreResult, ResultPeerEra, decode_typed_complete,
    };
    // Byte-preservation/transport boundaries, not claims of valid native trees.
    for size in [1usize, 24_000, 24_001, 30_489, 32_768, 32_769, 128 * 1024] {
        let bytes: Vec<u8> = (0..size).map(|n| n as u8).collect();
        let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        let reference = json!({"acknowledged":{"journal_id":"j","head":"h","frames":3},"uncertain_effect":true,"original_key":"original"});
        let raw = packet(
            "fortress.wait",
            None,
            None,
            json!({"ok":true,"original":{"original_source":{"native_capture_hex":hex}}}),
            reference.clone(),
        );
        let complete = presentation::present(raw).expect("bounded complete result");
        let encoded = CoreResult::Final(FinalCoreResult::ToolsCall {
            result: complete.clone(),
            diagnostic: None,
        })
        .encode()
        .expect("guarded modern tools/call encoding");
        assert!(
            decode_typed_complete::<FinalCallToolResult>(&encoded, ResultPeerEra::Modern).is_ok()
        );
        let result: Value = serde_json::from_str(&encoded).expect("complete JSON");
        assert!(result.get("_meta").is_none());
        let outer = json!({"jsonrpc":"2.0","id":"x".repeat(254),"result":result}).to_string();
        assert!(outer.len() < RESPONSE_BYTES as usize);
        assert_eq!(
            complete.payload.content.len(),
            1 + size.div_ceil(presentation::CHUNK_BYTES)
        );
        let texts: Vec<&str> = complete
            .payload
            .content
            .iter()
            .map(|block| {
                let ContentBlock::Text { text, .. } = block else {
                    panic!("text content")
                };
                assert!(text.len() <= 65_536);
                text.as_str()
            })
            .collect();
        let value: Value = serde_json::from_str(texts[0]).expect("small complete packet");
        spine(&value);
        assert_eq!(value["retained_reference"], reference);
        let descriptor = &value["result"]["original"]["original_source"]["native_capture_hex"];
        assert_eq!(descriptor["schema"], "dfmcp.tree-native-capture/1");
        assert_eq!(
            descriptor["field_path"],
            "/result/original/original_source/native_capture_hex"
        );
        assert_eq!(descriptor["byte_length"], size);
        assert_eq!(descriptor["sha256"], Digest32::of_bytes(&bytes).to_hex());
        let mut joined = String::new();
        for (index, part) in descriptor["blocks"]
            .as_array()
            .expect("parts")
            .iter()
            .enumerate()
        {
            assert_eq!(part["content_index"], index + 1);
            assert_eq!(part["offset_bytes"], joined.len() / 2);
            let text = texts[index + 1];
            assert_eq!(part["byte_length"], text.len() / 2);
            let offset = joined.len() / 2;
            assert_eq!(
                part["sha256"],
                Digest32::of_bytes(&bytes[offset..offset + text.len() / 2]).to_hex()
            );
            joined.push_str(text);
        }
        assert_eq!(joined, hex);
    }
    // Every occurrence is extracted, including arrays, without sharing block IDs.
    let raw = packet(
        "fortress.query",
        None,
        None,
        json!({"nested":[{"native_capture_hex":"0001"},{"deeper":{"native_capture_hex":"0203"}}]}),
        Value::Null,
    );
    let result = presentation::present(raw).expect("nested evidence");
    assert_eq!(result.payload.content.len(), 3);
    let ContentBlock::Text { text, .. } = &result.payload.content[0] else {
        panic!("packet")
    };
    let value: Value = serde_json::from_str(text).expect("nested packet");
    assert_eq!(
        value["result"]["nested"][0]["native_capture_hex"]["field_path"],
        "/result/nested/0/native_capture_hex"
    );
    assert_eq!(
        value["result"]["nested"][1]["deeper"]["native_capture_hex"]["blocks"][0]["content_index"],
        2
    );
}

#[test]
fn tree_packet_remaining_metadata_overflow_preserves_ack_and_uncertainty() {
    use fastmcp_rust::ContentBlock;
    let session = SessionId::new((1u128 << 127) | FAMILY | 1);
    let request = RequestId::new(19);
    let reference = json!({"acknowledged":{"journal_id":"j","head":"h","frames":3},"uncertain_effect":true,"original_key":"original"});
    // Below the product packet ceiling, above the pinned single-string ceiling.
    let raw = packet(
        "fortress.commit",
        Some(session),
        Some(request),
        json!({"ok":true,"other_metadata":"x".repeat(70_000)}),
        reference.clone(),
    );
    let complete = presentation::present(raw).expect("fallback");
    assert_eq!(complete.payload.content.len(), 1);
    let ContentBlock::Text { text, .. } = &complete.payload.content[0] else {
        panic!("packet")
    };
    let value: Value = serde_json::from_str(text).expect("fallback packet");
    spine(&value);
    assert_eq!(value["result"]["error"]["code"], "budget_exceeded");
    assert_eq!(value["agent_turn"]["session_id"], session.to_string());
    assert_eq!(value["agent_turn"]["request_id"], request.to_string());
    assert_eq!(
        value["agent_turn"]["active_work"]["indeterminate_effects"],
        json!([reference.clone()])
    );
    assert_eq!(value["retained_reference"], reference);
    assert_eq!(value["result"]["custody"], reference);
}

#[test]
fn tree_packet_numeric_id_full_envelope_preserves_chunks_at_budget_boundary() {
    use dfmcp_core::Digest32;
    use fastmcp_rust::{
        ContentBlock, CoreResult, FinalCoreResult, JsonRpcMessage, JsonRpcResponse,
        RequestId as WireRequestId, ResultPeerEra, decode_strict_jsonrpc_message,
        decode_strict_jsonrpc_response, decode_typed_complete,
    };
    // Synthetic byte-preservation and envelope accounting, not native tree facts.
    // Search the real packet/presentation boundary, rather than constructing an
    // unproducible CompleteResult or checking a small response only.
    let left = "a5".repeat(120_000);
    let right = "5a".repeat(120_000);
    let reference = json!({"acknowledged":{"journal_id":"original-journal","head":"original-head","frames":3},"original_key":"original","uncertain_effect":true});
    let mut low = 0usize;
    let mut high = 65_536usize;
    let mut best = None;
    while low <= high {
        let padding = low + (high - low) / 2;
        let raw = packet(
            "fortress.wait",
            None,
            None,
            json!({"ok":true,"left":{"native_capture_hex":left},"right":{"native_capture_hex":right},"padding":"x".repeat(padding)}),
            reference.clone(),
        );
        let complete = presentation::present(raw).expect("bounded real presentation");
        if complete.payload.content.len() == 11 {
            best = Some(complete);
            low = padding + 1;
        } else if padding == 0 {
            break;
        } else {
            high = padding - 1;
        }
    }
    let complete = best.expect("large lossless presentation fits");
    let ContentBlock::Text { text, .. } = &complete.payload.content[0] else {
        panic!("Agent Turn text")
    };
    let packet: Value = serde_json::from_str(text).expect("complete Agent Turn");
    spine(&packet);
    assert_eq!(packet["retained_reference"], reference);
    for (name, expected, byte) in [("left", &left, 0xa5), ("right", &right, 0x5a)] {
        let descriptor = &packet["result"][name]["native_capture_hex"];
        assert_eq!(descriptor["byte_length"], 120_000);
        assert_eq!(
            descriptor["sha256"],
            Digest32::of_bytes(&vec![byte; 120_000]).to_hex()
        );
        let mut joined = String::new();
        for part in descriptor["blocks"].as_array().expect("ordered blocks") {
            assert_eq!(part["offset_bytes"], joined.len() / 2);
            let index = part["content_index"].as_u64().expect("content index") as usize;
            let ContentBlock::Text { text, .. } = &complete.payload.content[index] else {
                panic!("native evidence text")
            };
            assert_eq!(part["byte_length"], text.len() / 2);
            assert_eq!(
                part["sha256"],
                Digest32::of_bytes(&vec![byte; text.len() / 2]).to_hex()
            );
            joined.push_str(text);
        }
        assert_eq!(&joined, expected);
    }
    let digits = "9".repeat(4096);
    let request = format!(
        r#"{{"jsonrpc":"2.0","id":{digits},"method":"tools/call","params":{{"name":"fortress.wait","arguments":{{}}}}}}"#
    );
    let JsonRpcMessage::Request(request) =
        decode_strict_jsonrpc_message(request.as_bytes(), 1024 * 1024).expect("maximum numeric ID")
    else {
        panic!("request")
    };
    let id = request.id.expect("request ID");
    assert!(matches!(&id, WireRequestId::Integer(value) if value == &digits));
    let encoded = CoreResult::Final(FinalCoreResult::ToolsCall {
        result: complete,
        diagnostic: None,
    })
    .encode()
    .expect("modern complete result");
    let result: Value = serde_json::from_str(&encoded).expect("result JSON");
    let wire = serde_json::to_vec(&JsonRpcResponse::success(id, result)).expect("response wire");
    assert!(wire.len() + 1 <= RESPONSE_BYTES as usize);
    assert!(
        wire.len() + 1 > RESPONSE_BYTES as usize - 64,
        "fixture must reach the actual budget boundary"
    );
    let admitted = decode_strict_jsonrpc_response(&wire, 1024 * 1024).expect("modern response");
    assert!(
        matches!(admitted.response().id.as_ref(), Some(WireRequestId::Integer(value)) if value == &digits)
    );
    assert!(
        decode_typed_complete::<FinalCallToolResult>(
            admitted.raw_result().expect("raw result"),
            ResultPeerEra::Modern,
        )
        .is_ok()
    );
    let overlong = format!(
        r#"{{"jsonrpc":"2.0","id":{},"method":"ping"}}"#,
        "9".repeat(4097)
    );
    assert!(decode_strict_jsonrpc_message(overlong.as_bytes(), 1024 * 1024).is_err());
}
