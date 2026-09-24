//! Map-only query extension. Terrain routes are advisory models, never effects.
use super::*;
use dfmcp_adapter::live_map::tile_entity_id;
use dfmcp_core::Digest32;
use dfmcp_world::map_region::{MAX_ROUTE_WORK, ROUTE_POLICY};

pub(super) fn vector(value: &Value) -> Result<[u32; 3]> {
    let a = value.as_array().filter(|v| v.len() == 3).ok_or_else(|| {
        error(
            ErrorCode::InvalidRequest,
            "coordinate must contain exactly three unsigned integers",
        )
    })?;
    let mut result = [0; 3];
    for (i, v) in a.iter().enumerate() {
        result[i] = v
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .filter(|n| *n < 32768)
            .ok_or_else(|| error(ErrorCode::InvalidRequest, "coordinate exceeds map bounds"))?;
    }
    Ok(result)
}
pub(super) fn parse_region(value: &Value) -> Result<Region> {
    let o = value
        .as_object()
        .filter(|o| o.len() == 2 && o.contains_key("origin") && o.contains_key("size"))
        .ok_or_else(|| {
            error(
                ErrorCode::InvalidRequest,
                "region requires exactly origin and size",
            )
        })?;
    let r = Region {
        origin: vector(&o["origin"])?,
        size: vector(&o["size"])?,
    };
    r.volume().map_err(map_error)?;
    Ok(r)
}
pub(super) fn schema() -> Result<Value> {
    let mut schema: Value =
        serde_json::from_str(include_str!("../../../schemas/mcp_query_v1.json")).map_err(|_| {
            error(
                ErrorCode::InternalInvariantViolation,
                "base query schema invalid",
            )
        })?;
    let extension: Value = serde_json::from_str(include_str!(
        "../../../schemas/mcp_map_route_v1.json"
    ))
    .map_err(|_| {
        error(
            ErrorCode::InternalInvariantViolation,
            "map query schema invalid",
        )
    })?;
    schema["$defs"]["query"]["oneOf"]
        .as_array_mut()
        .ok_or_else(|| {
            error(
                ErrorCode::InternalInvariantViolation,
                "base query variants missing",
            )
        })?
        .push(extension);
    Ok(schema)
}
fn cursor(offset: usize, identity: Digest32) -> String {
    let mut bytes = identity.as_bytes().to_vec();
    bytes.extend_from_slice(&(offset as u64).to_be_bytes());
    format!("mr1:{offset}:{}", Digest32::of_bytes(&bytes))
}
pub(super) fn route(
    session: &MapSession,
    context: &OperationContext,
    input: &Value,
) -> Result<Value> {
    context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    if context.anchor != session.anchor()? {
        return Err(error(ErrorCode::StaleAnchor, "map route context is stale"));
    }
    let outer = input.as_object().ok_or_else(|| {
        error(
            ErrorCode::InvalidRequest,
            "map query envelope must be an object",
        )
    })?;
    if input["schema"] != "dfmcp.query/1"
        || outer
            .keys()
            .any(|key| !matches!(key.as_str(), "schema" | "query" | "expected_anchor"))
    {
        return Err(error(
            ErrorCode::InvalidRequest,
            "unknown map query envelope fields",
        ));
    }
    if input
        .get("expected_anchor")
        .filter(|v| !v.is_null())
        .is_some_and(|v| v != &anchor_json(context.anchor))
    {
        return Err(error(ErrorCode::StaleAnchor, "map expected anchor changed"));
    }
    let query = input["query"]
        .as_object()
        .ok_or_else(|| error(ErrorCode::InvalidRequest, "map_route must be an object"))?;
    if input["query"]["kind"] != "map_route"
        || query.keys().any(|key| {
            !matches!(
                key.as_str(),
                "kind" | "start" | "goal" | "limit" | "max_work" | "continuation"
            )
        })
    {
        return Err(error(
            ErrorCode::InvalidRequest,
            "unsupported map_route fields",
        ));
    }
    let start = vector(&input["query"]["start"])?;
    let goal = vector(&input["query"]["goal"])?;
    let integer = |key: &str, default: u64, maximum: u64| -> Result<u64> {
        match query.get(key) {
            None => Ok(default),
            Some(v) => v
                .as_u64()
                .filter(|n| *n > 0 && *n <= maximum)
                .ok_or_else(|| error(ErrorCode::InvalidRequest, "map route limit outside bounds")),
        }
    };
    let limit = integer("limit", 64, 256)? as usize;
    let work = integer("max_work", MAX_ROUTE_WORK, MAX_ROUTE_WORK)?;
    let source = session.state.observation().ok_or_else(|| {
        error(
            ErrorCode::InternalInvariantViolation,
            "map observation missing",
        )
    })?;
    if source.map.cells.len().saturating_add(1) > context.budget.max_entities as usize {
        return Err(error(ErrorCode::BudgetExceeded, "map scan budget exceeded"));
    }
    let route = source.map.route(start, goal, work).map_err(map_error)?;
    let mut identity = b"dfmcp-map-route-cursor/1\0".to_vec();
    identity.extend_from_slice(&session.id.get().to_be_bytes());
    for n in [
        context.anchor.fortress_id.get(),
        context.anchor.cursor.epoch,
        context.anchor.cursor.sequence,
        context.anchor.tick.0,
    ] {
        identity.extend_from_slice(&n.to_be_bytes());
    }
    identity.extend_from_slice(context.anchor.state_hash.as_bytes());
    identity.extend_from_slice(ROUTE_POLICY.as_bytes());
    identity.extend_from_slice(&work.to_be_bytes());
    for p in [start, goal] {
        for n in p {
            identity.extend_from_slice(&n.to_be_bytes());
        }
    }
    let identity = Digest32::of_bytes(&identity);
    let offset = match query.get("continuation") {
        None | Some(Value::Null) => 0,
        Some(v) => {
            let token = v
                .as_str()
                .filter(|v| v.len() <= 128)
                .ok_or_else(|| error(ErrorCode::InvalidRequest, "invalid route continuation"))?;
            let fields: Vec<_> = token.split(':').collect();
            if fields.len() != 3
                || fields[0] != "mr1"
                || fields[1].is_empty()
                || !fields[1].bytes().all(|b| b.is_ascii_digit())
            {
                return Err(error(
                    ErrorCode::InvalidRequest,
                    "invalid route continuation shape",
                ));
            }
            let n = fields[1]
                .parse::<usize>()
                .map_err(|_| error(ErrorCode::InvalidRequest, "route offset overflow"))?;
            if n == 0 || n >= route.path.len() || cursor(n, identity) != token {
                return Err(error(
                    ErrorCode::StaleAnchor,
                    "route continuation belongs to another request, session or snapshot",
                ));
            }
            n
        }
    };
    let found = !route.path.is_empty();
    let mut result = json!({"schema":"dfmcp.query.result/1","kind":"map_route","anchor":anchor_json(context.anchor),
        "policy":ROUTE_POLICY,"status":if route.endpoint_excluded{"endpoint_excluded"}else if found{"candidate_found"}else{"no_route_in_observed_model"},
        "path_vertices":route.path.len(),"model_steps":route.path.len().checked_sub(1),
        "visited_tiles":route.visited_tiles,"work_units":route.work_units,"touched_region_boundary":route.touched_region_boundary,
        "unit_path_proven":false,"safety_proven":false,"global_unreachability_proven":false,
        "region":{"origin":source.map.region.origin,"size":source.map.region.size},
        "rows":[],"returned":0,"truncated":false,"continuation":null});
    let maximum = context
        .budget
        .max_bytes
        .min(u64::from(context.budget.max_output_tokens) * 4) as usize;
    let end = offset.saturating_add(limit).min(route.path.len());
    let mut accepted = offset;
    for index in offset..end {
        let position = route.path[index];
        let row = json!({"step":index,"position":position,"entity_id":tile_entity_id(position)?.to_string()});
        result["rows"]
            .as_array_mut()
            .ok_or_else(|| error(ErrorCode::InternalInvariantViolation, "route rows missing"))?
            .push(row);
        result["returned"] = json!(index + 1 - offset);
        result["truncated"] = json!(index + 1 < route.path.len());
        result["continuation"] = if index + 1 < route.path.len() {
            json!(cursor(index + 1, identity))
        } else {
            Value::Null
        };
        if result.to_string().len() > maximum {
            result["rows"]
                .as_array_mut()
                .ok_or_else(|| error(ErrorCode::InternalInvariantViolation, "route rows missing"))?
                .pop();
            break;
        }
        accepted = index + 1;
    }
    if found && accepted == offset {
        return Err(error(
            ErrorCode::BudgetExceeded,
            "no complete route vertex fits the result budget",
        ));
    }
    result["returned"] = json!(accepted - offset);
    result["truncated"] = json!(accepted < route.path.len());
    result["continuation"] = if accepted < route.path.len() {
        json!(cursor(accepted, identity))
    } else {
        Value::Null
    };
    if result.to_string().len() > maximum {
        return Err(error(
            ErrorCode::BudgetExceeded,
            "route summary exceeds output budget",
        ));
    }
    Ok(result)
}
