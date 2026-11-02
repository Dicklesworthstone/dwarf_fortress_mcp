//! Manager-only output packaging; evidence bytes never become control authority.
use super::{RESPONSE_BYTES, overflow_packet, parse_id};
use dfmcp_adapter::manager_appointment::MAX_CAPTURE_BYTES;
use dfmcp_core::{Digest32, RequestId};
use fastmcp_rust::{
    CompleteResult, ContentBlock, CoreResult, FinalCallToolResult, FinalCoreResult, McpError,
    McpResult, ResultMeta,
};
use serde_json::{Value, json};

pub(super) const CHUNK_BYTES: usize = 24_000;
// Pin 180a7c8 admits 4096-byte mathematical-integer ID lexemes. The 256-byte
// limit applies only to encoded string IDs. Reserve the largest admitted ID,
// complete JSON-RPC framing and newline within the existing product ceiling.
const MAX_JSONRPC_ID_BYTES: usize = 4096;
const ENVELOPE_BYTES: usize = b"{\"jsonrpc\":\"2.0\",\"id\":".len()
    + MAX_JSONRPC_ID_BYTES
    + b",\"result\":".len()
    + b"}\n".len();

fn complete(texts: Vec<String>) -> CompleteResult<FinalCallToolResult> {
    CompleteResult::new(
        FinalCallToolResult {
            content: texts.into_iter().map(ContentBlock::text).collect(),
            is_error: false,
            structured_content: None,
        },
        ResultMeta::empty(),
    )
}

fn fits(result: &CompleteResult<FinalCallToolResult>) -> bool {
    // Use the same guarded modern codec as tools/call dispatch. CompleteResult
    // deliberately has no Serde serializer; the codec enforces metadata,
    // content ordering and the pinned 1MiB/64KiB guards before measuring bytes.
    CoreResult::Final(FinalCoreResult::ToolsCall {
        result: result.clone(),
        diagnostic: None,
    })
    .encode()
    .is_ok_and(|s| s.len() + ENVELOPE_BYTES <= RESPONSE_BYTES as usize)
}

fn decode_hex(value: &str) -> Option<Vec<u8>> {
    if value.is_empty() || value.len() > 2 * MAX_CAPTURE_BYTES || !value.len().is_multiple_of(2) {
        return None;
    }
    let nibble = |b| match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        _ => None,
    };
    value
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|p| Some((nibble(p[0])? << 4) | nibble(p[1])?))
        .collect()
}

fn detach(value: &mut Value, path: &str, blocks: &mut Vec<String>) -> Option<()> {
    match value {
        Value::Object(object) => {
            for (key, child) in object {
                let path = format!("{path}/{}", key.replace('~', "~0").replace('/', "~1"));
                if key == "native_capture_hex" {
                    let text = child.as_str()?;
                    let bytes = decode_hex(text)?;
                    let mut parts = Vec::new();
                    for (index, chunk) in bytes.chunks(CHUNK_BYTES).enumerate() {
                        let offset = index * CHUNK_BYTES;
                        parts.push(json!({"content_index":blocks.len(),"offset_bytes":offset,
                            "byte_length":chunk.len(),"sha256":Digest32::of_bytes(chunk).to_hex()}));
                        blocks.push(text[offset * 2..(offset + chunk.len()) * 2].to_owned());
                    }
                    *child = json!({"schema":"dfmcp.manager-native-capture/1","field_path":path,
                        "encoding":"hex","byte_length":bytes.len(),"sha256":Digest32::of_bytes(&bytes).to_hex(),"blocks":parts});
                } else {
                    detach(child, &path, blocks)?;
                }
            }
        }
        Value::Array(array) => {
            for (index, child) in array.iter_mut().enumerate() {
                detach(child, &format!("{path}/{index}"), blocks)?;
            }
        }
        _ => {}
    }
    Some(())
}

