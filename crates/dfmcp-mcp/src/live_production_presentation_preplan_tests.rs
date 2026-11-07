//! The pre-plan information loop never creates a goal or a dispatch permit.
use super::*;
use dfmcp_core::{Digest32, FortressId, GameTick, ObservationCursor};

fn fixture() -> (StateAnchor, Value) {
    let anchor = StateAnchor {
        fortress_id: FortressId::new(7),
        tick: GameTick(130),
        cursor: ObservationCursor {
            epoch: 4,
            sequence: 5,
        },
        state_hash: Digest32::of_bytes(b"preplan citizen observation"),
    };
    let source = StateAnchor {
        cursor: ObservationCursor {
            epoch: 0,
            sequence: 2,
        },
        state_hash: Digest32::of_bytes(b"preplan operations observation"),
        ..anchor
    };
    let source_anchor = anchor_value(source);
    let view = json!({"ok":true,"anchor":anchor_value(anchor),
        "observation_tick_floor":130,"original_plan":null,"frontier":null,"review":null,
        "labor_control_enabled":false,"production_control_enabled":true,
        "observation":{"seal":"a".repeat(64),"paused":true,
            "workforce_choices":{"options":[],
                "existing_labor_options":[{"labor":"CARPENTER","eligible_enabled_ids":["43"]}],
                "selection_request":{"units":["43","252"],"option_offset":8,"existing_labor_offset":4}}},
        "production_inspection_source":{"scope":"preplan_captured_operations",
            "attribution":"not_order_attributed","order_step_id":null,
            "anchor":source_anchor,"parent_anchor":anchor_value(anchor),
            "canonical_anchor":anchor_value(anchor),"source_digest":"b".repeat(64),
            "restored_from_history":false,"mutation_authority":false,
            "diagnosis_request":{"schema":"dfmcp.query/1","expected_anchor":source_anchor,
                "query":{"kind":"production_diagnosis","limit":4}},
            "inventory_request":{"schema":"dfmcp.query/1","expected_anchor":source_anchor,
                "query":{"kind":"entities","kinds":["item"],"limit":4}},
            "query_kinds":["production_diagnosis","inventory_plan","entities","inspect","traverse","dependencies"]},
        "production_inspection":null,"original_goal_proven":false,"current_state_proven":false});
    (anchor, view)
}

fn render(anchor: StateAnchor, view: &Value, mode: &str, verified: bool, uncertain: bool) -> Value {
    let (raw, overflow) = packet(
        "fortress.query",
        Some(SessionId::new((1u128 << 127) | (24u128 << 57) | 3)),
        Some(anchor),
        Some(mode),
        None,
        None,
        view.clone(),
        Some(view),
        verified,
        uncertain,
    );
    assert!(!overflow);
    serde_json::from_str(&raw).expect("pre-plan packet")
}

#[test]
fn absent_preplan_source_recommends_explicit_capture_without_replacing_worker_selection() {
    let (anchor, mut view) = fixture();
    view["production_inspection_source"] = Value::Null;
    let result = render(anchor, &view, "control", true, false);
    let next = &result["agent_turn"]["recommendations"][0];
    assert_eq!(next["tool"], "fortress_observe");
    let request: Value =
        serde_json::from_str(next["arguments"]["request"].as_str().expect("request"))
            .expect("closed observation request");
    assert_eq!(
        request,
        json!({"include_production":true,"units":["43","252"],
        "option_offset":8,"existing_labor_offset":4})
    );
    assert_eq!(next["risk"], "read_only");
    assert_eq!(next["requires_confirmation"], false);
    assert_eq!(next["estimated_cost"]["game_ticks"], 0);
    assert_eq!(
        next["confidence"]["evidence"][0]["canonical_anchor"],
        anchor_value(anchor)
    );
    assert_eq!(
        next["confidence"]["evidence"][0]["observation_seal"],
        view["observation"]["seal"]
    );
    assert_eq!(
        result["agent_turn"]["briefing"]["preplan_production_inspection_available"],
        false
    );
    assert_eq!(
        result["agent_turn"]["active_work"]["confirmations"],
        json!([])
    );
}

