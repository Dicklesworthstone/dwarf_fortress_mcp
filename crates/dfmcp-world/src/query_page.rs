//! Snapshot- and query-bound pagination over the private reference evaluator.
//!
//! These deterministic commitments detect accidental cursor reuse and edits;
//! they are not authentication tokens. The effect shell must still authorize
//! every page and bind any externally exposed continuation to its session.

use dfmcp_core::{DfmcpError, Digest32, ErrorCode, Result};

use crate::canonical::{put_bytes, put_str, put_u32, put_u64};
use crate::{ContinuationToken, EdgeKind, EntityKind, Predicate, QueryOrder, QueryResult};
use crate::{WorldQuery, WorldSnapshot};

const MAX_QUERY_WORK: usize = 4_000_000;
const MAX_QUERY_IDENTITY_BYTES: usize = 256 * 1024;
const MAX_KIND_BYTES: usize = 128;
const PREFIX: &str = "q1";

/// Execute a bounded page. A continuation resumes the same filters and order
/// at the same fortress, epoch, sequence, game tick, and semantic state hash.
/// Page width and byte budget may change without changing the result set.
pub fn execute_query(
    snapshot: &WorldSnapshot,
    query: &WorldQuery,
    hard_limit: u32,
) -> Result<QueryResult> {
    execute_bounded_query(snapshot, query, hard_limit, None)
}

/// Like `execute_query`, also enforcing a whole-row canonical byte budget.
/// Legacy offset-only continuations require restarting the query.
pub fn execute_bounded_query(
    snapshot: &WorldSnapshot,
    query: &WorldQuery,
    hard_limit: u32,
    byte_limit: Option<usize>,
) -> Result<QueryResult> {
    query.validate(hard_limit)?;
    let per_entity = query
        .kinds
        .len()
        .max(1)
        .saturating_add(query.predicate.as_ref().map_or(1, Predicate::complexity));
    if snapshot.graph.entities.len().saturating_mul(per_entity) > MAX_QUERY_WORK {
        return Err(DfmcpError::new(
            ErrorCode::BudgetExceeded,
            "query scan and predicate work exceed the conservative operation budget",
        ));
    }
    if !snapshot.hash_is_valid() {
        return Err(DfmcpError::new(
            ErrorCode::InternalInvariantViolation,
            "query snapshot state hash is invalid",
        ));
    }
    let identity = query_identity(snapshot, query)?;
    let offset = match query.continuation.as_deref() {
        Some(token) => decode_cursor(token, identity)?,
        None => 0,
    };
    let mut inner = query.clone();
    inner.continuation = query
        .continuation
        .as_ref()
        .map(|_| ContinuationToken::new(snapshot.fortress_id, snapshot.cursor, offset).encode());
    let mut result = match byte_limit {
        Some(limit) => {
            crate::query::execute_bounded_query(snapshot, &inner, hard_limit, Some(limit))?
        }
        None => crate::query::execute_query(snapshot, &inner, hard_limit)?,
    };
    if let Some(token) = result.continuation.take() {
        let offset = ContinuationToken::decode(&token)?.offset;
        result.continuation = Some(encode_cursor(offset, identity));
    }
    Ok(result)
}

fn query_identity(snapshot: &WorldSnapshot, query: &WorldQuery) -> Result<Digest32> {
    let mut bytes = Vec::new();
    put_str(&mut bytes, "dfmcp-query-identity-v1");
    put_u64(&mut bytes, snapshot.fortress_id.get());
    put_u64(&mut bytes, snapshot.cursor.epoch);
    put_u64(&mut bytes, snapshot.cursor.sequence);
    put_u64(&mut bytes, snapshot.tick.0);
    put_bytes(&mut bytes, snapshot.state_hash.as_bytes());
    let mut kinds = query.kinds.iter().collect::<Vec<_>>();
    kinds.sort();
    kinds.dedup();
    put_u64(&mut bytes, kinds.len() as u64);
    for kind in kinds {
        encode_entity_kind(&mut bytes, kind)?;
    }
    bytes.push(match query.order {
        QueryOrder::EntityIdAscending => 0,
        QueryOrder::EntityIdDescending => 1,
        QueryOrder::RevisionDescending => 2,
        QueryOrder::LabelAscending => 3,
    });
    match &query.predicate {
        Some(predicate) => encode_predicate(&mut bytes, predicate)?,
        None => encode_predicate(&mut bytes, &Predicate::True)?,
    }
    Ok(Digest32::of_bytes(&bytes))
}

fn encode_entity_kind(bytes: &mut Vec<u8>, kind: &EntityKind) -> Result<()> {
    // Other("unit") is not the built-in Unit selector, even though both have
    // the same display name. Keep that distinction in the query commitment.
    bytes.push(u8::from(matches!(kind, EntityKind::Other(_))));
    encode_kind_name(bytes, kind.as_str())
}

