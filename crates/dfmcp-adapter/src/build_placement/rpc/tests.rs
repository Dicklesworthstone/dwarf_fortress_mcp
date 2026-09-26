//! Joined TCP peers exercise the actual fixed transport; these are explicit doubles.
use super::*;
use crate::build_placement::journal::{BuildGuard, BuildJournal, BuildMode, BuildStage};
use crate::build_placement::tests::fixture;
use crate::control_effect_journal::EffectJournalStorage;
use dfmcp_core::{
    CapabilityGrant, CapabilityScope, ObservationCursor, RequestId, SessionId, StateAnchor,
    WorkBudget,
};
use std::io::{Cursor, Read, Seek, SeekFrom, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::AtomicUsize;
use std::thread::{self, JoinHandle};
use std::time::Instant;

fn plan() -> Result<BuildPlan> {
    BuildPlan::new("golden", BuildCapture::decode(&fixture("capture")?)?)
}
fn context() -> Result<OperationContext> {
    let plan = plan()?;
    let id = plan.before().fortress_id();
    Ok(OperationContext {
        session_id: SessionId::new(1),
        request_id: RequestId::new(2),
        anchor: StateAnchor {
            fortress_id: id,
            cursor: ObservationCursor::ORIGIN,
            tick: GameTick(plan.before().tick()),
            state_hash: plan.before().witness(),
        },
        budget: WorkBudget {
            max_wall_millis: 5000,
            max_bytes: 32 * 1024 * 1024,
            ..WorkBudget::CONSERVATIVE_DEFAULT
        },
        grants: [
            Capability::Query,
            Capability::Observe,
            Capability::Plan,
            Capability::Construct,
        ]
        .into_iter()
        .map(|capability| CapabilityGrant {
            capability,
            scope: CapabilityScope {
                fortress_id: Some(id),
                ..CapabilityScope::default()
            },
            max_risk: RiskTier::Guarded,
            expires_at_tick: None,
            remaining_uses: None,
        })
        .collect(),
        cancellation_requested: false,
    })
}

#[derive(Default)]
struct Memory {
    inner: Cursor<Vec<u8>>,
    syncs: Arc<AtomicUsize>,
}
impl Read for Memory {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        self.inner.read(bytes)
    }
}
impl Write for Memory {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.inner.write(bytes)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}
impl Seek for Memory {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        self.inner.seek(from)
    }
}
impl EffectJournalStorage for Memory {
    fn sync(&mut self) -> io::Result<()> {
        self.syncs.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    fn truncate(&mut self, _: u64) -> io::Result<()> {
        Err(io::Error::other("test rejects every repair/truncation"))
    }
}
struct Guard;
impl BuildGuard for Guard {
    fn check(
        &mut self,
        _: BuildStage,
        _: &BuildBinding,
        _: Option<&BuildPlan>,
        _: BuildSelection,
        _: &OperationContext,
    ) -> Result<()> {
        Ok(())
    }
}

struct Step {
    op: usize,
    record: Option<Vec<u8>>,
    replayed: bool,
    unresolved: bool,
    retained: u64,
    generation: u64,
    extra: Vec<u8>,
    drop_reply: bool,
    signal: Option<Arc<AtomicBool>>,
}
impl Step {
    fn new(op: usize, record: Option<&str>) -> Result<Self> {
        Ok(Self {
            op,
            record: record.map(fixture).transpose()?,
            replayed: false,
            unresolved: record == Some("indeterminate"),
            retained: u64::from(record.is_some()),
            generation: 41,
            extra: Vec::new(),
            drop_reply: false,
            signal: None,
        })
    }
}
struct Dialogue {
    generation: u64,
    retained: u64,
    unresolved: bool,
    steps: Vec<Step>,
}
impl Dialogue {
    fn new(steps: Vec<Step>) -> Self {
        Self {
            generation: 41,
            retained: 0,
            unresolved: false,
            steps,
        }
    }
}
struct Peer {
    endpoint: SocketAddr,
    join: JoinHandle<Result<Vec<usize>>>,
}
fn read_request(stream: &mut TcpStream) -> Result<(i16, Vec<u8>)> {
    let mut header = [0; 8];
    stream.read_exact(&mut header).map_err(io_error)?;
    let method = i16::from_le_bytes([header[0], header[1]]);
    let n = i32::from_le_bytes([header[4], header[5], header[6], header[7]]);
    require((0..=2048).contains(&n), "test peer request bound")?;
    let mut bytes = vec![0; n as usize];
    stream.read_exact(&mut bytes).map_err(io_error)?;
    Ok((method, bytes))
}
fn send(stream: &mut TcpStream, raw: &[u8]) -> Result<()> {
    stream
        .write_all(&codec::header(-1, raw.len() as i32))
        .map_err(io_error)?;
    stream.write_all(raw).map_err(io_error)
}
fn reply(nonce: &[u8], generation: u64, retained: u64, unresolved: bool) -> Vec<u8> {
    let mut out = Vec::new();
    number(&mut out, 1, 1);
    number(&mut out, 2, 0);
    bytes(&mut out, 3, nonce);
    number(&mut out, 4, 1);
    number(&mut out, 5, 19);
    number(&mut out, 6, generation);
    bytes(&mut out, 7, b"53.01");
    bytes(&mut out, 8, b"53.01-r1");
    number(&mut out, 12, u64::from(unresolved));
    number(&mut out, 13, retained);
    out
}
fn validate_request(raw: &[u8], op: usize) -> Result<Vec<u8>> {
    let request = Message::parse(raw, 13)?;
    let mut fields = vec![1, 2, 3, 4];
    if matches!(op, 1 | 2) {
        fields.extend(5..=9);
    }
    if matches!(op, 2..=5) {
        fields.extend([10, 12]);
    }
    if op == 2 {
        fields.push(11);
    }
    if matches!(op, 3 | 5) {
        fields.push(13);
    }
    request.exact(&fields)?;
    require(
        request.bytes(1)? == [b't'; 32]
            && request.bytes(2)?.len() == 32
            && request.number(3)? == 1
            && request.number(4)? == 19,
        "test peer credential/profile mismatch",
    )?;
    let plan = plan()?;
    if matches!(op, 1 | 2) {
        for (tag, n) in (5..=9).zip(plan.before().selection().values()) {
            require(
                request.number(tag)? == u64::from(n),
                "test peer selection differs",
            )?;
        }
    }
    if matches!(op, 2..=5) {
        require(
            request.bytes(10)? == plan.key().as_bytes()
                && request.bytes(12)? == plan.digest().as_bytes(),
            "test peer plan differs",
        )?;
    }
    if op == 2 {
        require(
            request.bytes(11)? == plan.before().witness().as_bytes(),
            "test peer witness differs",
        )?;
    }
    if matches!(op, 3 | 5) {
        require(
            request.bytes(13)? == plan.token(),
            "test peer token differs",
        )?;
    }
    Ok(request.bytes(2)?.to_vec())
}
fn peer(dialogues: Vec<Dialogue>, syncs: Option<Arc<AtomicUsize>>) -> Result<Peer> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(io_error)?;
    let endpoint = listener.local_addr().map_err(io_error)?;
    let join = thread::spawn(move || {
        let mut calls = Vec::new();
        for dialogue in dialogues {
            let (mut stream, _) = listener.accept().map_err(io_error)?;
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .map_err(io_error)?;
            stream
                .set_write_timeout(Some(Duration::from_secs(5)))
                .map_err(io_error)?;
            let mut greeting = [0; 12];
            stream.read_exact(&mut greeting).map_err(io_error)?;
            require(&greeting == b"DFHack?\n\x01\0\0\0", "test peer greeting")?;
            stream.write_all(b"DFHack!\n\x01\0\0\0").map_err(io_error)?;
            for (index, name) in METHODS.iter().enumerate() {
                let (id, raw) = read_request(&mut stream)?;
                require(id == 0, "test peer binding method")?;
                let request = Message::parse(&raw, 4)?;
                request.exact(&[1, 2, 3, 4])?;
                require(
                    request.bytes(1)? == name.as_bytes()
                        && request.bytes(2)? == b"dfmcp.build.v1_19.Request"
                        && request.bytes(3)? == b"dfmcp.build.v1_19.Reply"
                        && request.bytes(4)? == b"dfmcp_build_v1_19",
                    "test peer incorrect native binding",
                )?;
                let mut raw = Vec::new();
                number(&mut raw, 1, index as u64 + 2);
                send(&mut stream, &raw)?;
            }
            let (id, raw) = read_request(&mut stream)?;
            require(id == 2, "test peer handshake method")?;
            let nonce = validate_request(&raw, 0)?;
            send(
                &mut stream,
                &reply(
                    &nonce,
                    dialogue.generation,
                    dialogue.retained,
                    dialogue.unresolved,
                ),
            )?;
            for step in dialogue.steps {
                let (id, raw) = read_request(&mut stream)?;
                require(
                    id == step.op as i16 + 2,
                    "test peer unexpected native operation",
                )?;
                let nonce = validate_request(&raw, step.op)?;
                calls.push(step.op);
                if step.op == 3 {
                    if let Some(syncs) = &syncs {
                        require(
                            syncs.load(Ordering::SeqCst) >= 4,
                            "commit preceded dispatch sync",
                        )?;
                    }
                }
                if step.drop_reply {
                    break;
                }
                if let Some(signal) = step.signal {
                    signal.store(true, Ordering::Release);
                    thread::sleep(Duration::from_millis(250));
                    break;
                }
                let mut raw = reply(&nonce, step.generation, step.retained, step.unresolved);
                if step.op == 1 {
                    bytes(&mut raw, 9, &fixture("capture")?);
                }
                if let Some(record) = step.record {
                    bytes(&mut raw, 10, &record);
                }
                if step.op == 2 {
                    number(&mut raw, 11, u64::from(step.replayed));
                }
                raw.extend_from_slice(&step.extra);
                send(&mut stream, &raw)?;
            }
        }
        Ok(calls)
    });
    Ok(Peer { endpoint, join })
}
fn join(peer: Peer) -> Result<Vec<usize>> {
    peer.join.join().map_err(|_| malformed())?
}
fn connect(
    peer: &Peer,
    context: &OperationContext,
    cancellation: BuildCancellation,
    permission: BuildPermission,
) -> Result<BuildRpc> {
    let plan = plan()?;
    BuildRpc::connect_trusted(
        peer.endpoint,
        vec![b't'; 32],
        [b'n'; 32],
        plan.before().fortress().clone(),
        plan.before().selection(),
        context,
        cancellation,
        permission,
    )
}
fn steps_to_commit(record: &str, lose: bool) -> Result<Vec<Step>> {
    let mut before_commit = Step::new(1, None)?;
    before_commit.retained = 1;
    let mut commit = Step::new(3, Some(record))?;
    commit.drop_reply = lose;
    Ok(vec![
        Step::new(1, None)?,
        Step::new(1, None)?,
        Step::new(4, None)?,
        Step::new(2, Some("prepared"))?,
        before_commit,
        commit,
    ])
}

