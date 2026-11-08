//! Snapshot- and query-bound pagination over the private reference evaluator.
//!
//! These deterministic commitments detect accidental cursor reuse and edits;
//! they are not authentication tokens. The effect shell must still authorize
//! every page and bind any externally exposed continuation to its session.

use dfmcp_core::{DfmcpError, Digest32, ErrorCode, Result};

use crate::canonical::{put_bytes, put_str, put_u32, put_u64};
use crate::{
    ContinuationToken, EdgeKind, EntityKind, EntityRecord, Predicate, QueryOrder, QueryResult,
};
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
    let (inner, identity) = prepare_query(snapshot, query, hard_limit)?;
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

/// Borrowed candidates for a caller that budgets its own field projection.
/// Source validation, matching, ordering and cursor identity are identical to
/// `execute_query`; no complete entity is cloned or charged as rendered output.
/// The caller must admit source work and fit its entire output envelope before
/// publishing a prefix and the corresponding continuation.
#[derive(Debug)]
pub struct QueryCandidates<'a> {
    entities: Vec<&'a EntityRecord>,
    matched: u64,
    offset: usize,
    identity: Digest32,
}

impl<'a> QueryCandidates<'a> {
    #[must_use]
    pub fn entities(&self) -> &[&'a EntityRecord] {
        &self.entities
    }

    #[must_use]
    pub fn matched(&self) -> u64 {
        self.matched
    }

    /// Continue after exactly the published prefix. Empty-progress cursors and
    /// counts beyond this admitted page are refused, never used to skip rows.
    pub fn continuation_after(&self, published: usize) -> Result<Option<String>> {
        if published > self.entities.len() || (published == 0 && !self.entities.is_empty()) {
            return Err(DfmcpError::new(
                ErrorCode::InvalidRequest,
                "projected query continuation requires a nonempty admitted prefix",
            ));
        }
        let consumed = self.offset.checked_add(published).ok_or_else(|| {
            DfmcpError::new(ErrorCode::BudgetExceeded, "query page offset overflowed")
        })?;
        if (consumed as u64) < self.matched {
            let offset = u32::try_from(consumed).map_err(|_| {
                DfmcpError::new(ErrorCode::BudgetExceeded, "query page offset exceeds u32")
            })?;
            Ok(Some(encode_cursor(offset, self.identity)))
        } else {
            Ok(None)
        }
    }
}

/// Select at most the requested page width while borrowing the canonical
/// records. The retained candidates are O(page width); selection and sorting
/// still perform the same bounded full-source work as the canonical evaluator.
pub fn query_candidates<'a>(
    snapshot: &'a WorldSnapshot,
    query: &WorldQuery,
    hard_limit: u32,
) -> Result<QueryCandidates<'a>> {
    let (inner, identity) = prepare_query(snapshot, query, hard_limit)?;
    let (selected, offset) = crate::query::select_query_candidates(snapshot, &inner, hard_limit)?;
    let matched = u64::try_from(selected.len())
        .map_err(|_| DfmcpError::new(ErrorCode::BudgetExceeded, "query match count exceeds u64"))?;
    let entities = selected
        .into_iter()
        .skip(offset)
        .take(query.limit as usize)
        .collect();
    Ok(QueryCandidates {
        entities,
        matched,
        offset,
        identity,
    })
}

