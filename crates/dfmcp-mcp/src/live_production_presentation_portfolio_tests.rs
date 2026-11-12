use super::*;
use dfmcp_core::{Digest32, FortressId, GameTick, ObservationCursor};

fn fixture(assignment: bool) -> (StateAnchor, Value) {
    let anchor = StateAnchor {
        fortress_id: FortressId::new(7),
        cursor: ObservationCursor {
            epoch: 1,
            sequence: 9,
        },
        tick: GameTick(130),
        state_hash: Digest32::of_bytes(b"complete existing worker portfolio"),
    };
    let mut steps = if assignment {
        vec![json!({"step_id":0,"action":{"kind":"set_labor"},"depends_on":[]})]
    } else {
        vec![]
    };
    let mut orders = Vec::new();
    let mut anchors = Vec::new();
    for ordinal in 0..2 {
        let id = ordinal + usize::from(assignment);
        let mut child = anchor_value(anchor);
        child["sequence"] = (ordinal + 3).into();
        child["state_hash"] = Digest32::of_bytes(&[ordinal as u8]).to_hex().into();
        let timing = json!({"deadline_tick":300,"poll_interval_ticks":5,"stable_samples":2});
        steps.push(json!({"step_id":id,"action":{"kind":"create_work_order"},
            "depends_on":if assignment { json!([0]) } else { json!([]) },"obligation":timing}));
        orders.push(json!({"step_id":id,"goal":{"step_id":id,"scope":"selected_original_order_step",
            "kind":"retained_order_completion","anchor":child,"current_truth":"true","proven_at_anchor":true,
            "receipt_linked":true,"history":{"fenced":false},"deadline_tick":300,"poll_interval_ticks":5,"stable_samples":2}}));
        anchors.push(json!({"step_id":id,"anchor":child}));
    }
    let parent = json!({"scope":"complete_original_production_portfolio","anchor":anchor_value(anchor),
        "selected_anchors":anchors,"current_truth":"true","original_goal_proven":true,
        "labor":[{"unit_id":"43","labor":"MINE","requested_enabled":true,"observed_enabled":true,"current_truth":"true"}]});
    (
        anchor,
        json!({"original_plan":{"steps":steps},"portfolio":{"schema":"dfmcp.production-portfolio/1",
        "selected_order_step_id":usize::from(assignment),"orders":orders,"complete_original_goal_proven":true},
        "current_parent_goal":parent,"original_goal_proven":true}),
    )
}

#[test]
fn complete_parent_presentation_accepts_each_exact_original_layout() {
    for assignment in [false, true] {
        let (anchor, view) = fixture(assignment);
        assert!(portfolio_parent_valid(
            &view,
            &view["current_parent_goal"],
            Some(anchor)
        ));
        let session = SessionId::new((1u128 << 127) | (24u128 << 57) | 1);
        let (raw, overflow) = packet(
            "fortress.wait",
            Some(session),
            Some(anchor),
            Some("control"),
            None,
            None,
            json!({"ok":true}),
            Some(&view),
            true,
            false,
        );
        assert!(!overflow);
        let rendered: Value = serde_json::from_str(&raw).expect("packet");
        assert_eq!(
            rendered["agent_turn"]["briefing"]["original_goal_proven"],
            true
        );
        assert_eq!(
            rendered["agent_turn"]["briefing"]["current_state_proven"],
            false
        );
        let compacted = compact(&view);
        assert_eq!(compacted["original_goal_proven"], false);
        assert_eq!(
            compacted["portfolio"]["orders"][0]["step_id"],
            usize::from(assignment)
        );
    }
}

#[test]
fn shifted_partial_or_synthetic_dependency_portfolio_proof_is_withheld() {
    let (anchor, view) = fixture(false);
    for changed in [
        {
            let mut v = view.clone();
            v["portfolio"]["orders"][0]["step_id"] = 1.into();
            v
        },
        {
            let mut v = view.clone();
            v["current_parent_goal"]["selected_anchors"][0]["step_id"] = 1.into();
            v
        },
        {
            let mut v = view.clone();
            v["original_plan"]["steps"][0]["depends_on"] = json!([0]);
            v
        },
        {
            let mut v = view.clone();
            v["original_plan"]["steps"][1]["step_id"] = 0.into();
            v
        },
        {
            let mut v = view.clone();
            v["portfolio"]["orders"]
                .as_array_mut()
                .expect("orders")
                .pop();
            v
        },
        {
            let mut v = view.clone();
            v["portfolio"]["orders"][1]["goal"]["deadline_tick"] = 301.into();
            v
        },
        {
            let mut v = view.clone();
            v["current_parent_goal"]["labor"][0]["observed_enabled"] = false.into();
            v
        },
    ] {
        assert!(
            !portfolio_parent_valid(&changed, &changed["current_parent_goal"], Some(anchor)),
            "{changed}"
        );
    }
}