#[test]
fn actual_tcp_placement_requires_synced_dispatch_and_never_replays_commit() -> Result<()> {
    let memory = Memory::default();
    let syncs = memory.syncs.clone();
    let peer = peer(
        vec![Dialogue::new(steps_to_commit("placed", false)?)],
        Some(syncs),
    )?;
    let context = context()?;
    let plan = plan()?;
    let mut source = connect(
        &peer,
        &context,
        BuildCancellation::default(),
        Box::new(|_| Ok(())),
    )?;
    let mut journal = BuildJournal::open(
        memory,
        &context,
        BuildMode::Control,
        Some(source.binding().clone()),
        Some([1; 32]),
    )?;
    let entry = journal.prepare(&mut source, &plan, &context, &mut Guard)?;
    assert_eq!(
        entry.native().map(BuildRecord::phase),
        Some(BuildPhase::Prepared)
    );
    let entry = journal.commit(&mut source, plan.key(), plan.digest(), &context, &mut Guard)?;
    assert_eq!(
        entry.native().map(BuildRecord::phase),
        Some(BuildPhase::Placed)
    );
    assert!(!entry.unresolved());
    let duplicate = journal.commit(&mut source, plan.key(), plan.digest(), &context, &mut Guard)?;
    assert_eq!(entry, duplicate);
    drop(source);
    assert_eq!(join(peer)?, vec![1, 1, 4, 2, 1, 3]);
    Ok(())
}

