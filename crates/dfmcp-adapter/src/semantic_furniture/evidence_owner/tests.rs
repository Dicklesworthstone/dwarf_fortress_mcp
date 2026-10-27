use super::*;
use crate::live_jobs_rpc::operations::map::spatial::SpatialLimits;
use crate::live_jobs_rpc::operations::paged::PagedOperationsLimits;
use crate::live_spatial::LiveSpatialObservation;
use dfmcp_core::{CapabilityGrant, CapabilityScope, Digest32, RequestId, SessionId, WorkBudget};
use dfmcp_world::{EvidenceCoverage, EvidenceSource, FactSource, WorldSnapshot};
use std::cell::RefCell;
use std::io::Cursor;

fn invalid(message: &str) -> DfmcpError {
    error(ErrorCode::InternalInvariantViolation, message)
}
fn field(out: &mut Vec<u8>, raw: &[u8]) -> Result<()> {
    let length = u32::try_from(raw.len()).map_err(|_| invalid("fixture field length"))?;
    out.extend_from_slice(&length.to_be_bytes());
    out.extend_from_slice(raw);
    Ok(())
}
fn text_field(out: &mut Vec<u8>, raw: &[u8]) -> Result<()> {
    let length = u16::try_from(raw.len()).map_err(|_| invalid("fixture text length"))?;
    out.extend_from_slice(&length.to_be_bytes());
    out.extend_from_slice(raw);
    Ok(())
}

fn original() -> Result<LiveSpatialCitizenState> {
    let raw = include_str!("../../../tests/fixtures/spatial_v1_6.hex").trim();
    let bytes = raw
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let text = std::str::from_utf8(pair).map_err(|_| invalid("fixture hex text"))?;
            u8::from_str_radix(text, 16).map_err(|_| invalid("fixture hex byte"))
        })
        .collect::<Result<Vec<_>>>()?;
    let spatial =
        LiveSpatialObservation::decode_payload(&bytes, 801, "df".to_owned(), "dfhack".to_owned())?;
    let mut citizens = b"DFMC1800".to_vec();
    citizens.extend_from_slice(&150u32.to_be_bytes());
    for id in 0u32..150 {
        citizens.extend_from_slice(&id.to_be_bytes());
        text_field(&mut citizens, &[b'U'; 128])?;
        text_field(&mut citizens, b"dwarf")?;
        citizens.extend_from_slice(&3i32.to_be_bytes());
        for value in [1i32, 1, 5] {
            citizens.extend_from_slice(&value.to_be_bytes());
        }
        citizens.extend_from_slice(&0b1_0101_1111u16.to_be_bytes());
        citizens.extend_from_slice(&0i32.to_be_bytes());
        citizens.extend_from_slice(&[1, 1]);
        citizens.extend_from_slice(&0u16.to_be_bytes());
    }
    let mut payload = b"DFMS1800".to_vec();
    field(&mut payload, &spatial.encode_payload()?)?;
    field(&mut payload, &citizens)?;
    let observation = LiveSpatialCitizenObservation::decode_payload(
        &payload,
        801,
        "df".to_owned(),
        "dfhack".to_owned(),
    )?;
    let mut state = LiveSpatialCitizenState::default();
    state.publish(observation)?;
    Ok(state)
}
fn limits(state: &LiveSpatialCitizenState) -> Result<CitizenSpatialLimits> {
    let observation = state
        .observation_full()
        .ok_or_else(|| invalid("fixture observation"))?;
    Ok(CitizenSpatialLimits {
        spatial: SpatialLimits {
            region: observation.spatial().terrain().map.region,
            operations: PagedOperationsLimits {
                jobs: 64,
                buildings: 64,
                items: 64,
                payload_bytes: 64 * 1024,
                page_bytes: 16 * 1024,
            },
        },
        citizens: 256,
    })
}
fn context(state: &LiveSpatialCitizenState) -> Result<OperationContext> {
    let snapshot = state
        .snapshot()
        .ok_or_else(|| invalid("fixture snapshot"))?;
    Ok(OperationContext {
        session_id: SessionId::new(72),
        request_id: RequestId::new(1),
        anchor: snapshot.anchor(),
        budget: WorkBudget {
            max_wall_millis: 60_000,
            max_game_ticks: 0,
            max_entities: 4096,
            max_bytes: 64 * 1024 * 1024,
            max_actions: 1,
            max_output_tokens: 4096,
        },
        grants: [Capability::Query, Capability::Observe]
            .into_iter()
            .map(|capability| CapabilityGrant {
                capability,
                scope: CapabilityScope {
                    fortress_id: Some(snapshot.fortress_id),
                    ..CapabilityScope::default()
                },
                max_risk: RiskTier::ReadOnly,
                expires_at_tick: None,
                remaining_uses: None,
            })
            .collect(),
        cancellation_requested: false,
    })
}
fn policy(snapshot: &WorldSnapshot) -> EvidencePolicy {
    let mut value = EvidencePolicy::at(snapshot.anchor());
    value.all_entities = EvidenceCoverage::Observed;
    value.paused = true;
    for entity in snapshot.graph.entities.values() {
        for fact in entity.fields.values() {
            if let FactSource::DfhackField(field) = &fact.source {
                value.sources.insert(EvidenceSource::Observed {
                    field: field.clone(),
                    source_digest: fact.source_digest,
                });
            }
        }
    }
    value
}