#[test]
fn negative_existing_worker_requirements_remain_visible_without_weakening_positive_proof() {
    for assignment in [false, true] {
        let (anchor, mut view) = fixture(assignment);
        view["original_goal_proven"] = false.into();
        view["portfolio"]["complete_original_goal_proven"] = false.into();
        view["current_parent_goal"]["original_goal_proven"] = false.into();
        for negative in ["false", "unknown"] {
            // Eligibility is part of the existing-worker prerequisite even
            // when the separately displayed enabled-labor bit is still true.
            view["current_parent_goal"]["current_truth"] = negative.into();
            assert_eq!(
                portfolio_parent_valid(&view, &view["current_parent_goal"], Some(anchor)),
                !assignment
            );
        }
        view["portfolio"]["orders"][0]["goal"]["current_truth"] = "false".into();
        view["portfolio"]["orders"][0]["goal"]["proven_at_anchor"] = false.into();
        for inconsistent in ["true", "unknown"] {
            view["current_parent_goal"]["current_truth"] = inconsistent.into();
            assert!(!portfolio_parent_valid(
                &view,
                &view["current_parent_goal"],
                Some(anchor)
            ));
        }
    }
}

fn stock_targets_fixture() -> (StateAnchor, Value) {
    stock_targets_fixture_with_assignment(false)
}

fn stock_targets_fixture_with_assignment(assignment: bool) -> (StateAnchor, Value) {
    let (anchor, mut view) = fixture(assignment);
    let offset = u32::from(assignment);
    let target =
        |recipe: &str, kind: &str, minimum: u64, baseline: u64, amount: u64, step: Option<u32>| {
            json!({
                "schema":"dfmcp.stock-target/1",
                "status":if amount == 0 {"already_satisfied"}else{"planned"},
                "recipe":recipe,"item_type":kind,"baseline_stack_units":baseline,
                "target_stack_units":minimum,"deficit_stack_units":amount,"amount":amount,
                "max_order_amount":100,"baseline_anchor":anchor_value(anchor),
                "source_digest":"a".repeat(64),"source_generation":30,
                "source_counters":[18,4,96],"assessment_witness":"b".repeat(64),
                "order_step_id":step,"work_required":amount > 0,"mutation_planned":amount > 0,
                "labor_assignment_requested":assignment,
                "labor_assignment_planned":assignment && amount > 0,
                "satisfied_at_anchor":amount == 0,
                "current_inventory_proven":true,"fresh_for_current_call":true
            })
        };
    view["stock_targets"] = json!({
        "schema":"dfmcp.stock-targets/1","status":"planned",
        "targets":[
            target("wooden_bed","BED",7,4,3,Some(offset)),
            target("wooden_door","DOOR",2,0,2,Some(offset + 1)),
            target("wooden_table","TABLE",1,1,0,None)
        ],
        "baseline_anchor":anchor_value(anchor),
        "source_digest":"a".repeat(64),"source_generation":30,"source_counters":[18,4,96],
        "assessment_witness":"c".repeat(64),"consumed_tick_floor":130,
        "work_required":true,"mutation_planned":true,"revalidation_failed":false,
        "worker_prerequisite_checked":!assignment,
        "labor_assignment_requested":assignment,"labor_assignment_planned":assignment,
        "current_inventory_proven":true,"fresh_for_current_call":true
    });
    view["current_parent_goal"]["inventory_requirements"] = json!([
        {"item_type":"BED","minimum_stack_units":7,"observed_stack_units":7,"current_truth":"true"},
        {"item_type":"DOOR","minimum_stack_units":2,"observed_stack_units":2,"current_truth":"true"},
        {"item_type":"TABLE","minimum_stack_units":1,"observed_stack_units":1,"current_truth":"true"}
    ]);
    view["current_parent_goal"]["parent_stability"] = json!({
        "required_samples":2,"accepted_samples":2,"poll_interval_ticks":5,
        "deadline_tick":300,"first_candidate_tick":125,"fulfilled_at_tick":130,
        "last_sample_tick":130,"status":"fulfilled","proven_at_anchor":true,
        "restored_from_history":false
    });
    (anchor, view)
}

