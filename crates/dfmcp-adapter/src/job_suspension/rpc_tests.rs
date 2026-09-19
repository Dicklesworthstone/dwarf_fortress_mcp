use super::*;
use std::io::Cursor;

const TIMEOUT: Duration = Duration::from_secs(5);
const OBSERVATION: &str = include_str!("../../tests/fixtures/job_suspension_observation_v1_9.hex");
const EFFECT: &str = include_str!("../../tests/fixtures/job_suspension_effect_v1_9.hex");
fn hex(text: &str) -> Result<Vec<u8>> {
    text.trim().as_bytes().chunks_exact(2).map(|pair| {
        let a = char::from(pair[0]).to_digit(16).ok_or_else(malformed)?;
        let b = char::from(pair[1]).to_digit(16).ok_or_else(malformed)?;
        Ok((a * 16 + b) as u8)
    }).collect()
}
fn plan() -> Result<SuspensionPlan> { SuspensionPlan::new(JobObservation::decode(&hex(OBSERVATION)?)?, "job-001", true) }
fn prepared() -> Result<Vec<u8>> { let mut e = hex(EFFECT)?; e[117..192].fill(0); Ok(e) }
fn frame(payload: &[u8]) -> Vec<u8> { let mut v = header(-1, payload.len() as i32).to_vec(); v.extend_from_slice(payload); v }
fn envelope(generation: u64) -> Vec<u8> {
    let mut r = Vec::new(); number(&mut r, 1, 1); number(&mut r, 2, 0); bytes(&mut r, 3, &[b'n'; 16]);
    number(&mut r, 4, 1); number(&mut r, 5, 9); number(&mut r, 6, generation);
    bytes(&mut r, 7, b"df"); bytes(&mut r, 8, b"dfhack"); r
}
fn observation_reply() -> Result<Vec<u8>> { let mut r = envelope(7); bytes(&mut r, 9, &hex(OBSERVATION)?); Ok(r) }
fn effect_reply(effect: &[u8], replayed: Option<bool>) -> Vec<u8> {
    let mut r = envelope(7); bytes(&mut r, 10, effect);
    if let Some(replayed) = replayed { number(&mut r, 11, u64::from(replayed)); } r
}
fn startup(ids: [u64; 5]) -> Vec<u8> {
    let mut v = b"DFHack!\n".to_vec(); v.extend_from_slice(&1i32.to_le_bytes());
    for id in ids { let mut r = Vec::new(); number(&mut r, 1, id); v.extend(frame(&r)); }
    v.extend(frame(&envelope(7))); v
}
struct Script { input: Cursor<Vec<u8>>, output: Vec<u8>, begins: usize, fragment: usize }
impl Read for Script {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let length = out.len().min(self.fragment); self.input.read(&mut out[..length])
    }
}
impl Write for Script {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        let length = data.len().min(self.fragment); self.output.extend_from_slice(&data[..length]); Ok(length)
    }
    fn flush(&mut self) -> io::Result<()> { Ok(()) }
}
impl JobControlStream for Script {
    fn set_deadline(&mut self, _: Instant) -> io::Result<()> { self.begins += 1; Ok(()) }
}
fn script(input: Vec<u8>, fragment: usize) -> Script { Script { input: Cursor::new(input), output: Vec::new(), begins: 0, fragment } }
fn client(extra: &[Vec<u8>]) -> Result<JobControlRpcClient<Script>> {
    let mut input = startup([2, 3, 4, 5, 6]);
    for r in extra { input.extend(frame(r)); }
    JobControlRpcClient::negotiate(script(input, 1), vec![b't'; 32], vec![b'n'; 16], TIMEOUT)
}
fn sent(stream: &Script) -> Result<Vec<(i16, Vec<u8>)>> {
    if !stream.output.starts_with(b"DFHack?\n\x01\0\0\0") { return Err(malformed()); }
    let mut offset = 12; let mut calls = Vec::new();
    while offset < stream.output.len() {
        let head = stream.output.get(offset..offset + 8).ok_or_else(malformed)?; offset += 8;
        let id = i16::from_le_bytes([head[0], head[1]]);
        let len = u32::from_le_bytes([head[4], head[5], head[6], head[7]]) as usize;
        let payload = stream.output.get(offset..offset + len).ok_or_else(malformed)?.to_vec(); offset += len;
        calls.push((id, payload));
    }
    Ok(calls)
}

