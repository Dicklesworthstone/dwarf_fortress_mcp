use super::*;
use crate::live_spatial_citizens_server as server;
use dfmcp_adapter::live_spatial::citizens::LiveSpatialCitizenObservation;
use dfmcp_core::{CapabilityGrant, CapabilityScope, SessionId, WorkBudget};
use std::collections::VecDeque;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;

#[path = "../../dfmcp-adapter/tests/support/shelter_fixture.rs"]
#[allow(dead_code)]
mod fixture;

fn request() -> Value {
    json!({"schema":"dfmcp.query/1","query":{"kind":"shelter_status",
        "citizens":[{"entity_id":"3298534883338","generation":1},{"entity_id":"3298534883339","generation":1}],
        "shelters":[{"min":[0,0,5],"max":[1,1,5]}],"limit":1}})
}
fn state() -> Result<LiveSpatialCitizenState> {
    fixture::state(&[(10, [0, 0, 5], 0x11f), (11, [2, 0, 5], 0x11f)])
}

#[test]
fn registered_dispatch_keeps_whole_selection_truth_on_every_page() -> Result<()> {
    let state = state()?;
    let c = fixture::context(&state)?;
    let example: Value = serde_json::from_str(include_str!(
        "../../../schemas/examples/shelter_status_v1.json"
    ))
    .map_err(|_| invalid("checked-in shelter example"))?;
    assert_eq!(execute(&state, &c, &example)?["arrival_truth"], "false");
    assert!(super::super::handles(&request()));
    let first = super::super::execute(&state, &c, &request())?;
    assert_eq!(first["arrival_truth"], "false");
    assert_eq!(first["counts"]["inside"], 1);
    assert_eq!(first["counts"]["outside"], 1);
    assert_eq!(first["total_rows"], 2);
    assert_eq!(first["returned"], 1);
    assert_eq!(first["rows"][0]["status"], "inside");
    let mut next = request();
    next["query"]["continuation"] = first["continuation"].clone();
    let second = super::super::execute(&state, &c, &next)?;
    assert_eq!(second["rows"][0]["status"], "outside");
    assert_eq!(second["arrival_truth"], "false");
    assert_eq!(first["selection_digest"], second["selection_digest"]);
    assert_eq!(second["truncated"], false);
    assert!(second["continuation"].is_null());
    assert_eq!(first["native_captures"], 0);
    assert_eq!(first["mutation_dispatched"], false);
    Ok(())
}

#[test]
fn missing_and_recycled_targets_are_returned_not_dropped() -> Result<()> {
    let state = state()?;
    let c = fixture::context(&state)?;
    let mut q = request();
    q["query"]["limit"] = json!(128);
    q["query"]["citizens"][0]["generation"] = json!(2);
    q["query"]["citizens"][1]["entity_id"] = json!("3298534883340");
    let out = execute(&state, &c, &q)?;
    assert_eq!(out["arrival_truth"], "unknown");
    assert_eq!(out["counts"]["unresolved"], 2);
    assert_eq!(out["rows"][0]["status"], "incarnation_changed");
    assert_eq!(out["rows"][1]["status"], "not_observed");
    assert!(out["rows"][0]["position"].is_null());
    Ok(())
}

#[test]
fn continuations_bind_session_goal_generation_source_and_work_allowance() -> Result<()> {
    let state = state()?;
    let c = fixture::context(&state)?;
    let first = execute(&state, &c, &request())?;
    let mut next = request();
    next["query"]["continuation"] = first["continuation"].clone();
    for case in 0..4 {
        let mut q = next.clone();
        let mut changed = c.clone();
        match case {
            0 => changed.session_id = SessionId::new(991),
            1 => q["query"]["citizens"][0]["generation"] = json!(2),
            2 => q["query"]["shelters"][0]["max"] = json!([2, 1, 5]),
            _ => q["query"]["max_work"] = json!(999_999),
        }
        assert!(matches!(execute(&state,&changed,&q),Err(e) if e.code==ErrorCode::StaleAnchor));
    }
    let mut moved = state.clone();
    moved.publish(fixture::observation(
        &[(10, [0, 0, 5], 0x11f), (11, [0, 0, 5], 0x11f)],
        1,
        7,
    )?)?;
    assert!(
        matches!(execute(&moved,&fixture::context(&moved)?,&next),Err(e) if e.code==ErrorCode::StaleAnchor)
    );
    let mut reversed = request();
    reversed["query"]["citizens"]
        .as_array_mut()
        .ok_or_else(|| invalid("fixture citizens"))?
        .reverse();
    assert_eq!(first, execute(&state, &c, &reversed)?);
    Ok(())
}

