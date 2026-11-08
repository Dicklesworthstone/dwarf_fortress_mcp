//! Production framing with joined loopback peers and real private custody.
//! Canonical native vectors are SDK doubles, never live-game evidence.
use super::*;
use crate::tree_harvest::{
    journal::journal_tests::{Directory, binding, context},
    journal::{self, Mode, Original},
    tests::{capture, plan, vector},
};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread::{self, JoinHandle};
use std::time::Instant;
fn malformed() -> dfmcp_core::DfmcpError {
    error(ErrorCode::AdapterRejected, "tree TCP test peer mismatch")
}
fn io_error(_: std::io::Error) -> dfmcp_core::DfmcpError {
    malformed()
}
struct Step {
    op: usize,
    observation: Option<Vec<u8>>,
    record: Option<Vec<u8>>,
    plan: Option<Plan>,
}
impl Step {
    fn new(op: usize, name: Option<&str>) -> Result<Self> {
        Ok(Self {
            op,
            observation: None,
            record: name.map(vector).transpose()?,
            plan: None,
        })
    }
    fn observed(op: usize, raw: Vec<u8>) -> Self {
        Self {
            op,
            observation: Some(raw),
            record: None,
            plan: None,
        }
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
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}
fn receive(s: &mut TcpStream) -> Result<(i16, Vec<u8>)> {
    let mut h = [0; 8];
    s.read_exact(&mut h).map_err(io_error)?;
    require(h[2..4] == [0, 0], "tree peer header padding")?;
    let n = i32::from_le_bytes(h[4..8].try_into().map_err(|_| malformed())?);
    require((0..=2048).contains(&n), "tree peer request bound")?;
    let mut raw = vec![0; n as usize];
    s.read_exact(&mut raw).map_err(io_error)?;
    Ok((i16::from_le_bytes([h[0], h[1]]), raw))
}
fn send(s: &mut TcpStream, raw: &[u8]) -> Result<()> {
    let mut h = vec![];
    h.extend_from_slice(&(-1i16).to_le_bytes());
    h.extend([0, 0]);
    h.extend_from_slice(&(raw.len() as i32).to_le_bytes());
    s.write_all(&h)
        .and_then(|_| s.write_all(raw))
        .map_err(io_error)
}
fn expected(op: usize, selected: Option<&Plan>) -> Result<Vec<u8>> {
    let p = selected.cloned().map_or_else(plan, Ok)?;
    let mut raw = vec![];
    bytes(&mut raw, 1, &[b't'; 32]);
    bytes(&mut raw, 2, &[b'n'; 32]);
    number(&mut raw, 3, 1);
    number(&mut raw, 4, 21);
    if matches!(op, 1 | 2 | 6) {
        for (tag, n) in [6, 7, 8].into_iter().zip(p.before.root) {
            number(&mut raw, tag, u64::from(n));
        }
    }
    if matches!(op, 2..=5) {
        bytes(&mut raw, 9, p.key().as_bytes());
        bytes(&mut raw, 11, p.digest.as_bytes());
    }
    if op == 2 {
        bytes(&mut raw, 10, p.before.witness().as_bytes());
    }
    if matches!(op, 3 | 5) {
        bytes(&mut raw, 12, &p.token);
    }
    if op == 6 {
        number(&mut raw, 13, u64::from(p.before.next_item));
        number(&mut raw, 14, p.before.species as i64 as u64);
    }
    Ok(raw)
}
fn response(generation: u64, retained: u64) -> Vec<u8> {
    let mut raw = vec![];
    number(&mut raw, 1, 1);
    number(&mut raw, 2, 0);
    bytes(&mut raw, 3, &[b'n'; 32]);
    number(&mut raw, 4, 1);
    number(&mut raw, 5, 21);
    number(&mut raw, 6, generation);
    bytes(&mut raw, 7, b"test-df");
    bytes(&mut raw, 8, b"test-dfhack");
    number(&mut raw, 12, 0);
    number(&mut raw, 13, retained);
    raw
}
fn start_peer(dialogues: Vec<Vec<Step>>, generation: u64) -> Result<Peer> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(io_error)?;
    let endpoint = listener.local_addr().map_err(io_error)?;
    listener.set_nonblocking(true).map_err(io_error)?;
    let join = thread::spawn(move || {
        let mut calls = vec![];
        for steps in dialogues {
            let deadline = Instant::now() + Duration::from_secs(5);
            let mut stream = loop {
                match listener.accept() {
                    Ok((s, _)) => break s,
                    Err(e)
                        if e.kind() == std::io::ErrorKind::WouldBlock
                            && Instant::now() < deadline =>
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
            require(&hello == b"DFHack?\n\x01\0\0\0", "tree greeting")?;
            stream.write_all(b"DFHack!\n\x01\0\0\0").map_err(io_error)?;
            for (index, name) in METHODS.iter().enumerate() {
                let (id, raw) = receive(&mut stream)?;
                let r = Message::parse(&raw, 4)?;
                r.exact(&[1, 2, 3, 4])?;
                require(
                    id == 0
                        && r.bytes(1)? == name.as_bytes()
                        && r.bytes(2)? == b"dfmcp.tree_harvest.v1_21.Request"
                        && r.bytes(3)? == b"dfmcp.tree_harvest.v1_21.Reply"
                        && r.bytes(4)? == b"dfmcp_tree_harvest_v1_21",
                    "tree method binding",
                )?;
                let mut raw = vec![];
                number(&mut raw, 1, index as u64 + 2);
                send(&mut stream, &raw)?;
            }
            let (id, raw) = receive(&mut stream)?;
            require(id == 2 && raw == expected(0, None)?, "tree handshake")?;
            send(&mut stream, &response(generation, 0))?;
            let mut retained = 0;
            for step in steps {
                let (id, raw) = receive(&mut stream)?;
                require(
                    id == step.op as i16 + 2 && raw == expected(step.op, step.plan.as_ref())?,
                    "tree exact request",
                )?;
                calls.push(step.op);
                retained = retained.max(u64::from(step.record.is_some()));
                let mut raw = response(generation, retained);
                if let Some(obs) = step.observation {
                    bytes(&mut raw, 9, &obs);
                }
                if let Some(record) = step.record {
                    bytes(&mut raw, 10, &record);
                }
                if step.op == 2 {
                    number(&mut raw, 11, 0);
                }
                send(&mut stream, &raw)?;
            }
            // Negative cases must close without even one unexpected commit frame.
            let mut extra = [0; 8];
            require(
                stream.read(&mut extra).map_err(io_error)? == 0,
                "unexpected tree frame after bounded dialogue",
            )?;
        }
        Ok(calls)
    });
    Ok(Peer {
        endpoint,
        join: Some(join),
    })
}
fn connect(peer: &Peer, c: &OperationContext, generation: u64) -> Result<TreeRpc> {
    let mut b = binding(peer.endpoint)?;
    b.generation = generation;
    TreeRpc::connect(
        b,
        vec![b't'; 32],
        [b'n'; 32],
        c,
        BuildCancellation::default(),
        Box::new(|_| Ok(())),
    )
}
#[test]
fn tree_actual_tcp_exact_scope_and_returned_tick_expiry() -> Result<()> {
    let cap = capture()?;
    let p = plan()?;
    let peer = start_peer(
        vec![vec![
            Step::observed(1, vector("capture")?),
            Step::new(2, Some("prepared"))?,
        ]],
        cap.generation,
    )?;
    let c = context()?;
    let mut rpc = connect(&peer, &c, cap.generation)?;
    assert_eq!(rpc.observe(cap.root, &c)?, cap);
    assert_eq!(rpc.prepare(&p, &c)?.phase, 0);
    drop(rpc);
    assert_eq!(peer.finish()?, [1, 2]);
    let peer = start_peer(vec![vec![]], cap.generation)?;
    let mut rpc = connect(&peer, &c, cap.generation)?;
    let mut outside = c.clone();
    for grant in &mut outside.grants {
        if matches!(grant.capability, Capability::Plan | Capability::Designate) {
            grant.scope.map_area = Some(MapCuboid::new(
                MapCoord::new(0, 0, 0),
                MapCoord::new(1, 1, 1),
            )?);
        }
    }
    assert!(rpc.prepare(&p, &outside).is_err());
    drop(rpc);
    assert!(peer.finish()?.is_empty());
    for progress in [false, true] {
        let mut observed = if progress {
            vector("progress_supply")?
        } else {
            vector("capture")?
        };
        observed[24..32].copy_from_slice(&(cap.tick + 1).to_be_bytes());
        let peer = start_peer(
            vec![vec![Step::observed(if progress { 6 } else { 1 }, observed)]],
            cap.generation,
        )?;
        let mut expiry = c.clone();
        for grant in &mut expiry.grants {
            grant.expires_at_tick = Some(dfmcp_core::GameTick(cap.tick));
        }
        let mut rpc = connect(&peer, &expiry, cap.generation)?;
        let e = if progress {
            rpc.progress(&p, &expiry).map(|_| ())
        } else {
            rpc.observe(cap.root, &expiry).map(|_| ())
        };
        assert!(e.is_err());
        if progress {
            assert_eq!(
                rpc.consumed_progress.as_ref().map(|p| p.tick),
                Some(cap.tick + 1)
            );
        }
        drop(rpc);
        assert_eq!(peer.finish()?, [if progress { 6 } else { 1 }]);
    }
    Ok(())
}
#[test]
fn tree_actual_tcp_payload_generation_cannot_borrow_envelope_identity() -> Result<()> {
    let cap = capture()?;
    let peer = start_peer(
        vec![vec![Step::observed(6, vector("progress_supply")?)]],
        cap.generation + 1,
    )?;
    let c = context()?;
    let mut rpc = connect(&peer, &c, cap.generation + 1)?;
    assert!(rpc.progress(&plan()?, &c).is_err());
    assert!(rpc.consumed_progress.is_none());
    drop(rpc);
    assert_eq!(peer.finish()?, [6]);
    Ok(())
}
#[test]
fn tree_actual_tcp_same_connection_permit_and_cleared_or_cross_connection_refusal() -> Result<()> {
    let cap = capture()?;
    for variant in 0..4 {
        let p = if variant == 3 {
            Plan::new("different-native-key", cap.clone())?
        } else {
            plan()?
        };
        let mut prep = Step::new(2, Some("prepared"))?;
        if variant == 3 {
            // A real first connection prepares B; a second connection prepares A.
            // The durable B dispatch may not borrow A's transport preparation.
            let old = vector("prepared")?;
            let old_len = u16::from_be_bytes([old[8], old[9]]) as usize;
            let mut raw = old[..8].to_vec();
            field(&mut raw, p.key().as_bytes());
            raw.extend_from_slice(&old[10 + old_len..old.len() - 32]);
            let at = 10 + p.key().len();
            raw[at..at + 32].copy_from_slice(p.digest.as_bytes());
            raw[at + 32..at + 48].copy_from_slice(&p.token);
            let digest = hash(b"dfmcp-tree-harvest-receipt/1", &raw);
            raw.extend_from_slice(digest.as_bytes());
            prep.record = Some(raw);
            prep.plan = Some(p.clone());
        }
        let steps = vec![prep];
        let dialogues = match variant {
            0 => vec![
                steps
                    .into_iter()
                    .chain([Step::new(3, Some("designated"))?])
                    .collect(),
            ],
            1 => vec![
                steps
                    .into_iter()
                    .chain([Step::new(4, Some("prepared"))?])
                    .collect(),
            ],
            2 => vec![steps, vec![]],
            _ => vec![steps, vec![Step::new(2, Some("prepared"))?]],
        };
        let peer = start_peer(dialogues, cap.generation)?;
        let c = context()?;
        let mut rpc = connect(&peer, &c, cap.generation)?;
        let directory = Directory::new()?;
        let b = binding(peer.endpoint)?;
        let (mut j, mut ack) = journal::open_private(
            &directory.0,
            &c,
            Mode::Control,
            &b,
            cap.root,
            true,
            None,
            [9; 32],
        )?;
        let original = Original::new(p.clone(), cap.tick + 12000)?;
        j.intent(original.clone(), &c, &mut ack)?;
        let prepared = rpc.prepare(&p, &c)?;
        j.prepared(prepared, &c, &mut ack)?;
        if variant == 1 {
            assert!(rpc.query(&p, &c)?.is_some());
        } else if variant >= 2 {
            drop(rpc);
            rpc = connect(&peer, &c, cap.generation)?;
            if variant == 3 {
                assert_eq!(rpc.prepare(&plan()?, &c)?.phase, 0);
            }
        }
        let result = j.dispatch(
            original.seal(),
            &c,
            &mut ack,
            &mut |_| Ok(()),
            |permit, current| rpc.commit(permit, current),
        );
        assert_eq!(result.is_ok(), variant == 0);
        drop(rpc);
        let calls = peer.finish()?;
        assert_eq!(
            calls.iter().filter(|&&op| op == 3).count(),
            usize::from(variant == 0)
        );
        assert!(!j.dispatchable());
    }
    Ok(())
}

#[test]
fn tree_actual_tcp_consumed_clock_floor_rejects_stale_supply_and_preserves_the_capture() -> Result<()> {
    let cap = capture()?;
    let c = context()?;
    for current in [false, true] {
        let tick = cap.tick + if current { 10 } else { 0 };
        let mut raw = vector("progress_supply")?;
        raw[24..32].copy_from_slice(&tick.to_be_bytes());
        let peer = start_peer(vec![vec![Step::observed(6, raw)]], cap.generation)?;
        let mut rpc = connect(&peer, &c, cap.generation)?;
        rpc.require_progress_at_or_after(cap.tick + 10);
        rpc.require_progress_at_or_after(cap.tick);
        let result = rpc.progress(&plan()?, &c);
        assert_eq!(result.is_ok(), current);
        assert_eq!(rpc.consumed_progress.as_ref().map(|p| p.tick), Some(tick));
        if !current {
            assert_eq!(result.err().map(|e| e.code), Some(ErrorCode::StaleAnchor));
            // A lower later context cannot revive the consumed connection.
            assert!(rpc.observe(cap.root, &c).is_err());
        }
        drop(rpc);
        assert_eq!(peer.finish()?, [6]);
    }
    Ok(())
}
