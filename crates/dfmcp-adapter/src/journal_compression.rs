//! Storage representation only. Every retained observation is still decoded,
//! source-verified and published through its fixed native profile on replay.
use super::*;
use dfmcp_world::journal_delta;
use std::borrow::Cow;

pub(super) const DELTA_RECORD: &[u8; 8] = b"DFMODLT1";
const RAW_INTERVAL: u64 = 64;

/// Exact payload accounting; framing overhead remains in retained_bytes().
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct JournalStorageStats {
    pub raw_records: u64,
    pub delta_records: u64,
    pub expanded_payload_bytes: u64,
    pub stored_payload_bytes: u64,
}
impl JournalStorageStats {
    pub(super) fn include(&mut self, other: Self) {
        // Closed profiles admit <=16 MiB and <=4096 records; totals fit u64.
        self.raw_records += other.raw_records;
        self.delta_records += other.delta_records;
        self.expanded_payload_bytes += other.expanded_payload_bytes;
        self.stored_payload_bytes += other.stored_payload_bytes;
    }
}

pub(super) struct PayloadBase {
    pub digest: Digest32,
    anchor: StateAnchor,
    bytes: Vec<u8>,
}

pub(super) struct Decoded<P: JournalProfile> {
    pub entry: JournalEntry,
    pub observation: P::Observation,
    pub payload_base: Option<PayloadBase>,
    pub measurement: JournalStorageStats,
}

pub(super) fn is_record_prefix<P: JournalProfile>(prefix: &[u8]) -> bool {
    let n = prefix.len().min(8);
    prefix[..n] == RECORD[..n] || (P::DELTA_STORAGE && prefix[..n] == DELTA_RECORD[..n])
}

pub(super) fn encode<P: JournalProfile>(
    id: Digest32,
    number: u64,
    previous: Digest32,
    anchor: StateAnchor,
    observation: &P::Observation,
    base: Option<&PayloadBase>,
) -> Result<Vec<u8>> {
    // Reuse the exact legacy encoder; unchanged profiles remain byte-for-byte
    // compatible. First, reset, periodic and incompressible records stay raw.
    let mut raw = encode_profile_frame::<P>(id, number, previous, anchor, observation)?;
    let Some(base) = base.filter(|base| {
        P::DELTA_STORAGE
            && base.digest == previous
            && number % RAW_INTERVAL != 1
            && base.anchor.fortress_id == anchor.fortress_id
            && base.anchor.cursor.epoch == anchor.cursor.epoch
            && base.anchor.cursor.sequence < anchor.cursor.sequence
    }) else {
        return Ok(raw);
    };
    let end = raw.len() - 40;
    let mut body = Reader(&raw[FRAME_HEADER_BYTES..end]);
    // number, predecessor, anchor, source digest, native bridge generation.
    body.take(144)?;
    body.text()?;
    body.text()?;
    let n = body.u32()? as usize;
    let payload = body.take(n)?;
    if !body.0.is_empty() {
        return Err(corrupt("encoded journal payload has trailing data"));
    }
    let Some(delta) = journal_delta::encode(&base.bytes, payload)
        .map_err(|_| budget("journal payload delta exceeds codec bounds"))?
    else {
        return Ok(raw);
    };
    // Keep all semantic metadata verbatim. Only the payload representation and
    // its frame marker/length/checksums change; old records are never rewritten.
    raw.truncate(end - n - 4);
    raw.extend_from_slice(&(delta.len() as u32).to_be_bytes());
    raw.extend_from_slice(&delta);
    raw[..8].copy_from_slice(DELTA_RECORD);
    let body_length = (raw.len() - FRAME_HEADER_BYTES) as u32;
    raw[8..12].copy_from_slice(&body_length.to_be_bytes());
    let header_digest = frame_header_hash(id, &raw[..12]);
    raw[12..FRAME_HEADER_BYTES].copy_from_slice(header_digest.as_bytes());
    let digest = frame_hash(id, &raw);
    raw.extend_from_slice(digest.as_bytes());
    raw.extend_from_slice(FOOTER);
    Ok(raw)
}

