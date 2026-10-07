//! Typed, bounded projections of canonical observations.
//!
//! Inclusion describes delivered data, not complete knowledge of a game domain.
//! The source anchor is retained separately from the projected snapshot hash.
//! Profiles grant no authority, and decoding a provenance claim does not prove
//! that claim: consumers with the source can verify_against_source.

use crate::canonical::{put_anchor, put_bytes, put_str, put_u64};
use crate::{
    EntityKind, Fact, FactPresence, ObservationCapsule, Value, WorldSnapshot, apply_delta,
    diff_snapshots,
};
use dfmcp_core::{
    DfmcpError, Digest32, ErrorCode, FortressId, GameTick, ObservationCursor, Result, StateAnchor,
};
use std::collections::BTreeMap;

pub const MAX_PROFILE_BYTES: usize = 16 * 1024 * 1024;
const MAX_EXTENSIONS: usize = 64;
const MAX_EXTENSION_BYTES: usize = 128 * 1024;
const SNAPSHOT_DOMAIN: &str = "dfmcp-profiled-snapshot-v1";
const CAPSULE_DOMAIN: &str = "dfmcp-profiled-capsule-v1";

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CompletenessProfile {
    ControlMinimum,
    Operations,
    Spatial,
    Historical,
    ResearchFull,
}

