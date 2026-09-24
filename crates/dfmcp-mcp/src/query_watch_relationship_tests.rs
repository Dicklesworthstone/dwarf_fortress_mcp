use super::*;
use dfmcp_core::{
    CapabilityGrant, CapabilityScope, EdgeId, FortressId, GameTick, ObservationCursor, RequestId,
    WorkBudget,
};
use dfmcp_world::{Fact, WorldGraph};

fn native(value: WorldValue, tick: u64) -> Fact {
    Fact::known(
        value,
        GameTick(tick),
        FactSource::DfhackField("relationship-fixture.field".into()),
        Digest32::of_bytes(b"same-native-relationship-capture"),
    )
}
fn entity(id: u64, kind: EntityKind) -> EntityRecord {
    EntityRecord {
        id: EntityId::new(id),
        generation: 1,
        revision: 3,
        kind,
        label: String::new(),
        fields: BTreeMap::new(),
    }
}
fn edge(id: u128, from: u64, to: u64, kind: EdgeKind) -> EdgeRecord {
    EdgeRecord {
        id: EdgeId::new(id),
        revision: 3,
        from: EntityId::new(from),
        to: EntityId::new(to),
        kind,
        fields: BTreeMap::from([("relation".into(), native(WorldValue::Bool(true), 3))]),
    }
}
fn world(graph: WorldGraph, tick: u64) -> WorldSnapshot {
    WorldSnapshot::new(
        FortressId::new(19),
        GameTick(tick),
        ObservationCursor {
            epoch: 1,
            sequence: tick,
        },
        true,
        graph,
    )
}
fn fixture() -> WorldSnapshot {
    let mut graph = WorldGraph::default();
    for (id, kind) in [
        (10, EntityKind::Building),
        (11, EntityKind::Job),
        (12, EntityKind::Job),
        (20, EntityKind::Item),
        (21, EntityKind::Item),
        (30, EntityKind::Unit),
        (40, EntityKind::Item),
    ] {
        graph.entities.insert(EntityId::new(id), entity(id, kind));
    }
    for (id, field, value) in [
        (11, "ready", WorldValue::Bool(true)),
        (12, "ready", WorldValue::Bool(false)),
        (20, "stack_size", WorldValue::U64(5)),
        (21, "stack_size", WorldValue::U64(7)),
        (40, "stack_size", WorldValue::U64(1)),
    ] {
        if let Some(row) = graph.entities.get_mut(&EntityId::new(id)) {
            row.fields.insert(field.into(), native(value, 3));
        }
    }
    for e in [
        edge(1, 11, 10, EdgeKind::ContainedIn),
        edge(2, 12, 10, EdgeKind::ContainedIn),
        edge(3, 11, 20, EdgeKind::Uses),
        edge(4, 11, 20, EdgeKind::Uses),
        edge(5, 11, 21, EdgeKind::Uses),
        edge(6, 30, 11, EdgeKind::Performs),
        edge(7, 20, 40, EdgeKind::ContainedIn),
        edge(8, 21, 40, EdgeKind::ContainedIn),
    ] {
        graph.edges.insert(e.id, e);
    }
    world(graph, 3)
}
fn related(id: u64, relation: Relation, direction: Direction) -> Predicate {
    Predicate::Related {
        entity_id: id.to_string(),
        generation: 1,
        relation,
        direction,
    }
}
fn condition(predicate: Predicate, kind: Kind, comparison: Comparison, value: u64) -> Condition {
    Condition::EntityCount {
        scope: Scope::ObservedProjection,
        kind,
        predicate,
        comparison,
        value,
    }
}
fn count(
    snapshot: &WorldSnapshot,
    predicate: Predicate,
    kind: Kind,
    comparison: Comparison,
    value: u64,
) -> Result<(Truth, Probe)> {
    let mut probe = Probe::default();
    let truth = probe.evaluate(&condition(predicate, kind, comparison, value), snapshot)?;
    Ok((truth, probe))
}
fn context(snapshot: &WorldSnapshot) -> OperationContext {
    OperationContext {
        session_id: SessionId::new(9_019_031),
        request_id: RequestId::new(1),
        anchor: snapshot.anchor(),
        budget: WorkBudget {
            max_entities: 1000,
            max_bytes: 1024 * 1024,
            max_output_tokens: 65536,
            max_wall_millis: 60_000,
            max_game_ticks: 1000,
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
fn call(store: &Mutex<Store>, snapshot: &WorldSnapshot, input: Value) -> Result<Value> {
    let raw = execute_in(store, snapshot, &context(snapshot), &input, |value| {
        Ok(value.to_string())
    })?;
    serde_json::from_str(&raw).map_err(|_| invalid("test JSON"))
}
fn watch(predicate: Predicate) -> Value {
    json!({"schema":"dfmcp.query/1","query":{"kind":"watch","key":"workshop",
        "condition":condition(predicate,Kind::Job,Comparison::Ge,1),
        "deadline_tick":100,"poll_interval_ticks":1,"stable_observations":2}})
}

#[test]
fn workshop_membership_composes_with_field_filters() -> Result<()> {
    let s = fixture();
    let selected = Predicate::All {
        args: vec![
            related(10, Relation::ContainedIn, Direction::Incoming),
            Predicate::Field {
                field: "ready".into(),
                comparison: Comparison::Eq,
                value: Literal::Bool(true),
            },
        ],
    };
    let (truth, probe) = count(&s, selected, Kind::Job, Comparison::Eq, 1)?;
    assert_eq!(truth, Truth::True);
    assert_eq!(probe.facts[0]["matched_min"], 1);
    assert_eq!(probe.facts[0]["matched_max"], 1);
    assert_eq!(
        probe.facts[0]["relationship_selection"]["references"][0]["indexed_endpoints"],
        2
    );
    assert_eq!(probe.facts[0]["complete_world_count_proven"], false);
    assert!(!probe.invalid_generation);
    Ok(())
}

#[test]
fn attachment_roles_do_not_double_count_items_or_stack_units() -> Result<()> {
    let s = fixture();
    let predicate = related(11, Relation::Uses, Direction::Outgoing);
    let (truth, probe) = count(&s, predicate.clone(), Kind::Item, Comparison::Eq, 2)?;
    assert_eq!(truth, Truth::True);
    assert_eq!(probe.facts[0]["matched_min"], 2);
    let result = quantity::query(
        &s,
        &context(&s),
        &json!({"schema":"dfmcp.query/1","query":{
        "kind":"item_quantity","scope":"observed_projection","quantity_unit":"stack_units",
        "predicate":predicate}}),
    )?;
    assert_eq!(result["quantity"]["quantity_min"], 12);
    assert_eq!(result["quantity"]["quantity_max"], 12);
    assert_eq!(result["quantity"]["matched_records_min"], 2);
    assert_eq!(
        result["quantity"]["relationship_selection"]["duplicate_edges_count_once"],
        true
    );
    assert_eq!(result["native_captures"], 0);
    assert_eq!(result["watch_registered"], false);
    assert_eq!(result["quantity"]["usable_supply_proven"], false);
    let condition = Condition::ItemQuantity {
        scope: Scope::ObservedProjection,
        quantity_unit: quantity::QuantityUnit::StackUnits,
        predicate,
        comparison: Comparison::Ge,
        value: 12,
    };
    let mut probe = Probe::default();
    assert_eq!(probe.evaluate(&condition, &s)?, Truth::True);
    Ok(())
}

#[test]
fn container_contents_and_workers_use_root_relative_direction() -> Result<()> {
    let s = fixture();
    for (root, relation, direction, kind, n) in [
        (
            40,
            Relation::ContainedIn,
            Direction::Incoming,
            Kind::Item,
            2,
        ),
        (30, Relation::Performs, Direction::Outgoing, Kind::Job, 1),
        (11, Relation::Performs, Direction::Incoming, Kind::Unit, 1),
        (30, Relation::Performs, Direction::Incoming, Kind::Job, 0),
    ] {
        assert_eq!(
            count(
                &s,
                related(root, relation, direction),
                kind,
                Comparison::Eq,
                n
            )?
            .0,
            Truth::True
        );
    }
    Ok(())
}

#[test]
fn untrusted_or_stale_edge_evidence_remains_unknown_and_is_not_disclosed() -> Result<()> {
    let base = fixture();
    for case in 0..10 {
        let mut graph = base.graph.clone();
        graph.edges.retain(|id, _| *id == EdgeId::new(3));
        let e = graph
            .edges
            .get_mut(&EdgeId::new(3))
            .ok_or_else(|| invalid("edge"))?;
        let fact = e
            .fields
            .get_mut("relation")
            .ok_or_else(|| invalid("edge fact"))?;
        match case {
            0 => fact.source = FactSource::Derived("not native".into()),
            1 => fact.source_digest = Digest32::ZERO,
            2 => fact.observed_at = GameTick(2),
            3 => fact.presence = Some(FactPresence::Redacted("secret backing value".into())),
            4 => fact.presence = Some(FactPresence::Absent),
            5 => fact.presence = Some(FactPresence::Known(WorldValue::Bool(false))),
            6 => fact.source = FactSource::Replay,
            7 => fact.source = FactSource::AgentAssertion("untrusted".into()),
            8 => e.revision = 0,
            _ => {
                let mut other = native(WorldValue::Bool(true), 3);
                other.source_digest = Digest32::of_bytes(b"different capture");
                e.fields.insert("other".into(), other);
            }
        }
        let s = world(graph, 3);
        let (truth, probe) = count(
            &s,
            related(11, Relation::Uses, Direction::Outgoing),
            Kind::Item,
            Comparison::Eq,
            0,
        )?;
        assert_eq!(truth, Truth::Unknown, "case={case}");
        assert_eq!(probe.facts[0]["matched_min"], 0);
        assert_eq!(probe.facts[0]["matched_max"], 1);
        assert!(
            !json!(probe.facts)
                .to_string()
                .contains("secret backing value")
        );
        let negated = Predicate::Not {
            arg: Box::new(related(11, Relation::Uses, Direction::Outgoing)),
        };
        assert_eq!(
            count(&s, negated, Kind::Item, Comparison::Eq, 2)?.0,
            Truth::Unknown
        );
    }
    Ok(())
}

#[test]
fn a_proven_parallel_edge_wins_over_unknown_evidence_in_either_order() -> Result<()> {
    for bad in [3, 4] {
        let mut graph = fixture().graph;
        graph
            .edges
            .retain(|id, _| *id == EdgeId::new(3) || *id == EdgeId::new(4));
        graph
            .edges
            .get_mut(&EdgeId::new(bad))
            .ok_or_else(|| invalid("edge"))?
            .fields
            .clear();
        assert_eq!(
            count(
                &world(graph, 3),
                related(11, Relation::Uses, Direction::Outgoing),
                Kind::Item,
                Comparison::Eq,
                1
            )?
            .0,
            Truth::True
        );
    }
    Ok(())
}

#[test]
fn canonical_edge_kind_aliases_have_equal_anchors_and_decisions() -> Result<()> {
    let s = fixture();
    let mut graph = s.graph.clone();
    for edge in graph.edges.values_mut() {
        if edge.kind == EdgeKind::Uses {
            edge.kind = EdgeKind::Custom("uses".into());
        }
    }
    let alias = world(graph, 3);
    assert_eq!(s.state_hash, alias.state_hash);
    let predicate = related(11, Relation::Uses, Direction::Outgoing);
    let (_, a) = count(&s, predicate.clone(), Kind::Item, Comparison::Eq, 2)?;
    let (_, b) = count(&alias, predicate, Kind::Item, Comparison::Eq, 2)?;
    assert_eq!(a.facts, b.facts);
    Ok(())
}

#[test]
fn empty_populations_and_boolean_branches_cannot_hide_missing_or_recycled_roots() -> Result<()> {
    for missing in [true, false] {
        let mut graph = WorldGraph::default();
        if !missing {
            let mut root = entity(10, EntityKind::Building);
            root.generation = 2;
            graph.entities.insert(root.id, root);
        }
        let s = world(graph, 3);
        for predicate in [
            related(10, Relation::ContainedIn, Direction::Incoming),
            Predicate::Any {
                args: vec![
                    Predicate::Always {},
                    related(10, Relation::ContainedIn, Direction::Incoming),
                ],
            },
        ] {
            let (truth, probe) = count(&s, predicate.clone(), Kind::Job, Comparison::Eq, 0)?;
            assert_eq!(truth, Truth::Unknown);
            assert_eq!(probe.invalid_generation, !missing);
            let condition = condition(predicate.clone(), Kind::Job, Comparison::Eq, 0);
            let result = inspection::query(
                &s,
                &context(&s),
                &json!({"schema":"dfmcp.query/1",
                "query":{"kind":"condition_evaluation","condition":condition}}),
            )?;
            assert_eq!(
                result["evaluation"]["status"],
                if missing {
                    "blocked_unknown"
                } else {
                    "invalidated_reference"
                }
            );
            let result = quantity::query(
                &s,
                &context(&s),
                &json!({"schema":"dfmcp.query/1","query":{
                "kind":"item_quantity","scope":"observed_projection","quantity_unit":"stack_units",
                "predicate":predicate}}),
            )?;
            assert_eq!(result["quantity"]["quantity_min"], 0);
            assert!(result["quantity"]["quantity_max"].is_null());
            assert_eq!(result["quantity"]["quantity_exact"], false);
            assert_eq!(
                result["quantity"]["relationship_selection"]["references_established"],
                false
            );
        }
    }
    Ok(())
}

#[test]
fn an_existing_empty_container_can_be_measured_without_a_world_absence_claim() -> Result<()> {
    let mut graph = WorldGraph::default();
    graph
        .entities
        .insert(EntityId::new(40), entity(40, EntityKind::Item));
    let (truth, probe) = count(
        &world(graph, 3),
        related(40, Relation::ContainedIn, Direction::Incoming),
        Kind::Item,
        Comparison::Eq,
        0,
    )?;
    assert_eq!(truth, Truth::True);
    assert_eq!(
        probe.facts[0]["relationship_selection"]["complete_world_relationships_proven"],
        false
    );
    Ok(())
}

#[test]
fn dangling_endpoints_and_malformed_root_identity_fail_closed() -> Result<()> {
    let base = fixture();
    for case in 0..4 {
        let mut graph = base.graph.clone();
        match case {
            0 => {
                graph.entities.remove(&EntityId::new(20));
            }
            1 => {
                graph
                    .entities
                    .get_mut(&EntityId::new(11))
                    .ok_or_else(|| invalid("root"))?
                    .id = EntityId::new(999);
            }
            2 => {
                graph
                    .entities
                    .get_mut(&EntityId::new(11))
                    .ok_or_else(|| invalid("root"))?
                    .revision = 0;
            }
            _ => {
                graph
                    .entities
                    .get_mut(&EntityId::new(11))
                    .ok_or_else(|| invalid("root"))?
                    .generation = 0;
            }
        }
        assert_eq!(
            count(
                &world(graph, 3),
                related(11, Relation::Uses, Direction::Outgoing),
                Kind::Item,
                Comparison::Ge,
                0
            )?
            .0,
            Truth::Unknown
        );
    }
    Ok(())
}

#[test]
fn generation_mismatch_invalidates_retained_watch_instead_of_proving_progress() -> Result<()> {
    let s = fixture();
    let store = Mutex::new(Store::default());
    let created = call(
        &store,
        &s,
        watch(related(10, Relation::ContainedIn, Direction::Incoming)),
    )?;
    assert_eq!(created["record"]["status"], "candidate");
    let mut graph = s.graph.clone();
    graph
        .entities
        .get_mut(&EntityId::new(10))
        .ok_or_else(|| invalid("root"))?
        .generation = 2;
    let next = world(graph, 4);
    let result = call(
        &store,
        &next,
        json!({"schema":"dfmcp.query/1","query":{
        "kind":"poll_watch","watch":created["record"]["watch"]}}),
    )?;
    assert_eq!(result["record"]["status"], "invalidated");
    assert_eq!(
        result["record"]["evaluation"]["facts"][0]["relationship_selection"]["generation_mismatch"],
        true
    );
    Ok(())
}

#[test]
fn join_budget_or_output_refusal_never_publishes_partial_evidence_or_a_watch() -> Result<()> {
    let s = fixture();
    let predicate = related(10, Relation::ContainedIn, Direction::Incoming);
    let mut budget = EvaluationBudget::new(60_000);
    budget.used = MAX_EVALUATION_WORK - 1;
    let mut probe = Probe::default();
    probe.facts.push(json!({"sentinel":true}));
    let before = probe.facts.clone();
    assert!(
        matches!(super::super::evaluate(&mut probe,&s,Kind::Job,&predicate,Comparison::Ge,1,&mut budget),
        Err(error) if error.code==ErrorCode::BudgetExceeded)
    );
    assert_eq!(probe.facts, before);
    assert!(!probe.invalid_generation);
    let store = Mutex::new(Store::default());
    assert!(
        execute_in(&store, &s, &context(&s), &watch(predicate), |_| Err(
            bounded("injected render failure")
        ))
        .is_err()
    );
    assert!(lock(&store)?.entries.is_empty());
    Ok(())
}

#[test]
fn related_schema_is_closed_and_references_are_canonically_validated() -> Result<()> {
    let valid = json!({"op":"related","entity_id":"10","generation":1,
        "relation":"contained_in","direction":"incoming"});
    let parsed: Predicate =
        serde_json::from_value(valid.clone()).map_err(|_| invalid("valid related"))?;
    assert_eq!(validate(&parsed, 2)?, 1);
    for (field, value) in [
        ("relation", json!("arbitrary")),
        ("direction", json!("both")),
        ("unexpected", json!(true)),
    ] {
        let mut invalid_input = valid.clone();
        invalid_input[field] = value;
        assert!(serde_json::from_value::<Predicate>(invalid_input).is_err());
    }
    for id in ["0", "01", "-1", "18446744073709551616"] {
        let mut value = valid.clone();
        value["entity_id"] = json!(id);
        let parsed: Predicate =
            serde_json::from_value(value).map_err(|_| invalid("reference input"))?;
        assert!(validate(&parsed, 2).is_err());
    }
    let mut value = valid;
    value["generation"] = json!(0);
    let parsed: Predicate =
        serde_json::from_value(value).map_err(|_| invalid("generation input"))?;
    assert!(validate(&parsed, 2).is_err());
    Ok(())
}
