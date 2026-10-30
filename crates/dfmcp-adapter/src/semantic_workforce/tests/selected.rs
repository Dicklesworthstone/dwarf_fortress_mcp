//! Real family-owner regressions for a selected step of the unchanged parent.
use super::*;
use crate::semantic_selection::SelectedSemanticStep;

fn mixed(f: &Fixture) -> PreparedPlan {
    let mut original = f.plan.clone();
    let mut selected = original.steps[0].clone();
    selected.id = StepId::new(1);
    selected.depends_on = vec![StepId::ZERO];
    selected.idempotency_key = derive_step_idempotency_key(
        original.intent_id,
        original.anchor,
        selected.id,
        &selected.action,
    );
    let action = Action::CreateWorkOrder {
        name: "Original wooden bed pursuit".to_owned(),
        job_token: "wooden_bed".to_owned(),
        amount: 2,
        conditions: Vec::new(),
    }
    .normalized();
    let mut first = selected.clone();
    first.id = StepId::ZERO;
    first.idempotency_key =
        derive_step_idempotency_key(original.intent_id, original.anchor, first.id, &action);
    first.required_capability = action.capability();
    first.risk = action.risk();
    first.action = action;
    first.depends_on.clear();
    first.obligation = Some(ObligationSpec {
        terminal: Predicate::Paused(true),
        failure: None,
        deadline_tick: GameTick(180),
        poll_interval_ticks: 2,
        stable_for_observations: 2,
    });
    original.steps = vec![first, selected];
    original.required_capabilities = original
        .steps
        .iter()
        .map(|s| s.required_capability)
        .collect();
    original.max_risk = original
        .steps
        .iter()
        .map(|s| s.risk)
        .max()
        .map_or(RiskTier::ReadOnly, |risk| risk);
    reseal(&mut original);
    original
}
fn select(f: &mut Fixture, original: PreparedPlan) -> Result<SemanticWorkforceReview> {
    f.owner.observe_selected(
        SelectedSemanticStep::new(original, StepId::new(1))?,
        &mut f.observer,
        &f.context,
        |binding, c| f.native.connect(binding, c),
    )
}
fn prepare_selected(f: &mut Fixture, original: PreparedPlan) -> Result<SemanticWorkforceReview> {
    let review = select(f, original)?;
    f.owner
        .prepare(review.seal(), &mut f.observer, &f.context, |binding, c| {
            f.native.connect(binding, c)
        })?;
    Ok(review)
}

#[test]
fn nonzero_selected_labor_uses_original_key_scope_action_and_obligation() -> Result<()> {
    let mut f = fixture()?;
    let original = mixed(&f);
    original.validate_structure()?;
    assert!(
        f.owner
            .observe(original.clone(), &mut f.observer, &f.context, never)
            .is_err()
    );
    let review = prepare_selected(&mut f, original.clone())?;
    assert_eq!(review.original_plan(), &original);
    assert_eq!(review.step_id(), StepId::new(1));
    assert_eq!(review.selected_step(), &original.steps[1]);
    assert_eq!(
        review.native_plan().key(),
        original.steps[1].idempotency_key
    );
    assert_ne!(
        review.native_plan().key(),
        original.steps[0].idempotency_key
    );
    let result = f.owner.commit(
        review.seal(),
        true,
        &mut f.observer,
        &f.context,
        |binding, c| f.native.connect(binding, c),
    )?;
    assert!(matches!(
        result.action_result(),
        SingleLaborResult::Verified { .. }
    ));
    assert_eq!(
        result.pending_obligation(),
        original.steps[1].obligation.as_ref()
    );
    assert_ne!(
        result.pending_obligation(),
        original.steps[0].obligation.as_ref()
    );
    assert!(!result.original_goal_proven());
    assert_eq!(f.native.data.borrow().calls[2], 1);
    // The legacy monitor cannot prove step zero or the parent from this review.
    let evidence = f.observer.routing_evidence()?;
    assert!(
        f.owner
            .begin_goal_monitor(review.seal(), &evidence, &f.context)
            .is_err()
    );
    let history = crate::semantic_workforce::GoalHistoryStore::open(
        Storage::default(),
        f.owner.associations.native_journal(),
        &review,
        true,
        false,
        &f.context,
    )?;
    let failure = f
        .owner
        .begin_durable_goal_monitor(history, review.seal(), &evidence, &f.context, |_, _| {
            Err(invalid("mixed goal must refuse before policy issuance"))
        })
        .err()
        .ok_or_else(|| invalid("mixed durable goal unexpectedly opened"))?;
    assert_eq!(failure.code, ErrorCode::AdapterRejected);
    assert!(failure.message.contains("requires one step"));
    assert_eq!(f.owner.high_tick(), original.anchor.tick.get());
    Ok(())
}

