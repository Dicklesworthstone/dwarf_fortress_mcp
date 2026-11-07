use super::*;
use std::collections::{BTreeMap, BTreeSet};

use dfmcp_core::{
    CapabilityGrant, CapabilityScope, EdgeId, EntityId, EventId, FortressId, GameTick,
    ObservationCursor, RequestId, SessionId, WorkBudget,
};
use dfmcp_world::{
    ChunkCoord, EdgeKind, EdgeRecord, EntityKind, EntityRecord, MapChunk, TerrainRun,
    WorldEvent, WorldEventKind, WorldGraph,
};

type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;

fn fixture() -> (WorldSnapshot, OperationContext) {
    let mut graph = WorldGraph::default();
    for id in 1..=2 {
        let entity = EntityRecord {
            id: EntityId::new(id),
            generation: 1,
            revision: 1,
            kind: EntityKind::Unit,
            label: format!("Urist {id}"),
            fields: BTreeMap::new(),
        };
        graph.entities.insert(entity.id, entity);
    }
    let edge = EdgeRecord {
        id: EdgeId::new(1),
        revision: 1,
        kind: EdgeKind::Requires,
        from: EntityId::new(1),
        to: EntityId::new(2),
        fields: BTreeMap::new(),
    };
    graph.edges.insert(edge.id, edge);
    let coord = ChunkCoord { x: 0, y: 0, z: 0 };
    graph.chunks.insert(
        coord,
        MapChunk {
            coord,
            revision: 1,
            width: 16,
            height: 16,
            terrain_runs: vec![TerrainRun {
                tile_code: 1,
                length: 256,
            }],
            sparse_overlays: BTreeMap::new(),
        },
    );
    let event = WorldEvent {
        id: EventId::new(1),
        tick: GameTick(1),
        kind: WorldEventKind::Announcement,
        subject: None,
        summary: "retained announcement".to_owned(),
        fields: BTreeMap::new(),
    };
    graph.events.insert(event.id, event);
    let source = WorldSnapshot::new(
        FortressId::new(1),
        GameTick(1),
        ObservationCursor::ORIGIN,
        true,
        graph,
    );
    let context = OperationContext {
        session_id: SessionId::new(1),
        request_id: RequestId::new(1),
        anchor: source.anchor(),
        budget: WorkBudget {
            max_wall_millis: 60_000,
            max_output_tokens: 8192,
            ..WorkBudget::default()
        },
        grants: vec![CapabilityGrant {
            capability: Capability::Query,
            scope: CapabilityScope {
                fortress_id: Some(source.fortress_id),
                ..CapabilityScope::default()
            },
            max_risk: RiskTier::ReadOnly,
            expires_at_tick: None,
            remaining_uses: None,
        }],
        cancellation_requested: false,
    };
    (source, context)
}

#[test]
fn every_canonical_record_and_byte_is_charged_before_execution() -> TestResult {
    let (source, mut context) = fixture();
    let bytes = source.canonical_bytes().len() as u64;
    context.budget.max_entities = 4;
    assert_eq!(
        admit_sources(&[&source], &context).map_err(|error| error.code),
        Err(ErrorCode::BudgetExceeded)
    );
    context.budget.max_entities = 5;
    context.budget.max_bytes = bytes;
    let work = admit_sources(&[&source], &context)?;
    assert_eq!(work, json!({"records":5, "canonical_bytes":bytes}));
    context.budget.max_bytes = bytes - 1;
    assert_eq!(
        admit_sources(&[&source], &context).map_err(|error| error.code),
        Err(ErrorCode::BudgetExceeded)
    );
    Ok(())
}

#[test]
fn two_sources_share_one_record_and_byte_allowance() -> TestResult {
    let (before, mut context) = fixture();
    let mut after = before.clone();
    after.tick = GameTick(2);
    after.cursor = after.cursor.checked_next().ok_or("cursor exhausted")?;
    after.refresh_hash();
    context.anchor = after.anchor();
    context.budget.max_entities = 9;
    assert!(admit_sources(&[&before], &context).is_ok());
    assert!(admit_sources(&[&after], &context).is_ok());
    assert_eq!(
        admit_sources(&[&before, &after], &context).map_err(|error| error.code),
        Err(ErrorCode::BudgetExceeded)
    );
    let bytes = (before.canonical_bytes().len() + after.canonical_bytes().len()) as u64;
    context.budget.max_entities = 10;
    context.budget.max_bytes = bytes - 1;
    assert_eq!(
        admit_sources(&[&before, &after], &context).map_err(|error| error.code),
        Err(ErrorCode::BudgetExceeded)
    );
    context.budget.max_bytes = bytes;
    assert_eq!(
        admit_sources(&[&before, &after], &context)?,
        json!({"records":10, "canonical_bytes":bytes})
    );
    Ok(())
}