impl CompletenessProfile {
    pub const ALL: [Self; 5] = [
        Self::ControlMinimum,
        Self::Operations,
        Self::Spatial,
        Self::Historical,
        Self::ResearchFull,
    ];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ControlMinimum => "control-minimum",
            Self::Operations => "operations",
            Self::Spatial => "spatial",
            Self::Historical => "historical",
            Self::ResearchFull => "research-full",
        }
    }

    pub fn parse(name: &str) -> Result<Self> {
        match name {
            "control-minimum" => Ok(Self::ControlMinimum),
            "operations" => Ok(Self::Operations),
            "spatial" => Ok(Self::Spatial),
            "historical" => Ok(Self::Historical),
            "research-full" => Ok(Self::ResearchFull),
            _ => Err(invalid("unknown canonical completeness profile")),
        }
    }

    /// Retain all identities; deliver fields only for the declared domain.
    /// This is a projection policy, never evidence that all such entities exist
    /// in the source. Registered action preconditions still use canonical state.
    #[must_use]
    pub fn includes_entity_fields(self, kind: &EntityKind) -> bool {
        // Canonical names, including reserved aliases, have one policy.
        let name = kind.as_str();
        if matches!(
            name,
            "fortress" | "plan" | "action" | "obligation" | "lease" | "checkpoint" | "evidence"
        ) {
            return true;
        }
        match self {
            // Conservatively retain current state used by registered actions.
            // A projection cannot establish which preconditions are eligible.
            Self::ControlMinimum => true,
            Self::Operations => matches!(
                name,
                "unit"
                    | "item"
                    | "building"
                    | "job"
                    | "work_order"
                    | "stockpile"
                    | "zone"
                    | "burrow"
                    | "squad"
                    | "military_order"
                    | "announcement"
                    | "syndrome"
                    | "stock_ledger"
                    | "dig_designation"
            ),
            Self::Spatial => matches!(
                name,
                "unit"
                    | "item"
                    | "building"
                    | "stockpile"
                    | "zone"
                    | "burrow"
                    | "tile_feature"
                    | "plant"
                    | "creature"
                    | "dig_designation"
            ),
            Self::Historical => {
                matches!(name, "historical_figure" | "civilization" | "announcement")
            }
            Self::ResearchFull => true,
        }
    }

    #[must_use]
    pub const fn includes_map_chunks(self) -> bool {
        matches!(
            self,
            Self::ControlMinimum | Self::Spatial | Self::ResearchFull
        )
    }

    #[must_use]
    pub const fn includes_events(self) -> bool {
        matches!(
            self,
            Self::Operations | Self::Historical | Self::ResearchFull
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectionProvenance {
    pub source_schema: String,
    pub source_manifest: Digest32,
}

/// Immutable projection envelope. Optional extensions are opaque, ordered,
/// retained byte-for-byte, and covered by identity; unknown critical identities
/// such as a profile or schema envelope version are rejected.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfiledSnapshot {
    profile: CompletenessProfile,
    source_anchor: StateAnchor,
    provenance: ProjectionProvenance,
    snapshot: WorldSnapshot,
    extensions: BTreeMap<String, Vec<u8>>,
    digest: Digest32,
}

impl ProfiledSnapshot {
    pub fn project(
        source: &WorldSnapshot,
        profile: CompletenessProfile,
        provenance: ProjectionProvenance,
        extensions: BTreeMap<String, Vec<u8>>,
    ) -> Result<Self> {
        validate_provenance(&provenance, &extensions)?;
        validate_snapshot(source)?;
        let mut snapshot = WorldSnapshot::from_canonical_bytes(&source.canonical_bytes())?;
        for entity in snapshot.graph.entities.values_mut() {
            if !profile.includes_entity_fields(&entity.kind) {
                omit_fields(&mut entity.fields, profile);
            }
        }
        for edge in snapshot.graph.edges.values_mut() {
            let included = [edge.from, edge.to].iter().all(|id| {
                snapshot
                    .graph
                    .entities
                    .get(id)
                    .is_some_and(|entity| profile.includes_entity_fields(&entity.kind))
            });
            if !included {
                omit_fields(&mut edge.fields, profile);
            }
        }
        if !profile.includes_map_chunks() {
            snapshot.graph.chunks.clear();
        }
        if !profile.includes_events() {
            snapshot.graph.events.clear();
        }
        snapshot.refresh_hash();
        Self::seal(profile, source.anchor(), provenance, snapshot, extensions)
    }

    fn seal(
        profile: CompletenessProfile,
        source_anchor: StateAnchor,
        provenance: ProjectionProvenance,
        snapshot: WorldSnapshot,
        extensions: BTreeMap<String, Vec<u8>>,
    ) -> Result<Self> {
        validate_provenance(&provenance, &extensions)?;
        validate_snapshot(&snapshot)?;
        if source_anchor.fortress_id == FortressId::NIL
            || source_anchor.state_hash == Digest32::ZERO
            || source_anchor.fortress_id != snapshot.fortress_id
            || source_anchor.cursor != snapshot.cursor
            || source_anchor.tick != snapshot.tick
        {
            return Err(invalid(
                "projection source anchor does not match its observation identity",
            ));
        }
        if profile == CompletenessProfile::ResearchFull
            && source_anchor.state_hash != snapshot.state_hash
        {
            return Err(invalid(
                "research-full must retain the complete canonical source bytes",
            ));
        }
        validate_projection(&snapshot, profile)?;
        let mut result = Self {
            profile,
            source_anchor,
            provenance,
            snapshot,
            extensions,
            digest: Digest32::ZERO,
        };
        result.digest = Digest32::of_bytes(&result.canonical_bytes()?);
        Ok(result)
    }

    #[must_use]
    pub const fn profile(&self) -> CompletenessProfile {
        self.profile
    }
    #[must_use]
    pub const fn source_anchor(&self) -> StateAnchor {
        self.source_anchor
    }
    #[must_use]
    pub const fn snapshot(&self) -> &WorldSnapshot {
        &self.snapshot
    }
    #[must_use]
    pub const fn provenance(&self) -> &ProjectionProvenance {
        &self.provenance
    }
    #[must_use]
    pub const fn extensions(&self) -> &BTreeMap<String, Vec<u8>> {
        &self.extensions
    }
    #[must_use]
    pub const fn digest(&self) -> Digest32 {
        self.digest
    }

    /// Verify the projection against the independently retained canonical
    /// source. An envelope digest alone is integrity, not source authenticity.
    pub fn verify_against_source(&self, source: &WorldSnapshot) -> Result<()> {
        if source.anchor() != self.source_anchor {
            return Err(stale("projection names a different canonical source"));
        }
        let expected = Self::project(
            source,
            self.profile,
            self.provenance.clone(),
            self.extensions.clone(),
        )?;
        if expected != *self {
            return Err(invalid(
                "projected content does not follow its declared source and profile",
            ));
        }
        Ok(())
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>> {
        validate_provenance(&self.provenance, &self.extensions)?;
        validate_snapshot(&self.snapshot)?;
        let mut bytes = Vec::new();
        put_str(&mut bytes, SNAPSHOT_DOMAIN);
        encode_identity(&mut bytes, self.profile, &self.provenance, &self.extensions);
        put_anchor(&mut bytes, self.source_anchor);
        put_bytes(&mut bytes, &self.snapshot.canonical_bytes());
        finish_bytes(bytes)
    }

    pub fn from_canonical_bytes(bytes: &[u8]) -> Result<Self> {
        let mut reader = Reader::new(bytes)?;
        reader.domain(SNAPSHOT_DOMAIN)?;
        let (profile, provenance, extensions) = reader.identity()?;
        let source_anchor = reader.anchor()?;
        let snapshot = WorldSnapshot::from_canonical_bytes(reader.bytes()?)?;
        reader.finish()?;
        let result = Self::seal(profile, source_anchor, provenance, snapshot, extensions)?;
        if result.canonical_bytes()? != bytes {
            return Err(invalid("projection envelope is not canonically encoded"));
        }
        Ok(result)
    }
}

/// A delta of one exact projection identity, retaining canonical source
/// anchors separately. It cannot be applied to another profile, manifest,
/// optional-extension set, source anchor, or projected base.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfiledObservationCapsule {
    basis_digest: Digest32,
    successor_source: StateAnchor,
    successor_digest: Digest32,
    capsule: ObservationCapsule,
    digest: Digest32,
}

impl ProfiledObservationCapsule {
    pub fn between(
        base: &ProfiledSnapshot,
        target: &ProfiledSnapshot,
        published_at_tick: GameTick,
    ) -> Result<Self> {
        if base.profile != target.profile
            || base.provenance != target.provenance
            || base.extensions != target.extensions
        {
            return Err(stale(
                "profile, source manifest/schema, or optional extensions changed; a full projection is required",
            ));
        }
        let delta = diff_snapshots(&base.snapshot, &target.snapshot)?;
        validate_delta_encoding(&delta)?;
        let capsule = ObservationCapsule::new(
            base.snapshot.anchor(),
            target.snapshot.anchor(),
            delta,
            published_at_tick,
        )?;
        let mut result = Self {
            basis_digest: base.digest,
            successor_source: target.source_anchor,
            successor_digest: target.digest,
            capsule,
            digest: Digest32::ZERO,
        };
        result.digest = Digest32::of_bytes(&result.canonical_bytes()?);
        Ok(result)
    }

    #[must_use]
    pub const fn digest(&self) -> Digest32 {
        self.digest
    }
    #[must_use]
    pub const fn capsule(&self) -> &ObservationCapsule {
        &self.capsule
    }

    pub fn apply(&self, base: &ProfiledSnapshot) -> Result<ProfiledSnapshot> {
        validate_delta_encoding(&self.capsule.delta)?;
        if base.digest != self.basis_digest
            || base.snapshot.anchor() != self.capsule.basis_anchor
            || !self.capsule.integrity_is_valid()
        {
            return Err(stale("profiled capsule does not match its exact base"));
        }
        let snapshot = apply_delta(&base.snapshot, &self.capsule.delta)?;
        let target = ProfiledSnapshot::seal(
            base.profile,
            self.successor_source,
            base.provenance.clone(),
            snapshot,
            base.extensions.clone(),
        )?;
        if target.digest != self.successor_digest {
            return Err(invalid(
                "profiled capsule did not reconstruct its declared envelope",
            ));
        }
        Ok(target)
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>> {
        validate_delta_encoding(&self.capsule.delta)?;
        let mut bytes = Vec::new();
        put_str(&mut bytes, CAPSULE_DOMAIN);
        put_bytes(&mut bytes, self.basis_digest.as_bytes());
        put_anchor(&mut bytes, self.successor_source);
        put_bytes(&mut bytes, self.successor_digest.as_bytes());
        put_anchor(&mut bytes, self.capsule.basis_anchor);
        put_anchor(&mut bytes, self.capsule.successor_anchor);
        put_u64(&mut bytes, self.capsule.published_at_tick.0);
        put_bytes(&mut bytes, &self.capsule.delta.canonical_bytes());
        finish_bytes(bytes)
    }

    /// Decoding requires the exact base so delta content, profile provenance,
    /// and the reconstructed target can all be verified before use.
    pub fn from_canonical_bytes(bytes: &[u8], base: &ProfiledSnapshot) -> Result<Self> {
        let mut reader = Reader::new(bytes)?;
        reader.domain(CAPSULE_DOMAIN)?;
        let basis_digest = reader.digest()?;
        let successor_source = reader.anchor()?;
        let successor_digest = reader.digest()?;
        let basis_anchor = reader.anchor()?;
        let successor_anchor = reader.anchor()?;
        let published_at_tick = GameTick(reader.u64()?);
        let delta = crate::StateDelta::from_canonical_bytes(reader.bytes()?)?;
        reader.finish()?;
        validate_delta_encoding(&delta)?;
        let capsule =
            ObservationCapsule::new(basis_anchor, successor_anchor, delta, published_at_tick)?;
        let result = Self {
            basis_digest,
            successor_source,
            successor_digest,
            capsule,
            digest: Digest32::of_bytes(bytes),
        };
        result.apply(base)?;
        if result.canonical_bytes()? != bytes {
            return Err(invalid("profiled capsule is not canonically encoded"));
        }
        Ok(result)
    }
}

fn invalid(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::InvalidRequest, message)
}
fn stale(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::StaleAnchor, message)
}
fn bound() -> DfmcpError {
    DfmcpError::new(
        ErrorCode::BudgetExceeded,
        "profiled observation exceeds its explicit encoding bound",
    )
}

