use super::anchor_json;
use super::production::{execute, query_schema};
use dfmcp_adapter::live_jobs::LiveJobObservation;
use dfmcp_adapter::live_operations::{
    LiveItem, LiveOperationsObservation, LiveOperationsState, OperationsProfile, item_entity_id,
};
use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, DfmcpError, ErrorCode, MapCoord,
    OperationContext, RequestId, Result, RiskTier, SessionId, WorkBudget,
};
use serde_json::{Value, json};

fn invariant(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::InternalInvariantViolation, message)
}

fn item(id: u32, kind: &str, x: i32, material_index: i32) -> LiveItem {
    LiveItem {
        native_id: id,
        item_type: 1,
        type_key: kind.to_owned(),
        subtype: -1,
        material_type: 0,
        material_index,
        stack_size: 1,
        raw_position: MapCoord::new(x, 10, 0),
        flags: 64,
        container_native_id: None,
        holder_building_native_id: None,
    }
}

fn observation(items: Vec<LiveItem>) -> LiveOperationsObservation {
    LiveOperationsObservation {
        jobs: LiveJobObservation {
            bridge_generation: 7,
            df_version: "df".to_owned(),
            dfhack_version: "dfhack".to_owned(),
            year: 105,
            year_tick: 3,
            paused: true,
            site_id: 2,
            world_folder: "region1".to_owned(),
            next_job_id: 0,
            jobs: Vec::new(),
        },
        next_building_id: 0,
        next_item_id: items
            .iter()
            .map(|item| item.native_id)
            .max()
            .map_or(0, |id| id + 1),
        buildings: Vec::new(),
        items,
        attachments: Vec::new(),
    }
}

fn state(observation: LiveOperationsObservation) -> Result<LiveOperationsState> {
    let mut state = LiveOperationsState::default();
    state.publish(observation)?;
    Ok(state)
}

fn context(state: &LiveOperationsState) -> Result<OperationContext> {
    let anchor = state
        .snapshot()
        .ok_or_else(|| invariant("test snapshot"))?
        .anchor();
    Ok(OperationContext {
        session_id: SessionId::new(57),
        request_id: RequestId::new(1),
        anchor,
        budget: WorkBudget {
            max_wall_millis: 30_000,
            max_game_ticks: 1_000,
            max_entities: 100_000,
            max_bytes: 65_536,
            max_output_tokens: 16_384,
            max_actions: 1,
        },
        grants: vec![CapabilityGrant {
            capability: Capability::Query,
            scope: CapabilityScope {
                fortress_id: Some(anchor.fortress_id),
                ..CapabilityScope::default()
            },
            max_risk: RiskTier::ReadOnly,
            expires_at_tick: None,
            remaining_uses: None,
        }],
        cancellation_requested: false,
    })
}

fn slot(name: &str, kind: &str, x: u32) -> Value {
    json!({"name":name,"kind":kind,"target":[x,10,0]})
}

fn request(slots: Vec<Value>) -> Value {
    json!({"schema":"dfmcp.query/1","query":{"kind":"furniture_allocation",
        "world_folder":"region1","site":2,"slots":slots}})
}

