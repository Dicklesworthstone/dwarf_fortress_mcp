#![forbid(unsafe_code)]

use dfmcp_core::{DfmcpError, EdgeId, EntityId, ErrorCode, Result};

use crate::{EdgeKind, EntityKind, EntityRecord, Value, WorldSnapshot};

const MAX_QUERY_KINDS: usize = 64;
const MAX_QUERY_PREDICATE_DEPTH: usize = 64;
const MAX_QUERY_PREDICATE_NODES: usize = 4_096;
const MAX_QUERY_FIELD_BYTES: usize = 256;
const MAX_QUERY_KIND_BYTES: usize = 128;
const MAX_QUERY_VALUE_DEPTH: usize = 64;
const MAX_QUERY_VALUE_NODES: usize = 4_096;
const MAX_QUERY_VALUE_BYTES: usize = 64 * 1_024;
const MAX_QUERY_SCAN_ENTITIES: usize = 1_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CompareOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Predicate {
    True,
    False,
    EntityExists(EntityId),
    EntityKind {
        entity_id: EntityId,
        kind: EntityKind,
    },
    FieldCompare {
        entity_id: EntityId,
        field: String,
        op: CompareOp,
        value: Value,
    },
    EdgeExists {
        edge_id: EdgeId,
        kind: Option<EdgeKind>,
    },
    Paused(bool),
    All(Vec<Predicate>),
    Any(Vec<Predicate>),
    Not(Box<Predicate>),
}

impl Predicate {
    /// Validates the bounded shape required before recursively evaluating,
    /// normalizing, or canonically encoding a predicate.
    pub fn validate_shape(&self) -> Result<()> {
        validate_predicate_shape(self)
    }

    #[must_use]
    pub fn depth(&self) -> usize {
        match self {
            Self::All(children) | Self::Any(children) => children
                .iter()
                .map(Predicate::depth)
                .fold(0, usize::max)
                .saturating_add(1),
            Self::Not(inner) => inner.depth().saturating_add(1),
            _ => 1,
        }
    }

    #[must_use]
    pub fn complexity(&self) -> usize {
        match self {
            Self::All(children) | Self::Any(children) => {
                children.iter().fold(1usize, |complexity, child| {
                    complexity.saturating_add(child.complexity())
                })
            }
            Self::Not(inner) => 1usize.saturating_add(inner.complexity()),
            _ => 1,
        }
    }

    #[must_use]
    pub fn normalized(&self) -> Self {
        match self {
            Self::All(predicates) => normalize_variadic(predicates, true),
            Self::Any(predicates) => normalize_variadic(predicates, false),
            Self::Not(predicate) => match predicate.normalized() {
                Self::True => Self::False,
                Self::False => Self::True,
                Self::Not(inner) => *inner,
                normalized => Self::Not(Box::new(normalized)),
            },
            other => other.clone(),
        }
    }

    #[must_use]
    pub fn canonical_bytes(&self) -> Vec<u8> {
        encoded_bytes(&self.normalized())
    }

    fn encode(&self, output: &mut Vec<u8>) {
        use crate::canonical::{put_str, put_u64};
        match self {
            Self::True => output.push(0),
            Self::False => output.push(1),
            Self::EntityExists(entity_id) => {
                output.push(2);
                put_u64(output, entity_id.get());
            }
            Self::EntityKind { entity_id, kind } => {
                output.push(3);
                put_u64(output, entity_id.get());
                kind.encode(output);
            }
            Self::FieldCompare {
                entity_id,
                field,
                op,
                value,
            } => {
                output.push(4);
                put_u64(output, entity_id.get());
                put_str(output, field);
                output.push(match op {
                    CompareOp::Eq => 0,
                    CompareOp::Ne => 1,
                    CompareOp::Lt => 2,
                    CompareOp::Le => 3,
                    CompareOp::Gt => 4,
                    CompareOp::Ge => 5,
                });
                value.encode(output);
            }
            Self::EdgeExists { edge_id, kind } => {
                output.push(5);
                output.extend_from_slice(&edge_id.get().to_be_bytes());
                match kind {
                    Some(kind) => {
                        output.push(1);
                        kind.encode(output);
                    }
                    None => output.push(0),
                }
            }
            Self::Paused(value) => {
                output.push(6);
                output.push(u8::from(*value));
            }
            Self::All(predicates) => {
                output.push(7);
                put_u64(output, predicates.len() as u64);
                for predicate in predicates {
                    predicate.encode(output);
                }
            }
            Self::Any(predicates) => {
                output.push(8);
                put_u64(output, predicates.len() as u64);
                for predicate in predicates {
                    predicate.encode(output);
                }
            }
            Self::Not(predicate) => {
                output.push(9);
                predicate.encode(output);
            }
        }
    }
}

