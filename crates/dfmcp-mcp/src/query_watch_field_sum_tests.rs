use super::*;
use dfmcp_core::{
    CapabilityGrant, CapabilityScope, FortressId, GameTick, ObservationCursor, RequestId, WorkBudget,
};
use dfmcp_world::{Fact, WorldGraph};

fn known(value: WorldValue) -> Fact {
    Fact::known(
        value,
        GameTick(1),
        FactSource::Derived("dfmcp.lab-scenario/1".to_owned()),
        Digest32::ZERO,
    )
}

fn world(amounts: &[u64]) -> WorldSnapshot {
    let mut graph = WorldGraph::default();
    for (index, amount) in amounts.iter().enumerate() {
        let id = EntityId::new(index as u64 + 1);
        graph.entities.insert(
            id,
            EntityRecord {
                id,
                generation: 1,
                revision: 1,
                kind: EntityKind::Other(dfmcp_intent::effects::STOCK_LEDGER_KIND.to_owned()),
                label: String::new(),
                fields: BTreeMap::from([
                    ("amount".to_owned(), known(WorldValue::U64(*amount))),
                    ("selected".to_owned(), known(WorldValue::Bool(true))),
                ]),
            },
        );
    }
    WorldSnapshot::new(
        FortressId::new(971),
        GameTick(10),
        ObservationCursor { epoch: 1, sequence: 1 },
        false,
        graph,
    )
}

