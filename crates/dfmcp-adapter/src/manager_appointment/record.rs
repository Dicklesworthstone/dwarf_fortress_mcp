use super::*;
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    key: String,
    pub before: Capture,
    pub digest: Digest32,
    pub token: [u8; 16],
}
impl Plan {
    pub fn new(key: &str, before: Capture) -> Result<Self> {
        validate_key(key)?;
        require(
            before.eligible() && before.encode() == before.bytes(),
            "manager plan requires exact eligible capture",
        )?;
        let mut b = before.selection.encode();
        b.extend_from_slice(before.witness().as_bytes());
        let digest = hash(b"dfmcp-manager-plan/1", &b);
        let mut b = Vec::new();
        field(&mut b, key.as_bytes());
        b.extend_from_slice(digest.as_bytes());
        let mut token = [0; 16];
        token.copy_from_slice(&hash(b"dfmcp-manager-token/1", &b).as_bytes()[..16]);
        Ok(Self {
            key: key.to_owned(),
            before,
            digest,
            token,
        })
    }
    pub fn key(&self) -> &str {
        &self.key
    }
    pub fn encode(&self) -> Vec<u8> {
        let mut b = self.before.protocol.magic(b'L').to_vec();
        field(&mut b, self.key.as_bytes());
        blob(&mut b, self.before.bytes());
        b
    }
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let mut r = Reader(bytes);
        let protocol = ManagerProtocol::decode_magic(r.take(8)?, b'L')?;
        let key = text(&mut r, 128)?;
        let before = Capture::decode(read_blob(&mut r, MAX_CAPTURE_BYTES)?)?;
        require(
            before.protocol == protocol,
            "manager plan generation differs",
        )?;
        r.finish()?;
        Self::new(&key, before)
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Insertion {
    pub unit: u32,
    pub figure: u32,
    pub group: i32,
    pub assignment: i32,
    pub event: i32,
    pub lookup: bool,
    pub noble: bool,
}
impl Insertion {
    fn read(r: &mut Reader<'_>, p: &Plan) -> Result<Self> {
        require(
            r.take(8)? == p.before.protocol.magic(b'I'),
            "manager proof generation differs",
        )?;
        let out = Self {
            unit: r.u32()?,
            figure: r.u32()?,
            group: i32(r)?,
            assignment: i32(r)?,
            event: i32(r)?,
            lookup: r.boolean()?,
            noble: r.boolean()?,
        };
        require(
            out.unit == p.before.selection.unit
                && out.figure == p.before.selection.figure
                && out.group == p.before.group.id
                && p.before
                    .target()
                    .is_some_and(|(_, a)| a.id == out.assignment)
                && out.event == p.before.next_event as i32
                && out.lookup
                && out.noble,
            "manager dual getter proof differs",
        )?;
        Ok(out)
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    raw: Vec<u8>,
    pub plan: Plan,
    pub phase: u8,
    pub reason: u8,
    pub after: Option<Capture>,
    pub insertion: Option<Insertion>,
    pub receipt: Digest32,
}
impl Record {
    pub fn decode(bytes: &[u8], plan: &Plan) -> Result<Self> {
        require(
            (32..=MAX_RECORD_BYTES).contains(&bytes.len()),
            "manager record bound",
        )?;
        let (body, checksum) = bytes.split_at(bytes.len() - 32);
        let receipt = hash(b"dfmcp-manager-receipt/1", body);
        require(receipt.as_bytes() == checksum, "manager record checksum")?;
        let mut r = Reader(body);
        require(
            r.take(8)? == plan.before.protocol.magic(b'R') && text(&mut r, 128)? == plan.key(),
            "manager record original key",
        )?;
        require(
            r.take(32)? == plan.digest.as_bytes() && r.take(16)? == plan.token,
            "manager record plan/token",
        )?;
        require(
            Capture::decode(read_blob(&mut r, MAX_CAPTURE_BYTES)?)? == plan.before,
            "manager record before differs",
        )?;
        let phase = r.byte()?;
        let reason = r.byte()?;
        let after = if r.boolean()? {
            Some(Capture::decode(read_blob(&mut r, MAX_CAPTURE_BYTES)?)?)
        } else {
            None
        };
        let insertion = if r.boolean()? {
            Some(Insertion::read(&mut r, plan)?)
        } else {
            None
        };
        r.finish()?;
        require(
            match phase {
                0 => reason == 0 && after.is_none() && insertion.is_none(),
                1 => reason == 5 && after.is_none() && insertion.is_none(),
                2 => {
                    reason == 0
                        && insertion.is_some()
                        && after.as_ref() == Some(&plan.before.expected_after()?)
                }
                3 => (1..=3).contains(&reason) && after.is_none() && insertion.is_none(),
                4 => reason == 4 && after.is_none() && insertion.is_none(),
                _ => false,
            },
            "manager impossible phase or incomplete transaction proof",
        )?;
        Ok(Self {
            raw: bytes.to_vec(),
            plan: plan.clone(),
            phase,
            reason,
            after,
            insertion,
            receipt,
        })
    }
    pub fn bytes(&self) -> &[u8] {
        &self.raw
    }
    pub fn complete(&self) -> bool {
        self.phase == 2
    }
}
