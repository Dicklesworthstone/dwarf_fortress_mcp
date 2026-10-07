//! Source- and coverage-qualified predicate evidence.
//!
//! A canonical hash establishes content integrity, not observation authority.
//! These scopes are issued by trusted observation owners after source admission
//! and completeness checks. They deliberately have no serialization interface:
//! a caller-supplied profile, source label, digest, confidence, or coverage report
//! is not a scope. The borrowed snapshot cannot change after the scope is bound.
//!
//! The laboratory constructor is an explicit contract for the complete reference
//! world maintained by the memory adapter. It admits only the two registered
//! reference derivations with their existing zero-digest convention. It must not
//! be used to promote imported, projected, hypothetical, or live snapshots into
//! complete laboratory observations. Live sources require independently issued
//! exact source/digest and domain grants through `scoped`.

use std::collections::{BTreeMap, BTreeSet};

use dfmcp_core::{
    DfmcpError, Digest32, EdgeId, EntityId, ErrorCode, GameTick, MapCoord, MapCuboid, Result,
    StateAnchor,
};

use crate::{
    EdgeKind, EntityKind, Fact, FactSource, Predicate, PredicateTruth, Value, WorldSnapshot,
};

const MAX_EVIDENCE_SOURCES: usize = 256;
const MAX_EVIDENCE_KINDS: usize = 64;
const MAX_EVIDENCE_SOURCE_BYTES: usize = 256;
const MAX_EVIDENCE_KIND_BYTES: usize = 128;
const MAX_EVIDENCE_TERRAIN_REGIONS: usize = 64;
const MAX_EVIDENCE_TERRAIN_TILES: u64 = 262_144;
const LAB_SCENARIO: &str = "dfmcp.lab-scenario/1";
const LAB_EFFECTS: &str = "dfmcp.reference-effects/1";

/// Positive observation and complete-domain absence are distinct permissions.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EvidenceCoverage {
    #[default]
    Unknown,
    /// Present members of this domain are authoritative; missing members are unknown.
    Observed,
    /// All members of this domain are observed, allowing scoped absence evidence.
    Complete,
}

impl EvidenceCoverage {
    const fn observes_members(self) -> bool {
        matches!(self, Self::Observed | Self::Complete)
    }

    const fn proves_absence(self) -> bool {
        matches!(self, Self::Complete)
    }
}

/// An exact source authorization issued by a trusted observation owner.
///
/// Observed grants require prior bridge compatibility/source validation. A
/// certified derivation grant requires a registered derivation whose complete
/// input provenance the issuer has validated. Supplying a matching string and
/// digest is not itself that validation. Replay and agent assertions cannot be
/// represented by these variants and never acquire mutation eligibility here.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum EvidenceSource {
    Observed {
        field: String,
        source_digest: Digest32,
    },
    CertifiedDerived {
        derivation: String,
        source_digest: Digest32,
    },
}

impl EvidenceSource {
    fn validate(&self) -> Result<()> {
        let (name, source_digest) = match self {
            Self::Observed {
                field,
                source_digest,
            } => (field, source_digest),
            Self::CertifiedDerived {
                derivation,
                source_digest,
            } => (derivation, source_digest),
        };
        validate_name(name, MAX_EVIDENCE_SOURCE_BYTES, "evidence source")?;
        if *source_digest == Digest32::ZERO {
            return Err(DfmcpError::new(
                ErrorCode::InvalidRequest,
                "observed and certified-derived source grants require a nonzero exact digest",
            ));
        }
        Ok(())
    }

    fn admits(&self, fact: &Fact) -> bool {
        match (self, &fact.source) {
            (
                Self::Observed {
                    field,
                    source_digest,
                },
                FactSource::DfhackField(actual),
            ) => field == actual && *source_digest == fact.source_digest,
            (
                Self::CertifiedDerived {
                    derivation,
                    source_digest,
                },
                FactSource::Derived(actual),
            ) => derivation == actual && *source_digest == fact.source_digest,
            _ => false,
        }
    }
}

/// Trusted, process-local observation authority for one exact source anchor.
///
/// The default grants nothing and is deliberately unbound. Use `at` to bind an
/// empty scope, then add only domains and sources independently established by
/// the observing shell. A complete kind domain does not prove unqualified
/// entity/edge absence across all other kinds. Terrain grants name bounded
/// observed regions; missing or malformed canonical tiles remain unknown.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EvidencePolicy {
    pub anchor: Option<StateAnchor>,
    pub sources: BTreeSet<EvidenceSource>,
    pub all_entities: EvidenceCoverage,
    pub entity_kinds: BTreeMap<EntityKind, EvidenceCoverage>,
    pub all_edges: EvidenceCoverage,
    pub edge_kinds: BTreeMap<EdgeKind, EvidenceCoverage>,
    pub paused: bool,
    pub terrain_regions: Vec<MapCuboid>,
}

