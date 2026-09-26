use super::super::{anchor_json, semantic_query};
#[path = "watch_refresh.rs"]
mod watch_refresh;
use super::*;
use dfmcp_adapter::{ObservationFrame, ObservationPayload};
use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, Digest32, EntityId, FortressId, GameTick,
    ObservationCursor, OperationContext, RequestId, RiskTier, SessionId, WorkBudget,
};
use dfmcp_world::{
    EntityKind, EntityRecord, Fact, FactSource, Value as WorldValue, WorldGraph, WorldSnapshot,
};
use std::collections::BTreeMap;

fn snapshot(tick: u64, sequence: u64) -> WorldSnapshot {
    let mut graph = WorldGraph::default();
    graph.entities.insert(
        EntityId::new(1),
        EntityRecord {
            id: EntityId::new(1),
            generation: 1,
            revision: sequence + 1,
            kind: EntityKind::Unit,
            label: "Urist".to_owned(),
            fields: BTreeMap::from([(
                "sane".to_owned(),
                Fact::known(
                    WorldValue::Bool(true),
                    GameTick(tick),
                    FactSource::DfhackField("unit.sane".to_owned()),
                    Digest32::of_bytes(b"observed-fixture"),
                ),
            )]),
        },
    );
    WorldSnapshot::new(
        FortressId::new(1),
        GameTick(tick),
        ObservationCursor { epoch: 0, sequence },
        true,
        graph,
    )
}
fn context(snapshot: &WorldSnapshot, session: u128) -> OperationContext {
    OperationContext {
        session_id: SessionId::new(session),
        request_id: RequestId::new(1),
        anchor: snapshot.anchor(),
        budget: WorkBudget {
            max_game_ticks: 1000,
            max_bytes: 8192,
            max_output_tokens: 2048,
            ..WorkBudget::default()
        },
        grants: [Capability::Query, Capability::Observe]
            .into_iter()
            .map(|capability| CapabilityGrant {
                capability,
                scope: CapabilityScope {
                    fortress_id: Some(snapshot.fortress_id),
                    ..CapabilityScope::default()
                },
                max_risk: RiskTier::ReadOnly,
                expires_at_tick: None,
                remaining_uses: None,
            })
            .collect(),
        cancellation_requested: false,
    }
}
fn view(context: &OperationContext) -> QueryResponseProjection {
    QueryResponseProjection {
        session_id: context.session_id.to_string(),
        request_id: context.request_id.to_string(),
        anchor: super::super::anchor_json(context.anchor),
        briefing: json!({"read_only":true,"runtime_admitted":false,"mutation_admissible":false}),
        attention: vec![json!({"finding":"retain source warning","severity":"high"})],
        affordances: Vec::new(),
        uncertainty: vec![json!({"domain":"history","state":"unknown"})],
        coverage: json!({"status":"partial","partial_domains":["history"],"omitted_domains":["jobs"]}),
        budget: json!({"admitted":{"max_bytes":8192}}),
        references: Vec::new(),
        maximum_bytes: 8192,
    }
}
fn request(query: Value) -> Value {
    json!({"schema":"dfmcp.query/1","query":query})
}
fn register() -> Value {
    request(json!({"kind":"watch","key":"integration-condition",
    "label":"Monitor Urist's sanity","deadline_tick":100,"stable_observations":2,
    "condition":{"op":"field","entity_id":"1","generation":1,"field":"sane",
        "comparison":"eq","value":{"type":"bool","value":true}}}))
}
fn decode(raw: &str) -> Result<Value> {
    serde_json::from_str(raw)
        .map_err(|_| DfmcpError::new(ErrorCode::InternalInvariantViolation, "fixture JSON"))
}
fn call(snapshot: &WorldSnapshot, context: &OperationContext, input: &Value) -> Result<Value> {
    let view = view(context);
    let mut narrowed = context.clone();
    narrowed.budget.max_bytes = view.result_byte_budget()? as u64;
    let raw = semantic_query::execute_with_publisher(snapshot, &narrowed, input, |value| {
        view.finish(value)
    })?;
    assert!(raw.len() <= 8192);
    decode(&raw)
}
fn handle(payload: &Value) -> Result<String> {
    payload["record"]["watch"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| {
            DfmcpError::new(
                ErrorCode::InternalInvariantViolation,
                "fixture watch missing",
            )
        })
}

