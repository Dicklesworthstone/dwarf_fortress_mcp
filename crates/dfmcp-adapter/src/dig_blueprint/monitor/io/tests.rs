//! Execute the actual map/1.5 client through fragmented, bounded owned streams.
use super::super::tests::{capture as fixture_capture, spec as fixture_spec};
use super::*;
use crate::dig_blueprint::BlueprintPart;
use crate::live_jobs_rpc::JobsManifest;
use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, MapCoord, MapCuboid, ObservationCursor,
    RequestId, RiskTier, SessionId, StateAnchor, WorkBudget,
};
use std::cell::RefCell;
use std::io::Cursor;
use std::rc::Rc;

fn spec() -> Result<MonitorSpec> {
    fixture_spec(vec![
        BlueprintPart {
            origin: [2, 2, 2],
            size: [2, 1, 1],
        },
        BlueprintPart {
            origin: [2, 4, 3],
            size: [1, 1, 1],
        },
    ])
}
fn context(spec: &MonitorSpec) -> Result<OperationContext> {
    let region = spec.region();
    let [x, y, z] = region.origin;
    let [w, h, d] = region.size;
    let area = MapCuboid::new(
        MapCoord::new(x as i32, y as i32, z as i32),
        MapCoord::new((x + w - 1) as i32, (y + h - 1) as i32, (z + d - 1) as i32),
    )?;
    let fortress = spec.parent().binding().fortress_id();
    Ok(OperationContext {
        session_id: SessionId::new(91),
        request_id: RequestId::new(1),
        anchor: StateAnchor {
            fortress_id: fortress,
            cursor: ObservationCursor::ORIGIN,
            tick: GameTick(spec.start_tick()),
            state_hash: spec.parent().id(),
        },
        budget: WorkBudget {
            max_wall_millis: 5000,
            max_game_ticks: 0,
            max_entities: region.volume().map_err(|_| exhausted())? as u32,
            max_bytes: READ_BYTES,
            max_output_tokens: 8192,
            max_actions: 1,
        },
        cancellation_requested: false,
        grants: [
            (Capability::Query, spec.parent().binding().scope()),
            (Capability::Observe, area),
        ]
        .into_iter()
        .map(|(capability, area)| CapabilityGrant {
            capability,
            scope: CapabilityScope {
                fortress_id: Some(fortress),
                map_area: Some(area),
                ..CapabilityScope::default()
            },
            max_risk: RiskTier::ReadOnly,
            expires_at_tick: None,
            remaining_uses: None,
        })
        .collect(),
    })
}
fn manifest(spec: &MonitorSpec) -> JobsManifest {
    JobsManifest {
        generation: spec.source().generation(),
        df_version: spec.source().df_version().into(),
        dfhack_version: spec.source().dfhack_version().into(),
    }
}
fn varint(out: &mut Vec<u8>, mut value: u64) {
    while value >= 128 {
        out.push((value as u8 & 127) | 128);
        value >>= 7;
    }
    out.push(value as u8);
}
fn number(out: &mut Vec<u8>, field: u32, value: u64) {
    varint(out, u64::from(field) << 3);
    varint(out, value);
}
fn bytes(out: &mut Vec<u8>, field: u32, value: &[u8]) {
    varint(out, u64::from(field) << 3 | 2);
    varint(out, value.len() as u64);
    out.extend_from_slice(value);
}
fn frame(id: i16, payload: &[u8]) -> Vec<u8> {
    let mut out = vec![0; 8];
    out[..2].copy_from_slice(&id.to_le_bytes());
    out[4..].copy_from_slice(&(payload.len() as i32).to_le_bytes());
    out.extend_from_slice(payload);
    out
}
fn reply(pin: &JobsManifest, nonce: &[u8], payload: Option<&[u8]>) -> Vec<u8> {
    let mut out = Vec::new();
    number(&mut out, 1, 1);
    number(&mut out, 2, 0);
    bytes(&mut out, 3, nonce);
    number(&mut out, 4, 1);
    number(&mut out, 5, 5);
    number(&mut out, 6, pin.generation);
    bytes(&mut out, 7, pin.df_version.as_bytes());
    bytes(&mut out, 8, pin.dfhack_version.as_bytes());
    if let Some(payload) = payload {
        bytes(&mut out, 9, payload);
    }
    out
}
fn script(handshake: &[u8], observation: &[u8]) -> Vec<u8> {
    let mut out = b"DFHack!\n".to_vec();
    out.extend_from_slice(&1i32.to_le_bytes());
    for id in [2, 3] {
        let mut body = Vec::new();
        number(&mut body, 1, id);
        out.extend(frame(-1, &body));
    }
    out.extend(frame(-1, handshake));
    out.extend(frame(-1, observation));
    out
}
fn wire(capture: &LiveMapObservation, handshake: &JobsManifest) -> Result<Vec<u8>> {
    let current = JobsManifest {
        generation: capture.bridge_generation,
        df_version: capture.df_version.clone(),
        dfhack_version: capture.dfhack_version.clone(),
    };
    Ok(script(
        &reply(handshake, &[b'n'; 16], None),
        &reply(&current, &[b'n'; 16], Some(&capture.encode_payload()?)),
    ))
}
fn request(spec: &MonitorSpec) -> Vec<u8> {
    let mut out = Vec::new();
    bytes(&mut out, 1, &[b't'; 32]);
    bytes(&mut out, 2, &[b'n'; 16]);
    number(&mut out, 3, 1);
    number(&mut out, 4, 5);
    for (index, value) in spec
        .region()
        .origin
        .into_iter()
        .chain(spec.region().size)
        .enumerate()
    {
        number(&mut out, index as u32 + 5, u64::from(value));
    }
    number(&mut out, 11, MAX_PAYLOAD_BYTES as u64);
    out
}

