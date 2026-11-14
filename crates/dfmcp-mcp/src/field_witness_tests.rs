use super::*;
use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, FortressId, GameTick, IntentId,
    ObservationCursor, OperationContext, RequestId, RiskTier, SessionId,
};
use dfmcp_intent::{Constraint, Intent, RequestedAction, StaticPlanner};
use dfmcp_world::{CompareOp, EntityRecord, FactSource, WorldGraph};

type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;
const UNIT: EntityId = EntityId::new(1);
const BURROW: EntityId = EntityId::new(2);

fn fact(value: Value) -> Fact {
    Fact::known(
        value,
        GameTick(10),
        FactSource::Derived("dfmcp.lab-scenario/1".into()),
        Digest32::ZERO,
    )
}

fn world() -> WorldSnapshot {
    let mut graph = WorldGraph::default();
    for (id, kind) in [(UNIT, EntityKind::Unit), (BURROW, EntityKind::Burrow)] {
        graph.entities.insert(
            id,
            EntityRecord {
                id,
                generation: 1,
                revision: 1,
                kind,
                label: format!("entity {id}"),
                fields: BTreeMap::from([("alive".into(), fact(Value::Bool(true)))]),
            },
        );
    }
    WorldSnapshot::new(
        FortressId::new(1),
        GameTick(10),
        ObservationCursor {
            epoch: 1,
            sequence: 1,
        },
        true,
        graph,
    )
}

fn labor(token: &str) -> Action {
    Action::SetLabor {
        units: vec![UNIT],
        labor: token.into(),
        enabled: true,
    }
}

fn requested(action: Action) -> RequestedAction {
    RequestedAction {
        action,
        preconditions: Vec::new(),
        postconditions: Vec::new(),
        compensation: None,
        obligation: None,
        depends_on: Vec::new(),
    }
}

fn prepare(
    snapshot: &WorldSnapshot,
    requested: RequestedAction,
) -> dfmcp_core::Result<PreparedPlan> {
    let intent = Intent {
        id: IntentId::new(u128::from(snapshot.cursor.sequence)),
        anchor: snapshot.anchor(),
        summary: "Independent unit configuration".into(),
        terminal_condition: Predicate::All(effects::default_postconditions(
            &requested.action,
            "fixture",
            snapshot.fortress_id,
        )),
        constraints: vec![Constraint::MaxRisk(RiskTier::Reversible)],
        requested_actions: vec![requested],
    };
    StaticPlanner::default().prepare_laboratory(
        snapshot,
        &intent,
        &OperationContext {
            session_id: SessionId::new(1),
            request_id: RequestId::new(1),
            anchor: snapshot.anchor(),
            budget: WorkBudget::default(),
            grants: vec![CapabilityGrant {
                capability: Capability::Plan,
                scope: CapabilityScope::default(),
                max_risk: RiskTier::ReadOnly,
                expires_at_tick: None,
                remaining_uses: None,
            }],
            cancellation_requested: false,
        },
    )
}

fn concurrent(base: &WorldSnapshot) -> dfmcp_core::Result<WorldSnapshot> {
    let mut now = base.clone();
    effects::apply_effect(&mut now, &labor("BREW"), "other-agent")?;
    now.cursor.sequence += 1;
    now.refresh_hash();
    Ok(now)
}

#[test]
fn disjoint_fields_of_one_unit_replay_and_preserve_both_effects() -> TestResult {
    let base = world();
    let old = prepare(&base, requested(labor("MINE")))?;
    let mut now = concurrent(&base)?;
    assert!(
        super::super::ReadWitness::of(&old)
            .first_change(&base, &now)
            .is_some()
    );
    let proof = FieldWitness::prove(&old, &base, &now, WorkBudget::default())?;
    let replayed = prepare(&now, requested(labor("MINE")))?;
    assert!(super::super::same_actions(&old, &replayed));
    let coarse = super::super::ReadWitness::of(&old)
        .first_change(&base, &now)
        .ok_or("coarse conflict")?;
    let certificate = proof.certificate(&old, &replayed, &coarse);
    assert_eq!(
        certificate["certificate"]["schema"],
        "dfmcp.field-witness-rebase/1"
    );
    assert_eq!(
        certificate["certificate"]["witness"]["fields"][0]["presence"],
        "missing"
    );
    assert_eq!(
        certificate["certificate"]["witness"]["fields"][0]["written"],
        true
    );
    let again = FieldWitness::prove(&old, &base, &now, WorkBudget::default())?;
    assert_eq!(certificate, again.certificate(&old, &replayed, &coarse));
    assert_eq!(certificate["certificate"]["coarse_conflict"], coarse);
    effects::apply_effect(
        &mut now,
        &replayed.steps[0].action,
        &replayed.steps[0].idempotency_key,
    )?;
    for field in ["labor.MINE", "labor.BREW"] {
        assert_eq!(
            now.graph.entities[&UNIT].fields[field].known_value(),
            Some(&Value::Bool(true))
        );
    }
    Ok(())
}

