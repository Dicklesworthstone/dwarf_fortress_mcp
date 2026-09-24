use super::*;
use dfmcp_core::{
    CapabilityGrant, CapabilityScope, FortressId, GameTick, ObservationCursor, RequestId,
    WorkBudget,
};
use dfmcp_world::{EntityKind, EntityRecord, Fact, WorldGraph};

fn world(tick: u64, sequence: u64, healthy: bool) -> WorldSnapshot {
    let fact = Fact::known(
        WorldValue::Bool(healthy),
        GameTick(tick),
        FactSource::DfhackField("unit.sane".to_owned()),
        Digest32::of_bytes(b"fixture-source"),
    );
    let mut graph = WorldGraph::default();
    graph.entities.insert(
        EntityId::new(1),
        EntityRecord {
            id: EntityId::new(1),
            generation: 1,
            revision: sequence + 1,
            kind: EntityKind::Unit,
            label: "Urist".to_owned(),
            fields: BTreeMap::from([("sane".to_owned(), fact)]),
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
            max_bytes: 262144,
            max_output_tokens: 65536,
            ..WorkBudget::default()
        },
        grants: vec![CapabilityGrant {
            capability: Capability::Query,
            scope: CapabilityScope {
                fortress_id: Some(snapshot.fortress_id),
                ..CapabilityScope::default()
            },
            max_risk: RiskTier::ReadOnly,
            expires_at_tick: None,
            remaining_uses: None,
        }],
        cancellation_requested: false,
    }
}
fn condition() -> Value {
    json!({"op":"field","entity_id":"1","generation":1,"field":"sane",
        "comparison":"eq","value":{"type":"bool","value":true}})
}
fn create() -> Value {
    json!({"kind":"watch","key":"healthy-urist","condition":condition(),"deadline_tick":100})
}
fn envelope(query: Value) -> Value {
    json!({"schema":"dfmcp.query/1","query":query})
}
fn json_result(raw: &str) -> Result<Value> {
    serde_json::from_str(raw).map_err(|_| invalid("test response is not JSON"))
}
fn run(store: &Mutex<Store>, snapshot: &WorldSnapshot, query: Value) -> Result<Value> {
    let ctx = context(snapshot, 7);
    json_result(&execute_in(
        store,
        snapshot,
        &ctx,
        &envelope(query),
        |value| Ok(value.to_string()),
    )?)
}
fn handle(value: &Value) -> Result<String> {
    value["record"]["watch"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| invalid("fixture watch missing"))
}
fn poll(watch: &str) -> Value {
    json!({"kind":"poll_watch","watch":watch})
}
fn edit_fact(snapshot: &mut WorldSnapshot, change: impl FnOnce(&mut Fact)) -> Result<()> {
    let fact = snapshot
        .graph
        .entities
        .get_mut(&EntityId::new(1))
        .and_then(|entity| entity.fields.get_mut("sane"))
        .ok_or_else(|| invalid("fixture fact missing"))?;
    change(fact);
    snapshot.refresh_hash();
    Ok(())
}

#[test]
fn repeated_reads_do_not_manufacture_stability_and_terminal_evidence_is_immutable() -> Result<()> {
    let store = Mutex::new(Store::default());
    let initial = world(1, 0, true);
    let first = run(&store, &initial, create())?;
    let watch = handle(&first)?;
    assert_eq!(first["record"]["status"], "candidate");
    assert_eq!(first["record"]["stable_observations"], 1);
    for _ in 0..20 {
        let same = run(&store, &initial, poll(&watch))?;
        assert_eq!(same["record"], first["record"]);
    }
    let complete = run(&store, &world(2, 1, true), poll(&watch))?;
    assert_eq!(complete["record"]["status"], "satisfied");
    assert_eq!(complete["record"]["stable_observations"], 2);
    let later = run(&store, &world(3, 2, false), poll(&watch))?;
    assert_eq!(
        later["record"]["evidence_digest"],
        complete["record"]["evidence_digest"]
    );
    assert_eq!(later["record"]["status"], "satisfied");
    assert_eq!(later["record"]["evaluation_current"], false);
    assert_eq!(later["_condition_watch_work"], json!([]));
    Ok(())
}

