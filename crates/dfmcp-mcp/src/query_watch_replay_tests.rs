use super::*;
use dfmcp_core::{
    CapabilityGrant, CapabilityScope, FortressId, GameTick, ObservationCursor, RequestId,
    WorkBudget,
};
use dfmcp_world::{EntityKind, EntityRecord, Fact, WorldGraph};

fn snapshot(tick: u64, sequence: u64, paused: bool, generation: u32) -> WorldSnapshot {
    let mut graph = WorldGraph::default();
    let id = EntityId::new(1);
    graph.entities.insert(
        id,
        EntityRecord {
            id,
            generation,
            revision: sequence,
            kind: EntityKind::Unit,
            label: String::new(),
            fields: BTreeMap::from([(
                "ready".into(),
                Fact::known(
                    WorldValue::Bool(true),
                    GameTick(tick),
                    FactSource::DfhackField("fixture.ready".into()),
                    Digest32::of_bytes(b"replay-fixture"),
                ),
            )]),
        },
    );
    WorldSnapshot::new(
        FortressId::new(23),
        GameTick(tick),
        ObservationCursor { epoch: 0, sequence },
        paused,
        graph,
    )
}
fn context(anchor: StateAnchor) -> OperationContext {
    OperationContext {
        session_id: SessionId::new(91_823),
        request_id: RequestId::new(1),
        anchor,
        budget: WorkBudget {
            max_entities: 100_000,
            max_bytes: 1024 * 1024,
            max_output_tokens: 65536,
            max_wall_millis: 60_000,
            ..WorkBudget::default()
        },
        cancellation_requested: false,
        grants: vec![CapabilityGrant {
            capability: Capability::Query,
            scope: CapabilityScope::default(),
            max_risk: RiskTier::ReadOnly,
            expires_at_tick: None,
            remaining_uses: None,
        }],
    }
}
fn definition(stable: u32, cadence: u64, deadline: u64) -> Value {
    json!({"condition":{"op":"paused","value":true},"deadline_tick":deadline,
        "poll_interval_ticks":cadence,"stable_observations":stable})
}
fn evaluate(input: &Value, samples: &[WorldSnapshot], details: bool) -> Result<Value> {
    let first = samples.first().ok_or_else(|| invalid("no test samples"))?;
    let last = samples.last().ok_or_else(|| invalid("no test samples"))?;
    let c = context(last.anchor());
    let mut replay = Replay::new(
        input,
        first.anchor(),
        1,
        samples.len() as u64,
        Digest32::of_bytes(b"exact-archive-range"),
        &c,
    )?;
    for (index, sample) in samples.iter().enumerate() {
        replay.advance(index as u64 + 1, sample, &c)?;
    }
    replay.finish(details, &c)
}

#[test]
fn retrospective_stability_uses_the_actual_reset_and_sample_rules() -> Result<()> {
    let samples: Vec<_> = [true, false, true, true, true]
        .iter()
        .enumerate()
        .map(|(i, p)| snapshot(i as u64 + 1, i as u64 + 1, *p, 1))
        .collect();
    let value = evaluate(&definition(3, 1, 50), &samples, true)?;
    assert_eq!(value["status"], "satisfied");
    assert_eq!(value["stable_observations"], 3);
    assert_eq!(value["sample_count"], 5);
    assert_eq!(value["first_terminal_record"], 5);
    assert_eq!(value["status_change_count"], 4);
    assert_eq!(value["records_evaluated"], 5);
    assert_eq!(value["watch_registered"], false);
    assert_eq!(value["original_watch_activity_proven"], false);
    assert!(value.get("watch").is_none());
    Ok(())
}

#[test]
fn cadence_and_sequence_gaps_are_not_replaced_by_record_counting() -> Result<()> {
    let samples: Vec<_> = [1, 2, 4, 5, 7]
        .iter()
        .enumerate()
        .map(|(i, t)| snapshot(*t, i as u64 + 1, true, 1))
        .collect();
    let value = evaluate(&definition(3, 3, 50), &samples, false)?;
    assert_eq!(value["status"], "satisfied");
    assert_eq!(value["sample_count"], 3);
    assert_eq!(value["first_terminal_record"], 5);
    let samples = vec![
        snapshot(1, 1, true, 1),
        snapshot(2, 3, true, 1),
        snapshot(3, 4, true, 1),
    ];
    let value = evaluate(&definition(3, 1, 50), &samples, false)?;
    assert_eq!(value["status"], "candidate");
    assert_eq!(value["stable_observations"], 2);
    Ok(())
}

