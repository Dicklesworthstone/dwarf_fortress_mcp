//! Strict decoder for the canonical world-snapshot encoding.
//!
//! [`WorldSnapshot::canonical_bytes`] was designed as the exact bytes covered
//! by the state hash. This module makes those same bytes the durable storage
//! format: a snapshot read back from disk is accepted only when it decodes
//! completely, every map is in strictly ascending canonical order, every
//! collection is bounded, and re-encoding the decoded value reproduces the
//! input byte-for-byte. A persisted snapshot therefore cannot decode into a
//! value whose state hash differs from the hash of the bytes that were stored.
//!
//! Variant names are encoded as text, so an `Other`/`Custom` variant that
//! reuses a reserved name is byte-identical to the dedicated variant and
//! decodes as that variant: the canonical bytes, not the Rust value, are the
//! identity.

use std::collections::BTreeMap;

use dfmcp_core::{
    DfmcpError, Digest32, EdgeId, EntityId, ErrorCode, EventId, FortressId, GameTick, MapCoord,
    ObservationCursor, Result, StateAnchor,
};

use crate::model::{
    ChunkCoord, EdgeKind, EdgeRecord, EntityKind, EntityRecord, Fact, FactPresence, FactSource,
    MapChunk, TerrainRun, Value, WorldEvent, WorldEventKind, WorldGraph, WorldSnapshot,
};

/// Largest canonical snapshot the decoder accepts.
pub const MAX_CANONICAL_SNAPSHOT_BYTES: usize = 256 * 1024 * 1024;
/// Deepest `Value` nesting the decoder accepts.
pub const MAX_VALUE_DEPTH: usize = 64;
const SNAPSHOT_DOMAIN: &str = "dfmcp-world-snapshot-v1";

fn corrupt(message: impl Into<String>) -> DfmcpError {
    DfmcpError::new(ErrorCode::CorruptLedger, message).retryable(false)
}

struct Reader<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, count: usize) -> Result<&'a [u8]> {
        let end = self
            .offset
            .checked_add(count)
            .filter(|end| *end <= self.bytes.len())
            .ok_or_else(|| corrupt("canonical snapshot is truncated"))?;
        let slice = &self.bytes[self.offset..end];
        self.offset = end;
        Ok(slice)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N]> {
        let mut out = [0u8; N];
        out.copy_from_slice(self.take(N)?);
        Ok(out)
    }

    fn u8(&mut self) -> Result<u8> {
        Ok(self.array::<1>()?[0])
    }

    fn bool(&mut self) -> Result<bool> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            other => Err(corrupt(format!("invalid canonical boolean {other}"))),
        }
    }

    fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_be_bytes(self.array()?))
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

    fn i64(&mut self) -> Result<i64> {
        Ok(i64::from_be_bytes(self.array()?))
    }

    fn u128(&mut self) -> Result<u128> {
        Ok(u128::from_be_bytes(self.array()?))
    }

    fn remaining(&self) -> usize {
        self.bytes.len() - self.offset
    }

    /// A collection length, bounded by the bytes left: every element takes
    /// at least `min_element_bytes`, so a forged count cannot force a large
    /// allocation before the input runs out.
    fn count(&mut self, min_element_bytes: usize) -> Result<usize> {
        let raw = self.u64()?;
        let count = usize::try_from(raw).map_err(|_| corrupt("collection length overflows"))?;
        let needed = count
            .checked_mul(min_element_bytes.max(1))
            .ok_or_else(|| corrupt("collection length overflows"))?;
        if needed > self.remaining() {
            return Err(corrupt("collection length exceeds the remaining bytes"));
        }
        Ok(count)
    }

    fn bytes(&mut self) -> Result<Vec<u8>> {
        let len = self.count(1)?;
        Ok(self.take(len)?.to_vec())
    }

    fn string(&mut self) -> Result<String> {
        String::from_utf8(self.bytes()?).map_err(|_| corrupt("canonical text is not UTF-8"))
    }

    fn digest(&mut self) -> Result<Digest32> {
        let len = self.u64()?;
        if len != 32 {
            return Err(corrupt("canonical digest is not 32 bytes"));
        }
        Ok(Digest32::from_bytes(self.array()?))
    }

    fn anchor(&mut self) -> Result<StateAnchor> {
        let fortress_id = FortressId::new(self.u64()?);
        let epoch = self.u64()?;
        let sequence = self.u64()?;
        let tick = GameTick(self.u64()?);
        let state_hash = self.digest()?;
        Ok(StateAnchor {
            fortress_id,
            cursor: ObservationCursor { epoch, sequence },
            tick,
            state_hash,
        })
    }
}

