//! Dependency-preserving proposals retain exact model and source namespaces.

use std::collections::{BTreeMap, BTreeSet};

use dfmcp_core::{Digest32, EntityId, ErrorCode, FortressId, GameTick, ObservationCursor, Result};
use dfmcp_intent::{
    Action, BuildingKind, InventoryStockpile, ProductionLogisticsCompiler, ProductionPlan,
    ProductionPlanningLimits, ProductionQuota, ProductionRecipe, RequestedAction,
    WorkOrderCondition, effects,
};
use dfmcp_world::{EntityKind, EntityRecord, Fact, FactSource, Value, WorldGraph, WorldSnapshot};

fn quota(token: &str, minimum_stock: u32) -> ProductionQuota {
    ProductionQuota {
        item_token: token.to_owned(),
        minimum_stock,
    }
}

fn recipe(output: &str, output_batch_size: u32, inputs: &[(&str, u32)]) -> ProductionRecipe {
    ProductionRecipe {
        output_token: output.to_owned(),
        output_batch_size,
        input_tokens: inputs
            .iter()
            .map(|(token, n)| ((*token).to_owned(), *n))
            .collect(),
        workshop: BuildingKind::Workshop("declared".to_owned()),
        // Deliberately shared: a job label is not a unique order identity.
        job_token: "DECLARED_JOB".to_owned(),
    }
}

fn bindings(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(a, b)| ((*a).to_owned(), (*b).to_owned()))
        .collect()
}

fn chain() -> Result<ProductionPlan> {
    let mut compiler = ProductionLogisticsCompiler::without_recipes();
    compiler.register_recipe(recipe("A", 1, &[("C", 2), ("RAW", 1)]));
    compiler.register_recipe(recipe("B", 1, &[("C", 1)]));
    compiler.register_recipe(recipe("C", 3, &[("RAW", 2)]));
    let mut stock = InventoryStockpile::new();
    stock.set_stock("RAW", 8);
    compiler.plan_quotas(
        &[quota("A", 2), quota("B", 2), quota("C", 1)],
        &stock,
        ProductionPlanningLimits::default(),
    )
}

fn name(action: &RequestedAction) -> &str {
    match &action.action {
        Action::CreateWorkOrder { name, .. } => name,
        _ => "unexpected action",
    }
}

fn conditions(action: &RequestedAction) -> &[WorkOrderCondition] {
    match &action.action {
        Action::CreateWorkOrder { conditions, .. } => conditions,
        _ => &[],
    }
}

fn namespace() -> Digest32 {
    Digest32::of_bytes(b"one exact frozen production pursuit")
}

#[test]
fn joint_chain_preserves_shared_supplier_edges_and_per_batch_inputs() -> Result<()> {
    let report = chain()?;
    let steps = report.steps().to_vec();
    let actions = report.into_requested_actions(
        namespace(),
        &bindings(&[("RAW", "RAW_MATERIAL"), ("C", "COMPONENT_MATERIAL")]),
    )?;
    assert_eq!(actions.len(), 3);
    assert_eq!(steps[0].output_token, "C");
    assert_eq!(steps[0].batches, 3);
    assert_eq!(actions[0].depends_on, Vec::<u32>::new());
    assert!(
        conditions(&actions[0]).contains(&WorkOrderCondition::MaterialAvailable {
            material_token: "RAW_MATERIAL".to_owned(),
            minimum: 2, // Six total inputs must not become a per-batch minimum.
        })
    );
    let unique: BTreeSet<_> = actions.iter().map(name).collect();
    assert_eq!(
        unique.len(),
        3,
        "same job labels must not alias order names"
    );
    for (index, action) in actions.iter().enumerate() {
        let step = &steps[index];
        assert_eq!(
            action.depends_on,
            step.depends_on
                .iter()
                .map(|n| *n as u32)
                .collect::<Vec<_>>()
        );
        assert!(
            conditions(action).contains(&WorkOrderCondition::ItemCountBelow {
                item_token: step.output_token.clone(),
                threshold: step.inventory_threshold,
            })
        );
        for predecessor in &step.depends_on {
            assert!(
                conditions(action).contains(&WorkOrderCondition::CompletedOrder {
                    order_name: name(&actions[*predecessor]).to_owned(),
                })
            );
        }
        assert!(name(action).len() <= 256);
        assert!(action.preconditions.is_empty());
        assert!(action.postconditions.is_empty());
        assert!(
            action.obligation.is_none(),
            "ordinary preparation must supply evidence semantics"
        );
        effects::work_order_conditions_value(conditions(action))?;
    }
    Ok(())
}

