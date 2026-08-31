#![forbid(unsafe_code)]

//! Dependency-free safe-Rust client for the supported DFHack remote protocol
//! and the read-only `dfmcp_bridge` plugin service.
//!
//! DFHack's transport is not gRPC. It uses a 12-byte native-layout handshake,
//! then 8-byte native-layout RPC headers with protobuf-Lite payloads. This
//! module implements only the protobuf wire types required by `BindMethod`,
//! `Handshake`, and `ReadObservation`. It owns no socket/runtime policy: the
//! caller supplies an already connected bounded `Read + Write` stream.

use std::collections::BTreeSet;
use std::io::{Read, Write};

use dfmcp_core::{DfmcpError, ErrorCode, Result};

pub const DFHACK_RPC_VERSION: i32 = 1;
pub const BRIDGE_PROTOCOL_MAJOR: u32 = 1;
pub const BRIDGE_PROTOCOL_MINOR: u32 = 0;
pub const MAX_RPC_PAYLOAD_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_TEXT_NOTIFICATION_BYTES: usize = 64 * 1024;
pub const MAX_BRIDGE_TOKEN_BYTES: usize = 256;
pub const MIN_BRIDGE_TOKEN_BYTES: usize = 32;
pub const MAX_NONCE_BYTES: usize = 64;
pub const MIN_NONCE_BYTES: usize = 16;
pub const MAX_CLIENT_NAME_BYTES: usize = 128;
pub const MAX_CLIENT_VERSION_BYTES: usize = 64;
pub const MAX_CITIZENS_PER_PAGE: u32 = 4096;
pub const MAX_UNIT_NAME_BYTES: usize = 256;
pub const MAX_RACE_NAME_BYTES: usize = 128;
pub const MAX_WORLD_NAME_BYTES: usize = 256;
pub const MAX_WORLD_FOLDER_BYTES: usize = 512;

const REQUEST_MAGIC: &[u8; 8] = b"DFHack?\n";
const RESPONSE_MAGIC: &[u8; 8] = b"DFHack!\n";
const HANDSHAKE_HEADER_BYTES: usize = 12;
const MESSAGE_HEADER_BYTES: usize = 8;
const RPC_REPLY_RESULT: i16 = -1;
const RPC_REPLY_FAIL: i16 = -2;
const RPC_REPLY_TEXT: i16 = -3;
const RPC_REQUEST_QUIT: i16 = -4;
const BIND_METHOD_ID: i16 = 0;
const PLUGIN_NAME: &str = "dfmcp_bridge";
const HANDSHAKE_METHOD: &str = "Handshake";
const OBSERVATION_METHOD: &str = "ReadObservation";
const HANDSHAKE_INPUT_TYPE: &str = "dfmcp.bridge.v1.HandshakeRequest";
const HANDSHAKE_OUTPUT_TYPE: &str = "dfmcp.bridge.v1.HandshakeReply";
const OBSERVATION_INPUT_TYPE: &str = "dfmcp.bridge.v1.ReadObservationRequest";
const OBSERVATION_OUTPUT_TYPE: &str = "dfmcp.bridge.v1.ReadObservationReply";

fn error(code: ErrorCode, message: impl Into<String>) -> DfmcpError {
    DfmcpError::new(code, message)
}

fn io_error(operation: &str, source: &std::io::Error) -> DfmcpError {
    error(
        ErrorCode::AdapterUnavailable,
        format!("DFHack RPC {operation} failed: {source}"),
    )
    .retryable(true)
}

fn validate_len(value: &[u8], field: &str, minimum: usize, maximum: usize) -> Result<()> {
    if value.len() < minimum || value.len() > maximum {
        return Err(error(
            ErrorCode::InvalidRequest,
            format!(
                "{field} length {} is outside the admitted {minimum}..={maximum} byte range",
                value.len()
            ),
        ));
    }
    Ok(())
}