#[test]
fn burrow_assignment_and_labor_rebase_with_exact_target_identity() -> TestResult {
    let base = world();
    let action = Action::SetBurrowMembership {
        units: vec![UNIT],
        burrow: BURROW,
        assigned: true,
    };
    let old = prepare(&base, requested(action.clone()))?;
    let now = concurrent(&base)?;
    let proof = FieldWitness::prove(&old, &base, &now, WorkBudget::default())?;
    let replayed = prepare(&now, requested(action))?;
    assert!(super::super::same_actions(&old, &replayed));
    assert_eq!(
        proof.certificate(&old, &replayed, &json!({"read":"entity","entity_id":"1"}))["certificate"]
            ["witness"]["fields"][0]["field"],
        "burrow.2"
    );
    let mut recycled = now;
    recycled
        .graph
        .entities
        .get_mut(&BURROW)
        .ok_or("burrow")?
        .generation += 1;
    recycled.refresh_hash();
    assert!(FieldWitness::prove(&old, &base, &recycled, WorkBudget::default()).is_err());
    Ok(())
}

#[test]
fn field_presence_provenance_and_prior_write_are_exact_reads() -> TestResult {
    let mut base = world();
    base.graph
        .entities
        .get_mut(&UNIT)
        .ok_or("unit")?
        .fields
        .insert("labor.MINE".into(), fact(Value::Bool(false)));
    base.refresh_hash();
    let old = prepare(&base, requested(labor("MINE")))?;
    let now = concurrent(&base)?;
    FieldWitness::prove(&old, &base, &now, WorkBudget::default())?;
    let mut changes = vec![None];
    for value in [Value::Bool(true), Value::Null] {
        changes.push(Some(fact(value)));
    }
    for presence in [
        FactPresence::Absent,
        FactPresence::Unknown("not observed".into()),
        FactPresence::Redacted("private".into()),
        FactPresence::Stale(base.anchor()),
    ] {
        let mut changed = fact(Value::Bool(false));
        changed.presence = Some(presence);
        changes.push(Some(changed));
    }
    let mut changed = fact(Value::Bool(false));
    changed.source = FactSource::Derived("dfmcp.reference-effects/1".into());
    changes.push(Some(changed));
    let mut changed = fact(Value::Bool(false));
    changed.observed_at = GameTick(9);
    changes.push(Some(changed));
    let mut changed = fact(Value::Bool(false));
    changed.source_digest = Digest32::of_bytes(b"changed producer");
    changes.push(Some(changed));
    for changed in changes {
        let mut conflict = now.clone();
        let fields = &mut conflict.graph.entities.get_mut(&UNIT).ok_or("unit")?.fields;
        match changed {
            Some(value) => {
                fields.insert("labor.MINE".into(), value);
            }
            None => {
                fields.remove("labor.MINE");
            }
        }
        conflict.refresh_hash();
        assert!(FieldWitness::prove(&old, &base, &conflict, WorkBudget::default()).is_err());
    }
    Ok(())
}

#[test]
fn unavailable_presence_is_not_unchanged_false_or_absence() -> TestResult {
    let mut base = world();
    base.graph
        .entities
        .get_mut(&UNIT)
        .ok_or("unit")?
        .fields
        .insert(
            "labor.MINE".into(),
            Fact::with_presence(
                FactPresence::Unknown("unobserved".into()),
                GameTick(10),
                FactSource::Derived("dfmcp.lab-scenario/1".into()),
                Digest32::ZERO,
            ),
        );
    base.refresh_hash();
    let old = prepare(&base, requested(labor("MINE")))?;
    let now = concurrent(&base)?;
    assert!(FieldWitness::prove(&old, &base, &now, WorkBudget::default()).is_err());
    Ok(())
}

