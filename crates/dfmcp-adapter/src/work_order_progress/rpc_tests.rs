use super::*;
use std::cell::RefCell;
use std::io::Cursor;
use std::rc::Rc;
const NONCE: &[u8] = b"nnnnnnnnnnnnnnnn";
fn fixture() -> Result<Vec<u8>> {
    let text = include_str!("../../tests/fixtures/work_order_progress_v1_12.hex");
    text.trim().as_bytes().chunks_exact(2).map(|p| {
        let text=std::str::from_utf8(p).map_err(|_|bad())?;
        u8::from_str_radix(text,16).map_err(|_|bad())
    }).collect()
}
fn reply(observation: Option<&[u8]>) -> Vec<u8> { reply_with_minor(observation, 12) }
fn reply_with_minor(observation: Option<&[u8]>, minor: u64) -> Vec<u8> {
    let mut r=Vec::new();number(&mut r,1,1);number(&mut r,2,0);bytes(&mut r,3,NONCE);
    number(&mut r,4,1);number(&mut r,5,minor);number(&mut r,6,7);
    bytes(&mut r,7,b"df");bytes(&mut r,8,b"dfhack");
    if let Some(o)=observation {bytes(&mut r,9,o);}r
}
fn framed(body:&[u8])->Vec<u8>{let mut b=header(-1,body.len() as i32).to_vec();b.extend_from_slice(body);b}
fn bootstrap(second_id:u8)->Vec<u8>{
    let mut b=b"DFHack!\n".to_vec();b.extend_from_slice(&1i32.to_le_bytes());
    b.extend_from_slice(&framed(&[8,2]));b.extend_from_slice(&framed(&[8,second_id]));
    b.extend_from_slice(&framed(&reply(None)));b
}
struct Script {input:Cursor<Vec<u8>>,output:Rc<RefCell<Vec<u8>>>,chunk:usize}
impl Script {fn new(bytes:Vec<u8>)->Self{Self{input:Cursor::new(bytes),output:Rc::new(RefCell::new(Vec::new())),chunk:1}}}
impl Read for Script {fn read(&mut self,out:&mut[u8])->io::Result<usize>{let n=out.len().min(self.chunk);self.input.read(&mut out[..n])}}
impl Write for Script {
    fn write(&mut self,bytes:&[u8])->io::Result<usize>{let n=bytes.len().min(self.chunk);self.output.borrow_mut().extend_from_slice(&bytes[..n]);Ok(n)}
    fn flush(&mut self)->io::Result<()>{Ok(())}
}
impl ProgressStream for Script {fn set_deadline(&mut self,_:Instant)->io::Result<()>{Ok(())}}
fn client(tail:Vec<u8>)->Result<(ProgressRpcClient<Script>,Rc<RefCell<Vec<u8>>>)>{
    let mut bytes=bootstrap(3);bytes.extend_from_slice(&tail);let stream=Script::new(bytes);let output=stream.output.clone();
    Ok((ProgressRpcClient::negotiate(stream,vec![b'x';32],NONCE.to_vec(),Duration::from_secs(2))?,output))
}
#[test]
fn fragmented_native_vector_round_trip_binds_only_two_methods()->Result<()> {
    let raw=fixture()?;let (mut c,output)=client(framed(&reply(Some(&raw))))?;
    let o=c.read(&[3,8],Duration::from_secs(1))?;assert_eq!(o.canonical_bytes(),raw);
    let bytes=output.borrow();let s=String::from_utf8_lossy(&bytes);
    assert!(s.contains("Handshake")&&s.contains("ReadObservation"));assert!(!s.contains("Commit"));
    assert!(!c.poisoned());Ok(())
}
#[test]
fn invalid_local_selection_does_not_write_or_fence()->Result<()> {
    let (mut c,out)=client(Vec::new())?;let n=out.borrow().len();
    for ids in [vec![],vec![3,3],vec![8,3],vec![u32::MAX]]{assert!(c.read(&ids,Duration::from_secs(1)).is_err());}
    assert_eq!(out.borrow().len(),n);assert!(!c.poisoned());Ok(())
}
#[test]
fn lost_reply_fences_and_never_implicitly_reconnects()->Result<()> {
    let (mut c,out)=client(vec![255,255])?;
    assert!(c.read(&[3,8],Duration::from_secs(1)).is_err());assert!(c.poisoned());let n=out.borrow().len();
    assert!(c.read(&[3,8],Duration::from_secs(1)).is_err());assert_eq!(out.borrow().len(),n);Ok(())
}
#[test]
fn reply_identity_shape_unknown_and_duplicate_fields_fail_closed()->Result<()> {
    let raw=fixture()?;let good=reply(Some(&raw));
    for minor in [10, 11] { assert!(decode_reply(&reply_with_minor(Some(&raw),minor),NONCE,Some(&[3,8])).is_err()); }
    assert!(decode_reply(&good,NONCE,None).is_err());assert!(decode_reply(&reply(None),NONCE,Some(&[3,8])).is_err());
    assert!(decode_reply(&good,b"wrong-nonce-value",Some(&[3,8])).is_err());
    for suffix in [vec![8,1],vec![80,1],vec![0],vec![0x80,0]]{
        let mut bad=good.clone();bad.extend_from_slice(&suffix);assert!(decode_reply(&bad,NONCE,Some(&[3,8])).is_err());
    }
    let mut altered=raw;altered[15]=8;
    let (mut c,_)=client(framed(&reply(Some(&altered))))?;
    assert!(c.read(&[3,8],Duration::from_secs(1)).is_err());assert!(c.poisoned());Ok(())
}
#[test]
fn invalid_bindings_oversized_frames_and_notifications_are_bounded()->Result<()> {
    assert!(ProgressRpcClient::negotiate(Script::new(bootstrap(2)),vec![b'x';32],NONCE.to_vec(),Duration::from_secs(1)).is_err());
    for tail in [header(-1,32769).to_vec(),header(-1,-1).to_vec(),header(-3,65537).to_vec(),header(-2,0).to_vec(),header(4,0).to_vec()]{
        let (mut c,_)=client(tail)?;assert!(c.read(&[3,8],Duration::from_secs(1)).is_err());assert!(c.poisoned());
    }
    let mut tail=Vec::new();for _ in 0..9{tail.extend_from_slice(&header(-3,0));}
    let (mut c,_)=client(tail)?;assert!(c.read(&[3,8],Duration::from_secs(1)).is_err());Ok(())
}
#[test]
fn nonminimal_varints_and_full_record_corruption_are_rejected()->Result<()> {
    for raw in [vec![0x88,0,1],vec![8,0x81,0],vec![8,255,255,255,255,255,255,255,255,255,2]]{
        assert!(Message::decode(&raw,9).is_err());
    }
    let good=fixture()?;
    for size in 0..good.len(){let (mut c,_)=client(framed(&reply(Some(&good[..size]))))?;
        assert!(c.read(&[3,8],Duration::from_secs(1)).is_err());assert!(c.poisoned());}
    Ok(())
}