#[test]
fn assigned_stock_targets_bind_real_order_ids_and_order_timing_to_parent_proof() {
    let (anchor, view) = stock_targets_fixture_with_assignment(true);
    assert!(portfolio_parent_valid(
        &view,
        &view["current_parent_goal"],
        Some(anchor)
    ));
    assert_eq!(
        stock_targets::stability_proven(&view, &view["current_parent_goal"]),
        Some(true)
    );
    for changed in [
        {
            let mut v = view.clone();
            v["original_plan"]["steps"]
                .as_array_mut()
                .expect("steps")
                .remove(0);
            v
        },
        {
            let mut v = view.clone();
            v["original_plan"]["steps"][0]["step_id"] = 1.into();
            v
        },
        {
            let mut v = view.clone();
            v["original_plan"]["steps"][0]["depends_on"] = json!([1]);
            v
        },
        {
            let mut v = view.clone();
            v["original_plan"]["steps"][0]["obligation"] =
                v["original_plan"]["steps"][1]["obligation"].clone();
            v
        },
        {
            let mut v = view.clone();
            v["original_plan"]["steps"][1]["depends_on"] = json!([]);
            v
        },
        {
            let mut v = view.clone();
            v["original_plan"]["steps"][2]["obligation"]["poll_interval_ticks"] = 10.into();
            v
        },
        {
            let mut v = view.clone();
            v["stock_targets"]["labor_assignment_requested"] = false.into();
            v
        },
        {
            let mut v = view.clone();
            v["stock_targets"]["labor_assignment_planned"] = false.into();
            v
        },
        {
            let mut v = view.clone();
            v["stock_targets"]["targets"][0]["order_step_id"] = 0.into();
            v
        },
        {
            let mut v = view.clone();
            v["stock_targets"]["targets"][1]["labor_assignment_planned"] = false.into();
            v
        },
        {
            let mut v = view.clone();
            v["stock_targets"]["targets"][2]["order_step_id"] = 3.into();
            v
        },
        {
            let mut v = view.clone();
            v["stock_targets"]["targets"][2]["labor_assignment_planned"] = true.into();
            v
        },
    ] {
        assert!(!portfolio_parent_valid(
            &changed,
            &changed["current_parent_goal"],
            Some(anchor)
        ));
        assert_eq!(
            stock_targets::stability_proven(&changed, &changed["current_parent_goal"]),
            None
        );
    }
}

#[test]
fn assigned_stock_targets_keep_intent_and_real_ids_through_compaction_and_overflow() {
    let (anchor, mut view) = stock_targets_fixture_with_assignment(true);
    for retained in [compact(&view), compact(&compact(&view))] {
        let targets = &retained["stock_targets"];
        assert_eq!(targets["labor_assignment_requested"], true);
        assert_eq!(targets["labor_assignment_planned"], true);
        assert_eq!(targets["targets"][0]["order_step_id"], 1);
        assert_eq!(targets["targets"][1]["order_step_id"], 2);
        assert_eq!(targets["targets"][2]["labor_assignment_requested"], true);
        assert_eq!(targets["targets"][2]["labor_assignment_planned"], false);
        assert!(targets["targets"][2]["order_step_id"].is_null());
        assert_eq!(targets["current_inventory_proven"], false);
        assert_eq!(retained["original_goal_proven"], false);
    }
    view["extra"] = json!("x".repeat(80_000));
    let (raw, overflow) = packet(
        "fortress.wait",
        Some(SessionId::new(1)),
        Some(anchor),
        Some("control"),
        None,
        None,
        view.clone(),
        Some(&view),
        true,
        false,
    );
    assert!(overflow);
    let rendered: Value = serde_json::from_str(&raw).expect("assigned overflow packet");
    assert_eq!(
        rendered["retained_reference"]["stock_targets"]["labor_assignment_requested"],
        true
    );
    assert_eq!(
        rendered["retained_reference"]["stock_targets"]["targets"][0]["order_step_id"],
        1
    );
    assert_eq!(
        rendered["agent_turn"]["active_work"]["confirmations"],
        json!([])
    );
}