#[test]
fn every_boolean_branch_and_compensation_write_is_witnessed() -> TestResult {
    let base = world();
    let mut request = requested(labor("MINE"));
    request.preconditions.push(Predicate::Any(vec![
        Predicate::Paused(true),
        Predicate::FieldCompare {
            entity_id: UNIT,
            field: "alive".into(),
            op: CompareOp::Eq,
            value: Value::Bool(true),
        },
    ]));
    request.compensation = Some(labor("CARPENTRY"));
    let old = prepare(&base, request.clone())?;
    for field in ["alive", "labor.CARPENTRY"] {
        let mut now = concurrent(&base)?;
        now.graph
            .entities
            .get_mut(&UNIT)
            .ok_or("unit")?
            .fields
            .insert(field.into(), fact(Value::Bool(false)));
        now.refresh_hash();
        // The Any precondition still succeeds through pause; its other read
        // must nevertheless conflict. Compensation has its own write set.
        prepare(&now, request.clone())?;
        assert!(FieldWitness::prove(&old, &base, &now, WorkBudget::default()).is_err());
    }
    Ok(())
}

#[test]
fn entity_replacement_kind_revision_and_lineage_changes_refuse() -> TestResult {
    let base = world();
    let old = prepare(&base, requested(labor("MINE")))?;
    for change in 0..8 {
        let mut now = concurrent(&base)?;
        match change {
            0 => {
                now.graph.entities.remove(&UNIT);
            }
            1 => now.graph.entities.get_mut(&UNIT).ok_or("unit")?.generation += 1,
            2 => {
                now.graph.entities.get_mut(&UNIT).ok_or("unit")?.kind =
                    EntityKind::Other("unit".into())
            }
            3 => now.graph.entities.get_mut(&UNIT).ok_or("unit")?.id = BURROW,
            4 => now.graph.entities.get_mut(&UNIT).ok_or("unit")?.revision = 0,
            5 => now.cursor.epoch += 1,
            6 => now.cursor.sequence = base.cursor.sequence,
            _ => now.tick.0 += 1,
        }
        now.refresh_hash();
        assert!(
            FieldWitness::prove(&old, &base, &now, WorkBudget::default()).is_err(),
            "case {change}"
        );
    }
    Ok(())
}

#[test]
fn model_declarations_and_misplaced_task_markers_are_domain_reads() -> TestResult {
    let base = world();
    let old = prepare(&base, requested(labor("MINE")))?;
    for marker in MODEL_FIELDS {
        let mut now = concurrent(&base)?;
        now.graph
            .entities
            .get_mut(&BURROW)
            .ok_or("burrow")?
            .fields
            .insert(marker.into(), fact(Value::Null));
        now.refresh_hash();
        assert!(
            FieldWitness::prove(&old, &base, &now, WorkBudget::default()).is_err(),
            "marker {marker}"
        );
    }
    Ok(())
}