impl EvidencePolicy {
    #[must_use]
    pub fn at(anchor: StateAnchor) -> Self {
        Self {
            anchor: Some(anchor),
            ..Self::default()
        }
    }

    fn validate(&self) -> Result<()> {
        if self.sources.len() > MAX_EVIDENCE_SOURCES
            || self.entity_kinds.len() > MAX_EVIDENCE_KINDS
            || self.edge_kinds.len() > MAX_EVIDENCE_KINDS
            || self.terrain_regions.len() > MAX_EVIDENCE_TERRAIN_REGIONS
        {
            return Err(DfmcpError::new(
                ErrorCode::BudgetExceeded,
                "predicate evidence source or coverage collections exceed their explicit bounds",
            ));
        }
        for source in &self.sources {
            source.validate()?;
        }
        let mut entity_names = BTreeSet::new();
        for kind in self.entity_kinds.keys() {
            validate_name(
                kind.as_str(),
                MAX_EVIDENCE_KIND_BYTES,
                "evidence entity kind",
            )?;
            if !entity_names.insert(kind.as_str()) {
                return Err(DfmcpError::new(
                    ErrorCode::InvalidRequest,
                    "predicate evidence has duplicate canonical entity-kind grants",
                ));
            }
        }
        let mut edge_names = BTreeSet::new();
        for kind in self.edge_kinds.keys() {
            validate_name(
                kind.as_str(),
                MAX_EVIDENCE_KIND_BYTES,
                "evidence relation kind",
            )?;
            if !edge_names.insert(kind.as_str()) {
                return Err(DfmcpError::new(
                    ErrorCode::InvalidRequest,
                    "predicate evidence has duplicate canonical relation-kind grants",
                ));
            }
        }
        let mut terrain_tiles = 0u64;
        for area in &self.terrain_regions {
            terrain_tiles = terrain_tiles
                .checked_add(crate::terrain::validate_region(*area)?)
                .ok_or_else(|| {
                    DfmcpError::new(
                        ErrorCode::BudgetExceeded,
                        "evidence terrain count overflowed",
                    )
                })?;
            if terrain_tiles > MAX_EVIDENCE_TERRAIN_TILES {
                return Err(DfmcpError::new(
                    ErrorCode::BudgetExceeded,
                    "predicate evidence terrain coverage exceeds its explicit tile bound",
                ));
            }
        }
        Ok(())
    }
}

fn validate_name(value: &str, maximum: usize, field: &str) -> Result<()> {
    if value.is_empty() || value.len() > maximum || value.chars().any(char::is_control) {
        return Err(DfmcpError::new(
            ErrorCode::InvalidRequest,
            format!("{field} must contain 1..={maximum} non-control UTF-8 bytes"),
        ));
    }
    Ok(())
}

/// A known fact that can be consumed by the registered laboratory simulation.
///
/// This is intentionally narrower than `Fact::known_value`: assertions,
/// replay, arbitrary derived labels, live field labels, changed producer
/// digests, unavailable presence, and future observations do not supply inputs
/// from which the simulator may manufacture reference-derived outputs.
#[must_use]
pub fn laboratory_fact_value(fact: &Fact, snapshot_tick: GameTick) -> Option<&Value> {
    if fact.observed_at > snapshot_tick
        || fact.source_digest != Digest32::ZERO
        || !matches!(&fact.source, FactSource::Derived(name) if name == LAB_SCENARIO || name == LAB_EFFECTS)
    {
        return None;
    }
    fact.known_value()
}

#[must_use]
pub fn lab_fact_is_eligible(fact: &Fact, snapshot_tick: GameTick) -> bool {
    laboratory_fact_value(fact, snapshot_tick).is_some()
}

/// Immutable predicate evidence bound to a single validated world version.
#[derive(Clone, Debug)]
pub struct PredicateEvidence<'a> {
    snapshot: &'a WorldSnapshot,
    policy: EvidencePolicy,
    laboratory: bool,
}

impl<'a> PredicateEvidence<'a> {
    /// Preserve a snapshot for inspection without granting any fact authority.
    pub fn untrusted(snapshot: &'a WorldSnapshot) -> Result<Self> {
        Self::scoped(snapshot, EvidencePolicy::at(snapshot.anchor()))
    }

