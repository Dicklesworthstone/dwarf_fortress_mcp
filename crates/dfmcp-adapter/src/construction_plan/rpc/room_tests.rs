//! Independent fixed-profile TCP peer for one complete original room bracket.
//! This reuses the receipt fixtures and independent outer codec of its parent,
//! while independently writing map payloads and validating the six-call binding.
use super::*;
use crate::construction_plan::rpc::room;
use dfmcp_world::map_region::Region;

const MAP_SECRET: [u8; 32] = [b'm'; 32];
const MAP_GENERATION: u64 = 1701;

fn region() -> Region {
    Region {
        origin: [14, 14, 2],
        size: [25, 13, 1],
    }
}

fn map_payload(goal: &Goal, region: Region) -> Result<Vec<u8>> {
    let before = goal.records()[0].plan().before();
    let tick = before.tick() + 1;
    let mut raw = b"DFMM1500".to_vec();
    numbers(&mut raw, [(tick / 403200) as u32, (tick % 403200) as u32]);
    raw.push(1);
    numbers(&mut raw, [before.site()]);
    field(&mut raw, before.folder().as_bytes());
    numbers(&mut raw, before.dimensions());
    numbers(&mut raw, region.origin);
    numbers(&mut raw, region.size);
    let count = region.volume().map_err(crate::live_map::map_error)?;
    numbers(&mut raw, [count as u32]);
    for index in 0..count {
        let position = region.position(index).ok_or_else(invalid)?;
        let occupied = goal.records().iter().any(|record| {
            record
                .insertion()
                .is_some_and(|insertion| insertion.position() == position)
        });
        raw.push(2);
        numbers(&mut raw, [219]);
        raw.extend_from_slice(&[3, 0, 0, 0, 0, u8::from(occupied), 0]);
        numbers(&mut raw, [1]);
        raw.extend_from_slice(&10015u16.to_be_bytes());
        raw.extend_from_slice(&10015u16.to_be_bytes());
    }
    Ok(raw)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RoomEvent {
    Original(Event),
    MapBind(usize),
    MapHandshake,
    MapBefore,
    MapAfter,
}
fn room_events(count: usize, pages: usize) -> Vec<RoomEvent> {
    (0..4)
        .map(|index| RoomEvent::Original(Event::Bind(index)))
        .chain([
            RoomEvent::MapBind(0),
            RoomEvent::MapBind(1),
            RoomEvent::Original(Event::BuildHandshake),
            RoomEvent::Original(Event::OperationsHandshake),
            RoomEvent::MapHandshake,
        ])
        .chain((0..count).map(|index| RoomEvent::Original(Event::Before(index))))
        .chain([RoomEvent::MapBefore])
        .chain((0..pages).map(|index| RoomEvent::Original(Event::Page(index))))
        .chain([RoomEvent::Original(Event::Release), RoomEvent::MapAfter])
        .chain((0..count).map(|index| RoomEvent::Original(Event::After(index))))
        .collect()
}
#[derive(Default)]
struct RoomOptions {
    change: Option<(RoomEvent, Change)>,
    signal: Option<(RoomEvent, Arc<AtomicBool>)>,
    cancellation: Option<(RoomEvent, BuildCancellation)>,
}
#[derive(Debug, Default)]
struct RoomReport {
    connections: usize,
    events: Vec<RoomEvent>,
    query_keys: Vec<String>,
}
struct RoomPeer {
    endpoint: SocketAddr,
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<Result<RoomReport>>>,
}
impl RoomPeer {
    fn start(goal: &Goal, operations: Vec<u8>, map: Vec<u8>, options: RoomOptions) -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").map_err(io_error)?;
        let endpoint = listener.local_addr().map_err(io_error)?;
        listener.set_nonblocking(true).map_err(io_error)?;
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let records = goal.records().to_vec();
        let join = thread::spawn(move || {
            let mut report = RoomReport::default();
            while !stopped.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        report.connections += 1;
                        require(report.connections == 1)?;
                        stream
                            .set_read_timeout(Some(Duration::from_secs(2)))
                            .map_err(io_error)?;
                        stream
                            .set_write_timeout(Some(Duration::from_secs(2)))
                            .map_err(io_error)?;
                        stream.set_nodelay(true).map_err(io_error)?;
                        serve_room(
                            &mut stream,
                            &records,
                            &operations,
                            &map,
                            &options,
                            &mut report,
                        )?;
                    }
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(1))
                    }
                    Err(error) => return Err(io_error(error)),
                }
            }
            Ok(report)
        });
        Ok(Self {
            endpoint,
            stop,
            join: Some(join),
        })
    }
    fn binding(&self, goal: &Goal) -> Result<BuildBinding> {
        BuildBinding::new(
            self.endpoint,
            "53.01",
            "53.01-r1",
            goal.records()[0].plan().before(),
        )
    }
    fn finish(mut self) -> Result<RoomReport> {
        self.stop.store(true, Ordering::Release);
        self.join
            .take()
            .ok_or_else(invalid)?
            .join()
            .map_err(|_| invalid())?
    }
}
impl Drop for RoomPeer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

