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

#[test]
fn portfolio_clock_actions_require_an_exact_order_without_retargeting_goal_actions() -> Result<()> {
    for scope in [None, Some("goal"), Some("project"), Some("session"), Some("unknown")] {
        for step in [Some(0), Some(1), Some(4), Some(5)] {
            let commit = Action::Commit {
                seal: "review".into(),
                confirmed: true,
                scope: scope.map(str::to_owned),
                order_step_id: step,
            };
            let cancel = Action::Cancel {
                scope: scope.map(str::to_owned),
                order_step_id: step,
            };
            for (production, portfolio) in [(false, false), (false, true), (true, false), (true, true)] {
                assert!(commit.validate_order_scope(production, portfolio).is_err());
                assert!(cancel.validate_order_scope(production, portfolio).is_err());
            }
        }
    }
    for scope in ["clock", "inventory_clock"] {
        for step in [None, Some(0), Some(1), Some(4), Some(5), Some(u32::MAX)] {
            let commit = Action::Commit {
                seal: "review".into(),
                confirmed: true,
                scope: Some(scope.into()),
                order_step_id: step,
            };
            let cancel = Action::Cancel {
                scope: Some(scope.into()),
                order_step_id: step,
            };
            for action in [commit, cancel] {
                assert_eq!(
                    action.validate_order_scope(true, true).is_ok(),
                    step.is_some_and(|value| value <= 4)
                );
                assert_eq!(action.validate_order_scope(true, false).is_ok(), step.is_none());
                assert_eq!(action.validate_order_scope(false, true).is_ok(), step.is_none());
            }
        }
    }
    Action::Commit {
        seal: "goal-review".into(),
        confirmed: true,
        scope: None,
        order_step_id: None,
    }.validate_order_scope(true, true)?;
    Action::Cancel { scope: None, order_step_id: None }.validate_order_scope(true, true)?;
    Ok(())
}

#[test]
fn portfolio_clock_requests_preserve_the_original_native_request() -> Result<()> {
    let seal = Digest32::of_bytes(b"exact original order review").to_hex();
    for kind in ["run", "inventory_run"] {
        let native = json!({
            "kind":kind,
            "original_paired_seal":seal,
            "game_ticks":10,
            "wall_millis":1000,
        });
        let (scalar, selected) = ProductionClockRequest::parse(&native.to_string(), false)?;
        assert!(selected.is_none());
        assert_eq!(serde_json::from_str::<Value>(&scalar).ok(), Some(native.clone()));
        assert!(ProductionClockRequest::parse(&native.to_string(), true).is_err());
        for step in 0..=4 {
            let mut scoped = native.clone();
            scoped["order_step_id"] = json!(step);
            let (canonical, selected) = ProductionClockRequest::parse(&scoped.to_string(), true)?;
            assert_eq!(selected, Some(step));
            assert_eq!(serde_json::from_str::<Value>(&canonical).ok(), Some(native.clone()));
            if kind == "run" {
                RunRequest::parse(&canonical)?;
            } else {
                InventoryRunRequest::parse(&canonical)?;
            }
            assert!(ProductionClockRequest::parse(&scoped.to_string(), false).is_err());
        }
    }
    Ok(())
}

#[test]
fn portfolio_clock_requests_reject_ambiguous_selectors_and_extra_authority() {
    let native = json!({
        "kind":"run",
        "original_paired_seal":Digest32::of_bytes(b"exact original order review").to_hex(),
        "game_ticks":10,
        "wall_millis":1000,
        "order_step_id":1,
    });
    for selector in [Value::Null, json!(-1), json!(1.5), json!("1"), json!(5), json!(u64::MAX)] {
        let mut request = native.clone();
        request["order_step_id"] = selector;
        assert!(ProductionClockRequest::parse(&request.to_string(), true).is_err());
    }
    for field in ["confirmed", "target", "deadline_tick", "journal_id", "generation"] {
        let mut request = native.clone();
        request[field] = json!(true);
        assert!(ProductionClockRequest::parse(&request.to_string(), true).is_err(), "{field}");
    }
    let raw = native.to_string();
    for duplicate in [
        raw.replacen("\"order_step_id\":1", "\"order_step_id\":0,\"order_step_id\":1", 1),
        raw.replacen("\"kind\":\"run\"", "\"kind\":\"inventory_run\",\"kind\":\"run\"", 1),
    ] {
        assert_ne!(duplicate, raw);
        assert!(ProductionClockRequest::parse(&duplicate, true).is_err());
    }
    let mut foreign = native;
    foreign["kind"] = json!("stock_targets");
    assert!(ProductionClockRequest::parse(&foreign.to_string(), true).is_err());
    assert!(ProductionClockRequest::parse(&" ".repeat(2049), true).is_err());
}
