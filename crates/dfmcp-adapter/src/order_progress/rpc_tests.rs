use super::*;
use crate::order_progress::tests::{fixture, sample};
use std::io::Cursor;
struct Stream {
    input: Cursor<Vec<u8>>,
    output: Vec<u8>,
    fragment: usize,
}
impl Read for Stream {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let n = out.len().min(self.fragment);
        self.input.read(&mut out[..n])
    }
}
impl Write for Stream {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        let n = data.len().min(self.fragment);
        self.output.extend_from_slice(&data[..n]);
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
impl ProgressStream for Stream {
    fn set_deadline(&mut self, _: Instant) -> io::Result<()> {
        Ok(())
    }
}
fn frame(payload: &[u8]) -> Vec<u8> {
    let mut out = header(-1, payload.len() as i32).to_vec();
    out.extend_from_slice(payload);
    out
}
fn reply(raw: Option<&[u8]>) -> Vec<u8> {
    let mut out = Vec::new();
    number(&mut out, 1, 1);
    number(&mut out, 2, 0);
    bytes(&mut out, 3, &[b'n'; 16]);
    number(&mut out, 4, 1);
    number(&mut out, 5, 11);
    number(&mut out, 6, 7);
    bytes(&mut out, 7, b"test-df");
    bytes(&mut out, 8, b"test-dfhack");
    if let Some(raw) = raw {
        bytes(&mut out, 9, raw);
    }
    out
}
fn input() -> Vec<u8> {
    let mut out = b"DFHack!\n".to_vec();
    out.extend_from_slice(&1i32.to_le_bytes());
    for id in [2, 3] {
        let mut p = Vec::new();
        number(&mut p, 1, id);
        out.extend(frame(&p));
    }
    out.extend(frame(&reply(None)));
    out
}
fn client(tail: Vec<u8>, fragment: usize) -> ProgressClient<Stream> {
    let mut data = input();
    data.extend(tail);
    ProgressClient::negotiate(
        Stream {
            input: Cursor::new(data),
            output: Vec::new(),
            fragment,
        },
        vec![b's'; 32],
        vec![b'n'; 16],
        Duration::from_secs(1),
    )
    .unwrap()
}
#[test]
fn fixed_binding_fragmentation_and_native_vectors() {
    for fragment in 1..=7 {
        let mut c = client(frame(&reply(Some(&fixture("active")))), fragment);
        let o = c.read_order(10, Duration::from_secs(1)).unwrap();
        assert_eq!(o.classification(), "active");
        assert!(String::from_utf8_lossy(&c.stream.output).contains("ReadOrderProgress"));
        assert!(!String::from_utf8_lossy(&c.stream.output).contains("CommitOrder"));
    }
}
#[test]
fn every_failed_wire_or_evidence_read_fences_the_connection() {
    for tail in [
        Vec::new(),
        header(-2, 4).to_vec(),
        header(-1, 5000).to_vec(),
        frame(&reply(None)),
        frame(&reply(Some(b"bad"))),
    ] {
        let mut c = client(tail, 2);
        assert!(c.read_order(10, Duration::from_secs(1)).is_err());
        assert!(c.poisoned());
        let written = c.stream.output.len();
        assert!(c.read_order(10, Duration::from_secs(1)).is_err());
        assert_eq!(c.stream.output.len(), written);
    }
}
#[test]
fn duplicate_fields_wrong_nonce_and_nonminimal_varints_fail_closed() {
    let mut duplicate = reply(Some(&fixture("active")));
    number(&mut duplicate, 1, 1);
    let mut nonce = reply(Some(&fixture("active")));
    let at = nonce.windows(16).position(|w| w == [b'n'; 16]).unwrap();
    nonce[at] = b'x';
    let mut unknown = reply(Some(&fixture("active")));
    number(&mut unknown, 10, 1);
    for payload in [
        duplicate,
        nonce,
        unknown,
        vec![0x88, 0, 1],
        vec![8, 0x81, 0],
    ] {
        let mut c = client(frame(&payload), 1);
        assert!(c.read_order(10, Duration::from_secs(1)).is_err());
        assert!(c.poisoned());
    }
}
#[test]
fn replayed_sample_and_identity_changes_do_not_publish_observations() {
    let raw = fixture("active");
    let mut tail = frame(&reply(Some(&raw)));
    tail.extend(frame(&reply(Some(&raw))));
    let mut c = client(tail, 3);
    c.read_order(10, Duration::from_secs(1)).unwrap();
    assert!(c.read_order(10, Duration::from_secs(1)).is_err());
    let mut wrong = sample(1, 10, 5).canonical_bytes().to_vec();
    wrong[15] = 8;
    let mut c = client(frame(&reply(Some(&wrong))), 3);
    assert!(c.read_order(10, Duration::from_secs(1)).is_err());
}
#[test]
fn local_invalid_arguments_write_nothing_and_notifications_are_bounded() {
    let mut c = client(Vec::new(), 2);
    let written = c.stream.output.len();
    assert!(c.read_order(u32::MAX, Duration::from_secs(1)).is_err());
    assert!(c.read_order(10, Duration::ZERO).is_err());
    assert_eq!(c.stream.output.len(), written);
    assert!(!c.poisoned());
    let mut tail = Vec::new();
    for _ in 0..9 {
        tail.extend_from_slice(&header(-3, 0));
    }
    let mut c = client(tail, 3);
    assert!(c.read_order(10, Duration::from_secs(1)).is_err());
    assert!(c.poisoned());
    let mut tail = Vec::new();
    for _ in 0..8 {
        tail.extend_from_slice(&header(-3, 0));
    }
    tail.extend(frame(&reply(Some(&fixture("active")))));
    let mut c = client(tail, 3);
    assert!(c.read_order(10, Duration::from_secs(1)).is_ok());
}
