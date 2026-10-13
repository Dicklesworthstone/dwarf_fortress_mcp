//! Actual codec and TCP client; native replies are explicit fixtures, not a game.
//! Beads: df-dfhack-bridge-plane-c-pic.4/.5.
use super::*;
use crate::build_placement::tests::fixture;
use dfmcp_core::{
    CapabilityGrant, CapabilityScope, ObservationCursor, RequestId, SessionId, StateAnchor,
    WorkBudget,
};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;
use std::time::Instant;

fn record(name: &str) -> Result<BuildRecord> {
    BuildRecord::decode(&fixture(name)?)
}

#[test]
fn full_successor_matrix_preserves_ordinary_immutability() -> Result<()> {
    let values = ["prepared", "indeterminate", "placed", "expired", "cancelled"]
        .into_iter().map(record).collect::<Result<Vec<_>>>()?;
    let plan = values[0].plan();
    for old in &values {
        for next in &values {
            for generation in [41, 42] {
                for request in [Request::Prepare(plan), Request::Commit(plan),
                    Request::Query(plan), Request::Cancel(plan)] {
                    let ordinary = old.validate_successor(next).is_ok();
                    let recovery = matches!(request, Request::Query(_))
                        && generation == 41 && old.phase() == BuildPhase::Indeterminate
                        && next.phase() == BuildPhase::Placed;
                    assert_eq!(validate_query_successor(request, old, next, generation, false).is_ok(),
                        ordinary || recovery);
                    assert_eq!(validate_query_successor(request, old, next, generation, true).is_ok(), ordinary);
                }
            }
        }
    }
    Ok(())
}

#[test]
fn only_original_positive_query_can_explain_a_cleared_native_fence() -> Result<()> {
    let placed = record("placed")?;
    let plan = placed.plan();
    for request in [Request::Handshake, Request::Observe(plan.before().selection()),
        Request::Prepare(plan), Request::Commit(plan), Request::Query(plan), Request::Cancel(plan)] {
        for name in ["prepared", "indeterminate", "placed", "expired", "cancelled"] {
            let value = record(name)?;
            for generation in [0, 40, 41, 42, u64::MAX] {
                assert_eq!(original_query_proof(request, Some(&value), generation),
                    matches!(request, Request::Query(_)) && name == "placed" && generation == 41);
            }
        }
        assert!(!original_query_proof(request, None, 41));
    }
    let other = BuildPlan::new("another", plan.before().clone())?;
    assert!(!original_query_proof(Request::Query(&other), Some(&placed), 41));
    assert!(validate_query_successor(Request::Query(&other), &record("indeterminate")?,
        &placed, 41, false).is_err());
    Ok(())
}

fn context(plan: &BuildPlan) -> OperationContext {
    OperationContext {
        session_id: SessionId::new(11), request_id: RequestId::new(12),
        anchor: StateAnchor {
            fortress_id: plan.before().fortress_id(), tick: GameTick(plan.before().tick()),
            cursor: ObservationCursor::ORIGIN, state_hash: plan.before().witness(),
        },
        budget: WorkBudget { max_wall_millis: 5000, max_bytes: 32 * 1024 * 1024,
            ..WorkBudget::CONSERVATIVE_DEFAULT },
        grants: vec![CapabilityGrant { capability: Capability::Query,
            scope: CapabilityScope::default(), max_risk: RiskTier::ReadOnly,
            expires_at_tick: None, remaining_uses: None }],
        cancellation_requested: false,
    }
}

