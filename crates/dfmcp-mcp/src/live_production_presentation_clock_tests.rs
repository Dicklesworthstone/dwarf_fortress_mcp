//! Clock guidance exercises the real presentation boundary with retained views.
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
        state_hash: Digest32::of_bytes(b"current labor and inventory"),
    };
    let selected = StateAnchor {
        state_hash: Digest32::of_bytes(b"selected inventory"),
        ..anchor
    };
    let run = json!({"review_seal":"e".repeat(64),"key":"original-clock-run",
        "plan_digest":"f".repeat(64),"state":"prepared","pending":true,"unresolved":false,
        "native_order_id":100,"before_tick":130,"game_ticks":10,"wall_millis":1000,
        "purpose":"advance_original_order","native":null});
    let view = json!({"ok":true,"anchor":anchor_value(anchor),"observation_tick_floor":130,
        "original_plan":{"plan_digest":"a".repeat(64),"expires_at_tick":300,"steps":[
            {"step_id":0,"action":{"kind":"set_labor"}},
            {"step_id":1,"action":{"kind":"create_work_order"},"obligation":{
                "deadline_tick":300,"poll_interval_ticks":5,"stable_samples":2}}]},
        "frontier":{"all_applied":true,"needs_reconciliation":false,"entries":[],"next_ready":null},
        "review":null,"original_goal_proven":false,"clock_control_enabled":true,"clock_invalidates_goal":false,
        "recovery":{"clock_journal_id":"1".repeat(64)},
        "selected_inventory_goal":{"scope":"selected_original_order_step","step_id":1,
            "kind":"observed_inventory_minimum","item_type":"BED","minimum_stack_units":1,
            "current_truth":"false","proven_at_anchor":false,"anchor":anchor_value(selected),
            "receipt_linked":true,"deadline_tick":300,"poll_interval_ticks":5,"stable_samples":2,
            "history":{"present":true,"fenced":false},"terminal_failure":false},
        "current_parent_goal":{"scope":"complete_original_labor_and_inventory_goal",
            "current_truth":"false","original_goal_proven":false,"anchor":anchor_value(anchor),
            "selected_anchor":anchor_value(selected)},
        "clock":{"source":"order-run/1.14","target_scope":"selected_original_order_step",
            "journal_id":"1".repeat(64),"head":"2".repeat(64),"target_digest":"3".repeat(64),
            "original_paired_seal":"4".repeat(64),"mode":"control","review_available":prepared,
            "pending":prepared,"unresolved":false,"may_be_running":false,"needs_reconciliation":prepared,
            "high_tick":130,"run_count":u32::from(prepared),
            "active_run":if prepared {run.clone()}else{Value::Null},
            "runs":if prepared {vec![run]}else{vec![]}}});
    (anchor, view)
}

