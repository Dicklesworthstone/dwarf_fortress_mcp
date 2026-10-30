//! Closed append-only format. Every recovered frontier is recomputed from the
//! complete original plan and every validated transition, never deserialized as
//! an authoritative mutable snapshot.
use super::*;

const MAGIC: &[u8; 8] = b"DFMLDJ01";
const HEADER_PREFIX: usize = 108;
const FRAME_PREFIX: usize = 51;
const FRAME_OVERHEAD: usize = FRAME_PREFIX + 32;

pub(super) struct Header {
    pub raw: Vec<u8>,
    pub id: Digest32,
}
pub(super) fn header(plan: &PreparedPlan, c: &OperationContext, work: &mut Work) -> Result<Header> {
    if plan.anchor.fortress_id != c.anchor.fortress_id {
        return Err(denied());
    }
    work.floor(plan.anchor.tick.get());
    work.current()?;
    work.charge(6 * MAX_PLAN_BYTES)?;
    let (archive, seal) =
        plan_archive::encode_with_check(plan, limits(), || work.current().map(|_| ()))?;
    // Routability is established before a new file header can be published.
    LiveDispatchCoordinator::new(plan.clone())?;
    let mut raw = MAGIC.to_vec();
    raw.extend_from_slice(&c.session_id.get().to_be_bytes());
    raw.extend_from_slice(&c.request_id.get().to_be_bytes());
    raw.extend_from_slice(seal.plan_digest.as_bytes());
    raw.extend_from_slice(seal.object_digest.as_bytes());
    raw.extend_from_slice(&(archive.len() as u32).to_be_bytes());
    raw.extend_from_slice(&archive);
    let id = hash(b"dfmcp-live-dispatch-journal/1\0", &raw);
    raw.extend_from_slice(id.as_bytes());
    work.current()?;
    Ok(Header { raw, id })
}
pub(super) fn tail(bytes: &[u8]) -> Result<Digest32> {
    Ok(Digest32::from_bytes(
        bytes
            .get(bytes.len().saturating_sub(32)..)
            .ok_or_else(corrupt)?
            .try_into()
            .map_err(|_| corrupt())?,
    ))
}
fn body(event: &Event) -> (u8, u32, Vec<u8>) {
    let (kind, step, digest, optional) = match *event {
        Event::Prepare(step) => (1, step.get(), None, false),
        Event::Prepared(step, digest) => (2, step.get(), Some(digest), false),
        Event::Dispatch(step) => (3, step.get(), None, false),
        Event::Applied(step, digest) => (4, step.get(), Some(digest), false),
        Event::Failed(step, digest) => (5, step.get(), digest, true),
        Event::Cancelled(step, digest) => (6, step.get(), digest, true),
        Event::Absent(step) => (7, step.get(), None, false),
        Event::RecoveredPrepared(step, digest) => (8, step.get(), Some(digest), false),
        Event::Unknown(step) => (9, step.get(), None, false),
        Event::Cancel(step) => (10, step.get(), None, false),
        Event::Floor => (11, u32::MAX, None, false),
    };
    let mut bytes = Vec::new();
    if optional {
        bytes.push(u8::from(digest.is_some()));
    }
    if let Some(digest) = digest {
        bytes.extend_from_slice(digest.as_bytes());
    }
    (kind, step, bytes)
}
pub(super) fn frame(
    sequence: usize,
    previous: Digest32,
    tick: u64,
    event: &Event,
) -> Result<Vec<u8>> {
    if sequence >= MAX_FRAMES {
        return Err(bounded());
    }
    let (kind, step, body) = body(event);
    let mut raw = Vec::with_capacity(FRAME_OVERHEAD + body.len());
    raw.extend_from_slice(previous.as_bytes());
    raw.extend_from_slice(&(sequence as u32).to_be_bytes());
    raw.push(kind);
    raw.extend_from_slice(&step.to_be_bytes());
    raw.extend_from_slice(&tick.to_be_bytes());
    raw.extend_from_slice(&(body.len() as u16).to_be_bytes());
    raw.extend_from_slice(&body);
    let digest = hash(b"dfmcp-live-dispatch-frame/1\0", &raw);
    raw.extend_from_slice(digest.as_bytes());
    require(raw.len() <= MAX_FRAME_BYTES)?;
    Ok(raw)
}
struct Reader<'a>(&'a [u8]);
impl<'a> Reader<'a> {
    fn take(&mut self, len: usize) -> Result<&'a [u8]> {
        let value = self.0.get(..len).ok_or_else(corrupt)?;
        self.0 = self.0.get(len..).ok_or_else(corrupt)?;
        Ok(value)
    }
    fn array<const N: usize>(&mut self) -> Result<[u8; N]> {
        self.take(N)?.try_into().map_err(|_| corrupt())
    }
    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_be_bytes(self.array()?))
    }
    fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_be_bytes(self.array()?))
    }
    fn digest(&mut self) -> Result<Digest32> {
        Ok(Digest32::from_bytes(self.array()?))
    }
}
fn decode(kind: u8, step: u32, payload: &[u8]) -> Result<Event> {
    let mut reader = Reader(payload);
    let step = StepId::new(step);
    let optional = |reader: &mut Reader<'_>| -> Result<Option<Digest32>> {
        match reader.take(1)?[0] {
            0 => Ok(None),
            1 => Ok(Some(reader.digest()?)),
            _ => Err(corrupt()),
        }
    };
    let event = match kind {
        1 => Event::Prepare(step),
        2 => Event::Prepared(step, reader.digest()?),
        3 => Event::Dispatch(step),
        4 => Event::Applied(step, reader.digest()?),
        5 => Event::Failed(step, optional(&mut reader)?),
        6 => Event::Cancelled(step, optional(&mut reader)?),
        7 => Event::Absent(step),
        8 => Event::RecoveredPrepared(step, reader.digest()?),
        9 => Event::Unknown(step),
        10 => Event::Cancel(step),
        11 if step.get() == u32::MAX => Event::Floor,
        _ => return Err(corrupt()),
    };
    require(reader.0.is_empty())?;
    Ok(event)
}
pub(super) struct Replay {
    pub compiled: LiveDispatchCoordinator,
    pub seal: PlanArchiveSeal,
    pub archive: Vec<u8>,
    pub id: Digest32,
    pub head: Digest32,
    pub heads: Vec<Digest32>,
    pub entries: Vec<Entry>,
    pub high_tick: u64,
    pub frames: usize,
}
pub(super) fn replay(raw: &[u8], work: &mut Work) -> Result<Replay> {
    require(raw.len() >= HEADER_PREFIX + 32 && raw.len() <= MAX_STORE_BYTES)?;
    work.charge(raw.len().saturating_mul(5))?;
    let mut reader = Reader(raw);
    require(reader.take(8)? == MAGIC)?;
    let _original_session = reader.array::<16>()?;
    let _original_request = reader.array::<16>()?;
    let seal = PlanArchiveSeal {
        plan_digest: reader.digest()?,
        object_digest: reader.digest()?,
    };
    let length = reader.u32()? as usize;
    require(length <= MAX_PLAN_BYTES)?;
    let archive = reader.take(length)?.to_vec();
    let id = reader.digest()?;
    require(
        id != Digest32::ZERO
            && id
                == hash(
                    b"dfmcp-live-dispatch-journal/1\0",
                    &raw[..HEADER_PREFIX + length],
                ),
    )?;
    let plan =
        plan_archive::decode_with_check(&archive, seal, limits(), || work.current().map(|_| ()))?;
    let compiled = LiveDispatchCoordinator::new(plan)?;
    let mut entries: Vec<_> = compiled
        .entries
        .iter()
        .map(|entry| Entry {
            step: entry.step,
            family: entry.family,
            idempotency_key: entry.idempotency_key.clone(),
            depends_on: entry.depends_on.clone(),
            phase: Phase::Pending,
            preparation: None,
            receipt: None,
            dispatch_attempted: false,
        })
        .collect();
    let mut high_tick = compiled.plan.anchor.tick.get();
    let mut frames = 0usize;
    let mut head = id;
    let mut heads = vec![id];
    while !reader.0.is_empty() {
        work.current()?;
        if frames >= MAX_FRAMES {
            return Err(bounded());
        }
        let remaining = reader.0;
        require(remaining.len() >= FRAME_OVERHEAD)?;
        require(reader.digest()? == head && reader.u32()? as usize == frames)?;
        let kind = reader.take(1)?[0];
        let step = reader.u32()?;
        let tick = reader.u64()?;
        require(tick >= high_tick)?;
        let length = usize::from(u16::from_be_bytes(reader.array()?));
        require(length + FRAME_OVERHEAD <= MAX_FRAME_BYTES)?;
        let event = decode(kind, step, reader.take(length)?)?;
        let next_head = reader.digest()?;
        require(
            next_head
                == hash(
                    b"dfmcp-live-dispatch-frame/1\0",
                    &remaining[..FRAME_PREFIX + length],
                ),
        )?;
        entries = transition(&entries, &event)?;
        high_tick = tick;
        head = next_head;
        heads.push(head);
        frames += 1;
    }
    work.current()?;
    Ok(Replay {
        compiled,
        seal,
        archive,
        id,
        head,
        heads,
        entries,
        high_tick,
        frames,
    })
}