#[test]
fn failure_and_unknown_guards_keep_foreground_precedence() -> Result<()> {
    let samples = vec![snapshot(1, 1, true, 1), snapshot(2, 2, true, 1)];
    let mut input = definition(1, 1, 50);
    input["failure_condition"] = json!({"op":"paused","value":true});
    let failed = evaluate(&input, &samples, false)?;
    assert_eq!(failed["status"], "failed");
    assert_eq!(failed["first_terminal_record"], 1);
    input["failure_condition"] = json!({"op":"field","entity_id":"1","generation":1,
        "field":"not_observed","comparison":"eq","value":{"type":"bool","value":true}});
    let unknown = evaluate(&input, &samples, true)?;
    assert_eq!(unknown["status"], "blocked_unknown");
    assert_eq!(unknown["stable_observations"], 0);
    Ok(())
}

#[test]
fn recycled_identity_is_not_hidden_by_a_decisive_boolean_branch() -> Result<()> {
    let mut input = definition(1, 1, 50);
    input["condition"] = json!({"op":"any","args":[{"op":"paused","value":true},
        {"op":"field","entity_id":"1","generation":1,"field":"ready",
            "comparison":"eq","value":{"type":"bool","value":true}}]});
    let value = evaluate(&input, &[snapshot(1, 1, true, 2)], true)?;
    assert_eq!(value["status"], "invalidated");
    assert_eq!(value["first_terminal_record"], 1);
    Ok(())
}

#[test]
fn deadline_expiration_is_evaluated_at_retained_observations_only() -> Result<()> {
    let samples = vec![
        snapshot(1, 1, true, 1),
        snapshot(3, 2, true, 1),
        snapshot(5, 3, true, 1),
        snapshot(7, 4, true, 1),
    ];
    let value = evaluate(&definition(4, 1, 5), &samples, true)?;
    assert_eq!(value["status"], "expired");
    assert_eq!(value["first_terminal_record"], 3);
    assert_eq!(value["last_evaluated_anchor"]["game_tick"], 5);
    assert_eq!(value["records_after_terminal"], 1);
    let partial = evaluate(&definition(4, 1, 5), &samples[..2], false)?;
    assert_eq!(partial["status"], "candidate");
    assert!(partial["first_terminal_record"].is_null());
    Ok(())
}

#[test]
fn epoch_reset_invalidates_unfinished_work_but_does_not_reopen_terminal_evidence() -> Result<()> {
    let first = snapshot(3, 1, true, 1);
    let mut next = snapshot(1, 0, false, 2);
    next.cursor.epoch = 1;
    next.refresh_hash();
    let value = evaluate(&definition(2, 1, 50), &[first.clone(), next.clone()], false)?;
    assert_eq!(value["status"], "invalidated");
    let value = evaluate(&definition(1, 1, 50), &[first, next], false)?;
    assert_eq!(value["status"], "satisfied");
    assert_eq!(value["records_evaluated"], 1);
    assert_eq!(value["records_verified"], 2);
    assert_eq!(value["records_after_terminal"], 1);
    Ok(())
}

#[test]
fn evidence_identity_is_deterministic_and_detail_does_not_change_the_result() -> Result<()> {
    let samples = vec![snapshot(1, 1, true, 1), snapshot(2, 2, true, 1)];
    let minimal = json!({"condition":{"op":"paused","value":true},"deadline_tick":50});
    let summary = evaluate(&minimal, &samples, false)?;
    let full = evaluate(&definition(2, 1, 50), &samples, true)?;
    assert_eq!(summary["replay_id"], full["replay_id"]);
    assert_eq!(summary["evidence_digest"], full["evidence_digest"]);
    assert!(summary.get("evaluation").is_none());
    assert!(full["evaluation"].is_object());
    assert_eq!(summary["status_changes_complete"], false);
    assert_eq!(full["status_changes_complete"], true);
    Ok(())
}

