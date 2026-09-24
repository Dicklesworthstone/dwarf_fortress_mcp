//! Actual MCP handlers over a nonjournaled session: the path that previously
//! published before target-tick authorization. All fixtures are process-local.
use super::*;
#[path = "../../dfmcp-adapter/tests/support/production_portfolio_spatial.rs"]
mod fixture;
use dfmcp_core::GameTick;
use std::collections::VecDeque;

static SERIAL: Mutex<()> = Mutex::new(());
struct Script {
    values: VecDeque<LiveSpatialCitizenObservation>,
    calls: Arc<AtomicUsize>,
    fenced: bool,
}
impl Source for Script {
    fn read(&mut self, _: Duration) -> Result<LiveSpatialCitizenObservation> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.values
            .pop_front()
            .ok_or_else(|| error(ErrorCode::AdapterUnavailable, "handler capture exhausted"))
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
    deadline: u64,
}
impl Registered {
    fn raw(&self) -> Option<String> {
        Some(self.id.to_string())
    }
}
impl Drop for Registered {
    fn drop(&mut self) {
        // Explicitly release this test's volatile watch records as well as its
        // session; no persisted evidence is configured by this fixture.
        let _ = fortress_cancel(self.raw(), Some("session".into()), Some(true));
    }
}
fn register(tick: u32, expires: bool) -> Result<Registered> {
    let first = fixture::observation(3, 2, 2, false)?;
    let region = first.spatial().terrain().map.region;
    let mut state = LiveSpatialCitizenState::default();
    state.publish(first)?;
    let anchor = state
        .snapshot()
        .ok_or_else(|| error(ErrorCode::InvalidRequest, "fixture state"))?
        .anchor();
    let limits = CitizenSpatialLimits {
        spatial: SpatialLimits {
            operations: PagedOperationsLimits::default(),
            region,
        },
        citizens: 4096,
    };
    let calls = Arc::new(AtomicUsize::new(0));
    let id = next_id()?;
    let source = Script {
        values: VecDeque::from([fixture::observation(tick, 2, 2, false)?]),
        calls: calls.clone(),
        fenced: false,
    };
    let session = Session {
        id,
        source: Box::new(source),
        state,
        limits,
        journal: None,
        budget: WorkBudget {
            max_entities: limits.entity_limit(),
            max_bytes: 1024 * 1024,
            max_output_tokens: 65536,
            max_wall_millis: 60000,
            ..WorkBudget::default()
        },
        grants: [Capability::Query, Capability::Observe, Capability::Doctor]
            .into_iter()
            .map(|capability| CapabilityGrant {
                capability,
                scope: CapabilityScope {
                    fortress_id: Some(anchor.fortress_id),
                    ..CapabilityScope::default()
                },
                max_risk: RiskTier::ReadOnly,
                expires_at_tick: if expires && capability == Capability::Observe {
                    Some(GameTick(anchor.tick.0 + 1))
                } else {
                    None
                },
                remaining_uses: None,
            })
            .collect(),
        request: 0,
        _watch_journal: None,
        _slot: Slot::reserve()?,
    };
    lock(&SESSIONS)?.insert(id, Arc::new(Mutex::new(session)));
    Ok(Registered {
        id,
        calls,
        deadline: anchor.tick.0 + 100,
    })
}
fn decode(raw: String) -> Result<Value> {
    serde_json::from_str(&raw).map_err(|_| error(ErrorCode::InvalidRequest, "handler JSON"))
}
fn ask(s: &Registered, query: Value) -> Result<Value> {
    decode(fortress_query(
        s.raw(),
        None,
        Some(json!({"schema":"dfmcp.query/1","query":query})),
    ))
}
fn watch(s: &Registered, key: &str, stable: u32) -> Result<Value> {
    let value = ask(
        s,
        json!({"kind":"watch","key":key,"deadline_tick":s.deadline,
        "condition":{"op":"paused","value":true},"stable_observations":stable}),
    )?;
    assert_eq!(value["ok"], true, "{value}");
    Ok(value["record"]["watch"].clone())
}
fn current(s: &Registered) -> Result<StateAnchor> {
    let handle = resolve(s.raw())?;
    let session = lock(&handle)?;
    session.anchor()
}

#[test]
fn public_observe_and_wait_do_not_publish_expired_authority_captures() -> Result<()> {
    let _serial = lock(&SERIAL)?;
    for call in [fortress_observe, fortress_wait] {
        let s = register(5, true)?;
        let old = current(&s)?;
        let value = decode(call(s.raw()))?;
        assert_eq!(value["ok"], false);
        assert_eq!(value["error"]["code"], "capability_denied");
        assert_eq!(value["anchor"], anchor_json(old));
        assert_eq!(current(&s)?, old);
        assert_eq!(value["agent_turn"]["continuity"]["status"], "stale");
        assert_eq!(
            ask(
                &s,
                json!({"kind":"aggregate","group_by":{"kind":"entity_kind"}})
            )?["error"]["code"],
            "adapter_unavailable"
        );
        assert_eq!(s.calls.load(Ordering::SeqCst), 1);
    }
    Ok(())
}

#[test]
fn single_and_batch_await_keep_the_prior_watch_set_when_capture_is_refused() -> Result<()> {
    let _serial = lock(&SERIAL)?;
    for batch in [false, true] {
        let s = register(5, true)?;
        let a = watch(&s, "a", 3)?;
        watch(&s, "b", 3)?;
        let old = current(&s)?;
        let before = ask(&s, json!({"kind":"watches"}))?;
        let query = if batch {
            json!({"kind":"await_watches"})
        } else {
            json!({"kind":"await_watch","watch":a})
        };
        let value = ask(&s, query)?;
        assert_eq!(value["ok"], false);
        assert_eq!(value["error"]["code"], "capability_denied");
        assert_eq!(current(&s)?, old);
        assert_eq!(s.calls.load(Ordering::SeqCst), 1);
        let after = ask(&s, json!({"kind":"watches"}))?;
        assert_eq!(after["ok"], true, "{after}");
        assert_eq!(after["records"], before["records"]);
    }
    Ok(())
}

#[test]
fn accepted_shared_capture_still_advances_all_selected_watches_once() -> Result<()> {
    let _serial = lock(&SERIAL)?;
    let s = register(4, false)?;
    watch(&s, "a", 2)?;
    watch(&s, "b", 2)?;
    let old = current(&s)?;
    let value = ask(&s, json!({"kind":"await_watches"}))?;
    assert_eq!(value["ok"], true, "{value}");
    assert_eq!(value["all_satisfied"], true);
    assert_eq!(value["sampled"], 2);
    assert_eq!(value["native_captures"], 1);
    assert_eq!(current(&s)?.cursor.sequence, old.cursor.sequence + 1);
    assert_eq!(s.calls.load(Ordering::SeqCst), 1);
    for row in value["records"]
        .as_array()
        .ok_or_else(|| error(ErrorCode::InvalidRequest, "watch rows"))?
    {
        assert_eq!(row["last_evaluated_anchor"], value["anchor"]);
        assert_eq!(row["sample_count"], 2);
    }
    Ok(())
}