#[test]
fn fragmented_full_lifecycle_binds_only_fixed_methods_and_exact_request_fields() -> Result<()> {
    let p = plan()?; let pre = prepared()?; let applied = hex(EFFECT)?;
    let mut c = client(&[observation_reply()?, effect_reply(&pre, Some(false)),
        effect_reply(&applied, None), effect_reply(&applied, None)])?;
    assert_eq!(c.read_job(3, TIMEOUT)?, *p.observation());
    let prepared = c.prepare(&p, TIMEOUT)?; assert!(!prepared.replayed());
    assert_eq!(c.commit_prepared(&p, prepared.effect(), TIMEOUT)?.state(), SuspensionState::Applied);
    assert_eq!(c.query(&p, TIMEOUT)?.map(|v| v.state()), Some(SuspensionState::Applied));
    assert!(!c.poisoned()); assert_eq!(c.stream.begins, 5);
    let calls = sent(&c.stream)?; assert_eq!(calls.len(), 10);
    for (index, name) in METHODS.iter().enumerate() {
        assert_eq!(calls[index].0, 0); let m = Message::parse(&calls[index].1, 4)?;
        assert_eq!(m.bytes(1, 128)?, name.as_bytes()); assert_eq!(m.bytes(4, 128)?, PLUGIN.as_bytes());
        assert_eq!(m.bytes(2, 128)?, REQUEST_TYPE.as_bytes()); assert_eq!(m.bytes(3, 128)?, REPLY_TYPE.as_bytes());
    }
    for (index, fields) in [(5, vec![1,2,3,4]), (6, vec![1,2,3,4,6]),
        (7, vec![1,2,3,4,5,6,7,8,9]), (8, vec![1,2,3,4,5,9,10]), (9, vec![1,2,3,4,5,9])]
    {
        let m = Message::parse(&calls[index].1, 10)?;
        assert_eq!(m.0.keys().copied().collect::<Vec<_>>(), fields);
        assert_eq!(m.number(4)?, 9); assert_eq!(m.bytes(1, 256)?, &[b't'; 32]);
    }
    assert_eq!(calls[8].0, 5); assert_eq!(calls[9].0, 6);
    Ok(())
}

#[test]
fn lost_commit_reply_fences_stream_and_recovery_queries_never_redispatch() -> Result<()> {
    let p = plan()?; let preparation = SuspensionEffect::decode(&prepared()?, &p)?;
    let mut c = client(&[])?; let before = c.stream.output.len();
    assert!(c.commit_prepared(&p, &preparation, TIMEOUT).is_err());
    assert!(c.poisoned()); let dispatched = c.stream.output.len(); assert!(dispatched > before);
    assert!(c.commit_prepared(&p, &preparation, TIMEOUT).is_err());
    assert!(c.query(&p, TIMEOUT).is_err()); assert_eq!(c.stream.output.len(), dispatched);
    let mut recovery = client(&[effect_reply(&hex(EFFECT)?, None)])?;
    assert_eq!(recovery.query(&p, TIMEOUT)?.map(|v| v.state()), Some(SuspensionState::Applied));
    assert_eq!(sent(&recovery.stream)?.last().map(|v| v.0), Some(6));
    Ok(())
}

#[test]
fn missing_native_record_and_unknown_effect_never_become_negative_success() -> Result<()> {
    let p = plan()?; let mut unknown = prepared()?; unknown[117] = 1;
    let mut c = client(&[envelope(7), effect_reply(&unknown, None)])?;
    assert!(c.query(&p, TIMEOUT)?.is_none());
    let effect = c.query(&p, TIMEOUT)?.ok_or_else(malformed)?;
    assert_eq!(effect.state(), SuspensionState::Unknown); assert!(effect.receipt().is_none());
    let before = c.stream.output.len();
    assert!(c.commit_prepared(&p, &effect, TIMEOUT).is_err());
    assert_eq!(c.stream.output.len(), before); assert!(!c.poisoned());
    Ok(())
}

#[test]
fn local_validation_errors_do_not_write_or_poison_the_connection() -> Result<()> {
    let mut c = client(&[])?; let before = c.stream.output.len();
    assert!(c.read_job(u32::MAX, TIMEOUT).is_err());
    assert!(c.read_job(3, Duration::ZERO).is_err());
    let mut raw = hex(OBSERVATION)?; raw[8..16].copy_from_slice(&8u64.to_be_bytes());
    let other = SuspensionPlan::new(JobObservation::decode(&raw)?, "job-001", true)?;
    assert!(c.prepare(&other, TIMEOUT).is_err()); assert!(c.query(&other, TIMEOUT).is_err());
    assert_eq!(c.stream.output.len(), before); assert!(!c.poisoned());
    assert!(JobControlRpcClient::connect(SocketAddr::from(([192,0,2,1], 5000)), vec![b't';32], vec![b'n';16], TIMEOUT).is_err());
    assert!(JobControlRpcClient::connect(SocketAddr::from(([127,0,0,1], 0)), vec![b't';32], vec![b'n';16], TIMEOUT).is_err());
    assert!(deadline(Duration::from_secs(61)).is_err());
    Ok(())
}

