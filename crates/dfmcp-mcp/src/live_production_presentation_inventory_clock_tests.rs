//! Independent inventory-clock guidance at the real Agent Turn boundary.
use super::*;
use dfmcp_core::{Digest32, FortressId, GameTick, ObservationCursor};

fn fixture(prepared: bool) -> (StateAnchor, Value) {
    let anchor = StateAnchor {
        fortress_id: FortressId::new(7),
        cursor: ObservationCursor {
            epoch: 1,
            sequence: 5,
        },
        tick: GameTick(130),
        state_hash: Digest32::of_bytes(b"current combined parent"),
    };
    let selected = StateAnchor {
        state_hash: Digest32::of_bytes(b"original selected inventory"),
        ..anchor
    };
    let run = json!({"review_seal":"e".repeat(64),"key":"original-inventory-clock-run",
        "plan_digest":"f".repeat(64),"state":"prepared","pending":true,"unresolved":false,
        "before_tick":130,"game_ticks":5,"wall_millis":1000,
        "purpose":"sample_original_inventory","native":null});
    let view = json!({"ok":true,"anchor":anchor_value(anchor),"observation_tick_floor":130,
        "original_plan":{"plan_digest":"a".repeat(64),"expires_at_tick":300,"steps":[
            {"step_id":0,"action":{"kind":"set_labor"}},
            {"step_id":1,"action":{"kind":"create_work_order"},"obligation":{
                "deadline_tick":300,"poll_interval_ticks":5,"stable_samples":2}}]},
        "frontier":{"all_applied":true,"needs_reconciliation":false,"entries":[],"next_ready":null},
        "review":null,"original_goal_proven":false,"clock_control_enabled":true,
        "inventory_clock_control_enabled":true,"clock_invalidates_goal":false,
        "inventory_clock_invalidates_goal":false,
        "recovery":{"clock_journal_id":"1".repeat(64),"inventory_clock_journal_id":"5".repeat(64)},
        "selected_inventory_goal":{"scope":"selected_original_order_step","step_id":1,
            "kind":"observed_inventory_minimum","item_type":"BED","minimum_stack_units":1,
            "current_truth":"true","proven_at_anchor":false,"anchor":anchor_value(selected),
            "receipt_linked":false,"deadline_tick":300,"poll_interval_ticks":5,"stable_samples":2,
            "history":{"present":true,"fenced":false},"terminal_failure":false},
        "current_parent_goal":{"scope":"complete_original_labor_and_inventory_goal",
            "current_truth":"true","original_goal_proven":false,"anchor":anchor_value(anchor),
            "selected_anchor":anchor_value(selected)},
        "clock":{"source":"order-run/1.14","target_scope":"selected_original_order_step",
            "journal_id":"1".repeat(64),"head":"2".repeat(64),"target_digest":"3".repeat(64),
            "original_paired_seal":"4".repeat(64),"mode":"control","review_available":false,
            "pending":false,"unresolved":false,"may_be_running":false,"needs_reconciliation":false,
            "high_tick":130,"run_count":0,"active_run":null},
        "inventory_clock":{"source":"run/1.13","target_scope":"selected_original_inventory_goal",
            "journal_id":"5".repeat(64),"head":"6".repeat(64),"target_digest":"7".repeat(64),
            "original_paired_seal":"4".repeat(64),"mode":"control","independent_authority":true,
            "review_available":prepared,"pending":prepared,"unresolved":false,"may_be_running":false,
            "needs_reconciliation":prepared,"high_tick":130,"run_count":u32::from(prepared),
            "active_run":if prepared {run}else{Value::Null}}});
    (anchor, view)
}

fn output(anchor: StateAnchor, view: &Value, mode: &str, verified: bool, ticks: u64) -> Value {
    let (raw, overflow) = packet(
        "fortress.query",
        Some(SessionId::new((1u128 << 127) | (24u128 << 57) | 12)),
        Some(anchor),
        Some(mode),
        None,
        Some(WorkBudget {
            max_game_ticks: ticks,
            max_wall_millis: 60000,
            ..WorkBudget::default()
        }),
        if verified {
            view.clone()
        } else {
            json!({"ok":false,"retained_reference":compact(view)})
        },
        Some(view),
        verified,
        false,
    );
    assert!(!overflow);
    serde_json::from_str(&raw).expect("complete independent clock packet")
}

fn enabled(value: &Value, scope: &str, action: &str) -> bool {
    value["agent_turn"]["affordances"]
        .as_array()
        .expect("affordances")
        .iter()
        .any(|item| item["scope"] == scope && item["action"] == action && item["enabled"] == true)
}

