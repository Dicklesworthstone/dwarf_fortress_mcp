use dfmcp_core::{DfmcpError, EdgeId, EntityId, ErrorCode, Result};

use crate::{EdgeKind, EntityKind, EntityRecord, Value, WorldSnapshot};

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
        return if all { Predicate::True } else { Predicate::False };
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
        Predicate::EdgeExists { edge_id, kind } => snapshot
            .graph
            .edges
            .get(edge_id)
            .is_some_and(|edge| match kind.as_ref() {
                Some(expected) => &edge.kind == expected,
                None => true,
            }),
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
}

impl WorldQuery {
    pub fn validate(&self, hard_limit: u32) -> Result<()> {
        if self.limit == 0 || self.limit > hard_limit {
            return Err(DfmcpError::new(
                ErrorCode::InvalidRequest,
                format!(
                    "query limit must be between 1 and the negotiated hard limit {hard_limit}"
                ),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueryResult {
    pub entities: Vec<EntityRecord>,
    pub matched: u64,
    pub truncated: bool,
}

pub fn execute_query(
    snapshot: &WorldSnapshot,
    query: &WorldQuery,
    hard_limit: u32,
) -> Result<QueryResult> {
    query.validate(hard_limit)?;
    let mut entities: Vec<_> = snapshot
        .graph
        .entities
        .values()
        .filter(|entity| query.kinds.is_empty() || query.kinds.contains(&entity.kind))
        .filter(|entity| match query.predicate.as_ref() {
            Some(predicate) => evaluate_for_candidate(snapshot, entity.id, predicate),
            None => true,
        })
        .cloned()
        .collect();
    let matched = entities.len() as u64;
    match query.order {
        QueryOrder::EntityIdAscending => entities.sort_by_key(|entity| entity.id),
        QueryOrder::EntityIdDescending => {
            entities.sort_by(|left, right| right.id.cmp(&left.id));
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
                left.label.cmp(&right.label).then_with(|| left.id.cmp(&right.id))
            });
        }
    }
    let limit = query.limit as usize;
    let truncated = entities.len() > limit;
    entities.truncate(limit);
    Ok(QueryResult {
        entities,
        matched,
        truncated,
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

    use dfmcp_core::{Digest32, EntityId, FortressId, GameTick, ObservationCursor};

    use super::{execute_query, CompareOp, Predicate, QueryOrder, WorldQuery};
    use crate::{EntityKind, EntityRecord, Fact, FactSource, Value, WorldGraph, WorldSnapshot};

    fn snapshot() -> WorldSnapshot {
        let mut graph = WorldGraph::default();
        for (id, label, stress) in [(1, "Urist", 50), (2, "Domas", 10)] {
            let mut fields = BTreeMap::new();
            fields.insert(
                "stress".to_owned(),
                Fact {
                    value: Value::I64(stress),
                    observed_at: GameTick(1),
                    source: FactSource::Replay,
                    source_digest: Digest32::ZERO,
                },
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
}
