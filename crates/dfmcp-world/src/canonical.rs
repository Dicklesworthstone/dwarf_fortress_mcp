pub(crate) fn put_bool(output: &mut Vec<u8>, value: bool) {
    output.push(u8::from(value));
}

pub(crate) fn put_u16(output: &mut Vec<u8>, value: u16) {
    output.extend_from_slice(&value.to_be_bytes());
}

pub(crate) fn put_u32(output: &mut Vec<u8>, value: u32) {
    output.extend_from_slice(&value.to_be_bytes());
}

pub(crate) fn put_i32(output: &mut Vec<u8>, value: i32) {
    output.extend_from_slice(&value.to_be_bytes());
}

pub(crate) fn put_u64(output: &mut Vec<u8>, value: u64) {
    output.extend_from_slice(&value.to_be_bytes());
}

pub(crate) fn put_i64(output: &mut Vec<u8>, value: i64) {
    output.extend_from_slice(&value.to_be_bytes());
}

pub(crate) fn put_bytes(output: &mut Vec<u8>, value: &[u8]) {
    put_u64(output, value.len() as u64);
    output.extend_from_slice(value);
}

pub(crate) fn put_str(output: &mut Vec<u8>, value: &str) {
    put_bytes(output, value.as_bytes());
}

pub(crate) fn put_anchor(output: &mut Vec<u8>, anchor: dfmcp_core::StateAnchor) {
    put_u64(output, anchor.fortress_id.get());
    put_u64(output, anchor.cursor.epoch);
    put_u64(output, anchor.cursor.sequence);
    put_u64(output, anchor.tick.0);
    put_bytes(output, anchor.state_hash.as_bytes());
}