#[test]
fn partial_out_of_order_or_failed_replays_cannot_publish() -> Result<()> {
    let first = snapshot(1, 1, true, 1);
    let c = context(first.anchor());
    let new = || {
        Replay::new(
            &definition(2, 1, 50),
            first.anchor(),
            1,
            2,
            Digest32::of_bytes(b"range"),
            &c,
        )
    };
    let mut replay = new()?;
    replay.advance(1, &first, &c)?;
    assert!(replay.finish(false, &c).is_err());
    let mut replay = new()?;
    assert!(replay.advance(2, &first, &c).is_err());
    assert!(replay.advance(1, &first, &c).is_err());
    assert!(replay.finish(false, &c).is_err());
    let mut replay = new()?;
    let mut bad = first.clone();
    bad.paused = false;
    assert!(replay.advance(1, &bad, &c).is_err());
    assert!(replay.finish(false, &c).is_err());
    Ok(())
}

#[test]
fn historical_ticks_do_not_revive_expired_revoked_or_cancelled_authority() -> Result<()> {
    let first = snapshot(1, 1, true, 1);
    let mut c = context(snapshot(100, 10, true, 1).anchor());
    for case in 0..3 {
        let mut denied = c.clone();
        match case {
            0 => denied.grants[0].expires_at_tick = Some(GameTick(5)),
            1 => denied.grants.clear(),
            _ => denied.cancellation_requested = true,
        }
        assert!(
            Replay::new(
                &definition(2, 1, 50),
                first.anchor(),
                1,
                1,
                Digest32::of_bytes(b"range"),
                &denied
            )
            .is_err()
        );
    }
    let mut replay = Replay::new(
        &definition(2, 1, 50),
        first.anchor(),
        1,
        1,
        Digest32::of_bytes(b"range"),
        &c,
    )?;
    c.grants.clear();
    assert!(replay.advance(1, &first, &c).is_err());
    assert!(replay.finish(false, &c).is_err());
    Ok(())
}

#[test]
fn strict_definition_range_and_output_bounds_are_enforced() -> Result<()> {
    let first = snapshot(1, 1, true, 1);
    let c = context(first.anchor());
    let binding = Digest32::of_bytes(b"range");
    for (start, end) in [(0, 1), (2, 1), (1, 33)] {
        assert!(
            Replay::new(
                &definition(2, 1, 50),
                first.anchor(),
                start,
                end,
                binding,
                &c
            )
            .is_err()
        );
    }
    for input in [
        definition(0, 1, 50),
        definition(65, 1, 50),
        definition(2, 0, 50),
        definition(2, 1, 1),
        json!({"condition":{"op":"paused","value":true},"deadline_tick":50,"unknown":true}),
    ] {
        assert!(Replay::new(&input, first.anchor(), 1, 1, binding, &c).is_err());
    }
    let mut replay = Replay::new(&definition(1, 1, 50), first.anchor(), 1, 1, binding, &c)?;
    replay.advance(1, &first, &c)?;
    let mut narrow = c;
    narrow.budget.max_bytes = 1;
    assert!(matches!(replay.finish(true,&narrow),Err(e)if e.code==ErrorCode::BudgetExceeded));
    Ok(())
}

#[test]
fn all_samples_share_one_population_evaluation_work_allowance() -> Result<()> {
    let mut first = snapshot(1, 1, true, 1);
    let template = first
        .graph
        .entities
        .get(&EntityId::new(1))
        .cloned()
        .ok_or_else(|| invalid("template"))?;
    for n in 2..=5000 {
        let mut entity = template.clone();
        entity.id = EntityId::new(n);
        first.graph.entities.insert(entity.id, entity);
    }
    first.refresh_hash();
    let c = context(first.anchor());
    let input = json!({"condition":{"op":"entity_count","scope":"observed_projection","kind":"unit",
        "predicate":{"op":"all","args":[{"op":"always"},{"op":"always"},{"op":"always"},{"op":"always"},{"op":"always"}]},
        "comparison":"ge","value":1},"stable_observations":64,"deadline_tick":100});
    let mut replay = Replay::new(
        &input,
        first.anchor(),
        1,
        32,
        Digest32::of_bytes(b"bounded-range"),
        &c,
    )?;
    let mut exhausted = false;
    for record in 1..=32 {
        let mut sample = first.clone();
        sample.tick = GameTick(record);
        sample.cursor.sequence = record;
        sample.refresh_hash();
        if let Err(error) = replay.advance(record, &sample, &c) {
            assert_eq!(error.code, ErrorCode::BudgetExceeded);
            exhausted = true;
            break;
        }
    }
    assert!(exhausted);
    assert!(replay.finish(false, &c).is_err());
    Ok(())
}
