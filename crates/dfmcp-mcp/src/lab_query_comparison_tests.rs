use super::*;
use std::collections::{BTreeMap, BTreeSet};

use dfmcp_core::{
    CapabilityGrant, CapabilityScope, EntityId, EventId, FortressId, GameTick, ObservationCursor,
    RequestId, SessionId, WorkBudget,
};
use dfmcp_world::{
    EntityKind, EntityRecord, Fact, FactPresence, FactSource, Value as WorldValue, WorldEvent,
    WorldEventKind, WorldGraph,
};

fn world(tick: u64, sequence: u64, values: &[(u64, u32, i64)]) -> WorldSnapshot {
    let mut graph = WorldGraph::default();
    for &(id, generation, value) in values {
        let fact = Fact::known(
            WorldValue::I64(value),
            GameTick(tick),
            FactSource::DfhackField("unit.value".into()),
            Digest32::of_bytes(&tick.to_be_bytes()),
        );
        graph.entities.insert(
            EntityId::new(id),
            EntityRecord {
                id: EntityId::new(id),
                generation,
                revision: sequence + 1,
                kind: EntityKind::Unit,
                label: format!("Urist {id}"),
                fields: BTreeMap::from([("value".into(), fact)]),
            },
        );
    }
    WorldSnapshot::new(
        FortressId::new(41),
        GameTick(tick),
        ObservationCursor { epoch: 3, sequence },
        true,
        graph,
    )
}

fn context(world: &WorldSnapshot) -> OperationContext {
    OperationContext {
        session_id: SessionId::new(481),
        request_id: RequestId::new(1),
        anchor: world.anchor(),
        budget: WorkBudget {
            max_entities: 2000,
            max_bytes: 4 * 1024 * 1024,
            max_output_tokens: 65_536,
            max_wall_millis: 60_000,
            ..WorkBudget::default()
        },
        grants: vec![CapabilityGrant {
            capability: Capability::Query,
            scope: CapabilityScope {
                fortress_id: Some(world.fortress_id),
                ..CapabilityScope::default()
            },
            max_risk: RiskTier::ReadOnly,
            expires_at_tick: None,
            remaining_uses: None,
        }],
        cancellation_requested: false,
    }
}

fn selection() -> Value {
    json!({"kind":"entities","kinds":["unit"],"fields":["value"]})
}

fn input(before: &WorldSnapshot, after: &WorldSnapshot) -> Value {
    json!({"mode":"query_changes", "before":before.state_hash.to_string(),
        "after":after.state_hash.to_string(), "select":selection()})
}

fn request(value: &Value) -> Result<Request> {
    parse(&value.to_string())?.ok_or_else(|| invalid("test query_changes request absent"))
}

fn compare(before: &WorldSnapshot, after: &WorldSnapshot) -> Result<Value> {
    execute(
        before,
        after,
        &context(after),
        &request(&input(before, after))?,
    )
}

