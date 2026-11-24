use super::*;

fn digest_text(value: u8) -> String {
    format!("{value:02x}").repeat(32)
}

/// Canonical current-owner source fixture shared by both Agent Turn surfaces.
/// Portfolio selection is actual original StepId 3, not a zero-based row index.
pub(crate) fn fixture(portfolio: bool, inventory: bool, prepared: bool) -> Value {
    let selected = if portfolio { 3 } else { 1 };
    let ids = if portfolio { vec![1, 2, 3, 4] } else { vec![1] };
    let anchor = json!({"fortress_id":"1","epoch":0,
        "sequence":if portfolio { 9 } else { 7 }, "tick":110,"state_hash":digest_text(80)});
    let goal_anchor = |id| json!({"fortress_id":"1","epoch":0,"sequence":7,
        "tick":110,"state_hash":digest_text(90 + id as u8)});
    let goal = |id| json!({
        "scope":"selected_original_order_step","step_id":id,
        "kind":if inventory { "observed_inventory_minimum" } else { "retained_order_completion" },
        "deadline_tick":300,"poll_interval_ticks":5,"stable_samples":2,
        "anchor":goal_anchor(id),"source_digest":digest_text(100),
        "current_truth":if inventory { "true" } else { "false" },
        "proven_at_anchor":false,"receipt_linked":true,
        "terminal_failure":false,"history":{"present":true,"fenced":false},
        "historical_obligation":"Pending","original_goal_proven":false
    });
    let clock = |id| {
        let active = (prepared && id == selected).then(|| json!({
            "review_seal":digest_text(50 + id as u8),"key":format!("original-clock-{id}"),
            "plan_digest":digest_text(60 + id as u8),"state":"prepared",
            "pending":true,"unresolved":false,"before_tick":110,
            "game_ticks":5,"wall_millis":1000,
            "native":{"phase":"prepared","unpause_attempted":false,
                "current_pause_proven":false,"original_goal_proven":false}
        }));
        json!({
            "journal_id":digest_text(10 + id as u8),"head":digest_text(20 + id as u8),
            "target_digest":digest_text(30 + id as u8),
            "original_paired_seal":digest_text(40 + id as u8),
            "source":if inventory { "run/1.13" } else { "order-run/1.14" },
            "target_scope":if inventory { "selected_original_inventory_goal" } else { "selected_original_order_step" },
            "mode":"control","inspection_required":false,"review_available":false,
            "pending":active.is_some(),"unresolved":false,"may_be_running":false,
            "needs_reconciliation":active.is_some(),"high_tick":110,
            "high_source_sequence":10,"run_count":usize::from(active.is_some()),
            "runs":active.iter().cloned().collect::<Vec<_>>(),"active_run":active,
            "current_pause_proven":false,"original_goal_proven":false
        })
    };
    let field = if inventory { "inventory_clock" } else { "clock" };
    let scope = if inventory { "inventory_run" } else { "run" };
    let mut steps = vec![json!({"step_id":0,"action":{"kind":"set_labor"}})];
    steps.extend(ids.iter().map(|id| json!({"step_id":id,"key":format!("original-{id}"),
        "action":{"kind":"create_work_order","amount":1},
        "obligation":{"deadline_tick":300,"poll_interval_ticks":5,"stable_samples":2}})));
    let requirements = if inventory { vec![json!({"item_type":"BED",
        "minimum_stack_units":1,"observed_stack_units":1,"current_truth":"true"})] } else { Vec::new() };
    let mut parent = json!({
        "scope":if portfolio { "complete_original_production_portfolio" }
            else if inventory { "complete_original_labor_and_inventory_goal" }
            else { "complete_original_labor_and_order_goal" },
        "anchor":anchor,"selected_anchor":goal_anchor(selected),
        "selected_anchors":ids.iter().map(|id| json!({"step_id":id,"anchor":goal_anchor(*id)})).collect::<Vec<_>>(),
        "source_digest":digest_text(110),"workforce_source_digest":digest_text(111),
        "current_truth":if inventory { "true" } else { "false" },
        "labor":[{"unit_id":"43","labor":"CARPENTER","requested_enabled":true,
            "observed_enabled":true,"current_truth":"true"}],
        "inventory_requirements":requirements,
        "original_goal_proven":false,"restored_from_history":false
    });
    if portfolio && inventory {
        parent["parent_stability"] = json!({"status":"pending","required_samples":2,
            "accepted_samples":1,"poll_interval_ticks":5,"deadline_tick":300,
            "proven_at_anchor":false,"restored_from_history":false});
    }
    let rows = ids.iter().map(|id| {
        let mut row = json!({"step_id":id,"goal":goal(*id),"clock":null,"inventory_clock":null});
        row[field] = clock(*id);
        row
    }).collect::<Vec<_>>();
    let recovery_rows = ids.iter().map(|id| {
        let mut row = json!({"step_id":id,"goal_history_present":true,
            "clock_journal_id":null,"inventory_clock_journal_id":null});
        row[format!("{field}_journal_id")] = json!(digest_text(10 + *id as u8));
        row
    }).collect::<Vec<_>>();
    let mut recovery = json!({"clock_journal_id":null,"inventory_clock_journal_id":null});
    recovery[format!("{field}_journal_id")] = json!(digest_text(11));
    if portfolio { recovery["portfolio"] = json!(recovery_rows); }
    let mut request = json!({"kind":scope,"original_paired_seal":digest_text(40 + selected as u8),
        "game_ticks":5,"wall_millis":1000});
    if portfolio { request["order_step_id"] = json!(selected); }
    let mut view = json!({
        "ok":true,"mode":"control","anchor":anchor,"observation_tick_floor":110,
        "original_plan":{"plan_digest":digest_text(1),"expires_at_tick":300,"steps":steps},
        "frontier":{"all_applied":true,"needs_reconciliation":false,"next_ready":null},
        "original_goal_proven":false,"custody_initialization_may_be_partial":false,
        "parent_goal_observation_fenced":false,"clock_invalidates_goal":false,
        "inventory_clock_invalidates_goal":false,"clock":null,"inventory_clock":null,
        "clock_control_enabled":!inventory,"inventory_clock_control_enabled":inventory,
        "no_work_stock_goal":null,"selected_parent_goal":null,
        "current_parent_goal":parent,"selected_inventory_goal":null,"selected_order_goal":null,
        "portfolio":if portfolio { json!({"schema":"dfmcp.production-portfolio/1",
            "selected_order_step_id":selected,"orders":rows}) } else { Value::Null },
        "recovery":recovery,
        "clock_guidance":{
            "schema":SCHEMA,"authority_free":true,"anchor":anchor,
            "original_plan_digest":digest_text(1),"selected_order_step_id":selected,
            "max_game_ticks":100,"max_wall_millis":5000,
            "offers":[{"scope":scope,"order_step_id":selected,"plan_request":request,
                "state":if prepared { "review_prepared" } else { "new_interval" },
                "reason":"Prepare this separately reviewed original interval.",
                "evidence":[format!("/{field}"),"/original_plan",
                    if inventory { "/selected_inventory_goal" } else { "/selected_order_goal" },
                    if portfolio { "/portfolio" } else { "/current_parent_goal" }]}],
            "blockers":[]
        }
    });
    view[field] = clock(selected);
    view[if inventory { "selected_inventory_goal" } else { "selected_order_goal" }] = goal(selected);
    view
}

