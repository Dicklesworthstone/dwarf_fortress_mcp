use super::*;
use crate::work_orders::{WorkOrderRecipe, WorkOrderSpec};
use std::cell::RefCell;
use std::io::Cursor;
use std::rc::Rc;

const OBS: &str = include_str!("../../tests/fixtures/work_order_observation_v1_10.hex");
const PREPARED: &str = include_str!("../../tests/fixtures/work_order_prepared_v1_10.hex");
const CREATED: &str = include_str!("../../tests/fixtures/work_order_created_v1_10.hex");
const UNKNOWN: &str = include_str!("../../tests/fixtures/work_order_unknown_v1_10.hex");
const WAIT: Duration = Duration::from_secs(2);
fn hex(s: &str) -> Result<Vec<u8>> {
    let s = s.trim();
    if s.len() % 2 != 0 {
        return Err(malformed());
    }
    s.as_bytes()
        .chunks_exact(2)
        .map(|p| {
            let a = char::from(p[0]).to_digit(16).ok_or_else(malformed)?;
            let b = char::from(p[1]).to_digit(16).ok_or_else(malformed)?;
            Ok((a * 16 + b) as u8)
        })
        .collect()
}
fn plan() -> Result<WorkOrderPlan> {
    WorkOrderPlan::new(
        WorkOrderObservation::decode(&hex(OBS)?)?,
        "order-001",
        WorkOrderSpec::new(WorkOrderRecipe::WoodenBed, 5)?,
    )
}
fn prepared() -> Result<WorkOrderEffect> {
    WorkOrderEffect::decode(&hex(PREPARED)?, &plan()?)
}
fn envelope(observation: Option<&[u8]>, effect: Option<&[u8]>, replayed: Option<bool>) -> Vec<u8> {
    let mut out = Vec::new();
    number(&mut out, 1, 1);
    number(&mut out, 2, 0);
    bytes(&mut out, 3, &[9; 16]);
    number(&mut out, 4, 1);
    number(&mut out, 5, 10);
    number(&mut out, 6, 7);
    bytes(&mut out, 7, b"test-df");
    bytes(&mut out, 8, b"test-dfhack");
    if let Some(value) = observation {
        bytes(&mut out, 9, value);
    }
    if let Some(value) = effect {
        bytes(&mut out, 10, value);
    }
    if let Some(value) = replayed {
        number(&mut out, 11, u64::from(value));
    }
    out
}
fn rejection(code: u64) -> Vec<u8> {
    let mut out = Vec::new();
    number(&mut out, 1, 0);
    number(&mut out, 2, code);
    bytes(&mut out, 3, &[9; 16]);
    number(&mut out, 4, 1);
    number(&mut out, 5, 10);
    number(&mut out, 6, 0);
    bytes(&mut out, 7, b"");
    bytes(&mut out, 8, b"");
    out
}
fn frame(out: &mut Vec<u8>, data: &[u8]) {
    out.extend_from_slice(&header(-1, data.len() as i32));
    out.extend_from_slice(data);
}
fn bootstrap(ids: [u64; 5]) -> Vec<u8> {
    let mut out = b"DFHack!\n".to_vec();
    out.extend_from_slice(&1i32.to_le_bytes());
    for id in ids {
        let mut bind_reply = Vec::new();
        number(&mut bind_reply, 1, id);
        frame(&mut out, &bind_reply);
    }
    frame(&mut out, &envelope(None, None, None));
    out
}
struct Scripted {
    input: Cursor<Vec<u8>>,
    output: Rc<RefCell<Vec<u8>>>,
    fragment: usize,
    until: Option<Instant>,
    fail_write: bool,
}
impl Scripted {
    fn new(input: Vec<u8>, fragment: usize) -> Self {
        Self {
            input: Cursor::new(input),
            output: Rc::default(),
            fragment,
            until: None,
            fail_write: false,
        }
    }
    fn check(&self) -> io::Result<()> {
        if self.until.is_none_or(|until| Instant::now() >= until) {
            return Err(io::Error::new(io::ErrorKind::TimedOut, "test deadline"));
        }
        Ok(())
    }
}
impl WorkOrderStream for Scripted {
    fn set_deadline(&mut self, until: Instant) -> io::Result<()> {
        self.until = Some(until);
        self.check()
    }
}
impl Read for Scripted {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        self.check()?;
        let count = out.len().min(self.fragment);
        self.input.read(&mut out[..count])
    }
}
impl Write for Scripted {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.check()?;
        if self.fail_write {
            return Err(io::Error::other("injected write failure"));
        }
        let count = data.len().min(self.fragment);
        self.output.borrow_mut().extend_from_slice(&data[..count]);
        Ok(count)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.check()
    }
}
fn client(replies: Vec<Vec<u8>>) -> Result<WorkOrderRpcClient<Scripted>> {
    let mut input = bootstrap([2, 3, 4, 5, 6]);
    for reply in replies {
        frame(&mut input, &reply);
    }
    WorkOrderRpcClient::negotiate(Scripted::new(input, 1), vec![7; 32], vec![9; 16], WAIT)
}