struct PeerState {
    input: Cursor<Vec<u8>>,
    output: Vec<u8>,
    reads: usize,
    writes: usize,
    flushes: usize,
    deadlines: Vec<Instant>,
    revoked: bool,
    revoke_after_read: Option<usize>,
    revoke_when_complete: bool,
    fail_deadline: bool,
    dropped: bool,
}
struct Peer {
    state: Rc<RefCell<PeerState>>,
    fragment: usize,
}
impl Peer {
    fn new(input: Vec<u8>, fragment: usize) -> (Self, Rc<RefCell<PeerState>>) {
        let state = Rc::new(RefCell::new(PeerState {
            input: Cursor::new(input),
            output: vec![],
            reads: 0,
            writes: 0,
            flushes: 0,
            deadlines: vec![],
            revoked: false,
            revoke_after_read: None,
            revoke_when_complete: false,
            fail_deadline: false,
            dropped: false,
        }));
        (
            Self {
                state: state.clone(),
                fragment,
            },
            state,
        )
    }
}
impl Drop for Peer {
    fn drop(&mut self) {
        self.state.borrow_mut().dropped = true;
    }
}
impl Read for Peer {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let mut state = self.state.borrow_mut();
        state.reads += 1;
        let length = out.len().min(self.fragment);
        let count = state.input.read(&mut out[..length])?;
        if state.revoke_after_read == Some(state.reads)
            || (state.revoke_when_complete
                && state.input.position() == state.input.get_ref().len() as u64)
        {
            state.revoked = true;
        }
        Ok(count)
    }
}
impl Write for Peer {
    fn write(&mut self, input: &[u8]) -> io::Result<usize> {
        let mut state = self.state.borrow_mut();
        state.writes += 1;
        let length = input.len().min(self.fragment);
        state.output.extend_from_slice(&input[..length]);
        Ok(length)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.state.borrow_mut().flushes += 1;
        Ok(())
    }
}
impl ProofStream for Peer {
    fn set_deadline(&mut self, deadline: Instant) -> io::Result<()> {
        let mut state = self.state.borrow_mut();
        state.deadlines.push(deadline);
        if state.fail_deadline {
            return Err(io::Error::other("private fixture detail"));
        }
        Ok(())
    }
}
fn active(peer: &Rc<RefCell<PeerState>>) -> Result<()> {
    if peer.borrow().revoked {
        return Err(error(
            ErrorCode::CancellationRequested,
            "fixture runtime cancelled",
        ));
    }
    Ok(())
}
fn acquire(peer: Peer, spec: &MonitorSpec, c: &OperationContext) -> Result<LiveMapObservation> {
    let state = peer.state.clone();
    read_map(peer, spec, vec![b't'; 32], vec![b'n'; 16], c, move |_| {
        active(&state)
    })
}
fn frames(raw: &[u8]) -> Result<Vec<(i16, Vec<u8>)>> {
    let hello = b"DFHack?\n\x01\0\0\0";
    if !raw.starts_with(hello) {
        return Err(exhausted());
    }
    let mut rest = &raw[hello.len()..];
    let mut frames = vec![];
    while !rest.is_empty() {
        let header = rest.get(..8).ok_or_else(exhausted)?;
        assert_eq!(&header[2..4], &[0, 0]);
        let id = i16::from_le_bytes(header[..2].try_into().map_err(|_| exhausted())?);
        let length = i32::from_le_bytes(header[4..8].try_into().map_err(|_| exhausted())?);
        let length = usize::try_from(length).map_err(|_| exhausted())?;
        let body = rest.get(8..8 + length).ok_or_else(exhausted)?;
        frames.push((id, body.to_vec()));
        rest = &rest[8 + length..];
    }
    Ok(frames)
}
fn bind(method: &[u8]) -> Vec<u8> {
    let mut out = vec![];
    for (field, value) in [
        (1, method),
        (2, &b"dfmcp.map.v1_5.Request"[..]),
        (3, &b"dfmcp.map.v1_5.Reply"[..]),
        (4, &b"dfmcp_map_v1_5"[..]),
    ] {
        bytes(&mut out, field, value);
    }
    out
}

