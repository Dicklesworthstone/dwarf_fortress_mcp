//! Production framing with joined loopback peers and real private custody.
//! Canonical native vectors are SDK doubles, never live-game evidence.
use super::*;
use crate::manager_appointment::{
    journal::journal_tests::{Directory, binding, context},
    journal::{self, Mode, Original},
    tests::{native_capture as capture, native_plan as plan, vector},
};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread::{self, JoinHandle};
use std::time::Instant;
fn malformed() -> dfmcp_core::DfmcpError {
    error(ErrorCode::AdapterRejected, "manager TCP test peer mismatch")
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
    require(h[2..4] == [0, 0], "manager peer header padding")?;
    let n = i32::from_le_bytes(h[4..8].try_into().map_err(|_| malformed())?);
    require((0..=2048).contains(&n), "manager peer request bound")?;
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
fn expected(op: usize, selected: Option<&Plan>, wall: u64) -> Result<Vec<u8>> {
    let p = selected.cloned().map_or_else(plan, Ok)?;
    let mut raw = vec![];
    bytes(&mut raw, 1, &[b't'; 32]);
    bytes(&mut raw, 2, &[b'n'; 32]);
    number(&mut raw, 3, 1);
    number(&mut raw, 4, 22);
    number(&mut raw, 5, wall);
    if matches!(op, 1 | 2) {
        number(&mut raw, 6, u64::from(p.before.selection.unit));
        number(&mut raw, 7, u64::from(p.before.selection.figure));
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
    Ok(raw)
}
fn response(generation: u64, retained: u64) -> Vec<u8> {
    let mut raw = vec![];
    number(&mut raw, 1, 1);
    number(&mut raw, 2, 0);
    bytes(&mut raw, 3, &[b'n'; 32]);
    number(&mut raw, 4, 1);
    number(&mut raw, 5, 22);
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
            require(&hello == b"DFHack?\n\x01\0\0\0", "manager greeting")?;
            stream.write_all(b"DFHack!\n\x01\0\0\0").map_err(io_error)?;
            for (index, name) in METHODS.iter().enumerate() {
                let (id, raw) = receive(&mut stream)?;
                let r = Message::parse(&raw, 4)?;
                r.exact(&[1, 2, 3, 4])?;
                require(
                    id == 0
                        && r.bytes(1)? == name.as_bytes()
                        && r.bytes(2)? == b"dfmcp.manager.v1_22.Request"
                        && r.bytes(3)? == b"dfmcp.manager.v1_22.Reply"
                        && r.bytes(4)? == b"dfmcp_manager_v1_22",
                    "manager method binding",
                )?;
                let mut raw = vec![];
                number(&mut raw, 1, index as u64 + 2);
                send(&mut stream, &raw)?;
            }
            let (id, raw) = receive(&mut stream)?;
            require(
                id == 2 && raw == expected(0, None, 5000)?,
                "manager handshake",
            )?;
            send(&mut stream, &response(generation, 0))?;
            let mut retained = 0;
            for step in steps {
                let (id, raw) = receive(&mut stream)?;
                require(
                    id == step.op as i16 + 2
                        && raw
                            == expected(
                                step.op,
                                step.plan.as_ref(),
                                Message::parse(&raw, 12)?.number(5)?,
                            )?
                        && (1..=5000).contains(&Message::parse(&raw, 12)?.number(5)?),
                    "manager exact request",
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
                "unexpected manager frame after bounded dialogue",
            )?;
        }
        Ok(calls)
    });
    Ok(Peer {
        endpoint,
        join: Some(join),
    })
}
fn connect(peer: &Peer, c: &OperationContext, generation: u64) -> Result<ManagerRpc> {
    let mut b = binding(peer.endpoint)?;
    b.generation = generation;
    ManagerRpc::connect(
        b,
        vec![b't'; 32],
        [b'n'; 32],
        c,
        BuildCancellation::default(),
        Box::new(|_| Ok(())),
    )
}

