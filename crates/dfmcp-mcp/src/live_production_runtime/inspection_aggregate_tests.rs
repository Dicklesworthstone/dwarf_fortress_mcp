//! Captured stock summaries reuse the exact aggregate engine and public anchors.
use super::*;
use dfmcp_adapter::live_jobs::LiveJobObservation;
use dfmcp_adapter::live_operations::{LiveItem, LiveOperationsObservation, LiveOperationsState};
use dfmcp_core::MapCoord;

fn source() -> Result<LiveOperationsState> {
    let mut source = LiveOperationsState::default();
    source.publish(LiveOperationsObservation {
        jobs: LiveJobObservation {
            bridge_generation: 71,
            df_version: "0.51.11".into(),
            dfhack_version: "0.51.11-r1".into(),
            year: 0,
            year_tick: 120,
            paused: true,
            site_id: 4,
            world_folder: "region1".into(),
            next_job_id: 100,
            jobs: Vec::new(),
        },
        next_building_id: 100,
        next_item_id: 100,
        buildings: Vec::new(),
        items: [
            (2, 0, "WOOD", 1, 16, Some(3)),
            (3, 6, "BIN", 1, 65, None),
            (4, 0, "WOOD", 7, 64, None),
            (5, 0, "WOOD", 99, 72, None),
            (6, 5, "BED", 1, 64, None),
        ]
        .into_iter()
        .map(
            |(native_id, item_type, type_key, stack_size, flags, container_native_id)| LiveItem {
                native_id,
                item_type,
                type_key: type_key.into(),
                subtype: -1,
                material_type: 420,
                material_index: 0,
                stack_size,
                raw_position: MapCoord::new(1, 2, 3),
                flags,
                container_native_id,
                holder_building_native_id: None,
            },
        )
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

fn group<'a>(result: &'a Value, item_type: &str) -> &'a Value {
    result["groups"]
        .as_array()
        .expect("complete groups")
        .iter()
        .find(|group| group["key"]["value"]["value"] == item_type)
        .expect("item-type group")
}

fn filtered_total(snapshot: &WorldSnapshot, field: &str, value: &str) -> Value {
    let mut input = inventory_totals_request(snapshot.anchor());
    input["query"]
        .as_object_mut()
        .expect("query")
        .remove("group_by");
    input["query"]["where"] = json!({"op":"compare","field":field,
        "comparison":"eq","value":{"type":"text","value":value}});
    input
}

#[test]
fn copied_stock_descriptor_reports_exact_record_counts_and_stack_totals() -> Result<()> {
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
        let before = snapshot.canonical_bytes();
        let input = inventory_totals_request(snapshot.anchor());
        let parsed = InspectionRequest::parse(&input.to_string())?;
        let result = execute_structured(&snapshot, &context(&snapshot), &parsed.0)?;
        assert_eq!(result["anchor"], input["expected_anchor"]);
        assert!(result["anchor"].get("game_tick").is_none());
        assert_eq!(result["kind"], "aggregate");
        assert_eq!(result["matched"], 5);
        assert_eq!(result["groups"].as_array().map(Vec::len), Some(3));
        assert_eq!(group(&result, "WOOD")["count"], 3);
        let wood = &group(&result, "WOOD")["metrics"]["stack_units"];
        assert_eq!(wood["samples"], 3);
        assert_eq!(wood["sum"], "107");
        assert_eq!(wood["minimum"], "1");
        assert_eq!(wood["maximum"], "99");
        assert_eq!(wood["mean"], json!({"numerator":"107","denominator":3}));
        assert_eq!(wood["sources"]["observed"], 3);
        for item_type in ["BED", "BIN"] {
            assert_eq!(group(&result, item_type)["count"], 1);
            assert_eq!(
                group(&result, item_type)["metrics"]["stack_units"]["sum"],
                "1"
            );
        }
        assert_eq!(result["stock_semantics"], INVENTORY_TOTALS_SEMANTICS);
        assert_eq!(result["eligibility"], "not_assessed");
        assert_eq!(result["reservation_created"], false);
        assert_eq!(result["original_goal_evidence"], false);
        assert_eq!(result["coverage"]["absence_proven"], false);
        assert_eq!(result["truncated"], false);
        assert!(result["continuation"].is_null());
        assert_eq!(snapshot.canonical_bytes(), before);
    }
    Ok(())
}