#[test]
fn fragmented_real_wire_reads_one_complete_enclosure_with_only_closed_read_methods() -> Result<()> {
    let spec = spec()?;
    let expected = fixture_capture(&spec, 1010)?;
    let (peer, state) = Peer::new(wire(&expected, &manifest(&spec))?, 3);
    let mut checks = vec![];
    let observed = read_map(
        peer,
        &spec,
        vec![b't'; 32],
        vec![b'n'; 16],
        &context(&spec)?,
        |c| {
            active(&state)?;
            checks.push((c.budget.max_bytes, c.budget.max_wall_millis, c.anchor.tick));
            Ok(())
        },
    )?;
    assert_eq!(observed, expected);
    spec.validate_capture(&observed)?;
    let peer = state.borrow();
    assert!(peer.dropped);
    assert_eq!(peer.input.position(), peer.input.get_ref().len() as u64);
    assert_eq!(checks.len(), peer.reads + peer.writes + peer.flushes);
    assert_eq!(checks.len(), peer.deadlines.len());
    assert!(peer.deadlines.windows(2).all(|v| v[0] == v[1]));
    assert!(
        checks
            .windows(2)
            .all(|v| v[1].0 <= v[0].0 && v[1].1 <= v[0].1)
    );
    assert!(
        checks
            .iter()
            .all(|v| v.0 <= WIRE_BYTES && v.2 == GameTick(1000))
    );
    assert_eq!(
        frames(&peer.output)?,
        vec![
            (0, bind(b"Handshake")),
            (0, bind(b"ReadObservation")),
            (2, request(&spec)),
            (3, request(&spec)),
        ]
    );
    Ok(())
}