#[test]
fn closed_decoding_and_bounds_refuse_forged_position_or_control_inputs() -> Result<()> {
    let state = state()?;
    let c = fixture::context(&state)?;
    for key in [
        "position",
        "authority",
        "paused_target",
        "source_digest",
        "assume_safe",
    ] {
        let mut q = request();
        q["query"][key] = json!(true);
        assert!(execute(&state, &c, &q).is_err());
    }
    for id in ["0", "01", "+1", "1\n", "18446744073709551616", "1001"] {
        let mut q = request();
        q["query"]["citizens"][0]["entity_id"] = json!(id);
        assert!(execute(&state, &c, &q).is_err());
    }
    for case in 0..7 {
        let mut q = request();
        match case {
            0 => q["query"]["citizens"] = json!([]),
            1 => q["query"]["citizens"][0]["generation"] = json!(0),
            2 => q["query"]["citizens"][1] = q["query"]["citizens"][0].clone(),
            3 => q["query"]["shelters"][0]["max"] = json!([-1, 0, 5]),
            4 => q["query"]["limit"] = json!(0),
            5 => q["query"]["max_work"] = json!(0),
            _ => q["expected_anchor"] = json!({}),
        }
        assert!(execute(&state, &c, &q).is_err());
    }
    let mut denied = c.clone();
    denied.grants.clear();
    assert!(
        matches!(execute(&state,&denied,&request()),Err(e) if e.code==ErrorCode::CapabilityDenied)
    );
    let mut tiny = c;
    tiny.budget.max_output_tokens = 1;
    assert!(matches!(execute(&state,&tiny,&request()),Err(e) if e.code==ErrorCode::BudgetExceeded));
    Ok(())
}

struct Script {
    values: VecDeque<LiveSpatialCitizenObservation>,
    calls: Arc<AtomicUsize>,
    fenced: bool,
}
impl server::Source for Script {
    fn read(&mut self, _: Duration) -> Result<LiveSpatialCitizenObservation> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.values
            .pop_front()
            .ok_or_else(|| invalid("shelter fixture exhausted"))
    }
    fn poisoned(&self) -> bool {
        self.fenced
    }
    fn fence(&mut self) {
        self.fenced = true;
    }
    fn pages(&self) -> u32 {
        1
    }
}
struct Registered {
    id: SessionId,
    calls: Arc<AtomicUsize>,
}
impl Drop for Registered {
    fn drop(&mut self) {
        if let Ok(mut sessions) = server::SESSIONS.lock() {
            sessions.remove(&self.id);
        }
    }
}
fn register(tokens: u32) -> Result<Registered> {
    let state = state()?;
    let c = fixture::context(&state)?;
    let region = state
        .observation_full()
        .ok_or_else(|| invalid("fixture observation"))?
        .spatial()
        .terrain()
        .map
        .region;
    let limits = server::CitizenSpatialLimits {
        spatial: server::SpatialLimits {
            operations: server::PagedOperationsLimits::default(),
            region,
        },
        citizens: 4096,
    };
    let calls = Arc::new(AtomicUsize::new(0));
    let id = server::next_id()?;
    let session = server::Session {
        id,
        state,
        limits,
        journal: None,
        request: 0,
        _watch_journal: None,
        _slot: server::Slot::reserve()?,
        source: Box::new(Script {
            calls: calls.clone(),
            fenced: false,
            values: VecDeque::from([fixture::observation(
                &[(10, [0, 0, 5], 0x11f), (11, [0, 0, 5], 0x11f)],
                1,
                7,
            )?]),
        }),
        budget: WorkBudget {
            max_entities: limits.entity_limit(),
            max_bytes: 1024 * 1024,
            max_output_tokens: tokens,
            max_wall_millis: 60_000,
            ..WorkBudget::default()
        },
        grants: [Capability::Query, Capability::Observe, Capability::Doctor]
            .into_iter()
            .map(|capability| CapabilityGrant {
                capability,
                max_risk: RiskTier::ReadOnly,
                scope: CapabilityScope {
                    fortress_id: Some(c.anchor.fortress_id),
                    ..CapabilityScope::default()
                },
                expires_at_tick: None,
                remaining_uses: None,
            })
            .collect(),
    };
    server::lock(&server::SESSIONS)?.insert(id, Arc::new(Mutex::new(session)));
    Ok(Registered { id, calls })
}
fn decoded(raw: &str) -> Result<Value> {
    serde_json::from_str(raw).map_err(|_| invalid("native handler response"))
}
fn ask(s: &Registered, q: Value) -> Result<Value> {
    decoded(&server::fortress_query(
        Some(s.id.to_string()),
        None,
        Some(q),
    ))
}

