//! Real retained owners exclude each other's prepared intervals without native
//! contact, while settled histories continue to permit either original goal.
use super::*;

fn furniture_call(
    h: &mut RoomRunHarness,
    io: &mut ClockIo,
    op: &str,
    action: clock::Action,
) -> Result<Value> {
    let c = h.room.state.context(true, None)?;
    let raw = clock::run_action(&mut h.room.state, c, op, action, Instant::now(), io);
    assert!(raw.len() as u64 <= OUTPUT_BYTES);
    serde_json::from_str(&raw).map_err(|_| exhausted())
}

#[test]
fn settled_room_and_furniture_clocks_coexist_but_prepared_owners_exclude_each_other() -> Result<()>
{
    let mut h = RoomRunHarness::setup(2, 64, false)?;
    let room_id = h.id()?;
    // This helper creates the actual original furniture monitor and private run
    // journal, opens both grant domains and prepares through the public handler.
    // The already retained, settled room clock must permit this preparation.
    let shared_tick = h.room.state.authority_tick.max(h.room.io.tick);
    let mut furniture_io = prepare_original_furniture_clock_at(&mut h.room, shared_tick)?;
    let furniture_id = h
        .room
        .state
        .completion
        .as_ref()
        .ok_or_else(missing_fixture)?
        .store
        .definition()
        .id();
    assert_ne!(room_id, furniture_id);
    assert_eq!(h.clock()?.definition().id(), room_id);
    assert!(
        h.room
            .state
            .clock
            .as_ref()
            .ok_or_else(missing_fixture)?
            .cached_view()
            .pending
    );
    let furniture_pin = h.room.state.clock_recovery;
    let room_pin = h.room.state.room_clock_recovery;
    let room_calls = h.io.game.borrow().calls.clone();
    let furniture_calls = furniture_io.game.borrow().calls.clone();
    let room_connections = h.io.connections;
    let room_acquisitions = h.io.acquisitions;
    let denied = h.prepare()?;
    assert_eq!(denied["result"]["ok"], false, "{denied}");
    assert_eq!(h.io.connections, room_connections);
    assert_eq!(h.io.acquisitions, room_acquisitions);
    assert_eq!(h.io.game.borrow().calls, room_calls);
    assert_eq!(furniture_io.game.borrow().calls, furniture_calls);
    assert_eq!(h.room.state.clock_recovery, furniture_pin);
    assert_eq!(h.room.state.room_clock_recovery, room_pin);
    assert!(h.clock()?.cached_view().runs.is_empty());

    let cancelled = furniture_call(
        &mut h,
        &mut furniture_io,
        "fortress.cancel",
        clock::Action::Cancel(furniture_id),
    )?;
    assert_eq!(cancelled["result"]["ok"], true, "{cancelled}");
    assert!(
        !h.room
            .state
            .clock
            .as_ref()
            .ok_or_else(missing_fixture)?
            .cached_view()
            .pending
    );
    assert!(!h.clock()?.cached_view().pending);
    let prepared = h.prepare()?;
    assert_eq!(prepared["result"]["ok"], true, "{prepared}");
    assert!(h.clock()?.cached_view().pending);
    assert!(h.clock()?.prepared_seal().is_some());

    let room_pin = h.room.state.room_clock_recovery;
    let furniture_pin = h.room.state.clock_recovery;
    let room_calls = h.io.game.borrow().calls.clone();
    let furniture_calls = furniture_io.game.borrow().calls.clone();
    let connections = furniture_io.connections;
    let acquisitions = furniture_io.acquisitions;
    let denied = furniture_call(
        &mut h,
        &mut furniture_io,
        "fortress.plan",
        clock::Action::Prepare(clock::Request {
            monitor_id: furniture_id,
            game_ticks: 10,
            wall_millis: 5000,
        }),
    )?;
    assert_eq!(denied["result"]["ok"], false, "{denied}");
    assert_eq!(furniture_io.connections, connections);
    assert_eq!(furniture_io.acquisitions, acquisitions);
    assert_eq!(furniture_io.game.borrow().calls, furniture_calls);
    assert_eq!(h.io.game.borrow().calls, room_calls);
    assert_eq!(h.room.state.clock_recovery, furniture_pin);
    assert_eq!(h.room.state.room_clock_recovery, room_pin);
    assert!(h.clock()?.cached_view().pending);
    assert_eq!(h.clock()?.definition().id(), room_id);

    // Retirement releases only the selected original interval. It neither
    // substitutes goal identity nor leaves a permanent cross-owner blockade.
    let cancelled = h.call(room_clock::Action::Cancel(room_id))?;
    assert_eq!(cancelled["result"]["ok"], true, "{cancelled}");
    // No gameplay occurred, so the source tick/sequence still matches the
    // cancelled 10-tick intent. Request a distinct 11-tick interval explicitly;
    // replaying the identical cancelled key must never mint another permit.
    let prepared = furniture_call(
        &mut h,
        &mut furniture_io,
        "fortress.plan",
        clock::Action::Prepare(clock::Request {
            monitor_id: furniture_id,
            game_ticks: 11,
            wall_millis: 5000,
        }),
    )?;
    assert_eq!(prepared["result"]["ok"], true, "{prepared}");
    assert_eq!(h.id()?, room_id);
    assert_eq!(
        h.room
            .state
            .clock
            .as_ref()
            .ok_or_else(missing_fixture)?
            .definition()
            .id(),
        furniture_id
    );
    assert!(!h.io.game.borrow().calls.contains(&"commit"));
    assert!(!furniture_io.game.borrow().calls.contains(&"commit"));
    Ok(())
}