#[test]
fn full_1024_cell_capture_is_one_unclipped_multilevel_read() -> Result<()> {
    let spec = fixture_spec(vec![
        BlueprintPart {
            origin: [2, 2, 2],
            size: [1, 1, 1],
        },
        BlueprintPart {
            origin: [9, 9, 17],
            size: [1, 1, 1],
        },
    ])?;
    assert_eq!(spec.region().volume().map_err(|_| exhausted())?, 1024);
    let expected = fixture_capture(&spec, 1010)?;
    assert!(expected.encode_payload()?.len() < MAX_PAYLOAD_BYTES);
    let (peer, state) = Peer::new(wire(&expected, &manifest(&spec))?, 7);
    let observed = acquire(peer, &spec, &context(&spec)?)?;
    assert_eq!(observed.map.cells.len(), 1024);
    assert_eq!(observed.map.region, spec.region());
    assert_eq!(observed, expected);
    assert_eq!(
        frames(&state.borrow().output)?
            .iter()
            .filter(|v| v.0 == 3)
            .count(),
        1
    );
    Ok(())
}

#[test]
fn existing_native_map_fixture_passes_actual_fragmented_codec_unchanged() -> Result<()> {
    let spec = fixture_spec(vec![BlueprintPart {
        origin: [14, 15, 1],
        size: [4, 3, 2],
    }])?;
    let hex = include_str!("../../../../tests/fixtures/map_v1_5.hex").trim();
    let raw = (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).map_err(|_| exhausted()))
        .collect::<Result<Vec<_>>>()?;
    let pin = manifest(&spec);
    let input = script(
        &reply(&pin, &[b'n'; 16], None),
        &reply(&pin, &[b'n'; 16], Some(&raw)),
    );
    let (peer, _) = Peer::new(input, 1);
    let observed = acquire(peer, &spec, &context(&spec)?)?;
    assert_eq!(observed.map.cells.len(), 24);
    assert_eq!(observed.encode_payload()?, raw);
    Ok(())
}

#[test]
fn scope_budget_cancellation_and_expired_retained_grants_refuse_before_any_io() -> Result<()> {
    let spec = spec()?;
    let base = context(&spec)?;
    let expected = fixture_capture(&spec, 1010)?;
    let input = wire(&expected, &manifest(&spec))?;
    let mut variants = vec![];
    for capability in [Capability::Query, Capability::Observe] {
        let mut c = base.clone();
        c.grants.retain(|g| g.capability != capability);
        variants.push((c, ErrorCode::CapabilityDenied));
    }
    let mut c = base.clone();
    c.grants[1].scope.map_area = Some(MapCuboid::new(
        MapCoord::new(2, 2, 2),
        MapCoord::new(3, 3, 3),
    )?);
    variants.push((c, ErrorCode::CapabilityDenied));
    let mut c = base.clone();
    c.budget.max_bytes = READ_BYTES - 1;
    variants.push((c, ErrorCode::BudgetExceeded));
    let mut c = base.clone();
    c.budget.max_entities -= 1;
    variants.push((c, ErrorCode::BudgetExceeded));
    let mut c = base.clone();
    c.budget.max_wall_millis = 60_001;
    variants.push((c, ErrorCode::BudgetExceeded));
    let mut c = base.clone();
    c.cancellation_requested = true;
    variants.push((c, ErrorCode::CancellationRequested));
    let mut c = base;
    c.anchor.tick = GameTick(0);
    c.grants
        .iter_mut()
        .for_each(|g| g.expires_at_tick = Some(GameTick(999)));
    variants.push((c, ErrorCode::CapabilityDenied));
    for (c, code) in variants {
        let (peer, state) = Peer::new(input.clone(), 3);
        let cause = acquire(peer, &spec, &c).err().ok_or_else(exhausted)?;
        assert_eq!(cause.code, code);
        let peer = state.borrow();
        assert!(peer.output.is_empty());
        assert_eq!(peer.reads + peer.writes + peer.flushes, 0);
        assert!(peer.deadlines.is_empty());
    }
    Ok(())
}