#[test]
fn observed_stock_totals_do_not_replace_conservative_inventory_allocation() -> Result<()> {
    let source = source()?;
    let snapshot = source.snapshot().expect("operations snapshot");
    let context = context(snapshot);
    let before = snapshot.anchor();
    let totals = execute_structured(snapshot, &context, &inventory_totals_request(before))?;
    let allocation = production::execute(
        &source,
        &context,
        &json!({
            "schema":"dfmcp.query/1","expected_anchor":anchor_json(before),
            "query":{"kind":"inventory_plan","quantity_unit":"stack_units","limit":8,
                "demands":[{"key":"first","units":4,"item_types":["WOOD"]},
                    {"key":"second","units":4,"item_types":["WOOD"]}]}
        }),
    )?;
    assert_eq!(
        group(&totals, "WOOD")["metrics"]["stack_units"]["sum"],
        "107"
    );
    assert_eq!(allocation["summary"]["candidate_stack_units"], 7);
    assert_eq!(allocation["summary"]["allocated_units"], 7);
    assert_eq!(allocation["certificate"]["shortage"]["deficit"], 1);
    assert_eq!(allocation["model_feasible"], false);
    assert_eq!(allocation["reservation_created"], false);
    assert_eq!(source.snapshot().expect("same source").anchor(), before);
    Ok(())
}

#[test]
fn stock_filters_distinguish_false_unknown_and_empty_numeric_populations() -> Result<()> {
    let source = source()?;
    let snapshot = source.snapshot().expect("operations snapshot");
    let context = context(snapshot);
    for (field, value, matched, excluded_false, excluded_unknown, sum) in [
        ("type_key", "BED", 1, 4, 0, json!("1")),
        ("type_key", "DOOR", 0, 5, 0, Value::Null),
        ("native_recipe_requirement", "WOOD", 0, 0, 5, Value::Null),
    ] {
        let result =
            execute_structured(snapshot, &context, &filtered_total(snapshot, field, value))?;
        assert_eq!(result["matched"], matched);
        assert_eq!(result["selection"]["selected_entities"], 5);
        assert_eq!(result["selection"]["excluded_false"], excluded_false);
        assert_eq!(result["selection"]["excluded_unknown"], excluded_unknown);
        assert_eq!(
            result["selection"]["complete_within_projection"],
            excluded_unknown == 0
        );
        assert_eq!(result["groups"][0]["key"], json!({"kind":"all"}));
        assert_eq!(result["groups"][0]["count"], matched);
        let metric = &result["groups"][0]["metrics"]["stack_units"];
        assert_eq!(metric["sum"], sum);
        if matched == 0 {
            for field in ["sum", "minimum", "maximum", "mean"] {
                assert!(metric[field].is_null(), "empty {field} must remain unknown");
            }
        }
        assert_eq!(result["coverage"]["absence_proven"], false);
    }
    // A declared numeric type is not a conversion request. Native stack_size
    // is u64, so asking for i64 cannot manufacture five numeric zeroes.
    let mut input = filtered_total(snapshot, "type_key", "WOOD");
    input["query"]["metrics"][0]["numeric_type"] = json!("i64");
    let result = execute_structured(snapshot, &context, &input)?;
    let metric = &result["groups"][0]["metrics"]["stack_units"];
    assert_eq!(result["matched"], 3);
    assert_eq!(metric["samples"], 0);
    assert_eq!(metric["excluded"]["type_or_scale_mismatch"], 3);
    assert!(metric["sum"].is_null());
    Ok(())
}

