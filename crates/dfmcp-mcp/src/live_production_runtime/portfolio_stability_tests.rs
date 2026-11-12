use super::*;
use dfmcp_core::{FortressId, ObservationCursor};

fn anchor(tick: u64, sequence: u64) -> StateAnchor {
    StateAnchor {
        fortress_id: FortressId::new(7),
        cursor: ObservationCursor { epoch: 3, sequence },
        tick: GameTick(tick),
        state_hash: Digest32::of_bytes(format!("stock-cohort-{tick}-{sequence}").as_bytes()),
    }
}

fn pending() -> Result<ParentStability> {
    ParentStability::new(anchor(120, 0), GameTick(220), 5, 2)
}

#[test]
fn earlier_met_target_regression_requires_a_complete_new_stability_window() -> Result<()> {
    let mut state = pending()?;
    // BED's separate original history can qualify at both 125 and 130. The
    // complete original goal also contains a DOOR target needing no action.
    assert!(!state.observe(anchor(125, 1), PredicateTruth::False)?);
    assert_eq!(state.accepted_samples, 0);
    assert!(!state.observe(anchor(130, 2), PredicateTruth::True)?);
    assert_eq!(state.accepted_samples, 1);
    assert_eq!(state.first_candidate_tick, Some(GameTick(130)));
    assert!(state.needs_time(anchor(130, 2)));
    assert!(state.observe(anchor(135, 3), PredicateTruth::True)?);
    assert_eq!(state.fulfilled_at_tick, Some(GameTick(135)));
    assert!(!state.needs_time(anchor(135, 3)));
    let proof = state.to_json(true);
    assert_eq!(proof["required_samples"], 2);
    assert_eq!(proof["accepted_samples"], 2);
    assert_eq!(proof["first_candidate_tick"], 130);
    assert_eq!(proof["fulfilled_at_tick"], 135);
    assert_eq!(proof["last_sample_tick"], 135);
    assert_eq!(proof["proven_at_anchor"], true);
    Ok(())
}

#[test]
fn repeated_ticks_never_count_and_off_cadence_negative_evidence_interrupts() -> Result<()> {
    for contradiction in [PredicateTruth::False, PredicateTruth::Unknown] {
        let mut state = pending()?;
        assert!(!state.observe(anchor(125, 1), PredicateTruth::True)?);
        assert!(!state.observe(anchor(125, 2), PredicateTruth::True)?);
        assert_eq!(
            state.accepted_samples, 1,
            "a new cursor is not elapsed game time"
        );
        assert!(!state.observe(anchor(126, 3), contradiction)?);
        assert_eq!(state.accepted_samples, 0);
        assert_eq!(state.first_candidate_tick, None);
        assert_eq!(state.last_sample_tick, None);
        assert!(!state.observe(anchor(127, 4), PredicateTruth::True)?);
        assert_eq!(
            state.accepted_samples, 0,
            "an interruption does not renew cadence"
        );
        assert!(!state.observe(anchor(130, 5), PredicateTruth::True)?);
        assert!(state.observe(anchor(135, 6), PredicateTruth::True)?);
    }
    Ok(())
}

#[test]
fn interrupted_or_reopened_parent_does_not_recover_a_leafs_unfinished_proof() -> Result<()> {
    let mut state = pending()?;
    state.observe(anchor(125, 1), PredicateTruth::True)?;
    state.interrupt();
    assert_eq!(state.accepted_samples, 0);
    assert!(!state.observe(anchor(125, 2), PredicateTruth::True)?);
    assert_eq!(state.accepted_samples, 0);
    assert!(!state.observe(anchor(130, 3), PredicateTruth::True)?);
    assert!(state.observe(anchor(135, 4), PredicateTruth::True)?);
    let mut reopened = pending()?;
    assert!(!reopened.observe(anchor(135, 4), PredicateTruth::True)?);
    assert_eq!(reopened.accepted_samples, 1);
    assert!(reopened.observe(anchor(140, 5), PredicateTruth::True)?);
    let mut late_reopen = pending()?;
    assert!(!late_reopen.observe(anchor(225, 6), PredicateTruth::True)?);
    assert_eq!(late_reopen.to_json(false)["status"], "expired");
    assert_eq!(late_reopen.deadline_tick, GameTick(220));
    Ok(())
}