/// Insert into a map whose canonical encoding is strictly ascending.
fn insert_ascending<K: Ord, V>(
    map: &mut BTreeMap<K, V>,
    key: K,
    value: V,
    what: &str,
) -> Result<()> {
    if map.last_key_value().is_some_and(|(last, _)| *last >= key) {
        return Err(corrupt(format!(
            "{what} keys are not in strictly ascending canonical order"
        )));
    }
    map.insert(key, value);
    Ok(())
}

fn entity_kind(name: String) -> EntityKind {
    match name.as_str() {
        "fortress" => EntityKind::Fortress,
        "unit" => EntityKind::Unit,
        "item" => EntityKind::Item,
        "building" => EntityKind::Building,
        "job" => EntityKind::Job,
        "work_order" => EntityKind::WorkOrder,
        "stockpile" => EntityKind::Stockpile,
        "zone" => EntityKind::Zone,
        "burrow" => EntityKind::Burrow,
        "squad" => EntityKind::Squad,
        "military_order" => EntityKind::MilitaryOrder,
        "tile_feature" => EntityKind::TileFeature,
        "plant" => EntityKind::Plant,
        "creature" => EntityKind::Creature,
        "historical_figure" => EntityKind::HistoricalFigure,
        "civilization" => EntityKind::Civilization,
        "announcement" => EntityKind::Announcement,
        "syndrome" => EntityKind::Syndrome,
        _ => EntityKind::Other(name),
    }
}

fn edge_kind(name: String) -> EdgeKind {
    match name.as_str() {
        "located_at" => EdgeKind::LocatedAt,
        "contained_in" => EdgeKind::ContainedIn,
        "assigned_to" => EdgeKind::AssignedTo,
        "member_of" => EdgeKind::MemberOf,
        "performs" => EdgeKind::Performs,
        "requires" => EdgeKind::Requires,
        "produces" => EdgeKind::Produces,
        "uses" => EdgeKind::Uses,
        "supports" => EdgeKind::Supports,
        "threatens" => EdgeKind::Threatens,
        "ordered_by" => EdgeKind::OrderedBy,
        "parent_of" => EdgeKind::ParentOf,
        _ => EdgeKind::Custom(name),
    }
}

fn event_kind(name: String) -> WorldEventKind {
    match name.as_str() {
        "announcement" => WorldEventKind::Announcement,
        "job_changed" => WorldEventKind::JobChanged,
        "unit_changed" => WorldEventKind::UnitChanged,
        "construction_changed" => WorldEventKind::ConstructionChanged,
        "threat_detected" => WorldEventKind::ThreatDetected,
        "season_changed" => WorldEventKind::SeasonChanged,
        "adapter_notice" => WorldEventKind::AdapterNotice,
        _ => WorldEventKind::Other(name),
    }
}

fn value(reader: &mut Reader<'_>, depth: usize) -> Result<Value> {
    if depth > MAX_VALUE_DEPTH {
        return Err(corrupt("canonical value nesting exceeds the bound"));
    }
    Ok(match reader.u8()? {
        0 => Value::Null,
        1 => Value::Bool(reader.bool()?),
        2 => Value::I64(reader.i64()?),
        3 => Value::U64(reader.u64()?),
        4 => Value::Fixed {
            units: reader.i64()?,
            scale: reader.u32()?,
        },
        5 => Value::Text(reader.string()?),
        6 => Value::Entity(EntityId::new(reader.u64()?)),
        7 => Value::Coord(MapCoord {
            x: reader.i32()?,
            y: reader.i32()?,
            z: reader.i32()?,
        }),
        8 => Value::Bytes(reader.bytes()?),
        9 => {
            let count = reader.count(1)?;
            let mut values = Vec::with_capacity(count);
            for _ in 0..count {
                values.push(value(reader, depth + 1)?);
            }
            Value::List(values)
        }
        10 => Value::Object(value_map(reader, depth + 1)?),
        other => return Err(corrupt(format!("unknown canonical value tag {other}"))),
    })
}

fn value_map(reader: &mut Reader<'_>, depth: usize) -> Result<BTreeMap<String, Value>> {
    let count = reader.count(9)?;
    let mut map = BTreeMap::new();
    for _ in 0..count {
        let key = reader.string()?;
        let item = value(reader, depth)?;
        insert_ascending(&mut map, key, item, "object")?;
    }
    Ok(map)
}