fn owner() -> Owner {
    Owner {
        inspected: true, target_bound: true,
        original_paired_seal: Some(Digest32::of_bytes(b"original paired seal")),
        pending: false, unresolved: false, run_count: 0, run_limit: 32, prepared: None,
    }
}

fn budget() -> WorkBudget {
    WorkBudget {
        max_bytes: 8 * 1024 * 1024 * 1024,
        max_wall_millis: 5000,
        max_game_ticks: 100,
        max_entities: 4096,
        max_actions: 5,
        max_output_tokens: 32768,
    }
}

#[test]
fn current_scalar_and_portfolio_offers_bind_actual_originals() {
    for portfolio in [false, true] {
        for inventory in [false, true] {
            for prepared in [false, true] {
                let view = fixture(portfolio, inventory, prepared);
                let guide = from_view(&view).expect("qualified current owner");
                assert_eq!(guide.offers.len(), 1);
                let offer = &guide.offers[0];
                assert_eq!(offer.scope, if inventory { "inventory_run" } else { "run" });
                assert_eq!(offer.state, if prepared { "review_prepared" } else { "new_interval" });
                assert_eq!(offer.order_step_id, if portfolio { 3 } else { 1 });
                assert_eq!(offer.plan_request["order_step_id"],
                    if portfolio { json!(3) } else { Value::Null });
                assert!(view["clock"]["review_available"] != true);
                assert!(view["inventory_clock"]["review_available"] != true);
            }
        }
    }
}

