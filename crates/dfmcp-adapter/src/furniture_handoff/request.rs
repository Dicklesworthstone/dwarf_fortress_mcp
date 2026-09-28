//! Closed, bounded furniture requests compatible with the Python allocator.
//!
//! Normalization is pure and structurally bounded. A request names intent only;
//! its digest supplies neither observation evidence nor mutation authority.

use dfmcp_core::{DfmcpError, Digest32, ErrorCode, Result};

use crate::furniture_allocation::{self, Kind, Request, Slot};

pub const MAX_REQUEST_BYTES: usize = 16_384;
const SCHEMA: &str = "dfmcp.furniture-request/1";

fn invalid(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::InvalidRequest, message)
}
fn require(ok: bool, message: &str) -> Result<()> {
    if ok { Ok(()) } else { Err(invalid(message)) }
}

/// Immutable normalized intent with Python-compatible ASCII JSON and digest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FurnitureRequest {
    folder: String,
    site: u32,
    request: Request,
    bytes: Vec<u8>,
    digest: Digest32,
}

impl FurnitureRequest {
    pub fn new(folder: String, site: u32, request: Request) -> Result<Self> {
        require(
            (1..=512).contains(&folder.len()) && !folder.contains('\0'),
            "invalid furniture request fortress folder",
        )?;
        require(site <= i32::MAX as u32, "invalid furniture request site")?;
        // The model bounds every collection and string before cloning/sorting;
        // pure normalization has no I/O, authority or external work allowance.
        let request = furniture_allocation::normalize(&request, &mut || Ok(()))?;
        let bytes = canonical(&folder, site, &request)?;
        let mut digest_input = b"dfmcp-furniture-request/1\0".to_vec();
        digest_input.extend_from_slice(&bytes);
        let digest = Digest32::of_bytes(&digest_input);
        Ok(Self {
            folder,
            site,
            request,
            bytes,
            digest,
        })
    }

    pub fn decode(raw: &[u8]) -> Result<Self> {
        require(
            (1..=MAX_REQUEST_BYTES).contains(&raw.len()),
            "furniture request exceeds 16 KiB",
        )?;
        std::str::from_utf8(raw).map_err(|_| invalid("invalid furniture request UTF-8"))?;
        check_depth(raw)?;
        let mut parser = Json(raw);
        parser.symbol(b'{')?;
        let (mut schema, mut folder, mut site, mut slots, mut excluded) =
            (None, None, None, None, None);
        if !parser.consume(b'}') {
            loop {
                let field = parser.string(16)?;
                parser.symbol(b':')?;
                match field.as_str() {
                    "schema" if schema.is_none() => schema = Some(parser.string(32)?),
                    "world_folder" if folder.is_none() => folder = Some(parser.string(512)?),
                    "site" if site.is_none() => site = Some(parser.unsigned()?),
                    "slots" if slots.is_none() => slots = Some(parser.slots()?),
                    "excluded_items" if excluded.is_none() => excluded = Some(parser.excluded()?),
                    _ => return Err(invalid("unknown or duplicate furniture request field")),
                }
                if parser.consume(b'}') {
                    break;
                }
                parser.symbol(b',')?;
            }
        }
        parser.whitespace();
        require(parser.0.is_empty(), "trailing furniture request data")?;
        require(schema.as_deref() == Some(SCHEMA), "unsupported furniture request schema")?;
        let missing = || invalid("furniture request lacks required fields");
        Self::new(
            folder.ok_or_else(missing)?,
            site.ok_or_else(missing)?,
            Request {
                slots: slots.ok_or_else(missing)?,
                excluded_items: excluded.unwrap_or_default(),
            },
        )
    }

    pub fn folder(&self) -> &str {
        &self.folder
    }
    pub fn site(&self) -> u32 {
        self.site
    }
    pub fn request(&self) -> &Request {
        &self.request
    }
    pub fn canonical_bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub fn digest(&self) -> Digest32 {
        self.digest
    }
}

fn canonical(folder: &str, site: u32, request: &Request) -> Result<Vec<u8>> {
    let mut out = String::from("{\"excluded_items\":[");
    for (index, identity) in request.excluded_items.iter().enumerate() {
        if index != 0 {
            out.push(',');
        }
        out.push_str(&identity.to_string());
        canonical_bound(&out)?;
    }
    out.push_str(&format!("],\"schema\":\"{SCHEMA}\",\"site\":{site},\"slots\":["));
    for (index, slot) in request.slots.iter().enumerate() {
        if index != 0 {
            out.push(',');
        }
        out.push_str("{\"after\":[");
        for (index, dependency) in slot.after.iter().enumerate() {
            if index != 0 {
                out.push(',');
            }
            // normalize restricts every dependency and name to safe ASCII.
            out.push('"');
            out.push_str(dependency);
            out.push('"');
            canonical_bound(&out)?;
        }
        let material = match slot.material {
            Some((kind, index)) => format!("[{kind},{index}]"),
            None => "null".to_owned(),
        };
        let subtype = slot.subtype.map_or_else(|| "null".to_owned(), |value| value.to_string());
        let [x, y, z] = slot.target;
        out.push_str(&format!(
            "],\"kind\":\"{}\",\"material\":{material},\"max_distance\":{},\"name\":\"{}\",\"subtype\":{subtype},\"target\":[{x},{y},{z}]}}",
            slot.kind.as_str(), slot.max_distance, slot.name
        ));
        canonical_bound(&out)?;
    }
    out.push_str("],\"world_folder\":");
    ascii_string(folder, &mut out);
    out.push('}');
    canonical_bound(&out)?;
    Ok(out.into_bytes())
}

