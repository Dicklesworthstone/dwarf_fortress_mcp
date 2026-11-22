use super::*;
use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, FortressId, GameTick, IntentId,
    ObservationCursor, OperationContext, RequestId, RiskTier, SessionId,
};
use dfmcp_intent::{Constraint, Intent, RequestedAction, StaticPlanner};
use dfmcp_world::{PredicateEvidence, PredicateTruth, WorldGraph};

type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;
const STOCKPILE: EntityId = EntityId::new(2001);

fn stockpile(bins: Option<u32>, barrels: Option<u32>) -> Action {
    Action::ConfigureStockpile {
        stockpile: STOCKPILE,
        accepts: ["FOOD".to_owned()].into_iter().collect(),
        max_bins: bins,
        max_barrels: barrels,
        max_wheelbarrows: None,
    }
}

fn order(key: &str, value: &str) -> Action {
    Action::SetStandingOrder {
        key: key.into(),
        value: value.into(),
    }
}

fn world() -> WorldSnapshot {
    let mut graph = WorldGraph::default();
    graph.entities.insert(
        STOCKPILE,
        EntityRecord {
            id: STOCKPILE,
            generation: 1,
            revision: 1,
            kind: EntityKind::Stockpile,
            label: "food stockpile".into(),
            fields: BTreeMap::new(),
        },
    );
    WorldSnapshot::new(
        FortressId::new(7),
        GameTick(10),
        ObservationCursor {
            epoch: 1,
            sequence: 1,
        },
        true,
        graph,
    )
}

