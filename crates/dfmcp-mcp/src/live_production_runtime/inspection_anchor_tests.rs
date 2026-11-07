//! The combined host's exact source handles must work with both query engines.
use super::*;
use dfmcp_adapter::live_jobs::{LiveJob, LiveJobObservation};
use dfmcp_adapter::live_operations::{
    LiveItem, LiveOperationsObservation, LiveOperationsState, item_entity_id,
};
use dfmcp_core::MapCoord;
use dfmcp_world::{Fact, FactPresence, FactSource, Value as WorldValue};
use std::collections::BTreeMap;

fn source() -> Result<LiveOperationsState> {
    let mut source = LiveOperationsState::default();
    source.publish(LiveOperationsObservation {
        jobs: LiveJobObservation {
            bridge_generation: 601,
            df_version: "0.51.11".into(),
            dfhack_version: "0.51.11-r1".into(),
            year: 0,
            year_tick: 120,
            paused: true,
            site_id: 4,
            world_folder: "region1".into(),
            next_job_id: 100,
            jobs: vec![LiveJob {
                native_id: 3,
                job_type: 0,
                type_key: "ConstructBed".into(),
                reaction: String::new(),
                suspended: true,
                repeating: false,
                position: MapCoord::new(1, 2, 3),
                worker_native_id: None,
                holder_native_id: None,
                completion_timer: -1,
                attached_item_count: 0,
                required_item_filter_count: 1,
            }],
        },
        next_building_id: 200,
        next_item_id: 300,
        buildings: Vec::new(),
        items: [8, 9]
            .into_iter()
            .map(|native_id| LiveItem {
                native_id,
                item_type: 5,
                type_key: "BED".into(),
                subtype: -1,
                material_type: 420,
                material_index: 0,
                stack_size: 1,
                raw_position: MapCoord::new(1, 2, 3),
                flags: 64,
                container_native_id: None,
                holder_building_native_id: None,
            })
            .collect(),
        attachments: Vec::new(),
    })?;
    Ok(source)
}

