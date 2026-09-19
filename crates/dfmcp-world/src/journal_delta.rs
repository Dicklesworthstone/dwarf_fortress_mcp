#![forbid(unsafe_code)]

//! Bounded byte-exact payload deltas, independent of observation semantics.
//!
//! The containing journal MUST authenticate the frame, bind its predecessor to
//! the supplied base, and validate the reconstructed observation/source digest.
//! This codec alone is not an integrity, authority, or migration boundary.

const MAGIC: &[u8; 8] = b"DFMDLT01";
const HEADER: usize = 20;
const MIN_COPY: usize = 32;
/// Delta records are used only when they save at least this many payload bytes.
pub const MIN_SAVINGS: usize = 128;
pub const MAX_PAYLOAD: usize = 16 * 1024 * 1024;
pub const MAX_COMMANDS: usize = 32_768;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    LimitExceeded,
    InvalidEncoding,
    WrongBaseLength,
}

struct Encoder {
    bytes: Vec<u8>,
    commands: usize,
    maximum: usize,
}
impl Encoder {
    fn room(&self, amount: usize) -> bool {
        self.commands < MAX_COMMANDS
            && amount <= self.maximum.saturating_sub(self.bytes.len())
    }
    fn literal(&mut self, value: &[u8]) -> bool {
        if value.is_empty() { return true; }
        if !self.room(5 + value.len()) { return false; }
        self.bytes.push(0);
        self.bytes.extend_from_slice(&(value.len() as u32).to_be_bytes());
        self.bytes.extend_from_slice(value);
        self.commands += 1;
        true
    }
    fn copy(&mut self, offset: usize, length: usize) -> bool {
        if length == 0 { return true; }
        if !self.room(9) { return false; }
        self.bytes.push(1);
        self.bytes.extend_from_slice(&(offset as u32).to_be_bytes());
        self.bytes.extend_from_slice(&(length as u32).to_be_bytes());
        self.commands += 1;
        true
    }
}

/// Deterministic linear scan: aligned unchanged runs plus a possibly shifted
/// common suffix. Insertions/deletions near the front do not force copying the
/// whole suffix. Incompressible or fragmented input falls back to the raw format.
/// `None` is a deliberate raw-storage decision, never a partial delta.
pub fn encode(base: &[u8], target: &[u8]) -> Result<Option<Vec<u8>>, Error> {
    if base.len() > MAX_PAYLOAD || target.len() > MAX_PAYLOAD {
        return Err(Error::LimitExceeded);
    }
    let Some(maximum) = target.len().checked_sub(MIN_SAVINGS) else { return Ok(None); };
    if maximum < HEADER + 9 { return Ok(None); }
    let mut bytes = Vec::with_capacity((HEADER + 256).min(maximum));
    bytes.extend_from_slice(MAGIC);
    bytes.extend_from_slice(&(base.len() as u32).to_be_bytes());
    bytes.extend_from_slice(&(target.len() as u32).to_be_bytes());
    bytes.extend_from_slice(&0u32.to_be_bytes());
    let mut encoder = Encoder { bytes, commands: 0, maximum };
    let suffix = base.iter().rev().zip(target.iter().rev())
        .take_while(|(a, b)| a == b).count();
    let suffix = if suffix >= MIN_COPY { suffix } else { 0 };
    let end = target.len() - suffix;
    let mut position = 0usize;
    let mut literal_start = 0usize;
    while position < end {
        let run_start = position;
        while position < end && position < base.len() && target[position] == base[position] {
            position += 1;
        }
        if position - run_start >= MIN_COPY {
            if !encoder.literal(&target[literal_start..run_start])
                || !encoder.copy(run_start, position - run_start) { return Ok(None); }
            literal_start = position;
        }
        if position < end { position += 1; }
    }
    if !encoder.literal(&target[literal_start..end])
        || !encoder.copy(base.len() - suffix, suffix) { return Ok(None); }
    encoder.bytes[16..20].copy_from_slice(&(encoder.commands as u32).to_be_bytes());
    Ok(Some(encoder.bytes))
}

struct Reader<'a> { bytes: &'a [u8] }
impl<'a> Reader<'a> {
    fn take(&mut self, length: usize) -> Result<&'a [u8], Error> {
        let value = self.bytes.get(..length).ok_or(Error::InvalidEncoding)?;
        self.bytes = &self.bytes[length..];
        Ok(value)
    }
    fn u32(&mut self) -> Result<usize, Error> {
        let bytes = self.take(4)?.try_into().map_err(|_| Error::InvalidEncoding)?;
        usize::try_from(u32::from_be_bytes(bytes)).map_err(|_| Error::LimitExceeded)
    }
}

/// Read-only preflight. It walks every command and checks all input/output ranges
/// before the decoder allocates the declared output. No length is trusted merely
/// because it came from a checksummed outer journal frame.
fn validate<'a>(base: &[u8], encoded: &'a [u8], maximum: usize)
    -> Result<(usize, usize, &'a [u8]), Error> {
    if maximum > MAX_PAYLOAD || base.len() > MAX_PAYLOAD || encoded.len() > MAX_PAYLOAD {
        return Err(Error::LimitExceeded);
    }
    let mut reader = Reader { bytes: encoded };
    if reader.take(8)? != MAGIC { return Err(Error::InvalidEncoding); }
    if reader.u32()? != base.len() { return Err(Error::WrongBaseLength); }
    let length = reader.u32()?;
    if length > maximum { return Err(Error::LimitExceeded); }
    let commands = reader.u32()?;
    if commands == 0 || commands > MAX_COMMANDS { return Err(Error::InvalidEncoding); }
    let body = reader.bytes;
    let mut produced = 0usize;
    for _ in 0..commands {
        let tag = reader.take(1)?[0];
        let amount = match tag {
            0 => {
                let n = reader.u32()?;
                reader.take(n)?;
                n
            }
            1 => {
                let offset = reader.u32()?;
                let n = reader.u32()?;
                if offset > base.len() || n > base.len() - offset { return Err(Error::InvalidEncoding); }
                n
            }
            _ => return Err(Error::InvalidEncoding),
        };
        if amount == 0 || amount > length.saturating_sub(produced) { return Err(Error::InvalidEncoding); }
        produced += amount;
    }
    if produced != length || !reader.bytes.is_empty() { return Err(Error::InvalidEncoding); }
    Ok((length, commands, body))
}

/// Reconstruct an exact payload under a separate *expanded* byte allowance.
/// Compression never widens the acquisition budget. Output does not escape on
/// malformed input; the caller must still validate its semantic/native codec.
pub fn decode(base: &[u8], encoded: &[u8], maximum: usize) -> Result<Vec<u8>, Error> {
    let (length, commands, body) = validate(base, encoded, maximum)?;
    let mut output = Vec::with_capacity(length);
    let mut reader = Reader { bytes: body };
    for _ in 0..commands {
        match reader.take(1)?[0] {
            0 => {
                let n = reader.u32()?;
                output.extend_from_slice(reader.take(n)?);
            }
            1 => {
                let offset = reader.u32()?;
                let n = reader.u32()?;
                output.extend_from_slice(base.get(offset..offset + n).ok_or(Error::InvalidEncoding)?);
            }
            _ => return Err(Error::InvalidEncoding),
        }
    }
    Ok(output)
}

#[cfg(test)]
#[path = "journal_delta_tests.rs"]
mod tests;
