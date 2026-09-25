//! Real TCP protocol doubles; not a live DFHack or generated-protobuf test.
use super::*;
use crate::control_effect_journal::EffectJournalStorage;
use crate::excavation_run::coordinator::ExcavationCoordinator;
use crate::excavation_run::tests::{plan, prepared, stopped};
use dfmcp_core::{CapabilityGrant, CapabilityScope, GameTick, ObservationCursor, RequestId,
    SessionId, StateAnchor, WorkBudget};
use std::io::{Read, Seek, SeekFrom, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Mutex, atomic::AtomicUsize};
use std::thread::{self, JoinHandle};
use std::time::Instant;

fn context() -> Result<OperationContext> {
    let p = plan()?;
    let id = p.before().fortress().fortress_id();
    Ok(OperationContext {
        session_id: SessionId::new(1), request_id: RequestId::new(2),
        anchor: StateAnchor { fortress_id: id, cursor: ObservationCursor::ORIGIN,
            tick: GameTick(p.before().tick()), state_hash: p.before().witness() },
        budget: WorkBudget { max_wall_millis: 5000, max_bytes: 32 * 1024 * 1024,
            ..WorkBudget::CONSERVATIVE_DEFAULT },
        grants: [Capability::Query, Capability::Plan, Capability::ControlClock].into_iter()
            .map(|capability| CapabilityGrant { capability,
                scope: CapabilityScope { fortress_id: Some(id), ..CapabilityScope::default() },
                max_risk: RiskTier::Guarded, expires_at_tick: None, remaining_uses: None }).collect(),
        cancellation_requested: false,
    })
}
fn refusal() -> Vec<u8> {
    let p = plan().unwrap();
    let mut raw = prepared().unwrap().canonical_bytes().to_vec();
    let offset = 8 + 2 + p.key().len() + 24 + 2 + p.before().canonical_bytes().len() + 48;
    raw[offset..offset + 5].copy_from_slice(&[4, 3, 0, 0, 0]);
    let n = raw.len() - 32;
    let h = hash(b"dfmcp-excavation-run-receipt/1", &raw[..n]);
    raw[n..].copy_from_slice(h.as_bytes()); raw
}
#[derive(Clone, Default)]
struct Memory { bytes: Arc<Mutex<Vec<u8>>>, pos: u64, syncs: Arc<AtomicUsize>, fail_at: Arc<AtomicUsize> }
impl Read for Memory {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let data = self.bytes.lock().unwrap(); let start = (self.pos as usize).min(data.len());
        let n = out.len().min(data.len() - start); out[..n].copy_from_slice(&data[start..start + n]);
        self.pos += n as u64; Ok(n)
    }
}
impl Write for Memory {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        let mut out = self.bytes.lock().unwrap(); let start = self.pos as usize;
        let end = out.len().max(start + data.len()); out.resize(end, 0);
        out[start..start + data.len()].copy_from_slice(data); self.pos += data.len() as u64; Ok(data.len())
    }
    fn flush(&mut self) -> io::Result<()> { Ok(()) }
}
impl Seek for Memory {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        let n = match from { SeekFrom::Start(n) => i128::from(n), SeekFrom::Current(n) => i128::from(self.pos) + i128::from(n),
            SeekFrom::End(n) => self.bytes.lock().unwrap().len() as i128 + i128::from(n) };
        self.pos = u64::try_from(n).map_err(|_| io::Error::other("seek"))?; Ok(self.pos)
    }
}
impl EffectJournalStorage for Memory {
    fn sync(&mut self) -> io::Result<()> {
        let n = self.syncs.fetch_add(1, Ordering::SeqCst) + 1;
        if n == self.fail_at.load(Ordering::SeqCst) { return Err(io::Error::other("injected sync")); }
        Ok(())
    }
    fn truncate(&mut self, _: u64) -> io::Result<()> { panic!("no repair or truncation") }
}
#[derive(Default)]
struct Behavior {
    lose_commit: bool, bad_nonce: bool, alias: bool, flood: bool, oversized: bool,
    generation: u64, drift: bool, absent: bool, greeting_delay: Duration,
    record: Option<Vec<u8>>, required_syncs: Option<Arc<AtomicUsize>>,
}
struct Server {
    address: SocketAddr, behavior: Arc<Mutex<Behavior>>, calls: Arc<Mutex<Vec<usize>>>,
    stop: Arc<AtomicBool>, thread: Option<JoinHandle<()>>,
}
impl Server {
    fn new(behavior: Behavior) -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        listener.set_nonblocking(true).unwrap(); let address = listener.local_addr().unwrap();
        let state = Arc::new(Mutex::new(behavior)); let calls = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let s = state.clone(); let log = calls.clone(); let stopping = stop.clone();
        let worker = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(15);
            while !stopping.load(Ordering::Acquire) && Instant::now() < deadline {
                match listener.accept() {
                    Ok((mut socket, _)) => {
                        socket.set_read_timeout(Some(Duration::from_millis(200))).unwrap();
                        socket.set_write_timeout(Some(Duration::from_millis(200))).unwrap();
                        socket.set_nodelay(true).unwrap();
                        let _ = Self::serve(&mut socket, &s, &log, &stopping);
                    }
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => thread::sleep(Duration::from_millis(1)),
                    Err(e) => panic!("accept: {e}"),
                }
            }
        });
        Self { address, behavior: state, calls, stop, thread: Some(worker) }
    }
    fn serve(socket: &mut TcpStream, state: &Mutex<Behavior>, calls: &Mutex<Vec<usize>>, stop: &AtomicBool) -> io::Result<()> {
        let mut greeting = [0; 12]; socket.read_exact(&mut greeting)?;
        assert_eq!(&greeting, b"DFHack?\n\x01\0\0\0");
        thread::sleep(state.lock().unwrap().greeting_delay);
        socket.write_all(b"DFHack!\n\x01\0\0\0")?;
        let mut bound = 0;
        while !stop.load(Ordering::Acquire) {
            let mut h = [0; 8]; socket.read_exact(&mut h)?;
            let method = i16::from_le_bytes([h[0], h[1]]);
            let n = i32::from_le_bytes([h[4], h[5], h[6], h[7]]);
            assert!((0..=2048).contains(&n)); let mut raw = vec![0; n as usize]; socket.read_exact(&mut raw)?;
            let request = Message::parse(&raw, 19).unwrap();
            let mut out = Vec::new();
            if method == 0 {
                request.exact(&[1, 2, 3, 4]).unwrap();
                assert_eq!(request.bytes(1).unwrap(), METHODS[bound].as_bytes());
                assert_eq!(request.bytes(2).unwrap(), b"dfmcp.excavation_run.v1_18.Request");
                assert_eq!(request.bytes(3).unwrap(), b"dfmcp.excavation_run.v1_18.Reply");
                assert_eq!(request.bytes(4).unwrap(), b"dfmcp_excavation_run_v1_18");
                let behavior = state.lock().unwrap();
                if behavior.flood { for _ in 0..9 { socket.write_all(&codec::header(-3, 0))?; } }
                number(&mut out, 1, if behavior.alias { 2 } else { bound as u64 + 2 }); bound += 1;
            } else {
                let index = usize::try_from(method - 2).unwrap(); assert!(index < 6); calls.lock().unwrap().push(index);
                assert_eq!(request.bytes(1).unwrap(), &[b't'; 32]);
                assert_eq!((request.number(3).unwrap(), request.number(4).unwrap()), (1, 18));
                let mut behavior = state.lock().unwrap();
                let generation = if behavior.generation == 0 { 41 } else { behavior.generation };
                number(&mut out, 1, 1); number(&mut out, 2, 0);
                bytes(&mut out, 3, if behavior.bad_nonce { &[b'x'; 32] } else { request.bytes(2).unwrap() });
                number(&mut out, 4, 1); number(&mut out, 5, 18);
                number(&mut out, 6, generation + u64::from(behavior.drift && index != 0));
                bytes(&mut out, 7, b"df"); bytes(&mut out, 8, b"dfhack");
                match index {
                    0 => request.exact(&[1, 2, 3, 4]).unwrap(),
                    1 => {
                        request.exact(&[1, 2, 3, 4, 11, 12, 13, 14, 15]).unwrap();
                        bytes(&mut out, 9, plan().unwrap().before().canonical_bytes());
                    }
                    2 => {
                        request.exact(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 11, 12, 13, 14, 15, 16, 17, 18, 19]).unwrap();
                        behavior.record = Some(prepared().unwrap().canonical_bytes().to_vec());
                    }
                    3 => {
                        request.exact(&[1, 2, 3, 4, 5, 9, 10]).unwrap();
                        if let Some(syncs) = &behavior.required_syncs { assert!(syncs.load(Ordering::SeqCst) >= 4); }
                        behavior.record = Some(stopped().unwrap().canonical_bytes().to_vec());
                        if behavior.lose_commit { return Ok(()); } // Native effect retained; reply lost.
                    }
                    4 => request.exact(&[1, 2, 3, 4, 5, 9]).unwrap(),
                    5 => {
                        request.exact(&[1, 2, 3, 4, 5, 9, 10]).unwrap(); behavior.record = Some(refusal());
                    }
                    _ => unreachable!(),
                }
                if index >= 2 && !(index == 4 && behavior.absent) {
                    if let Some(record) = &behavior.record { bytes(&mut out, 10, record); }
                }
                number(&mut out, 11, 0);
                number(&mut out, 12, u64::from(behavior.record.is_some()));
                if behavior.oversized { socket.write_all(&codec::header(-1, 4097))?; return Ok(()); }
            }
            let mut frame = codec::header(-1, out.len() as i32).to_vec(); frame.extend_from_slice(&out);
            for chunk in frame.chunks(7) { socket.write_all(chunk)?; }
        }
        Ok(())
    }
    fn connection(&self, recovery: bool, clock: Arc<AtomicBool>, c: &OperationContext,
        cancellation: ExcavationCancellation) -> Result<ExcavationRpc>
    {
        let settings = Settings { endpoint: self.address, secret: vec![b't'; 32], clock: true };
        let permission: Permission = Box::new(move |required| if required && !clock.load(Ordering::Acquire) { Err(denied()) } else { Ok(()) });
        let p = plan()?;
        let wire = Wire::negotiate(settings, permission, [b'n'; 32], p.before().fortress().clone(), c, cancellation, !recovery)?;
        let binding = ExcavationBinding::new(self.address, "df", "dfhack", p.before())?;
        ExcavationRpc::bootstrap(wire, p.before().region(), recovery.then_some(&binding), c)
    }
    fn connect(&self, recovery: bool) -> Result<ExcavationRpc> {
        self.connection(recovery, Arc::new(AtomicBool::new(true)), &context()?, ExcavationCancellation::default())
    }
    fn count(&self, index: usize) -> usize { self.calls.lock().unwrap().iter().filter(|n| **n == index).count() }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.thread.take() {
            let result = worker.join();
            if !thread::panicking() { assert!(result.is_ok(), "TCP double failed"); }
        }
    }
}