#[test]
fn normalized_equivalent_models_and_unused_bindings_keep_identical_proposals() -> Result<()> {
    let mut compiler = ProductionLogisticsCompiler::without_recipes();
    compiler.register_recipe(recipe("A", 1, &[("RAW", 1), ("RAW", 1)]));
    let mut stock = InventoryStockpile::new();
    stock.set_stock("RAW", 4);
    let a = compiler.plan_quotas(
        &[quota("A", 2), quota("A", 1)],
        &stock,
        ProductionPlanningLimits::default(),
    )?;
    compiler.register_recipe(recipe("A", 1, &[("RAW", 2)]));
    let b = compiler.plan_quotas(
        &[quota("A", 2)],
        &stock,
        ProductionPlanningLimits::default(),
    )?;
    assert_ne!(a.work_used(), b.work_used());
    assert_eq!(
        a.into_requested_actions(
            namespace(),
            &bindings(&[("RAW", "WOOD"), ("UNUSED", "OTHER")])
        )?,
        b.into_requested_actions(namespace(), &bindings(&[("RAW", "WOOD")]))?,
    );
    Ok(())
}

#[test]
fn names_bind_the_pursuit_model_and_material_mapping() -> Result<()> {
    let inputs = bindings(&[("RAW", "WOOD"), ("C", "COMPONENT")]);
    let original = chain()?.into_requested_actions(namespace(), &inputs)?;
    let retry = chain()?.into_requested_actions(namespace(), &inputs)?;
    assert_eq!(original, retry);
    let new_pursuit =
        chain()?.into_requested_actions(Digest32::of_bytes(b"new source"), &inputs)?;
    let changed_binding = chain()?.into_requested_actions(
        namespace(),
        &bindings(&[("RAW", "METAL"), ("C", "COMPONENT")]),
    )?;
    for changed in [new_pursuit, changed_binding] {
        assert!(
            original
                .iter()
                .zip(&changed)
                .all(|(a, b)| name(a) != name(b))
        );
    }
    let mut compiler = ProductionLogisticsCompiler::without_recipes();
    let mut stock = InventoryStockpile::new();
    stock.set_stock("RAW", 8);
    compiler.register_recipe(recipe("A", 1, &[("RAW", 1)]));
    let mut model_names = BTreeSet::new();
    for amount in [1, 2] {
        let model = compiler.plan_quotas(
            &[quota("A", amount)],
            &stock,
            ProductionPlanningLimits::default(),
        )?;
        let actions = model.into_requested_actions(namespace(), &inputs)?;
        model_names.insert(name(&actions[0]).to_owned());
    }
    assert_eq!(model_names.len(), 2);
    Ok(())
}

#[test]
fn missing_aliased_and_unbounded_material_bindings_refuse_as_a_whole() -> Result<()> {
    for input in [
        bindings(&[("RAW", "WOOD")]),
        bindings(&[("RAW", "WOOD"), ("C", "WOOD")]),
        bindings(&[("RAW", "WOOD\n"), ("C", "COMPONENT")]),
        bindings(&[("RAW", ""), ("C", "COMPONENT")]),
    ] {
        assert!(
            chain()?
                .into_requested_actions(namespace(), &input)
                .is_err()
        );
    }
    let input = bindings(&[("RAW", "WOOD"), ("C", "COMPONENT")]);
    assert!(
        chain()?
            .into_requested_actions(Digest32::ZERO, &input)
            .is_err()
    );
    let oversized = (0..4_097)
        .map(|n| (format!("R{n}"), format!("M{n}")))
        .collect();
    assert!(
        chain()?
            .into_requested_actions(namespace(), &oversized)
            .is_err_and(|error| error.code == ErrorCode::BudgetExceeded)
    );
    Ok(())
}

#[test]
fn condition_capacity_refuses_instead_of_dropping_the_last_input() -> Result<()> {
    let mut compiler = ProductionLogisticsCompiler::without_recipes();
    let mut stock = InventoryStockpile::new();
    let mut input = BTreeMap::new();
    let names: Vec<_> = (0..64).map(|n| format!("R{n:02}")).collect();
    for token in &names {
        stock.set_stock(token, 1);
        input.insert(token.clone(), token.clone());
    }
    for count in [63, 64] {
        let recipe_inputs: Vec<_> = names[..count]
            .iter()
            .map(|token| (token.as_str(), 1))
            .collect();
        compiler.register_recipe(recipe("A", 1, &recipe_inputs));
        let plan = compiler.plan_quotas(
            &[quota("A", 1)],
            &stock,
            ProductionPlanningLimits::default(),
        )?;
        let lowered = plan.into_requested_actions(namespace(), &input);
        if count == 63 {
            assert_eq!(conditions(&lowered?[0]).len(), 64);
        } else {
            assert!(lowered.is_err_and(|error| error.code == ErrorCode::BudgetExceeded));
        }
    }
    Ok(())
}

