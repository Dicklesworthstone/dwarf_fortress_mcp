#![forbid(unsafe_code)]

//! Protobuf-Lite extension codec for bridge protocol 1.1 announcements.
//!
//! The existing DFHack RPC transport continues to carry one
//! `ReadObservation` request and reply. Protocol 1.1 appends request fields 8-9
//! and reply fields 20-25. Keeping this codec independent makes the additive
//! wire generation testable without weakening the audited protocol-1.0 parser.

use dfmcp_core::{DfmcpError, ErrorCode, Result};

use crate::{
    AnnouncementContinuity, AnnouncementCoverage, AnnouncementRecord, LiveAnnouncementBatch,
    MAX_ANNOUNCEMENTS_PER_BATCH, MAX_ANNOUNCEMENT_TEXT_BYTES,
};

pub const ANNOUNCEMENT_AFTER_ID_FIELD: u32 = 8;
pub const MAX_ANNOUNCEMENTS_FIELD: u32 = 9;
pub const ANNOUNCEMENT_OLDEST_AVAILABLE_ID_FIELD: u32 = 20;
pub const ANNOUNCEMENT_LATEST_AVAILABLE_ID_FIELD: u32 = 21;
pub const ANNOUNCEMENT_REQUESTED_AFTER_ID_FIELD: u32 = 22;
pub const ANNOUNCEMENT_GAP_BEFORE_WINDOW_FIELD: u32 = 23;
pub const ANNOUNCEMENT_COMPLETE_THROUGH_LATEST_FIELD: u32 = 24;
pub const ANNOUNCEMENT_RECORD_FIELD: u32 = 25;

const MAX_PROTO_PAYLOAD_BYTES: usize = 8 * 1024 * 1024;
const MAX_PROTO_FIELDS: u32 = 1_000_000;

fn error(code: ErrorCode, message: impl Into<String>) -> DfmcpError {
    DfmcpError::new(code, message)
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WireType {
    Varint = 0,
    Fixed64 = 1,
    LengthDelimited = 2,
    Fixed32 = 5,
}

impl WireType {
    fn from_u8(value: u8) -> Result<Self> {
        match value {
            0 => Ok(Self::Varint),
            1 => Ok(Self::Fixed64),
            2 => Ok(Self::LengthDelimited),
            5 => Ok(Self::Fixed32),
            _ => Err(error(
                ErrorCode::AdapterRejected,
                format!("unsupported protobuf wire type {value}"),
            )),
        }
    }
}

#[derive(Default)]
struct ProtoWriter {
    bytes: Vec<u8>,
}

impl ProtoWriter {
    fn key(&mut self, field: u32, wire: WireType) {
        self.varint(u64::from((field << 3) | u32::from(wire as u8)));
    }

    fn varint(&mut self, mut value: u64) {
        while value >= 0x80 {
            self.bytes.push((value as u8 & 0x7f) | 0x80);
            value >>= 7;
        }
        self.bytes.push(value as u8);
    }

    fn uint32(&mut self, field: u32, value: u32) {
        self.key(field, WireType::Varint);
        self.varint(u64::from(value));
    }

    fn sint32(&mut self, field: u32, value: i32) {
        self.key(field, WireType::Varint);
        let zigzag = ((value as u32) << 1) ^ ((value >> 31) as u32);
        self.varint(u64::from(zigzag));
    }

    fn boolean(&mut self, field: u32, value: bool) {
        self.key(field, WireType::Varint);
        self.varint(u64::from(value));
    }

    fn bytes(&mut self, field: u32, value: &[u8]) -> Result<()> {
        if value.len() > MAX_PROTO_PAYLOAD_BYTES {
            return Err(error(
                ErrorCode::BudgetExceeded,
                format!("protobuf field {field} exceeds the payload ceiling"),
            ));
        }
        self.key(field, WireType::LengthDelimited);
        self.varint(u64::try_from(value.len()).map_err(|_| {
            error(
                ErrorCode::BudgetExceeded,
                "protobuf field length does not fit u64",
            )
        })?);
        self.bytes.extend_from_slice(value);
        Ok(())
    }

    fn string(&mut self, field: u32, value: &str) -> Result<()> {
        self.bytes(field, value.as_bytes())
    }

    fn finish(self) -> Vec<u8> {
        self.bytes
    }
}

struct ProtoReader<'a> {
    bytes: &'a [u8],
    offset: usize,
    fields_seen: u32,
}

