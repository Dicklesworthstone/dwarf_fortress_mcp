// Included only by the library unit module: the integration target links the
// ordinary production library, whose real monotonic clock has no test controls.
use crate::semantic_workforce::evidence_owner::TestClock;

#[test]
fn over_deadline_reads_close_without_retries_or_old_scope() -> Result<()> {
    let _clock = TestClock::start();
    let f = fixture()?;
    let (mut owner, state) = evidence_owner(f.observer.capsule, &f.context, small_limits())?;
    let mut c = f.context.clone();
    state.borrow_mut().after_read = Some(Box::new(|| TestClock::advance(Duration::from_millis(11))));
    c.budget.max_wall_millis = 10;
    let before = state.borrow().reads;
    let opens = state.borrow().opens;
    let authorizations = state.borrow().authorizations;
    let failure = owner.refresh(&c).expect_err("injected read must refuse");
    assert_eq!(failure.code, ErrorCode::BudgetExceeded);
    assert!(matches!(owner.routing_evidence(), Err(e) if e.code == ErrorCode::AdapterUnavailable));
    assert_eq!(state.borrow().reads, before + 1);
    assert_eq!(state.borrow().opens, opens + 1);
    assert_eq!(state.borrow().authorizations, authorizations);
    assert_eq!(state.borrow().active, 0);
    Ok(())
}

#[test]
fn pre_io_deadline_refusal_withdraws_without_reads_and_clock_restores_on_unwind() -> Result<()> {
    let _clock = TestClock::start();
    let f = fixture()?;
    let (mut owner, state) = evidence_owner(f.observer.capsule, &f.context, small_limits())?;
    let unwound = std::panic::catch_unwind(|| {
        let _nested = TestClock::start();
        TestClock::advance(Duration::from_secs(60));
        panic!("intentional nested fixture clock unwind");
    });
    assert!(unwound.is_err());
    // An unrestored inner clock would expire this original publication.
    assert!(owner.routing_evidence().is_ok());
    let before = {
        let data = state.borrow();
        (data.opens, data.reads, data.authorizations)
    };
    let mut c = f.context.clone();
    c.budget.max_wall_millis = 10;
    // The next now establishes the original deadline; subsequent validation
    // sees expiry, before the source factory or page reader can be called.
    TestClock::advance_after_next_now(Duration::from_millis(11));
    let failure = owner.refresh(&c).expect_err("pre-I/O deadline must refuse");
    assert_eq!(failure.code, ErrorCode::BudgetExceeded);
    assert!(matches!(owner.routing_evidence(), Err(e) if e.code == ErrorCode::AdapterUnavailable));
    let data = state.borrow();
    assert_eq!((data.opens, data.reads, data.authorizations), before);
    assert_eq!(data.active, 0);
    Ok(())
}

