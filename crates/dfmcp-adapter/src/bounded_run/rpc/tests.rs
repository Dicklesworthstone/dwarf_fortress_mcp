use super::*;
use super::super::tests::{context, observation, plan, raw_record};
use std::io::Cursor;
struct Script { input: Cursor<Vec<u8>>, output: Vec<u8> }
impl Read for Script { fn read(&mut self, out: &mut [u8]) -> io::Result<usize> { let n=out.len().min(7); self.input.read(&mut out[..n]) } }
impl Write for Script {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> { self.output.extend_from_slice(bytes); Ok(bytes.len()) }
    fn flush(&mut self) -> io::Result<()> { Ok(()) }
}
fn frame(payload: &[u8]) -> Vec<u8> {
    let mut out = (-1i16).to_le_bytes().to_vec(); out.extend_from_slice(&[0,0]);
    out.extend_from_slice(&(payload.len() as i32).to_le_bytes()); out.extend_from_slice(payload); out
}
fn reply(record: Option<&[u8]>, observation: Option<&[u8]>, active: bool) -> Vec<u8> {
    let mut out=Vec::new();
    for (id,n) in [(1,1),(2,0),(4,1),(5,13),(6,41),(11,u64::from(active)),(12,u64::from(record.is_some()))] { number(&mut out,id,n); }
    bytes(&mut out,3,&[b'n';32]); bytes(&mut out,7,b"fake-df"); bytes(&mut out,8,b"fake-dfhack");
    if let Some(v)=observation { bytes(&mut out,9,v); }
    if let Some(v)=record { bytes(&mut out,10,v); }
    out
}
fn script(responses: &[Vec<u8>]) -> Script {
    let mut input=b"DFHack!\n\x01\0\0\0".to_vec();
    for id in 2..8 { let mut bind=Vec::new(); number(&mut bind,1,id); input.extend(frame(&bind)); }
    input.extend(frame(&reply(None,None,false)));
    for response in responses { input.extend(frame(response)); }
    Script { input:Cursor::new(input),output:Vec::new() }
}
fn client(responses: &[Vec<u8>]) -> Result<RunRpcClient<Script>> {
    RunRpcClient::negotiate(script(responses),vec![b's';32],vec![b'n';32],&context())
}
#[test]
fn six_fixed_methods_and_fragmented_observation() -> Result<()> {
    let o=observation()?; let mut c=client(&[reply(None,Some(o.canonical_bytes()),false)])?;
    assert_eq!(c.methods,[2,3,4,5,6,7]); assert_eq!(c.observe(&context())?,o);
    for name in METHODS { assert!(c.stream.output.windows(name.len()).any(|w| w==name.as_bytes())); } Ok(())
}
#[test]
fn absent_query_remains_absent_not_a_receipt() -> Result<()> {
    let mut c=client(&[reply(None,None,false)])?;
    assert!(c.query(&plan()?,&context())?.is_none()); assert!(!c.fenced()); Ok(())
}
#[test]
fn malformed_commit_fences_and_never_replays() -> Result<()> {
    let p=plan()?; let mut invalid=raw_record(&p,1,0,true,false,Some(100)); invalid[10]^=1;
    let mut c=client(&[reply(Some(&invalid),None,true)])?;
    assert!(c.commit(&p,&context()).is_err()); assert!(c.fenced());
    let sent=c.stream.output.len(); assert!(c.commit(&p,&context()).is_err()); assert_eq!(sent,c.stream.output.len()); Ok(())
}
#[test]
fn active_record_requires_same_generation_and_owner() -> Result<()> {
    let p=plan()?; let running=raw_record(&p,1,0,true,false,Some(100));
    let mut c=client(&[reply(Some(&running),None,false)])?; assert!(c.query(&p,&context()).is_err()); Ok(())
}
#[test]
fn unauthorized_commit_does_not_touch_transport() -> Result<()> {
    let mut c=client(&[])?; let mut cx=context(); cx.grants.retain(|g|g.capability!=Capability::ControlClock);
    let sent=c.stream.output.len(); assert!(c.commit(&plan()?,&cx).is_err());
    assert_eq!(sent,c.stream.output.len()); assert!(!c.fenced()); Ok(())
}
#[test]
fn duplicate_unknown_overlong_and_boolean_fields_fail_closed() {
    for raw in [&[8,1,8,1][..], &[8,128,0][..], &[104,0][..], &[11][..]] { assert!(Message::parse(raw,12).is_err()); }
    assert!(Message::parse(&[8,2],12).and_then(|m|m.boolean(1)).is_err());
}
#[test]
fn deadline_and_endpoint_rejected_before_network() {
    let mut cx=context(); cx.budget.max_wall_millis=0;
    assert!(RunRpcClient::connect(SocketAddr::from(([127,0,0,1],1)),vec![1;32],vec![1;32],&cx).is_err());
    cx.budget.max_wall_millis=1000;
    assert!(RunRpcClient::connect(SocketAddr::from(([192,0,2,1],5000)),vec![1;32],vec![1;32],&cx).is_err());
}