impl<'a> ProtoReader<'a> {
    fn new(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() > MAX_PROTO_PAYLOAD_BYTES {
            return Err(error(
                ErrorCode::BudgetExceeded,
                "protobuf payload exceeds the 8 MiB ceiling",
            ));
        }
        Ok(Self {
            bytes,
            offset: 0,
            fields_seen: 0,
        })
    }

    fn varint(&mut self) -> Result<u64> {
        let mut value = 0u64;
        for index in 0..10u32 {
            let byte = *self.bytes.get(self.offset).ok_or_else(|| {
                error(ErrorCode::AdapterRejected, "truncated protobuf varint")
            })?;
            self.offset = self.offset.saturating_add(1);
            if index == 9 && byte > 1 {
                return Err(error(
                    ErrorCode::AdapterRejected,
                    "protobuf varint overflows u64",
                ));
            }
            value |= u64::from(byte & 0x7f) << (index * 7);
            if byte & 0x80 == 0 {
                if index > 0 && byte == 0 {
                    return Err(error(
                        ErrorCode::AdapterRejected,
                        "protobuf varint is not minimally encoded",
                    ));
                }
                return Ok(value);
            }
        }
        Err(error(
            ErrorCode::AdapterRejected,
            "protobuf varint exceeds ten bytes",
        ))
    }

    fn next_key(&mut self) -> Result<Option<(u32, WireType)>> {
        if self.offset == self.bytes.len() {
            return Ok(None);
        }
        self.fields_seen = self.fields_seen.checked_add(1).ok_or_else(|| {
            error(
                ErrorCode::BudgetExceeded,
                "protobuf field counter overflowed",
            )
        })?;
        if self.fields_seen > MAX_PROTO_FIELDS {
            return Err(error(
                ErrorCode::BudgetExceeded,
                "protobuf message exceeds the field-count ceiling",
            ));
        }
        let key = self.varint()?;
        let field = u32::try_from(key >> 3).map_err(|_| {
            error(
                ErrorCode::AdapterRejected,
                "protobuf field number does not fit u32",
            )
        })?;
        if field == 0 {
            return Err(error(
                ErrorCode::AdapterRejected,
                "protobuf field zero is invalid",
            ));
        }
        Ok(Some((field, WireType::from_u8((key & 7) as u8)?)))
    }

    fn require_wire(actual: WireType, expected: WireType, field: u32) -> Result<()> {
        if actual != expected {
            return Err(error(
                ErrorCode::AdapterRejected,
                format!(
                    "protobuf field {field} uses wire type {actual:?}, expected {expected:?}"
                ),
            ));
        }
        Ok(())
    }

    fn uint64(&mut self, wire: WireType, field: u32) -> Result<u64> {
        Self::require_wire(wire, WireType::Varint, field)?;
        self.varint()
    }

    fn uint32(&mut self, wire: WireType, field: u32) -> Result<u32> {
        u32::try_from(self.uint64(wire, field)?).map_err(|_| {
            error(
                ErrorCode::AdapterRejected,
                format!("protobuf field {field} exceeds u32"),
            )
        })
    }

    fn sint32(&mut self, wire: WireType, field: u32) -> Result<i32> {
        let encoded = self.uint32(wire, field)?;
        Ok((encoded >> 1) as i32 ^ -((encoded & 1) as i32))
    }

    fn boolean(&mut self, wire: WireType, field: u32) -> Result<bool> {
        match self.uint64(wire, field)? {
            0 => Ok(false),
            1 => Ok(true),
            value => Err(error(
                ErrorCode::AdapterRejected,
                format!("protobuf bool field {field} has noncanonical value {value}"),
            )),
        }
    }