struct Step {
    method: usize,
    record: Option<&'static str>,
    unresolved: bool,
}
fn read_frame(stream: &mut TcpStream) -> Result<(i16, Vec<u8>)> {
    let mut header = [0; 8];
    stream.read_exact(&mut header).map_err(io_error)?;
    let n = i32::from_le_bytes(header[4..8].try_into().map_err(|_| malformed())?);
    require((0..=2048).contains(&n), "test frame bound")?;
    let mut raw = vec![0; n as usize];
    stream.read_exact(&mut raw).map_err(io_error)?;
    Ok((i16::from_le_bytes([header[0], header[1]]), raw))
}
fn send(stream: &mut TcpStream, raw: &[u8]) -> Result<()> {
    stream.write_all(&codec::header(-1, raw.len() as i32)).map_err(io_error)?;
    stream.write_all(raw).map_err(io_error)
}
fn response(nonce: &[u8], generation: u64, unresolved: bool) -> Vec<u8> {
    let mut out = Vec::new();
    for (tag, value) in [(1, 1), (2, 0), (4, 1), (5, 19), (6, generation),
        (12, u64::from(unresolved)), (13, 1)] {
        number(&mut out, tag, value);
    }
    bytes(&mut out, 3, nonce);
    bytes(&mut out, 7, b"53.01");
    bytes(&mut out, 8, b"53.01-r1");
    out
}

/// The peer is joined before propagating client failure. Accept and I/O are
/// bounded, including when a regression prevents the expected connection.
fn exchange<T>(steps: Vec<Step>, generation: u64,
    action: impl FnOnce(&mut BuildRpc, &BuildPlan, &OperationContext) -> Result<T>) -> Result<T> {
    let plan = record("prepared")?.plan().clone();
    let context = context(&plan);
    let listener = TcpListener::bind("127.0.0.1:0").map_err(io_error)?;
    let endpoint = listener.local_addr().map_err(io_error)?;
    listener.set_nonblocking(true).map_err(io_error)?;
    let expected = plan.clone();
    let peer = thread::spawn(move || -> Result<usize> {
        let start = Instant::now();
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(e) if e.kind() == io::ErrorKind::WouldBlock
                    && start.elapsed() < Duration::from_secs(5) => {
                    thread::sleep(Duration::from_millis(1));
                }
                Err(e) => return Err(io_error(e)),
            }
        };
        stream.set_read_timeout(Some(Duration::from_secs(5))).map_err(io_error)?;
        stream.set_write_timeout(Some(Duration::from_secs(5))).map_err(io_error)?;
        let mut greeting = [0; 12];
        stream.read_exact(&mut greeting).map_err(io_error)?;
        require(&greeting == b"DFHack?\n\x01\0\0\0", "test greeting")?;
        stream.write_all(b"DFHack!\n\x01\0\0\0").map_err(io_error)?;
        for (index, name) in METHODS.iter().enumerate() {
            let (id, raw) = read_frame(&mut stream)?;
            let message = Message::parse(&raw, 4)?;
            message.exact(&[1, 2, 3, 4])?;
            require(id == 0 && message.bytes(1)? == name.as_bytes()
                && message.bytes(2)? == b"dfmcp.build.v1_19.Request"
                && message.bytes(3)? == b"dfmcp.build.v1_19.Reply"
                && message.bytes(4)? == b"dfmcp_build_v1_19", "test method binding")?;
            let mut answer = Vec::new();
            number(&mut answer, 1, index as u64 + 2);
            send(&mut stream, &answer)?;
        }
        let (id, raw) = read_frame(&mut stream)?;
        let handshake = Message::parse(&raw, 4)?;
        handshake.exact(&[1, 2, 3, 4])?;
        require(id == 2 && handshake.bytes(1)? == [b't'; 32]
            && handshake.number(3)? == 1 && handshake.number(4)? == 19, "test handshake")?;
        let nonce = handshake.bytes(2)?.to_vec();
        send(&mut stream, &response(&nonce, generation, true))?;
        let count = steps.len();
        for step in steps {
            let (id, raw) = read_frame(&mut stream)?;
            let message = Message::parse(&raw, 13)?;
            let mut fields = vec![1, 2, 3, 4, 10, 12];
            if step.method == 5 { fields.push(13); }
            message.exact(&fields)?;
            require(id == step.method as i16 + 2 && matches!(step.method, 4 | 5)
                && message.bytes(1)? == [b't'; 32] && message.bytes(2)? == nonce
                && message.number(3)? == 1 && message.number(4)? == 19
                && message.bytes(10)? == expected.key().as_bytes()
                && message.bytes(12)? == expected.digest().as_bytes(), "test original query")?;
            if step.method == 5 {
                require(message.bytes(13)? == expected.token(), "test cancel token")?;
            }
            let mut answer = response(&nonce, generation, step.unresolved);
            if let Some(name) = step.record { bytes(&mut answer, 10, &fixture(name)?); }
            send(&mut stream, &answer)?;
        }
        Ok(count)
    });
    let result = (|| {
        let original = BuildBinding::new(endpoint, "53.01", "53.01-r1", plan.before())?;
        let mut source = BuildRpc::connect_trusted_recovery(&original, vec![b't'; 32],
            [b'n'; 32], plan.before().selection(), &context, BuildCancellation::default(),
            Box::new(|placing| require(!placing, "test has no placement authority")))?;
        let out = action(&mut source, &plan, &context);
        source.fence();
        out
    })();
    let joined = peer.join().map_err(|_| malformed())?;
    joined?;
    result
}

