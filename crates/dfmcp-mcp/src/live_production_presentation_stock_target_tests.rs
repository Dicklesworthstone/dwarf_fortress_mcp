use super::*;
use dfmcp_core::{Digest32, FortressId, GameTick, ObservationCursor};

fn fixture(planned: bool) -> (StateAnchor, Value) {
    let anchor = StateAnchor {
        fortress_id: FortressId::new(7),
        cursor: ObservationCursor {
            epoch: 4,
            sequence: 5,
        },
        tick: GameTick(130),
        state_hash: Digest32::of_bytes(b"stock target presentation"),
    };
    let target = json!({"schema":"dfmcp.stock-target/1",
        "status":if planned {"planned"}else{"already_satisfied"},
        "recipe":"wooden_bed","item_type":"BED",
        "baseline_stack_units":if planned {4}else{9},"target_stack_units":7,
        "deficit_stack_units":if planned {3}else{0},"amount":if planned {3}else{0},
        "max_order_amount":10,"baseline_anchor":anchor_value(anchor),
        "source_digest":"b".repeat(64),"source_generation":30,
        "source_counters":[18,4,96],"assessment_witness":"c".repeat(64),
        "consumed_tick_floor":130,"revalidation_failed":false,
        "last_check_witness":"d".repeat(64),"work_required":planned,
        "mutation_planned":planned,"satisfied_at_anchor":!planned,
        "labor_assignment_requested":false,"labor_assignment_planned":false,
        "order_step_id":if planned {Some(0)}else{None::<u64>},
        // Presentation must withdraw caller-provided current-proof claims.
        "fresh_for_current_call":true,"current_inventory_proven":true});
    let mut view = json!({"ok":true,"anchor":anchor_value(anchor),
        "stock_target":target,"original_plan":null,"frontier":null,"review":null,
        "observation":{"seal":"a".repeat(64),"paused":true},
        "production_control_enabled":true,"labor_control_enabled":false,
        "current_parent_goal":null,"current_state_proven":false,"original_goal_proven":false});
    if planned {
        view["original_plan"] = json!({"plan_digest":"e".repeat(64),"steps":[
            {"step_id":0,"action":{"kind":"create_work_order","amount":3}}]});
        view["frontier"] = json!({"next_ready":0,"all_applied":false,
            "needs_reconciliation":false,"entries":[]});
    }
    (anchor, view)
}

fn render(anchor: StateAnchor, view: &Value, verified: bool) -> Value {
    let (raw, overflow) = packet(
        "fortress.plan",
        Some(SessionId::new(1)),
        Some(anchor),
        Some("control"),
        None,
        None,
        view.clone(),
        Some(view),
        verified,
        false,
    );
    assert!(!overflow);
    serde_json::from_str(&raw).expect("stock target packet")
}

#[test]
fn stock_target_retention_preserves_original_sizing_without_current_proof() {
    let (_, view) = fixture(true);
    for retained in [compact(&view), compact(&compact(&view))] {
        let target = &retained["stock_target"];
        for field in [
            "baseline_stack_units",
            "target_stack_units",
            "deficit_stack_units",
            "amount",
            "max_order_amount",
            "baseline_anchor",
            "source_digest",
            "source_generation",
            "source_counters",
            "assessment_witness",
            "labor_assignment_requested",
            "labor_assignment_planned",
            "order_step_id",
        ] {
            assert_eq!(target[field], view["stock_target"][field], "{field}");
        }
        assert_eq!(target["fresh_for_current_call"], false);
        assert_eq!(target["current_inventory_proven"], false);
        assert!(target["last_check_witness"].is_null());
        assert_eq!(target["continuous_maintenance"], false);
    }
}

#[test]
fn met_stock_target_reports_no_action_and_no_confirmation() {
    let (anchor, view) = fixture(false);
    let result = render(anchor, &view, true);
    assert!(result["original_plan"].is_null());
    assert_eq!(result["stock_target"]["status"], "already_satisfied");
    assert_eq!(result["stock_target"]["mutation_planned"], false);
    assert_eq!(result["stock_target"]["satisfied_at_anchor"], true);
    assert_eq!(result["original_goal_proven"], false);
    assert_eq!(result["agent_turn"]["active_work"]["actions"], json!([]));
    assert_eq!(
        result["agent_turn"]["active_work"]["confirmations"],
        json!([])
    );
    assert_eq!(
        result["agent_turn"]["recommendations"][0]["tool"],
        "fortress_observe"
    );
    assert_eq!(result["stock_target"]["current_inventory_proven"], false);
}

#[test]
fn failed_stock_target_check_withholds_dispatch_guidance_and_keeps_original() {
    let (anchor, mut view) = fixture(true);
    view["stock_target"]["revalidation_failed"] = json!(true);
    view["review"] = json!({"family":"work-orders/1.10","step_id":0,
        "paired_seal":"f".repeat(64)});
    let result = render(anchor, &view, true);
    assert!(result["review"].is_null());
    assert_eq!(
        result["agent_turn"]["active_work"]["confirmations"],
        json!([])
    );
    assert!(
        result["agent_turn"]["recommendations"]
            .as_array()
            .expect("recommendations")
            .iter()
            .all(|row| row["tool"] != "fortress_commit" && row["tool"] != "fortress_plan")
    );
    assert_eq!(result["stock_target"]["amount"], 3);
    assert_eq!(result["stock_target"]["target_stack_units"], 7);
    assert!(
        result["agent_turn"]["uncertainty"]
            .as_array()
            .expect("uncertainty")
            .iter()
            .any(|row| row["scope"] == "original_stock_target_dispatch_basis")
    );
}