#[test]
fn harvest_history_that_refuses_effects_cannot_certify_model_free_refinement() -> TestResult {
    let base = world();
    let old = prepare(&base, requested(labor("MINE")))?;
    // These are the implicit harvest reads, independently of the refinement's
    // MODEL_FIELDS inventory. A new producer field must not evade the proof.
    for marker in [
        effects::LABORATORY_HARVEST_MODEL_FIELD,
        effects::HARVEST_SOURCE_MODEL_FIELD,
        effects::HARVEST_TASK_MODEL_FIELD,
        effects::HARVEST_RESOURCE_FIELD,
        effects::HARVEST_INITIAL_FIELD,
        effects::HARVEST_REMAINING_FIELD,
        effects::HARVEST_YIELDED_FIELD,
        effects::HARVEST_SOURCE_FIELD,
        effects::HARVEST_SOURCE_GENERATION_FIELD,
        effects::HARVEST_LEDGER_FIELD,
        effects::HARVEST_LEDGER_GENERATION_FIELD,
    ] {
        for on_edge in [false, true] {
            let mut now = concurrent(&base)?;
            let fields = if on_edge {
                &mut now
                    .graph
                    .edges
                    .entry(dfmcp_core::EdgeId::new(3))
                    .or_insert(dfmcp_world::EdgeRecord {
                        id: dfmcp_core::EdgeId::new(3),
                        revision: 1,
                        kind: dfmcp_world::EdgeKind::AssignedTo,
                        from: UNIT,
                        to: BURROW,
                        fields: BTreeMap::new(),
                    })
                    .fields
            } else {
                &mut now.graph.entities.get_mut(&BURROW).ok_or("burrow")?.fields
            };
            fields.insert(marker.into(), fact(Value::Null));
            now.refresh_hash();
            let mut effect_shadow = now.clone();
            assert!(
                effects::apply_effect(&mut effect_shadow, &labor("MINE"), "new-setter").is_err()
            );
            assert_eq!(effect_shadow, now);
            assert!(
                FieldWitness::prove(&old, &base, &now, WorkBudget::default()).is_err(),
                "harvest marker {marker}, edge {on_edge}"
            );
        }
    }
    let mut now = concurrent(&base)?;
    now.graph.entities.insert(
        EntityId::new(3),
        EntityRecord {
            id: EntityId::new(3),
            generation: 1,
            revision: 1,
            kind: EntityKind::Other(effects::HARVEST_TASK_KIND.into()),
            label: "retained harvest identity with missing history".into(),
            fields: BTreeMap::new(),
        },
    );
    now.refresh_hash();
    let mut effect_shadow = now.clone();
    assert!(effects::apply_effect(&mut effect_shadow, &labor("MINE"), "new-setter").is_err());
    assert_eq!(effect_shadow, now);
    assert!(FieldWitness::prove(&old, &base, &now, WorkBudget::default()).is_err());
    Ok(())
}

#[test]
fn unsupported_domains_actions_and_temporal_reads_remain_conservative() -> TestResult {
    let base = world();
    let original = prepare(&base, requested(labor("MINE")))?;
    let now = concurrent(&base)?;
    let mut changes = Vec::new();
    let mut changed = original.clone();
    changed.steps[0].compensation = Some(Action::Pause { paused: false });
    changes.push(changed);
    let mut changed = original.clone();
    changed.steps[0]
        .preconditions
        .push(Predicate::EntityQuantityIncrease {
            kind: EntityKind::Unit,
            filter_field: "alive".into(),
            filter_value: Value::Bool(true),
            quantity_field: None,
            baseline: 1,
            increase: 1,
            baseline_anchor: base.anchor(),
            baseline_source_digest: Digest32::of_bytes(b"retained baseline"),
            source_generation: 1,
            baseline_source_counters: [0, 0, 1],
        });
    changes.push(changed);
    let mut changed = original.clone();
    changed.steps[0].obligation = Some(dfmcp_intent::ObligationSpec {
        terminal: Predicate::Paused(true),
        failure: None,
        deadline_tick: GameTick(20),
        poll_interval_ticks: 1,
        stable_for_observations: 1,
    });
    changes.push(changed);
    for changed in changes {
        assert!(FieldWitness::prove(&changed, &base, &now, WorkBudget::default()).is_err());
    }
    Ok(())
}

#[test]
fn incomplete_work_budget_is_a_refusal() -> TestResult {
    let base = world();
    let old = prepare(&base, requested(labor("MINE")))?;
    let now = concurrent(&base)?;
    for budget in [
        WorkBudget {
            max_entities: 1,
            ..WorkBudget::default()
        },
        WorkBudget {
            max_bytes: 1,
            ..WorkBudget::default()
        },
        WorkBudget {
            max_actions: 0,
            ..WorkBudget::default()
        },
        WorkBudget {
            max_wall_millis: 0,
            ..WorkBudget::default()
        },
    ] {
        assert!(FieldWitness::prove(&old, &base, &now, budget).is_err());
    }
    Ok(())
}