    fn length_delimited(
        &mut self,
        wire: WireType,
        field: u32,
        maximum: usize,
    ) -> Result<&'a [u8]> {
        Self::require_wire(wire, WireType::LengthDelimited, field)?;
        let length = usize::try_from(self.varint()?).map_err(|_| {
            error(
                ErrorCode::AdapterRejected,
                format!("protobuf field {field} length does not fit usize"),
            )
        })?;
        if length > maximum {
            return Err(error(
                ErrorCode::BudgetExceeded,
                format!("protobuf field {field} exceeds its {maximum}-byte bound"),
            ));
        }
        let end = self.offset.checked_add(length).ok_or_else(|| {
            error(
                ErrorCode::AdapterRejected,
                "protobuf field length overflows the input cursor",
            )
        })?;
        let value = self.bytes.get(self.offset..end).ok_or_else(|| {
            error(
                ErrorCode::AdapterRejected,
                format!("protobuf field {field} is truncated"),
            )
        })?;
        self.offset = end;
        Ok(value)
    }

    fn string(&mut self, wire: WireType, field: u32, maximum: usize) -> Result<String> {
        let bytes = self.length_delimited(wire, field, maximum)?;
        std::str::from_utf8(bytes)
            .map(str::to_owned)
            .map_err(|_| {
                error(
                    ErrorCode::AdapterRejected,
                    format!("protobuf string field {field} is not valid UTF-8"),
                )
            })
    }

    fn advance_exact(&mut self, length: usize, field: u32) -> Result<()> {
        let end = self.offset.checked_add(length).ok_or_else(|| {
            error(
                ErrorCode::AdapterRejected,
                "protobuf cursor overflow while skipping a field",
            )
        })?;
        if end > self.bytes.len() {
            return Err(error(
                ErrorCode::AdapterRejected,
                format!("protobuf field {field} is truncated"),
            ));
        }
        self.offset = end;
        Ok(())
    }

    fn skip(&mut self, wire: WireType, field: u32) -> Result<()> {
        match wire {
            WireType::Varint => {
                let _ignored = self.varint()?;
            }
            WireType::Fixed64 => self.advance_exact(8, field)?,
            WireType::LengthDelimited => {
                let length = usize::try_from(self.varint()?).map_err(|_| {
                    error(
                        ErrorCode::AdapterRejected,
                        format!("unknown protobuf field {field} length does not fit usize"),
                    )
                })?;
                if length > MAX_PROTO_PAYLOAD_BYTES {
                    return Err(error(
                        ErrorCode::BudgetExceeded,
                        format!("unknown protobuf field {field} exceeds the payload ceiling"),
                    ));
                }
                self.advance_exact(length, field)?;
            }
            WireType::Fixed32 => self.advance_exact(4, field)?,
        }
        Ok(())
    }
}

fn set_once<T>(slot: &mut Option<T>, value: T, field: &str) -> Result<()> {
    if slot.is_some() {
        return Err(error(
            ErrorCode::AdapterRejected,
            format!("protobuf field {field} appears more than once"),
        ));
    }
    *slot = Some(value);
    Ok(())
}

fn required<T>(slot: Option<T>, field: &str) -> Result<T> {
    slot.ok_or_else(|| {
        error(
            ErrorCode::AdapterRejected,
            format!("required protobuf field {field} is missing"),
        )
    })
}

fn validate_request(after_report_id: i32, maximum: u32) -> Result<()> {
    if after_report_id < -1 {
        return Err(error(
            ErrorCode::InvalidRequest,
            "announcement_after_id must be -1 or nonnegative",
        ));
    }
    let hard = u32::try_from(MAX_ANNOUNCEMENTS_PER_BATCH).map_err(|_| {
        error(
            ErrorCode::InternalInvariantViolation,
            "announcement hard limit does not fit u32",
        )
    })?;
    if maximum == 0 || maximum > hard {
        return Err(error(
            ErrorCode::InvalidRequest,
            format!("max_announcements must be in 1..={hard}"),
        ));
    }
    Ok(())
}

/// Encode only the protocol-1.1 extension fields for `ReadObservationRequest`.
/// The caller appends these bytes to the canonical protocol-1.0 request.
pub fn encode_announcement_request_fields(
    after_report_id: i32,
    maximum: u32,
) -> Result<Vec<u8>> {
    validate_request(after_report_id, maximum)?;
    let mut writer = ProtoWriter::default();
    writer.sint32(ANNOUNCEMENT_AFTER_ID_FIELD, after_report_id);
    writer.uint32(MAX_ANNOUNCEMENTS_FIELD, maximum);
    Ok(writer.finish())
}

