use super::*;
use std::collections::{BTreeMap, BTreeSet};

use dfmcp_core::{
    CapabilityGrant, CapabilityScope, EntityId, FortressId, GameTick, ObservationCursor, RequestId,
    SessionId, WorkBudget,
};
use dfmcp_world::{
    EntityKind, EntityRecord, Fact, FactPresence, FactSource, Value as WorldValue, WorldGraph,
};

fn world(tick: u64, sequence: u64, ready: bool) -> WorldSnapshot {
    let id = EntityId::new(1);
    let mut graph = WorldGraph::default();
    graph.entities.insert(
        id,
        EntityRecord {
            id,
            generation: 1,
            revision: sequence + 1,
            kind: EntityKind::Unit,
            label: "Urist".into(),
            fields: BTreeMap::from([(
                "ready".into(),
                Fact::known(
                    WorldValue::Bool(ready),
                    GameTick(tick),
                    FactSource::Derived("dfmcp.lab-scenario/1".into()),
                    Digest32::ZERO,
                ),
            )]),
        },
    );
    WorldSnapshot::new(
        FortressId::new(78),
        GameTick(tick),
        ObservationCursor { epoch: 3, sequence },
        false,
        graph,
    )
}

fn context(world: &WorldSnapshot) -> OperationContext {
    OperationContext {
        session_id: SessionId::new(781),
        request_id: RequestId::new(1),
        anchor: world.anchor(),
        budget: WorkBudget {
            max_entities: 10_000,
            max_bytes: 4 * 1024 * 1024,
            max_output_tokens: 65_536,
            max_wall_millis: 60_000,
            max_game_ticks: 0,
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

fn history(sources: &[WorldSnapshot]) -> VersionRetention {
    let mut history = VersionRetention::new(64, 2);
    for source in sources {
        history.record(source);
    }
    history
}

fn definition(stable: u32) -> Value {
    json!({"condition":{"op":"field","entity_id":"1","generation":1,
        "field":"ready","comparison":"eq","value":{"type":"bool","value":true}},
        "deadline_tick":100,"poll_interval_ticks":1,"stable_observations":stable})
}

fn input(first: &WorldSnapshot, last: &WorldSnapshot, stable: u32) -> Value {
    json!({"mode":"condition_replay","from":first.state_hash.to_string(),
        "to":last.state_hash.to_string(),"definition":definition(stable),"detail":"evidence"})
}

fn request(input: &Value) -> Result<Request> {
    parse(&input.to_string())?.ok_or_else(|| invalid("test condition replay missing"))
}

fn replay(sources: &[WorldSnapshot], input: &Value) -> Result<Value> {
    let last = sources
        .last()
        .ok_or_else(|| invalid("test source missing"))?;
    execute(&history(sources), last, &context(last), &request(input)?)
}

#[test]
fn closed_wrapper_rejects_duplicate_fields_and_native_policy_inputs() -> Result<()> {
    let source = world(1, 1, true);
    let valid = input(&source, &source, 1);
    assert!(parse("summary")?.is_none());
    assert!(parse(r#"{"mode":"temporal"}"#)?.is_none());
    assert!(parse(&valid.to_string())?.is_some());
    for case in 0..8 {
        let mut bad = valid.clone();
        match case {
            0 => bad["from"] = json!("A".repeat(64)),
            1 => bad["to"] = json!("a".repeat(63)),
            2 => bad["detail"] = json!("partial"),
            3 => bad["evidence_policy"] = json!("native"),
            4 => bad["definition"]["key"] = json!("retained-watch"),
            5 => bad["definition"]["condition"] = json!({"op":"item_quantity","value":1}),
            6 => bad["definition"]["condition"]["source_policy"] = json!("native"),
            _ => bad["from"] = Value::Null,
        }
        assert!(
            matches!(parse(&bad.to_string()), Err(e) if e.code == ErrorCode::InvalidRequest),
            "case={case}"
        );
    }
    let raw = valid.to_string();
    for field in ["from", "definition", "mode"] {
        let duplicate = format!("{{{field:?}:{},{}", valid[field], &raw[1..]);
        assert!(parse(&duplicate).is_err(), "{field}");
    }
    let duplicate = raw.replace("\"generation\":1", "\"generation\":1,\"generation\":2");
    assert!(parse(&duplicate).is_err());
    assert!(
        matches!(parse(&" ".repeat(MAX_REQUEST_BYTES + 1)), Err(e) if e.code == ErrorCode::BudgetExceeded)
    );
    Ok(())
}

#[test]
fn retained_samples_reveal_a_reversal_and_use_actual_stability_rules() -> Result<()> {
    let sources: Vec<_> = [true, false, true, true, true]
        .iter()
        .enumerate()
        .map(|(i, ready)| world(i as u64 + 1, i as u64 + 1, *ready))
        .collect();
    let retained = history(&sources);
    let before = format!("{retained:?}");
    let last = &sources[4];
    let value = execute(
        &retained,
        last,
        &context(last),
        &request(&input(&sources[0], last, 3))?,
    )?;
    assert_eq!(value["status"], "satisfied");
    assert_eq!(value["first_terminal_record"], 5);
    assert_eq!(value["sample_count"], 5);
    assert_eq!(value["status_change_count"], 4);
    assert_eq!(value["stable_observations"], 3);
    assert_eq!(value["source_work"]["records"], 5);
    assert_eq!(value["range"]["records"], 5);
    assert_eq!(value["range"]["observation_cursor_gaps"], 0);
    assert_eq!(value["watch_registered"], false);
    assert_eq!(value["original_watch_activity_proven"], false);
    assert_eq!(value["continuous_between_observations"], false);
    assert_eq!(value["eligible_as_mutation_evidence"], false);
    assert_eq!(value["evidence_policy"], "laboratory_reference/1");
    assert_eq!(format!("{retained:?}"), before);
    assert!(value.get("watch").is_none());
    Ok(())
}

#[test]
fn cadence_same_tick_and_unrecorded_cursor_gaps_cannot_manufacture_stability() -> Result<()> {
    let sources = vec![
        world(1, 1, true),
        world(1, 2, true),
        world(3, 4, true),
        world(4, 5, true),
    ];
    let value = replay(&sources, &input(&sources[0], &sources[3], 3))?;
    assert_eq!(value["status"], "candidate");
    assert_eq!(value["sample_count"], 3);
    assert_eq!(value["stable_observations"], 2);
    assert_eq!(value["range"]["observation_cursor_gaps"], 1);
    let mut ask = input(&sources[0], &sources[3], 3);
    ask["definition"]["poll_interval_ticks"] = json!(3);
    let value = replay(&sources, &ask)?;
    assert_eq!(value["status"], "candidate");
    assert_eq!(value["sample_count"], 2);
    assert_eq!(value["stable_observations"], 1);
    Ok(())
}

#[test]
fn unknowns_failure_guards_and_recycled_generations_preserve_monitor_precedence() -> Result<()> {
    let sources = vec![world(1, 1, true), world(2, 2, true)];
    let mut ask = input(&sources[0], &sources[1], 1);
    ask["definition"]["failure_condition"] = json!({"op":"paused","value":false});
    assert_eq!(replay(&sources, &ask)?["status"], "failed");
    ask["definition"]["failure_condition"] = definition(1)["condition"].clone();
    ask["definition"]["failure_condition"]["field"] = json!("not_observed");
    assert_eq!(replay(&sources, &ask)?["status"], "blocked_unknown");
    let mut changed = sources[1].clone();
    changed
        .graph
        .entities
        .get_mut(&EntityId::new(1))
        .ok_or_else(encoding_error)?
        .generation = 2;
    changed.refresh_hash();
    let changed_sources = vec![sources[0].clone(), changed.clone()];
    let mut ask = input(&changed_sources[0], &changed, 3);
    ask["definition"]["condition"] = json!({"op":"any","args":[
        {"op":"paused","value":false}, definition(3)["condition"]]});
    assert_eq!(replay(&changed_sources, &ask)?["status"], "invalidated");
    for source_kind in [
        FactSource::AgentAssertion("not evidence".into()),
        FactSource::DfhackField("ready".into()),
    ] {
        let mut source = sources[0].clone();
        source
            .graph
            .entities
            .get_mut(&EntityId::new(1))
            .ok_or_else(encoding_error)?
            .fields
            .get_mut("ready")
            .ok_or_else(encoding_error)?
            .source = source_kind;
        source.refresh_hash();
        let value = replay(std::slice::from_ref(&source), &input(&source, &source, 1))?;
        assert_eq!(value["status"], "blocked_unknown");
    }
    let mut redacted = sources[0].clone();
    let fact = redacted
        .graph
        .entities
        .get_mut(&EntityId::new(1))
        .ok_or_else(encoding_error)?
        .fields
        .get_mut("ready")
        .ok_or_else(encoding_error)?;
    fact.value = WorldValue::Text("concealed-replay-value".into());
    fact.presence = Some(FactPresence::Redacted("policy".into()));
    redacted.refresh_hash();
    let value = replay(
        std::slice::from_ref(&redacted),
        &input(&redacted, &redacted, 1),
    )?;
    assert_eq!(value["status"], "blocked_unknown");
    assert!(!value.to_string().contains("concealed-replay-value"));
    Ok(())
}

#[test]
fn deadlines_freeze_terminal_evidence_but_later_sources_still_require_validation() -> Result<()> {
    let sources = vec![
        world(1, 1, true),
        world(3, 2, true),
        world(5, 3, true),
        world(7, 4, false),
    ];
    let mut ask = input(&sources[0], &sources[3], 4);
    ask["definition"]["deadline_tick"] = json!(5);
    let expired = replay(&sources, &ask)?;
    assert_eq!(expired["status"], "expired");
    assert_eq!(expired["first_terminal_record"], 3);
    assert_eq!(expired["last_evaluated_anchor"]["game_tick"], 5);
    assert_eq!(expired["records_after_terminal"], 1);
    ask["definition"]["stable_observations"] = json!(1);
    let satisfied = replay(&sources, &ask)?;
    assert_eq!(satisfied["status"], "satisfied");
    assert_eq!(satisfied["records_evaluated"], 1);
    assert_eq!(satisfied["records_verified"], 4);
    let mut corrupt = sources.clone();
    corrupt[3].paused = true;
    assert_eq!(
        replay(&corrupt, &ask).unwrap_err().code,
        ErrorCode::ChecksumMismatch
    );
    Ok(())
}

#[test]
fn interval_budgets_cover_every_source_before_an_early_terminal_result() -> Result<()> {
    let sources = vec![world(1, 1, true), world(2, 2, true), world(3, 3, true)];
    let retained = history(&sources);
    let current = &sources[2];
    let request = request(&input(&sources[0], current, 1))?;
    let mut ctx = context(current);
    ctx.budget.max_entities = 2;
    assert_eq!(
        execute(&retained, current, &ctx, &request)
            .unwrap_err()
            .code,
        ErrorCode::BudgetExceeded
    );
    ctx = context(current);
    ctx.budget.max_bytes = sources
        .iter()
        .map(|s| s.canonical_bytes().len() as u64)
        .sum::<u64>()
        - 1;
    assert_eq!(
        execute(&retained, current, &ctx, &request)
            .unwrap_err()
            .code,
        ErrorCode::BudgetExceeded
    );
    ctx = context(current);
    ctx.budget.max_output_tokens = 32;
    assert_eq!(
        execute(&retained, current, &ctx, &request)
            .unwrap_err()
            .code,
        ErrorCode::BudgetExceeded
    );
    Ok(())
}

#[test]
fn current_authority_precedes_retention_and_historical_ticks_never_renew_grants() -> Result<()> {
    let sources = vec![world(1, 1, true), world(2, 2, true), world(50, 3, true)];
    let retained = history(&sources);
    let current = &sources[2];
    let historical = request(&input(&sources[0], &sources[1], 2))?;
    let baseline = execute(&retained, current, &context(current), &historical)?;
    assert_eq!(baseline["status"], "satisfied");
    assert_eq!(baseline["current_anchor"]["game_tick"], 50);
    assert_eq!(baseline["anchor"]["game_tick"], 2);
    for mode in 0..4 {
        let mut ctx = context(current);
        match mode {
            0 => ctx.grants.clear(),
            1 => ctx.grants[0].expires_at_tick = Some(GameTick(20)),
            2 => ctx.grants[0].remaining_uses = Some(0),
            _ => ctx.grants[0].scope.entity_ids = BTreeSet::from([EntityId::new(1)]),
        }
        assert_eq!(
            execute(&retained, current, &ctx, &historical)
                .unwrap_err()
                .code,
            ErrorCode::CapabilityDenied,
            "mode={mode}"
        );
    }
    let mut ctx = context(current);
    ctx.grants.clear();
    let mut missing = input(&sources[0], &sources[1], 2);
    missing["from"] = json!("f".repeat(64));
    assert_eq!(
        execute(&retained, current, &ctx, &request(&missing)?)
            .unwrap_err()
            .code,
        ErrorCode::CapabilityDenied
    );
    Ok(())
}

#[test]
fn exact_intervals_refuse_collected_interiors_reverse_ranges_and_epoch_crossings() -> Result<()> {
    let sources: Vec<_> = (1..=4).map(|i| world(i, i, true)).collect();
    let mut retained = VersionRetention::new(1, 2);
    for source in &sources {
        retained.record(source);
    }
    retained.collect(
        &[sources[0].state_hash, sources[2].state_hash]
            .into_iter()
            .collect(),
    );
    assert_eq!(
        execute(
            &retained,
            &sources[3],
            &context(&sources[3]),
            &request(&input(&sources[0], &sources[2], 1))?
        )
        .unwrap_err()
        .code,
        ErrorCode::StaleAnchor
    );
    assert_eq!(
        replay(&sources, &input(&sources[3], &sources[0], 1))
            .unwrap_err()
            .code,
        ErrorCode::InvalidRequest
    );
    let mut restored = sources[3].clone();
    restored.cursor.epoch += 1;
    restored.refresh_hash();
    let crossed = vec![sources[0].clone(), restored.clone()];
    assert_eq!(
        replay(&crossed, &input(&crossed[0], &restored, 1))
            .unwrap_err()
            .code,
        ErrorCode::StaleAnchor
    );
    let many: Vec<_> = (1..=33).map(|i| world(i, i, true)).collect();
    assert_eq!(
        replay(&many, &input(&many[0], &many[32], 1))
            .unwrap_err()
            .code,
        ErrorCode::BudgetExceeded
    );
    Ok(())
}

#[test]
fn evidence_identity_binds_every_sample_and_is_stable_across_detail_and_later_appends() -> Result<()>
{
    let sources = vec![world(1, 1, true), world(2, 2, true)];
    let full = replay(&sources, &input(&sources[0], &sources[1], 2))?;
    let mut ask = input(&sources[0], &sources[1], 2);
    ask["detail"] = json!("summary");
    ask["definition"]
        .as_object_mut()
        .ok_or_else(encoding_error)?
        .remove("poll_interval_ticks");
    ask["definition"]
        .as_object_mut()
        .ok_or_else(encoding_error)?
        .remove("stable_observations");
    let mut appended = sources.clone();
    appended.push(world(3, 3, false));
    let summary = replay(&appended, &ask)?;
    assert_eq!(full["replay_id"], summary["replay_id"]);
    assert_eq!(full["evidence_digest"], summary["evidence_digest"]);
    assert_eq!(full["range"]["binding"], summary["range"]["binding"]);
    assert!(summary.get("evaluation").is_none());
    assert_eq!(summary["status_changes_complete"], false);
    let different = vec![world(1, 1, false), sources[1].clone()];
    let changed = replay(&different, &input(&different[0], &different[1], 2))?;
    assert_ne!(full["replay_id"], changed["replay_id"]);
    Ok(())
}

#[test]
fn current_endpoint_requires_prior_session_observation_and_canonical_record_keys() -> Result<()> {
    let first = world(1, 1, true);
    let current = world(2, 2, true);
    let mut ask = input(&first, &current, 1);
    ask.as_object_mut().ok_or_else(encoding_error)?.remove("to");
    let missing = execute(
        &history(std::slice::from_ref(&first)),
        &current,
        &context(&current),
        &request(&ask)?,
    )
    .unwrap_err();
    assert_eq!(missing.code, ErrorCode::StaleAnchor);
    assert!(missing.message.contains("observe"));
    let mut bad = current.clone();
    let entity = bad
        .graph
        .entities
        .remove(&EntityId::new(1))
        .ok_or_else(encoding_error)?;
    bad.graph.entities.insert(EntityId::new(2), entity);
    bad.refresh_hash();
    assert_eq!(
        replay(&[first.clone(), bad.clone()], &input(&first, &bad, 1))
            .unwrap_err()
            .code,
        ErrorCode::InternalInvariantViolation
    );
    Ok(())
}