fn context(snapshot: &WorldSnapshot) -> OperationContext {
    OperationContext {
        session_id: SessionId::new(7),
        request_id: RequestId::new(1),
        anchor: snapshot.anchor(),
        budget: WorkBudget {
            max_wall_millis: 10_000,
            max_bytes: 512 * MIB,
            max_entities: 8192,
            max_actions: 1,
            max_game_ticks: 0,
            max_output_tokens: MAX_QUERY_TOKENS,
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

fn request(snapshot: &WorldSnapshot, query: Value) -> Value {
    json!({"schema":"dfmcp.query/1","expected_anchor":anchor_json(snapshot.anchor()),"query":query})
}

#[test]
fn captured_source_inventory_handles_work_with_source_local_and_selected_cursors() -> Result<()> {
    let source = source()?;
    for cursor in [
        ObservationCursor::ORIGIN,
        ObservationCursor {
            epoch: 4,
            sequence: 7,
        },
    ] {
        let mut snapshot = source.snapshot().expect("operations snapshot").clone();
        snapshot.cursor = cursor;
        snapshot.refresh_hash();
        let context = context(&snapshot);
        let input = request(
            &snapshot,
            json!({"kind":"entities","kinds":["item"],
            "fields":["type_key","stack_size","material_type","material_index"],"limit":16}),
        );
        let parsed = InspectionRequest::parse(&input.to_string())?;
        let result = execute_structured(&snapshot, &context, &parsed.0)?;
        assert_eq!(result["anchor"], input["expected_anchor"]);
        assert_eq!(result["matched"], 2);
        assert_eq!(result["returned"], 2);
        assert_eq!(
            result["rows"][0]["fields"]["stack_size"]["value"]["value"],
            1
        );
        assert!(result["anchor"].get("game_tick").is_none());

        // The engine's other hosts retain their established anchor contract.
        assert!(matches!(structured::execute(&snapshot, &context, &input),
            Err(error) if error.code == ErrorCode::StaleAnchor));
        let mut direct = input.clone();
        direct["expected_anchor"] = structured::anchor_json(snapshot.anchor());
        let direct = structured::execute(&snapshot, &context, &direct)?;
        assert_eq!(direct["anchor"]["game_tick"], snapshot.tick.get());
        assert!(direct["anchor"].get("tick").is_none());
    }
    Ok(())
}

#[test]
fn diagnosis_drilldowns_and_graph_witnesses_retain_the_public_source_anchor() -> Result<()> {
    let source = source()?;
    let snapshot = source.snapshot().expect("operations snapshot");
    let context = context(snapshot);
    let diagnosis = production::execute(
        &source,
        &context,
        &request(snapshot, json!({"kind":"production_diagnosis","limit":4})),
    )?;
    assert_eq!(
        diagnosis["rows"].as_array().expect("diagnosis rows").len(),
        1
    );
    for field in ["inspect_assignment", "inspect_relationships"] {
        let followup = &diagnosis["rows"][0][field];
        let parsed = InspectionRequest::parse(&followup.to_string())?;
        let result = execute_structured(snapshot, &context, &parsed.0)?;
        assert_eq!(result["anchor"], followup["expected_anchor"]);
        if field == "inspect_relationships" {
            assert_eq!(result["witness"]["anchor"], result["anchor"]);
            assert!(result["witness"]["anchor"].get("game_tick").is_none());
        } else {
            assert_eq!(result["row"]["entity_id"], followup["query"]["entity_id"]);
        }
    }
    let result = execute_structured(
        snapshot,
        &context,
        &request(
            snapshot,
            json!({"kind":"dependencies","edge_kinds":["uses"]}),
        ),
    )?;
    assert_eq!(result["witness"]["anchor"], anchor_json(snapshot.anchor()));
    Ok(())
}

#[test]
fn translation_refuses_every_changed_or_malformed_complete_anchor() -> Result<()> {
    let source = source()?;
    let snapshot = source.snapshot().expect("operations snapshot");
    let context = context(snapshot);
    for case in 0..9 {
        let mut input = request(snapshot, json!({"kind":"entities","kinds":["item"]}));
        let expected = &mut input["expected_anchor"];
        match case {
            0 => expected["fortress_id"] = json!("0"),
            1 => expected["epoch"] = json!(99),
            2 => expected["sequence"] = json!(99),
            3 => expected["tick"] = json!(121),
            4 => expected["state_hash"] = json!("f".repeat(64)),
            5 => expected["tick"] = json!("120"),
            6 => *expected = structured::anchor_json(snapshot.anchor()),
            7 => expected["game_tick"] = json!(120),
            _ => {
                expected
                    .as_object_mut()
                    .expect("anchor")
                    .remove("state_hash");
            }
        }
        let parsed = InspectionRequest::parse(&input.to_string())?;
        assert!(
            matches!(execute_structured(snapshot, &context, &parsed.0),
            Err(error) if error.code == ErrorCode::StaleAnchor),
            "case {case}"
        );
    }
    // Omitting the optional precondition still queries the exact retained source.
    let input = json!({"schema":"dfmcp.query/1","query":{"kind":"entities","kinds":["item"]}});
    assert_eq!(
        execute_structured(snapshot, &context, &input)?["matched"],
        2
    );
    Ok(())
}

#[test]
fn translated_pagination_keeps_opaque_session_fields_and_snapshot_binding() -> Result<()> {
    let source = source()?;
    let snapshot = source.snapshot().expect("operations snapshot");
    let context = context(snapshot);
    let input = request(
        snapshot,
        json!({"kind":"entities","kinds":["item"],"limit":1}),
    );
    let first = execute_structured(snapshot, &context, &input)?;
    let mut direct = input.clone();
    direct["expected_anchor"] = structured::anchor_json(snapshot.anchor());
    assert_eq!(
        first["continuation"],
        structured::execute(snapshot, &context, &direct)?["continuation"]
    );
    let resumed = request(
        snapshot,
        json!({"kind":"entities","kinds":["item"],"limit":1,
        "continuation":first["continuation"]}),
    );
    let second = execute_structured(snapshot, &context, &resumed)?;
    assert_eq!(
        second["rows"][0]["entity_id"],
        item_entity_id(9).to_string()
    );
    assert!(second["continuation"].is_null());
    for case in 0..3 {
        let mut changed = snapshot.clone();
        let mut current = context.clone();
        let mut request = resumed.clone();
        match case {
            0 => current.session_id = SessionId::new(8),
            1 => request["query"]["fields"] = json!(["stack_size"]),
            _ => {
                changed.cursor.sequence += 1;
                changed.refresh_hash();
                current.anchor = changed.anchor();
                request["expected_anchor"] = anchor_json(changed.anchor());
            }
        }
        assert!(
            matches!(execute_structured(&changed, &current, &request),
            Err(error) if error.code == ErrorCode::StaleAnchor),
            "case {case}"
        );
    }
    Ok(())
}

#[test]
fn stale_fact_anchors_keep_historical_identity_and_object_values_are_untouched() -> Result<()> {
    let source = source()?;
    let mut snapshot = source.snapshot().expect("operations snapshot").clone();
    let historical = StateAnchor {
        tick: GameTick(119),
        state_hash: Digest32::of_bytes(b"historical fact"),
        ..snapshot.anchor()
    };
    let fields = &mut snapshot
        .graph
        .entities
        .get_mut(&item_entity_id(8))
        .expect("item")
        .fields;
    fields.insert(
        "previous".into(),
        Fact::with_presence(
            FactPresence::Stale(historical),
            GameTick(119),
            FactSource::Replay,
            Digest32::of_bytes(b"old source"),
        ),
    );
    fields.insert(
        "object".into(),
        Fact::known(
            WorldValue::Object(BTreeMap::from([("game_tick".into(), WorldValue::U64(77))])),
            GameTick(120),
            FactSource::Replay,
            Digest32::of_bytes(b"object source"),
        ),
    );
    snapshot.refresh_hash();
    let context = context(&snapshot);
    for query in [
        json!({"kind":"inspect","entity_id":item_entity_id(8).to_string(),"generation":1,
            "fields":["previous","object"]}),
        json!({"kind":"entities","kinds":["item"],"fields":["previous","object"],"limit":1}),
    ] {
        let result = execute_structured(&snapshot, &context, &request(&snapshot, query))?;
        let row = if result["kind"] == "inspect" {
            &result["row"]
        } else {
            &result["rows"][0]
        };
        assert_eq!(row["fields"]["previous"]["reason"], anchor_json(historical));
        assert_eq!(row["fields"]["previous"]["presence"], "stale");
        assert_eq!(
            row["fields"]["object"]["value"]["value"]["game_tick"]["value"],
            77
        );
    }
    Ok(())
}
