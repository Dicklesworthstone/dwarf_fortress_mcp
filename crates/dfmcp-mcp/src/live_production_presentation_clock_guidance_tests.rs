//! The standalone packet consumes the live owner's closed advice, never recovery.
use super::*;
use dfmcp_core::{Digest32, FortressId, GameTick, ObservationCursor};

/// Supply nominal source offers for older presentation fixtures. The shared
/// decoder still validates every source, owner and original field; this helper
/// does not repair them. Runtime generation has its own typed-owner tests.
pub(super) fn attach_guidance(view: &mut Value, ticks: u64, wall: u64) {
    let inventory = view["selected_inventory_goal"].is_object();
    let goal_field = if inventory { "selected_inventory_goal" } else { "selected_order_goal" };
    let goal = &view[goal_field];
    let Some(step) = goal["step_id"].as_u64() else {
        return;
    };
    let portfolio = view["portfolio"].is_object();
    let tick = view["anchor"]["tick"].as_u64().unwrap_or(0);
    let deadline = goal["deadline_tick"].as_u64().unwrap_or(tick)
        .min(view["original_plan"]["expires_at_tick"].as_u64().unwrap_or(tick));
    let remaining = deadline.saturating_sub(tick).min(ticks).min(1200);
    let proposed = if goal["current_truth"] == "true" {
        remaining.min(goal["poll_interval_ticks"].as_u64().unwrap_or(0))
    } else {
        remaining
    };
    let mut offers = Vec::new();
    for owner in [INVENTORY_CLOCK, ORDER_CLOCK] {
        let clock = owner.view(view);
        if !clock.is_object() || view[owner.authority_field] != true
            || owner.other().view(view)["pending"] == true
            || owner == ORDER_CLOCK && goal["receipt_linked"] != true
            || owner == INVENTORY_CLOCK && (!inventory || goal["current_truth"] != "true")
        {
            continue;
        }
        let active = &clock["active_run"];
        let (state, game_ticks, wall_millis) = if active["state"] == "prepared" {
            ("review_prepared", active["game_ticks"].clone(), active["wall_millis"].clone())
        } else if clock_settled(clock)
            && clock["run_count"].as_u64().is_some_and(|count| count < owner.max_runs)
        {
            ("new_interval", json!(proposed), json!(wall.min(5000)))
        } else {
            continue;
        };
        let mut request = json!({"kind":owner.scope,
            "original_paired_seal":clock["original_paired_seal"],
            "game_ticks":game_ticks,"wall_millis":wall_millis});
        if portfolio {
            request["order_step_id"] = json!(step);
        }
        offers.push(json!({"scope":owner.scope,"order_step_id":step,"plan_request":request,
            "state":state,"reason":"Prepare a separately reviewed interval for this exact original.",
            "evidence":[format!("/{}",owner.field),"/original_plan",format!("/{goal_field}"),
                if portfolio {"/portfolio"} else {"/current_parent_goal"}]}));
    }
    view["clock_guidance"] = if offers.is_empty() {
        Value::Null
    } else {
        json!({"schema":"dfmcp.production-clock-guidance/1","authority_free":true,
            "anchor":view["anchor"],"original_plan_digest":view["original_plan"]["plan_digest"],
            "selected_order_step_id":step,"max_game_ticks":ticks,"max_wall_millis":wall,
            "offers":offers,"blockers":[]})
    };
}

fn render(
    anchor: StateAnchor,
    view: &Value,
    mode: &str,
    verified: bool,
    result: Value,
) -> Value {
    let (raw, _) = packet(
        "fortress.query",
        Some(SessionId::new(1)),
        Some(anchor),
        Some(mode),
        None,
        Some(WorkBudget {
            max_game_ticks: 100,
            max_wall_millis: 5000,
            ..WorkBudget::default()
        }),
        result,
        Some(view),
        verified,
        false,
    );
    serde_json::from_str(&raw).expect("complete standalone packet")
}

fn request(value: &Value) -> Option<Value> {
    value["agent_turn"]["recommendations"][0]["arguments"]["request"]
        .as_str()
        .and_then(|raw| serde_json::from_str(raw).ok())
}

#[test]
fn packet_copies_the_source_offer_and_references_its_exact_original() {
    let (anchor, mut view) = clock_tests::fixture(false);
    attach_guidance(&mut view, 7, 4000);
    let offer = view["clock_guidance"]["offers"][0].clone();
    let value = render(anchor, &view, "control", true, view.clone());
    assert_eq!(request(&value), Some(offer["plan_request"].clone()));
    let next = &value["agent_turn"]["recommendations"][0];
    assert_eq!(next["tool"], "fortress_plan");
    assert_eq!(next["reason"], offer["reason"]);
    assert_eq!(next["requires_confirmation"], false);
    assert_eq!(next["estimated_cost"]["game_ticks"], 0);
    assert_eq!(next["confidence"]["evidence"][0]["anchor"], view["anchor"]);
    assert_eq!(next["confidence"]["evidence"][0]["original_plan_digest"],
        view["original_plan"]["plan_digest"]);
    assert_eq!(value["agent_turn"]["active_work"]["confirmations"], json!([]));
    assert_eq!(value["agent_turn"]["briefing"]["original_goal_proven"], false);
    assert_eq!(value["agent_turn"]["briefing"]["current_pause_proven"], false);
}

