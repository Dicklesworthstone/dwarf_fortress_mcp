use super::*;
use crate::work_order_control::WorkOrderState;
use crate::work_order_control::tests::{context, plan, setup};

#[test]
fn selected_queue_seals_exact_intent_and_replay_cannot_duplicate_creation() -> Result<()> {
    let (j, source) = setup()?;
    let mut c = context()?;
    let mut session = WorkOrderSession::new(j, Some(source), &c)?;
    let o = session.observe(&c)?;
    c.anchor.tick = GameTick(o.tick());
    c.anchor.state_hash = o.witness();
    let spec = plan("order-001")?.spec();
    let prepared = session.plan("order-001", spec, o.witness(), &c)?;
    assert_eq!(session.plan("order-001", spec, o.witness(), &c)?, prepared);
    assert!(
        session
            .plan(
                "order-001",
                WorkOrderSpec::new(spec.recipe(), 6)?,
                o.witness(),
                &c
            )
            .is_err()
    );
    let created = session.commit("order-001", prepared.plan().digest(), o.witness(), &c)?;
    assert!(session.selected(&c)?.is_none());
    assert_eq!(
        session.commit("order-001", prepared.plan().digest(), o.witness(), &c)?,
        created
    );
    let source = session.source.as_ref().ok_or_else(exhausted)?;
    assert_eq!(source.prepares, 1);
    assert_eq!(source.commits, 1);
    Ok(())
}

#[test]
fn failed_refresh_clears_selection_and_session_rebinding_is_rejected() -> Result<()> {
    let (j, source) = setup()?;
    let c = context()?;
    let mut session = WorkOrderSession::new(j, Some(source), &c)?;
    let o = session.observe(&c)?;
    session.source.as_mut().ok_or_else(exhausted)?.read_fails = true;
    assert!(session.observe(&c).is_err());
    assert!(session.selected(&c)?.is_none());
    assert!(
        session
            .plan("new", plan("new")?.spec(), o.witness(), &c)
            .is_err()
    );
    let mut other = c;
    other.session_id = SessionId::new(999);
    assert!(session.summary(&other).is_err());
    Ok(())
}

#[test]
fn query_expiry_is_checked_against_new_observation_not_the_old_anchor_tick() -> Result<()> {
    let (j, source) = setup()?;
    let mut c = context()?;
    c.anchor.tick = GameTick(0);
    for grant in &mut c.grants {
        grant.expires_at_tick = Some(GameTick(1));
    }
    let mut session = WorkOrderSession::new(j, Some(source), &c)?;
    assert!(session.observe(&c).is_err());
    assert_eq!(session.high_tick(), plan("order-001")?.observation().tick());
    assert!(session.selected(&c).is_err());
    assert!(session.summary(&c).is_err());
    assert!(session.observe(&c).is_err());
    assert_eq!(session.source.as_ref().ok_or_else(exhausted)?.reads, 1);
    assert!(session.selected(&context()?)?.is_none());
    Ok(())
}

#[test]
fn reopened_native_records_retain_query_and_control_clock_floors() -> Result<()> {
    let current = context()?;
    let p = plan("order-001")?;
    for expiry_capability in [Capability::Query, Capability::ConfigureProduction] {
        let (mut journal, mut source) = setup()?;
        journal.prepare(&mut source, &p, &current)?;
        let mut stale = current.clone();
        stale.anchor.tick = GameTick(0);
        for grant in &mut stale.grants {
            if grant.capability == expiry_capability {
                grant.expires_at_tick = Some(GameTick(1));
            }
        }
        let opening = WorkOrderSession::new(journal, Some(source), &stale);
        if expiry_capability == Capability::Query {
            assert!(opening.is_err());
        } else {
            let mut session = opening?;
            assert_eq!(session.high_tick(), p.observation().tick());
            assert!(session.cancel(p.key(), p.digest(), &stale).is_err());
            assert_eq!(
                session.record(p.key(), p.digest(), &stale)?.state(),
                CreationState::Prepared
            );
            assert_eq!(session.source.as_ref().ok_or_else(exhausted)?.commits, 0);
        }
    }
    Ok(())
}