#[test]
fn retired_order_can_finish_original_inventory_stability_under_its_own_clock() {
    for (ticks, deadline, runs, expected) in [(1200, 300, 0, 5), (2, 300, 63, 2), (1200, 133, 0, 3)]
    {
        let (anchor, mut view) = fixture(false);
        view["inventory_clock"]["run_count"] = json!(runs);
        view["selected_inventory_goal"]["deadline_tick"] = json!(deadline);
        view["original_plan"]["steps"][1]["obligation"]["deadline_tick"] = json!(deadline);
        let value = output(anchor, &view, "control", true, ticks);
        assert!(enabled(&value, "inventory_run", "fortress_plan"));
        assert!(!enabled(&value, "run", "fortress_plan"));
        let next = &value["agent_turn"]["recommendations"][0];
        assert_eq!(next["tool"], "fortress_plan");
        assert_eq!(next["risk"], "guarded");
        assert_eq!(next["requires_confirmation"], false);
        let request: Value =
            serde_json::from_str(next["arguments"]["request"].as_str().expect("request"))
                .expect("closed request");
        assert_eq!(
            request,
            json!({"kind":"inventory_run","original_paired_seal":"4".repeat(64),
            "game_ticks":expected,"wall_millis":5000})
        );
        assert_eq!(
            value["agent_turn"]["active_work"]["clock"]["journal_id"],
            "1".repeat(64)
        );
        assert_eq!(
            value["agent_turn"]["active_work"]["inventory_clock"]["journal_id"],
            "5".repeat(64)
        );
        assert_eq!(
            value["agent_turn"]["active_work"]["inventory_clock"]["run_count"],
            runs
        );
        assert_eq!(
            value["agent_turn"]["briefing"]["original_goal_proven"],
            false
        );
    }
    let (anchor, mut view) = fixture(false);
    view["selected_inventory_goal"]["receipt_linked"] = json!(true);
    let value = output(anchor, &view, "control", true, 1200);
    assert!(enabled(&value, "run", "fortress_plan"));
    let request: Value = serde_json::from_str(
        value["agent_turn"]["recommendations"][0]["arguments"]["request"]
            .as_str()
            .expect("preferred request"),
    )
    .expect("preferred closed request");
    assert_eq!(request["kind"], "inventory_run");
    view["clock_control_enabled"] = json!(false);
    let value = output(anchor, &view, "control", true, 1200);
    assert!(!enabled(&value, "run", "fortress_plan"));
    assert!(enabled(&value, "inventory_run", "fortress_plan"));
}

#[test]
fn inventory_preparation_requires_its_separate_review_and_cancellation_scope() {
    let (anchor, view) = fixture(true);
    let value = output(anchor, &view, "control", true, 1200);
    assert!(enabled(&value, "inventory_run", "fortress_commit"));
    assert!(enabled(&value, "inventory_run", "fortress_cancel"));
    assert!(!enabled(&value, "run", "fortress_commit"));
    let next = &value["agent_turn"]["recommendations"][0];
    assert_eq!(next["arguments"]["scope"], "inventory_run");
    assert_eq!(next["arguments"]["paired_seal"], "e".repeat(64));
    assert_eq!(next["requires_confirmation"], true);
    assert_eq!(
        value["agent_turn"]["active_work"]["confirmations"][0]["scope"],
        "inventory_run"
    );
    assert_eq!(
        value["agent_turn"]["recommendations"][1]["arguments"]["scope"],
        "inventory_run"
    );
    let mut reopened = view;
    reopened["inventory_clock"]["review_available"] = json!(false);
    let value = output(anchor, &reopened, "control", true, 1200);
    assert!(enabled(&value, "inventory_run", "fortress_plan"));
    assert!(!enabled(&value, "inventory_run", "fortress_commit"));
    let request: Value = serde_json::from_str(
        value["agent_turn"]["recommendations"][0]["arguments"]["request"]
            .as_str()
            .expect("re-review"),
    )
    .expect("closed re-review");
    assert_eq!(
        request,
        json!({"kind":"inventory_run","original_paired_seal":"4".repeat(64),
        "game_ticks":5,"wall_millis":1000})
    );
    assert!(
        value["agent_turn"]["active_work"]["inventory_clock"]["active_run"]["review_seal"]
            .is_null()
    );
    assert_eq!(
        value["agent_turn"]["active_work"]["confirmations"],
        json!([])
    );
}