#[test]
fn reserved_and_aliasing_method_bindings_are_refused() {
    for ids in [[1,3,4,5,6], [2,3,3,5,6], [2,3,4,5,32768]] {
        assert!(JobControlRpcClient::negotiate(script(startup(ids), 2), vec![b't';32], vec![b'n';16], TIMEOUT).is_err());
    }
}

#[test]
fn malformed_method_shapes_nonce_and_generation_fence_even_after_complete_frames() -> Result<()> {
    let mut duplicate = observation_reply()?; number(&mut duplicate, 4, 1);
    let mut extra = observation_reply()?; number(&mut extra, 11, 0);
    let mut changed_generation = envelope(8); bytes(&mut changed_generation, 9, &hex(OBSERVATION)?);
    let mut wrong_job = hex(OBSERVATION)?; wrong_job[35] = 4;
    let mut wrong_job_reply = envelope(7); bytes(&mut wrong_job_reply, 9, &wrong_job);
    let mut wrong_nonce = observation_reply()?;
    let index = wrong_nonce.windows(16).position(|b| b == &[b'n';16]).ok_or_else(malformed)?;
    wrong_nonce[index] = b'x';
    for reply in [duplicate, extra, changed_generation, wrong_job_reply, wrong_nonce, envelope(7)] {
        let mut c = client(&[reply])?; assert!(c.read_job(3, TIMEOUT).is_err()); assert!(c.poisoned());
        let before = c.stream.output.len(); assert!(c.read_job(3, TIMEOUT).is_err()); assert_eq!(c.stream.output.len(), before);
    }
    Ok(())
}

#[test]
fn prepare_replay_and_commit_states_must_match_the_native_contract() -> Result<()> {
    let p = plan()?; let applied = hex(EFFECT)?; let pre = prepared()?;
    let mut c = client(&[effect_reply(&applied, Some(true))])?;
    let replay = c.prepare(&p, TIMEOUT)?; assert!(replay.replayed()); assert_eq!(replay.effect().state(), SuspensionState::Applied);
    for response in [effect_reply(&applied, Some(false)), effect_reply(&pre, None)] {
        let mut c = client(&[response])?; assert!(c.prepare(&p, TIMEOUT).is_err()); assert!(c.poisoned());
    }
    let mut c = client(&[effect_reply(&pre, None)])?;
    assert!(c.commit_prepared(&p, &SuspensionEffect::decode(&pre, &p)?, TIMEOUT).is_err()); assert!(c.poisoned());
    let mut tampered = applied; tampered[191] ^= 1;
    let mut c = client(&[effect_reply(&tampered, None)])?;
    assert!(c.query(&p, TIMEOUT).is_err()); assert!(c.poisoned());
    Ok(())
}

#[test]
fn native_error_envelope_never_asserts_an_effect_did_not_run() -> Result<()> {
    let mut rejected = Vec::new(); number(&mut rejected, 1, 0); number(&mut rejected, 2, 5);
    bytes(&mut rejected, 3, &[b'n';16]); number(&mut rejected, 4, 1); number(&mut rejected, 5, 9);
    number(&mut rejected, 6, 0); bytes(&mut rejected, 7, b""); bytes(&mut rejected, 8, b"");
    let p = plan()?; let pre = SuspensionEffect::decode(&prepared()?, &p)?;
    let mut c = client(&[rejected])?;
    let result = c.commit_prepared(&p, &pre, TIMEOUT);
    assert!(matches!(result, Err(e) if e.code == ErrorCode::AdapterFailure)); assert!(c.poisoned());
    Ok(())
}

#[test]
fn framing_and_protobuf_budgets_reject_before_unbounded_allocation() -> Result<()> {
    for payload in [&[8,1,8,1][..], &[8,128,0][..], &[8,255,255,255,255,255,255,255,255,255,2][..], &[10,255,255,255,255,15][..]] {
        assert!(Message::parse(payload, 11).is_err());
    }
    for h in [header(-1, MAX_RPC as i32 + 1), header(-3, MAX_NOTIFICATION as i32 + 1), header(-1, -1), header(3, 0)] {
        let mut input = startup([2,3,4,5,6]); input.extend_from_slice(&h);
        let mut c = JobControlRpcClient::negotiate(script(input, 3), vec![b't';32], vec![b'n';16], TIMEOUT)?;
        assert!(c.read_job(3, TIMEOUT).is_err()); assert!(c.poisoned());
    }
    let mut input = startup([2,3,4,5,6]);
    for _ in 0..9 { input.extend_from_slice(&header(-3, 0)); }
    let mut c = JobControlRpcClient::negotiate(script(input, 3), vec![b't';32], vec![b'n';16], TIMEOUT)?;
    assert!(c.read_job(3, TIMEOUT).is_err()); assert!(c.poisoned());
    Ok(())
}