fn room_reply(
    event: RoomEvent,
    records: &[BuildRecord],
    operations: &[u8],
    map: &[u8],
    method: i16,
    fields: &Fields,
    report: &mut RoomReport,
) -> Result<Fields> {
    match event {
        RoomEvent::Original(event) => {
            validate_request(event, method, fields, records)?;
            let mut response = reply(event, records.len());
            match event {
                Event::Bind(index) => {
                    response = BTreeMap::from([(1, Wire::Number(index as u64 + 2))])
                }
                Event::Before(index) | Event::After(index) => {
                    report
                        .query_keys
                        .push(records[index].plan().key().to_owned());
                    response.insert(10, Wire::Bytes(records[index].canonical_bytes().to_vec()));
                }
                Event::Page(index) => {
                    let offset = index * PAGE;
                    let end = (offset + PAGE).min(operations.len());
                    response.extend([
                        (9, Wire::Bytes(operations[offset..end].to_vec())),
                        (10, Wire::Bytes(SNAPSHOT.to_vec())),
                        (11, Wire::Number(offset as u64)),
                        (12, Wire::Number(operations.len() as u64)),
                        (
                            13,
                            Wire::Bytes(Digest32::of_bytes(operations).as_bytes().to_vec()),
                        ),
                        (14, Wire::Number(u64::from(end == operations.len()))),
                    ]);
                }
                Event::Release => {
                    response.insert(10, Wire::Bytes(SNAPSHOT.to_vec()));
                }
                Event::BuildHandshake | Event::OperationsHandshake => {}
            }
            Ok(response)
        }
        RoomEvent::MapBind(index) => {
            exact(fields, &[1, 2, 3, 4])?;
            require(
                method == 0
                    && bytes(fields, 1)? == [b"Handshake".as_slice(), b"ReadObservation"][index]
                    && bytes(fields, 2)? == b"dfmcp.map.v1_5.Request"
                    && bytes(fields, 3)? == b"dfmcp.map.v1_5.Reply"
                    && bytes(fields, 4)? == b"dfmcp_map_v1_5",
            )?;
            Ok(BTreeMap::from([(1, Wire::Number(index as u64 + 6))]))
        }
        RoomEvent::MapHandshake | RoomEvent::MapBefore | RoomEvent::MapAfter => {
            exact(fields, &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11])?;
            require(
                method
                    == if event == RoomEvent::MapHandshake {
                        6
                    } else {
                        7
                    }
                    && bytes(fields, 1)? == MAP_SECRET
                    && bytes(fields, 2)? == NONCE
                    && number(fields, 3)? == 1
                    && number(fields, 4)? == 5
                    && number(fields, 11)? == crate::live_map::MAX_MAP_BYTES as u64,
            )?;
            for (tag, value) in (5..=10).zip(region().origin.into_iter().chain(region().size)) {
                require(number(fields, tag)? == u64::from(value))?;
            }
            let mut response = BTreeMap::from([
                (1, Wire::Number(1)),
                (2, Wire::Number(0)),
                (3, Wire::Bytes(NONCE.to_vec())),
                (4, Wire::Number(1)),
                (5, Wire::Number(5)),
                (6, Wire::Number(MAP_GENERATION)),
                (7, Wire::Bytes(b"53.01".to_vec())),
                (8, Wire::Bytes(b"53.01-r1".to_vec())),
            ]);
            if event != RoomEvent::MapHandshake {
                response.insert(9, Wire::Bytes(map.to_vec()));
            }
            Ok(response)
        }
    }
}