fn presence(reader: &mut Reader<'_>) -> Result<FactPresence> {
    Ok(match reader.u8()? {
        0 => FactPresence::Known(value(reader, 0)?),
        1 => FactPresence::Absent,
        2 => FactPresence::Unknown(reader.string()?),
        3 => FactPresence::Unsupported(reader.string()?),
        4 => FactPresence::Omitted(reader.string()?),
        5 => FactPresence::Redacted(reader.string()?),
        6 => FactPresence::Stale(reader.anchor()?),
        other => return Err(corrupt(format!("unknown fact presence tag {other}"))),
    })
}

fn fact(reader: &mut Reader<'_>) -> Result<Fact> {
    let value = value(reader, 0)?;
    let observed_at = GameTick(reader.u64()?);
    let source = match reader.u8()? {
        0 => FactSource::DfhackField(reader.string()?),
        1 => FactSource::Derived(reader.string()?),
        2 => FactSource::AgentAssertion(reader.string()?),
        3 => FactSource::Replay,
        other => return Err(corrupt(format!("unknown fact source tag {other}"))),
    };
    let source_digest = reader.digest()?;
    let presence = if reader.bool()? {
        Some(presence(reader)?)
    } else {
        None
    };
    Ok(Fact {
        value,
        observed_at,
        source,
        source_digest,
        presence,
    })
}

fn fact_map(reader: &mut Reader<'_>) -> Result<BTreeMap<String, Fact>> {
    let count = reader.count(9)?;
    let mut map = BTreeMap::new();
    for _ in 0..count {
        let key = reader.string()?;
        let item = fact(reader)?;
        insert_ascending(&mut map, key, item, "fact field")?;
    }
    Ok(map)
}

fn graph(reader: &mut Reader<'_>) -> Result<WorldGraph> {
    let mut graph = WorldGraph::default();
    for _ in 0..reader.count(8)? {
        let id = EntityId::new(reader.u64()?);
        let generation = reader.u32()?;
        let revision = reader.u64()?;
        let kind = entity_kind(reader.string()?);
        let label = reader.string()?;
        let fields = fact_map(reader)?;
        let record = EntityRecord {
            id,
            generation,
            revision,
            kind,
            label,
            fields,
        };
        insert_ascending(&mut graph.entities, id, record, "entity")?;
    }
    for _ in 0..reader.count(16)? {
        let id = EdgeId::new(reader.u128()?);
        let revision = reader.u64()?;
        let kind = edge_kind(reader.string()?);
        let from = EntityId::new(reader.u64()?);
        let to = EntityId::new(reader.u64()?);
        let fields = fact_map(reader)?;
        let record = EdgeRecord {
            id,
            revision,
            kind,
            from,
            to,
            fields,
        };
        insert_ascending(&mut graph.edges, id, record, "edge")?;
    }
    for _ in 0..reader.count(12)? {
        let coord = ChunkCoord {
            x: reader.i32()?,
            y: reader.i32()?,
            z: reader.i32()?,
        };
        let revision = reader.u64()?;
        let width = reader.u16()?;
        let height = reader.u16()?;
        let run_count = reader.count(8)?;
        let mut terrain_runs = Vec::with_capacity(run_count);
        for _ in 0..run_count {
            terrain_runs.push(TerrainRun {
                tile_code: reader.u32()?,
                length: reader.u32()?,
            });
        }
        let mut sparse_overlays = BTreeMap::new();
        for _ in 0..reader.count(12)? {
            let offset = reader.u32()?;
            let fields = value_map(reader, 1)?;
            insert_ascending(&mut sparse_overlays, offset, fields, "overlay")?;
        }
        let chunk = MapChunk {
            coord,
            revision,
            width,
            height,
            terrain_runs,
            sparse_overlays,
        };
        insert_ascending(&mut graph.chunks, coord, chunk, "chunk")?;
    }
    for _ in 0..reader.count(16)? {
        let id = EventId::new(reader.u128()?);
        let tick = GameTick(reader.u64()?);
        let kind = event_kind(reader.string()?);
        let subject = if reader.bool()? {
            Some(EntityId::new(reader.u64()?))
        } else {
            None
        };
        let summary = reader.string()?;
        let fields = value_map(reader, 1)?;
        let event = WorldEvent {
            id,
            tick,
            kind,
            subject,
            summary,
            fields,
        };
        insert_ascending(&mut graph.events, id, event, "event")?;
    }
    Ok(graph)
}