#[test]
fn requests_match_independent_golden_envelopes() -> Result<()> {
    let p = plan()?;
    let requests = [Request::Handshake, Request::Observe(p.before().region()), Request::Prepare(&p),
        Request::Commit(&p), Request::Query(&p), Request::Cancel(&p)];
    let rows: Vec<_> = include_str!("../../../tests/fixtures/excavation_run_rpc_v1_18.txt").lines().collect();
    assert_eq!(rows.len(), requests.len());
    for (request, row) in requests.into_iter().zip(rows) {
        let (name, hex) = row.split_once(' ').unwrap(); assert_eq!(name, METHODS[request.index()]);
        let bytes: Vec<_> = (0..hex.len()).step_by(2).map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap()).collect();
        assert_eq!(request.encode(&[b't'; 32], &[b'n'; 32]), bytes);
    }
    Ok(())
}
#[test]
fn closed_envelope_rejects_wrong_types_nonminimal_overflow_duplicate_unknown_and_truncation() {
    for bad in [vec![8, 128, 0], vec![8, 128], vec![8; 11], vec![0], vec![104, 1], vec![8, 1, 8, 1],
        vec![10, 4, 1], vec![11], vec![8, 255, 255, 255, 255, 255, 255, 255, 255, 255, 2]]
    { assert!(Message::parse(&bad, 12).is_err()); }
    let m = Message::parse(&[10, 1, 1], 12).unwrap(); assert!(m.number(1).is_err());
    assert!(Message::parse(&[8, 2], 12).unwrap().boolean(1).is_err());
    assert!(Message::parse(&vec![0; 4097], 12).is_err());
}
#[test]
fn operator_environment_is_exact_and_loopback_only() {
    let valid = BTreeMap::from([(OPT_IN.to_owned(), "1".to_owned()), (TOKEN.to_owned(), "t".repeat(32))]);
    assert!(!Settings::parse(&valid).unwrap().clock);
    for (key, value) in [(OPT_IN, "true"), (TOKEN, "short"), (CLOCK, "yes"), (ENDPOINT, "localhost:5000"),
        (ENDPOINT, "192.168.1.1:5000"), (ENDPOINT, "127.0.0.1:05000"), (ENDPOINT, "127.0.0.1:0"),
        ("DFMCP_ADMITTED_BRIDGE_PROTOCOL", "1.18"), ("DFMCP_RUN_TOKEN", "other")]
    { let mut v = valid.clone(); v.insert(key.to_owned(), value.to_owned()); assert!(Settings::parse(&v).is_err()); }
}
#[test]
fn coordinator_joins_real_tcp_start_to_synced_dispatch() -> Result<()> {
    let memory = Memory::default();
    let server = Server::new(Behavior { required_syncs: Some(memory.syncs.clone()), ..Behavior::default() });
    let mut source = server.connect(false)?; let p = plan()?; let c = context()?;
    let mut owner = ExcavationCoordinator::create(memory.clone(), source.binding().clone(), &c)?;
    let result = owner.start(&mut source, p.clone(), p.digest(), &c)?;
    assert!(result.resolved()); assert_eq!(server.count(3), 1); assert_eq!(owner.pending_count(), 0);
    let replay = ExcavationCoordinator::open(memory, p.before().fortress(), &c)?;
    assert_eq!(replay.entry(p.key()).unwrap().native(), Some(&result));
    Ok(())
}
#[test]
fn lost_commit_reconnects_query_only_without_another_unpause() -> Result<()> {
    let memory = Memory::default(); let server = Server::new(Behavior { lose_commit: true, ..Behavior::default() });
    let p = plan()?; let c = context()?;
    {
        let mut source = server.connect(false)?;
        let mut owner = ExcavationCoordinator::create(memory.clone(), source.binding().clone(), &c)?;
        assert!(owner.start(&mut source, p.clone(), p.digest(), &c).is_err());
        assert!(source.is_fenced()); assert_eq!(owner.pending_count(), 1);
    }
    let observations = server.count(1);
    let mut source = server.connect(true)?;
    let mut owner = ExcavationCoordinator::open(memory, p.before().fortress(), &c)?;
    assert!(owner.recover(&mut source, p.key(), false, &c)?.unwrap().resolved());
    assert_eq!(server.count(3), 1); assert_eq!(server.count(1), observations);
    Ok(())
}
#[test]
fn recovery_refuses_control_before_native_dispatch() -> Result<()> {
    let server = Server::new(Behavior::default()); let mut source = server.connect(true)?;
    assert!(source.prepare(&plan()?, &context()?, Duration::from_secs(1)).is_err());
    assert!(source.is_fenced()); assert_eq!(*server.calls.lock().unwrap(), vec![0]);
    Ok(())
}
#[test]
fn later_generation_can_return_old_terminal_history_but_cannot_cancel_old_source() -> Result<()> {
    let server = Server::new(Behavior { generation: 42, record: Some(stopped()?.canonical_bytes().to_vec()), ..Behavior::default() });
    let mut source = server.connect(true)?; let p = plan()?; let c = context()?;
    assert_eq!(source.binding().generation(), 42);
    assert!(source.query(&p, &c, Duration::from_secs(1))?.unwrap().resolved());
    assert!(source.cancel(&p, &c, Duration::from_secs(1)).is_err()); assert_eq!(server.count(5), 0);
    Ok(())
}
#[test]
fn source_drift_fences_connection() -> Result<()> {
    let server = Server::new(Behavior { drift: true, ..Behavior::default() });
    let mut source = server.connect(true)?; let p = plan()?; let c = context()?;
    assert!(source.query(&p, &c, Duration::from_secs(1)).is_err());
    assert!(source.query(&p, &c, Duration::from_secs(1)).is_err()); assert_eq!(server.count(4), 1);
    Ok(())
}
#[test]
fn bad_nonce_alias_notification_flood_and_oversize_refuse_negotiation() {
    for behavior in [Behavior { bad_nonce: true, ..Behavior::default() }, Behavior { alias: true, ..Behavior::default() },
        Behavior { flood: true, ..Behavior::default() }, Behavior { oversized: true, ..Behavior::default() }]
    { let server = Server::new(behavior); assert!(server.connect(false).is_err()); }
}
#[test]
fn revoked_unpause_permission_leaves_authorized_query_and_cancel() -> Result<()> {
    let server = Server::new(Behavior::default()); let clock = Arc::new(AtomicBool::new(true));
    let c = context()?; let p = plan()?;
    let mut source = server.connection(false, clock.clone(), &c, ExcavationCancellation::default())?;
    source.prepare(&p, &c, Duration::from_secs(1))?;
    clock.store(false, Ordering::Release);
    assert!(source.query(&p, &c, Duration::from_secs(1))?.is_some());
    assert_eq!(source.cancel(&p, &c, Duration::from_secs(1))?.phase(), RunPhase::Refused);
    assert_eq!(server.count(5), 1); assert_eq!(server.count(3), 0);
    Ok(())
}
#[test]
fn missing_clock_grant_and_short_horizon_refuse_before_prepare() -> Result<()> {
    for expire in [false, true] {
        let server = Server::new(Behavior::default()); let mut source = server.connect(false)?;
        let mut c = context()?; let p = plan()?;
        if expire { for grant in &mut c.grants { grant.expires_at_tick = Some(GameTick(p.before().tick() + 99)); } }
        else { c.grants.retain(|g| g.capability != Capability::ControlClock); }
        assert!(source.prepare(&p, &c, Duration::from_secs(1)).is_err()); assert_eq!(server.count(2), 0);
    }
    Ok(())
}
#[test]
fn dispatch_sync_failure_prevents_the_real_tcp_commit() -> Result<()> {
    let memory = Memory::default(); memory.fail_at.store(4, Ordering::SeqCst);
    let server = Server::new(Behavior::default()); let mut source = server.connect(false)?;
    let p = plan()?; let c = context()?;
    let mut owner = ExcavationCoordinator::create(memory, source.binding().clone(), &c)?;
    assert!(owner.start(&mut source, p.clone(), p.digest(), &c).is_err());
    assert!(owner.is_fenced()); assert_eq!(server.count(2), 1); assert_eq!(server.count(3), 0);
    Ok(())
}
#[test]
fn a_seen_native_record_cannot_disappear() -> Result<()> {
    let server = Server::new(Behavior { record: Some(prepared()?.canonical_bytes().to_vec()), ..Behavior::default() });
    let mut source = server.connect(true)?; let p = plan()?; let c = context()?;
    source.query(&p, &c, Duration::from_secs(1))?; server.behavior.lock().unwrap().absent = true;
    assert!(source.query(&p, &c, Duration::from_secs(1)).is_err()); assert!(source.is_fenced());
    Ok(())
}
#[test]
fn cancellation_is_checked_during_a_stalled_greeting() -> Result<()> {
    let server = Server::new(Behavior { greeting_delay: Duration::from_millis(250), ..Behavior::default() });
    let cancellation = ExcavationCancellation::default(); let signal = cancellation.clone();
    let started = Instant::now(); let c = context()?;
    thread::scope(|scope| {
        scope.spawn(move || { thread::sleep(Duration::from_millis(20)); signal.cancel(); });
        assert!(server.connection(true, Arc::new(AtomicBool::new(true)), &c, cancellation).is_err());
    });
    assert!(started.elapsed() < Duration::from_secs(1));
    Ok(())
}
#[test]
fn greeting_consumes_original_deadline_and_pre_cancel_does_not_connect() -> Result<()> {
    let server = Server::new(Behavior { greeting_delay: Duration::from_millis(100), ..Behavior::default() });
    let mut c = context()?; c.budget.max_wall_millis = 10;
    assert!(server.connection(true, Arc::new(AtomicBool::new(true)), &c, ExcavationCancellation::default()).is_err());
    let cancellation = ExcavationCancellation::default(); cancellation.cancel();
    assert!(server.connection(true, Arc::new(AtomicBool::new(true)), &context()?, cancellation).is_err());
    assert!(server.calls.lock().unwrap().is_empty()); Ok(())
}
#[test]
fn connection_call_budget_cannot_be_renewed_by_later_contexts() -> Result<()> {
    let server = Server::new(Behavior::default()); let mut source = server.connect(true)?;
    let p = plan()?; let c = context()?;
    for _ in 0..57 { assert!(source.query(&p, &c, Duration::from_secs(5))?.is_none()); }
    assert!(source.query(&p, &c, Duration::from_secs(1)).is_err());
    assert_eq!(server.count(4), 57); assert!(source.is_fenced()); Ok(())
}