fn omit_fields(fields: &mut BTreeMap<String, Fact>, profile: CompletenessProfile) {
    for fact in fields.values_mut() {
        *fact = Fact::with_presence(
            FactPresence::Omitted(profile.as_str().to_owned()),
            fact.observed_at,
            fact.source.clone(),
            fact.source_digest,
        );
    }
}

fn validate_projection(snapshot: &WorldSnapshot, profile: CompletenessProfile) -> Result<()> {
    let omitted = |fact: &Fact| {
        fact.value == Value::Null
            && fact.presence.as_ref() == Some(&FactPresence::Omitted(profile.as_str().to_owned()))
    };
    for entity in snapshot.graph.entities.values() {
        if !profile.includes_entity_fields(&entity.kind) && !entity.fields.values().all(omitted) {
            return Err(invalid(
                "excluded entity fields must explicitly report profile omission",
            ));
        }
    }
    for edge in snapshot.graph.edges.values() {
        let included = [edge.from, edge.to].iter().all(|id| {
            snapshot
                .graph
                .entities
                .get(id)
                .is_some_and(|e| profile.includes_entity_fields(&e.kind))
        });
        if !included && !edge.fields.values().all(omitted) {
            return Err(invalid(
                "excluded relation fields must explicitly report profile omission",
            ));
        }
    }
    if (!profile.includes_map_chunks() && !snapshot.graph.chunks.is_empty())
        || (!profile.includes_events() && !snapshot.graph.events.is_empty())
    {
        return Err(invalid(
            "projection includes a domain excluded by its profile",
        ));
    }
    Ok(())
}

