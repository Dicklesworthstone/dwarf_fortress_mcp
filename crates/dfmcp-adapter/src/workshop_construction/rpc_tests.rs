//! Joined loopback peers exercise production workshop framing and real disk custody.
//! Native vectors are explicit component fixtures, never live DF evidence.
use super::*;
use crate::workshop_construction::journal::{
    WorkshopGuard, WorkshopMode, WorkshopStage, open_private_workshop,
};
use crate::workshop_construction::tests::{capture, plan, vector};
use dfmcp_core::{
    CapabilityGrant, CapabilityScope, ObservationCursor, RequestId, SessionId, StateAnchor,
    WorkBudget,
};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use std::thread::{self, JoinHandle};
use std::time::Instant;

fn io_error(_: std::io::Error) -> dfmcp_core::DfmcpError {
    malformed()
}
fn context() -> Result<OperationContext> {
    let c = capture()?;
    Ok(OperationContext {
        session_id: SessionId::new(1),
        request_id: RequestId::new(2),
        anchor: StateAnchor {
            fortress_id: c.fortress_id(),
            cursor: ObservationCursor::ORIGIN,
            tick: GameTick(c.tick()),
            state_hash: c.witness(),
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
                fortress_id: Some(c.fortress_id()),
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
struct Guard;
impl WorkshopGuard for Guard {
    fn acknowledge(
        &mut self,
        _: crate::workshop_construction::journal::WorkshopPin,
        _: &OperationContext,
    ) -> Result<()> {
        Ok(())
    }
    fn check(
        &mut self,
        _: WorkshopStage,
        _: &WorkshopBinding,
        _: WorkshopSelection,
        _: Option<[u32; 3]>,
        _: &OperationContext,
    ) -> Result<()> {
        Ok(())
    }
}
struct Step {
    op: usize,
    observation: Option<Vec<u8>>,
    record: Option<Vec<u8>>,
    drop_reply: bool,
    generation: Option<u64>,
    extra: Vec<u8>,
}
impl Step {
    fn new(op: usize, name: Option<&str>) -> Result<Self> {
        Ok(Self {
            op,
            observation: if op == 1 {
                Some(vector("capture")?)
            } else {
                None
            },
            record: name.map(vector).transpose()?,
            drop_reply: false,
            generation: None,
            extra: vec![],
        })
    }
}
struct Peer {
    endpoint: SocketAddr,
    join: Option<JoinHandle<Result<Vec<usize>>>>,
}
impl Peer {
    fn finish(mut self) -> Result<Vec<usize>> {
        self.join
            .take()
            .ok_or_else(malformed)?
            .join()
            .map_err(|_| malformed())?
    }
}
impl Drop for Peer {
    fn drop(&mut self) {
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}
fn read(stream: &mut TcpStream) -> Result<(i16, Vec<u8>)> {
    let mut h = [0; 8];
    stream.read_exact(&mut h).map_err(io_error)?;
    let n = i32::from_le_bytes(h[4..8].try_into().map_err(|_| malformed())?);
    require((0..=2048).contains(&n), "test request too large")?;
    let mut b = vec![0; n as usize];
    stream.read_exact(&mut b).map_err(io_error)?;
    Ok((i16::from_le_bytes([h[0], h[1]]), b))
}
fn send(stream: &mut TcpStream, b: &[u8]) -> Result<()> {
    let mut h = vec![];
    h.extend_from_slice(&(-1i16).to_le_bytes());
    h.extend_from_slice(&[0; 2]);
    h.extend_from_slice(&(b.len() as i32).to_le_bytes());
    stream
        .write_all(&h)
        .and_then(|_| stream.write_all(b))
        .map_err(io_error)
}
fn response(nonce: &[u8], generation: u64, retained: u64) -> Vec<u8> {
    let mut b = vec![];
    number(&mut b, 1, 1);
    number(&mut b, 2, 0);
    bytes(&mut b, 3, nonce);
    number(&mut b, 4, 1);
    number(&mut b, 5, 20);
    number(&mut b, 6, generation);
    bytes(&mut b, 7, b"test-df");
    bytes(&mut b, 8, b"test-dfhack");
    number(&mut b, 12, 0);
    number(&mut b, 13, retained);
    b
}
fn validate(raw: &[u8], op: usize) -> Result<Vec<u8>> {
    let r = Message::parse(raw, 13)?;
    let mut fields = vec![1, 2, 3, 4];
    if matches!(op, 1 | 2) {
        fields.extend(5..=8);
    }
    if matches!(op, 2..=5) {
        fields.extend([9, 11]);
    }
    if op == 2 {
        fields.push(10);
    }
    if matches!(op, 3 | 5) {
        fields.push(12);
    }
    if op == 6 {
        fields.extend([5, 13]);
    }
    fields.sort_unstable();
    r.exact(&fields)?;
    require(
        r.bytes(1)? == [b't'; 32]
            && r.bytes(2)?.len() == 32
            && r.number(3)? == 1
            && r.number(4)? == 20,
        "test request identity",
    )?;
    let p = plan()?;
    if matches!(op, 1 | 2) {
        let [x, y, z] = p.before().selection().center();
        for (tag, n) in [(5, 42), (6, x), (7, y), (8, z)] {
            require(r.number(tag)? == u64::from(n), "test selection")?;
        }
    }
    if matches!(op, 2..=5) {
        require(
            r.bytes(9)? == p.key().as_bytes() && r.bytes(11)? == p.digest().as_bytes(),
            "test plan",
        )?;
    }
    if op == 2 {
        require(
            r.bytes(10)? == p.before().witness().as_bytes(),
            "test witness",
        )?;
    }
    if matches!(op, 3 | 5) {
        require(r.bytes(12)? == p.token(), "test token")?;
    }
    if op == 6 {
        require(
            r.number(5)? == 42 && r.number(13)? == 70,
            "test original building",
        )?;
    }
    Ok(r.bytes(2)?.to_vec())
}
fn peer(dialogues: Vec<Vec<Step>>) -> Result<Peer> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(io_error)?;
    let endpoint = listener.local_addr().map_err(io_error)?;
    listener.set_nonblocking(true).map_err(io_error)?;
    let generation = capture()?.generation();
    let join = thread::spawn(move || {
        let mut calls = vec![];
        for steps in dialogues {
            let end = Instant::now() + Duration::from_secs(5);
            let mut stream = loop {
                match listener.accept() {
                    Ok((s, _)) => break s,
                    Err(e)
                        if e.kind() == std::io::ErrorKind::WouldBlock && Instant::now() < end =>
                    {
                        thread::sleep(Duration::from_millis(2))
                    }
                    Err(e) => return Err(io_error(e)),
                }
            };
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .map_err(io_error)?;
            stream
                .set_write_timeout(Some(Duration::from_secs(5)))
                .map_err(io_error)?;
            let mut hello = [0; 12];
            stream.read_exact(&mut hello).map_err(io_error)?;
            require(&hello == b"DFHack?\n\x01\0\0\0", "test greeting")?;
            stream.write_all(b"DFHack!\n\x01\0\0\0").map_err(io_error)?;
            for (index, name) in METHODS.iter().enumerate() {
                let (id, b) = read(&mut stream)?;
                let r = Message::parse(&b, 4)?;
                r.exact(&[1, 2, 3, 4])?;
                require(
                    id == 0
                        && r.bytes(1)? == name.as_bytes()
                        && r.bytes(2)? == b"dfmcp.workshop.v1_20.Request"
                        && r.bytes(3)? == b"dfmcp.workshop.v1_20.Reply"
                        && r.bytes(4)? == b"dfmcp_workshop_v1_20",
                    "test method binding",
                )?;
                let mut b = vec![];
                number(&mut b, 1, index as u64 + 2);
                send(&mut stream, &b)?;
            }
            let (id, b) = read(&mut stream)?;
            require(id == 2, "test handshake")?;
            let nonce = validate(&b, 0)?;
            send(&mut stream, &response(&nonce, generation, 0))?;
            let mut retained = 0;
            for step in steps {
                let (id, b) = read(&mut stream)?;
                require(id == step.op as i16 + 2, "test operation")?;
                let nonce = validate(&b, step.op)?;
                calls.push(step.op);
                if step.drop_reply {
                    break;
                }
                retained = retained.max(u64::from(step.record.is_some()));
                let mut b = response(&nonce, step.generation.unwrap_or(generation), retained);
                if let Some(obs) = step.observation {
                    bytes(&mut b, 9, &obs);
                }
                if let Some(r) = step.record {
                    bytes(&mut b, 10, &r);
                }
                if step.op == 2 {
                    number(&mut b, 11, 0);
                }
                b.extend_from_slice(&step.extra);
                send(&mut stream, &b)?;
            }
        }
        Ok(calls)
    });
    Ok(Peer {
        endpoint,
        join: Some(join),
    })
}
fn connect(
    peer: &Peer,
    c: &OperationContext,
    permission: WorkshopPermission,
) -> Result<WorkshopRpc> {
    WorkshopRpc::connect(
        peer.endpoint,
        vec![b't'; 32],
        [b'n'; 32],
        capture()?.fortress().clone(),
        c,
        BuildCancellation::default(),
        permission,
    )
}
fn directory() -> Result<std::path::PathBuf> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let path = std::env::temp_dir().join(format!(
        "dfmcp-workshop-rpc-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir(&path).map_err(io_error)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))
            .map_err(io_error)?;
    }
    Ok(path)
}
fn preparation() -> Result<Vec<Step>> {
    Ok(vec![
        Step::new(1, None)?,
        Step::new(4, None)?,
        Step::new(2, Some("prepared"))?,
    ])
}
macro_rules! journal {
    ($peer:expr,$c:expr,$path:expr) => {
        open_private_workshop(
            &$path.join("workshop.journal"),
            $c,
            WorkshopMode::Control,
            Some(WorkshopBinding::new(
                $peer.endpoint,
                "test-df",
                "test-dfhack",
                &capture()?,
            )?),
            [9; 32],
            None,
        )?
    };
}

#[test]
fn actual_tcp_workshop_prepares_commits_and_reads_original_progress() -> Result<()> {
    let mut steps = preparation()?;
    steps.extend([Step::new(1, None)?, Step::new(3, Some("placed"))?]);
    let mut progress = Step::new(6, None)?;
    progress.observation = Some(vector("progress_complete")?);
    let peer = peer(vec![steps, vec![progress]])?;
    let c = context()?;
    let p = plan()?;
    let path = directory()?;
    let mut j = journal!(peer, &c, path);
    let mut rpc = connect(&peer, &c, Box::new(|_| Ok(())))?;
    j.prepare(&mut rpc, &p, &c, &mut Guard)?;
    let e = j.commit(&mut rpc, p.key(), p.digest(), &c, &mut Guard)?;
    assert!(e.placed() && !e.goal_complete());
    drop(rpc);
    let mut rpc = connect(&peer, &c, Box::new(|_| Ok(())))?;
    let e = j.sample(&mut rpc, p.key(), p.digest(), &c, &mut Guard)?;
    assert_eq!(e.goal(), Some(WorkshopGoal::Complete));
    drop(rpc);
    drop(j);
    let mut recovered = open_private_workshop(
        &path.join("workshop.journal"),
        &c,
        WorkshopMode::Offline,
        None,
        [0; 32],
        None,
    )?;
    assert_eq!(
        recovered.get(p.key(), p.digest(), &c)?.goal(),
        Some(WorkshopGoal::Complete)
    );
    assert_eq!(peer.finish()?, vec![1, 4, 2, 1, 3, 6]);
    Ok(())
}
#[test]
fn actual_tcp_lost_reply_reopens_without_second_writer() -> Result<()> {
    let mut steps = preparation()?;
    steps.push(Step::new(1, None)?);
    let mut lost = Step::new(3, None)?;
    lost.drop_reply = true;
    steps.push(lost);
    let peer = peer(vec![steps, vec![Step::new(4, Some("placed"))?]])?;
    let c = context()?;
    let p = plan()?;
    let path = directory()?;
    let mut j = journal!(peer, &c, path);
    let mut rpc = connect(&peer, &c, Box::new(|_| Ok(())))?;
    j.prepare(&mut rpc, &p, &c, &mut Guard)?;
    let acknowledged = j.pin();
    assert_eq!(
        j.commit(&mut rpc, p.key(), p.digest(), &c, &mut Guard)
            .err()
            .map(|e| e.code),
        Some(ErrorCode::EffectIndeterminate)
    );
    assert!(rpc.is_fenced());
    drop(rpc);
    drop(j);
    let mut j = open_private_workshop(
        &path.join("workshop.journal"),
        &c,
        WorkshopMode::Recover,
        None,
        [0; 32],
        Some(acknowledged),
    )?;
    assert!(!j.has_permit(p.key()));
    let mut rpc = connect(&peer, &c, Box::new(|_| Ok(())))?;
    let e = j.recover(&mut rpc, p.key(), p.digest(), false, &c, &mut Guard)?;
    assert!(e.placed() && !e.goal_complete());
    assert_eq!(peer.finish()?, vec![1, 4, 2, 1, 3, 4]);
    Ok(())
}
#[test]
fn actual_tcp_malformed_generation_and_reply_bound_are_refused() -> Result<()> {
    for kind in 0..3 {
        let mut step = Step::new(1, None)?;
        match kind {
            0 => step.generation = Some(capture()?.generation() + 1),
            1 => number(&mut step.extra, 14, 1),
            _ => step.observation = Some(vec![0; 8193]),
        }
        let peer = peer(vec![vec![step]])?;
        let c = context()?;
        let mut rpc = connect(&peer, &c, Box::new(|_| Ok(())))?;
        assert!(
            rpc.observe(capture()?.selection(), &c, Duration::from_secs(1))
                .is_err()
        );
        assert!(rpc.is_fenced());
        assert_eq!(peer.finish()?, vec![1]);
    }
    Ok(())
}
#[test]
fn actual_tcp_stale_capture_refuses_before_writer() -> Result<()> {
    let mut steps = preparation()?;
    let mut stale = Step::new(1, None)?;
    let mut b = vector("capture")?;
    b[23] ^= 1;
    stale.observation = Some(b);
    steps.push(stale);
    let peer = peer(vec![steps])?;
    let c = context()?;
    let p = plan()?;
    let path = directory()?;
    let mut j = journal!(peer, &c, path);
    let mut rpc = connect(&peer, &c, Box::new(|_| Ok(())))?;
    j.prepare(&mut rpc, &p, &c, &mut Guard)?;
    assert!(
        j.commit(&mut rpc, p.key(), p.digest(), &c, &mut Guard)
            .is_err()
    );
    assert!(!j.get(p.key(), p.digest(), &c)?.dispatch_started());
    assert_eq!(peer.finish()?, vec![1, 4, 2, 1]);
    Ok(())
}
#[test]
fn actual_tcp_duplicate_preparation_never_renews_permission() -> Result<()> {
    let peer = peer(vec![vec![Step::new(2, Some("prepared"))?]])?;
    let c = context()?;
    let p = plan()?;
    let mut rpc = connect(&peer, &c, Box::new(|_| Ok(())))?;
    assert!(!rpc.prepare(&p, &c, Duration::from_secs(1))?.replayed());
    assert!(rpc.prepare(&p, &c, Duration::from_secs(1)).is_err());
    assert!(rpc.is_fenced());
    assert_eq!(peer.finish()?, vec![2]);
    Ok(())
}
#[test]
fn actual_tcp_authority_revocation_fences_before_send() -> Result<()> {
    for deny in [false, true] {
        let peer = peer(vec![vec![]])?;
        let c = context()?;
        let revoked = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&revoked);
        let mut rpc = connect(
            &peer,
            &c,
            Box::new(move |_| {
                if flag.load(Ordering::Acquire) {
                    Err(denied())
                } else {
                    Ok(())
                }
            }),
        )?;
        let mut c = c;
        if deny {
            revoked.store(true, Ordering::Release);
        } else {
            c.grants.clear();
        }
        assert!(
            rpc.observe(capture()?.selection(), &c, Duration::from_secs(1))
                .is_err()
        );
        assert!(rpc.is_fenced());
        assert!(peer.finish()?.is_empty());
    }
    Ok(())
}

#[test]
fn actual_tcp_cross_connection_and_query_cleared_preparations_cannot_commit() -> Result<()> {
    for cross_connection in [false, true] {
        let mut first = preparation()?;
        let dialogues = if cross_connection {
            vec![first, vec![Step::new(1, None)?]]
        } else {
            first.extend([Step::new(4, Some("prepared"))?, Step::new(1, None)?]);
            vec![first]
        };
        let peer = peer(dialogues)?;
        let c = context()?;
        let p = plan()?;
        let path = directory()?;
        let mut j = journal!(peer, &c, path);
        let mut rpc = connect(&peer, &c, Box::new(|_| Ok(())))?;
        j.prepare(&mut rpc, &p, &c, &mut Guard)?;
        if cross_connection {
            drop(rpc);
            rpc = connect(&peer, &c, Box::new(|_| Ok(())))?;
        } else {
            assert!(rpc.query(&p, &c, Duration::from_secs(1))?.is_some());
        }
        assert_eq!(
            j.commit(&mut rpc, p.key(), p.digest(), &c, &mut Guard)
                .err()
                .map(|e| e.code),
            Some(ErrorCode::EffectIndeterminate)
        );
        assert!(rpc.is_fenced());
        let calls = peer.finish()?;
        assert!(!calls.contains(&3));
        assert_eq!(
            calls,
            if cross_connection {
                vec![1, 4, 2, 1]
            } else {
                vec![1, 4, 2, 4, 1]
            }
        );
    }
    Ok(())
}

#[test]
fn acknowledged_pin_rejects_valid_prefix_rollback_and_missing_pin() -> Result<()> {
    let mut steps = preparation()?;
    steps.extend([Step::new(1, None)?, Step::new(3, Some("placed"))?]);
    let peer = peer(vec![steps])?;
    let c = context()?;
    let p = plan()?;
    let path = directory()?;
    let mut j = journal!(peer, &c, path);
    let initial = j.pin();
    let prefix = std::fs::read(path.join("workshop.journal")).map_err(io_error)?;
    let mut rpc = connect(&peer, &c, Box::new(|_| Ok(())))?;
    j.prepare(&mut rpc, &p, &c, &mut Guard)?;
    let before_dispatch = j.pin();
    j.commit(&mut rpc, p.key(), p.digest(), &c, &mut Guard)?;
    let acknowledged = j.pin();
    drop(j);
    drop(rpc);
    assert!(
        acknowledged.frames > before_dispatch.frames && before_dispatch.frames > initial.frames
    );
    let rollback = path.join("rolled-back.journal");
    std::fs::write(&rollback, &prefix).map_err(io_error)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&rollback, std::fs::Permissions::from_mode(0o600))
            .map_err(io_error)?;
    }
    // The prefix is structurally valid, but predates an external acknowledgement.
    let untrusted =
        open_private_workshop(&rollback, &c, WorkshopMode::Offline, None, [0; 32], None)?;
    assert_eq!(untrusted.pin(), initial);
    drop(untrusted);
    for mode in [WorkshopMode::Control, WorkshopMode::Recover] {
        assert!(
            open_private_workshop(&rollback, &c, mode, None, [0; 32], Some(before_dispatch))
                .is_err()
        );
        assert!(
            open_private_workshop(
                &path.join("workshop.journal"),
                &c,
                mode,
                None,
                [0; 32],
                None
            )
            .is_err()
        );
    }
    // A checked descendant resumes the original evidence, never a dispatch permit.
    let mut resumed = open_private_workshop(
        &path.join("workshop.journal"),
        &c,
        WorkshopMode::Recover,
        None,
        [0; 32],
        Some(before_dispatch),
    )?;
    assert_eq!(resumed.pin(), acknowledged);
    assert!(!resumed.has_permit(p.key()));
    assert!(resumed.get(p.key(), p.digest(), &c)?.placed());
    assert_eq!(std::fs::read(&rollback).map_err(io_error)?, prefix);
    assert_eq!(peer.finish()?, vec![1, 4, 2, 1, 3]);
    Ok(())
}