#[test]
fn aggregate_refuses_group_overflow_and_invalid_metrics_without_partial_results() -> Result<()> {
    let source = source()?;
    let snapshot = source.snapshot().expect("operations snapshot");
    let context = context(snapshot);
    let before = snapshot.canonical_bytes();
    for maximum in [0, 2, 129] {
        let mut input = inventory_totals_request(snapshot.anchor());
        input["query"]["max_groups"] = json!(maximum);
        assert!(matches!(execute_structured(snapshot, &context, &input),
            Err(error) if error.code == ErrorCode::BudgetExceeded));
    }
    for metric in [
        json!({"name":"stack_units","field":"stack_size","numeric_type":"integer"}),
        json!({"name":"stack_units","field":"stack_size","numeric_type":"fixed"}),
        json!({"name":"stack_units","field":"stack_size","numeric_type":"u64","scale":1}),
        json!({"name":"stack_units","field":"stack_size","numeric_type":"u64","reserve":true}),
    ] {
        let mut input = inventory_totals_request(snapshot.anchor());
        input["query"]["metrics"] = json!([metric]);
        assert!(matches!(execute_structured(snapshot, &context, &input),
            Err(error) if error.code == ErrorCode::InvalidRequest));
    }
    let mut duplicate = inventory_totals_request(snapshot.anchor());
    let metric = duplicate["query"]["metrics"][0].clone();
    duplicate["query"]["metrics"] = json!([metric.clone(), metric]);
    assert!(matches!(execute_structured(snapshot, &context, &duplicate),
        Err(error) if error.code == ErrorCode::InvalidRequest));
    assert_eq!(snapshot.canonical_bytes(), before);
    Ok(())
}

#[test]
fn aggregates_require_the_complete_public_anchor_and_current_query_authority() -> Result<()> {
    let source = source()?;
    let snapshot = source.snapshot().expect("operations snapshot");
    let context = context(snapshot);
    let input = inventory_totals_request(snapshot.anchor());
    let mut bad_anchors = vec![structured::anchor_json(snapshot.anchor())];
    for (field, value) in [
        ("tick", json!(snapshot.tick.get() + 1)),
        ("sequence", json!(snapshot.cursor.sequence + 1)),
        ("state_hash", json!(Digest32::ZERO.to_hex())),
        ("game_tick", json!(snapshot.tick.get())),
    ] {
        let mut anchor = input["expected_anchor"].clone();
        anchor[field] = value;
        bad_anchors.push(anchor);
    }
    for anchor in bad_anchors {
        let mut rejected = input.clone();
        rejected["expected_anchor"] = anchor;
        assert!(matches!(execute_structured(snapshot, &context, &rejected),
            Err(error) if error.code == ErrorCode::StaleAnchor));
    }
    let mut denied = context.clone();
    denied
        .grants
        .retain(|grant| grant.capability != Capability::Query);
    assert!(matches!(execute_structured(snapshot, &denied, &input),
        Err(error) if error.code == ErrorCode::CapabilityDenied));
    let mut expired = context.clone();
    for grant in &mut expired.grants {
        grant.expires_at_tick = Some(GameTick(snapshot.tick.get() - 1));
    }
    assert!(execute_structured(snapshot, &expired, &input).is_err());
    let mut cancelled = context;
    cancelled.cancellation_requested = true;
    assert!(execute_structured(snapshot, &cancelled, &input).is_err());
    Ok(())
}

#[test]
fn aggregate_source_and_final_stock_metadata_stay_inside_their_budgets() -> Result<()> {
    let source = source()?;
    let snapshot = source.snapshot().expect("operations snapshot");
    let context = context(snapshot);
    let input = inventory_totals_request(snapshot.anchor());
    let result = execute_structured(snapshot, &context, &input)?;
    let mut limited = context.clone();
    limited.budget.max_entities = snapshot.graph.entities.len() as u32 - 1;
    assert!(matches!(execute_structured(snapshot, &limited,
        &filtered_total(snapshot, "type_key", "DOOR")),
        Err(error) if error.code == ErrorCode::BudgetExceeded));
    // The added interpretation fields are inside the final aggregate allowance,
    // even if the shared engine's smaller undecorated result would fit.
    let bytes = serde_json::to_vec(&result)
        .expect("bounded aggregate")
        .len() as u64;
    limited = context.clone();
    limited.budget.max_bytes = bytes;
    assert_eq!(execute_structured(snapshot, &limited, &input)?, result);
    limited.budget.max_bytes -= 1;
    assert!(matches!(execute_structured(snapshot, &limited, &input),
        Err(error) if error.code == ErrorCode::BudgetExceeded));
    limited = context;
    limited.budget.max_output_tokens = 1;
    assert!(matches!(execute_structured(snapshot, &limited, &input),
        Err(error) if error.code == ErrorCode::BudgetExceeded));
    Ok(())
}
