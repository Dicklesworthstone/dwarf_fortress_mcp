use super::*;
use crate::semantic_work_orders::NativeOrderCreation;

fn native_calls(f: &ProductionFixture) -> ([usize; 5], [usize; 4]) {
    let orders = f.orders.borrow();
    (
        f.workforce.data.borrow().calls,
        [
            orders.reads,
            orders.prepares,
            orders.commits,
            orders.queries,
        ],
    )
}

fn custody_bytes(f: &ProductionFixture) -> Vec<Vec<u8>> {
    [
        &f.workforce.journal,
        &f.workforce.associations,
        &f.order_journal,
        &f.order_associations,
    ]
    .into_iter()
    .map(|storage| storage.0.borrow().file.get_ref().clone())
    .collect()
}

#[test]
fn selected_clock_custody_is_query_only_exact_and_has_no_native_or_file_effects() -> Result<()> {
    let mut f = ProductionFixture::new(1)?;
    f.applied(0)?;
    f.applied(1)?;
    let calls = native_calls(&f);
    let bytes = custody_bytes(&f);
    let mut query = f.c.clone();
    query
        .grants
        .retain(|grant| grant.capability == Capability::Query);
    let inspected = f
        .executor
        .inspect_selected_order_custody(StepId::new(1), &query)?;
    assert_eq!(inspected.review().original_plan(), &f.plan);
    assert_eq!(inspected.review().step_id(), StepId::new(1));
    assert_eq!(inspected.review().selected_step(), &f.plan.steps[1]);
    assert_eq!(inspected.native_record().state(), CreationState::Created);
    assert_eq!(
        inspected.native_record().plan().key(),
        f.plan.steps[1].idempotency_key
    );
    let NativeOrderCreation::Inserted {
        native_order_id,
        receipt,
    } = inspected.creation_result()
    else {
        return Err(invalid("selected fixture insertion must be verified"));
    };
    assert_eq!(
        inspected.native_record().effect().created_order_id(),
        Some(*native_order_id)
    );
    assert_eq!(inspected.native_record().effect().receipt(), Some(*receipt));
    assert_eq!(
        inspected.pending_obligation(),
        f.plan.steps[1].obligation.as_ref()
    );
    assert!(!inspected.original_goal_proven());
    assert_eq!(native_calls(&f), calls);
    assert_eq!(custody_bytes(&f), bytes);
    assert!(f.executor.prepared.is_none());
    Ok(())
}

#[test]
fn selected_clock_custody_recovers_without_sources_or_restoring_dispatch_authority() -> Result<()> {
    let mut f = ProductionFixture::new(1)?;
    f.applied(0)?;
    f.applied(1)?;
    let expected = f
        .executor
        .inspect_selected_order_custody(StepId::new(1), &f.c)?;
    let mut f = f.reopen_query()?;
    f.workforce.data.borrow_mut().fail_connect = true;
    let calls = native_calls(&f);
    let bytes = custody_bytes(&f);
    assert!(f.executor.evidence.is_none());
    assert!(f.executor.latest_step_evidence().is_none());
    let inspected = f
        .executor
        .inspect_selected_order_custody(StepId::new(1), &f.c)?;
    assert_eq!(inspected.review().seal(), expected.review().seal());
    assert_eq!(inspected.native_record(), expected.native_record());
    assert_eq!(inspected.creation_result(), expected.creation_result());
    assert_eq!(inspected.review().original_plan(), &f.plan);
    assert!(f.executor.prepared.is_none());
    assert!(f.executor.latest_step_evidence().is_none());
    assert!(f.prepare(1).is_err());
    assert_eq!(native_calls(&f), calls);
    assert_eq!(custody_bytes(&f), bytes);
    Ok(())
}

#[test]
fn selected_clock_custody_refuses_absent_prepared_and_uncertain_original_insertions() -> Result<()>
{
    let mut f = ProductionFixture::new(1)?;
    f.applied(0)?;
    let calls = native_calls(&f);
    for step in [StepId::ZERO, StepId::new(1), StepId::new(2)] {
        assert!(
            f.executor
                .inspect_selected_order_custody(step, &f.c)
                .is_err()
        );
    }
    assert_eq!(native_calls(&f), calls);
    f.prepare(1)?;
    let calls = native_calls(&f);
    assert!(
        f.executor
            .inspect_selected_order_custody(StepId::new(1), &f.c)
            .is_err()
    );
    assert!(f.executor.prepared.is_none());
    assert_eq!(native_calls(&f), calls);

    let mut uncertain = ProductionFixture::new(1)?;
    uncertain.applied(0)?;
    let prepared = uncertain.prepare(1)?;
    uncertain.orders.borrow_mut().lose_reply = true;
    assert!(uncertain.commit(1, prepared).is_err());
    let calls = native_calls(&uncertain);
    assert!(
        uncertain
            .executor
            .inspect_selected_order_custody(StepId::new(1), &uncertain.c)
            .is_err()
    );
    assert_eq!(native_calls(&uncertain), calls);
    assert_eq!(uncertain.orders.borrow().commits, 1);
    Ok(())
}