fn validate_text(value: &str, field: &str, maximum: usize) -> Result<()> {
    validate_len(value.as_bytes(), field, 1, maximum)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BridgeCredentials {
    token: Vec<u8>,
    nonce: Vec<u8>,
}

impl BridgeCredentials {
    pub fn new(token: Vec<u8>, nonce: Vec<u8>) -> Result<Self> {
        validate_len(
            &token,
            "bridge bearer token",
            MIN_BRIDGE_TOKEN_BYTES,
            MAX_BRIDGE_TOKEN_BYTES,
        )?;
        validate_len(
            &nonce,
            "bridge client nonce",
            MIN_NONCE_BYTES,
            MAX_NONCE_BYTES,
        )?;
        Ok(Self { token, nonce })
    }

    #[must_use]
    pub fn nonce(&self) -> &[u8] {
        &self.nonce
    }

    fn token(&self) -> &[u8] {
        &self.token
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BridgeManifest {
    pub bridge_version: String,
    pub dfhack_version: String,
    pub df_version: String,
    pub world_loaded: bool,
    pub fortress_mode: bool,
    pub bridge_generation: u64,
    pub supported_methods: BTreeSet<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CitizenRecord {
    pub unit_id: i32,
    pub name: String,
    pub race: String,
    pub profession: i32,
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub alive: bool,
    pub sane: bool,
    pub active: bool,
    pub visible: bool,
    pub citizen: bool,
    pub resident: bool,
    pub baby: bool,
    pub child: bool,
    pub adult: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObservationPage {
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

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct ProtoWriter {
    bytes: Vec<u8>,
}

impl ProtoWriter {
    fn key(&mut self, field: u32, wire: WireType) {
        self.varint(u64::from((field << 3) | wire as u32));
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
        let zigzag = ((value << 1) ^ (value >> 31)) as u32;
        self.varint(u64::from(zigzag));
    }

    fn boolean(&mut self, field: u32, value: bool) {
        self.key(field, WireType::Varint);
        self.varint(u64::from(value));
    }

    fn bytes(&mut self, field: u32, value: &[u8]) -> Result<()> {
        self.key(field, WireType::LengthDelimited);
        let length = u64::try_from(value.len()).map_err(|_| {
            error(
                ErrorCode::BudgetExceeded,
                "protobuf field length does not fit in u64",
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
}

impl<'a> ProtoReader<'a> {
    fn new(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() > MAX_RPC_PAYLOAD_BYTES {
            return Err(error(
                ErrorCode::BudgetExceeded,
                "protobuf payload exceeds the 8 MiB client ceiling",
            ));
        }
        Ok(Self { bytes, offset: 0 })
    }

    fn remaining(&self) -> usize {
        self.bytes.len().saturating_sub(self.offset)
    }

    fn varint(&mut self) -> Result<u64> {
        let mut value = 0u64;
        for shift in (0..70).step_by(7) {
            let byte = *self.bytes.get(self.offset).ok_or_else(|| {
                error(
                    ErrorCode::AdapterRejected,
                    "truncated protobuf varint",
                )
            })?;
            self.offset += 1;
            if shift == 63 && byte > 1 {
                return Err(error(
                    ErrorCode::AdapterRejected,
                    "protobuf varint overflows u64",
                ));
            }
            value |= u64::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
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
        let key = self.varint()?;
        let field = u32::try_from(key >> 3).map_err(|_| {
            error(
                ErrorCode::AdapterRejected,
                "protobuf field number does not fit in u32",
            )
        })?;
        if field == 0 {
            return Err(error(
                ErrorCode::AdapterRejected,
                "protobuf field number zero is invalid",
            ));
        }
        let wire = WireType::from_u8((key & 0x07) as u8)?;
        Ok(Some((field, wire)))
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
        let decoded = (encoded >> 1) as i32 ^ -((encoded & 1) as i32);
        Ok(decoded)
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
        String::from_utf8(bytes.to_vec()).map_err(|_| {
            error(
                ErrorCode::AdapterRejected,
                format!("protobuf string field {field} is not valid UTF-8"),
            )
        })
    }

    fn skip(&mut self, wire: WireType, field: u32) -> Result<()> {
        match wire {
            WireType::Varint => {
                let _ = self.varint()?;
            }
            WireType::Fixed64 => {
                self.advance_exact(8, field)?;
            }
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
            WireType::Fixed32 => {
                self.advance_exact(4, field)?;
            }
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
                let value = reader.uint32(wire, field)?;
                let value = i16::try_from(value).map_err(|_| {
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
    if id < 0 {
        return Err(error(
            ErrorCode::AdapterRejected,
            "DFHack assigned a negative method ID",
        ));
    }
    Ok(id)
}

fn encode_handshake_request(
    credentials: &BridgeCredentials,
    client_name: &str,
    client_version: &str,
) -> Result<Vec<u8>> {
    validate_text(client_name, "bridge client name", MAX_CLIENT_NAME_BYTES)?;
    validate_text(
        client_version,
        "bridge client version",
        MAX_CLIENT_VERSION_BYTES,
    )?;
    let mut writer = ProtoWriter::default();
    writer.uint32(1, BRIDGE_PROTOCOL_MAJOR);
    writer.uint32(2, BRIDGE_PROTOCOL_MINOR);
    writer.string(3, client_name)?;
    writer.string(4, client_version)?;
    writer.bytes(5, credentials.nonce())?;
    writer.bytes(6, credentials.token())?;
    Ok(writer.finish())
}

fn encode_observation_request(
    credentials: &BridgeCredentials,
    offset: u32,
    maximum: u32,
    include_names: bool,
) -> Result<Vec<u8>> {
    if maximum > MAX_CITIZENS_PER_PAGE {
        return Err(error(
            ErrorCode::BudgetExceeded,
            format!(
                "requested {maximum} citizens exceeds the hard page limit of {MAX_CITIZENS_PER_PAGE}"
            ),
        ));
    }
    let mut writer = ProtoWriter::default();
    writer.uint32(1, BRIDGE_PROTOCOL_MAJOR);
    writer.uint32(2, BRIDGE_PROTOCOL_MINOR);
    writer.bytes(3, credentials.nonce())?;
    writer.bytes(4, credentials.token())?;
    writer.uint32(5, offset);
    writer.uint32(6, maximum);
    writer.boolean(7, include_names);
    Ok(writer.finish())
}

fn validate_protocol(major: u32, minor: u32) -> Result<()> {
    if major != BRIDGE_PROTOCOL_MAJOR || minor != BRIDGE_PROTOCOL_MINOR {
        return Err(error(
            ErrorCode::VersionMismatch,
            format!(
                "bridge protocol {major}.{minor} does not match required {BRIDGE_PROTOCOL_MAJOR}.{BRIDGE_PROTOCOL_MINOR}"
            ),
        ));
    }
    Ok(())
}

fn reject_bridge(failure_code: String, failure_message: String) -> DfmcpError {
    error(
        ErrorCode::AdapterRejected,
        if failure_message.is_empty() {
            "DFHack bridge rejected the request".to_owned()
        } else {
            failure_message
        },
    )
    .with_detail("bridge_failure_code", failure_code)
}

fn decode_handshake_reply(bytes: &[u8], expected_nonce: &[u8]) -> Result<BridgeManifest> {
    let mut reader = ProtoReader::new(bytes)?;
    let mut accepted = None;
    let mut failure_code = None;
    let mut failure_message = None;
    let mut major = None;
    let mut minor = None;
    let mut bridge_version = None;
    let mut dfhack_version = None;
    let mut df_version = None;
    let mut world_loaded = None;
    let mut fortress_mode = None;
    let mut nonce = None;
    let mut generation = None;
    let mut methods = BTreeSet::new();

    while let Some((field, wire)) = reader.next_key()? {
        match field {
            1 => set_once(&mut accepted, reader.boolean(wire, field)?, "accepted")?,
            2 => set_once(
                &mut failure_code,
                reader.string(wire, field, 64)?,
                "failure_code",
            )?,
            3 => set_once(
                &mut failure_message,
                reader.string(wire, field, 1024)?,
                "failure_message",
            )?,
            4 => set_once(&mut major, reader.uint32(wire, field)?, "protocol_major")?,
            5 => set_once(&mut minor, reader.uint32(wire, field)?, "protocol_minor")?,
            6 => set_once(
                &mut bridge_version,
                reader.string(wire, field, 64)?,
                "bridge_version",
            )?,
            7 => set_once(
                &mut dfhack_version,
                reader.string(wire, field, 128)?,
                "dfhack_version",
            )?,
            8 => set_once(
                &mut df_version,
                reader.string(wire, field, 128)?,
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
                &mut nonce,
                reader
                    .length_delimited(wire, field, MAX_NONCE_BYTES)?
                    .to_vec(),
                "client_nonce",
            )?,
            12 => set_once(
                &mut generation,
                reader.uint64(wire, field)?,
                "bridge_generation",
            )?,
            13 => {
                let method = reader.string(wire, field, 64)?;
                if !methods.insert(method) {
                    return Err(error(
                        ErrorCode::AdapterRejected,
                        "bridge handshake repeats a supported method",
                    ));
                }
            }
            _ => reader.skip(wire, field)?,
        }
    }

    let accepted = required(accepted, "accepted")?;
    let failure_code = required(failure_code, "failure_code")?;
    let failure_message = required(failure_message, "failure_message")?;
    let major = required(major, "protocol_major")?;
    let minor = required(minor, "protocol_minor")?;
    validate_protocol(major, minor)?;
    let nonce = required(nonce, "client_nonce")?;
    if nonce != expected_nonce {
        return Err(error(
            ErrorCode::AdapterRejected,
            "bridge handshake nonce does not match the client nonce",
        ));
    }
    if !accepted {
        return Err(reject_bridge(failure_code, failure_message));
    }
    if !failure_code.is_empty() || !failure_message.is_empty() {
        return Err(error(
            ErrorCode::AdapterRejected,
            "accepted bridge handshake carries failure details",
        ));
    }
    let expected_methods = BTreeSet::from([
        HANDSHAKE_METHOD.to_owned(),
        OBSERVATION_METHOD.to_owned(),
    ]);
    if methods != expected_methods {
        return Err(error(
            ErrorCode::VersionMismatch,
            "bridge method set does not match protocol 1.0",
        ));
    }

    Ok(BridgeManifest {
        bridge_version: required(bridge_version, "bridge_version")?,
        dfhack_version: required(dfhack_version, "dfhack_version")?,
        df_version: required(df_version, "df_version")?,
        world_loaded: required(world_loaded, "world_loaded")?,
        fortress_mode: required(fortress_mode, "fortress_mode")?,
        bridge_generation: required(generation, "bridge_generation")?,
        supported_methods: methods,
    })
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
    expected_nonce: &[u8],
    expected_generation: u64,
    requested_offset: u32,
    requested_maximum: u32,
) -> Result<ObservationPage> {
    let mut reader = ProtoReader::new(bytes)?;
    let mut accepted = None;
    let mut failure_code = None;
    let mut failure_message = None;
    let mut major = None;
    let mut minor = None;
    let mut nonce = None;
    let mut generation = None;
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
                reader.string(wire, field, 64)?,
                "failure_code",
            )?,
            3 => set_once(
                &mut failure_message,
                reader.string(wire, field, 1024)?,
                "failure_message",
            )?,
            4 => set_once(&mut major, reader.uint32(wire, field)?, "protocol_major")?,
            5 => set_once(&mut minor, reader.uint32(wire, field)?, "protocol_minor")?,
            6 => set_once(
                &mut nonce,
                reader
                    .length_delimited(wire, field, MAX_NONCE_BYTES)?
                    .to_vec(),
                "client_nonce",
            )?,
            7 => set_once(
                &mut generation,
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
                        "citizen hard limit does not fit usize",
                    )
                })? {
                    return Err(error(
                        ErrorCode::BudgetExceeded,
                        "observation reply exceeds the citizen hard limit",
                    ));
                }
                let nested = reader.length_delimited(wire, field, MAX_RPC_PAYLOAD_BYTES)?;
                citizens.push(decode_citizen(nested)?);
            }
            _ => reader.skip(wire, field)?,
        }
    }

    let accepted = required(accepted, "accepted")?;
    let failure_code = required(failure_code, "failure_code")?;
    let failure_message = required(failure_message, "failure_message")?;
    validate_protocol(
        required(major, "protocol_major")?,
        required(minor, "protocol_minor")?,
    )?;
    if required(nonce, "client_nonce")? != expected_nonce {
        return Err(error(
            ErrorCode::AdapterRejected,
            "observation nonce does not match the negotiated client nonce",
        ));
    }
    let generation = required(generation, "bridge_generation")?;
    if generation != expected_generation {
        return Err(error(
            ErrorCode::StaleAnchor,
            "DFHack bridge generation changed after handshake",
        ));
    }
    if !accepted {
        return Err(reject_bridge(failure_code, failure_message));
    }
    if !failure_code.is_empty() || !failure_message.is_empty() {
        return Err(error(
            ErrorCode::AdapterRejected,
            "accepted observation carries failure details",
        ));
    }

    let total = required(citizen_count_total, "citizen_count_total")?;
    let offset = required(citizen_offset, "citizen_offset")?;
    let expected_offset = requested_offset.min(total);
    if offset != expected_offset {
        return Err(error(
            ErrorCode::AdapterRejected,
            format!(
                "observation offset {offset} does not match canonical expected offset {expected_offset}"
            ),
        ));
    }
    let returned = u32::try_from(citizens.len()).map_err(|_| {
        error(
            ErrorCode::BudgetExceeded,
            "citizen page length does not fit u32",
        )
    })?;
    if returned > requested_maximum || offset.saturating_add(returned) > total {
        return Err(error(
            ErrorCode::AdapterRejected,
            "observation page violates requested or total citizen bounds",
        ));
    }
    for pair in citizens.windows(2) {
        if pair[0].unit_id >= pair[1].unit_id {
            return Err(error(
                ErrorCode::AdapterRejected,
                "citizen records are not in strict unit-ID order",
            ));
        }
    }
    let complete = required(complete, "complete")?;
    if complete != (offset.saturating_add(returned) == total) {
        return Err(error(
            ErrorCode::AdapterRejected,
            "observation completeness flag disagrees with page coverage",
        ));
    }

    Ok(ObservationPage {
        bridge_generation: generation,
        world_loaded: required(world_loaded, "world_loaded")?,
        fortress_mode: required(fortress_mode, "fortress_mode")?,
        paused: required(paused, "paused")?,
        current_year: required(current_year, "current_year")?,
        current_year_tick: required(current_year_tick, "current_year_tick")?,
        world_name: required(world_name, "world_name")?,
        world_folder: required(world_folder, "world_folder")?,
        site_id: required(site_id, "site_id")?,
        citizen_count_total: total,
        citizen_offset: offset,
        complete,
        citizens,
    })
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

pub struct DfHackRpcClient<S> {
    stream: S,
    credentials: BridgeCredentials,
    manifest: BridgeManifest,
    handshake_method_id: i16,
    observation_method_id: i16,
}

impl<S: Read + Write> DfHackRpcClient<S> {
    pub fn negotiate(
        mut stream: S,
        credentials: BridgeCredentials,
        client_name: &str,
        client_version: &str,
    ) -> Result<Self> {
        stream
            .write_all(&encode_handshake_header(REQUEST_MAGIC))
            .map_err(|source| io_error("handshake write", &source))?;
        stream
            .flush()
            .map_err(|source| io_error("handshake flush", &source))?;

        let mut response = [0u8; HANDSHAKE_HEADER_BYTES];
        stream
            .read_exact(&mut response)
            .map_err(|source| io_error("handshake read", &source))?;
        if &response[..8] != RESPONSE_MAGIC {
            return Err(error(
                ErrorCode::AdapterRejected,
                "DFHack handshake response magic is invalid",
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
                "DFHack assigned the same ID to two bridge methods",
            ));
        }

        let handshake_request =
            encode_handshake_request(&credentials, client_name, client_version)?;
        let handshake_reply = Self::call(&mut stream, handshake_method_id, &handshake_request)?;
        let manifest = decode_handshake_reply(&handshake_reply, credentials.nonce())?;

        Ok(Self {
            stream,
            credentials,
            manifest,
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
                "attempted to call a negative DFHack method ID",
            ));
        }
        if payload.len() > MAX_RPC_PAYLOAD_BYTES {
            return Err(error(
                ErrorCode::BudgetExceeded,
                "DFHack request exceeds the 8 MiB client ceiling",
            ));
        }
        let size = i32::try_from(payload.len()).map_err(|_| {
            error(
                ErrorCode::BudgetExceeded,
                "DFHack request length does not fit i32",
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

        loop {
            let mut header = [0u8; MESSAGE_HEADER_BYTES];
            stream
                .read_exact(&mut header)
                .map_err(|source| io_error("reply header read", &source))?;
            let (reply_id, reply_size) = decode_message_header(&header);
            if reply_id == RPC_REPLY_FAIL {
                return Err(error(
                    ErrorCode::AdapterFailure,
                    format!("DFHack method {method_id} failed with code {reply_size}"),
                ));
            }
            if reply_size < 0 {
                return Err(error(
                    ErrorCode::AdapterRejected,
                    "DFHack reply has a negative payload length",
                ));
            }
            let length = usize::try_from(reply_size).map_err(|_| {
                error(
                    ErrorCode::AdapterRejected,
                    "DFHack reply length does not fit usize",
                )
            })?;
            let limit = if reply_id == RPC_REPLY_TEXT {
                MAX_TEXT_NOTIFICATION_BYTES
            } else {
                MAX_RPC_PAYLOAD_BYTES
            };
            if length > limit {
                return Err(error(
                    ErrorCode::BudgetExceeded,
                    format!("DFHack reply exceeds the {limit}-byte bound"),
                ));
            }
            let mut response = vec![0u8; length];
            stream
                .read_exact(&mut response)
                .map_err(|source| io_error("reply payload read", &source))?;
            match reply_id {
                RPC_REPLY_TEXT => continue,
                RPC_REPLY_RESULT => return Ok(response),
                other => {
                    return Err(error(
                        ErrorCode::AdapterRejected,
                        format!("unexpected DFHack reply ID {other}"),
                    ));
                }
            }
        }
    }

    #[must_use]
    pub fn manifest(&self) -> &BridgeManifest {
        &self.manifest
    }

    #[must_use]
    pub const fn method_ids(&self) -> (i16, i16) {
        (self.handshake_method_id, self.observation_method_id)
    }

    pub fn read_observation(
        &mut self,
        offset: u32,
        maximum: u32,
        include_names: bool,
    ) -> Result<ObservationPage> {
        let request =
            encode_observation_request(&self.credentials, offset, maximum, include_names)?;
        let response = Self::call(&mut self.stream, self.observation_method_id, &request)?;
        decode_observation_reply(
            &response,
            self.credentials.nonce(),
            self.manifest.bridge_generation,
            offset,
            maximum,
        )
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

    fn rpc_result(payload: &[u8]) -> Result<Vec<u8>> {
        let size = i32::try_from(payload.len()).map_err(|_| {
            error(
                ErrorCode::BudgetExceeded,
                "test reply length does not fit i32",
            )
        })?;
        let mut bytes = encode_message_header(RPC_REPLY_RESULT, size).to_vec();
        bytes.extend_from_slice(payload);
        Ok(bytes)
    }

    fn bind_reply(id: u32) -> Result<Vec<u8>> {
        let mut writer = ProtoWriter::default();
        writer.uint32(1, id);
        rpc_result(&writer.finish())
    }

    fn handshake_reply(nonce: &[u8], generation: u64) -> Result<Vec<u8>> {
        let mut writer = ProtoWriter::default();
        writer.boolean(1, true);
        writer.string(2, "")?;
        writer.string(3, "")?;
        writer.uint32(4, BRIDGE_PROTOCOL_MAJOR);
        writer.uint32(5, BRIDGE_PROTOCOL_MINOR);
        writer.string(6, "0.1.0")?;
        writer.string(7, "0.51.11-r1")?;
        writer.string(8, "0.51.11")?;
        writer.boolean(9, true);
        writer.boolean(10, true);
        writer.bytes(11, nonce)?;
        writer.key(12, WireType::Varint);
        writer.varint(generation);
        writer.string(13, HANDSHAKE_METHOD)?;
        writer.string(13, OBSERVATION_METHOD)?;
        rpc_result(&writer.finish())
    }

    fn citizen(unit_id: i32, name: &str) -> Result<Vec<u8>> {
        let mut writer = ProtoWriter::default();
        writer.sint32(1, unit_id);
        writer.string(2, name)?;
        writer.string(3, "dwarf")?;
        writer.sint32(4, 4);
        writer.sint32(5, 10 + unit_id);
        writer.sint32(6, 20);
        writer.sint32(7, 30);
        for field in 8..=16 {
            writer.boolean(field, true);
        }
        Ok(writer.finish())
    }

    fn observation_reply(nonce: &[u8], generation: u64, ids: &[i32]) -> Result<Vec<u8>> {
        let mut writer = ProtoWriter::default();
        writer.boolean(1, true);
        writer.string(2, "")?;
        writer.string(3, "")?;
        writer.uint32(4, BRIDGE_PROTOCOL_MAJOR);
        writer.uint32(5, BRIDGE_PROTOCOL_MINOR);
        writer.bytes(6, nonce)?;
        writer.key(7, WireType::Varint);
        writer.varint(generation);
        writer.boolean(8, true);
        writer.boolean(9, true);
        writer.boolean(10, true);
        writer.uint32(11, 105);
        writer.uint32(12, 12345);
        writer.string(13, "The Balanced Realm")?;
        writer.string(14, "region1")?;
        writer.sint32(15, 7);
        writer.uint32(16, u32::try_from(ids.len()).map_err(|_| {
            error(ErrorCode::BudgetExceeded, "test citizen count does not fit u32")
        })?);
        writer.uint32(17, 0);
        writer.boolean(18, true);
        for id in ids {
            writer.bytes(19, &citizen(*id, &format!("Urist {id}"))?)?;
        }
        rpc_result(&writer.finish())
    }

    fn scripted_session(nonce: &[u8], ids: &[i32]) -> Result<ScriptedIo> {
        let generation = 42;
        let mut reads = encode_handshake_header(RESPONSE_MAGIC).to_vec();
        reads.extend_from_slice(&bind_reply(41)?);
        reads.extend_from_slice(&bind_reply(42)?);
        reads.extend_from_slice(&handshake_reply(nonce, generation)?);
        reads.extend_from_slice(&observation_reply(nonce, generation, ids)?);
        Ok(ScriptedIo::new(reads))
    }

    #[test]
    fn message_header_matches_dfhack_native_layout() {
        let header = encode_message_header(0x1234, 0x0102_0304);
        assert_eq!(header, [0x34, 0x12, 0, 0, 0x04, 0x03, 0x02, 0x01]);
    }

    #[test]
    fn negotiate_and_read_a_canonical_page() -> Result<()> {
        let nonce = vec![9; MIN_NONCE_BYTES];
        let credentials = BridgeCredentials::new(vec![7; MIN_BRIDGE_TOKEN_BYTES], nonce.clone())?;
        let stream = scripted_session(&nonce, &[1, 2])?;
        let mut client = DfHackRpcClient::negotiate(stream, credentials, "dfmcp", "0.0.1")?;
        assert_eq!(client.method_ids(), (41, 42));
        assert_eq!(client.manifest().bridge_generation, 42);
        let page = client.read_observation(0, 2, true)?;
        assert!(page.complete);
        assert_eq!(page.citizen_count_total, 2);
        assert_eq!(page.citizens[0].unit_id, 1);
        assert_eq!(page.citizens[1].unit_id, 2);
        Ok(())
    }

    #[test]
    fn nonce_mismatch_is_rejected() -> Result<()> {
        let expected = vec![9; MIN_NONCE_BYTES];
        let response_nonce = vec![8; MIN_NONCE_BYTES];
        let credentials =
            BridgeCredentials::new(vec![7; MIN_BRIDGE_TOKEN_BYTES], expected.clone())?;
        let generation = 42;
        let mut reads = encode_handshake_header(RESPONSE_MAGIC).to_vec();
        reads.extend_from_slice(&bind_reply(41)?);
        reads.extend_from_slice(&bind_reply(42)?);
        reads.extend_from_slice(&handshake_reply(&response_nonce, generation)?);
        let result = DfHackRpcClient::negotiate(
            ScriptedIo::new(reads),
            credentials,
            "dfmcp",
            "0.0.1",
        );
        assert!(result.is_err());
        Ok(())
    }

    #[test]
    fn noncanonical_citizen_order_is_rejected() -> Result<()> {
        let nonce = vec![9; MIN_NONCE_BYTES];
        let credentials = BridgeCredentials::new(vec![7; MIN_BRIDGE_TOKEN_BYTES], nonce.clone())?;
        let stream = scripted_session(&nonce, &[2, 1])?;
        let mut client = DfHackRpcClient::negotiate(stream, credentials, "dfmcp", "0.0.1")?;
        assert!(client.read_observation(0, 2, true).is_err());
        Ok(())
    }

    #[test]
    fn duplicate_required_field_is_rejected() -> Result<()> {
        let mut writer = ProtoWriter::default();
        writer.uint32(1, 5);
        writer.uint32(1, 6);
        assert!(decode_bind_reply(&writer.finish()).is_err());
        Ok(())
    }
}