#[test]
fn malformed_credentials_refuse_before_stream_or_runtime_work() -> Result<()> {
    let spec = spec()?;
    for (token, nonce) in [
        (vec![b't'; 31], vec![b'n'; 16]),
        (vec![b't'; 32], vec![b'n'; 15]),
    ] {
        let (peer, state) = Peer::new(vec![], 3);
        let mut checks = 0;
        assert!(
            read_map(peer, &spec, token, nonce, &context(&spec)?, |_| {
                checks += 1;
                Ok(())
            })
            .is_err()
        );
        assert_eq!(checks, 0);
        assert!(state.borrow().output.is_empty());
    }
    Ok(())
}

#[test]
fn runtime_revocation_between_fragments_preserves_exact_error_and_stops_io() -> Result<()> {
    let spec = spec()?;
    let expected = fixture_capture(&spec, 1010)?;
    let (peer, state) = Peer::new(wire(&expected, &manifest(&spec))?, 3);
    state.borrow_mut().revoke_after_read = Some(1);
    let cause = acquire(peer, &spec, &context(&spec)?)
        .err()
        .ok_or_else(exhausted)?;
    assert_eq!(cause.code, ErrorCode::CancellationRequested);
    let peer = state.borrow();
    assert_eq!(peer.reads, 1);
    assert_eq!(peer.input.position(), 3);
    assert_eq!(peer.output, b"DFHack?\n\x01\0\0\0");
    assert!(peer.dropped);
    Ok(())
}

#[test]
fn completely_acquired_evidence_survives_late_revocation_and_new_tick_expiry() -> Result<()> {
    let spec = spec()?;
    let expected = fixture_capture(&spec, 1010)?;
    let (peer, state) = Peer::new(wire(&expected, &manifest(&spec))?, 3);
    state.borrow_mut().revoke_when_complete = true;
    let mut c = context(&spec)?;
    c.grants
        .iter_mut()
        .for_each(|g| g.expires_at_tick = Some(GameTick(1009)));
    let raw = acquire(peer, &spec, &c)?;
    assert_eq!(raw, expected);
    assert!(state.borrow().revoked);
    assert_eq!(
        active(&state).err().ok_or_else(exhausted)?.code,
        ErrorCode::CancellationRequested
    );
    c.anchor.tick = raw.tick();
    assert_eq!(
        spec.authorize(&c, true).err().ok_or_else(exhausted)?.code,
        ErrorCode::CapabilityDenied
    );
    Ok(())
}

#[test]
fn valid_changed_source_or_regressed_tick_is_returned_for_durable_invalidation() -> Result<()> {
    let spec = spec()?;
    for changed in 0..4 {
        let mut raw = fixture_capture(&spec, 1010)?;
        let mut handshake = manifest(&spec);
        match changed {
            0 => raw.bridge_generation += 1,
            1 => {
                raw.df_version.push_str("-other");
                handshake.df_version = raw.df_version.clone();
            }
            2 => raw.year_tick = 999,
            _ => raw.map_dimensions[0] -= 1,
        }
        raw.validate()?;
        let (peer, _) = Peer::new(wire(&raw, &handshake)?, 3);
        let observed = acquire(peer, &spec, &context(&spec)?)?;
        assert_eq!(observed, raw);
        assert!(spec.validate_capture(&observed).is_err());
    }
    Ok(())
}