#[test]
fn exact_manager_rpc_and_one_attempt_commit_then_query() -> Result<()> {
    let cap = capture()?;
    let p = plan()?;
    let peer = start_peer(
        vec![vec![
            Step::observed(1, vector("capture")?),
            Step::new(2, Some("prepared"))?,
            Step::new(3, Some("assigned"))?,
            Step::new(4, Some("assigned"))?,
        ]],
        cap.generation,
    )?;
    let c = context()?;
    let mut rpc = connect(&peer, &c, cap.generation)?;
    assert_eq!(rpc.observe(cap.selection, &c)?, cap);
    let d = Directory::new()?;
    let b = binding(peer.endpoint)?;
    let (mut j, mut ack) = journal::open_private(
        &d.0,
        &c,
        Mode::Control,
        &b,
        cap.selection,
        true,
        None,
        [9; 32],
    )?;
    let o = Original::new(p.clone(), cap.tick + 12000)?;
    j.intent(o.clone(), &c, &mut ack)?;
    j.prepared(rpc.prepare(&p, &c)?, &c, &mut ack)?;
    let assigned = j.dispatch(o.seal(), &c, &mut ack, &mut |_| Ok(()), |permit, c| {
        rpc.commit(permit, c)
    })?;
    assert!(assigned.complete());
    assert_eq!(rpc.query(&p, &c)?, Some(assigned));
    drop(rpc);
    assert_eq!(peer.finish()?, [1, 2, 3, 4]);
    Ok(())
}
#[test]
fn unauthorized_candidate_scope_and_expiry_send_no_prepare() -> Result<()> {
    let cap = capture()?;
    let p = plan()?;
    for which in 0..4 {
        let peer = start_peer(vec![vec![]], cap.generation)?;
        let c = context()?;
        let mut rpc = connect(&peer, &c, cap.generation)?;
        let mut bad = c.clone();
        if which == 0 {
            bad.grants
                .retain(|g| g.capability != Capability::ConfigureProduction);
        }
        if which == 1 {
            for g in &mut bad.grants {
                g.expires_at_tick = Some(dfmcp_core::GameTick(cap.tick - 1));
            }
        }
        if which == 2 {
            bad.budget.max_wall_millis = 0;
        }
        if which == 3 {
            for grant in &mut bad.grants {
                if matches!(
                    grant.capability,
                    Capability::ConfigureProduction | Capability::Plan
                ) {
                    grant.scope.map_area = Some(MapCuboid::new(
                        MapCoord::new(0, 0, 0),
                        MapCoord::new(1, 1, 1),
                    )?);
                }
            }
        }
        assert!(rpc.prepare(&p, &bad).is_err());
        drop(rpc);
        assert!(peer.finish()?.is_empty());
    }
    Ok(())
}
#[test]
fn mismatched_payload_source_and_failed_connection_never_prepare() -> Result<()> {
    let cap = capture()?;
    let peer = start_peer(
        vec![vec![Step::observed(1, vector("capture")?)]],
        cap.generation + 1,
    )?;
    let c = context()?;
    let mut rpc = connect(&peer, &c, cap.generation + 1)?;
    assert!(rpc.observe(cap.selection, &c).is_err());
    assert!(rpc.prepare(&plan()?, &c).is_err());
    drop(rpc);
    assert_eq!(peer.finish()?, [1]);
    Ok(())
}

