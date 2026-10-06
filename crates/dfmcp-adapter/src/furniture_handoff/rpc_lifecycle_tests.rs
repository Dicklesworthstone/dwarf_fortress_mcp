//! Native custody ends at verified release; request custody does not.
//! Beads: df-dfhack-bridge-plane-c-pic.3 / df-dfhack-bridge-plane-c-pic.5.
use super::*;
use dfmcp_core::{
    CapabilityGrant, CapabilityScope, Digest32, ObservationCursor, RequestId, SessionId,
    StateAnchor, WorkBudget,
};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
use std::thread;

const TICK: u64 = 806_500;
const NONCE: [u8; 32] = [b'n'; 32];
const SNAPSHOT: [u8; 16] = [b's'; 16];

fn io_error(error: std::io::Error) -> DfmcpError {
    invalid(&format!("inventory lifecycle peer I/O: {error}"))
}
fn number(out: &mut Vec<u8>, mut value: u64) {
    while value >= 128 {
        out.push((value as u8 & 127) | 128);
        value >>= 7;
    }
    out.push(value as u8);
}
fn field(out: &mut Vec<u8>, tag: u32, value: u64) {
    number(out, u64::from(tag) << 3);
    number(out, value);
}
fn blob(out: &mut Vec<u8>, tag: u32, value: &[u8]) {
    number(out, (u64::from(tag) << 3) | 2);
    number(out, value.len() as u64);
    out.extend_from_slice(value);
}
fn capture() -> Vec<u8> {
    // Independent empty-roster operations/1.4 fixture. The fortress root is
    // still an entity; no furniture projection is needed to test ownership.
    let mut jobs = b"DFMJ1200".to_vec();
    jobs.extend_from_slice(&2u32.to_be_bytes());
    jobs.extend_from_slice(&100u32.to_be_bytes());
    jobs.push(1);
    jobs.extend_from_slice(&2u32.to_be_bytes());
    jobs.extend_from_slice(&0u32.to_be_bytes());
    jobs.extend_from_slice(&7u16.to_be_bytes());
    jobs.extend_from_slice(b"region1");
    jobs.extend_from_slice(&0u32.to_be_bytes());
    let mut out = b"DFMO1400".to_vec();
    out.extend_from_slice(&(jobs.len() as u32).to_be_bytes());
    out.extend_from_slice(&jobs);
    out.extend_from_slice(&[0; 20]);
    out
}
fn manifest() -> Vec<u8> {
    let mut out = Vec::new();
    field(&mut out, 1, 1);
    field(&mut out, 2, 0);
    blob(&mut out, 3, &NONCE);
    field(&mut out, 4, 1);
    field(&mut out, 5, 4);
    field(&mut out, 6, 987);
    blob(&mut out, 7, b"53.01");
    blob(&mut out, 8, b"53.01-r1");
    out
}
fn serve(mut stream: TcpStream) -> Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(2))).map_err(io_error)?;
    stream.set_write_timeout(Some(Duration::from_secs(2))).map_err(io_error)?;
    let mut greeting = [0; 12];
    stream.read_exact(&mut greeting).map_err(io_error)?;
    require(greeting == *b"DFHack?\n\x01\0\0\0", "unexpected lifecycle greeting")?;
    stream.write_all(b"DFHack!\n\x01\0\0\0").map_err(io_error)?;
    let raw = capture();
    for (index, expected) in [0i16, 0, 2, 3, 3].into_iter().enumerate() {
        let mut header = [0; 8];
        stream.read_exact(&mut header).map_err(io_error)?;
        let count = i32::from_le_bytes([header[4], header[5], header[6], header[7]]);
        require(i16::from_le_bytes([header[0], header[1]]) == expected
            && header[2..4] == [0; 2] && (0..=2048).contains(&count),
            "unexpected lifecycle request")?;
        let mut request = vec![0; count as usize];
        stream.read_exact(&mut request).map_err(io_error)?;
        let mut reply = if index < 2 {
            let mut reply = Vec::new();
            field(&mut reply, 1, index as u64 + 2);
            reply
        } else { manifest() };
        if index == 3 {
            blob(&mut reply, 9, &raw);
            blob(&mut reply, 10, &SNAPSHOT);
            field(&mut reply, 11, 0);
            field(&mut reply, 12, raw.len() as u64);
            blob(&mut reply, 13, Digest32::of_bytes(&raw).as_bytes());
            field(&mut reply, 14, 1);
        } else if index == 4 {
            blob(&mut reply, 10, &SNAPSHOT);
        }
        let mut frame = (-1i16).to_le_bytes().to_vec();
        frame.extend_from_slice(&[0; 2]);
        frame.extend_from_slice(&(reply.len() as i32).to_le_bytes());
        frame.extend(reply);
        stream.write_all(&frame).map_err(io_error)?;
    }
    let mut extra = [0];
    require(stream.read(&mut extra).map_err(io_error)? == 0,
        "native connection was retained or reused after verified release")
}