#[test]
fn fragmented_fixed_method_read_prepare_commit_and_query_match_native_vectors() -> Result<()> {
    let mut c = client(vec![
        envelope(Some(&hex(OBS)?), None, None),
        envelope(None, Some(&hex(PREPARED)?), Some(false)),
        envelope(None, Some(&hex(CREATED)?), None),
        envelope(None, Some(&hex(CREATED)?), None),
    ])?;
    let obs = c.read_orders(WAIT)?;
    let p = WorkOrderPlan::new(obs, "order-001", plan()?.spec())?;
    let prepared = c.prepare(&p, WAIT)?;
    assert!(!prepared.replayed());
    let created = c.commit_prepared(&p, prepared.effect(), WAIT)?;
    assert_eq!(created.created_order_id(), Some(10));
    assert_eq!(c.query(&p, WAIT)?, Some(created));
    assert!(!c.poisoned());
    let output = c.stream.output.borrow();
    for name in METHODS {
        assert!(
            output
                .windows(name.len())
                .any(|part| part == name.as_bytes())
        );
    }
    assert!(
        !output
            .windows(b"RunCommand".len())
            .any(|part| part == b"RunCommand")
    );
    Ok(())
}

#[test]
fn missing_records_unknown_and_replayed_terminal_preparations_remain_distinct() -> Result<()> {
    let p = plan()?;
    let mut c = client(vec![
        envelope(None, None, None),
        envelope(None, Some(&hex(UNKNOWN)?), None),
        envelope(None, Some(&hex(CREATED)?), Some(true)),
    ])?;
    assert_eq!(c.query(&p, WAIT)?, None);
    assert_eq!(
        c.query(&p, WAIT)?.map(|effect| effect.state()),
        Some(WorkOrderState::Unknown)
    );
    let result = c.prepare(&p, WAIT)?;
    assert!(result.replayed());
    assert_eq!(result.effect().state(), WorkOrderState::Created);
    let sent = c.stream.output.borrow().len();
    assert!(c.commit_prepared(&p, result.effect(), WAIT).is_err());
    assert_eq!(sent, c.stream.output.borrow().len());
    Ok(())
}

#[test]
fn every_commit_io_failure_is_indeterminate_and_fences_further_dispatch() -> Result<()> {
    let p = plan()?;
    let prepared = prepared()?;
    for response in [
        None,
        Some(rejection(1)),
        Some(rejection(7)),
        Some(envelope(None, Some(&hex(PREPARED)?), None)),
    ] {
        let mut c = client(response.into_iter().collect())?;
        assert!(
            matches!(c.commit_prepared(&p, &prepared, WAIT), Err(e) if e.code == ErrorCode::EffectIndeterminate)
        );
        assert!(c.poisoned());
        let sent = c.stream.output.borrow().len();
        assert!(c.commit_prepared(&p, &prepared, WAIT).is_err());
        assert!(c.query(&p, WAIT).is_err());
        assert_eq!(c.stream.output.borrow().len(), sent);
    }
    let mut c = client(vec![])?;
    c.stream.fail_write = true;
    assert!(
        matches!(c.commit_prepared(&p, &prepared, WAIT), Err(e) if e.code == ErrorCode::EffectIndeterminate)
    );
    Ok(())
}

#[test]
fn wrong_manifest_nonce_evidence_and_reply_shape_poison_the_stream() -> Result<()> {
    let good = envelope(Some(&hex(OBS)?), None, None);
    let mut nonce = good.clone();
    nonce[6] ^= 1;
    let mut version = good.clone();
    let position = version
        .windows(7)
        .position(|p| p == b"test-df")
        .ok_or_else(malformed)?;
    version[position] = b'x';
    let mut generation = good.clone();
    let position = generation
        .windows(2)
        .position(|p| p == [48, 7])
        .ok_or_else(malformed)?;
    generation[position + 1] = 8;
    let mut bad_observation = hex(OBS)?;
    bad_observation[8..16].copy_from_slice(&8u64.to_be_bytes());
    let mut duplicate = good;
    number(&mut duplicate, 1, 1);
    for response in [
        nonce,
        version,
        generation,
        duplicate,
        envelope(None, None, None),
        envelope(Some(&bad_observation), None, None),
        envelope(Some(&hex(OBS)?), None, Some(false)),
    ] {
        let mut c = client(vec![response])?;
        assert!(c.read_orders(WAIT).is_err());
        assert!(c.poisoned());
        let sent = c.stream.output.borrow().len();
        assert!(c.read_orders(WAIT).is_err());
        assert_eq!(c.stream.output.borrow().len(), sent);
    }
    let mut forged = hex(CREATED)?;
    forged[163] ^= 1;
    let mut c = client(vec![envelope(None, Some(&forged), None)])?;
    assert!(c.query(&plan()?, WAIT).is_err());
    assert!(c.poisoned());
    Ok(())
}