// Independent reply encoding exercises the existing fixed RPC codec, including
// fragment assembly, whole-payload digest checking and verified release.
fn varint(out: &mut Vec<u8>, mut value: u64) {
    while value >= 128 {
        out.push((value as u8 & 127) | 128);
        value >>= 7;
    }
    out.push(value as u8);
}
fn number(out: &mut Vec<u8>, tag: u32, value: u64) {
    varint(out, u64::from(tag) << 3);
    varint(out, value);
}
fn data(out: &mut Vec<u8>, tag: u32, value: &[u8]) {
    varint(out, (u64::from(tag) << 3) | 2);
    varint(out, value.len() as u64);
    out.extend_from_slice(value);
}
fn frame(out: &mut Vec<u8>, payload: &[u8]) -> Result<()> {
    let length = i32::try_from(payload.len()).map_err(|_| invalid("fixture frame length"))?;
    out.extend_from_slice(&(-1i16).to_le_bytes());
    out.extend_from_slice(&[0, 0]);
    out.extend_from_slice(&length.to_le_bytes());
    out.extend_from_slice(payload);
    Ok(())
}
fn envelope() -> Vec<u8> {
    let mut value = Vec::new();
    for (tag, number_value) in [(1, 1), (2, 0), (4, 1), (5, 8), (6, 801)] {
        number(&mut value, tag, number_value);
    }
    data(&mut value, 3, &[2; 16]);
    data(&mut value, 7, b"df");
    data(&mut value, 8, b"dfhack");
    value
}
fn wire(state: &LiveSpatialCitizenState, fault: Option<&str>) -> Result<Vec<u8>> {
    let observation = state
        .observation_full()
        .ok_or_else(|| invalid("fixture observation"))?;
    let payload = observation.encode_payload()?;
    let width = limits(state)?.spatial.operations.page_bytes;
    assert!(payload.len() > width);
    let mut input = b"DFHack!\n".to_vec();
    input.extend_from_slice(&1i32.to_le_bytes());
    for method in [2, 3] {
        let mut binding = Vec::new();
        number(&mut binding, 1, method);
        frame(&mut input, &binding)?;
    }
    frame(&mut input, &envelope())?;
    for (index, part) in payload.chunks(width).enumerate() {
        let mut reply = envelope();
        data(&mut reply, 9, part);
        data(&mut reply, 10, &[3; 16]);
        number(&mut reply, 11, (index * width) as u64);
        number(&mut reply, 12, payload.len() as u64);
        let digest = if fault == Some("digest") {
            Digest32::of_bytes(b"different complete capture")
        } else {
            Digest32::of_bytes(&payload)
        };
        data(&mut reply, 13, digest.as_bytes());
        number(
            &mut reply,
            14,
            u64::from((index + 1) * width >= payload.len()),
        );
        frame(&mut input, &reply)?;
    }
    if fault != Some("release_lost") {
        let mut released = envelope();
        data(
            &mut released,
            10,
            if fault == Some("release_identity") {
                &[4; 16]
            } else {
                &[3; 16]
            },
        );
        frame(&mut input, &released)?;
    }
    Ok(input)
}
#[derive(Default)]
struct IoTrace {
    output: Vec<u8>,
    consumed: u64,
    timeouts: Vec<Duration>,
    closes: usize,
}
struct WireIo {
    input: Cursor<Vec<u8>>,
    trace: Rc<RefCell<IoTrace>>,
}
impl Read for WireIo {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        let length = output.len().min(7);
        let count = self.input.read(&mut output[..length])?;
        self.trace.borrow_mut().consumed = self.input.position();
        Ok(count)
    }
}
impl Write for WireIo {
    fn write(&mut self, input: &[u8]) -> io::Result<usize> {
        let count = input.len().min(11);
        self.trace
            .borrow_mut()
            .output
            .extend_from_slice(&input[..count]);
        Ok(count)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
impl SpatialFurnitureEvidenceIo for WireIo {
    fn narrow_timeout(&mut self, timeout: Duration) -> io::Result<()> {
        self.trace.borrow_mut().timeouts.push(timeout);
        Ok(())
    }
}
impl Drop for WireIo {
    fn drop(&mut self) {
        self.trace.borrow_mut().closes += 1;
    }
}
fn source(
    state: &LiveSpatialCitizenState,
    context: &OperationContext,
    fault: Option<&str>,
    trace: &Rc<RefCell<IoTrace>>,
) -> Result<SpatialFurnitureRpcSource<WireIo>> {
    SpatialFurnitureRpcSource::negotiate(
        WireIo {
            input: Cursor::new(wire(state, fault)?),
            trace: Rc::clone(trace),
        },
        vec![1; 32],
        vec![2; 16],
        limits(state)?,
        context,
    )
}

#[test]
fn actual_spatial_wire_assembles_and_releases_before_independent_policy() -> Result<()> {
    let state = original()?;
    let c = context(&state)?;
    let bound = limits(&state)?;
    let bytes = wire(&state, None)?;
    let total = bytes.len() as u64;
    let trace = Rc::new(RefCell::new(IoTrace::default()));
    let wire_trace = Rc::clone(&trace);
    let factory = move |c: &OperationContext| {
        SpatialFurnitureRpcSource::negotiate(
            WireIo {
                input: Cursor::new(bytes.clone()),
                trace: Rc::clone(&wire_trace),
            },
            vec![1; 32],
            vec![2; 16],
            bound,
            c,
        )
    };
    let authority_trace = Rc::clone(&trace);
    let issued = Rc::new(Cell::new(0));
    let issuer_count = Rc::clone(&issued);
    let authorizer = move |_: &LiveSpatialCitizenObservation,
                           state: &LiveSpatialCitizenState,
                           _: &OperationContext| {
        assert_eq!(authority_trace.borrow().closes, 1);
        assert_eq!(authority_trace.borrow().consumed, total);
        issuer_count.set(issuer_count.get() + 1);
        Ok(policy(
            state.snapshot().ok_or_else(|| invalid("policy snapshot"))?,
        ))
    };
    let owner = SpatialFurnitureEvidenceOwner::new(state, bound, factory, authorizer, &c)?;
    assert_eq!(owner.routing_evidence()?.anchor(), c.anchor);
    assert_eq!(issued.get(), 2);
    let recorded = trace.borrow();
    assert!(recorded.output.starts_with(b"DFHack?\n"));
    assert!(recorded.timeouts.len() > 10);
    assert!(recorded.timeouts.windows(2).all(|pair| pair[1] <= pair[0]));
    Ok(())
}

#[test]
fn actual_source_is_single_use_after_success_and_every_acquisition_failure() -> Result<()> {
    for fault in [
        None,
        Some("digest"),
        Some("release_lost"),
        Some("release_identity"),
    ] {
        let state = original()?;
        let c = context(&state)?;
        let trace = Rc::new(RefCell::new(IoTrace::default()));
        let mut source = source(&state, &c, fault, &trace)?;
        let result = source.read_observation(&c);
        if fault.is_none() {
            assert_eq!(
                result?,
                *state
                    .observation_full()
                    .ok_or_else(|| invalid("observation"))?
            );
        } else {
            assert!(result.is_err());
        }
        let before = trace.borrow().output.len();
        assert!(source.read_observation(&c).is_err());
        assert_eq!(trace.borrow().output.len(), before);
        drop(source);
        assert_eq!(trace.borrow().closes, 1);
    }
    Ok(())
}

#[test]
fn revoked_authority_cancellation_and_byte_exhaustion_fence_without_more_io() -> Result<()> {
    for case in 0..4 {
        let state = original()?;
        let c = context(&state)?;
        let trace = Rc::new(RefCell::new(IoTrace::default()));
        let mut source = source(&state, &c, None, &trace)?;
        let before = trace.borrow().output.len();
        let mut refused = c.clone();
        match case {
            0 => refused
                .grants
                .retain(|grant| grant.capability != Capability::Observe),
            1 => refused
                .grants
                .retain(|grant| grant.capability != Capability::Query),
            2 => refused.cancellation_requested = true,
            _ => refused.budget.max_bytes = 1,
        }
        assert!(source.read_observation(&refused).is_err());
        assert!(source.read_observation(&c).is_err());
        assert_eq!(trace.borrow().output.len(), before);
    }
    Ok(())
}

#[test]
fn larger_next_context_never_renews_the_original_connection_deadline() -> Result<()> {
    let state = original()?;
    let c = context(&state)?;
    let trace = Rc::new(RefCell::new(IoTrace::default()));
    let mut source = source(&state, &c, None, &trace)?;
    let before = trace.borrow().output.len();
    let mut allowance = source.allowance.get();
    allowance.deadline = Instant::now()
        .checked_sub(Duration::from_secs(1))
        .ok_or_else(exhausted)?;
    source.allowance.set(allowance);
    let mut larger = c.clone();
    larger.budget.max_wall_millis *= 2;
    assert!(source.read_observation(&larger).is_err());
    assert!(source.read_observation(&c).is_err());
    assert_eq!(trace.borrow().output.len(), before);
    Ok(())
}

#[test]
fn negotiation_rejects_insufficient_shared_bytes_before_writing() -> Result<()> {
    let state = original()?;
    let mut c = context(&state)?;
    c.budget.max_bytes = 11;
    let trace = Rc::new(RefCell::new(IoTrace::default()));
    assert!(source(&state, &c, None, &trace).is_err());
    assert!(trace.borrow().output.is_empty());
    assert_eq!(trace.borrow().closes, 1);
    Ok(())
}