#[test]
fn furniture_global_assignment_returns_the_actual_complete_plan_and_source_handles() -> Result<()> {
    let state = state(observation(vec![
        item(100, "BED", 10, 1),
        item(101, "BED", 20, 2),
        item(102, "CHAIR", 12, 3),
        item(103, "TABLE", 13, 4),
    ]))?;
    let context = context(&state)?;
    let mut special = slot("b-special", "bed", 11);
    special["material"] = json!([0, 1]);
    let mut table = slot("d-table", "table", 13);
    table["after"] = json!(["c-chair", "a-generic"]);
    let input = request(vec![
        table,
        special,
        slot("a-generic", "bed", 10),
        slot("c-chair", "chair", 12),
    ]);
    let result = execute(&state, &context, &input)?;
    assert_eq!(result["status"], "allocated");
    assert_eq!(result["summary"]["maximum_assignable"], 4);
    assert_eq!(result["total_distance"], 11);
    assert_eq!(result["assignments"][0]["item"], 101);
    assert_eq!(result["assignments"][1]["item"], 100);
    assert_eq!(
        result["plan"],
        json!({"schema":"dfmcp.furniture-plan/1","steps":[
            {"name":"a-generic","kind":"bed","item":101,"target":[10,10,0],"after":[]},
            {"name":"b-special","kind":"bed","item":100,"target":[11,10,0],"after":[]},
            {"name":"c-chair","kind":"chair","item":102,"target":[12,10,0],"after":[]},
            {"name":"d-table","kind":"table","item":103,"target":[13,10,0],"after":["a-generic","c-chair"]}
        ]})
    );
    for (index, id) in [101, 100, 102, 103].into_iter().enumerate() {
        assert_eq!(
            result["assignments"][index]["item_handle"]["entity_id"],
            item_entity_id(id).to_string()
        );
        assert_eq!(result["assignments"][index]["item_handle"]["generation"], 1);
    }
    assert_eq!(result["anchor"], anchor_json(context.anchor));
    assert_eq!(result["source_digest"], state.source_digest()?.to_string());
    assert_eq!(result["source"]["bridge_generation"], 7);
    assert_eq!(result["mutation_authority"], false);
    assert_eq!(result["placement_eligibility_proven"], false);
    assert_eq!(result["commit_compatible"], false);
    assert_eq!(result["truncated"], false);
    assert!(result["shortage"].is_null());
    Ok(())
}

#[test]
fn furniture_ascii_artifact_digests_match_the_independent_python_codec() -> Result<()> {
    let mut observed = observation(vec![item(100, "BED", 10, 0)]);
    observed.jobs.world_folder = "région🪨\u{7f}".to_owned();
    let state = state(observed)?;
    let mut input = request(vec![slot("bed", "bed", 10)]);
    input["query"]["world_folder"] = json!("région🪨\u{7f}");
    let result = execute(&state, &context(&state)?, &input)?;
    // Golden values produced by scripts/furniture_allocation.py Request.digest
    // and scripts/furniture_plan.py FurniturePlan.digest; no subprocess at runtime.
    assert_eq!(
        result["request_digest"],
        "d873e71ebb429a94c7fc33acc107a56fcac41bb3f644a1dd99985e6e355e9b5c"
    );
    assert_eq!(
        result["plan_digest"],
        "0b4afcf7bb06e0be09117df20dfeac5dfc2944d1b829f73609d0290ac90a6a10"
    );
    Ok(())
}

#[test]
fn furniture_joint_shortage_keeps_complete_candidate_evidence_and_emits_no_partial_plan()
-> Result<()> {
    let state = state(observation(vec![
        item(100, "BED", 10, 1),
        item(101, "CHAIR", 12, 1),
    ]))?;
    let result = execute(
        &state,
        &context(&state)?,
        &request(vec![
            slot("a", "bed", 10),
            slot("b", "bed", 11),
            slot("chair", "chair", 12),
        ]),
    )?;
    assert_eq!(result["status"], "shortage");
    assert_eq!(result["summary"]["maximum_assignable"], 2);
    assert_eq!(result["shortage"]["slots"], json!(["a", "b"]));
    assert_eq!(result["shortage"]["candidate_items"], json!([100]));
    assert_eq!(result["shortage"]["missing"], 1);
    assert_eq!(
        result["shortage"]["candidate_evidence"][0]["item_handle"]["entity_id"],
        item_entity_id(100).to_string()
    );
    assert_eq!(result["assignments"], json!([]));
    assert!(result["plan"].is_null());
    assert!(result["plan_digest"].is_null());
    assert!(result["total_distance"].is_null());
    assert!(result["continuation"].is_null());
    Ok(())
}