#[test]
fn lost_commit_recovers_exact_history_on_query_only_connection() -> Result<()> {
    let mut recovery = Dialogue::new(vec![Step::new(4, Some("placed"))?]);
    recovery.retained = 1;
    let peer = peer(
        vec![Dialogue::new(steps_to_commit("placed", true)?), recovery],
        None,
    )?;
    let context = context()?;
    let plan = plan()?;
    let mut source = connect(
        &peer,
        &context,
        BuildCancellation::default(),
        Box::new(|_| Ok(())),
    )?;
    let original = source.binding().clone();
    let mut journal = BuildJournal::open(
        Memory::default(),
        &context,
        BuildMode::Control,
        Some(original.clone()),
        Some([1; 32]),
    )?;
    journal.prepare(&mut source, &plan, &context, &mut Guard)?;
    assert!(
        journal
            .commit(&mut source, plan.key(), plan.digest(), &context, &mut Guard)
            .is_err()
    );
    assert!(source.is_fenced());
    drop(source);
    let mut recovery = BuildRpc::connect_trusted_recovery(
        &original,
        vec![b't'; 32],
        [b'r'; 32],
        plan.before().selection(),
        &context,
        BuildCancellation::default(),
        Box::new(|_| Ok(())),
    )?;
    assert!(recovery.initial_capture().is_none());
    let entry = journal.recover(
        &mut recovery,
        plan.key(),
        plan.digest(),
        false,
        &context,
        &mut Guard,
    )?;
    assert_eq!(
        entry.native().map(BuildRecord::phase),
        Some(BuildPhase::Placed)
    );
    assert!(
        recovery
            .prepare(&plan, &context, Duration::from_secs(1))
            .is_err()
    );
    drop(recovery);
    assert_eq!(join(peer)?, vec![1, 1, 4, 2, 1, 3, 4]);
    Ok(())
}

