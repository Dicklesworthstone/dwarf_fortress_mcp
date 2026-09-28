//! Check the production work quantum without a large or timing-sensitive solver fixture.
use super::*;
use std::cell::Cell;
use std::time::Duration;

#[test]
fn owner_is_checked_before_work_and_at_each_256_unit_boundary() -> Result<()> {
    let calls = Cell::new(0);
    let cancelled = Cell::new(false);
    let mut check = || {
        calls.set(calls.get() + 1);
        if cancelled.get() {
            Err(DfmcpError::new(ErrorCode::CancellationRequested, "owner cancelled"))
        } else {
            Ok(())
        }
    };
    let mut work = Work::with_check(1024, 60_000, &mut check)?;
    assert_eq!(calls.get(), 1);
    for _ in 0..CHECK_INTERVAL - 1 {
        work.charge()?;
    }
    assert_eq!(calls.get(), 1);
    work.charge()?;
    assert_eq!(calls.get(), 2);
    cancelled.set(true);
    for _ in 0..CHECK_INTERVAL - 1 {
        work.charge()?;
    }
    assert_eq!(work.charge().unwrap_err().code, ErrorCode::CancellationRequested);
    assert_eq!(calls.get(), 3);
    assert_eq!(work.used, 2 * CHECK_INTERVAL);
    assert_eq!(work.checkpoint().unwrap_err().code, ErrorCode::CancellationRequested);
    Ok(())
}

#[test]
fn owner_checks_cannot_renew_work_or_wall_allowances() -> Result<()> {
    let mut allow = || Ok(());
    let mut work = Work::with_check(1, 60_000, &mut allow)?;
    work.checkpoint()?;
    work.charge()?;
    work.checkpoint()?;
    assert_eq!(work.charge().unwrap_err().code, ErrorCode::BudgetExceeded);
    assert_eq!(work.checkpoint().unwrap_err().code, ErrorCode::BudgetExceeded);

    let mut work = Work::with_check(1024, 60_000, &mut allow)?;
    work.wall_millis = 1;
    work.started = Instant::now() - Duration::from_millis(2);
    assert_eq!(work.checkpoint().unwrap_err().code, ErrorCode::BudgetExceeded);
    assert_eq!(work.charge().unwrap_err().code, ErrorCode::BudgetExceeded);
    Ok(())
}

#[test]
fn time_spent_in_owner_check_consumes_the_same_deadline() {
    let mut check = || {
        std::thread::sleep(Duration::from_millis(5));
        Ok(())
    };
    // No upper latency assertion: a slow machine can only make this expire.
    let result = Work::with_check(1024, 1, &mut check);
    assert!(matches!(result, Err(error) if error.code == ErrorCode::BudgetExceeded));
}