pub(super) fn decode<P: JournalProfile>(
    frame: &[u8],
    id: Digest32,
    offset: u64,
    base: Option<&PayloadBase>,
    maximum_payload: usize,
) -> Result<Decoded<P>> {
    let mut r = Reader(frame);
    let prefix = r.take(FRAME_HEADER_BYTES)?;
    let size = decode_frame_header::<P>(prefix, id)?;
    if frame.len() != size + FRAME_HEADER_BYTES + 40 {
        return Err(corrupt("invalid journal frame length"));
    }
    let is_delta = &prefix[..8] == DELTA_RECORD;
    let mut body = Reader(r.take(size)?);
    let record_digest = r.digest()?;
    // Verify stored bytes before following a single copy command.
    if r.take(8)? != FOOTER || record_digest != frame_hash(id, &frame[..size + FRAME_HEADER_BYTES])
    {
        return Err(corrupt(
            "journal checksum or commit footer failed; no repair applied",
        ));
    }
    let number = body.u64()?;
    let previous_digest = body.digest()?;
    let anchor = StateAnchor {
        fortress_id: FortressId::new(body.u64()?),
        cursor: ObservationCursor {
            epoch: body.u64()?,
            sequence: body.u64()?,
        },
        tick: GameTick(body.u64()?),
        state_hash: body.digest()?,
    };
    let source_digest = body.digest()?;
    let generation = body.u64()?;
    let df = body.text()?;
    let dfhack = body.text()?;
    let n = body.u32()? as usize;
    let stored = body.take(n)?;
    if !body.0.is_empty() {
        return Err(corrupt("trailing journal record data"));
    }
    let maximum = maximum_payload.min(P::MAX_PAYLOAD);
    let payload = if is_delta {
        let base =
            base.ok_or_else(|| corrupt("delta record has no verified predecessor payload"))?;
        if previous_digest != base.digest
            || number % RAW_INTERVAL == 1
            || anchor.fortress_id != base.anchor.fortress_id
            || anchor.cursor.epoch != base.anchor.cursor.epoch
            || anchor.cursor.sequence <= base.anchor.cursor.sequence
        {
            return Err(corrupt(
                "delta record crosses its predecessor, keyframe or observation epoch",
            ));
        }
        let expanded = journal_delta::expanded_length(&base.bytes, stored, P::MAX_PAYLOAD)
            .map_err(|_| corrupt("invalid bounded journal payload delta"))?;
        if expanded > maximum {
            return Err(budget(
                "expanded archived observation exceeds acquisition budget",
            ));
        }
        Cow::Owned(
            journal_delta::decode(&base.bytes, stored, maximum)
                .map_err(|_| corrupt("invalid bounded journal payload delta"))?,
        )
    } else {
        if n > P::MAX_PAYLOAD {
            return Err(corrupt("journal payload exceeds its fixed profile limit"));
        }
        if n > maximum {
            return Err(budget("archived observation exceeds acquisition budget"));
        }
        Cow::Borrowed(stored)
    };
    let observation = P::decode(&payload, generation, df, dfhack)
        .map_err(|_| corrupt("journal payload is invalid for its fixed observation profile"))?;
    let measurement = JournalStorageStats {
        raw_records: u64::from(!is_delta),
        delta_records: u64::from(is_delta),
        expanded_payload_bytes: payload.len() as u64,
        stored_payload_bytes: n as u64,
    };
    let payload_base = if P::DELTA_STORAGE {
        Some(PayloadBase {
            digest: record_digest,
            anchor,
            bytes: payload.into_owned(),
        })
    } else {
        None
    };
    Ok(Decoded {
        entry: JournalEntry {
            number,
            anchor,
            source_digest,
            record_digest,
            previous_digest,
            offset,
            encoded_bytes: frame.len() as u32,
        },
        observation,
        payload_base,
        measurement,
    })
}
