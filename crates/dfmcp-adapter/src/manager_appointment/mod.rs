#![forbid(unsafe_code)]
//! One vacant MANAGER appointment in the unadmitted 1.22 development profile.
//! No role selector, displacement, clock advancement or production admission.
//! Beads dfmcp-reality-wiring-proof-4bf.14 and .14.2 remain independently verified.
mod capture;
pub mod journal;
mod record;
pub mod rpc;
use crate::bounded_run::{Reader, error, hash, require, validate_key};
pub use crate::order_run::FortressIdentity;
pub use capture::*;
use dfmcp_core::{Digest32, ErrorCode, Result};
pub use record::*;
pub const MAX_CAPTURE_BYTES: usize = 65536;
pub const MAX_RECORD_BYTES: usize = 2 * MAX_CAPTURE_BYTES + 1024;
pub(crate) fn u16(r: &mut Reader<'_>) -> Result<usize> {
    Ok(usize::from(u16::from_be_bytes(r.array()?)))
}
pub(crate) fn i32(r: &mut Reader<'_>) -> Result<i32> {
    Ok(r.u32()? as i32)
}
pub(crate) fn field(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(&(bytes.len() as u16).to_be_bytes());
    out.extend_from_slice(bytes);
}
pub(crate) fn blob(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
    out.extend_from_slice(bytes);
}
pub(crate) fn read_field<'a>(r: &mut Reader<'a>, max: usize) -> Result<&'a [u8]> {
    let n = u16(r)?;
    require(n <= max, "manager field bound")?;
    r.take(n)
}
pub(crate) fn read_blob<'a>(r: &mut Reader<'a>, max: usize) -> Result<&'a [u8]> {
    let n = r.u32()? as usize;
    require(n <= max, "manager blob bound")?;
    r.take(n)
}
pub(crate) fn text(r: &mut Reader<'_>, max: usize) -> Result<String> {
    let s = std::str::from_utf8(read_field(r, max)?)
        .map_err(|_| error(dfmcp_core::ErrorCode::AdapterRejected, "manager UTF-8"))?;
    require(!s.is_empty() && !s.contains('\0'), "manager text")?;
    Ok(s.to_owned())
}
pub(crate) fn put(out: &mut Vec<u8>, values: impl IntoIterator<Item = i32>) {
    for v in values {
        out.extend_from_slice(&v.to_be_bytes());
    }
}
#[cfg(test)]
mod tests;