fn quantity_guard(
    kind: EntityKind,
    field: &str,
    value: Value,
    quantity: Option<&str>,
) -> Predicate {
    Predicate::EntityQuantity {
        kind,
        filter_field: field.into(),
        filter_value: value,
        quantity_field: quantity.map(str::to_owned),
        op: CompareOp::Ge,
        value: 1,
    }
}

#[test]
fn quantity_refinement_binds_all_members_and_only_relevant_fields() -> TestResult {
    let mut base = world();
    for (id, selected, amount) in [(10, true, 2), (11, false, 3)] {
        let id = EntityId::new(id);
        base.graph.entities.insert(
            id,
            EntityRecord {
                id,
                generation: 1,
                revision: 1,
                kind: EntityKind::Item,
                label: "inventory".into(),
                fields: BTreeMap::from([
                    ("selected".into(), fact(Value::Bool(selected))),
                    ("amount".into(), fact(Value::U64(amount))),
                ]),
            },
        );
    }
    base.refresh_hash();
    let mut request = requested(labor("MINE"));
    request.preconditions.push(quantity_guard(
        EntityKind::Item,
        "selected",
        Value::Bool(true),
        Some("amount"),
    ));
    let old = prepare(&base, request.clone())?;
    let mut now = concurrent(&base)?;
    let nonmatch = now
        .graph
        .entities
        .get_mut(&EntityId::new(11))
        .ok_or("nonmatch")?;
    nonmatch.revision += 1;
    nonmatch
        .fields
        .insert("amount".into(), fact(Value::U64(999)));
    nonmatch.label = "an unrelated label".into();
    now.refresh_hash();
    let proof = FieldWitness::prove(&old, &base, &now, WorkBudget::default())?;
    let replayed = prepare(&now, request)?;
    assert!(super::super::same_actions(&old, &replayed));
    let certificate = proof.certificate(&old, &replayed, &json!({"read":"entity_domains"}));
    let domain = &certificate["certificate"]["witness"]["aggregate_domains"][0];
    assert_eq!(domain["coverage"], "complete_laboratory_kind_domain");
    assert_eq!(domain["members"], json!(["10", "11"]));
    assert_eq!(domain["matched_members"], json!(["10"]));
    assert_eq!(domain["total"], 2);
    assert!(domain["predicate_digest"].is_string());
    let fields = certificate["certificate"]["witness"]["fields"]
        .as_array()
        .ok_or("fields")?;
    assert!(
        fields
            .iter()
            .any(|read| read["entity_id"] == "11" && read["field"] == "selected")
    );
    assert!(
        !fields
            .iter()
            .any(|read| read["entity_id"] == "11" && read["field"] == "amount")
    );
    Ok(())
}

#[test]
fn quantity_refinement_refuses_phantoms_selector_swaps_and_reused_generations() -> TestResult {
    let mut base = world();
    let other = EntityId::new(3);
    let mut record = base.graph.entities[&UNIT].clone();
    record.id = other;
    record
        .fields
        .insert("alive".into(), fact(Value::Bool(false)));
    base.graph.entities.insert(other, record);
    base.refresh_hash();
    let mut request = requested(labor("MINE"));
    request.preconditions.push(quantity_guard(
        EntityKind::Unit,
        "alive",
        Value::Bool(true),
        None,
    ));
    let old = prepare(&base, request)?;
    for change in [
        "phantom",
        "alias_phantom",
        "removed_nonmatch",
        "swap",
        "generation",
        "unknown",
    ] {
        let mut now = concurrent(&base)?;
        match change {
            "phantom" | "alias_phantom" => {
                let mut record = now.graph.entities[&other].clone();
                record.id = EntityId::new(4);
                if change == "alias_phantom" {
                    record.kind = EntityKind::Other("unit".into());
                }
                now.graph.entities.insert(record.id, record);
            }
            "removed_nonmatch" => {
                now.graph.entities.remove(&other);
            }
            "swap" => {
                now.graph
                    .entities
                    .get_mut(&UNIT)
                    .ok_or("unit")?
                    .fields
                    .insert("alive".into(), fact(Value::Bool(false)));
                now.graph
                    .entities
                    .get_mut(&other)
                    .ok_or("other")?
                    .fields
                    .insert("alive".into(), fact(Value::Bool(true)));
            }
            "generation" => {
                now.graph
                    .entities
                    .get_mut(&other)
                    .ok_or("other")?
                    .generation += 1;
            }
            _ => {
                now.graph
                    .entities
                    .get_mut(&other)
                    .ok_or("other")?
                    .fields
                    .get_mut("alive")
                    .ok_or("alive")?
                    .presence = Some(FactPresence::Unknown("selector not established".into()));
            }
        }
        now.refresh_hash();
        assert!(
            FieldWitness::prove(&old, &base, &now, WorkBudget::default()).is_err(),
            "{change}"
        );
    }
    Ok(())
}