fn decode_record(bytes: &[u8]) -> Result<AnnouncementRecord> {
    let mut reader = ProtoReader::new(bytes)?;
    let mut report_id = None;
    let mut report_type = None;
    let mut text = None;
    let mut year = None;
    let mut year_tick = None;
    let mut repeat_count = None;
    let mut continuation = None;
    let mut unconscious = None;
    let mut announcement = None;
    while let Some((field, wire)) = reader.next_key()? {
        match field {
            1 => set_once(&mut report_id, reader.sint32(wire, field)?, "report_id")?,
            2 => set_once(
                &mut report_type,
                reader.sint32(wire, field)?,
                "report_type",
            )?,
            3 => set_once(
                &mut text,
                reader.string(wire, field, MAX_ANNOUNCEMENT_TEXT_BYTES)?,
                "text",
            )?,
            4 => set_once(&mut year, reader.sint32(wire, field)?, "year")?,
            5 => set_once(
                &mut year_tick,
                reader.sint32(wire, field)?,
                "year_tick",
            )?,
            6 => set_once(
                &mut repeat_count,
                reader.sint32(wire, field)?,
                "repeat_count",
            )?,
            7 => set_once(
                &mut continuation,
                reader.boolean(wire, field)?,
                "continuation",
            )?,
            8 => set_once(
                &mut unconscious,
                reader.boolean(wire, field)?,
                "unconscious",
            )?,
            9 => set_once(
                &mut announcement,
                reader.boolean(wire, field)?,
                "announcement",
            )?,
            _ => reader.skip(wire, field)?,
        }
    }
    let record = AnnouncementRecord {
        report_id: required(report_id, "report_id")?,
        report_type: required(report_type, "report_type")?,
        text: required(text, "text")?,
        year: required(year, "year")?,
        year_tick: required(year_tick, "year_tick")?,
        repeat_count: required(repeat_count, "repeat_count")?,
        continuation: required(continuation, "continuation")?,
        unconscious: required(unconscious, "unconscious")?,
        announcement: required(announcement, "announcement")?,
    };
    record.validate()?;
    Ok(record)
}