#[test]
fn selected_recovery_requires_full_original_and_actual_step_after_lost_reply() -> Result<()> {
    let mut f = fixture()?;
    let original = mixed(&f);
    let review = prepare_selected(&mut f, original.clone())?;
    f.native.data.borrow_mut().lose_commit = true;
    assert!(
        f.owner
            .commit(
                review.seal(),
                true,
                &mut f.observer,
                &f.context,
                |binding, c| f.native.connect(binding, c)
            )
            .is_err()
    );
    let mut c = f.context.clone();
    c.session_id = SessionId::new(22);
    c.anchor.tick = original.expires_at_tick;
    c.grants
        .retain(|grant| grant.capability == Capability::Query);
    let journal =
        WorkforceJournal::open(f.native.journal.clone(), &c, WorkforceMode::Recover, None)?;
    let (native, view) = WorkforceSession::new(journal, &c)?;
    let associations =
        AssociationStore::open(f.native.associations.clone(), view.id, false, true, &c)?;
    let mut owner = SemanticWorkforceSession::new(native, associations, &c)?;
    assert!(owner.reattach(original.clone(), &c).is_err());
    let mut changed = original.clone();
    changed.terminal_condition = Predicate::Paused(false);
    reseal(&mut changed);
    assert!(
        owner
            .reattach_selected(SelectedSemanticStep::new(changed, StepId::new(1))?, &c)
            .is_err()
    );
    let attached = owner.reattach_selected(
        SelectedSemanticStep::new(original.clone(), StepId::new(1))?,
        &c,
    )?;
    assert_eq!(attached.seal(), review.seal());
    let result = owner.reconcile(attached.seal(), &c, |binding, current| {
        f.native.connect(binding, current)
    })?;
    assert!(matches!(
        result.action_result(),
        SingleLaborResult::Verified { .. }
    ));
    assert_eq!(result.review().original_plan(), &original);
    assert_eq!(result.review().step_id(), StepId::new(1));
    assert_eq!(f.native.data.borrow().calls[2], 1);
    Ok(())
}

#[test]
fn selected_final_source_guard_keeps_dispatch_uncertain_without_native_commit() -> Result<()> {
    let mut f = fixture()?;
    let original = mixed(&f);
    let review = prepare_selected(&mut f, original)?;
    f.observer.fail_at = Some(5);
    assert!(
        f.owner
            .commit(
                review.seal(),
                true,
                &mut f.observer,
                &f.context,
                |binding, c| f.native.connect(binding, c)
            )
            .is_err()
    );
    assert_eq!(f.native.data.borrow().calls[2], 0);
    let inventory = f.owner.inventory(&f.context)?;
    assert_eq!(
        inventory.records[0].state(),
        AssignmentState::DispatchStarted
    );
    assert!(
        f.owner
            .commit(review.seal(), true, &mut f.observer, &f.context, never)
            .is_err()
    );
    Ok(())
}

#[test]
fn selected_guard_revalidates_entire_parent_before_native_effect() -> Result<()> {
    let mut f = fixture()?;
    let original = mixed(&f);
    let review = prepare_selected(&mut f, original)?;
    // Only this crate's test can touch the private review. The native guard must
    // still validate the unrelated parent's original semantic seal at commit.
    let attached = f
        .owner
        .attached
        .as_mut()
        .ok_or_else(|| invalid("missing review"))?;
    attached.original.steps[0].postconditions = vec![Predicate::Paused(false)];
    assert!(
        f.owner
            .commit(review.seal(), true, &mut f.observer, &f.context, never)
            .is_err()
    );
    assert_eq!(f.native.data.borrow().calls[2], 0);
    assert_eq!(
        f.owner.inventory(&f.context)?.records[0].state(),
        AssignmentState::Prepared
    );
    let attached = f
        .owner
        .attached
        .as_mut()
        .ok_or_else(|| invalid("missing review"))?;
    reseal(&mut attached.original);
    assert!(
        f.owner
            .commit(review.seal(), true, &mut f.observer, &f.context, never)
            .is_err()
    );
    assert_eq!(f.native.data.borrow().calls[2], 0);
    Ok(())
}