#[test]
fn cadence_counts_distinct_ticks_but_does_not_delay_failure_detection() -> Result<()> {
    let store = Mutex::new(Store::default());
    let mut request = create();
    request["poll_interval_ticks"] = json!(5);
    request["failure_condition"] = json!({"op":"paused","value":false});
    let first = run(&store, &world(1, 0, true), request)?;
    let watch = handle(&first)?;
    let second = run(&store, &world(1, 1, true), poll(&watch))?;
    assert_eq!(second["record"]["stable_observations"], 1);
    assert_eq!(second["record"]["sample_count"], 1);
    let early = run(&store, &world(2, 2, true), poll(&watch))?;
    assert_eq!(early["record"]["stable_observations"], 1);
    let mut failure = world(2, 3, true);
    failure.paused = false;
    failure.refresh_hash();
    let failed = run(&store, &failure, poll(&watch))?;
    assert_eq!(failed["record"]["status"], "failed");
    assert_eq!(failed["record"]["evaluation"]["sample_due"], false);
    Ok(())
}

#[test]
fn false_or_unknown_observations_reset_a_candidate_even_between_due_samples() -> Result<()> {
    let store = Mutex::new(Store::default());
    let mut request = create();
    request["poll_interval_ticks"] = json!(3);
    let watch = handle(&run(&store, &world(1, 0, true), request)?)?;
    let reset = run(&store, &world(1, 1, false), poll(&watch))?;
    assert_eq!(reset["record"]["status"], "waiting");
    assert_eq!(reset["record"]["stable_observations"], 0);
    let retry = run(&store, &world(1, 2, true), poll(&watch))?;
    assert_eq!(retry["record"]["stable_observations"], 0);
    let next = run(&store, &world(4, 3, true), poll(&watch))?;
    assert_eq!(next["record"]["stable_observations"], 1);
    let done = run(&store, &world(7, 4, true), poll(&watch))?;
    assert_eq!(done["record"]["status"], "satisfied");
    Ok(())
}

#[test]
fn unknowns_type_mismatches_and_nonobserved_sources_never_pass_under_negation() -> Result<()> {
    for case in 0..10 {
        let store = Mutex::new(Store::default());
        let mut snapshot = world(1, 0, true);
        edit_fact(&mut snapshot, |fact| match case {
            0 => fact.presence = Some(FactPresence::Omitted("not read".to_owned())),
            1 => fact.presence = Some(FactPresence::Absent),
            2 => fact.presence = Some(FactPresence::Known(WorldValue::Bool(false))),
            3 => fact.source = FactSource::AgentAssertion("agent".to_owned()),
            4 => fact.source = FactSource::Derived("uncertified".to_owned()),
            5 => fact.source = FactSource::Replay,
            6 => fact.observed_at = GameTick(0),
            7 => fact.observed_at = GameTick(2),
            8 => fact.value = WorldValue::Text("true".to_owned()),
            _ => fact.presence = Some(FactPresence::Unknown("not established".to_owned())),
        })?;
        let mut request = create();
        request["condition"] = json!({"op":"not","arg":condition()});
        request["stable_observations"] = json!(1);
        let result = run(&store, &snapshot, request)?;
        assert_eq!(result["record"]["status"], "blocked_unknown", "case={case}");
        assert_eq!(result["record"]["stable_observations"], 0);
    }
    Ok(())
}

#[test]
fn unknown_failure_guard_blocks_success_and_a_known_failure_wins_over_success() -> Result<()> {
    let store = Mutex::new(Store::default());
    let mut request = create();
    let mut missing = condition();
    missing["field"] = json!("missing");
    request["failure_condition"] = missing;
    request["stable_observations"] = json!(1);
    let result = run(&store, &world(1, 0, true), request)?;
    assert_eq!(result["record"]["status"], "blocked_unknown");
    let mut request = create();
    request["key"] = json!("failure-wins");
    request["failure_condition"] = condition();
    request["stable_observations"] = json!(1);
    let result = run(&store, &world(1, 0, true), request)?;
    assert_eq!(result["record"]["status"], "failed");
    Ok(())
}