#[test]
fn original_deadline_does_not_supply_a_shortened_or_repeated_final_interval() -> Result<()> {
    let mut on_time = pending()?;
    assert!(!on_time.observe(anchor(215, 1), PredicateTruth::True)?);
    assert!(on_time.observe(anchor(220, 2), PredicateTruth::True)?);
    let mut too_late = pending()?;
    assert!(!too_late.observe(anchor(218, 1), PredicateTruth::True)?);
    assert!(!too_late.observe(anchor(220, 2), PredicateTruth::True)?);
    assert_eq!(too_late.accepted_samples, 0);
    assert!(too_late.expired);
    assert_eq!(too_late.fulfilled_at_tick, None);
    assert!(!too_late.observe(anchor(225, 3), PredicateTruth::True)?);
    assert!(!too_late.needs_time(anchor(225, 3)));
    Ok(())
}

#[test]
fn achieved_parent_history_requires_fresh_truth_and_keeps_its_original_achievement_tick()
-> Result<()> {
    let mut state = pending()?;
    state.observe(anchor(125, 1), PredicateTruth::True)?;
    assert!(state.observe(anchor(130, 2), PredicateTruth::True)?);
    assert!(!state.observe(anchor(135, 3), PredicateTruth::False)?);
    assert_eq!(state.to_json(false)["status"], "fulfilled");
    assert_eq!(state.to_json(false)["proven_at_anchor"], false);
    state.interrupt();
    assert!(state.observe(anchor(225, 4), PredicateTruth::True)?);
    assert_eq!(state.fulfilled_at_tick, Some(GameTick(130)));
    assert_eq!(state.first_candidate_tick, Some(GameTick(125)));
    assert_eq!(state.last_sample_tick, Some(GameTick(225)));
    assert!(!state.needs_time(anchor(225, 4)));
    Ok(())
}

#[test]
fn lineage_regression_and_same_cursor_forks_cannot_advance_parent_stability() -> Result<()> {
    for case in 0..4 {
        let mut state = pending()?;
        state.observe(anchor(125, 1), PredicateTruth::True)?;
        let mut rejected = anchor(130, 2);
        match case {
            0 => rejected.fortress_id = FortressId::new(8),
            1 => rejected.cursor.epoch += 1,
            2 => rejected.tick = GameTick(124),
            _ => rejected.cursor.sequence = 1,
        }
        assert!(
            state.observe(rejected, PredicateTruth::True).is_err(),
            "case {case}"
        );
        assert_eq!(state.accepted_samples, 0);
        assert_eq!(state.fulfilled_at_tick, None);
        assert!(!state.observe(anchor(130, 2), PredicateTruth::True)?);
        assert!(state.observe(anchor(135, 3), PredicateTruth::True)?);
    }
    assert!(ParentStability::new(anchor(120, 0), GameTick(120), 5, 2).is_err());
    assert!(ParentStability::new(anchor(120, 0), GameTick(220), 0, 2).is_err());
    assert!(ParentStability::new(anchor(120, 0), GameTick(220), 5, 0).is_err());
    Ok(())
}

#[test]
fn initial_clock_basis_requires_the_exact_current_cohort_member_without_anchor_relabeling() {
    let current = anchor(130, 9);
    let selected = anchor(130, 3);
    let mut parent = ProductionPortfolioGoalProgress {
        anchor: current,
        selected_anchors: vec![(StepId::ZERO, selected)],
        current_truth: PredicateTruth::True,
        original_goal_proven: false,
        source_digest: Digest32::of_bytes(b"coherent-stock-and-workers"),
        workforce_source_digest: Digest32::of_bytes(b"current-workers"),
        labor: Vec::new(),
        inventory_requirements: Vec::new(),
    };
    assert!(cohort_covers_selected(
        &parent,
        current,
        StepId::ZERO,
        selected
    ));
    assert!(!cohort_covers_selected(
        &parent,
        current,
        StepId::new(1),
        selected
    ));
    assert!(!cohort_covers_selected(
        &parent,
        anchor(130, 10),
        StepId::ZERO,
        selected
    ));
    let mut wrong_source = selected;
    wrong_source.state_hash = Digest32::of_bytes(b"another-same-tick-source");
    assert!(!cohort_covers_selected(
        &parent,
        current,
        StepId::ZERO,
        wrong_source
    ));
    assert_ne!(
        current, selected,
        "the complete parent and selected projection keep different anchors"
    );
    for truth in [PredicateTruth::False, PredicateTruth::Unknown] {
        parent.current_truth = truth;
        assert!(!cohort_covers_selected(
            &parent,
            current,
            StepId::ZERO,
            selected
        ));
    }
}