    /// Bind independently issued source/domain grants to this exact snapshot.
    /// The caller owns source admission; validating this scope does not perform
    /// bridge admission, derivation verification, or capability authorization.
    pub fn scoped(snapshot: &'a WorldSnapshot, policy: EvidencePolicy) -> Result<Self> {
        policy.validate()?;
        if policy.anchor != Some(snapshot.anchor()) {
            return Err(DfmcpError::new(
                ErrorCode::StaleAnchor,
                "predicate evidence policy must name the exact supplied state anchor",
            ));
        }
        // Canonical graph bytes encode record identities, not the BTreeMap's
        // lookup keys. Check both before treating a lookup as anchored evidence.
        if snapshot
            .graph
            .entities
            .iter()
            .any(|(id, entity)| *id != entity.id)
            || snapshot.graph.edges.iter().any(|(id, edge)| *id != edge.id)
            || snapshot
                .graph
                .chunks
                .iter()
                .any(|(coord, chunk)| *coord != chunk.coord)
            || snapshot
                .graph
                .events
                .iter()
                .any(|(id, event)| *id != event.id)
        {
            return Err(DfmcpError::new(
                ErrorCode::InternalInvariantViolation,
                "predicate evidence requires canonical lookup keys matching record identities",
            ));
        }
        if !snapshot.hash_is_valid() {
            return Err(DfmcpError::new(
                ErrorCode::ChecksumMismatch,
                "predicate evidence requires a hash-valid canonical snapshot",
            ));
        }
        Ok(Self {
            snapshot,
            policy,
            laboratory: false,
        })
    }

    /// Trust the complete reference model owned by a laboratory adapter.
    ///
    /// This explicit constructor is not a live-source or imported-data
    /// admission path. Entity and edge universes and pause are complete within
    /// that model, while terrain remains unknown wherever no canonical tile is
    /// stored. Only registered laboratory derivation facts are usable inputs.
    pub fn laboratory(snapshot: &'a WorldSnapshot) -> Result<Self> {
        let mut policy = EvidencePolicy::at(snapshot.anchor());
        policy.all_entities = EvidenceCoverage::Complete;
        policy.all_edges = EvidenceCoverage::Complete;
        policy.paused = true;
        let mut evidence = Self::scoped(snapshot, policy)?;
        evidence.laboratory = true;
        Ok(evidence)
    }