#[test]
fn interval_is_bounded_by_allowance_deadline_and_original_cadence() {
    let owner = owner();
    assert_eq!(interval(&owner, 110, 300, 5, false, budget()).unwrap(), (100, 5000, "new_interval"));
    assert_eq!(interval(&owner, 110, 300, 5, true, budget()).unwrap(), (5, 5000, "new_interval"));
    assert_eq!(interval(&owner, 298, 300, 5, true, budget()).unwrap(), (2, 5000, "new_interval"));
    assert_eq!(interval(&owner, 300, 300, 5, true, budget()).unwrap_err().0, "original_deadline_reached");
    let mut current = budget();
    current.max_game_ticks = 0;
    assert_eq!(interval(&owner, 110, 300, 5, false, current).unwrap_err().0, "game_tick_allowance_zero");
    current = budget();
    current.max_wall_millis = 0;
    assert_eq!(interval(&owner, 110, 300, 5, false, current).unwrap_err().0, "clock_wall_allowance_zero");
}

#[test]
fn retained_preparation_is_exact_even_at_history_limit() {
    for run_limit in [32, 64] {
        let mut retained = owner();
        retained.run_limit = run_limit;
        retained.run_count = run_limit;
        assert_eq!(interval(&retained, 110, 300, 5, false, budget()).unwrap_err().0, "clock_history_limit");
        retained.pending = true;
        retained.prepared = Some(Prepared { before_tick: 110, game_ticks: 70, wall_millis: 4000 });
        assert_eq!(interval(&retained, 110, 300, 5, true, budget()).unwrap(), (70, 4000, "review_prepared"));
        assert_eq!(interval(&retained, 111, 300, 5, false, budget()).unwrap_err().0,
            "retained_preparation_requires_inspection");
        let mut narrower = budget();
        narrower.max_game_ticks = 69;
        assert!(interval(&retained, 110, 300, 5, false, narrower).is_err());
        narrower = budget();
        narrower.max_wall_millis = 3999;
        assert!(interval(&retained, 110, 300, 5, false, narrower).is_err());
        retained.inspected = false;
        assert_eq!(interval(&retained, 110, 300, 5, false, budget()).unwrap_err().0,
            "clock_reconciliation_required");
    }
}

#[test]
fn no_pending_uncertain_or_unbound_owner_becomes_a_new_interval() {
    let mut current = owner();
    current.pending = true;
    assert!(interval(&current, 110, 300, 5, false, budget()).is_err());
    current.pending = false;
    current.unresolved = true;
    assert!(interval(&current, 110, 300, 5, false, budget()).is_err());
    current.unresolved = false;
    current.target_bound = false;
    assert!(interval(&current, 110, 300, 5, false, budget()).is_err());
}

