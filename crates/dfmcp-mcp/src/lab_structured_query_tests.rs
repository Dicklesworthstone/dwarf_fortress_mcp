//! Actual default-facade consumers of the shared stateless query executors.
//! Owning implementation bead: df-lab-structured-query-f7q.

use super::*;
use dfmcp_core::EdgeId;
use dfmcp_world::{
    EdgeRecord, EntityKind, EntityRecord, Fact, FactPresence, FactSource, Value as WorldValue,
    WorldGraph,
};
use serde_json::Value;

type TestResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn parsed(text: &str) -> TestResult<Value> {
    Ok(serde_json::from_str(text)?)
}

fn open_fixture() -> TestResult<(String, WorldSnapshot)> {
    let opened = parsed(&crate::agent_facade::fortress_open_session(
        Some(true),
        None,
        Some(vec![
            ("observe".to_owned(), "read_only".to_owned()),
            ("query".to_owned(), "read_only".to_owned()),
            ("plan".to_owned(), "read_only".to_owned()),
            ("control_clock".to_owned(), "reversible".to_owned()),
        ]),
        Some(60_000),
        None,
        Some(128),
        Some(1024 * 1024),
        Some(8192),
        None,
        Some("empty".to_owned()),
        Some(false),
        Some(false),
    ))?;
    assert_eq!(opened["ok"], true, "{opened}");
    let session_id = opened["session_id"]
        .as_str()
        .ok_or("session missing")?
        .to_owned();
    let session = resolve_session(Some(session_id.clone()))?;
    let snapshot = {
        let mut guard = session.lock().map_err(|_| "session poisoned")?;
        let mut source = guard.adapter.snapshot().clone();
        source.tick = GameTick(10);
        source.cursor = source.cursor.checked_next().ok_or("cursor exhausted")?;
        source.graph = WorldGraph::default();
        for id in 1..=4 {
            let known = |value| {
                Fact::known(
                    value,
                    source.tick,
                    FactSource::Derived("dfmcp.lab-scenario/1".to_owned()),
                    Digest32::of_bytes(b"structured-query-fixture"),
                )
            };
            let mut concealed = known(WorldValue::Text("concealedpayload".to_owned()));
            concealed.presence = Some(FactPresence::Redacted("test policy".to_owned()));
            source.graph.entities.insert(
                EntityId::new(id),
                EntityRecord {
                    id: EntityId::new(id),
                    generation: 1,
                    revision: id,
                    kind: EntityKind::Unit,
                    label: format!("Urist {id}"),
                    fields: BTreeMap::from([
                        ("alive".to_owned(), known(WorldValue::Bool(id % 2 == 0))),
                        ("stock".to_owned(), known(WorldValue::U64(id))),
                        ("concealed".to_owned(), concealed),
                    ]),
                },
            );
        }
        for (id, from, to) in [(1, 1, 2), (2, 2, 3)] {
            source.graph.edges.insert(
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
        source.refresh_hash();
        guard.adapter.inject_snapshot(source.clone())?;
        source
    };
    Ok((session_id, snapshot))
}

fn wrapper(query: Value, at: Option<Digest32>) -> Value {
    let mut value = json!({"mode": "structured",
        "request": {"schema": "dfmcp.query/1", "query": query}});
    if let Some(hash) = at {
        value["at"] = json!(hash.to_string());
    }
    value
}

fn query(session: &str, request: Value) -> TestResult<Value> {
    parsed(&crate::agent_facade::fortress_query(
        Some(session.to_owned()),
        Some(request.to_string()),
    ))
}

fn expect_refusal(result: &Value, code: &str) {
    assert_eq!(result["ok"], false, "{result}");
    assert_eq!(result["error"]["code"], code, "{result}");
    assert!(result.get("rows").is_none(), "{result}");
    assert!(result.get("groups").is_none(), "{result}");
    assert!(result["agent_turn"].is_object(), "{result}");
    assert_eq!(
        result["agent_turn"]["coverage"]["absence_proof_scope"],
        json!([])
    );
}

fn advance(session_id: &str) -> TestResult<WorldSnapshot> {
    let session = resolve_session(Some(session_id.to_owned()))?;
    let mut guard = session.lock().map_err(|_| "session poisoned")?;
    remember_version(&mut guard);
    let mut snapshot = guard.adapter.snapshot().clone();
    snapshot.tick = GameTick(snapshot.tick.0 + 10);
    snapshot.cursor = snapshot.cursor.checked_next().ok_or("cursor exhausted")?;
    snapshot
        .graph
        .entities
        .get_mut(&EntityId::new(1))
        .ok_or("unit missing")?
        .generation += 1;
    snapshot.refresh_hash();
    guard.adapter.inject_snapshot(snapshot.clone())?;
    Ok(snapshot)
}

#[test]
fn lab_structured_projection_inspect_aggregate_and_search_reach_the_public_facade() -> TestResult {
    let _serial = crate::test_serial();
    let (session_id, source) = open_fixture()?;
    let request = wrapper(
        json!({"kind": "entities", "kinds": ["unit"],
            "fields": ["alive", "concealed"], "limit": 4, "order": "id_descending",
            "where": {"op": "compare", "field": "alive", "comparison": "eq",
                "value": {"type": "bool", "value": true}}}),
        None,
    );
    let result = query(&session_id, request.clone())?;
    assert_eq!(result["ok"], true, "{result}");
    assert_eq!(result["matched"], 2);
    assert_eq!(result["rows"][0]["entity_id"], "4");
    assert_eq!(result["rows"][1]["entity_id"], "2");
    assert_eq!(result["rows"][0]["fields"]["alive"]["value"]["value"], true);
    assert_eq!(
        result["rows"][0]["fields"]["concealed"]["presence"],
        "redacted"
    );
    assert!(result["rows"][0]["fields"]["concealed"]["value"].is_null());
    assert!(!result["rows"].to_string().contains("concealedpayload"));
    assert_eq!(result["anchor"], anchor_json(&source.anchor()));
    assert_eq!(result["agent_turn"]["anchor"], result["anchor"]);
    assert_eq!(result["source_work"]["records"], 6);
    assert_eq!(result["rows"], query(&session_id, request)?["rows"]);

    let inspection = query(
        &session_id,
        wrapper(
            json!({"kind": "inspect", "entity_id": "2", "generation": 1,
            "fields": ["stock"]}),
            None,
        ),
    )?;
    assert_eq!(inspection["ok"], true, "{inspection}");
    assert_eq!(inspection["row"]["fields"]["stock"]["value"]["value"], 2);
    expect_refusal(
        &query(
            &session_id,
            wrapper(
                json!({"kind": "inspect",
        "entity_id": "2", "generation": 2}),
                None,
            ),
        )?,
        "conflict",
    );

    let aggregate = query(
        &session_id,
        wrapper(
            json!({"kind": "aggregate", "kinds": ["unit"],
            "group_by": {"kind": "entity_kind"},
            "metrics": [{"name": "stock", "field": "stock", "numeric_type": "u64"}]}),
            None,
        ),
    )?;
    assert_eq!(aggregate["ok"], true, "{aggregate}");
    assert_eq!(aggregate["groups"][0]["count"], 4);
    assert_eq!(aggregate["groups"][0]["metrics"]["stock"]["sum"], "10");
    assert_eq!(aggregate["epistemic_state"], "inferred");
    assert_eq!(
        aggregate["agent_turn"]["coverage"]["absence_proof_scope"],
        json!([])
    );

    let search = query(
        &session_id,
        wrapper(
            json!({"kind": "search", "text": "Urist",
        "kinds": ["unit"], "limit": 2}),
            None,
        ),
    )?;
    assert_eq!(search["ok"], true, "{search}");
    assert_eq!(search["matched"], 4);
    assert!(search["continuation"].is_string());
    assert_eq!(search["agent_turn"]["continuity"]["status"], "partial");

    // Prove the public feature is useful with the actual terrain-bearing
    // starter world and unchanged negotiated defaults, not just a tiny fixture.
    let starter = parsed(&crate::agent_facade::fortress_open_session(
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        Some("starter_fortress".to_owned()),
        None,
        None,
    ))?;
    assert_eq!(starter["ok"], true, "{starter}");
    let starter_id = starter["session_id"]
        .as_str()
        .ok_or("starter session missing")?;
    let page = query(
        starter_id,
        wrapper(
            json!({"kind": "entities", "kinds": ["unit"],
                "fields": ["profession"], "limit": 1}),
            None,
        ),
    )?;
    assert_eq!(page["ok"], true, "{page}");
    assert_eq!(page["matched"], 7);
    assert_eq!(page["returned"], 1);
    assert_eq!(page["query_budget"]["max_output_tokens"], 4096);
    assert!(
        page["source_work"]["records"]
            .as_u64()
            .is_some_and(|n| n > 6)
    );
    let totals = query(
        starter_id,
        wrapper(json!({"kind": "aggregate", "kinds": ["unit"]}), None),
    )?;
    assert_eq!(totals["ok"], true, "{totals}");
    assert_eq!(totals["matched"], 7);
    assert_eq!(totals["groups"][0]["count"], 7);
    Ok(())
}

#[test]
fn lab_structured_graph_queries_return_real_dependency_witnesses_and_paths() -> TestResult {
    let _serial = crate::test_serial();
    let (session_id, source) = open_fixture()?;
    let dependencies = query(
        &session_id,
        wrapper(
            json!({"kind": "dependencies",
        "edge_kinds": ["requires"]}),
            None,
        ),
    )?;
    assert_eq!(dependencies["ok"], true, "{dependencies}");
    assert_eq!(
        dependencies["dependency_order"],
        json!(["3", "2", "1", "4"])
    );
    assert_eq!(dependencies["critical_chain"], json!(["1", "2", "3"]));
    assert_eq!(dependencies["blocked_by_cycles"], json!([]));
    assert_eq!(
        dependencies["witness"]["anchor"],
        anchor_json(&source.anchor())
    );
    assert!(dependencies["witness"]["decision_digest"].is_string());
    let route = query(
        &session_id,
        wrapper(
            json!({"kind": "traverse", "roots": ["1"],
        "edge_kinds": ["requires"], "max_depth": 2, "target": "3"}),
            None,
        ),
    )?;
    assert_eq!(route["ok"], true, "{route}");
    assert_eq!(route["path"]["vertices"], json!(["1", "2", "3"]));
    assert_eq!(
        route["path"]["edges"][1]["edge_id"],
        json!(EdgeId::new(2).to_string())
    );
    assert_eq!(route["coverage"]["absence_proven"], false);
    Ok(())
}

#[test]
fn lab_structured_continuations_bind_source_session_projection_and_current_authority() -> TestResult
{
    let _serial = crate::test_serial();
    let (session_id, before) = open_fixture()?;
    let mut request = wrapper(
        json!({"kind": "entities", "kinds": ["unit"],
        "fields": ["stock"], "limit": 1}),
        None,
    );
    let first = query(&session_id, request.clone())?;
    assert_eq!(first["ok"], true, "{first}");
    assert_eq!(first["rows"][0]["entity_id"], "1");
    assert!(first["continuation"].is_string());
    request["request"]["query"]["continuation"] = first["continuation"].clone();
    let second = query(&session_id, request.clone())?;
    assert_eq!(second["rows"][0]["entity_id"], "2");

    let mut projected = request.clone();
    projected["request"]["query"]["fields"] = json!(["alive"]);
    expect_refusal(&query(&session_id, projected)?, "stale_anchor");
    let (other_session, _) = open_fixture()?;
    expect_refusal(&query(&other_session, request.clone())?, "stale_anchor");

    let current = advance(&session_id)?;
    expect_refusal(&query(&session_id, request.clone())?, "stale_anchor");
    request["at"] = json!(before.state_hash.to_string());
    let retained = query(&session_id, request.clone())?;
    assert_eq!(retained["ok"], true, "{retained}");
    assert_eq!(retained["rows"][0]["entity_id"], "2");
    assert_eq!(retained["anchor"], anchor_json(&before.anchor()));
    assert_eq!(retained["current_anchor"], anchor_json(&current.anchor()));
    assert_eq!(retained["historical"], true);
    assert_eq!(retained["agent_turn"]["anchor"], retained["current_anchor"]);
    assert_eq!(retained["agent_turn"]["continuity"]["status"], "stale");
    assert_eq!(
        retained["agent_turn"]["coverage"]["anchor"],
        retained["anchor"]
    );
    assert_eq!(
        retained["agent_turn"]["coverage"]["eligible_as_mutation_evidence"],
        false
    );

    let mut wrong_anchor = request.clone();
    wrong_anchor["request"]["expected_anchor"] = anchor_json(&current.anchor());
    expect_refusal(&query(&session_id, wrong_anchor)?, "stale_anchor");
    let mut missing = request.clone();
    missing["at"] = json!("f".repeat(64));
    expect_refusal(&query(&session_id, missing)?, "stale_anchor");
    Ok(())
}

#[test]
fn lab_structured_historical_reads_cannot_revive_expired_revoked_or_narrow_grants() -> TestResult {
    let _serial = crate::test_serial();
    for restriction in ["expired", "revoked", "entities", "region", "exhausted"] {
        let (session_id, before) = open_fixture()?;
        let current = advance(&session_id)?;
        let session = resolve_session(Some(session_id.clone()))?;
        {
            let mut guard = session.lock().map_err(|_| "session poisoned")?;
            let grant = guard
                .grants
                .iter_mut()
                .find(|grant| grant.capability == Capability::Query)
                .ok_or("Query grant missing")?;
            match restriction {
                "expired" => grant.expires_at_tick = Some(GameTick(current.tick.0 - 1)),
                "revoked" => grant.capability = Capability::Observe,
                "entities" => {
                    grant.scope.entity_ids.insert(EntityId::new(1));
                }
                "region" => {
                    grant.scope.map_area = Some(MapCuboid::from_corners(
                        dfmcp_core::MapCoord::new(0, 0, 0),
                        dfmcp_core::MapCoord::new(1, 1, 1),
                    ))
                }
                "exhausted" => grant.remaining_uses = Some(0),
                _ => return Err("unknown test restriction".into()),
            }
        }
        let result = query(
            &session_id,
            wrapper(
                json!({"kind": "inspect",
            "entity_id": "1", "generation": 1, "fields": ["stock"]}),
                Some(before.state_hash),
            ),
        )?;
        expect_refusal(&result, "capability_denied");
        assert!(result.get("row").is_none());
    }
    Ok(())
}

#[test]
fn lab_structured_unknown_facts_do_not_match_even_under_negation() -> TestResult {
    let _serial = crate::test_serial();
    let (session_id, _) = open_fixture()?;
    for field in ["concealed", "not_observed"] {
        let result = query(
            &session_id,
            wrapper(
                json!({"kind": "entities", "kinds": ["unit"],
            "fields": [field], "where": {"op": "not", "arg": {"op": "compare",
                "field": field, "comparison": "eq",
                "value": {"type": "text", "value": "concealedpayload"}}}}),
                None,
            ),
        )?;
        assert_eq!(result["ok"], true, "{result}");
        assert_eq!(result["matched"], 0);
        assert_eq!(result["rows"], json!([]));
        assert_eq!(
            result["agent_turn"]["coverage"]["absence_proof_scope"],
            json!([])
        );
    }
    Ok(())
}

#[test]
fn lab_structured_events_reach_the_facade_and_resume_exact_retained_history() -> TestResult {
    let _serial = crate::test_serial();
    let (session_id, _) = open_fixture()?;
    let before = {
        let session = resolve_session(Some(session_id.clone()))?;
        let mut guard = session.lock().map_err(|_| "session poisoned")?;
        let mut snapshot = guard.adapter.snapshot().clone();
        for (id, tick) in [(9, 2), (1, 4), (2, 4)] {
            let event_id = dfmcp_core::EventId::new(id);
            snapshot.graph.events.insert(
                event_id,
                dfmcp_world::WorldEvent {
                    id: event_id,
                    tick: GameTick(tick),
                    kind: dfmcp_world::WorldEventKind::JobChanged,
                    subject: Some(EntityId::new(1)),
                    summary: format!("job observation {id}"),
                    fields: BTreeMap::from([("count".to_owned(), WorldValue::U64(id as u64))]),
                },
            );
        }
        snapshot.refresh_hash();
        guard.adapter.inject_snapshot(snapshot.clone())?;
        snapshot
    };
    let mut request = wrapper(
        json!({"kind":"events", "from_tick":2,"through_tick":4,
        "kinds":["job_changed"],"subjects":["1"],"fields":["count","missing"],"limit":1}),
        None,
    );
    let first = query(&session_id, request.clone())?;
    assert_eq!(first["ok"], true, "{first}");
    assert_eq!(
        first["rows"][0]["event_id"],
        dfmcp_core::EventId::new(9).to_string()
    );
    assert_eq!(first["rows"][0]["fields"]["missing"]["presence"], "unknown");
    assert_eq!(first["agent_turn"]["anchor"], anchor_json(&before.anchor()));
    assert_eq!(first["coverage"]["history_complete"], false);
    assert_eq!(first["coverage"]["absence_proven"], false);
    request["request"]["query"]["continuation"] = first["continuation"].clone();
    let current = advance(&session_id)?;
    expect_refusal(&query(&session_id, request.clone())?, "stale_anchor");
    request["at"] = json!(before.state_hash.to_string());
    let historical = query(&session_id, request.clone())?;
    assert_eq!(historical["ok"], true, "{historical}");
    assert_eq!(
        historical["rows"][0]["event_id"],
        dfmcp_core::EventId::new(1).to_string()
    );
    assert_eq!(historical["anchor"], anchor_json(&before.anchor()));
    assert_eq!(
        historical["agent_turn"]["anchor"],
        anchor_json(&current.anchor())
    );
    assert_eq!(historical["agent_turn"]["continuity"]["status"], "stale");
    assert_eq!(historical["historical"], true);
    {
        let session = resolve_session(Some(session_id.clone()))?;
        let mut guard = session.lock().map_err(|_| "session poisoned")?;
        guard
            .grants
            .iter_mut()
            .find(|grant| grant.capability == Capability::Query)
            .ok_or("Query grant missing")?
            .expires_at_tick = Some(GameTick(before.tick.0));
    }
    expect_refusal(&query(&session_id, request)?, "capability_denied");
    Ok(())
}

#[test]
fn history_scenario_produces_queryable_events_through_the_normal_public_workflow() -> TestResult {
    let _serial = crate::test_serial();
    let opened = parsed(&crate::agent_facade::fortress_open_session(
        Some(false),
        None,
        Some(
            [
                ("observe", "read_only"),
                ("query", "read_only"),
                ("plan", "read_only"),
                ("control_clock", "reversible"),
                ("configure_production", "reversible"),
            ]
            .into_iter()
            .map(|(capability, risk)| (capability.to_owned(), risk.to_owned()))
            .collect(),
        ),
        Some(60_000),
        Some(1_000),
        Some(1_000),
        Some(1024 * 1024),
        Some(16_384),
        Some(16),
        Some("starter_fortress_history".to_owned()),
        Some(false),
        Some(false),
    ))?;
    assert_eq!(opened["ok"], true, "{opened}");
    let session = opened["session_id"]
        .as_str()
        .ok_or("session missing")?
        .to_owned();
    let initial = query(
        &session,
        wrapper(json!({"kind":"events", "fields":["transition"]}), None),
    )?;
    assert_eq!(initial["ok"], true, "{initial}");
    assert_eq!(initial["matched"], 1);
    assert_eq!(
        initial["coverage"]["producer"]["model"],
        dfmcp_lab::event_history::MODEL
    );
    let planned = parsed(&crate::agent_facade::fortress_plan(
        Some(session.clone()),
        None,
        None,
        Some(
            json!([{"action":{"kind":"create_work_order","name":"queryable brewing history",
            "job_token":"BREW_DRINK","amount":1}}])
            .to_string(),
        ),
        None,
    ))?;
    assert_eq!(planned["ok"], true, "{planned}");
    let order = planned["steps"][0]["creates_entity_id"]
        .as_str()
        .ok_or("order identity missing")?;
    let committed = parsed(&crate::agent_facade::fortress_commit(
        Some(session.clone()),
        planned["plan_digest"]
            .as_str()
            .ok_or("plan digest missing")?
            .to_owned(),
    ))?;
    assert_eq!(committed["ok"], true, "{committed}");
    let waited = parsed(&crate::agent_facade::fortress_wait(
        Some(session.clone()),
        Some(50),
    ))?;
    assert_eq!(waited["ok"], true, "{waited}");
    let mut request = wrapper(
        json!({"kind":"events", "kinds":["job_changed"],
        "subjects":[order], "fields":["change"], "limit":1}),
        None,
    );
    let first = query(&session, request.clone())?;
    assert_eq!(first["ok"], true, "{first}");
    assert!(
        first["matched"].as_u64().is_some_and(|count| count >= 2),
        "{first}"
    );
    assert_eq!(first["coverage"]["history_complete"], false);
    assert_eq!(first["coverage"]["eligible_as_mutation_evidence"], false);
    assert!(first["continuation"].is_string(), "{first}");
    let retained_hash = first["anchor"]["state_hash"]
        .as_str()
        .ok_or("event anchor missing")?
        .to_owned();
    request["request"]["query"]["continuation"] = first["continuation"].clone();
    let waited = parsed(&crate::agent_facade::fortress_wait(
        Some(session.clone()),
        Some(1),
    ))?;
    assert_eq!(waited["ok"], true, "{waited}");
    expect_refusal(&query(&session, request.clone())?, "stale_anchor");
    request["at"] = json!(retained_hash);
    let resumed = query(&session, request)?;
    assert_eq!(resumed["ok"], true, "{resumed}");
    assert_eq!(resumed["historical"], true);
    assert_eq!(resumed["anchor"], first["anchor"]);
    assert_ne!(resumed["rows"][0]["event_id"], first["rows"][0]["event_id"]);
    Ok(())
}

#[test]
fn lab_structured_closed_wrapper_refuses_stateful_modes_and_keeps_legacy_queries() -> TestResult {
    let _serial = crate::test_serial();
    let (session_id, source) = open_fixture()?;
    for request in [
        json!({"mode": "structured", "request": {"schema": "dfmcp.query/1", "query": {"kind": "watch"}}}),
        json!({"mode": "structured", "request": {"schema": "dfmcp.query/2", "query": {"kind": "entities"}}}),
        json!({"mode": "structured", "request": {"schema": "dfmcp.query/1", "query": {"kind": "entities"}, "extra": true}}),
        json!({"mode": "structured", "request": {"schema": "dfmcp.query/1", "query": {"kind": "entities"}}, "extra": true}),
        wrapper(json!({"kind": "entities", "unexpected": true}), None),
        {
            let mut request = wrapper(json!({"kind": "entities"}), None);
            request["at"] = json!("F".repeat(64));
            request
        },
    ] {
        expect_refusal(&query(&session_id, request)?, "invalid_request");
    }
    for raw in [
        "entities".to_owned(),
        json!({"mode":"entities", "kind":"unit", "limit":1}).to_string(),
        json!({"mode":"entities", "kind":"unit", "limit":1, "at":source.state_hash.to_string()}).to_string(),
        json!({"mode":"changes", "since":source.state_hash.to_string()}).to_string(),
        json!({"mode":"search", "text":"Urist", "limit":1}).to_string(),
        json!({"mode":"observation", "completeness_profile":"research-full", "section":"entities", "limit":1}).to_string(),
    ] {
        let result = parsed(&crate::agent_facade::fortress_query(Some(session_id.clone()), Some(raw)))?;
        assert_eq!(result["ok"], true, "{result}");
    }
    for raw in [
        format!("{{{}", " ".repeat(crate::lab_world::MAX_QUERY_JSON_BYTES)),
        format!(
            "{}{}",
            "entities",
            " ".repeat(crate::lab_world::MAX_QUERY_JSON_BYTES)
        ),
        format!(
            "{{\"mode\":\"entities\"}}{}",
            " ".repeat(crate::lab_world::MAX_QUERY_JSON_BYTES)
        ),
    ] {
        expect_refusal(
            &parsed(&crate::agent_facade::fortress_query(
                Some(session_id.clone()),
                Some(raw),
            ))?,
            "budget_exceeded",
        );
    }
    let duplicate = r#"{"mode":"structured","mode":"structured","request":{"schema":"dfmcp.query/1","query":{"kind":"entities"}}}"#;
    expect_refusal(
        &parsed(&crate::agent_facade::fortress_query(
            Some(session_id.clone()),
            Some(duplicate.to_owned()),
        ))?,
        "invalid_request",
    );
    let long = wrapper(
        json!({"kind": "inspect", "entity_id":"1", "generation":1,
        "fields": (0..16).map(|i| format!("{i}-{}", "x".repeat(110))).collect::<Vec<_>>()}),
        None,
    );
    assert!(long.to_string().len() > crate::lab_world::MAX_QUERY_JSON_BYTES);
    assert_eq!(query(&session_id, long)?["ok"], true);
    Ok(())
}

#[test]
fn lab_structured_full_packet_refuses_clipping_and_smaller_pages_resume_without_loss() -> TestResult
{
    let _serial = crate::test_serial();
    let (session_id, source) = open_fixture()?;
    let mut request = wrapper(
        json!({"kind":"entities", "kinds":["unit"], "limit":4,
        "fields": ["alive", "stock", "unseen_a", "unseen_b", "unseen_c", "unseen_d", "unseen_e", "unseen_f"]}),
        None,
    );
    let raw = super::fortress_query(Some(session_id.clone()), Some(request.to_string()));
    assert_eq!(parsed(&raw)?["ok"], true, "{raw}");
    let max_tokens = u32::try_from((raw.len() + 512).div_ceil(4))?;
    let session = resolve_session(Some(session_id.clone()))?;
    {
        let mut guard = session.lock().map_err(|_| "session poisoned")?;
        guard.budget.max_output_tokens = max_tokens;
    }
    let refusal = query(&session_id, request.clone())?;
    expect_refusal(&refusal, "budget_exceeded");
    assert!(refusal.get("continuation").is_none());
    assert!(refusal.to_string().len() <= max_tokens as usize * 4);
    assert_eq!(
        refusal["agent_turn"]["anchor"],
        anchor_json(&source.anchor())
    );

    request["request"]["query"]["limit"] = json!(1);
    let mut ids = Vec::new();
    for _ in 0..4 {
        let result = query(&session_id, request.clone())?;
        assert_eq!(result["ok"], true, "{result}");
        assert!(result.to_string().len() <= max_tokens as usize * 4);
        assert_eq!(result["returned"], 1);
        assert_eq!(result["rows"].as_array().ok_or("rows missing")?.len(), 1);
        ids.push(result["rows"][0]["entity_id"].clone());
        if result["continuation"].is_null() {
            break;
        }
        request["request"]["query"]["continuation"] = result["continuation"].clone();
    }
    assert_eq!(ids, vec![json!("1"), json!("2"), json!("3"), json!("4")]);
    assert_eq!(
        session
            .lock()
            .map_err(|_| "session poisoned")?
            .adapter
            .snapshot(),
        &source
    );
    Ok(())
}

#[test]
fn lab_structured_source_and_final_byte_limits_and_deadlines_are_independent() -> TestResult {
    let _serial = crate::test_serial();
    let (session_id, _) = open_fixture()?;
    let request = wrapper(
        json!({"kind": "entities", "kinds": ["unit"], "limit": 1}),
        None,
    );
    let session = resolve_session(Some(session_id.clone()))?;
    {
        let mut guard = session.lock().map_err(|_| "session poisoned")?;
        guard.budget.max_entities = 5;
    }
    expect_refusal(&query(&session_id, request.clone())?, "budget_exceeded");
    {
        let mut guard = session.lock().map_err(|_| "session poisoned")?;
        guard.budget.max_entities = 128;
        guard.budget.max_bytes = 4096;
    }
    let result = query(&session_id, request.clone())?;
    assert!(result.to_string().len() <= 4096, "{result}");
    assert!(result["agent_turn"].is_object(), "{result}");
    {
        let mut guard = session.lock().map_err(|_| "session poisoned")?;
        guard.budget.max_bytes = 1024 * 1024;
    }
    let result = query(&session_id, request)?;
    assert_eq!(result["ok"], true, "{result}");
    let late = parsed(&query_budget::finish_response(
        result.to_string(),
        std::time::Duration::from_millis(60_000),
    ))?;
    expect_refusal(&late, "budget_exceeded");
    assert!(
        late["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("wall-time"))
    );
    assert_eq!(late["error"]["effect_may_have_occurred"], false);
    Ok(())
}

#[test]
fn lab_spatial_entities_resume_the_exact_source_and_recheck_current_authority() -> TestResult {
    let _serial = crate::test_serial();
    let (session_id, _) = open_fixture()?;
    let session = resolve_session(Some(session_id.clone()))?;
    let source = {
        let mut guard = session.lock().map_err(|_| "session poisoned")?;
        let mut source = guard.adapter.snapshot().clone();
        for entity in source.graph.entities.values_mut() {
            let coordinate = entity.id.get() as i32;
            let mut position = Fact::known(
                WorldValue::Coord(dfmcp_core::MapCoord::new(coordinate, coordinate, 0)),
                source.tick,
                FactSource::Derived("dfmcp.lab-scenario/1".to_owned()),
                Digest32::of_bytes(b"spatial-lab-fixture"),
            );
            if entity.id == EntityId::new(4) {
                position.presence = Some(FactPresence::Redacted("test policy".to_owned()));
            }
            entity.fields.insert("position".to_owned(), position);
        }
        source.refresh_hash();
        guard.adapter.inject_snapshot(source.clone())?;
        source
    };
    let mut request = wrapper(
        json!({
            "kind":"spatial_entities",
            "region":{"min":[1,1,0],"max":[3,3,0]},
            "geometry":{"kind":"point","field":"position"},
            "relation":"within", "kinds":["unit"], "fields":["concealed"], "limit":1
        }),
        None,
    );
    let first = query(&session_id, request.clone())?;
    assert_eq!(first["ok"], true, "{first}");
    assert_eq!(first["matched"], 3);
    assert_eq!(first["selection"]["unknown_geometry"], 1);
    assert_eq!(first["rows"][0]["entity_id"], "1");
    assert_eq!(first["rows"][0]["geometry"]["facts"]["position"]["epistemic_state"], "inferred");
    assert_eq!(first["rows"][0]["fields"]["concealed"]["presence"], "redacted");
    assert!(!first["rows"].to_string().contains("concealedpayload"));
    assert_eq!(first["coverage"]["absence_proven"], false);
    assert_eq!(first["agent_turn"]["anchor"], anchor_json(&source.anchor()));

    request["request"]["query"]["continuation"] = first["continuation"].clone();
    let current = advance(&session_id)?;
    expect_refusal(&query(&session_id, request.clone())?, "stale_anchor");
    request["at"] = json!(source.state_hash.to_string());
    let historical = query(&session_id, request.clone())?;
    assert_eq!(historical["ok"], true, "{historical}");
    assert_eq!(historical["historical"], true);
    assert_eq!(historical["anchor"], anchor_json(&source.anchor()));
    assert_eq!(historical["current_anchor"], anchor_json(&current.anchor()));
    assert_eq!(historical["agent_turn"]["anchor"], anchor_json(&current.anchor()));
    assert_eq!(historical["rows"][0]["entity_id"], "2");
    assert_eq!(historical["rows"][0]["generation"], 1);
    assert_eq!(historical["coverage"]["eligible_as_mutation_evidence"], false);
    {
        let mut guard = session.lock().map_err(|_| "session poisoned")?;
        guard.grants.iter_mut()
            .find(|grant| grant.capability == Capability::Query)
            .ok_or("Query grant missing")?
            .expires_at_tick = Some(source.tick);
    }
    expect_refusal(&query(&session_id, request)?, "capability_denied");
    assert_eq!(session.lock().map_err(|_| "session poisoned")?.adapter.snapshot(), &current);
    Ok(())
}
