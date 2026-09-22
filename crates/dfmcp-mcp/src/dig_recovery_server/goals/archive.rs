//! Decode the existing Python excavation journal, not its printed status report.
use std::net::SocketAddr;
use dfmcp_adapter::excavation_goal::{FloorGoal, FloorProgress};
use dfmcp_adapter::live_map::LiveMapObservation;
use dfmcp_core::{Digest32, Result};
use dfmcp_world::map_region::Region;
use serde_json::{Value, json};
use super::{MAX_BYTES, denied};

pub(super) const MAX_FRAME: usize = 16_384;
const DOMAIN: &[u8] = b"dfmcp-excavation-journal/1\0";

fn object<'a>(value: &'a Value, keys: &[&str]) -> Result<&'a serde_json::Map<String, Value>> {
    let fields = value.as_object().ok_or_else(denied)?;
    if fields.len() != keys.len() || keys.iter().any(|k| !fields.contains_key(*k)) { return Err(denied()); }
    Ok(fields)
}
fn number(value: &Value) -> Result<u64> { value.as_u64().ok_or_else(denied) }
fn small(value: &Value) -> Result<u32> { u32::try_from(number(value)?).map_err(|_| denied()) }
fn string(value: &Value) -> Result<&str> { value.as_str().ok_or_else(denied) }
pub(super) fn hex(raw: &str, maximum: usize) -> Result<Vec<u8>> {
    if raw.is_empty() || raw.len() % 2 != 0 || raw.len() > maximum * 2
        || !raw.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) { return Err(denied()); }
    raw.as_bytes().chunks_exact(2).map(|pair| {
        let s = std::str::from_utf8(pair).map_err(|_| denied())?;
        u8::from_str_radix(s,16).map_err(|_| denied())
    }).collect()
}
pub(super) fn digest(raw: &str) -> Result<Digest32> {
    let bytes = hex(raw,32)?;
    Ok(Digest32::from_bytes(bytes.try_into().map_err(|_| denied())?))
}

/// Python ensure_ascii=True, sort_keys=True, separators=(',', ':'). Numbers
/// outside signed/unsigned integer encoding are not part of this journal format.
pub(super) fn canonical(value: &Value) -> Result<Vec<u8>> {
    fn quoted(text: &str, out: &mut Vec<u8>) {
        out.push(b'"');
        for c in text.chars() {
            match c {
                '"' => out.extend_from_slice(b"\\\""), '\\' => out.extend_from_slice(b"\\\\"),
                '\x08' => out.extend_from_slice(b"\\b"), '\t' => out.extend_from_slice(b"\\t"),
                '\n' => out.extend_from_slice(b"\\n"), '\x0c' => out.extend_from_slice(b"\\f"),
                '\r' => out.extend_from_slice(b"\\r"),
                ' '..='~' => out.push(c as u8),
                _ => {
                    let mut units = [0;2];
                    for unit in c.encode_utf16(&mut units) {
                        out.extend_from_slice(format!("\\u{unit:04x}").as_bytes());
                    }
                }
            }
        }
        out.push(b'"');
    }
    fn write(value: &Value, out: &mut Vec<u8>, depth: usize) -> Result<()> {
        if depth > 12 || out.len() > MAX_FRAME { return Err(denied()); }
        match value {
            Value::Null => out.extend_from_slice(b"null"),
            Value::Bool(v) => out.extend_from_slice(if *v { b"true" } else { b"false" }),
            Value::Number(v) => {
                let text = if let Some(n)=v.as_u64() { n.to_string() }
                    else if let Some(n)=v.as_i64() { n.to_string() } else { return Err(denied()); };
                out.extend_from_slice(text.as_bytes());
            }
            Value::String(s) => quoted(s,out),
            Value::Array(items) => {
                out.push(b'[');
                for (i,item) in items.iter().enumerate() {
                    if i!=0 {out.push(b',');} write(item,out,depth+1)?;
                }
                out.push(b']');
            }
            Value::Object(items) => {
                out.push(b'{');
                let mut keys=items.keys().collect::<Vec<_>>(); keys.sort_unstable();
                for (i,key) in keys.iter().enumerate() {
                    if i!=0 {out.push(b',');} quoted(key,out);out.push(b':');write(&items[*key],out,depth+1)?;
                }
                out.push(b'}');
            }
        }
        if out.len()>MAX_FRAME {return Err(denied());} Ok(())
    }
    let mut bytes=Vec::new();write(value,&mut bytes,0)?;Ok(bytes)
}
fn region(v: &Value) -> Result<Region> {
    object(v,&["origin","size"])?;
    let triple=|v:&Value|->Result<[u32;3]> {
        let a=v.as_array().filter(|a|a.len()==3).ok_or_else(denied)?;
        Ok([small(&a[0])?,small(&a[1])?,small(&a[2])?])
    };
    Ok(Region{origin:triple(&v["origin"])?,size:triple(&v["size"])?})
}
fn goal(v: &Value) -> Result<FloorGoal> {
    object(v,&["region","folder","site","deadline_tick","stable_ticks","required_samples","max_gap_ticks"])?;
    FloorGoal::new(region(&v["region"])?,string(&v["folder"])?.to_owned(),small(&v["site"])?,
        number(&v["deadline_tick"])?,number(&v["stable_ticks"])?,small(&v["required_samples"])?,number(&v["max_gap_ticks"])?)
}
fn capture(v: &Value, selected: Region) -> Result<LiveMapObservation> {
    object(v,&["manifest","capture_hex"])?;
    let manifest=&v["manifest"];object(manifest,&["generation","df_version","dfhack_version"])?;
    let bytes=hex(string(&v["capture_hex"])?,2048)?;
    let value=LiveMapObservation::decode_payload(&bytes,number(&manifest["generation"])?,
        string(&manifest["df_version"])?.to_owned(),string(&manifest["dfhack_version"])?.to_owned())?;
    if value.map.region!=selected {return Err(denied());}Ok(value)
}