#[test]
fn disappearing_entities_are_unknown_but_generation_reuse_invalidates_even_an_or_branch()
-> Result<()> {
    let store = Mutex::new(Store::default());
    let watch = handle(&run(&store, &world(1, 0, true), create())?)?;
    let mut missing = world(2, 1, true);
    missing.graph.entities.clear();
    missing.refresh_hash();
    let result = run(&store, &missing, poll(&watch))?;
    assert_eq!(result["record"]["status"], "blocked_unknown");
    assert_eq!(result["record"]["stable_observations"], 0);
    let mut request = create();
    request["key"] = json!("generation-fence");
    request["condition"] = json!({"op":"any","args":[{"op":"paused","value":true},condition()]});
    let watch = handle(&run(&store, &world(1, 0, true), request)?)?;
    let mut reused = world(2, 1, true);
    reused
        .graph
        .entities
        .get_mut(&EntityId::new(1))
        .ok_or_else(|| invalid("fixture"))?
        .generation = 2;
    reused.refresh_hash();
    let result = run(&store, &reused, poll(&watch))?;
    assert_eq!(result["record"]["status"], "invalidated");
    Ok(())
}

#[test]
fn skipped_published_observations_reset_streak_instead_of_claiming_continuity() -> Result<()> {
    let store = Mutex::new(Store::default());
    let watch = handle(&run(&store, &world(1, 0, true), create())?)?;
    let gap = run(&store, &world(3, 2, true), poll(&watch))?;
    assert_eq!(gap["record"]["stable_observations"], 1);
    assert_eq!(
        gap["record"]["evaluation"]["skipped_observations_reset_streak"],
        true
    );
    assert_eq!(
        gap["record"]["evaluation"]["continuous_between_observations"],
        false
    );
    assert_eq!(
        run(&store, &world(4, 3, true), poll(&watch))?["record"]["status"],
        "satisfied"
    );
    Ok(())
}

#[test]
fn epoch_fortress_clock_and_same_cursor_forks_invalidate_nonterminal_work() -> Result<()> {
    for case in 0..5 {
        let store = Mutex::new(Store::default());
        let first = world(10, 5, true);
        let watch = handle(&run(&store, &first, create())?)?;
        let mut changed = world(11, 6, true);
        match case {
            0 => changed.cursor.epoch = 1,
            1 => changed.fortress_id = FortressId::new(2),
            2 => changed.tick = GameTick(9),
            3 => changed.cursor.sequence = 4,
            _ => {
                changed = first.clone();
                changed.paused = false;
            }
        }
        changed.refresh_hash();
        assert_eq!(
            run(&store, &changed, poll(&watch))?["record"]["status"],
            "invalidated",
            "case={case}"
        );
    }
    Ok(())
}

#[test]
fn deadline_allows_exact_boundary_proof_but_never_late_success() -> Result<()> {
    for (tick, required, expected) in [(3, 2, "satisfied"), (3, 3, "expired"), (4, 2, "expired")] {
        let store = Mutex::new(Store::default());
        let mut request = create();
        request["deadline_tick"] = json!(3);
        request["stable_observations"] = json!(required);
        let watch = handle(&run(&store, &world(1, 0, true), request)?)?;
        assert_eq!(
            run(&store, &world(tick, 1, true), poll(&watch))?["record"]["status"],
            expected
        );
    }
    let store = Mutex::new(Store::default());
    let mut request = create();
    request["deadline_tick"] = json!(1);
    assert!(run(&store, &world(1, 0, true), request).is_err());
    Ok(())
}