#[test]
fn actual_native_handler_discovers_and_pages_shelter_status_without_extra_reads() -> Result<()> {
    let _serial = crate::test_serial();
    let s = register(8192)?;
    let out = ask(&s, request())?;
    assert_eq!(out["ok"], true, "{out}");
    assert_eq!(out["arrival_truth"], "false");
    assert_eq!(out["total_rows"], 2);
    assert!(out["agent_turn"].is_object());
    assert_eq!(s.calls.load(Ordering::SeqCst), 0);
    let mut next = request();
    next["query"]["continuation"] = out["continuation"].clone();
    let page = ask(&s, next)?;
    assert_eq!(page["ok"], true, "{page}");
    assert_eq!(page["rows"][0]["status"], "outside");
    assert_eq!(s.calls.load(Ordering::SeqCst), 0);
    Ok(())
}

#[test]
fn actual_observe_refresh_recomputes_arrival_and_fences_old_continuations() -> Result<()> {
    let _serial = crate::test_serial();
    let s = register(65_536)?;
    let first = ask(&s, request())?;
    assert_eq!(first["ok"], true, "{first}");
    let refreshed = decoded(&server::fortress_observe(Some(s.id.to_string())))?;
    assert_eq!(refreshed["ok"], true, "{refreshed}");
    let mut old = request();
    old["query"]["continuation"] = first["continuation"].clone();
    let refused = ask(&s, old)?;
    assert_eq!(refused["ok"], false);
    let current = ask(&s, request())?;
    assert_eq!(current["ok"], true, "{current}");
    assert_eq!(current["arrival_truth"], "true");
    assert_eq!(current["counts"]["inside"], 2);
    assert_eq!(current["selection_digest"], first["selection_digest"]);
    assert_eq!(s.calls.load(Ordering::SeqCst), 1);
    Ok(())
}

#[test]
fn actual_schema_and_fenced_source_preserve_the_existing_native_boundary() -> Result<()> {
    let _serial = crate::test_serial();
    let s = register(65_536)?;
    let schema = crate::live_spatial_citizens_server::schema_paging::test_decode_schema(
        &server::fortress_query(Some(s.id.to_string()), Some("schema".to_owned()), None),
    )?;
    assert_eq!(schema["ok"], true, "{schema}");
    let variants = schema["query_schema"]["$defs"]["query"]["oneOf"]
        .as_array()
        .ok_or_else(|| invalid("schema variants"))?;
    assert!(
        variants
            .iter()
            .any(|v| v["properties"]["kind"]["const"] == "shelter_status")
    );
    {
        let handle = server::resolve(Some(s.id.to_string()))?;
        server::lock(&handle)?.source.fence();
    }
    let out = ask(&s, request())?;
    assert_eq!(out["ok"], false, "{out}");
    assert_eq!(s.calls.load(Ordering::SeqCst), 0);
    Ok(())
}