fn encoded_bytes(predicate: &Predicate) -> Vec<u8> {
    let mut output = Vec::new();
    predicate.encode(&mut output);
    output
}

fn normalize_variadic(predicates: &[Predicate], all: bool) -> Predicate {
    let mut normalized = Vec::new();
    for predicate in predicates {
        let predicate = predicate.normalized();
        match (all, predicate) {
            (true, Predicate::False) => return Predicate::False,
            (false, Predicate::True) => return Predicate::True,
            (true, Predicate::True) | (false, Predicate::False) => {}
            (true, Predicate::All(children)) | (false, Predicate::Any(children)) => {
                normalized.extend(children);
            }
            (_, predicate) => normalized.push(predicate),
        }
    }
    normalized.sort_by_key(encoded_bytes);
    normalized.dedup();
    if normalized.is_empty() {
        return if all {
            Predicate::True
        } else {
            Predicate::False
        };
    }
    if normalized.len() == 1 {
        return normalized.remove(0);
    }
    if all {
        Predicate::All(normalized)
    } else {
        Predicate::Any(normalized)
    }
}

#[must_use]
pub fn evaluate(snapshot: &WorldSnapshot, predicate: &Predicate) -> bool {
    match predicate {
        Predicate::True => true,
        Predicate::False => false,
        Predicate::EntityExists(entity_id) => snapshot.graph.entities.contains_key(entity_id),
        Predicate::EntityKind { entity_id, kind } => snapshot
            .graph
            .entities
            .get(entity_id)
            .is_some_and(|entity| &entity.kind == kind),
        Predicate::FieldCompare {
            entity_id,
            field,
            op,
            value,
        } => snapshot
            .graph
            .entities
            .get(entity_id)
            .and_then(|entity| entity.fields.get(field))
            .is_some_and(|fact| compare(&fact.value, *op, value)),
        Predicate::EdgeExists { edge_id, kind } => {
            snapshot
                .graph
                .edges
                .get(edge_id)
                .is_some_and(|edge| match kind.as_ref() {
                    Some(expected) => &edge.kind == expected,
                    None => true,
                })
        }
        Predicate::Paused(expected) => snapshot.paused == *expected,
        Predicate::All(predicates) => predicates
            .iter()
            .all(|predicate| evaluate(snapshot, predicate)),
        Predicate::Any(predicates) => predicates
            .iter()
            .any(|predicate| evaluate(snapshot, predicate)),
        Predicate::Not(predicate) => !evaluate(snapshot, predicate),
    }
}

