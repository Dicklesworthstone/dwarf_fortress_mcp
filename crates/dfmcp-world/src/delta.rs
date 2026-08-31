use dfmcp_core::{
    DfmcpError, Digest32, EdgeId, EntityId, ErrorCode, EventId, FortressId, GameTick,
    ObservationCursor, Result,
};

use crate::{ChunkCoord, EdgeRecord, EntityRecord, MapChunk, WorldEvent, WorldSnapshot};

pub const MAX_STATE_DELTA_CHANGES: usize = 1_000_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorldChange {
    UpsertEntity(EntityRecord),
    RemoveEntity {
        id: EntityId,
        expected_generation: u32,
        expected_revision: u64,
    },
    UpsertEdge(EdgeRecord),
    RemoveEdge {
        id: EdgeId,
        expected_revision: u64,
    },
    UpsertMapChunk(MapChunk),
    RemoveMapChunk {
        coord: ChunkCoord,
        expected_revision: u64,
    },
    AppendEvent(WorldEvent),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StateDelta {
    pub fortress_id: FortressId,
    pub base_cursor: ObservationCursor,
    pub target_cursor: ObservationCursor,
    pub base_hash: Digest32,
    pub target_hash: Digest32,
    pub target_tick: GameTick,
    pub changes: Vec<WorldChange>,
    pub truncated: bool,
    pub continuation: Option<String>,
}

impl StateDelta {
    /// Canonical, length-delimited encoding suitable for integrity digests.
    #[must_use]
    pub fn canonical_bytes(&self) -> Vec<u8> {
        use crate::canonical::{put_bool, put_bytes, put_i32, put_str, put_u32, put_u64};

        let mut output = Vec::new();
        put_str(&mut output, "dfmcp-state-delta-v1");
        put_u64(&mut output, self.fortress_id.get());
        put_u64(&mut output, self.base_cursor.epoch);
        put_u64(&mut output, self.base_cursor.sequence);
        put_u64(&mut output, self.target_cursor.epoch);
        put_u64(&mut output, self.target_cursor.sequence);
        put_bytes(&mut output, self.base_hash.as_bytes());
        put_bytes(&mut output, self.target_hash.as_bytes());
        put_u64(&mut output, self.target_tick.0);
        put_u64(&mut output, self.changes.len() as u64);
        for change in &self.changes {
            match change {
                WorldChange::UpsertEntity(entity) => {
                    output.push(0);
                    entity.encode(&mut output);
                }
                WorldChange::RemoveEntity {
                    id,
                    expected_generation,
                    expected_revision,
                } => {
                    output.push(1);
                    put_u64(&mut output, id.get());
                    put_u32(&mut output, *expected_generation);
                    put_u64(&mut output, *expected_revision);
                }
                WorldChange::UpsertEdge(edge) => {
                    output.push(2);
                    edge.encode(&mut output);
                }
                WorldChange::RemoveEdge {
                    id,
                    expected_revision,
                } => {
                    output.push(3);
                    output.extend_from_slice(&id.get().to_be_bytes());
                    put_u64(&mut output, *expected_revision);
                }
                WorldChange::UpsertMapChunk(chunk) => {
                    output.push(4);
                    chunk.encode(&mut output);
                }
                WorldChange::RemoveMapChunk {
                    coord,
                    expected_revision,
                } => {
                    output.push(5);
                    put_i32(&mut output, coord.x);
                    put_i32(&mut output, coord.y);
                    put_i32(&mut output, coord.z);
                    put_u64(&mut output, *expected_revision);
                }
                WorldChange::AppendEvent(event) => {
                    output.push(6);
                    event.encode(&mut output);
                }
            }
        }
        put_bool(&mut output, self.truncated);
        match &self.continuation {
            Some(token) => {
                output.push(1);
                put_str(&mut output, token);
            }
            None => output.push(0),
        }
        output
    }
}

pub fn build_delta(
    base: &WorldSnapshot,
    target_cursor: ObservationCursor,
    target_tick: GameTick,
    changes: Vec<WorldChange>,
) -> Result<StateDelta> {
    if changes.len() > MAX_STATE_DELTA_CHANGES {
        return Err(DfmcpError::new(
            ErrorCode::BudgetExceeded,
            "state delta exceeds the implementation change-count safety bound",
        ));
    }
    validate_cursor_transition(base.cursor, target_cursor)?;
    if target_tick < base.tick {
        return Err(DfmcpError::new(
            ErrorCode::InvalidRequest,
            "delta target tick must not precede the base tick",
        ));
    }
    if !base.hash_is_valid() {
        return Err(DfmcpError::new(
            ErrorCode::InternalInvariantViolation,
            "base snapshot state hash is invalid",
        ));
    }
    let mut candidate = base.clone();
    candidate.cursor = target_cursor;
    candidate.tick = target_tick;
    apply_changes(&mut candidate, &changes)?;
    candidate.refresh_hash();
    Ok(StateDelta {
        fortress_id: base.fortress_id,
        base_cursor: base.cursor,
        target_cursor,
        base_hash: base.state_hash,
        target_hash: candidate.state_hash,
        target_tick,
        changes,
        truncated: false,
        continuation: None,
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContinuationToken {
    pub fortress_id: FortressId,
    pub cursor: ObservationCursor,
    pub offset: u32,
}

pub(crate) const MAX_CONTINUATION_TOKEN_BYTES: usize = 128;

impl ContinuationToken {
    #[must_use]
    pub fn new(fortress_id: FortressId, cursor: ObservationCursor, offset: u32) -> Self {
        Self {
            fortress_id,
            cursor,
            offset,
        }
    }

    #[must_use]
    pub fn encode(&self) -> String {
        format!(
            "cont:{}:{}:{}:{}",
            self.fortress_id.get(),
            self.cursor.epoch,
            self.cursor.sequence,
            self.offset
        )
    }

    pub fn decode(token: &str) -> Result<Self> {
        if token.len() > MAX_CONTINUATION_TOKEN_BYTES {
            return Err(DfmcpError::new(
                ErrorCode::BudgetExceeded,
                "continuation token exceeds its explicit byte bound",
            ));
        }
        let parts: Vec<&str> = token.split(':').collect();
        if parts.len() != 5 || parts[0] != "cont" {
            return Err(DfmcpError::new(
                ErrorCode::InvalidRequest,
                "invalid continuation token format",
            ));
        }
        let fid: u64 = parts[1].parse().map_err(|_| {
            DfmcpError::new(ErrorCode::InvalidRequest, "invalid fortress ID in token")
        })?;
        let epoch: u64 = parts[2]
            .parse()
            .map_err(|_| DfmcpError::new(ErrorCode::InvalidRequest, "invalid epoch in token"))?;
        let seq: u64 = parts[3]
            .parse()
            .map_err(|_| DfmcpError::new(ErrorCode::InvalidRequest, "invalid sequence in token"))?;
        let offset: u32 = parts[4]
            .parse()
            .map_err(|_| DfmcpError::new(ErrorCode::InvalidRequest, "invalid offset in token"))?;
        Ok(Self {
            fortress_id: FortressId::new(fid),
            cursor: ObservationCursor {
                epoch,
                sequence: seq,
            },
            offset,
        })
    }
}

pub fn compute_snapshot_diff(
    base: &WorldSnapshot,
    target: &WorldSnapshot,
) -> Result<Vec<WorldChange>> {
    if !base.hash_is_valid() || !target.hash_is_valid() {
        return Err(DfmcpError::new(
            ErrorCode::InternalInvariantViolation,
            "cannot diff a snapshot whose canonical state hash is invalid",
        ));
    }
    if target.fortress_id != base.fortress_id {
        return Err(DfmcpError::new(
            ErrorCode::InvalidRequest,
            "cannot diff snapshots of different fortresses",
        ));
    }

    let mut changes = Vec::new();

    // 1. Entity removals (in base but not in target)
    for (id, entity) in &base.graph.entities {
        if !target.graph.entities.contains_key(id) {
            changes.push(WorldChange::RemoveEntity {
                id: *id,
                expected_generation: entity.generation,
                expected_revision: entity.revision,
            });
        }
    }

    // 2. Entity upserts (in target and not in base, or modified)
    for (id, target_entity) in &target.graph.entities {
        if let Some(base_entity) = base.graph.entities.get(id) {
            if base_entity != target_entity {
                changes.push(WorldChange::UpsertEntity(target_entity.clone()));
            }
        } else {
            changes.push(WorldChange::UpsertEntity(target_entity.clone()));
        }
    }

    // 3. Edge removals
    for (id, edge) in &base.graph.edges {
        if !target.graph.edges.contains_key(id) {
            changes.push(WorldChange::RemoveEdge {
                id: *id,
                expected_revision: edge.revision,
            });
        }
    }

    // 4. Edge upserts
    for (id, target_edge) in &target.graph.edges {
        if let Some(base_edge) = base.graph.edges.get(id) {
            if base_edge != target_edge {
                changes.push(WorldChange::UpsertEdge(target_edge.clone()));
            }
        } else {
            changes.push(WorldChange::UpsertEdge(target_edge.clone()));
        }
    }

    // 5. Map chunk removals
    for (coord, chunk) in &base.graph.chunks {
        if !target.graph.chunks.contains_key(coord) {
            changes.push(WorldChange::RemoveMapChunk {
                coord: *coord,
                expected_revision: chunk.revision,
            });
        }
    }

    // 6. Map chunk upserts
    for (coord, target_chunk) in &target.graph.chunks {
        if let Some(base_chunk) = base.graph.chunks.get(coord) {
            if base_chunk != target_chunk {
                changes.push(WorldChange::UpsertMapChunk(target_chunk.clone()));
            }
        } else {
            changes.push(WorldChange::UpsertMapChunk(target_chunk.clone()));
        }
    }

    // 7. Events are immutable. Existing IDs cannot be edited or removed by a
    // delta; only genuinely new events can be appended.
    for (id, base_event) in &base.graph.events {
        match target.graph.events.get(id) {
            Some(target_event) if target_event == base_event => {}
            Some(_) => {
                return Err(DfmcpError::new(
                    ErrorCode::Conflict,
                    format!("event {id} changed after publication"),
                ));
            }
            None => {
                return Err(DfmcpError::new(
                    ErrorCode::Conflict,
                    format!("event {id} disappeared after publication"),
                ));
            }
        }
    }
    for (id, event) in &target.graph.events {
        if !base.graph.events.contains_key(id) {
            changes.push(WorldChange::AppendEvent(event.clone()));
        }
    }

    Ok(changes)
}

pub fn diff_snapshots(base: &WorldSnapshot, target: &WorldSnapshot) -> Result<StateDelta> {
    let changes = compute_snapshot_diff(base, target)?;
    let delta = build_delta(base, target.cursor, target.tick, changes)?;
    if delta.target_hash != target.state_hash {
        return Err(DfmcpError::new(
            ErrorCode::InternalInvariantViolation,
            "snapshot diff did not reconstruct the declared target state",
        )
        .with_detail("computed", delta.target_hash.to_string())
        .with_detail("declared", target.state_hash.to_string()));
    }
    Ok(delta)
}

pub fn apply_delta(base: &WorldSnapshot, delta: &StateDelta) -> Result<WorldSnapshot> {
    if !base.hash_is_valid() {
        return Err(DfmcpError::new(
            ErrorCode::InternalInvariantViolation,
            "base snapshot state hash is invalid",
        ));
    }
    if delta.truncated || delta.continuation.is_some() {
        return Err(DfmcpError::new(
            ErrorCode::CursorGap,
            "a partial delta cannot be applied as a complete state transition",
        )
        .retryable(true));
    }
    if delta.changes.len() > MAX_STATE_DELTA_CHANGES {
        return Err(DfmcpError::new(
            ErrorCode::BudgetExceeded,
            "state delta exceeds the implementation change-count safety bound",
        ));
    }
    if delta.fortress_id != base.fortress_id {
        return Err(DfmcpError::new(
            ErrorCode::StaleAnchor,
            "delta belongs to a different fortress",
        ));
    }
    if delta.base_cursor != base.cursor || delta.base_hash != base.state_hash {
        return Err(DfmcpError::new(
            ErrorCode::CursorGap,
            "delta base anchor does not match snapshot",
        )
        .retryable(true)
        .with_detail("expected_cursor", format!("{:?}", base.cursor))
        .with_detail("received_cursor", format!("{:?}", delta.base_cursor)));
    }
    validate_cursor_transition(base.cursor, delta.target_cursor)?;
    if delta.target_tick < base.tick {
        return Err(DfmcpError::new(
            ErrorCode::InvalidRequest,
            "delta target tick must not precede the base tick",
        ));
    }
    let mut candidate = base.clone();
    candidate.cursor = delta.target_cursor;
    candidate.tick = delta.target_tick;
    apply_changes(&mut candidate, &delta.changes)?;
    candidate.refresh_hash();
    if candidate.state_hash != delta.target_hash {
        return Err(DfmcpError::new(
            ErrorCode::InternalInvariantViolation,
            "delta target hash does not match the reconstructed state",
        )
        .with_detail("computed", candidate.state_hash.to_string())
        .with_detail("declared", delta.target_hash.to_string()));
    }
    Ok(candidate)
}

fn validate_cursor_transition(base: ObservationCursor, target: ObservationCursor) -> Result<()> {
    if target.epoch != base.epoch {
        return Err(DfmcpError::new(
            ErrorCode::CursorGap,
            "epoch changes require a full snapshot rather than a delta",
        )
        .retryable(true));
    }
    if target.sequence <= base.sequence {
        return Err(DfmcpError::new(
            ErrorCode::InvalidRequest,
            "delta target cursor must advance beyond the base cursor",
        ));
    }
    Ok(())
}

fn apply_changes(snapshot: &mut WorldSnapshot, changes: &[WorldChange]) -> Result<()> {
    for change in changes {
        match change {
            WorldChange::UpsertEntity(incoming) => {
                validate_entity_upsert(snapshot, incoming)?;
                snapshot
                    .graph
                    .entities
                    .insert(incoming.id, incoming.clone());
            }
            WorldChange::RemoveEntity {
                id,
                expected_generation,
                expected_revision,
            } => {
                let existing = snapshot.graph.entities.get(id).ok_or_else(|| {
                    DfmcpError::new(
                        ErrorCode::StaleAnchor,
                        format!("entity {id} does not exist for removal"),
                    )
                })?;
                if existing.generation != *expected_generation
                    || existing.revision != *expected_revision
                {
                    return Err(DfmcpError::new(
                        ErrorCode::StaleAnchor,
                        format!("entity {id} revision changed before removal"),
                    ));
                }
                snapshot.graph.entities.remove(id);
            }
            WorldChange::UpsertEdge(incoming) => {
                validate_edge_upsert(snapshot, incoming)?;
                snapshot.graph.edges.insert(incoming.id, incoming.clone());
            }
            WorldChange::RemoveEdge {
                id,
                expected_revision,
            } => {
                let existing = snapshot.graph.edges.get(id).ok_or_else(|| {
                    DfmcpError::new(
                        ErrorCode::StaleAnchor,
                        format!("edge {id} does not exist for removal"),
                    )
                })?;
                if existing.revision != *expected_revision {
                    return Err(DfmcpError::new(
                        ErrorCode::StaleAnchor,
                        format!("edge {id} revision changed before removal"),
                    ));
                }
                snapshot.graph.edges.remove(id);
            }
            WorldChange::UpsertMapChunk(incoming) => {
                validate_chunk(incoming)?;
                if let Some(existing) = snapshot.graph.chunks.get(&incoming.coord) {
                    if incoming.revision < existing.revision {
                        return Err(DfmcpError::new(
                            ErrorCode::StaleAnchor,
                            "map chunk revision regressed",
                        ));
                    }
                    if incoming.revision == existing.revision && incoming != existing {
                        return Err(DfmcpError::new(
                            ErrorCode::Conflict,
                            "same map chunk revision carries different content",
                        ));
                    }
                }
                snapshot
                    .graph
                    .chunks
                    .insert(incoming.coord, incoming.clone());
            }
            WorldChange::RemoveMapChunk {
                coord,
                expected_revision,
            } => {
                let existing = snapshot.graph.chunks.get(coord).ok_or_else(|| {
                    DfmcpError::new(
                        ErrorCode::StaleAnchor,
                        "map chunk does not exist for removal",
                    )
                })?;
                if existing.revision != *expected_revision {
                    return Err(DfmcpError::new(
                        ErrorCode::StaleAnchor,
                        "map chunk revision changed before removal",
                    ));
                }
                snapshot.graph.chunks.remove(coord);
            }
            WorldChange::AppendEvent(incoming) => {
                if let Some(existing) = snapshot.graph.events.get(&incoming.id) {
                    if existing != incoming {
                        return Err(DfmcpError::new(
                            ErrorCode::Conflict,
                            "event identifier was reused with different content",
                        ));
                    }
                } else {
                    snapshot.graph.events.insert(incoming.id, incoming.clone());
                }
            }
        }
    }
    validate_graph(snapshot)
}

fn validate_entity_upsert(snapshot: &WorldSnapshot, incoming: &EntityRecord) -> Result<()> {
    if incoming.id == EntityId::NIL {
        return Err(DfmcpError::new(
            ErrorCode::InvalidRequest,
            "entity identifier zero is reserved",
        ));
    }
    if let Some(existing) = snapshot.graph.entities.get(&incoming.id) {
        if incoming.generation < existing.generation {
            return Err(DfmcpError::new(
                ErrorCode::StaleAnchor,
                format!("entity {} generation regressed", incoming.id),
            ));
        }
        if incoming.generation == existing.generation && incoming.revision < existing.revision {
            return Err(DfmcpError::new(
                ErrorCode::StaleAnchor,
                format!("entity {} revision regressed", incoming.id),
            ));
        }
        if incoming.generation == existing.generation
            && incoming.revision == existing.revision
            && incoming != existing
        {
            return Err(DfmcpError::new(
                ErrorCode::Conflict,
                format!(
                    "entity {} has different content at the same generation and revision",
                    incoming.id
                ),
            ));
        }
    }
    Ok(())
}

fn validate_edge_upsert(snapshot: &WorldSnapshot, incoming: &EdgeRecord) -> Result<()> {
    if incoming.id == EdgeId::NIL {
        return Err(DfmcpError::new(
            ErrorCode::InvalidRequest,
            "edge identifier zero is reserved",
        ));
    }
    if let Some(existing) = snapshot.graph.edges.get(&incoming.id) {
        if incoming.revision < existing.revision {
            return Err(DfmcpError::new(
                ErrorCode::StaleAnchor,
                format!("edge {} revision regressed", incoming.id),
            ));
        }
        if incoming.revision == existing.revision && incoming != existing {
            return Err(DfmcpError::new(
                ErrorCode::Conflict,
                format!("edge {} changed without a revision advance", incoming.id),
            ));
        }
    }
    Ok(())
}

fn validate_chunk(chunk: &MapChunk) -> Result<()> {
    if chunk.width == 0 || chunk.height == 0 {
        return Err(DfmcpError::new(
            ErrorCode::InvalidRequest,
            "map chunk dimensions must be nonzero",
        ));
    }
    if chunk.encoded_tile_count() != Some(chunk.tile_count()) {
        return Err(DfmcpError::new(
            ErrorCode::InvalidRequest,
            "map chunk terrain runs do not cover exactly the declared tile count",
        ));
    }
    if chunk
        .sparse_overlays
        .keys()
        .any(|offset| *offset >= chunk.tile_count())
    {
        return Err(DfmcpError::new(
            ErrorCode::InvalidRequest,
            "map chunk sparse overlay offset is out of bounds",
        ));
    }
    Ok(())
}

fn validate_graph(snapshot: &WorldSnapshot) -> Result<()> {
    for edge in snapshot.graph.edges.values() {
        if !snapshot.graph.entities.contains_key(&edge.from)
            || !snapshot.graph.entities.contains_key(&edge.to)
        {
            return Err(DfmcpError::new(
                ErrorCode::InternalInvariantViolation,
                format!("edge {} refers to a missing endpoint", edge.id),
            ));
        }
    }
    for (id, event) in &snapshot.graph.events {
        if *id == EventId::NIL {
            return Err(DfmcpError::new(
                ErrorCode::InternalInvariantViolation,
                "event identifier zero is reserved",
            ));
        }
        if event.id != *id {
            return Err(DfmcpError::new(
                ErrorCode::InternalInvariantViolation,
                "event map key does not match event identifier",
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use dfmcp_core::{
        DfmcpError, Digest32, EntityId, ErrorCode, EventId, FortressId, GameTick, ObservationCursor,
    };

    use super::{WorldChange, apply_delta, build_delta, diff_snapshots};
    use crate::{
        EntityKind, EntityRecord, Fact, FactSource, Value, WorldEvent, WorldEventKind, WorldGraph,
        WorldSnapshot,
    };

    fn unit(revision: u64, stress: i64) -> EntityRecord {
        let mut fields = BTreeMap::new();
        fields.insert(
            "stress".to_owned(),
            Fact::known(
                Value::I64(stress),
                GameTick(revision),
                FactSource::Replay,
                Digest32::ZERO,
            ),
        );
        EntityRecord {
            id: EntityId::new(1),
            generation: 1,
            revision,
            kind: EntityKind::Unit,
            label: "Urist".to_owned(),
            fields,
        }
    }

    fn base() -> WorldSnapshot {
        let mut graph = WorldGraph::default();
        graph.entities.insert(EntityId::new(1), unit(1, 10));
        WorldSnapshot::new(
            FortressId::new(7),
            GameTick(1),
            ObservationCursor::ORIGIN,
            true,
            graph,
        )
    }

    #[test]
    fn delta_round_trip_preserves_declared_hash() -> Result<(), DfmcpError> {
        let base = base();
        let delta = build_delta(
            &base,
            base.cursor.next(),
            GameTick(2),
            vec![WorldChange::UpsertEntity(unit(2, 30))],
        )?;
        let target = apply_delta(&base, &delta)?;
        assert_eq!(target.state_hash, delta.target_hash);
        assert_eq!(
            target
                .graph
                .entities
                .get(&EntityId::new(1))
                .and_then(|entity| entity.fields.get("stress"))
                .map(|fact| &fact.value),
            Some(&Value::I64(30))
        );
        Ok(())
    }

    #[test]
    fn stale_base_is_not_silently_bridged() -> Result<(), DfmcpError> {
        let base = base();
        let delta = build_delta(
            &base,
            base.cursor.next(),
            GameTick(2),
            vec![WorldChange::UpsertEntity(unit(2, 30))],
        )?;
        let mut altered = base.clone();
        altered.tick = GameTick(9);
        altered.refresh_hash();
        let result = apply_delta(&altered, &delta);
        assert!(result.is_err());
        Ok(())
    }

    #[test]
    fn published_events_cannot_be_edited_or_removed_by_diff() {
        let mut base = base();
        let event_id = EventId::new(1);
        base.graph.events.insert(
            event_id,
            WorldEvent {
                id: event_id,
                tick: GameTick(1),
                kind: WorldEventKind::Announcement,
                subject: Some(EntityId::new(1)),
                summary: "original".to_owned(),
                fields: BTreeMap::new(),
            },
        );
        base.refresh_hash();

        let mut edited = base.clone();
        edited.cursor = edited.cursor.next();
        edited.tick = GameTick(2);
        if let Some(event) = edited.graph.events.get_mut(&event_id) {
            event.summary = "edited".to_owned();
        }
        edited.refresh_hash();
        let edited_error = diff_snapshots(&base, &edited);
        assert!(matches!(edited_error, Err(ref error) if error.code == ErrorCode::Conflict));

        let mut removed = base.clone();
        removed.cursor = removed.cursor.next();
        removed.tick = GameTick(2);
        removed.graph.events.remove(&event_id);
        removed.refresh_hash();
        let removed_error = diff_snapshots(&base, &removed);
        assert!(matches!(removed_error, Err(ref error) if error.code == ErrorCode::Conflict));
    }
}