#[test]
fn manager_binding_requires_full_fortress_effect_scope_before_network() -> Result<()> {
    let mut b = binding("127.0.0.1:5000".parse().map_err(|_| malformed())?)?;
    assert_eq!(b.effect_scope()?, b.read_scope()?);
    b.scope = MapCuboid::new(MapCoord::new(0, 0, 0), MapCoord::new(1, 1, 1))?;
    assert!(b.validate().is_err());
    assert!(b.effect_scope().is_err());
    assert!(
        ManagerRpc::connect(
            b,
            vec![b't'; 32],
            [b'n'; 32],
            &context()?,
            BuildCancellation::default(),
            Box::new(|_| Ok(()))
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn manager_prepare_requires_authority_at_capture_tick_without_prior_observe() -> Result<()> {
    let cap = capture()?;
    let p = plan()?;
    for allowed in [false, true] {
        let steps = if allowed {
            vec![Step::new(2, Some("prepared"))?]
        } else {
            vec![]
        };
        let peer = start_peer(vec![steps], cap.generation)?;
        let mut old = context()?;
        old.anchor.tick = dfmcp_core::GameTick(cap.tick - 2);
        for grant in &mut old.grants {
            if matches!(
                grant.capability,
                Capability::Plan | Capability::ConfigureProduction
            ) {
                grant.expires_at_tick = Some(dfmcp_core::GameTick(if allowed {
                    cap.tick + 1
                } else {
                    cap.tick - 1
                }));
            }
        }
        let mut rpc = connect(&peer, &old, cap.generation)?;
        // A public adapter caller may already possess the exact capture. No
        // observe on this connection is needed to exercise the grant boundary.
        let result = rpc.prepare(&p, &old);
        if allowed {
            assert_eq!(result?.phase, 0);
        } else {
            assert_eq!(
                result.map(|_| ()).err().map(|e| e.code),
                Some(ErrorCode::CapabilityDenied)
            );
        }
        drop(rpc);
        assert_eq!(peer.finish()?, if allowed { vec![2] } else { vec![] });
    }
    Ok(())
}

#[test]
fn manager_plan_recovery_edges_preserve_observed_tick_high_water() -> Result<()> {
    let cap = capture()?;
    let p = plan()?;
    // A later paused observation is an explicit protocol double, not a native
    // or live capture. Original-key lookup/retirement remains legitimate.
    let mut later = cap.clone();
    later.tick += 2;
    later.year_tick += 2;
    later.raw = later.encode();
    let later = Capture::decode(later.bytes())?;
    for op in [4, 5] {
        for allowed in [false, true] {
            let mut steps = vec![
                Step::new(2, Some("prepared"))?,
                Step::observed(1, later.bytes().to_vec()),
            ];
            if allowed {
                let record = if op == 4 {
                    vector("prepared")?
                } else {
                    crate::manager_appointment::tests::record(&p, 4, 4, None, false)
                };
                steps.push(Step {
                    op,
                    observation: None,
                    record: Some(record),
                    plan: None,
                });
            }
            let peer = start_peer(vec![steps], cap.generation)?;
            let c = context()?;
            let mut rpc = connect(&peer, &c, cap.generation)?;
            assert_eq!(rpc.prepare(&p, &c)?.phase, 0);
            assert_eq!(rpc.observe(cap.selection, &c)?, later);
            let mut old = c.clone();
            old.anchor.tick = dfmcp_core::GameTick(cap.tick - 2);
            for grant in &mut old.grants {
                if matches!(grant.capability, Capability::Query | Capability::Observe) {
                    grant.expires_at_tick = Some(dfmcp_core::GameTick(if allowed {
                        later.tick + 1
                    } else {
                        later.tick - 1
                    }));
                }
            }
            let result = if op == 4 {
                rpc.query(&p, &old).map(|r| r.map(|r| r.phase))
            } else {
                rpc.cancel(&p, &old).map(|r| Some(r.phase))
            };
            if allowed {
                assert_eq!(result?, Some(if op == 4 { 0 } else { 4 }));
            } else {
                assert_eq!(
                    result.err().map(|e| e.code),
                    Some(ErrorCode::CapabilityDenied)
                );
            }
            drop(rpc);
            assert_eq!(
                peer.finish()?,
                if allowed { vec![2, 1, op] } else { vec![2, 1] }
            );
        }
    }
    Ok(())
}

#[test]
fn manager_dispatch_guard_and_writer_use_retained_tick_for_actual_lease() -> Result<()> {
    let cap = capture()?;
    let p = plan()?;
    for allowed in [false, true] {
        let mut steps = vec![Step::new(2, Some("prepared"))?];
        if allowed {
            steps.push(Step::new(3, Some("assigned"))?);
        }
        let peer = start_peer(vec![steps], cap.generation)?;
        let c = context()?;
        let mut rpc = connect(&peer, &c, cap.generation)?;
        let d = Directory::new()?;
        let b = binding(peer.endpoint)?;
        let (mut j, mut ack) = journal::open_private(
            &d.0,
            &c,
            Mode::Control,
            &b,
            cap.selection,
            true,
            None,
            [9; 32],
        )?;
        let original = Original::new(p.clone(), cap.tick + 12000)?;
        j.intent(original.clone(), &c, &mut ack)?;
        j.prepared(rpc.prepare(&p, &c)?, &c, &mut ack)?;
        let mut old = c.clone();
        old.anchor.tick = dfmcp_core::GameTick(cap.tick - 2);
        let mut leases = dfmcp_core::LeaseManager::new();
        let area = b.effect_scope()?;
        let lease = leases.acquire_spatial_lease(
            c.session_id,
            area,
            true,
            old.anchor.tick,
            if allowed { 3 } else { 1 },
        )?;
        let mut guards = 0;
        let mut writers = 0;
        let mut guard = |edge: &OperationContext| {
            guards += 1;
            assert_eq!(edge.anchor.tick, dfmcp_core::GameTick(cap.tick));
            assert!(edge.budget.max_wall_millis <= old.budget.max_wall_millis);
            leases.verify_exclusive_spatial(lease, c.session_id, area, edge.anchor.tick)
        };
        let result = j.dispatch(
            original.seal(),
            &old,
            &mut ack,
            &mut guard,
            |permit, edge| {
                writers += 1;
                assert_eq!(edge.anchor.tick, dfmcp_core::GameTick(cap.tick));
                assert!(edge.budget.max_wall_millis <= old.budget.max_wall_millis);
                rpc.commit(permit, edge)
            },
        );
        if allowed {
            assert!(result?.complete());
            assert_eq!((guards, writers), (2, 1));
            assert!(j.dispatch_started);
        } else {
            assert_eq!(result.err().map(|e| e.code), Some(ErrorCode::LeaseDenied));
            assert_eq!((guards, writers), (1, 0));
            assert!(!j.dispatch_started);
        }
        assert!(!j.dispatchable());
        drop(rpc);
        assert_eq!(peer.finish()?, if allowed { vec![2, 3] } else { vec![2] });
    }
    Ok(())
}