fn compare(left: &Value, op: CompareOp, right: &Value) -> bool {
    let ordering = match (left, right) {
        (Value::I64(left), Value::I64(right)) => left.partial_cmp(right),
        (Value::U64(left), Value::U64(right)) => left.partial_cmp(right),
        (
            Value::Fixed {
                units: left,
                scale: left_scale,
            },
            Value::Fixed {
                units: right,
                scale: right_scale,
            },
        ) if left_scale == right_scale => left.partial_cmp(right),
        (Value::Text(left), Value::Text(right)) => left.partial_cmp(right),
        (Value::Bool(left), Value::Bool(right)) => left.partial_cmp(right),
        (Value::Entity(left), Value::Entity(right)) => left.partial_cmp(right),
        (Value::Coord(left), Value::Coord(right)) => left.partial_cmp(right),
        _ => None,
    };
    match op {
        CompareOp::Eq => left == right,
        CompareOp::Ne => left != right,
        CompareOp::Lt => ordering.is_some_and(|value| value.is_lt()),
        CompareOp::Le => ordering.is_some_and(|value| value.is_le()),
        CompareOp::Gt => ordering.is_some_and(|value| value.is_gt()),
        CompareOp::Ge => ordering.is_some_and(|value| value.is_ge()),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QueryPlanCost {
    pub estimated_scanned_entities: usize,
    pub estimated_predicate_cost: usize,
    pub estimated_total_cost: usize,
}

impl QueryPlanCost {
    #[must_use]
    pub fn exceeds_budget(
        &self,
        max_complexity: usize,
        _max_depth: usize,
        max_entities: usize,
    ) -> bool {
        self.estimated_predicate_cost > max_complexity
            || self.estimated_scanned_entities > max_entities
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QueryCost {
    pub estimated_scanned_entities: u64,
    pub estimated_predicate_nodes: u64,
    pub exceeds_budget: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QueryOrder {
    EntityIdAscending,
    EntityIdDescending,
    RevisionDescending,
    LabelAscending,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorldQuery {
    pub kinds: Vec<EntityKind>,
    pub predicate: Option<Predicate>,
    pub order: QueryOrder,
    pub limit: u32,
    pub continuation: Option<String>,
}

impl WorldQuery {
    pub fn validate(&self, hard_limit: u32) -> Result<()> {
        if self.limit == 0 || self.limit > hard_limit {
            return Err(DfmcpError::new(
                ErrorCode::InvalidRequest,
                format!("query limit must be between 1 and the negotiated hard limit {hard_limit}"),
            ));
        }
        if self.kinds.len() > MAX_QUERY_KINDS
            || self
                .kinds
                .iter()
                .any(|kind| kind.as_str().len() > MAX_QUERY_KIND_BYTES)
        {
            return Err(DfmcpError::new(
                ErrorCode::BudgetExceeded,
                "query kind selector exceeds its explicit bound",
            ));
        }
        if let Some(continuation) = &self.continuation
            && continuation.len() > crate::delta::MAX_CONTINUATION_TOKEN_BYTES
        {
            return Err(DfmcpError::new(
                ErrorCode::BudgetExceeded,
                "query continuation token exceeds its explicit byte bound",
            ));
        }
        if let Some(predicate) = &self.predicate {
            predicate.validate_shape()?;
        }
        Ok(())
    }

    #[must_use]
    pub fn estimate_cost(&self, snapshot: &WorldSnapshot) -> QueryPlanCost {
        let estimated_scanned = snapshot
            .graph
            .entities
            .values()
            .filter(|e| self.kinds.is_empty() || self.kinds.contains(&e.kind))
            .count();

        let pred_cost = self.predicate.as_ref().map_or(1, Predicate::complexity);
        let total = estimated_scanned.saturating_mul(pred_cost).max(1);

        QueryPlanCost {
            estimated_scanned_entities: estimated_scanned,
            estimated_predicate_cost: pred_cost,
            estimated_total_cost: total,
        }
    }
}

fn validate_predicate_shape(root: &Predicate) -> Result<()> {
    let mut pending = vec![(root, 1usize)];
    let mut nodes = 0usize;
    while let Some((predicate, depth)) = pending.pop() {
        nodes = nodes.checked_add(1).ok_or_else(|| {
            DfmcpError::new(
                ErrorCode::BudgetExceeded,
                "query predicate node count overflowed",
            )
        })?;
        if depth > MAX_QUERY_PREDICATE_DEPTH || nodes > MAX_QUERY_PREDICATE_NODES {
            return Err(DfmcpError::new(
                ErrorCode::BudgetExceeded,
                "query predicate exceeds its explicit depth or node bound",
            ));
        }
        match predicate {
            Predicate::All(children) | Predicate::Any(children) => {
                if children.len() > MAX_QUERY_PREDICATE_NODES.saturating_sub(nodes) {
                    return Err(DfmcpError::new(
                        ErrorCode::BudgetExceeded,
                        "query predicate exceeds its explicit node bound",
                    ));
                }
                let child_depth = depth.checked_add(1).ok_or_else(|| {
                    DfmcpError::new(
                        ErrorCode::BudgetExceeded,
                        "query predicate depth overflowed",
                    )
                })?;
                pending.extend(children.iter().map(|child| (child, child_depth)));
            }
            Predicate::Not(child) => {
                let child_depth = depth.checked_add(1).ok_or_else(|| {
                    DfmcpError::new(
                        ErrorCode::BudgetExceeded,
                        "query predicate depth overflowed",
                    )
                })?;
                pending.push((child, child_depth));
            }
            Predicate::FieldCompare { field, .. } if field.len() > MAX_QUERY_FIELD_BYTES => {
                return Err(DfmcpError::new(
                    ErrorCode::BudgetExceeded,
                    "query field name exceeds its explicit byte bound",
                ));
            }
            Predicate::FieldCompare { value, .. } => validate_query_value(value)?,
            _ => {}
        }
    }
    Ok(())
}

fn validate_query_value(root: &Value) -> Result<()> {
    let mut pending = vec![(root, 1usize)];
    let mut nodes = 0usize;
    let mut bytes = 0usize;
    while let Some((value, depth)) = pending.pop() {
        nodes = nodes.checked_add(1).ok_or_else(|| {
            DfmcpError::new(
                ErrorCode::BudgetExceeded,
                "query predicate value node count overflowed",
            )
        })?;
        if depth > MAX_QUERY_VALUE_DEPTH || nodes > MAX_QUERY_VALUE_NODES {
            return Err(DfmcpError::new(
                ErrorCode::BudgetExceeded,
                "query predicate value exceeds its explicit depth or node bound",
            ));
        }
        let child_depth = depth.checked_add(1).ok_or_else(|| {
            DfmcpError::new(
                ErrorCode::BudgetExceeded,
                "query predicate value depth overflowed",
            )
        })?;
        match value {
            Value::Text(value) => add_query_value_bytes(&mut bytes, value.len())?,
            Value::Bytes(value) => add_query_value_bytes(&mut bytes, value.len())?,
            Value::List(values) => {
                if values.len() > MAX_QUERY_VALUE_NODES.saturating_sub(nodes) {
                    return Err(DfmcpError::new(
                        ErrorCode::BudgetExceeded,
                        "query predicate value exceeds its explicit node bound",
                    ));
                }
                pending.extend(values.iter().map(|value| (value, child_depth)));
            }
            Value::Object(values) => {
                if values.len() > MAX_QUERY_VALUE_NODES.saturating_sub(nodes) {
                    return Err(DfmcpError::new(
                        ErrorCode::BudgetExceeded,
                        "query predicate value exceeds its explicit node bound",
                    ));
                }
                for (key, value) in values {
                    add_query_value_bytes(&mut bytes, key.len())?;
                    pending.push((value, child_depth));
                }
            }
            Value::Null
            | Value::Bool(_)
            | Value::I64(_)
            | Value::U64(_)
            | Value::Fixed { .. }
            | Value::Entity(_)
            | Value::Coord(_) => {}
        }
    }
    Ok(())
}

fn add_query_value_bytes(total: &mut usize, additional: usize) -> Result<()> {
    *total = total.checked_add(additional).ok_or_else(|| {
        DfmcpError::new(
            ErrorCode::BudgetExceeded,
            "query predicate value byte count overflowed",
        )
    })?;
    if *total > MAX_QUERY_VALUE_BYTES {
        return Err(DfmcpError::new(
            ErrorCode::BudgetExceeded,
            "query predicate value exceeds its explicit byte bound",
        ));
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueryResult {
    pub entities: Vec<EntityRecord>,
    pub matched: u64,
    pub truncated: bool,
    pub continuation: Option<String>,
}

pub fn execute_query(
    snapshot: &WorldSnapshot,
    query: &WorldQuery,
    hard_limit: u32,
) -> Result<QueryResult> {
    execute_bounded_query(snapshot, query, hard_limit, None)
}

pub fn execute_bounded_query(
    snapshot: &WorldSnapshot,
    query: &WorldQuery,
    hard_limit: u32,
    byte_limit: Option<usize>,
) -> Result<QueryResult> {
    query.validate(hard_limit)?;
    if snapshot.graph.entities.len() > MAX_QUERY_SCAN_ENTITIES {
        return Err(DfmcpError::new(
            ErrorCode::BudgetExceeded,
            "query snapshot exceeds the implementation scan safety bound",
        ));
    }

    let offset = if let Some(ref cont) = query.continuation {
        let token = crate::ContinuationToken::decode(cont)?;
        if token.fortress_id != snapshot.fortress_id || token.cursor != snapshot.cursor {
            return Err(DfmcpError::new(
                ErrorCode::StaleAnchor,
                "continuation token anchor does not match target snapshot",
            ));
        }
        usize::try_from(token.offset).map_err(|_| {
            DfmcpError::new(
                ErrorCode::InvalidRequest,
                "query continuation offset cannot be represented on this platform",
            )
        })?
    } else {
        0
    };

    let mut entities: Vec<_> = snapshot
        .graph
        .entities
        .values()
        .filter(|entity| query.kinds.is_empty() || query.kinds.contains(&entity.kind))
        .filter(|entity| match query.predicate.as_ref() {
            Some(predicate) => evaluate_for_candidate(snapshot, entity.id, predicate),
            None => true,
        })
        .collect();

    let matched = u64::try_from(entities.len()).map_err(|_| {
        DfmcpError::new(
            ErrorCode::InternalInvariantViolation,
            "query match count cannot be represented",
        )
    })?;

    match query.order {
        QueryOrder::EntityIdAscending => entities.sort_by_key(|entity| entity.id),
        QueryOrder::EntityIdDescending => {
            entities.sort_by_key(|entity| std::cmp::Reverse(entity.id));
        }
        QueryOrder::RevisionDescending => {
            entities.sort_by(|left, right| {
                right
                    .revision
                    .cmp(&left.revision)
                    .then_with(|| left.id.cmp(&right.id))
            });
        }
        QueryOrder::LabelAscending => {
            entities.sort_by(|left, right| {
                left.label
                    .cmp(&right.label)
                    .then_with(|| left.id.cmp(&right.id))
            });
        }
    }

    if query.continuation.is_some() && offset >= entities.len() {
        return Err(DfmcpError::new(
            ErrorCode::CursorGap,
            "query continuation offset is at or beyond the deterministic result horizon",
        ));
    }
    let remaining = if offset < entities.len() {
        &entities[offset..]
    } else {
        &[]
    };

    let limit = query.limit as usize;
    let mut selected = Vec::new();
    let mut current_bytes = 0usize;
    let mut truncated = false;

    for entity in remaining {
        if selected.len() >= limit {
            truncated = true;
            break;
        }
        let entity_bytes = entity.canonical_bytes().len();
        let next_bytes = current_bytes.checked_add(entity_bytes).ok_or_else(|| {
            DfmcpError::new(
                ErrorCode::BudgetExceeded,
                "query output byte count overflowed",
            )
        })?;
        if let Some(max_bytes) = byte_limit
            && next_bytes > max_bytes
        {
            if selected.is_empty() {
                return Err(DfmcpError::new(
                    ErrorCode::BudgetExceeded,
                    "one query row exceeds the negotiated output byte bound",
                ));
            }
            truncated = true;
            break;
        }
        current_bytes = next_bytes;
        selected.push((*entity).clone());
    }

    let consumed = offset.checked_add(selected.len()).ok_or_else(|| {
        DfmcpError::new(
            ErrorCode::BudgetExceeded,
            "query continuation offset overflowed",
        )
    })?;
    if consumed < entities.len() {
        truncated = true;
    }

    let next_continuation = if truncated {
        let next_offset = u32::try_from(consumed).map_err(|_| {
            DfmcpError::new(
                ErrorCode::BudgetExceeded,
                "query continuation offset exceeds the wire representation",
            )
        })?;
        let token =
            crate::ContinuationToken::new(snapshot.fortress_id, snapshot.cursor, next_offset);
        Some(token.encode())
    } else {
        None
    };

    Ok(QueryResult {
        entities: selected,
        matched,
        truncated,
        continuation: next_continuation,
    })
}

fn evaluate_for_candidate(
    snapshot: &WorldSnapshot,
    candidate: EntityId,
    predicate: &Predicate,
) -> bool {
    match predicate {
        Predicate::EntityExists(entity_id) if *entity_id == EntityId::NIL => {
            snapshot.graph.entities.contains_key(&candidate)
        }
        Predicate::EntityKind { entity_id, kind } if *entity_id == EntityId::NIL => snapshot
            .graph
            .entities
            .get(&candidate)
            .is_some_and(|entity| &entity.kind == kind),
        Predicate::FieldCompare {
            entity_id,
            field,
            op,
            value,
        } if *entity_id == EntityId::NIL => snapshot
            .graph
            .entities
            .get(&candidate)
            .and_then(|entity| entity.fields.get(field))
            .is_some_and(|fact| compare(&fact.value, *op, value)),
        Predicate::All(predicates) => predicates
            .iter()
            .all(|predicate| evaluate_for_candidate(snapshot, candidate, predicate)),
        Predicate::Any(predicates) => predicates
            .iter()
            .any(|predicate| evaluate_for_candidate(snapshot, candidate, predicate)),
        Predicate::Not(predicate) => !evaluate_for_candidate(snapshot, candidate, predicate),
        _ => evaluate(snapshot, predicate),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use dfmcp_core::{Digest32, EntityId, ErrorCode, FortressId, GameTick, ObservationCursor};

    use super::{
        CompareOp, Predicate, QueryOrder, WorldQuery, execute_bounded_query, execute_query,
    };
    use crate::{EntityKind, EntityRecord, Fact, FactSource, Value, WorldGraph, WorldSnapshot};

    fn snapshot() -> WorldSnapshot {
        let mut graph = WorldGraph::default();
        for (id, label, stress) in [(1, "Urist", 50), (2, "Domas", 10)] {
            let mut fields = BTreeMap::new();
            fields.insert(
                "stress".to_owned(),
                Fact::known(
                    Value::I64(stress),
                    GameTick(1),
                    FactSource::Replay,
                    Digest32::ZERO,
                ),
            );
            graph.entities.insert(
                EntityId::new(id),
                EntityRecord {
                    id: EntityId::new(id),
                    generation: 1,
                    revision: 1,
                    kind: EntityKind::Unit,
                    label: label.to_owned(),
                    fields,
                },
            );
        }
        WorldSnapshot::new(
            FortressId::new(1),
            GameTick(1),
            ObservationCursor::ORIGIN,
            true,
            graph,
        )
    }

    #[test]
    fn candidate_relative_query_is_deterministic() -> Result<(), dfmcp_core::DfmcpError> {
        let query = WorldQuery {
            kinds: vec![EntityKind::Unit],
            predicate: Some(Predicate::FieldCompare {
                entity_id: EntityId::NIL,
                field: "stress".to_owned(),
                op: CompareOp::Gt,
                value: Value::I64(20),
            }),
            order: QueryOrder::EntityIdAscending,
            limit: 10,
            continuation: None,
        };
        let result = execute_query(&snapshot(), &query, 100)?;
        assert_eq!(result.matched, 1);
        assert_eq!(result.entities[0].label, "Urist");
        Ok(())
    }

    #[test]
    fn logical_predicates_have_order_independent_canonical_bytes() {
        let left = Predicate::All(vec![
            Predicate::Paused(true),
            Predicate::EntityExists(EntityId::new(7)),
            Predicate::Paused(true),
            Predicate::True,
        ]);
        let right = Predicate::All(vec![
            Predicate::EntityExists(EntityId::new(7)),
            Predicate::Paused(true),
        ]);
        assert_eq!(left.canonical_bytes(), right.canonical_bytes());
        assert_eq!(left.normalized(), right.normalized());
    }

    #[test]
    fn byte_bound_never_returns_an_oversized_first_row() {
        let query = WorldQuery {
            kinds: vec![EntityKind::Unit],
            predicate: None,
            order: QueryOrder::EntityIdAscending,
            limit: 10,
            continuation: None,
        };
        let result = execute_bounded_query(&snapshot(), &query, 100, Some(1));
        assert!(result.is_err());
    }

    #[test]
    fn continuation_past_result_set_is_a_cursor_gap() {
        let query = WorldQuery {
            kinds: vec![EntityKind::Unit],
            predicate: None,
            order: QueryOrder::EntityIdAscending,
            limit: 10,
            continuation: Some("cont:1:0:0:3".to_owned()),
        };
        let result = execute_query(&snapshot(), &query, 100);
        let is_ok = result.is_ok();
        let error = match result {
            Ok(_) => {
                assert!(!is_ok, "offset beyond the two-row result must fail");
                return;
            }
            Err(error) => error,
        };
        assert_eq!(error.code, ErrorCode::CursorGap);
    }

    #[test]
    fn continuation_at_result_horizon_is_a_cursor_gap() {
        let query = WorldQuery {
            kinds: vec![EntityKind::Unit],
            predicate: None,
            order: QueryOrder::EntityIdAscending,
            limit: 10,
            continuation: Some("cont:1:0:0:2".to_owned()),
        };
        let result = execute_query(&snapshot(), &query, 100);
        assert!(matches!(result, Err(ref error) if error.code == ErrorCode::CursorGap));
    }

    #[test]
    fn deeply_nested_predicate_is_rejected_before_recursive_evaluation() {
        let mut predicate = Predicate::True;
        for _ in 0..65 {
            predicate = Predicate::Not(Box::new(predicate));
        }
        let query = WorldQuery {
            kinds: Vec::new(),
            predicate: Some(predicate),
            order: QueryOrder::EntityIdAscending,
            limit: 1,
            continuation: None,
        };
        assert!(
            matches!(query.validate(10), Err(ref error) if error.code == ErrorCode::BudgetExceeded)
        );
    }

    #[test]
    fn deeply_nested_predicate_value_is_rejected_before_encoding() {
        let mut value = Value::Null;
        for _ in 0..65 {
            value = Value::List(vec![value]);
        }
        let predicate = Predicate::FieldCompare {
            entity_id: EntityId::NIL,
            field: "bounded".to_owned(),
            op: CompareOp::Eq,
            value,
        };
        assert!(
            matches!(predicate.validate_shape(), Err(ref error) if error.code == ErrorCode::BudgetExceeded)
        );
    }
}