#[test]
fn inventory_guidance_never_borrows_order_authority_or_old_goal_evidence() {
    for case in 0..24 {
        let (anchor, mut view) = fixture(true);
        let mut mode = "control";
        let mut verified = true;
        let mut ticks = 1200;
        match case {
            0 => view["inventory_clock_control_enabled"] = json!(false),
            1 => view["inventory_clock"]["mode"] = json!("reconcile"),
            2 => mode = "reconcile",
            3 => verified = false,
            4 => ticks = 0,
            5 => view["recovery"]["inventory_clock_journal_id"] = json!("1".repeat(64)),
            6 => view["inventory_clock"]["active_run"]["before_tick"] = json!(129),
            7 => view["inventory_clock"]["high_tick"] = json!(131),
            8 => view["clock"]["high_tick"] = json!(131),
            9 => view["inventory_clock_invalidates_goal"] = json!(true),
            10 => view["clock_invalidates_goal"] = json!(true),
            11 => view["parent_goal_observation_fenced"] = json!(true),
            12 => view["current_parent_goal"]["current_truth"] = json!("false"),
            13 => view["current_parent_goal"]["current_truth"] = json!("unknown"),
            14 => view["selected_inventory_goal"]["current_truth"] = json!("false"),
            15 => view["selected_inventory_goal"]["current_truth"] = json!("unknown"),
            16 => view["original_plan"]["expires_at_tick"] = json!(130),
            17 => view["selected_inventory_goal"]["history"]["fenced"] = json!(true),
            18 => view["selected_inventory_goal"]["terminal_failure"] = json!(true),
            19 => view["selected_inventory_goal"]["deadline_tick"] = json!(301),
            20 => view["inventory_clock"]["source"] = json!("order-run/1.14"),
            21 => view["inventory_clock"]["target_scope"] = json!("selected_original_order_step"),
            22 => view["inventory_clock"]["active_run"]["game_ticks"] = json!(1201),
            _ => {
                view["current_parent_goal"]["selected_anchor"]["state_hash"] = json!("9".repeat(64))
            }
        }
        let value = output(anchor, &view, mode, verified, ticks);
        assert!(
            !enabled(&value, "inventory_run", "fortress_commit"),
            "case {case}"
        );
        assert!(
            !enabled(&value, "inventory_run", "fortress_plan"),
            "case {case}"
        );
        assert_eq!(
            value["agent_turn"]["active_work"]["confirmations"],
            json!([]),
            "case {case}"
        );
    }
    let (anchor, mut view) = fixture(false);
    view["inventory_clock"]["run_count"] = json!(64);
    let value = output(anchor, &view, "control", true, 1200);
    assert!(!enabled(&value, "inventory_run", "fortress_plan"));
    assert_eq!(
        value["agent_turn"]["active_work"]["inventory_clock"]["run_count"],
        64
    );
    view["inventory_clock"]["run_count"] = json!(0);
    view["inventory_clock_control_enabled"] = json!(false);
    let value = output(anchor, &view, "control", true, 1200);
    assert!(!enabled(&value, "inventory_run", "fortress_plan"));
    assert!(!enabled(&value, "run", "fortress_plan"));
}

#[test]
fn either_retained_owner_blocks_new_dispatch_and_withdraws_current_proof() {
    let (anchor, mut both) = fixture(true);
    both["selected_inventory_goal"]["receipt_linked"] = json!(true);
    let run = both["inventory_clock"]["active_run"].clone();
    both["clock"]["active_run"] = run;
    both["clock"]["pending"] = json!(true);
    both["clock"]["review_available"] = json!(true);
    both["clock"]["needs_reconciliation"] = json!(true);
    let value = output(anchor, &both, "control", true, 1200);
    for scope in ["run", "inventory_run"] {
        assert!(!enabled(&value, scope, "fortress_plan"));
        assert!(!enabled(&value, scope, "fortress_commit"));
        assert!(enabled(&value, scope, "fortress_cancel"));
    }
    for owner in CLOCK_OWNERS {
        let (anchor, mut view) = fixture(false);
        view["selected_inventory_goal"]["proven_at_anchor"] = json!(true);
        view["current_parent_goal"]["original_goal_proven"] = json!(true);
        view["original_goal_proven"] = json!(true);
        view["original_plan"]["expires_at_tick"] = json!(130);
        view[owner.field]["active_run"] = json!({"key":"retained-clock","state":"cancel_requested",
            "unresolved":true,"native":{"phase":"stopping","unpause_attempted":true}});
        view[owner.field]["pending"] = json!(true);
        view[owner.field]["unresolved"] = json!(true);
        view[owner.field]["may_be_running"] = json!(true);
        view[owner.field]["needs_reconciliation"] = json!(true);
        let value = output(anchor, &view, "control", true, 0);
        assert_eq!(value["original_goal_proven"], false);
        assert!(value["current_parent_goal"].is_null());
        assert_eq!(value["selected_inventory_goal"]["proven_at_anchor"], false);
        assert_eq!(value["agent_turn"]["continuity"]["status"], "indeterminate");
        assert_eq!(
            value["agent_turn"]["recommendations"][0]["tool"],
            "fortress_wait"
        );
        assert_eq!(
            value["agent_turn"]["recommendations"][1]["arguments"]["scope"],
            owner.scope
        );
        assert_eq!(
            value["agent_turn"]["active_work"]["cancellation_drains"][0]["scope"],
            owner.scope
        );
        assert!(enabled(&value, owner.scope, "fortress_cancel"));
        for scope in ["run", "inventory_run"] {
            assert!(!enabled(&value, scope, "fortress_plan"));
            assert!(!enabled(&value, scope, "fortress_commit"));
        }
        let offline = output(anchor, &view, "offline", true, 0);
        assert_eq!(
            offline["agent_turn"]["recommendations"][0]["arguments"]["scope"],
            "session"
        );
        assert!(!enabled(&offline, owner.scope, "fortress_cancel"));
    }
}