#[test]
fn all_met_assignment_request_retains_opt_in_without_planning_an_assignment() {
    let (_, mut view) = stock_targets_fixture_with_assignment(true);
    view["original_plan"] = Value::Null;
    let targets = &mut view["stock_targets"];
    targets["status"] = "already_satisfied".into();
    targets["work_required"] = false.into();
    targets["mutation_planned"] = false.into();
    targets["labor_assignment_planned"] = false.into();
    targets["satisfied_at_anchor"] = true.into();
    for row in targets["targets"].as_array_mut().expect("targets") {
        row["status"] = "already_satisfied".into();
        row["baseline_stack_units"] = row["target_stack_units"].clone();
        row["deficit_stack_units"] = 0.into();
        row["amount"] = 0.into();
        row["order_step_id"] = Value::Null;
        row["work_required"] = false.into();
        row["mutation_planned"] = false.into();
        row["labor_assignment_planned"] = false.into();
        row["satisfied_at_anchor"] = true.into();
    }
    for retained in [compact(&view), compact(&compact(&view))] {
        let targets = &retained["stock_targets"];
        assert_eq!(targets["order_count"], 0);
        assert_eq!(targets["labor_assignment_requested"], true);
        assert_eq!(targets["labor_assignment_planned"], false);
        assert_eq!(targets["satisfied_at_anchor"], true);
        assert_eq!(targets["current_inventory_proven"], false);
        assert!(targets["targets"].as_array().expect("targets").iter().all(
            |row| row["labor_assignment_requested"] == true
                && row["labor_assignment_planned"] == false
                && row["order_step_id"].is_null()
        ));
    }
}

#[test]
fn stock_targets_parent_requires_every_original_minimum_and_complete_stability() {
    let (anchor, view) = stock_targets_fixture();
    assert!(portfolio_parent_valid(
        &view,
        &view["current_parent_goal"],
        Some(anchor)
    ));
    for changed in [
        {
            let mut v = view.clone();
            v["current_parent_goal"]["inventory_requirements"]
                .as_array_mut()
                .expect("findings")
                .pop();
            v
        },
        {
            let mut v = view.clone();
            v["current_parent_goal"]["inventory_requirements"][2]["minimum_stack_units"] = 2.into();
            v
        },
        {
            let mut v = view.clone();
            v["current_parent_goal"]["inventory_requirements"][2]["observed_stack_units"] =
                0.into();
            v
        },
        {
            let mut v = view.clone();
            v["current_parent_goal"]["inventory_requirements"][2]["observed_stack_units"] =
                Value::Null;
            v
        },
        {
            let mut v = view.clone();
            v["current_parent_goal"]["parent_stability"] = Value::Null;
            v
        },
        {
            let mut v = view.clone();
            v["current_parent_goal"]["parent_stability"]["accepted_samples"] = 1.into();
            v
        },
        {
            let mut v = view.clone();
            v["current_parent_goal"]["parent_stability"]["first_candidate_tick"] = 130.into();
            v
        },
        {
            let mut v = view.clone();
            v["current_parent_goal"]["parent_stability"]["last_sample_tick"] = 125.into();
            v
        },
        {
            let mut v = view.clone();
            v["current_parent_goal"]["parent_stability"]["fulfilled_at_tick"] = Value::Null;
            v
        },
        {
            let mut v = view.clone();
            v["current_parent_goal"]["parent_stability"]["fulfilled_at_tick"] = 301.into();
            v
        },
        {
            let mut v = view.clone();
            v["current_parent_goal"]["parent_stability"]["restored_from_history"] = true.into();
            v
        },
        {
            let mut v = view.clone();
            v["stock_targets"] = Value::Null;
            v
        },
    ] {
        assert!(!portfolio_parent_valid(
            &changed,
            &changed["current_parent_goal"],
            Some(anchor)
        ));
        let (raw, overflow) = packet(
            "fortress.wait",
            Some(SessionId::new(1)),
            Some(anchor),
            Some("control"),
            None,
            None,
            changed.clone(),
            Some(&changed),
            true,
            false,
        );
        assert!(!overflow);
        let rendered: Value = serde_json::from_str(&raw).expect("target packet");
        assert_eq!(
            rendered["agent_turn"]["briefing"]["original_goal_proven"],
            false
        );
        assert_eq!(rendered["original_goal_proven"], false);
        assert!(rendered["current_parent_goal"].is_null());
    }
}

