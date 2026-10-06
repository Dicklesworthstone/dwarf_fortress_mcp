//! Closed bounded retained-handoff codec. The exact plan is derived, not copied.
use super::*;
use dfmcp_core::{EntityId, FortressId, GameTick, ObservationCursor};

const MAGIC: &[u8; 8] = b"DFMFHA01";
fn field(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
    out.extend_from_slice(bytes);
}
impl Handoff {
    pub(super) fn encode(&self) -> Vec<u8> {
        let mut out = MAGIC.to_vec();
        field(&mut out, self.request.canonical_bytes());
        field(&mut out, self.source.endpoint.to_string().as_bytes());
        let source = &self.source;
        out.extend_from_slice(&source.generation.to_be_bytes());
        field(&mut out, source.df_version.as_bytes());
        field(&mut out, source.dfhack_version.as_bytes());
        for value in [
            source.anchor.fortress_id.get(),
            source.anchor.cursor.epoch,
            source.anchor.cursor.sequence,
            source.anchor.tick.get(),
        ] {
            out.extend_from_slice(&value.to_be_bytes());
        }
        for digest in [
            source.anchor.state_hash,
            source.source_digest,
            source.capture_digest,
        ] {
            out.extend_from_slice(digest.as_bytes());
        }
        for value in [
            source.capture_bytes,
            source.next_job_id,
            source.next_building_id,
            source.next_item_id,
            self.items.len() as u32,
        ] {
            out.extend_from_slice(&value.to_be_bytes());
        }
        for item in &self.items {
            // Lexical slot order is fixed by the embedded normalized request.
            let candidate = item.candidate;
            for value in [
                candidate.native_id,
                item.native_type,
                candidate.position[0],
                candidate.position[1],
                candidate.position[2],
            ] {
                out.extend_from_slice(&value.to_be_bytes());
            }
            for value in [
                candidate.material_type,
                candidate.material_index,
                candidate.subtype,
            ] {
                out.extend_from_slice(&value.to_be_bytes());
            }
            out.extend_from_slice(&item.handle.entity_id.get().to_be_bytes());
            out.extend_from_slice(&item.handle.generation.to_be_bytes());
            out.extend_from_slice(&item.handle.revision.to_be_bytes());
            out.extend_from_slice(&item.distance.to_be_bytes());
        }
        out
    }

    /// Validate historical immutable evidence. This does not recreate the full
    /// inventory or independently recertify its original global optimum.
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        require(
            bytes.len() <= MAX_HANDOFF_BYTES,
            "furniture handoff exceeds bound",
        )?;
        let mut reader = Reader(bytes);
        require(reader.take(8)? == MAGIC, "wrong furniture handoff format")?;
        let request = FurnitureRequest::decode(reader.field(MAX_REQUEST_BYTES)?)?;
        let endpoint = reader
            .text(64)?
            .parse()
            .map_err(|_| invalid("invalid handoff endpoint"))?;
        let generation = reader.u64()?;
        let df_version = reader.text(128)?;
        let dfhack_version = reader.text(128)?;
        let fortress_id = FortressId::new(reader.u64()?);
        let cursor = ObservationCursor {
            epoch: reader.u64()?,
            sequence: reader.u64()?,
        };
        let tick = GameTick(reader.u64()?);
        let anchor = StateAnchor {
            fortress_id,
            cursor,
            tick,
            state_hash: reader.digest()?,
        };
        let source = Source {
            endpoint,
            generation,
            df_version,
            dfhack_version,
            anchor,
            source_digest: reader.digest()?,
            capture_digest: reader.digest()?,
            capture_bytes: reader.u32()?,
            next_job_id: reader.u32()?,
            next_building_id: reader.u32()?,
            next_item_id: reader.u32()?,
        };
        let count = reader.u32()? as usize;
        require(
            count == request.request().slots.len(),
            "handoff item count differs from complete request",
        )?;
        let mut items = Vec::with_capacity(count);
        for slot in &request.request().slots {
            let native_id = reader.u32()?;
            let native_type = reader.u32()?;
            let position = [reader.u32()?, reader.u32()?, reader.u32()?];
            let candidate = Candidate {
                native_id,
                kind: slot.kind,
                position,
                material_type: reader.i32()?,
                material_index: reader.i32()?,
                subtype: reader.i32()?,
            };
            let handle = AnalysisHandle {
                entity_id: EntityId::new(reader.u64()?),
                generation: reader.u32()?,
                revision: reader.u64()?,
            };
            items.push(SelectedItem {
                slot: slot.name.clone(),
                candidate,
                native_type,
                handle,
                distance: reader.u32()?,
            });
        }
        require(reader.0.is_empty(), "trailing handoff bytes")?;
        let out = Self::assemble(request, source, items)?;
        require(out.canonical_bytes() == bytes, "noncanonical handoff bytes")?;
        Ok(out)
    }
}

struct Reader<'a>(&'a [u8]);
impl<'a> Reader<'a> {
    fn take(&mut self, length: usize) -> Result<&'a [u8]> {
        require(length <= self.0.len(), "truncated furniture handoff")?;
        let (value, rest) = self.0.split_at(length);
        self.0 = rest;
        Ok(value)
    }
    fn array<const N: usize>(&mut self) -> Result<[u8; N]> {
        self.take(N)?
            .try_into()
            .map_err(|_| invalid("invalid handoff field width"))
    }
    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_be_bytes(self.array()?))
    }
    fn i32(&mut self) -> Result<i32> {
        Ok(i32::from_be_bytes(self.array()?))
    }
    fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_be_bytes(self.array()?))
    }
    fn digest(&mut self) -> Result<Digest32> {
        Ok(Digest32::from_bytes(self.array()?))
    }
    fn field(&mut self, maximum: usize) -> Result<&'a [u8]> {
        let length = self.u32()? as usize;
        require(length <= maximum, "handoff field exceeds bound")?;
        self.take(length)
    }
    fn text(&mut self, maximum: usize) -> Result<String> {
        let value = std::str::from_utf8(self.field(maximum)?)
            .map_err(|_| invalid("invalid handoff UTF-8"))?;
        require(
            !value.is_empty() && !value.contains('\0'),
            "invalid handoff text",
        )?;
        Ok(value.to_owned())
    }
}