#[test]
fn furniture_normalization_is_stable_and_source_reappearance_changes_the_bound_identity()
-> Result<()> {
    let original = observation(vec![item(100, "BED", 10, 1), item(101, "BED", 10, 1)]);
    let mut state = state(original.clone())?;
    let initial_context = context(&state)?;
    let mut first = request(vec![slot("b", "bed", 11), slot("a", "bed", 10)]);
    first["query"]["excluded_items"] = json!([200, 199]);
    let mut reordered = request(vec![slot("a", "bed", 10), slot("b", "bed", 11)]);
    reordered["query"]["excluded_items"] = json!([199, 200]);
    let result = execute(&state, &initial_context, &first)?;
    let canonical = execute(&state, &initial_context, &reordered)?;
    for key in [
        "request",
        "request_digest",
        "analysis_digest",
        "plan",
        "plan_digest",
        "assignments",
    ] {
        assert_eq!(result[key], canonical[key]);
    }
    assert_eq!(result["assignments"][0]["item"], 100);
    let mut absent = original.clone();
    absent.items.clear();
    absent.jobs.year_tick += 1;
    state.publish(absent)?;
    let mut returned = original;
    returned.jobs.year_tick += 2;
    state.publish(returned)?;
    assert!(
        matches!(execute(&state, &initial_context, &first), Err(e) if e.code == ErrorCode::StaleAnchor)
    );
    let changed = execute(&state, &context(&state)?, &first)?;
    assert_eq!(result["request_digest"], changed["request_digest"]);
    assert_eq!(result["plan_digest"], changed["plan_digest"]);
    assert_ne!(result["analysis_digest"], changed["analysis_digest"]);
    assert_ne!(result["source_digest"], changed["source_digest"]);
    assert_eq!(changed["assignments"][0]["item_handle"]["generation"], 2);
    Ok(())
}

#[test]
fn furniture_strict_requests_reject_wrong_fortress_constraints_and_partial_plan_controls()
-> Result<()> {
    let state = state(observation(vec![item(100, "BED", 10, 1)]))?;
    let context = context(&state)?;
    let input = request(vec![slot("a", "bed", 10)]);
    let mutations = [
        ("world_folder", json!("other")),
        ("site", json!(3)),
        ("site", json!(2_147_483_648u64)),
        ("maximum_work", json!(0)),
        ("maximum_work", json!(1)),
        ("maximum_work", json!(10_000_001)),
        ("limit", json!(1)),
        ("continuation", Value::Null),
        ("unexpected", json!(true)),
        ("excluded_items", json!([100, 100])),
        ("excluded_items", json!([-1])),
    ];
    for (field, value) in mutations {
        let mut invalid = input.clone();
        invalid["query"][field] = value;
        assert!(
            execute(&state, &context, &invalid).is_err(),
            "accepted {field}"
        );
    }
    for (field, value) in [
        ("kind", json!("door")),
        ("target", json!([0, 10, 0])),
        ("target", json!([10, 10, 0, 1])),
        ("target", json!([10.0, 10, 0])),
        ("after", json!(["missing"])),
        ("after", json!(["a"])),
        ("material", json!([-1, 0])),
        ("material", json!([0])),
        ("max_distance", Value::Null),
        ("max_distance", json!(65_533)),
        ("subtype", json!(-2)),
        ("extra", json!(true)),
    ] {
        let mut invalid = input.clone();
        invalid["query"]["slots"][0][field] = value;
        assert!(
            execute(&state, &context, &invalid).is_err(),
            "accepted slot {field}"
        );
    }
    for slots in [
        vec![],
        vec![slot("a", "bed", 10); 33],
        vec![slot("a", "bed", 10), slot("b", "bed", 10)],
    ] {
        assert!(execute(&state, &context, &request(slots)).is_err());
    }
    let mut stale = input;
    stale["expected_anchor"] = json!({});
    assert!(
        matches!(execute(&state, &context, &stale), Err(e) if e.code == ErrorCode::StaleAnchor)
    );
    Ok(())
}