#[test]
fn decoder_rejects_stale_withdrawn_and_mismatched_current_sources() {
    let mutations: &[(&str, Value)] = &[
        ("/ok", json!(false)), ("/mode", json!("offline")),
        ("/observation_tick_floor", json!(111)),
        ("/parent_goal_observation_fenced", json!(true)),
        ("/clock_invalidates_goal", json!(true)),
        ("/custody_initialization_may_be_partial", json!(true)),
        ("/frontier/all_applied", json!(false)),
        ("/frontier/needs_reconciliation", json!(true)),
        ("/original_goal_proven", json!(true)),
        ("/selected_order_goal/history/fenced", json!(true)),
        ("/selected_order_goal/terminal_failure", json!(true)),
        ("/selected_order_goal/current_truth", json!("unknown")),
        ("/selected_order_goal/anchor/tick", json!(109)),
        ("/selected_order_goal/anchor/epoch", json!(1)),
        ("/current_parent_goal/workforce_source_digest", json!("")),
        ("/current_parent_goal/labor/0/current_truth", json!("unknown")),
        ("/current_parent_goal/restored_from_history", json!(true)),
        ("/current_parent_goal/current_truth", json!("unknown")),
        ("/current_parent_goal/current_truth", json!("true")),
        ("/current_parent_goal/scope", json!("complete_original_labor_and_inventory_goal")),
        ("/selected_order_goal/kind", json!("observed_inventory_minimum")),
        ("/clock/inspection_required", json!(true)),
        ("/clock/unresolved", json!(true)),
        ("/clock/high_tick", json!(111)),
        ("/clock_control_enabled", json!(false)),
        ("/clock_guidance/anchor/tick", json!(109)),
        ("/clock_guidance/original_plan_digest", json!(digest_text(2))),
        ("/clock_guidance/selected_order_step_id", json!(2)),
        ("/clock_guidance/authority_free", json!(false)),
        ("/clock_guidance/max_game_ticks", json!(0)),
        ("/clock_guidance/offers/0/plan_request/game_ticks", json!(191)),
        ("/clock_guidance/offers/0/plan_request/wall_millis", json!(5001)),
        ("/clock_guidance/offers/0/plan_request/original_paired_seal", json!(digest_text(2))),
    ];
    for (pointer, value) in mutations {
        let mut view = fixture(false, false, false);
        *view.pointer_mut(pointer).expect(pointer) = value.clone();
        assert!(from_view(&view).is_none(), "accepted mutation {pointer}");
    }
    let mut view = fixture(false, false, false);
    view["clock_guidance"]["offers"][0]["plan_request"]["confirmed"] = json!(true);
    assert!(from_view(&view).is_none());
    let mut view = fixture(false, false, false);
    view["clock_guidance"]["offers"][0]["plan_request"]["order_step_id"] = Value::Null;
    assert!(from_view(&view).is_none());
    let mut view = fixture(false, false, false);
    view["clock_guidance"]["restored_from_history"] = json!(true);
    assert!(from_view(&view).is_none());
}

#[test]
fn every_inactive_clock_blocks_other_original_offers() {
    for inventory in [false, true] {
        let field = if inventory { "inventory_clock" } else { "clock" };
        for pointer in ["pending", "unresolved", "inspection_required", "may_be_running"] {
            let mut view = fixture(true, inventory, false);
            view["portfolio"]["orders"][0][field][pointer] = json!(true);
            assert!(from_view(&view).is_none(), "inactive {field}.{pointer}");
        }
        let mut view = fixture(true, inventory, false);
        view["portfolio"]["orders"][0][field]["active_run"] =
            json!({"state":"prepared","before_tick":110,"game_ticks":5,"wall_millis":1000});
        assert!(from_view(&view).is_none());
        let mut view = fixture(true, inventory, false);
        view["portfolio"]["orders"][0][field]["high_tick"] = json!(111);
        assert!(from_view(&view).is_none());
        let mut view = fixture(true, inventory, false);
        view["portfolio"]["orders"][2][field]["head"] = json!(digest_text(2));
        assert!(from_view(&view).is_none(), "selected row must equal actual owner");
        let mut view = fixture(true, inventory, false);
        view["recovery"]["portfolio"][2][format!("{field}_journal_id")] = json!(digest_text(2));
        assert!(from_view(&view).is_none(), "must retain actual selected journal identity");
    }
}

#[test]
fn portfolio_selection_and_distinct_composite_source_are_preserved() {
    let mut view = fixture(true, false, false);
    assert_ne!(view["anchor"]["sequence"], view["selected_order_goal"]["anchor"]["sequence"]);
    assert!(from_view(&view).is_some(), "whole false cohort still qualifies selected order evidence");
    view["clock_guidance"]["offers"][0]["plan_request"]["order_step_id"] = json!(2);
    assert!(from_view(&view).is_none(), "original StepId is not its row ordinal");
    let mut view = fixture(true, false, false);
    view["current_parent_goal"]["selected_anchors"][2]["anchor"]["state_hash"] = json!(digest_text(2));
    assert!(from_view(&view).is_none(), "same-tick different source is not selected membership");
    let mut mismatched = fixture(true, false, false);
    mismatched["portfolio"]["orders"][2]["goal"]["current_truth"] = json!("true");
    assert!(from_view(&mismatched).is_none(), "selected row and current selected goal must agree");
    let mut scalar = fixture(false, false, false);
    scalar["selected_order_goal"]["anchor"]["sequence"] = json!(6);
    scalar["current_parent_goal"]["selected_anchor"]["sequence"] = json!(6);
    assert!(from_view(&scalar).is_none(), "scalar cursor binding remains exact");
}