#[test]
fn actual_dispatch_refresh_and_packet_complete_a_watch_without_game_effects() -> Result<()> {
    let initial = snapshot(1, 0);
    let ctx = context(&initial, 900001);
    let first = call(&initial, &ctx, &register())?;
    let watch = handle(&first)?;
    assert_eq!(first["record"]["status"], "candidate");
    assert_eq!(
        first["agent_turn"]["active_work"]["obligations"]
            .as_array()
            .map(Vec::len),
        Some(1)
    );
    assert!(first.get("_condition_watch_work").is_none());
    let wait = request(json!({"kind":"await_watch","watch":watch}));
    for _ in 0..3 {
        assert!(semantic_query::prepare_await(&initial, &ctx, &wait)?);
    }
    let unchanged = call(
        &initial,
        &ctx,
        &request(json!({"kind":"poll_watch","watch":watch})),
    )?;
    assert_eq!(
        unchanged["record"]["evidence_digest"],
        first["record"]["evidence_digest"]
    );
    let target = snapshot(2, 1);
    let before = target.clone();
    let mut calls = 0;
    let refreshed = watch_refresh::once(&ctx, |_, _| {
        calls += 1;
        Ok(ObservationFrame {
            payload: ObservationPayload::Snapshot(target.clone()),
            evidence: Vec::new(),
            warnings: vec!["source warning".to_owned()],
            truncated: false,
            continuation: None,
        })
    })?;
    assert_eq!(calls, 1);
    let mut next = context(&target, 900001);
    next.anchor = refreshed.anchor;
    let view = view(&next);
    next.budget.max_bytes = view.result_byte_budget()? as u64;
    let raw = semantic_query::execute_with_publisher(
        &target,
        &next,
        &request(json!({"kind":"poll_watch","watch":watch})),
        |mut value| {
            value["kind"] = json!("await_watch");
            value["observation_refresh"] = refreshed.summary;
            view.finish(value)
        },
    )?;
    assert!(raw.len() <= 8192);
    let result = decode(&raw)?;
    assert_eq!(result["record"]["status"], "satisfied");
    assert_eq!(result["record"]["stable_observations"], 2);
    assert_eq!(result["observation_refresh"]["read_calls"], 1);
    assert_eq!(
        result["agent_turn"]["continuity"]["basis"],
        super::super::anchor_json(ctx.anchor)
    );
    assert_eq!(
        result["agent_turn"]["active_work"]["obligations"],
        json!([])
    );
    assert_eq!(
        result["agent_turn"]["coverage"]["condition_watch"]["continuous_between_observations"],
        false
    );
    assert_eq!(
        result["agent_turn"]["attention"][0]["finding"],
        "retain source warning"
    );
    assert_eq!(result["agent_turn"]["briefing"]["runtime_admitted"], false);
    assert!(!semantic_query::prepare_await(&target, &next, &wait)?);
    assert_eq!(target, before);
    Ok(())
}

#[test]
fn final_packet_failure_does_not_publish_a_registration_or_a_sample() -> Result<()> {
    let initial = snapshot(1, 0);
    let ctx = context(&initial, 900002);
    let mut too_small = view(&ctx);
    too_small.maximum_bytes = 64;
    assert!(
        semantic_query::execute_with_publisher(&initial, &ctx, &register(), |value| too_small
            .finish(value))
        .is_err()
    );
    let list = call(&initial, &ctx, &request(json!({"kind":"watches"})))?;
    assert_eq!(list["records"], json!([]));
    let first = call(&initial, &ctx, &register())?;
    let watch = handle(&first)?;
    let target = snapshot(2, 1);
    let next = context(&target, 900002);
    assert!(
        semantic_query::execute_with_publisher(
            &target,
            &next,
            &request(json!({"kind":"poll_watch","watch":watch})),
            |value| too_small.finish(value)
        )
        .is_err()
    );
    let old = call(
        &initial,
        &ctx,
        &request(json!({"kind":"poll_watch","watch":watch})),
    )?;
    assert_eq!(
        old["record"]["evidence_digest"],
        first["record"]["evidence_digest"]
    );
    assert_eq!(old["record"]["stable_observations"], 1);
    Ok(())
}