#[test]
fn initially_met_stock_loss_is_visible_and_regaining_it_needs_original_stability() {
    let (anchor, mut view) = stock_targets_fixture();
    view["original_goal_proven"] = false.into();
    view["portfolio"]["complete_original_goal_proven"] = false.into();
    view["current_parent_goal"]["original_goal_proven"] = false.into();
    view["current_parent_goal"]["current_truth"] = "false".into();
    view["current_parent_goal"]["inventory_requirements"][2]["observed_stack_units"] = 0.into();
    view["current_parent_goal"]["inventory_requirements"][2]["current_truth"] = "false".into();
    view["current_parent_goal"]["parent_stability"] = json!({
        "required_samples":2,"accepted_samples":0,"poll_interval_ticks":5,
        "deadline_tick":300,"first_candidate_tick":null,"fulfilled_at_tick":null,
        "last_sample_tick":null,"status":"pending","proven_at_anchor":false,
        "restored_from_history":false
    });
    assert!(portfolio_parent_valid(
        &view,
        &view["current_parent_goal"],
        Some(anchor)
    ));
    // The two action leaves are already historically fulfilled, but the
    // initially met TABLE target must start the whole-target streak again.
    view["current_parent_goal"]["current_truth"] = "true".into();
    view["current_parent_goal"]["inventory_requirements"][2]["observed_stack_units"] = 1.into();
    view["current_parent_goal"]["inventory_requirements"][2]["current_truth"] = "true".into();
    view["current_parent_goal"]["parent_stability"]["accepted_samples"] = 1.into();
    view["current_parent_goal"]["parent_stability"]["first_candidate_tick"] = 130.into();
    view["current_parent_goal"]["parent_stability"]["last_sample_tick"] = 130.into();
    assert!(portfolio_parent_valid(
        &view,
        &view["current_parent_goal"],
        Some(anchor)
    ));
    let (raw, overflow) = packet(
        "fortress.wait",
        Some(SessionId::new(1)),
        Some(anchor),
        Some("control"),
        None,
        None,
        view.clone(),
        Some(&view),
        true,
        false,
    );
    assert!(!overflow);
    let rendered: Value = serde_json::from_str(&raw).expect("pending target packet");
    assert_eq!(rendered["original_goal_proven"], false);
    assert_eq!(
        rendered["agent_turn"]["briefing"]["current_parent_stability"]["accepted_samples"],
        1
    );
    assert_eq!(
        rendered["agent_turn"]["active_work"]["current_parent_goal"]["inventory_requirements"]
            .as_array()
            .expect("requirements")
            .len(),
        3
    );
}

#[test]
fn stock_targets_compaction_and_overflow_retain_zero_deficit_targets_without_proof() {
    let (anchor, mut view) = stock_targets_fixture();
    for retained in [compact(&view), compact(&compact(&view))] {
        let targets = &retained["stock_targets"]["targets"];
        assert_eq!(targets.as_array().expect("targets").len(), 3);
        assert_eq!(targets[0]["amount"], 3);
        assert_eq!(targets[2]["amount"], 0);
        assert_eq!(targets[2]["target_stack_units"], 1);
        assert!(targets[2]["order_step_id"].is_null());
        assert_eq!(
            retained["stock_targets"]["assessment_witness"],
            "c".repeat(64)
        );
        assert_eq!(retained["stock_targets"]["current_inventory_proven"], false);
        assert!(
            targets
                .as_array()
                .expect("targets")
                .iter()
                .all(|row| row["current_inventory_proven"] == false
                    && row["last_check_witness"].is_null())
        );
        assert!(retained["current_parent_goal"].is_null());
        assert_eq!(retained["original_goal_proven"], false);
    }
    view["extra"] = json!("x".repeat(80_000));
    let (raw, overflow) = packet(
        "fortress.wait",
        Some(SessionId::new(1)),
        Some(anchor),
        Some("control"),
        None,
        None,
        view.clone(),
        Some(&view),
        true,
        false,
    );
    assert!(overflow);
    let rendered: Value = serde_json::from_str(&raw).expect("overflow target packet");
    assert_eq!(
        rendered["retained_reference"]["stock_targets"]["targets"]
            .as_array()
            .expect("targets")
            .len(),
        3
    );
    assert_eq!(rendered["original_goal_proven"], false);
    assert_eq!(
        rendered["agent_turn"]["active_work"]["confirmations"],
        json!([])
    );
}

