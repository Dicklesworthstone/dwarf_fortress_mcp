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