#[test]
fn recovery_read_relinquishes_fresh_permit_and_cannot_reprepare() -> Result<()> {
    let peer = peer(
        vec![Dialogue::new(vec![
            Step::new(1, None)?,
            Step::new(2, Some("prepared"))?,
            Step::new(4, Some("prepared"))?,
        ])],
        None,
    )?;
    let context = context()?;
    let plan = plan()?;
    let mut source = connect(
        &peer,
        &context,
        BuildCancellation::default(),
        Box::new(|_| Ok(())),
    )?;
    let preparation = source.prepare(&plan, &context, Duration::from_secs(1))?;
    assert!(!preparation.replayed());
    assert!(source.prepared.is_some());
    source.query(&plan, &context, Duration::from_secs(1))?;
    assert!(source.prepared.is_none());
    assert!(
        source
            .prepare(&plan, &context, Duration::from_secs(1))
            .is_err()
    );
    drop(source);
    assert_eq!(join(peer)?, vec![1, 2, 4]);
    Ok(())
}

#[test]
fn replayed_preparation_never_creates_permit() -> Result<()> {
    for name in ["prepared", "placed", "indeterminate"] {
        let mut step = Step::new(2, Some(name))?;
        step.replayed = true;
        let peer = peer(vec![Dialogue::new(vec![Step::new(1, None)?, step])], None)?;
        let context = context()?;
        let plan = plan()?;
        let mut source = connect(
            &peer,
            &context,
            BuildCancellation::default(),
            Box::new(|_| Ok(())),
        )?;
        assert!(
            source
                .prepare(&plan, &context, Duration::from_secs(1))?
                .replayed()
        );
        assert!(source.prepared.is_none());
        drop(source);
        assert_eq!(join(peer)?, vec![1, 2]);
    }
    Ok(())
}

#[test]
fn immutable_indeterminate_record_cannot_be_replaced_by_placed_history() -> Result<()> {
    let first = Step::new(4, Some("indeterminate"))?;
    let mut second = Step::new(4, Some("placed"))?;
    second.unresolved = true;
    let peer = peer(
        vec![Dialogue::new(vec![Step::new(1, None)?, first, second])],
        None,
    )?;
    let context = context()?;
    let plan = plan()?;
    let mut source = connect(
        &peer,
        &context,
        BuildCancellation::default(),
        Box::new(|_| Ok(())),
    )?;
    assert_eq!(
        source
            .query(&plan, &context, Duration::from_secs(1))?
            .map(|r| r.phase()),
        Some(BuildPhase::Indeterminate)
    );
    assert!(
        source
            .query(&plan, &context, Duration::from_secs(1))
            .is_err()
    );
    assert!(source.is_fenced());
    drop(source);
    assert_eq!(join(peer)?, vec![1, 4, 4]);
    Ok(())
}