fn encode_edge_kind(bytes: &mut Vec<u8>, kind: &EdgeKind) -> Result<()> {
    bytes.push(u8::from(matches!(kind, EdgeKind::Custom(_))));
    encode_kind_name(bytes, kind.as_str())
}

fn encode_kind_name(bytes: &mut Vec<u8>, name: &str) -> Result<()> {
    if name.len() > MAX_KIND_BYTES {
        return Err(DfmcpError::new(
            ErrorCode::BudgetExceeded,
            "query kind name exceeds its bound",
        ));
    }
    put_str(bytes, name);
    Ok(())
}

fn encode_predicate(bytes: &mut Vec<u8>, predicate: &Predicate) -> Result<()> {
    match predicate {
        Predicate::True => bytes.push(0),
        Predicate::False => bytes.push(1),
        Predicate::EntityExists(id) => {
            bytes.push(2);
            put_u64(bytes, id.get());
        }
        Predicate::EntityKind { entity_id, kind } => {
            bytes.push(3);
            put_u64(bytes, entity_id.get());
            encode_entity_kind(bytes, kind)?;
        }
        Predicate::FieldCompare {
            entity_id,
            field,
            op,
            value,
        } => {
            bytes.push(4);
            put_u64(bytes, entity_id.get());
            put_str(bytes, field);
            bytes.push(match op {
                crate::CompareOp::Eq => 0,
                crate::CompareOp::Ne => 1,
                crate::CompareOp::Lt => 2,
                crate::CompareOp::Le => 3,
                crate::CompareOp::Gt => 4,
                crate::CompareOp::Ge => 5,
            });
            value.encode(bytes);
        }
        Predicate::EdgeExists { edge_id, kind } => {
            bytes.push(5);
            bytes.extend_from_slice(&edge_id.get().to_be_bytes());
            match kind {
                Some(kind) => {
                    bytes.push(1);
                    encode_edge_kind(bytes, kind)?;
                }
                None => bytes.push(0),
            }
        }
        Predicate::Paused(paused) => {
            bytes.push(6);
            bytes.push(u8::from(*paused));
        }
        Predicate::All(children) | Predicate::Any(children) => {
            bytes.push(if matches!(predicate, Predicate::All(_)) {
                7
            } else {
                8
            });
            put_u64(bytes, children.len() as u64);
            for child in children {
                encode_predicate(bytes, child)?;
            }
        }
        Predicate::Not(child) => {
            bytes.push(9);
            encode_predicate(bytes, child)?;
        }
    }
    if bytes.len() > MAX_QUERY_IDENTITY_BYTES {
        return Err(DfmcpError::new(
            ErrorCode::BudgetExceeded,
            "query identity exceeds its aggregate byte bound",
        ));
    }
    Ok(())
}

fn cursor_digest(offset: u32, identity: Digest32) -> Digest32 {
    let mut bytes = Vec::new();
    put_str(&mut bytes, "dfmcp-query-cursor-v1");
    put_bytes(&mut bytes, identity.as_bytes());
    put_u32(&mut bytes, offset);
    Digest32::of_bytes(&bytes)
}

fn encode_cursor(offset: u32, identity: Digest32) -> String {
    format!("{PREFIX}:{offset}:{}", cursor_digest(offset, identity))
}

fn decode_cursor(token: &str, identity: Digest32) -> Result<u32> {
    if token.starts_with("cont:") || token.starts_with("offset:") {
        return Err(DfmcpError::new(
            ErrorCode::CursorGap,
            "legacy query continuation has no query/state binding; restart the query",
        ));
    }
    let mut parts = token.split(':');
    let (Some(prefix), Some(offset), Some(digest), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(DfmcpError::new(
            ErrorCode::InvalidRequest,
            "invalid query continuation shape",
        ));
    };
    if prefix != PREFIX
        || offset.is_empty()
        || !offset.bytes().all(|byte| byte.is_ascii_digit())
        || digest.len() != 64
        || !digest.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(DfmcpError::new(
            ErrorCode::InvalidRequest,
            "invalid query continuation encoding",
        ));
    }
    let parsed = offset.parse::<u32>().map_err(|_| {
        DfmcpError::new(
            ErrorCode::InvalidRequest,
            "query continuation offset exceeds u32",
        )
    })?;
    if parsed == 0 || offset.starts_with('0') {
        return Err(DfmcpError::new(
            ErrorCode::CursorGap,
            "query continuation offset is not canonical or makes no progress",
        ));
    }
    if digest != cursor_digest(parsed, identity).to_string() {
        return Err(DfmcpError::new(
            ErrorCode::StaleAnchor,
            "query continuation belongs to a different query or snapshot, or was modified",
        ));
    }
    Ok(parsed)
}
