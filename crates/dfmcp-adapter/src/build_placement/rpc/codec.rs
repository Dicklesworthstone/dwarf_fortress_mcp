//! Closed protobuf envelope, independent of canonical application-record bytes.
use super::super::{Reader, Result, require};
use std::collections::BTreeMap;

pub(super) const MAX_REQUEST: usize = 2048;
pub(super) const MAX_REPLY: usize = 8192;

fn varint(out: &mut Vec<u8>, mut n: u64) {
    while n >= 128 {
        out.push((n as u8 & 127) | 128);
        n >>= 7;
    }
    out.push(n as u8);
}
pub(super) fn number(out: &mut Vec<u8>, tag: u32, n: u64) {
    varint(out, u64::from(tag) << 3);
    varint(out, n);
}
pub(super) fn bytes(out: &mut Vec<u8>, tag: u32, value: &[u8]) {
    varint(out, (u64::from(tag) << 3) | 2);
    varint(out, value.len() as u64);
    out.extend_from_slice(value);
}
fn read_varint(r: &mut Reader<'_>) -> Result<u64> {
    let mut n = 0;
    for index in 0..10 {
        let b = r.byte()?;
        require(index < 9 || b <= 1, "furniture varint overflow")?;
        n |= u64::from(b & 127) << (index * 7);
        if b < 128 {
            require(index == 0 || b != 0, "nonminimal furniture varint")?;
            return Ok(n);
        }
    }
    Err(super::malformed())
}
#[derive(Clone, Copy)]
enum Field<'a> {
    Number(u64),
    Bytes(&'a [u8]),
}
pub(super) struct Message<'a>(BTreeMap<u32, Field<'a>>);
impl<'a> Message<'a> {
    pub(super) fn parse(data: &'a [u8], maximum: u32) -> Result<Self> {
        require(
            data.len() <= MAX_REPLY && maximum <= 13,
            "oversized furniture envelope",
        )?;
        let mut r = Reader(data);
        let mut fields = BTreeMap::new();
        while !r.0.is_empty() {
            let key = read_varint(&mut r)?;
            let tag = u32::try_from(key >> 3).map_err(|_| super::malformed())?;
            require(
                tag > 0 && tag <= maximum && !fields.contains_key(&tag),
                "duplicate or unknown furniture field",
            )?;
            let value = match key & 7 {
                0 => Field::Number(read_varint(&mut r)?),
                2 => {
                    let n =
                        usize::try_from(read_varint(&mut r)?).map_err(|_| super::malformed())?;
                    Field::Bytes(r.take(n)?)
                }
                _ => return Err(super::malformed()),
            };
            fields.insert(tag, value);
        }
        Ok(Self(fields))
    }
    pub(super) fn exact(&self, fields: &[u32]) -> Result<()> {
        require(
            self.0.len() == fields.len() && fields.iter().all(|n| self.0.contains_key(n)),
            "unexpected furniture reply field set",
        )
    }
    pub(super) fn has(&self, tag: u32) -> bool {
        self.0.contains_key(&tag)
    }
    pub(super) fn number(&self, tag: u32) -> Result<u64> {
        match self.0.get(&tag) {
            Some(Field::Number(n)) => Ok(*n),
            _ => Err(super::malformed()),
        }
    }
    pub(super) fn boolean(&self, tag: u32) -> Result<bool> {
        match self.number(tag)? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(super::malformed()),
        }
    }
    pub(super) fn bytes(&self, tag: u32) -> Result<&'a [u8]> {
        match self.0.get(&tag) {
            Some(Field::Bytes(value)) => Ok(value),
            _ => Err(super::malformed()),
        }
    }
}
pub(super) fn header(id: i16, length: i32) -> [u8; 8] {
    let mut out = [0; 8];
    out[..2].copy_from_slice(&id.to_le_bytes());
    out[4..].copy_from_slice(&length.to_le_bytes());
    out
}