#[test]
fn known_record_disappearance_fences_connection() -> Result<()> {
    let mut absent = Step::new(4, None)?;
    absent.retained = 1;
    let peer = peer(
        vec![Dialogue::new(vec![
            Step::new(1, None)?,
            Step::new(4, Some("prepared"))?,
            absent,
        ])],
        None,
    )?;
    let context = context()?;
    let plan = plan()?;
    let mut source = connect(
        &peer,
        &context,
        BuildCancellation::default(),
        Box::new(|_| Ok(())),
    )?;
    source.query(&plan, &context, Duration::from_secs(1))?;
    assert!(
        source
            .query(&plan, &context, Duration::from_secs(1))
            .is_err()
    );
    assert!(source.is_fenced());
    drop(source);
    assert_eq!(join(peer)?, vec![1, 4, 4]);
    Ok(())
}

#[test]
fn placement_revocation_preserves_authorized_preparation_retirement() -> Result<()> {
    let peer = peer(
        vec![Dialogue::new(vec![
            Step::new(1, None)?,
            Step::new(2, Some("prepared"))?,
            Step::new(5, Some("cancelled"))?,
        ])],
        None,
    )?;
    let context = context()?;
    let plan = plan()?;
    let allowed = Arc::new(AtomicBool::new(true));
    let guard = allowed.clone();
    let permission = Box::new(move |place| {
        if place && !guard.load(Ordering::Acquire) {
            Err(denied())
        } else {
            Ok(())
        }
    });
    let mut source = connect(&peer, &context, BuildCancellation::default(), permission)?;
    source.prepare(&plan, &context, Duration::from_secs(1))?;
    allowed.store(false, Ordering::Release);
    let mut read_only = context.clone();
    read_only
        .grants
        .retain(|g| g.capability == Capability::Query);
    assert_eq!(
        source
            .cancel(&plan, &read_only, Duration::from_secs(1))?
            .phase(),
        BuildPhase::Cancelled
    );
    assert!(source.prepared.is_none());
    drop(source);
    assert_eq!(join(peer)?, vec![1, 2, 5]);
    Ok(())
}

#[test]
fn dynamic_runtime_cancellation_interrupts_blocked_reply() -> Result<()> {
    let signal = Arc::new(AtomicBool::new(false));
    let mut step = Step::new(4, Some("prepared"))?;
    step.signal = Some(signal.clone());
    let peer = peer(vec![Dialogue::new(vec![Step::new(1, None)?, step])], None)?;
    let context = context()?;
    let plan = plan()?;
    let cancellation = BuildCancellation::with_check(Arc::new(move || {
        if signal.load(Ordering::Acquire) {
            Err(cancelled())
        } else {
            Ok(())
        }
    }));
    let mut source = connect(&peer, &context, cancellation, Box::new(|_| Ok(())))?;
    let started = Instant::now();
    assert!(
        source
            .query(&plan, &context, Duration::from_secs(1))
            .is_err()
    );
    assert!(started.elapsed() < Duration::from_millis(500));
    assert!(source.is_fenced());
    drop(source);
    assert_eq!(join(peer)?, vec![1, 4]);
    Ok(())
}

#[test]
fn canonical_envelope_rejects_duplicate_unknown_mistyped_and_nonminimal_fields() -> Result<()> {
    let mut duplicate = Vec::new();
    number(&mut duplicate, 12, 0);
    let mut unknown = Vec::new();
    number(&mut unknown, 14, 0);
    let mut mistyped = Vec::new();
    bytes(&mut mistyped, 10, &fixture("prepared")?);
    bytes(&mut mistyped, 11, b"0");
    for extra in [
        duplicate,
        unknown,
        vec![0x80, 0x00],
        vec![0x72, 0x01, 0x00],
        mistyped,
    ] {
        let mut step = Step::new(4, None)?;
        step.extra = extra;
        let peer = peer(vec![Dialogue::new(vec![Step::new(1, None)?, step])], None)?;
        let context = context()?;
        let plan = plan()?;
        let mut source = connect(
            &peer,
            &context,
            BuildCancellation::default(),
            Box::new(|_| Ok(())),
        )?;
        assert!(
            source
                .query(&plan, &context, Duration::from_secs(1))
                .is_err()
        );
        assert!(source.is_fenced());
        drop(source);
        assert_eq!(join(peer)?, vec![1, 4]);
    }
    Ok(())
}