#[test]
fn legacy_lowering_bytes_and_infeasible_refusal_are_preserved() -> Result<()> {
    let report = chain()?;
    let expected: Vec<_> = report
        .steps()
        .iter()
        .map(|step| Action::CreateWorkOrder {
            name: format!("Auto-JIT: {}", step.job_token),
            job_token: step.job_token.clone(),
            amount: step.batches,
            conditions: vec![WorkOrderCondition::ItemCountBelow {
                item_token: step.output_token.clone(),
                threshold: step.inventory_threshold,
            }],
        })
        .collect();
    assert_eq!(report.into_work_orders()?, expected);
    let mut compiler = ProductionLogisticsCompiler::without_recipes();
    compiler.register_recipe(recipe("A", 1, &[("RAW", 1)]));
    let missing = compiler.plan_quotas(
        &[quota("A", 1)],
        &InventoryStockpile::new(),
        ProductionPlanningLimits::default(),
    )?;
    assert!(
        missing
            .into_requested_actions(namespace(), &bindings(&[("RAW", "WOOD")]))
            .is_err_and(|error| error.code == ErrorCode::PreconditionsFailed)
    );
    Ok(())
}

fn fixture_fact(value: Value) -> Fact {
    Fact::known(
        value,
        GameTick(1),
        FactSource::Derived("dfmcp.reference-effects/1".to_owned()),
        Digest32::ZERO,
    )
}

#[test]
fn material_presence_cannot_release_a_consumer_before_its_exact_supplier_completes() -> Result<()> {
    let mut compiler = ProductionLogisticsCompiler::without_recipes();
    compiler.register_recipe(recipe("A", 1, &[("C", 1)]));
    compiler.register_recipe(recipe("C", 1, &[("RAW", 1)]));
    let mut stock = InventoryStockpile::new();
    stock.set_stock("RAW", 1);
    let actions = compiler
        .plan_quotas(
            &[quota("A", 1)],
            &stock,
            ProductionPlanningLimits::default(),
        )?
        .into_requested_actions(namespace(), &bindings(&[("RAW", "RAW"), ("C", "C")]))?;
    let ledger = EntityRecord {
        id: EntityId::new(1),
        generation: 1,
        revision: 1,
        kind: EntityKind::Other(effects::STOCK_LEDGER_KIND.to_owned()),
        label: "stocks".to_owned(),
        fields: BTreeMap::from([
            ("stock.item.A".to_owned(), fixture_fact(Value::U64(0))),
            ("stock.item.C".to_owned(), fixture_fact(Value::U64(0))),
            ("stock.material.RAW".to_owned(), fixture_fact(Value::U64(1))),
            ("stock.material.C".to_owned(), fixture_fact(Value::U64(1))),
            (
                effects::METABOLISM_TICKS_FIELD.to_owned(),
                fixture_fact(Value::U64(0)),
            ),
            (
                effects::STOCK_FOOD_FIELD.to_owned(),
                fixture_fact(Value::U64(0)),
            ),
            (
                effects::STOCK_DRINK_FIELD.to_owned(),
                fixture_fact(Value::U64(0)),
            ),
        ]),
    };
    let mut graph = WorldGraph::default();
    graph.entities.insert(ledger.id, ledger);
    let mut snapshot = WorldSnapshot::new(
        FortressId::new(1),
        GameTick(1),
        ObservationCursor::ORIGIN,
        false,
        graph,
    );
    // Deliberately dispatch both low-level proposals: the named condition is
    // still a real gate when a caller does not use RequestedAction scheduling.
    effects::apply_effect(&mut snapshot, &actions[0].action, "supplier")?;
    effects::apply_effect(&mut snapshot, &actions[1].action, "consumer")?;
    let supplier = effects::created_entity_id("supplier", 0);
    let consumer = effects::created_entity_id("consumer", 0);
    snapshot.tick = GameTick(51);
    effects::advance_effects(&mut snapshot, 50)?;
    assert_eq!(
        snapshot.graph.entities[&supplier].fields[effects::AMOUNT_REMAINING_FIELD].value,
        Value::U64(0)
    );
    assert_eq!(
        snapshot.graph.entities[&consumer].fields[effects::AMOUNT_REMAINING_FIELD].value,
        Value::U64(1)
    );
    assert_eq!(
        snapshot.graph.entities[&consumer].fields["work_ticks"].value,
        Value::U64(0)
    );
    snapshot.tick = GameTick(101);
    effects::advance_effects(&mut snapshot, 50)?;
    assert_eq!(
        snapshot.graph.entities[&consumer].fields[effects::AMOUNT_REMAINING_FIELD].value,
        Value::U64(0)
    );
    Ok(())
}