#[test]
fn verified_release_ends_native_ownership_but_not_request_checks() -> Result<()> {
    for stop in 0..4 {
        let listener = TcpListener::bind("127.0.0.1:0").map_err(io_error)?;
        let endpoint = listener.local_addr().map_err(io_error)?;
        listener.set_nonblocking(true).map_err(io_error)?;
        let peer = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(2);
            loop {
                match listener.accept() {
                    Ok((stream, _)) => return serve(stream),
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock && Instant::now() < deadline => {
                        thread::yield_now();
                    }
                    Err(e) => return Err(io_error(e)),
                }
            }
        });
        let fortress = FortressIdentity::new("region1", 2)?;
        let context = OperationContext {
            session_id: SessionId::new(1), request_id: RequestId::new(2),
            anchor: StateAnchor { fortress_id: fortress.fortress_id(),
                cursor: ObservationCursor::ORIGIN, tick: GameTick(TICK - 1),
                state_hash: Digest32::of_bytes(b"previous") },
            budget: WorkBudget { max_wall_millis: 60_000, max_bytes: MAX_NETWORK_BYTES,
                max_entities: MAX_ENTITIES, ..WorkBudget::CONSERVATIVE_DEFAULT },
            grants: vec![CapabilityGrant { capability: Capability::Query,
                scope: CapabilityScope { fortress_id: Some(fortress.fortress_id()),
                    ..CapabilityScope::default() }, max_risk: RiskTier::ReadOnly,
                expires_at_tick: None, remaining_uses: None }],
            cancellation_requested: false,
        };
        let cancellation = BuildCancellation::default();
        let revoked = Arc::new(AtomicBool::new(false));
        let signal = revoked.clone();
        let permission: ReadPermission = Box::new(move || {
            if signal.load(Ordering::Acquire) {
                Err(DfmcpError::new(ErrorCode::CapabilityDenied, "revoked after release"))
            } else { Ok(()) }
        });
        let deadline = Instant::now() + Duration::from_secs(60);
        let mut work = Work { context: &context, fortress: &fortress,
            cancellation: &cancellation, permission: &permission, deadline,
            high_tick: Cell::new(context.anchor.tick.get()) };
        let observed = read_observation(endpoint, &[b'o'; 32], &NONCE, &work);
        // Join BEFORE projection. This is an ownership assertion, not a race
        // between large-graph CPU time and a peer's idle socket timeout.
        let closed = peer.join().map_err(|_| invalid("lifecycle peer panicked"))?;
        let observed = observed?;
        closed?;
        assert_eq!(work.deadline, deadline);
        assert_eq!(work.high_tick.get(), TICK);
        match stop {
            0 => {
                work.check()?;
                let mut state = LiveOperationsState::with_profile(OperationsProfile::PagedV1_4);
                state.publish(observed)?;
                work.check()?;
                assert!(state.snapshot().is_some());
            }
            1 => { cancellation.cancel(); assert!(work.check().is_err()); }
            2 => { revoked.store(true, Ordering::Release); assert!(work.check().is_err()); }
            _ => { work.deadline = Instant::now(); assert!(work.check().is_err()); }
        }
    }
    Ok(())
}