fn validate_provenance(
    provenance: &ProjectionProvenance,
    extensions: &BTreeMap<String, Vec<u8>>,
) -> Result<()> {
    if provenance.source_schema.is_empty()
        || provenance.source_schema.len() > 256
        || provenance.source_schema.contains('\0')
        || provenance.source_manifest == Digest32::ZERO
    {
        return Err(invalid(
            "projection requires an explicit bounded source schema and nonzero manifest digest",
        ));
    }
    if extensions.len() > MAX_EXTENSIONS {
        return Err(bound());
    }
    let mut bytes = 0usize;
    for (key, value) in extensions {
        if key.is_empty() || key.len() > 128 || key.contains('\0') {
            return Err(invalid("invalid optional projection extension name"));
        }
        bytes = bytes
            .checked_add(key.len())
            .and_then(|n| n.checked_add(value.len()))
            .ok_or_else(bound)?;
        if bytes > MAX_EXTENSION_BYTES {
            return Err(bound());
        }
    }
    Ok(())
}

fn encode_identity(
    bytes: &mut Vec<u8>,
    profile: CompletenessProfile,
    provenance: &ProjectionProvenance,
    extensions: &BTreeMap<String, Vec<u8>>,
) {
    put_str(bytes, profile.as_str());
    put_str(bytes, &provenance.source_schema);
    put_bytes(bytes, provenance.source_manifest.as_bytes());
    put_u64(bytes, extensions.len() as u64);
    for (name, payload) in extensions {
        put_str(bytes, name);
        put_bytes(bytes, payload);
    }
}