fn prepare(snapshot: &WorldSnapshot, actions: &[Action]) -> dfmcp_core::Result<PreparedPlan> {
    let requested_actions = actions
        .iter()
        .map(|action| RequestedAction {
            action: action.clone(),
            preconditions: Vec::new(),
            postconditions: effects::complete_stockpile_postconditions(action).unwrap_or_else(
                || effects::default_postconditions(action, "configuration", snapshot.fortress_id),
            ),
            compensation: None,
            obligation: None,
            depends_on: Vec::new(),
        })
        .collect::<Vec<_>>();
    let terminal = requested_actions
        .iter()
        .flat_map(|requested| requested.postconditions.clone())
        .collect();
    let intent = Intent {
        id: IntentId::new(u128::from(snapshot.cursor.sequence)),
        anchor: snapshot.anchor(),
        summary: "Concurrent fortress configuration".into(),
        terminal_condition: Predicate::All(terminal),
        constraints: vec![Constraint::MaxRisk(RiskTier::Guarded)],
        requested_actions,
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

fn concurrent(base: &WorldSnapshot, action: &Action) -> dfmcp_core::Result<WorldSnapshot> {
    let mut now = base.clone();
    effects::apply_effect(&mut now, action, "other-session")?;
    now.cursor.sequence += 1;
    now.refresh_hash();
    Ok(now)
}

fn same_contract(old: &PreparedPlan, new: &PreparedPlan) -> bool {
    super::super::super::same_actions(old, new)
}

fn coarse(plan: &PreparedPlan, base: &WorldSnapshot, now: &WorldSnapshot) -> Option<Json> {
    super::super::super::ReadWitness::of(plan).first_change(base, now)
}

#[test]
fn independent_stockpile_capacities_merge_with_converged_filters_and_exact_certificate()
-> TestResult {
    let base = world();
    let actions = [stockpile(None, Some(8))];
    let old = prepare(&base, &actions)?;
    let mut now = concurrent(&base, &stockpile(Some(7), None))?;
    let conflict = coarse(&old, &base, &now).ok_or("expected coarse stockpile conflict")?;
    let proof = FieldWitness::prove(&old, &base, &now, WorkBudget::default())?;
    let replayed = prepare(&now, &actions)?;
    assert!(same_contract(&old, &replayed));
    let certificate = proof.certificate(&old, &replayed, &conflict);
    let body = &certificate["certificate"];
    assert_eq!(body["schema"], "dfmcp.configuration-merge/1");
    assert_eq!(body["from_digest"], old.digest.to_hex());
    assert_eq!(body["to_digest"], replayed.digest.to_hex());
    assert_eq!(body["base_state_hash"], base.state_hash.to_hex());
    assert_eq!(body["current_state_hash"], now.state_hash.to_hex());
    assert_eq!(body["witness"]["writes"][0]["field"], "accepts");
    assert_eq!(
        body["witness"]["writes"][0]["disposition"],
        "converged_to_requested_value"
    );
    assert_eq!(
        body["witness"]["writes"][0]["requested_value"],
        json!({"type":"text_set","value":["FOOD"]})
    );
    assert_eq!(body["witness"]["writes"][1]["field"], "max_barrels");
    assert_eq!(
        body["witness"]["writes"][1]["disposition"],
        "unchanged_base_fact"
    );
    assert_eq!(
        certificate["certificate_digest"],
        Digest32::of_bytes(body.to_string().as_bytes()).to_hex()
    );
    assert_eq!(
        certificate,
        FieldWitness::prove(&old, &base, &now, WorkBudget::default())?
            .certificate(&old, &replayed, &conflict)
    );
    effects::apply_effect(
        &mut now,
        &replayed.steps[0].action,
        &replayed.steps[0].idempotency_key,
    )?;
    let fields = &now.graph.entities[&STOCKPILE].fields;
    assert_eq!(fields["max_bins"].known_value(), Some(&Value::U64(7)));
    assert_eq!(fields["max_barrels"].known_value(), Some(&Value::U64(8)));
    now.refresh_hash();
    assert_eq!(
        PredicateEvidence::laboratory(&now)?.evaluate(&old.terminal_condition)?,
        PredicateTruth::True
    );
    Ok(())
}

#[test]
fn divergent_capacity_or_filter_writes_require_review() -> TestResult {
    let base = concurrent(&world(), &stockpile(Some(1), Some(2)))?;
    let old = prepare(&base, &[stockpile(Some(5), None)])?;
    let mut filter_change = stockpile(None, Some(4));
    if let Action::ConfigureStockpile { accepts, .. } = &mut filter_change {
        *accepts = ["METAL".to_owned()].into_iter().collect();
    }
    for other in [stockpile(Some(8), None), filter_change] {
        let now = concurrent(&base, &other)?;
        let refusal = FieldWitness::prove(&old, &base, &now, WorkBudget::default())
            .err()
            .ok_or("must conflict")?;
        assert!(refusal.contains("diverged"), "{refusal}");
    }
    // A matching other writer cannot conceal a second divergent field.
    let old = prepare(&base, &[stockpile(Some(5), Some(8))])?;
    let now = concurrent(&base, &stockpile(Some(5), Some(9)))?;
    assert!(FieldWitness::prove(&old, &base, &now, WorkBudget::default()).is_err());
    Ok(())
}

#[test]
fn a_converged_subset_does_not_hide_remaining_settings_or_structural_absence() -> TestResult {
    let base = world();
    let action = stockpile(Some(5), Some(8));
    let old = prepare(&base, &[action.clone()])?;
    let mut now = concurrent(&base, &stockpile(Some(5), None))?;
    let proof = FieldWitness::prove(&old, &base, &now, WorkBudget::default())?;
    let replayed = prepare(&now, &[action])?;
    assert!(same_contract(&old, &replayed));
    let certificate = proof.certificate(
        &old,
        &replayed,
        &coarse(&old, &base, &now).ok_or("conflict")?,
    );
    let writes = certificate["certificate"]["witness"]["writes"]
        .as_array()
        .ok_or("writes")?;
    assert_eq!(
        writes
            .iter()
            .find(|write| write["field"] == "max_bins")
            .ok_or("bins")?["disposition"],
        "converged_to_requested_value"
    );
    assert_eq!(
        writes
            .iter()
            .find(|write| write["field"] == "max_barrels")
            .ok_or("barrels")?["disposition"],
        "unchanged_base_fact"
    );
    let mut absent = now.clone();
    let mut fact = Fact::known(
        Value::U64(8),
        absent.tick,
        FactSource::Derived("dfmcp.reference-effects/1".into()),
        Digest32::ZERO,
    );
    fact.presence = Some(FactPresence::Absent);
    let record = absent
        .graph
        .entities
        .get_mut(&STOCKPILE)
        .ok_or("stockpile")?;
    record.fields.insert("max_barrels".into(), fact);
    record.revision += 1;
    absent.refresh_hash();
    // A newly present explicit absence is neither the original missing entry
    // nor evidence that the requested numeric value already holds.
    assert!(FieldWitness::prove(&old, &base, &absent, WorkBudget::default()).is_err());
    effects::apply_effect(
        &mut now,
        &replayed.steps[0].action,
        &replayed.steps[0].idempotency_key,
    )?;
    now.refresh_hash();
    assert_eq!(
        PredicateEvidence::laboratory(&now)?.evaluate(&old.terminal_condition)?,
        PredicateTruth::True
    );
    Ok(())
}

#[test]
fn distinct_standing_orders_compose_across_canonical_entity_creation() -> TestResult {
    for initial in [None, Some(order("forbid_used_ammo", "yes"))] {
        let base = match initial {
            None => world(),
            Some(action) => concurrent(&world(), &action)?,
        };
        let action = order("gather_refuse", "outside");
        let old = prepare(&base, &[action.clone()])?;
        let mut now = concurrent(&base, &order("gather_bodies", "yes"))?;
        let conflict = coarse(&old, &base, &now).ok_or("expected settings conflict")?;
        let proof = FieldWitness::prove(&old, &base, &now, WorkBudget::default())?;
        let replayed = prepare(&now, &[action])?;
        assert!(same_contract(&old, &replayed));
        let certificate = proof.certificate(&old, &replayed, &conflict);
        let settings = effects::fortress_settings_entity_id(base.fortress_id);
        assert_eq!(
            certificate["certificate"]["witness"]["identities"][0]["disposition"],
            if base.graph.entities.contains_key(&settings) {
                "retained_identity"
            } else {
                "reuse_concurrently_created_canonical_settings"
            }
        );
        effects::apply_effect(
            &mut now,
            &replayed.steps[0].action,
            &replayed.steps[0].idempotency_key,
        )?;
        let fields = &now.graph.entities[&settings].fields;
        assert_eq!(
            fields["standing_order.gather_refuse"].known_value(),
            Some(&Value::Text("outside".into()))
        );
        assert_eq!(
            fields["standing_order.gather_bodies"].known_value(),
            Some(&Value::Text("yes".into()))
        );
        if base.graph.entities.contains_key(&settings) {
            assert_eq!(
                fields["standing_order.forbid_used_ammo"],
                base.graph.entities[&settings].fields["standing_order.forbid_used_ammo"]
            );
        }
    }
    Ok(())
}

#[test]
fn merge_refuses_unavailable_unqualified_future_and_noncanonical_write_facts() -> TestResult {
    let base = concurrent(&world(), &stockpile(Some(1), Some(2)))?;
    let old = prepare(&base, &[stockpile(Some(5), None)])?;
    let now = concurrent(&base, &stockpile(Some(5), Some(4)))?;
    let mut changes = Vec::new();
    for presence in [
        FactPresence::Unknown("not observed".into()),
        FactPresence::Absent,
        FactPresence::Known(Value::U64(6)),
    ] {
        let mut changed = now.clone();
        changed
            .graph
            .entities
            .get_mut(&STOCKPILE)
            .ok_or("stockpile")?
            .fields
            .get_mut("max_bins")
            .ok_or("bins")?
            .presence = Some(presence);
        changes.push(changed);
    }
    let mut changed = now.clone();
    changed
        .graph
        .entities
        .get_mut(&STOCKPILE)
        .ok_or("stockpile")?
        .fields
        .get_mut("max_bins")
        .ok_or("bins")?
        .source = FactSource::Derived("unqualified".into());
    changes.push(changed);
    let mut changed = now.clone();
    changed
        .graph
        .entities
        .get_mut(&STOCKPILE)
        .ok_or("stockpile")?
        .fields
        .get_mut("max_bins")
        .ok_or("bins")?
        .source_digest = Digest32::of_bytes(b"unqualified source digest");
    changes.push(changed);
    let mut changed = now.clone();
    changed
        .graph
        .entities
        .get_mut(&STOCKPILE)
        .ok_or("stockpile")?
        .fields
        .get_mut("max_bins")
        .ok_or("bins")?
        .observed_at = GameTick(now.tick.0 + 1);
    changes.push(changed);
    let mut changed = now.clone();
    let filters = changed
        .graph
        .entities
        .get_mut(&STOCKPILE)
        .ok_or("stockpile")?
        .fields
        .get_mut("accepts")
        .ok_or("accepts")?;
    filters.value = Value::List(vec![Value::Text("FOOD".into()), Value::Text("FOOD".into())]);
    filters.presence = Some(FactPresence::Known(filters.value.clone()));
    changes.push(changed);
    for mut changed in changes {
        changed.refresh_hash();
        assert!(FieldWitness::prove(&old, &base, &changed, WorkBudget::default()).is_err());
    }
    Ok(())
}

#[test]
fn merge_refuses_identity_replacement_and_unqualified_settings_creation() -> TestResult {
    let base = world();
    let old = prepare(&base, &[order("gather_refuse", "outside")])?;
    let now = concurrent(&base, &order("gather_bodies", "yes"))?;
    let settings = effects::fortress_settings_entity_id(base.fortress_id);
    let mut changes = Vec::new();
    let mut changed = now.clone();
    changed
        .graph
        .entities
        .get_mut(&settings)
        .ok_or("settings")?
        .generation = 2;
    changes.push(changed);
    let mut changed = now.clone();
    changed
        .graph
        .entities
        .get_mut(&settings)
        .ok_or("settings")?
        .kind = EntityKind::Stockpile;
    changes.push(changed);
    let mut changed = now.clone();
    let record = changed
        .graph
        .entities
        .get_mut(&settings)
        .ok_or("settings")?;
    let fact = record.fields.values().next().ok_or("setting")?.clone();
    record.fields.insert("unregistered_setting".into(), fact);
    changes.push(changed);
    for mut changed in changes {
        changed.refresh_hash();
        assert!(FieldWitness::prove(&old, &base, &changed, WorkBudget::default()).is_err());
    }
    let base = concurrent(&world(), &stockpile(Some(1), Some(2)))?;
    let old = prepare(&base, &[stockpile(Some(5), None)])?;
    let now = concurrent(&base, &stockpile(None, Some(4)))?;
    for generation in [0, 2] {
        let mut changed = now.clone();
        changed
            .graph
            .entities
            .get_mut(&STOCKPILE)
            .ok_or("stockpile")?
            .generation = generation;
        changed.refresh_hash();
        assert!(FieldWitness::prove(&old, &base, &changed, WorkBudget::default()).is_err());
    }
    let mut changed = now;
    changed
        .graph
        .entities
        .get_mut(&STOCKPILE)
        .ok_or("stockpile")?
        .revision = base.graph.entities[&STOCKPILE].revision - 1;
    changed.refresh_hash();
    assert!(FieldWitness::prove(&old, &base, &changed, WorkBudget::default()).is_err());
    Ok(())
}

#[test]
fn guards_compensations_temporal_results_and_mixed_write_contracts_stay_reviewed() -> TestResult {
    let base = world();
    let old = prepare(&base, &[stockpile(Some(5), None)])?;
    let now = concurrent(&base, &stockpile(None, Some(4)))?;
    let mut changes = Vec::new();
    let mut changed = old.clone();
    changed.steps[0].preconditions.push(Predicate::True);
    changes.push(changed);
    let mut changed = old.clone();
    changed.steps[0].compensation = Some(stockpile(Some(0), None));
    changes.push(changed);
    let mut changed = old.clone();
    changed.steps[0].obligation = Some(dfmcp_intent::ObligationSpec {
        terminal: changed.terminal_condition.clone(),
        failure: None,
        deadline_tick: GameTick(20),
        poll_interval_ticks: 1,
        stable_for_observations: 1,
    });
    changes.push(changed);
    let mut changed = old.clone();
    changed.terminal_condition = Predicate::Any(vec![changed.terminal_condition, Predicate::True]);
    changes.push(changed);
    let mut changed = old.clone();
    changed.steps[0]
        .postconditions
        .push(Predicate::Paused(true));
    changes.push(changed);
    let mut changed = old.clone();
    changed.steps.push(changed.steps[0].clone());
    changed.steps[1].action = stockpile(Some(6), None);
    changes.push(changed);
    let mut changed = old.clone();
    changed.steps.push(changed.steps[0].clone());
    changed.steps[1].action = Action::Pause { paused: false };
    changes.push(changed);
    for changed in changes {
        assert!(FieldWitness::prove(&changed, &base, &now, WorkBudget::default()).is_err());
    }
    Ok(())
}

#[test]
fn model_lineage_and_budget_changes_never_gain_a_configuration_certificate() -> TestResult {
    let base = world();
    let old = prepare(&base, &[stockpile(Some(5), None)])?;
    let now = concurrent(&base, &stockpile(None, Some(4)))?;
    let mut changes = Vec::new();
    let mut changed = now.clone();
    let record = changed
        .graph
        .entities
        .get_mut(&STOCKPILE)
        .ok_or("stockpile")?;
    let fact = record.fields.get("accepts").ok_or("accepts")?.clone();
    record
        .fields
        .insert(effects::LABORATORY_WORKFORCE_MODEL_FIELD.into(), fact);
    changes.push(changed);
    let mut changed = now.clone();
    let id = EdgeId::new(30);
    changed.graph.edges.insert(
        id,
        EdgeRecord {
            id,
            revision: 1,
            kind: dfmcp_world::EdgeKind::AssignedTo,
            from: STOCKPILE,
            to: STOCKPILE,
            fields: BTreeMap::from([(
                effects::LABORATORY_HARVEST_MODEL_FIELD.into(),
                changed.graph.entities[&STOCKPILE].fields["accepts"].clone(),
            )]),
        },
    );
    changes.push(changed);
    let mut changed = now.clone();
    changed.tick = GameTick(now.tick.0 + 1);
    changes.push(changed);
    let mut changed = now.clone();
    changed.cursor.epoch += 1;
    changes.push(changed);
    for mut changed in changes {
        changed.refresh_hash();
        assert!(FieldWitness::prove(&old, &base, &changed, WorkBudget::default()).is_err());
    }
    let mut budget = WorkBudget::default();
    budget.max_bytes = 1;
    assert!(FieldWitness::prove(&old, &base, &now, budget).is_err());
    let mut budget = WorkBudget::default();
    budget.max_actions = 0;
    assert!(FieldWitness::prove(&old, &base, &now, budget).is_err());
    Ok(())
}
