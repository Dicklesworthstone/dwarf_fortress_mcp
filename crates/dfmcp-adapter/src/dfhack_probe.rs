#![forbid(unsafe_code)]

//! Acceptance-only client for exercising rejected DFHack bridge requests.
//!
//! The production [`crate::DfHackRpcClient`] correctly refuses malformed
//! credentials and impossible bounds before writing them. R2/R3 qualification
//! nevertheless has to prove that the native plugin also rejects those inputs.
//! This module therefore exposes a deliberately narrow raw-request laboratory:
//! callers may vary protocol, token, nonce, offset, and page-size fields within
//! a second, explicit transport ceiling. It still uses DFHack's supported
//! remote protocol, performs no mutation, admits no remote endpoint, and treats
//! every reply as hostile protobuf input.

#[cfg(not(target_endian = "little"))]
compile_error!("the admitted DFHack native RPC wire currently requires a little-endian target");

use std::collections::BTreeSet;
use std::fmt;
use std::io::{Read, Write};

use dfmcp_core::{DfmcpError, ErrorCode, Result};

use crate::{
    BRIDGE_PROTOCOL_MAJOR, BRIDGE_PROTOCOL_MINOR, CitizenRecord, DFHACK_RPC_VERSION,
    MAX_CITIZENS_PER_PAGE, MAX_RACE_NAME_BYTES, MAX_RPC_PAYLOAD_BYTES,
    MAX_TEXT_NOTIFICATIONS_PER_CALL, MAX_TEXT_NOTIFICATION_TOTAL_BYTES,
    MAX_UNIT_NAME_BYTES, MAX_WORLD_FOLDER_BYTES, MAX_WORLD_NAME_BYTES,
};

pub const MAX_PROBE_FIELD_BYTES: usize = 4_096;
pub const MAX_PROBE_METHODS: usize = 32;
pub const MAX_PROBE_TEXT_NOTIFICATION_BYTES: usize = 64 * 1024;

const REQUEST_MAGIC: &[u8; 8] = b"DFHack?\n";
const RESPONSE_MAGIC: &[u8; 8] = b"DFHack!\n";
const HANDSHAKE_HEADER_BYTES: usize = 12;
const MESSAGE_HEADER_BYTES: usize = 8;
const RPC_REPLY_RESULT: i16 = -1;
const RPC_REPLY_FAIL: i16 = -2;
const RPC_REPLY_TEXT: i16 = -3;
const RPC_REQUEST_QUIT: i16 = -4;
const BIND_METHOD_ID: i16 = 0;
const FIRST_PLUGIN_METHOD_ID: i16 = 2;
const PLUGIN_NAME: &str = "dfmcp_bridge";
const HANDSHAKE_METHOD: &str = "Handshake";
const OBSERVATION_METHOD: &str = "ReadObservation";
const HANDSHAKE_INPUT_TYPE: &str = "dfmcp.bridge.v1.HandshakeRequest";
const HANDSHAKE_OUTPUT_TYPE: &str = "dfmcp.bridge.v1.HandshakeReply";
const OBSERVATION_INPUT_TYPE: &str = "dfmcp.bridge.v1.ReadObservationRequest";
const OBSERVATION_OUTPUT_TYPE: &str = "dfmcp.bridge.v1.ReadObservationReply";
const TICKS_PER_YEAR: u32 = 403_200;
const MAX_PROTO_FIELDS: u32 = 1_000_000;

fn error(code: ErrorCode, message: impl Into<String>) -> DfmcpError {
    DfmcpError::new(code, message)
}

fn io_error(operation: &str, source: &std::io::Error) -> DfmcpError {
    error(
        ErrorCode::AdapterUnavailable,
        format!("DFHack acceptance probe {operation} failed: {source}"),
    )
    .retryable(true)
}

fn validate_probe_field(value: &[u8], field: &str) -> Result<()> {
    if value.len() > MAX_PROBE_FIELD_BYTES {
        return Err(error(
            ErrorCode::BudgetExceeded,
            format!(
                "acceptance probe {field} exceeds the {MAX_PROBE_FIELD_BYTES}-byte transport ceiling"
            ),
        ));
    }
    Ok(())
}

#[derive(Clone, PartialEq, Eq)]
pub struct ProbeHandshakeRequest {
    pub protocol_major: u32,
    pub protocol_minor: u32,
    pub client_name: String,
    pub client_version: String,
    pub client_nonce: Vec<u8>,
    pub bearer_token: Vec<u8>,
}

impl fmt::Debug for ProbeHandshakeRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProbeHandshakeRequest")
            .field("protocol_major", &self.protocol_major)
            .field("protocol_minor", &self.protocol_minor)
            .field("client_name", &self.client_name)
            .field("client_version", &self.client_version)
            .field("client_nonce_bytes", &self.client_nonce.len())
            .field("bearer_token", &"<redacted>")
            .field("bearer_token_bytes", &self.bearer_token.len())
            .finish()
    }
}