    #[must_use]
    pub const fn snapshot(&self) -> &'a WorldSnapshot {
        self.snapshot
    }

    /// Evaluate all leaves under this scope, preserving unknowns through logic.
    /// Bounds are validated across the entire tree before any short circuit.
    pub fn evaluate(&self, predicate: &Predicate) -> Result<PredicateTruth> {
        predicate.validate_shape()?;
        Ok(self.evaluate_validated(predicate))
    }

    /// Only an established true result can satisfy a precondition or proof.
    pub fn establishes(&self, predicate: &Predicate) -> Result<bool> {
        Ok(matches!(self.evaluate(predicate)?, PredicateTruth::True))
    }

    // EntityKind::Other("unit") and EdgeKind::Custom("assigned_to")
    // encode exactly like their named variants. Evidence must follow those
    // canonical identities, not the in-memory enum representation. Duplicate
    // canonical grants are rejected at construction, so lookup is unambiguous.
    fn entity_kind_coverage(&self, kind: &EntityKind) -> EvidenceCoverage {
        self.policy
            .entity_kinds
            .iter()
            .find(|(candidate, _)| candidate.as_str() == kind.as_str())
            .map_or(EvidenceCoverage::Unknown, |(_, coverage)| *coverage)
    }

    fn edge_kind_coverage(&self, kind: &EdgeKind) -> EvidenceCoverage {
        self.policy
            .edge_kinds
            .iter()
            .find(|(candidate, _)| candidate.as_str() == kind.as_str())
            .map_or(EvidenceCoverage::Unknown, |(_, coverage)| *coverage)
    }

    fn entity_observed(&self, id: EntityId) -> bool {
        id != EntityId::NIL
            && self.snapshot.graph.entities.get(&id).is_some_and(|entity| {
                self.policy.all_entities.observes_members()
                    || self.entity_kind_coverage(&entity.kind).observes_members()
            })
    }

    fn entity_absence_complete(&self, kind: Option<&EntityKind>) -> bool {
        self.policy.all_entities.proves_absence()
            || kind.is_some_and(|kind| self.entity_kind_coverage(kind).proves_absence())
    }

    fn edge_absence_complete(&self, kind: Option<&EdgeKind>) -> bool {
        self.policy.all_edges.proves_absence()
            || kind.is_some_and(|kind| self.edge_kind_coverage(kind).proves_absence())
    }

    fn fact_value<'f>(&self, fact: &'f Fact) -> Option<&'f Value> {
        if self.laboratory {
            return laboratory_fact_value(fact, self.snapshot.tick);
        }
        if fact.observed_at > self.snapshot.tick
            || !self.policy.sources.iter().any(|source| source.admits(fact))
        {
            return None;
        }
        fact.known_value()
    }

    fn terrain_observed(&self, coord: MapCoord) -> bool {
        self.laboratory
            || self
                .policy
                .terrain_regions
                .iter()
                .any(|area| area.contains(coord))
    }

    fn evaluate_validated(&self, predicate: &Predicate) -> PredicateTruth {
        use PredicateTruth::{False, True, Unknown};
        match predicate {
            Predicate::True => True,
            Predicate::False => False,
            Predicate::EntityExists(id) => {
                if *id == EntityId::NIL {
                    Unknown
                } else if self.snapshot.graph.entities.contains_key(id) {
                    if self.entity_observed(*id) {
                        True
                    } else {
                        Unknown
                    }
                } else if self.entity_absence_complete(None) {
                    False
                } else {
                    Unknown
                }
            }
            Predicate::EntityKind { entity_id, kind } => {
                if *entity_id == EntityId::NIL {
                    Unknown
                } else if let Some(entity) = self.snapshot.graph.entities.get(entity_id) {
                    if !self.entity_observed(*entity_id) {
                        Unknown
                    } else if entity.kind.as_str() == kind.as_str() {
                        True
                    } else {
                        False
                    }
                } else if self.entity_absence_complete(Some(kind)) {
                    False
                } else {
                    Unknown
                }
            }
            Predicate::FieldCompare {
                entity_id,
                field,
                op,
                value,
            } => {
                if !self.entity_observed(*entity_id) {
                    return Unknown;
                }
                self.snapshot
                    .graph
                    .entities
                    .get(entity_id)
                    .and_then(|entity| entity.fields.get(field))
                    .and_then(|fact| self.fact_value(fact))
                    .map_or(Unknown, |known| crate::query::compare(known, *op, value))
            }
            Predicate::EdgeExists { edge_id, kind } => {
                if *edge_id == EdgeId::NIL {
                    return Unknown;
                }
                if let Some(edge) = self.snapshot.graph.edges.get(edge_id) {
                    let observed = self.policy.all_edges.observes_members()
                        || self.edge_kind_coverage(&edge.kind).observes_members();
                    if !observed
                        || !self.entity_observed(edge.from)
                        || !self.entity_observed(edge.to)
                    {
                        Unknown
                    } else if kind
                        .as_ref()
                        .is_none_or(|kind| kind.as_str() == edge.kind.as_str())
                    {
                        True
                    } else {
                        False
                    }
                } else if self.edge_absence_complete(kind.as_ref()) {
                    False
                } else {
                    Unknown
                }
            }
            Predicate::Paused(expected) => {
                if !self.policy.paused {
                    Unknown
                } else if self.snapshot.paused == *expected {
                    True
                } else {
                    False
                }
            }
            Predicate::RegionTerrain { area, tile_code } => {
                let mut unknown = false;
                for coord in crate::terrain::region_tiles(*area) {
                    let code = self
                        .terrain_observed(coord)
                        .then(|| self.snapshot.tile_code_at(coord))
                        .flatten();
                    match code {
                        Some(code) if code == *tile_code => {}
                        Some(_) => return False,
                        None => unknown = true,
                    }
                }
                if unknown { Unknown } else { True }
            }
            Predicate::All(children) => {
                let mut result = True;
                for child in children {
                    match self.evaluate_validated(child) {
                        False => return False,
                        Unknown => result = Unknown,
                        True => {}
                    }
                }
                result
            }
            Predicate::Any(children) => {
                let mut result = False;
                for child in children {
                    match self.evaluate_validated(child) {
                        True => return True,
                        Unknown => result = Unknown,
                        False => {}
                    }
                }
                result
            }
            Predicate::Not(child) => match self.evaluate_validated(child) {
                True => False,
                False => True,
                Unknown => Unknown,
            },
        }
    }
}

#[cfg(test)]
mod tests;