pub(super) fn present(raw: String) -> McpResult<CompleteResult<FinalCallToolResult>> {
    let mut packet: Value = serde_json::from_str(&raw)
        .map_err(|_| McpError::internal_error("invalid Manager packet"))?;
    // Reserve content[0] for the entire Agent Turn, ACK and evidence descriptors.
    // Walk every object/array occurrence; extracting only observation would miss
    // the original source retained through commit, recovery and offline queries.
    let original = packet.clone();
    let mut texts = vec![String::new()];
    if detach(&mut packet, "", &mut texts).is_some() {
        texts[0] = packet.to_string();
        let result = complete(texts);
        if fits(&result) {
            return Ok(result);
        }
    }
    // Remaining metadata can itself exceed a string/envelope bound. Use the
    // same bounded AgentTurnBuilder fallback; preserve the original ACK/key and
    // uncertainty without claiming omitted evidence was delivered or verified.
    let turn = &original["agent_turn"];
    let session = turn["session_id"].as_str().and_then(|s| parse_id(s).ok());
    let request = turn["request_id"]
        .as_str()
        .and_then(|s| u128::from_str_radix(s, 16).ok())
        .map(RequestId::new);
    let fallback = complete(vec![overflow_packet(
        turn["operation"].as_str().unwrap_or("fortress.query"),
        session,
        request,
        original["retained_reference"].clone(),
    )]);
    if fits(&fallback) {
        Ok(fallback)
    } else {
        Err(McpError::internal_error(
            "Manager custody reference exceeds output bound",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fastmcp_rust::{
        JsonRpcMessage, JsonRpcResponse, MAX_RAW_JSON_NUMBER_BYTES, RequestId as RpcId,
        decode_strict_jsonrpc_message,
    };
    #[test]
    fn manager_numeric_id_actual_pinned_codec_and_exact_envelope_budget_boundary() {
        assert_eq!(MAX_JSONRPC_ID_BYTES, MAX_RAW_JSON_NUMBER_BYTES);
        let lexeme = "9".repeat(MAX_RAW_JSON_NUMBER_BYTES);
        let request = format!(r#"{{"jsonrpc":"2.0","method":"tools/list","id":{lexeme}}}"#);
        let JsonRpcMessage::Request(request) =
            decode_strict_jsonrpc_message(request.as_bytes(), 8192)
                .expect("pinned numeric request admission")
        else {
            panic!("request")
        };
        let id = request.id.expect("admitted correlation ID");
        assert_eq!(id, RpcId::Integer(lexeme.clone()));
        let too_wide = format!(r#"{{"jsonrpc":"2.0","method":"tools/list","id":{lexeme}9}}"#);
        assert!(decode_strict_jsonrpc_message(too_wide.as_bytes(), 8192).is_err());
        let encode = |result: CompleteResult<FinalCallToolResult>| {
            CoreResult::Final(FinalCoreResult::ToolsCall {
                result,
                diagnostic: None,
            })
            .encode()
            .expect("actual guarded tools/call codec")
        };
        // Synthetic valid MCP text blocks isolate the envelope boundary. These
        // are not native captures or proof of a naturally reachable game packet.
        let mut blocks = vec!["x".repeat(48_000); 10];
        blocks.push(String::new());
        let base = encode(complete(blocks.clone())).len();
        let exact = RESPONSE_BYTES as usize - ENVELOPE_BYTES;
        assert!(base < exact && exact - base < 48_000);
        for extra in [0usize, 1] {
            blocks[10] = "x".repeat(exact - base + extra);
            let result = complete(blocks.clone());
            let body = encode(result.clone());
            assert_eq!(body.len(), exact + extra);
            assert_eq!(fits(&result), extra == 0);
            let value: Value = serde_json::from_str(&body).expect("guarded result JSON");
            let response = JsonRpcResponse::success(id.clone(), value);
            let mut wire =
                serde_json::to_string(&response).expect("actual pinned numeric response codec");
            wire.push('\n');
            assert_eq!(wire.len(), RESPONSE_BYTES as usize + extra);
            assert!(
                wire.len() < 1024 * 1024,
                "product-budget boundary is below transport ceiling"
            );
            let JsonRpcMessage::Response(decoded) =
                decode_strict_jsonrpc_message(wire.as_bytes(), 1024 * 1024)
                    .expect("pinned response admission")
            else {
                panic!("response")
            };
            assert_eq!(decoded.id, Some(id.clone()));
            assert!(wire.contains(&lexeme));
            if extra == 1 {
                let old_reserve = ENVELOPE_BYTES - MAX_JSONRPC_ID_BYTES + 256;
                assert!(
                    body.len() + old_reserve <= RESPONSE_BYTES as usize,
                    "old string-only reserve admitted this oversize envelope"
                );
            }
        }
    }
}