#[cfg(all(target_os = "linux", any(target_arch = "x86_64", target_arch = "aarch64")))]
#[test]
fn actual_tcp_commit_loss_recovers_from_actual_private_disk_journal() -> Result<()> {
    use crate::excavation_run::private_file::{create_private_excavation, open_private_excavation, inspect_private_excavation};
    use std::os::unix::fs::DirBuilderExt;
    let dir = std::env::temp_dir().canonicalize().unwrap().join(format!("dfmcp-joined-rpc-file-{}-{}",
        std::process::id(), 17));
    // No deleting or reusing a prior run's evidence directory.
    let mut candidate = dir.clone(); let mut index = 0;
    loop {
        match std::fs::DirBuilder::new().mode(0o700).create(&candidate) {
            Ok(()) => break,
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => { index += 1; candidate = dir.with_extension(index.to_string()); }
            Err(e) => panic!("private test directory: {e}"),
        }
    }
    let server = Server::new(Behavior { lose_commit: true, ..Behavior::default() });
    let p = plan()?; let c = context()?;
    {
        let mut source = server.connect(false)?;
        let mut owner = create_private_excavation(&candidate, source.binding().clone(), &c)?;
        assert!(owner.start(&mut source, p.clone(), p.digest(), &c).is_err());
    }
    assert_eq!(inspect_private_excavation(&candidate, p.before().fortress(), &c)?.pending_count(), 1);
    let observations = server.count(1);
    {
        let mut owner = open_private_excavation(&candidate, p.before().fortress(), &c)?;
        let mut source = server.connect(true)?;
        assert!(owner.recover(&mut source, p.key(), false, &c)?.unwrap().resolved());
    }
    assert_eq!(server.count(3), 1); assert_eq!(server.count(1), observations);
    let archive = inspect_private_excavation(&candidate, p.before().fortress(), &c)?;
    assert_eq!(archive.pending_count(), 0); assert!(archive.entries()[0].native().unwrap().resolved());
    Ok(())
}
