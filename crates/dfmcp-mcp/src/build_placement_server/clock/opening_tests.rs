//! Opening publication repeats the exact local histories after its last runtime
//! callback. These fixtures never contact a running game.
use super::*;

fn reserve(h: &mut Harness) -> Result<(OperationContext, OpeningPublication)> {
    let c = h.state.context(true, None)?;
    let mut work = Work::new(&c, Instant::now())?;
    let clock = work.take(&c, clock::VIEW_BYTES)?;
    let reserved = OpeningPublication::reserve(&h.state, &c, &mut work, clock)?;
    Ok((c, reserved))
}

#[test]
fn opening_rechecks_every_published_history_after_the_last_callback() -> Result<()> {
    for which in ["placements", "batch", "completion", "clock"] {
        let mut h = setup_clock(2)?;
        let (c, reserved) = reserve(&mut h)?;
        let recovery = h.state.clock_recovery;
        let monitor_head = h
            .state
            .completion
            .as_ref()
            .ok_or_else(missing_fixture)?
            .store
            .head();
        let calls = h.io.game.borrow().calls.len();
        let acquisitions = h.io.acquisitions;
        let path = h.directory.file(which);
        let mut reached = false;
        let checked = finalize_opening_publication(&mut h.state, &reserved, recovery, |_| {
            reached = true;
            let mut bytes = fs::read(&path).map_err(storage_error)?;
            bytes.push(0);
            fs::write(&path, bytes).map_err(storage_error)
        });
        assert!(reached, "final callback was not reached for {which}");
        assert!(checked.is_err(), "opening accepted changed {which} custody");
        assert_eq!(h.io.game.borrow().calls.len(), calls);
        assert_eq!(h.io.acquisitions, acquisitions);
        assert_eq!(h.state.clock_recovery, recovery);
        assert_eq!(
            h.state
                .completion
                .as_ref()
                .ok_or_else(missing_fixture)?
                .store
                .head(),
            monitor_head
        );
        let failure = checked.err().ok_or_else(missing_fixture)?;
        let rendered: Value =
            serde_json::from_str(&h.state.failed("fortress.open_session", Some(&c), &failure))
                .map_err(|_| missing_fixture())?;
        assert_eq!(rendered["result"]["ok"], false);
        assert_eq!(rendered["result"]["completion"]["origin_verified"], false);
        assert_eq!(
            rendered["result"]["construction_clock"]["history_verified"],
            false
        );
    }
    Ok(())
}

#[test]
fn opening_preserves_exact_heads_without_native_contact_and_keeps_its_deadline() -> Result<()> {
    let mut h = setup_clock(2)?;
    let (_, mut reserved) = reserve(&mut h)?;
    let recovery = h.state.clock_recovery;
    let monitor_head = h
        .state
        .completion
        .as_ref()
        .ok_or_else(missing_fixture)?
        .store
        .head();
    let calls = h.io.game.borrow().calls.len();
    let acquisitions = h.io.acquisitions;
    let mut callbacks = 0;
    finalize_opening_publication(&mut h.state, &reserved, recovery, |_| {
        callbacks += 1;
        Ok(())
    })?;
    assert_eq!(callbacks, 1);
    assert_eq!(h.state.clock_recovery, recovery);
    assert_eq!(
        h.state
            .completion
            .as_ref()
            .ok_or_else(missing_fixture)?
            .store
            .head(),
        monitor_head
    );
    assert_eq!(h.io.game.borrow().calls.len(), calls);
    assert_eq!(h.io.acquisitions, acquisitions);

    reserved.deadline = Instant::now();
    let expired = finalize_opening_publication(&mut h.state, &reserved, recovery, |_| {
        callbacks += 1;
        Ok(())
    });
    assert!(expired.is_err());
    assert_eq!(
        callbacks, 1,
        "expired opening must not renew the final callback allowance"
    );
    assert_eq!(h.io.game.borrow().calls.len(), calls);
    Ok(())
}

#[test]
fn opening_reserves_the_complete_origin_and_monitor_before_clock_initialization() -> Result<()> {
    let mut h = setup_clock(2)?;
    let c = h.state.context(true, None)?;
    let mut work = Work::new(&c, Instant::now())?;
    let clock = work.take(&c, clock::VIEW_BYTES)?;
    let monitor = h.state.completion.as_ref().ok_or_else(missing_fixture)?;
    work.bytes = completion::source_reserve(monitor.store.definition().origin())?
        + monitor.store.byte_len() as u64
        + 8192
        - 1;
    let calls = h.io.game.borrow().calls.len();
    assert!(OpeningPublication::reserve(&h.state, &c, &mut work, clock).is_err());
    assert_eq!(h.io.game.borrow().calls.len(), calls);
    Ok(())
}

#[test]
fn every_clock_projection_retains_the_shared_consumed_tick_floor() -> Result<()> {
    let mut h = setup_clock(2)?;
    h.state.authority_tick = TICK + 40;
    for mut result in [
        json!({"ok":false}),
        json!({"construction_clock":{"authority_tick_floor":TICK + 1}}),
        json!({"construction_clock":{"authority_tick_floor":TICK + 50}}),
    ] {
        let previous = result["construction_clock"]["authority_tick_floor"]
            .as_u64()
            .map_or(0, |v| v);
        h.state.attach_batch(&mut result, None, false);
        assert_eq!(
            result["construction_clock"]["authority_tick_floor"],
            (TICK + 40).max(previous)
        );
    }
    h.state.clock = None;
    let mut missing_owner = json!({"ok":false});
    h.state.attach_batch(&mut missing_owner, None, false);
    assert_eq!(
        missing_owner["construction_clock"]["authority_tick_floor"],
        TICK + 40
    );
    assert_eq!(
        missing_owner["construction_clock"]["history_verified"],
        false
    );
    Ok(())
}
