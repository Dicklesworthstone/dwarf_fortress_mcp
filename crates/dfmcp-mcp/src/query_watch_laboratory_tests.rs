use super::*;
use dfmcp_core::{
    CapabilityGrant, CapabilityScope, FortressId, ObservationCursor, RequestId, WorkBudget,
};
use dfmcp_world::{EntityKind, EntityRecord, WorldGraph};

fn world(tick: u64, sequence: u64) -> WorldSnapshot {
    let mut graph = WorldGraph::default();
    graph.entities.insert(
        EntityId::new(1),
        EntityRecord {
            id: EntityId::new(1),
            generation: 1,
            revision: 1,
            kind: EntityKind::Unit,
            label: "Reference citizen".to_owned(),
            fields: BTreeMap::from([(
                "healthy".to_owned(),
                Fact::known(
                    WorldValue::Bool(true),
                    GameTick(1),
                    FactSource::Derived("dfmcp.lab-scenario/1".to_owned()),
                    Digest32::ZERO,
                ),
            )]),
        },
    );
    WorldSnapshot::new(
        FortressId::new(91),
        GameTick(tick),
        ObservationCursor { epoch: 0, sequence },
        true,
        graph,
    )
}

fn context(snapshot: &WorldSnapshot, owner: u128) -> OperationContext {
    OperationContext {
        session_id: SessionId::new(owner),
        request_id: RequestId::new(1),
        anchor: snapshot.anchor(),
        budget: WorkBudget {
            max_game_ticks: 1000,
            max_entities: 10_000,
            max_bytes: 1_048_576,
            max_output_tokens: 262_144,
            max_wall_millis: 60_000,
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

fn envelope(query: Value) -> Value {
    json!({"schema":"dfmcp.query/1","query":query})
}

fn field() -> Value {
    json!({"op":"field","entity_id":"1","generation":1,"field":"healthy",
        "comparison":"eq","value":{"type":"bool","value":true}})
}

fn create(key: &str) -> Value {
    json!({"kind":"watch","key":key,"condition":field(),"deadline_tick":100})
}

fn decode(text: &str) -> Result<Value> {
    serde_json::from_str(text).map_err(|_| invalid("invalid fixture JSON"))
}

fn run(store: &Mutex<Store>, snapshot: &WorldSnapshot, owner: u128, query: Value) -> Result<Value> {
    decode(&execute_in(
        store,
        snapshot,
        &context(snapshot, owner),
        &envelope(query),
        |value| Ok(value.to_string()),
    )?)
}

fn handle(result: &Value) -> Result<String> {
    result["record"]["watch"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| invalid("missing fixture handle"))
}

fn edit(snapshot: &mut WorldSnapshot, change: impl FnOnce(&mut Fact)) -> Result<()> {
    change(
        snapshot
            .graph
            .entities
            .get_mut(&EntityId::new(1))
            .and_then(|entity| entity.fields.get_mut("healthy"))
            .ok_or_else(|| invalid("missing fixture field"))?,
    );
    snapshot.refresh_hash();
    Ok(())
}

fn inspection(snapshot: &WorldSnapshot, condition: Value, failure: Option<Value>) -> Result<Value> {
    let mut input = json!({"kind":"condition_evaluation","condition":condition});
    if let Some(failure) = failure {
        input["failure_condition"] = failure;
    }
    condition_query(snapshot, &context(snapshot, 93_001), &envelope(input))
}

#[test]
fn laboratory_watch_identity_changes_across_incarnations_but_native_bytes_do_not() -> Result<()> {
    let snapshot = world(1, 0);
    let current = context(&snapshot, 1);
    let definition = Definition {
        key: "restart-owned".to_owned(),
        label: "restart-owned".to_owned(),
        condition: Condition::Paused { value: true },
        failure_condition: None,
        deadline_tick: 100,
        poll_interval_ticks: 1,
        stable_observations: 2,
    };
    let first = (11, 17);
    let laboratory = SourcePolicy::LaboratoryReference;
    let expected = laboratory.identity_with_incarnation(&current, 1, &definition, first)?;
    assert_eq!(
        laboratory.identity_with_incarnation(&current, 1, &definition, first)?,
        expected
    );
    let prior_native = digest(&json!({"domain":"dfmcp-condition-watch-v1",
        "session":current.session_id.to_string(), "serial":1,
        "anchor":anchor(current.anchor), "definition":definition}))?;
    for incarnation in [first, (13, 17), (11, 19)] {
        assert_eq!(
            SourcePolicy::Native.identity_with_incarnation(
                &current,
                1,
                &definition,
                incarnation
            )?,
            prior_native
        );
        if incarnation != first {
            assert_ne!(
                laboratory.identity_with_incarnation(&current, 1, &definition, incarnation)?,
                expected
            );
        }
    }
    assert_eq!(
        SourcePolicy::Native.identity(&current, 1, &definition)?,
        prior_native
    );
    assert_ne!(expected, prior_native);
    Ok(())
}

#[test]
fn laboratory_registration_uses_process_incarnation_and_keeps_key_replay_idempotent() -> Result<()>
{
    let snapshot = world(1, 0);
    let owner = 93_013;
    let current = context(&snapshot, owner);
    let store = Mutex::new(Store::default());
    let created = run(&store, &snapshot, owner, create("restart-owned"))?;
    let definition: Definition = serde_json::from_value(created["record"]["definition"].clone())
        .map_err(|_| invalid("fixture watch definition missing"))?;
    let identity = SourcePolicy::LaboratoryReference.identity_with_incarnation(
        &current,
        1,
        &definition,
        *INCARNATION,
    )?;
    assert_eq!(handle(&created)?, format!("watch:{identity}"));
    let repeated = run(&store, &snapshot, owner, create("restart-owned"))?;
    assert_eq!(repeated["replayed"], true);
    assert_eq!(repeated["record"], created["record"]);
    assert_eq!(lock(&store)?.serial, 1);

    let previous = (INCARNATION.0 ^ 1, INCARNATION.1);
    let old_identity = SourcePolicy::LaboratoryReference.identity_with_incarnation(
        &current,
        1,
        &definition,
        previous,
    )?;
    assert_ne!(old_identity, identity);
    assert!(
        run(
            &store,
            &snapshot,
            owner,
            json!({"kind":"poll_watch", "watch":format!("watch:{old_identity}")}),
        )
        .is_err()
    );
    assert_eq!(
        run(
            &store,
            &snapshot,
            owner,
            json!({"kind":"poll_watch", "watch":handle(&created)?})
        )?["record"],
        created["record"]
    );
    Ok(())
}

#[test]
fn registered_lab_sources_keep_unchanged_facts_eligible_at_later_ticks() -> Result<()> {
    for producer in ["dfmcp.lab-scenario/1", "dfmcp.reference-effects/1"] {
        let mut snapshot = world(20, 19);
        edit(&mut snapshot, |fact| {
            fact.source = FactSource::Derived(producer.to_owned())
        })?;
        let result = inspection(&snapshot, field(), None)?;
        assert_eq!(result["evaluation"]["status"], "condition_met");
        assert_eq!(result["evidence_policy"], "laboratory_reference/1");
        assert_eq!(result["watch_registered"], false);
        assert_eq!(result["evaluation"]["watch_completion_proven"], false);
        let native = super::super::condition_query(
            &snapshot,
            &context(&snapshot, 93_001),
            &envelope(json!({"kind":"condition_evaluation","condition":field()})),
        )?;
        assert_eq!(native["evaluation"]["status"], "blocked_unknown");
        assert!(native.get("evidence_policy").is_none());
    }
    Ok(())
}

#[test]
fn unqualified_facts_remain_unknown_even_under_negation() -> Result<()> {
    for case in 0..10 {
        let mut snapshot = world(5, 4);
        edit(&mut snapshot, |fact| match case {
            0 => fact.source = FactSource::Derived("unregistered-model/1".to_owned()),
            1 => fact.source = FactSource::AgentAssertion("agent".to_owned()),
            2 => fact.source = FactSource::Replay,
            3 => {
                fact.source = FactSource::DfhackField("unit.healthy".to_owned());
                fact.source_digest = Digest32::of_bytes(b"native-source");
                fact.observed_at = GameTick(5);
            }
            4 => fact.source_digest = Digest32::of_bytes(b"wrong-lab-producer-digest"),
            5 => fact.observed_at = GameTick(6),
            6 => fact.presence = Some(FactPresence::Unknown("missing".to_owned())),
            7 => fact.presence = Some(FactPresence::Known(WorldValue::Bool(false))),
            8 => fact.presence = Some(FactPresence::Redacted("private".to_owned())),
            _ => fact.value = WorldValue::Text("true".to_owned()),
        })?;
        let result = inspection(&snapshot, json!({"op":"not","arg":field()}), None)?;
        assert_eq!(
            result["evaluation"]["status"], "blocked_unknown",
            "case {case}"
        );
        assert_eq!(result["evaluation"]["eligible_success_sample"], false);
    }
    Ok(())
}

#[test]
fn laboratory_source_admission_checks_canonical_lookup_identity() -> Result<()> {
    let mut snapshot = world(1, 0);
    let entity = snapshot
        .graph
        .entities
        .remove(&EntityId::new(1))
        .ok_or_else(|| invalid("missing fixture entity"))?;
    snapshot.graph.entities.insert(EntityId::new(2), entity);
    snapshot.refresh_hash();
    assert!(snapshot.hash_is_valid());
    assert!(matches!(inspection(&snapshot, field(), None), Err(error)
        if error.code == ErrorCode::InternalInvariantViolation));
    let store = Mutex::new(Store::default());
    assert!(run(&store, &snapshot, 93_002, create("invalid-map")).is_err());
    assert!(lock(&store)?.entries.is_empty());
    Ok(())
}

#[test]
fn projected_population_uses_the_same_lab_fact_policy_and_interval_truth() -> Result<()> {
    let mut snapshot = world(8, 7);
    let mut second = snapshot.graph.entities[&EntityId::new(1)].clone();
    second.id = EntityId::new(2);
    second
        .fields
        .get_mut("healthy")
        .ok_or_else(|| invalid("fixture field"))?
        .source = FactSource::AgentAssertion("untrusted".to_owned());
    snapshot.graph.entities.insert(second.id, second);
    snapshot.refresh_hash();
    let count = json!({"op":"entity_count","scope":"observed_projection","kind":"unit",
        "predicate":{"op":"field","field":"healthy","comparison":"eq",
            "value":{"type":"bool","value":true}},"comparison":"ge","value":1});
    let result = inspection(&snapshot, count.clone(), None)?;
    assert_eq!(result["evaluation"]["status"], "condition_met");
    assert_eq!(result["evaluation"]["facts"][0]["matched_min"], 1);
    assert_eq!(result["evaluation"]["facts"][0]["matched_max"], 2);
    assert_eq!(
        result["evaluation"]["facts"][0]["complete_world_count_proven"],
        false
    );
    let mut ambiguous = count.clone();
    ambiguous["comparison"] = json!("eq");
    assert_eq!(
        inspection(&snapshot, ambiguous.clone(), None)?["evaluation"]["status"],
        "blocked_unknown"
    );
    assert_eq!(
        inspection(&snapshot, count, Some(ambiguous))?["evaluation"]["status"],
        "blocked_unknown"
    );
    Ok(())
}

#[test]
fn unsupported_native_condition_families_fail_before_registration() -> Result<()> {
    let conditions = [
        json!({"op":"item_quantity","scope":"observed_projection","quantity_unit":"stack_units",
            "predicate":{"op":"always"},"comparison":"ge","value":1}),
        json!({"op":"terrain_count","areas":[{"min":[0,0,0],"max":[0,0,0]}],
            "predicate":{"op":"always"},"comparison":"eq","value":0}),
        json!({"op":"entity_count","scope":"observed_projection","kind":"job",
            "predicate":{"op":"related","entity_id":"1","generation":1,
                "relation":"performs","direction":"outgoing"},"comparison":"eq","value":0}),
        json!({"op":"furniture_set","test":"all_complete","targets":[{
            "building_native_id":1,"building_generation":1,"kind":"bed","max_stage":1}]}),
        json!({"op":"shelter_set","citizens":[{
            "entity_id":dfmcp_adapter::live_spatial::citizens::citizen_entity_id(1).to_string(),
            "generation":1}],
            "shelters":[{"min":[0,0,0],"max":[0,0,0]}]}),
    ];
    let snapshot = world(1, 0);
    for condition in conditions {
        let store = Mutex::new(Store::default());
        let parsed: Condition = serde_json::from_value(condition.clone())
            .map_err(|_| invalid("native condition fixture must be well-formed"))?;
        validate_definition(&Definition {
            key: "native-fixture".to_owned(),
            label: "native-fixture".to_owned(),
            condition: parsed,
            failure_condition: None,
            deadline_tick: 100,
            poll_interval_ticks: 1,
            stable_observations: 1,
        })?;
        let mut request = create("unsupported");
        request["condition"] =
            json!({"op":"any","args":[{"op":"paused","value":true},condition.clone()]});
        assert!(run(&store, &snapshot, 93_003, request).is_err());
        assert!(inspection(&snapshot, field(), Some(condition)).is_err());
        assert!(lock(&store)?.entries.is_empty());
    }
    Ok(())
}

#[test]
fn lab_and_native_watch_policies_cannot_cross_poll_or_reinterpret_keys() -> Result<()> {
    let snapshot = world(1, 0);
    let owner = 93_004;
    let store = Mutex::new(Store::default());
    let created = run(&store, &snapshot, owner, create("owned"))?;
    let watch = handle(&created)?;
    let ctx = context(&snapshot, owner);
    for query in [
        create("owned"),
        json!({"kind":"poll_watch","watch":watch}),
        json!({"kind":"watches"}),
    ] {
        assert!(
            matches!(super::super::execute_in(&store, &snapshot, &ctx, &envelope(query), |value| Ok(value.to_string())),
            Err(error) if error.code == ErrorCode::Conflict)
        );
    }
    assert!(matches!(batch::prepare_in_policy(&store, &snapshot, &ctx,
        &envelope(json!({"kind":"poll_watches"})), SourcePolicy::Native, |value| Ok(value.to_string())),
        Err(error) if error.code == ErrorCode::Conflict));
    assert_eq!(
        run(
            &store,
            &snapshot,
            owner,
            json!({"kind":"poll_watch","watch":watch})
        )?["record"],
        created["record"]
    );

    let native_store = Mutex::new(Store::default());
    let native = decode(&super::super::execute_in(
        &native_store,
        &snapshot,
        &ctx,
        &envelope(create("owned")),
        |value| Ok(value.to_string()),
    )?)?;
    assert_ne!(native["record"]["watch"], created["record"]["watch"]);
    assert_eq!(native["record"]["status"], "blocked_unknown");
    assert!(native.get("_query_watch_work").is_none());
    assert!(native["record"].get("evidence_policy").is_none());
    assert!(
        matches!(run(&native_store, &snapshot, owner, json!({"kind":"watches"})),
        Err(error) if error.code == ErrorCode::Conflict)
    );
    Ok(())
}

#[test]
fn current_snapshot_batch_is_atomic_and_repeated_reads_never_add_samples() -> Result<()> {
    let store = Mutex::new(Store::default());
    let owner = 93_005;
    let snapshot = world(1, 0);
    let first = run(&store, &snapshot, owner, create("first"))?;
    run(&store, &snapshot, owner, create("second"))?;
    let before = retained_work_in(&store, &context(&snapshot, owner))?;
    assert_eq!(before["records"].as_array().map(Vec::len), Some(2));
    assert_eq!(before["active_work"].as_array().map(Vec::len), Some(2));
    let same = run(&store, &snapshot, owner, json!({"kind":"poll_watches"}))?;
    assert_eq!(same["sampled"], 0);
    assert_eq!(same["advanced"], 0);
    assert_eq!(same["_query_watch_work"], before["records"]);
    let next = world(2, 1);
    let result = execute_in(
        &store,
        &next,
        &context(&next, owner),
        &envelope(json!({"kind":"poll_watches"})),
        |_| Err(bounded("fixture complete-packet refusal")),
    );
    assert!(result.is_err());
    assert_eq!(
        retained_work_in(&store, &context(&snapshot, owner))?,
        before
    );
    let accepted = run(&store, &next, owner, json!({"kind":"poll_watches"}))?;
    assert_eq!(accepted["sampled"], 2);
    assert_eq!(accepted["all_satisfied"], true);
    assert_eq!(accepted["atomic_watch_publication"], true);
    assert_eq!(accepted["_condition_watch_work"], json!([]));
    assert_eq!(
        accepted["_query_watch_work"].as_array().map(Vec::len),
        Some(2)
    );
    assert_eq!(accepted["anchor"], anchor(next.anchor()));
    assert_eq!(first["record"]["stable_observations"], 1);
    assert_eq!(snapshot.tick, GameTick(1));
    Ok(())
}

#[test]
fn metadata_and_preflight_do_not_sample_and_owner_isolation_is_preserved() -> Result<()> {
    let store = Mutex::new(Store::default());
    let snapshot = world(1, 0);
    let owner = 93_006;
    let created = run(&store, &snapshot, owner, create("private"))?;
    let watch = handle(&created)?;
    let before = retained_work_in(&store, &context(&snapshot, owner))?;
    let later = world(2, 1);
    inspection(&later, field(), None)?;
    let metadata = retained_work_in(&store, &context(&later, owner))?;
    assert_eq!(metadata["records"][0]["stable_observations"], 1);
    assert_eq!(metadata["records"][0]["needs_poll"], true);
    assert_eq!(
        metadata["records"][0]["evidence_digest"],
        before["records"][0]["evidence_digest"]
    );
    let foreign = retained_work_in(&store, &context(&later, owner + 1))?;
    assert_eq!(foreign, json!({"records":[],"active_work":[]}));
    assert!(
        run(
            &store,
            &later,
            owner + 1,
            json!({"kind":"poll_watch","watch":watch})
        )
        .is_err()
    );
    assert!(
        run(
            &store,
            &later,
            owner + 1,
            json!({"kind":"poll_watches","watches":[watch]})
        )
        .is_err()
    );
    let mut revoked = context(&later, owner);
    revoked.grants.clear();
    assert!(retained_work_in(&store, &revoked).is_err());
    assert_eq!(
        retained_work_in(&store, &context(&snapshot, owner))?,
        before
    );
    Ok(())
}

#[test]
fn skipped_cursors_deadlines_and_restore_keep_existing_temporal_rules() -> Result<()> {
    let owner = 93_007;
    let store = Mutex::new(Store::default());
    let mut request = create("gap");
    request["stable_observations"] = json!(3);
    request["deadline_tick"] = json!(5);
    let watch = handle(&run(&store, &world(1, 0), owner, request)?)?;
    let poll = json!({"kind":"poll_watch","watch":watch});
    let gap = run(&store, &world(3, 2), owner, poll.clone())?;
    assert_eq!(gap["record"]["stable_observations"], 1);
    assert_eq!(
        gap["record"]["evaluation"]["skipped_observations_reset_streak"],
        true
    );
    let expired = run(&store, &world(5, 3), owner, poll)?;
    assert_eq!(expired["record"]["status"], "expired");
    let active = handle(&run(&store, &world(5, 3), owner, create("restore"))?)?;
    let mut restored = world(1, 0);
    restored.cursor.epoch = 1;
    restored.refresh_hash();
    let invalidated = run(
        &store,
        &restored,
        owner,
        json!({"kind":"poll_watch","watch":active}),
    )?;
    assert_eq!(invalidated["record"]["status"], "invalidated");
    assert_eq!(invalidated["record"]["stable_observations"], 0);
    Ok(())
}

#[test]
fn lab_cancel_release_and_failed_registration_preserve_complete_catalogs() -> Result<()> {
    let store = Mutex::new(Store::default());
    let snapshot = world(1, 0);
    let owner = 93_008;
    let ctx = context(&snapshot, owner);
    assert!(
        execute_in(
            &store,
            &snapshot,
            &ctx,
            &envelope(create("invisible")),
            |_| Err(bounded("refused"))
        )
        .is_err()
    );
    assert_eq!(
        retained_work_in(&store, &ctx)?,
        json!({"records":[],"active_work":[]})
    );
    let created = run(&store, &snapshot, owner, create("retained"))?;
    let watch = handle(&created)?;
    assert!(
        run(
            &store,
            &snapshot,
            owner,
            json!({"kind":"release_watch","watch":watch})
        )
        .is_err()
    );
    let cancelled = run(
        &store,
        &snapshot,
        owner,
        json!({"kind":"cancel_watch","watch":watch}),
    )?;
    assert_eq!(cancelled["record"]["status"], "cancelled");
    assert_eq!(cancelled["_condition_watch_work"], json!([]));
    assert_eq!(
        cancelled["_query_watch_work"].as_array().map(Vec::len),
        Some(1)
    );
    let released = run(
        &store,
        &snapshot,
        owner,
        json!({"kind":"release_watch","watch":watch}),
    )?;
    assert_eq!(released["released"], true);
    assert_eq!(released["_query_watch_work"], json!([]));
    assert_eq!(released["_condition_watch_work"], json!([]));
    assert_eq!(
        run(
            &store,
            &snapshot,
            owner,
            json!({"kind":"release_watch","watch":watch})
        )?["released"],
        false
    );
    Ok(())
}

#[test]
fn generation_reuse_and_failure_guards_cannot_hide_behind_true_branches() -> Result<()> {
    let snapshot = world(1, 0);
    let mut missing = field();
    missing["field"] = json!("missing");
    assert_eq!(
        inspection(&snapshot, field(), Some(missing))?["evaluation"]["status"],
        "blocked_unknown"
    );
    assert_eq!(
        inspection(&snapshot, field(), Some(field()))?["evaluation"]["status"],
        "failure_condition_met"
    );
    let mut reused = world(2, 1);
    reused
        .graph
        .entities
        .get_mut(&EntityId::new(1))
        .ok_or_else(|| invalid("fixture entity"))?
        .generation = 2;
    reused.refresh_hash();
    let result = inspection(
        &reused,
        json!({"op":"any","args":[{"op":"paused","value":true},field()]}),
        None,
    )?;
    assert_eq!(result["evaluation"]["status"], "invalidated_reference");
    assert_eq!(result["evaluation"]["eligible_success_sample"], false);
    Ok(())
}

#[test]
fn lab_batch_rejects_refresh_selection_ambiguity_and_narrowed_authority() -> Result<()> {
    let store = Mutex::new(Store::default());
    let snapshot = world(1, 0);
    let owner = 93_009;
    let watch = handle(&run(&store, &snapshot, owner, create("bounded"))?)?;
    let before = retained_work_in(&store, &context(&snapshot, owner))?;
    for query in [
        json!({"kind":"await_watches"}),
        json!({"kind":"poll_watches","watches":[]}),
        json!({"kind":"poll_watches","watches":[watch,watch]}),
    ] {
        assert!(run(&store, &snapshot, owner, query).is_err());
    }
    let mut denied = context(&snapshot, owner);
    denied.grants.clear();
    assert!(
        execute_in(
            &store,
            &snapshot,
            &denied,
            &envelope(json!({"kind":"poll_watches"})),
            |value| Ok(value.to_string())
        )
        .is_err()
    );
    let mut tiny = context(&snapshot, owner);
    tiny.budget.max_output_tokens = 1;
    assert!(
        execute_in(
            &store,
            &snapshot,
            &tiny,
            &envelope(json!({"kind":"poll_watches"})),
            |value| Ok(value.to_string())
        )
        .is_err()
    );
    assert_eq!(
        retained_work_in(&store, &context(&snapshot, owner))?,
        before
    );
    Ok(())
}