#[test]
fn achieved_stock_cohort_history_requires_current_truth_after_the_deadline() {
    let (_, mut view) = stock_targets_fixture();
    view["current_parent_goal"]["anchor"]["tick"] = 310.into();
    view["current_parent_goal"]["parent_stability"]["last_sample_tick"] = 310.into();
    for row in view["portfolio"]["orders"].as_array_mut().expect("orders") {
        row["goal"]["anchor"]["tick"] = 310.into();
    }
    for row in view["current_parent_goal"]["selected_anchors"]
        .as_array_mut()
        .expect("anchors")
    {
        row["anchor"]["tick"] = 310.into();
    }
    assert_eq!(
        stock_targets::stability_proven(&view, &view["current_parent_goal"]),
        Some(true)
    );
    view["current_parent_goal"]["current_truth"] = "false".into();
    view["current_parent_goal"]["original_goal_proven"] = false.into();
    view["current_parent_goal"]["parent_stability"]["proven_at_anchor"] = false.into();
    assert_eq!(
        stock_targets::stability_proven(&view, &view["current_parent_goal"]),
        Some(false)
    );
    assert_eq!(
        view["current_parent_goal"]["parent_stability"]["fulfilled_at_tick"],
        130
    );
    view["current_parent_goal"]["current_truth"] = "true".into();
    view["current_parent_goal"]["parent_stability"]["proven_at_anchor"] = true.into();
    assert_eq!(
        stock_targets::stability_proven(&view, &view["current_parent_goal"]),
        Some(true)
    );
}

fn pending_stock_targets_clock_fixture() -> (StateAnchor, Value) {
    pending_stock_targets_clock_fixture_with_assignment(false)
}

fn pending_stock_targets_clock_fixture_with_assignment(assignment: bool) -> (StateAnchor, Value) {
    let (anchor, mut view) = stock_targets_fixture_with_assignment(assignment);
    view["anchor"] = anchor_value(anchor);
    view["observation_tick_floor"] = 130.into();
    view["frontier"] = json!({"all_applied":true,"needs_reconciliation":false,
        "entries":[],"next_ready":null});
    view["original_goal_proven"] = false.into();
    view["portfolio"]["complete_original_goal_proven"] = false.into();
    view["current_parent_goal"]["original_goal_proven"] = false.into();
    view["current_parent_goal"]["parent_stability"] = json!({
        "required_samples":2,"accepted_samples":1,"poll_interval_ticks":5,
        "deadline_tick":300,"first_candidate_tick":130,"fulfilled_at_tick":null,
        "last_sample_tick":130,"status":"pending","proven_at_anchor":false,
        "restored_from_history":false
    });
    for order in view["portfolio"]["orders"].as_array_mut().expect("orders") {
        order["goal"]["kind"] = "observed_inventory_increase".into();
    }
    view["selected_inventory_goal"] = view["portfolio"]["orders"][0]["goal"].clone();
    view["selected_parent_goal"] = json!({
        "scope":"selected_original_labor_and_production_goal",
        "anchor":anchor_value(anchor),
        "selected_anchor":view["selected_inventory_goal"]["anchor"],
        "current_truth":"true","original_goal_proven":false,"parent_stability_pending":true,
        "labor":view["current_parent_goal"]["labor"],
        "inventory_requirements":view["current_parent_goal"]["inventory_requirements"]
    });
    view["inventory_clock_control_enabled"] = true.into();
    view["recovery"] = json!({"portfolio":[{"step_id":usize::from(assignment),
        "inventory_clock_journal_id":"5".repeat(64)}]});
    view["inventory_clock"] = json!({
        "source":"run/1.13","target_scope":"selected_original_inventory_goal",
        "journal_id":"5".repeat(64),"head":"6".repeat(64),"target_digest":"7".repeat(64),
        "original_paired_seal":"4".repeat(64),"mode":"control","independent_authority":true,
        "review_available":false,"pending":false,"unresolved":false,"may_be_running":false,
        "needs_reconciliation":false,"high_tick":130,"run_count":0,"active_run":null
    });
    (anchor, view)
}

fn stock_clock_packet(anchor: StateAnchor, view: &Value, ticks: u64) -> Value {
    let (raw, overflow) = packet(
        "fortress.wait",
        Some(SessionId::new(1)),
        Some(anchor),
        Some("control"),
        None,
        Some(WorkBudget {
            max_game_ticks: ticks,
            max_wall_millis: 5000,
            ..WorkBudget::default()
        }),
        view.clone(),
        Some(view),
        true,
        false,
    );
    assert!(!overflow);
    serde_json::from_str(&raw).expect("stock cohort clock packet")
}

