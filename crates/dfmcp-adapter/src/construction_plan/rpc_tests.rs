//! Joined, read-only TCP doubles for the complete receipt/capture bracket.
//! The peer owns an independent protobuf codec and rejects any unplanned call.
//! This exercises Rust source only; it is not native or live admission evidence.

use super::acquire_trusted;
use crate::build_placement::rpc::BuildCancellation;
use crate::build_placement::{BuildBinding, BuildCapture, BuildItem, BuildRecord};
use crate::construction_plan::{Goal, LinkedSample, Timing};
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
use std::time::Duration;

const PAGE: usize = 65536;
const BUILD_SECRET: [u8; 32] = [b'b'; 32];
const OPERATIONS_SECRET: [u8; 32] = [b'o'; 32];
const NONCE: [u8; 32] = [b'n'; 32];
const SNAPSHOT: [u8; 16] = [b's'; 16];
const BINDINGS: [(&str, &str, &str); 4] = [
    ("dfmcp_build_v1_19", "dfmcp.build.v1_19", "Handshake"),
    ("dfmcp_build_v1_19", "dfmcp.build.v1_19", "QueryPlacement"),
    (
        "dfmcp_operations_v1_4",
        "dfmcp.operations.v1_4",
        "Handshake",
    ),
    (
        "dfmcp_operations_v1_4",
        "dfmcp.operations.v1_4",
        "ReadObservation",
    ),
];

fn invalid() -> DfmcpError {
    DfmcpError::new(
        ErrorCode::AdapterRejected,
        "construction TCP test peer failed",
    )
}
fn io_error(_: io::Error) -> DfmcpError {
    invalid()
}
fn require(condition: bool) -> Result<()> {
    if condition { Ok(()) } else { Err(invalid()) }
}
fn field(out: &mut Vec<u8>, value: &[u8]) {
    out.extend_from_slice(&(value.len() as u16).to_be_bytes());
    out.extend_from_slice(value);
}
fn numbers(out: &mut Vec<u8>, values: impl IntoIterator<Item = u32>) {
    for value in values {
        out.extend_from_slice(&value.to_be_bytes());
    }
}
fn fixture(name: &str) -> Result<Vec<u8>> {
    let source =
        include_str!("../../../../bridge/common/tests/fixtures/build_placement_v1_19.json");
    let prefix = format!("\"{name}\": \"");
    let value = source
        .lines()
        .find_map(|line| line.trim().strip_prefix(&prefix))
        .and_then(|line| line.split('"').next())
        .ok_or_else(invalid)?;
    require(value.len().is_multiple_of(2))?;
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            u8::from_str_radix(std::str::from_utf8(pair).map_err(|_| invalid())?, 16)
                .map_err(|_| invalid())
        })
        .collect()
}