#[test]
fn retained_preparation_at_the_run_limit_needs_a_fresh_review_before_confirmation() {
    let (anchor, mut view) = clock_tests::fixture(true);
    view["clock"]["review_available"] = false.into();
    view["clock"]["run_count"] = 32.into();
    let mut runs = (0..31).map(|index| json!({"state":"resolved","index":index}))
        .collect::<Vec<_>>();
    runs.push(view["clock"]["active_run"].clone());
    view["clock"]["runs"] = json!(runs);
    attach_guidance(&mut view, 100, 5000);
    let value = render(anchor, &view, "control", true, view.clone());
    assert_eq!(value["agent_turn"]["recommendations"][0]["tool"], "fortress_plan");
    assert_eq!(request(&value), Some(view["clock_guidance"]["offers"][0]["plan_request"].clone()));
    assert_eq!(request(&value).expect("retained request")["game_ticks"], 10);
    assert_eq!(value["agent_turn"]["active_work"]["confirmations"], json!([]));
    assert!(value["agent_turn"]["active_work"]["clock"]["active_run"]["review_seal"].is_null());
    view["clock"]["review_available"] = true.into();
    let value = render(anchor, &view, "control", true, view.clone());
    assert_eq!(value["agent_turn"]["recommendations"][0]["tool"], "fortress_commit");
    assert_eq!(value["agent_turn"]["recommendations"][0]["arguments"]["scope"], "run");
    assert_eq!(value["agent_turn"]["recommendations"][0]["requires_confirmation"], true);
}

#[test]
fn withdrawn_prepared_source_refreshes_before_exact_review_without_forcing_cancellation() {
    let (anchor, mut fresh) = clock_tests::fixture(true);
    fresh["clock"]["review_available"] = false.into();
    attach_guidance(&mut fresh, 100, 5000);
    let mut withdrawn = fresh.clone();
    withdrawn["selected_inventory_goal"]["current_truth"] = Value::Null;
    withdrawn["selected_inventory_goal"]["anchor"] = Value::Null;
    withdrawn["selected_inventory_goal"]["source_digest"] = Value::Null;
    withdrawn["current_parent_goal"] = Value::Null;
    withdrawn["clock_guidance"]["offers"] = json!([]);
    withdrawn["clock_guidance"]["blockers"] = json!([{
        "scope":"all","code":"fresh_goal_sample_required",
        "message":"Acquire a fresh original-goal sample before reviewing this retained interval.",
        "next_action":"wait","order_step_id":1,
        "evidence":["/selected_inventory_goal","/current_parent_goal"]
    }]);
    let value = render(anchor, &withdrawn, "control", true, withdrawn.clone());
    assert_eq!(value["agent_turn"]["recommendations"][0]["tool"], "fortress_wait");
    assert_eq!(value["agent_turn"]["recommendations"][1]["tool"], "fortress_cancel");
    assert_eq!(value["agent_turn"]["recommendations"][1]["arguments"]["scope"], "run");
    assert_eq!(value["agent_turn"]["active_work"]["confirmations"], json!([]));
    let refreshed = render(anchor, &fresh, "control", true, fresh.clone());
    assert_eq!(refreshed["agent_turn"]["recommendations"][0]["tool"], "fortress_plan");
    assert_eq!(request(&refreshed), Some(fresh["clock_guidance"]["offers"][0]["plan_request"].clone()));
    assert_eq!(refreshed["agent_turn"]["active_work"]["confirmations"], json!([]));
}

#[test]
fn missing_stale_or_foreign_advice_cannot_be_rebuilt_from_retained_clock_fields() {
    let (anchor, mut view) = clock_tests::fixture(false);
    attach_guidance(&mut view, 7, 4000);
    for cause in 0..9 {
        let mut changed = view.clone();
        match cause {
            0 => changed["clock_guidance"] = Value::Null,
            1 => changed["clock_guidance"]["original_plan_digest"] = json!("f".repeat(64)),
            2 => changed["clock_guidance"]["anchor"]["sequence"] = json!(4),
            3 => changed["clock_guidance"]["selected_order_step_id"] = json!(0),
            4 => changed["clock_guidance"]["offers"][0]["plan_request"]["confirmed"] = true.into(),
            5 => changed["clock_guidance"]["offers"][0]["plan_request"]["original_paired_seal"] =
                json!("f".repeat(64)),
            6 => changed["observation_tick_floor"] = json!(131),
            7 => changed["current_parent_goal"]["source_digest"] = Value::Null,
            _ => {
                let retained = changed["clock_guidance"].take();
                changed["recovery"]["clock_guidance"] = retained;
            }
        }
        let value = render(anchor, &changed, "control", true, changed.clone());
        assert!(
            request(&value).is_none_or(|request|
                !matches!(request["kind"].as_str(), Some("run" | "inventory_run"))),
            "case {cause}"
        );
        assert!(value["agent_turn"]["briefing"]["clock_guidance"].is_null(), "{cause}");
    }
}