pub(super) struct Archive {
    pub id: Digest32,
    pub head: Digest32,
    pub events: usize,
    pub attempts: u32,
    pub pending_read: bool,
    pub progress: FloorProgress,
}
impl Archive {
    pub fn decode(raw: &[u8], mut checkpoint: impl FnMut()->Result<()>) -> Result<Self> {
        if raw.is_empty()||raw.len()>MAX_BYTES||!raw.ends_with(b"\n") {return Err(denied());}
        let mut previous=Digest32::ZERO;
        let mut history:Option<Self>=None;
        for (sequence,line) in raw.split_inclusive(|b|*b==b'\n').enumerate() {
            checkpoint()?;
            if sequence>=260||line.len()>MAX_FRAME {return Err(denied());}
            let v:Value=serde_json::from_slice(line).map_err(|_|denied())?;
            object(&v,&["event","sequence","previous","sha256"])?;
            if number(&v["sequence"])?!=sequence as u64||string(&v["previous"])?!=previous.to_string() {return Err(denied());}
            let mut exact=canonical(&v)?;exact.push(b'\n');
            // This also rejects duplicate JSON keys, floats, raw Unicode and
            // otherwise equivalent but noncanonical strings/integers/whitespace.
            if exact!=line {return Err(denied());}
            let body=json!({"event":v["event"],"sequence":sequence,"previous":previous.to_string()});
            let mut proof=DOMAIN.to_vec();proof.extend_from_slice(&canonical(&body)?);
            let head=digest(string(&v["sha256"])?)?;
            if head!=Digest32::of_bytes(&proof) {return Err(denied());}
            let event=&v["event"];
            let kind=string(&event["kind"])?;
            if let Some(h)=&mut history {
                if h.progress.status().terminal(){return Err(denied());}
                match kind {
                    "read_started"=>{
                        object(event,&["kind"])?;
                        if h.attempts>=128 {return Err(denied());}
                        if h.pending_read {h.progress=h.progress.interrupt(false)?;}
                        h.pending_read=true;h.attempts+=1;
                    }
                    "sample"=>{
                        object(event,&["kind","sample"])?;
                        if !h.pending_read{return Err(denied());}
                        h.progress=h.progress.sample(capture(&event["sample"],h.progress.goal().region())?)?;
                        h.pending_read=false;
                    }
                    "read_failed"=>{
                        object(event,&["kind"])?;
                        if !h.pending_read{return Err(denied());}
                        h.progress=h.progress.interrupt(true)?;h.pending_read=false;
                    }
                    "cancel"=>{object(event,&["kind"])?;h.progress=h.progress.cancel()?;h.pending_read=false;}
                    _=>return Err(denied()),
                }
                h.head=head;h.events=sequence+1;
            } else {
                object(event,&["kind","format","nonce","endpoint","goal","sample"])?;
                if kind!="begin"||string(&event["format"])?!="dfmcp.excavation-goal/1"
                    ||digest(string(&event["nonce"])?)?==Digest32::ZERO {return Err(denied());}
                let address=string(&event["endpoint"])?;
                let parsed:SocketAddr=address.parse().map_err(|_|denied())?;
                if !parsed.is_ipv4()||!parsed.ip().is_loopback()||parsed.port()==0||parsed.to_string()!=address{return Err(denied());}
                let g=goal(&event["goal"])?;
                let progress=g.begin(capture(&event["sample"],g.region())?)?;
                history=Some(Self{id:head,head,events:1,attempts:0,pending_read:false,progress});
            }
            previous=head;
        }
        checkpoint()?;history.ok_or_else(denied)
    }
}