#[test]
fn retained_history_can_cross_generation_only_on_explicit_recovery_connection() -> Result<()> {
    let mut query = Step::new(4, Some("placed"))?;
    query.generation = 42;
    let mut recovery = Dialogue::new(vec![query]);
    recovery.generation = 42;
    recovery.retained = 1;
    let peer = peer(
        vec![Dialogue::new(vec![Step::new(1, None)?]), recovery],
        None,
    )?;
    let context = context()?;
    let plan = plan()?;
    let source = connect(
        &peer,
        &context,
        BuildCancellation::default(),
        Box::new(|_| Ok(())),
    )?;
    let binding = source.binding().clone();
    drop(source);
    let mut source = BuildRpc::connect_trusted_recovery(
        &binding,
        vec![b't'; 32],
        [b'r'; 32],
        plan.before().selection(),
        &context,
        BuildCancellation::default(),
        Box::new(|_| Ok(())),
    )?;
    assert_eq!(source.binding().generation(), 42);
    assert_eq!(
        source
            .query(&plan, &context, Duration::from_secs(1))?
            .map(|r| r.phase()),
        Some(BuildPhase::Placed)
    );
    drop(source);
    assert_eq!(join(peer)?, vec![1, 4]);
    Ok(())
}

#[test]
fn connection_deadline_does_not_renew_and_scope_restrictions_fail_before_io() -> Result<()> {
    let peer = peer(vec![Dialogue::new(vec![Step::new(1, None)?])], None)?;
    let context = context()?;
    let plan = plan()?;
    let mut source = connect(
        &peer,
        &context,
        BuildCancellation::default(),
        Box::new(|_| Ok(())),
    )?;
    source
        .wire
        .link
        .narrow(Duration::from_millis(20), 512 * 1024)?;
    thread::sleep(Duration::from_millis(30));
    assert!(
        source
            .query(&plan, &context, Duration::from_secs(5))
            .is_err()
    );
    assert!(source.is_fenced());
    drop(source);
    assert_eq!(join(peer)?, vec![1]);
    let mut narrow = context.clone();
    for grant in &mut narrow.grants {
        grant.scope.map_area = Some(dfmcp_core::MapCuboid {
            min: dfmcp_core::MapCoord::new(0, 0, 0),
            max: dfmcp_core::MapCoord::new(63, 63, 7),
        });
    }
    assert!(
        authority(
            &narrow,
            plan.before().fortress(),
            true,
            true,
            plan.before().tick()
        )
        .is_err()
    );
    let mut expired = context;
    for grant in &mut expired.grants {
        grant.expires_at_tick = Some(GameTick(plan.before().tick() - 1));
    }
    assert!(
        authority(
            &expired,
            plan.before().fortress(),
            true,
            true,
            plan.before().tick()
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn standalone_environment_rejects_production_state_and_noncanonical_endpoints() -> Result<()> {
    let mut values = BTreeMap::from([
        (OPT_IN.to_owned(), "1".to_owned()),
        (TOKEN.to_owned(), "t".repeat(32)),
    ]);
    assert!(Settings::parse(&values).is_ok());
    for endpoint in [
        "localhost:5000",
        "192.0.2.1:5000",
        "127.0.0.1:05000",
        "[::1]:5000",
        "127.0.0.1:0",
    ] {
        values.insert(ENDPOINT.to_owned(), endpoint.to_owned());
        assert!(Settings::parse(&values).is_err());
    }
    values.remove(ENDPOINT);
    values.insert("DFMCP_ADMITTED_BRIDGE_PROTOCOL".to_owned(), String::new());
    assert!(Settings::parse(&values).is_err());
    Ok(())
}
