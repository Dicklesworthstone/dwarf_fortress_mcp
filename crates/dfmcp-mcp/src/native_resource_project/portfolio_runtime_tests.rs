//! The root admits only its original stock request mode and complete target set.
use super::*;

fn scalar_goal() -> Value {
    json!({
        "recipe":"wooden_bed","minimum_stack_units":5,"max_order_amount":8,
        "deadline_tick":900,"poll_interval_ticks":5,"stable_samples":2,"assign_labor":false,
    })
}

fn portfolio_goal(assign_labor: bool) -> Value {
    json!({
        "targets":[
            {"recipe":"wooden_bed","minimum_stack_units":5,"max_order_amount":8},
            {"recipe":"wooden_door","minimum_stack_units":2,"max_order_amount":3},
        ],
        "deadline_tick":900,"poll_interval_ticks":5,"stable_samples":2,
        "assign_labor":assign_labor,
    })
}

fn request(mut goal: Value, kind: &str) -> Value {
    goal["kind"] = json!(kind);
    goal["expected_observation"] =
        json!(Digest32::of_bytes(b"original worker observation").to_hex());
    goal["units"] = json!(["7"]);
    goal["labor"] = json!("CARPENTER");
    goal
}

#[test]
fn fixed_parent_routes_the_matching_stock_request_without_changing_modes()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    let scalar: ProjectStockGoal = serde_json::from_value(scalar_goal())?;
    let scalar_request = request(scalar_goal(), "stock_target");
    assert!(matches!(
        ProductionTargetRequest::parse(&scalar_request.to_string(), &scalar)?,
        ProductionTargetRequest::Single(_)
    ));
    for assigning in [false, true] {
        let goal: ProjectStockGoal = serde_json::from_value(portfolio_goal(assigning))?;
        let vector_request = request(portfolio_goal(assigning), "stock_targets");
        assert!(matches!(
            ProductionTargetRequest::parse(&vector_request.to_string(), &goal)?,
            ProductionTargetRequest::Portfolio(_)
        ));
        assert!(ProductionTargetRequest::parse(&vector_request.to_string(), &scalar).is_err());
        assert!(ProductionTargetRequest::parse(&scalar_request.to_string(), &goal).is_err());

        let mut reordered = vector_request;
        reordered["targets"]
            .as_array_mut()
            .ok_or("missing targets")?
            .reverse();
        assert!(matches!(
            ProductionTargetRequest::parse(&reordered.to_string(), &goal)?,
            ProductionTargetRequest::Portfolio(_)
        ));
    }
    Ok(())
}

#[test]
fn fixed_portfolio_rejects_dropped_targets_and_every_changed_root_term()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    let goal: ProjectStockGoal = serde_json::from_value(portfolio_goal(false))?;
    for cause in 0..11 {
        let mut changed = request(portfolio_goal(false), "stock_targets");
        match cause {
            0 => {
                assert!(
                    changed["targets"]
                        .as_array_mut()
                        .ok_or("missing targets")?
                        .pop()
                        .is_some()
                );
            }
            1 => changed["targets"][1]["recipe"] = json!("wooden_chair"),
            2 => changed["targets"][1]["minimum_stack_units"] = json!(3),
            3 => changed["targets"][1]["max_order_amount"] = json!(4),
            4 => changed["deadline_tick"] = json!(901),
            5 => changed["poll_interval_ticks"] = json!(6),
            6 => changed["stable_samples"] = json!(3),
            7 => changed["assign_labor"] = json!(true),
            8 => changed["labor"] = json!("MASON"),
            9 => changed["targets"][1] = changed["targets"][0].clone(),
            _ => changed["amount"] = json!(2),
        }
        assert!(
            ProductionTargetRequest::parse(&changed.to_string(), &goal).is_err(),
            "{cause}"
        );
    }
    let raw = request(portfolio_goal(false), "stock_targets").to_string();
    let duplicate = raw.replacen(
        "\"deadline_tick\":900",
        "\"deadline_tick\":899,\"deadline_tick\":900",
        1,
    );
    assert_ne!(duplicate, raw);
    assert!(ProductionTargetRequest::parse(&duplicate, &goal).is_err());
    Ok(())
}

#[test]
fn production_query_keeps_original_order_selection_explicit_and_stage_scoped() -> Result<()> {
    assert!(matches!(Action::query(None, None)?, Action::Inspect));
    for step in 0..=4 {
        let query = Action::query(None, Some(step))?;
        assert!(matches!(&query, Action::InspectOrder(selected) if *selected == step));
        assert!(query.is_inspection());
        query.validate_query_scope(true, true)?;
        assert!(query.validate_query_scope(false, true).is_err());
        assert!(query.validate_query_scope(true, false).is_err());
    }
    for step in [5, u32::MAX] {
        assert!(Action::query(None, Some(step)).is_err());
    }
    Ok(())
}

#[test]
fn production_queries_use_the_existing_closed_source_bound_inspector() -> Result<()> {
    for kind in [
        "production_diagnosis",
        "inventory_plan",
        "production_chain",
        "production_chain_compare",
        "aggregate",
        "entities",
        "inspect",
        "traverse",
        "dependencies",
        "search",
        "events",
        "spatial_entities",
        "related_entities",
        "ranked_entities",
    ] {
        let raw = json!({"schema":"dfmcp.query/1","query":{"kind":kind}}).to_string();
        for step in [None, Some(0), Some(4)] {
            let query = Action::query(Some(&raw), step)?;
            assert!(matches!(&query, Action::InspectProduction(_, selected) if *selected == step));
            assert!(query.is_inspection());
            query.validate_query_scope(true, true)?;
            assert!(query.validate_query_scope(false, true).is_err());
            assert_eq!(query.validate_query_scope(true, false).is_ok(), step.is_none());
        }
    }
    Ok(())
}

#[test]
fn production_query_parser_rejects_foreign_authority_and_unbounded_envelopes() {
    for raw in [
        r#"{"schema":"dfmcp.query/2","query":{"kind":"aggregate"}}"#,
        r#"{"schema":"dfmcp.query/1","query":{"kind":"run"}}"#,
        r#"{"schema":"dfmcp.query/1","query":{"kind":"aggregate"},"confirmed":true}"#,
        r#"{"schema":"dfmcp.query/1","schema":"dfmcp.query/1","query":{"kind":"aggregate"}}"#,
        r#"{"schema":"dfmcp.query/1","query":{"kind":"aggregate"},"query":{"kind":"entities"}}"#,
        r#"{"schema":"dfmcp.query/1","query":{"kind":"aggregate"},"order_step_id":0}"#,
        "{}",
        "null",
    ] {
        assert!(Action::query(Some(raw), None).is_err(), "{raw}");
    }
    let oversized = format!(
        r#"{{"schema":"dfmcp.query/1","query":{{"kind":"aggregate","padding":"{}"}}}}"#,
        "x".repeat(65_536)
    );
    assert!(Action::query(Some(&oversized), None).is_err());
}