#[test]
fn current_authority_precedes_source_cost_and_historical_selection() {
    let (source, mut context) = fixture();
    context.anchor.tick = GameTick(20);
    context.budget.max_entities = 1;
    for case in 0..4 {
        let mut denied = context.clone();
        match case {
            0 => denied.grants.clear(),
            1 => denied.grants[0].expires_at_tick = Some(GameTick(10)),
            2 => denied.grants[0].scope.entity_ids = BTreeSet::from([EntityId::new(1)]),
            _ => denied.grants[0].remaining_uses = Some(0),
        }
        assert_eq!(
            admit_sources(&[&source], &denied).map_err(|error| error.code),
            Err(ErrorCode::CapabilityDenied)
        );
    }
    context.cancellation_requested = true;
    assert_eq!(
        admit_sources(&[&source], &context).map_err(|error| error.code),
        Err(ErrorCode::CancellationRequested)
    );
}

fn page(mode: &str) -> Value {
    let query = json!({"mode":mode, "limit":16, "offset":0, "at":"exact-retained-source",
        "where":{"field":"alive", "op":"eq", "value":true}});
    let coverage = json!({"status":"complete_for_named_projection",
        "complete_domains":["laboratory.entities"], "partial_domains":[],
        "absence_proof_scope":["laboratory.entities"], "page_complete":true});
    json!({
        "ok":true, "mode":mode, "session_id":"1", "offset":0, "returned":16,
        "total":16, "next_offset":null, "continuation":null, "query":query,
        "truncated":false, "complete_domain":true, "absence_proven":false,
        "rows":(1..=16).map(|id| json!({"entity_id":id.to_string(),
            "label":"bounded complete entity ".repeat(12),
            "fields":{"coordinates":[1,2,3]}})).collect::<Vec<_>>(),
        "observation_coverage":coverage,
        "agent_turn":{
            "schema":"dfmcp.agent_turn/1", "operation":"fortress.query",
            "anchor":{"state_hash":"current"},
            "continuity":{"status":"continuous"}, "coverage":coverage,
            "active_work":{}, "recommendations":[], "uncertainty":[],
        },
    })
}

#[test]
fn legacy_whole_rows_and_cursors_fit_current_bytes_and_tokens_together() -> TestResult {
    let (_, mut context) = fixture();
    context.budget.max_bytes = 4095;
    context.budget.max_output_tokens = 8192;
    for mode in ["entities", "observation"] {
        let encoded = finish_response(
            response(Ok(page(mode)), &context, false),
            Duration::ZERO,
        );
        assert!(encoded.len() <= 4095);
        assert!(encoded.len().div_ceil(4) <= 1023);
        let result: Value = serde_json::from_str(&encoded)?;
        assert_eq!(result["ok"], true, "{result}");
        assert_eq!(result["output_budget"]["tier"], "truncated");
        let rows = result["rows"].as_array().ok_or("rows missing")?;
        assert!(!rows.is_empty() && rows.len() < 16);
        assert_eq!(result["returned"], rows.len());
        assert_eq!(result["continuation"]["offset"], rows.len());
        assert_eq!(result["continuation"]["at"], "exact-retained-source");
        assert_eq!(result["continuation"]["where"], page(mode)["query"]["where"]);
        assert_eq!(result["rows"][0]["fields"]["coordinates"], json!([1,2,3]));
        assert_eq!(result["complete_domain"], false);
        assert_eq!(result["observation_coverage"]["absence_proof_scope"], json!([]));
        assert_eq!(
            result["agent_turn"]["coverage"]["continuation"],
            result["continuation"]
        );
    }
    Ok(())
}

#[test]
fn structured_cursor_and_oversized_first_legacy_row_are_refused_without_advancing() -> TestResult {
    let (_, mut context) = fixture();
    context.budget.max_bytes = 4096;
    let mut oversized = page("entities");
    oversized["rows"] = json!([{"entity_id":"1", "label":"x".repeat(16_384)}]);
    oversized["total"] = json!(1);
    oversized["returned"] = json!(1);
    for (payload, structured) in [(page("structured"), true), (oversized, false)] {
        let encoded = finish_response(response(Ok(payload), &context, structured), Duration::ZERO);
        let result: Value = serde_json::from_str(&encoded)?;
        assert!(encoded.len() <= 4096);
        assert_eq!(result["ok"], false, "{result}");
        assert_eq!(result["error"]["code"], "budget_exceeded");
        assert!(result.get("rows").is_none());
        assert!(result.get("continuation").is_none());
        assert_eq!(result["agent_turn"]["anchor"]["state_hash"], "current");
        assert_eq!(result["agent_turn"]["coverage"]["absence_proof_scope"], json!([]));
    }
    Ok(())
}

#[test]
fn publication_deadline_refuses_a_legacy_result_without_a_hidden_page() -> TestResult {
    let (_, mut context) = fixture();
    context.budget.max_bytes = 4096;
    let encoded = finish_response(
        response(Ok(page("entities")), &context, false),
        Duration::from_millis(context.budget.max_wall_millis),
    );
    let result: Value = serde_json::from_str(&encoded)?;
    assert!(encoded.len() <= 4096);
    assert_eq!(result["ok"], false, "{result}");
    assert_eq!(result["error"]["code"], "budget_exceeded");
    assert!(
        result["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("wall-time"))
    );
    assert!(result.get("rows").is_none());
    assert!(result.get("continuation").is_none());
    assert_eq!(result["error"]["effect_may_have_occurred"], false);
    Ok(())
}
