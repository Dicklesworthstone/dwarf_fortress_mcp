//! Independent, joined TCP peers exercise the actual query-only connection.
use super::{MAX_ENTITIES, MAX_NETWORK_BYTES, ReadPermission, acquire_trusted};
use crate::build_placement::rpc::BuildCancellation;
use crate::live_operations::{LiveOperationsState, OperationsProfile};
use crate::order_run::FortressIdentity;
use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, DfmcpError, Digest32, ErrorCode, GameTick,
    ObservationCursor, OperationContext, RequestId, Result, RiskTier, SessionId, StateAnchor,
    WorkBudget,
};
use std::collections::BTreeMap;
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const PAGE: usize = 65536;
const TOKEN: [u8; 32] = [b'o'; 32];
const NONCE: [u8; 32] = [b'n'; 32];
const SNAPSHOT: [u8; 16] = [b's'; 16];
const TICK: u64 = 806500;
fn invalid() -> DfmcpError {
    DfmcpError::new(
        ErrorCode::AdapterRejected,
        "allocation TCP test peer failed",
    )
}
fn io_error(_: io::Error) -> DfmcpError {
    invalid()
}
fn require(value: bool) -> Result<()> {
    if value { Ok(()) } else { Err(invalid()) }
}
fn text_field(out: &mut Vec<u8>, value: &[u8]) {
    out.extend_from_slice(&(value.len() as u16).to_be_bytes());
    out.extend_from_slice(value);
}
fn numbers(out: &mut Vec<u8>, values: impl IntoIterator<Item = u32>) {
    for value in values {
        out.extend_from_slice(&value.to_be_bytes());
    }
}
/// Independently encode the complete native layout, including one attachment
/// at the very end of the capture. The first otherwise-free bed is attached.
fn capture(items: u32, full: bool, folder: &str, tick: u64) -> Vec<u8> {
    let jobs_count = if full { 4096 } else { 1 };
    let buildings = if full { 4096 } else { 0 };
    let mut jobs = b"DFMJ1200".to_vec();
    numbers(&mut jobs, [(tick / 403200) as u32, (tick % 403200) as u32]);
    jobs.push(1);
    numbers(&mut jobs, [2, jobs_count]);
    text_field(&mut jobs, folder.as_bytes());
    numbers(&mut jobs, [jobs_count]);
    for id in 0..jobs_count {
        numbers(&mut jobs, [id, 1]);
        text_field(&mut jobs, b"ConstructBuilding");
        text_field(&mut jobs, b"");
        jobs.extend_from_slice(&[0, 0]);
        numbers(&mut jobs, [10, 11, 2]);
        jobs.extend_from_slice(&[0, 0]);
        numbers(&mut jobs, [u32::MAX, u32::from(id == 0 && items > 0), 0]);
    }
    let mut out = b"DFMO1400".to_vec();
    numbers(&mut out, [jobs.len() as u32]);
    out.extend_from_slice(&jobs);
    numbers(&mut out, [buildings, items, buildings]);
    for id in 0..buildings {
        numbers(&mut out, [id, 1]);
        text_field(&mut out, b"Bed");
        numbers(&mut out, [10, 11, 10, 11, 2, 1, 1]);
    }
    numbers(&mut out, [items]);
    for id in 0..items {
        numbers(&mut out, [id, 101]);
        text_field(&mut out, b"BED");
        numbers(&mut out, [u32::MAX, 419, u32::MAX, 1, 10, 11, 2, 64]);
        out.extend_from_slice(&[0, 0]);
    }
    numbers(&mut out, [u32::from(items > 0)]);
    if items > 0 {
        numbers(&mut out, [0, 0, 0, u32::MAX]);
    }
    out
}
fn fortress() -> Result<FortressIdentity> {
    FortressIdentity::new("region1", 2)
}
fn context() -> Result<OperationContext> {
    let fortress = fortress()?;
    Ok(OperationContext {
        session_id: SessionId::new(1),
        request_id: RequestId::new(2),
        anchor: StateAnchor {
            fortress_id: fortress.fortress_id(),
            cursor: ObservationCursor::ORIGIN,
            tick: GameTick(TICK - 1),
            state_hash: Digest32::of_bytes(b"previous"),
        },
        budget: WorkBudget {
            max_wall_millis: 60_000,
            max_bytes: MAX_NETWORK_BYTES,
            max_entities: MAX_ENTITIES,
            ..WorkBudget::CONSERVATIVE_DEFAULT
        },
        grants: vec![CapabilityGrant {
            capability: Capability::Query,
            scope: CapabilityScope {
                fortress_id: Some(fortress.fortress_id()),
                ..CapabilityScope::default()
            },
            max_risk: RiskTier::ReadOnly,
            expires_at_tick: None,
            remaining_uses: None,
        }],
        cancellation_requested: false,
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Wire {
    Number(u64),
    Bytes(Vec<u8>),
}
type Fields = BTreeMap<u32, Wire>;
fn integer(input: &mut &[u8]) -> Result<u64> {
    let mut value = 0;
    for shift in 0..10 {
        let (&next, rest) = input.split_first().ok_or_else(invalid)?;
        *input = rest;
        require(shift < 9 || next <= 1)?;
        value |= u64::from(next & 127) << (shift * 7);
        if next < 128 {
            require(shift == 0 || next != 0)?;
            return Ok(value);
        }
    }
    Err(invalid())
}
fn read_fields(mut raw: &[u8]) -> Result<Fields> {
    let mut fields = BTreeMap::new();
    while !raw.is_empty() {
        let key = integer(&mut raw)?;
        let tag = (key >> 3) as u32;
        require((1..=14).contains(&tag) && !fields.contains_key(&tag))?;
        let value = match key & 7 {
            0 => Wire::Number(integer(&mut raw)?),
            2 => {
                let count = integer(&mut raw)? as usize;
                let (bytes, tail) = raw.split_at_checked(count).ok_or_else(invalid)?;
                raw = tail;
                Wire::Bytes(bytes.to_vec())
            }
            _ => return Err(invalid()),
        };
        fields.insert(tag, value);
    }
    Ok(fields)
}
fn varint(out: &mut Vec<u8>, mut value: u64) {
    while value >= 128 {
        out.push((value as u8 & 127) | 128);
        value >>= 7;
    }
    out.push(value as u8);
}
fn wire_field(out: &mut Vec<u8>, tag: u32, value: &Wire) {
    match value {
        Wire::Number(value) => {
            varint(out, u64::from(tag) << 3);
            varint(out, *value);
        }
        Wire::Bytes(value) => {
            varint(out, (u64::from(tag) << 3) | 2);
            varint(out, value.len() as u64);
            out.extend_from_slice(value);
        }
    }
}
fn encode(fields: &Fields) -> Vec<u8> {
    let mut out = Vec::new();
    for (tag, value) in fields {
        wire_field(&mut out, *tag, value);
    }
    out
}
fn number(fields: &Fields, tag: u32) -> Result<u64> {
    match fields.get(&tag) {
        Some(Wire::Number(value)) => Ok(*value),
        _ => Err(invalid()),
    }
}
fn bytes(fields: &Fields, tag: u32) -> Result<&[u8]> {
    match fields.get(&tag) {
        Some(Wire::Bytes(value)) => Ok(value),
        _ => Err(invalid()),
    }
}
fn exact(fields: &Fields, wanted: &[u32]) -> Result<()> {
    require(fields.len() == wanted.len() && wanted.iter().all(|tag| fields.contains_key(tag)))
}
fn header(method: i16, count: i32) -> [u8; 8] {
    let mut out = [0; 8];
    out[..2].copy_from_slice(&method.to_le_bytes());
    out[4..].copy_from_slice(&count.to_le_bytes());
    out
}
fn read_request(stream: &mut TcpStream) -> Result<Option<(i16, Fields)>> {
    let mut frame = [0; 8];
    match stream.read_exact(&mut frame) {
        Ok(()) => {}
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::UnexpectedEof | io::ErrorKind::ConnectionReset
            ) =>
        {
            return Ok(None);
        }
        Err(error) => return Err(io_error(error)),
    }
    require(frame[2..4] == [0; 2])?;
    let method = i16::from_le_bytes([frame[0], frame[1]]);
    let count = i32::from_le_bytes(frame[4..8].try_into().map_err(|_| invalid())?);
    require((0..=2048).contains(&count))?;
    let mut raw = vec![0; count as usize];
    stream.read_exact(&mut raw).map_err(io_error)?;
    Ok(Some((method, read_fields(&raw)?)))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Event {
    Bind(usize),
    Handshake,
    Page(usize),
    Release,
}
fn events(pages: usize) -> Vec<Event> {
    [Event::Bind(0), Event::Bind(1), Event::Handshake]
        .into_iter()
        .chain((0..pages).map(Event::Page))
        .chain([Event::Release])
        .collect()
}
#[derive(Clone)]
enum Fault {
    Field(u32, Wire),
    Drop,
    Frame(i16, i32),
}
#[derive(Default)]
struct Options {
    fault: Option<(Event, Fault)>,
    revoke: Option<(Event, Arc<AtomicBool>)>,
    cancel: Option<(Event, BuildCancellation)>,
    notifications: usize,
}
#[derive(Debug, Default)]
struct Report {
    connections: usize,
    events: Vec<Event>,
}
struct Peer {
    endpoint: SocketAddr,
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<Result<Report>>>,
}
impl Peer {
    fn start(capture: Vec<u8>, options: Options) -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").map_err(io_error)?;
        let endpoint = listener.local_addr().map_err(io_error)?;
        listener.set_nonblocking(true).map_err(io_error)?;
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let join = thread::spawn(move || {
            let mut report = Report::default();
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
                        serve(&mut stream, &capture, &options, &mut report)?;
                    }
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(1))
                    }
                    Err(e) => return Err(io_error(e)),
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
    fn finish(mut self) -> Result<Report> {
        self.stop.store(true, Ordering::Release);
        self.join
            .take()
            .ok_or_else(invalid)?
            .join()
            .map_err(|_| invalid())?
    }
}
impl Drop for Peer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}
fn validate_request(event: Event, method: i16, fields: &Fields) -> Result<()> {
    if let Event::Bind(index) = event {
        exact(fields, &[1, 2, 3, 4])?;
        return require(
            method == 0
                && bytes(fields, 1)? == [b"Handshake".as_slice(), b"ReadObservation"][index]
                && bytes(fields, 2)? == b"dfmcp.operations.v1_4.Request"
                && bytes(fields, 3)? == b"dfmcp.operations.v1_4.Reply"
                && bytes(fields, 4)? == b"dfmcp_operations_v1_4",
        );
    }
    let retained = matches!(event,Event::Page(n) if n>0) || event == Event::Release;
    let expected: Vec<_> = (1..=12).filter(|tag| *tag != 9 || retained).collect();
    exact(fields, &expected)?;
    require(bytes(fields, 1)? == TOKEN && bytes(fields, 2)? == NONCE)?;
    for (tag, value) in [
        (3, 1),
        (4, 4),
        (5, 4096),
        (6, 4096),
        (7, 65536),
        (8, 16777216),
        (11, PAGE as u64),
    ] {
        require(number(fields, tag)? == value)?;
    }
    if retained {
        require(bytes(fields, 9)? == SNAPSHOT)?;
    }
    let offset = match event {
        Event::Page(n) => n * PAGE,
        _ => 0,
    };
    require(
        number(fields, 10)? == offset as u64
            && number(fields, 12)? == u64::from(event == Event::Release)
            && method == if event == Event::Handshake { 2 } else { 3 },
    )
}
fn serve(
    stream: &mut TcpStream,
    capture: &[u8],
    options: &Options,
    report: &mut Report,
) -> Result<()> {
    let mut greeting = [0; 12];
    match stream.read_exact(&mut greeting) {
        Ok(()) => {}
        Err(e)
            if matches!(
                e.kind(),
                io::ErrorKind::UnexpectedEof | io::ErrorKind::ConnectionReset
            ) =>
        {
            return Ok(());
        }
        Err(e) => return Err(io_error(e)),
    }
    require(&greeting == b"DFHack?\n\x01\0\0\0")?;
    stream.write_all(b"DFHack!\n\x01\0\0\0").map_err(io_error)?;
    let expected = events(capture.len().div_ceil(PAGE));
    let digest = Digest32::of_bytes(capture);
    while let Some((method, fields)) = read_request(stream)? {
        let event = *expected.get(report.events.len()).ok_or_else(invalid)?;
        validate_request(event, method, &fields)?;
        report.events.push(event);
        let mut reply = BTreeMap::from([
            (1, Wire::Number(1)),
            (2, Wire::Number(0)),
            (3, Wire::Bytes(NONCE.to_vec())),
            (4, Wire::Number(1)),
            (5, Wire::Number(4)),
            (6, Wire::Number(987)),
            (7, Wire::Bytes(b"53.01".to_vec())),
            (8, Wire::Bytes(b"53.01-r1".to_vec())),
        ]);
        match event {
            Event::Bind(index) => reply = BTreeMap::from([(1, Wire::Number(index as u64 + 2))]),
            Event::Page(index) => {
                let offset = index * PAGE;
                let end = (offset + PAGE).min(capture.len());
                reply.extend([
                    (9, Wire::Bytes(capture[offset..end].to_vec())),
                    (10, Wire::Bytes(SNAPSHOT.to_vec())),
                    (11, Wire::Number(offset as u64)),
                    (12, Wire::Number(capture.len() as u64)),
                    (13, Wire::Bytes(digest.as_bytes().to_vec())),
                    (14, Wire::Number(u64::from(end == capture.len()))),
                ]);
            }
            Event::Release => {
                reply.insert(10, Wire::Bytes(SNAPSHOT.to_vec()));
            }
            Event::Handshake => {}
        }
        if let Some((at, fault)) = &options.fault {
            if *at == event {
                match fault {
                    Fault::Field(tag, value) => {
                        reply.insert(*tag, value.clone());
                    }
                    Fault::Drop => return Ok(()),
                    Fault::Frame(id, n) => {
                        let _ = stream.write_all(&header(*id, *n));
                        return Ok(());
                    }
                }
            }
        }
        if let Some((at, signal)) = &options.revoke {
            if *at == event {
                signal.store(true, Ordering::Release);
            }
        }
        if let Some((at, cancel)) = &options.cancel {
            if *at == event {
                // No reply arrives: the client's 100ms slices must discover
                // cancellation while blocked and close this connection.
                cancel.cancel();
                require(read_request(stream)?.is_none())?;
                return Ok(());
            }
        }
        for _ in 0..options.notifications {
            if stream.write_all(&header(-3, 0)).is_err() {
                return Ok(());
            }
        }
        let raw = encode(&reply);
        let mut frame = header(-1, raw.len() as i32).to_vec();
        frame.extend(raw);
        for chunk in frame.chunks(503) {
            if stream.write_all(chunk).is_err() {
                return Ok(());
            }
        }
    }
    Ok(())
}
fn fetch(peer: &Peer, context: &OperationContext) -> Result<LiveOperationsState> {
    acquire_trusted(
        peer.endpoint,
        &fortress()?,
        TOKEN.to_vec(),
        NONCE,
        context,
        BuildCancellation::default(),
        Box::new(|| Ok(())),
    )
}

#[test]
fn complete_maximum_roster_keeps_native_identity_and_last_attachment() -> Result<()> {
    let raw = capture(65536, true, "region1", TICK);
    let digest = Digest32::of_bytes(&raw);
    let pages = raw.len().div_ceil(PAGE);
    let peer = Peer::start(raw, Options::default())?;
    let state = fetch(&peer, &context()?)?;
    let report = peer.finish()?;
    assert_eq!(report.connections, 1);
    assert_eq!(report.events, events(pages));
    assert_eq!(state.profile(), OperationsProfile::PagedV1_4);
    let observation = state.observation().ok_or_else(invalid)?;
    assert_eq!(observation.jobs.bridge_generation, 987);
    assert_eq!(observation.jobs.df_version, "53.01");
    assert_eq!(observation.jobs.dfhack_version, "53.01-r1");
    assert_eq!(observation.jobs.jobs.len(), 4096);
    assert_eq!(observation.buildings.len(), 4096);
    assert_eq!(observation.items.len(), 65536);
    assert_eq!(observation.attachments.len(), 1);
    assert_eq!(observation.attachments[0].item_native_id, 0);
    assert_eq!(
        Digest32::of_bytes(&observation.encode_profile(OperationsProfile::PagedV1_4)?),
        digest
    );
    let snapshot = state.snapshot().ok_or_else(invalid)?;
    assert_eq!(snapshot.graph.entities.len(), MAX_ENTITIES as usize);
    assert_eq!(snapshot.anchor().tick, GameTick(TICK));
    assert!(snapshot.hash_is_valid());
    Ok(())
}

#[test]
fn complete_entity_allowance_includes_fortress_root() -> Result<()> {
    for (allowance, accepted) in [(3, false), (4, true)] {
        let peer = Peer::start(capture(2, false, "region1", TICK), Options::default())?;
        let mut context = context()?;
        context.budget.max_entities = allowance;
        let result = fetch(&peer, &context);
        let report = peer.finish()?;
        assert_eq!(result.is_ok(), accepted);
        assert_eq!(report.events.contains(&Event::Release), accepted);
    }
    Ok(())
}

#[test]
fn source_drift_and_incomplete_release_never_publish() -> Result<()> {
    let changes = [
        (Event::Bind(1), Fault::Field(1, Wire::Number(2))),
        (
            Event::Handshake,
            Fault::Field(3, Wire::Bytes(vec![b'x'; 32])),
        ),
        (Event::Handshake, Fault::Field(5, Wire::Number(3))),
        (Event::Page(0), Fault::Field(6, Wire::Number(988))),
        (
            Event::Page(0),
            Fault::Field(7, Wire::Bytes(b"changed".to_vec())),
        ),
        (Event::Page(0), Fault::Field(13, Wire::Bytes(vec![1; 32]))),
        (Event::Page(0), Fault::Field(11, Wire::Number(1))),
        (Event::Page(0), Fault::Frame(-1, 100000)),
        (Event::Page(0), Fault::Drop),
        (Event::Release, Fault::Field(6, Wire::Number(988))),
        (
            Event::Release,
            Fault::Field(10, Wire::Bytes(vec![b'x'; 16])),
        ),
        (Event::Release, Fault::Drop),
    ];
    for (at, fault) in changes {
        let peer = Peer::start(
            capture(2, false, "region1", TICK),
            Options {
                fault: Some((at, fault)),
                ..Options::default()
            },
        )?;
        assert!(
            fetch(&peer, &context()?).is_err(),
            "fault at {at:?} published"
        );
        let report = peer.finish()?;
        assert_eq!(report.connections, 1);
        assert_eq!(report.events.last(), Some(&at));
    }
    Ok(())
}

#[test]
fn foreign_fortress_regressed_clock_and_expired_query_refuse_before_release() -> Result<()> {
    for (folder, tick, expires) in [
        ("foreign", TICK, None),
        ("region1", TICK - 2, None),
        ("region1", TICK, Some(GameTick(TICK - 1))),
    ] {
        let peer = Peer::start(capture(2, false, folder, tick), Options::default())?;
        let mut context = context()?;
        context.grants[0].expires_at_tick = expires;
        assert!(fetch(&peer, &context).is_err());
        let report = peer.finish()?;
        assert!(!report.events.contains(&Event::Release));
    }
    Ok(())
}

#[test]
fn malformed_final_attachment_is_not_hidden_by_usable_items() -> Result<()> {
    let mut raw = capture(2, false, "region1", TICK);
    let at = raw.len() - 12;
    raw[at..at + 4].copy_from_slice(&99u32.to_be_bytes());
    let peer = Peer::start(raw, Options::default())?;
    assert!(fetch(&peer, &context()?).is_err());
    assert!(!peer.finish()?.events.contains(&Event::Release));
    Ok(())
}

#[test]
fn blocked_read_drains_cancellation_without_reconnect() -> Result<()> {
    let cancellation = BuildCancellation::default();
    let peer = Peer::start(
        capture(2, false, "region1", TICK),
        Options {
            cancel: Some((Event::Page(0), cancellation.clone())),
            ..Options::default()
        },
    )?;
    let start = Instant::now();
    let result = acquire_trusted(
        peer.endpoint,
        &fortress()?,
        TOKEN.to_vec(),
        NONCE,
        &context()?,
        cancellation,
        Box::new(|| Ok(())),
    );
    assert!(result.is_err());
    assert!(start.elapsed() < Duration::from_secs(2));
    let report = peer.finish()?;
    assert_eq!(report.connections, 1);
    assert_eq!(report.events.last(), Some(&Event::Page(0)));
    Ok(())
}

#[test]
fn revocation_during_release_discards_complete_capture() -> Result<()> {
    let signal = Arc::new(AtomicBool::new(false));
    let peer = Peer::start(
        capture(2, false, "region1", TICK),
        Options {
            revoke: Some((Event::Release, signal.clone())),
            ..Options::default()
        },
    )?;
    let permission: ReadPermission = Box::new(move || {
        if signal.load(Ordering::Acquire) {
            Err(DfmcpError::new(
                ErrorCode::CapabilityDenied,
                "test operator revoked",
            ))
        } else {
            Ok(())
        }
    });
    assert!(
        acquire_trusted(
            peer.endpoint,
            &fortress()?,
            TOKEN.to_vec(),
            NONCE,
            &context()?,
            BuildCancellation::default(),
            permission
        )
        .is_err()
    );
    assert_eq!(peer.finish()?.events.last(), Some(&Event::Release));
    Ok(())
}

#[test]
fn network_and_notification_limits_apply_to_whole_connection() -> Result<()> {
    for (bytes, notifications) in [(64, 0), (MAX_NETWORK_BYTES, 9)] {
        let peer = Peer::start(
            capture(2, false, "region1", TICK),
            Options {
                notifications,
                ..Options::default()
            },
        )?;
        let mut context = context()?;
        context.budget.max_bytes = bytes;
        assert!(fetch(&peer, &context).is_err());
        assert!(!peer.finish()?.events.contains(&Event::Release));
    }
    Ok(())
}

#[test]
fn invalid_authority_and_configuration_fail_before_contact() -> Result<()> {
    for fault in 0..5 {
        let peer = Peer::start(capture(2, false, "region1", TICK), Options::default())?;
        let mut context = context()?;
        let mut token = TOKEN.to_vec();
        let mut nonce = NONCE;
        match fault {
            0 => context.grants.clear(),
            1 => context.cancellation_requested = true,
            2 => context.budget.max_wall_millis = 60_001,
            3 => token[0] = 0,
            _ => nonce = [0; 32],
        }
        assert!(
            acquire_trusted(
                peer.endpoint,
                &fortress()?,
                token,
                nonce,
                &context,
                BuildCancellation::default(),
                Box::new(|| Ok(()))
            )
            .is_err()
        );
        assert_eq!(peer.finish()?.connections, 0);
    }
    Ok(())
}