fn prepare_query(
    snapshot: &WorldSnapshot,
    query: &WorldQuery,
    hard_limit: u32,
) -> Result<(WorldQuery, Digest32)> {
    query.validate(hard_limit)?;
    let per_entity = query
        .kinds
        .len()
        .max(1)
        .saturating_add(query.predicate.as_ref().map_or(1, Predicate::complexity));
    let access_work = query.predicate.as_ref().map_or(0, |predicate| {
        predicate.additional_scan_work(snapshot.graph.entities.len())
    });
    if (snapshot.graph.entities.len() as u64)
        .saturating_mul((per_entity as u64).saturating_add(access_work))
        > MAX_QUERY_WORK as u64
    {
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
    Ok((inner, identity))
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
        Predicate::RegionTerrain { area, tile_code } => {
            bytes.push(10);
            for value in [
                area.min.x, area.min.y, area.min.z, area.max.x, area.max.y, area.max.z,
            ] {
                bytes.extend_from_slice(&value.to_be_bytes());
            }
            bytes.extend_from_slice(&tile_code.to_be_bytes());
        }
        Predicate::LaboratoryRoomAccess {
            area,
            entry,
            targets,
        }
        | Predicate::LaboratoryRoomAccessPairedStairs {
            area,
            entry,
            targets,
        } => {
            bytes.push(
                if matches!(predicate, Predicate::LaboratoryRoomAccess { .. }) {
                    11
                } else {
                    13
                },
            );
            for at in [area.min, area.max, *entry] {
                bytes.extend_from_slice(&at.x.to_be_bytes());
                bytes.extend_from_slice(&at.y.to_be_bytes());
                bytes.extend_from_slice(&at.z.to_be_bytes());
            }
            let mut targets = targets.clone();
            targets.sort_unstable_by_key(|target| (target.z, target.y, target.x));
            put_u64(bytes, targets.len() as u64);
            for at in targets {
                bytes.extend_from_slice(&at.x.to_be_bytes());
                bytes.extend_from_slice(&at.y.to_be_bytes());
                bytes.extend_from_slice(&at.z.to_be_bytes());
            }
        }
        Predicate::EntityQuantity {
            kind,
            filter_field,
            filter_value,
            quantity_field,
            op,
            value,
        } => {
            bytes.push(12);
            encode_entity_kind(bytes, kind)?;
            put_str(bytes, filter_field);
            filter_value.encode(bytes);
            match quantity_field {
                Some(field) => {
                    bytes.push(1);
                    put_str(bytes, field);
                }
                None => bytes.push(0),
            }
            bytes.push(match op {
                crate::CompareOp::Eq => 0,
                crate::CompareOp::Ne => 1,
                crate::CompareOp::Lt => 2,
                crate::CompareOp::Le => 3,
                crate::CompareOp::Gt => 4,
                crate::CompareOp::Ge => 5,
            });
            put_u64(bytes, *value);
        }
        Predicate::LaboratoryCivilianReleaseSafety { burrow, civilians } => {
            bytes.push(14);
            put_u64(bytes, burrow.id.get());
            bytes.extend_from_slice(&burrow.generation.to_be_bytes());
            let mut civilians = civilians.clone();
            civilians.sort_unstable();
            put_u64(bytes, civilians.len() as u64);
            for civilian in civilians {
                put_u64(bytes, civilian.id.get());
                bytes.extend_from_slice(&civilian.generation.to_be_bytes());
            }
        }
        Predicate::EntityQuantityIncrease {
            kind,
            filter_field,
            filter_value,
            quantity_field,
            baseline,
            increase,
            baseline_anchor,
            baseline_source_digest,
            source_generation,
            baseline_source_counters,
        } => {
            bytes.push(15);
            encode_entity_kind(bytes, kind)?;
            put_str(bytes, filter_field);
            filter_value.encode(bytes);
            match quantity_field {
                Some(field) => {
                    bytes.push(1);
                    put_str(bytes, field);
                }
                None => bytes.push(0),
            }
            put_u64(bytes, *baseline);
            put_u64(bytes, *increase);
            crate::canonical::put_anchor(bytes, *baseline_anchor);
            bytes.extend_from_slice(baseline_source_digest.as_bytes());
            put_u64(bytes, *source_generation);
            for counter in baseline_source_counters {
                put_u64(bytes, *counter);
            }
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

#[cfg(test)]
mod projected_tests {
    use super::*;
    use crate::{CompareOp, Fact, FactSource, Value, WorldGraph};
    use dfmcp_core::{EntityId, FortressId, GameTick, ObservationCursor};
    use std::collections::BTreeMap;

    fn snapshot() -> WorldSnapshot {
        let mut graph = WorldGraph::default();
        for (id, label, revision) in [
            (1, "Zon", 1),
            (2, "Urist", 3),
            (3, "Urist", 3),
            (4, "Domas", 2),
        ] {
            let id = EntityId::new(id);
            graph.entities.insert(
                id,
                EntityRecord {
                    id,
                    generation: 1,
                    revision,
                    kind: EntityKind::Unit,
                    label: label.to_owned(),
                    fields: BTreeMap::from([(
                        "payload".to_owned(),
                        Fact::known(
                            Value::Text("large unselected value".repeat(4096)),
                            GameTick(8),
                            FactSource::Replay,
                            Digest32::ZERO,
                        ),
                    )]),
                },
            );
        }
        WorldSnapshot::new(
            FortressId::new(1),
            GameTick(8),
            ObservationCursor::ORIGIN,
            true,
            graph,
        )
    }

    fn query(order: QueryOrder, limit: u32) -> WorldQuery {
        WorldQuery {
            kinds: vec![EntityKind::Unit],
            predicate: None,
            order,
            limit,
            continuation: None,
        }
    }

    #[test]
    fn borrowed_pages_preserve_reference_order_and_existing_cursor_bytes() -> Result<()> {
        let snapshot = snapshot();
        for order in [
            QueryOrder::EntityIdAscending,
            QueryOrder::EntityIdDescending,
            QueryOrder::RevisionDescending,
            QueryOrder::LabelAscending,
        ] {
            let mut selection = query(order, 3);
            let owned = execute_query(&snapshot, &selection, 10)?;
            let borrowed = query_candidates(&snapshot, &selection, 10)?;
            assert_eq!(borrowed.matched(), owned.matched);
            for (reference, owned) in borrowed.entities().iter().zip(&owned.entities) {
                assert_eq!(*reference, owned);
                assert!(std::ptr::eq(
                    *reference,
                    &snapshot.graph.entities[&reference.id]
                ));
            }
            assert_eq!(borrowed.continuation_after(3)?, owned.continuation);
            selection.limit = 1;
            let one = execute_query(&snapshot, &selection, 10)?;
            assert_eq!(borrowed.continuation_after(1)?, one.continuation);
            selection.continuation = borrowed.continuation_after(1)?;
            selection.limit = 3;
            let rest = query_candidates(&snapshot, &selection, 10)?;
            let reference = execute_query(&snapshot, &selection, 10)?;
            assert_eq!(rest.entities().len(), 3);
            assert_eq!(
                rest.entities()
                    .iter()
                    .map(|entity| entity.id)
                    .collect::<Vec<_>>(),
                reference
                    .entities
                    .iter()
                    .map(|entity| entity.id)
                    .collect::<Vec<_>>()
            );
            assert_eq!(rest.continuation_after(3)?, None);
        }
        Ok(())
    }

    #[test]
    fn borrowed_selection_never_copies_or_budgets_unselected_full_records() -> Result<()> {
        let snapshot = snapshot();
        let selection = query(QueryOrder::EntityIdAscending, 2);
        assert!(
            matches!(execute_bounded_query(&snapshot, &selection, 10, Some(1024)),
            Err(error) if error.code == ErrorCode::BudgetExceeded)
        );
        let page = query_candidates(&snapshot, &selection, 10)?;
        assert_eq!(page.entities().len(), 2);
        assert_eq!(page.matched(), 4);
        assert!(page.continuation_after(0).is_err());
        assert!(page.continuation_after(3).is_err());
        Ok(())
    }

    #[test]
    fn borrowed_selection_keeps_unknown_predicates_and_source_identity_fail_closed() -> Result<()> {
        let mut snapshot = snapshot();
        let mut selection = query(QueryOrder::EntityIdAscending, 2);
        let page = query_candidates(&snapshot, &selection, 10)?;
        selection.continuation = page.continuation_after(1)?;
        let mut changed = selection.clone();
        changed.order = QueryOrder::EntityIdDescending;
        assert!(matches!(query_candidates(&snapshot, &changed, 10),
            Err(error) if error.code == ErrorCode::StaleAnchor));
        snapshot.paused = false;
        assert!(matches!(query_candidates(&snapshot, &selection, 10),
            Err(error) if error.code == ErrorCode::InternalInvariantViolation));
        snapshot.refresh_hash();
        assert!(matches!(query_candidates(&snapshot, &selection, 10),
            Err(error) if error.code == ErrorCode::StaleAnchor));
        selection.continuation = None;
        selection.predicate = Some(Predicate::Not(Box::new(Predicate::FieldCompare {
            entity_id: EntityId::NIL,
            field: "missing".to_owned(),
            op: CompareOp::Eq,
            value: Value::Null,
        })));
        let empty = query_candidates(&snapshot, &selection, 10)?;
        assert_eq!(empty.matched(), 0);
        assert!(empty.entities().is_empty());
        assert_eq!(empty.continuation_after(0)?, None);
        Ok(())
    }
}