#[test]
fn preplan_diagnosis_uses_the_separate_operations_anchor_and_source_evidence() {
    let (anchor, view) = fixture();
    let result = render(anchor, &view, "control", true, false);
    assert_eq!(result, render(anchor, &view, "control", true, false));
    let turn = &result["agent_turn"];
    let source = &view["production_inspection_source"];
    assert_ne!(source["anchor"]["epoch"], turn["anchor"]["epoch"]);
    assert_ne!(source["anchor"]["state_hash"], turn["anchor"]["state_hash"]);
    assert_eq!(turn["anchor"], anchor_value(anchor));
    assert_eq!(turn["active_work"]["production_inspection_source"], *source);
    assert_eq!(
        turn["briefing"]["production_inspection_scope"],
        "preplan_captured_operations"
    );
    assert_eq!(
        turn["briefing"]["preplan_production_inspection_available"],
        true
    );
    assert_eq!(turn["briefing"]["original_goal_proven"], false);
    assert_eq!(turn["briefing"]["current_state_proven"], false);
    assert_eq!(turn["active_work"]["obligations"], json!([]));
    let next = &turn["recommendations"][0];
    assert_eq!(next["tool"], "fortress_query");
    assert_eq!(
        serde_json::from_str::<Value>(next["arguments"]["request"].as_str().expect("request"))
            .expect("source query"),
        source["diagnosis_request"]
    );
    assert_eq!(
        next["confidence"]["evidence"][0]["source_anchor"],
        source["anchor"]
    );
    assert_eq!(
        next["confidence"]["evidence"][0]["source_digest"],
        source["source_digest"]
    );
    assert_eq!(next["estimated_cost"]["game_ticks"], 0);
    assert_eq!(next["estimated_cost"]["bridge_bytes"], 0);
    assert_eq!(turn["coverage"]["complete_domains"], json!([]));
    assert!(
        turn["coverage"]["partial_domains"]
            .as_array()
            .expect("partial domains")
            .contains(&json!("preplan_captured_operations"))
    );
    assert_eq!(turn["uncertainty"][0]["epistemic_status"], "unknown");
}

#[test]
fn inspected_preplan_conditions_allow_existing_worker_planning_without_promoting_readiness() {
    let (anchor, mut view) = fixture();
    let source = &view["production_inspection_source"];
    view["production_inspection"] = json!({"kind":"production_diagnosis",
        "anchor":source["anchor"],"source_digest":source["source_digest"],
        "canonical_anchor":anchor_value(anchor),"inspection_scope":"preplan_captured_operations",
        "summary":{"jobs_with_findings":0}});
    let result = render(anchor, &view, "control", true, false);
    let turn = &result["agent_turn"];
    assert_eq!(
        turn["briefing"]["observed_production_conditions"]["jobs_with_findings"],
        0
    );
    assert_eq!(turn["recommendations"][0]["tool"], "fortress_plan");
    assert!(
        turn["recommendations"][0]["reason"]
            .as_str()
            .expect("reason")
            .contains("existing_labor")
    );
    assert_eq!(turn["briefing"]["original_goal_proven"], false);
    assert_eq!(turn["active_work"]["confirmations"], json!([]));
    assert!(
        turn["uncertainty"][0]["finding"]
            .as_str()
            .expect("finding")
            .contains("do not establish native recipe feasibility")
    );
    view["production_control_enabled"] = json!(false);
    assert!(
        render(anchor, &view, "control", true, false)["agent_turn"]["recommendations"]
            .as_array()
            .expect("recommendations")
            .iter()
            .all(|r| r["tool"] != "fortress_plan")
    );
}

#[test]
fn stale_diagnosis_cannot_skip_inspection_of_the_current_preplan_source() {
    let (anchor, original) = fixture();
    for case in 0..4 {
        let mut view = original.clone();
        let source = &view["production_inspection_source"];
        view["production_inspection"] = json!({"kind":"production_diagnosis",
            "anchor":source["anchor"],"source_digest":source["source_digest"],
            "canonical_anchor":anchor_value(anchor),"inspection_scope":"preplan_captured_operations",
            "summary":{"jobs_with_findings":0}});
        let diagnosis = &mut view["production_inspection"];
        match case {
            0 => diagnosis["canonical_anchor"]["state_hash"] = json!("c".repeat(64)),
            1 => diagnosis["anchor"]["sequence"] = json!(1),
            2 => diagnosis["source_digest"] = json!("c".repeat(64)),
            _ => diagnosis["inspection_scope"] = json!("captured_operations"),
        }
        let result = render(anchor, &view, "control", true, false);
        let turn = &result["agent_turn"];
        assert!(
            turn["briefing"]["observed_production_conditions"].is_null(),
            "case {case}"
        );
        assert_eq!(
            turn["recommendations"][0]["tool"], "fortress_query",
            "case {case}"
        );
        assert_eq!(
            turn["briefing"]["original_goal_proven"], false,
            "case {case}"
        );
    }
}

