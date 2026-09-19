use super::*;
use crate::work_order_control::tests::{context, plan, setup};
use crate::work_order_control::WorkOrderState;

#[test]
fn selected_queue_seals_exact_intent_and_replay_cannot_duplicate_creation() -> Result<()> {
    let (j, source) = setup()?; let mut c = context()?;
    let mut session = WorkOrderSession::new(j, Some(source), &c)?;
    let o = session.observe(&c)?; c.anchor.tick = GameTick(o.tick()); c.anchor.state_hash = o.witness();
    let spec = plan("order-001")?.spec();
    let prepared = session.plan("order-001", spec, o.witness(), &c)?;
    assert_eq!(session.plan("order-001", spec, o.witness(), &c)?, prepared);
    assert!(session.plan("order-001", WorkOrderSpec::new(spec.recipe(), 6)?, o.witness(), &c).is_err());
    let created = session.commit("order-001", prepared.plan().digest(), o.witness(), &c)?;
    assert!(session.selected(&c)?.is_none());
    assert_eq!(session.commit("order-001", prepared.plan().digest(), o.witness(), &c)?, created);
    let source = session.source.as_ref().ok_or_else(exhausted)?;
    assert_eq!(source.prepares, 1); assert_eq!(source.commits, 1);
    Ok(())
}

#[test]
fn failed_refresh_clears_selection_and_session_rebinding_is_rejected() -> Result<()> {
    let (j, source) = setup()?; let c = context()?;
    let mut session = WorkOrderSession::new(j, Some(source), &c)?;
    let o = session.observe(&c)?;
    session.source.as_mut().ok_or_else(exhausted)?.read_fails = true;
    assert!(session.observe(&c).is_err()); assert!(session.selected(&c)?.is_none());
    assert!(session.plan("new", plan("new")?.spec(), o.witness(), &c).is_err());
    let mut other = c; other.session_id = SessionId::new(999);
    assert!(session.summary(&other).is_err());
    Ok(())
}

#[test]
fn query_expiry_is_checked_against_new_observation_not_the_old_anchor_tick() -> Result<()> {
    let (j, source) = setup()?; let mut c = context()?; c.anchor.tick = GameTick(0);
    for grant in &mut c.grants { grant.expires_at_tick = Some(GameTick(1)); }
    let mut session = WorkOrderSession::new(j, Some(source), &c)?;
    assert!(session.observe(&c).is_err()); assert!(session.selected(&c)?.is_none());
    Ok(())
}

#[test]
fn prepared_wait_and_terminal_wait_never_query_or_retire_preparations() -> Result<()> {
    let (j, source) = setup()?; let c = context()?;
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
fn lost_reply_blocks_new_intents_until_query_reconciles_and_a_new_selection_is_acquired() -> Result<()> {
    let (j, mut source) = setup()?; let c = context()?; source.commit_state = None;
    let mut session = WorkOrderSession::new(j, Some(source), &c)?;
    let o = session.observe(&c)?;
    let r = session.plan("order-001", plan("order-001")?.spec(), o.witness(), &c)?;
    assert_eq!(session.commit(r.plan().key(), r.plan().digest(), o.witness(), &c)?.state(), CreationState::Indeterminate);
    session.observe(&c)?;
    assert!(session.plan("new", r.plan().spec(), o.witness(), &c).is_err());
    assert_eq!(session.reconcile(r.plan().key(), r.plan().digest(), &c)?.effect().state(), WorkOrderState::Created);
    assert!(session.plan("new", r.plan().spec(), o.witness(), &c).is_err());
    session.observe(&c)?;
    assert!(session.plan("new", r.plan().spec(), o.witness(), &c).is_ok());
    assert_eq!(session.source.as_ref().ok_or_else(exhausted)?.commits, 1);
    Ok(())
}