fn canonical_bound(value: &str) -> Result<()> {
    require(value.len() <= MAX_REQUEST_BYTES, "normalized furniture request exceeds 16 KiB")
}

/// Match json.dumps(..., ensure_ascii=True), including UTF-16 surrogate pairs.
fn ascii_string(value: &str, out: &mut String) {
    out.push('"');
    for character in value.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{20}'..='\u{7e}' => out.push(character),
            _ => {
                for unit in character.encode_utf16(&mut [0; 2]) {
                    out.push_str(&format!("\\u{unit:04x}"));
                }
            }
        }
    }
    out.push('"');
}

fn check_depth(raw: &[u8]) -> Result<()> {
    let (mut depth, mut quoted, mut escaped) = (0u8, false, false);
    for &byte in raw {
        if quoted {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                quoted = false;
            }
        } else {
            match byte {
                b'"' => quoted = true,
                b'[' | b'{' => {
                    depth += 1;
                    require(depth <= 8, "furniture request nesting exceeds eight levels")?;
                }
                b']' | b'}' => {
                    depth = depth.checked_sub(1)
                        .ok_or_else(|| invalid("invalid furniture request nesting"))?;
                }
                _ => {}
            }
        }
    }
    require(depth == 0 && !quoted, "incomplete furniture request JSON")
}

