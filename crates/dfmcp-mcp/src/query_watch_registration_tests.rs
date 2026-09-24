use super::*;
use dfmcp_core::{
    CapabilityGrant, CapabilityScope, FortressId, GameTick, ObservationCursor, RequestId,
    WorkBudget,
};
use dfmcp_world::{EntityKind, EntityRecord, Fact, WorldGraph};

fn snapshot(sequence: u64, tick: u64) -> WorldSnapshot {
    WorldSnapshot::new(
        FortressId::new(7),
        GameTick(tick),
        ObservationCursor { epoch: 1, sequence },
        true,
        WorldGraph::default(),
    )
}
fn context(s: &WorldSnapshot) -> OperationContext {
    OperationContext {
        session_id: SessionId::new(923_001),
        request_id: RequestId::new(1),
        anchor: s.anchor(),
        budget: WorkBudget {
            max_entities: 20000,
            max_game_ticks: 1000,
            max_wall_millis: 60000,
            max_bytes: 262144,
            max_output_tokens: 65536,
            ..WorkBudget::default()
        },
        grants: vec![CapabilityGrant {
            capability: Capability::Query,
            scope: CapabilityScope::default(),
            max_risk: RiskTier::ReadOnly,
            expires_at_tick: None,
            remaining_uses: None,
        }],
        cancellation_requested: false,
    }
}
fn watch(key: &str) -> Value {
    json!({"key":key,"condition":{"op":"paused","value":true},"deadline_tick":100})
}
fn input(watches: Vec<Value>) -> Value {
    json!({"schema":"dfmcp.query/1","query":{"kind":"register_watches","watches":watches}})
}
fn encode(value: Value) -> Result<String> {
    Ok(value.to_string())
}
fn parse(text: String) -> Result<Value> {
    serde_json::from_str(&text).map_err(|_| invalid("test JSON"))
}
fn run(
    store: &Mutex<Store>,
    s: &WorldSnapshot,
    c: &OperationContext,
    watches: Vec<Value>,
) -> Result<Value> {
    parse(execute_in(store, s, c, &input(watches), encode)?)
}
fn fingerprint(store: &Mutex<Store>, c: &OperationContext) -> Result<(u64, Digest32)> {
    let store = lock(store)?;
    Ok((store.serial, registry(&store, c.session_id)?))
}
fn row<'a>(result: &'a Value, key: &str) -> Result<&'a Value> {
    result["records"]
        .as_array()
        .and_then(|r| r.iter().find(|r| r["key"] == key))
        .ok_or_else(|| invalid("test row"))
}

#[test]
fn registration_order_and_explicit_defaults_produce_identical_watch_identities() -> Result<()> {
    let s = snapshot(0, 1);
    let c = context(&s);
    let first = Mutex::new(Store::default());
    let second = Mutex::new(Store::default());
    let mut explicit = watch("a");
    explicit["label"] = json!("a");
    explicit["poll_interval_ticks"] = json!(1);
    explicit["stable_observations"] = json!(2);
    explicit["failure_condition"] = Value::Null;
    let a = run(&first, &s, &c, vec![watch("z"), watch("a")])?;
    let b = run(&second, &s, &c, vec![explicit, watch("z")])?;
    assert_eq!(a, b);
    assert_eq!(a["created"], 2);
    assert_eq!(a["native_captures"], 0);
    assert_eq!(a["records"][0]["key"], "a");
    assert_eq!(a["records"][1]["key"], "z");
    for key in ["a", "z"] {
        assert_eq!(row(&a, key)?["sample_count"], 1);
    }
    assert_eq!(
        a["next_step"]["arguments"]["query"]["query"]["kind"],
        "await_watches"
    );
    Ok(())
}

