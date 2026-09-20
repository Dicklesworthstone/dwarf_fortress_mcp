use super::*;
use std::io::Cursor;
use crate::workforce_control::tests::{applied, capture, context, plan, record};
struct Script { input: Cursor<Vec<u8>>, output: Vec<u8> }
impl Read for Script { fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
    let n = bytes.len().min(3); self.input.read(&mut bytes[..n])
} }
impl Write for Script {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> { let n = bytes.len().min(5); self.output.extend_from_slice(&bytes[..n]); Ok(n) }
    fn flush(&mut self) -> io::Result<()> { Ok(()) }
}
impl WorkforceStream for Script { fn narrow_deadline(&mut self, _: Duration) -> Result<()> { Ok(()) } }
fn framed(raw: &[u8]) -> Vec<u8> {
    let mut out = (-1i16).to_le_bytes().to_vec(); out.extend_from_slice(&[0, 0]);
    out.extend_from_slice(&(raw.len() as i32).to_le_bytes()); out.extend_from_slice(raw); out
}
fn reply(capture: Option<&[u8]>, effect: Option<&[u8]>) -> Vec<u8> {
    let mut out = Vec::new(); number(&mut out, 1, 1); number(&mut out, 2, 0);
    bytes(&mut out, 3, &[b'n'; 16]); number(&mut out, 4, 1); number(&mut out, 5, 17); number(&mut out, 6, 42);
    bytes(&mut out, 7, b"fake-df"); bytes(&mut out, 8, b"fake-dfhack");
    if let Some(v) = capture { bytes(&mut out, 9, v); }
    if let Some(v) = effect { bytes(&mut out, 10, v); }
    number(&mut out, 11, 0); number(&mut out, 12, u64::from(effect.is_some())); out
}
fn script(replies: Vec<Vec<u8>>, alias: bool) -> Script {
    let mut input = b"DFHack!\n\x01\0\0\0".to_vec();
    for n in 2..8 { let mut b = Vec::new(); number(&mut b, 1, if alias { 2 } else { n }); input.extend(framed(&b)); }
    input.extend(framed(&reply(None, None))); for r in replies { input.extend(framed(&r)); }
    Script { input: Cursor::new(input), output: Vec::new() }
}
#[test]
fn actual_protocol_bootstrap_fragmented_reads_and_complete_receipts() -> Result<()> {
    let ctx = context()?; let p = plan()?; let cap = capture()?;
    let prepared = record(&p, AssignmentPhase::Prepared, None)?;
    let stream = script(vec![reply(Some(cap.canonical_bytes()), None), reply(None, Some(&prepared)), reply(None, Some(&applied()?))], false);
    let mut c = WorkforceRpcClient::negotiate(stream, vec![b't'; 32], vec![b'n'; 16], &ctx)?;
    assert_eq!(c.observe(&[2, 5], &ctx)?, cap);
    assert_eq!(c.prepare(&p, &ctx)?.phase(), AssignmentPhase::Prepared);
    assert_eq!(c.commit(&p, &ctx)?.phase(), AssignmentPhase::Applied);
    assert_eq!(c.endpoint(), None); assert!(!c.fenced()); Ok(())
}
#[test]
fn absent_query_is_not_nonapplication_and_commit_failure_fences() -> Result<()> {
    let ctx = context()?; let p = plan()?;
    let mut c = WorkforceRpcClient::negotiate(script(vec![reply(None, None)], false), vec![b't';32], vec![b'n';16], &ctx)?;
    assert!(c.query(&p, &ctx)?.is_none());
    assert!(c.commit(&p, &ctx).is_err()); let sent = c.stream.output.len();
    assert!(c.fenced()); assert!(c.commit(&p, &ctx).is_err()); assert_eq!(c.stream.output.len(), sent); Ok(())
}
#[test]
fn malformed_bindings_fields_and_canonical_varints_are_rejected() -> Result<()> {
    assert!(WorkforceRpcClient::negotiate(script(vec![], true), vec![b't';32], vec![b'n';16], &context()?).is_err());
    for bytes in [&[8,1,8,1][..], &[8,128,0], &[10,3,1], &[0,1], &[8,255,255,255,255,255,255,255,255,255,2]] {
        assert!(Message::parse(bytes, 12).is_err());
    }
    let mut r = reply(None, None); number(&mut r, 12, 0);
    assert!(Message::parse(&r, 12).is_err()); Ok(())
}
#[test]
fn local_authority_and_invalid_selections_do_not_touch_socket() -> Result<()> {
    let ctx = context()?;
    let mut c = WorkforceRpcClient::negotiate(script(vec![], false), vec![b't';32], vec![b'n';16], &ctx)?;
    let sent = c.stream.output.len(); assert!(c.observe(&[5,2], &ctx).is_err());
    let mut denied = ctx; denied.grants.retain(|g| g.capability == Capability::Query);
    assert!(c.prepare(&plan()?, &denied).is_err());
    assert_eq!(c.stream.output.len(), sent); assert!(!c.fenced()); Ok(())
}