/// Schema-specific parsing avoids arbitrary trees and bounds each decoded field.
struct Json<'a>(&'a [u8]);
impl Json<'_> {
    fn whitespace(&mut self) {
        while self.0.first().is_some_and(|byte| matches!(byte, b' ' | b'\t' | b'\r' | b'\n')) {
            self.0 = &self.0[1..];
        }
    }
    fn consume(&mut self, byte: u8) -> bool {
        self.whitespace();
        if self.0.first() == Some(&byte) {
            self.0 = &self.0[1..];
            true
        } else {
            false
        }
    }
    fn symbol(&mut self, byte: u8) -> Result<()> {
        require(self.consume(byte), "invalid closed furniture request JSON")
    }
    fn next_byte(&mut self) -> Result<u8> {
        let (&byte, remaining) = self.0.split_first()
            .ok_or_else(|| invalid("incomplete furniture request JSON"))?;
        self.0 = remaining;
        Ok(byte)
    }
    fn hex_unit(&mut self) -> Result<u16> {
        let mut code = 0u16;
        for _ in 0..4 {
            let digit = char::from(self.next_byte()?).to_digit(16)
                .ok_or_else(|| invalid("invalid furniture request Unicode escape"))?;
            code = code * 16 + digit as u16;
        }
        Ok(code)
    }
    fn escaped_unicode(&mut self) -> Result<char> {
        let first = self.hex_unit()?;
        let code = if (0xd800..=0xdbff).contains(&first) {
            require(self.next_byte()? == b'\\' && self.next_byte()? == b'u',
                "unpaired furniture request Unicode surrogate")?;
            let second = self.hex_unit()?;
            require((0xdc00..=0xdfff).contains(&second),
                "invalid furniture request Unicode surrogate pair")?;
            0x1_0000 + ((u32::from(first) - 0xd800) << 10) + u32::from(second) - 0xdc00
        } else {
            u32::from(first)
        };
        char::from_u32(code).ok_or_else(|| invalid("unpaired furniture request Unicode surrogate"))
    }
    fn string(&mut self, maximum: usize) -> Result<String> {
        self.symbol(b'"')?;
        let mut out = Vec::new();
        loop {
            let byte = self.next_byte()?;
            if byte == b'"' {
                return String::from_utf8(out).map_err(|_| invalid("invalid furniture request string UTF-8"));
            }
            require(byte >= 0x20, "unescaped furniture request string control")?;
            if byte == b'\\' {
                let escaped = match self.next_byte()? {
                    b'"' => '"',
                    b'\\' => '\\',
                    b'/' => '/',
                    b'b' => '\u{8}',
                    b'f' => '\u{c}',
                    b'n' => '\n',
                    b'r' => '\r',
                    b't' => '\t',
                    b'u' => self.escaped_unicode()?,
                    _ => return Err(invalid("invalid furniture request string escape")),
                };
                out.extend_from_slice(escaped.encode_utf8(&mut [0; 4]).as_bytes());
            } else {
                out.push(byte);
            }
            require(out.len() <= maximum, "furniture request string exceeds bound")?;
        }
    }
    fn number(&mut self) -> Result<i32> {
        self.whitespace();
        let negative = if self.0.first() == Some(&b'-') {
            self.0 = &self.0[1..];
            true
        } else {
            false
        };
        let first = self.next_byte()?;
        require(first.is_ascii_digit(), "furniture request values require integers")?;
        let mut value = u32::from(first - b'0');
        while self.0.first().is_some_and(u8::is_ascii_digit) {
            require(first != b'0', "leading zero in furniture request integer")?;
            let digit = u32::from(self.next_byte()? - b'0');
            value = value.checked_mul(10).and_then(|value| value.checked_add(digit))
                .ok_or_else(|| invalid("furniture request integer exceeds bound"))?;
        }
        require(!self.0.first().is_some_and(|byte| matches!(byte, b'.' | b'e' | b'E')),
            "floating-point furniture request values are forbidden")?;
        let signed = if negative { -i64::from(value) } else { i64::from(value) };
        i32::try_from(signed).map_err(|_| invalid("furniture request integer exceeds bound"))
    }
    fn unsigned(&mut self) -> Result<u32> {
        u32::try_from(self.number()?).map_err(|_| invalid("negative furniture request integer"))
    }
    fn null(&mut self) -> bool {
        self.whitespace();
        if self.0.starts_with(b"null") {
            self.0 = &self.0[4..];
            true
        } else {
            false
        }
    }
    fn target(&mut self) -> Result<[u32; 3]> {
        self.symbol(b'[')?;
        let x = self.unsigned()?;
        self.symbol(b',')?;
        let y = self.unsigned()?;
        self.symbol(b',')?;
        let z = self.unsigned()?;
        self.symbol(b']')?;
        Ok([x, y, z])
    }
    fn material(&mut self) -> Result<Option<(i32, i32)>> {
        if self.null() {
            return Ok(None);
        }
        self.symbol(b'[')?;
        let kind = self.number()?;
        self.symbol(b',')?;
        let index = self.number()?;
        self.symbol(b']')?;
        Ok(Some((kind, index)))
    }
    fn dependencies(&mut self) -> Result<Vec<String>> {
        self.symbol(b'[')?;
        let mut out = Vec::new();
        if !self.consume(b']') {
            loop {
                require(out.len() < furniture_allocation::MAX_SLOTS - 1,
                    "furniture dependency bound exceeded")?;
                out.push(self.string(48)?);
                if self.consume(b']') {
                    break;
                }
                self.symbol(b',')?;
            }
        }
        Ok(out)
    }
    fn excluded(&mut self) -> Result<Vec<u32>> {
        self.symbol(b'[')?;
        let mut out = Vec::new();
        if !self.consume(b']') {
            loop {
                require(out.len() < furniture_allocation::MAX_EXCLUDED,
                    "furniture exclusion bound exceeded")?;
                out.push(self.unsigned()?);
                if self.consume(b']') {
                    break;
                }
                self.symbol(b',')?;
            }
        }
        Ok(out)
    }
    fn slots(&mut self) -> Result<Vec<Slot>> {
        self.symbol(b'[')?;
        let mut out = Vec::new();
        if !self.consume(b']') {
            loop {
                require(out.len() < furniture_allocation::MAX_SLOTS,
                    "furniture request exceeds 32 slots")?;
                out.push(self.slot()?);
                if self.consume(b']') {
                    break;
                }
                self.symbol(b',')?;
            }
        }
        Ok(out)
    }
    fn slot(&mut self) -> Result<Slot> {
        self.symbol(b'{')?;
        let (mut name, mut kind, mut target, mut after, mut material, mut subtype, mut distance) =
            (None, None, None, None, None, None, None);
        if !self.consume(b'}') {
            loop {
                let field = self.string(16)?;
                self.symbol(b':')?;
                match field.as_str() {
                    "name" if name.is_none() => name = Some(self.string(48)?),
                    "kind" if kind.is_none() => kind = Some(match self.string(8)?.as_str() {
                        "bed" => Kind::Bed,
                        "chair" => Kind::Chair,
                        "table" => Kind::Table,
                        _ => return Err(invalid("unsupported furniture request kind")),
                    }),
                    "target" if target.is_none() => target = Some(self.target()?),
                    "after" if after.is_none() => after = Some(self.dependencies()?),
                    "material" if material.is_none() => material = Some(self.material()?),
                    "subtype" if subtype.is_none() => subtype = Some(if self.null() { None } else { Some(self.number()?) }),
                    "max_distance" if distance.is_none() => distance = Some(self.unsigned()?),
                    _ => return Err(invalid("unknown or duplicate furniture request slot field")),
                }
                if self.consume(b'}') {
                    break;
                }
                self.symbol(b',')?;
            }
        }
        let missing = || invalid("furniture request slot lacks required fields");
        Ok(Slot {
            name: name.ok_or_else(missing)?,
            kind: kind.ok_or_else(missing)?,
            target: target.ok_or_else(missing)?,
            after: after.unwrap_or_default(),
            material: material.unwrap_or_default(),
            subtype: subtype.unwrap_or_default(),
            max_distance: distance.unwrap_or(furniture_allocation::MAX_DISTANCE),
        })
    }
}

#[cfg(test)]
#[path = "request_tests.rs"]
mod tests;