#[test]
fn exact_retries_and_mixed_sets_do_not_resample_existing_watches() -> Result<()> {
    let store = Mutex::new(Store::default());
    let s = snapshot(0, 1);
    let c = context(&s);
    let initial = run(&store, &s, &c, vec![watch("a"), watch("b")])?;
    let prior = fingerprint(&store, &c)?;
    let later = snapshot(1, 2);
    let mut next = c.clone();
    next.anchor = later.anchor();
    let replay = run(&store, &later, &next, vec![watch("b"), watch("a")])?;
    assert_eq!(replay["created"], 0);
    assert_eq!(replay["replayed"], 2);
    assert_eq!(fingerprint(&store, &c)?, prior);
    assert_eq!(
        replay["configuration_digest"],
        initial["configuration_digest"]
    );
    let mixed = run(&store, &later, &next, vec![watch("c"), watch("a")])?;
    assert_eq!(mixed["created"], 1);
    assert_eq!(mixed["replayed"], 1);
    assert_eq!(
        row(&mixed, "a")?["evidence_digest"],
        row(&initial, "a")?["evidence_digest"]
    );
    assert_eq!(
        row(&mixed, "c")?["last_evaluated_anchor"],
        anchor(later.anchor())
    );
    assert_eq!(lock(&store)?.entries.len(), 3);
    Ok(())
}

#[test]
fn any_invalid_or_conflicting_definition_refuses_the_entire_set_before_publication() -> Result<()> {
    let store = Mutex::new(Store::default());
    let s = snapshot(0, 1);
    let c = context(&s);
    run(&store, &s, &c, vec![watch("z-existing")])?;
    let prior = fingerprint(&store, &c)?;
    let mut changed = watch("z-existing");
    changed["deadline_tick"] = json!(99);
    let mut expired = watch("z-new");
    expired["deadline_tick"] = json!(1);
    let mut malformed = watch("z-new");
    malformed["background"] = json!(true);
    let mut nested = watch("z-new");
    nested["kind"] = json!("watch");
    for defs in [
        vec![watch("a-new"), changed],
        vec![watch("a-new"), expired],
        vec![watch("a-new"), malformed],
        vec![watch("a-new"), nested],
        vec![watch("a-new"), watch("a-new")],
        Vec::new(),
        (0..9).map(|n| watch(&n.to_string())).collect(),
    ] {
        let mut published = false;
        assert!(
            execute_in(&store, &s, &c, &input(defs), |v| {
                published = true;
                encode(v)
            })
            .is_err()
        );
        assert!(!published);
        assert_eq!(fingerprint(&store, &c)?, prior);
    }
    Ok(())
}

#[test]
fn full_retention_allows_exact_replay_but_no_partial_addition() -> Result<()> {
    let store = Mutex::new(Store::default());
    let s = snapshot(0, 1);
    let c = context(&s);
    let defs: Vec<_> = (0..8).map(|n| watch(&format!("watch-{n}"))).collect();
    run(&store, &s, &c, defs.clone())?;
    let prior = fingerprint(&store, &c)?;
    assert_eq!(run(&store, &s, &c, defs)?["created"], 0);
    assert!(
        matches!(run(&store,&s,&c,vec![watch("watch-0"),watch("new")]),Err(e)if e.code==ErrorCode::BudgetExceeded)
    );
    assert_eq!(fingerprint(&store, &c)?, prior);
    Ok(())
}

#[test]
fn failed_renderer_and_oversized_final_packets_do_not_consume_serials_or_keys() -> Result<()> {
    let store = Mutex::new(Store::default());
    let s = snapshot(0, 1);
    let c = context(&s);
    let request = input(vec![watch("a"), watch("b")]);
    let before = fingerprint(&store, &c)?;
    assert!(
        execute_in(&store, &s, &c, &request, |_| Err(bounded(
            "injected publisher refusal"
        )))
        .is_err()
    );
    assert!(execute_in(&store, &s, &c, &request, |_| Ok("x".repeat(262145))).is_err());
    let mut tiny = c.clone();
    tiny.budget.max_bytes = 64;
    assert!(execute_in(&store, &s, &tiny, &request, encode).is_err());
    assert_eq!(fingerprint(&store, &c)?, before);
    assert_eq!(
        run(&store, &s, &c, vec![watch("a"), watch("b")])?["created"],
        2
    );
    Ok(())
}