#[test]
fn selected_clock_custody_rejects_revoked_query_and_corrupt_predecessor_without_replay()
-> Result<()> {
    let mut f = ProductionFixture::new(1)?;
    f.applied(0)?;
    f.applied(1)?;
    let floor = f.executor.high_tick();
    let calls = native_calls(&f);
    let mut revoked = f.c.clone();
    revoked.grants.clear();
    assert!(
        f.executor
            .inspect_selected_order_custody(StepId::new(1), &revoked)
            .is_err()
    );
    assert_eq!(native_calls(&f), calls);
    f.workforce.associations.0.borrow_mut().file.get_mut()[0] ^= 1;
    let bytes = custody_bytes(&f);
    assert!(
        f.executor
            .inspect_selected_order_custody(StepId::new(1), &f.c)
            .is_err()
    );
    assert!(f.executor.high_tick() >= floor);
    assert!(f.executor.prepared.is_none());
    assert!(f.executor.latest_step_evidence().is_none());
    assert_eq!(native_calls(&f), calls);
    assert_eq!(custody_bytes(&f), bytes);
    Ok(())
}


#[test]
fn shared_clock_custody_preserves_nonzero_selection_deadline_and_identity_bytes() -> Result<()> {
    use crate::live_production::SelectedOrderClockCustody;

    let mut f = ProductionFixture::new(2)?;
    f.applied(0)?;
    f.applied(1)?;
    f.applied(2)?;
    let archive = validate_plan(&f.plan)?;
    let journal = WorkOrderJournal::open(
        f.order_journal.clone(),
        &f.c,
        JournalMode::Offline,
        false,
    )?
    .summary(&f.c)?
    .journal_id;
    let calls = native_calls(&f);
    let bytes = custody_bytes(&f);
    let mut identities = Vec::new();
    assert!(f.plan.steps[0].obligation.is_none());
    for index in 1..=2 {
        let result = f
            .executor
            .inspect_selected_order_custody(StepId::new(index as u32), &f.c)?;
        let custody = SelectedOrderClockCustody::new(&f.plan, archive, journal, &result)?;
        let NativeOrderCreation::Inserted {
            native_order_id,
            receipt,
        } = result.creation_result()
        else {
            return Err(invalid("fixture must retain its verified Created receipt"));
        };
        assert_eq!(custody.step_id(), f.plan.steps[index].id);
        assert_ne!(custody.step_id(), StepId::ZERO);
        assert_eq!(custody.original_anchor(), f.plan.anchor);
        assert_eq!(custody.deadline_tick(), GameTick(220));
        assert_eq!(custody.review_seal(), result.review().seal());
        assert_eq!(custody.native_plan(), result.review().native_plan());
        assert_eq!(custody.native_order_id(), *native_order_id);

        // The existing order-clock target's exact domain, widths and field
        // order are a durable identity contract, including the original StepId.
        let fields = [
            f.plan.digest.as_bytes().to_vec(),
            archive.object_digest.as_bytes().to_vec(),
            journal.as_bytes().to_vec(),
            (index as u32).to_be_bytes().to_vec(),
            f.plan.steps[index].idempotency_key.as_bytes().to_vec(),
            result.review().seal().as_bytes().to_vec(),
            receipt.as_bytes().to_vec(),
            native_order_id.to_be_bytes().to_vec(),
        ];
        let mut previous = b"dfmcp-combined-order-clock-target/1\0".to_vec();
        for field in fields {
            previous.extend_from_slice(&(field.len() as u64).to_be_bytes());
            previous.extend_from_slice(&field);
        }
        let mut shared = b"dfmcp-combined-order-clock-target/1\0".to_vec();
        shared.extend_from_slice(custody.identity_bytes());
        assert_eq!(shared, previous);
        identities.push(shared);
    }
    assert_ne!(identities[0], identities[1]);
    assert_eq!(native_calls(&f), calls);
    assert_eq!(custody_bytes(&f), bytes);
    assert!(!f.executor.original_goal_proven());
    assert!(
        f.c.grants
            .iter()
            .all(|grant| grant.capability != Capability::ControlClock)
    );
    Ok(())
}