/// Decode announcement extension fields from a complete
/// `ReadObservationReply` payload. Summary fields are supplied from the already
/// validated protocol-1.0 reply and become part of the canonical batch.
#[allow(clippy::too_many_arguments)]
pub fn decode_announcement_reply_fields(
    payload: &[u8],
    expected_after_report_id: i32,
    bridge_generation: u64,
    paused: bool,
    current_year: u32,
    current_year_tick: u32,
    site_id: i32,
) -> Result<LiveAnnouncementBatch> {
    if expected_after_report_id < -1 {
        return Err(error(
            ErrorCode::InvalidRequest,
            "expected announcement cursor must be -1 or nonnegative",
        ));
    }
    let mut reader = ProtoReader::new(payload)?;
    let mut oldest = None;
    let mut latest = None;
    let mut requested = None;
    let mut gap = None;
    let mut complete = None;
    let mut records = Vec::new();
    while let Some((field, wire)) = reader.next_key()? {
        match field {
            ANNOUNCEMENT_OLDEST_AVAILABLE_ID_FIELD => set_once(
                &mut oldest,
                reader.sint32(wire, field)?,
                "announcement_oldest_available_id",
            )?,
            ANNOUNCEMENT_LATEST_AVAILABLE_ID_FIELD => set_once(
                &mut latest,
                reader.sint32(wire, field)?,
                "announcement_latest_available_id",
            )?,
            ANNOUNCEMENT_REQUESTED_AFTER_ID_FIELD => set_once(
                &mut requested,
                reader.sint32(wire, field)?,
                "announcement_requested_after_id",
            )?,
            ANNOUNCEMENT_GAP_BEFORE_WINDOW_FIELD => set_once(
                &mut gap,
                reader.boolean(wire, field)?,
                "announcement_gap_before_window",
            )?,
            ANNOUNCEMENT_COMPLETE_THROUGH_LATEST_FIELD => set_once(
                &mut complete,
                reader.boolean(wire, field)?,
                "announcement_complete_through_latest",
            )?,
            ANNOUNCEMENT_RECORD_FIELD => {
                if records.len() >= MAX_ANNOUNCEMENTS_PER_BATCH {
                    return Err(error(
                        ErrorCode::BudgetExceeded,
                        "announcement reply exceeds the record-count ceiling",
                    ));
                }
                let nested = reader.length_delimited(wire, field, MAX_PROTO_PAYLOAD_BYTES)?;
                records.push(decode_record(nested)?);
            }
            _ => reader.skip(wire, field)?,
        }
    }
    let requested = required(requested, "announcement_requested_after_id")?;
    if requested != expected_after_report_id {
        return Err(error(
            ErrorCode::AdapterRejected,
            format!(
                "announcement reply cursor {requested} does not match requested cursor {expected_after_report_id}"
            ),
        ));
    }
    let continuity = if required(gap, "announcement_gap_before_window")? {
        AnnouncementContinuity::GapBeforeRetainedWindow
    } else {
        AnnouncementContinuity::CompleteSuffix
    };
    let returned = u32::try_from(records.len()).map_err(|_| {
        error(
            ErrorCode::BudgetExceeded,
            "announcement record count does not fit u32",
        )
    })?;
    let next_after_id = records.last().map_or(requested, |value| value.report_id);
    LiveAnnouncementBatch::new(
        bridge_generation,
        paused,
        current_year,
        current_year_tick,
        site_id,
        AnnouncementCoverage {
            requested_after_id: requested,
            oldest_available_id: required(oldest, "announcement_oldest_available_id")?,
            latest_available_id: required(latest, "announcement_latest_available_id")?,
            returned,
            complete_through_latest: required(
                complete,
                "announcement_complete_through_latest",
            )?,
            continuity,
            next_after_id,
        },
        records,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record_bytes(id: i32, text: &str) -> Result<Vec<u8>> {
        let mut writer = ProtoWriter::default();
        writer.sint32(1, id);
        writer.sint32(2, 7);
        writer.string(3, text)?;
        writer.sint32(4, 105);
        writer.sint32(5, 12_345 + id);
        writer.sint32(6, 0);
        writer.boolean(7, false);
        writer.boolean(8, false);
        writer.boolean(9, true);
        Ok(writer.finish())
    }

    fn reply(
        requested: i32,
        oldest: i32,
        latest: i32,
        ids: &[i32],
        gap: bool,
        complete: bool,
    ) -> Result<Vec<u8>> {
        let mut writer = ProtoWriter::default();
        writer.sint32(ANNOUNCEMENT_OLDEST_AVAILABLE_ID_FIELD, oldest);
        writer.sint32(ANNOUNCEMENT_LATEST_AVAILABLE_ID_FIELD, latest);
        writer.sint32(ANNOUNCEMENT_REQUESTED_AFTER_ID_FIELD, requested);
        writer.boolean(ANNOUNCEMENT_GAP_BEFORE_WINDOW_FIELD, gap);
        writer.boolean(ANNOUNCEMENT_COMPLETE_THROUGH_LATEST_FIELD, complete);
        for id in ids {
            writer.bytes(ANNOUNCEMENT_RECORD_FIELD, &record_bytes(*id, "A report")?)?;
        }
        Ok(writer.finish())
    }

    #[test]
    fn request_extension_is_canonical_and_bounded() -> Result<()> {
        assert_eq!(
            encode_announcement_request_fields(-1, 128)?,
            vec![0x40, 0x01, 0x48, 0x80, 0x01]
        );
        assert!(encode_announcement_request_fields(-2, 1).is_err());
        assert!(encode_announcement_request_fields(-1, 0).is_err());
        assert!(encode_announcement_request_fields(-1, 513).is_err());
        Ok(())
    }

    #[test]
    fn complete_suffix_decodes_to_canonical_batch() -> Result<()> {
        let payload = reply(9, 1, 11, &[10, 11], false, true)?;
        let batch = decode_announcement_reply_fields(
            &payload, 9, 42, true, 105, 12_400, 7,
        )?;
        assert_eq!(batch.coverage.next_after_id, 11);
        assert!(batch.coverage.complete_through_latest);
        assert!(!batch.coverage.has_gap());
        assert_eq!(batch.announcements.len(), 2);
        batch.validate()
    }

    #[test]
    fn retained_window_gap_is_preserved() -> Result<()> {
        let payload = reply(1, 10, 11, &[10, 11], true, true)?;
        let batch = decode_announcement_reply_fields(
            &payload, 1, 42, true, 105, 12_400, 7,
        )?;
        assert!(batch.coverage.has_gap());
        Ok(())
    }

    #[test]
    fn cursor_mismatch_is_rejected() -> Result<()> {
        let payload = reply(9, 1, 10, &[10], false, true)?;
        assert!(decode_announcement_reply_fields(
            &payload, 8, 42, true, 105, 12_400, 7,
        )
        .is_err());
        Ok(())
    }

    #[test]
    fn duplicate_required_extension_field_is_rejected() -> Result<()> {
        let mut payload = reply(9, 1, 10, &[10], false, true)?;
        let mut duplicate = ProtoWriter::default();
        duplicate.sint32(ANNOUNCEMENT_REQUESTED_AFTER_ID_FIELD, 9);
        payload.extend_from_slice(&duplicate.finish());
        assert!(decode_announcement_reply_fields(
            &payload, 9, 42, true, 105, 12_400, 7,
        )
        .is_err());
        Ok(())
    }

    #[test]
    fn noncanonical_boolean_is_rejected() -> Result<()> {
        let mut payload = ProtoWriter::default();
        payload.sint32(ANNOUNCEMENT_OLDEST_AVAILABLE_ID_FIELD, -1);
        payload.sint32(ANNOUNCEMENT_LATEST_AVAILABLE_ID_FIELD, -1);
        payload.sint32(ANNOUNCEMENT_REQUESTED_AFTER_ID_FIELD, -1);
        payload.key(ANNOUNCEMENT_GAP_BEFORE_WINDOW_FIELD, WireType::Varint);
        payload.varint(2);
        payload.boolean(ANNOUNCEMENT_COMPLETE_THROUGH_LATEST_FIELD, true);
        assert!(decode_announcement_reply_fields(
            &payload.finish(), -1, 42, false, 105, 12_400, 7,
        )
        .is_err());
        Ok(())
    }

    #[test]
    fn oversized_text_is_rejected_before_allocation_growth() -> Result<()> {
        let text = "x".repeat(MAX_ANNOUNCEMENT_TEXT_BYTES + 1);
        let mut payload = ProtoWriter::default();
        payload.sint32(ANNOUNCEMENT_OLDEST_AVAILABLE_ID_FIELD, 10);
        payload.sint32(ANNOUNCEMENT_LATEST_AVAILABLE_ID_FIELD, 10);
        payload.sint32(ANNOUNCEMENT_REQUESTED_AFTER_ID_FIELD, 9);
        payload.boolean(ANNOUNCEMENT_GAP_BEFORE_WINDOW_FIELD, false);
        payload.boolean(ANNOUNCEMENT_COMPLETE_THROUGH_LATEST_FIELD, true);
        payload.bytes(ANNOUNCEMENT_RECORD_FIELD, &record_bytes(10, &text)?)?;
        assert!(decode_announcement_reply_fields(
            &payload.finish(), 9, 42, true, 105, 12_400, 7,
        )
        .is_err());
        Ok(())
    }

    #[test]
    fn unknown_fields_are_skipped_without_changing_identity() -> Result<()> {
        let baseline = reply(9, 1, 10, &[10], false, true)?;
        let mut extended = baseline.clone();
        let mut unknown = ProtoWriter::default();
        unknown.uint32(99, 7);
        extended.extend_from_slice(&unknown.finish());
        let first = decode_announcement_reply_fields(
            &baseline, 9, 42, true, 105, 12_400, 7,
        )?;
        let second = decode_announcement_reply_fields(
            &extended, 9, 42, true, 105, 12_400, 7,
        )?;
        assert_eq!(first.content_digest, second.content_digest);
        Ok(())
    }
}
