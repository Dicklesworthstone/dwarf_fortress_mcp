//! Bounded, lossless discovery of the exact spatial query schema.
use dfmcp_core::{DfmcpError, Digest32, ErrorCode, OperationContext, Result};
use serde_json::{Value, json};

// A tool result is one MCP text string. Leave ample room for the complete
// Agent Turn and JSON escaping under the pinned transport's 65,536-byte limit.
pub(super) const MAX_INLINE_TEXT: usize = 60_000;
const CHUNK_BYTES: usize = 1024;
const MAX_SCHEMA_BYTES: usize = 1_048_576;
const PREFIX: &str = "schema.page.";

pub(super) fn is_schema_mode(mode: Option<&str>) -> bool {
    mode.is_some_and(|m| m == "schema" || m.starts_with(PREFIX))
}

fn invalid() -> DfmcpError {
    DfmcpError::new(
        ErrorCode::InvalidRequest,
        "invalid or stale schema continuation",
    )
}

fn seal(c: &OperationContext, digest: Digest32, offset: usize) -> Digest32 {
    let mut bytes = Vec::with_capacity(128);
    bytes.extend_from_slice(b"dfmcp-spatial-schema-page/1\0");
    bytes.extend_from_slice(&c.session_id.get().to_be_bytes());
    bytes.extend_from_slice(&c.anchor.fortress_id.get().to_be_bytes());
    bytes.extend_from_slice(&c.anchor.cursor.epoch.to_be_bytes());
    bytes.extend_from_slice(&c.anchor.cursor.sequence.to_be_bytes());
    bytes.extend_from_slice(&c.anchor.tick.0.to_be_bytes());
    bytes.extend_from_slice(c.anchor.state_hash.as_bytes());
    bytes.extend_from_slice(digest.as_bytes());
    bytes.extend_from_slice(&(offset as u64).to_be_bytes());
    Digest32::of_bytes(&bytes)
}