/// Preserve the engine fixture's complete native readback, changing only the
/// explicitly selected item, location and unique allocated IDs. The production
/// RPC never sees a test-only record constructor or a mocked decoded response.
fn receipt(index: u32) -> Result<BuildRecord> {
    let original = BuildRecord::decode(&fixture("placed")?)?;
    let mut before = original.plan().before().canonical_bytes().to_vec();
    // The checked-in native vector has a seven-byte folder, nine visible
    // fourteen-byte tiles, and one visible item. Assert that layout first.
    require(before.len() == 278 && &before[62..69] == b"region1")?;
    require(before[72] == 1 && before[215] == 2 && before[228] == 1)?;
    let kind = (index % 3 + 1) as u8;
    let position = [15 + 3 * (index % 8), 15 + 3 * (index / 8), 2];
    before[16..24].copy_from_slice(&(u64::from(index) * 2).to_be_bytes());
    for (offset, value) in [(48, 70 + index), (52, 90 + index), (56, 4 + index)] {
        before[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
    }
    before[72] = kind;
    before[73..77].copy_from_slice(&(42 + index).to_be_bytes());
    for (offset, value) in [77, 81, 85].into_iter().zip(position) {
        before[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
    }
    before[228] = kind;
    before[229..233].copy_from_slice(&(100 + u32::from(kind)).to_be_bytes());
    let before = BuildCapture::decode(&before)?;
    let plan = crate::build_placement::BuildPlan::new(&format!("construction-{index}"), before)?;
    let mut insertion = original
        .insertion()
        .ok_or_else(invalid)?
        .canonical_bytes()
        .to_vec();
    for (offset, value) in [(8, 70 + index), (12, 90 + index), (16, 42 + index)] {
        insertion[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
    }
    insertion[20] = kind;
    for (offset, value) in [21, 25, 29].into_iter().zip(position) {
        insertion[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
    }
    let mut raw = b"DFMBR019".to_vec();
    field(&mut raw, plan.key().as_bytes());
    field(&mut raw, plan.before().canonical_bytes());
    raw.extend_from_slice(plan.digest().as_bytes());
    raw.extend_from_slice(plan.token());
    raw.extend_from_slice(&[2, 0, 1, 1]);
    field(&mut raw, plan.before().expected_after()?.canonical_bytes());
    field(&mut raw, &insertion);
    let mut digest_input = b"dfmcp-build-receipt/1\0".to_vec();
    digest_input.extend_from_slice(&raw);
    raw.extend_from_slice(Digest32::of_bytes(&digest_input).as_bytes());
    BuildRecord::decode(&raw)
}
fn goal(count: u32) -> Result<Goal> {
    let records = (0..count).map(receipt).collect::<Result<Vec<_>>>()?;
    Goal::new(
        records,
        Timing {
            deadline: 806600,
            interval: 1,
            stable_samples: 2,
            stable_span: 1,
            max_gap: 1200,
            max_observations: 512,
        },
    )
}
fn context(goal: &Goal) -> OperationContext {
    let before = goal.records()[0].plan().before();
    OperationContext {
        session_id: SessionId::new(1),
        request_id: RequestId::new(2),
        anchor: StateAnchor {
            fortress_id: before.fortress_id(),
            cursor: ObservationCursor::ORIGIN,
            tick: GameTick(before.tick()),
            state_hash: before.witness(),
        },
        budget: WorkBudget {
            max_wall_millis: 5000,
            max_bytes: 32 * 1024 * 1024,
            max_entities: 100_000,
            max_actions: 512,
            ..WorkBudget::CONSERVATIVE_DEFAULT
        },
        grants: [Capability::Query]
            .into_iter()
            .map(|capability| CapabilityGrant {
                capability,
                scope: CapabilityScope {
                    fortress_id: Some(before.fortress_id()),
                    ..CapabilityScope::default()
                },
                max_risk: RiskTier::ReadOnly,
                expires_at_tick: None,
                remaining_uses: None,
            })
            .collect(),
        cancellation_requested: false,
    }
}

/// Independently write the published DFMO1400/DFMJ1200 binary layout. Every
/// selected building and installed original singleton item shares one tick.
fn capture(goal: &Goal, extra_items: u32) -> Result<Vec<u8>> {
    let records = goal.records();
    let before = records[0].plan().before();
    let last = records.last().ok_or_else(invalid)?;
    let tick = before.tick() + 1;
    let mut jobs = b"DFMJ1200".to_vec();
    numbers(&mut jobs, [(tick / 403200) as u32, (tick % 403200) as u32]);
    jobs.push(1);
    numbers(
        &mut jobs,
        [before.site(), last.plan().before().next_job_id() + 1],
    );
    field(&mut jobs, before.folder().as_bytes());
    numbers(&mut jobs, [0]);
    let mut out = b"DFMO1400".to_vec();
    numbers(&mut out, [jobs.len() as u32]);
    out.extend_from_slice(&jobs);
    numbers(
        &mut out,
        [
            last.plan().before().next_building_id() + 1,
            if extra_items == 0 {
                42 + records.len() as u32
            } else {
                1000 + extra_items
            },
            records.len() as u32,
        ],
    );
    for record in records {
        let insertion = record.insertion().ok_or_else(invalid)?;
        let kind = insertion.kind() as u32;
        let key = ["", "Bed", "Chair", "Table"][kind as usize];
        numbers(&mut out, [insertion.building_id(), kind]);
        field(&mut out, key.as_bytes());
        let [x, y, z] = insertion.position();
        numbers(
            &mut out,
            [x, y, x, y, z, insertion.max_stage(), insertion.max_stage()],
        );
    }
    numbers(&mut out, [records.len() as u32 + extra_items]);
    for record in records {
        let insertion = record.insertion().ok_or_else(invalid)?;
        let BuildItem::Visible(item) = record.plan().before().item() else {
            return Err(invalid());
        };
        numbers(&mut out, [insertion.item_id(), item.native_type()]);
        field(
            &mut out,
            [b"".as_slice(), b"BED", b"CHAIR", b"TABLE"][insertion.kind() as usize],
        );
        numbers(
            &mut out,
            [
                item.subtype() as u32,
                item.material() as u32,
                item.material_index() as u32,
                1,
            ],
        );
        numbers(&mut out, insertion.position());
        numbers(&mut out, [256]);
        out.extend_from_slice(&[0, 1]);
        numbers(&mut out, [insertion.building_id()]);
    }
    for id in 1000..1000 + extra_items {
        numbers(&mut out, [id, 101]);
        field(&mut out, b"BED");
        numbers(&mut out, [u32::MAX, 419, u32::MAX, 1, 10, 11, 2, 64]);
        out.extend_from_slice(&[0, 0]);
    }
    numbers(&mut out, [0]);
    Ok(out)
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
    BuildHandshake,
    OperationsHandshake,
    Before(usize),
    Page(usize),
    Release,
    After(usize),
}
fn events(count: usize, pages: usize) -> Vec<Event> {
    (0..4)
        .map(Event::Bind)
        .chain([Event::BuildHandshake, Event::OperationsHandshake])
        .chain((0..count).map(Event::Before))
        .chain((0..pages).map(Event::Page))
        .chain([Event::Release])
        .chain((0..count).map(Event::After))
        .collect()
}
#[derive(Clone, Debug)]
enum Change {
    Field(u32, Wire),
    Remove(u32),
    Duplicate(u32, Wire),
    Raw(Vec<u8>),
    Drop,
    Frame(i16, i32),
}
#[derive(Default)]
struct Options {
    change: Option<(Event, Change)>,
    notifications: usize,
    notification_bytes: usize,
    signal: Option<(Event, Arc<AtomicBool>)>,
    cancellation: Option<(Event, BuildCancellation)>,
}
impl Options {
    fn changed(event: Event, change: Change) -> Self {
        Self {
            change: Some((event, change)),
            ..Self::default()
        }
    }
}
#[derive(Debug, Default)]
struct Report {
    connections: usize,
    events: Vec<Event>,
    query_keys: Vec<String>,
}
struct Peer {
    endpoint: SocketAddr,
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<Result<Report>>>,
}
impl Peer {
    fn start(goal: &Goal, capture: Vec<u8>, options: Options) -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").map_err(io_error)?;
        let endpoint = listener.local_addr().map_err(io_error)?;
        listener.set_nonblocking(true).map_err(io_error)?;
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let records = goal.records().to_vec();
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
                        serve(&mut stream, &records, &capture, &options, &mut report)?;
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

fn reply(event: Event, retained: usize) -> Fields {
    let build = matches!(
        event,
        Event::BuildHandshake | Event::Before(_) | Event::After(_)
    );
    let mut fields = BTreeMap::from([
        (1, Wire::Number(1)),
        (2, Wire::Number(0)),
        (3, Wire::Bytes(NONCE.to_vec())),
        (4, Wire::Number(1)),
        (5, Wire::Number(if build { 19 } else { 4 })),
        (6, Wire::Number(if build { 41 } else { 987 })),
        (7, Wire::Bytes(b"53.01".to_vec())),
        (8, Wire::Bytes(b"53.01-r1".to_vec())),
    ]);
    if build {
        fields.insert(12, Wire::Number(0));
        fields.insert(13, Wire::Number(retained as u64));
    }
    fields
}
fn validate_request(
    event: Event,
    method: i16,
    fields: &Fields,
    records: &[BuildRecord],
) -> Result<()> {
    if let Event::Bind(index) = event {
        exact(fields, &[1, 2, 3, 4])?;
        let (plugin, profile, name) = BINDINGS[index];
        return require(
            method == 0
                && bytes(fields, 1)? == name.as_bytes()
                && bytes(fields, 2)? == format!("{profile}.Request").as_bytes()
                && bytes(fields, 3)? == format!("{profile}.Reply").as_bytes()
                && bytes(fields, 4)? == plugin.as_bytes(),
        );
    }
    let build = matches!(
        event,
        Event::BuildHandshake | Event::Before(_) | Event::After(_)
    );
    require(
        bytes(fields, 1)?
            == if build {
                &BUILD_SECRET
            } else {
                &OPERATIONS_SECRET
            },
    )?;
    require(
        bytes(fields, 2)? == NONCE
            && number(fields, 3)? == 1
            && number(fields, 4)? == if build { 19 } else { 4 },
    )?;
    match event {
        Event::BuildHandshake => {
            exact(fields, &[1, 2, 3, 4])?;
            require(method == 2)
        }
        Event::Before(index) | Event::After(index) => {
            exact(fields, &[1, 2, 3, 4, 10, 12])?;
            require(
                method == 3
                    && bytes(fields, 10)? == records[index].plan().key().as_bytes()
                    && bytes(fields, 12)? == records[index].plan().digest().as_bytes(),
            )
        }
        Event::OperationsHandshake | Event::Page(_) | Event::Release => {
            // Existing Rust operations/1.4 omits an empty snapshot token and
            // emits the explicit default offset/release values on handshake.
            let snapshot =
                matches!(event, Event::Page(index) if index > 0) || event == Event::Release;
            let expected: Vec<_> = (1..=12).filter(|tag| *tag != 9 || snapshot).collect();
            exact(fields, &expected)?;
            for (tag, value) in [
                (5, 4096),
                (6, 4096),
                (7, 65536),
                (8, 16777216),
                (11, PAGE as u64),
            ] {
                require(number(fields, tag)? == value)?;
            }
            if snapshot {
                require(bytes(fields, 9)? == SNAPSHOT)?;
            }
            let offset = match event {
                Event::Page(index) => index * PAGE,
                _ => 0,
            };
            require(
                number(fields, 10)? == offset as u64
                    && number(fields, 12)? == u64::from(event == Event::Release)
                    && method
                        == if event == Event::OperationsHandshake {
                            4
                        } else {
                            5
                        },
            )
        }
        Event::Bind(_) => Err(invalid()),
    }
}
fn serve(
    stream: &mut TcpStream,
    records: &[BuildRecord],
    capture: &[u8],
    options: &Options,
    report: &mut Report,
) -> Result<()> {
    let mut greeting = [0; 12];
    match stream.read_exact(&mut greeting) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(()),
        Err(error) => return Err(io_error(error)),
    }
    require(&greeting == b"DFHack?\n\x01\0\0\0")?;
    stream.write_all(b"DFHack!\n\x01\0\0\0").map_err(io_error)?;
    let expected = events(records.len(), capture.len().div_ceil(PAGE));
    while let Some((method, fields)) = read_request(stream)? {
        let event = *expected.get(report.events.len()).ok_or_else(invalid)?;
        validate_request(event, method, &fields, records).map_err(|_| {
            DfmcpError::new(ErrorCode::AdapterRejected, format!(
                "construction TCP peer unexpected request at {event:?}: method {method}, fields {:?}",
                fields.keys().collect::<Vec<_>>()
            ))
        })?;
        report.events.push(event);
        let mut response = reply(event, records.len());
        match event {
            Event::Bind(index) => response = BTreeMap::from([(1, Wire::Number(index as u64 + 2))]),
            Event::Before(index) | Event::After(index) => {
                report
                    .query_keys
                    .push(records[index].plan().key().to_owned());
                response.insert(10, Wire::Bytes(records[index].canonical_bytes().to_vec()));
            }
            Event::Page(index) => {
                let offset = index * PAGE;
                let end = (offset + PAGE).min(capture.len());
                response.extend([
                    (9, Wire::Bytes(capture[offset..end].to_vec())),
                    (10, Wire::Bytes(SNAPSHOT.to_vec())),
                    (11, Wire::Number(offset as u64)),
                    (12, Wire::Number(capture.len() as u64)),
                    (
                        13,
                        Wire::Bytes(Digest32::of_bytes(capture).as_bytes().to_vec()),
                    ),
                    (14, Wire::Number(u64::from(end == capture.len()))),
                ]);
            }
            Event::Release => {
                response.insert(10, Wire::Bytes(SNAPSHOT.to_vec()));
            }
            Event::BuildHandshake | Event::OperationsHandshake => {}
        }
        let mut suffix = Vec::new();
        let mut raw_override = None;
        if let Some((at, change)) = &options.change {
            if *at == event {
                match change {
                    Change::Field(tag, value) => {
                        response.insert(*tag, value.clone());
                    }
                    Change::Remove(tag) => {
                        response.remove(tag);
                    }
                    Change::Duplicate(tag, value) => wire_field(&mut suffix, *tag, value),
                    Change::Raw(raw) => raw_override = Some(raw.clone()),
                    Change::Drop => {
                        let _ = stream.write_all(&header(-1, 1000));
                        let _ = stream.write_all(&[8]);
                        return Ok(());
                    }
                    Change::Frame(method, count) => {
                        let _ = stream.write_all(&header(*method, *count));
                        return Ok(());
                    }
                }
            }
        }
        if let Some((at, signal)) = &options.signal {
            if *at == event {
                signal.store(true, Ordering::Release);
            }
        }
        if let Some((at, cancellation)) = &options.cancellation {
            if *at == event {
                cancellation.cancel();
            }
        }
        for _ in 0..options.notifications {
            let mut notification = header(-3, options.notification_bytes as i32).to_vec();
            notification.resize(8 + options.notification_bytes, 0);
            if stream.write_all(&notification).is_err() {
                return Ok(());
            }
        }
        let mut raw = raw_override.unwrap_or_else(|| encode(&response));
        raw.extend(suffix);
        let mut frame = header(-1, raw.len() as i32).to_vec();
        frame.extend(raw);
        for fragment in frame.chunks(503) {
            if stream.write_all(fragment).is_err() {
                return Ok(());
            }
        }
    }
    Ok(())
}

fn fetch(peer: &Peer, goal: &Goal, context: &OperationContext) -> Result<LinkedSample> {
    acquire_trusted(
        &peer.binding(goal)?,
        goal,
        OPERATIONS_SECRET.to_vec(),
        BUILD_SECRET.to_vec(),
        NONCE,
        context,
        BuildCancellation::default(),
        Box::new(|| Ok(())),
    )
}
fn refusal(goal: &Goal, raw: Vec<u8>, at: Event, change: Change) -> Result<Report> {
    let peer = Peer::start(goal, raw, Options::changed(at, change))?;
    let result = fetch(&peer, goal, &context(goal));
    let report = peer.finish()?;
    assert!(
        result.is_err(),
        "fault at {at:?} returned a complete sample"
    );
    assert_eq!(report.connections, 1);
    assert_eq!(
        report.events.last(),
        Some(&at),
        "fault crossed its receipt/capture boundary"
    );
    Ok(report)
}

#[test]
fn one_connection_brackets_all_distinct_furniture_and_never_binds_effects() -> Result<()> {
    for count in [1, 3, 32] {
        let goal = goal(count)?;
        let payload = capture(&goal, if count == 32 { 2000 } else { 0 })?;
        let pages = payload.len().div_ceil(PAGE);
        if count == 32 {
            assert!(pages > 1);
        }
        let peer = Peer::start(
            &goal,
            payload.clone(),
            Options {
                notifications: 1,
                notification_bytes: 31,
                ..Options::default()
            },
        )?;
        let result = fetch(&peer, &goal, &context(&goal));
        let report = peer.finish()?;
        let result = result?;
        let originals: Vec<_> = goal
            .records()
            .iter()
            .map(|record| record.canonical_bytes().to_vec())
            .collect();
        assert_eq!(result.before_records, originals);
        assert_eq!(result.after_records, originals);
        assert_eq!(result.capture, payload);
        assert_eq!(result.before.generation, 41);
        assert_eq!(result.after.generation, 41);
        assert_eq!(result.operations.generation, 987);
        assert_eq!(
            result.validate(&goal, &context(&goal))?.items.len(),
            count as usize + if count == 32 { 2000 } else { 0 }
        );
        assert_eq!(report.connections, 1);
        assert_eq!(report.events, events(count as usize, pages));
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
fn each_missing_substituted_or_modified_member_refuses_the_complete_bracket() -> Result<()> {
    let goal = goal(3)?;
    let raw = capture(&goal, 0)?;
    for index in 0..3 {
        let mut corrupted = goal.records()[index].canonical_bytes().to_vec();
        let last = corrupted.len() - 1;
        corrupted[last] ^= 1;
        for at in [Event::Before(index), Event::After(index)] {
            for change in [
                Change::Remove(10),
                Change::Field(
                    10,
                    Wire::Bytes(goal.records()[(index + 1) % 3].canonical_bytes().to_vec()),
                ),
                Change::Field(10, Wire::Bytes(corrupted.clone())),
            ] {
                refusal(&goal, raw.clone(), at, change)?;
            }
        }
    }
    Ok(())
}

#[test]
fn retention_must_cover_every_original_at_each_receipt_boundary() -> Result<()> {
    let goal = goal(32)?;
    let raw = capture(&goal, 0)?;
    for at in [
        Event::Before(0),
        Event::Before(31),
        Event::After(0),
        Event::After(31),
    ] {
        refusal(&goal, raw.clone(), at, Change::Field(13, Wire::Number(31)))?;
    }
    // Handshake does not attest this set. Each subsequent receipt query does.
    // Other retained work may grow the count without weakening the set.
    for (at, retained) in [(Event::BuildHandshake, 0), (Event::After(31), 256)] {
        let peer = Peer::start(
            &goal,
            raw.clone(),
            Options::changed(at, Change::Field(13, Wire::Number(retained))),
        )?;
        let result = fetch(&peer, &goal, &context(&goal));
        let report = peer.finish()?;
        result?;
        assert_eq!(report.events, events(32, 1));
    }
    Ok(())
}

#[test]
fn original_build_generation_and_every_live_manifest_remain_exact() -> Result<()> {
    let goal = goal(3)?;
    let raw = capture(&goal, 2000)?;
    for at in [
        Event::BuildHandshake,
        Event::Before(0),
        Event::Before(2),
        Event::After(0),
        Event::After(2),
    ] {
        for change in [
            Change::Field(6, Wire::Number(42)),
            Change::Field(7, Wire::Bytes(b"other-df".to_vec())),
            Change::Field(8, Wire::Bytes(b"other-dfhack".to_vec())),
        ] {
            refusal(&goal, raw.clone(), at, change)?;
        }
    }
    for at in [
        Event::OperationsHandshake,
        Event::Page(0),
        Event::Page(1),
        Event::Release,
    ] {
        refusal(
            &goal,
            raw.clone(),
            at,
            Change::Field(8, Wire::Bytes(b"other-dfhack".to_vec())),
        )?;
        if at != Event::OperationsHandshake {
            refusal(&goal, raw.clone(), at, Change::Field(6, Wire::Number(988)))?;
        }
    }
    Ok(())
}

#[test]
fn mismatching_original_software_cannot_be_replaced_by_matching_live_plugins() -> Result<()> {
    let goal = goal(3)?;
    let peer = Peer::start(&goal, capture(&goal, 0)?, Options::default())?;
    let binding = BuildBinding::new(
        peer.endpoint,
        "wrong-original",
        "53.01-r1",
        goal.records()[0].plan().before(),
    )?;
    let result = acquire_trusted(
        &binding,
        &goal,
        OPERATIONS_SECRET.to_vec(),
        BUILD_SECRET.to_vec(),
        NONCE,
        &context(&goal),
        BuildCancellation::default(),
        Box::new(|| Ok(())),
    );
    let report = peer.finish()?;
    assert!(result.is_err());
    assert!(
        report
            .events
            .iter()
            .all(|event| !matches!(event, Event::Before(_) | Event::Page(_) | Event::After(_)))
    );
    Ok(())
}

#[test]
fn original_generation_fortress_and_dimensions_must_match_before_contact() -> Result<()> {
    let goal = goal(3)?;
    for change in 0..3 {
        let mut raw = goal.records()[0].plan().before().canonical_bytes().to_vec();
        match change {
            0 => raw[8..16].copy_from_slice(&42u64.to_be_bytes()),
            1 => raw[62..69].copy_from_slice(b"region2"),
            _ => raw[36..40].copy_from_slice(&65u32.to_be_bytes()),
        }
        let before = BuildCapture::decode(&raw)?;
        let peer = Peer::start(&goal, capture(&goal, 0)?, Options::default())?;
        let binding = BuildBinding::new(peer.endpoint, "53.01", "53.01-r1", &before)?;
        let result = acquire_trusted(
            &binding,
            &goal,
            OPERATIONS_SECRET.to_vec(),
            BUILD_SECRET.to_vec(),
            NONCE,
            &context(&goal),
            BuildCancellation::default(),
            Box::new(|| Ok(())),
        );
        let report = peer.finish()?;
        assert!(result.is_err(), "original binding change {change}");
        assert_eq!(report.connections, 0);
    }
    Ok(())
}

#[test]
fn whole_capture_digest_token_offset_and_release_precede_every_trailing_query() -> Result<()> {
    let goal = goal(3)?;
    let raw = capture(&goal, 2000)?;
    let mut corrupted_final_page = raw[PAGE..].to_vec();
    corrupted_final_page[0] ^= 1;
    for (at, change) in [
        (Event::Page(0), Change::Field(10, Wire::Bytes(vec![1; 15]))),
        (Event::Page(1), Change::Field(10, Wire::Bytes(vec![2; 16]))),
        (Event::Page(1), Change::Field(11, Wire::Number(0))),
        (Event::Page(1), Change::Field(13, Wire::Bytes(vec![3; 32]))),
        (Event::Page(1), Change::Field(14, Wire::Number(0))),
        (Event::Page(1), Change::Field(9, Wire::Bytes(vec![4; 16]))),
        (
            Event::Page(1),
            Change::Field(9, Wire::Bytes(corrupted_final_page)),
        ),
        (Event::Page(1), Change::Drop),
        (Event::Release, Change::Field(10, Wire::Bytes(vec![5; 16]))),
        (Event::Release, Change::Field(9, Wire::Bytes(vec![6]))),
        (Event::Release, Change::Drop),
    ] {
        let report = refusal(&goal, raw.clone(), at, change)?;
        assert!(
            !report
                .events
                .iter()
                .any(|event| matches!(event, Event::After(_)))
        );
    }
    // A well-framed, digest-consistent payload still must decode completely.
    // Decoder refusal may precede release; no trailing queries are required
    // once the shared capture has already failed.
    let mut trailing = capture(&goal, 0)?;
    trailing.push(0);
    let peer = Peer::start(&goal, trailing, Options::default())?;
    let result = fetch(&peer, &goal, &context(&goal));
    let report = peer.finish()?;
    assert!(result.is_err());
    assert_eq!(report.connections, 1);
    assert!(report.events.contains(&Event::Page(0)));
    Ok(())
}

#[test]
fn loss_of_last_original_never_returns_a_subset_or_reconnects() -> Result<()> {
    let goal = goal(32)?;
    for at in [Event::Before(31), Event::After(31)] {
        refusal(&goal, capture(&goal, 0)?, at, Change::Drop)?;
    }
    Ok(())
}

#[test]
fn native_method_aliases_and_malformed_envelopes_stop_before_any_extra_call() -> Result<()> {
    let goal = goal(3)?;
    let raw = capture(&goal, 0)?;
    for index in 1..4 {
        refusal(
            &goal,
            raw.clone(),
            Event::Bind(index),
            Change::Field(1, Wire::Number(2)),
        )?;
    }
    for change in [
        Change::Duplicate(6, Wire::Number(41)),
        Change::Field(14, Wire::Number(0)),
        Change::Field(3, Wire::Bytes(vec![b'x'; 32])),
        Change::Field(4, Wire::Number(2)),
        Change::Field(5, Wire::Number(18)),
        Change::Field(12, Wire::Number(2)),
        Change::Raw(vec![8, 0x81, 0]),
        Change::Raw(vec![0x0d, 1, 0, 0, 0]),
        Change::Frame(-1, -1),
        Change::Frame(-2, 0),
        Change::Frame(-1, i32::MAX),
    ] {
        refusal(&goal, raw.clone(), Event::Before(0), change)?;
    }
    Ok(())
}

#[test]
fn cancellation_and_permission_revocation_at_the_last_receipt_prevent_publication() -> Result<()> {
    let goal = goal(3)?;
    for at in [Event::Before(1), Event::Page(0), Event::After(2)] {
        for cancel in [false, true] {
            let signal = Arc::new(AtomicBool::new(false));
            let cancellation = BuildCancellation::default();
            let options = if cancel {
                Options {
                    cancellation: Some((at, cancellation.clone())),
                    ..Options::default()
                }
            } else {
                Options {
                    signal: Some((at, signal.clone())),
                    ..Options::default()
                }
            };
            let peer = Peer::start(&goal, capture(&goal, 0)?, options)?;
            let result = acquire_trusted(
                &peer.binding(&goal)?,
                &goal,
                OPERATIONS_SECRET.to_vec(),
                BUILD_SECRET.to_vec(),
                NONCE,
                &context(&goal),
                cancellation,
                Box::new(move || {
                    if signal.load(Ordering::Acquire) {
                        Err(DfmcpError::new(
                            ErrorCode::CapabilityDenied,
                            "test permission revoked",
                        ))
                    } else {
                        Ok(())
                    }
                }),
            );
            let report = peer.finish()?;
            assert!(
                matches!(result, Err(ref error) if error.code == if cancel { ErrorCode::CancellationRequested } else { ErrorCode::CapabilityDenied })
            );
            assert_eq!(report.connections, 1);
            assert_eq!(report.events.last(), Some(&at));
        }
    }
    Ok(())
}

#[test]
fn sampled_tick_and_complete_entity_count_are_checked_before_release() -> Result<()> {
    let goal = goal(3)?;
    for expiry in [true, false] {
        let mut context = context(&goal);
        let expected = if expiry {
            // The original receipt tick is authorized. The fresh capture is
            // one tick later, so its current grant must be checked again.
            context.grants[0].expires_at_tick = Some(context.anchor.tick);
            ErrorCode::CapabilityDenied
        } else {
            // Three selected buildings plus three items also require the
            // observed world root. Six cannot fund that complete observation.
            context.budget.max_entities = 6;
            ErrorCode::BudgetExceeded
        };
        let peer = Peer::start(&goal, capture(&goal, 0)?, Options::default())?;
        let result = fetch(&peer, &goal, &context);
        let report = peer.finish()?;
        let cause = result.err().ok_or_else(invalid)?;
        assert_eq!(cause.code, expected);
        assert_eq!(report.connections, 1);
        assert_eq!(report.events.last(), Some(&Event::Page(0)));
        assert!(
            !report
                .events
                .iter()
                .any(|event| matches!(event, Event::Release | Event::After(_)))
        );
    }

    // Exactly seven entities permits the same complete bracket and the
    // downstream validator, using only Query authority throughout.
    let mut context = context(&goal);
    context.budget.max_entities = 7;
    let peer = Peer::start(&goal, capture(&goal, 0)?, Options::default())?;
    let result = fetch(&peer, &goal, &context);
    let report = peer.finish()?;
    let sample = result?;
    assert_eq!(sample.validate(&goal, &context)?.items.len(), 3);
    assert_eq!(report.connections, 1);
    assert_eq!(report.events, events(3, 1));
    Ok(())
}

#[test]
fn rejected_current_authority_or_credentials_never_open_the_socket() -> Result<()> {
    let goal = goal(3)?;
    for fault in 0..5 {
        let peer = Peer::start(&goal, capture(&goal, 0)?, Options::default())?;
        let mut context = context(&goal);
        let cancellation = BuildCancellation::default();
        let mut token = OPERATIONS_SECRET.to_vec();
        match fault {
            0 => context.grants.clear(),
            1 => context.cancellation_requested = true,
            2 => cancellation.cancel(),
            3 => token.truncate(31),
            _ => {}
        }
        let result = acquire_trusted(
            &peer.binding(&goal)?,
            &goal,
            token,
            BUILD_SECRET.to_vec(),
            NONCE,
            &context,
            cancellation,
            Box::new(move || {
                if fault == 4 {
                    Err(DfmcpError::new(ErrorCode::CapabilityDenied, "test denied"))
                } else {
                    Ok(())
                }
            }),
        );
        let report = peer.finish()?;
        assert!(result.is_err(), "preflight fault {fault}");
        assert_eq!(
            report.connections, 0,
            "preflight fault {fault} opened native socket"
        );
    }
    Ok(())
}

#[test]
fn notification_limits_cover_the_whole_connection_and_each_frame() -> Result<()> {
    let goal = goal(3)?;
    for (notifications, notification_bytes) in [(9, 0), (1, 65537), (4, 65536)] {
        let peer = Peer::start(
            &goal,
            capture(&goal, 0)?,
            Options {
                notifications,
                notification_bytes,
                ..Options::default()
            },
        )?;
        let result = fetch(&peer, &goal, &context(&goal));
        let report = peer.finish()?;
        assert!(
            result.is_err(),
            "unbounded native notifications returned a sample"
        );
        assert_eq!(report.connections, 1);
        if notifications == 4 {
            // Each call fits its local allowance; the ninth response exceeds
            // the shared 2 MiB allowance after eight successful responses.
            assert_eq!(report.events.len(), 9);
            assert_eq!(report.events.last(), Some(&Event::Before(2)));
        } else {
            assert_eq!(report.events, vec![Event::Bind(0)]);
        }
    }
    Ok(())
}

#[test]
fn one_connection_byte_allowance_is_shared_by_all_receipts_and_capture_pages() -> Result<()> {
    let goal = goal(32)?;
    let peer = Peer::start(&goal, capture(&goal, 2000)?, Options::default())?;
    let mut context = context(&goal);
    context.budget.max_bytes = 16384;
    let result = fetch(&peer, &goal, &context);
    let report = peer.finish()?;
    assert!(matches!(result, Err(error) if error.code == ErrorCode::BudgetExceeded));
    assert_eq!(report.connections, 1);
    let queried = report
        .events
        .iter()
        .filter(|event| matches!(event, Event::Before(_)))
        .count();
    assert!((1..32).contains(&queried));
    assert!(
        !report
            .events
            .iter()
            .any(|event| matches!(event, Event::Page(_) | Event::After(_)))
    );
    Ok(())
}