#[test]
fn fulfilled_leaf_can_request_a_separately_authorized_clock_for_pending_stock_cohort() {
    let (anchor, view) = pending_stock_targets_clock_fixture();
    assert!(portfolio_parent_valid(
        &view,
        &view["current_parent_goal"],
        Some(anchor)
    ));
    let rendered = stock_clock_packet(anchor, &view, 20);
    assert_eq!(rendered["original_goal_proven"], false);
    let recommendation = &rendered["agent_turn"]["recommendations"][0];
    assert_eq!(recommendation["tool"], "fortress_plan");
    assert_eq!(recommendation["requires_confirmation"], false);
    let request: Value = serde_json::from_str(
        recommendation["arguments"]["request"]
            .as_str()
            .expect("clock request"),
    )
    .expect("bounded clock request");
    assert_eq!(
        request,
        json!({"kind":"inventory_run","order_step_id":0,
        "original_paired_seal":"4".repeat(64),"game_ticks":5,"wall_millis":5000})
    );
    for case in 0..7 {
        let mut changed = view.clone();
        let ticks = if case == 6 { 0 } else { 20 };
        match case {
            0 => changed["inventory_clock_control_enabled"] = false.into(),
            1 => {
                changed["current_parent_goal"]["inventory_requirements"][2]["observed_stack_units"] =
                    0.into()
            }
            2 => {
                changed["current_parent_goal"]["inventory_requirements"]
                    .as_array_mut()
                    .expect("findings")
                    .pop();
            }
            3 => changed["current_parent_goal"]["parent_stability"] = Value::Null,
            4 => changed["stock_targets"] = Value::Null,
            5 => changed["current_parent_goal"]["parent_stability"]["status"] = "expired".into(),
            _ => {}
        }
        let refused = stock_clock_packet(anchor, &changed, ticks);
        assert!(
            !refused["agent_turn"]["affordances"]
                .as_array()
                .expect("affordances")
                .iter()
                .any(|item| item["scope"] == "inventory_run"
                    && item["action"] == "fortress_plan"
                    && item["enabled"] == true),
            "case {case}"
        );
    }
}

#[test]
fn assigned_stock_parent_clock_targets_the_real_order_instead_of_the_labor_step() {
    let (anchor, view) = pending_stock_targets_clock_fixture_with_assignment(true);
    assert!(portfolio_parent_valid(
        &view,
        &view["current_parent_goal"],
        Some(anchor)
    ));
    let rendered = stock_clock_packet(anchor, &view, 20);
    let recommendation = &rendered["agent_turn"]["recommendations"][0];
    assert_eq!(recommendation["tool"], "fortress_plan");
    let request: Value = serde_json::from_str(
        recommendation["arguments"]["request"]
            .as_str()
            .expect("assigned cohort clock"),
    )
    .expect("closed clock request");
    assert_eq!(request["kind"], "inventory_run");
    assert_eq!(request["order_step_id"], 1);
    assert_eq!(request["game_ticks"], 5);
    assert_eq!(rendered["original_goal_proven"], false);
    let mut changed = view;
    changed["selected_inventory_goal"]["step_id"] = 0.into();
    let refused = stock_clock_packet(anchor, &changed, 20);
    assert!(
        !refused["agent_turn"]["affordances"]
            .as_array()
            .expect("affordances")
            .iter()
            .any(|item| item["action"] == "fortress_plan"
                && item["scope"] == "inventory_run"
                && item["enabled"] == true)
    );
}

#[test]
fn unfinished_stock_cohort_at_original_deadline_keeps_expired_diagnostics_without_a_clock() {
    let (mut anchor, mut view) = pending_stock_targets_clock_fixture();
    anchor.tick = GameTick(300);
    view["anchor"] = anchor_value(anchor);
    view["observation_tick_floor"] = 300.into();
    view["current_parent_goal"]["anchor"] = anchor_value(anchor);
    view["current_parent_goal"]["parent_stability"] = json!({
        "required_samples":2,"accepted_samples":0,"poll_interval_ticks":5,
        "deadline_tick":300,"first_candidate_tick":null,"fulfilled_at_tick":null,
        "last_sample_tick":null,"status":"expired","proven_at_anchor":false,
        "restored_from_history":false
    });
    for order in view["portfolio"]["orders"].as_array_mut().expect("orders") {
        order["goal"]["anchor"]["tick"] = 300.into();
    }
    for selected in view["current_parent_goal"]["selected_anchors"]
        .as_array_mut()
        .expect("anchors")
    {
        selected["anchor"]["tick"] = 300.into();
    }
    view["selected_inventory_goal"] = view["portfolio"]["orders"][0]["goal"].clone();
    view["selected_parent_goal"]["anchor"] = anchor_value(anchor);
    view["selected_parent_goal"]["selected_anchor"] =
        view["selected_inventory_goal"]["anchor"].clone();
    assert!(portfolio_parent_valid(
        &view,
        &view["current_parent_goal"],
        Some(anchor)
    ));
    let rendered = stock_clock_packet(anchor, &view, 20);
    assert_eq!(
        rendered["agent_turn"]["briefing"]["current_parent_stability"]["status"],
        "expired"
    );
    assert_eq!(rendered["original_goal_proven"], false);
    assert!(
        !rendered["agent_turn"]["affordances"]
            .as_array()
            .expect("affordances")
            .iter()
            .any(|item| item["action"] == "fortress_plan" && item["enabled"] == true)
    );
}