#[test]
fn retired_inventory_can_finish_original_and_parent_stability() {
    let mut view = fixture(true, true, false);
    view["selected_inventory_goal"]["receipt_linked"] = json!(false);
    view["portfolio"]["orders"][2]["goal"] = view["selected_inventory_goal"].clone();
    assert!(from_view(&view).is_some());
    view["selected_inventory_goal"]["proven_at_anchor"] = json!(true);
    view["portfolio"]["orders"][2]["goal"] = view["selected_inventory_goal"].clone();
    assert!(from_view(&view).is_some(), "complete original parent stability is still pending");
    view["current_parent_goal"]["parent_stability"]["status"] = json!("expired");
    assert!(from_view(&view).is_none());
    let mut order = fixture(false, false, false);
    order["selected_order_goal"]["receipt_linked"] = json!(false);
    assert!(from_view(&order).is_none(), "retired default order is never inventory proof");
}

#[test]
fn prepared_offer_repeats_bounds_without_recovering_confirmation() {
    let original = fixture(false, false, true);
    let guide = from_view(&original).unwrap();
    assert_eq!(guide.offers[0].state, "review_prepared");
    assert_eq!(original["clock"]["review_available"], false);
    assert!(guide.offers[0].plan_request.get("paired_seal").is_none());
    for (pointer, value) in [
        ("/clock_guidance/offers/0/plan_request/game_ticks", json!(4)),
        ("/clock_guidance/offers/0/plan_request/wall_millis", json!(999)),
        ("/clock/active_run/before_tick", json!(109)),
        ("/clock/active_run/state", json!("dispatch_started")),
        ("/clock/active_run/native", json!({"phase":"running"})),
    ] {
        let mut changed = original.clone();
        *changed.pointer_mut(pointer).unwrap() = value;
        assert!(from_view(&changed).is_none(), "{pointer}");
    }
    let mut stale = original;
    stale["selected_order_goal"]["anchor"] = Value::Null;
    assert!(from_view(&stale).is_none(), "retained custody alone never emits a review offer");
}

#[test]
fn structured_blockers_survive_missing_samples_and_consumed_floors() {
    let mut view = fixture(false, false, false);
    view["observation_tick_floor"] = json!(120);
    view["selected_order_goal"]["anchor"] = Value::Null;
    view["clock_guidance"]["offers"] = json!([]);
    view["clock_guidance"]["blockers"] = json!([{
        "scope":"all","code":"fresh_goal_sample_required",
        "message":"Acquire a fresh joined goal sample at the retained source floor.",
        "next_action":"wait","order_step_id":1,
        "evidence":["/observation_tick_floor","/anchor"]
    }]);
    let guide = from_view(&view).unwrap();
    assert!(guide.offers.is_empty());
    assert_eq!(guide.blockers[0].next_action, NextAction::Wait);
    view["clock_guidance"]["blockers"][0]["code"] = json!("game_tick_allowance_zero");
    view["clock_guidance"]["blockers"][0]["next_action"] = json!("operator");
    view["clock_guidance"]["max_game_ticks"] = json!(0);
    assert_eq!(from_view(&view).unwrap().blockers[0].next_action, NextAction::Operator);
    view["clock_guidance"]["anchor"]["tick"] = json!(120);
    assert!(from_view(&view).is_none(), "a blocker cannot invent a current canonical anchor");
}

#[test]
fn recovery_or_missing_guidance_never_supplies_an_offer() {
    let mut view = fixture(false, false, false);
    let retained = view["clock_guidance"].take();
    view["recovery"]["clock_guidance"] = retained;
    assert!(from_view(&view).is_none());
    view["clock_guidance"] = view["recovery"]["clock_guidance"].clone();
    view.as_object_mut().unwrap().remove("ok");
    assert!(from_view(&view).is_none());
}