fn serve_room(
    stream: &mut TcpStream,
    records: &[BuildRecord],
    operations: &[u8],
    map: &[u8],
    options: &RoomOptions,
    report: &mut RoomReport,
) -> Result<()> {
    let mut greeting = [0; 12];
    match stream.read_exact(&mut greeting) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(()),
        Err(error) => return Err(io_error(error)),
    }
    require(&greeting == b"DFHack?\n\x01\0\0\0")?;
    stream.write_all(b"DFHack!\n\x01\0\0\0").map_err(io_error)?;
    let expected = room_events(records.len(), operations.len().div_ceil(PAGE));
    while let Some((method, fields)) = read_request(stream)? {
        let event = *expected.get(report.events.len()).ok_or_else(invalid)?;
        let mut response = room_reply(event, records, operations, map, method, &fields, report)
            .map_err(|_| {
                DfmcpError::new(
                    ErrorCode::AdapterRejected,
                    format!("unexpected room peer request at {event:?}"),
                )
            })?;
        report.events.push(event);
        let mut suffix = Vec::new();
        let mut raw_override = None;
        if let Some((at, change)) = &options.change
            && *at == event
        {
            match change {
                Change::Field(tag, value) => {
                    response.insert(*tag, value.clone());
                }
                Change::Remove(tag) => {
                    response.remove(tag);
                }
                Change::Duplicate(tag, value) => wire_field(&mut suffix, *tag, value),
                Change::Raw(raw) => raw_override = Some(raw.clone()),
                Change::Drop => return Ok(()),
                Change::Frame(method, count) => {
                    let _ = stream.write_all(&header(*method, *count));
                    return Ok(());
                }
            }
        }
        if let Some((at, signal)) = &options.signal
            && *at == event
        {
            signal.store(true, Ordering::Release);
        }
        if let Some((at, cancellation)) = &options.cancellation
            && *at == event
        {
            cancellation.cancel();
        }
        let mut raw = raw_override.unwrap_or_else(|| encode(&response));
        raw.extend(suffix);
        let mut framed = header(-1, raw.len() as i32).to_vec();
        framed.extend(raw);
        for fragment in framed.chunks(503) {
            if stream.write_all(fragment).is_err() {
                return Ok(());
            }
        }
    }
    Ok(())
}

fn room_fetch(peer: &RoomPeer, goal: &Goal, context: &OperationContext) -> Result<room::Capture> {
    room::acquire_trusted(
        &peer.binding(goal)?,
        goal,
        region(),
        OPERATIONS_SECRET.to_vec(),
        BUILD_SECRET.to_vec(),
        MAP_SECRET.to_vec(),
        NONCE,
        context,
        BuildCancellation::default(),
        Box::new(|| Ok(())),
    )
}
fn room_refusal(goal: &Goal, at: RoomEvent, change: Change) -> Result<RoomReport> {
    let peer = RoomPeer::start(
        goal,
        capture(goal, 0)?,
        map_payload(goal, region())?,
        RoomOptions {
            change: Some((at, change)),
            ..RoomOptions::default()
        },
    )?;
    let result = room_fetch(&peer, goal, &context(goal));
    let report = peer.finish()?;
    assert!(
        result.is_err(),
        "room fault at {at:?} returned complete evidence"
    );
    assert_eq!(report.connections, 1);
    assert_eq!(report.events.last(), Some(&at));
    Ok(report)
}