#[test]
fn furniture_authority_cancellation_and_complete_output_budgets_fail_closed() -> Result<()> {
    let state = state(observation(vec![item(100, "BED", 10, 1)]))?;
    let context = context(&state)?;
    let input = request(vec![slot("a", "bed", 10)]);
    let mut denied = context.clone();
    denied.grants.clear();
    assert!(
        matches!(execute(&state, &denied, &input), Err(e) if e.code == ErrorCode::CapabilityDenied)
    );
    denied = context.clone();
    denied.cancellation_requested = true;
    assert!(execute(&state, &denied, &input).is_err());
    denied = context.clone();
    denied.budget.max_entities = 1;
    assert!(
        matches!(execute(&state, &denied, &input), Err(e) if e.code == ErrorCode::BudgetExceeded)
    );
    denied = context.clone();
    denied.budget.max_bytes = 100;
    assert!(
        matches!(execute(&state, &denied, &input), Err(e) if e.code == ErrorCode::BudgetExceeded)
    );
    denied = context.clone();
    denied.budget.max_output_tokens = 1;
    assert!(
        matches!(execute(&state, &denied, &input), Err(e) if e.code == ErrorCode::BudgetExceeded)
    );
    // Fits the generic envelope bound but not the canonical furniture artifact.
    let mut oversized = input;
    oversized["query"]["excluded_items"] = json!((1_000_000..1_002_400).collect::<Vec<_>>());
    assert!(
        matches!(execute(&state, &context, &oversized), Err(e) if e.code == ErrorCode::BudgetExceeded)
    );
    Ok(())
}

#[test]
fn furniture_schema_discovery_advertises_exact_nonpaginated_request() -> Result<()> {
    let schema = query_schema()?;
    let variants = schema["$defs"]["query"]["oneOf"]
        .as_array()
        .ok_or_else(|| invariant("test query schema variants"))?;
    let furniture = variants
        .iter()
        .find(|query| query["properties"]["kind"]["const"] == "furniture_allocation")
        .ok_or_else(|| invariant("furniture query absent from schema"))?;
    assert_eq!(
        furniture["required"],
        json!(["kind", "world_folder", "site", "slots"])
    );
    assert_eq!(furniture["additionalProperties"], false);
    assert_eq!(furniture["properties"]["slots"]["maxItems"], 32);
    assert!(furniture["properties"].get("limit").is_none());
    assert!(furniture["properties"].get("continuation").is_none());
    assert_eq!(
        schema["$defs"]["operations_furniture_slot"]["properties"]["after"]["maxItems"],
        31
    );
    Ok(())
}