/// Reassemble `chunk` strings in offset order, then verify `schema_sha256`
/// over the concatenated UTF-8 bytes before parsing the JSON schema.
pub(super) fn page(
    schema: &Value,
    mode: &str,
    c: &OperationContext,
    profile: &str,
    stale: bool,
) -> Result<Value> {
    let bytes = schema.to_string();
    if bytes.len() > MAX_SCHEMA_BYTES {
        return Err(DfmcpError::new(
            ErrorCode::BudgetExceeded,
            "query schema exceeds discovery bound",
        ));
    }
    let digest = Digest32::of_bytes(bytes.as_bytes());
    let offset = if mode == "schema" {
        0
    } else {
        let token = mode.strip_prefix(PREFIX).ok_or_else(invalid)?;
        if token.len() > 75 {
            return Err(invalid());
        }
        let (raw, signature) = token.split_once('.').ok_or_else(invalid)?;
        if raw.is_empty()
            || raw.len() > 10
            || (raw.len() > 1 && raw.starts_with('0'))
            || !raw.bytes().all(|x| x.is_ascii_digit())
            || signature.len() != 64
            || !signature
                .bytes()
                .all(|x| x.is_ascii_digit() || (b'a'..=b'f').contains(&x))
        {
            return Err(invalid());
        }
        let offset: usize = raw.parse().map_err(|_| invalid())?;
        if offset == 0
            || offset >= bytes.len()
            || !bytes.is_char_boundary(offset)
            || signature != seal(c, digest, offset).to_string()
        {
            return Err(invalid());
        }
        offset
    };
    let mut end = (offset + CHUNK_BYTES).min(bytes.len());
    while !bytes.is_char_boundary(end) {
        end -= 1;
    }
    let next = if end < bytes.len() {
        Some(format!("{PREFIX}{end}.{}", seal(c, digest, end)))
    } else {
        None
    };
    Ok(
        json!({"mode":"schema", "profile":profile, "source_stale":stale,
        "schema_discovery":"utf8-json-fragments/1", "schema_sha256":digest.to_string(),
        "schema_bytes":bytes.len(), "offset":offset, "chunk":&bytes[offset..end],
        "truncated":next.is_some(), "continuation":next,
        "continuation_tool":"fortress.query", "continuation_argument":"mode"}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use dfmcp_core::{
        FortressId, GameTick, ObservationCursor, RequestId, SessionId, StateAnchor, WorkBudget,
    };
    use fastmcp_rust::{FinalCallToolResult, ResultPeerEra, decode_typed_complete};

    fn context() -> OperationContext {
        OperationContext {
            session_id: SessionId::new(7),
            request_id: RequestId::new(1),
            anchor: StateAnchor {
                fortress_id: FortressId::new(2),
                cursor: ObservationCursor::ORIGIN,
                tick: GameTick(1),
                state_hash: Digest32::of_bytes(b"anchor"),
            },
            budget: WorkBudget::default(),
            grants: vec![],
            cancellation_requested: false,
        }
    }

    #[test]
    fn actual_schema_roundtrips_and_pages_pass_pinned_result_decoder() -> Result<()> {
        let schema = super::super::watch_batch::extend_schema(
            super::super::workforce_queries::extend_schema(super::super::history::schema()?)?,
        )?;
        let expected = schema.to_string();
        let c = context();
        let mut mode = "schema".to_owned();
        let mut output = String::new();
        loop {
            let part = page(&schema, &mode, &c, "spatial/1.8", false)?;
            assert_eq!(part["offset"].as_u64(), Some(output.len() as u64));
            let chunk = part["chunk"].as_str().ok_or_else(invalid)?;
            output.push_str(chunk);
            let text = part.to_string();
            assert!(text.len() < MAX_INLINE_TEXT);
            let envelope = json!({"resultType":"complete","content":[{"type":"text","text":text}],"isError":false}).to_string();
            assert!(
                decode_typed_complete::<FinalCallToolResult>(&envelope, ResultPeerEra::Modern)
                    .is_ok()
            );
            match part["continuation"].as_str() {
                Some(next) => mode = next.to_owned(),
                None => break,
            }
        }
        assert_eq!(output, expected);
        assert_eq!(
            Digest32::of_bytes(output.as_bytes()),
            Digest32::of_bytes(expected.as_bytes())
        );
        Ok(())
    }

    #[test]
    fn utf8_boundaries_and_context_schema_binding_fail_closed() -> Result<()> {
        let schema = json!({"padding": "x".repeat(1022) + "🦊" + &"y".repeat(3000)});
        let c = context();
        let first = page(&schema, "schema", &c, "spatial/1.8", false)?;
        let next = first["continuation"].as_str().ok_or_else(invalid)?;
        let mut reconstructed = first["chunk"].as_str().ok_or_else(invalid)?.to_owned();
        let mut cursor = Some(next.to_owned());
        while let Some(mode) = cursor {
            let part = page(&schema, &mode, &c, "spatial/1.8", false)?;
            reconstructed.push_str(part["chunk"].as_str().ok_or_else(invalid)?);
            cursor = part["continuation"].as_str().map(str::to_owned);
        }
        assert_eq!(reconstructed, schema.to_string());
        let mut other = c.clone();
        other.session_id = SessionId::new(8);
        assert!(page(&schema, next, &other, "spatial/1.8", false).is_err());
        other = c.clone();
        other.anchor.tick = GameTick(2);
        assert!(page(&schema, next, &other, "spatial/1.8", false).is_err());
        assert!(page(&json!({"changed":true}), next, &c, "spatial/1.8", false).is_err());
        assert!(page(&schema, "schema.page.01.bad", &c, "spatial/1.8", false).is_err());
        assert!(
            page(
                &schema,
                &format!("schema.page.1.{}", "a".repeat(4096)),
                &c,
                "spatial/1.8",
                false
            )
            .is_err()
        );
        Ok(())
    }
}