#[test]
fn quantity_refinement_does_not_accept_equal_totals_or_unknown_contributions() -> TestResult {
    let mut base = world();
    for (id, amount) in [(10, 2), (11, 3)] {
        let id = EntityId::new(id);
        base.graph.entities.insert(
            id,
            EntityRecord {
                id,
                generation: 1,
                revision: 1,
                kind: EntityKind::Item,
                label: "contributor".into(),
                fields: BTreeMap::from([
                    ("selected".into(), fact(Value::Bool(true))),
                    ("amount".into(), fact(Value::U64(amount))),
                ]),
            },
        );
    }
    base.refresh_hash();
    let mut request = requested(labor("MINE"));
    request.preconditions.push(quantity_guard(
        EntityKind::Item,
        "selected",
        Value::Bool(true),
        Some("amount"),
    ));
    let old = prepare(&base, request)?;
    for change in [
        "equal_total",
        "unknown",
        "wrong_type",
        "provenance",
        "removed_quantity",
    ] {
        let mut now = concurrent(&base)?;
        let record = now
            .graph
            .entities
            .get_mut(&EntityId::new(10))
            .ok_or("contributor")?;
        match change {
            "equal_total" => {
                record.fields.insert("amount".into(), fact(Value::U64(1)));
                now.graph
                    .entities
                    .get_mut(&EntityId::new(11))
                    .ok_or("other")?
                    .fields
                    .insert("amount".into(), fact(Value::U64(4)));
            }
            "unknown" => {
                record.fields.get_mut("amount").ok_or("amount")?.presence =
                    Some(FactPresence::Omitted("not observed".into()));
            }
            "wrong_type" => {
                record.fields.insert("amount".into(), fact(Value::I64(2)));
            }
            "provenance" => {
                record.fields.get_mut("amount").ok_or("amount")?.source = FactSource::Replay;
            }
            _ => {
                record.fields.remove("amount");
            }
        }
        now.refresh_hash();
        assert!(
            FieldWitness::prove(&old, &base, &now, WorkBudget::default()).is_err(),
            "{change}"
        );
    }
    Ok(())
}

fn relation_world() -> WorldSnapshot {
    let mut base = world();
    let id = EdgeId::new(30);
    base.graph.edges.insert(
        id,
        EdgeRecord {
            id,
            revision: 1,
            kind: dfmcp_world::EdgeKind::AssignedTo,
            from: UNIT,
            to: BURROW,
            fields: BTreeMap::from([("eligible".into(), fact(Value::Bool(true)))]),
        },
    );
    base.refresh_hash();
    base
}