#[test]
fn duplicate_keys_are_idempotent_and_conflicting_definitions_do_not_replace_work() -> Result<()> {
    let store = Mutex::new(Store::default());
    let first = run(&store, &world(1, 0, true), create())?;
    let replay = run(&store, &world(2, 1, true), create())?;
    assert_eq!(handle(&first)?, handle(&replay)?);
    assert_eq!(replay["replayed"], true);
    assert_eq!(replay["record"]["stable_observations"], 1);
    let mut changed = create();
    changed["stable_observations"] = json!(3);
    assert!(
        matches!(run(&store,&world(2,1,true),changed),Err(error) if error.code==ErrorCode::Conflict)
    );
    assert_eq!(lock(&store)?.entries.len(), 1);
    Ok(())
}

#[test]
fn cancellation_is_effect_free_terminal_immutable_and_release_cannot_resurrect_handles()
-> Result<()> {
    let store = Mutex::new(Store::default());
    let snapshot = world(1, 0, true);
    let before = snapshot.clone();
    let watch = handle(&run(&store, &snapshot, create())?)?;
    assert!(
        run(
            &store,
            &snapshot,
            json!({"kind":"release_watch","watch":watch})
        )
        .is_err()
    );
    let cancelled = run(
        &store,
        &snapshot,
        json!({"kind":"cancel_watch","watch":watch}),
    )?;
    assert_eq!(cancelled["record"]["status"], "cancelled");
    let repeated = run(
        &store,
        &snapshot,
        json!({"kind":"cancel_watch","watch":watch}),
    )?;
    assert_eq!(cancelled["record"], repeated["record"]);
    assert_eq!(
        run(
            &store,
            &snapshot,
            json!({"kind":"release_watch","watch":watch})
        )?["released"],
        true
    );
    assert_eq!(
        run(
            &store,
            &snapshot,
            json!({"kind":"release_watch","watch":watch})
        )?["released"],
        false
    );
    let replacement = handle(&run(&store, &snapshot, create())?)?;
    assert_ne!(replacement, watch);
    assert!(run(&store, &snapshot, poll(&watch)).is_err());
    assert_eq!(snapshot, before);
    Ok(())
}

#[test]
fn rejected_publication_leaves_registration_poll_and_release_unchanged() -> Result<()> {
    let store = Mutex::new(Store::default());
    let snapshot = world(1, 0, true);
    let ctx = context(&snapshot, 7);
    let reject = |_: Value| Err::<String, _>(bounded("test renderer refused publication"));
    assert!(execute_in(&store, &snapshot, &ctx, &envelope(create()), reject).is_err());
    assert_eq!(lock(&store)?.entries.len(), 0);
    assert_eq!(lock(&store)?.serial, 0);
    let first = run(&store, &snapshot, create())?;
    let watch = handle(&first)?;
    let next = world(2, 1, true);
    let ctx = context(&next, 7);
    assert!(execute_in(&store, &next, &ctx, &envelope(poll(&watch)), reject).is_err());
    assert_eq!(
        run(&store, &snapshot, poll(&watch))?["record"],
        first["record"]
    );
    let complete = run(&store, &next, poll(&watch))?;
    assert_eq!(complete["record"]["status"], "satisfied");
    assert!(
        execute_in(
            &store,
            &next,
            &ctx,
            &envelope(json!({"kind":"release_watch","watch":watch})),
            reject
        )
        .is_err()
    );
    assert_eq!(
        run(&store, &next, poll(&watch))?["record"],
        complete["record"]
    );
    Ok(())
}