#[test]
fn oversized_negative_and_excessive_notification_frames_are_bounded() -> Result<()> {
    for tail in [
        header(-1, MAX_RPC_BYTES as i32 + 1).to_vec(),
        header(-1, -1).to_vec(),
        header(-3, 65537).to_vec(),
        header(-2, 1).to_vec(),
        header(17, 0).to_vec(),
    ] {
        let mut input = bootstrap([2, 3, 4, 5, 6]);
        input.extend_from_slice(&tail);
        let mut c =
            WorkOrderRpcClient::negotiate(Scripted::new(input, 2), vec![7; 32], vec![9; 16], WAIT)?;
        assert!(c.read_orders(WAIT).is_err());
        assert!(c.poisoned());
    }
    let mut input = bootstrap([2, 3, 4, 5, 6]);
    for _ in 0..9 {
        input.extend_from_slice(&header(-3, 0));
    }
    let mut c =
        WorkOrderRpcClient::negotiate(Scripted::new(input, 4), vec![7; 32], vec![9; 16], WAIT)?;
    assert!(matches!(c.read_orders(WAIT), Err(e) if e.code == ErrorCode::BudgetExceeded));
    assert!(c.poisoned());
    Ok(())
}

#[test]
fn duplicate_or_reserved_bindings_and_malformed_protobuf_are_refused() {
    for ids in [[2, 2, 4, 5, 6], [2, 3, 4, 5, 1], [2, 3, 4, 5, 32768]] {
        assert!(
            WorkOrderRpcClient::negotiate(
                Scripted::new(bootstrap(ids), 8),
                vec![7; 32],
                vec![9; 16],
                WAIT
            )
            .is_err()
        );
    }
    for raw in [
        vec![0, 0],
        vec![8, 128, 0],
        vec![8, 1, 8, 1],
        vec![96, 1],
        vec![10, 255, 255, 255, 255, 15],
        vec![15, 0],
    ] {
        assert!(Message::parse(&raw, 11).is_err());
    }
}

#[test]
fn local_validation_never_writes_or_renews_a_connection() -> Result<()> {
    let mut c = client(vec![])?;
    let p = plan()?;
    let prepared = prepared()?;
    let sent = c.stream.output.borrow().len();
    for timeout in [Duration::ZERO, Duration::from_secs(61)] {
        assert!(c.read_orders(timeout).is_err());
        assert!(c.prepare(&p, timeout).is_err());
        assert!(c.commit_prepared(&p, &prepared, timeout).is_err());
        assert!(c.query(&p, timeout).is_err());
    }
    let mut raw = hex(OBS)?;
    raw[8..16].copy_from_slice(&8u64.to_be_bytes());
    let other = WorkOrderPlan::new(WorkOrderObservation::decode(&raw)?, "other", p.spec())?;
    assert!(c.prepare(&other, WAIT).is_err());
    assert_eq!(sent, c.stream.output.borrow().len());
    assert!(!c.poisoned());
    for (token, nonce) in [(vec![7; 31], vec![9; 16]), (vec![7; 32], vec![9; 65])] {
        let stream = Scripted::new(vec![], 1);
        let output = stream.output.clone();
        assert!(WorkOrderRpcClient::negotiate(stream, token, nonce, WAIT).is_err());
        assert!(output.borrow().is_empty());
    }
    for endpoint in [
        SocketAddr::from(([127, 0, 0, 1], 0)),
        SocketAddr::from(([192, 0, 2, 1], 5000)),
    ] {
        assert!(WorkOrderRpcClient::connect(endpoint, vec![7; 32], vec![9; 16], WAIT).is_err());
    }
    Ok(())
}

#[test]
fn large_complete_membership_and_total_notification_budget_are_enforced() -> Result<()> {
    let mut raw = hex(OBS)?[..50].to_vec();
    raw[32..36].copy_from_slice(&4096u32.to_be_bytes());
    raw.extend_from_slice(&4096u32.to_be_bytes());
    for id in 0..4096u32 {
        raw.extend_from_slice(&id.to_be_bytes());
    }
    assert!(raw.len() > 8192);
    let mut c = client(vec![envelope(Some(&raw), None, None)])?;
    let complete = c.read_orders(WAIT)?;
    assert_eq!(complete.order_ids().len(), 4096);
    assert!(!complete.eligible());
    let mut input = bootstrap([2, 3, 4, 5, 6]);
    for _ in 0..5 {
        input.extend_from_slice(&header(-3, 65536));
        input.extend_from_slice(&vec![0; 65536]);
    }
    let mut c =
        WorkOrderRpcClient::negotiate(Scripted::new(input, 4096), vec![7; 32], vec![9; 16], WAIT)?;
    assert!(matches!(c.read_orders(WAIT), Err(e) if e.code == ErrorCode::BudgetExceeded));
    assert!(c.poisoned());
    Ok(())
}