#[test]
fn one_connection_preserves_every_receipt_and_both_complete_map_endpoints() -> Result<()> {
    for count in [1, 3, 32] {
        let goal = goal(count)?;
        let operations = capture(&goal, if count == 32 { 2000 } else { 0 })?;
        let map = map_payload(&goal, region())?;
        let pages = operations.len().div_ceil(PAGE);
        let peer = RoomPeer::start(
            &goal,
            operations.clone(),
            map.clone(),
            RoomOptions::default(),
        )?;
        let sample = room_fetch(&peer, &goal, &context(&goal));
        let report = peer.finish()?;
        let sample = sample?;
        assert_eq!(sample.before_capture, map);
        assert_eq!(sample.after_capture, map);
        assert_eq!(sample.before, sample.after);
        assert_eq!(sample.before.generation, MAP_GENERATION);
        assert_eq!(sample.furnishings.before.generation, 41);
        assert_eq!(sample.furnishings.operations.generation, 987);
        assert_eq!(sample.furnishings.capture, operations);
        let originals: Vec<_> = goal
            .records()
            .iter()
            .map(|record| record.canonical_bytes().to_vec())
            .collect();
        assert_eq!(sample.furnishings.before_records, originals);
        assert_eq!(sample.furnishings.after_records, originals);
        sample.furnishings.validate(&goal, &context(&goal))?;
        assert_eq!(report.connections, 1);
        assert_eq!(report.events, room_events(count as usize, pages));
        let keys: Vec<_> = goal
            .records()
            .iter()
            .map(|record| record.plan().key().to_owned())
            .collect();
        assert_eq!(report.query_keys, [keys.clone(), keys].concat());
    }
    Ok(())
}

#[test]
fn missing_trailing_map_or_last_receipt_never_publishes_a_subset_or_reconnects() -> Result<()> {
    let goal = goal(32)?;
    for event in [RoomEvent::MapAfter, RoomEvent::Original(Event::After(31))] {
        room_refusal(&goal, event, Change::Drop)?;
        room_refusal(
            &goal,
            event,
            Change::Remove(if event == RoomEvent::MapAfter { 9 } else { 10 }),
        )?;
    }
    Ok(())
}

#[test]
fn map_binding_aliases_nonce_profile_source_and_field_corruption_stop_at_boundary() -> Result<()> {
    let goal = goal(1)?;
    for (at, changes) in [
        (
            RoomEvent::MapBind(0),
            vec![
                Change::Field(1, Wire::Number(2)),
                Change::Field(1, Wire::Number(0)),
            ],
        ),
        (
            RoomEvent::MapBind(1),
            vec![Change::Field(1, Wire::Number(6))],
        ),
        (
            RoomEvent::MapHandshake,
            vec![
                Change::Field(3, Wire::Bytes(vec![b'x'; 32])),
                Change::Field(5, Wire::Number(6)),
                Change::Field(7, Wire::Bytes(b"other".to_vec())),
                Change::Duplicate(6, Wire::Number(MAP_GENERATION)),
                Change::Field(6, Wire::Number(0)),
            ],
        ),
        (
            RoomEvent::MapBefore,
            vec![
                Change::Field(6, Wire::Number(MAP_GENERATION + 1)),
                Change::Remove(9),
                Change::Field(9, Wire::Bytes(b"DFMM1500".to_vec())),
                Change::Field(10, Wire::Number(1)),
            ],
        ),
        (
            RoomEvent::MapAfter,
            vec![
                Change::Field(6, Wire::Number(MAP_GENERATION + 1)),
                Change::Field(8, Wire::Bytes(b"other".to_vec())),
            ],
        ),
    ] {
        for change in changes {
            room_refusal(&goal, at, change)?;
        }
    }
    Ok(())
}

