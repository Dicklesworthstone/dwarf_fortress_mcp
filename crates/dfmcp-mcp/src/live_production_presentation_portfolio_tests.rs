use super::*;
use dfmcp_core::{Digest32, FortressId, GameTick, ObservationCursor};

fn fixture(assignment: bool) -> (StateAnchor, Value) {
    let anchor = StateAnchor { fortress_id: FortressId::new(7), cursor: ObservationCursor { epoch:1, sequence:9 },
        tick:GameTick(130), state_hash:Digest32::of_bytes(b"complete existing worker portfolio") };
    let mut steps = if assignment { vec![json!({"step_id":0,"action":{"kind":"set_labor"},"depends_on":[]})] } else { vec![] };
    let mut orders = Vec::new();
    let mut anchors = Vec::new();
    for ordinal in 0..2 {
        let id = ordinal + usize::from(assignment);
        let mut child = anchor_value(anchor); child["sequence"] = (ordinal + 3).into();
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
    (anchor,json!({"original_plan":{"steps":steps},"portfolio":{"schema":"dfmcp.production-portfolio/1",
        "selected_order_step_id":usize::from(assignment),"orders":orders,"complete_original_goal_proven":true},
        "current_parent_goal":parent,"original_goal_proven":true}))
}

#[test]
fn complete_parent_presentation_accepts_each_exact_original_layout() {
    for assignment in [false,true] {
        let (anchor,view) = fixture(assignment);
        assert!(portfolio_parent_valid(&view,&view["current_parent_goal"],Some(anchor)));
        let session = SessionId::new((1u128 << 127) | (24u128 << 57) | 1);
        let (raw,overflow) = packet("fortress.wait",Some(session),Some(anchor),Some("control"),None,None,
            json!({"ok":true}),Some(&view),true,false);
        assert!(!overflow);
        let rendered: Value = serde_json::from_str(&raw).expect("packet");
        assert_eq!(rendered["agent_turn"]["briefing"]["original_goal_proven"],true);
        assert_eq!(rendered["agent_turn"]["briefing"]["current_state_proven"],false);
        let compacted = compact(&view);
        assert_eq!(compacted["original_goal_proven"],false);
        assert_eq!(compacted["portfolio"]["orders"][0]["step_id"],usize::from(assignment));
    }
}

#[test]
fn shifted_partial_or_synthetic_dependency_portfolio_proof_is_withheld() {
    let (anchor,view) = fixture(false);
    for changed in [
        { let mut v=view.clone(); v["portfolio"]["orders"][0]["step_id"]=1.into(); v },
        { let mut v=view.clone(); v["current_parent_goal"]["selected_anchors"][0]["step_id"]=1.into(); v },
        { let mut v=view.clone(); v["original_plan"]["steps"][0]["depends_on"]=json!([0]); v },
        { let mut v=view.clone(); v["original_plan"]["steps"][1]["step_id"]=0.into(); v },
        { let mut v=view.clone(); v["portfolio"]["orders"].as_array_mut().expect("orders").pop(); v },
        { let mut v=view.clone(); v["portfolio"]["orders"][1]["goal"]["deadline_tick"]=301.into(); v },
        { let mut v=view.clone(); v["current_parent_goal"]["labor"][0]["observed_enabled"]=false.into(); v },
    ] {
        assert!(!portfolio_parent_valid(&changed,&changed["current_parent_goal"],Some(anchor)),"{changed}");
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
            assert!(!portfolio_parent_valid(&view, &view["current_parent_goal"], Some(anchor)));
        }
    }
}