#[test]
fn mismatched_preplan_source_never_yields_current_inspection_or_a_stale_query() {
    let (anchor, original) = fixture();
    for case in 0..15 {
        let mut view = original.clone();
        let source = &mut view["production_inspection_source"];
        match case {
            0 => source["scope"] = json!("captured_operations"),
            1 => source["attribution"] = json!("order_attributed"),
            2 => source["order_step_id"] = json!(1),
            3 => source["parent_anchor"]["state_hash"] = json!("c".repeat(64)),
            4 => source["canonical_anchor"]["sequence"] = json!(6),
            5 => source["anchor"]["fortress_id"] = json!("8"),
            6 => source["anchor"]["tick"] = json!(131),
            7 => source["anchor"]["sequence"] = json!("2"),
            8 => source["anchor"]["state_hash"] = json!("0".repeat(64)),
            9 => source["source_digest"] = json!("0".repeat(64)),
            10 => source["mutation_authority"] = json!(true),
            11 => source["restored_from_history"] = json!(true),
            12 => source["diagnosis_request"]["expected_anchor"] = anchor_value(anchor),
            13 => view["original_plan"] = json!({"plan_digest":"d".repeat(64)}),
            _ => view["observation_tick_floor"] = json!(131),
        }
        let result = render(anchor, &view, "control", true, false);
        let turn = &result["agent_turn"];
        assert_eq!(
            turn["briefing"]["preplan_production_inspection_available"], false,
            "case {case}"
        );
        assert!(
            turn["active_work"]["production_inspection_source"].is_null(),
            "case {case}"
        );
        assert!(
            result["production_inspection_source"].is_null(),
            "case {case}"
        );
        assert!(result["production_inspection"].is_null(), "case {case}");
        assert!(
            turn["recommendations"]
                .as_array()
                .expect("recommendations")
                .iter()
                .all(|r| r["tool"] != "fortress_query"),
            "case {case}"
        );
    }
}

#[test]
fn failures_offline_closure_and_uncertainty_withdraw_preplan_information_guidance() {
    let (anchor, original) = fixture();
    for case in 0..5 {
        let mut view = original.clone();
        let (mode, verified, uncertain) = match case {
            0 => ("control", false, false),
            1 => ("control", true, true),
            2 => ("offline", true, false),
            3 => {
                view["closed"] = json!(true);
                ("control", true, false)
            }
            _ => {
                view["parent_goal_observation_fenced"] = json!(true);
                ("control", true, false)
            }
        };
        let result = render(anchor, &view, mode, verified, uncertain);
        let turn = &result["agent_turn"];
        assert_eq!(
            turn["briefing"]["preplan_production_inspection_available"], false,
            "case {case}"
        );
        assert!(
            turn["active_work"]["production_inspection_source"].is_null(),
            "case {case}"
        );
        assert!(
            turn["recommendations"]
                .as_array()
                .expect("recommendations")
                .iter()
                .all(|r| !matches!(
                    r["tool"].as_str(),
                    Some("fortress_query" | "fortress_observe" | "fortress_plan")
                )),
            "case {case}"
        );
        assert_eq!(turn["briefing"]["original_goal_proven"], false);
    }
}

#[test]
fn preplan_inspection_is_never_reconstructed_by_compaction_or_output_overflow() {
    let (anchor, view) = fixture();
    let retained = compact(&view);
    assert!(retained["production_inspection_source"].is_null());
    assert!(retained["production_inspection"].is_null());
    let (raw, overflow) = packet(
        "fortress.query",
        Some(SessionId::new((1u128 << 127) | (24u128 << 57) | 3)),
        Some(anchor),
        Some("control"),
        None,
        None,
        json!({"ok":true,"payload":"x".repeat(70_000)}),
        Some(&view),
        true,
        false,
    );
    assert!(overflow);
    let result: Value = serde_json::from_str(&raw).expect("overflow packet");
    assert!(result["retained_reference"]["production_inspection_source"].is_null());
    assert_eq!(
        result["agent_turn"]["briefing"]["preplan_production_inspection_available"],
        false
    );
    assert_eq!(result["agent_turn"]["recommendations"], json!([]));
}