#[test]
fn exact_relation_refinement_preserves_endpoints_fields_and_negative_reads() -> TestResult {
    let base = relation_world();
    let edge_id = EdgeId::new(30);
    let mut request = requested(labor("MINE"));
    request.preconditions.push(Predicate::All(vec![
        Predicate::EdgeExists {
            edge_id,
            kind: Some(dfmcp_world::EdgeKind::AssignedTo),
        },
        Predicate::Not(Box::new(Predicate::EdgeExists {
            edge_id: EdgeId::new(31),
            kind: None,
        })),
    ]));
    let old = prepare(&base, request.clone())?;
    let now = concurrent(&base)?;
    let proof = FieldWitness::prove(&old, &base, &now, WorkBudget::default())?;
    let replayed = prepare(&now, request)?;
    assert!(super::super::same_actions(&old, &replayed));
    let certificate = proof.certificate(&old, &replayed, &json!({"read":"entity"}));
    let edges = &certificate["certificate"]["witness"]["edges"];
    assert_eq!(edges[0]["presence"], "present");
    assert_eq!(edges[0]["edge_id"], edge_id.to_string());
    assert_eq!(edges[1]["presence"], "missing");
    assert_eq!(edges[1]["coverage"], "complete_laboratory_edge_identity");
    for change in [
        "removed",
        "kind",
        "field",
        "edge_revision",
        "endpoint_reuse",
        "dangling",
        "phantom",
    ] {
        let mut changed = concurrent(&base)?;
        match change {
            "removed" => {
                changed.graph.edges.remove(&edge_id);
            }
            "kind" => {
                changed.graph.edges.get_mut(&edge_id).ok_or("edge")?.kind =
                    dfmcp_world::EdgeKind::MemberOf;
            }
            "field" => {
                changed
                    .graph
                    .edges
                    .get_mut(&edge_id)
                    .ok_or("edge")?
                    .fields
                    .insert("eligible".into(), fact(Value::Bool(false)));
            }
            "edge_revision" => {
                changed
                    .graph
                    .edges
                    .get_mut(&edge_id)
                    .ok_or("edge")?
                    .revision += 1;
            }
            "endpoint_reuse" => {
                changed
                    .graph
                    .entities
                    .get_mut(&BURROW)
                    .ok_or("burrow")?
                    .generation += 1;
            }
            "dangling" => {
                changed.graph.entities.remove(&BURROW);
            }
            _ => {
                let mut phantom = changed.graph.edges[&edge_id].clone();
                phantom.id = EdgeId::new(31);
                changed.graph.edges.insert(phantom.id, phantom);
            }
        }
        changed.refresh_hash();
        assert!(
            FieldWitness::prove(&old, &base, &changed, WorkBudget::default()).is_err(),
            "{change}"
        );
    }
    Ok(())
}

#[test]
fn aggregate_refinement_refuses_incomplete_or_oversized_censuses() -> TestResult {
    let mut base = world();
    let mut request = requested(labor("MINE"));
    request.preconditions.push(quantity_guard(
        EntityKind::Unit,
        "alive",
        Value::Bool(true),
        None,
    ));
    let old = prepare(&base, request.clone())?;
    let now = concurrent(&base)?;
    assert!(
        FieldWitness::prove(
            &old,
            &base,
            &now,
            WorkBudget {
                max_entities: 1,
                ..WorkBudget::default()
            }
        )
        .is_err()
    );
    for id in 3..=259 {
        let mut record = base.graph.entities[&UNIT].clone();
        record.id = EntityId::new(id);
        base.graph.entities.insert(record.id, record);
    }
    base.refresh_hash();
    let old = prepare(&base, request)?;
    let now = concurrent(&base)?;
    assert!(FieldWitness::prove(&old, &base, &now, WorkBudget::default()).is_err());
    Ok(())
}

#[test]
fn quantity_refinement_keeps_unknown_and_overflow_branches_under_any() -> TestResult {
    for unavailable in ["unknown_selector", "overflow"] {
        let mut base = relation_world();
        for (id, amount) in [(10, u64::MAX), (11, 1)] {
            let id = EntityId::new(id);
            let mut selector = fact(Value::Bool(true));
            if unavailable == "unknown_selector" && id == EntityId::new(11) {
                selector.presence = Some(FactPresence::Unknown("unobserved selector".into()));
            }
            base.graph.entities.insert(
                id,
                EntityRecord {
                    id,
                    generation: 1,
                    revision: 1,
                    kind: EntityKind::Item,
                    label: "uncertain inventory".into(),
                    fields: BTreeMap::from([
                        ("selected".into(), selector),
                        ("amount".into(), fact(Value::U64(amount))),
                    ]),
                },
            );
        }
        base.refresh_hash();
        let mut request = requested(labor("MINE"));
        request.preconditions.push(Predicate::Any(vec![
            Predicate::EdgeExists {
                edge_id: EdgeId::new(30),
                kind: None,
            },
            quantity_guard(
                EntityKind::Item,
                "selected",
                Value::Bool(true),
                Some("amount"),
            ),
        ]));
        let old = prepare(&base, request)?;
        let now = concurrent(&base)?;
        let result = FieldWitness::prove(&old, &base, &now, WorkBudget::default());
        assert!(result.is_err(), "{unavailable}");
    }
    Ok(())
}