#[test]
fn later_quantity_overflow_cannot_publish_an_earlier_valid_registration() -> Result<()> {
    let store = Mutex::new(Store::default());
    let mut s = snapshot(0, 1);
    for (id, units) in [(1, u64::MAX), (2, 1)] {
        let id = EntityId::new(id);
        s.graph.entities.insert(
            id,
            EntityRecord {
                id,
                generation: 1,
                revision: 1,
                kind: EntityKind::Item,
                label: "stack".into(),
                fields: BTreeMap::from([(
                    "stack_size".into(),
                    Fact::known(
                        WorldValue::U64(units),
                        s.tick,
                        FactSource::DfhackField("item.stack_size".into()),
                        Digest32::of_bytes(b"test"),
                    ),
                )]),
            },
        );
    }
    s.refresh_hash();
    let c = context(&s);
    let prior = fingerprint(&store, &c)?;
    let mut quantity = watch("z-overflow");
    quantity["condition"] = json!({"op":"item_quantity",
        "scope":"observed_projection","quantity_unit":"stack_units","predicate":{"op":"always"},"comparison":"ge","value":1});
    assert!(
        execute_in(
            &store,
            &s,
            &c,
            &input(vec![watch("a-valid"), quantity]),
            encode
        )
        .is_err()
    );
    assert_eq!(fingerprint(&store, &c)?, prior);
    Ok(())
}

#[test]
fn population_scans_share_one_work_allowance_across_the_complete_installation() -> Result<()> {
    let mut s = snapshot(0, 1);
    for n in 1..=10000 {
        let id = EntityId::new(n);
        s.graph.entities.insert(
            id,
            EntityRecord {
                id,
                generation: 1,
                revision: 1,
                kind: EntityKind::Unit,
                label: "citizen".into(),
                fields: BTreeMap::new(),
            },
        );
    }
    s.refresh_hash();
    let c = context(&s);
    let mut a = watch("a");
    a["condition"] = json!({"op":"entity_count","scope":"observed_projection","kind":"unit",
        "predicate":{"op":"all","args":(0..60).map(|_|json!({"op":"always"})).collect::<Vec<_>>()},"comparison":"ge","value":1});
    let mut b = a.clone();
    b["key"] = json!("b");
    assert!(run(&Mutex::new(Store::default()), &s, &c, vec![a.clone()]).is_ok());
    let store = Mutex::new(Store::default());
    assert!(matches!(run(&store,&s,&c,vec![a,b]),Err(e)if e.code==ErrorCode::BudgetExceeded));
    assert_eq!(lock(&store)?.serial, 0);
    assert!(lock(&store)?.entries.is_empty());
    Ok(())
}

#[test]
fn authority_anchor_and_expired_replay_preserve_single_watch_rules() -> Result<()> {
    let store = Mutex::new(Store::default());
    let s = snapshot(0, 1);
    let c = context(&s);
    for case in 0..4 {
        let mut denied = c.clone();
        let mut request = input(vec![watch("a")]);
        match case {
            0 => denied.grants.clear(),
            1 => denied.cancellation_requested = true,
            2 => denied.grants[0].expires_at_tick = Some(GameTick(0)),
            _ => request["expected_anchor"] = json!({}),
        }
        assert!(execute_in(&store, &s, &denied, &request, encode).is_err());
        assert!(lock(&store)?.entries.is_empty());
    }
    let mut terminal = watch("done");
    terminal["stable_observations"] = json!(1);
    let first = run(&store, &s, &c, vec![terminal.clone()])?;
    let late = snapshot(1, 200);
    let mut now = c.clone();
    now.anchor = late.anchor();
    let replay = run(&store, &late, &now, vec![terminal])?;
    assert_eq!(replay["created"], 0);
    assert!(replay["next_step"].is_null());
    assert_eq!(
        row(&first, "done")?["evidence_digest"],
        row(&replay, "done")?["evidence_digest"]
    );
    Ok(())
}
