//! Public laboratory regression coverage for exact-source directed bottlenecks.
//! Owning bead: df-lab-directed-bottlenecks-r3k.

use super::*;
use dfmcp_core::EdgeId;
use dfmcp_world::{
    EdgeRecord, EntityKind, EntityRecord, Fact, FactSource, Value as WorldValue, WorldGraph,
};
use serde_json::Value;

type TestResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn parsed(text: &str) -> TestResult<Value> {
    Ok(serde_json::from_str(text)?)
}

fn fixture() -> TestResult<(String, WorldSnapshot)> {
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
    let id = opened["session_id"]
        .as_str()
        .ok_or("session missing")?
        .to_owned();
    let session = resolve_session(Some(id.clone()))?;
    let source = {
        let mut guard = session.lock().map_err(|_| "session poisoned")?;
        let mut source = guard.adapter.snapshot().clone();
        source.tick = GameTick(10);
        source.cursor = source.cursor.checked_next().ok_or("cursor exhausted")?;
        source.graph = WorldGraph::default();
        for id in 1..=7 {
            source.graph.entities.insert(
                EntityId::new(id),
                EntityRecord {
                    id: EntityId::new(id),
                    generation: 1,
                    revision: id,
                    kind: EntityKind::Building,
                    label: format!("Junction {id}"),
                    fields: BTreeMap::new(),
                },
            );
        }
        // An unreachable cycle points into the reachable diamond. It cannot
        // become a second entry and invalidate the root-relative bottleneck.
        for (id, from, to) in [
            (1, 1, 2),
            (2, 1, 3),
            (3, 2, 4),
            (4, 3, 4),
            (5, 4, 5),
            (6, 6, 7),
            (7, 7, 6),
            (8, 7, 4),
        ] {
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
    Ok((id, source))
}

fn request(target: Option<u64>) -> Value {
    let mut request = json!({"mode":"structured", "request":{
        "schema":"dfmcp.query/1", "query":{
            "kind":"dominators", "root":"1", "root_generation":1,
            "edge_kinds":["requires"]
        }
    }});
    if let Some(target) = target {
        request["request"]["query"]["target"] = json!(target.to_string());
        request["request"]["query"]["target_generation"] = json!(1);
    }
    request
}

fn query(session: &str, request: Value) -> TestResult<Value> {
    parsed(&crate::agent_facade::fortress_query(
        Some(session.to_owned()),
        Some(request.to_string()),
    ))
}

fn refusal(value: &Value, code: &str) {
    assert_eq!(value["ok"], false, "{value}");
    assert_eq!(value["error"]["code"], code, "{value}");
    for key in ["vertices", "dominator_chain", "witness", "unreachable"] {
        assert!(value.get(key).is_none(), "partial analysis leaked: {value}");
    }
    assert!(value["agent_turn"].is_object(), "{value}");
}

#[test]
fn lab_dominators_explain_diamond_bottlenecks_and_preserve_prepared_work() -> TestResult {
    let _serial = crate::test_serial();
    let (session_id, source) = fixture()?;
    let tree = query(&session_id, request(None))?;
    assert_eq!(tree["ok"], true, "{tree}");
    assert_eq!(tree["view"], "dominator_tree");
    assert_eq!(tree["reachable_count"], 5);
    assert_eq!(tree["unreachable"], json!(["6", "7"]));
    assert_eq!(tree["vertices"][3]["entity_id"], "4");
    assert_eq!(tree["vertices"][3]["immediate_dominator"], "1");
    assert_eq!(tree["vertices"][3]["dominated_descendants"], 1);
    assert_eq!(tree["source_work"]["records"], 15);

    let plan = parsed(&crate::agent_facade::fortress_plan(
        Some(session_id.clone()),
        Some("retain the separately reviewed pause plan".to_owned()),
        Some(false),
        None,
        None,
    ))?;
    assert_eq!(plan["ok"], true, "{plan}");
    let focused = query(&session_id, request(Some(5)))?;
    assert_eq!(focused["ok"], true, "{focused}");
    assert_eq!(focused["view"], "target_chain");
    assert_eq!(focused["dominator_chain"], json!(["1", "4", "5"]));
    assert_eq!(focused["returned"], 3);
    assert_eq!(focused["reachable_count"], 5);
    assert_eq!(focused["unreachable_count"], 2);
    assert!(focused["unreachable"].is_null());
    assert_eq!(focused["witness"], tree["witness"]);
    assert_eq!(focused["analysis_complete"], true);
    assert_eq!(focused["truncated"], false);
    assert!(focused["continuation"].is_null());
    assert_eq!(focused["native_captures"], 0);
    assert_eq!(focused["mutation_dispatched"], false);
    assert_eq!(focused["coverage"]["absence_proven"], false);
    assert_eq!(focused["coverage"]["walkability_proven"], false);
    assert_eq!(focused["coverage"]["mutation_authorized"], false);
    assert_eq!(
        focused["observation_coverage"]["eligible_as_mutation_evidence"],
        false
    );
    assert_eq!(focused["anchor"], anchor_json(&source.anchor()));
    assert_eq!(focused["agent_turn"]["anchor"], focused["anchor"]);
    assert_eq!(
        focused["agent_turn"]["active_work"]["pending_plans"][0]["plan_digest"],
        plan["plan_digest"]
    );
    // The unavoidable chain is not a path: there is no direct 1 -> 4 edge.
    assert!(
        !source
            .graph
            .edges
            .values()
            .any(|edge| { edge.from == EntityId::new(1) && edge.to == EntityId::new(4) })
    );
    let session = resolve_session(Some(session_id))?;
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
fn lab_dominators_keep_direction_and_unreachable_targets_explicit() -> TestResult {
    let _serial = crate::test_serial();
    let (session_id, source) = fixture()?;
    let unreachable = query(&session_id, request(Some(6)))?;
    assert_eq!(unreachable["ok"], true, "{unreachable}");
    assert_eq!(unreachable["target_status"], "unreachable_in_projection");
    assert_eq!(unreachable["vertices"], json!([]));
    assert!(unreachable["dominator_chain"].is_null());
    assert_eq!(unreachable["coverage"]["absence_proven"], false);
    refusal(&query(&session_id, request(Some(99)))?, "invalid_request");

    let mut incoming = request(Some(1));
    incoming["request"]["query"]["root"] = json!("5");
    incoming["request"]["query"]["direction"] = json!("incoming");
    let reversed = query(&session_id, incoming)?;
    assert_eq!(reversed["ok"], true, "{reversed}");
    assert_eq!(reversed["dominator_chain"], json!(["5", "4", "1"]));
    assert_eq!(reversed["reachable_count"], 7);
    assert_eq!(reversed["direction"], "incoming");
    assert_eq!(reversed["anchor"], anchor_json(&source.anchor()));

    let mut other_edges = request(Some(5));
    other_edges["request"]["query"]["edge_kinds"] = json!(["uses"]);
    let filtered = query(&session_id, other_edges)?;
    assert_eq!(filtered["ok"], true, "{filtered}");
    assert_eq!(filtered["target_status"], "unreachable_in_projection");
    assert_eq!(filtered["reachable_count"], 1);
    assert_eq!(filtered["coverage"]["absence_proven"], false);
    Ok(())
}

#[test]
fn lab_dominators_retain_original_generations_and_current_authority_after_restore() -> TestResult {
    let _serial = crate::test_serial();
    let (session_id, source) = fixture()?;
    let first = query(&session_id, request(Some(5)))?;
    assert_eq!(first["ok"], true, "{first}");
    let session = resolve_session(Some(session_id.clone()))?;
    let current = {
        let mut guard = session.lock().map_err(|_| "session poisoned")?;
        remember_version(&mut guard);
        let mut current = source.clone();
        current.tick = GameTick(2);
        current.cursor.epoch += 1;
        current.cursor.sequence = 0;
        current
            .graph
            .entities
            .get_mut(&EntityId::new(5))
            .ok_or("target missing")?
            .generation = 2;
        current.refresh_hash();
        guard.adapter.inject_snapshot(current.clone())?;
        guard
            .grants
            .iter_mut()
            .find(|grant| grant.capability == Capability::Query)
            .ok_or("Query grant missing")?
            .expires_at_tick = Some(GameTick(5));
        current
    };
    let mut retained = request(Some(5));
    retained["at"] = json!(source.state_hash.to_string());
    retained["request"]["expected_anchor"] = anchor_json(&source.anchor());
    let result = query(&session_id, retained.clone())?;
    assert_eq!(result["ok"], true, "{result}");
    assert_eq!(result["vertices"], first["vertices"]);
    assert_eq!(result["dominator_chain"], first["dominator_chain"]);
    assert_eq!(result["witness"], first["witness"]);
    assert_eq!(result["target_generation"], 1);
    assert_eq!(result["anchor"], anchor_json(&source.anchor()));
    assert_eq!(result["current_anchor"], anchor_json(&current.anchor()));
    assert_eq!(result["agent_turn"]["anchor"], result["current_anchor"]);
    assert_eq!(result["historical"], true);
    refusal(&query(&session_id, request(Some(5)))?, "conflict");
    let mut wrong_anchor = retained.clone();
    wrong_anchor["request"]["expected_anchor"] = anchor_json(&current.anchor());
    refusal(&query(&session_id, wrong_anchor)?, "stale_anchor");
    let mut missing = retained.clone();
    missing["at"] = json!(Digest32::of_bytes(b"unretained-dominator-source").to_string());
    refusal(&query(&session_id, missing)?, "stale_anchor");
    {
        let mut guard = session.lock().map_err(|_| "session poisoned")?;
        guard
            .grants
            .iter_mut()
            .find(|grant| grant.capability == Capability::Query)
            .ok_or("Query grant missing")?
            .expires_at_tick = Some(GameTick(1));
    }
    refusal(&query(&session_id, retained)?, "capability_denied");
    Ok(())
}

#[test]
fn lab_dominators_refuse_original_json_ambiguity_and_null_endpoint_handles() -> TestResult {
    let _serial = crate::test_serial();
    let (session_id, source) = fixture()?;
    for raw in [
        r#"{"mode":"structured","request":{"schema":"dfmcp.query/1","query":{"kind":"dominators","root":"1","root":"6","root_generation":1,"edge_kinds":["requires"]}}}"#,
        r#"{"mode":"structured","request":{"schema":"dfmcp.query/1","query":{"kind":"dominators","root":"1","root_generation":1,"edge_kinds":["requires"],"direction":"outgoing","direction":"incoming"}}}"#,
        r#"{"mode":"structured","request":{"schema":"dfmcp.query/1","query":{"kind":"dominators","root":"1","root_generation":1,"edge_kinds":["requires"],"target":"5","target_generation":1,"target_generation":2}}}"#,
        r#"{"mode":"structured","request":{"schema":"wrong","schema":"dfmcp.query/1","query":{"kind":"dominators","root":"1","root_generation":1,"edge_kinds":["requires"]}}}"#,
    ] {
        refusal(
            &parsed(&crate::agent_facade::fortress_query(
                Some(session_id.clone()),
                Some(raw.to_owned()),
            ))?,
            "invalid_request",
        );
    }
    for field in [
        "root",
        "root_generation",
        "edge_kinds",
        "target",
        "target_generation",
    ] {
        let mut malformed = request(Some(5));
        malformed["request"]["query"][field] = Value::Null;
        refusal(&query(&session_id, malformed)?, "invalid_request");
    }
    // A null optional direction is the documented default, not an endpoint.
    let mut default_direction = request(None);
    default_direction["request"]["query"]["direction"] = Value::Null;
    let accepted = query(&session_id, default_direction)?;
    assert_eq!(accepted["ok"], true, "{accepted}");
    assert_eq!(accepted["direction"], "outgoing");
    let session = resolve_session(Some(session_id))?;
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
fn lab_dominator_target_views_still_admit_complete_source_and_complete_packets() -> TestResult {
    let _serial = crate::test_serial();
    let (session_id, initial) = fixture()?;
    let session = resolve_session(Some(session_id.clone()))?;
    let source = {
        let mut guard = session.lock().map_err(|_| "session poisoned")?;
        let mut source = initial;
        source
            .graph
            .entities
            .get_mut(&EntityId::new(7))
            .ok_or("unreachable vertex missing")?
            .fields
            .insert(
                "unselected_payload".to_owned(),
                Fact::known(
                    WorldValue::Text("x".repeat(32_768)),
                    source.tick,
                    FactSource::Derived("dfmcp.lab-scenario/1".to_owned()),
                    Digest32::ZERO,
                ),
            );
        source.refresh_hash();
        guard.adapter.inject_snapshot(source.clone())?;
        source
    };
    let successful = query(&session_id, request(Some(5)))?;
    assert_eq!(successful["ok"], true, "{successful}");
    assert_eq!(successful["returned"], 3);
    assert_eq!(successful["source_work"]["records"], 15);
    let source_bytes = source.canonical_bytes().len() as u64;
    assert!(source_bytes > successful.to_string().len() as u64);
    {
        let mut guard = session.lock().map_err(|_| "session poisoned")?;
        // Seven vertices fit, but all eight relations are still admitted.
        guard.budget.max_entities = 14;
    }
    refusal(&query(&session_id, request(Some(5)))?, "budget_exceeded");
    {
        let mut guard = session.lock().map_err(|_| "session poisoned")?;
        guard.budget.max_entities = 128;
        guard.budget.max_bytes = source_bytes - 1;
    }
    refusal(&query(&session_id, request(Some(5)))?, "budget_exceeded");
    {
        let mut guard = session.lock().map_err(|_| "session poisoned")?;
        guard.budget.max_bytes = 1024 * 1024;
        guard.budget.max_output_tokens = 600;
    }
    refusal(&query(&session_id, request(Some(5)))?, "budget_exceeded");
    {
        let mut guard = session.lock().map_err(|_| "session poisoned")?;
        guard.budget.max_output_tokens = 8192;
    }
    let recovered = query(&session_id, request(Some(5)))?;
    assert_eq!(recovered["ok"], true, "{recovered}");
    assert_eq!(recovered["vertices"], successful["vertices"]);
    assert_eq!(recovered["witness"], successful["witness"]);
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