impl ProbeHandshakeRequest {
    pub fn validate_transport_bounds(&self) -> Result<()> {
        validate_probe_field(self.client_name.as_bytes(), "client name")?;
        validate_probe_field(self.client_version.as_bytes(), "client version")?;
        validate_probe_field(&self.client_nonce, "client nonce")?;
        validate_probe_field(&self.bearer_token, "bearer token")
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProbeHandshakeReply {
    pub accepted: bool,
    pub failure_code: String,
    pub failure_message: String,
    pub protocol_major: u32,
    pub protocol_minor: u32,
    pub bridge_version: String,
    pub dfhack_version: String,
    pub df_version: String,
    pub world_loaded: bool,
    pub fortress_mode: bool,
    pub client_nonce: Vec<u8>,
    pub bridge_generation: u64,
    pub supported_methods: BTreeSet<String>,
}

impl ProbeHandshakeReply {
    #[must_use]
    pub fn nonce_correlated(&self, expected: &[u8]) -> bool {
        self.client_nonce == expected
    }

    #[must_use]
    pub fn sensitive_manifest_disclosed(&self) -> bool {
        !self.bridge_version.is_empty()
            || !self.dfhack_version.is_empty()
            || !self.df_version.is_empty()
            || self.world_loaded
            || self.fortress_mode
            || self.bridge_generation != 0
            || !self.supported_methods.is_empty()
    }

    fn validate_shape(&self) -> Result<()> {
        validate_probe_field(self.failure_code.as_bytes(), "failure code")?;
        validate_probe_field(self.failure_message.as_bytes(), "failure message")?;
        validate_probe_field(self.bridge_version.as_bytes(), "bridge version")?;
        validate_probe_field(self.dfhack_version.as_bytes(), "DFHack version")?;
        validate_probe_field(self.df_version.as_bytes(), "Dwarf Fortress version")?;
        validate_probe_field(&self.client_nonce, "echoed client nonce")?;
        if self.supported_methods.len() > MAX_PROBE_METHODS {
            return Err(error(
                ErrorCode::BudgetExceeded,
                "handshake reply exceeds the supported-method bound",
            ));
        }
        if self.accepted {
            if !self.failure_code.is_empty() || !self.failure_message.is_empty() {
                return Err(error(
                    ErrorCode::AdapterRejected,
                    "accepted probe handshake carries failure details",
                ));
            }
            if self.protocol_major != BRIDGE_PROTOCOL_MAJOR
                || self.protocol_minor != BRIDGE_PROTOCOL_MINOR
                || self.bridge_generation == 0
                || self.bridge_version.is_empty()
                || self.dfhack_version.is_empty()
                || self.df_version.is_empty()
            {
                return Err(error(
                    ErrorCode::AdapterRejected,
                    "accepted probe handshake has an incomplete compatibility manifest",
                ));
            }
            let expected = BTreeSet::from([
                HANDSHAKE_METHOD.to_owned(),
                OBSERVATION_METHOD.to_owned(),
            ]);
            if self.supported_methods != expected {
                return Err(error(
                    ErrorCode::VersionMismatch,
                    "accepted probe handshake returned the wrong method set",
                ));
            }
        } else if self.failure_code.is_empty() {
            return Err(error(
                ErrorCode::AdapterRejected,
                "rejected probe handshake omitted its failure code",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct ProbeObservationRequest {
    pub protocol_major: u32,
    pub protocol_minor: u32,
    pub client_nonce: Vec<u8>,
    pub bearer_token: Vec<u8>,
    pub citizen_offset: u32,
    pub max_citizens: u32,
    pub include_names: bool,
}

impl fmt::Debug for ProbeObservationRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProbeObservationRequest")
            .field("protocol_major", &self.protocol_major)
            .field("protocol_minor", &self.protocol_minor)
            .field("client_nonce_bytes", &self.client_nonce.len())
            .field("bearer_token", &"<redacted>")
            .field("bearer_token_bytes", &self.bearer_token.len())
            .field("citizen_offset", &self.citizen_offset)
            .field("max_citizens", &self.max_citizens)
            .field("include_names", &self.include_names)
            .finish()
    }
}

impl ProbeObservationRequest {
    pub fn validate_transport_bounds(&self) -> Result<()> {
        validate_probe_field(&self.client_nonce, "client nonce")?;
        validate_probe_field(&self.bearer_token, "bearer token")
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProbeObservationReply {
    pub accepted: bool,
    pub failure_code: String,
    pub failure_message: String,
    pub protocol_major: u32,
    pub protocol_minor: u32,
    pub client_nonce: Vec<u8>,
    pub bridge_generation: u64,
    pub world_loaded: bool,
    pub fortress_mode: bool,
    pub paused: bool,
    pub current_year: u32,
    pub current_year_tick: u32,
    pub world_name: String,
    pub world_folder: String,
    pub site_id: i32,
    pub citizen_count_total: u32,
    pub citizen_offset: u32,
    pub complete: bool,
    pub citizens: Vec<CitizenRecord>,
}

impl ProbeObservationReply {
    #[must_use]
    pub fn nonce_correlated(&self, expected: &[u8]) -> bool {
        self.client_nonce == expected
    }

    #[must_use]
    pub fn world_posture_disclosed(&self) -> bool {
        self.world_loaded
            || self.fortress_mode
            || self.paused
            || self.current_year != 0
            || self.current_year_tick != 0
            || !self.world_name.is_empty()
            || !self.world_folder.is_empty()
            || self.site_id >= 0
            || self.citizen_count_total != 0
            || self.citizen_offset != 0
            || self.complete
            || !self.citizens.is_empty()
    }

    fn validate_shape(&self, request: &ProbeObservationRequest) -> Result<()> {
        validate_probe_field(self.failure_code.as_bytes(), "failure code")?;
        validate_probe_field(self.failure_message.as_bytes(), "failure message")?;
        validate_probe_field(&self.client_nonce, "echoed client nonce")?;
        if self.citizens.len() > usize::try_from(MAX_CITIZENS_PER_PAGE).map_err(|_| {
            error(
                ErrorCode::InternalInvariantViolation,
                "citizen page ceiling does not fit usize",
            )
        })? {
            return Err(error(
                ErrorCode::BudgetExceeded,
                "probe observation reply exceeds the citizen-page ceiling",
            ));
        }
        if self.accepted {
            if !self.failure_code.is_empty() || !self.failure_message.is_empty() {
                return Err(error(
                    ErrorCode::AdapterRejected,
                    "accepted probe observation carries failure details",
                ));
            }
            if self.protocol_major != BRIDGE_PROTOCOL_MAJOR
                || self.protocol_minor != BRIDGE_PROTOCOL_MINOR
                || self.bridge_generation == 0
                || !self.world_loaded
                || !self.fortress_mode
                || self.site_id < 0
                || self.current_year_tick >= TICKS_PER_YEAR
            {
                return Err(error(
                    ErrorCode::AdapterRejected,
                    "accepted probe observation has invalid live-world posture",
                ));
            }
            if request.max_citizens == 0 || request.max_citizens > MAX_CITIZENS_PER_PAGE {
                return Err(error(
                    ErrorCode::AdapterRejected,
                    "bridge accepted a citizen-page bound outside protocol V1",
                ));
            }
            let expected_offset = request.citizen_offset.min(self.citizen_count_total);
            if self.citizen_offset != expected_offset {
                return Err(error(
                    ErrorCode::AdapterRejected,
                    "accepted probe observation returned a noncanonical offset",
                ));
            }
            let returned = u32::try_from(self.citizens.len()).map_err(|_| {
                error(
                    ErrorCode::BudgetExceeded,
                    "probe citizen count does not fit u32",
                )
            })?;
            if returned > request.max_citizens
                || self.citizen_offset.saturating_add(returned) > self.citizen_count_total
                || self.complete
                    != (self.citizen_offset.saturating_add(returned)
                        == self.citizen_count_total)
                || (!self.complete && returned != request.max_citizens)
            {
                return Err(error(
                    ErrorCode::AdapterRejected,
                    "accepted probe observation violates page coverage semantics",
                ));
            }
            if !request.include_names
                && self.citizens.iter().any(|citizen| !citizen.name.is_empty())
            {
                return Err(error(
                    ErrorCode::AdapterRejected,
                    "name-omitted probe observation returned citizen names",
                ));
            }
            for pair in self.citizens.windows(2) {
                if pair[0].unit_id >= pair[1].unit_id {
                    return Err(error(
                        ErrorCode::AdapterRejected,
                        "probe citizen records are not in strict unit-ID order",
                    ));
                }
            }
        } else if self.failure_code.is_empty() {
            return Err(error(
                ErrorCode::AdapterRejected,
                "rejected probe observation omitted its failure code",
            ));
        }
        Ok(())
    }
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

    fn boolean(&mut self, field: u32, value: bool) {
        self.key(field, WireType::Varint);
        self.varint(if value { 1 } else { 0 });
    }

    fn bytes(&mut self, field: u32, value: &[u8]) -> Result<()> {
        if value.len() > MAX_RPC_PAYLOAD_BYTES {
            return Err(error(
                ErrorCode::BudgetExceeded,
                format!("protobuf field {field} exceeds the payload ceiling"),
            ));
        }
        self.key(field, WireType::LengthDelimited);
        let length = u64::try_from(value.len()).map_err(|_| {
            error(
                ErrorCode::BudgetExceeded,
                "protobuf field length does not fit u64",
            )
        })?;
        self.varint(length);
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
        if bytes.len() > MAX_RPC_PAYLOAD_BYTES {
            return Err(error(
                ErrorCode::BudgetExceeded,
                "protobuf payload exceeds the probe ceiling",
            ));
        }
        Ok(Self {
            bytes,
            offset: 0,
            fields_seen: 0,
        })
    }

    fn remaining(&self) -> usize {
        self.bytes.len().saturating_sub(self.offset)
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
        if self.remaining() == 0 {
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
                "protobuf field number zero is invalid",
            ));
        }
        Ok(Some((field, WireType::from_u8((key & 0x07) as u8)?)))
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

    fn uint32(&mut self, wire: WireType, field: u32) -> Result<u32> {
        Self::require_wire(wire, WireType::Varint, field)?;
        u32::try_from(self.varint()?).map_err(|_| {
            error(
                ErrorCode::AdapterRejected,
                format!("protobuf field {field} exceeds u32"),
            )
        })
    }

    fn uint64(&mut self, wire: WireType, field: u32) -> Result<u64> {
        Self::require_wire(wire, WireType::Varint, field)?;
        self.varint()
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
        let value = std::str::from_utf8(bytes).map_err(|_| {
            error(
                ErrorCode::AdapterRejected,
                format!("protobuf string field {field} is not valid UTF-8"),
            )
        })?;
        Ok(value.to_owned())
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
                if length > MAX_RPC_PAYLOAD_BYTES {
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

fn encode_bind_request(method: &str, input_type: &str, output_type: &str) -> Result<Vec<u8>> {
    let mut writer = ProtoWriter::default();
    writer.string(1, method)?;
    writer.string(2, input_type)?;
    writer.string(3, output_type)?;
    writer.string(4, PLUGIN_NAME)?;
    Ok(writer.finish())
}

fn decode_bind_reply(bytes: &[u8]) -> Result<i16> {
    let mut reader = ProtoReader::new(bytes)?;
    let mut assigned_id = None;
    while let Some((field, wire)) = reader.next_key()? {
        match field {
            1 => {
                let value = i16::try_from(reader.uint32(wire, field)?).map_err(|_| {
                    error(
                        ErrorCode::AdapterRejected,
                        "DFHack assigned method ID does not fit i16",
                    )
                })?;
                set_once(&mut assigned_id, value, "assigned_id")?;
            }
            _ => reader.skip(wire, field)?,
        }
    }
    let id = required(assigned_id, "assigned_id")?;
    if id < FIRST_PLUGIN_METHOD_ID {
        return Err(error(
            ErrorCode::AdapterRejected,
            "DFHack assigned a reserved core method ID to the probe",
        ));
    }
    Ok(id)
}

fn encode_handshake_request(request: &ProbeHandshakeRequest) -> Result<Vec<u8>> {
    request.validate_transport_bounds()?;
    let mut writer = ProtoWriter::default();
    writer.uint32(1, request.protocol_major);
    writer.uint32(2, request.protocol_minor);
    writer.string(3, &request.client_name)?;
    writer.string(4, &request.client_version)?;
    writer.bytes(5, &request.client_nonce)?;
    writer.bytes(6, &request.bearer_token)?;
    Ok(writer.finish())
}

fn encode_observation_request(request: &ProbeObservationRequest) -> Result<Vec<u8>> {
    request.validate_transport_bounds()?;
    let mut writer = ProtoWriter::default();
    writer.uint32(1, request.protocol_major);
    writer.uint32(2, request.protocol_minor);
    writer.bytes(3, &request.client_nonce)?;
    writer.bytes(4, &request.bearer_token)?;
    writer.uint32(5, request.citizen_offset);
    writer.uint32(6, request.max_citizens);
    writer.boolean(7, request.include_names);
    Ok(writer.finish())
}

fn decode_handshake_reply(bytes: &[u8]) -> Result<ProbeHandshakeReply> {
    let mut reader = ProtoReader::new(bytes)?;
    let mut accepted = None;
    let mut failure_code = None;
    let mut failure_message = None;
    let mut protocol_major = None;
    let mut protocol_minor = None;
    let mut bridge_version = None;
    let mut dfhack_version = None;
    let mut df_version = None;
    let mut world_loaded = None;
    let mut fortress_mode = None;
    let mut client_nonce = None;
    let mut bridge_generation = None;
    let mut supported_methods = BTreeSet::new();
    while let Some((field, wire)) = reader.next_key()? {
        match field {
            1 => set_once(&mut accepted, reader.boolean(wire, field)?, "accepted")?,
            2 => set_once(
                &mut failure_code,
                reader.string(wire, field, MAX_PROBE_FIELD_BYTES)?,
                "failure_code",
            )?,
            3 => set_once(
                &mut failure_message,
                reader.string(wire, field, MAX_PROBE_FIELD_BYTES)?,
                "failure_message",
            )?,
            4 => set_once(
                &mut protocol_major,
                reader.uint32(wire, field)?,
                "protocol_major",
            )?,
            5 => set_once(
                &mut protocol_minor,
                reader.uint32(wire, field)?,
                "protocol_minor",
            )?,
            6 => set_once(
                &mut bridge_version,
                reader.string(wire, field, MAX_PROBE_FIELD_BYTES)?,
                "bridge_version",
            )?,
            7 => set_once(
                &mut dfhack_version,
                reader.string(wire, field, MAX_PROBE_FIELD_BYTES)?,
                "dfhack_version",
            )?,
            8 => set_once(
                &mut df_version,
                reader.string(wire, field, MAX_PROBE_FIELD_BYTES)?,
                "df_version",
            )?,
            9 => set_once(
                &mut world_loaded,
                reader.boolean(wire, field)?,
                "world_loaded",
            )?,
            10 => set_once(
                &mut fortress_mode,
                reader.boolean(wire, field)?,
                "fortress_mode",
            )?,
            11 => set_once(
                &mut client_nonce,
                reader
                    .length_delimited(wire, field, MAX_PROBE_FIELD_BYTES)?
                    .to_vec(),
                "client_nonce",
            )?,
            12 => set_once(
                &mut bridge_generation,
                reader.uint64(wire, field)?,
                "bridge_generation",
            )?,
            13 => {
                if supported_methods.len() >= MAX_PROBE_METHODS {
                    return Err(error(
                        ErrorCode::BudgetExceeded,
                        "handshake reply exceeds the supported-method bound",
                    ));
                }
                let method = reader.string(wire, field, MAX_PROBE_FIELD_BYTES)?;
                if !supported_methods.insert(method) {
                    return Err(error(
                        ErrorCode::AdapterRejected,
                        "handshake reply repeats a supported method",
                    ));
                }
            }
            _ => reader.skip(wire, field)?,
        }
    }
    let reply = ProbeHandshakeReply {
        accepted: required(accepted, "accepted")?,
        failure_code: required(failure_code, "failure_code")?,
        failure_message: required(failure_message, "failure_message")?,
        protocol_major: required(protocol_major, "protocol_major")?,
        protocol_minor: required(protocol_minor, "protocol_minor")?,
        bridge_version: required(bridge_version, "bridge_version")?,
        dfhack_version: required(dfhack_version, "dfhack_version")?,
        df_version: required(df_version, "df_version")?,
        world_loaded: required(world_loaded, "world_loaded")?,
        fortress_mode: required(fortress_mode, "fortress_mode")?,
        client_nonce: required(client_nonce, "client_nonce")?,
        bridge_generation: required(bridge_generation, "bridge_generation")?,
        supported_methods,
    };
    reply.validate_shape()?;
    Ok(reply)
}

fn decode_citizen(bytes: &[u8]) -> Result<CitizenRecord> {
    let mut reader = ProtoReader::new(bytes)?;
    let mut unit_id = None;
    let mut name = None;
    let mut race = None;
    let mut profession = None;
    let mut x = None;
    let mut y = None;
    let mut z = None;
    let mut alive = None;
    let mut sane = None;
    let mut active = None;
    let mut visible = None;
    let mut citizen = None;
    let mut resident = None;
    let mut baby = None;
    let mut child = None;
    let mut adult = None;
    while let Some((field, wire)) = reader.next_key()? {
        match field {
            1 => set_once(&mut unit_id, reader.sint32(wire, field)?, "unit_id")?,
            2 => set_once(
                &mut name,
                reader.string(wire, field, MAX_UNIT_NAME_BYTES)?,
                "name",
            )?,
            3 => set_once(
                &mut race,
                reader.string(wire, field, MAX_RACE_NAME_BYTES)?,
                "race",
            )?,
            4 => set_once(
                &mut profession,
                reader.sint32(wire, field)?,
                "profession",
            )?,
            5 => set_once(&mut x, reader.sint32(wire, field)?, "x")?,
            6 => set_once(&mut y, reader.sint32(wire, field)?, "y")?,
            7 => set_once(&mut z, reader.sint32(wire, field)?, "z")?,
            8 => set_once(&mut alive, reader.boolean(wire, field)?, "alive")?,
            9 => set_once(&mut sane, reader.boolean(wire, field)?, "sane")?,
            10 => set_once(&mut active, reader.boolean(wire, field)?, "active")?,
            11 => set_once(&mut visible, reader.boolean(wire, field)?, "visible")?,
            12 => set_once(&mut citizen, reader.boolean(wire, field)?, "citizen")?,
            13 => set_once(&mut resident, reader.boolean(wire, field)?, "resident")?,
            14 => set_once(&mut baby, reader.boolean(wire, field)?, "baby")?,
            15 => set_once(&mut child, reader.boolean(wire, field)?, "child")?,
            16 => set_once(&mut adult, reader.boolean(wire, field)?, "adult")?,
            _ => reader.skip(wire, field)?,
        }
    }
    Ok(CitizenRecord {
        unit_id: required(unit_id, "unit_id")?,
        name: required(name, "name")?,
        race: required(race, "race")?,
        profession: required(profession, "profession")?,
        x: required(x, "x")?,
        y: required(y, "y")?,
        z: required(z, "z")?,
        alive: required(alive, "alive")?,
        sane: required(sane, "sane")?,
        active: required(active, "active")?,
        visible: required(visible, "visible")?,
        citizen: required(citizen, "citizen")?,
        resident: required(resident, "resident")?,
        baby: required(baby, "baby")?,
        child: required(child, "child")?,
        adult: required(adult, "adult")?,
    })
}

fn decode_observation_reply(
    bytes: &[u8],
    request: &ProbeObservationRequest,
) -> Result<ProbeObservationReply> {
    let mut reader = ProtoReader::new(bytes)?;
    let mut accepted = None;
    let mut failure_code = None;
    let mut failure_message = None;
    let mut protocol_major = None;
    let mut protocol_minor = None;
    let mut client_nonce = None;
    let mut bridge_generation = None;
    let mut world_loaded = None;
    let mut fortress_mode = None;
    let mut paused = None;
    let mut current_year = None;
    let mut current_year_tick = None;
    let mut world_name = None;
    let mut world_folder = None;
    let mut site_id = None;
    let mut citizen_count_total = None;
    let mut citizen_offset = None;
    let mut complete = None;
    let mut citizens = Vec::new();
    while let Some((field, wire)) = reader.next_key()? {
        match field {
            1 => set_once(&mut accepted, reader.boolean(wire, field)?, "accepted")?,
            2 => set_once(
                &mut failure_code,
                reader.string(wire, field, MAX_PROBE_FIELD_BYTES)?,
                "failure_code",
            )?,
            3 => set_once(
                &mut failure_message,
                reader.string(wire, field, MAX_PROBE_FIELD_BYTES)?,
                "failure_message",
            )?,
            4 => set_once(
                &mut protocol_major,
                reader.uint32(wire, field)?,
                "protocol_major",
            )?,
            5 => set_once(
                &mut protocol_minor,
                reader.uint32(wire, field)?,
                "protocol_minor",
            )?,
            6 => set_once(
                &mut client_nonce,
                reader
                    .length_delimited(wire, field, MAX_PROBE_FIELD_BYTES)?
                    .to_vec(),
                "client_nonce",
            )?,
            7 => set_once(
                &mut bridge_generation,
                reader.uint64(wire, field)?,
                "bridge_generation",
            )?,
            8 => set_once(
                &mut world_loaded,
                reader.boolean(wire, field)?,
                "world_loaded",
            )?,
            9 => set_once(
                &mut fortress_mode,
                reader.boolean(wire, field)?,
                "fortress_mode",
            )?,
            10 => set_once(&mut paused, reader.boolean(wire, field)?, "paused")?,
            11 => set_once(
                &mut current_year,
                reader.uint32(wire, field)?,
                "current_year",
            )?,
            12 => set_once(
                &mut current_year_tick,
                reader.uint32(wire, field)?,
                "current_year_tick",
            )?,
            13 => set_once(
                &mut world_name,
                reader.string(wire, field, MAX_WORLD_NAME_BYTES)?,
                "world_name",
            )?,
            14 => set_once(
                &mut world_folder,
                reader.string(wire, field, MAX_WORLD_FOLDER_BYTES)?,
                "world_folder",
            )?,
            15 => set_once(&mut site_id, reader.sint32(wire, field)?, "site_id")?,
            16 => set_once(
                &mut citizen_count_total,
                reader.uint32(wire, field)?,
                "citizen_count_total",
            )?,
            17 => set_once(
                &mut citizen_offset,
                reader.uint32(wire, field)?,
                "citizen_offset",
            )?,
            18 => set_once(&mut complete, reader.boolean(wire, field)?, "complete")?,
            19 => {
                if citizens.len() >= usize::try_from(MAX_CITIZENS_PER_PAGE).map_err(|_| {
                    error(
                        ErrorCode::InternalInvariantViolation,
                        "citizen page ceiling does not fit usize",
                    )
                })? {
                    return Err(error(
                        ErrorCode::BudgetExceeded,
                        "probe observation reply exceeds the citizen-page ceiling",
                    ));
                }
                let nested = reader.length_delimited(wire, field, MAX_RPC_PAYLOAD_BYTES)?;
                citizens.push(decode_citizen(nested)?);
            }
            _ => reader.skip(wire, field)?,
        }
    }
    let reply = ProbeObservationReply {
        accepted: required(accepted, "accepted")?,
        failure_code: required(failure_code, "failure_code")?,
        failure_message: required(failure_message, "failure_message")?,
        protocol_major: required(protocol_major, "protocol_major")?,
        protocol_minor: required(protocol_minor, "protocol_minor")?,
        client_nonce: required(client_nonce, "client_nonce")?,
        bridge_generation: required(bridge_generation, "bridge_generation")?,
        world_loaded: required(world_loaded, "world_loaded")?,
        fortress_mode: required(fortress_mode, "fortress_mode")?,
        paused: required(paused, "paused")?,
        current_year: required(current_year, "current_year")?,
        current_year_tick: required(current_year_tick, "current_year_tick")?,
        world_name: required(world_name, "world_name")?,
        world_folder: required(world_folder, "world_folder")?,
        site_id: required(site_id, "site_id")?,
        citizen_count_total: required(citizen_count_total, "citizen_count_total")?,
        citizen_offset: required(citizen_offset, "citizen_offset")?,
        complete: required(complete, "complete")?,
        citizens,
    };
    reply.validate_shape(request)?;
    Ok(reply)
}

fn encode_handshake_header(magic: &[u8; 8]) -> [u8; HANDSHAKE_HEADER_BYTES] {
    let mut header = [0u8; HANDSHAKE_HEADER_BYTES];
    header[..8].copy_from_slice(magic);
    header[8..12].copy_from_slice(&DFHACK_RPC_VERSION.to_le_bytes());
    header
}

fn encode_message_header(id: i16, size: i32) -> [u8; MESSAGE_HEADER_BYTES] {
    let mut header = [0u8; MESSAGE_HEADER_BYTES];
    header[..2].copy_from_slice(&id.to_le_bytes());
    header[4..8].copy_from_slice(&size.to_le_bytes());
    header
}

fn decode_message_header(header: &[u8; MESSAGE_HEADER_BYTES]) -> (i16, i32) {
    (
        i16::from_le_bytes([header[0], header[1]]),
        i32::from_le_bytes([header[4], header[5], header[6], header[7]]),
    )
}

pub struct DfHackProbeClient<S> {
    stream: S,
    handshake_method_id: i16,
    observation_method_id: i16,
}

impl<S: Read + Write> DfHackProbeClient<S> {
    pub fn negotiate_transport(mut stream: S) -> Result<Self> {
        stream
            .write_all(&encode_handshake_header(REQUEST_MAGIC))
            .map_err(|source| io_error("transport handshake write", &source))?;
        stream
            .flush()
            .map_err(|source| io_error("transport handshake flush", &source))?;
        let mut response = [0u8; HANDSHAKE_HEADER_BYTES];
        stream
            .read_exact(&mut response)
            .map_err(|source| io_error("transport handshake read", &source))?;
        if &response[..8] != RESPONSE_MAGIC {
            return Err(error(
                ErrorCode::AdapterRejected,
                "DFHack probe handshake response magic is invalid",
            ));
        }
        let version = i32::from_le_bytes([response[8], response[9], response[10], response[11]]);
        if version != DFHACK_RPC_VERSION {
            return Err(error(
                ErrorCode::VersionMismatch,
                format!(
                    "DFHack RPC version {version} does not match required {DFHACK_RPC_VERSION}"
                ),
            ));
        }
        let handshake_method_id = Self::bind_method(
            &mut stream,
            HANDSHAKE_METHOD,
            HANDSHAKE_INPUT_TYPE,
            HANDSHAKE_OUTPUT_TYPE,
        )?;
        let observation_method_id = Self::bind_method(
            &mut stream,
            OBSERVATION_METHOD,
            OBSERVATION_INPUT_TYPE,
            OBSERVATION_OUTPUT_TYPE,
        )?;
        if handshake_method_id == observation_method_id {
            return Err(error(
                ErrorCode::AdapterRejected,
                "DFHack assigned the same method ID to both probe methods",
            ));
        }
        Ok(Self {
            stream,
            handshake_method_id,
            observation_method_id,
        })
    }

    fn bind_method(
        stream: &mut S,
        method: &str,
        input_type: &str,
        output_type: &str,
    ) -> Result<i16> {
        let request = encode_bind_request(method, input_type, output_type)?;
        let response = Self::call(stream, BIND_METHOD_ID, &request)?;
        decode_bind_reply(&response)
    }

    fn call(stream: &mut S, method_id: i16, payload: &[u8]) -> Result<Vec<u8>> {
        if method_id < 0 {
            return Err(error(
                ErrorCode::InternalInvariantViolation,
                "attempted to call a negative DFHack probe method ID",
            ));
        }
        if payload.len() > MAX_RPC_PAYLOAD_BYTES {
            return Err(error(
                ErrorCode::BudgetExceeded,
                "DFHack probe request exceeds the payload ceiling",
            ));
        }
        let size = i32::try_from(payload.len()).map_err(|_| {
            error(
                ErrorCode::BudgetExceeded,
                "DFHack probe request length does not fit i32",
            )
        })?;
        stream
            .write_all(&encode_message_header(method_id, size))
            .map_err(|source| io_error("message header write", &source))?;
        stream
            .write_all(payload)
            .map_err(|source| io_error("message payload write", &source))?;
        stream
            .flush()
            .map_err(|source| io_error("message flush", &source))?;

        let mut text_notifications = 0u32;
        let mut text_bytes = 0usize;
        loop {
            let mut header = [0u8; MESSAGE_HEADER_BYTES];
            stream
                .read_exact(&mut header)
                .map_err(|source| io_error("reply header read", &source))?;
            let (reply_id, reply_size) = decode_message_header(&header);
            if reply_id == RPC_REPLY_FAIL {
                return Err(error(
                    ErrorCode::AdapterFailure,
                    format!("DFHack probe method {method_id} failed with code {reply_size}"),
                ));
            }
            if reply_size < 0 {
                return Err(error(
                    ErrorCode::AdapterRejected,
                    "DFHack probe reply has a negative payload length",
                ));
            }
            let length = usize::try_from(reply_size).map_err(|_| {
                error(
                    ErrorCode::AdapterRejected,
                    "DFHack probe reply length does not fit usize",
                )
            })?;
            let limit = if reply_id == RPC_REPLY_TEXT {
                MAX_PROBE_TEXT_NOTIFICATION_BYTES
            } else {
                MAX_RPC_PAYLOAD_BYTES
            };
            if length > limit {
                return Err(error(
                    ErrorCode::BudgetExceeded,
                    format!("DFHack probe reply exceeds the {limit}-byte bound"),
                ));
            }
            let mut response = vec![0u8; length];
            stream
                .read_exact(&mut response)
                .map_err(|source| io_error("reply payload read", &source))?;
            match reply_id {
                RPC_REPLY_TEXT => {
                    text_notifications = text_notifications.checked_add(1).ok_or_else(|| {
                        error(
                            ErrorCode::BudgetExceeded,
                            "probe text-notification counter overflowed",
                        )
                    })?;
                    text_bytes = text_bytes.checked_add(length).ok_or_else(|| {
                        error(
                            ErrorCode::BudgetExceeded,
                            "probe text-notification byte counter overflowed",
                        )
                    })?;
                    if text_notifications > MAX_TEXT_NOTIFICATIONS_PER_CALL
                        || text_bytes > MAX_TEXT_NOTIFICATION_TOTAL_BYTES
                    {
                        return Err(error(
                            ErrorCode::BudgetExceeded,
                            "DFHack probe call exceeded the text-notification budget",
                        ));
                    }
                }
                RPC_REPLY_RESULT => return Ok(response),
                other => {
                    return Err(error(
                        ErrorCode::AdapterRejected,
                        format!("unexpected DFHack probe reply ID {other}"),
                    ));
                }
            }
        }
    }

    pub fn handshake(&mut self, request: &ProbeHandshakeRequest) -> Result<ProbeHandshakeReply> {
        let payload = encode_handshake_request(request)?;
        let response = Self::call(&mut self.stream, self.handshake_method_id, &payload)?;
        decode_handshake_reply(&response)
    }

    pub fn read_observation(
        &mut self,
        request: &ProbeObservationRequest,
    ) -> Result<ProbeObservationReply> {
        let payload = encode_observation_request(request)?;
        let response = Self::call(&mut self.stream, self.observation_method_id, &payload)?;
        decode_observation_reply(&response, request)
    }

    pub fn close(mut self) -> Result<S> {
        self.stream
            .write_all(&encode_message_header(RPC_REQUEST_QUIT, 0))
            .map_err(|source| io_error("quit write", &source))?;
        self.stream
            .flush()
            .map_err(|source| io_error("quit flush", &source))?;
        Ok(self.stream)
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Read, Write};

    use super::*;

    #[derive(Default)]
    struct ScriptedIo {
        reads: Cursor<Vec<u8>>,
        writes: Vec<u8>,
    }

    impl ScriptedIo {
        fn new(reads: Vec<u8>) -> Self {
            Self {
                reads: Cursor::new(reads),
                writes: Vec::new(),
            }
        }
    }

    impl Read for ScriptedIo {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            self.reads.read(buffer)
        }
    }

    impl Write for ScriptedIo {
        fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
            self.writes.extend_from_slice(buffer);
            Ok(buffer.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn rpc_reply(reply_id: i16, payload: &[u8]) -> Result<Vec<u8>> {
        let size = i32::try_from(payload.len()).map_err(|_| {
            error(
                ErrorCode::BudgetExceeded,
                "test reply length does not fit i32",
            )
        })?;
        let mut bytes = encode_message_header(reply_id, size).to_vec();
        bytes.extend_from_slice(payload);
        Ok(bytes)
    }

    fn rpc_result(payload: &[u8]) -> Result<Vec<u8>> {
        rpc_reply(RPC_REPLY_RESULT, payload)
    }

    fn bind_reply(id: u32) -> Result<Vec<u8>> {
        let mut writer = ProtoWriter::default();
        writer.uint32(1, id);
        rpc_result(&writer.finish())
    }

    fn rejected_handshake_reply(code: &str) -> Result<Vec<u8>> {
        let mut writer = ProtoWriter::default();
        writer.boolean(1, false);
        writer.string(2, code)?;
        writer.string(3, "rejected by test bridge")?;
        writer.uint32(4, BRIDGE_PROTOCOL_MAJOR);
        writer.uint32(5, BRIDGE_PROTOCOL_MINOR);
        writer.string(6, "")?;
        writer.string(7, "")?;
        writer.string(8, "")?;
        writer.boolean(9, false);
        writer.boolean(10, false);
        writer.bytes(11, b"")?;
        writer.key(12, WireType::Varint);
        writer.varint(0);
        rpc_result(&writer.finish())
    }

    fn rejected_observation_reply(code: &str, nonce: &[u8], generation: u64) -> Result<Vec<u8>> {
        let mut writer = ProtoWriter::default();
        writer.boolean(1, false);
        writer.string(2, code)?;
        writer.string(3, "rejected by test bridge")?;
        writer.uint32(4, BRIDGE_PROTOCOL_MAJOR);
        writer.uint32(5, BRIDGE_PROTOCOL_MINOR);
        writer.bytes(6, nonce)?;
        writer.key(7, WireType::Varint);
        writer.varint(generation);
        writer.boolean(8, false);
        writer.boolean(9, false);
        writer.boolean(10, false);
        writer.uint32(11, 0);
        writer.uint32(12, 0);
        writer.string(13, "")?;
        writer.string(14, "")?;
        writer.key(15, WireType::Varint);
        writer.varint(1);
        writer.uint32(16, 0);
        writer.uint32(17, 0);
        writer.boolean(18, false);
        rpc_result(&writer.finish())
    }

    fn transport_prefix() -> Result<Vec<u8>> {
        let mut reads = encode_handshake_header(RESPONSE_MAGIC).to_vec();
        reads.extend_from_slice(&bind_reply(41)?);
        reads.extend_from_slice(&bind_reply(42)?);
        Ok(reads)
    }

    #[test]
    fn raw_probe_sends_locally_invalid_credentials_and_returns_typed_rejection() -> Result<()> {
        let mut reads = transport_prefix()?;
        reads.extend_from_slice(&rejected_handshake_reply("AUTH_REQUIRED")?);
        let mut client = DfHackProbeClient::negotiate_transport(ScriptedIo::new(reads))?;
        let request = ProbeHandshakeRequest {
            protocol_major: BRIDGE_PROTOCOL_MAJOR,
            protocol_minor: BRIDGE_PROTOCOL_MINOR,
            client_name: "dfmcp-probe".to_owned(),
            client_version: "0.0.1".to_owned(),
            client_nonce: vec![1; 15],
            bearer_token: Vec::new(),
        };
        let reply = client.handshake(&request)?;
        assert!(!reply.accepted);
        assert_eq!(reply.failure_code, "AUTH_REQUIRED");
        assert!(!reply.sensitive_manifest_disclosed());
        assert!(!format!("{request:?}").contains("bearer_token: []"));
        Ok(())
    }

    #[test]
    fn raw_probe_can_send_an_oversized_protocol_page_bound() -> Result<()> {
        let nonce = vec![7; 16];
        let mut reads = transport_prefix()?;
        reads.extend_from_slice(&rejected_observation_reply(
            "INVALID_BOUND",
            &nonce,
            42,
        )?);
        let mut client = DfHackProbeClient::negotiate_transport(ScriptedIo::new(reads))?;
        let request = ProbeObservationRequest {
            protocol_major: BRIDGE_PROTOCOL_MAJOR,
            protocol_minor: BRIDGE_PROTOCOL_MINOR,
            client_nonce: nonce.clone(),
            bearer_token: vec![9; 32],
            citizen_offset: 0,
            max_citizens: MAX_CITIZENS_PER_PAGE + 1,
            include_names: true,
        };
        let reply = client.read_observation(&request)?;
        assert!(!reply.accepted);
        assert_eq!(reply.failure_code, "INVALID_BOUND");
        assert!(reply.nonce_correlated(&nonce));
        assert_eq!(reply.bridge_generation, 42);
        assert!(!reply.world_posture_disclosed());
        Ok(())
    }

    #[test]
    fn duplicate_required_reply_field_is_rejected() -> Result<()> {
        let mut writer = ProtoWriter::default();
        writer.boolean(1, false);
        writer.boolean(1, false);
        assert!(decode_handshake_reply(&writer.finish()).is_err());
        Ok(())
    }

    #[test]
    fn probe_request_debug_output_redacts_token_contents() {
        let request = ProbeObservationRequest {
            protocol_major: 1,
            protocol_minor: 0,
            client_nonce: vec![1; 16],
            bearer_token: vec![b'x'; 32],
            citizen_offset: 0,
            max_citizens: 1,
            include_names: true,
        };
        let rendered = format!("{request:?}");
        assert!(rendered.contains("<redacted>"));
        assert!(!rendered.contains(&"x".repeat(32)));
    }

    #[test]
    fn overlong_varint_is_rejected() -> Result<()> {
        let mut reader = ProtoReader::new(&[0x80, 0x00])?;
        assert!(reader.varint().is_err());
        Ok(())
    }
}