#[test]
fn failed_publication_keeps_stock_identity_but_no_fresh_check() {
    let (anchor, mut view) = fixture(true);
    view["extra"] = json!("x".repeat(80_000));
    let (raw, overflow) = packet(
        "fortress.plan",
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
    let result: Value = serde_json::from_str(&raw).expect("overflow packet");
    let retained = &result["retained_reference"]["stock_target"];
    assert_eq!(
        retained["assessment_witness"],
        view["stock_target"]["assessment_witness"]
    );
    assert_eq!(retained["amount"], 3);
    assert_eq!(retained["current_inventory_proven"], false);
    assert!(retained["last_check_witness"].is_null());
}

#[test]
fn assignment_stock_target_retains_two_step_identity_and_separate_confirmation() {
    let (anchor, mut view) = fixture(true);
    view["stock_target"]["labor_assignment_requested"] = json!(true);
    view["stock_target"]["labor_assignment_planned"] = json!(true);
    view["stock_target"]["order_step_id"] = json!(1);
    view["stock_target"]["worker_prerequisite_checked"] = json!(false);
    view["labor_control_enabled"] = json!(true);
    view["original_plan"]["steps"] = json!([
        {"step_id":0,"action":{"kind":"set_labor","enabled":true}},
        {"step_id":1,"depends_on":[0],"action":{"kind":"create_work_order","amount":3}}
    ]);
    let retained = compact(&compact(&view));
    assert_eq!(retained["stock_target"]["labor_assignment_requested"], true);
    assert_eq!(retained["stock_target"]["labor_assignment_planned"], true);
    assert_eq!(retained["stock_target"]["order_step_id"], 1);
    assert_eq!(retained["stock_target"]["worker_prerequisite_checked"], false);
    assert_eq!(retained["stock_target"]["current_inventory_proven"], false);

    view["review"] = json!({"family":"workforce/1.17","step_id":0,
        "paired_seal":"f".repeat(64)});
    let result = render(anchor, &view, true);
    assert_eq!(result["stock_target"]["labor_assignment_planned"], true);
    assert_eq!(result["stock_target"]["order_step_id"], 1);
    assert_eq!(result["agent_turn"]["recommendations"][0]["tool"], "fortress_commit");
    assert_eq!(
        result["agent_turn"]["recommendations"][0]["arguments"]["paired_seal"],
        view["review"]["paired_seal"]
    );
    assert_eq!(result["original_goal_proven"], false);
}

#[test]
fn satisfied_assignment_target_does_not_offer_a_labor_effect() {
    let (anchor, mut view) = fixture(false);
    view["stock_target"]["labor_assignment_requested"] = json!(true);
    view["stock_target"]["labor_assignment_planned"] = json!(false);
    view["stock_target"]["order_step_id"] = Value::Null;
    view["stock_target"]["worker_prerequisite_checked"] = json!(false);
    let result = render(anchor, &view, true);
    assert_eq!(result["stock_target"]["labor_assignment_requested"], true);
    assert_eq!(result["stock_target"]["labor_assignment_planned"], false);
    assert!(result["stock_target"]["order_step_id"].is_null());
    assert_eq!(result["stock_target"]["mutation_planned"], false);
    assert_eq!(result["agent_turn"]["active_work"]["actions"], json!([]));
    assert_eq!(
        result["agent_turn"]["active_work"]["confirmations"],
        json!([])
    );
    assert_eq!(
        result["agent_turn"]["recommendations"][0]["tool"],
        "fortress_observe"
    );
}

#[test]
fn assignment_stock_target_overflow_retains_intent_without_a_commit_permit() {
    let (anchor, mut view) = fixture(true);
    view["stock_target"]["labor_assignment_requested"] = json!(true);
    view["stock_target"]["labor_assignment_planned"] = json!(true);
    view["stock_target"]["order_step_id"] = json!(1);
    view["stock_target"]["worker_prerequisite_checked"] = json!(false);
    view["labor_control_enabled"] = json!(true);
    view["original_plan"]["steps"] = json!([
        {"step_id":0,"action":{"kind":"set_labor","enabled":true}},
        {"step_id":1,"depends_on":[0],"action":{"kind":"create_work_order","amount":3}}
    ]);
    view["review"] = json!({"family":"workforce/1.17","step_id":0,
        "paired_seal":"f".repeat(64)});
    view["extra"] = json!("x".repeat(80_000));
    let (raw, overflow) = packet(
        "fortress.plan",
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
    let result: Value = serde_json::from_str(&raw).expect("overflow packet");
    let target = &result["retained_reference"]["stock_target"];
    assert_eq!(target["labor_assignment_requested"], true);
    assert_eq!(target["labor_assignment_planned"], true);
    assert_eq!(target["order_step_id"], 1);
    assert_eq!(target["amount"], 3);
    assert_eq!(target["current_inventory_proven"], false);
    assert!(result["review"].is_null());
    assert!(result["retained_reference"]["frontier"]["review"].is_null());
}