#[test]
fn within_connection_source_drift_and_nonce_corruption_never_publish_raw() -> Result<()> {
    let spec = spec()?;
    let raw = fixture_capture(&spec, 1010)?;
    for variant in 0..3 {
        let mut handshake = manifest(&spec);
        let mut observed = raw.clone();
        let input = match variant {
            0 => {
                handshake.generation += 1;
                wire(&observed, &handshake)?
            }
            1 => {
                observed.dfhack_version.push_str("-other");
                wire(&observed, &handshake)?
            }
            _ => script(
                &reply(&handshake, &[b'x'; 16], None),
                &reply(&handshake, &[b'n'; 16], Some(&raw.encode_payload()?)),
            ),
        };
        let (peer, _) = Peer::new(input, 3);
        assert!(acquire(peer, &spec, &context(&spec)?).is_err());
    }
    Ok(())
}

#[test]
fn truncation_oversized_reply_and_duplicate_fields_fail_without_extra_methods() -> Result<()> {
    let spec = spec()?;
    let raw = fixture_capture(&spec, 1010)?;
    let pin = manifest(&spec);
    let complete = wire(&raw, &pin)?;
    let mut duplicate = reply(&pin, &[b'n'; 16], Some(&raw.encode_payload()?));
    number(&mut duplicate, 1, 1);
    let mut oversized = script(&reply(&pin, &[b'n'; 16], None), &[]);
    let offset = oversized.len() - 4;
    oversized[offset..].copy_from_slice(&((MAX_PAYLOAD_BYTES + 4097) as i32).to_le_bytes());
    for input in [
        complete[..complete.len() - 1].to_vec(),
        script(&reply(&pin, &[b'n'; 16], None), &duplicate),
        oversized,
    ] {
        let (peer, state) = Peer::new(input, 3);
        assert!(acquire(peer, &spec, &context(&spec)?).is_err());
        let peer = state.borrow();
        assert_eq!(frames(&peer.output)?.iter().filter(|v| v.0 == 3).count(), 1);
        assert!(peer.dropped);
    }
    Ok(())
}

#[test]
fn exhausted_absolute_deadline_and_failed_timeout_setup_precede_first_native_byte() -> Result<()> {
    let spec = spec()?;
    let c = context(&spec)?;
    let (peer, state) = Peer::new(vec![], 3);
    let mut work = Work::new(&spec, &c, |_| Ok(()))?;
    work.deadline = Instant::now();
    let result = MapRpcClient::negotiate(
        GuardedStream {
            stream: peer,
            work: &mut work,
        },
        vec![b't'; 32],
        vec![b'n'; 16],
        spec.region(),
        MAX_PAYLOAD_BYTES,
    );
    assert!(result.is_err());
    drop(result);
    assert_eq!(
        work.failed.as_ref().ok_or_else(exhausted)?.code,
        ErrorCode::BudgetExceeded
    );
    assert!(state.borrow().output.is_empty());
    let (peer, state) = Peer::new(vec![], 3);
    state.borrow_mut().fail_deadline = true;
    let cause = acquire(peer, &spec, &c).err().ok_or_else(exhausted)?;
    assert_eq!(cause.code, ErrorCode::AdapterUnavailable);
    assert!(!cause.message.contains("private fixture detail"));
    assert!(state.borrow().output.is_empty());
    assert_eq!(state.borrow().reads + state.borrow().writes, 0);
    Ok(())
}

#[test]
fn bounded_stream_never_transfers_beyond_remaining_shared_wire_allowance() -> Result<()> {
    let spec = spec()?;
    let c = context(&spec)?;
    let (peer, state) = Peer::new(vec![1; 10], 20);
    let mut work = Work::new(&spec, &c, |_| Ok(()))?;
    work.bytes = 3;
    let mut guarded = GuardedStream {
        stream: peer,
        work: &mut work,
    };
    assert!(guarded.write_all(&[2; 10]).is_err());
    drop(guarded);
    assert_eq!(state.borrow().output, vec![2; 3]);
    assert_eq!(work.bytes, 0);
    assert_eq!(
        work.failed.as_ref().ok_or_else(exhausted)?.code,
        ErrorCode::BudgetExceeded
    );
    Ok(())
}