#[test]
fn real_query_recovers_after_uncertain_handshake_and_on_one_connection() -> Result<()> {
    for seen_first in [false, true] {
        let mut steps = Vec::new();
        if seen_first { steps.push(Step { method: 4, record: Some("indeterminate"), unresolved: true }); }
        steps.push(Step { method: 4, record: Some("placed"), unresolved: false });
        steps.push(Step { method: 4, record: Some("placed"), unresolved: false });
        let out = exchange(steps, 41, |source, plan, c| {
            if seen_first { source.query(plan, c, Duration::from_secs(5))?; }
            let value = source.query(plan, c, Duration::from_secs(5))?.ok_or_else(malformed)?;
            let duplicate = source.query(plan, c, Duration::from_secs(5))?.ok_or_else(malformed)?;
            Ok((value, duplicate, source.native_unresolved(), source.control,
                source.prepared.is_some(), source.prepare(plan, c, Duration::from_secs(5)).is_err()))
        })?;
        assert_eq!(out.0, record("placed")?);
        assert_eq!(out.0, out.1);
        assert_eq!((out.2, out.3, out.4, out.5), (false, false, false, true));
    }
    Ok(())
}

#[test]
fn flag_only_or_cross_generation_resolution_fails_with_original_uncertainty() -> Result<()> {
    for (name, generation) in [(None, 41), (Some("prepared"), 41), (Some("expired"), 41),
        (Some("cancelled"), 41), (Some("indeterminate"), 41), (Some("placed"), 42)] {
        let out = exchange(vec![Step { method: 4, record: name, unresolved: false }], generation,
            |source, plan, c| {
                let rejected = source.query(plan, c, Duration::from_secs(5)).is_err();
                Ok((rejected, source.is_fenced(), source.native_unresolved()))
            })?;
        assert_eq!(out, (true, true, true));
    }
    Ok(())
}

#[test]
fn invalid_successor_does_not_publish_a_false_cleared_summary() -> Result<()> {
    let out = exchange(vec![
        Step { method: 4, record: Some("cancelled"), unresolved: true },
        Step { method: 4, record: Some("placed"), unresolved: false },
    ], 41, |source, plan, c| {
        let old = source.query(plan, c, Duration::from_secs(5))?.ok_or_else(malformed)?;
        let rejected = source.query(plan, c, Duration::from_secs(5)).is_err();
        Ok((old.clone(), source.seen.get(plan.key()).cloned(), rejected,
            source.is_fenced(), source.native_unresolved()))
    })?;
    assert_eq!(out.0.phase(), BuildPhase::Cancelled);
    assert_eq!(out.1, Some(out.0));
    assert_eq!((out.2, out.3, out.4), (true, true, true));
    Ok(())
}

#[test]
fn cancellation_cannot_be_used_as_a_reconciliation_rpc() -> Result<()> {
    let out = exchange(vec![
        Step { method: 4, record: Some("indeterminate"), unresolved: true },
        Step { method: 5, record: Some("placed"), unresolved: false },
    ], 41, |source, plan, c| {
        source.query(plan, c, Duration::from_secs(5))?;
        let rejected = source.cancel(plan, c, Duration::from_secs(5)).is_err();
        Ok((rejected, source.is_fenced(), source.native_unresolved()))
    })?;
    assert_eq!(out, (true, true, true));
    Ok(())
}