#[test]
fn map_byte_change_after_released_operations_blocks_every_trailing_receipt() -> Result<()> {
    let goal = goal(3)?;
    let mut map = map_payload(&goal, region())?;
    let last = map.len() - 1;
    map[last] ^= 1; // Valid temperature change, not a malformed payload.
    let report = room_refusal(
        &goal,
        RoomEvent::MapAfter,
        Change::Field(9, Wire::Bytes(map)),
    )?;
    assert!(report.events.contains(&RoomEvent::Original(Event::Release)));
    assert!(
        !report
            .events
            .iter()
            .any(|event| matches!(event, RoomEvent::Original(Event::After(_))))
    );
    Ok(())
}

#[test]
fn map_fortress_dimensions_region_pause_and_shared_tick_are_exact() -> Result<()> {
    let goal = goal(1)?;
    let map = map_payload(&goal, region())?;
    let before = goal.records()[0].plan().before();
    let dimensions_at = 23 + before.folder().len();
    let region_at = dimensions_at + 12;
    for (at, bytes) in [
        (17usize, (before.site() + 1).to_be_bytes().to_vec()),
        (
            dimensions_at,
            (before.dimensions()[0] - 1).to_be_bytes().to_vec(),
        ),
        (region_at, (region().origin[0] + 1).to_be_bytes().to_vec()),
        (16, vec![0]),
    ] {
        let mut changed = map.clone();
        changed[at..at + bytes.len()].copy_from_slice(&bytes);
        room_refusal(
            &goal,
            RoomEvent::MapBefore,
            Change::Field(9, Wire::Bytes(changed)),
        )?;
    }
    let mut later = map.clone();
    later[12..16].copy_from_slice(&((before.tick() % 403200 + 2) as u32).to_be_bytes());
    let peer = RoomPeer::start(&goal, capture(&goal, 0)?, later, RoomOptions::default())?;
    assert!(room_fetch(&peer, &goal, &context(&goal)).is_err());
    let report = peer.finish()?;
    assert_eq!(
        report.events.last(),
        Some(&RoomEvent::Original(Event::Page(0)))
    );
    assert!(!report.events.contains(&RoomEvent::Original(Event::Release)));
    Ok(())
}

#[test]
fn later_map_tick_expires_query_before_any_operations_capture() -> Result<()> {
    let goal = goal(1)?;
    let peer = RoomPeer::start(
        &goal,
        capture(&goal, 0)?,
        map_payload(&goal, region())?,
        RoomOptions::default(),
    )?;
    let mut current = context(&goal);
    current.grants[0].expires_at_tick = Some(current.anchor.tick);
    let mut floor = current.anchor.tick.get();
    let result = room::acquire_trusted_with_floor(
        &peer.binding(&goal)?,
        &goal,
        region(),
        OPERATIONS_SECRET.to_vec(),
        BUILD_SECRET.to_vec(),
        MAP_SECRET.to_vec(),
        NONCE,
        &current,
        BuildCancellation::default(),
        Box::new(|| Ok(())),
        &mut floor,
    );
    assert!(result.is_err());
    assert_eq!(floor, current.anchor.tick.get() + 1);
    let report = peer.finish()?;
    assert_eq!(report.events.last(), Some(&RoomEvent::MapBefore));
    Ok(())
}

#[test]
fn decoded_native_tick_survives_incomplete_or_changed_trailing_map() -> Result<()> {
    let goal = goal(1)?;
    let c = context(&goal);
    let mut changed = map_payload(&goal, region())?;
    changed[12..16].copy_from_slice(&((c.anchor.tick.get() % 403200 + 2) as u32).to_be_bytes());
    for (change, expected) in [
        (Change::Drop, c.anchor.tick.get() + 1),
        (
            Change::Field(9, Wire::Bytes(changed)),
            c.anchor.tick.get() + 2,
        ),
    ] {
        let peer = RoomPeer::start(
            &goal,
            capture(&goal, 0)?,
            map_payload(&goal, region())?,
            RoomOptions {
                change: Some((RoomEvent::MapAfter, change)),
                ..RoomOptions::default()
            },
        )?;
        let mut floor = c.anchor.tick.get();
        let result = room::acquire_trusted_with_floor(
            &peer.binding(&goal)?,
            &goal,
            region(),
            OPERATIONS_SECRET.to_vec(),
            BUILD_SECRET.to_vec(),
            MAP_SECRET.to_vec(),
            NONCE,
            &c,
            BuildCancellation::default(),
            Box::new(|| Ok(())),
            &mut floor,
        );
        assert!(result.is_err());
        assert_eq!(floor, expected);
        assert_eq!(peer.finish()?.events.last(), Some(&RoomEvent::MapAfter));
    }
    Ok(())
}