#[test]
fn failed_offline_closed_and_overflow_packets_withdraw_all_clock_advice() {
    let (anchor, mut view) = clock_tests::fixture(false);
    attach_guidance(&mut view, 7, 4000);
    for (mode, verified, result) in [
        ("control", false, json!({"ok":false,"clock_guidance":view["clock_guidance"],
            "retained_reference":view})),
        ("offline", true, view.clone()),
        ("control", true, json!({"ok":true,"closed":true,"clock_guidance":view["clock_guidance"]})),
        ("control", true, json!({"ok":true,"clock_guidance":view["clock_guidance"],
            "payload":"x".repeat(70_000)})),
    ] {
        let value = render(anchor, &view, mode, verified, result);
        assert!(value["clock_guidance"].is_null());
        assert!(value["retained_reference"]["clock_guidance"].is_null());
        assert!(value["agent_turn"]["briefing"]["clock_guidance"].is_null());
        assert!(value["agent_turn"]["recommendations"].as_array().expect("recommendations")
            .iter().all(|next| next["tool"] != "fortress_plan"));
        assert_eq!(value["agent_turn"]["active_work"]["confirmations"], json!([]));
    }
    assert!(compact(&view)["clock_guidance"].is_null());
}

#[test]
fn explicit_blockers_replace_nonprogressing_waits_but_keep_fresh_sample_requests() {
    let (anchor, mut view) = clock_tests::fixture(false);
    attach_guidance(&mut view, 7, 4000);
    view["clock_guidance"]["offers"] = json!([]);
    view["clock_guidance"]["blockers"] = json!([{
        "scope":"run","code":"game_tick_allowance_zero",
        "message":"This session has no game-tick allowance; reopen this exact owner or manage time separately.",
        "next_action":"operator","order_step_id":1,"evidence":["/clock","/original_plan"]
    }]);
    view["clock_guidance"]["max_game_ticks"] = json!(0);
    let value = render(anchor, &view, "control", true, view.clone());
    assert_eq!(value["agent_turn"]["recommendations"][0]["tool"], "fortress_explain");
    assert_eq!(value["agent_turn"]["briefing"]["clock_guidance"]["blockers"][0]["code"],
        "game_tick_allowance_zero");
    view["observation_tick_floor"] = json!(131);
    view["clock_guidance"]["blockers"][0]["code"] = json!("fresh_goal_sample_required");
    view["clock_guidance"]["blockers"][0]["message"] = json!("Acquire a fresh joined sample at the consumed source floor.");
    view["clock_guidance"]["blockers"][0]["next_action"] = json!("wait");
    let value = render(anchor, &view, "control", true, view.clone());
    assert_eq!(value["agent_turn"]["recommendations"][0]["tool"], "fortress_wait");
    assert_eq!(value["agent_turn"]["briefing"]["clock_guidance"]["blockers"][0]["code"],
        "fresh_goal_sample_required");
}

#[test]
fn a_whole_portfolio_can_advance_selected_stability_without_claiming_the_other_leaf() {
    let mut view = crate::live_production_runtime::clock_guidance::tests::fixture(true, false, false);
    let anchor = StateAnchor {
        fortress_id: FortressId::new(1),
        cursor: ObservationCursor { epoch: 0, sequence: 9 },
        tick: GameTick(110),
        state_hash: Digest32::of_bytes(b"complete current portfolio"),
    };
    view["anchor"] = anchor_value(anchor);
    view["clock_guidance"]["anchor"] = anchor_value(anchor);
    view["current_parent_goal"]["anchor"] = anchor_value(anchor);
    view["portfolio"]["complete_original_goal_proven"] = false.into();
    for step in view["original_plan"]["steps"].as_array_mut().expect("steps") {
        step["depends_on"] = if step["step_id"] == 0 { json!([]) } else { json!([0]) };
    }
    // Other original orders remain False, but this exact selected original is
    // True and still needs its own original sampled stability.
    view["selected_order_goal"]["current_truth"] = "true".into();
    view["portfolio"]["orders"][2]["goal"]["current_truth"] = "true".into();
    let value = render(anchor, &view, "control", true, view.clone());
    let next = &value["agent_turn"]["recommendations"][0];
    assert_eq!(next["tool"], "fortress_plan");
    assert_eq!(request(&value), Some(view["clock_guidance"]["offers"][0]["plan_request"].clone()));
    assert_eq!(request(&value).expect("original selector")["order_step_id"], 3);
    assert_eq!(value["agent_turn"]["briefing"]["original_goal_proven"], false);
    assert_eq!(value["agent_turn"]["briefing"]["current_parent_goal_truth"], "false");
}