fn assigned_portfolio() -> Result<PreparedPlan> {
    use super::super::portfolio_request::PortfolioRequest;
    use super::super::stock_target_request::tests::{stock_fixture, stock_policy};
    use dfmcp_world::{CompareOp, EntityKind, Predicate, Value as FactValue};

    let (projection, mut c) = stock_fixture()?;
    c.budget.max_actions = 3;
    let evidence =
        PredicateEvidence::scoped(&projection.snapshot, stock_policy(&projection.snapshot))?;
    let request = PortfolioRequest::parse(
        &json!({
            "kind":"portfolio", "expected_observation":"11".repeat(32),
            "units":["43","44"], "labor":"CARPENTER", "enabled":true,
            "orders":[{"recipe":"wooden_bed","amount":4},{"recipe":"wooden_door","amount":2}],
            "deadline_tick":300, "poll_interval_ticks":5, "stable_samples":2
        })
        .to_string(),
    )?;
    let mut plan = request.compile(&evidence, &c)?;
    let mut terminal = vec![plan.terminal_condition.clone()];
    for (item_type, minimum) in [("BED", 6), ("DOOR", 3)] {
        terminal.push(Predicate::EntityQuantity {
            kind: EntityKind::Item,
            filter_field: "type_key".into(),
            filter_value: FactValue::Text(item_type.into()),
            quantity_field: Some("stack_size".into()),
            op: CompareOp::Ge,
            value: minimum,
        });
    }
    plan.terminal_condition = Predicate::All(terminal).normalized();
    plan.digest = plan.compute_digest();
    plan.id = plan.expected_id();
    plan.validate_structure()?;
    Ok(plan)
}

#[test]
fn assigned_parent_stability_uses_order_timing_without_requiring_a_labor_obligation() -> Result<()>
{
    let plan = assigned_portfolio()?;
    let layout = live_production::ProductionLayout::from_plan(&plan)?;
    assert_eq!(layout.assignment_step(), Some(StepId::ZERO));
    assert_eq!(layout.inventory_minima().len(), 2);
    assert!(plan.steps[0].obligation.is_none());
    let mut state = ParentStability::for_plan(&plan)?;
    assert_eq!(state.original_anchor, plan.anchor);
    assert_eq!(state.deadline_tick, GameTick(300));
    assert_eq!(state.poll_interval_ticks, 5);
    assert_eq!(state.required_samples, 2);
    let mut current = plan.anchor;
    current.tick = GameTick(plan.anchor.tick.get() + 5);
    current.cursor.sequence += 1;
    current.state_hash = Digest32::of_bytes(b"first complete assigned portfolio");
    assert!(!state.observe(current, PredicateTruth::True)?);
    current.tick = GameTick(current.tick.get() + 5);
    current.cursor.sequence += 1;
    current.state_hash = Digest32::of_bytes(b"second complete assigned portfolio");
    assert!(state.observe(current, PredicateTruth::True)?);
    Ok(())
}

#[test]
fn assigned_parent_stability_refuses_mismatched_original_order_timing() -> Result<()> {
    let original = assigned_portfolio()?;
    for case in 0..3 {
        let mut plan = original.clone();
        let timing = plan.steps[2]
            .obligation
            .as_mut()
            .ok_or_else(|| denied("original fixture order lacks its obligation"))?;
        match case {
            0 => timing.deadline_tick = GameTick(timing.deadline_tick.get() + 1),
            1 => timing.poll_interval_ticks += 1,
            _ => timing.stable_for_observations += 1,
        }
        plan.digest = plan.compute_digest();
        plan.id = plan.expected_id();
        plan.validate_structure()?;
        assert!(ParentStability::for_plan(&plan).is_err(), "case {case}");
    }
    Ok(())
}