#[test]
fn both_map_endpoints_and_complete_operations_share_the_entity_allowance() -> Result<()> {
    let goal = goal(1)?;
    let cells = region().volume().map_err(crate::live_map::map_error)? as u32;
    for fits in [false, true] {
        let peer = RoomPeer::start(
            &goal,
            capture(&goal, 0)?,
            map_payload(&goal, region())?,
            RoomOptions::default(),
        )?;
        let mut c = context(&goal);
        c.budget.max_entities = 2 * cells + if fits { 3 } else { 2 };
        let result = room_fetch(&peer, &goal, &c);
        assert_eq!(result.is_ok(), fits);
        let report = peer.finish()?;
        assert_eq!(
            report.events.contains(&RoomEvent::Original(Event::Release)),
            fits
        );
        if !fits {
            assert_eq!(
                report.events.last(),
                Some(&RoomEvent::Original(Event::Page(0)))
            );
        }
    }
    Ok(())
}

#[test]
fn complete_room_bounds_and_credentials_refuse_before_connecting() -> Result<()> {
    let goal = goal(1)?;
    for case in 0..6 {
        let peer = RoomPeer::start(
            &goal,
            capture(&goal, 0)?,
            map_payload(&goal, region())?,
            RoomOptions::default(),
        )?;
        let mut current = context(&goal);
        let mut selected = region();
        let mut token = MAP_SECRET.to_vec();
        match case {
            0 => current.grants.clear(),
            1 => current.budget.max_bytes = 22 * 1024 * 1024 - 1,
            2 => current.budget.max_entities = 1,
            3 => selected.size[2] = 2,
            4 => selected.size = [128, 128, 1],
            _ => token.clear(),
        }
        let result = room::acquire_trusted(
            &peer.binding(&goal)?,
            &goal,
            selected,
            OPERATIONS_SECRET.to_vec(),
            BUILD_SECRET.to_vec(),
            token,
            NONCE,
            &current,
            BuildCancellation::default(),
            Box::new(|| Ok(())),
        );
        assert!(result.is_err());
        assert_eq!(peer.finish()?.connections, 0);
    }
    Ok(())
}

#[test]
fn last_receipt_revocation_and_cancellation_withhold_the_complete_room() -> Result<()> {
    let goal = goal(3)?;
    let last = RoomEvent::Original(Event::After(2));
    for revoked in [false, true] {
        let signal = Arc::new(AtomicBool::new(false));
        let visible = signal.clone();
        let cancellation = BuildCancellation::default();
        let options = if revoked {
            RoomOptions {
                signal: Some((last, signal)),
                ..RoomOptions::default()
            }
        } else {
            RoomOptions {
                cancellation: Some((last, cancellation.clone())),
                ..RoomOptions::default()
            }
        };
        let peer = RoomPeer::start(
            &goal,
            capture(&goal, 0)?,
            map_payload(&goal, region())?,
            options,
        )?;
        let result = room::acquire_trusted(
            &peer.binding(&goal)?,
            &goal,
            region(),
            OPERATIONS_SECRET.to_vec(),
            BUILD_SECRET.to_vec(),
            MAP_SECRET.to_vec(),
            NONCE,
            &context(&goal),
            cancellation,
            Box::new(move || {
                if visible.load(Ordering::Acquire) {
                    Err(invalid())
                } else {
                    Ok(())
                }
            }),
        );
        assert!(result.is_err());
        let report = peer.finish()?;
        assert_eq!(report.events.last(), Some(&last));
    }
    Ok(())
}