#[test]
fn shared_clock_custody_rejects_uncreated_results_and_foreign_parent_bindings() -> Result<()> {
    use crate::live_production::SelectedOrderClockCustody;

    let mut f = ProductionFixture::new(1)?;
    f.applied(0)?;
    let prepared = f.prepare(1)?;
    let archive = validate_plan(&f.plan)?;
    let journal = WorkOrderJournal::open(
        f.order_journal.clone(),
        &f.c,
        JournalMode::Offline,
        false,
    )?
    .summary(&f.c)?
    .journal_id;
    let Some(StepEvidence::WorkOrder(result)) = f.executor.latest_step_evidence() else {
        return Err(invalid("fixture must retain its original prepared order"));
    };
    assert_eq!(result.native_record().state(), CreationState::Prepared);
    assert!(SelectedOrderClockCustody::new(&f.plan, archive, journal, result).is_err());

    f.commit(1, prepared)?;
    let result = f
        .executor
        .inspect_selected_order_custody(StepId::new(1), &f.c)?;
    let calls = native_calls(&f);
    let bytes = custody_bytes(&f);
    let mut wrong_archive = archive;
    wrong_archive.object_digest = Digest32::of_bytes(b"another original plan archive");
    assert!(SelectedOrderClockCustody::new(&f.plan, wrong_archive, journal, &result).is_err());
    assert!(SelectedOrderClockCustody::new(&f.plan, archive, Digest32::ZERO, &result).is_err());
    let other = ProductionFixture::new(2)?;
    assert!(
        SelectedOrderClockCustody::new(
            &other.plan,
            validate_plan(&other.plan)?,
            journal,
            &result,
        )
        .is_err()
    );
    assert_eq!(native_calls(&f), calls);
    assert_eq!(custody_bytes(&f), bytes);
    assert!(!result.original_goal_proven());
    Ok(())
}

#[test]
fn retained_order_custody_preserves_another_original_order_preparation() -> Result<()> {
    let mut f = ProductionFixture::new(2)?;
    f.applied(0)?;
    f.applied(1)?;
    let prepared = f.prepare(2)?;
    let original_preparation = f.executor.prepared;
    let calls = native_calls(&f);
    let bytes = custody_bytes(&f);
    let mut query = f.c.clone();
    query.grants.retain(|grant| grant.capability == Capability::Query);
    let inspected = f.executor.inspect_retained_order_custody(StepId::new(1), &query)?;
    assert_eq!(inspected.review().original_plan(), &f.plan);
    assert_eq!(inspected.review().step_id(), StepId::new(1));
    assert_eq!(inspected.native_record().state(), CreationState::Created);
    assert_eq!(f.executor.prepared, original_preparation);
    assert_eq!(native_calls(&f), calls);
    assert_eq!(custody_bytes(&f), bytes);
    let Some(StepEvidence::WorkOrder(latest)) = f.executor.latest_step_evidence() else {
        return Err(invalid("retained inspection replaced the prepared step evidence"));
    };
    assert_eq!(latest.review().step_id(), StepId::new(2));
    assert_eq!(latest.native_record().state(), CreationState::Prepared);
    // This exercises the real native owner's still-current queue selection and
    // exact single-use preparation, not just the executor's bookkeeping.
    f.commit(2, prepared)?;
    assert_eq!(f.orders.borrow().commits, 2);
    assert!(f.executor.prepared.is_none());
    Ok(())
}

#[test]
fn retained_order_custody_recovery_is_read_only_and_rejects_uncreated_steps() -> Result<()> {
    let mut f = ProductionFixture::new(2)?;
    f.applied(0)?;
    f.applied(1)?;
    let expected = f.executor.inspect_retained_order_custody(StepId::new(1), &f.c)?;
    let mut f = f.reopen_query()?;
    let calls = native_calls(&f);
    let bytes = custody_bytes(&f);
    let inspected = f.executor.inspect_retained_order_custody(StepId::new(1), &f.c)?;
    assert_eq!(inspected.review().seal(), expected.review().seal());
    assert_eq!(inspected.native_record(), expected.native_record());
    for step in [StepId::ZERO, StepId::new(2), StepId::new(3)] {
        assert!(f.executor.inspect_retained_order_custody(step, &f.c).is_err());
    }
    assert!(f.executor.prepared.is_none());
    assert!(f.prepare(2).is_err());
    assert_eq!(native_calls(&f), calls);
    assert_eq!(custody_bytes(&f), bytes);
    assert!(!inspected.original_goal_proven());
    Ok(())
}

#[test]
fn retained_order_custody_refuses_corrupt_association_without_native_contact() -> Result<()> {
    let mut f = ProductionFixture::new(2)?;
    f.applied(0)?;
    f.applied(1)?;
    f.prepare(2)?;
    f.order_associations.0.borrow_mut().file.get_mut()[0] ^= 1;
    let calls = native_calls(&f);
    let bytes = custody_bytes(&f);
    assert!(f.executor.inspect_retained_order_custody(StepId::new(1), &f.c).is_err());
    assert!(f.executor.prepared.is_none());
    assert_eq!(native_calls(&f), calls);
    assert_eq!(custody_bytes(&f), bytes);
    Ok(())
}