#[test]
fn unrelated_queries_keep_watch_work_visible_and_cancellation_clears_it() -> Result<()> {
    let initial = snapshot(1, 0);
    let ctx = context(&initial, 900003);
    let first = call(&initial, &ctx, &register())?;
    let watch = handle(&first)?;
    let entities = call(
        &initial,
        &ctx,
        &request(json!({"kind":"entities","fields":["sane"]})),
    )?;
    assert_eq!(entities["returned"], 1);
    assert_eq!(
        entities["agent_turn"]["active_work"]["obligations"][0]["watch"],
        watch
    );
    let projection = view(&ctx);
    let raw = semantic_query::execute_with_publisher(
        &initial,
        &ctx,
        &request(json!({"kind":"cancel_watch","watch":watch})),
        |mut value| {
            value["source_stale"] = json!(true);
            projection.finish(value)
        },
    )?;
    let cancelled = decode(&raw)?;
    assert_eq!(cancelled["record"]["status"], "cancelled");
    assert_eq!(cancelled["agent_turn"]["continuity"]["status"], "stale");
    assert_eq!(
        cancelled["agent_turn"]["active_work"]["obligations"],
        json!([])
    );
    let released = call(
        &initial,
        &ctx,
        &request(json!({"kind":"release_watch","watch":watch})),
    )?;
    assert_eq!(released["released"], true);
    Ok(())
}

#[test]
fn await_preflight_rejects_wrong_session_anchor_and_extra_fields_before_a_read() -> Result<()> {
    let initial = snapshot(1, 0);
    let ctx = context(&initial, 900004);
    let watch = handle(&call(&initial, &ctx, &register())?)?;
    let valid = request(json!({"kind":"await_watch","watch":watch}));
    for case in 0..5 {
        let mut input = valid.clone();
        let mut other = ctx.clone();
        match case {
            0 => other.session_id = SessionId::new(900005),
            1 => input["expected_anchor"] = json!({"sequence":0}),
            2 => input["query"]["arbitrary_command"] = json!("forbidden"),
            3 => other.grants.clear(),
            _ => input["schema"] = json!("unknown"),
        }
        let mut reads = 0;
        let outcome = semantic_query::prepare_await(&initial, &other, &input).and_then(|needed| {
            if needed {
                reads += 1;
            }
            Ok(())
        });
        assert!(outcome.is_err());
        assert_eq!(reads, 0);
    }
    assert!(semantic_query::prepare_await(&initial, &ctx, &valid)?);
    Ok(())
}

#[test]
fn refresh_reset_and_historical_terminals_preserve_temporal_truth() -> Result<()> {
    let initial = snapshot(1, 0);
    let ctx = context(&initial, 900006);
    let first = call(&initial, &ctx, &register())?;
    let watch = handle(&first)?;
    let mut target = snapshot(2, 1);
    target.cursor.epoch = 1;
    target.refresh_hash();
    let next = context(&target, 900006);
    let projection = view(&next);
    let raw = semantic_query::execute_with_publisher(
        &target,
        &next,
        &request(json!({"kind":"poll_watch","watch":watch})),
        |mut value| {
            value["kind"] = json!("await_watch");
            value["observation_refresh"] = json!({"basis":super::super::anchor_json(ctx.anchor),
                "target":super::super::anchor_json(next.anchor),"reset":true,"read_calls":1});
            projection.finish(value)
        },
    )?;
    let result = decode(&raw)?;
    assert_eq!(result["record"]["status"], "invalidated");
    assert_eq!(result["agent_turn"]["continuity"]["status"], "reset");
    assert_eq!(
        result["agent_turn"]["continuity"]["reset_reason"],
        "condition_wait_observation_epoch_reset"
    );
    assert_eq!(
        result["agent_turn"]["coverage"]["omitted_domains"],
        json!(["jobs"])
    );
    assert_eq!(
        result["agent_turn"]["active_work"]["obligations"],
        json!([])
    );
    Ok(())
}