#[test]
fn parser_owns_one_closed_mode_and_rejects_ambiguous_or_paged_selections() -> Result<()> {
    assert!(parse("summary")?.is_none());
    assert!(parse(r#"{"mode":"entities"}"#)?.is_none());
    let before = world(10, 1, &[]);
    let after = world(20, 2, &[]);
    let valid = input(&before, &after);
    assert!(parse(&valid.to_string())?.is_some());
    for case in 0..11 {
        let mut bad = valid.clone();
        match case {
            0 => bad["extra"] = json!(true),
            1 => bad["before"] = json!("a".repeat(63)),
            2 => bad["before"] = json!("A".repeat(64)),
            3 => bad["after"] = json!("g".repeat(64)),
            4 => bad["limit"] = json!(0),
            5 => bad["limit"] = json!(129),
            6 => bad["select"]["kind"] = json!("search"),
            7 => bad["select"]["limit"] = json!(1),
            8 => bad["select"]["continuation"] = json!("not-an-endpoint"),
            9 => bad["continuation"] = json!("x".repeat(129)),
            _ => bad["continuation"] = json!(""),
        }
        assert!(
            matches!(parse(&bad.to_string()), Err(e) if e.code == ErrorCode::InvalidRequest),
            "case={case}"
        );
    }
    let text = valid.to_string();
    for key in ["mode", "before", "select"] {
        let duplicate = format!("{{{key:?}:{},{}", valid[key], &text[1..]);
        assert!(parse(&duplicate).is_err(), "duplicate={key}");
    }
    let duplicate_fields = text.replace(
        "\"fields\":[\"value\"]",
        "\"fields\":[\"value\"],\"fields\":[\"other\"]",
    );
    assert!(parse(&duplicate_fields).is_err());
    assert!(
        matches!(parse(&" ".repeat(MAX_REQUEST_BYTES + 1)), Err(e) if e.code == ErrorCode::BudgetExceeded)
    );
    Ok(())
}

#[test]
fn endpoint_changes_keep_generations_presence_and_exact_source_context() -> Result<()> {
    let before = world(10, 1, &[(1, 1, 10), (2, 1, 20), (4, 1, 40)]);
    let after = world(20, 2, &[(1, 1, 11), (3, 1, 30), (4, 2, 40)]);
    let result = compare(&before, &after)?;
    assert_eq!(result["mode"], "query_changes");
    assert_eq!(result["kind"], "historical_changes");
    assert_eq!(result["basis"], anchor_json(before.anchor()));
    assert_eq!(result["anchor"], anchor_json(after.anchor()));
    assert_eq!(result["current_anchor"], anchor_json(after.anchor()));
    assert_eq!(result["historical"], false);
    assert_eq!(result["change_count"], 5);
    assert_eq!(
        result["change_counts"],
        json!({"changed_in_result":1,"entered_result":2,"left_result":2})
    );
    assert_eq!(result["changes"][3]["entity_id"], "4");
    assert_eq!(result["changes"][3]["generation"], 1);
    assert_eq!(result["changes"][3]["kind"], "left_result");
    assert_eq!(result["changes"][4]["generation"], 2);
    assert_eq!(result["changes"][4]["kind"], "entered_result");
    assert_eq!(
        result["source_work"],
        json!({"records":6,
        "canonical_bytes":before.canonical_bytes().len() + after.canonical_bytes().len()})
    );
    assert_eq!(result["baseline_created"], false);
    assert_eq!(result["comparison_persisted"], false);
    assert_eq!(result["mutation_authority"], false);
    assert_eq!(result["eligible_as_mutation_evidence"], false);
    assert_eq!(result["observation_coverage"]["status"], "partial");
    assert_eq!(
        result["observation_coverage"]["absence_proof_scope"],
        json!([])
    );
    assert_eq!(
        result["coverage"]["intermediate_observations_evaluated"],
        false
    );
    Ok(())
}

#[test]
fn filtered_departure_and_unknown_fact_never_claim_entity_deletion() -> Result<()> {
    let before = world(10, 1, &[(1, 1, 10), (2, 1, 0)]);
    let mut after = world(20, 2, &[(1, 1, 10), (2, 1, 10)]);
    after
        .graph
        .entities
        .get_mut(&EntityId::new(1))
        .and_then(|entity| entity.fields.get_mut("value"))
        .ok_or_else(|| invalid("test value absent"))?
        .presence = Some(FactPresence::Unknown("not observed".into()));
    after.refresh_hash();
    let mut ask = input(&before, &after);
    ask["select"]["where"] = json!({"op":"compare","field":"value","comparison":"gt",
        "value":{"type":"i64","value":5}});
    let filtered = execute(&before, &after, &context(&after), &request(&ask)?)?;
    assert_eq!(filtered["matched_before"], 1);
    assert_eq!(filtered["matched_after"], 1);
    assert_eq!(filtered["changes"][0]["kind"], "left_result");
    assert_eq!(filtered["changes"][1]["kind"], "entered_result");
    assert_eq!(filtered["coverage"]["absence_proven"], false);
    let unfiltered = compare(&before, &after)?;
    assert_eq!(unfiltered["changes"][0]["kind"], "changed_in_result");
    assert_eq!(
        unfiltered["changes"][0]["after"]["fields"]["value"]["value"],
        Value::Null
    );
    Ok(())
}

#[test]
fn same_endpoints_and_provenance_refreshes_keep_honest_temporal_coverage() -> Result<()> {
    let before = world(10, 1, &[(1, 1, 7), (2, 1, 8)]);
    let after = world(20, 2, &[(1, 1, 7), (2, 1, 8)]);
    let refresh = compare(&before, &after)?;
    assert_eq!(refresh["change_count"], 0);
    assert_eq!(refresh["provenance_only_refreshes"], 2);
    assert_eq!(refresh["anchor_advanced"], true);
    let same = compare(&before, &before)?;
    assert_eq!(same["changes"], json!([]));
    assert_eq!(same["provenance_only_refreshes"], 0);
    assert_eq!(same["anchor_advanced"], false);
    assert_eq!(same["basis_result_digest"], same["target_result_digest"]);
    assert_eq!(
        same["observation_coverage"]["temporal_coverage"],
        "endpoint_comparison_only"
    );
    assert_eq!(same["source_work"]["records"], 2);
    Ok(())
}

#[test]
fn equal_complete_endpoint_anchors_charge_one_source_without_trusting_unhashed_bytes() -> Result<()>
{
    let mut source = world(10, 1, &[(1, 1, 7), (2, 1, 8)]);
    source.graph.events.insert(
        EventId::new(1),
        WorldEvent {
            id: EventId::new(1),
            tick: source.tick,
            kind: WorldEventKind::Announcement,
            subject: None,
            summary: "x".repeat(8192),
            fields: BTreeMap::new(),
        },
    );
    source.refresh_hash();
    let source_bytes = source.canonical_bytes().len() as u64;
    let mut c = context(&source);
    c.budget.max_entities = 3;
    // Conservative preflight reserves record working space beyond canonical
    // bytes. The large unselected event leaves room for that overhead while
    // keeping a second complete source outside both byte and record budgets.
    c.budget.max_bytes = source_bytes + source_bytes / 2;
    assert!(c.budget.max_bytes > source_bytes);
    assert!(c.budget.max_bytes < source_bytes * 2);
    let ask = request(&input(&source, &source))?;
    let result = execute(&source, &source, &c, &ask)?;
    assert_eq!(
        result["source_work"],
        json!({"records":3,
        "canonical_bytes":source_bytes})
    );
    assert_eq!(result["change_count"], 0);
    let mut forged = source.clone();
    forged
        .graph
        .entities
        .get_mut(&EntityId::new(1))
        .ok_or_else(|| invalid("test entity absent"))?
        .label = "Urist 9".into();
    assert!(
        matches!(execute(&source, &forged, &c, &ask), Err(e) if e.code == ErrorCode::InternalInvariantViolation)
    );
    Ok(())
}

#[test]
fn exact_historical_target_is_distinct_from_current_and_omitted_target() -> Result<()> {
    let before = world(10, 1, &[(1, 1, 1)]);
    let after = world(20, 2, &[(1, 1, 2)]);
    let latest = world(30, 3, &[(1, 1, 3)]);
    let c = context(&latest);
    let ask = input(&before, &after);
    let result = execute(&before, &after, &c, &request(&ask)?)?;
    assert_eq!(result["historical"], true);
    assert_eq!(result["anchor"], anchor_json(after.anchor()));
    assert_eq!(result["current_anchor"], anchor_json(latest.anchor()));
    assert_eq!(result["observation_coverage"]["historical"], true);
    let mut implicit = ask.clone();
    implicit
        .as_object_mut()
        .ok_or_else(|| invalid("test object absent"))?
        .remove("after");
    assert!(
        matches!(execute(&before, &after, &c, &request(&implicit)?), Err(e) if e.code == ErrorCode::StaleAnchor)
    );
    assert!(execute(&before, &latest, &c, &request(&implicit)?).is_ok());
    let mut wrong = ask;
    wrong["before"] = json!(latest.state_hash.to_string());
    assert!(
        matches!(execute(&before, &after, &c, &request(&wrong)?), Err(e) if e.code == ErrorCode::StaleAnchor)
    );
    wrong["before"] = json!(before.state_hash.to_string());
    wrong["after"] = json!(latest.state_hash.to_string());
    assert!(
        matches!(execute(&before, &after, &c, &request(&wrong)?), Err(e) if e.code == ErrorCode::StaleAnchor)
    );
    Ok(())
}

#[test]
fn reversed_restored_foreign_future_and_same_cursor_forked_worlds_refuse() -> Result<()> {
    let before = world(10, 1, &[(1, 1, 1)]);
    let after = world(20, 2, &[(1, 1, 2)]);
    let latest = world(30, 3, &[(1, 1, 3)]);
    let c = context(&latest);
    for case in 0..9 {
        let mut basis = before.clone();
        let mut target = after.clone();
        match case {
            0 => std::mem::swap(&mut basis, &mut target),
            1 => basis.cursor.epoch = 2,
            2 => {
                basis.cursor.epoch = 2;
                target.cursor.epoch = 2;
            }
            3 => target.tick = GameTick(9),
            4 => target.cursor = basis.cursor,
            5 => target.cursor = latest.cursor,
            6 => target.fortress_id = FortressId::new(42),
            7 => target.cursor.sequence = 4,
            _ => target.tick = GameTick(31),
        }
        basis.refresh_hash();
        target.refresh_hash();
        assert!(
            matches!(execute(&basis, &target, &c, &request(&input(&basis, &target))?), Err(e) if e.code == ErrorCode::StaleAnchor),
            "case={case}"
        );
    }
    Ok(())
}

#[test]
fn current_authority_is_checked_before_any_historical_or_source_budget_work() -> Result<()> {
    let before = world(10, 1, &[(1, 1, 1)]);
    let after = world(20, 2, &[(1, 1, 2)]);
    let latest = world(100, 3, &[]);
    let ask = request(&input(&before, &after))?;
    for case in 0..5 {
        let mut c = context(&latest);
        c.budget.max_entities = 1;
        c.budget.max_bytes = 1;
        match case {
            0 => c.grants.clear(),
            1 => c.grants[0].expires_at_tick = Some(GameTick(50)),
            2 => c.grants[0].remaining_uses = Some(0),
            3 => c.grants[0].remaining_uses = Some(1),
            _ => c.grants[0].scope.entity_ids = BTreeSet::from([EntityId::new(1)]),
        }
        assert!(
            matches!(execute(&before, &after, &c, &ask), Err(e) if e.code == ErrorCode::CapabilityDenied),
            "case={case}"
        );
    }
    let mut c = context(&latest);
    c.cancellation_requested = true;
    assert!(
        matches!(execute(&before, &after, &c, &ask), Err(e) if e.code == ErrorCode::CancellationRequested)
    );
    Ok(())
}

#[test]
fn continuation_keeps_sources_selection_session_and_current_anchor_but_allows_page_width()
-> Result<()> {
    let before = world(10, 1, &[(1, 1, 1), (2, 1, 2), (3, 1, 3)]);
    let after = world(20, 2, &[(1, 1, 4), (2, 1, 5), (3, 1, 6)]);
    let c = context(&after);
    let mut ask = input(&before, &after);
    ask["limit"] = json!(1);
    let first = execute(&before, &after, &c, &request(&ask)?)?;
    assert_eq!(first["returned"], 1);
    assert_eq!(first["truncated"], true);
    assert_eq!(
        first["observation_coverage"]["continuation"],
        first["continuation"]
    );
    let token = first["continuation"]
        .as_str()
        .ok_or_else(|| invalid("test cursor absent"))?;
    ask["continuation"] = json!(token);
    ask["limit"] = json!(128);
    let rest = execute(&before, &after, &c, &request(&ask)?)?;
    assert_eq!(rest["returned"], 2);
    assert_eq!(rest["changes"][0]["entity_id"], "2");
    assert_eq!(rest["continuation"], Value::Null);
    assert_eq!(rest["comparison_digest"], first["comparison_digest"]);
    let mut other_request = c.clone();
    other_request.request_id = RequestId::new(2);
    assert_eq!(
        execute(&before, &after, &other_request, &request(&ask)?)?["changes"],
        rest["changes"]
    );
    for case in 0..5 {
        let mut changed = ask.clone();
        let mut cc = c.clone();
        let mut old = before.clone();
        match case {
            0 => changed["select"]["fields"] = json!([]),
            1 => changed["select"]["order"] = json!("id_descending"),
            2 => cc.session_id = SessionId::new(482),
            3 => cc.anchor = world(30, 3, &[]).anchor(),
            _ => {
                old.graph.events.insert(
                    EventId::new(1),
                    WorldEvent {
                        id: EventId::new(1),
                        tick: old.tick,
                        kind: WorldEventKind::Announcement,
                        subject: None,
                        summary: "unselected source changed".into(),
                        fields: BTreeMap::new(),
                    },
                );
                old.refresh_hash();
                changed["before"] = json!(old.state_hash.to_string());
            }
        }
        assert!(
            execute(&old, &after, &cc, &request(&changed)?).is_err(),
            "case={case}"
        );
    }
    Ok(())
}

#[test]
fn shared_native_endpoint_tokens_cannot_cross_into_the_laboratory_domain() -> Result<()> {
    let before = world(10, 1, &[(1, 1, 1), (2, 1, 2)]);
    let after = world(20, 2, &[(1, 1, 3), (2, 1, 4)]);
    let c = context(&after);
    let external = super::super::semantic_query::compare_endpoints(
        &before,
        &after,
        &c,
        &selection(),
        Digest32::ZERO,
        Some(1),
        None,
    )?;
    let mut ask = input(&before, &after);
    ask["continuation"] = external["continuation"].clone();
    assert!(
        matches!(execute(&before, &after, &c, &request(&ask)?), Err(e) if e.code == ErrorCode::StaleAnchor)
    );
    Ok(())
}

#[test]
fn both_complete_sources_share_the_work_budget_even_for_one_selected_change() -> Result<()> {
    let before = world(10, 1, &[(1, 1, 1), (2, 1, 2), (3, 1, 3)]);
    let after = world(20, 2, &[(1, 1, 4), (2, 1, 5), (3, 1, 6)]);
    let mut ask = input(&before, &after);
    ask["limit"] = json!(1);
    let mut c = context(&after);
    c.budget.max_entities = 5;
    assert!(
        matches!(execute(&before, &after, &c, &request(&ask)?), Err(e) if e.code == ErrorCode::BudgetExceeded)
    );
    c.budget.max_entities = 6;
    c.budget.max_bytes =
        (before.canonical_bytes().len() + after.canonical_bytes().len() - 1) as u64;
    assert!(
        matches!(execute(&before, &after, &c, &request(&ask)?), Err(e) if e.code == ErrorCode::BudgetExceeded)
    );
    let values: Vec<_> = (1..=257).map(|id| (id, 1, id as i64)).collect();
    let many = world(20, 2, &values);
    let empty = world(10, 1, &[]);
    let mut large = input(&empty, &many);
    large["limit"] = json!(1);
    assert!(
        matches!(execute(&empty, &many, &context(&many), &request(&large)?), Err(e) if e.code == ErrorCode::BudgetExceeded)
    );
    Ok(())
}

#[test]
fn output_reserves_source_context_and_never_clips_a_before_after_row() -> Result<()> {
    let before = world(10, 1, &[(1, 1, 1), (2, 1, 2), (3, 1, 3)]);
    let after = world(20, 2, &[(1, 1, 4), (2, 1, 5), (3, 1, 6)]);
    let mut ask = input(&before, &after);
    ask["limit"] = json!(1);
    let mut c = context(&after);
    c.budget.max_output_tokens = 2048;
    let first = execute(&before, &after, &c, &request(&ask)?)?;
    assert!(first.to_string().len() <= 8192);
    assert_eq!(first["returned"], 1);
    assert_eq!(first["changes"][0]["before"]["entity_id"], "1");
    assert_eq!(first["changes"][0]["after"]["entity_id"], "1");
    assert_eq!(first["truncated"], true);
    c.budget.max_output_tokens = 64;
    assert!(
        matches!(execute(&before, &after, &c, &request(&ask)?), Err(e) if e.code == ErrorCode::BudgetExceeded)
    );
    Ok(())
}

#[test]
fn malformed_canonical_world_is_not_accepted_under_its_old_hash() -> Result<()> {
    let before = world(10, 1, &[(1, 1, 1)]);
    let mut after = world(20, 2, &[(1, 1, 2)]);
    let c = context(&after);
    let ask = request(&input(&before, &after))?;
    after
        .graph
        .entities
        .get_mut(&EntityId::new(1))
        .ok_or_else(|| invalid("test entity absent"))?
        .label = "unhashed edit".into();
    assert!(
        matches!(execute(&before, &after, &c, &ask), Err(e) if e.code == ErrorCode::InternalInvariantViolation)
    );
    Ok(())
}