#[test]
fn later_restore_epoch_can_query_history_but_cannot_adopt_old_native_clock() -> Result<()> {
    let (journal, source) = setup()?;
    let current = context()?;
    let mut session = WorkOrderSession::new(journal, Some(source), &current)?;
    let observation = session.observe(&current)?;
    let prepared = session.plan(
        "order-001",
        plan("order-001")?.spec(),
        observation.witness(),
        &current,
    )?;
    let mut expired = current.clone();
    expired.anchor.tick = GameTick(0);
    for grant in &mut expired.grants {
        grant.expires_at_tick = Some(GameTick(1));
    }
    assert!(session.summary(&expired).is_err());
    let mut restored = expired.clone();
    restored.anchor.cursor.epoch = current.anchor.cursor.epoch + 1;
    assert_eq!(session.summary(&restored)?.records, 1);
    assert_eq!(
        session.record(prepared.plan().key(), prepared.plan().digest(), &restored)?,
        prepared
    );
    assert!(session.observe(&restored).is_err());
    assert!(
        session
            .plan("new", plan("new")?.spec(), observation.witness(), &restored)
            .is_err()
    );
    assert!(
        session
            .commit(
                prepared.plan().key(),
                prepared.plan().digest(),
                observation.witness(),
                &restored
            )
            .is_err()
    );
    assert!(session.summary(&expired).is_err());
    assert_eq!(session.high_tick(), observation.tick());
    let source = session.source.as_ref().ok_or_else(exhausted)?;
    assert_eq!(source.reads, 1);
    assert_eq!(source.prepares, 1);
    assert_eq!(source.commits, 0);
    assert_eq!(source.queries, 0);
    Ok(())
}

#[test]
fn decoded_queue_floor_survives_source_identity_refusal() -> Result<()> {
    let (journal, mut source) = setup()?;
    let mut c = context()?;
    c.anchor.tick = GameTick(0);
    source.manifest.generation += 1;
    let mut session = WorkOrderSession::new(journal, Some(source), &c)?;
    assert!(session.observe(&c).is_err());
    assert_eq!(session.high_tick(), plan("order-001")?.observation().tick());
    assert!(session.selected(&context()?)?.is_none());
    assert!(session.source.as_ref().ok_or_else(exhausted)?.fenced);
    Ok(())
}

#[test]
fn prepared_wait_and_terminal_wait_never_query_or_retire_preparations() -> Result<()> {
    let (j, source) = setup()?;
    let c = context()?;
    let mut session = WorkOrderSession::new(j, Some(source), &c)?;
    let o = session.observe(&c)?;
    let r = session.plan("order-001", plan("order-001")?.spec(), o.witness(), &c)?;
    assert_eq!(session.reconcile(r.plan().key(), r.plan().digest(), &c)?, r);
    let r = session.cancel(r.plan().key(), r.plan().digest(), &c)?;
    assert_eq!(session.reconcile(r.plan().key(), r.plan().digest(), &c)?, r);
    assert_eq!(session.source.as_ref().ok_or_else(exhausted)?.queries, 0);
    Ok(())
}

#[test]
fn lost_reply_blocks_new_intents_until_query_reconciles_and_a_new_selection_is_acquired()
-> Result<()> {
    let (j, mut source) = setup()?;
    let c = context()?;
    source.commit_state = None;
    let mut session = WorkOrderSession::new(j, Some(source), &c)?;
    let o = session.observe(&c)?;
    let r = session.plan("order-001", plan("order-001")?.spec(), o.witness(), &c)?;
    assert_eq!(
        session
            .commit(r.plan().key(), r.plan().digest(), o.witness(), &c)?
            .state(),
        CreationState::Indeterminate
    );
    session.observe(&c)?;
    assert!(
        session
            .plan("new", r.plan().spec(), o.witness(), &c)
            .is_err()
    );
    assert_eq!(
        session
            .reconcile(r.plan().key(), r.plan().digest(), &c)?
            .effect()
            .state(),
        WorkOrderState::Created
    );
    assert!(
        session
            .plan("new", r.plan().spec(), o.witness(), &c)
            .is_err()
    );
    session.observe(&c)?;
    assert!(
        session
            .plan("new", r.plan().spec(), o.witness(), &c)
            .is_ok()
    );
    assert_eq!(session.source.as_ref().ok_or_else(exhausted)?.commits, 1);
    Ok(())
}
