use super::{
    BTreeMap, ChunkCoord, EdgeId, EdgeRecord, EntityId, EntityRecord, EventId, FortressId,
    GameTick, MAX_CANONICAL_SNAPSHOT_BYTES, MapChunk, ObservationCursor, Reader, Result,
    TerrainRun, WorldEvent, corrupt, edge_kind, entity_kind, event_kind, fact_map,
    insert_ascending, value_map,
};

/// Decode the exact canonical delta bytes without applying them. Semantic
/// transition verification still requires apply_delta with the exact base.
impl crate::StateDelta {
    pub fn from_canonical_bytes(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > MAX_CANONICAL_SNAPSHOT_BYTES {
            return Err(corrupt("canonical delta exceeds the decoder bound"));
        }
        let mut reader = Reader { bytes, offset: 0 };
        if reader.string()? != "dfmcp-state-delta-v1" {
            return Err(corrupt("canonical delta domain tag is not recognized"));
        }
        let fortress_id = FortressId::new(reader.u64()?);
        let base_cursor = ObservationCursor {
            epoch: reader.u64()?,
            sequence: reader.u64()?,
        };
        let target_cursor = ObservationCursor {
            epoch: reader.u64()?,
            sequence: reader.u64()?,
        };
        let base_hash = reader.digest()?;
        let target_hash = reader.digest()?;
        let target_tick = GameTick(reader.u64()?);
        let count = reader.count(21)?;
        if count > crate::delta::MAX_STATE_DELTA_CHANGES {
            return Err(corrupt("canonical delta exceeds its change-count bound"));
        }
        let mut changes = Vec::new();
        for _ in 0..count {
            let change = match reader.u8()? {
                0 => crate::WorldChange::UpsertEntity(EntityRecord {
                    id: EntityId::new(reader.u64()?),
                    generation: reader.u32()?,
                    revision: reader.u64()?,
                    kind: entity_kind(reader.string()?),
                    label: reader.string()?,
                    fields: fact_map(&mut reader)?,
                }),
                1 => crate::WorldChange::RemoveEntity {
                    id: EntityId::new(reader.u64()?),
                    expected_generation: reader.u32()?,
                    expected_revision: reader.u64()?,
                },
                2 => crate::WorldChange::UpsertEdge(EdgeRecord {
                    id: EdgeId::new(reader.u128()?),
                    revision: reader.u64()?,
                    kind: edge_kind(reader.string()?),
                    from: EntityId::new(reader.u64()?),
                    to: EntityId::new(reader.u64()?),
                    fields: fact_map(&mut reader)?,
                }),
                3 => crate::WorldChange::RemoveEdge {
                    id: EdgeId::new(reader.u128()?),
                    expected_revision: reader.u64()?,
                },
                4 => {
                    let coord = ChunkCoord {
                        x: reader.i32()?,
                        y: reader.i32()?,
                        z: reader.i32()?,
                    };
                    let revision = reader.u64()?;
                    let width = reader.u16()?;
                    let height = reader.u16()?;
                    let count = reader.count(8)?;
                    let mut terrain_runs = Vec::new();
                    for _ in 0..count {
                        terrain_runs.push(TerrainRun {
                            tile_code: reader.u32()?,
                            length: reader.u32()?,
                        });
                    }
                    let mut sparse_overlays = BTreeMap::new();
                    for _ in 0..reader.count(12)? {
                        let offset = reader.u32()?;
                        let fields = value_map(&mut reader, 1)?;
                        insert_ascending(&mut sparse_overlays, offset, fields, "delta overlay")?;
                    }
                    crate::WorldChange::UpsertMapChunk(MapChunk {
                        coord,
                        revision,
                        width,
                        height,
                        terrain_runs,
                        sparse_overlays,
                    })
                }
                5 => crate::WorldChange::RemoveMapChunk {
                    coord: ChunkCoord {
                        x: reader.i32()?,
                        y: reader.i32()?,
                        z: reader.i32()?,
                    },
                    expected_revision: reader.u64()?,
                },
                6 => crate::WorldChange::AppendEvent(WorldEvent {
                    id: EventId::new(reader.u128()?),
                    tick: GameTick(reader.u64()?),
                    kind: event_kind(reader.string()?),
                    subject: if reader.bool()? {
                        Some(EntityId::new(reader.u64()?))
                    } else {
                        None
                    },
                    summary: reader.string()?,
                    fields: value_map(&mut reader, 1)?,
                }),
                other => {
                    return Err(corrupt(format!(
                        "unknown canonical delta change tag {other}"
                    )));
                }
            };
            changes.push(change);
        }
        let truncated = reader.bool()?;
        let continuation = if reader.bool()? {
            Some(reader.string()?)
        } else {
            None
        };
        if reader.remaining() != 0 {
            return Err(corrupt("canonical delta has trailing bytes"));
        }
        let delta = Self {
            fortress_id,
            base_cursor,
            target_cursor,
            base_hash,
            target_hash,
            target_tick,
            changes,
            truncated,
            continuation,
        };
        if delta.canonical_bytes() != bytes {
            return Err(corrupt(
                "canonical delta does not re-encode to its stored bytes",
            ));
        }
        Ok(delta)
    }
}
