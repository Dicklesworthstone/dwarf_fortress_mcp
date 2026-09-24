use super::*;
use std::cell::RefCell;
use std::io::Cursor;
use std::rc::Rc;

use super::super::tests::{fixture, plan};
use dfmcp_core::{
    CapabilityGrant, CapabilityScope, MapCoord, MapCuboid, ObservationCursor, RequestId, SessionId,
    StateAnchor, WorkBudget,
};

const NONCE: &[u8] = b"nnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnn";
type Written = Rc<RefCell<Vec<u8>>>;
struct Stream {
    input: Cursor<Vec<u8>>,
    writes: Written,
}
impl Read for Stream {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let n = out.len().min(3);
        self.input.read(&mut out[..n])
    }
}
impl Write for Stream {
    fn write(&mut self, raw: &[u8]) -> io::Result<usize> {
        let n = raw.len().min(7);
        self.writes.borrow_mut().extend_from_slice(&raw[..n]);
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
impl DigStream for Stream {
    fn narrow_deadline(&mut self, _: Duration) -> Result<()> {
        Ok(())
    }
}
fn context() -> Result<OperationContext> {
    let plan = plan()?;
    let fortress = plan.before().fortress_id();
    Ok(OperationContext {
        session_id: SessionId::new(1),
        request_id: RequestId::new(1),
        anchor: StateAnchor {
            fortress_id: fortress,
            cursor: ObservationCursor::ORIGIN,
            tick: GameTick(12345),
            state_hash: plan.before().witness(),
        },
        budget: WorkBudget {
            max_wall_millis: 5000,
            max_bytes: CONNECT_BYTES + 10 * RPC_BYTES,
            max_entities: 300,
            max_game_ticks: 0,
            max_output_tokens: 32768,
            max_actions: 1,
        },
        grants: [
            Capability::Query,
            Capability::Observe,
            Capability::Plan,
            Capability::Designate,
        ]
        .into_iter()
        .map(|capability| CapabilityGrant {
            capability,
            scope: CapabilityScope {
                fortress_id: Some(fortress),
                entity_ids: Default::default(),
                map_area: Some(MapCuboid {
                    min: MapCoord::new(0, 0, 1),
                    max: MapCoord::new(31, 31, 3),
                }),
            },
            max_risk: RiskTier::Guarded,
            expires_at_tick: None,
            remaining_uses: None,
        })
        .collect(),
        cancellation_requested: false,
    })
}
fn envelope(payload: &[u8], kind: i16) -> Vec<u8> {
    let mut out = kind.to_le_bytes().to_vec();
    out.extend_from_slice(&[0, 0]);
    out.extend_from_slice(&(payload.len() as i32).to_le_bytes());
    out.extend_from_slice(payload);
    out
}
fn reply(capture: bool, effect: Option<&[u8]>, replayed: Option<bool>) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    for (field, value) in [(1, 1), (2, 0), (4, 1), (5, 16), (6, 7)] {
        number(&mut out, field, value);
    }
    bytes(&mut out, 3, NONCE);
    bytes(&mut out, 7, b"test-df");
    bytes(&mut out, 8, b"test-dfhack");
    if capture {
        bytes(&mut out, 9, &fixture("observation")?);
    }
    if let Some(raw) = effect {
        bytes(&mut out, 10, raw);
    }
    if let Some(flag) = replayed {
        number(&mut out, 11, u64::from(flag));
    }
    Ok(out)
}
fn setup(replies: Vec<Vec<u8>>) -> Result<(DigRpcClient<Stream>, Written)> {
    let mut input = b"DFHack!\n\x01\0\0\0".to_vec();
    for id in 2..8 {
        let mut binding = Vec::new();
        number(&mut binding, 1, id);
        input.extend_from_slice(&envelope(&binding, -1));
    }
    input.extend_from_slice(&envelope(&reply(false, None, None)?, -1));
    for payload in replies {
        input.extend_from_slice(&envelope(&payload, -1));
    }
    let writes = Rc::new(RefCell::new(Vec::new()));
    let stream = Stream {
        input: Cursor::new(input),
        writes: writes.clone(),
    };
    let client = DigRpcClient::negotiate(
        stream,
        vec![b's'; 32],
        NONCE.to_vec(),
        plan()?.before().region(),
        &context()?,
    )?;
    Ok((client, writes))
}
fn methods(writes: &[u8]) -> Vec<i16> {
    assert_eq!(&writes[..12], b"DFHack?\n\x01\0\0\0");
    let mut at = 12;
    let mut out = Vec::new();
    while at < writes.len() {
        let method = i16::from_le_bytes([writes[at], writes[at + 1]]);
        let n = i32::from_le_bytes([
            writes[at + 4],
            writes[at + 5],
            writes[at + 6],
            writes[at + 7],
        ]);
        assert!((0..=2048).contains(&n));
        out.push(method);
        at += 8 + n as usize;
    }
    assert_eq!(at, writes.len());
    out
}

#[test]
fn fragmented_observe_prepare_commit_and_query_use_only_fixed_native_methods() -> Result<()> {
    let prepared = fixture("prepared")?;
    let designated = fixture("designated")?;
    let (mut client, written) = setup(vec![
        reply(true, None, None)?,
        reply(false, Some(&prepared), Some(false))?,
        reply(false, Some(&designated), None)?,
        reply(false, Some(&designated), None)?,
    ])?;
    let ctx = context()?;
    let plan = plan()?;
    assert_eq!(
        client.observe(plan.before().region(), &ctx)?,
        *plan.before()
    );
    assert!(!client.prepare(&plan, &ctx)?.replayed());
    assert_eq!(client.commit(&plan, &ctx)?.phase(), DigPhase::Designated);
    assert_eq!(
        client
            .query(&plan, &ctx)?
            .ok_or_else(|| error(ErrorCode::InvalidRequest, "missing fixture effect"))?
            .phase(),
        DigPhase::Designated
    );
    assert_eq!(
        methods(&written.borrow()),
        [0, 0, 0, 0, 0, 0, 2, 3, 4, 5, 6]
    );
    let previous = written.borrow().len();
    assert!(client.commit(&plan, &ctx).is_err());
    assert_eq!(written.borrow().len(), previous);
    assert!(client.prepare(&plan, &ctx).is_err());
    assert_eq!(written.borrow().len(), previous);
    Ok(())
}

#[test]
fn replayed_preparations_and_queried_prepared_records_never_authorize_dispatch() -> Result<()> {
    for name in ["prepared", "designated", "cancelled"] {
        let raw = fixture(name)?;
        let (mut client, written) = setup(vec![reply(false, Some(&raw), Some(true))?])?;
        assert!(client.prepare(&plan()?, &context()?)?.replayed());
        let previous = written.borrow().len();
        assert!(client.commit(&plan()?, &context()?).is_err());
        assert_eq!(written.borrow().len(), previous);
    }
    let raw = fixture("prepared")?;
    let (mut client, written) = setup(vec![reply(false, Some(&raw), None)?])?;
    assert_eq!(
        client
            .query(&plan()?, &context()?)?
            .ok_or_else(|| error(ErrorCode::InvalidRequest, "missing fixture effect"))?
            .phase(),
        DigPhase::Prepared
    );
    let previous = written.borrow().len();
    assert!(client.commit(&plan()?, &context()?).is_err());
    assert_eq!(written.borrow().len(), previous);
    Ok(())
}

#[test]
fn failed_commit_fences_stream_and_new_connection_can_only_query() -> Result<()> {
    let raw = fixture("prepared")?;
    let (mut client, written) = setup(vec![reply(false, Some(&raw), Some(false))?])?;
    client.prepare(&plan()?, &context()?)?;
    assert!(client.commit(&plan()?, &context()?).is_err());
    assert!(client.fenced());
    let previous = written.borrow().len();
    assert!(client.commit(&plan()?, &context()?).is_err());
    assert!(client.query(&plan()?, &context()?).is_err());
    assert_eq!(written.borrow().len(), previous);
    let raw = fixture("designated")?;
    let (mut recovery, written) = setup(vec![reply(false, Some(&raw), None)?])?;
    assert!(recovery.commit(&plan()?, &context()?).is_err());
    assert_eq!(
        recovery
            .query(&plan()?, &context()?)?
            .ok_or_else(|| error(ErrorCode::InvalidRequest, "missing fixture effect"))?
            .phase(),
        DigPhase::Designated
    );
    assert!(!methods(&written.borrow()).contains(&5));
    Ok(())
}

#[test]
fn scoped_authority_includes_halo_and_shared_block_writes_and_current_grants() -> Result<()> {
    let p = plan()?;
    let r = p.before().region();
    assert!(
        authorize(
            &context()?,
            p.before().fortress_id(),
            12345,
            r,
            false,
            true,
            true
        )
        .is_ok()
    );
    for capability in [
        Capability::Query,
        Capability::Observe,
        Capability::Plan,
        Capability::Designate,
    ] {
        let mut ctx = context()?;
        ctx.grants.retain(|g| g.capability != capability);
        assert!(authorize(&ctx, p.before().fortress_id(), 12345, r, false, true, true).is_err());
    }
    for grant in [Capability::Plan, Capability::Designate] {
        let mut ctx = context()?;
        ctx.grants
            .iter_mut()
            .find(|g| g.capability == grant)
            .ok_or_else(|| error(ErrorCode::InvalidRequest, "missing fixture grant"))?
            .scope
            .map_area = Some(r.halo());
        assert!(authorize(&ctx, p.before().fortress_id(), 12345, r, false, true, true).is_err());
    }
    let mut ctx = context()?;
    ctx.cancellation_requested = true;
    assert!(authorize(&ctx, p.before().fortress_id(), 12345, r, false, true, true).is_err());
    for limited in [true, false] {
        let mut ctx = context()?;
        for g in &mut ctx.grants {
            if limited {
                g.remaining_uses = Some(1);
            } else {
                g.expires_at_tick = Some(GameTick(12344));
            }
        }
        assert!(authorize(&ctx, p.before().fortress_id(), 12345, r, false, true, true).is_err());
    }
    assert!(authorize(&context()?, FortressId::new(0), 12345, r, false, true, true).is_err());
    Ok(())
}

#[test]
fn authority_revocation_and_region_changes_fail_before_writing_request() -> Result<()> {
    let (mut client, written) = setup(vec![])?;
    let previous = written.borrow().len();
    let mut ctx = context()?;
    ctx.grants.clear();
    assert!(client.observe(plan()?.before().region(), &ctx).is_err());
    assert!(client.prepare(&plan()?, &ctx).is_err());
    assert!(
        client
            .observe(DigRegion::new(1, 1, 1, 1, 1)?, &context()?)
            .is_err()
    );
    assert_eq!(written.borrow().len(), previous);
    Ok(())
}

#[test]
fn wrong_source_profile_nonce_and_reply_shape_fence_native_stream() -> Result<()> {
    for (field, value) in [(3, 1), (5, 15), (6, 8), (11, 0)] {
        let mut raw = reply(true, None, None)?;
        number(&mut raw, field, value); // duplicate fields are always rejected
        let (mut client, written) = setup(vec![raw])?;
        assert!(
            client
                .observe(plan()?.before().region(), &context()?)
                .is_err()
        );
        assert!(client.fenced());
        let previous = written.borrow().len();
        assert!(
            client
                .observe(plan()?.before().region(), &context()?)
                .is_err()
        );
        assert_eq!(written.borrow().len(), previous);
    }
    let mut raw = reply(false, None, None)?;
    let mut capture = fixture("observation")?;
    capture[15] = 8;
    bytes(&mut raw, 9, &capture);
    let (mut client, _) = setup(vec![raw])?;
    assert!(
        client
            .observe(plan()?.before().region(), &context()?)
            .is_err()
    );
    assert!(client.fenced());
    Ok(())
}

#[test]
fn expired_actual_capture_and_exhausted_entity_allowance_are_rejected() -> Result<()> {
    let (mut client, written) = setup(vec![reply(true, None, None)?])?;
    let mut ctx = context()?;
    ctx.budget.max_entities = 47;
    let previous = written.borrow().len();
    assert!(client.observe(plan()?.before().region(), &ctx).is_err());
    assert_eq!(written.borrow().len(), previous);
    let mut ctx = context()?;
    ctx.anchor.tick = GameTick(12340);
    for grant in &mut ctx.grants {
        grant.expires_at_tick = Some(GameTick(12344));
    }
    assert!(client.observe(plan()?.before().region(), &ctx).is_err());
    assert!(client.fenced());
    Ok(())
}

#[test]
fn native_missing_query_is_not_a_negative_effect_receipt() -> Result<()> {
    let (mut client, written) = setup(vec![reply(false, None, None)?])?;
    assert!(client.query(&plan()?, &context()?)?.is_none());
    assert!(!methods(&written.borrow()).contains(&5));
    assert!(client.commit(&plan()?, &context()?).is_err());
    Ok(())
}

#[test]
fn cancellation_retires_fresh_permit_and_never_replays_a_setter() -> Result<()> {
    let prepared = fixture("prepared")?;
    let cancelled = fixture("cancelled")?;
    let (mut client, written) = setup(vec![
        reply(false, Some(&prepared), Some(false))?,
        reply(false, Some(&cancelled), None)?,
    ])?;
    client.prepare(&plan()?, &context()?)?;
    assert_eq!(
        client.cancel(&plan()?, &context()?)?.phase(),
        DigPhase::Refused
    );
    assert!(client.commit(&plan()?, &context()?).is_err());
    assert!(!methods(&written.borrow()).contains(&5));
    Ok(())
}

#[test]
fn malformed_protobuf_frames_and_binding_aliases_are_refused() -> Result<()> {
    for raw in [
        vec![8, 128, 0],
        vec![8, 1, 8, 1],
        vec![0, 0],
        vec![11],
        vec![18, 5, b'a'],
        vec![96, 0],
        vec![8, 255, 255, 255, 255, 255, 255, 255, 255, 255, 2],
    ] {
        assert!(Message::parse(&raw, 11).is_err());
    }
    let mut raw = Vec::new();
    number(&mut raw, 1, u64::MAX);
    assert_eq!(Message::parse(&raw, 11)?.number(1)?, u64::MAX);
    let mut input = b"DFHack!\n\x01\0\0\0".to_vec();
    for _ in 0..2 {
        let mut raw = Vec::new();
        number(&mut raw, 1, 2);
        input.extend_from_slice(&envelope(&raw, -1));
    }
    let writes = Rc::new(RefCell::new(Vec::new()));
    assert!(
        DigRpcClient::negotiate(
            Stream {
                input: Cursor::new(input),
                writes: writes.clone()
            },
            vec![b's'; 32],
            NONCE.to_vec(),
            plan()?.before().region(),
            &context()?
        )
        .is_err()
    );
    assert_eq!(methods(&writes.borrow()), [0, 0]);
    Ok(())
}

#[test]
fn notification_count_and_aggregate_bytes_are_bounded() -> Result<()> {
    for (count, size) in [(9, 1), (5, 65536)] {
        let mut input = Vec::new();
        for _ in 0..count {
            input.extend_from_slice(&envelope(&vec![0; size], -3));
        }
        let mut stream = Stream {
            input: Cursor::new(input),
            writes: Rc::new(RefCell::new(Vec::new())),
        };
        assert!(frame(&mut stream, 3, &[]).is_err());
    }
    Ok(())
}

#[test]
fn connection_byte_reservations_are_not_renewed_by_new_contexts() -> Result<()> {
    let (mut client, written) = setup(vec![reply(true, None, None)?])?;
    client.remaining_bytes = RPC_BYTES;
    client.observe(plan()?.before().region(), &context()?)?;
    let previous = written.borrow().len();
    assert!(
        client
            .observe(plan()?.before().region(), &context()?)
            .is_err()
    );
    assert_eq!(written.borrow().len(), previous);
    Ok(())
}