#[test]
fn authority_session_isolation_and_budget_refusal_precede_state_changes() -> Result<()> {
    let store = Mutex::new(Store::default());
    let snapshot = world(1, 0, true);
    for case in 0..4 {
        let mut ctx = context(&snapshot, 7);
        match case {
            0 => ctx.grants.clear(),
            1 => ctx.cancellation_requested = true,
            2 => ctx.grants[0].remaining_uses = Some(0),
            _ => ctx.budget.max_bytes = 1,
        }
        assert!(
            execute_in(&store, &snapshot, &ctx, &envelope(create()), |value| Ok(
                value.to_string()
            ))
            .is_err()
        );
        assert_eq!(lock(&store)?.entries.len(), 0);
    }
    let watch = handle(&run(&store, &snapshot, create())?)?;
    let other = context(&snapshot, 8);
    assert!(
        execute_in(
            &store,
            &snapshot,
            &other,
            &envelope(poll(&watch)),
            |value| Ok(value.to_string())
        )
        .is_err()
    );
    let listed = json_result(&execute_in(
        &store,
        &snapshot,
        &other,
        &envelope(json!({"kind":"watches"})),
        |value| Ok(value.to_string()),
    )?)?;
    assert_eq!(listed["records"], json!([]));
    assert_eq!(listed["_condition_watch_work"], json!([]));
    Ok(())
}

#[test]
fn retention_is_bounded_and_releasing_a_terminal_watch_reclaims_capacity() -> Result<()> {
    let store = Mutex::new(Store::default());
    let snapshot = world(1, 0, true);
    let mut handles = Vec::new();
    for i in 0..MAX_PER_SESSION {
        let mut request = create();
        request["key"] = json!(format!("key-{i}"));
        handles.push(handle(&run(&store, &snapshot, request)?)?);
    }
    assert!(
        matches!(run(&store,&snapshot,create()),Err(error) if error.code==ErrorCode::BudgetExceeded)
    );
    run(
        &store,
        &snapshot,
        json!({"kind":"cancel_watch","watch":handles[0]}),
    )?;
    run(
        &store,
        &snapshot,
        json!({"kind":"release_watch","watch":handles[0]}),
    )?;
    assert_eq!(
        run(&store, &snapshot, create())?["record"]["status"],
        "candidate"
    );
    Ok(())
}

#[test]
fn malformed_unbounded_and_stale_requests_fail_without_retaining_work() -> Result<()> {
    let snapshot = world(1, 0, true);
    let ctx = context(&snapshot, 7);
    let store = Mutex::new(Store::default());
    for (field, value) in [
        ("stable_observations", json!(0)),
        ("stable_observations", json!(65)),
        ("poll_interval_ticks", json!(0)),
        ("poll_interval_ticks", json!(1_000_001)),
        ("deadline_tick", json!(1002)),
        ("key", json!("x".repeat(65))),
        ("condition", json!({"op":"all","args":[]})),
    ] {
        let mut request = create();
        request[field] = value;
        assert!(run(&store, &snapshot, request).is_err());
    }
    let mut request = envelope(create());
    request["expected_anchor"] = json!({"sequence":0});
    assert!(
        execute_in(&store, &snapshot, &ctx, &request, |value| Ok(
            value.to_string()
        ))
        .is_err()
    );
    let mut request = create();
    request["condition"] = json!({"op":"not","arg":condition()});
    for _ in 0..9 {
        request["condition"] = json!({"op":"not","arg":request["condition"].clone()});
    }
    assert!(run(&store, &snapshot, request).is_err());
    assert!(
        run(
            &store,
            &snapshot,
            json!({"kind":"watches","unexpected":true})
        )
        .is_err()
    );
    assert_eq!(lock(&store)?.entries.len(), 0);
    Ok(())
}

#[test]
fn strong_kleene_logic_preserves_decisive_known_branches_without_unknown_negation() -> Result<()> {
    let snapshot = world(1, 0, true);
    let mut unknown = condition();
    unknown["field"] = json!("missing");
    for (op, known, expected) in [
        ("all", false, "waiting"),
        ("all", true, "blocked_unknown"),
        ("any", false, "blocked_unknown"),
        ("any", true, "satisfied"),
    ] {
        let store = Mutex::new(Store::default());
        let mut request = create();
        request["stable_observations"] = json!(1);
        request["condition"] = json!({"op":op,"args":[unknown,{"op":"paused","value":known}]});
        assert_eq!(
            run(&store, &snapshot, request)?["record"]["status"],
            expected
        );
    }
    Ok(())
}