fn finish_bytes(bytes: Vec<u8>) -> Result<Vec<u8>> {
    if bytes.len() > MAX_PROFILE_BYTES {
        Err(bound())
    } else {
        Ok(bytes)
    }
}

/// A conservative preflight bounds encoding before recursive encoding or a
/// clone can allocate an unbounded output. Each record charge exceeds its
/// fixed framing; variable content is charged separately.
struct EncodingBudget(usize);
impl EncodingBudget {
    fn add(&mut self, bytes: usize) -> Result<()> {
        self.0 = self.0.checked_add(bytes).ok_or_else(bound)?;
        if self.0 > MAX_PROFILE_BYTES - MAX_EXTENSION_BYTES - 4096 {
            Err(bound())
        } else {
            Ok(())
        }
    }
    fn text(&mut self, text: &str) -> Result<()> {
        self.add(8usize.saturating_add(text.len()))
    }
    fn value(&mut self, value: &Value, depth: usize) -> Result<()> {
        if depth > crate::canonical_decode::MAX_VALUE_DEPTH {
            return Err(bound());
        }
        self.add(17)?;
        match value {
            Value::Text(text) => self.text(text)?,
            Value::Bytes(bytes) => self.add(bytes.len())?,
            Value::List(values) => {
                for value in values {
                    self.value(value, depth + 1)?;
                }
            }
            Value::Object(values) => {
                for (key, value) in values {
                    self.text(key)?;
                    self.value(value, depth + 1)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    fn fact(&mut self, fact: &Fact, fortress: FortressId, tick: GameTick) -> Result<()> {
        self.add(128)?;
        self.value(&fact.value, 0)?;
        if fact.observed_at > tick {
            return Err(invalid("fact observation tick is ahead of its snapshot"));
        }
        match &fact.source {
            crate::FactSource::DfhackField(name)
            | crate::FactSource::Derived(name)
            | crate::FactSource::AgentAssertion(name) => self.text(name)?,
            crate::FactSource::Replay => {}
        }
        match &fact.presence {
            Some(FactPresence::Known(value)) => {
                self.value(value, 0)?;
                if value != &fact.value {
                    return Err(invalid("known presence disagrees with the fact value"));
                }
            }
            Some(presence) => {
                if fact.value != Value::Null {
                    return Err(invalid("nonknown presence retains a contradictory value"));
                }
                match presence {
                    FactPresence::Unknown(reason)
                    | FactPresence::Unsupported(reason)
                    | FactPresence::Omitted(reason)
                    | FactPresence::Redacted(reason) => self.text(reason)?,
                    FactPresence::Stale(anchor)
                        if anchor.fortress_id != fortress
                            || anchor.tick > tick
                            || anchor.state_hash == Digest32::ZERO =>
                    {
                        return Err(invalid(
                            "stale fact names a different fortress, future observation, or missing anchor hash",
                        ));
                    }
                    _ => {}
                }
            }
            None => {}
        }
        Ok(())
    }
    fn entity(
        &mut self,
        entity: &crate::EntityRecord,
        fortress: FortressId,
        tick: GameTick,
    ) -> Result<()> {
        self.add(64)?;
        self.text(entity.kind.as_str())?;
        self.text(&entity.label)?;
        for (key, fact) in &entity.fields {
            self.text(key)?;
            self.fact(fact, fortress, tick)?;
        }
        Ok(())
    }
    fn edge(
        &mut self,
        edge: &crate::EdgeRecord,
        fortress: FortressId,
        tick: GameTick,
    ) -> Result<()> {
        self.add(64)?;
        self.text(edge.kind.as_str())?;
        for (key, fact) in &edge.fields {
            self.text(key)?;
            self.fact(fact, fortress, tick)?;
        }
        Ok(())
    }
    fn chunk(&mut self, chunk: &crate::MapChunk) -> Result<()> {
        self.add(64)?;
        self.add(chunk.terrain_runs.len().checked_mul(8).ok_or_else(bound)?)?;
        for fields in chunk.sparse_overlays.values() {
            self.add(12)?;
            for (key, value) in fields {
                self.text(key)?;
                self.value(value, 1)?;
            }
        }
        Ok(())
    }
    fn event(&mut self, event: &crate::WorldEvent) -> Result<()> {
        self.add(64)?;
        self.text(event.kind.as_str())?;
        self.text(&event.summary)?;
        for (key, value) in &event.fields {
            self.text(key)?;
            self.value(value, 1)?;
        }
        Ok(())
    }
}

fn validate_snapshot(snapshot: &WorldSnapshot) -> Result<()> {
    let mut budget = EncodingBudget(256);
    if snapshot.fortress_id == FortressId::NIL {
        return Err(invalid("snapshot fortress identifier is reserved"));
    }
    for (id, entity) in &snapshot.graph.entities {
        budget.entity(entity, snapshot.fortress_id, snapshot.tick)?;
        if *id == dfmcp_core::EntityId::NIL || *id != entity.id {
            return Err(invalid("entity map identity is invalid"));
        }
    }
    for (id, edge) in &snapshot.graph.edges {
        budget.edge(edge, snapshot.fortress_id, snapshot.tick)?;
        if *id == dfmcp_core::EdgeId::NIL
            || *id != edge.id
            || !snapshot.graph.entities.contains_key(&edge.from)
            || !snapshot.graph.entities.contains_key(&edge.to)
        {
            return Err(invalid("relation identity or endpoint is invalid"));
        }
    }
    for (coord, chunk) in &snapshot.graph.chunks {
        budget.chunk(chunk)?;
        if *coord != chunk.coord
            || chunk.width == 0
            || chunk.height == 0
            || chunk.encoded_tile_count() != Some(chunk.tile_count())
        {
            return Err(invalid("spatial chunk identity or coverage is invalid"));
        }
        for offset in chunk.sparse_overlays.keys() {
            if *offset >= chunk.tile_count() {
                return Err(invalid("spatial overlay lies outside its chunk"));
            }
        }
    }
    for (id, event) in &snapshot.graph.events {
        budget.event(event)?;
        if *id == dfmcp_core::EventId::NIL || *id != event.id || event.tick > snapshot.tick {
            return Err(invalid("event identity or tick is invalid"));
        }
    }
    if !snapshot.hash_is_valid() {
        return Err(invalid("snapshot canonical hash is invalid"));
    }
    Ok(())
}

fn validate_delta_encoding(delta: &crate::StateDelta) -> Result<()> {
    let mut budget = EncodingBudget(512);
    if delta.changes.len() > crate::delta::MAX_STATE_DELTA_CHANGES {
        return Err(bound());
    }
    for change in &delta.changes {
        budget.add(32)?;
        match change {
            crate::WorldChange::UpsertEntity(entity) => {
                budget.entity(entity, delta.fortress_id, delta.target_tick)?
            }
            crate::WorldChange::UpsertEdge(edge) => {
                budget.edge(edge, delta.fortress_id, delta.target_tick)?
            }
            crate::WorldChange::UpsertMapChunk(chunk) => budget.chunk(chunk)?,
            crate::WorldChange::AppendEvent(event) => budget.event(event)?,
            crate::WorldChange::RemoveEntity { .. }
            | crate::WorldChange::RemoveEdge { .. }
            | crate::WorldChange::RemoveMapChunk { .. } => {}
        }
    }
    if let Some(continuation) = &delta.continuation {
        budget.text(continuation)?;
    }
    Ok(())
}

struct Reader<'a> {
    bytes: &'a [u8],
    offset: usize,
}
impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() > MAX_PROFILE_BYTES {
            return Err(bound());
        }
        Ok(Self { bytes, offset: 0 })
    }
    fn take(&mut self, count: usize) -> Result<&'a [u8]> {
        let end = self
            .offset
            .checked_add(count)
            .filter(|end| *end <= self.bytes.len())
            .ok_or_else(|| invalid("profile envelope is truncated"))?;
        let bytes = &self.bytes[self.offset..end];
        self.offset = end;
        Ok(bytes)
    }
    fn u64(&mut self) -> Result<u64> {
        let mut raw = [0; 8];
        raw.copy_from_slice(self.take(8)?);
        Ok(u64::from_be_bytes(raw))
    }
    fn bytes(&mut self) -> Result<&'a [u8]> {
        let count = usize::try_from(self.u64()?).map_err(|_| bound())?;
        self.take(count)
    }
    fn text(&mut self) -> Result<String> {
        String::from_utf8(self.bytes()?.to_vec())
            .map_err(|_| invalid("profile envelope text is not UTF-8"))
    }
    fn digest(&mut self) -> Result<Digest32> {
        let bytes = self.bytes()?;
        if bytes.len() != 32 {
            return Err(invalid("profile digest has the wrong length"));
        }
        let mut raw = [0; 32];
        raw.copy_from_slice(bytes);
        Ok(Digest32::from_bytes(raw))
    }
    fn anchor(&mut self) -> Result<StateAnchor> {
        Ok(StateAnchor {
            fortress_id: FortressId::new(self.u64()?),
            cursor: ObservationCursor {
                epoch: self.u64()?,
                sequence: self.u64()?,
            },
            tick: GameTick(self.u64()?),
            state_hash: self.digest()?,
        })
    }
    fn domain(&mut self, domain: &str) -> Result<()> {
        if self.text()? == domain {
            Ok(())
        } else {
            Err(invalid("unknown profile envelope version"))
        }
    }
    fn identity(
        &mut self,
    ) -> Result<(
        CompletenessProfile,
        ProjectionProvenance,
        BTreeMap<String, Vec<u8>>,
    )> {
        let profile = CompletenessProfile::parse(&self.text()?)?;
        let provenance = ProjectionProvenance {
            source_schema: self.text()?,
            source_manifest: self.digest()?,
        };
        let count = usize::try_from(self.u64()?).map_err(|_| bound())?;
        if count > MAX_EXTENSIONS {
            return Err(bound());
        }
        let mut extensions = BTreeMap::new();
        for _ in 0..count {
            let key = self.text()?;
            if extensions
                .last_key_value()
                .is_some_and(|(last, _)| last >= &key)
            {
                return Err(invalid("projection extensions are not strictly ordered"));
            }
            extensions.insert(key, self.bytes()?.to_vec());
        }
        validate_provenance(&provenance, &extensions)?;
        Ok((profile, provenance, extensions))
    }
    fn finish(self) -> Result<()> {
        if self.offset == self.bytes.len() {
            Ok(())
        } else {
            Err(invalid("profile envelope has trailing bytes"))
        }
    }
}

#[cfg(test)]
mod tests;