fn context(snapshot: &WorldSnapshot) -> OperationContext {
    OperationContext {
        session_id: SessionId::new(971_001),
        request_id: RequestId::new(1),
        anchor: snapshot.anchor(),
        budget: WorkBudget {
            max_entities: 10_000,
            max_bytes: 1024 * 1024,
            max_output_tokens: 65_536,
            max_wall_millis: 60_000,
            max_game_ticks: 1000,
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

fn condition(comparison: &str, value: u64) -> Value {
    json!({"op":"field_sum","scope":"observed_projection","kind":"stock_ledger","field":"amount",
        "predicate":{"op":"field","field":"selected","comparison":"eq",
            "value":{"type":"bool","value":true}},"comparison":comparison,"value":value})
}

fn inspect(snapshot: &WorldSnapshot, condition: Value) -> Result<Value> {
    laboratory::condition_query(
        snapshot,
        &context(snapshot),
        &json!({"schema":"dfmcp.query/1","query":{"kind":"condition_evaluation","condition":condition}}),
    )
}

fn edit(
    snapshot: &mut WorldSnapshot,
    id: u64,
    field: &str,
    change: impl FnOnce(&mut Fact),
) -> Result<()> {
    change(
        snapshot
            .graph
            .entities
            .get_mut(&EntityId::new(id))
            .and_then(|entity| entity.fields.get_mut(field))
            .ok_or_else(|| invalid("missing sum fixture fact"))?,
    );
    snapshot.refresh_hash();
    Ok(())
}

fn advance(snapshot: &mut WorldSnapshot) -> Result<()> {
    snapshot.tick = snapshot.tick.checked_add(1).ok_or_else(|| bounded("test tick"))?;
    snapshot.cursor = snapshot.cursor.checked_next().ok_or_else(|| bounded("test cursor"))?;
    snapshot.refresh_hash();
    Ok(())
}

fn raw_comparison(actual: u64, comparison: Comparison, value: u64) -> bool {
    match comparison {
        Comparison::Eq => actual == value,
        Comparison::Ne => actual != value,
        Comparison::Lt => actual < value,
        Comparison::Le => actual <= value,
        Comparison::Gt => actual > value,
        Comparison::Ge => actual >= value,
    }
}

#[test]
fn finite_interval_comparisons_match_every_possible_total() {
    for lower in 0..=7 {
        for upper in lower..=7 {
            for threshold in 0..=8 {
                for comparison in [
                    Comparison::Eq, Comparison::Ne, Comparison::Lt,
                    Comparison::Le, Comparison::Gt, Comparison::Ge,
                ] {
                    let all = (lower..=upper).all(|value| raw_comparison(value, comparison, threshold));
                    let none = (lower..=upper).all(|value| !raw_comparison(value, comparison, threshold));
                    let expected = if all { Truth::True } else if none { Truth::False } else { Truth::Unknown };
                    assert_eq!(
                        Bounds { lower, upper: Some(upper) }.compare(comparison, threshold),
                        expected,
                        "interval {lower}..={upper}, {comparison:?} {threshold}"
                    );
                }
            }
        }
    }
}

#[test]
fn unbounded_interval_comparisons_preserve_sound_one_sided_conclusions() {
    let bounds = Bounds { lower: 5, upper: None };
    for (comparison, threshold, expected) in [
        (Comparison::Eq, 4, Truth::False),
        (Comparison::Eq, 5, Truth::Unknown),
        (Comparison::Ne, 4, Truth::True),
        (Comparison::Ne, 5, Truth::Unknown),
        (Comparison::Lt, 5, Truth::False),
        (Comparison::Lt, 6, Truth::Unknown),
        (Comparison::Le, 4, Truth::False),
        (Comparison::Le, 5, Truth::Unknown),
        (Comparison::Gt, 4, Truth::True),
        (Comparison::Gt, 5, Truth::Unknown),
        (Comparison::Ge, 5, Truth::True),
        (Comparison::Ge, 6, Truth::Unknown),
        (Comparison::Eq, u64::MAX, Truth::Unknown),
    ] {
        assert_eq!(bounds.compare(comparison, threshold), expected);
    }
}

#[test]
fn exact_sum_counts_each_selected_row_once_and_discloses_projection_scope() -> Result<()> {
    let snapshot = world(&[4, 6, 10]);
    let mut selected = condition("eq", 20);
    let membership = selected["predicate"].clone();
    selected["predicate"] = json!({"op":"any","args":[membership.clone(),membership]});
    let first = inspect(&snapshot, selected.clone())?;
    assert_eq!(first["evaluation"]["condition_truth"], "true");
    let fact = &first["evaluation"]["facts"][0];
    assert_eq!(fact["sum_min"], 20);
    assert_eq!(fact["sum_max"], 20);
    assert_eq!(fact["sum_exact"], true);
    assert_eq!(fact["population"], 3);
    assert_eq!(fact["matched_records_min"], 3);
    assert_eq!(fact["value_type"], "u64");
    assert_eq!(fact["complete_world_quantity_proven"], false);
    assert_eq!(fact["usable_supply_proven"], false);
    assert_eq!(fact["matching_examples"].as_array().map(Vec::len), Some(2));
    assert_eq!(fact["examples_complete"], false);
    assert_eq!(inspect(&snapshot, selected)?, first);
    Ok(())
}

#[test]
fn unknown_membership_has_finite_bounds_and_known_zero_can_still_be_exact() -> Result<()> {
    let mut snapshot = world(&[4, 6]);
    edit(&mut snapshot, 2, "selected", |fact| {
        fact.presence = Some(FactPresence::Unknown("not observed".into()));
    })?;
    let ambiguous = inspect(&snapshot, condition("eq", 4))?;
    assert_eq!(ambiguous["evaluation"]["condition_truth"], "unknown");
    assert_eq!(ambiguous["evaluation"]["facts"][0]["sum_min"], 4);
    assert_eq!(ambiguous["evaluation"]["facts"][0]["sum_max"], 10);
    assert_eq!(ambiguous["evaluation"]["facts"][0]["unestablished_membership_records"], 1);
    assert_eq!(inspect(&snapshot, condition("ge", 4))?["evaluation"]["condition_truth"], "true");
    edit(&mut snapshot, 2, "amount", |fact| *fact = known(WorldValue::U64(0)))?;
    let zero = inspect(&snapshot, condition("eq", 4))?;
    assert_eq!(zero["evaluation"]["condition_truth"], "true");
    assert_eq!(zero["evaluation"]["facts"][0]["sum_exact"], true);
    assert_eq!(zero["evaluation"]["facts"][0]["unestablished_records"], 1);
    Ok(())
}

#[test]
fn unavailable_values_never_become_zero_but_definitely_excluded_rows_do_not_contribute() -> Result<()> {
    let mut snapshot = world(&[4, 6]);
    snapshot.graph.entities.get_mut(&EntityId::new(2))
        .ok_or_else(|| invalid("missing entity"))?.fields.remove("amount");
    snapshot.refresh_hash();
    let missing = inspect(&snapshot, condition("eq", 4))?;
    assert_eq!(missing["evaluation"]["condition_truth"], "unknown");
    assert_eq!(missing["evaluation"]["facts"][0]["sum_min"], 4);
    assert_eq!(missing["evaluation"]["facts"][0]["sum_max"], Value::Null);
    assert_eq!(inspect(&snapshot, condition("ge", 4))?["evaluation"]["condition_truth"], "true");
    edit(&mut snapshot, 2, "selected", |fact| *fact = known(WorldValue::Bool(false)))?;
    let excluded = inspect(&snapshot, condition("eq", 4))?;
    assert_eq!(excluded["evaluation"]["condition_truth"], "true");
    assert_eq!(excluded["evaluation"]["facts"][0]["excluded_records"], 1);
    assert_eq!(excluded["evaluation"]["facts"][0]["unestablished_value_records"], 0);
    let empty = inspect(&world(&[]), condition("eq", 0))?;
    assert_eq!(empty["evaluation"]["facts"][0]["sum_exact"], true);
    assert_eq!(empty["evaluation"]["facts"][0]["complete_world_quantity_proven"], false);
    Ok(())
}

#[test]
fn value_and_membership_use_exact_laboratory_source_and_presence_policy() -> Result<()> {
    for field in ["amount", "selected"] {
        for variant in 0..10 {
            let mut snapshot = world(&[7]);
            edit(&mut snapshot, 1, field, |fact| match variant {
                0 => fact.source = FactSource::AgentAssertion("asserted".into()),
                1 => fact.source = FactSource::Replay,
                2 => fact.source = FactSource::Derived("unregistered".into()),
                3 => {
                    fact.source = FactSource::DfhackField("native.quantity".into());
                    fact.source_digest = Digest32::of_bytes(b"native");
                }
                4 => fact.source_digest = Digest32::of_bytes(b"changed-lab-source"),
                5 => fact.observed_at = GameTick(11),
                6 => fact.presence = Some(FactPresence::Known(WorldValue::U64(99))),
                7 => fact.presence = Some(FactPresence::Unknown("unknown".into())),
                8 => fact.presence = Some(FactPresence::Redacted("redacted".into())),
                _ => fact.presence = Some(FactPresence::Omitted("omitted".into())),
            })?;
            let output = inspect(&snapshot, condition("eq", 7))?;
            assert_eq!(output["evaluation"]["condition_truth"], "unknown", "{field} {variant}");
            assert_eq!(output["evaluation"]["eligible_success_sample"], false);
        }
    }
    for value in [
        WorldValue::I64(7), WorldValue::I64(-7), WorldValue::Bool(true),
        WorldValue::Text("7".into()), WorldValue::Fixed { units: 7, scale: 0 },
    ] {
        let mut snapshot = world(&[7]);
        edit(&mut snapshot, 1, "amount", |fact| *fact = known(value))?;
        assert_eq!(inspect(&snapshot, condition("eq", 7))?["evaluation"]["condition_truth"], "unknown");
    }
    let mut snapshot = world(&[7]);
    edit(&mut snapshot, 1, "amount", |fact| {
        fact.source = FactSource::Derived("dfmcp.reference-effects/1".into());
    })?;
    assert_eq!(inspect(&snapshot, condition("eq", 7))?["evaluation"]["condition_truth"], "true");
    Ok(())
}

#[test]
fn aggregate_overflow_refuses_without_saturating_known_or_possible_totals() -> Result<()> {
    let snapshot = world(&[u64::MAX, 1]);
    assert!(matches!(inspect(&snapshot, condition("ge", 1)), Err(error)
        if error.code == ErrorCode::BudgetExceeded));
    let mut snapshot = snapshot;
    edit(&mut snapshot, 2, "selected", |fact| {
        fact.presence = Some(FactPresence::Unknown("unknown".into()));
    })?;
    assert!(matches!(inspect(&snapshot, condition("ge", 1)), Err(error)
        if error.code == ErrorCode::BudgetExceeded));
    assert_eq!(
        inspect(&world(&[u64::MAX]), condition("eq", u64::MAX))?["evaluation"]["condition_truth"],
        "true"
    );
    Ok(())
}

#[test]
fn dynamic_membership_recomputes_generations_and_invalid_identity_is_unknown() -> Result<()> {
    let mut snapshot = world(&[4]);
    let before = inspect(&snapshot, condition("eq", 4))?;
    let entity = snapshot.graph.entities.get_mut(&EntityId::new(1))
        .ok_or_else(|| invalid("missing entity"))?;
    entity.generation = 2;
    entity.revision = 1;
    entity.fields.insert("amount".into(), known(WorldValue::U64(9)));
    advance(&mut snapshot)?;
    let after = inspect(&snapshot, condition("eq", 9))?;
    assert_eq!(after["evaluation"]["condition_truth"], "true");
    assert_eq!(after["evaluation"]["facts"][0]["matching_examples"][0]["generation"], 2);
    assert_ne!(before["evidence_digest"], after["evidence_digest"]);
    for variant in 0..3 {
        let mut invalid_snapshot = snapshot.clone();
        let mut entity = invalid_snapshot.graph.entities.remove(&EntityId::new(1))
            .ok_or_else(|| invalid("missing entity"))?;
        match variant {
            0 => entity.generation = 0,
            1 => entity.revision = 0,
            _ => entity.id = EntityId::NIL,
        }
        invalid_snapshot.graph.entities.insert(entity.id, entity);
        invalid_snapshot.refresh_hash();
        let invalid_identity = inspect(&invalid_snapshot, condition("eq", 9))?;
        assert_eq!(invalid_identity["evaluation"]["condition_truth"], "unknown", "case {variant}");
        assert_eq!(invalid_identity["evaluation"]["facts"][0]["unestablished_identity_records"], 1);
        assert_eq!(invalid_identity["evaluation"]["facts"][0]["sum_max"], Value::Null);
    }
    Ok(())
}

#[test]
fn canonical_kind_aliases_do_not_change_the_sum() -> Result<()> {
    let mut snapshot = world(&[7]);
    snapshot.graph.entities.get_mut(&EntityId::new(1))
        .ok_or_else(|| invalid("missing entity"))?.kind = EntityKind::Unit;
    snapshot.refresh_hash();
    let mut selected = condition("eq", 7);
    selected["kind"] = json!("unit");
    let canonical = inspect(&snapshot, selected.clone())?;
    snapshot.graph.entities.get_mut(&EntityId::new(1))
        .ok_or_else(|| invalid("missing entity"))?.kind = EntityKind::Other("unit".into());
    snapshot.refresh_hash();
    assert_eq!(inspect(&snapshot, selected)?, canonical);
    Ok(())
}

#[test]
fn native_entry_points_reject_sums_in_all_boolean_and_failure_positions() -> Result<()> {
    let snapshot = world(&[7]);
    let selected = condition("eq", 7);
    let true_branch = json!({"op":"tick_at_least","value":0});
    for nested in [
        selected.clone(),
        json!({"op":"not","arg":selected}),
        json!({"op":"all","args":[true_branch.clone(),selected.clone()]}),
        json!({"op":"any","args":[true_branch.clone(),selected]}),
    ] {
        for failure_position in [false, true] {
            let mut query = json!({"kind":"condition_evaluation","condition":nested});
            if failure_position {
                query["failure_condition"] = query["condition"].clone();
                query["condition"] = true_branch.clone();
            }
            let result = super::super::super::condition_query(
                &snapshot, &context(&snapshot),
                &json!({"schema":"dfmcp.query/1","query":query}),
            );
            assert!(matches!(result, Err(error) if error.code == ErrorCode::InvalidRequest));
        }
    }
    let parsed: Condition = serde_json::from_value(condition("eq", 7))
        .map_err(|_| invalid("fixture condition"))?;
    assert!(matches!(Probe::default().evaluate(&parsed, &snapshot), Err(error)
        if error.code == ErrorCode::InvalidRequest));
    Ok(())
}

#[test]
fn source_work_authority_deadline_and_response_limits_remain_binding() -> Result<()> {
    let snapshot = world(&[1, 2, 3]);
    let input = json!({"schema":"dfmcp.query/1","query":{
        "kind":"condition_evaluation","condition":condition("eq", 6)}});
    for variant in 0..5 {
        let mut ctx = context(&snapshot);
        match variant {
            0 => ctx.grants.clear(),
            1 => ctx.cancellation_requested = true,
            2 => ctx.budget.max_entities = 2,
            3 => ctx.budget.max_wall_millis = 0,
            _ => ctx.budget.max_output_tokens = 1,
        }
        assert!(laboratory::condition_query(&snapshot, &ctx, &input).is_err(), "case {variant}");
    }
    let condition: Condition = serde_json::from_value(condition("eq", 6))
        .map_err(|_| invalid("fixture condition"))?;
    let mut probe = Probe { source_policy: SourcePolicy::LaboratoryReference, ..Probe::default() };
    let mut work = EvaluationBudget::new(60_000);
    work.used = MAX_EVALUATION_WORK - 2;
    assert!(matches!(probe.evaluate_bounded(&condition, &snapshot, &mut work), Err(error)
        if error.code == ErrorCode::BudgetExceeded));
    assert!(probe.facts.is_empty());
    Ok(())
}

#[test]
fn shape_validation_bounds_the_sum_field_and_its_membership_tree() -> Result<()> {
    let snapshot = world(&[7]);
    for field in ["".to_owned(), "x".repeat(129), "bad\0field".to_owned()] {
        let mut selected = condition("eq", 7);
        selected["field"] = json!(field);
        assert!(inspect(&snapshot, selected).is_err());
    }
    let mut selected = condition("eq", 7);
    selected["predicate"] = json!({"op":"related","entity_id":"1","generation":1,
        "relation":"performs","direction":"outgoing"});
    assert!(inspect(&snapshot, selected.clone()).is_err());
    let selected: Condition = serde_json::from_value(selected)
        .map_err(|_| invalid("fixture related condition"))?;
    let mut probe = Probe { source_policy: SourcePolicy::LaboratoryReference, ..Probe::default() };
    assert!(matches!(probe.evaluate(&selected, &snapshot), Err(error)
        if error.code == ErrorCode::InvalidRequest));
    assert!(probe.facts.is_empty());
    let mut selected = condition("eq", 7);
    selected["predicate"] = json!({"op":"all","args":vec![json!({"op":"always"});64]});
    assert!(matches!(inspect(&snapshot, selected), Err(error)
        if error.code == ErrorCode::BudgetExceeded));
    Ok(())
}

#[test]
fn retrospective_sum_monitor_keeps_real_stability_and_historical_terminal_evidence() -> Result<()> {
    let mut samples = Vec::new();
    for (index, quantity) in [2, 7, 3, 7, 8, 1].into_iter().enumerate() {
        let mut sample = world(&[quantity]);
        sample.tick = GameTick(index as u64 + 10);
        sample.cursor.sequence = index as u64 + 1;
        sample.refresh_hash();
        samples.push(sample);
    }
    let first = samples.first().ok_or_else(|| invalid("first sample"))?;
    let last = samples.last().ok_or_else(|| invalid("last sample"))?;
    let ctx = context(last);
    let definition = json!({"condition":condition("ge", 7),"deadline_tick":100,
        "poll_interval_ticks":1,"stable_observations":2});
    let mut replay = HistoricalWatchReplay::laboratory(
        &definition, first.anchor(), samples.len() as u64,
        Digest32::of_bytes(b"field-sum-interval"), &ctx,
    )?;
    for (index, sample) in samples.iter().enumerate() {
        replay.advance(index as u64 + 1, sample, &ctx)?;
    }
    let result = replay.finish(true, &ctx)?;
    assert_eq!(result["status"], "satisfied");
    assert_eq!(result["first_terminal_record"], 5);
    assert_eq!(result["records_after_terminal"], 1);
    assert_eq!(result["evaluation"]["facts"][0]["sum_min"], 8);
    assert_eq!(result["watch_registered"], false);
    assert_eq!(inspect(last, condition("ge", 7))?["evaluation"]["condition_truth"], "false");
    Ok(())
}