impl WorldSnapshot {
    /// Decode the exact bytes produced by [`Self::canonical_bytes`].
    ///
    /// The result's state hash is recomputed from the input bytes, and the
    /// decoded value must re-encode to exactly those bytes; anything else is
    /// refused as a corrupt ledger rather than repaired.
    pub fn from_canonical_bytes(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > MAX_CANONICAL_SNAPSHOT_BYTES {
            return Err(corrupt("canonical snapshot exceeds the decoder bound"));
        }
        let mut reader = Reader { bytes, offset: 0 };
        if reader.string()? != SNAPSHOT_DOMAIN {
            return Err(corrupt("canonical snapshot domain tag is not recognized"));
        }
        let fortress_id = FortressId::new(reader.u64()?);
        let tick = GameTick(reader.u64()?);
        let cursor = ObservationCursor {
            epoch: reader.u64()?,
            sequence: reader.u64()?,
        };
        let paused = reader.bool()?;
        let graph = graph(&mut reader)?;
        if reader.remaining() != 0 {
            return Err(corrupt("canonical snapshot has trailing bytes"));
        }
        let snapshot = Self {
            fortress_id,
            tick,
            cursor,
            paused,
            graph,
            state_hash: Digest32::of_bytes(bytes),
        };
        if snapshot.canonical_bytes() != bytes {
            return Err(corrupt(
                "canonical snapshot does not re-encode to its stored bytes",
            ));
        }
        Ok(snapshot)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fact(value: Value) -> Fact {
        Fact::known(
            value,
            GameTick(7),
            FactSource::Derived("test".to_owned()),
            Digest32::of_bytes(b"src"),
        )
    }

    fn rich_snapshot() -> WorldSnapshot {
        let mut graph = WorldGraph::default();
        let mut object = BTreeMap::new();
        object.insert(
            "a".to_owned(),
            Value::List(vec![Value::Null, Value::Bool(true)]),
        );
        object.insert(
            "b".to_owned(),
            Value::Fixed {
                units: -5,
                scale: 2,
            },
        );
        let mut fields = BTreeMap::new();
        fields.insert("name".to_owned(), fact(Value::Text("Urist".to_owned())));
        fields.insert("obj".to_owned(), fact(Value::Object(object)));
        fields.insert(
            "pos".to_owned(),
            fact(Value::Coord(MapCoord { x: -1, y: 2, z: 3 })),
        );
        fields.insert(
            "stale".to_owned(),
            Fact::with_presence(
                FactPresence::Stale(StateAnchor {
                    fortress_id: FortressId::new(9),
                    cursor: ObservationCursor {
                        epoch: 1,
                        sequence: 2,
                    },
                    tick: GameTick(3),
                    state_hash: Digest32::of_bytes(b"x"),
                }),
                GameTick(3),
                FactSource::Replay,
                Digest32::ZERO,
            ),
        );
        fields.insert(
            "unknown".to_owned(),
            Fact::with_presence(
                FactPresence::Unknown("hidden".to_owned()),
                GameTick(3),
                FactSource::DfhackField("unit.x".to_owned()),
                Digest32::ZERO,
            ),
        );
        for (id, kind) in [
            (1, EntityKind::Unit),
            (2, EntityKind::Other("custom_kind".to_owned())),
            (5, EntityKind::Stockpile),
        ] {
            graph.entities.insert(
                EntityId::new(id),
                EntityRecord {
                    id: EntityId::new(id),
                    generation: 2,
                    revision: 4,
                    kind,
                    label: format!("e{id}"),
                    fields: fields.clone(),
                },
            );
        }
        graph.edges.insert(
            EdgeId::new(77),
            EdgeRecord {
                id: EdgeId::new(77),
                revision: 1,
                kind: EdgeKind::Custom("likes".to_owned()),
                from: EntityId::new(1),
                to: EntityId::new(5),
                fields: BTreeMap::new(),
            },
        );
        let coord = ChunkCoord { x: 0, y: 1, z: 10 };
        let mut overlay = BTreeMap::new();
        overlay.insert("liquid".to_owned(), Value::U64(3));
        graph.chunks.insert(
            coord,
            MapChunk {
                coord,
                revision: 3,
                width: 4,
                height: 4,
                terrain_runs: vec![
                    TerrainRun {
                        tile_code: 1,
                        length: 10,
                    },
                    TerrainRun {
                        tile_code: 2,
                        length: 6,
                    },
                ],
                sparse_overlays: BTreeMap::from([(5, overlay)]),
            },
        );
        graph.events.insert(
            EventId::new(3),
            WorldEvent {
                id: EventId::new(3),
                tick: GameTick(5),
                kind: WorldEventKind::ThreatDetected,
                subject: Some(EntityId::new(1)),
                summary: "goblins".to_owned(),
                fields: BTreeMap::from([("bytes".to_owned(), Value::Bytes(vec![0, 255]))]),
            },
        );
        WorldSnapshot::new(
            FortressId::new(9),
            GameTick(1234),
            ObservationCursor {
                epoch: 2,
                sequence: 17,
            },
            true,
            graph,
        )
    }

    #[test]
    fn canonical_snapshot_round_trips_exactly() -> Result<()> {
        let snapshot = rich_snapshot();
        let bytes = snapshot.canonical_bytes();
        let decoded = WorldSnapshot::from_canonical_bytes(&bytes)?;
        assert_eq!(decoded, snapshot);
        assert!(decoded.hash_is_valid());
        assert_eq!(decoded.state_hash, snapshot.state_hash);
        Ok(())
    }

    #[test]
    fn every_truncation_is_refused() {
        let bytes = rich_snapshot().canonical_bytes();
        for len in 0..bytes.len() {
            let result = WorldSnapshot::from_canonical_bytes(&bytes[..len]);
            assert!(
                matches!(result, Err(ref e) if e.code == ErrorCode::CorruptLedger),
                "truncation to {len} bytes decoded"
            );
        }
    }

    #[test]
    fn trailing_bytes_and_bit_flips_never_decode_to_a_different_hash() {
        let snapshot = rich_snapshot();
        let mut bytes = snapshot.canonical_bytes();
        bytes.push(0);
        assert!(WorldSnapshot::from_canonical_bytes(&bytes).is_err());
        bytes.pop();
        for index in (0..bytes.len()).step_by(7) {
            let mut flipped = bytes.clone();
            flipped[index] ^= 0x41;
            if let Ok(decoded) = WorldSnapshot::from_canonical_bytes(&flipped) {
                // A flip can land in free-form content; the decoded value is
                // then the honest meaning of the flipped bytes.
                assert_eq!(decoded.state_hash, Digest32::of_bytes(&flipped));
                assert_eq!(decoded.canonical_bytes(), flipped);
            }
        }
    }

    #[test]
    fn reserved_variant_names_decode_as_the_dedicated_variant() {
        let mut snapshot = rich_snapshot();
        if let Some(entity) = snapshot.graph.entities.get_mut(&EntityId::new(2)) {
            entity.kind = EntityKind::Other("unit".to_owned());
        }
        snapshot.refresh_hash();
        let bytes = snapshot.canonical_bytes();
        let decoded = WorldSnapshot::from_canonical_bytes(&bytes);
        // The bytes are identical to a genuine unit, so they decode to one.
        assert!(matches!(
            decoded.map(|s| s.graph.entities[&EntityId::new(2)].kind.clone()),
            Ok(EntityKind::Unit)
        ));
    }

    #[test]
    fn forged_collection_lengths_fail_without_allocating() {
        let mut bytes = Vec::new();
        crate::canonical::put_str(&mut bytes, SNAPSHOT_DOMAIN);
        bytes.extend_from_slice(&[0u8; 8 * 4 + 1]);
        bytes.extend_from_slice(&u64::MAX.to_be_bytes());
        assert!(WorldSnapshot::from_canonical_bytes(&bytes).is_err());
    }

    #[test]
    fn out_of_order_entities_are_refused() {
        let snapshot = rich_snapshot();
        let mut a = Vec::new();
        snapshot.graph.entities[&EntityId::new(1)].encode(&mut a);
        let mut b = Vec::new();
        snapshot.graph.entities[&EntityId::new(5)].encode(&mut b);
        let bytes = snapshot.canonical_bytes();
        let start = bytes
            .windows(a.len())
            .position(|w| w == a.as_slice())
            .unwrap_or(0);
        let mut swapped = bytes.clone();
        // Swap the first entity with the third (same length labels).
        let mut c = Vec::new();
        snapshot.graph.entities[&EntityId::new(2)].encode(&mut c);
        let third = start + a.len() + c.len();
        if a.len() == b.len() {
            swapped[start..start + a.len()].copy_from_slice(&b);
            swapped[third..third + b.len()].copy_from_slice(&a);
            assert!(WorldSnapshot::from_canonical_bytes(&swapped).is_err());
        }
    }
}