#[test]
fn inventory_receipt_needs_a_fresh_joined_sample_and_source_loss_needs_inspection() {
    let (anchor, mut view) = fixture(false);
    view["inventory_clock_invalidates_goal"] = json!(true);
    view["selected_inventory_goal"]["first_satisfied_anchor"] =
        view["selected_inventory_goal"]["anchor"].clone();
    let value = output(anchor, &view, "control", true, 1200);
    assert_eq!(
        value["agent_turn"]["recommendations"][0]["tool"],
        "fortress_wait"
    );
    assert!(value["current_parent_goal"].is_null());
    assert!(value["selected_inventory_goal"]["first_satisfied_anchor"].is_object());
    assert_eq!(
        value["agent_turn"]["briefing"]["current_pause_proven"],
        false
    );
    assert!(!enabled(&value, "inventory_run", "fortress_plan"));
    view["inventory_clock"]["active_run"] =
        json!({"state":"terminal","native":{"phase":"source_lost"}});
    view["inventory_clock"]["unresolved"] = json!(true);
    view["inventory_clock"]["needs_reconciliation"] = json!(false);
    let value = output(anchor, &view, "control", true, 0);
    assert_eq!(
        value["agent_turn"]["recommendations"][0]["tool"],
        "fortress_doctor"
    );
}

#[test]
fn bounded_failed_and_closed_packets_keep_both_journals_without_old_confirmations() {
    let (anchor, mut view) = fixture(true);
    view["inventory_clock"]["run_count"] = json!(64);
    view["inventory_clock"]["unbounded_private_value"] =
        json!("must-not-survive-inventory-fallback");
    view["clock"]["active_run"] = json!({"key":"original-order-clock-run","state":"prepared",
        "review_seal":"d".repeat(64)});
    for closed in [false, true] {
        let (raw, overflow) = packet(
            "fortress.commit",
            Some(SessionId::new((1u128 << 127) | (24u128 << 57) | 13)),
            Some(anchor),
            Some("control"),
            None,
            None,
            json!({"ok":false,"closed":closed,"payload":"x".repeat(70_000)}),
            Some(&view),
            false,
            false,
        );
        assert!(overflow);
        assert!(!raw.contains("must-not-survive-inventory-fallback"));
        let value: Value = serde_json::from_str(&raw).expect("bounded inventory fallback");
        for owner in CLOCK_OWNERS {
            assert_eq!(
                value["retained_reference"]["recovery"][owner.recovery_field],
                view["recovery"][owner.recovery_field]
            );
            assert_eq!(
                value["agent_turn"]["active_work"][owner.field]["active_run"]["key"],
                view[owner.field]["active_run"]["key"]
            );
            assert!(
                value["agent_turn"]["active_work"][owner.field]["active_run"]["review_seal"]
                    .is_null()
            );
            for action in ["fortress_plan", "fortress_commit", "fortress_cancel"] {
                assert!(!enabled(&value, owner.scope, action));
            }
        }
        assert_eq!(
            value["retained_reference"]["inventory_clock"]["run_count"],
            64
        );
        assert_eq!(
            value["agent_turn"]["active_work"]["confirmations"],
            json!([])
        );
        if closed {
            assert_eq!(value["agent_turn"]["recommendations"], json!([]));
        }
    }
}