fn output(anchor: StateAnchor, view: &Value, mode: &str, verified: bool, ticks: u64) -> Value {
    let (raw, overflow) = packet(
        "fortress.query",
        Some(SessionId::new((1u128 << 127) | (24u128 << 57) | 10)),
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
    serde_json::from_str(&raw).expect("complete clock packet")
}

fn enabled(value: &Value, action: &str) -> bool {
    value["agent_turn"]["affordances"]
        .as_array()
        .expect("affordances")
        .iter()
        .any(|item| item["scope"] == "run" && item["action"] == action && item["enabled"] == true)
}

#[test]
fn clock_proposals_keep_original_identity_deadline_and_stability_cadence() {
    for (truth, ticks, deadline, expected) in [
        ("false", 1200, 300, 170),
        ("false", 9, 300, 9),
        ("true", 1200, 300, 5),
        ("true", 1200, 133, 3),
    ] {
        let (anchor, mut view) = fixture(false);
        view["selected_inventory_goal"]["current_truth"] = json!(truth);
        view["current_parent_goal"]["current_truth"] = json!(truth);
        view["selected_inventory_goal"]["deadline_tick"] = json!(deadline);
        view["original_plan"]["steps"][1]["obligation"]["deadline_tick"] = json!(deadline);
        let value = output(anchor, &view, "control", true, ticks);
        assert!(enabled(&value, "fortress_plan"));
        let next = &value["agent_turn"]["recommendations"][0];
        assert_eq!(next["tool"], "fortress_plan");
        assert_eq!(next["risk"], "guarded");
        assert_eq!(next["requires_confirmation"], false);
        let request: Value =
            serde_json::from_str(next["arguments"]["request"].as_str().expect("request"))
                .expect("closed request");
        assert_eq!(
            request,
            json!({"kind":"run","original_paired_seal":"4".repeat(64),
            "game_ticks":expected,"wall_millis":5000})
        );
        assert_eq!(
            value["agent_turn"]["briefing"]["original_goal_proven"],
            false
        );
        assert_eq!(
            value["agent_turn"]["briefing"]["current_pause_proven"],
            false
        );
    }
}

#[test]
fn clock_confirmations_require_separate_current_review_and_allow_explicit_cancellation() {
    let (anchor, view) = fixture(true);
    let value = output(anchor, &view, "control", true, 1200);
    assert!(enabled(&value, "fortress_commit"));
    assert!(enabled(&value, "fortress_cancel"));
    assert!(!enabled(&value, "fortress_plan"));
    let next = &value["agent_turn"]["recommendations"][0];
    assert_eq!(next["tool"], "fortress_commit");
    assert_eq!(next["arguments"]["scope"], "run");
    assert_eq!(next["arguments"]["paired_seal"], "e".repeat(64));
    assert_eq!(next["arguments"]["confirmed"], true);
    assert_eq!(next["requires_confirmation"], true);
    assert_eq!(
        value["agent_turn"]["active_work"]["confirmations"][0]["scope"],
        "run"
    );
    assert_eq!(
        value["agent_turn"]["active_work"]["confirmations"][0]["game_ticks"],
        10
    );
    assert_eq!(
        value["agent_turn"]["recommendations"][1]["arguments"]["scope"],
        "run"
    );
    assert_eq!(
        value["agent_turn"]["recommendations"][1]["tool"],
        "fortress_cancel"
    );

    // A reopened prepared run may be reviewed explicitly using the same
    // bounded request. Reading it alone cannot resurrect its old confirmation.
    let mut reopened = view.clone();
    reopened["clock"]["review_available"] = json!(false);
    let value = output(anchor, &reopened, "control", true, 1200);
    assert!(enabled(&value, "fortress_plan"));
    assert!(!enabled(&value, "fortress_commit"));
    assert!(enabled(&value, "fortress_cancel"));
    let next = &value["agent_turn"]["recommendations"][0];
    assert_eq!(next["tool"], "fortress_plan");
    assert_eq!(next["requires_confirmation"], false);
    let request: Value = serde_json::from_str(
        next["arguments"]["request"]
            .as_str()
            .expect("re-review request"),
    )
    .expect("closed re-review request");
    assert_eq!(
        request,
        json!({"kind":"run","original_paired_seal":"4".repeat(64),
        "game_ticks":10,"wall_millis":1000})
    );
    assert_eq!(
        value["agent_turn"]["active_work"]["confirmations"],
        json!([])
    );
    assert!(value["agent_turn"]["active_work"]["clock"]["active_run"]["review_seal"].is_null());
}

#[test]
fn clock_dispatch_guidance_rejects_revocation_stale_sources_retirement_and_old_reviews() {
    for case in 0..16 {
        let (anchor, mut view) = fixture(true);
        let mut mode = "control";
        let mut verified = true;
        let mut ticks = 1200;
        match case {
            0 => view["clock_control_enabled"] = json!(false),
            1 => ticks = 0,
            2 => mode = "reconcile",
            3 => view["clock"]["mode"] = json!("reconcile"),
            4 => view["recovery"]["clock_journal_id"] = json!("9".repeat(64)),
            5 => view["clock"]["active_run"]["before_tick"] = json!(129),
            6 => view["clock"]["high_tick"] = json!(131),
            7 => view["clock"]["review_available"] = json!(false),
            8 => view["selected_inventory_goal"]["receipt_linked"] = json!(false),
            9 => view["clock_invalidates_goal"] = json!(true),
            10 => view["parent_goal_observation_fenced"] = json!(true),
            11 => view["current_parent_goal"]["current_truth"] = json!("unknown"),
            12 => view["original_plan"]["expires_at_tick"] = json!(130),
            13 => view["selected_inventory_goal"]["history"]["fenced"] = json!(true),
            14 => verified = false,
            _ => view["clock"]["active_run"]["game_ticks"] = json!(1201),
        }
        let value = output(anchor, &view, mode, verified, ticks);
        assert!(!enabled(&value, "fortress_commit"), "case {case}");
        assert_eq!(enabled(&value, "fortress_plan"), case == 7, "case {case}");
        assert_eq!(
            value["agent_turn"]["active_work"]["confirmations"],
            json!([]),
            "case {case}"
        );
        assert!(
            value["agent_turn"]["active_work"]["clock"]["active_run"]["review_seal"].is_null(),
            "case {case}"
        );
    }
    for truth in ["true", "false"] {
        let (anchor, mut view) = fixture(false);
        view["selected_inventory_goal"]["current_truth"] = json!(truth);
        view["current_parent_goal"]["current_truth"] = json!(truth);
        view["selected_inventory_goal"]["receipt_linked"] = json!(false);
        let value = output(anchor, &view, "control", true, 1200);
        assert!(
            !enabled(&value, "fortress_plan"),
            "retired order, inventory {truth}"
        );
    }
}

#[test]
fn unresolved_clock_withdraws_goal_proof_and_keeps_stop_and_reconcile_guidance() {
    let (anchor, mut view) = fixture(true);
    view["selected_inventory_goal"]["proven_at_anchor"] = json!(true);
    view["selected_inventory_goal"]["current_truth"] = json!("true");
    view["current_parent_goal"]["current_truth"] = json!("true");
    view["current_parent_goal"]["original_goal_proven"] = json!(true);
    view["original_goal_proven"] = json!(true);
    view["parent_goal_observation_fenced"] = json!(true);
    view["clock"]["unresolved"] = json!(true);
    view["clock"]["may_be_running"] = json!(true);
    view["clock"]["active_run"]["state"] = json!("tracking");
    view["clock"]["active_run"]["unresolved"] = json!(true);
    view["clock"]["active_run"]["native"] = json!({"phase":"running","unpause_attempted":true});
    let value = output(anchor, &view, "control", true, 0);
    assert_eq!(value["original_goal_proven"], false);
    assert!(value["current_parent_goal"].is_null());
    assert_eq!(value["selected_inventory_goal"]["proven_at_anchor"], false);
    assert_eq!(value["agent_turn"]["continuity"]["status"], "indeterminate");
    assert_eq!(
        value["agent_turn"]["briefing"]["original_goal_proven"],
        false
    );
    assert_eq!(
        value["agent_turn"]["briefing"]["clock_may_be_running"],
        true
    );
    assert!(!enabled(&value, "fortress_commit"));
    assert!(!enabled(&value, "fortress_plan"));
    assert!(enabled(&value, "fortress_cancel"));
    assert_eq!(
        value["agent_turn"]["recommendations"][0]["tool"],
        "fortress_wait"
    );
    assert_eq!(
        value["agent_turn"]["recommendations"][1]["arguments"]["scope"],
        "run"
    );
    let offline = output(anchor, &view, "offline", true, 0);
    assert_eq!(
        offline["agent_turn"]["recommendations"][0]["arguments"]["scope"],
        "session"
    );
    assert!(!enabled(&offline, "fortress_cancel"));

    // A failed journal inspection is negative evidence even if an older
    // prepared snapshot still carries apparently clean convenience flags.
    let (anchor, mut inspecting) = fixture(true);
    inspecting["parent_goal_observation_fenced"] = json!(true);
    inspecting["clock"]["inspection_required"] = json!(true);
    let result = output(anchor, &inspecting, "control", true, 0);
    assert_eq!(
        result["agent_turn"]["recommendations"][0]["tool"],
        "fortress_wait"
    );
    assert!(enabled(&result, "fortress_cancel"));
    assert!(!enabled(&result, "fortress_commit"));
    assert_eq!(compact(&inspecting)["clock"]["inspection_required"], true);
    let failed = output(anchor, &inspecting, "control", false, 0);
    assert_eq!(
        failed["agent_turn"]["recommendations"][0]["tool"],
        "fortress_wait"
    );
    assert!(!enabled(&failed, "fortress_cancel"));
    assert_eq!(
        failed["agent_turn"]["active_work"]["confirmations"],
        json!([])
    );
}

#[test]
fn settled_clock_receipt_requires_fresh_goal_sample_and_source_loss_requires_inspection() {
    let (anchor, mut view) = fixture(false);
    view["clock_invalidates_goal"] = json!(true);
    view["clock"]["runs"] = json!([{"native":{"phase":"stopped","historical_pause_verified":true,
        "predicate_observed":true,"original_goal_proven":true}}]);
    view["selected_inventory_goal"]["first_satisfied_anchor"] =
        view["selected_inventory_goal"]["anchor"].clone();
    let value = output(anchor, &view, "control", true, 1200);
    assert_eq!(
        value["agent_turn"]["recommendations"][0]["tool"],
        "fortress_wait"
    );
    assert_eq!(
        value["agent_turn"]["briefing"]["original_goal_proven"],
        false
    );
    assert_eq!(
        value["agent_turn"]["briefing"]["current_pause_proven"],
        false
    );
    assert!(value["selected_inventory_goal"]["first_satisfied_anchor"].is_object());
    let (anchor, mut lost) = fixture(true);
    lost["clock"]["unresolved"] = json!(true);
    lost["clock"]["may_be_running"] = json!(true);
    lost["clock"]["needs_reconciliation"] = json!(false);
    lost["clock"]["active_run"]["state"] = json!("terminal");
    lost["clock"]["active_run"]["native"] = json!({"phase":"source_lost"});
    let result = output(anchor, &lost, "control", true, 1200);
    assert_eq!(
        result["agent_turn"]["recommendations"][0]["tool"],
        "fortress_doctor"
    );
    assert_eq!(
        result["agent_turn"]["briefing"]["current_pause_proven"],
        false
    );
}

#[test]
fn failed_closed_and_oversized_packets_keep_clock_custody_without_old_confirmations() {
    let (anchor, mut view) = fixture(true);
    view["clock"]["unbounded_private_value"] = json!("must-not-survive-clock-fallback");
    for (closed, running) in [(false, false), (true, false), (true, true)] {
        if running {
            view["clock"]["unresolved"] = json!(true);
            view["clock"]["active_run"]["state"] = json!("tracking");
        }
        let (raw, overflow) = packet(
            "fortress.commit",
            Some(SessionId::new((1u128 << 127) | (24u128 << 57) | 11)),
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
        assert!(!raw.contains("must-not-survive-clock-fallback"));
        let result: Value = serde_json::from_str(&raw).expect("bounded fallback");
        assert_eq!(
            result["retained_reference"]["recovery"]["clock_journal_id"],
            "1".repeat(64)
        );
        assert_eq!(
            result["retained_reference"]["clock"]["active_run"]["key"],
            "original-clock-run"
        );
        assert!(result["retained_reference"]["clock"]["active_run"]["review_seal"].is_null());
        assert_eq!(
            result["agent_turn"]["active_work"]["confirmations"],
            json!([])
        );
        assert!(!enabled(&result, "fortress_commit"));
        assert!(!enabled(&result, "fortress_plan"));
        assert!(!enabled(&result, "fortress_cancel"));
        if closed {
            assert_eq!(result["closed"], true);
            assert_eq!(result["agent_turn"]["recommendations"], json!([]));
        }
    }
}