#[test]
fn prepared_stock_parent_clock_keeps_confirmation_after_current_cohort_is_withdrawn() {
    let (anchor, mut view) = pending_stock_targets_clock_fixture();
    // Preparing a run refreshes the selected all-target parent but withdraws
    // the complete cohort. Its pending diagnostic carries no completion proof.
    view["current_parent_goal"] = Value::Null;
    view["inventory_clock"]["review_available"] = true.into();
    view["inventory_clock"]["pending"] = true.into();
    view["inventory_clock"]["needs_reconciliation"] = true.into();
    view["inventory_clock"]["run_count"] = 1.into();
    view["inventory_clock"]["active_run"] = json!({
        "review_seal":"e".repeat(64),"key":"original-stock-cohort-run",
        "plan_digest":"f".repeat(64),"state":"prepared","pending":true,"unresolved":false,
        "before_tick":130,"game_ticks":5,"wall_millis":1000,
        "purpose":"sample_original_inventory","native":null
    });
    let rendered = stock_clock_packet(anchor, &view, 20);
    assert_eq!(rendered["original_goal_proven"], false);
    assert_eq!(
        rendered["agent_turn"]["briefing"]["original_goal_proven"],
        false
    );
    let recommendation = &rendered["agent_turn"]["recommendations"][0];
    assert_eq!(recommendation["tool"], "fortress_commit");
    assert_eq!(recommendation["arguments"]["scope"], "inventory_run");
    assert_eq!(recommendation["arguments"]["order_step_id"], 0);
    assert_eq!(recommendation["arguments"]["paired_seal"], "e".repeat(64));
    assert_eq!(recommendation["requires_confirmation"], true);
    view["selected_parent_goal"]["parent_stability_pending"] = false.into();
    let withheld = stock_clock_packet(anchor, &view, 20);
    assert!(
        !withheld["agent_turn"]["affordances"]
            .as_array()
            .expect("affordances")
            .iter()
            .any(|item| item["action"] == "fortress_commit" && item["enabled"] == true)
    );
}

#[test]
fn complete_stock_cohort_can_prepare_clock_without_relabeling_the_selected_anchor() {
    let (anchor, mut view) = pending_stock_targets_clock_fixture();
    view["selected_parent_goal"] = Value::Null;
    let selected_anchor = view["selected_inventory_goal"]["anchor"].clone();
    assert_ne!(selected_anchor, view["current_parent_goal"]["anchor"]);
    let rendered = stock_clock_packet(anchor, &view, 20);
    let recommendation = &rendered["agent_turn"]["recommendations"][0];
    assert_eq!(recommendation["tool"], "fortress_plan");
    let request: Value = serde_json::from_str(
        recommendation["arguments"]["request"]
            .as_str()
            .expect("cohort clock request"),
    )
    .expect("closed cohort request");
    assert_eq!(request["kind"], "inventory_run");
    assert_eq!(request["order_step_id"], 0);
    assert_eq!(
        rendered["selected_inventory_goal"]["anchor"],
        selected_anchor
    );
    assert_eq!(
        rendered["current_parent_goal"]["anchor"],
        anchor_value(anchor)
    );
    assert_eq!(rendered["original_goal_proven"], false);
    view["selected_inventory_goal"]["anchor"]["sequence"] = 8.into();
    let refused = stock_clock_packet(anchor, &view, 20);
    assert!(
        !refused["agent_turn"]["affordances"]
            .as_array()
            .expect("affordances")
            .iter()
            .any(|item| item["action"] == "fortress_plan" && item["enabled"] == true)
    );
}
