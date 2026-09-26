//! Bounded structured inspection over one authorized canonical snapshot.
//! This presentation layer never reads the bridge or dispatches an effect.

use std::collections::BTreeMap;

use dfmcp_core::{
    Capability, DfmcpError, Digest32, EntityId, ErrorCode, MapCoord, OperationContext, Result,
    RiskTier, StateAnchor,
};
use dfmcp_world::graph_query::{
    GraphBudget, GraphDirection, GraphTraversalQuery, GraphWitness, analyze_dependencies,
    traverse_graph,
};
use dfmcp_world::{
    CompareOp, EdgeKind, EntityKind, EntityRecord, Fact, FactPresence, FactSource, Predicate,
    QueryOrder, Value as WorldValue, WorldQuery, WorldSnapshot, execute_bounded_query,
};
use serde::Deserialize;
use serde_json::{Value, json};

const SCHEMA: &str = "dfmcp.query/1";
const MAX_INPUT_BYTES: usize = 65_536;
const MAX_INPUT_NODES: usize = 4_096;
const MAX_INPUT_DEPTH: usize = 24;
const MAX_ROWS: u32 = 512;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    schema: String,
    expected_anchor: Option<Value>,
    query: Query,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Query {
    Entities {
        #[serde(default)]
        kinds: Vec<String>,
        #[serde(rename = "where")]
        predicate: Option<Filter>,
        #[serde(default)]
        fields: Vec<String>,
        order: Option<Order>,
        limit: Option<u32>,
        continuation: Option<String>,
    },
    Inspect {
        entity_id: String,
        generation: u32,
        #[serde(default)]
        fields: Vec<String>,
    },
    Traverse {
        roots: Vec<String>,
        #[serde(default)]
        edge_kinds: Vec<String>,
        direction: Option<Direction>,
        max_depth: u32,
        target: Option<String>,
    },
    Dependencies {
        edge_kinds: Vec<String>,
    },
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Order {
    IdAscending,
    IdDescending,
    LabelAscending,
    RevisionDescending,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Direction {
    Outgoing,
    Incoming,
    Undirected,
}

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
enum Filter {
    All {
        args: Vec<Filter>,
    },
    Any {
        args: Vec<Filter>,
    },
    Not {
        arg: Box<Filter>,
    },
    Compare {
        field: String,
        comparison: Comparison,
        value: Literal,
    },
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Comparison {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

#[derive(Deserialize)]
#[serde(
    tag = "type",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
enum Literal {
    Null,
    Bool(bool),
    I64(i64),
    U64(u64),
    Text(String),
    Fixed { units: i64, scale: u32 },
    Coord { x: i32, y: i32, z: i32 },
    Entity(String),
}

fn invalid(message: impl Into<String>) -> DfmcpError {
    DfmcpError::new(ErrorCode::InvalidRequest, message)
}

fn budget(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::BudgetExceeded, message)
}

/// Validate before recursive deserialization, cloning, or serialization.
fn validate_input(input: &Value) -> Result<()> {
    let mut pending = vec![(input, 0usize)];
    let mut nodes = 0usize;
    let mut bytes = 0usize;
    while let Some((value, depth)) = pending.pop() {
        nodes += 1;
        bytes = bytes.saturating_add(16);
        if depth > MAX_INPUT_DEPTH || nodes > MAX_INPUT_NODES {
            return Err(budget("structured query exceeds its depth or node bound"));
        }
        match value {
            Value::String(text) => bytes = bytes.saturating_add(text.len()),
            Value::Array(values) => {
                if values
                    .len()
                    .saturating_add(pending.len())
                    .saturating_add(nodes)
                    > MAX_INPUT_NODES
                {
                    return Err(budget("structured query exceeds its node bound"));
                }
                pending.extend(values.iter().map(|value| (value, depth + 1)));
            }
            Value::Object(values) => {
                if values
                    .len()
                    .saturating_add(pending.len())
                    .saturating_add(nodes)
                    > MAX_INPUT_NODES
                {
                    return Err(budget("structured query exceeds its node bound"));
                }
                for (key, value) in values {
                    bytes = bytes.saturating_add(key.len());
                    pending.push((value, depth + 1));
                }
            }
            Value::Number(number) => bytes = bytes.saturating_add(number.to_string().len()),
            _ => {}
        }
        if bytes > MAX_INPUT_BYTES {
            return Err(budget(
                "structured query exceeds its aggregate input-byte bound",
            ));
        }
    }
    Ok(())
}

fn bounded_names(mut names: Vec<String>, maximum: usize) -> Result<Vec<String>> {
    if names.len() > maximum
        || names
            .iter()
            .any(|name| name.is_empty() || name.len() > 128 || name.contains('\0'))
    {
        return Err(invalid(
            "selector names exceed their count or UTF-8 byte bound",
        ));
    }
    names.sort();
    names.dedup();
    Ok(names)
}

fn entity_id(raw: &str) -> Result<EntityId> {
    if raw.is_empty()
        || raw.len() > 20
        || raw.starts_with('0')
        || !raw.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(invalid(
            "entity IDs must be canonical positive decimal u64 strings",
        ));
    }
    raw.parse::<u64>()
        .map(EntityId::new)
        .map_err(|_| invalid("entity ID exceeds u64"))
}

fn entity_kind(raw: &str) -> Result<EntityKind> {
    Ok(match raw {
        "fortress" => EntityKind::Fortress,
        "unit" => EntityKind::Unit,
        "item" => EntityKind::Item,
        "building" => EntityKind::Building,
        "job" => EntityKind::Job,
        "work_order" => EntityKind::WorkOrder,
        "stockpile" => EntityKind::Stockpile,
        "zone" => EntityKind::Zone,
        "burrow" => EntityKind::Burrow,
        "squad" => EntityKind::Squad,
        "military_order" => EntityKind::MilitaryOrder,
        "tile_feature" => EntityKind::TileFeature,
        "plant" => EntityKind::Plant,
        "creature" => EntityKind::Creature,
        "historical_figure" => EntityKind::HistoricalFigure,
        "civilization" => EntityKind::Civilization,
        "announcement" => EntityKind::Announcement,
        "syndrome" => EntityKind::Syndrome,
        value if value.starts_with("other:") && value.len() > 6 => {
            EntityKind::Other(value[6..].to_owned())
        }
        _ => return Err(invalid(format!("unknown entity kind {raw:?}"))),
    })
}

fn edge_kind(raw: &str) -> Result<EdgeKind> {
    Ok(match raw {
        "located_at" => EdgeKind::LocatedAt,
        "contained_in" => EdgeKind::ContainedIn,
        "assigned_to" => EdgeKind::AssignedTo,
        "member_of" => EdgeKind::MemberOf,
        "performs" => EdgeKind::Performs,
        "requires" => EdgeKind::Requires,
        "produces" => EdgeKind::Produces,
        "uses" => EdgeKind::Uses,
        "supports" => EdgeKind::Supports,
        "threatens" => EdgeKind::Threatens,
        "ordered_by" => EdgeKind::OrderedBy,
        "parent_of" => EdgeKind::ParentOf,
        value if value.starts_with("custom:") && value.len() > 7 => {
            EdgeKind::Custom(value[7..].to_owned())
        }
        _ => return Err(invalid(format!("unknown edge kind {raw:?}"))),
    })
}

impl Literal {
    fn into_world(self) -> Result<WorldValue> {
        Ok(match self {
            Self::Null => WorldValue::Null,
            Self::Bool(value) => WorldValue::Bool(value),
            Self::I64(value) => WorldValue::I64(value),
            Self::U64(value) => WorldValue::U64(value),
            Self::Text(value) => WorldValue::Text(value),
            Self::Fixed { units, scale } => WorldValue::Fixed { units, scale },
            Self::Coord { x, y, z } => WorldValue::Coord(MapCoord::new(x, y, z)),
            Self::Entity(value) => WorldValue::Entity(entity_id(&value)?),
        })
    }
}

impl Filter {
    fn into_world(self) -> Result<Predicate> {
        Ok(match self {
            Self::All { args } => Predicate::All(
                args.into_iter()
                    .map(Self::into_world)
                    .collect::<Result<_>>()?,
            ),
            Self::Any { args } => Predicate::Any(
                args.into_iter()
                    .map(Self::into_world)
                    .collect::<Result<_>>()?,
            ),
            Self::Not { arg } => Predicate::Not(Box::new(arg.into_world()?)),
            Self::Compare {
                field,
                comparison,
                value,
            } => {
                if field.is_empty() || field.len() > 128 || field.contains('\0') {
                    return Err(invalid("predicate field name violates its byte bound"));
                }
                Predicate::FieldCompare {
                    entity_id: EntityId::NIL,
                    field,
                    op: match comparison {
                        Comparison::Eq => CompareOp::Eq,
                        Comparison::Ne => CompareOp::Ne,
                        Comparison::Lt => CompareOp::Lt,
                        Comparison::Le => CompareOp::Le,
                        Comparison::Gt => CompareOp::Gt,
                        Comparison::Ge => CompareOp::Ge,
                    },
                    value: value.into_world()?,
                }
            }
        })
    }
}

pub(crate) fn anchor_json(anchor: StateAnchor) -> Value {
    json!({"fortress_id": anchor.fortress_id.to_string(), "epoch": anchor.cursor.epoch,
        "sequence": anchor.cursor.sequence, "game_tick": anchor.tick.0,
        "state_hash": anchor.state_hash.to_string()})
}

fn world_value(value: &WorldValue) -> Value {
    match value {
        WorldValue::Null => json!({"type":"null"}),
        WorldValue::Bool(v) => json!({"type":"bool","value":v}),
        WorldValue::I64(v) => json!({"type":"i64","value":v}),
        WorldValue::U64(v) => json!({"type":"u64","value":v}),
        WorldValue::Text(v) => json!({"type":"text","value":v}),
        WorldValue::Fixed { units, scale } => {
            json!({"type":"fixed","value":{"units":units,"scale":scale}})
        }
        WorldValue::Entity(v) => json!({"type":"entity","value":v.to_string()}),
        WorldValue::Coord(v) => json!({"type":"coord","value":{"x":v.x,"y":v.y,"z":v.z}}),
        WorldValue::Bytes(v) => json!({"type":"bytes","value":v}),
        WorldValue::List(v) => {
            json!({"type":"list","value":v.iter().map(world_value).collect::<Vec<_>>()})
        }
        WorldValue::Object(v) => json!({"type":"object","value":v.iter()
            .map(|(key, value)| (key.clone(), world_value(value))).collect::<BTreeMap<_,_>>()}),
    }
}

fn fact_json(fact: &Fact) -> Value {
    let source_state = match &fact.source {
        FactSource::DfhackField(_) => "observed",
        FactSource::Derived(_) => "inferred",
        FactSource::AgentAssertion(_) | FactSource::Replay => "assumed",
    };
    let (presence, state, reason, known) = match &fact.presence {
        None => ("known", source_state, Value::Null, true),
        Some(FactPresence::Known(value)) if value == &fact.value => {
            ("known", source_state, Value::Null, true)
        }
        Some(FactPresence::Known(_)) => (
            "unknown",
            "contradicted",
            json!("conflicting known-value representations"),
            false,
        ),
        Some(FactPresence::Absent) => ("absent", source_state, Value::Null, false),
        Some(FactPresence::Unknown(reason)) => ("unknown", "unknown", json!(reason), false),
        Some(FactPresence::Unsupported(reason)) => ("unsupported", "unknown", json!(reason), false),
        Some(FactPresence::Omitted(reason)) => ("omitted", "unknown", json!(reason), false),
        Some(FactPresence::Redacted(reason)) => ("redacted", "unknown", json!(reason), false),
        Some(FactPresence::Stale(anchor)) => ("stale", "stale", anchor_json(*anchor), false),
    };
    json!({"presence":presence, "epistemic_state":state, "reason":reason,
        "value":if known { world_value(&fact.value) } else { Value::Null },
        "observed_at_game_tick":fact.observed_at.0, "source":format!("{:?}",fact.source),
        "source_digest":fact.source_digest.to_string()})
}

fn entity_json(entity: &EntityRecord, fields: &[String]) -> Value {
    let fields: BTreeMap<_, _> = fields
        .iter()
        .map(|field| {
            let fact = entity.fields.get(field).map_or_else(|| json!({
            "presence":"unknown", "epistemic_state":"unknown", "value":null,
            "reason":"field is not present in this observed projection; absence is not established"
        }), fact_json);
            (field.clone(), fact)
        })
        .collect();
    json!({"entity_id":entity.id.to_string(), "generation":entity.generation,
        "revision":entity.revision, "kind":entity.kind.as_str(), "label":entity.label,
        "fields":fields})
}

fn witness_json(witness: &GraphWitness) -> Value {
    json!({"anchor":anchor_json(witness.source_anchor),
        "authorization_scope_digest":witness.authorization_scope_digest.to_string(),
        "projection_digest":witness.projection_digest.to_string(),
        "decision_digest":witness.decision_digest.to_string(),
        "scanned_vertices":witness.scanned_vertices, "scanned_edges":witness.scanned_edges,
        "examined_arcs":witness.examined_arcs, "work_units":witness.work_units})
}

fn ids_json(ids: &[EntityId]) -> Vec<String> {
    ids.iter().map(ToString::to_string).collect()
}

fn session_projection(context: &OperationContext, fields: &[String]) -> Result<Digest32> {
    let bytes = serde_json::to_vec(&json!({"domain":"dfmcp-query-presentation-v1",
        "session":context.session_id.to_string(), "fields":fields}))
    .map_err(|_| invalid("query projection identity cannot be encoded"))?;
    Ok(Digest32::of_bytes(&bytes))
}

fn unwrap_continuation(raw: Option<String>, identity: Digest32) -> Result<Option<String>> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    if raw.len() > 256 {
        return Err(budget("query continuation exceeds its byte bound"));
    }
    let prefix = format!("sp1:{identity}:");
    let Some(inner) = raw.strip_prefix(&prefix) else {
        return Err(DfmcpError::new(
            ErrorCode::StaleAnchor,
            "continuation belongs to another session or field projection; restart this query",
        ));
    };
    if !inner.starts_with("q1:") {
        return Err(invalid("unsupported structured query continuation"));
    }
    Ok(Some(inner.to_owned()))
}

/// Execute only on a snapshot owned by the resolved server session. Whole-fortress
/// Query authority is required; scoped grants are not widened by empty selectors.
/// The caller must additionally enforce adapter health and bound the final MCP packet.
pub fn execute(
    snapshot: &WorldSnapshot,
    context: &OperationContext,
    input: &Value,
) -> Result<Value> {
    context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    if context.anchor != snapshot.anchor() {
        return Err(DfmcpError::new(
            ErrorCode::StaleAnchor,
            "query context does not name this snapshot",
        ));
    }
    if snapshot.graph.entities.len() > context.budget.max_entities as usize {
        return Err(budget("query exceeds the session entity-scan budget"));
    }
    validate_input(input)?;
    let request: Envelope = serde_json::from_value(input.clone())
        .map_err(|error| invalid(format!("invalid structured query: {error}")))?;
    if request.schema != SCHEMA {
        return Err(invalid("structured query schema must be dfmcp.query/1"));
    }
    if request
        .expected_anchor
        .as_ref()
        .is_some_and(|anchor| *anchor != anchor_json(snapshot.anchor()))
    {
        return Err(DfmcpError::new(
            ErrorCode::StaleAnchor,
            "expected_anchor does not match the complete current anchor",
        ));
    }
    if !snapshot.hash_is_valid() {
        return Err(DfmcpError::new(
            ErrorCode::InternalInvariantViolation,
            "query snapshot hash is invalid",
        ));
    }
    let max_bytes = context
        .budget
        .max_bytes
        .min(u64::from(context.budget.max_output_tokens).saturating_mul(4));
    let byte_limit =
        usize::try_from(max_bytes).map_err(|_| budget("query byte limit cannot be represented"))?;
    let graph_budget = GraphBudget {
        max_vertices: (context.budget.max_entities as usize).min(100_000),
        max_edges: (context.budget.max_entities as usize)
            .saturating_mul(8)
            .min(1_000_000),
        max_frontier: (context.budget.max_entities as usize).min(4_096),
        max_work: 2_000_000,
    };
    let scope = session_projection(context, &[])?;
    let mut payload = match request.query {
        Query::Entities {
            kinds,
            predicate,
            fields,
            order,
            limit,
            continuation,
        } => {
            let fields = bounded_names(fields, 32)?;
            let kinds = bounded_names(kinds, 32)?
                .iter()
                .map(|kind| entity_kind(kind))
                .collect::<Result<_>>()?;
            let identity = session_projection(context, &fields)?;
            let hard_limit = context.budget.max_entities.min(MAX_ROWS);
            let query = WorldQuery {
                kinds,
                predicate: predicate.map(Filter::into_world).transpose()?,
                order: match order {
                    None | Some(Order::IdAscending) => QueryOrder::EntityIdAscending,
                    Some(Order::IdDescending) => QueryOrder::EntityIdDescending,
                    Some(Order::LabelAscending) => QueryOrder::LabelAscending,
                    Some(Order::RevisionDescending) => QueryOrder::RevisionDescending,
                },
                limit: limit.map_or(hard_limit.min(4), |value| value),
                continuation: unwrap_continuation(continuation, identity)?,
            };
            let page = execute_bounded_query(snapshot, &query, hard_limit, Some(byte_limit))?;
            json!({"kind":"entities", "matched":page.matched, "returned":page.entities.len(),
                "truncated":page.truncated,
                "continuation":page.continuation.map(|token| format!("sp1:{identity}:{token}")),
                "rows":page.entities.iter().map(|entity| entity_json(entity,&fields)).collect::<Vec<_>>()})
        }
        Query::Inspect {
            entity_id: raw_id,
            generation,
            fields,
        } => {
            let fields = bounded_names(fields, 32)?;
            let id = entity_id(&raw_id)?;
            let entity = snapshot.graph.entities.get(&id).ok_or_else(|| {
                invalid(
                    "entity is not in the observed projection; world absence is not established",
                )
            })?;
            if entity.generation != generation {
                return Err(DfmcpError::new(
                    ErrorCode::Conflict,
                    "entity generation changed; refresh its handle before inspection",
                ));
            }
            json!({"kind":"inspect", "truncated":false, "continuation":null,
                "row":entity_json(entity,&fields)})
        }
        Query::Traverse {
            roots,
            edge_kinds,
            direction,
            max_depth,
            target,
        } => {
            if roots.len() > 64 {
                return Err(budget("graph root count exceeds 64"));
            }
            let roots = roots
                .iter()
                .map(|id| entity_id(id))
                .collect::<Result<_>>()?;
            let target = target.as_deref().map(entity_id).transpose()?;
            let edge_kinds = bounded_names(edge_kinds, 32)?
                .iter()
                .map(|kind| edge_kind(kind))
                .collect::<Result<_>>()?;
            let query = GraphTraversalQuery {
                roots,
                edge_kinds,
                max_depth,
                direction: match direction {
                    None | Some(Direction::Outgoing) => GraphDirection::Outgoing,
                    Some(Direction::Incoming) => GraphDirection::Incoming,
                    Some(Direction::Undirected) => GraphDirection::Undirected,
                },
            };
            let result = traverse_graph(snapshot, scope, &query, graph_budget)?;
            let path = target
                .map(|id| result.observed_path_to(id))
                .transpose()?
                .flatten();
            json!({"kind":"traverse", "truncated":!result.depth_frontier.is_empty(), "continuation":null,
                "depth_frontier":ids_json(&result.depth_frontier),
                "visits":result.visits.iter().map(|visit| json!({
                    "entity_id":visit.entity_id.to_string(), "generation":visit.generation,
                    "revision":visit.revision, "root":visit.root.to_string(), "depth":visit.depth,
                    "parent":visit.parent.map(|id| id.to_string()),
                    "via_edge":visit.via_edge.map(|id| id.to_string()), "via_edge_revision":visit.via_edge_revision
                })).collect::<Vec<_>>(),
                "path":path.map(|path| json!({"vertices":ids_json(&path.vertices),
                    "edges":path.edges.iter().map(|(id,revision)| json!({"edge_id":id.to_string(),"revision":revision})).collect::<Vec<_>>()})),
                "witness":witness_json(&result.witness)})
        }
        Query::Dependencies { edge_kinds } => {
            if edge_kinds.is_empty() {
                return Err(invalid(
                    "dependency analysis requires explicit dependent-to-prerequisite edge kinds",
                ));
            }
            let edge_kinds = bounded_names(edge_kinds, 32)?
                .iter()
                .map(|kind| edge_kind(kind))
                .collect::<Result<Vec<_>>>()?;
            let result = analyze_dependencies(snapshot, scope, &edge_kinds, graph_budget)?;
            json!({"kind":"dependencies", "truncated":false, "continuation":null,
                "components":result.components.iter().map(|component| json!({
                    "component_id":component.id.to_string(), "members":ids_json(&component.members), "cyclic":component.cyclic
                })).collect::<Vec<_>>(),
                "blocked_by_cycles":ids_json(&result.blocked_by_cycles),
                "dependency_order":result.dependency_order.as_deref().map(ids_json),
                "critical_chain":result.critical_chain.as_deref().map(ids_json),
                "witness":witness_json(&result.witness)})
        }
    };
    payload["schema"] = json!("dfmcp.query.result/1");
    payload["anchor"] = anchor_json(snapshot.anchor());
    payload["coverage"] = json!({"domain":"observed_projection", "absence_proven":false,
        "note":"filters, paths, and dependencies describe this projection only; missing data is not complete-world absence"});
    let encoded =
        serde_json::to_vec(&payload).map_err(|_| invalid("query result cannot be encoded"))?;
    if encoded.len() > byte_limit {
        return Err(budget(
            "rendered query result exceeds the output budget; reduce limit, selected fields, roots, or graph depth",
        ));
    }
    Ok(payload)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dfmcp_core::{
        CapabilityGrant, CapabilityScope, EdgeId, FortressId, GameTick, ObservationCursor,
        RequestId, SessionId, WorkBudget,
    };
    use dfmcp_world::{EdgeRecord, WorldGraph};
    use std::collections::BTreeSet;

    fn fixture() -> (WorldSnapshot, OperationContext) {
        let mut graph = WorldGraph::default();
        for id in 1..=4 {
            graph.entities.insert(
                EntityId::new(id),
                EntityRecord {
                    id: EntityId::new(id),
                    generation: 1,
                    revision: 1,
                    kind: EntityKind::Unit,
                    label: format!("Urist {id}"),
                    fields: BTreeMap::from([
                        (
                            "sane".to_owned(),
                            Fact::known(
                                WorldValue::Bool(id % 2 == 0),
                                GameTick(1),
                                FactSource::DfhackField("sane".to_owned()),
                                Digest32::of_bytes(b"fixture"),
                            ),
                        ),
                        (
                            "unknown".to_owned(),
                            Fact::with_presence(
                                FactPresence::Omitted("not projected".to_owned()),
                                GameTick(1),
                                FactSource::Replay,
                                Digest32::ZERO,
                            ),
                        ),
                    ]),
                },
            );
        }
        for (id, from, to) in [(1, 1, 2), (2, 2, 3), (3, 3, 2), (4, 4, 1)] {
            graph.edges.insert(
                EdgeId::new(id),
                EdgeRecord {
                    id: EdgeId::new(id),
                    revision: 1,
                    kind: EdgeKind::Requires,
                    from: EntityId::new(from),
                    to: EntityId::new(to),
                    fields: BTreeMap::new(),
                },
            );
        }
        let snapshot = WorldSnapshot::new(
            FortressId::new(1),
            GameTick(1),
            ObservationCursor::ORIGIN,
            true,
            graph,
        );
        let context = OperationContext {
            session_id: SessionId::new(7),
            request_id: RequestId::new(1),
            anchor: snapshot.anchor(),
            budget: WorkBudget {
                max_output_tokens: 16384,
                ..WorkBudget::default()
            },
            grants: vec![CapabilityGrant {
                capability: Capability::Query,
                scope: CapabilityScope {
                    fortress_id: Some(snapshot.fortress_id),
                    ..CapabilityScope::default()
                },
                max_risk: RiskTier::ReadOnly,
                expires_at_tick: None,
                remaining_uses: None,
            }],
            cancellation_requested: false,
        };
        (snapshot, context)
    }
    fn envelope(query: Value) -> Value {
        json!({"schema":SCHEMA,"query":query})
    }

    #[test]
    fn typed_filters_and_fact_provenance_are_executable() -> Result<()> {
        let (snapshot, ctx) = fixture();
        let input = envelope(
            json!({"kind":"entities","kinds":["unit"],"fields":["sane","missing"],
            "where":{"op":"compare","field":"sane","comparison":"eq","value":{"type":"bool","value":true}}}),
        );
        let result = execute(&snapshot, &ctx, &input)?;
        assert_eq!(result["matched"], 2);
        assert_eq!(result["rows"][0]["entity_id"], "2");
        assert_eq!(
            result["rows"][0]["fields"]["sane"]["epistemic_state"],
            "observed"
        );
        assert_eq!(
            result["rows"][0]["fields"]["missing"]["presence"],
            "unknown"
        );
        assert_eq!(result["coverage"]["absence_proven"], false);
        Ok(())
    }
    #[test]
    fn omitted_fields_do_not_match_even_under_negation() -> Result<()> {
        let (snapshot, ctx) = fixture();
        let result = execute(
            &snapshot,
            &ctx,
            &envelope(json!({"kind":"entities","where":{
                "op":"not","arg":{"op":"compare","field":"unknown","comparison":"eq","value":{"type":"null"}}
            }})),
        )?;
        assert_eq!(result["matched"], 0);
        Ok(())
    }
    #[test]
    fn pages_resume_but_cannot_cross_session_projection_or_snapshot() -> Result<()> {
        let (snapshot, ctx) = fixture();
        let first = execute(
            &snapshot,
            &ctx,
            &envelope(json!({"kind":"entities","limit":1})),
        )?;
        let resume =
            envelope(json!({"kind":"entities","limit":2,"continuation":first["continuation"]}));
        let next = execute(&snapshot, &ctx, &resume)?;
        assert_eq!(next["rows"][0]["entity_id"], "2");
        assert_eq!(next["rows"][1]["entity_id"], "3");
        let mut other = ctx.clone();
        other.session_id = SessionId::new(8);
        assert!(
            matches!(execute(&snapshot,&other,&resume),Err(e) if e.code==ErrorCode::StaleAnchor)
        );
        let mut changed = resume.clone();
        changed["query"]["fields"] = json!(["sane"]);
        assert!(
            matches!(execute(&snapshot,&ctx,&changed),Err(e) if e.code==ErrorCode::StaleAnchor)
        );
        let mut fork = snapshot.clone();
        fork.paused = false;
        fork.refresh_hash();
        other = ctx.clone();
        other.anchor = fork.anchor();
        assert!(matches!(execute(&fork,&other,&resume),Err(e) if e.code==ErrorCode::StaleAnchor));
        Ok(())
    }
    #[test]
    fn inspection_checks_generation_and_preserves_unknowns() -> Result<()> {
        let (snapshot, ctx) = fixture();
        let input = envelope(
            json!({"kind":"inspect","entity_id":"1","generation":1,"fields":["unknown","missing"]}),
        );
        let result = execute(&snapshot, &ctx, &input)?;
        assert_eq!(result["row"]["fields"]["unknown"]["presence"], "omitted");
        let mut stale = input;
        stale["query"]["generation"] = json!(2);
        assert!(matches!(execute(&snapshot,&ctx,&stale),Err(e) if e.code==ErrorCode::Conflict));
        Ok(())
    }
    #[test]
    fn structured_graph_modes_return_paths_cycles_and_blockers() -> Result<()> {
        let (snapshot, ctx) = fixture();
        let traversal = execute(
            &snapshot,
            &ctx,
            &envelope(json!({"kind":"traverse","roots":["4"],
            "edge_kinds":["requires"],"max_depth":4,"target":"3"})),
        )?;
        assert_eq!(traversal["path"]["vertices"], json!(["4", "1", "2", "3"]));
        assert_eq!(
            traversal["witness"]["anchor"],
            anchor_json(snapshot.anchor())
        );
        let dependencies = execute(
            &snapshot,
            &ctx,
            &envelope(json!({"kind":"dependencies","edge_kinds":["requires"]})),
        )?;
        assert_eq!(
            dependencies["blocked_by_cycles"],
            json!(["1", "2", "3", "4"])
        );
        assert!(dependencies["dependency_order"].is_null());
        assert!(dependencies["critical_chain"].is_null());
        Ok(())
    }
    #[test]
    fn authority_cancellation_expiry_and_scope_remain_fail_closed() {
        let (snapshot, ctx) = fixture();
        let input = envelope(json!({"kind":"entities"}));
        for change in 0..5 {
            let mut denied = ctx.clone();
            match change {
                0 => denied.grants.clear(),
                1 => denied.cancellation_requested = true,
                2 => denied.grants[0].scope.entity_ids = BTreeSet::from([EntityId::new(1)]),
                3 => denied.grants[0].expires_at_tick = Some(GameTick(0)),
                _ => denied.grants[0].remaining_uses = Some(1),
            }
            assert!(execute(&snapshot, &denied, &input).is_err());
        }
    }
    #[test]
    fn malformed_versions_fields_types_and_ids_are_rejected() {
        let (snapshot, ctx) = fixture();
        for query in [
            json!({"kind":"entities","limit":0}),
            json!({"kind":"entities","limit":513}),
            json!({"kind":"entities","unknown_key":true}),
            json!({"kind":"entities","kinds":["unt"]}),
            json!({"kind":"inspect","entity_id":"01","generation":1}),
            json!({"kind":"dependencies","edge_kinds":[]}),
            json!({"kind":"entities","where":{"op":"compare","field":"sane","comparison":"eq","value":{"type":"u64","value":-1}}}),
        ] {
            assert!(execute(&snapshot, &ctx, &envelope(query)).is_err());
        }
        assert!(
            execute(
                &snapshot,
                &ctx,
                &json!({"schema":"dfmcp.query/99","query":{"kind":"entities"}})
            )
            .is_err()
        );
    }
    #[test]
    fn aggregate_inputs_and_rendered_output_are_bounded() {
        let (snapshot, mut ctx) = fixture();
        let input = envelope(json!({"kind":"entities","fields":["x".repeat(65536)]}));
        assert!(execute(&snapshot, &ctx, &input).is_err());
        ctx.budget.max_bytes = 1;
        assert!(
            matches!(execute(&snapshot,&ctx,&envelope(json!({"kind":"entities"}))),Err(e) if e.code==ErrorCode::BudgetExceeded)
        );
    }
    #[test]
    fn exact_expected_anchor_and_replay_are_checked() -> Result<()> {
        let (snapshot, ctx) = fixture();
        let mut input = envelope(json!({"kind":"entities"}));
        input["expected_anchor"] = anchor_json(snapshot.anchor());
        assert_eq!(
            execute(&snapshot, &ctx, &input)?,
            execute(&snapshot, &ctx, &input)?
        );
        input["expected_anchor"]["game_tick"] = json!(2);
        assert!(matches!(execute(&snapshot,&ctx,&input),Err(e) if e.code==ErrorCode::StaleAnchor));
        Ok(())
    }
}