// One handler-level test holds at most one production session at a time.
// Other tests use private sealed states and cannot consume global session slots.
#[test]
fn furniture_actual_query_route_covers_both_operations_profiles_complete_budget_and_fencing()
-> Result<()> {
    use super::{
        OperationsLimits, OperationsSession, OperationsSource, SESSIONS, SessionSlot,
        fortress_commit, fortress_query, lock, next_id, resolve,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;
    struct NoRead {
        calls: Arc<AtomicUsize>,
        fenced: bool,
    }
    impl OperationsSource for NoRead {
        fn read(&mut self, _: Duration) -> Result<LiveOperationsObservation> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Err(invariant(
                "furniture query unexpectedly acquired native state",
            ))
        }
        fn poisoned(&self) -> bool {
            self.fenced
        }
        fn fence(&mut self) {
            self.fenced = true;
        }
    }
    struct Registered(SessionId);
    impl Drop for Registered {
        fn drop(&mut self) {
            if let Ok(mut sessions) = SESSIONS.lock() {
                sessions.remove(&self.0);
            }
        }
    }
    let decode =
        |raw: &str| serde_json::from_str::<Value>(raw).map_err(|_| invariant("test response JSON"));
    for profile in [OperationsProfile::V1_3, OperationsProfile::PagedV1_4] {
        let kinds = ["BED", "CHAIR", "TABLE"];
        let mut state = LiveOperationsState::with_profile(profile);
        state.publish(observation(
            (0..32)
                .map(|i| item(100 + i, kinds[i as usize % 3], 10 + i as i32, 0))
                .collect(),
        ))?;
        let mut allowed = context(&state)?;
        let query_grant = allowed.grants[0].clone();
        allowed
            .grants
            .extend(
                [Capability::Observe, Capability::Doctor].map(|capability| CapabilityGrant {
                    capability,
                    ..query_grant.clone()
                }),
            );
        let id = next_id()?;
        let calls = Arc::new(AtomicUsize::new(0));
        let session = OperationsSession {
            id,
            source: Box::new(NoRead {
                calls: Arc::clone(&calls),
                fenced: false,
            }),
            state,
            journal: None,
            limits: OperationsLimits::default(),
            budget: allowed.budget,
            grants: allowed.grants,
            request: 0,
            _slot: SessionSlot::reserve()?,
        };
        lock(&SESSIONS)?.insert(id, Arc::new(Mutex::new(session)));
        let _registered = Registered(id);
        let handle = Some(id.to_string());
        let watch = json!({"schema":"dfmcp.query/1","query":{"kind":"watch",
            "key":"furnishing-review","label":"Wait for review tick",
            "condition":{"op":"tick_at_least","value":105u64*403200+50},
            "deadline_tick":105u64*403200+100,"poll_interval_ticks":1,"stable_observations":1}});
        let watched = decode(&fortress_query(handle.clone(), None, Some(watch)))?;
        assert_eq!(watched["ok"], true, "{watched}");
        let input = request(
            (0..32)
                .map(|i| {
                    slot(
                        &format!("furniture-{i:02}"),
                        kinds[i as usize % 3].to_ascii_lowercase().as_str(),
                        10 + i,
                    )
                })
                .collect(),
        );
        let raw = fortress_query(handle.clone(), None, Some(input.clone()));
        assert!(raw.len() <= 65_536);
        let result = decode(&raw)?;
        assert_eq!(result["ok"], true, "{raw}");
        assert_eq!(result["status"], "allocated");
        assert_eq!(result["plan"]["steps"].as_array().map(Vec::len), Some(32));
        assert_eq!(result["assignments"].as_array().map(Vec::len), Some(32));
        assert_eq!(
            result["agent_turn"]["briefing"]["bridge_protocol"],
            profile.protocol()
        );
        assert_eq!(result["agent_turn"]["briefing"]["runtime_admitted"], false);
        assert_eq!(
            result["agent_turn"]["active_work"]["obligations"]
                .as_array()
                .map(Vec::len),
            Some(1)
        );
        assert_eq!(result["anchor"], result["agent_turn"]["anchor"]);
        assert_eq!(
            decode(&fortress_commit(handle.clone()))?["error"]["code"],
            "capability_denied"
        );
        lock(&*resolve(handle.clone())?)?.budget.max_output_tokens = 2048;
        let raw = fortress_query(handle.clone(), None, Some(input.clone()));
        assert!(raw.len() <= 8192);
        let refused = decode(&raw)?;
        assert_eq!(refused["error"]["code"], "budget_exceeded");
        assert!(refused.get("plan").is_none());
        assert_eq!(
            refused["agent_turn"]["active_work"]["obligations"]
                .as_array()
                .map(Vec::len),
            Some(1)
        );
        let released = json!({"schema":"dfmcp.query/1","query":{"kind":"cancel_watch","watch":watched["record"]["watch"]}});
        assert_eq!(
            decode(&fortress_query(handle.clone(), None, Some(released)))?["ok"],
            true
        );
        let released = json!({"schema":"dfmcp.query/1","query":{"kind":"release_watch","watch":watched["record"]["watch"]}});
        assert_eq!(
            decode(&fortress_query(handle.clone(), None, Some(released)))?["ok"],
            true
        );
        lock(&*resolve(handle.clone())?)?.source.fence();
        let fenced = decode(&fortress_query(handle, None, Some(input)))?;
        assert_eq!(fenced["error"]["code"], "adapter_unavailable");
        assert_eq!(fenced["agent_turn"]["continuity"]["status"], "stale");
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }
    Ok(())
}
